# Where the build spec is wrong

One item, and it is in the part of the spec that decides whether a worktree is correct.
Raised rather than quietly worked around, per the brief. The implementation currently
takes the safe route described at the end; the decision about what to do instead is not
mine to make.

## §3.4 / §4 step 5: "stat-clean" does not mean "what a checkout would write"

### The claim

Spec §3.4 argues the verification rule can be index-based:

> if the source worktree's index entry for a path is **stat-clean**, git itself considers
> that file to be an unmodified checkout of the oid in that entry. […] So if the entry is
> stat-clean, its oid equals the blob oid we want, and the attributes governing that path
> are identical in both commits, then the source's working-tree bytes are by definition
> what a checkout would write at the destination — conversions and all.

`language.md` leans on the same claim to conclude that Windows with
`core.autocrlf=true` accelerates normally.

### Why it is not true

Stat-clean means git will not re-read the file. What git concludes when it does read it is
`clean(worktree_bytes) == blob`. A checkout writes `smudge(blob)`. Those are the same thing
only when the conversion round-trips for that particular file, and the whole point of the
CRLF conversion is that it does not: `clean` strips CRLF, so a file that is already all-LF
is clean *and* is not what a checkout would write.

This is not a corner case that has to be constructed. Git warns about it by name:

```
warning: in the working copy of 'src/a.txt', LF will be replaced by CRLF the
next time Git touches it
```

Measured, on git 2.55.0, with `*.txt text eol=crlf`:

| | bytes |
| --- | --- |
| blob | 6 |
| source working tree, reported clean by `git status` | 6 |
| what `git worktree add` writes | **7** |

Every way it can arise:

- a file written by an editor or a script and `git add`ed, never checked out, under
  `eol=crlf`, `text=auto eol=crlf`, or `core.autocrlf=true`
- the same after `.gitattributes` gains a `text` rule and nothing is re-checked-out
- `core.autocrlf=input`, where `smudge` is the identity but `clean` still strips CRLF, so a
  clean CRLF file is not the blob
- `ident` and `working-tree-encoding`, on the same argument
- a clean/smudge filter pair that is not injective, Git LFS included

An implementation that follows §4 step 5 as written produces a worktree whose files differ
from `git worktree add`'s, silently, with `git status` clean in both. That is exactly the
failure mode §4's closing paragraph promises cannot happen.

### What is implemented instead

A path is cloned only when checking it out would not rewrite the blob's bytes at all. That
is decided by one `git check-attr -z --stdin text eol ident filter working-tree-encoding`
pass over the candidate paths, plus `core.autocrlf` and `core.eol`, in
`crates/git-sprout/src/attributes.rs`. When no conversion applies,
`smudge == clean == identity`, and then — and only then — stat-clean does mean the source's
bytes are what a checkout would write. Everything else is left to git.

This keeps the correctness promise and costs one extra git invocation. It is exact, not
heuristic: no path that a checkout would rewrite is ever cloned.

### What it costs, and it is the cost the spec was trying to avoid

- A repository with `core.autocrlf=true` clones **nothing**. Measured: 0 of 7 paths.
- A repository with `* text=auto eol=crlf` clones nothing.
- A repository with no conversion attributes clones everything, which is the common case on
  Linux and macOS and includes the kernel fixture.

So §6.1's and `language.md`'s Windows story does not hold as written. Windows on a Dev Drive
still gets the disk saving for binaries and for any repo that does not convert, but the
default Git-for-Windows configuration accelerates nothing.

### Ways to get it back, none of them chosen here

1. **`git cat-file --batch --filters`.** Git will produce `smudge(blob)` for a list of
   `<oid> <path>` pairs in one process. Hash that against the source file and the rule
   becomes exact for every conversion, LFS included. Cost is one filter pass over the
   converted paths — real CPU, but the disk saving, which §1 says is the actual pitch, is
   untouched. This is the option I would pick.
