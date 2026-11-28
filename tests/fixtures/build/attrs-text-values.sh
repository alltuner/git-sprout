#!/bin/sh
# ABOUTME: `text=unset`, `crlf=unset` and `text=foo`, values git does not know, beside `-text`
# ABOUTME: and `-crlf` under core.autocrlf=true: only `-text` and `-crlf` make a path binary.
. "$(dirname "$0")/../lib.sh"

init_repo
printf '%s\n' \
    '*.txt text=unset' \
    '*.cfg crlf=unset' \
    '*.dat -text' \
    '*.raw -crlf' \
    '*.bin binary' \
    '*.ini text=foo crlf' \
    '*.log text=foo -crlf' > "$REPO/.gitattributes"
base_history
write_text "src/one.txt" 20
write_text "src/two.txt" 30
write_text "etc/app.cfg" 6
# LF text behind a genuine `-text` and `-crlf`: binary to git, so a checkout writes it as
# stored even under core.autocrlf=true, and a clone of it is a checkout.
write_text "assets/notes.dat" 5
write_text "assets/notes.raw" 5
write_binary "assets/blob.bin" 4
# A `text` value git does not know passes the decision to `crlf`: text for etc/app.ini,
# binary for assets/notes.log.
write_text "etc/app.ini" 6
write_text "assets/notes.log" 5
commit "lf tree"
finish_repo

# `git check-attr` prints `unset` for `text=unset` as it does for `-text`, but git reads
# the string as a value it does not know and falls back to core.autocrlf, so a checkout
# writes CRLF for src/ and etc/ while their stat-clean working copies hold LF.
g config core.autocrlf true
for path in src/one.txt etc/app.cfg etc/app.ini assets/notes.dat assets/notes.raw assets/notes.log; do
    case "$(g ls-files --eol -- "$path")" in
        *w/lf*) ;;
        *) echo "fixture precondition: $path should hold LF in the working tree" >&2; exit 1 ;;
    esac
done
# Git's own reading of the attributes, which `ls-files --eol` shows: nothing for the
# strings, `-text` for the genuine articles.
for path in src/one.txt etc/app.cfg; do
    case "$(g ls-files --eol -- "$path")" in
        *"attr/ "*) ;;
        *) echo "fixture precondition: git should not know the value on $path" >&2; exit 1 ;;
    esac
done
for path in assets/notes.dat assets/notes.raw assets/notes.log; do
    case "$(g ls-files --eol -- "$path")" in
        *attr/-text*) ;;
        *) echo "fixture precondition: git should read $path as -text" >&2; exit 1 ;;
    esac
done
case "$(g ls-files --eol -- etc/app.ini)" in
    *"attr/text "*) ;;
    *) echo "fixture precondition: git should read etc/app.ini as text" >&2; exit 1 ;;
esac
if [ -n "$(g status --porcelain)" ]; then
    echo "fixture precondition: the source checkout should be clean" >&2
    exit 1
fi
