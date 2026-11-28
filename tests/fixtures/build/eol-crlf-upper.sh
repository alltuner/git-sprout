#!/bin/sh
# ABOUTME: `* text=auto` with core.eol=CRLF, which git reads without regard to case,
# ABOUTME: over working copies that hold LF, where a checkout writes CRLF.
. "$(dirname "$0")/../lib.sh"

init_repo
printf '* text=auto\n' > "$REPO/.gitattributes"
spelled_option core.eol CRLF
