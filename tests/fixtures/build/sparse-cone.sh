#!/bin/sh
# ABOUTME: A repository in cone-mode sparse checkout, whose sparse state the new
# ABOUTME: worktree does not inherit.
. "$(dirname "$0")/../lib.sh"

init_repo
base_history
write_text "included/one.txt" 6
write_text "excluded/two.txt" 6
write_text "excluded/deep/three.txt" 6
commit "two subtrees"
gq sparse-checkout init --cone 2>/dev/null || skip "this git has no cone-mode sparse checkout"
gq sparse-checkout set included
finish_repo
