// ABOUTME: Works out which paths a checkout would rewrite on the way to the working tree.
// ABOUTME: A path whose bytes get converted cannot be judged by the index stat cache alone.

use std::collections::HashMap;

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

/// Whether git's `native` line ending, which is also what an unset `core.eol` means, is
/// CRLF. It is in Git for Windows' build and LF everywhere else.
const NATIVE_EOL_IS_CRLF: bool = cfg!(windows);

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
    /// A text path with no `eol` attribute of its own may be checked out with CRLF,
    /// because `core.eol` says `crlf` or a value this does not know, or says `native` or
    /// nothing where native is CRLF.
    pub text_crlf: bool,
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
        let eol = eol.map(str::to_ascii_lowercase);
        let (crlf, text_crlf) = match eol.as_deref() {
            None | Some("native") => (false, NATIVE_EOL_IS_CRLF),
            Some("lf") => (false, false),
            // `crlf`, or a value this does not know, which is left to git.
            Some(_) => (true, true),
        };
        LineEndings {
            autocrlf: !matches!(autocrlf, None | Some("false")),
            crlf,
            text_crlf,
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
///
/// A text path checked out with LF is the one converting case that is not rewritten: git
/// only changes line endings on the way out when it writes CRLF, so a checkout writes the
/// blob's own bytes. That needs every one of: no `ident`, no `filter`, no
/// `working-tree-encoding`, an `eol` attribute that is unspecified, unset or `lf`, and a
/// configuration known not to ask for CRLF (`core.autocrlf` unset or false, and
/// `core.eol` unset, `lf`, or `native` where native is LF). `core.autocrlf=input` would
/// also check out verbatim, but it is left to git with the rest of `core.autocrlf`.
///
/// For such a path the stat cache still only says the file *cleans* back to the blob. A
/// file written with CRLF and added is stored as LF and is stat-clean while holding CRLF.
/// Cleaning one of these paths only ever deletes CRs, so a clean file the size of its blob
/// is the blob, and `drop_resized_paths`, which runs on every planned path after this,
/// is what settles it.
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
    let eol = values.get("eol").unwrap_or(&Value::Unspecified);
    if !eol.applies() && !text.applies() {
        return endings.autocrlf || endings.crlf;
    }
    if endings.autocrlf || endings.text_crlf {
        return true;
    }
    match eol {
        Value::Unspecified | Value::Unset => false,
        Value::Named(eol) => eol != "lf",
        // A bare `eol` names no line ending; git treats it as unset, but nothing here
        // depends on guessing that, so it goes to git.
        Value::Set => true,
    }
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
        text_crlf: false,
    };

    fn lf_eol() -> Value {
        Value::Named("lf".into())
    }

    #[test]
    fn a_path_with_no_attributes_is_written_verbatim() {
        assert!(!converts(&values(&[]), PLAIN));
    }

    #[test]
    fn a_text_path_checked_out_with_lf_is_written_verbatim() {
        let auto = Value::Named("auto".into());
        for attributes in [
            values(&[("text", auto.clone()), ("eol", lf_eol())]),
            values(&[("text", auto.clone())]),
            values(&[("text", Value::Set)]),
            values(&[("eol", lf_eol())]),
            values(&[("text", Value::Set), ("eol", Value::Unset)]),
        ] {
            assert!(!converts(&attributes, PLAIN), "{attributes:?}");
        }
    }

    #[test]
    fn a_text_path_checked_out_with_crlf_is_rewritten() {
        let auto = Value::Named("auto".into());
        let crlf = Value::Named("crlf".into());
        assert!(converts(
            &values(&[("text", auto.clone()), ("eol", crlf.clone())]),
            PLAIN
        ));
        assert!(converts(
            &values(&[("text", Value::Set), ("eol", crlf.clone())]),
            PLAIN
        ));
        assert!(converts(&values(&[("eol", crlf)]), PLAIN));
        // A bare `eol` names no line ending; git treats it as unset, but nothing here
        // depends on guessing that, so it goes to git.
        assert!(converts(
            &values(&[("text", Value::Set), ("eol", Value::Set)]),
            PLAIN
        ));
        let lf_attributes = values(&[("text", auto), ("eol", lf_eol())]);
        for config in [
            LineEndings::from_config(Some("true"), None),
            LineEndings::from_config(Some("input"), None),
            LineEndings::from_config(None, Some("crlf")),
        ] {
            assert!(converts(&lf_attributes, config), "{config:?}");
        }
    }

    #[test]
    fn every_other_conversion_is_rewritten_even_with_lf() {
        for name in ["ident", "filter", "working-tree-encoding"] {
            let value = if name == "ident" {
                Value::Set
            } else {
                Value::Named("x".into())
            };
            let attributes = values(&[
                ("text", Value::Named("auto".into())),
                ("eol", lf_eol()),
                (name, value),
            ]);
            assert!(converts(&attributes, PLAIN), "{name}");
        }
    }

    #[test]
    fn native_line_endings_follow_the_platform() {
        let unset = LineEndings::from_config(None, None);
        let native = LineEndings::from_config(None, Some("native"));
        assert_eq!(unset.text_crlf, cfg!(windows));
        assert_eq!(native.text_crlf, cfg!(windows));
        assert!(!LineEndings::from_config(Some("false"), Some("lf")).text_crlf);
        assert!(!LineEndings::from_config(None, Some("LF")).text_crlf);
        for eol in ["crlf", "CRLF", "bogus"] {
            assert!(
                LineEndings::from_config(None, Some(eol)).text_crlf,
                "core.eol={eol}"
            );
        }
        // Where native is CRLF, text with no `eol` of its own is checked out with CRLF,
        // while a path nothing marks as text is still written verbatim.
        let text = values(&[("text", Value::Named("auto".into()))]);
        assert_eq!(converts(&text, unset), cfg!(windows));
        assert!(!converts(&values(&[]), unset));
    }

    /// Git compares core.eol without regard to case, and a value it does not define is
    /// not guessed at: either way text is left to git rather than cloned.
    #[test]
    fn text_under_any_other_core_eol_is_rewritten() {
        let auto = values(&[("text", Value::Named("auto".into()))]);
        for eol in ["CRLF", "Crlf", "bogus"] {
            assert!(
                converts(&auto, LineEndings::from_config(None, Some(eol))),
                "core.eol={eol}"
            );
        }
        assert!(!converts(&auto, LineEndings::from_config(None, Some("LF"))));
    }

    #[test]
    fn only_settings_known_to_leave_checkout_alone_are_plain() {
        // Where native is CRLF, an unset or `native` core.eol still makes text CRLF; that
        // is `native_line_endings_follow_the_platform`'s business, not this test's.
        for (autocrlf, eol) in [
            (None, None),
            (Some("false"), Some("lf")),
            (None, Some("LF")),
            (None, Some("Native")),
        ] {
            let endings = LineEndings::from_config(autocrlf, eol);
            assert!(!endings.autocrlf && !endings.crlf, "{autocrlf:?} {eol:?}");
        }
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
        // Native CRLF, as on Windows: text is checked out with CRLF, while a path nothing
        // marks as text is written as it is.
        let native_crlf = LineEndings {
            autocrlf: false,
            crlf: false,
            text_crlf: true,
        };
        assert!(!converts(&values(&[]), native_crlf));
        assert!(converts(&values(&[("crlf", Value::Set)]), native_crlf));
        assert!(converts(
            &values(&[("crlf", Value::Named("input".into()))]),
            native_crlf
        ));
        assert!(!converts(&values(&[("crlf", Value::Unset)]), native_crlf));
        // An explicit `text` wins over the deprecated spelling.
        assert!(!converts(
            &values(&[("text", Value::Unset), ("crlf", Value::Set)]),
            native_crlf
        ));
        // Checked out with LF, text marked only by `crlf` is written verbatim like any
        // other, and so is `crlf` under `eol=lf`.
        assert!(!converts(&values(&[("crlf", Value::Set)]), PLAIN));
        assert!(!converts(
            &values(&[("crlf", Value::Set), ("eol", lf_eol())]),
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
                crlf: true,
                text_crlf: true,
            }
        ));
    }

    #[test]
    fn autocrlf_rewrites_paths_with_no_attributes_of_their_own() {
        assert!(converts(
            &values(&[]),
            LineEndings {
                autocrlf: true,
                crlf: false,
                text_crlf: false,
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
