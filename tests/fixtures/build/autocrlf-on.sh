#!/bin/sh
# ABOUTME: core.autocrlf=on, a boolean spelling of true, over working copies
# ABOUTME: that hold LF, where a checkout writes CRLF.
. "$(dirname "$0")/../lib.sh"

init_repo
spelled_option core.autocrlf on
