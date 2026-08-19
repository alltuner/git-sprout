#!/bin/sh
# ABOUTME: A repository configured for index version 4, whose entries use path prefix
# ABOUTME: compression rather than fixed padding.
. "$(dirname "$0")/../lib.sh"

init_repo
g config index.version 4
base_history
write_text "src/deeply/nested/directory/one.txt" 6
write_text "src/deeply/nested/directory/two.txt" 6
write_text "src/deeply/nested/other/three.txt" 6
commit "v4 tree"
gq update-index --index-version 4 2>/dev/null || true
finish_repo
