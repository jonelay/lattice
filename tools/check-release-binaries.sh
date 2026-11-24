#!/bin/sh
# Refuse a release binary that a glibc at the floor cannot load cleanly, or that carries
# the builder's home path.
#
# usage: tools/check-release-binaries.sh FLOOR BINARY...
#
# Every GLIBC_ version a binary's `.gnu.version_r` names counts, weak ones too: where a
# weak version is missing the loader still prints "weak version `GLIBC_x' not found" on
# every run, on stderr, where Lattice reports findings. A path under $HOME is a source
# location the build did not remap; 0.7.1's binaries carried 111.
#
# Exit 0 when every binary passes, 1 naming each failure, 2 on bad usage, a missing
# readelf, or a file readelf cannot read.
set -eu

usage() {
    echo "usage: $0 FLOOR BINARY..." >&2
    exit 2
}

[ $# -ge 2 ] || usage
floor=$1
shift
case $floor in
    "" | .* | *. | *..* | *[!0-9.]*) usage ;;
esac
if ! command -v readelf >/dev/null 2>&1; then
    echo "error: readelf not found; it ships with binutils" >&2
    exit 2
fi
home=${HOME:-}
if [ -z "$home" ] || [ "$home" = / ]; then
    echo "error: HOME is unset or /, so the home-path check has nothing to look for" >&2
    exit 2
fi

status=0
for bin in "$@"; do
    if ! verneed=$(readelf -V "$bin" 2>&1); then
        echo "error: readelf cannot read $bin: $verneed" >&2
        exit 2
    fi
    over=$(printf '%s\n' "$verneed" | awk -v floor="$floor" -v bin="$bin" '
        # True when dotted version a is newer than b.
        function newer(a, b,    x, y, n, m, i) {
            n = split(a, x, ".")
            m = split(b, y, ".")
            if (m > n) n = m
            for (i = 1; i <= n; i++)
                if (x[i] + 0 != y[i] + 0) return x[i] + 0 > y[i] + 0
            return 0
        }
        match($0, /Name: GLIBC_[0-9.]+/) {
            v = substr($0, RSTART + 12, RLENGTH - 12)
            weak = ($0 ~ /Flags: WEAK/) ? " (weak)" : ""
            if (newer(v, floor) && !seen[v weak]++)
                printf "%s: needs GLIBC_%s%s, above the glibc %s floor\n", bin, v, weak, floor
        }')
    if [ -n "$over" ]; then
        printf '%s\n' "$over" >&2
        status=1
    fi
    if grep -qF "$home/" "$bin"; then
        echo "$bin: carries the builder's home path ($home/)" >&2
        status=1
    fi
done
if [ "$status" -eq 0 ]; then
    echo "check-release-binaries: $# binaries need at most glibc $floor and carry no home path"
fi
exit "$status"
