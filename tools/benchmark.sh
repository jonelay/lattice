#!/bin/sh
set -eu

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
root=$(CDPATH= cd "$script_dir/.." && pwd)
cd "$root"

usage() {
    printf 'usage: %s [--save-baseline <name> | --baseline <name>]\n' "$0" >&2
    exit 2
}

mode=none
baseline=
case $# in
    0) ;;
    2)
        baseline=$2
        # Every run deletes the directories of these names, so no baseline can live there.
        case $baseline in
            '' | new | change) usage ;;
        esac
        case $1 in
            --save-baseline) mode=saved ;;
            --baseline) mode=compared ;;
            *) usage ;;
        esac
        ;;
    *) usage ;;
esac

criterion_dir=${LATTICE_CRITERION_DIR:-$root/target/criterion}
fixture_manifest=$root/crates/latt-core/benches/fixture.json
report=$root/target/benchmarks/lattice-benchmark-v2.json

# new/ holds one run's results and change/ its comparison; a stale one must not read as
# this run's. Named baselines and Criterion's base/ sit beside them and are kept.
if [ -d "$criterion_dir" ]; then
    find "$criterion_dir" -type d \( -name new -o -name change \) -prune -exec rm -rf {} +
fi

set --
case $mode in
    saved) set -- --save-baseline "$baseline" ;;
    compared)
        if [ -n "$(find "$criterion_dir" -path "*/$baseline/estimates.json" -print -quit 2>/dev/null)" ]; then
            set -- --baseline-lenient "$baseline"
        else
            printf 'No saved baseline named %s under %s; running without comparison\n' \
                "$baseline" "$criterion_dir"
            mode=none
        fi
        ;;
esac

CRITERION_HOME=$criterion_dir cargo bench -p latt-core --bench core --locked -- --noplot "$@"

set -- --mode "$mode"
if [ -n "$baseline" ]; then
    set -- "$@" --baseline "$baseline"
fi
cargo run --quiet -p latt-core --example benchmark_report --locked -- \
    "$criterion_dir" "$fixture_manifest" "$report" "$@"
test -s "$report"
printf 'Benchmark report saved to %s\n' "$report"
