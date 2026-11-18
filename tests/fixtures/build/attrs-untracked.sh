#!/bin/sh
# ABOUTME: An ignored `.gitattributes` that only the source's working tree holds, marking
# ABOUTME: LF text `-crlf` under core.autocrlf=true, where a checkout of the target writes CRLF.
. "$(dirname "$0")/../lib.sh"

init_repo
mkdir -p "$REPO/assets"
printf '* -text\n' > "$REPO/assets/.gitattributes"
base_history
write_text_tree "src" 12 8 2
# LF text behind a tracked `-text`: binary on both sides, so it is still cloned.
write_text "assets/notes.bin" 5
commit "lf tree"
finish_repo

g config core.autocrlf true
# Git reads a directory's `.gitattributes` from the working tree whether or not the index
# records it, so the source sees src/ as binary while a checkout of the target, which has
# no such file, writes CRLF there. Ignored, so the source checkout stays clean.
printf '*.txt -crlf\n' > "$REPO/src/.gitattributes"
printf '/src/.gitattributes\n' >> "$REPO/.git/info/exclude"
case "$(g ls-files --eol -- src/group0/file0.txt)" in
    *w/lf*attr/-text*) ;;
    *) echo "fixture precondition: src/ should hold LF and read as -text in the source" >&2; exit 1 ;;
esac
if [ -n "$(g status --porcelain)" ]; then
    echo "fixture precondition: the source checkout should be clean" >&2
    exit 1
fi
