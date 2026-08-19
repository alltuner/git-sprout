#!/bin/sh
# ABOUTME: Paths with spaces, unicode, emoji and a leading dot, plus an untracked
# ABOUTME: empty directory that must not appear in the new worktree.
. "$(dirname "$0")/../lib.sh"

init_repo
base_history
mkdir -p "$REPO/a dir with spaces" "$REPO/ünïcode/日本語"
write_text "a dir with spaces/a file.txt" 4
write_text "ünïcode/日本語/ファイル.txt" 4
write_text "ünïcode/emoji-🌱-name.txt" 3
write_text ".hidden/config.txt" 2
printf 'quote"and\\backslash\n' > "$REPO/weird\"name.txt" 2>/dev/null || true
commit "odd paths"
mkdir -p "$REPO/empty-untracked-dir"
finish_repo
