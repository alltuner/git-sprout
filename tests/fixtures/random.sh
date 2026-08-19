#!/bin/sh
# ABOUTME: Builds a small repository whose shape is drawn from a seed, for the
# ABOUTME: property test. The same seed always produces the same repository.
. "$(dirname "$0")/lib.sh"

SEED="${2:?usage: random.sh <destination> <seed>}"

init_repo
python3 - "$REPO" "$SEED" <<'GEN'
import os, random, sys

root, seed = sys.argv[1], int(sys.argv[2])
rng = random.Random(seed)

names = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "a file", "ünï", "🌱"]

# One always-tracked file, so the commit below never has an empty tree to record.
open(os.path.join(root, "manifest.txt"), "w").write("seed %d\n" % seed)
extensions = [".txt", ".bin", ".c", ".md", ""]

directories = [""]
for _ in range(rng.randint(0, 4)):
    parent = rng.choice(directories)
    directories.append(os.path.join(parent, rng.choice(names) + "-dir"))

for index in range(rng.randint(1, 25)):
    directory = rng.choice(directories)
    name = "%s%d%s" % (rng.choice(names), index, rng.choice(extensions))
    path = os.path.join(root, directory, name)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    if rng.random() < 0.2:
        body = bytes(rng.randrange(256) for _ in range(rng.randint(1, 2048)))
        open(path, "wb").write(body)
    else:
        lines = "".join("line %d\n" % n for n in range(rng.randint(0, 40)))
        open(path, "w").write(lines)
    if rng.random() < 0.15:
        os.chmod(path, 0o755)

if rng.random() < 0.5:
    rules = []
    if rng.random() < 0.5:
        rules.append("*.txt text eol=%s" % rng.choice(["lf", "crlf"]))
    if rng.random() < 0.5:
        rules.append("*.c ident")
    if rng.random() < 0.3:
        rules.append("*.bin -text")
    if rules:
        open(os.path.join(root, ".gitattributes"), "w").write("\n".join(rules) + "\n")

if rng.random() < 0.5:
    open(os.path.join(root, ".gitignore"), "w").write("*.log\nignored-dir/\n")
    os.makedirs(os.path.join(root, "ignored-dir"), exist_ok=True)
    open(os.path.join(root, "ignored-dir", "thing.txt"), "w").write("ignored\n")
    open(os.path.join(root, "noise.log"), "w").write("ignored\n")

if rng.random() < 0.6:
    target = "does-not-exist" if rng.random() < 0.3 else "."
    link = os.path.join(root, "a-link")
    try:
        os.symlink(target, link)
    except OSError:
        pass
GEN

commit "generated base"

# A second commit so HEAD~1 resolves, sometimes touching the tree.
python3 - "$REPO" "$SEED" <<'GEN'
import os, random, sys
root, seed = sys.argv[1], int(sys.argv[2])
rng = random.Random(seed + 1)
# Always a change, so the second commit exists and HEAD~1 resolves for every seed.
open(os.path.join(root, "second.txt"), "w").write("second commit %d\n" % rng.randrange(1000))
GEN
commit "generated second"

finish_repo
