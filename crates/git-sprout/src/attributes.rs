// ABOUTME: Works out which paths a checkout would rewrite on the way to the working tree.
// ABOUTME: A path whose bytes get converted cannot be judged by the index stat cache alone.

use std::borrow::Cow;
use std::collections::HashMap;

use gix_hash::ObjectId;

/// The attributes that decide whether checking a blob out rewrites its bytes. `crlf` is
/// the deprecated spelling of `text`, which git still reads where `text` is unspecified.
pub const CONVERTING_ATTRIBUTES: &[&str] = &[
    "text",
    "crlf",
    "eol",
    "ident",
    "filter",
    "working-tree-encoding",
];

/// What `git check-attr` reports for one attribute on one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Unspecified,
    Unset,
    Set,
    Named(String),
}

impl Value {
    fn parse(info: &[u8]) -> Value {
        match info {
            b"unspecified" => Value::Unspecified,
            b"unset" => Value::Unset,
            b"set" => Value::Set,
            other => Value::Named(String::from_utf8_lossy(other).into_owned()),
        }
    }

    fn applies(&self) -> bool {
        matches!(self, Value::Set | Value::Named(_))
    }
}

/// The byte order mark git skips at the start of an attributes file it reads from disk.
const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// The length from which git ignores a line of an attributes file, its
/// `ATTR_MAX_LINE_LENGTH`.
const MAX_LINE_LENGTH: usize = 2048;

/// Whether git reads from an attributes file's working copy the rules a checkout reads
/// from the blob the index records for it.
///
/// The copy has to be the blob byte for byte, or the blob with every LF written as CRLF,
/// which is how a checkout writes an LF blob where it converts line endings and which git
/// reads as the same lines. Nothing is fetched to compare: the copy is hashed the way git
/// hashes a blob, as it is and then with its CRLF turned back into LF, and is the blob
/// when either names it.
///
/// Being the blob is still not enough, because git reads a working copy line by line and
/// a blob out of the index as one buffer, and the two readers part company in three
/// places. Only the first skips a UTF-8 byte order mark. A NUL ends the line for the first
/// and the whole file for the second. And the first drops a line's CR before measuring
/// the line against the length limit, which the second does not, so a line that reaches
/// the limit only with its CR is ignored by the second alone. A blob any of that applies
/// to is read one way in the source and another way by a checkout.
pub fn same_rules(copy: &[u8], blob: &ObjectId) -> bool {
    blob_behind(copy, blob).is_some_and(|blob| {
        !blob.starts_with(UTF8_BOM)
            && !blob.contains(&0)
            && !blob
                .split(|byte| *byte == b'\n')
                .any(|line| line.len() == MAX_LINE_LENGTH && line.ends_with(b"\r"))
    })
}

/// The bytes of `blob`, if `copy` is that blob as a checkout may have written it.
fn blob_behind<'a>(copy: &'a [u8], blob: &ObjectId) -> Option<Cow<'a, [u8]>> {
    if blob_id(copy, blob.kind()).as_ref() == Some(blob) {
        return Some(Cow::Borrowed(copy));
    }
    let mut unconverted = Vec::with_capacity(copy.len());
    for (position, byte) in copy.iter().enumerate() {
        if *byte != b'\r' || copy.get(position + 1) != Some(&b'\n') {
            unconverted.push(*byte);
        }
    }
    (unconverted.len() != copy.len() && blob_id(&unconverted, blob.kind()).as_ref() == Some(blob))
        .then_some(Cow::Owned(unconverted))
}

/// The id git gives a blob holding these bytes.
fn blob_id(bytes: &[u8], kind: gix_hash::Kind) -> Option<ObjectId> {
    let mut hasher = gix_hash::hasher(kind);
    hasher.update(format!("blob {}\0", bytes.len()).as_bytes());
    hasher.update(bytes);
    hasher.try_finalize().ok()
}

/// The line-ending configuration in force for a worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineEndings {
    /// `core.autocrlf` is anything but unset or false: `true` or `input`, where checkout
    /// and add disagree about line endings, so a clean file need not equal what a checkout
    /// would write, or a value git would not accept.
    pub autocrlf: bool,
    /// `core.eol` is anything but unset, `lf` or `native`: `crlf`, which asks for CRLF in
    /// the working tree, or a value this does not know.
    pub crlf: bool,
}

impl LineEndings {
    /// Reads the two options the way git does. `autocrlf` is `core.autocrlf` as
    /// `git config --type=bool-or-str` prints it, so every spelling of a boolean git
    /// accepts (`yes`, `on`, `1`, `TRUE`, a key with no value) arrives as `true` or
    /// `false`, while `input` and anything else arrive as written. `eol` is `core.eol`,
    /// which git compares without regard to case. `None` is an unset key.
    ///
    /// Only the settings known to leave a checkout alone count as no conversion; any other
    /// value, including one git would reject, counts as converting, which leaves the path
    /// to git.
    pub fn from_config(autocrlf: Option<&str>, eol: Option<&str>) -> Self {
        LineEndings {
            autocrlf: !matches!(autocrlf, None | Some("false")),
            crlf: !eol.is_none_or(|eol| {
                ["lf", "native"]
                    .iter()
                    .any(|known| eol.eq_ignore_ascii_case(known))
            }),
        }
    }
}

