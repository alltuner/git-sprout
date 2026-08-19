#!/bin/sh
# ABOUTME: A repository in non-cone sparse checkout, driven by patterns rather than
# ABOUTME: directory prefixes.
. "$(dirname "$0")/../lib.sh"

init_repo
base_history
write_text "keep/one.txt" 6
write_text "drop/two.txt" 6
commit "two subtrees"
gq sparse-checkout init --no-cone 2>/dev/null || skip "this git has no non-cone sparse checkout"
printf '/*\n!/drop/\n' > "$REPO/.git/info/sparse-checkout"
gq read-tree -mu HEAD
finish_repo
