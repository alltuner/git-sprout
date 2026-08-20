#!/usr/bin/env bash
# ABOUTME: Enforces the site's over-the-wire byte and request budget.
# ABOUTME: Also keeps the "this page is N KB" line on the page equal to the measured size.
set -euo pipefail

DOCS="$(cd "$(dirname "$0")" && pwd)"
PAGE="${1:-$DOCS/index.html}"
PAGE_NAME="$(basename "$PAGE")"

# Budget from the site spec: 60 KB over the wire including fonts, 3 requests.
MAX_BYTES=$((60 * 1024))
MAX_REQUESTS=3

WRITE=0
[ "${1:-}" = "--write" ] && WRITE=1

# The HTML is served compressed; the fonts are woff2 and already are. gzip is the
# conservative estimate — the edge negotiates brotli, which is smaller.
html_bytes=$(gzip -9c "$PAGE" | wc -c | tr -d ' ')
total=$html_bytes
requests=1

printf '%-28s %8s\n' "$PAGE_NAME (gzip)" "$html_bytes"
for font in "$DOCS"/fonts/*.woff2; do
    [ -e "$font" ] || continue
    size=$(wc -c <"$font" | tr -d ' ')
    printf '%-28s %8s\n' "$(basename "$font")" "$size"
    total=$((total + size))
    requests=$((requests + 1))
done

kb=$(( (total + 512) / 1024 ))
printf '%-28s %8s  (%s KB, %s requests)\n' "total" "$total" "$kb" "$requests"

fail=0
if [ "$total" -gt "$MAX_BYTES" ]; then
    echo "FAIL: $total bytes exceeds the $MAX_BYTES byte budget." >&2
    fail=1
fi
if [ "$requests" -gt "$MAX_REQUESTS" ]; then
    echo "FAIL: $requests requests exceeds the $MAX_REQUESTS request budget." >&2
    fail=1
fi

# The page states its own size. Keep the two in step, or the claim is worthless.
marker_value() {
    sed -n "s/.*<!--budget:$1-->\([^<]*\)<!--\/budget-->.*/\1/p" "$PAGE" | head -1
}
replace_marker() {
    python3 - "$PAGE" "$1" "$2" <<'PY'
import io, re, sys
path, key, value = sys.argv[1], sys.argv[2], sys.argv[3]
text = io.open(path, encoding="utf-8").read()
pattern = "(<!--budget:%s-->).*?(<!--/budget-->)" % re.escape(key)
text, n = re.subn(pattern, lambda m: m.group(1) + value + m.group(2), text, flags=re.S)
if n == 0:
    sys.exit("missing budget marker: %s" % key)
io.open(path, "w", encoding="utf-8").write(text)
PY
}

for key in site.size site.requests; do
    if ! grep -q "<!--budget:$key-->" "$PAGE"; then
        echo "FAIL: the page has no <!--budget:$key--> marker." >&2
        fail=1
    fi
done
[ "$fail" -eq 1 ] && exit 1

want_size="$kb KB"
want_requests="$requests"
have_size=$(marker_value site.size)
have_requests=$(marker_value site.requests)

if [ "$have_size" != "$want_size" ] || [ "$have_requests" != "$want_requests" ]; then
    if [ "$WRITE" -eq 1 ]; then
        replace_marker site.size "$want_size"
        replace_marker site.requests "$want_requests"
        echo "updated the page's own figures to $want_size / $want_requests requests."
    else
        echo "FAIL: the page says '$have_size and $have_requests requests'," >&2
        echo "      but it measures '$want_size and $want_requests requests'." >&2
        echo "      Run: ./docs/check-budget.sh --write" >&2
        exit 1
    fi
fi

echo "OK: $kb KB over $requests requests, within budget."
