<p align="center">
  <img src="https://brand.alltuner.com/logos/sprout/horizontal.png" alt="git-sprout" width="500">
</p>

<p align="center">
  <strong>Stop paying for the same tree twice.</strong><br>
  A drop-in replacement for <code>git worktree add</code> that doesn't copy your tree.
</p>

<p align="center">
  <a href="https://sprout.alltuner.com">Website</a> &middot;
  <a href="https://alltuner.com/sponsor">Sponsor</a>
</p>

<p align="center">
  <img src="https://img.shields.io/crates/v/git-sprout?color=5B2333" alt="crates.io">
  <img src="https://img.shields.io/github/license/alltuner/git-sprout?color=5B2333" alt="License">
  <img src="https://img.shields.io/github/stars/alltuner/git-sprout?color=5B2333" alt="Stars">
</p>

---

> [!NOTE]
> **Verified on macOS.** The compatibility contract below is checked against real
> `git worktree add` by a differential suite that passes 28 repository fixtures and the
> Linux kernel, and that proves itself by detecting nineteen injected differences. The
> Linux and Windows suites have not run yet, and the smaller benchmark rows are still
> the research prototype's rather than this implementation's.

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

| workload | `git worktree add` | `git sprout add` |
| --- | --- | --- |
| **Linux kernel, <!--bench:kernel.files-->95 056<!--/bench--> files, <!--bench:kernel.bytes-->2.0 GB<!--/bench-->** | **<!--bench:kernel.time.git-->11.06s<!--/bench--> · <!--bench:kernel.disk.git-->1814 MB<!--/bench-->** | **<!--bench:kernel.time.sprout-->11.51s<!--/bench--> · <!--bench:kernel.disk.sprout-->44 MB<!--/bench-->** |
| 250 MB, 2000 files | <!--bench:medium.time.git-->0.85s<!--/bench--> · <!--bench:medium.disk.git-->251 MB<!--/bench--> | <!--bench:medium.time.sprout-->0.21s<!--/bench--> · <!--bench:medium.disk.sprout-->~0 MB<!--/bench--> |
| 188 MB, 3000 files, source 6 commits behind | <!--bench:cross.time.git-->0.83s<!--/bench--> · <!--bench:cross.disk.git-->187 MB<!--/bench--> | <!--bench:cross.time.sprout-->0.15s<!--/bench--> · <!--bench:cross.disk.sprout-->~1.5 MB<!--/bench--> |
| btrfs, 188 MB | <!--bench:btrfs.time.git-->0.33s<!--/bench--> · <!--bench:btrfs.disk.git-->187 MB<!--/bench--> | <!--bench:btrfs.time.sprout-->0.05s<!--/bench--> · <!--bench:btrfs.disk.sprout-->0.1 MB<!--/bench--> |
| ext4 (no block cloning) | <!--bench:ext4.time.git-->0.41s<!--/bench--> · <!--bench:ext4.disk.git-->187 MB<!--/bench--> | falls back, identical |

One worktree of the Linux kernel: <!--bench:kernel.disk.ratio-->41x<!--/bench--> less disk, and
no meaningful difference in wall clock. That is <!--bench:kernel.disk.saved-->1.73 GB<!--/bench--> that never gets allocated every
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

There is no git setting that can redirect `git worktree add`: git ignores an alias that
shadows a builtin, and builtins never go through `GIT_EXEC_PATH` or `PATH`. Both were
tested. So there are two ways, and both are things you install deliberately.

**1. A shell function**, for what you type yourself. It only affects the interactive shell,
and it matches only `worktree add` as the first two words.

```bash
# bash / zsh — ~/.bashrc, ~/.zshrc
git() {
  if [ "${1:-}" = worktree ] && [ "${2:-}" = add ]; then
    shift 2
    command git sprout add "$@"
  else
    command git "$@"
  fi
}
```

```fish
# fish — ~/.config/fish/config.fish
function git
    if test (count $argv) -ge 2; and test "$argv[1]" = worktree; and test "$argv[2]" = add
        command git sprout add $argv[3..]
    else
        command git $argv
    end
end
```

```powershell
# PowerShell — $PROFILE
function git {
    $real = (Get-Command git -CommandType Application | Select-Object -First 1).Source
    if ($args.Count -ge 2 -and $args[0] -eq 'worktree' -and $args[1] -eq 'add') {
        $rest = @($args | Select-Object -Skip 2)
        & $real sprout add @rest
    } else {
        & $real @args
    }
}
```

```nu
# nushell — $nu.config-path
def --wrapped git [...args] {
    if ($args | length) >= 2 and $args.0 == "worktree" and $args.1 == "add" {
        ^git sprout add ...($args | skip 2)
    } else {
        ^git ...$args
    }
}
```

The bash, zsh, fish and nushell blocks were executed against a real git before release.
The PowerShell one was reviewed line by line but has not been run on any machine yet, so
treat it as unverified and [say so if it misbehaves](https://github.com/alltuner/git-sprout/issues).

**2. A `git` shim on `PATH`**, for everything else. Editors, worktree managers, CI jobs and
agent harnesses spawn `git` themselves, so a shell function never sees them. The shim is a
small `git` wrapper in its own directory that you put ahead of the real git on `PATH`; it
rewrites `worktree add` and passes every other command straight through to the real git.

```bash
git sprout install-shim      # prints the directory it wrote and the PATH line to add
git sprout uninstall-shim    # removes it
```

`brew install` never does this on its own. A wrapper in front of `git` is yours to opt
into, and one command to undo.

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

If this project was useful to you, [consider supporting its development](https://alltuner.com/sponsor).

---

<p align="center">
  Built by <a href="https://davidpoblador.com">David Poblador i Garcia</a> with the support of <a href="https://alltuner.com">All Tuner Labs</a>.<br>
  Made with ❤️ in Poblenou, Barcelona.
</p>
