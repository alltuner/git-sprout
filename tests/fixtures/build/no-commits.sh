#!/bin/sh
# ABOUTME: A freshly initialised repository with no commits at all, where HEAD is
# ABOUTME: unborn and only --orphan can produce a worktree.
. "$(dirname "$0")/../lib.sh"

init_repo
finish_repo
