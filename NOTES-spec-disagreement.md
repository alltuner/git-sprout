# Where the build specification is wrong or underspecified

Found while implementing `git-sprout` and building its differential test suite
against real `git worktree add` (git 2.55.0, macOS 26.6.1, Apple M2). Nothing here
was acted on by quietly diverging from the spec: each item was raised, and the two
that changed behaviour were decided deliberately and are recorded as such.

Items 1-10 surfaced from the differential harness, 11-13 from the implementation.
Items 11 and 12 are the load-bearing ones: the first says the spec's verification
rule is unsound, the second that its recommended construction does not do what it
claims. Both were measured, not reasoned.

Three decisions were taken on these findings and are final unless revisited:

- **Item 11 (verification):** accept the limit. The tool clones only where checking
  out would not rewrite the bytes at all, so a repository with `core.autocrlf=true`
  or `* text=auto eol=crlf` accelerates nothing, by design. The documentation says so.
- **Item 9 (mtimes):** leave the behaviour and document it. Cloned files keep the
  source checkout's modification time.
- **Item 10 (progress output):** exempt git's checkout progress meter from §3.2's
  byte-for-byte requirement. The meter measures how much work the checkout did, and
  doing less work is the entire technique.

---

## Found from the differential harness

## 1. §8 contradicts §3.5.1 on `git status --porcelain`

§8 lists among the things to compare:

> `git status --porcelain` (must be empty in both)

§3.5.1 says the opposite, and is right:

> The differential test asserts the dirty set matches, not that it is empty. A test
> that asserts `git status` is clean will fail on the kernel for both implementations
> and teach the next person to skip the fixture.

The harness compares the sorted sets for equality and never asserts emptiness. The
parenthesis in §8 should be deleted, because it is the sentence a reader skims.

## 2. The tree oid is not a parity signal

§3.5.1 and the §9.1 table both offer "identical tree oid `92b9cabb…`" as corroboration
that the prototype matched git on the kernel. It corroborates nothing. `git write-tree`
builds the tree from the **index**, not from the working tree, so it reproduces
`HEAD^{tree}` even with thirteen files physically wrong on disk. Any implementation
that writes a plausible index passes this check, including a badly broken one.

The real signals are the porcelain set, the on-disk content hashes, and the index
entry comparison. `tests/differential/tests/kernel.rs` carries a comment saying so, at
the place where somebody would be tempted to reintroduce it as a cheap check.

## 3. "mtimes where git sets them deterministically" describes an empty set

§8 asks the harness to compare

> every file in the working tree, including mode bits, symlink targets, and mtimes
> where git sets them deterministically

There are no such mtimes. Git writes checked-out files at the time of the checkout, so
two runs of the same operation legitimately differ, and this is the same fact §8 states
one line later about index stat data. The harness therefore compares type, mode bits,
symlink target and content, and does not compare mtimes. It is worth saying explicitly
in the spec, because "compare mtimes" is exactly the plausible-sounding assertion that
would make the suite flaky and get it disabled.

(The related question of what mtime a *cloned* file ends up with is item 9, which is a
design decision rather than a spec defect.)

## 4. §3.3's hook ordering is a summary, not the sequence

§3.3 lists

```
reference-transaction   (preparing / prepared / committed, several times)
post-index-change       1 0
post-checkout           <null-oid> <new-head> 1
```

The measured sequence (`output-semantics.txt`, reproduced here) has further
`reference-transaction` batches *after* `post-index-change`, including two that abort,
and `post-checkout` last. An implementation that fires the three groups in the order
written would not match. The harness compares the full ordered list with arguments, so
it is covered, but the spec should say the listing is a summary.

## 5. §4 step 5a's stat-clean rule is invalidated by copying a repository

Not a spec error, but a consequence the spec does not draw out and that anyone building
tooling around it will hit. "The source worktree's index entry is stat-clean (size,
mtime, and **inode** match)" means a repository that has just been copied - by a test
harness, by a container image build, by `cp -r` - has an index whose entries are all
stat-dirty, even though every file is byte-identical. Acceleration silently drops to
zero and correctness tests still pass.

The harness works around it by preserving mtimes when it copies a fixture and then
running `git update-index --refresh` in each copy. Worth a sentence in §4 or §11, since
"it accelerated on my machine and not in CI" is otherwise a very confusing bug.

## 6. Split-index repositories: the shared index filename is not comparable

Not addressed by the spec at all. In a `core.splitIndex` repository the worktree admin
directory holds a `sharedindex.<oid>` file whose name is a hash of its own contents -
**including stat data**. Two correct runs of the same operation therefore produce two
differently named shared index files. A byte-or-name comparison of the admin directory
reports a difference that is not one; the harness parses the shared index and compares
its entries instead.

## 7. stderr carries git's progress meter, and the spec's "byte for byte" cannot mean it

§3.2 and §8 require stdout and stderr to match byte for byte. On a large tree they
cannot, as written. `git worktree add` emits its checkout progress on stderr **even
when stderr is a pipe**:

```
Updating files:  8% (8107/95056)\rUpdating files:  9% (8556/95056)\r…100% (95056/95056), done.
```

The percentages are whatever the scheduler allowed, so two runs of *real git against
real git* differ. Measured on the kernel fixture: that single line was the only
difference between two control runs.

The harness compares stderr as a terminal would render it - the last
carriage-return-separated frame of each line - which keeps
`Updating files: 100% (95056/95056), done.` and drops the intermediate frames. That
still catches a checkout that touched a different number of paths, which is the part
an implementation could get wrong.

