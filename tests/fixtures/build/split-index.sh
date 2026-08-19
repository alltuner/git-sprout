#!/bin/sh
# ABOUTME: A repository using the split index, so the source index is a delta over a
# ABOUTME: shared index file rather than a self-contained one.
. "$(dirname "$0")/../lib.sh"

init_repo
base_history
write_text "src/one.txt" 10
write_text "src/two.txt" 10
commit "tree"
g config core.splitIndex true
gq update-index --split-index 2>/dev/null || skip "this git has no split index support"
write_text "src/three.txt" 10
commit "after split"
finish_repo
