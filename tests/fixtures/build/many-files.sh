#!/bin/sh
# ABOUTME: A repository with feature.manyFiles=true, which changes the index version
# ABOUTME: and turns on the untracked cache.
. "$(dirname "$0")/../lib.sh"

init_repo
g config feature.manyFiles true
base_history
write_text_tree "src" 80 4 8
commit "many files"
finish_repo