/// Whether checking this path out would write something other than the blob's own bytes.
///
/// A conversion breaks the rule the clone plan rests on. The index stat cache says the
/// working-tree file is *clean*, which means the file run back through the clean filter
/// equals the blob — it does not mean the file equals what a checkout would write. The two
/// part company whenever the conversion is not reversible: a file with LF endings under
/// `eol=crlf` is clean and yet a checkout would write CRLF, which git itself warns about
/// as "LF will be replaced by CRLF the next time Git touches it". Such a path is left to
/// git, which writes the converted form.
pub fn converts(values: &HashMap<String, Value>, endings: LineEndings) -> bool {
    for name in ["ident", "filter", "working-tree-encoding"] {
        if values.get(name).is_some_and(Value::applies) {
            return true;
        }
    }
    let text = text_attribute(values);
    if text == &Value::Unset {
        return false;
    }
    if values.get("eol").is_some_and(Value::applies) {
        return true;
    }
    if text.applies() {
        return true;
    }
    endings.autocrlf || endings.crlf
}

/// The path's `text` attribute, or where that is unspecified its deprecated `crlf`
/// spelling, the way git's own conversion reads them: `crlf` means `text`, `-crlf` means
/// `-text`, and `crlf=input` means `text eol=lf`.
fn text_attribute(values: &HashMap<String, Value>) -> &Value {
    match values.get("text") {
        Some(text) if text != &Value::Unspecified => text,
        _ => values.get("crlf").unwrap_or(&Value::Unspecified),
    }
}

