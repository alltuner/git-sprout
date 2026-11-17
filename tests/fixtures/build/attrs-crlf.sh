#!/bin/sh
# ABOUTME: A repository marked by the deprecated `crlf` attribute, which git still reads as
# ABOUTME: `text`, holding a file whose working copy keeps the CRLF git add normalised away.
. "$(dirname "$0")/../lib.sh"

init_repo
printf '*.txt crlf\n*.bin -crlf\n' > "$REPO/.gitattributes"
base_history
write_text "src/a.txt" 6
write_binary "src/data.bin" 4
commit "legacy attributes"

# Written with CRLF and added: `crlf` means `text`, so git stores LF while the working
# copy keeps CRLF, and the stat cache calls it clean because cleaning it gives the blob
# back. A checkout writes LF, so it may not be cloned.
printf 'first\r\nsecond\r\n' > "$REPO/src/old.txt"
commit "crlf working copy"
finish_repo

case "$(g ls-files --eol -- src/old.txt)" in
    *i/lf*w/crlf*) ;;
    *) echo "fixture precondition: src/old.txt should be stored LF and hold CRLF" >&2; exit 1 ;;
esac
if [ -n "$(g status --porcelain)" ]; then
    echo "fixture precondition: the source checkout should be clean" >&2
    exit 1
fi
