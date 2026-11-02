#!/bin/sh
set -eu

# --no-api-compat skips the API compatibility comparison and says so: for a tree
# whose history cannot hold the baseline commit (the GitHub release snapshot).
# Coverage and the advisory check still run.
api_compat=yes
case "${1:-}" in
    --no-api-compat) api_compat=no ;;
    "") ;;
    *) echo "usage: $0 [--no-api-compat]" >&2; exit 2 ;;
esac

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
cd "$script_dir/.."

llvm_cov_version='cargo-llvm-cov 0.9.1'
semver_checks_version='cargo-semver-checks 0.51.0'
# Verbatim: cargo-audit's version line repeats its subcommand name.
audit_version='cargo-audit-audit 0.22.2'
api_baseline='2c6172111d418afa444dc579d14a9be568ac04a6'
# yes: a baseline advance on a patch line after contracts/released/VERSION is refused
# even when listed. Turned on when 0.8.0 is published; until then a re-cut of 0.8.0
# may still carry a listed break.
refuse_patch_breaks=no
coverage_report='target/coverage/lcov.info'

actual_llvm_cov=$(cargo llvm-cov --version 2>/dev/null || true)
if [ "$actual_llvm_cov" != "$llvm_cov_version" ]; then
    echo "error: expected $llvm_cov_version, got '${actual_llvm_cov:-not installed}'" >&2
    echo "install with: cargo install cargo-llvm-cov --version 0.9.1 --locked" >&2
    exit 2
fi

if [ "$api_compat" = yes ]; then
    actual_semver_checks=$(cargo semver-checks --version 2>/dev/null || true)
    if [ "$actual_semver_checks" != "$semver_checks_version" ]; then
        echo "error: expected $semver_checks_version, got '${actual_semver_checks:-not installed}'" >&2
        echo "install with: cargo install cargo-semver-checks --version 0.51.0 --locked" >&2
        exit 2
    fi
fi

actual_audit=$(cargo audit --version 2>/dev/null || true)
if [ "$actual_audit" != "$audit_version" ]; then
    echo "error: expected $audit_version, got '${actual_audit:-not installed}'" >&2
    echo "install with: cargo install cargo-audit --version 0.22.2 --locked" >&2
    exit 2
fi

if [ "$api_compat" = yes ]; then
    if ! git cat-file -e "$api_baseline^{commit}" 2>/dev/null; then
        echo "error: API baseline commit $api_baseline is unavailable; fetch repository history" >&2
        exit 2
    fi

    # The committed baseline in a revision's copy of this script, or nothing.
    baseline_at() {
        git show "$1:tools/verify-rust-tooling.sh" 2>/dev/null |
            sed -n "s/^api_baseline='\([0-9a-f]*\)'\$/\1/p"
    }

    # The baseline this change supersedes. A working tree that edits the baseline
    # supersedes HEAD's; otherwise a baseline advanced anywhere in the pushed range
    # supersedes the range start's. CI names the range start in LATTICE_RANGE_BASE;
    # unset or all zeros (a branch's first push) falls back to the parent commit.
    superseded=$(baseline_at HEAD)
    if [ "$superseded" = "$api_baseline" ]; then
        range_base=${LATTICE_RANGE_BASE:-}
        if [ -z "$(printf '%s' "$range_base" | tr -d 0)" ] ||
            ! git cat-file -e "$range_base^{commit}" 2>/dev/null; then
            range_base=HEAD^
        fi
        superseded=$(baseline_at "$range_base")
    fi
    if [ "$superseded" = "$api_baseline" ]; then
        superseded=
    fi
    if [ -n "$superseded" ] && ! git cat-file -e "$superseded^{commit}" 2>/dev/null; then
        echo "error: superseded API baseline commit $superseded is unavailable; fetch repository history" >&2
        exit 2
    fi
fi

cargo fetch --locked
if [ "$api_compat" = yes ]; then
    # A baseline resolves its own dependency set. A crate it locks that main has since
    # dropped is absent from a cold cache after the fetch above, and the offline check
    # below would fail to resolve the baseline rather than compare it.
    for rev in "$api_baseline" $superseded; do
        baseline_tree=$(mktemp -d)
        git archive "$rev" Cargo.toml Cargo.lock crates | tar -x -C "$baseline_tree"
        (cd "$baseline_tree" && cargo fetch --locked)
        rm -rf "$baseline_tree"
    done
fi

semver_logs=$(mktemp -d)
trap 'rm -rf "$semver_logs"' EXIT

