#!/usr/bin/env bash
# Build the GitHub release snapshot commit for a revision: its tree holds only the
# paths tools/snapshot-files ships, its one parent is the previous snapshot, and none
# of the development history comes with it.
#
# Usage: tools/release-snapshot.sh <rev> [<parent>] [<message>]
#
# <parent> defaults to github/main; <message> to "lattice <version>", the version read
# from <rev>'s Cargo.toml. The rules are read from <rev>'s own tools/snapshot-files,
# so a snapshot is rebuilt identically from the same revision. Every tracked file
# must fall under an entry, the longest match deciding; an unclassified file exits 2
# by name, so a new path neither ships nor drops out unseen.
#
# Prints the new commit. Nothing is pushed and neither the working tree nor the
# index is touched. Publishing is a separate, deliberate step, the branch and then
# the release tag on the snapshot commit, each by full ref:
#   git push github <commit>:refs/heads/main
#   git push github <commit>:refs/tags/v<version>
# Never push the local tag by name (`git push github v<version>`): it points at the
# development commit and would publish its whole history.
set -euo pipefail

die() {
    echo "release-snapshot: $*" >&2
    exit 2
}

[ $# -ge 1 ] && [ $# -le 3 ] || die "usage: $0 <rev> [<parent>] [<message>]"
rev=$(git rev-parse --verify --quiet "$1^{commit}") || die "$1 is not a commit"
parent=$(git rev-parse --verify --quiet "${2:-github/main}^{commit}") ||
    die "parent ${2:-github/main} is not a commit; fetch it or name another"
rules=$(git show "$rev:tools/snapshot-files" 2>/dev/null) ||
    die "$1 has no tools/snapshot-files"
version=$(git show "$rev:Cargo.toml" | sed -n 's/^version = "\(.*\)"$/\1/p' | head -n 1)
message=${3:-lattice $version}

# Each tracked file, prefixed by the decision of its longest matching entry:
# "+", "-", or "?" when no entry covers it.
decided=$(git ls-tree -r --name-only -z "$rev" | tr '\0' '\n' | awk -v rules="$rules" '
BEGIN {
    n = split(rules, line, "\n")
    for (i = 1; i <= n; i++) {
        if (line[i] ~ /^#/ || line[i] ~ /^[ \t]*$/) continue
        if (line[i] !~ /^[+-] [^ ]/) { print "!" line[i]; bad = 1; continue }
        rule[substr(line[i], 3)] = substr(line[i], 1, 1)
    }
}
{
    best = ""
    for (p in rule)
        if (($0 == p || index($0, p "/") == 1) && length(p) > length(best)) best = p
    print (best == "" ? "?" : rule[best]) $0
}')

malformed=$(printf '%s\n' "$decided" | sed -n 's/^!//p')
[ -z "$malformed" ] || die "malformed tools/snapshot-files line: $malformed"
unclassified=$(printf '%s\n' "$decided" | sed -n 's/^?//p')
if [ -n "$unclassified" ]; then
    printf '%s\n' "$unclassified" | sed 's/^/release-snapshot: unclassified: /' >&2
    die "add each path above to tools/snapshot-files as + (ship) or - (withhold)"
fi

scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
export GIT_INDEX_FILE=$scratch/index
git read-tree --empty
printf '%s\n' "$decided" | sed -n 's/^+//p' | tr '\n' '\0' |
    xargs -0 git ls-tree -z "$rev" -- | git update-index -z --index-info
tree=$(git write-tree)
git commit-tree "$tree" -p "$parent" -m "$message"
