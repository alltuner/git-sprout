// ABOUTME: Parses the `git worktree add` command line and git's own global options.
// ABOUTME: Anything it does not fully understand becomes a delegation, never an error.

use std::ffi::{OsStr, OsString};

/// Git's global options that take a separate value argument.
const GLOBAL_WITH_VALUE: &[&str] = &["-C", "-c", "--git-dir", "--work-tree", "--namespace"];

/// Git's global options that stand alone.
const GLOBAL_STANDALONE: &[&str] = &[
    "-p",
    "--paginate",
    "-P",
    "--no-pager",
    "--bare",
    "--no-replace-objects",
    "--literal-pathspecs",
    "--glob-pathspecs",
    "--noglob-pathspecs",
    "--icase-pathspecs",
    "--no-optional-locks",
];

/// `git worktree add` long options that stand alone.
const ADD_STANDALONE: &[&str] = &[
    "--force",
    "--no-force",
    "--detach",
    "--no-detach",
    "--checkout",
    "--no-checkout",
    "--orphan",
    "--no-orphan",
    "--lock",
    "--no-lock",
    "--no-reason",
    "--quiet",
    "--no-quiet",
    "--track",
    "--no-track",
    "--guess-remote",
    "--no-guess-remote",
    "--relative-paths",
    "--no-relative-paths",
];

/// `git worktree add` long options that take a value.
const ADD_WITH_VALUE: &[&str] = &["--reason"];

/// A `git worktree add` invocation parsed well enough to consider accelerating.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddCommand {
    /// Git's global options, in order, to place before the `worktree` subcommand.
    pub globals: Vec<OsString>,
    /// The arguments for `git worktree add`, with sprout's own flags removed.
    pub passthrough: Vec<OsString>,
    /// Position of `--` within `passthrough`, if the user wrote one.
    pub double_dash: Option<usize>,
    /// The destination worktree path.
    pub path: OsString,
    /// The commit-ish to check out, when the user named one.
    pub commit_ish: Option<OsString>,
    pub quiet: bool,
    /// False when `--no-checkout` was given.
    pub checkout: bool,
    pub orphan: bool,
    /// True when the user asked for the plain `git worktree add` path.
    pub no_cow: bool,
}

impl AddCommand {
    /// The argument list that reproduces this request through `git` itself.
    pub fn git_args(&self) -> Vec<OsString> {
        let mut args = self.globals.clone();
        args.push(OsString::from("worktree"));
        args.push(OsString::from("add"));
        args.extend(self.passthrough.iter().cloned());
        args
    }

    /// The subcommand arguments for step 2, which creates the worktree without files.
    /// The globals are left out; whoever runs git puts them in front.
    pub fn worktree_add_args_no_checkout(&self) -> Vec<OsString> {
        let mut args = vec![OsString::from("worktree"), OsString::from("add")];
        let insert_at = self.double_dash.unwrap_or(self.passthrough.len());
        args.extend(self.passthrough[..insert_at].iter().cloned());
        args.push(OsString::from("--no-checkout"));
        args.extend(self.passthrough[insert_at..].iter().cloned());
        args
    }
}

/// What the command line asks the tool to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invocation {
    /// Understood; acceleration may be attempted.
    Add(Box<AddCommand>),
    /// Not understood. Run `git` with these arguments and exit with its status.
    Delegate {
        git_args: Vec<OsString>,
        reason: &'static str,
    },
    /// Report the tool's own version.
    Version,
}

fn delegate(args: &[OsString], reason: &'static str) -> Invocation {
    let mut git_args = vec![OsString::from("worktree")];
    git_args.extend(args.iter().cloned());
    Invocation::Delegate { git_args, reason }
}

/// Splits `--name=value` into its two halves.
fn split_assignment(token: &str) -> Option<(&str, &str)> {
    token
        .strip_prefix("--")
        .and_then(|rest| rest.split_once('='))
        .map(|(name, value)| (&token[..name.len() + 2], value))
}

/// Consumes git's global options from the front of `args`, returning them and the rest.
fn take_globals(args: &[OsString]) -> Option<(Vec<OsString>, &[OsString])> {
    let mut globals = Vec::new();
    let mut rest = args;
    while let Some(first) = rest.first() {
        let Some(token) = first.to_str() else { break };
        if !token.starts_with('-') {
            break;
        }
        if GLOBAL_STANDALONE.contains(&token) {
            globals.push(first.clone());
            rest = &rest[1..];
        } else if GLOBAL_WITH_VALUE.contains(&token) {
            let value = rest.get(1)?;
            globals.push(first.clone());
            globals.push(value.clone());
            rest = &rest[2..];
        } else if split_assignment(token).is_some_and(|(name, _)| GLOBAL_WITH_VALUE.contains(&name))
        {
            globals.push(first.clone());
            rest = &rest[1..];
        } else {
            return None;
        }
    }
    Some((globals, rest))
}

