![git-sprout](https://brand.alltuner.com/logos/sprout/horizontal.png)

**Stop paying for the same tree twice.** A drop-in replacement for `git worktree add`
that doesn't copy your tree.

[Website](https://sprout.alltuner.com) ·
[Repository](https://github.com/alltuner/git-sprout) ·
[Sponsor](https://alltuner.com/sponsor)

Every worktree you create is a second full copy of your repository on disk. `git-sprout`
materialises the new worktree with filesystem copy-on-write clones of a checkout you
already have, instead of inflating every blob out of the object store into fresh blocks.
The clone shares disk blocks with the source until something writes to them, so the
second worktree costs almost nothing until it diverges.

The pitch is disk, not speed. One worktree of the Linux kernel —
<!--bench:kernel.files-->95 299<!--/bench--> files,
<!--bench:kernel.bytes-->2.0 GB<!--/bench--> — takes
<!--bench:kernel.disk.git-->1816 MB<!--/bench--> through `git worktree add` and
<!--bench:kernel.disk.sprout-->36 MB<!--/bench--> through `git sprout add`, with no
meaningful difference in wall clock. That is
<!--bench:kernel.disk.saved-->1.78 GB<!--/bench--> that never gets allocated, every
time anyone creates one.

## Install

```bash
cargo install git-sprout
```

Or `brew install alltuner/tap/git-sprout`, or a binary from
[the releases page](https://github.com/alltuner/git-sprout/releases).

That puts `git-sprout` and `git-worktree-fast` on your `PATH`, where git picks them up
as subcommands:

```bash
git sprout add ../myrepo-feature -b feature     # or: git worktree-fast add
```

## What has to be true

The filesystem needs block cloning: **APFS** on macOS; **btrfs, XFS with reflinks, or
bcachefs** on Linux; **a ReFS volume or a Windows 11 Dev Drive** on Windows. Everywhere
else, ext4 and NTFS included, it runs plain `git worktree add`.

And the repository has to be one that does not convert files on checkout, because a file
can only be shared when checking it out would not rewrite its bytes. `core.autocrlf`,
which Git for Windows turns on by default, and `* text=auto eol=crlf` on any platform
both leave nothing to share. Repositories with no conversion attributes clone everything,
which is the common case on macOS and Linux, the kernel included.

## Measuring it yourself

One property will otherwise waste your afternoon. **A repository that arrives as a copy
accelerates nothing until its stat cache is rebuilt.** `git sprout add` clones a file only
when git already considers the source's copy unmodified, which it decides from the inode,
size and mtime recorded in the index. A `cp -r`, an rsync, a container image layer, a
restored CI cache or an unpacked tarball gives every file a new inode and a fresh mtime,
so every entry looks modified even though every byte is identical. The plan comes back
empty, the worktree is still correct, and nothing is shared.

Run this once in the source repository first, and the numbers appear:

```bash
git update-index --refresh
```

Ask for the counts if you want to be certain a run actually cloned:

```bash
SPROUT_STATS=1 git sprout add ../wt -b feature   # cloned=… on stderr
```

## Compatibility

`git sprout add` is meant to be indistinguishable from `git worktree add` in every
observable way except time and disk. Same flags, same stdout, same exit codes, same
hooks in the same order, same files and modes and index. Your repository's configuration
is never modified, and any flag it does not fully understand is handed to git rather than
refused.

That contract is compared against real `git worktree add` on every commit, across a
matrix of repository fixtures and argument shapes, on macOS, Linux (btrfs, XFS, ext4)
and Windows (NTFS, ReFS), plus the Linux kernel on two filesystems. The two deliberate
differences, and the reasoning behind the whole design, are written out in the
[repository README](https://github.com/alltuner/git-sprout#readme) and at
[sprout.alltuner.com](https://sprout.alltuner.com).

## License

[MIT](https://github.com/alltuner/git-sprout/blob/main/LICENSE).

Built by [David Poblador i Garcia](https://davidpoblador.com) with the support of
[All Tuner Labs](https://alltuner.com). If this was useful to you,
[consider supporting its development](https://alltuner.com/sponsor).
