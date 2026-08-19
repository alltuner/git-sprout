#!/bin/sh
# ABOUTME: A repository whose text files are checked out with CRLF endings, so the
# ABOUTME: working-tree bytes are not the blob bytes.
. "$(dirname "$0")/../lib.sh"

init_repo
printf '*.txt text eol=crlf\n' > "$REPO/.gitattributes"
base_history
write_text "src/one.txt" 20
write_text "src/two.txt" 30
write_binary "src/data.bin" 4
commit "crlf tree"
canonical_checkout
finish_repo
