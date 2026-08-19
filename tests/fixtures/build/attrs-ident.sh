#!/bin/sh
# ABOUTME: A repository using the ident attribute, where checkout expands $Id$ and
# ABOUTME: the working-tree file is longer than its blob.
. "$(dirname "$0")/../lib.sh"

init_repo
printf '*.c ident\n' > "$REPO/.gitattributes"
base_history
mkdir -p "$REPO/src"
printf 'const char *id = "$Id$";\nint main(void) { return 0; }\n' > "$REPO/src/main.c"
printf 'static const char *v = "$Id$";\n' > "$REPO/src/version.c"
commit "ident tree"
finish_repo
