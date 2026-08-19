#!/bin/sh
# ABOUTME: A pair of tracked paths differing only by case, which a case-insensitive
# ABOUTME: volume cannot hold; git loses one and the replacement must lose the same.
. "$(dirname "$0")/../lib.sh"

init_repo
base_history
mkdir -p "$REPO/net"
printf 'upper contents\n' > "$REPO/net/XT_MARK.h"
commit "upper"
# Add the lower-case twin through the index so a case-insensitive checkout of the
# working tree cannot silently merge the two.
printf 'lower contents\n' > "$REPO/net/tmp-lower"
blob="$(g hash-object -w net/tmp-lower)"
rm -f "$REPO/net/tmp-lower"
g update-index --add --cacheinfo "100644,$blob,net/xt_mark.h"
gq commit -q -m "lower twin"
g read-tree -u --reset HEAD 2>/dev/null || true
finish_repo
