# ABOUTME: Creates and mounts loopback btrfs / XFS / ext4 images so the Linux
# ABOUTME: filesystem scenarios can run in CI or a privileged container.

# loopfs_available <fs> -> 0 if the image can be built and mounted, 1 otherwise.
# On failure LOOPFS_REASON explains why, so the scenario can skip with a stated
# reason instead of failing.
loopfs_available() {
  local fs="$1"
  LOOPFS_REASON=""
  if [ "$(uname -s)" != Linux ]; then
    LOOPFS_REASON="$fs requires Linux; this run was on $(uname -s) $(uname -m)"
    return 1
  fi
  if [ "$(id -u)" != 0 ]; then
    LOOPFS_REASON="$fs needs root to mount a loopback image; this run was unprivileged"
    return 1
  fi
  local mkfs
  case "$fs" in
    btrfs) mkfs=mkfs.btrfs ;;
    xfs) mkfs=mkfs.xfs ;;
    ext4) mkfs=mkfs.ext4 ;;
    *) LOOPFS_REASON="unknown filesystem $fs"; return 1 ;;
  esac
  if ! PATH="$PATH:/sbin:/usr/sbin" command -v "$mkfs" >/dev/null; then
    LOOPFS_REASON="$mkfs is not installed"
    return 1
  fi
  return 0
}

# loopfs_mount <fs> <image-dir> <mountpoint>
loopfs_mount() {
  local fs="$1" dir="$2" mnt="$3" img="$2/$1.img"
  export PATH="$PATH:/sbin:/usr/sbin"
  mkdir -p "$dir" "$mnt"
  rm -f "$img"
  truncate -s "${LOOPFS_SIZE_MB:-4096}M" "$img"
  case "$fs" in
    btrfs) mkfs.btrfs -q -f "$img" >/dev/null ;;
    xfs) mkfs.xfs -q -m reflink=1 -f "$img" >/dev/null ;;
    ext4) mkfs.ext4 -q -F "$img" >/dev/null ;;
  esac
  mount -o loop "$img" "$mnt"
}

loopfs_umount() {
  local mnt="$1"
  umount "$mnt" 2>/dev/null || true
}

# The filesystem reporting its own sharing is a stronger artefact than a benchmark
# claiming it, so the btrfs scenario captures this verbatim.
loopfs_btrfs_du() {
  PATH="$PATH:/sbin:/usr/sbin" btrfs filesystem du -s "$@" 2>/dev/null
}
