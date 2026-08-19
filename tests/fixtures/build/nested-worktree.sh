#!/bin/sh
# ABOUTME: A repository that already has a second worktree, so the add happens
# ABOUTME: alongside existing entries in .git/worktrees.
. "$(dirname "$0")/../lib.sh"

init_repo
base_history
write_text "src/file.txt" 8
commit "tree"
gq worktree add -q ../already-there -b already 2>/dev/null || skip "could not create the sibling worktree"
finish_repo
