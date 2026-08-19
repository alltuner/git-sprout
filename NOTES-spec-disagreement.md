# Where the spec is wrong or underspecified

Written while building the differential harness (§8). Nothing here was acted on by
diverging from the spec; the harness follows the spec as written except where a later
section already contradicts an earlier one, and each of those is listed below.

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
