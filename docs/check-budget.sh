#!/usr/bin/env bash
# ABOUTME: Enforces the site's over-the-wire byte and request budget.
# ABOUTME: Also keeps each page's "this page is N KB" line equal to the measured size.
set -euo pipefail

DOCS="$(cd "$(dirname "$0")" && pwd)"

# Budget from the site spec: 60 KB over the wire, counting the HTML, the fonts and
# anything the page fetches from another host.
MAX_BYTES=$((60 * 1024))
MAX_REQUESTS=4

WRITE=0
PAGES=()
for arg in "$@"; do
    case "$arg" in
        --write) WRITE=1 ;;
        *) PAGES+=("$arg") ;;
    esac
done
[ "${#PAGES[@]}" -eq 0 ] && PAGES=("$DOCS/index.html" "$DOCS/details.html" "$DOCS/404.html")

# Subresources on another host, which the browser must fetch to finish the page.
#
# Deliberately not every absolute URL. An <a href> costs the visitor nothing, and
# neither does a <link rel=canonical>: it names the page, it is not a fetch. Counting
# one would make the page's figure wrong in the other direction, and the first version
# of this did exactly that — it dutifully downloaded the page's own canonical URL and
# billed the page for a second copy of itself.
external_subresources() {
    python3 - "$1" <<'PY'
import io, re, sys

FETCHING_RELS = {"stylesheet", "preload", "prefetch", "icon", "apple-touch-icon",
                 "shortcut icon", "manifest"}

text = io.open(sys.argv[1], encoding="utf-8").read()
urls = re.findall(r'<(?:script|img|iframe)\b[^>]*\bsrc="(https?://[^"]+)"', text)
for tag in re.findall(r'<link\b[^>]*>', text):
    href = re.search(r'\bhref="(https?://[^"]+)"', tag)
    rel = re.search(r'\brel="([^"]+)"', tag)
    if href and rel and rel.group(1).strip().lower() in FETCHING_RELS:
        urls.append(href.group(1))

seen = set()
for url in urls:
    if url not in seen:
        seen.add(url)
        print(url)
PY
}

marker_value() {
    sed -n "s/.*<!--budget:$2-->\([^<]*\)<!--\/budget-->.*/\1/p" "$1" | head -1
}

replace_marker() {
    python3 - "$1" "$2" "$3" <<'PY'
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

check_page() {
    local page="$1"
    local name total requests kb fail key
    name="$(basename "$page")"

    # The HTML is served compressed; the fonts are woff2 and already are. gzip is the
    # conservative estimate — the edge negotiates brotli, which is smaller. Measured
    # against the deployed page they agree to within a rounding: 12,970 gzip against
    # 13,130 brotli, both 13 KB. Using gzip keeps this runnable without a brotli binary,
    # and the budget is stated in whole kilobytes, so the substitution cannot change the
    # answer.
    total=$(gzip -9c "$page" | wc -c | tr -d ' ')
    requests=1
    printf '%-28s %8s\n' "$name (gzip)" "$total"

    for font in "$DOCS"/fonts/*.woff2; do
        [ -e "$font" ] || continue
        local size
        size=$(wc -c <"$font" | tr -d ' ')
        printf '%-28s %8s\n' "$(basename "$font")" "$size"
        total=$((total + size))
        requests=$((requests + 1))
    done

    fail=0

    # Anything the page pulls from another host is a request the visitor pays for, so it
    # counts here too. Measured over the wire rather than assumed: a figure the page
    # states about itself is only worth having if nothing can quietly fall out of it.
    local url size
    for url in $(external_subresources "$page"); do
        if ! size=$(curl -fsS --compressed -o /dev/null -w '%{size_download}' "$url" 2>/dev/null); then
            echo "FAIL: $name loads $url, which could not be measured." >&2
            fail=1
            continue
        fi
        printf '%-28s %8s\n' "$(basename "$url")" "$size"
        total=$((total + size))
        requests=$((requests + 1))
    done

    kb=$(( (total + 512) / 1024 ))
    printf '%-28s %8s  (%s KB, %s requests)\n' "total" "$total" "$kb" "$requests"

    if [ "$total" -gt "$MAX_BYTES" ]; then
        echo "FAIL: $name is $total bytes, over the $MAX_BYTES byte budget." >&2
        fail=1
    fi
    if [ "$requests" -gt "$MAX_REQUESTS" ]; then
        echo "FAIL: $name needs $requests requests, over the $MAX_REQUESTS request budget." >&2
        fail=1
    fi

    # The page states its own size. Keep the two in step, or the claim is worthless.
    for key in site.size site.requests; do
        if ! grep -q "<!--budget:$key-->" "$page"; then
            echo "FAIL: $name has no <!--budget:$key--> marker." >&2
            fail=1
        fi
    done
    [ "$fail" -eq 1 ] && return 1

    local want_size have_size have_requests
    want_size="$kb KB"
    have_size=$(marker_value "$page" site.size)
    have_requests=$(marker_value "$page" site.requests)

    if [ "$have_size" != "$want_size" ] || [ "$have_requests" != "$requests" ]; then
        if [ "$WRITE" -eq 1 ]; then
            replace_marker "$page" site.size "$want_size"
            replace_marker "$page" site.requests "$requests"
            echo "updated $name's own figures to $want_size / $requests requests."
        else
            echo "FAIL: $name says '$have_size and $have_requests requests'," >&2
            echo "      but it measures '$want_size and $requests requests'." >&2
            echo "      Run: ./docs/check-budget.sh --write" >&2
            return 1
        fi
    fi

    echo "OK: $name is $kb KB over $requests requests, within budget."
}

status=0
for page in "${PAGES[@]}"; do
    check_page "$page" || status=1
done
exit "$status"
