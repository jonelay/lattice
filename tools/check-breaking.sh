#!/bin/sh
# Fail unless every lint a cargo-semver-checks log reports is listed in the changelog.
#
# usage: check-breaking.sh [--released VERSION] CHANGELOG VERSION LOG...
#
# A log is the output of one or more semver-checks runs, each preceded by a
# `== semver-checks: <package> <configuration>` line. Each failing lint and item it
# names, per configuration, must be listed in a Breaking subsection of the
# changelog's `[Unreleased]` section or of the `[VERSION]` section: a single bullet
# names the lint in backticks and names the item in backticks. A Rust item is named
# by its bare name or a path ending in it (`.` and `::` alike); a containing module
# or type alone does not name it. A contract item (configuration `contracts <file>`)
# is named verbatim, e.g. `latt validate --profile`. The VERSION section counts so
# that a release commit, which moves Unreleased under its version, still reconciles
# the baseline advance before it.
#
# With --released, the last release: when VERSION is a patch release after it, every
# failing lint is refused, listed or not, as `crates/latt/tests/contracts.rs` refuses a
# contract break.
#
# Exit 0 when every lint is listed, 1 when one is not, 2 on bad usage or input,
# including a failing lint that names no item.
set -eu

released=
if [ "${1:-}" = --released ] && [ "$#" -ge 2 ]; then
    released=$2
    shift 2
fi
if [ "$#" -lt 3 ]; then
    echo "usage: $0 [--released VERSION] CHANGELOG VERSION LOG..." >&2
    exit 2
fi
changelog=$1
version=$2
shift 2
for f in "$changelog" "$@"; do
    if [ ! -r "$f" ]; then
        echo "error: cannot read $f" >&2
        exit 2
    fi
done

awk -v version="$version" -v released="$released" '
# Split version v into major, minor, patch in s[1..3]; return 1 when it is a pre-release.
function semver(v, s,    pre) {
    pre = sub(/-.*$/, "", v)
    split(v, s, ".")
    return pre
}
# True when the release version heads for is a minor (or major) one after the release r.
# A pre-release of r is still on its way to r: from 0.8.0-rc.8, 0.8.0-rc.9 heads for 0.8.0.
function pending_minor(r, v,    rs, vs, r_pre) {
    r_pre = semver(r, rs)
    semver(v, vs)
    if (vs[1] + 0 != rs[1] + 0) return vs[1] + 0 > rs[1] + 0
    if (vs[2] + 0 != rs[2] + 0) return vs[2] + 0 > rs[2] + 0
    return r_pre && vs[3] + 0 == rs[3] + 0
}
# A Rust path with `.` spelled `::`, or "" when s is not one.
function rust_path(s) {
    sub(/\(\)$/, "", s)
    sub(/,$/, "", s)
    gsub(/\./, "::", s)
    return s ~ /^[A-Za-z_][A-Za-z0-9_]*(::[A-Za-z_][A-Za-z0-9_]*)*$/ ? s : ""
}
# True when the shorter of paths a and b is a `::`-aligned suffix of the longer.
function same_tail(a, b,    na, nb, sa, sb, k, i) {
    na = split(a, sa, "::")
    nb = split(b, sb, "::")
    k = (na < nb) ? na : nb
    for (i = 0; i < k; i++) if (sa[na - i] != sb[nb - i]) return 0
    return 1
}
# True when span names the item: verbatim for a contract, else by a path of the item.
function names(span, item, contract,    p, n, tok, t, q) {
    if (contract) return span == item
    if ((p = rust_path(span)) == "") return 0
    n = split(item, tok, " ")
    # `field kinds of struct Node`: the member is named through its container.
    if (n == 5 && tok[3] == "of") return same_tail(p, tok[5] "::" tok[2])
    # A leading kind word (`function`, `struct`) is not the item.
    for (t = (n > 1 && tok[1] ~ /^[a-z]+$/) ? 2 : 1; t <= n; t++)
        if ((q = rust_path(tok[t])) != "" && same_tail(p, q)) return 1
    return 0
}
function end_bullet() { if (bullet != "") bullets[++nb] = bullet; bullet = "" }

FILENAME == ARGV[1] { in_changelog = 1 } FILENAME != ARGV[1] { in_changelog = 0 }

# The changelog: collect the bullets of the accepted Breaking subsections.
in_changelog && /^## / {
    end_bullet()
    accepted = ($0 ~ /^## \[Unreleased\]/) || index($0, "## [" version "]") == 1
    breaking = 0
    next
}
in_changelog && /^### / { end_bullet(); breaking = accepted && $0 ~ /^### Breaking[ \t]*$/; next }
in_changelog {
    if (!breaking) next
    if (/^- /) { end_bullet(); bullet = $0 }
    else if (/^[ \t]*$/) end_bullet()
    else if (bullet != "") bullet = bullet " " $0
    next
}

# The logs: (configuration, lint, item) for every item a failing lint names.
/^== semver-checks: / { config = substr($0, 19); items = 0; next }
/^--- failure / {
    lint = $3; sub(/:$/, "", lint); items = 0
    failure = config SUBSEP lint
    if (!(failure in witnessed)) { witnessed[failure] = 0; failures[++nf] = failure }
    next
}
/^Failed in:/ { items = 1; next }
items && /^  [^ ]/ {
    item = $0
    sub(/^ +/, "", item)
    # The location, in every form semver-checks prints it, is not the item: an
    # absolute path, or a relative `path:line` from a rustdoc-JSON comparison.
    sub(/,? (previously )?(in( file)?|at) (\/.*|[^ ]+:[0-9]+)$/, "", item)
    witnessed[failure]++
    key = config SUBSEP lint SUBSEP item
    if (!(key in seen)) { seen[key] = 1; order[++nt] = key }
    next
}
items { items = 0 }

END {
    end_bullet()
    for (f = 1; f <= nf; f++) {
        if (witnessed[failures[f]]) continue
        split(failures[f], part, SUBSEP)
        printf "error: %s: %s fails but names no item; the log is truncated or malformed\n", part[1], part[2] > "/dev/stderr"
        bad = 1
    }
    if (bad) exit 2
    if (released != "" && nt && !pending_minor(released, version)) {
        for (t = 1; t <= nt; t++) {
            split(order[t], part, SUBSEP)
            printf "refused: %s: %s: %s\n", part[1], part[2], part[3]
        }
        printf "error: %s is a patch release after %s; a break is refused even when listed\n", version, released > "/dev/stderr"
        exit 1
    }
    unlisted = 0
    for (t = 1; t <= nt; t++) {
        split(order[t], part, SUBSEP)
        listed = 0
        for (b = 1; b <= nb && !listed; b++) {
            if (index(bullets[b], "`" part[2] "`") == 0) continue
            rest = bullets[b]
            while (!listed && match(rest, /`[^`]+`/)) {
                span = substr(rest, RSTART + 1, RLENGTH - 2)
                rest = substr(rest, RSTART + RLENGTH)
                if (names(span, part[3], part[1] ~ /^contracts /)) listed = 1
            }
        }
        if (!listed) {
            printf "unlisted: %s: %s: %s\n", part[1], part[2], part[3]
            unlisted++
        }
    }
    if (unlisted) {
        printf "error: %d of %d breaking lint(s) are not listed under Breaking in [Unreleased] or [%s]\n", unlisted, nt, version > "/dev/stderr"
        exit 1
    }
    printf "check-breaking: %d breaking lint(s), all listed\n", nt
}
' "$changelog" "$@"
