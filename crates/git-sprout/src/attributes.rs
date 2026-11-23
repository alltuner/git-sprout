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

/// Makes `git ls-files -z` write `<path> NUL <eolattr> NUL` for every index entry.
pub const EOLATTR_FORMAT: &str = "--format=%(path)%x00%(eolattr)";

/// What git itself makes of a path's `text`, `crlf` and `eol` attributes taken together,
/// as `git ls-files` reports it in `<eolattr>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolved {
    /// `-text`: the bytes are never converted, whatever the configuration says.
    Binary,
    /// No attribute decides, so the configuration does.
    Unspecified,
    /// `text`, `text=auto` or an `eol`: line endings are converted.
    Text,
}

impl Resolved {
    fn parse(eolattr: &[u8]) -> Resolved {
        match eolattr {
            b"-text" => Resolved::Binary,
            b"" => Resolved::Unspecified,
            _ => Resolved::Text,
        }
    }
}

/// Reads the `<path> NUL <eolattr> NUL` pairs `git ls-files -z` writes for [`EOLATTR_FORMAT`].
pub fn parse_eolattr(output: &[u8]) -> impl Iterator<Item = (&[u8], Resolved)> {
    let mut fields = output.split(|byte| *byte == 0);
    std::iter::from_fn(move || {
        let path = fields.next().filter(|path| !path.is_empty())?;
        Some((path, Resolved::parse(fields.next()?)))
    })
}

/// Whether `git check-attr` printed `unset` for the path's `text` or `crlf`.
///
/// It prints that word for `-text`, and prints it just the same for the string value
/// `text=unset`. Git reads only the first as binary: a value of `text` or `crlf` it does
/// not know leaves the attribute unspecified, so the other attributes and
/// `core.autocrlf` decide. The word alone cannot say which of the two a path has.
pub fn unset_is_ambiguous(values: &HashMap<String, Value>) -> bool {
    ["text", "crlf"]
        .iter()
        .any(|name| values.get(*name) == Some(&Value::Unset))
}

/// Replaces what `git check-attr` said about `text` and `crlf` with git's own decision.
pub fn settle_unset(values: &mut HashMap<String, Value>, resolved: Resolved) {
    let text = match resolved {
        Resolved::Binary => Value::Unset,
        Resolved::Unspecified => Value::Unspecified,
        Resolved::Text => Value::Set,
    };
    values.insert("text".to_string(), text);
    values.insert("crlf".to_string(), Value::Unspecified);
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

    const AUTOCRLF: LineEndings = LineEndings {
        autocrlf: true,
        crlf: false,
    };

    #[test]
    fn reads_the_eolattr_pairs() {
        let output = b"a.txt\0text eol=crlf\0b.bin\0-text\0c\0\0d e\0text=auto\0".as_slice();
        let parsed: Vec<_> = parse_eolattr(output).collect();
        assert_eq!(
            parsed,
            [
                (b"a.txt".as_slice(), Resolved::Text),
                (b"b.bin".as_slice(), Resolved::Binary),
                (b"c".as_slice(), Resolved::Unspecified),
                (b"d e".as_slice(), Resolved::Text),
            ]
        );
        assert_eq!(parse_eolattr(b"").count(), 0);
    }

    #[test]
    fn only_an_unset_text_or_crlf_is_ambiguous() {
        assert!(unset_is_ambiguous(&values(&[("text", Value::Unset)])));
        assert!(unset_is_ambiguous(&values(&[("crlf", Value::Unset)])));
        assert!(!unset_is_ambiguous(&values(&[
            ("text", Value::Set),
            ("crlf", Value::Named("input".into())),
            ("eol", Value::Unset),
            ("filter", Value::Unset),
        ])));
    }

    /// `text=unset` and `crlf=unset` reach here as `Value::Unset`, like `-text`. Git's own
    /// decision for the path says which it was.
    #[test]
    fn an_unset_text_is_settled_by_what_git_resolved() {
        for name in ["text", "crlf"] {
            let reported = values(&[(name, Value::Unset)]);
            // Left as check-attr printed it, the path reads as binary.
            assert!(!converts(&reported, AUTOCRLF));

            let mut binary = reported.clone();
            settle_unset(&mut binary, Resolved::Binary);
            assert!(!converts(&binary, AUTOCRLF));

            let mut unspecified = reported.clone();
            settle_unset(&mut unspecified, Resolved::Unspecified);
            assert!(converts(&unspecified, AUTOCRLF));
            assert!(!converts(&unspecified, PLAIN));

            let mut text = reported.clone();
            settle_unset(&mut text, Resolved::Text);
            assert!(converts(&text, PLAIN));
        }
    }

    /// `text=unset` beside a genuine `-crlf`: git skips the string and reads `-crlf`.
    #[test]
    fn a_settled_path_no_longer_reads_its_crlf() {
        let mut both = values(&[("text", Value::Unset), ("crlf", Value::Set)]);
        settle_unset(&mut both, Resolved::Binary);
        assert!(!converts(&both, AUTOCRLF));
        let mut both = values(&[("text", Value::Unset), ("crlf", Value::Unset)]);
        settle_unset(&mut both, Resolved::Unspecified);
        assert!(converts(&both, AUTOCRLF));
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
