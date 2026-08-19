#!/bin/sh
# ABOUTME: A repository using the SHA-256 object format, whose index entries and
# ABOUTME: trailing checksum are 32 bytes rather than 20.
. "$(dirname "$0")/../lib.sh"

init_repo sha256 2>/dev/null || skip "this git cannot create sha256 repositories"
if [ "$(g rev-parse --show-object-format)" != "sha256" ]; then
    skip "this git ignored --object-format=sha256"
fi
base_history
write_text "src/one.txt" 6
write_binary "src/data.bin" 4
commit "sha256 tree"
finish_repo
