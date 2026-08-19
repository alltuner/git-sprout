#!/bin/sh
# ABOUTME: A repository whose large files are stored through Git LFS, the real-world
# ABOUTME: case where working-tree bytes and blob bytes are entirely unrelated.
. "$(dirname "$0")/../lib.sh"

command -v git-lfs >/dev/null 2>&1 || skip "git-lfs is not installed"

init_repo
base_history
g lfs install --local >/dev/null 2>&1 || skip "git lfs install failed"
g lfs track "*.bin" >/dev/null 2>&1 || skip "git lfs track failed"
write_binary "assets/big.bin" 64
write_binary "assets/small.bin" 8
commit "lfs tree"
finish_repo
