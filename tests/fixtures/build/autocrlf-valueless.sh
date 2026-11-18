#!/bin/sh
# ABOUTME: core.autocrlf given with no value, which git reads as true, over
# ABOUTME: working copies that hold LF, where a checkout writes CRLF.
. "$(dirname "$0")/../lib.sh"

init_repo
spelled_option core.autocrlf