# RustSec advisories against the committed lockfile, which `cargo fetch --locked`
# above has already proven current, so cargo-audit never resolves one of its own.
# Not hermetic, deliberately: the advisory database is fetched on every run, so this
# step can go red with no change in the repository. A vulnerability fails (exit 1);
# unmaintained, unsound and yanked warnings are printed and gate nothing. cargo-audit
# also exits 1 when the fetch fails, with stdout empty, so only a report saying it
# found a vulnerability counts as one and any other failure exits 2. Never add
# `--no-fetch` or `--stale`: offline, those pass over whatever database is cached.
audit_status=0
cargo audit --file Cargo.lock --json >"$semver_logs/audit.json" || audit_status=$?
if [ "$audit_status" -ne 0 ]; then
    if grep -q '"vulnerabilities":{"found":true' "$semver_logs/audit.json"; then
        cat "$semver_logs/audit.json"
        echo >&2
        echo "error: cargo audit found RustSec vulnerabilities in Cargo.lock" >&2
        exit 1
    fi
    echo "error: cargo audit could not check Cargo.lock (exit $audit_status); fetching the RustSec advisory database needs network access" >&2
    exit 2
fi
# The report carries the warnings, which exit 0, and the database's last update.
cat "$semver_logs/audit.json"
echo
echo "cargo audit: no RustSec vulnerability in Cargo.lock"

# Compare every supported surface with baseline $1, one log per run in directory $2.
# Left to derive the release type from the two version strings, cargo-semver-checks
# treats any pre-release pair as a major change and skips every lint, so the gate
# passed nine breaking changes between rc.1 and rc.3. Assuming a minor release makes
# every breaking change fail until the baseline is advanced on purpose. Default
# features are compared on their own: an item can move between features, so the
# all-features and no-features surfaces do not imply the default one.
# Return 1 when any run fails; exit 2 when one ran no lints, because a skipped
# comparison must never read as a clean one.
compare_surfaces() {
    baseline=$1
    log_dir=$2
    mkdir -p "$log_dir"
    status=0
    n=0
    for run in 'latt-core all-features' 'latt-core default-features' \
        'latt-core only-explicit-features' 'latt-adapter default-features'; do
        set -- $run
        n=$((n + 1))
        log="$log_dir/$n.log"
        echo "== semver-checks: $1 $2" >"$log"
        # A package the baseline lacks (renamed at 0.8.0: lattice-core is latt-core)
        # has nothing semver-checks can compare. Report it in the log's own failure
        # form, so check-breaking.sh demands a Breaking bullet as for any other lint.
        if ! git show "$baseline:Cargo.lock" | grep -qx "name = \"$1\""; then
            printf -- '--- failure package_missing_from_baseline: %s ---\n\nFailed in:\n  crate %s\n\n' \
                "$1" "$(printf '%s' "$1" | tr - _)" >>"$log"
            cat "$log"
            status=1
            continue
        fi
        if ! CARGO_NET_OFFLINE=true cargo semver-checks --package "$1" \
            --baseline-rev "$baseline" "--$2" --release-type minor >>"$log" 2>&1; then
            status=1
        fi
        cat "$log"
        if ! grep -q 'Checked.* [1-9][0-9]* checks:' "$log"; then
            echo "error: cargo semver-checks ran no lints for $1 $2; the comparison was skipped, not passed" >&2
            exit 2
        fi
    done
    return "$status"
}

if [ "$api_compat" = yes ]; then
    version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
    if ! compare_surfaces "$api_baseline" "$semver_logs/current"; then
        # Say whether the comparison is red for listed reasons: the state of a breaking
        # commit before the commit that advances the baseline to it.
        tools/check-breaking.sh CHANGELOG.md "$version" "$semver_logs"/current/*.log || true
        exit 1
    fi
    # An advance waives nothing: every lint against the baseline it supersedes must be
    # listed under Breaking.
    if [ -n "$superseded" ]; then
        echo "API baseline advanced from $superseded; reconciling against Breaking"
        compare_surfaces "$superseded" "$semver_logs/superseded" || true
        released_args=
        if [ "$refuse_patch_breaks" = yes ]; then
            released_args="--released $(cat contracts/released/VERSION)"
        fi
        # Unquoted: two words or none.
        tools/check-breaking.sh $released_args CHANGELOG.md "$version" "$semver_logs"/superseded/*.log
    fi
else
    echo "API compatibility: skipped (--no-api-compat); the comparison gates elsewhere"
fi

mkdir -p "$(dirname "$coverage_report")"
# Keep coverage artifacts isolated while allowing the live adapter fixtures to locate
# the matching instrumented binaries through the development wrappers.
export CARGO_TARGET_DIR="$script_dir/../target/llvm-cov-target"
# `show-env` names the directory the raw profiles land in; `clean` run before it
# cleans a nested default directory instead and leaves every past run's profiles
# behind. A toolchain bump then fails the merge on their older raw-profile format.
eval "$(cargo llvm-cov show-env --sh)"
cargo llvm-cov clean --workspace
cargo build --workspace --all-features --locked
cargo test --workspace --all-features --locked
cargo llvm-cov report --lcov --output-path "$coverage_report"
if [ ! -s "$coverage_report" ]; then
    echo "error: coverage report $coverage_report was not created or is empty" >&2
    exit 2
fi
