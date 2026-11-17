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

/// Which of the words `git check-attr` prints for a state name a configured filter driver.
///
/// `filter` takes any string as the name of a driver, so `filter=unset` and
/// `filter=unspecified` filter the path where a driver by that name exists, and are no
/// filter at all where none does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FilterDrivers {
    pub unset: bool,
    pub unspecified: bool,
}

impl FilterDrivers {
    /// Reads the `filter.<driver>.<key>` names the configuration holds.
    pub fn from_config_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Self {
        let mut drivers = FilterDrivers::default();
        for name in names {
            match name
                .strip_prefix("filter.")
                .and_then(|rest| rest.rsplit_once('.'))
            {
                Some(("unset", _)) => drivers.unset = true,
                Some(("unspecified", _)) => drivers.unspecified = true,
                _ => {}
            }
        }
        drivers
    }

    fn names(self, word: &str) -> bool {
        match word {
            "unset" => self.unset,
            "unspecified" => self.unspecified,
            _ => false,
        }
    }
}

/// `git check-attr` prints two states as words a string value can spell too: `unset` is
/// `-name` or `name=unset`, and `unspecified` is no setting or `name=unspecified`, so its
/// output cannot tell them apart. This is the word, and the `attr` pathspec magic prefix
/// that matches only the state: `:(attr:-name)` and `:(attr:!name)`.
fn state_word(value: &Value) -> Option<(&'static str, char)> {
    match value {
        Value::Unset => Some(("unset", '-')),
        Value::Unspecified => Some(("unspecified", '!')),
        _ => None,
    }
}

/// What git makes of the string value for attribute `name` that `check-attr` spells as
/// `value`, where that differs from what it makes of the state.
///
/// `text` and its deprecated spelling `crlf` ignore a value they do not know and fall
/// through to the next source, `crlf` and then the configuration, as an unspecified
/// attribute does, which is not what `-text` means. `filter` takes the word as the name
/// of a driver, which matters only where one by that name is configured. Every other
/// reading is the state's own: `eol` and `ident` ignore a value they do not know, and a
/// `working-tree-encoding` git cannot convert to is reported and the bytes left alone.
fn string_reading(name: &str, value: &Value, drivers: FilterDrivers) -> Option<Value> {
    let (word, _) = state_word(value)?;
    match name {
        "text" | "crlf" if *value == Value::Unset => Some(Value::Unspecified),
        "filter" if drivers.names(word) => Some(Value::Named(word.to_string())),
        _ => None,
    }
}

/// The ambiguous readings on one path, each as the `attr` pathspec magic (`-text`,
/// `!filter`) that matches the path only where git holds the state.
fn ambiguous(
    values: &HashMap<String, Value>,
    drivers: FilterDrivers,
) -> impl Iterator<Item = (&String, String, Value)> {
    values.iter().filter_map(move |(name, value)| {
        let string = string_reading(name, value, drivers)?;
        let (_, magic) = state_word(value)?;
        Some((name, format!("{magic}{name}"), string))
    })
}

/// The path's attributes with every ambiguous state that `confirmed` does not vouch for
/// read as the string value, which is what git makes of it then. `confirmed` is asked
/// with the `attr` pathspec magic, `-text` or `!filter`, whether git matched the path.
pub fn resolve_ambiguous(
    values: &HashMap<String, Value>,
    drivers: FilterDrivers,
    confirmed: impl Fn(&str) -> bool,
) -> HashMap<String, Value> {
    let mut resolved = values.clone();
    for (name, magic, string) in ambiguous(values, drivers) {
        if !confirmed(&magic) {
            resolved.insert(name.clone(), string);
        }
    }
    resolved
}

