#!/bin/sh
# ABOUTME: A repository normalised by `* text=auto eol=lf`, whose checkout writes the blob's
# ABOUTME: own bytes, plus three files whose working copies still hold the CRLF git normalised.
. "$(dirname "$0")/../lib.sh"

init_repo
# `legacy/` has no `eol` of its own, so core.eol decides its line ending, and an unset
# core.eol is `native`, which is CRLF on Windows. Pin it, so the fixture is the same
# repository on every platform.
g config core.eol lf
printf '* text=auto eol=lf\nlegacy/** !text !eol crlf\n' > "$REPO/.gitattributes"
base_history
write_text_tree "src" 40 20 4
write_binary "assets/blob.bin" 16
commit "lf tree"

# Written with CRLF and added: git stores LF, the working copy keeps CRLF, and the stat
# cache calls them clean because cleaning them gives the blob back. A checkout writes
# LF, so none may be cloned; each is longer than its blob, which is how the clone plan
# tells them apart. One sits at the root, where it is cloned on its own, one
# inside a directory that is otherwise clonable whole, and one is marked text only by
# the deprecated `crlf` attribute, which git still honours where `text` is unspecified.
printf 'first\r\nsecond\r\n' > "$REPO/crlf-top.txt"
printf 'third\r\nfourth\r\n' > "$REPO/src/group0/crlf.txt"
mkdir -p "$REPO/legacy"
printf 'fifth\r\nsixth\r\n' > "$REPO/legacy/old.txt"
commit "crlf working copies"
finish_repo

for path in crlf-top.txt src/group0/crlf.txt legacy/old.txt; do
    case "$(g ls-files --eol -- "$path")" in
        *w/crlf*) ;;
        *) echo "fixture precondition: $path should hold CRLF in the working tree" >&2; exit 1 ;;
    esac
done
if [ -n "$(g status --porcelain)" ]; then
    echo "fixture precondition: the source checkout should be clean" >&2
    exit 1
fi
