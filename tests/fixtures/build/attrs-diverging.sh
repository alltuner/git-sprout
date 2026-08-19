#!/bin/sh
# ABOUTME: A repository whose .gitattributes differ between the source commit and the
# ABOUTME: target commit, which must disable any reuse of the affected subtree.
. "$(dirname "$0")/../lib.sh"

init_repo
base_history
mkdir -p "$REPO/sub"
write_text "sub/page.txt" 15
write_text "top.txt" 10
printf 'page.txt text eol=lf\n' > "$REPO/sub/.gitattributes"
commit "lf in the subtree"

printf 'page.txt text eol=crlf\n' > "$REPO/sub/.gitattributes"
commit "crlf in the subtree"
canonical_checkout
finish_repo