/// The ambiguous states on this path that git has to confirm before `decide` may rely
/// on them, as the `attr` pathspec magic that matches each: all of them when reading
/// every one as its string value changes the answer, and none when the two readings
/// agree, in which case the answer stands either way.
pub fn ambiguous_to_confirm<T: PartialEq>(
    values: &HashMap<String, Value>,
    drivers: FilterDrivers,
    decide: impl Fn(&HashMap<String, Value>) -> T,
) -> Vec<String> {
    let magic: Vec<String> = ambiguous(values, drivers)
        .map(|(_, magic, _)| magic)
        .collect();
    if magic.is_empty() || decide(values) == decide(&resolve_ambiguous(values, drivers, |_| false))
    {
        return Vec::new();
    }
    magic
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
///
/// An `Unset` or `Unspecified` here is taken as the state; [`resolve_ambiguous`] is what
/// makes that so.
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

    const AUTOCRLF: LineEndings = LineEndings {
        autocrlf: true,
        crlf: false,
    };

    const NO_DRIVERS: FilterDrivers = FilterDrivers {
        unset: false,
        unspecified: false,
    };
    const BOTH_DRIVERS: FilterDrivers = FilterDrivers {
        unset: true,
        unspecified: true,
    };

    /// `git check-attr` prints `unset` both for `-crlf` and for `crlf=unset`. Git ignores
    /// the string and applies core.autocrlf, so until git confirms `-crlf` the path is
    /// read as having no `crlf` at all. The same holds for `text`.
    #[test]
    fn an_unconfirmed_unset_is_read_as_the_string_git_ignores() {
        for name in ["crlf", "text"] {
            let reported = values(&[(name, Value::Unset)]);
            let unconfirmed = resolve_ambiguous(&reported, NO_DRIVERS, |_| false);
            assert_eq!(unconfirmed[name], Value::Unspecified, "{name}");
            assert!(converts(&unconfirmed, AUTOCRLF), "{name}");
            let magic = format!("-{name}");
            let confirmed = resolve_ambiguous(&reported, NO_DRIVERS, |asked| asked == magic);
            assert_eq!(confirmed[name], Value::Unset, "{name}");
            assert!(!converts(&confirmed, AUTOCRLF), "{name}");
        }
    }

    /// `filter=unset` and `filter=unspecified` name a driver, which filters the path
    /// only where one by that name is configured.
    #[test]
    fn a_filter_word_names_a_driver_only_where_one_is_configured() {
        for (state, magic) in [(Value::Unset, "-filter"), (Value::Unspecified, "!filter")] {
            let reported = values(&[("filter", state.clone())]);
            assert_eq!(
                resolve_ambiguous(&reported, NO_DRIVERS, |_| false),
                reported
            );
            let unconfirmed = resolve_ambiguous(&reported, BOTH_DRIVERS, |_| false);
            assert!(converts(&unconfirmed, PLAIN), "{magic}");
            let confirmed = resolve_ambiguous(&reported, BOTH_DRIVERS, |asked| asked == magic);
            assert_eq!(confirmed, reported, "{magic}");
        }
    }

    #[test]
    fn other_attributes_read_the_word_as_the_state() {
        let reported = values(&[
            ("eol", Value::Unset),
            ("ident", Value::Unset),
            ("working-tree-encoding", Value::Unset),
            ("text", Value::Unspecified),
            ("crlf", Value::Unspecified),
        ]);
        assert_eq!(
            resolve_ambiguous(&reported, BOTH_DRIVERS, |_| false),
            reported
        );
    }

    #[test]
    fn reads_the_configured_filter_drivers() {
        assert_eq!(
            FilterDrivers::from_config_names(["filter.lfs.clean", "core.autocrlf"]),
            NO_DRIVERS
        );
        assert_eq!(
            FilterDrivers::from_config_names(["filter.unset.smudge"]),
            FilterDrivers {
                unset: true,
                unspecified: false
            }
        );
        assert_eq!(
            FilterDrivers::from_config_names(["filter.unspecified.process", "filter.unset.clean"]),
            BOTH_DRIVERS
        );
        // A driver name may itself hold dots; only the last one ends it.
        assert_eq!(
            FilterDrivers::from_config_names(["filter.my.unset.clean"]),
            NO_DRIVERS
        );
    }

    /// Asking git costs a pass over the index, so it is asked only where the answer
    /// depends on it.
    #[test]
    fn a_state_is_confirmed_only_where_the_readings_disagree() {
        let decide =
            |endings: LineEndings| move |values: &HashMap<String, Value>| converts(values, endings);
        let binary = values(&[("text", Value::Unset)]);
        assert!(ambiguous_to_confirm(&binary, NO_DRIVERS, decide(PLAIN)).is_empty());
        assert_eq!(
            ambiguous_to_confirm(&binary, NO_DRIVERS, decide(AUTOCRLF)),
            ["-text"]
        );
        let mut both = ambiguous_to_confirm(
            &values(&[("text", Value::Unset), ("crlf", Value::Unset)]),
            NO_DRIVERS,
            decide(AUTOCRLF),
        );
        both.sort();
        assert_eq!(both, ["-crlf", "-text"]);
        // A path another attribute converts either way needs nothing confirmed.
        let with_ident = values(&[("crlf", Value::Unset), ("ident", Value::Set)]);
        assert!(ambiguous_to_confirm(&with_ident, NO_DRIVERS, decide(PLAIN)).is_empty());
        // With a driver named `unspecified`, a path with no filter has to be confirmed.
        let plain = values(&[("filter", Value::Unspecified)]);
        assert!(ambiguous_to_confirm(&plain, NO_DRIVERS, decide(PLAIN)).is_empty());
        assert_eq!(
            ambiguous_to_confirm(&plain, BOTH_DRIVERS, decide(PLAIN)),
            ["!filter"]
        );
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
