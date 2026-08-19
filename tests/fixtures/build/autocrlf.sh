#!/bin/sh
# ABOUTME: A text-heavy repository with core.autocrlf=true, the Git for Windows
# ABOUTME: default, where every text file's working-tree bytes differ from its blob.
. "$(dirname "$0")/../lib.sh"

init_repo
g config core.autocrlf true
base_history
write_text_tree "src" 60 25 60
commit "text heavy tree"
finish_repo