The spec should say that progress frames are exempt and the final frame is not, since
"stderr byte for byte" otherwise reads as a requirement no implementation can meet.

## 8. `--orphan`'s destination is created before the branch check

Observed, not a disagreement: for the `--orphan` cases the failure text and the state
left on disk differ between a repository that has commits and one that does not. Both
sides of the comparison agree, so the contract holds; it is recorded here only because
the flag matrix deliberately includes cases whose correct answer is an identical
failure, and a reader of the suite may wonder why those are not skipped.

## 9. Open design question: cloned files inherit the source's modification time

**Measured, not inferred.** `tests/differential/tests/mtimes.rs` compares each file in
the new worktree against the same path in the repository it was created from:

```
control (real git worktree add):   0 of 122 checked-out files kept the source's mtime
candidate (git-sprout f4d7553):  122 of 122 checked-out files kept the source's mtime
```

`clonefile(2)` copies the timestamps along with the blocks, so every cloned path lands
with whatever mtime the source checkout had - which may be days old. `git worktree add`
stamps every file with the moment of the checkout.

**This is an observable difference, and §3 claims indistinguishability in everything
except disk and time.** It is not a stat field that merely differs between two runs,
like an inode: it differs *systematically and in one direction*, and the thing that
reads it is every mtime-driven build system there is. `make` in a fresh sprout worktree
sees sources older than an artefact restored from a cache and skips work it should do;
`cargo`, `ninja` and `tsc` all key on mtime too. That audience is exactly the audience
for a tool whose pitch is "your fifth worktree costs no disk".

The fix looks cheap: `utimensat` each clone to the checkout's timestamp before writing
the scratch index, and record that timestamp in the index entry so the entry stays
stat-clean. It is one extra syscall per cloned path - against roughly 26 000 clones per
second, a `utimensat` per file is in the same order and would cost something, but the
kernel measurement gives room (5.9s against git's 18.2s).

Three options, in the order I would rank them:

1. **Stamp cloned files with the checkout time.** Matches git exactly, costs one syscall
   per path. My recommendation.
2. **Leave the source's mtime and document it** as the one deliberate exception to §3,
   in the README next to the compatibility promise rather than buried. Defensible only
   if the syscall cost turns out to be real.
3. **Make it a flag.** Worst of the three: it puts a correctness-shaped decision in the
   user's hands and doubles the surface the differential suite has to cover.

Whichever is chosen, it should be chosen. The test above prints the verdict on every
run and asserts the control invariant (git never inherits an mtime), so the current
behaviour is visible rather than accidental - but it deliberately does not fail on the
difference, because the contract has not yet said which answer is correct.

## 10. Progress output is a real divergence, not just a normalisation problem

Item 7 said the spec cannot mean "stderr byte for byte" while git's progress meter is
timing-dependent. Running the suite against the real binary turned that from a
normalisation question into a finding:

```
control   stderr: "Preparing worktree (new branch '…')\nUpdating files: 100% (95056/95056), done.\n"
candidate stderr: "Preparing worktree (new branch '…')\n"
```

The candidate emits no progress line at all - not different frames, none. That follows
from the technique: git's final checkout has almost nothing left to write, so it never
starts a meter. Only the kernel fixture catches it, because git does not show progress
on a small checkout, so every other fixture is silent on both sides.

So §3.2 needs a decision too: either the tool synthesises
`Updating files: 100% (N/N), done.` (it knows N), or the spec exempts progress output
explicitly. The harness keeps the final frame precisely so this stays visible.

---

## Found while implementing the tool

Raised rather than quietly worked around. The implementation takes the safe route in
every case; where that costs acceleration, the decision to accept the cost is recorded
at the top of this file.

## 11. §3.4 / §4 step 5: "stat-clean" does not mean "what a checkout would write"

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

## 12. §3.5.1: the construction the spec recommends does not settle case collisions

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

## 13. Smaller things that are underspecified rather than wrong

- **§4 step 6 says "Version 2 (or 3 if any entry needs extended flags)".** Git keeps the
  version of the index it reads, so a version-2 scratch index makes the *final* index
  version 2 even in a repository configured for `index.version=4`. Observable, and caught by
  comparing the two indexes. Handled by computing the version git would pick and declining
  to accelerate when it is not 2.
- **§4 step 1 lists `--orphan` as an argv condition, but git infers it without the flag.** In a
  repository with no commits, `git worktree add` prints "No possible source branch, inferring
  '--orphan'" and proceeds — so an argv scan never fires, step 2 adds `--no-checkout`, and git
  rejects the combination it just inferred with `fatal: options '--orphan' and '--no-checkout'
  cannot be used together`. No worktree, exit 128, where git succeeds. Every plain `add` in a
  fresh `git init` hits it. An unborn HEAD has to be a delegation condition in its own right.
- **§6.1's "take modes from the tree, never from `stat`" applies to the file on disk, not only
  to the index entry, and it is not a Windows concern.** A block clone copies the source's
  permission bits with its blocks, so a source file somebody ran `chmod 600` on arrives with
  permissions git would never have written. It applies to directories too: a subtree clone
  carries the source directory's mode. Worth stating exactly what a checkout writes, because
  it is not the tree's `0644`/`0755` — git opens with `0666`, or `0777` when the tree marks
  the path executable, and lets the umask do the rest. Under `umask 027` that is `0640` and
  `0750`, and the executable bit is masked independently of the read bits, so the value has
  to come from the real umask rather than from probing a file git created.
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
