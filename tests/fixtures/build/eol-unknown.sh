#!/bin/sh
# ABOUTME: `* text=auto` with a core.eol value git does not define, over working
# ABOUTME: copies that hold LF. Git reads it as unset; the clone plan leaves it to git.
. "$(dirname "$0")/../lib.sh"

init_repo
printf '* text=auto\n' > "$REPO/.gitattributes"
spelled_option core.eol bogus
