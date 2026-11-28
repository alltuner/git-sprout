#!/bin/sh
# ABOUTME: A repository under `* text=auto eol=crlf`, where every text file's checkout
# ABOUTME: rewrites its line endings, so nothing in it may be cloned.
. "$(dirname "$0")/../lib.sh"

init_repo
printf '* text=auto eol=crlf\n' > "$REPO/.gitattributes"
base_history
write_text_tree "src" 30 20 3
commit "crlf text tree"
canonical_checkout
finish_repo
