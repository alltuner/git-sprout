#!/bin/sh
# ABOUTME: A repository with a filesystem monitor hook configured, which adds an FSMN
# ABOUTME: extension to the index.
. "$(dirname "$0")/../lib.sh"

init_repo
base_history
write_text "src/one.txt" 6
commit "tree"
mkdir -p "$REPO/.git/hooks"
# A monitor that always reports "everything may have changed" is a valid monitor and
# keeps the fixture independent of any daemon being installed.
printf '#!/bin/sh\nprintf "/\\0"\nexit 0\n' > "$REPO/.git/hooks/fsmonitor-watchman"
chmod +x "$REPO/.git/hooks/fsmonitor-watchman"
g config core.fsmonitor .git/hooks/fsmonitor-watchman
gq update-index --fsmonitor 2>/dev/null || skip "this git has no fsmonitor support"
gq status --porcelain
finish_repo
