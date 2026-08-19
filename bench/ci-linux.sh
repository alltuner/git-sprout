#!/usr/bin/env bash
# ABOUTME: Entry point for the Linux-only benchmark scenarios, which need root and
# ABOUTME: loopback btrfs / ext4 images; run it in CI or a privileged container.
set -euo pipefail

# Locally:
#   docker run --rm --privileged -v "$PWD:/src" -w /src debian:stable-slim \
#     bash bench/ci-linux.sh
# In CI, as a step on a Linux runner:
#   sudo -E bench/ci-linux.sh --out bench/results/linux.json
#
# Then fold the result into the macOS report:
#   ./bench/run.sh --merge bench/results/linux.json --promote

BENCH_DIR="$(cd "$(dirname "$0")" && pwd)"
OUT="$BENCH_DIR/results/linux.json"
ARGS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --out) OUT="$2"; shift ;;
    *) ARGS+=("$1") ;;
  esac
  shift
done

if [ "$(id -u)" != 0 ]; then
  command -v sudo >/dev/null || { echo "bench: needs root to mount loopback images" >&2; exit 1; }
  exec sudo -E "$0" --out "$OUT" ${ARGS[@]+"${ARGS[@]}"}
fi

export PATH="$PATH:/sbin:/usr/sbin"
MISSING=""
for tool in git mkfs.btrfs mkfs.ext4 python3; do
  command -v "$tool" >/dev/null || MISSING="$MISSING $tool"
done
if [ -n "$MISSING" ] && command -v apt-get >/dev/null; then
  echo "bench: installing$MISSING" >&2
  DEBIAN_FRONTEND=noninteractive apt-get -qq update >/dev/null
  DEBIAN_FRONTEND=noninteractive apt-get -qq install -y \
    git btrfs-progs e2fsprogs python3 >/dev/null
fi

exec "$BENCH_DIR/run.sh" --out "$OUT" ${ARGS[@]+"${ARGS[@]}"} btrfs ext4