/// Reads the `<path> NUL <attribute> NUL <value> NUL` triples `git check-attr -z` writes.
pub fn parse_check_attr(output: &[u8]) -> HashMap<Vec<u8>, HashMap<String, Value>> {
    let mut fields = output.split(|byte| *byte == 0);
    let mut paths: HashMap<Vec<u8>, HashMap<String, Value>> = HashMap::new();
    while let (Some(path), Some(attribute), Some(info)) =
        (fields.next(), fields.next(), fields.next())
    {
        if path.is_empty() {
            break;
        }
        paths.entry(path.to_vec()).or_default().insert(
            String::from_utf8_lossy(attribute).into_owned(),
            Value::parse(info),
        );
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(pairs: &[(&str, Value)]) -> HashMap<String, Value> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_string(), value.clone()))
            .collect()
    }

    const PLAIN: LineEndings = LineEndings {
        autocrlf: false,
        crlf: false,
    };

    #[test]
    fn a_path_with_no_attributes_is_written_verbatim() {
        assert!(!converts(&values(&[]), PLAIN));
    }

    #[test]
    fn an_eol_attribute_rewrites_the_bytes() {
        assert!(converts(
            &values(&[("text", Value::Set), ("eol", Value::Named("crlf".into()))]),
            PLAIN
        ));
    }

    #[test]
    fn text_alone_rewrites_the_bytes() {
        assert!(converts(
            &values(&[("text", Value::Named("auto".into()))]),
            PLAIN
        ));
    }

    #[test]
    fn only_settings_known_to_leave_checkout_alone_are_plain() {
        assert_eq!(LineEndings::from_config(None, None), PLAIN);
        assert_eq!(LineEndings::from_config(Some("false"), Some("lf")), PLAIN);
        assert_eq!(LineEndings::from_config(None, Some("LF")), PLAIN);
        assert_eq!(LineEndings::from_config(None, Some("Native")), PLAIN);
        for autocrlf in ["true", "input", "INPUT", "bogus", ""] {
            assert!(
                LineEndings::from_config(Some(autocrlf), None).autocrlf,
                "core.autocrlf={autocrlf:?}"
            );
        }
        for eol in ["crlf", "CRLF", "bogus", ""] {
            assert!(
                LineEndings::from_config(None, Some(eol)).crlf,
                "core.eol={eol:?}"
            );
        }
    }

    #[test]
    fn the_deprecated_crlf_attribute_marks_text() {
        assert!(converts(&values(&[("crlf", Value::Set)]), PLAIN));
        assert!(converts(
            &values(&[("crlf", Value::Named("input".into()))]),
            PLAIN
        ));
        assert!(!converts(&values(&[("crlf", Value::Unset)]), PLAIN));
        // An explicit `text` wins over the deprecated spelling.
        assert!(!converts(
            &values(&[("text", Value::Unset), ("crlf", Value::Set)]),
            PLAIN
        ));
    }

    #[test]
    fn a_binary_path_is_never_rewritten() {
        let binary = values(&[("text", Value::Unset)]);
        assert!(!converts(&binary, PLAIN));
        assert!(!converts(
            &binary,
            LineEndings {
                autocrlf: true,
                crlf: true
            }
        ));
    }

    #[test]
    fn autocrlf_rewrites_paths_with_no_attributes_of_their_own() {
        assert!(converts(
            &values(&[]),
            LineEndings {
                autocrlf: true,
                crlf: false
            }
        ));
    }

    #[test]
    fn a_filter_rewrites_the_bytes() {
        assert!(converts(
            &values(&[("filter", Value::Named("lfs".into()))]),
            PLAIN
        ));
        assert!(converts(&values(&[("ident", Value::Set)]), PLAIN));
        assert!(converts(
            &values(&[("working-tree-encoding", Value::Named("UTF-16".into()))]),
            PLAIN
        ));
    }

    fn blob(bytes: &[u8], kind: gix_hash::Kind) -> ObjectId {
        blob_id(bytes, kind).expect("hash")
    }

    #[test]
    fn hashes_a_blob_the_way_git_does() {
        // `git hash-object --stdin </dev/null`, in a SHA-1 and in a SHA-256 repository.
        assert_eq!(
            blob(b"", gix_hash::Kind::Sha1).to_string(),
            "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"
        );
        assert_eq!(
            blob(b"", gix_hash::Kind::Sha256).to_string(),
            "473a0f4c3be8a93681a267e3b1e9a7dcda1185436fe141f7749120a303721813"
        );
        // `printf '*.txt -text\n' | git hash-object --stdin`
        assert_eq!(
            blob(b"*.txt -text\n", gix_hash::Kind::Sha1).to_string(),
            "ae35d5c2a88fc564d5318d6b90beae827451ff99"
        );
    }

    #[test]
    fn line_endings_a_checkout_writes_change_no_rule() {
        for kind in [gix_hash::Kind::Sha1, gix_hash::Kind::Sha256] {
            let recorded = blob(b"*.txt text\n*.bin -text\n", kind);
            assert!(same_rules(b"*.txt text\n*.bin -text\n", &recorded));
            assert!(same_rules(b"*.txt text\r\n*.bin -text\r\n", &recorded));
            assert!(!same_rules(b"*.txt -text\n*.bin -text\n", &recorded));
            assert!(!same_rules(b"*.txt text\n*.bin -text\n\n", &recorded));
            assert!(!same_rules(b"", &recorded));
            // A checkout never takes CRLF out of a blob, so this copy is somebody's edit.
            let crlf = blob(b"*.txt text\r\n", kind);
            assert!(same_rules(b"*.txt text\r\n", &crlf));
            assert!(!same_rules(b"*.txt text\n", &crlf));
        }
    }

    /// Git reads the working copy with one reader and the blob with another, and these
    /// are the bytes the two make different rules of.
    #[test]
    fn bytes_git_reads_two_ways_are_not_the_same_rules() {
        let alike = |bytes: &[u8]| same_rules(bytes, &blob(bytes, gix_hash::Kind::Sha1));

        assert!(!alike(b"\xEF\xBB\xBF*.txt -text\n"));
        assert!(alike(b"# \xEF\xBB\xBF\n*.txt -text\n"));
        assert!(!alike(b"# a\0b\n*.txt -text\n"));

        let line = |length: usize| {
            let mut line = b"*.txt -text".to_vec();
            line.resize(length, b' ');
            line.extend_from_slice(b"\r\n");
            line
        };
        // With its CR this line is 2048 bytes, which the reader that keeps the CR ignores
        // and the one that drops it does not. A byte shorter both read it, and a byte
        // longer neither does.
        assert!(alike(&line(2046)));
        assert!(!alike(&line(2047)));
        assert!(alike(&line(2048)));
    }

    #[test]
    fn reads_the_check_attr_triples() {
        let output = b"a.txt\0text\0set\0a.txt\0eol\0crlf\0b.bin\0text\0unspecified\0".as_slice();
        let parsed = parse_check_attr(output);
        assert_eq!(parsed[b"a.txt".as_slice()]["text"], Value::Set);
        assert_eq!(
            parsed[b"a.txt".as_slice()]["eol"],
            Value::Named("crlf".into())
        );
        assert_eq!(parsed[b"b.bin".as_slice()]["text"], Value::Unspecified);
    }

    #[test]
    fn reads_line_ending_configuration() {
        assert!(LineEndings::from_config(Some("true"), None).autocrlf);
        assert!(LineEndings::from_config(Some("input"), None).autocrlf);
        assert!(!LineEndings::from_config(Some("false"), None).autocrlf);
        assert!(LineEndings::from_config(None, Some("crlf")).crlf);
        assert!(!LineEndings::from_config(None, Some("lf")).crlf);
    }
}
