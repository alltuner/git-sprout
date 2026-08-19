# ABOUTME: Shared helpers for the fixture builders: repository setup, deterministic
# ABOUTME: content, and the refs every flag case in the matrix expects to find.

set -eu

# A builder calls this when the fixture cannot exist on this machine. The harness
# reports a skip rather than a pass, so an absent fixture is never silent.
skip() {
    echo "$*" >&2
    exit 77
}

# Every builder is handed the directory to build in.
REPO="${1:?usage: <builder> <destination>}"
mkdir -p "$REPO"

g() {
    git -C "$REPO" "$@"
}

# Quiet git that still fails loudly.
gq() {
    git -C "$REPO" "$@" >/dev/null
}

init_repo() {
    gq init -q ${1:+--object-format="$1"}
    g config core.autocrlf false
    g config gc.auto 0
    g config advice.detachedHead false
}

commit() {
    g add -A
    gq commit -q -m "$1"
}

# Deterministic text of a requested line count, with a stable body per file name.
write_text() {
    _path="$1"
    _lines="$2"
    mkdir -p "$(dirname "$REPO/$_path")"
    _n=1
    : > "$REPO/$_path"
    while [ "$_n" -le "$_lines" ]; do
        printf 'line %d of %s\n' "$_n" "$(basename "$_path")" >> "$REPO/$_path"
        _n=$((_n + 1))
    done
}

# Deterministic bytes, derived from the path so two files differ.
write_binary() {
    _path="$1"
    _kib="$2"
    mkdir -p "$(dirname "$REPO/$_path")"
    python3 - "$REPO/$_path" "$_kib" <<'PY'
import hashlib, sys
path, kib = sys.argv[1], int(sys.argv[2])
seed = hashlib.sha256(path.encode()).digest()
out = bytearray()
block = seed
while len(out) < kib * 1024:
    block = hashlib.sha256(block).digest()
    out.extend(block)
open(path, "wb").write(bytes(out[: kib * 1024]))
PY
}

# A whole subtree of deterministic text files, generated in one pass because the
# text-heavy fixtures need hundreds of them.
write_text_tree() {
    _dir="$1"
    _files="$2"
    _lines="$3"
    _groups="${4:-1}"
    python3 - "$REPO/$_dir" "$_files" "$_lines" "$_groups" <<'TREE'
import os, sys
root, files, lines, groups = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
for n in range(files):
    directory = os.path.join(root, "group%d" % (n % groups))
    os.makedirs(directory, exist_ok=True)
    name = "file%d.txt" % n
    with open(os.path.join(directory, name), "w") as handle:
        for line in range(1, lines + 1):
            handle.write("line %d of %s\n" % (line, name))
TREE
}

# A file whose working-tree bytes are UTF-16, for the working-tree-encoding fixture.
write_utf16() {
    _path="$1"
    _lines="$2"
    mkdir -p "$(dirname "$REPO/$_path")"
    python3 - "$REPO/$_path" "$_lines" <<'ENC'
import sys
path, lines = sys.argv[1], int(sys.argv[2])
text = "".join("line %d of utf16\n" % n for n in range(1, lines + 1))
open(path, "wb").write(text.encode("utf-16"))
ENC
}

# The refs the flag matrix reaches for: a tag, a branch that already exists, and a
# remote with two tracking branches so --track and --guess-remote have something to
# resolve. Best effort: a repository with no commits gets none of them.
finish_repo() {
    if ! g rev-parse --verify -q HEAD >/dev/null 2>&1; then
        return 0
    fi
    _head="$(g rev-parse HEAD)"
    g tag v1 "$_head"
    g branch existing "$_head"
    g remote add origin ../fake-remote
    g update-ref refs/remotes/origin/main "$_head"
    g update-ref refs/remotes/origin/topic "$_head"
    # Leave the stat cache valid so an implementation that verifies through the
    # source index sees a clean checkout rather than an unrefreshed one.
    g update-index --refresh -q >/dev/null 2>&1 || true
}

# Two commits so that HEAD~1 resolves in every fixture that has history.
base_history() {
    write_text "README.md" 3
    commit "base"
}
