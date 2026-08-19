#!/bin/sh
# ABOUTME: A repository with the untracked cache enabled, which adds a UNTR extension
# ABOUTME: carrying directory stat data to the index.
. "$(dirname "$0")/../lib.sh"

init_repo
base_history
write_text "src/one.txt" 6
write_text "src/two.txt" 6
commit "tree"
g config core.untrackedCache true
gq update-index --untracked-cache 2>/dev/null || skip "this filesystem cannot support the untracked cache"
gq status --porcelain
finish_repo
