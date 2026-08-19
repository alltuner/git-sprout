#!/bin/sh
# ABOUTME: The reference repository: a small tree with nested directories, a symlink,
# ABOUTME: an executable, a binary blob and two commits.
. "$(dirname "$0")/../lib.sh"

init_repo
base_history

write_text "src/a.txt" 4
write_text "src/nested/deep/b.txt" 6
write_binary "src/c.bin" 8
printf '#!/bin/sh\necho hi\n' > "$REPO/src/run.sh"
chmod +x "$REPO/src/run.sh"
if ln -s a.txt "$REPO/src/link.txt" 2>/dev/null; then
    :
else
    skip "this filesystem cannot create symlinks"
fi
printf 'build/\n*.log\n' > "$REPO/.gitignore"
commit "tree"

mkdir -p "$REPO/build"
printf 'ignored output\n' > "$REPO/build/artifact.o"
printf 'ignored\n' > "$REPO/debug.log"

finish_repo
