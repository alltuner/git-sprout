#!/usr/bin/env bash
# ABOUTME: Publishes docs/ to the sprout.alltuner.com Garage bucket over S3.
# ABOUTME: Uploads through the tailnet-only endpoint; the public one corrupts small PUTs.
set -euo pipefail

DOCS="$(cd "$(dirname "$0")" && pwd)"
BUCKET="sprout.alltuner.com"

# depo.internal.alltuner.com only. Cloudflare's proxy breaks Expect: 100-continue,
# which S3 clients send on small simple PUTs, and those land as 0-byte objects.
# That is exactly this site's upload profile. See infrastructure/GARAGE.md.
ENDPOINT="${SITES_S3_ENDPOINT:-https://depo.internal.alltuner.com}"
case "$ENDPOINT" in
    *depo.internal.alltuner.com*) ;;
    *) echo "refusing to publish through $ENDPOINT; use the tailnet endpoint." >&2; exit 1 ;;
esac

command -v aws >/dev/null || { echo "aws cli not found." >&2; exit 1; }

# Credentials come from the environment in CI and from fnox on a workstation.
# Never printed, never written to a file.
# The keys live in the infrastructure repository's fnox.toml rather than the global
# one, and fnox resolves per directory, so they are invisible from here. Reading them
# from there keeps one copy: the same `sites-key` owns every other site bucket.
INFRA="${SITES_FNOX_DIR:-$HOME/repos/infrastructure}"
if [ -z "${AWS_ACCESS_KEY_ID:-}" ]; then
    command -v fnox >/dev/null || { echo "no AWS credentials in the environment and no fnox." >&2; exit 1; }
    if ! AWS_ACCESS_KEY_ID="$(cd "$INFRA" 2>/dev/null && fnox get SITES_S3_ACCESS_KEY 2>/dev/null)" ||
       [ -z "$AWS_ACCESS_KEY_ID" ]; then
        {
            echo "could not read SITES_S3_ACCESS_KEY."
            echo "It lives in the infrastructure repository's fnox.toml, and fnox resolves"
            echo "per directory, so it is not visible from this one. Either:"
            echo "  - point SITES_FNOX_DIR at that checkout (currently '$INFRA'), or"
            echo "  - export AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY yourself."
        } >&2
        exit 1
    fi
    AWS_SECRET_ACCESS_KEY="$(cd "$INFRA" && fnox get SITES_S3_SECRET_KEY)"
fi
export AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY
export AWS_DEFAULT_REGION="${AWS_DEFAULT_REGION:-garage}"

"$DOCS/check-budget.sh"

s3() { aws --endpoint-url "$ENDPOINT" s3 "$@"; }

# The page changes; it gets a short cache. The fonts are content-addressed by name
# and never change under an existing name; they get a year.
s3 sync "$DOCS/" "s3://$BUCKET/" --delete \
    --exclude '*.sh' --exclude 'fonts/*' \
    --cache-control 'public, max-age=300'

for font in "$DOCS"/fonts/*.woff2; do
    [ -e "$font" ] || continue
    s3 cp "$font" "s3://$BUCKET/fonts/$(basename "$font")" \
        --content-type 'font/woff2' \
        --cache-control 'public, max-age=31536000, immutable'
done

echo "published to https://$BUCKET/"
