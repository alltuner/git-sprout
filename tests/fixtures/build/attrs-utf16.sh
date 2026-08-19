#!/bin/sh
# ABOUTME: A repository with working-tree-encoding=UTF-16, where checkout re-encodes
# ABOUTME: every governed file.
. "$(dirname "$0")/../lib.sh"

init_repo
printf '*.txt working-tree-encoding=UTF-16 text\n' > "$REPO/.gitattributes"
base_history
write_utf16 "src/utf.txt" 12
write_utf16 "src/other.txt" 7
if ! commit "utf16 tree" 2>/dev/null; then
    skip "this git cannot re-encode working-tree-encoding=UTF-16"
fi
finish_repo
