#!/bin/sh
set -eu

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
cd "$script_dir/.."

cargo fmt --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo build --workspace --locked
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
# The feature-restricted test runs above rebuilt target/debug/latt without its
# defaults; the suites and the self-audit below run the default binary.
cargo build --workspace --locked
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
./tools/verify-rust-tooling.sh
.venv/bin/python -m pytest -q
openspec validate --specs --strict
target/debug/latt validate --profile profiles/openspec.yaml \
    --adapter ./adapters/openspec --target .
# The agent-docs eval is a workspace of its own, absent from the release snapshot.
# Its target directory is the root one, where its suites expect the shipped binaries.
if [ -d crates/latt-eval ]; then
    eval_cargo="--manifest-path crates/latt-eval/Cargo.toml --target-dir target --locked"
    cargo fmt --check --manifest-path crates/latt-eval/Cargo.toml
    cargo clippy $eval_cargo --all-targets -- -D warnings
    cargo test $eval_cargo
    RUSTDOCFLAGS="-D warnings" cargo doc $eval_cargo --no-deps --document-private-items
    # Which commands and finding codes the eval's answer keys exercise: printed,
    # gating nothing (agent-docs-eval spec, Feature claims).
    target/debug/latt-eval claims
    # The self-audit again, with the eval's suites citing requirements too.
    target/debug/latt validate --profile crates/latt-eval/openspec.yaml \
        --adapter ./adapters/openspec --target .
fi
