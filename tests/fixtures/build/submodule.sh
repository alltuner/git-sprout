#!/bin/sh
# ABOUTME: A repository containing a submodule, whose gitlink must be handed to git
# ABOUTME: rather than materialised by anything else.
. "$(dirname "$0")/../lib.sh"

init_repo
base_history

INNER="$REPO/../submodule-source"
rm -rf "$INNER"
mkdir -p "$INNER"
git -C "$INNER" init -q
printf 'inner content\n' > "$INNER/inner.txt"
git -C "$INNER" add -A
git -C "$INNER" commit -q -m "inner base"

g -c protocol.file.allow=always submodule add -q ../submodule-source vendor/inner >/dev/null 2>&1 \
    || skip "this git refuses to add a local submodule"
commit "with submodule"
finish_repo
