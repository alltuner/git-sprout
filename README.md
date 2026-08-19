<p align="center">
  <strong>git-sprout</strong>
</p>

<p align="center">
  <strong>Stop paying for the same tree twice.</strong><br>
  A drop-in replacement for <code>git worktree add</code> that materialises the new
  worktree with filesystem copy-on-write clones instead of copying your tree.
</p>

<p align="center">
  <a href="https://alltuner.com/sponsor">Sponsor</a>
</p>

<p align="center">
  <img src="https://img.shields.io/crates/v/git-sprout?color=5B2333" alt="crates.io">
  <img src="https://img.shields.io/github/license/alltuner/git-sprout?color=5B2333" alt="License">
  <img src="https://img.shields.io/github/stars/alltuner/git-sprout?color=5B2333" alt="Stars">
</p>

---

> **Status: under construction.** The compatibility promise below is the goal, not
> yet a verified claim. It becomes a claim when the differential suite is green.

## What it does

Every worktree you create is a second full copy of your tree on disk. On APFS,
btrfs, XFS with reflinks, bcachefs and ReFS it does not have to be: the files can
share disk blocks with a checkout you already have until something writes to them.

```bash
git sprout add ../myrepo-feature -b feature     # or: git worktree-fast add
```

Same flags, same output, same hooks, same index. On a filesystem without block
cloning it simply runs `git worktree add`.

## Development

```bash
just            # menu of dev tasks
just build      # build the binaries
just test       # run the workspace tests
just check      # fmt + clippy
```

## License

[MIT](LICENSE)

---

<p align="center">
  Built by <a href="https://davidpoblador.com">David Poblador i Garcia</a> with the support of <a href="https://alltuner.com">All Tuner Labs</a>.<br>
  Made with ❤️ in Poblenou, Barcelona.
</p>