/// Parses the argument tail the binary was invoked with.
pub fn parse(args: &[OsString]) -> Invocation {
    if args.len() == 1 && (args[0] == "--version" || args[0] == "-V") {
        return Invocation::Version;
    }

    let Some((globals, rest)) = take_globals(args) else {
        return delegate(args, "unrecognised git global option");
    };

    match rest.first().and_then(|arg| arg.to_str()) {
        Some("add") => {}
        _ => return delegate(args, "not a `worktree add` invocation"),
    }

    match parse_add(globals, &rest[1..]) {
        Ok(add) => Invocation::Add(Box::new(add)),
        Err(reason) => delegate(args, reason),
    }
}

/// The state built up while walking the `add` arguments.
struct AddParse {
    passthrough: Vec<OsString>,
    double_dash: Option<usize>,
    positionals: Vec<OsString>,
    quiet: bool,
    checkout: bool,
    orphan: bool,
    no_cow: bool,
}

fn parse_add(globals: Vec<OsString>, args: &[OsString]) -> Result<AddCommand, &'static str> {
    let mut state = AddParse {
        passthrough: Vec::new(),
        double_dash: None,
        positionals: Vec::new(),
        quiet: false,
        checkout: true,
        orphan: false,
        no_cow: false,
    };

    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        index += 1;

        if state.double_dash.is_some() {
            state.positionals.push(arg.clone());
            state.passthrough.push(arg.clone());
            continue;
        }

        let Some(token) = arg.to_str() else {
            state.positionals.push(arg.clone());
            state.passthrough.push(arg.clone());
            continue;
        };

        if token == "--" {
            state.double_dash = Some(state.passthrough.len());
            state.passthrough.push(arg.clone());
            continue;
        }

        if token == "--no-cow" {
            state.no_cow = true;
            continue;
        }

        if token.starts_with("--") {
            parse_long(&mut state, token, args, &mut index)?;
            continue;
        }

        if token.len() > 1 && token.starts_with('-') {
            parse_shorts(&mut state, arg, token, args, &mut index)?;
            continue;
        }

        state.positionals.push(arg.clone());
        state.passthrough.push(arg.clone());
    }

    if state.positionals.is_empty() {
        return Err("no worktree path given");
    }
    if state.positionals.len() > 2 {
        return Err("more positional arguments than `git worktree add` takes");
    }

    Ok(AddCommand {
        globals,
        passthrough: state.passthrough,
        double_dash: state.double_dash,
        path: state.positionals[0].clone(),
        commit_ish: state.positionals.get(1).cloned(),
        quiet: state.quiet,
        checkout: state.checkout,
        orphan: state.orphan,
        no_cow: state.no_cow,
    })
}

fn parse_long(
    state: &mut AddParse,
    token: &str,
    args: &[OsString],
    index: &mut usize,
) -> Result<(), &'static str> {
    if ADD_STANDALONE.contains(&token) {
        match token {
            "--quiet" => state.quiet = true,
            "--no-quiet" => state.quiet = false,
            "--no-checkout" => state.checkout = false,
            "--checkout" => state.checkout = true,
            "--orphan" => state.orphan = true,
            "--no-orphan" => state.orphan = false,
            _ => {}
        }
        state.passthrough.push(OsString::from(token));
        return Ok(());
    }

    if ADD_WITH_VALUE.contains(&token) {
        let value = args.get(*index).ok_or("long option is missing its value")?;
        *index += 1;
        state.passthrough.push(OsString::from(token));
        state.passthrough.push(value.clone());
        return Ok(());
    }

    if let Some((name, _)) = split_assignment(token) {
        if ADD_WITH_VALUE.contains(&name) {
            state.passthrough.push(OsString::from(token));
            return Ok(());
        }
    }

    Err("unrecognised `git worktree add` option")
}