2. **`git ls-files --eol`.** Reports the observed working-tree line ending per path
   alongside the attribute, so the eol family alone can be settled without hashing. Does not
   cover `ident`, `working-tree-encoding` or filters, and it reads every file anyway.
3. **Accept the limit** and say so in the docs: sprout accelerates repositories that do not
   convert on checkout, and passes through the ones that do.

Whichever is chosen, §3.4 and `language.md`'s "Does Rust get us Windows?" section need
rewriting, and §8's repository matrix should assert `cloned > 0` on the `core.autocrlf=true`
fixture — which it already says to do, and which is what would have caught this.

## §3.5.1: the construction the spec recommends does not settle case collisions

### The claim

> The clone plan must not create a file git would not have created, and must not win a
> collision that git would have lost. **The safest construction is the one in §4: clone only
> what the plan says, then let git's own checkout run last and settle every collision
> itself.**

### Why it is not enough

Git's checkout writes entries in index order and each write unlinks and recreates the shared
file, so on a case-folding filesystem the *last* entry written decides both the content and
the name on disk, and the earlier one is reported modified. Letting git run last only settles
collisions among the paths git actually writes — and git skips everything the scratch index
already vouches for.

Measured on the kernel fixture, `bd5f485f3`, case-insensitive APFS. In each colliding pair
only one member is stat-clean in the source (the one whose blob is on disk), so the plan
naturally contains exactly one of the two. Cloning it makes it the *first* writer, and git
then writes the other one last, inverting the winner:

| | on disk | content of `xt_CONNMARK.h` | dirty set |
| --- | --- | --- | --- |
| `git worktree add` | `xt_connmark.h` | `41b578cc` (lowercase's blob) | the 13 **uppercase** names |
| clone-then-let-git-finish | `xt_CONNMARK.h` | `36cc956e` (uppercase's blob) | the 13 **lowercase** names |

Thirteen paths wrong, `git status` clean-looking on both sides, and both worktrees report
exactly 13 modified paths — so any test that counts the dirty set instead of comparing it
passes. §8 already says to compare it, which is what caught this.

### What is implemented

`plan::colliding_paths` groups every tracked path by an ASCII-folded key and drops whole
groups, so neither member is ever cloned and git writes both in its own order. Cost on the
kernel: 26 paths out of 95,056. Result is byte-identical to the control, including the name
on disk and which 13 paths are reported modified.

A filesystem that folds beyond ASCII, as APFS does for Unicode, can still collide on paths
this leaves in the plan. There the second clone fails with `EEXIST` and demotes the run,
which is slow rather than wrong.

## Smaller things that are underspecified rather than wrong

- **§4 step 6 says "Version 2 (or 3 if any entry needs extended flags)".** Git keeps the
  version of the index it reads, so a version-2 scratch index makes the *final* index
  version 2 even in a repository configured for `index.version=4`. Observable, and caught by
  comparing the two indexes. Handled by computing the version git would pick and declining
  to accelerate when it is not 2.
- **§3.3 does not mention that `git worktree add` propagates the `post-checkout` hook's exit
  status.** Measured: a hook exiting 7 makes `git worktree add` exit 7. `git hook run` does
  the same, so firing the hook last and exiting with its status reproduces it.
- **§3.3's `git hook run` invocation needs `--ignore-missing`.** Without it, a repository
  with no `post-checkout` hook gets `error: cannot find a hook named post-checkout` on
  stderr and exit 1.
- **§3.5 lists `link` among the extensions a fresh worktree index carries.** It does not;
  that came from a byte-search in the probe matching the path `src/link.txt`. A fresh
  worktree index is `DIRC v2` with `TREE` only. Already flagged by the team lead.
- **§4 step 4's directory clone needs a condition the spec does not state.** `clonefile(2)`
  on a directory copies whatever is on disk, so a subtree may only be cloned whole when the
  source directory holds *exactly* the tracked entries at every depth. Without that check,
  untracked and ignored files leak into the new worktree, which §3.6 forbids.
