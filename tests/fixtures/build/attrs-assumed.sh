#!/bin/sh
# ABOUTME: A tracked `.gitattributes` changed in the source and marked --assume-unchanged,
# ABOUTME: so its index entry no longer says what rules git reads from the working tree.
. "$(dirname "$0")/../lib.sh"

init_repo
mkdir -p "$REPO/src" "$REPO/assets"
printf '*.txt text\n' > "$REPO/src/.gitattributes"
printf '* -text\n' > "$REPO/assets/.gitattributes"
base_history
write_text_tree "src" 12 8 2
# LF text behind a tracked `-text`: binary on both sides, so it is still cloned.
write_text "assets/notes.bin" 5
commit "lf tree"
finish_repo

g config core.autocrlf true
# Git reads the working tree's file, which now marks src/ binary, while a checkout of the
# target reads the committed `text` and writes CRLF. The flag keeps the source clean and
# keeps `git diff-files` from reporting the change.
printf '*.txt -crlf\n' > "$REPO/src/.gitattributes"
g update-index --assume-unchanged -- src/.gitattributes
case "$(g ls-files --eol -- src/group0/file0.txt)" in
    *w/lf*attr/-text*) ;;
    *) echo "fixture precondition: src/ should hold LF and read as -text in the source" >&2; exit 1 ;;
esac
if [ -n "$(g status --porcelain)" ]; then
    echo "fixture precondition: the source checkout should be clean" >&2
    exit 1
fi
