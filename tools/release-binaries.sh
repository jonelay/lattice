#!/bin/sh
# Build a revision's release binaries for Linux x86_64 and aarch64 against glibc 2.28,
# write SHA256SUMS beside them, and check them with check-release-binaries.sh.
#
# usage: tools/release-binaries.sh <rev> <out-dir>
#
# The tree is `git archive <rev>`, built with cargo-zigbuild, whose target suffix sets
# the glibc the binaries link against; a host build links against the host's glibc
# instead. Source paths are remapped so no binary names the builder's home. Each binary
# target of the `latt` package becomes `<name>-<arch>-linux` in <out-dir>, which must
# not exist yet. Nothing is uploaded.
set -eu

glibc_floor='2.28'
zigbuild_version='cargo-zigbuild 0.24.0'
zig_version='0.17.0'

die() {
    echo "release-binaries: $*" >&2
    exit 2
}

[ $# -eq 2 ] || die "usage: $0 <rev> <out-dir>"
script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
repo=$(CDPATH= cd "$script_dir/.." && pwd)
rev=$(git -C "$repo" rev-parse --verify --quiet "$1^{commit}") || die "$1 is not a commit"
out=$2
[ ! -e "$out" ] || die "$out exists; name a new directory"

actual=$(cargo-zigbuild --version 2>/dev/null || true)
[ "$actual" = "$zigbuild_version" ] ||
    die "expected $zigbuild_version, got '${actual:-not installed}'; install with: cargo install cargo-zigbuild --version 0.24.0 --locked"
# cargo-zigbuild runs `zig` from PATH, else `python3 -m ziglang` (pip install ziglang).
actual=$(zig version 2>/dev/null || python3 -m ziglang version 2>/dev/null || true)
[ "$actual" = "$zig_version" ] ||
    die "expected zig $zig_version, got '${actual:-not installed}'; install with: pip install ziglang==$zig_version"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
git -C "$repo" archive "$rev" | tar -x -C "$work"
bins=$(awk '/^\[/ { bin = ($0 == "[[bin]]") } bin && /^name = / { gsub(/^name = "|"$/, ""); print }' \
    "$work/crates/latt/Cargo.toml")
[ -n "$bins" ] || die "crates/latt/Cargo.toml at $rev lists no [[bin]] target"

cargo_home=${CARGO_HOME:-$HOME/.cargo}
rustup_home=${RUSTUP_HOME:-$HOME/.rustup}
remap="--remap-path-prefix=$work=/lattice --remap-path-prefix=$cargo_home=/cargo"
remap="$remap --remap-path-prefix=$rustup_home=/rustup"

mkdir -p "$out"
for arch in x86_64 aarch64; do
    (cd "$work" && RUSTFLAGS="$remap" CARGO_TARGET_DIR="$work/target" \
        cargo zigbuild --release --locked -p latt --bins \
        --target "$arch-unknown-linux-gnu.$glibc_floor")
    for name in $bins; do
        cp "$work/target/$arch-unknown-linux-gnu/release/$name" "$out/$name-$arch-linux"
    done
done

(cd "$out" && sha256sum -- *-linux >SHA256SUMS)
"$script_dir/check-release-binaries.sh" "$glibc_floor" "$out"/*-linux
