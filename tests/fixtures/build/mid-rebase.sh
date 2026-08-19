#!/bin/sh
# ABOUTME: A repository stopped in the middle of a conflicted rebase, so the source
# ABOUTME: index carries unmerged stages and rebase state exists.
. "$(dirname "$0")/../lib.sh"

init_repo
base_history
printf 'original\n' > "$REPO/conflict.txt"
commit "base conflict file"

g checkout -q -b sidebranch
printf 'side change\n' > "$REPO/conflict.txt"
commit "side"

g checkout -q main 2>/dev/null || g checkout -q master
printf 'main change\n' > "$REPO/conflict.txt"
commit "main"

g checkout -q sidebranch
g rebase main >/dev/null 2>&1 || true
if [ ! -d "$REPO/.git/rebase-merge" ] && [ ! -d "$REPO/.git/rebase-apply" ]; then
    skip "the rebase did not stop with a conflict"
fi
finish_repo
