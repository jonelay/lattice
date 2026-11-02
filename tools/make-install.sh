#!/usr/bin/env bash
# Build the release-style install: <dest>/latt/ holding bin/ (every [[bin]] target of
# the latt package, from <bin-dir>)
# beside the repository paths tools/install-files lists.
#
# Usage: tools/make-install.sh <bin-dir> <dest>
#
# Every input is checked before anything is written, and the install is
# assembled beside <dest>/latt and renamed into place only once complete, so
# a failed run leaves nothing to mistake for an install. From a git checkout
# only tracked files ship; from an extracted archive every file does except
# __pycache__. A selected file missing on disk fails the run by name.
set -euo pipefail

die() {
    echo "make-install: $*" >&2
    exit 1
}

[ $# -eq 2 ] || die "usage: $0 <bin-dir> <dest>"
bin_dir=$1
dest=$2
repo=$(cd "$(dirname "$0")/.." && pwd)
final=$dest/latt

[ ! -e "$final" ] || die "$final already exists; remove it or choose another <dest>"

# The binaries are the latt package's [[bin]] targets, which its manifest lists in
# full (autobins = false); a binary left in <bin-dir> by an earlier build is not shipped.
binaries=()
while IFS= read -r name; do
    binaries+=("$name")
done < <(awk '/^\[/ { bin = ($0 == "[[bin]]") } bin && /^name = / { gsub(/^name = "|"$/, ""); print }' \
    "$repo/crates/latt/Cargo.toml")
[ "${#binaries[@]}" -gt 0 ] || die "crates/latt/Cargo.toml lists no [[bin]] target"
for name in "${binaries[@]}"; do
    [ -f "$bin_dir/$name" ] || die "$bin_dir has no $name"
done

paths=()
while IFS= read -r line; do
    case $line in '' | '#'*) continue ;; esac
    [ -e "$repo/$line" ] || die "tools/install-files lists $line, which is missing"
    paths+=("$line")
done <"$repo/tools/install-files"

# The checkout's own top level, not an enclosing repository's: an archive
# extracted inside some other work tree is still an archive.
files=()
if top=$(git -C "$repo" rev-parse --show-toplevel 2>&1) && [ "$top" = "$repo" ]; then
    echo "make-install: git checkout, shipping tracked files only" >&2
    while IFS= read -r -d '' file; do
        files+=("$file")
    done < <(git -C "$repo" ls-files -z -- "${paths[@]}")
else
    echo "make-install: not this checkout's work tree, shipping every file under the listed paths (git: ${top%%$'\n'*})" >&2
    while IFS= read -r -d '' file; do
        files+=("${file#./}")
    done < <(cd "$repo" && find "${paths[@]}" -name __pycache__ -prune -o -type f -print0)
fi
missing=()
for file in "${files[@]}"; do
    [ -e "$repo/$file" ] || missing+=("$file")
done
[ ${#missing[@]} -eq 0 ] || die "tracked but missing on disk: ${missing[*]}"

mkdir -p "$dest"
staging=$(mktemp -d "$dest/.latt.XXXXXX")
trap 'rm -rf "$staging"' EXIT
chmod 755 "$staging"
mkdir "$staging/bin"
for name in "${binaries[@]}"; do
    cp -p "$bin_dir/$name" "$staging/bin/$name"
    chmod +x "$staging/bin/$name"
done
for file in "${files[@]}"; do
    mkdir -p "$staging/$(dirname "$file")"
    cp -p "$repo/$file" "$staging/$file"
done
mv "$staging" "$final"
trap - EXIT
echo "installed $final"
