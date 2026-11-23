#!/bin/sh
# ABOUTME: A relative core.attributesFile under core.autocrlf=true, naming an ignored file
# ABOUTME: only the source holds, so the source reads its text as binary and git does not.
. "$(dirname "$0")/../lib.sh"

init_repo
base_history
write_text_tree "src" 12 8 2
write_binary "assets/blob.bin" 4
commit "lf tree"
finish_repo
# Git resolves a relative core.attributesFile from each worktree's own root, so the
# same name selects this file in the source and nothing in a new worktree, which a
# checkout then writes with CRLF.
g config core.autocrlf true
g config core.attributesFile local.attributes
printf '* -text\n' > "$REPO/local.attributes"
printf 'local.attributes\n' >> "$REPO/.git/info/exclude"
case "$(g ls-files --eol -- src/group0/file0.txt)" in
    *w/lf*attr/-text*) ;;
    *) echo "fixture precondition: the source should read LF working copies as binary" >&2; exit 1 ;;
esac
if [ -n "$(g status --porcelain)" ]; then
    echo "fixture precondition: the source checkout should be clean" >&2
    exit 1
fi
