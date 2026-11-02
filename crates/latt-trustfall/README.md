# latt-trustfall

Development-only. Not published, not installed, not part of the supported surface.

Cross-checks lattice's validators against [Trustfall](https://github.com/obi1kenobi/trustfall),
an engine lattice did not write. A resolved profile becomes a Trustfall schema (`schema`),
a `LatticeGraph` is served through an adapter built on `latt-core`'s public items alone
(`adapter`), each validation-plan entry with a Trustfall spelling becomes queries
(`translate`), and the cross-check (`crosscheck`) compares lattice's `(code, node_id)` set
with the queries' union over the same graph.

```sh
cargo test -p latt-trustfall --locked            # schema, adapter, cross-check
cargo test -p latt-trustfall --locked crosscheck # the comparison alone
```

The comparison runs over the shipped profiles whose fixture register loads in-process
(the native adapters' `mini-*` fixtures and the synthetic trace baseline), over seeded
generated graphs for every shipped profile and for `tests/fixtures/shapes.yaml` (every
shape and operator, which no shipped profile declares), and over an operator matrix of
single-condition `CONSTRAINT` profiles. A failure names the profile, the entry, the seed
and the IDs each side alone reported.

Where Trustfall has no single spelling the crate combines queries in Rust and says so in
the code: a negated conjunction (`CONSTRAINT.expect`), an ordering operator lattice fails
on an absent attr (`>=` ∪ `is_null`), and the `COVERAGE_DEEP` fixpoint. Shapes it cannot
translate are reported as skipped with the reason, never passed in silence.
`deliberate::RULES` lists the differences that are meant; the operator matrix asserts
each still occurs, so a row cannot outlive the behaviour it excuses.

Contract: the development-verification spec, "Validators are cross-checked against an
independent engine".
