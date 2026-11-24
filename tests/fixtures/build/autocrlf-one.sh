#!/bin/sh
# ABOUTME: core.autocrlf=1, an integer spelling of true, over working copies
# ABOUTME: that hold LF, where a checkout writes CRLF.
. "$(dirname "$0")/../lib.sh"

init_repo
spelled_option core.autocrlf 1
