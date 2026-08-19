#!/bin/sh
# ABOUTME: A repository whose checkout has modified, staged, deleted and untracked
# ABOUTME: paths, none of which may reach the new worktree.
. "$(dirname "$0")/../lib.sh"

init_repo
base_history
write_text "src/modified.txt" 8
write_text "src/staged.txt" 8
write_text "src/deleted.txt" 8
write_text "src/untouched.txt" 8
commit "tree"

printf 'modified in the working tree\n' >> "$REPO/src/modified.txt"
printf 'staged change\n' >> "$REPO/src/staged.txt"
g add src/staged.txt
rm -f "$REPO/src/deleted.txt"
printf 'untracked\n' > "$REPO/src/untracked.txt"
finish_repo