fn parse_shorts(
    state: &mut AddParse,
    arg: &OsStr,
    token: &str,
    args: &[OsString],
    index: &mut usize,
) -> Result<(), &'static str> {
    for (offset, flag) in token[1..].char_indices() {
        match flag {
            'f' => {}
            'd' => {}
            'q' => state.quiet = true,
            'b' | 'B' => {
                let sticky = &token[1 + offset + flag.len_utf8()..];
                if sticky.is_empty() {
                    let value = args
                        .get(*index)
                        .ok_or("short option is missing its value")?;
                    *index += 1;
                    state.passthrough.push(arg.to_os_string());
                    state.passthrough.push(value.clone());
                } else {
                    state.passthrough.push(arg.to_os_string());
                }
                return Ok(());
            }
            _ => return Err("unrecognised `git worktree add` option"),
        }
    }
    state.passthrough.push(arg.to_os_string());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(tokens: &[&str]) -> Vec<OsString> {
        tokens.iter().map(OsString::from).collect()
    }

    fn strings(tokens: &[OsString]) -> Vec<String> {
        tokens
            .iter()
            .map(|token| token.to_string_lossy().into_owned())
            .collect()
    }

    fn add_of(tokens: &[&str]) -> AddCommand {
        match parse(&argv(tokens)) {
            Invocation::Add(add) => *add,
            other => panic!("expected an add invocation, got {other:?}"),
        }
    }

    fn delegation_reason(tokens: &[&str]) -> &'static str {
        match parse(&argv(tokens)) {
            Invocation::Delegate { reason, .. } => reason,
            other => panic!("expected a delegation, got {other:?}"),
        }
    }

    #[test]
    fn takes_the_path_and_commit_ish() {
        let add = add_of(&["add", "../wt", "v1.0"]);
        assert_eq!(add.path, OsString::from("../wt"));
        assert_eq!(add.commit_ish, Some(OsString::from("v1.0")));
        assert!(add.checkout);
        assert!(!add.quiet);
    }

    #[test]
    fn keeps_git_options_in_the_passthrough() {
        let add = add_of(&["add", "-b", "feature", "../wt"]);
        assert_eq!(strings(&add.passthrough), ["-b", "feature", "../wt"]);
        assert_eq!(
            strings(&add.git_args()),
            ["worktree", "add", "-b", "feature", "../wt"]
        );
    }

    #[test]
    fn removes_its_own_flag_from_the_passthrough() {
        let add = add_of(&["add", "--no-cow", "../wt"]);
        assert!(add.no_cow);
        assert_eq!(strings(&add.passthrough), ["../wt"]);
    }

    #[test]
    fn understands_bundled_and_sticky_short_options() {
        let add = add_of(&["add", "-fq", "-bfeature", "../wt"]);
        assert!(add.quiet);
        assert_eq!(strings(&add.passthrough), ["-fq", "-bfeature", "../wt"]);
    }

    #[test]
    fn understands_a_long_option_with_an_attached_value() {
        let add = add_of(&["add", "--lock", "--reason=busy", "../wt"]);
        assert_eq!(
            strings(&add.passthrough),
            ["--lock", "--reason=busy", "../wt"]
        );
    }

    #[test]
    fn understands_a_long_option_with_a_separate_value() {
        let add = add_of(&["add", "--lock", "--reason", "busy", "../wt"]);
        assert_eq!(
            strings(&add.passthrough),
            ["--lock", "--reason", "busy", "../wt"]
        );
    }

    #[test]
    fn records_no_checkout_and_orphan() {
        assert!(!add_of(&["add", "--no-checkout", "../wt"]).checkout);
        assert!(add_of(&["add", "--orphan", "../wt"]).orphan);
    }

    #[test]
    fn a_later_checkout_flag_wins() {
        assert!(add_of(&["add", "--no-checkout", "--checkout", "../wt"]).checkout);
        assert!(!add_of(&["add", "--checkout", "--no-checkout", "../wt"]).checkout);
    }

    #[test]
    fn inserts_no_checkout_before_a_double_dash() {
        let add = add_of(&["add", "--", "../wt"]);
        assert_eq!(
            strings(&add.worktree_add_args_no_checkout()),
            ["worktree", "add", "--no-checkout", "--", "../wt"]
        );
    }

    #[test]
    fn appends_no_checkout_when_there_is_no_double_dash() {
        let add = add_of(&["add", "-b", "feature", "../wt"]);
        assert_eq!(
            strings(&add.worktree_add_args_no_checkout()),
            ["worktree", "add", "-b", "feature", "../wt", "--no-checkout"]
        );
    }

    #[test]
    fn carries_git_global_options_ahead_of_the_subcommand() {
        let add = add_of(&["-C", "/repo", "-c", "core.bare=false", "add", "../wt"]);
        assert_eq!(
            strings(&add.git_args()),
            [
                "-C",
                "/repo",
                "-c",
                "core.bare=false",
                "worktree",
                "add",
                "../wt"
            ]
        );
    }

    #[test]
    fn an_unknown_option_is_a_delegation_not_an_error() {
        assert_eq!(
            delegation_reason(&["add", "--tomorrows-flag", "../wt"]),
            "unrecognised `git worktree add` option"
        );
        assert_eq!(
            delegation_reason(&["add", "-Z", "../wt"]),
            "unrecognised `git worktree add` option"
        );
        assert_eq!(
            delegation_reason(&["--tomorrows-global", "add", "../wt"]),
            "unrecognised git global option"
        );
    }

    #[test]
    fn an_abbreviated_option_is_a_delegation() {
        assert_eq!(
            delegation_reason(&["add", "--deta", "../wt"]),
            "unrecognised `git worktree add` option"
        );
    }

    #[test]
    fn a_delegation_reproduces_the_original_argv() {
        match parse(&argv(&["add", "--tomorrows-flag", "../wt"])) {
            Invocation::Delegate { git_args, .. } => assert_eq!(
                strings(&git_args),
                ["worktree", "add", "--tomorrows-flag", "../wt"]
            ),
            other => panic!("expected a delegation, got {other:?}"),
        }
    }

    #[test]
    fn other_subcommands_go_straight_to_git() {
        assert_eq!(
            delegation_reason(&["list"]),
            "not a `worktree add` invocation"
        );
    }

    #[test]
    fn a_missing_or_extra_path_is_a_delegation() {
        assert_eq!(delegation_reason(&["add"]), "no worktree path given");
        assert_eq!(
            delegation_reason(&["add", "a", "b", "c"]),
            "more positional arguments than `git worktree add` takes"
        );
    }

    #[test]
    fn reports_its_version() {
        assert_eq!(parse(&argv(&["--version"])), Invocation::Version);
    }
}
