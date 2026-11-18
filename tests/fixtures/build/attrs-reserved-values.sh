#!/bin/sh
# ABOUTME: Attribute values spelled like the states `git check-attr` prints, `unset` and
# ABOUTME: `unspecified`, which git reads as strings, beside the genuine states.
. "$(dirname "$0")/../lib.sh"

init_repo
printf '%s\n' \
    '*.txt crlf=unset' \
    '*.md text=unset' \
    '*.bin -crlf' \
    '*.dat -text' \
    '*.up -crlf filter=unspecified' \
    '*.down -crlf filter=unset' > "$REPO/.gitattributes"
# A filter driver named `unspecified`, which `filter=unspecified` names as a string.
g config filter.unspecified.clean "sed s/SMUDGED/BLOB/"
g config filter.unspecified.smudge "sed s/BLOB/SMUDGED/"
# And one named `unset` that only a worktree on a branch sees: the source checkout is left
# detached below, so its configuration has no such driver while a new worktree's does.
printf '[filter "unset"]\n\tclean = sed s/SMUDGED/BLOB/\n\tsmudge = sed s/BLOB/SMUDGED/\n' \
    > "$REPO/.git/branch-drivers"
g config 'includeIf.onbranch:**.path' branch-drivers
base_history
write_text_tree "src" 12 8 2
write_text "docs/guide.md" 6
# LF text behind genuine `-crlf` and `-text`: binary to git, so a checkout writes it as
# stored even under core.autocrlf=true, and a clone of it is a checkout.
write_text "assets/notes.bin" 5
write_text "assets/notes.dat" 5
write_binary "assets/blob.bin" 4
# Cleaning leaves it as it is, so the stat cache calls it clean, but a checkout smudges it.
printf 'value BLOB here\n' > "$REPO/assets/marked.up"
printf 'value BLOB here\n' > "$REPO/assets/marked.down"
commit "lf tree"
finish_repo
g checkout -q --detach

# `git check-attr` prints `unset` both for `-crlf` and for `crlf=unset`, but git reads
# the string as a value it does not know and falls back to core.autocrlf, so a checkout
# writes CRLF for src/ and docs/ while their stat-clean working copies hold LF.
g config core.autocrlf true
for path in src/group0/file0.txt docs/guide.md assets/notes.bin; do
    case "$(g ls-files --eol -- "$path")" in
        *w/lf*) ;;
        *) echo "fixture precondition: $path should hold LF in the working tree" >&2; exit 1 ;;
    esac
done
# Git's own reading of the attributes, which `ls-files --eol` shows: nothing for the
# string, `-text` for the genuine article.
case "$(g ls-files --eol -- src/group0/file0.txt)" in
    *"attr/ "*) ;;
    *) echo "fixture precondition: git should ignore crlf=unset" >&2; exit 1 ;;
esac
case "$(g ls-files --eol -- assets/notes.bin)" in
    *attr/-text*) ;;
    *) echo "fixture precondition: git should read -crlf as -text" >&2; exit 1 ;;
esac
if g config --get-regexp '^filter[.]unset[.]' >/dev/null; then
    echo "fixture precondition: the detached source should not see the unset driver" >&2
    exit 1
fi
if [ -n "$(g status --porcelain)" ]; then
    echo "fixture precondition: the source checkout should be clean" >&2
    exit 1
fi
