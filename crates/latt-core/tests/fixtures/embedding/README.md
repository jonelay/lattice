# Synthetic embedding fixture

`actor-a.json` and `actor-b.json` implement the two-actor acceptance matrix. They
use profile-declared vocabulary and real JSON file/line provenance; none is live
host data. `embedding.rs` constructs an ordered mock event stream, projects
whole records and allowed attrs for each actor before ingestion, and reconstructs
it for the replay check. This does not test actual host settlement or persistence.

`cli-baseline.json` contains 324 pre-service CLI stdout/stderr/exit observations,
covering plain/json/rich output. Captured at proposal commit `a63fc23`, whose runtime
sources are identical to `29a9e87`, before changing any production code. The only
normalization is the per-test temporary target directory to `<TARGET>`; revision
labels, messages, whitespace and ANSI bytes remain captured. Individual expected
exits and fixture counts are independently asserted in the tests.

The committed baseline was captured at the pre-service runtime using the original,
since-removed `LATTICE_CAPTURE_SERVICE_BASELINE` mode. Current candidate capture uses
`cargo test -p latt-core --test service_cli_compat capture_service_baseline --
--ignored --exact --nocapture`; it writes the reported system-temporary path and never
overwrites or verifies against the committed baseline. Do not regenerate to make a
service refactor pass; compare a candidate against the original runtime in a separate
checkout if a future intentional contract change needs a new baseline. Existing
trace/fuse baselines remain independent additional gates.

Re-captured once since, for an intentional change (the CLI applies the profile's
`SUPPRESS` to adapter diagnostics). The
candidate was diffed key by key against the committed baseline before replacing it:
the only keys that changed are the `suppressed-error` and `rollup-error` scenarios'
stderr, in every format and command, and those scenarios' `summary` exit, 1 → 0. Every
other key is byte-identical to the original runtime's capture.

Re-captured again: with every adapter issue suppressed, `plain` and `rich` no
longer print an empty report. The key-by-key diff changed 52 keys, all stderr in the
same two scenarios' `plain` and `rich` runs: `"\n"` → `""` (plain) and
`"No findings.\n"` → `""` (rich). `json` and every exit are unchanged.

Re-captured again (self-labelled-plain change): `trace`, `query at` and `coverage` label
their `plain` fields. The key-by-key diff changed 21 keys, all `plain` stdout of `trace`,
`trace-strict`, `at`, `at-dir` and `coverage` in the five scenarios that render them;
each new value equals the old one rewritten field by field (padding dropped, labels
added, the coverage header row removed). `json`, `rich`, stderr and every exit are
unchanged.

Re-captured again (orphan checks follow the allowed pairs): `assertion` can be no edge's
target and `claim` and `reading` no edge's source, so their `UNREFERENCED` and
`UNTRACED` findings are gone. The key-by-key diff changed 84 keys, all stdout of
`validate`, `strict`, `trace`, `trace-strict`, `at`, `at-dir`, `summary` and `overlay`,
each losing only orphan findings and their counts; `strict` and `trace-strict` exit
1 → 0 in `clean`, `rollup-error` and `suppressed-error`, whose only warnings they were.
`warning` still exits 1 under `--strict`. `query orphans` and every stderr are unchanged.
