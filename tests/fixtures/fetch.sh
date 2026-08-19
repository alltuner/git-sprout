#!/usr/bin/env bash
# ABOUTME: Fetches the fixture repositories that are too large to build: the shallow
# ABOUTME: Linux kernel clone the case-collision parity test needs.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
CACHE="${SPROUT_FIXTURE_CACHE:-$HERE/cache}"
KERNEL="$CACHE/linux"

mkdir -p "$CACHE"

if [ -d "$KERNEL/.git" ]; then
    echo "kernel fixture already present at $KERNEL"
else
    echo "cloning the kernel into $KERNEL (about 2 GB, once)"
    git clone --depth 1 https://github.com/torvalds/linux "$KERNEL"
fi

# The fixture is the clone source, so its working tree has to be populated: with an
# empty index there is nothing to clone from and no implementation can accelerate.
if [ -z "$(git -C "$KERNEL" ls-files | head -1)" ]; then
    echo "populating the kernel working tree"
    git -C "$KERNEL" checkout
fi

tracked="$(git -C "$KERNEL" ls-files | wc -l | tr -d ' ')"
echo "kernel fixture ready: $tracked tracked files at $(git -C "$KERNEL" rev-parse --short HEAD)"
echo
echo "run it with:  SPROUT_KERNEL_REPO=$KERNEL cargo test --release --manifest-path tests/differential/Cargo.toml --test kernel -- --ignored --nocapture"
