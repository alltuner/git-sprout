#!/bin/sh
# ABOUTME: Symlinks (including one to a directory and one dangling) and executables,
# ABOUTME: the file types a checkout has to reproduce exactly.
. "$(dirname "$0")/../lib.sh"

init_repo
base_history
mkdir -p "$REPO/bin" "$REPO/lib/inner"
printf '#!/bin/sh\nexit 0\n' > "$REPO/bin/tool"
chmod +x "$REPO/bin/tool"
printf 'plain\n' > "$REPO/bin/notes.txt"
write_text "lib/inner/impl.txt" 5
# Each kind is guarded separately: Windows grants the privilege for file symlinks and
# directory symlinks independently, so the first can succeed and the second fail, and
# an unguarded failure here is a broken fixture rather than an unsupported platform.
ln -s notes.txt "$REPO/bin/notes-link.txt" 2>/dev/null ||
    skip "this filesystem cannot create file symlinks"
ln -s ../lib/inner "$REPO/bin/inner-link" 2>/dev/null ||
    skip "this filesystem cannot create directory symlinks"
ln -s nowhere-at-all "$REPO/bin/dangling-link" 2>/dev/null ||
    skip "this filesystem cannot create dangling symlinks"
commit "links and modes"
finish_repo
