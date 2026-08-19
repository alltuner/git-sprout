#!/bin/sh
# ABOUTME: A tree with enough files that a working acceleration is measurable and a
# ABOUTME: silently disabled one is visible in the statistics.
. "$(dirname "$0")/../lib.sh"

init_repo
base_history
write_text_tree "src" 120 20 12
write_binary "assets/blob.bin" 256
commit "many text files"
finish_repo
