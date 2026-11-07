<p align="center">
  <img src="https://brand.alltuner.com/logos/sprout/horizontal.png" alt="git-sprout" width="500">
</p>

<p align="center">
  <strong>Stop paying for the same tree twice.</strong><br>
  A drop-in replacement for <code>git worktree add</code> that doesn't copy your tree.
</p>

<p align="center">
  <a href="https://sprout.alltuner.com">Website</a> &middot;
  <a href="https://alltuner.com/sponsor/">Sponsor</a>
</p>

<p align="center">
  <img src="https://img.shields.io/crates/v/git-sprout?color=5B2333" alt="crates.io">
  <img src="https://img.shields.io/github/license/alltuner/git-sprout?color=5B2333" alt="License">
  <img src="https://img.shields.io/github/stars/alltuner/git-sprout?color=5B2333" alt="Stars">
</p>

---

> [!NOTE]
> **Checked on every commit.** The contract below is compared against real
> `git worktree add` across 26 repository fixtures and 41 argument shapes, on macOS,
> Linux (btrfs, XFS, ext4) and Windows (NTFS, ReFS), plus the Linux kernel on two
> filesystems. The suite proves itself by detecting nineteen deliberately injected
> differences and finding none when git is compared against itself. The kernel figures
> are measured on the released binary; the smaller benchmark rows are still the
> research prototype's.

## Get Started

```bash
brew install alltuner/tap/git-sprout
```

