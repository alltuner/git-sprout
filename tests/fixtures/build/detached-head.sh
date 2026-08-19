#!/bin/sh
# ABOUTME: A repository whose checkout is on a detached HEAD, so the source has no
# ABOUTME: current branch to reason about.
. "$(dirname "$0")/../lib.sh"

init_repo
base_history
write_text "src/one.txt" 6
commit "tree"
write_text "src/two.txt" 6
commit "second"
finish_repo
g checkout -q --detach HEAD~1
