#!/bin/sh
# ABOUTME: A repository with a clean/smudge filter, so the checked-out bytes differ
# ABOUTME: from the stored blob for every governed path.
. "$(dirname "$0")/../lib.sh"

init_repo
printf '*.dat filter=marker\n' > "$REPO/.gitattributes"
g config filter.marker.clean "sed s/SMUDGED/BLOB/"
g config filter.marker.smudge "sed s/BLOB/SMUDGED/"
base_history
mkdir -p "$REPO/src"
printf 'value SMUDGED here\nsecond line\n' > "$REPO/src/first.dat"
printf 'another SMUDGED value\n' > "$REPO/src/second.dat"
commit "filtered tree"
finish_repo