Or `cargo install git-sprout`, or a binary from [the releases page](https://github.com/alltuner/git-sprout/releases).

```bash
git sprout add ../myrepo-feature -b feature     # or: git worktree-fast add
```

Two things have to be true. The filesystem needs block cloning: **APFS** on macOS; **btrfs,
XFS with reflinks, or bcachefs** on Linux; **a ReFS volume or a Windows 11 Dev Drive** on
Windows. Everywhere else, ext4 and NTFS included, it runs plain `git worktree add`.

Acceleration also requires Git 2.40 or later for immutable target-commit
attribute queries (`git check-attr --source`). Older Git versions retain normal
Git checkout behavior instead of using the clone path.

And the repository has to be one that does not convert files on checkout, because a file can
only be shared when checking it out would not rewrite its bytes. Git for Windows turns
`core.autocrlf` on by default, and with that setting there is nothing to share, so on a
typical Windows repository sprout passes straight through to `git worktree add`. The same
goes for `* text=auto eol=crlf` on any platform. Repositories with no conversion attributes
clone everything, which is the common case on macOS and Linux, the kernel included.

## What is git-sprout?

Every worktree you create is a second full copy of your repository on disk. `git-sprout`
materialises the new worktree with filesystem copy-on-write clones of a checkout you
already have, instead of inflating every blob out of the object store into fresh blocks.
The clone shares disk blocks with the source until something writes to them, so the second
worktree costs almost nothing until it diverges.

The pitch is disk, not speed. This is not a speed-up: on a small repository it saves under a
second, and on the kernel the two commands finish within half a second of each other. The disk
saving is close to total at every size, and cost there scales with file count rather than with
bytes.

## The numbers

The measurements below predate the concurrency safety checks. Each accepted clone
is now read once to verify its Git blob identity, and APFS uses per-file clones
instead of cloning entire directories. Copy-on-write block sharing is retained,
but the timings and metadata overhead below need to be remeasured for this version.

| workload | `git worktree add` | `git sprout add` |
| --- | --- | --- |
| **Linux kernel, <!--bench:kernel.files-->95 299<!--/bench--> files, <!--bench:kernel.bytes-->2.0 GB<!--/bench-->** | **<!--bench:kernel.disk.git-->1816 MB<!--/bench-->** | **<!--bench:kernel.disk.sprout-->36 MB<!--/bench-->** |
| 250 MB, 2000 files | <!--bench:medium.time.git-->0.85s<!--/bench--> · <!--bench:medium.disk.git-->251 MB<!--/bench--> | <!--bench:medium.time.sprout-->0.21s<!--/bench--> · <!--bench:medium.disk.sprout-->~0 MB<!--/bench--> |
| 188 MB, 3000 files, source 6 commits behind | <!--bench:cross.time.git-->0.83s<!--/bench--> · <!--bench:cross.disk.git-->187 MB<!--/bench--> | <!--bench:cross.time.sprout-->0.15s<!--/bench--> · <!--bench:cross.disk.sprout-->~1.5 MB<!--/bench--> |
| btrfs, 188 MB | <!--bench:btrfs.time.git-->0.33s<!--/bench--> · <!--bench:btrfs.disk.git-->187 MB<!--/bench--> | <!--bench:btrfs.time.sprout-->0.05s<!--/bench--> · <!--bench:btrfs.disk.sprout-->0.1 MB<!--/bench--> |
| ext4 (no block cloning) | <!--bench:ext4.time.git-->0.41s<!--/bench--> · <!--bench:ext4.disk.git-->187 MB<!--/bench--> | falls back, identical |

One worktree of the Linux kernel: <!--bench:kernel.disk.ratio-->50x<!--/bench--> less disk, and
no meaningful difference in wall clock. That is <!--bench:kernel.disk.saved-->1.78 GB<!--/bench--> that never gets allocated every
time anyone creates one. Ten engineers with five worktrees each is
<!--bench:fleet.disk.git-->90 GB<!--/bench--> of kernel checkouts on git, and about
<!--bench:fleet.disk.sprout-->2 GB<!--/bench--> on sprout.

The filesystem's own accounting, verbatim:

```
<!--bench:btrfs.du-->       Total   Exclusive  Set shared  Filename
   187.00MiB       0.00B   187.00MiB  repo/src        <- source
   187.00MiB       0.00B   187.00MiB  wt-sprout/src   <- git sprout add
   187.00MiB   187.00MiB       0.00B  wt-plain/src    <- git worktree add<!--/bench-->
```

Provisional figures. The kernel row is measured on the implementation itself, on a dedicated
APFS image; the smaller rows are still the research prototype's. Machine:
<!--bench:env.macos-->Apple M2, 8 cores, macOS 26.6.1, git 2.55.0<!--/bench-->; Linux
figures on <!--bench:env.linux-->kernel 7.0.12, git 2.47.3, loopback btrfs and XFS<!--/bench-->.
The harness in [`bench/`](bench/) re-measures them and rewrites this table.

### Measuring it yourself

One property will otherwise waste your afternoon. **A repository that arrives as a copy
accelerates nothing until its stat cache is rebuilt.** `git sprout add` clones a file
only when git already considers the source's copy unmodified, which it decides from the
inode, size and mtime recorded in the index. A `cp -r`, an rsync, a container image
layer, a restored CI cache or an unpacked tarball gives every file a new inode and a
fresh mtime, so every entry looks modified even though every byte is identical. The
plan comes back empty, the worktree is still correct, and nothing is shared.

Run this once in the source repository first, and the numbers appear:

```bash
git update-index --refresh
```

Nothing else about the tool depends on it — the worktree is correct either way, which is
exactly why it is easy to miss. Ask for the counts if you want to be certain a run
actually cloned:

```bash
SPROUT_STATS=1 git sprout add ../wt -b feature   # cloned=… on stderr
```

## Compatibility

`git sprout add` is meant to be indistinguishable from `git worktree add` in every
observable way except time, disk, and the two differences named below. That is the
contract, and the differential suite in [`tests/differential/`](tests/differential/) is
being written to prove it.

- Same flags, same stdout, same exit codes. Same stderr too, apart from the progress meter
  noted below.
- Same hooks, in the same order, with the same arguments.
- Same files, same modes, same index, compared byte for byte against real
  `git worktree add` across a matrix that includes `eol` conversion, `ident`, custom
  filters, LFS, submodules, sparse checkout, split index, SHA-256 repositories and
  case-insensitive filesystems, where the correct answer is the same set of
  already-modified paths git itself leaves behind rather than a clean worktree.
- Untracked and ignored files are not copied, exactly as git does not copy them.
- **Your repository's configuration is never modified.**
- On a filesystem without block cloning, or in a repository that converts files on checkout,
  it simply runs `git worktree add`.
- Any flag or combination it does not fully understand is not an error. It hands the whole
  command to git and exits with git's status.

Two differences you can observe, both deliberate. Files that were cloned keep the timestamp
they had in the checkout they came from, rather than the moment the worktree was created.
Nothing git does depends on it, but `make` and anything else that reads modification times
can see it.

And on a big repository `git worktree add` prints a progress meter while it writes the files
out. sprout has almost no files left to write, so git never starts one: you see less output
because less happened.

Beyond those two, and beyond time and disk, anything you can tell apart is a bug.

## Make it automatic

On Unix Bash/Zsh, the installed binary includes an opt-in PATH shim and agent
integration. Preview the files first, then apply:

```bash
git sprout install-shim --agents --claude-worktrees
git sprout install-shim --agents --claude-worktrees --apply
```

The shim routes supported `git [global options] worktree add` invocations through
Sprout and delegates other argv unchanged to the recorded real Git. `--agents` adds
managed startup activation and guidance for discovered Codex/Claude profiles.
The optional `--claude-worktrees` also integrates Claude's native worktree isolation;
its cleanup retains branches and refuses modified, untracked or ignored files.
Omit these options for a standalone shim and source the printed activation line.
Homebrew installation never activates the shim automatically.

Restart agents after installation, or explicitly launch an executable/account wrapper
with the inherited shim PATH:

```bash
git sprout run -- codex
git sprout run -- env CODEX_HOME="$HOME/.codex3" codex
git sprout run -- /path/to/claude-account-wrapper

git sprout uninstall-shim          # preview
git sprout uninstall-shim --apply  # remove managed integration, retain backups
```

Absolute Git paths, replacement PATH environments, shell-only account aliases, and
external tools that bypass Git require separate handling. No approval or sandbox
policy is changed. See [agent integration](integrations/agents/README.md) for profile
discovery, explicit profile paths, lifecycle hook conflicts, limitations and tests.

## Prior art

[`josharian/git-cow-worktree`](https://github.com/josharian/git-cow-worktree) got here
first, in Go. Two of its ideas are in sprout: choosing the source worktree by commit
distance, and hashing clones in parallel to write real stat data into the index instead
of leaning on `core.checkStat`. [`joeinnes/cow`](https://github.com/joeinnes/cow) takes
a looser approach to the same idea.

## Development

```bash
git clone https://github.com/alltuner/git-sprout.git
cd git-sprout
just            # menu of dev tasks
just build      # build the binaries
just test       # run the workspace tests
just check      # fmt + clippy
```

## License

[MIT](LICENSE)

## Support the project

git-sprout is an open source project built by [David Poblador i Garcia](https://davidpoblador.com/) through [All Tuner Labs](https://www.alltuner.com/).

If this project was useful to you, [consider supporting its development](https://alltuner.com/sponsor/).

---

<p align="center">
  Built by <a href="https://davidpoblador.com">David Poblador i Garcia</a> with the support of <a href="https://alltuner.com">All Tuner Labs</a>.<br>
  Made with ❤️ in Poblenou, Barcelona.
</p>
