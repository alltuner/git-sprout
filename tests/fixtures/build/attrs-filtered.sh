#!/bin/sh
# ABOUTME: A tracked `.gitattributes` run through a clean filter, whose working copy holds
# ABOUTME: rules its blob does not while the filter keeps the source checkout clean.
. "$(dirname "$0")/../lib.sh"

init_repo
mkdir -p "$REPO/src" "$REPO/assets"
g config filter.rules.clean "sed s/-crlf/text/g"
g config filter.rules.smudge cat
printf '*.txt text\n.gitattributes filter=rules\n' > "$REPO/src/.gitattributes"
printf '* -text\n' > "$REPO/assets/.gitattributes"
base_history
write_text_tree "src" 12 8 2
# LF text behind a tracked `-text`: binary on both sides, so it is still cloned.
write_text "assets/notes.bin" 5
commit "lf tree"
finish_repo

g config core.autocrlf true
# Git reads the working copy, which now marks src/ binary, while a checkout of the target
# reads the committed `text` and writes CRLF. Cleaning the working copy gives the blob
# back, so adding it again stages nothing and leaves the stat cache vouching for it.
printf '*.txt -crlf\n.gitattributes filter=rules\n' > "$REPO/src/.gitattributes"
g add -- src/.gitattributes
case "$(g ls-files --eol -- src/group0/file0.txt)" in
    *w/lf*attr/-text*) ;;
    *) echo "fixture precondition: src/ should hold LF and read as -text in the source" >&2; exit 1 ;;
esac
if [ -n "$(g status --porcelain)" ]; then
    echo "fixture precondition: the source checkout should be clean" >&2
    exit 1
fi
