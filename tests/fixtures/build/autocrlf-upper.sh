#!/bin/sh
# ABOUTME: core.autocrlf=TRUE, git reads booleans without regard to case, over
# ABOUTME: working copies that hold LF, where a checkout writes CRLF.
. "$(dirname "$0")/../lib.sh"

init_repo
spelled_option core.autocrlf TRUE
