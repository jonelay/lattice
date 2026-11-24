#!/bin/sh
set -eu

case "${1:-}" in
    --snapshot)
        tooling_flag=--no-api-compat
        export LATT_GATE_SNAPSHOT=1
        ;;
    "")
        tooling_flag=
        unset LATT_GATE_SNAPSHOT
        ;;
    *) echo "usage: $0 [--snapshot]" >&2; exit 2 ;;
esac

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
cd "$script_dir/.."

target_dir="${CARGO_TARGET_DIR:-target}"
check_latt_version() {
    version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
    actual_version=$("$target_dir/debug/latt" --version)
    printf '%s\n' "$actual_version"
    if [ -z "$version" ] || [ "$actual_version" != "latt, version $version" ]; then
        echo "error: expected latt, version $version from $target_dir/debug/latt" >&2
        exit 1
    fi
}

cargo fmt --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo build --workspace --locked
check_latt_version
cargo bench -p latt-core --bench core --locked -- --test
cargo test --workspace --locked
# A host's dependencies may unify `serde_json/preserve_order` into latt-core;
# JSON output must not move. Only this run can build an unsorted attr map, so
# it is the gate, not a check of one. `--locked` holds because toml_edit
# already brings `indexmap` into Cargo.lock.
cargo test -p latt-core -p latt --features serde_json/preserve_order --locked
# Repeats the doctests `cargo test` already ran, on purpose: documentation stays
# its own gate if the ordinary test runner changes (development-verification spec).
cargo test --workspace --doc --locked
cargo test -p latt-core --no-default-features --locked
cargo clippy -p latt-core --all-targets --no-default-features --locked -- -D warnings
cargo test -p latt --no-default-features --locked
# Every other configuration builds `tui` beside `native-adapters`.
cargo clippy -p latt --all-targets --no-default-features --features tui --locked -- -D warnings
# The feature-restricted test runs above rebuilt latt without its
# defaults; the suites and the self-audit below run the default binary.
cargo build --workspace --locked
check_latt_version
# latt's crossterm dependency exists only to select `use-dev-tty` in
# ratatui's copy through feature unification, which works while the tree holds
# exactly one crossterm. A second one would make the feature apply to the wrong
# copy without warning; this fails first.
crossterms=$(cargo tree -p latt --prefix none --no-dedupe --locked \
    | grep '^crossterm v' | sort -u | wc -l)
[ "$crossterms" -eq 1 ] || { echo "error: $crossterms crossterm versions in the tui tree" >&2; exit 1; }
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
RUSTDOCFLAGS="-D warnings" cargo doc -p latt-core --no-default-features --no-deps --locked
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --document-private-items --locked
./tools/verify-rust-tooling.sh $tooling_flag
export LATT_GATE=1
.venv/bin/python -m pytest -q -rs
openspec validate --specs --strict
"$target_dir/debug/latt" validate --profile profiles/openspec.yaml \
    --adapter ./adapters/openspec --target .
# The agent-docs eval is a workspace of its own, absent from the release snapshot.
# Build it beside the shipped binaries used by its suites.
if [ -d crates/latt-eval ]; then
    cargo fmt --check --manifest-path crates/latt-eval/Cargo.toml
    set -- --manifest-path crates/latt-eval/Cargo.toml --target-dir "$target_dir" --locked
    cargo clippy "$@" --all-targets -- -D warnings
    cargo test "$@"
    RUSTDOCFLAGS="-D warnings" cargo doc "$@" --no-deps --document-private-items
    # Which commands and finding codes the eval's answer keys exercise: printed,
    # gating nothing (agent-docs-eval spec, Feature claims).
    "$target_dir/debug/latt-eval" claims --binaries "$target_dir/debug"
    # The self-audit again, with the eval's suites citing requirements too.
    "$target_dir/debug/latt" validate --profile crates/latt-eval/openspec.yaml \
        --adapter ./adapters/openspec --target .
fi
