// ABOUTME: The compatibility matrix: every fixture repository crossed with every
// ABOUTME: flag case, asserting nothing observable differs from git worktree add.

use differential::{case, fixtures};

macro_rules! fixture_tests {
    ($($id:ident => $name:literal),* $(,)?) => {
        $(
            #[test]
            fn $id() {
                case::check_fixture($name, &case::all_cases());
            }
        )*
    };
}

fixture_tests! {
    plain => "plain",
    attrs_eol_crlf => "attrs-eol-crlf",
    attrs_ident => "attrs-ident",
    attrs_utf16 => "attrs-utf16",
    attrs_filter => "attrs-filter",
    attrs_lfs => "attrs-lfs",
    attrs_diverging => "attrs-diverging",
    autocrlf => "autocrlf",
    symlinks_exec => "symlinks-exec",
    odd_paths => "odd-paths",
    case_collision => "case-collision",
    submodule => "submodule",
    nested_worktree => "nested-worktree",
    sparse_cone => "sparse-cone",
    sparse_nocone => "sparse-nocone",
    split_index => "split-index",
    index_v4 => "index-v4",
    many_files => "many-files",
    untracked_cache => "untracked-cache",
    fsmonitor => "fsmonitor",
    dirty_source => "dirty-source",
    mid_rebase => "mid-rebase",
    detached_head => "detached-head",
    sha256 => "sha256",
    no_commits => "no-commits",
    text_heavy => "text-heavy",
}

/// The fixture list in the harness and the builders on disk have to stay in step, or
/// a fixture can be added and never run, or removed and silently stop being covered.
#[test]
fn fixture_scripts_match_the_list() {
    let mut declared: Vec<String> = fixtures::ALL.iter().map(|s| s.to_string()).collect();
    declared.sort();
    let found = fixtures::discover().expect("tests/fixtures/build is readable");
    assert_eq!(
        declared, found,
        "the fixture list in fixtures::ALL does not match tests/fixtures/build/"
    );
}

/// Every fixture must be reachable by the per-fixture tests above. The count is the
/// cheap invariant that catches a fixture added to the list but never given a test.
#[test]
fn every_fixture_has_a_test() {
    let source = include_str!("differential.rs");
    for name in fixtures::ALL {
        let needle = format!("=> {name:?}");
        assert!(source.contains(&needle), "fixture {name} has no test in the matrix");
    }
}
