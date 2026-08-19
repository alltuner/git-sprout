// ABOUTME: Parses a git index file into the facts a comparison cares about:
// ABOUTME: version, entries with oid/mode/flags, and the real extension chain.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

/// Extensions whose payload is a pure function of the tracked content, so a
/// difference in their bytes is a real difference. Everything else (untracked
/// cache, fsmonitor) embeds stat data or timestamps and is compared by presence.
const DETERMINISTIC_EXTENSIONS: &[&str] = &["TREE", "REUC"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub path: String,
    pub oid: String,
    pub mode: u32,
    pub stage: u16,
    pub assume_valid: bool,
    pub extended: bool,
    pub skip_worktree: bool,
    pub intent_to_add: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexExtension {
    pub signature: String,
    pub size: u32,
    /// Digest of the payload, only for extensions whose payload is deterministic.
    pub digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexFacts {
    pub version: u32,
    pub declared_entries: u32,
    pub entries: Vec<IndexEntry>,
    pub extensions: Vec<IndexExtension>,
    /// Set when the file could not be parsed; the rest of the facts are then partial.
    pub parse_error: Option<String>,
}

impl IndexFacts {
    pub fn missing(reason: &str) -> IndexFacts {
        IndexFacts {
            version: 0,
            declared_entries: 0,
            entries: Vec::new(),
            extensions: Vec::new(),
            parse_error: Some(reason.to_string()),
        }
    }
}

/// Number of bytes an object id occupies in the index, per the repository's
/// configured object format.
pub fn oid_len(object_format: &str) -> usize {
    match object_format {
        "sha256" => 32,
        _ => 20,
    }
}

pub fn read(path: &Path, object_format: &str) -> IndexFacts {
    match std::fs::read(path) {
        Ok(bytes) => parse(&bytes, oid_len(object_format)),
        Err(e) => IndexFacts::missing(&format!("cannot read {}: {e}", path.display())),
    }
}

pub fn parse(data: &[u8], oid_len: usize) -> IndexFacts {
    let mut facts = IndexFacts {
        version: 0,
        declared_entries: 0,
        entries: Vec::new(),
        extensions: Vec::new(),
        parse_error: None,
    };
    if data.len() < 12 + oid_len {
        facts.parse_error = Some(format!("index truncated: {} bytes", data.len()));
        return facts;
    }
    if &data[0..4] != b"DIRC" {
        facts.parse_error = Some(format!("bad signature {:?}", String::from_utf8_lossy(&data[0..4])));
        return facts;
    }
    facts.version = be32(&data[4..8]);
    facts.declared_entries = be32(&data[8..12]);
    if !matches!(facts.version, 2 | 3 | 4) {
        facts.parse_error = Some(format!("unsupported index version {}", facts.version));
        return facts;
    }

    // The trailing checksum is over the whole file including stat data, which
    // legitimately differs between two runs, so it is bounds, not content.
    let body_end = data.len() - oid_len;
    let mut pos = 12usize;
    let mut previous_path: Vec<u8> = Vec::new();

    for i in 0..facts.declared_entries {
        match read_entry(data, pos, body_end, oid_len, facts.version, &previous_path) {
            Ok((entry, next, path_bytes)) => {
                previous_path = path_bytes;
                pos = next;
                facts.entries.push(entry);
            }
            Err(e) => {
                facts.parse_error = Some(format!("entry {i}: {e}"));
                return facts;
            }
        }
    }

    while pos + 8 <= body_end {
        let signature = String::from_utf8_lossy(&data[pos..pos + 4]).into_owned();
        let size = be32(&data[pos + 4..pos + 8]);
        let start = pos + 8;
        let end = match start.checked_add(size as usize) {
            Some(end) if end <= body_end => end,
            _ => {
                facts.parse_error =
                    Some(format!("extension {signature} claims {size} bytes past end of index"));
                return facts;
            }
        };
        let digest = if DETERMINISTIC_EXTENSIONS.contains(&signature.as_str()) {
            Some(hex(&Sha256::digest(&data[start..end])))
        } else {
            None
        };
        facts.extensions.push(IndexExtension { signature, size, digest });
        pos = end;
    }
    if pos != body_end {
        facts.parse_error =
            Some(format!("{} trailing bytes after the extension chain", body_end - pos));
    }
    facts
}

fn read_entry(
    data: &[u8],
    pos: usize,
    body_end: usize,
    oid_len: usize,
    version: u32,
    previous_path: &[u8],
) -> Result<(IndexEntry, usize, Vec<u8>), String> {
    let fixed = 40 + oid_len + 2;
    if pos + fixed > body_end {
        return Err("truncated".to_string());
    }
    let mode = be32(&data[pos + 24..pos + 28]);
    let oid = hex(&data[pos + 40..pos + 40 + oid_len]);
    let flags = be16(&data[pos + 40 + oid_len..pos + 42 + oid_len]);
    let assume_valid = flags & 0x8000 != 0;
    let extended = flags & 0x4000 != 0;
    let stage = (flags & 0x3000) >> 12;
    let name_len = (flags & 0x0FFF) as usize;

    let mut cursor = pos + fixed;
    let (mut skip_worktree, mut intent_to_add) = (false, false);
    if extended {
        if version < 3 {
            return Err(format!("extended flag set in a v{version} index"));
        }
        if cursor + 2 > body_end {
            return Err("truncated extended flags".to_string());
        }
        let extra = be16(&data[cursor..cursor + 2]);
        skip_worktree = extra & 0x4000 != 0;
        intent_to_add = extra & 0x2000 != 0;
        cursor += 2;
    }

    let (path_bytes, next) = if version >= 4 {
        let (strip, after) = decode_varint(data, cursor, body_end)?;
        if strip as usize > previous_path.len() {
            return Err("prefix strip longer than the previous path".to_string());
        }
        let keep = previous_path.len() - strip as usize;
        let nul = memchr(data, after, body_end).ok_or("unterminated path")?;
        let mut path = previous_path[..keep].to_vec();
        path.extend_from_slice(&data[after..nul]);
        (path, nul + 1)
    } else {
        let nul = memchr(data, cursor, body_end).ok_or("unterminated path")?;
        let path = data[cursor..nul].to_vec();
        // v2/v3 entries are NUL-padded to a multiple of eight bytes.
        let raw = nul - pos;
        let padded = (raw + 8) & !7;
        (path, pos + padded)
    };

    // The 12-bit name length saturates at 0xFFF, above which git stops recording it.
    if name_len < 0x0FFF && name_len != path_bytes.len() {
        return Err(format!(
            "flags claim a {name_len}-byte path but the entry holds {}",
            path_bytes.len()
        ));
    }
    if next > body_end {
        return Err("entry runs past the end of the index".to_string());
    }

    let entry = IndexEntry {
        path: String::from_utf8_lossy(&path_bytes).into_owned(),
        oid,
        mode,
        stage,
        assume_valid,
        extended,
        skip_worktree,
        intent_to_add,
    };
    Ok((entry, next, path_bytes))
}

/// git's variable-width integer, as used for index v4 path prefix compression.
fn decode_varint(data: &[u8], mut pos: usize, end: usize) -> Result<(u64, usize), String> {
    if pos >= end {
        return Err("truncated varint".to_string());
    }
    let mut byte = data[pos];
    pos += 1;
    let mut value = (byte & 0x7f) as u64;
    while byte & 0x80 != 0 {
        if pos >= end {
            return Err("truncated varint".to_string());
        }
        value += 1;
        byte = data[pos];
        pos += 1;
        value = (value << 7) + (byte & 0x7f) as u64;
    }
    Ok((value, pos))
}

fn memchr(data: &[u8], from: usize, end: usize) -> Option<usize> {
    (from..end).find(|&i| data[i] == 0)
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn be16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}

pub fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
