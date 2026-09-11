# Changelog

All notable changes to lattice are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

This project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
**While below 1.0.0, minor bumps (0.x.0) may include breaking changes to
public surfaces (node IDs, profile `id_pattern`s, trace-JSON payload schema).
Patch bumps (0.x.y) do not change public surfaces.**

See `openspec/specs/trace-report/spec.md` for which trace-JSON fields are
public and what each version axis governs.

## [0.7.1] — 2026-09-11

### Fixed
- **GitHub CI**: adapter binaries now included in the artifact passed to
  the Python test job, fixing all adapter test failures since 0.7.0.
- **CLI help text**: clearer subcommand descriptions, fixed broken
  `query orphans` summary.

### Changed
- **Docs rewritten for clarity.**

## [0.7.0] — 2026-09-10

### Added
- **`--check-resolved` on `query reaches` and `query reached-by`.** Each
  reached node gains a `tainted` boolean: true when every path from the
  origin to it crosses an undeclared endpoint, false when at least one path
  is declared end to end. Undeclared endpoints are still traversed through
  and still never reported. JSON carries `tainted` per node; plain and rich
  append ` (tainted)` to the node's line, and rich adds a tainted tally to
  its footer. Without the flag the answer is byte-identical to before and no
  second walk runs.
- **`lattice coverage` subcommand.** Per node kind: total nodes, how many
  have at least one incoming edge, how many have at least one outgoing
  edge, and each as a percentage rounded to one decimal. Every declared
  kind appears even at zero, plus any undeclared kind the register
  carries, ordered by name. An edge counts whether or not its far endpoint
  resolves, the same terms as `query orphans`; a dangling endpoint is not
  a node of any kind. JSON carries a `kinds` array of `kind`, `total`,
  `incoming`, `outgoing`, `incoming_pct`, `outgoing_pct`. A report, not a
  validation pass: no `--strict`, adapter issues go to stderr, exit 0 or
  2, never 1.
- **`query diff` reports edge attr changes.** An edge present at both
  revisions under the same (src, tgt, kind) with different attrs lands in a
  new `edges_changed` section carrying both attr maps. Parallel edges pair
  up in document order; unmatched surplus stays in
  `edges_added`/`edges_removed`. Previously an attr-only change on an edge
  was invisible to diff. `nodes_changed` entries gain the same `attrs_a`
  and `attrs_b` keys (additive; `id` and `kind` are unchanged).
- **`lattice summary` works without a `SUMMARY` config.** A profile that
  declares none gets a structural report instead of exit 2: node counts by
  kind, edge counts by kind, and finding tallies by code and severity from
  an internal validation pass (never `--strict`). Suppressed findings are
  left out of the tallies. JSON carries `node_counts`, `edge_counts`, and
  `finding_counts`; the configured rollup's output is unchanged. Finding
  severity in the tallies never affects the exit code; only adapter
  issues do, as before.
- **Finding suppression.** A profile's `validations` list accepts
  `SUPPRESS: {code, node_ids?}` entries. A suppressed finding keeps its
  resolved severity, code, message and provenance, never counts toward
  exit code 1, is omitted from `plain` and `rich`, and stays in `json`
  with `"suppressed": true`. Unsuppressed findings carry no `suppressed`
  key, so existing JSON consumers see no change. Suppression runs after
  `--strict` promotion; strict does not unsuppress. `CONFIG_ERROR`
  cannot be suppressed (load error). Entries for one code merge their
  `node_ids`; a suppress-all supersedes ID-specific entries.
- **`SUPPRESS_UNUSED` finding code** (info). Emitted once per `SUPPRESS`
  entry whose `(code, node_ids)` selector matched no finding this run,
  with the profile as provenance. Overridable and strict-promotable, so
  a profile can gate on stale suppressions; itself suppressible in one
  deterministic pass.
- **Suppression in `lattice fuse`.** A source finding's `suppressed`
  flag survives the merge. Fuse-profile `SUPPRESS` entries apply to
  cross-source and composed-graph findings, with `node_ids` matched
  against composed `source:id`; stale entries report against
  `<fuse-profile>`. Trace-JSON findings gain an optional `suppressed`
  key (absent when false), so `trace_version` is unchanged.

### Changed
- **BREAKING: configured summary JSON key `files` renamed to `groups`.**
  The array of per-group rows in `lattice summary --format json` was named
  after a specific consumer's grouping attr; rows are keyed by whatever
  `group_by_attr` the profile names, and the payload key now says so.
  `totals` and the row shape are unchanged. Update consumers reading
  `.files` to read `.groups`.
- **BREAKING: `ORPHAN_NODE` retired, replaced by directional codes.**
  `UNREFERENCED` (no incoming edges) and `UNTRACED` (no outgoing edges)
  replace the undirected `ORPHAN_NODE`. A fully disconnected node now
  receives both findings. Nodes that previously had edges in only one
  direction (e.g. a root requirement with outgoing traces but no incoming
  edges) were "connected" under the old semantics and raised no finding;
  they now receive the appropriate directional finding. Registers relying
  on `--strict` may see new exit-code failures for these nodes. Add
  `orphan_ok` or a severity override to suppress. Profiles overriding
  `ORPHAN_NODE` severity should override both `UNREFERENCED` and `UNTRACED`
  instead; an override naming `ORPHAN_NODE` becomes a silent no-op.
  `orphan_ok: true` suppresses both new codes.
- **BREAKING: `axes` renamed to `pathways` throughout.** The profile key
  `axes:` is now `pathways:`, the per-validation binding key `axis:` is now
  `pathway:`, and finding codes `AXIS_UNRESOLVED` and `AXIS_INVALID` are
  now `PATHWAY_UNRESOLVED` and `PATHWAY_INVALID`. The `position_attr`
  binding key is unchanged. Profiles using `axes:` or overriding the old
  codes need updating.

### Fixed
- **Ordering condition operators reject non-comparable values at load.**
  `lt`, `gt`, `lte`, `gte` in `CONSTRAINT` entries now require an integer
  or a string as the threshold. Arrays, objects, booleans, nulls, and floats
  previously parsed without error but silently never matched at evaluation;
  the condition evaluated as unsatisfied with no diagnostic. A profile using
  such a value now fails to load with a `CONFIG_ERROR`. This is a load-time
  change only; evaluation behaviour for valid profiles is unchanged.

## [0.6.0] — 2026-09-09

### Added
- **`lattice fuse` subcommand.** Core Rust implementation of multi-source
  graph composition. Reads a fuse manifest, runs `lattice trace` per source,
  assembles a composed graph with `source:` qualified IDs and kinds (colon
  separator), resolves cross-source edges via fuse-profile allowed pairings,
  and runs standard validators on the result. Supports `--strict` and all
  three output formats.
- Fuse manifest uses `name`/`version`/`fuse_profile` keys (renamed from
  `program`/`program_version`/`program_profile`).
- Pathway preservation through fuse: source pathways carried as
  `source:pathway_name`.
- **Composed-ID `id_pattern` validation.** Fuse validates composed node IDs
  against the source profile's declared `id_pattern`, with anchor stripping
  for compatibility with `^...$`-style patterns. Node kinds added only from
  fuse edge endpoint declarations accept any ID.
- Fuse emits a CONFIG_ERROR warning when a source profile cannot be
  reloaded for ID validation.

### Changed
- **BREAKING (finding code).** `DANGLING_REF` renamed to `VACANCY`. Profiles
  that override its severity and downstream consumers matching the code string
  need updating. Adopts the crystal lattice vocabulary from `docs/glossary.md`:
  a missing target node is a vacancy in the lattice structure.
- **Fuse COVERAGE routes through the standard validator.** COVERAGE validations
  in a fuse profile now use the standard `validate` pass, gaining `where:`
  filtering, `COVERAGE_UNKNOWN` hints, `state: "unknown"` for unattributed
  sources, and per-code last-wins severity (replaces per-entry severity).
  Undeclared `edge_kind` now produces CONFIG_ERROR (previously, the custom
  loop would match no nodes and emit no findings).
- Fuse profile COVERAGE entries accept a `where:` condition block.
- `load_profile_value` extracted from `load_profile` for in-memory profile
  construction.

### Removed
- **`tools/lattice-compose` retired.** The Python composition shim is fully
  superseded by `lattice fuse`. Parity gate demonstrated matching nodes,
  edges, and finding codes on the mini-fuse fixture before deletion.

## [0.5.1] — 2026-09-09

### Changed
- Trace baseline test uses a self-contained fixture profile.
- Contract and pinned-behaviour tests exercise the openspec profile.
- Removed a project-specific adapter and profile that belonged in the
  consumer repo.

## [0.5.0] — 2026-09-08

### Added
- **Markdown-table adapter.** Reads `|`-delimited tables from
  profile-selected `.md` files, mapping columns to node attrs and edge
  targets. `crates/adapter-md/`, `profiles/md.yaml`.
- **TOML adapter.** A Rust adapter replaces the Python `tomlreg` adapter,
  with profile-driven multi-kind array-of-tables dispatch, an optional
  header-table reader, an optional axis reader, and `id_prefix` support.
  `crates/adapter-toml/`, `profiles/toml.yaml`.
- **Markdown multi-kind dispatch.** The markdown adapter can map tables under
  different headings to distinct node kinds via `adapter.tables`.
- **Program composition.** `tools/lattice-compose` composes multiple source
  graphs and resolves cross-source edges from a program manifest and profile.
- **Cross-source edge policy.** Edge kinds accept a `cross_source` flag that
  demotes unresolved standalone `DANGLING_REF` findings to hint severity for
  later resolution during program composition.
- **GitHub Issues adapter.** Reads issues via `gh api`, maps labels to
  node kinds, extracts edges from body text via regex patterns, derives
  ordering axes from milestones. `crates/adapter-github/`,
  `profiles/github.yaml`.
- **GitLab Issues adapter.** Reads issues and issue links via `glab api`,
  maps labels to node kinds, extracts edges from descriptions and the
  links API with configurable direction reversal. Uses `iid` (not
  instance-wide number). `crates/adapter-gitlab/`,
  `profiles/gitlab.yaml`.
- **Capability specs for adapter-md, GitHub, and GitLab adapters.** Each
  adapter now has a normative OpenSpec spec under `openspec/specs/`.
- **`FindingCode` enum with exhaustive `default_severity` match.**
  Adding a new finding code without a severity mapping is now a compile
  error.
- **PathReport endpoint invariant in the query spec.** The path query
  spec now documents the src/tgt endpoint constraint and self-loop
  validity.

### Changed
- **BREAKING (lattice-core public API).** `Profile` fields replaced by
  read-only accessors; `add_edge` takes a named `EdgeSpec` struct;
  `ingest_document` takes ownership (`Value`, not `&Value`); `Payload`
  is `#[non_exhaustive]` and `Debug`; `PathReport` uses a
  `Found`/`NotFound` enum with a validated constructor. Adapter-crate
  visibility narrowed to `pub(crate)` (binary-only, no external surface).
- **GitLab adapter: bounded-concurrency link fetching.** 8-worker
  scoped-thread pool replaces sequential per-issue subprocess calls;
  worker panics propagate via `resume_unwind`.
- **Algorithmic improvements.** Deep-coverage propagation uses a
  queue-based worklist, O(V+E). Cycle detection uses iterative Kosaraju
  SCC, O(V+E). `query at` pre-filters nodes before building the trace
  report. JSON output serializes through typed `Serialize` views; BFS
  returns an iterator instead of collecting per call.

### Fixed
- GitLab adapter emits `PARSE_ERROR` when either `project_id` is
  missing on an issue link, instead of silently assuming the link is
  local (cross-project misclassification via IID collision).
- adapter-md parser iterates `chars()` instead of `bytes()` in
  `split_cells`, fixing non-ASCII cell content corruption.
- Scratch profile file uses `create_new(true)` with a random nonce
  instead of a predictable PID-only path, preventing symlink redirect
  and concurrent-call collision.

## [0.4.0] — 2026-09-07

### Added
- **`text_chunk_line_prefix` on node kinds.** The suggestion sidecar ranks
  by best-matching block rather than whole text. Profile-declared line
  prefix, not a regex. Hit@1 0.507 → 0.635, MRR 0.615 → 0.743 on the
  pinned citation labels. Sources are never chunked.
  `profiles/openspec.yaml` 1.3.0.
- **`text_attrs` on node kinds.** Which attrs carry rankable text is now
  profile data. A kind declaring nothing falls back to `summary_attr`;
  `[]` means no text to rank.
- **Openspec adapter carries body text.** Requirement nodes carry prose
  and rationale; test nodes carry the function body (brace-bounded for
  Rust, dedent-bounded for Python, `PARSE_ERROR` where bounds fail).
  Hit@1 0.265 → 0.507, MRR 0.372 → 0.615.
- **Letter-suffix `spec-goal` IDs.** `3.6a` sits between `3.6` and
  `3.7` without renumbering. One lowercase letter on the last component
  only; a 27th insertion forces a decision. Profile-only change, no code.
- **Entomologist adapter.** Reads a git-backed issue tracker's register
  from its `entomologist-data` orphan branch. Fully-qualified refs pinned
  to one commit, repository-root identity check, fetch scoped to
  represented paths. `profiles/entomologist.yaml`.
- **Stacked citations in the openspec adapter.** Consecutive
  `Requirement:` comment lines bind the test group to every named
  requirement. Non-consecutive citations keep replace semantics.
- **`COVERAGE_DEEP`: least-fixed-point coverage rollup.** Profile binds
  `COVERAGE_DEEP {target_kind, via, evidence}`: deep-covered by direct
  evidence or by every deriving child being covered. Evidence-free cycles
  stay uncovered. The RM profile (1.9.0) replaces its flat `COVERAGE`
  entry; on a live consumer the 19 flat findings become 18 deep, and a
  requirement covered through children correctly loses its finding.
- **Openspec adapter and profile: lattice audits its own register.**
  `adapters/openspec` reads `openspec/specs/*/spec.md` and `Requirement:`
  citation comments in test files. `profiles/openspec.yaml` keeps
  `DANGLING_REF` at `error` and demotes `COVERAGE` to `hint`. The
  self-audit exits 0.
- **Coverage evidence model, `orphan_ok`, and the `hint` tier.**
  `COVERAGE` gains a `state` field (`unverified` / `unknown`).
  `COVERAGE_UNKNOWN` reports at severity `hint`: never promoted by
  `--strict`, never part of the exit code. `orphan_ok: true` on a node
  kind exempts it from `ORPHAN_NODE`. Contract version 1.1.
- **Openspec register carries summary text.** `title` attr on
  `requirement` nodes, `summary_attr` bindings on both kinds.
  `profiles/openspec.yaml` 1.1.0.
- **`tomlreg` adapter.** Generic TOML register as a standing non-markdown
  format gate. Axis named `stage` (not `phase`) to prove axis names are
  profile data. `profiles/tomlreg.yaml`, `tests/fixtures/mini-tomlreg`.
- **`summary_attr` on node kinds.** Moves the trace "Key Attr" column
  into the profile. The hardcoded `KEY_ATTRS` table is deleted.
  Profile-only change.
- Dual license: MIT or Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`).

### Changed
- **Trace gate baselines are synthetic.** Replaced consumer-captured
  fixtures with `synthetic.*`: 31 nodes, 27 edges, findings spanning
  every severity. Finding-code coverage is a superset of the old
  baselines'.
- Example profile declares `text_attrs: [function, docstring]` on its
  `test` kind. Migration only - the sidecar already read both.
- `profiles/openspec.yaml` declares body attrs and `text_attrs` (1.2.0).
- **BREAKING (provenance strings).** Registers read as UTF-8; provenance
  paths rendered relative to the target. Same findings multiset on live
  consumers, different rendered bytes.
- Adapter file-walking deduplication: TOML adapter `validate` 144 → 89 ms.

### Fixed
- Suggestion sidecar ranks duplicated node IDs once, over the union of
  occurrences' text. Previously kept only the last occurrence.
- Loader rejects `text_attrs` entries whose attr type is not textual
  (`string` or `enum`). No ranked profile affected.

### Removed
- **BREAKING.** A consumer-specific TOML adapter, profile, tests, fixture,
  and capability spec moved to the consumer repo. Callers name the adapter
  at its path in the consumer repo. Node IDs and the contract document are
  unchanged.

## [0.3.0] — 2026-08-29

The core is now a Rust binary. Output is byte-identical to 0.2.0's on
both real registers across three commands and three formats, including
exit codes.

### Changed
- **BREAKING (adapter contract).** `--adapter` names an executable
  program, not an importable Python module. The program receives
  `--profile` and `--target` and writes a serialized graph-and-issues
  document to stdout. Duplicate node IDs resolved by the core at ingest.
- **BREAKING (installation).** No `lattice` Python package. Build with
  `cargo build --release`; run `target/release/lattice`. The Python
  distribution is `lattice-adapters` (adapters only, stdlib + pyyaml).
- **Trace payload edge order is specified.** Edge kinds sort
  lexicographically; targets sort within a kind.

### Performance

Whole-command wall clock (best of seven):

| | 0.2.0 | 0.3.0 |
|---|---|---|
| TOML consumer `validate` | 375 ms | 138 ms |

In-process core: 24.3 → 3.5 ms total, `validate` alone 11.3 → 0.39 ms.
At 20× scale (18,360 nodes, 22,560 edges) the core is ~95 ms and grows
linearly.

### Not delivered
- `GROUP_CHILDREN_DISCHARGED` remains known-unsound and has never fired
  true. Do not build on this code.

## [0.2.0] — 2026-08-29

### Added
- **Ordering axis.** `axes: [<name>]` in the profile, with `axis` and
  `position_attr` on a validation entry. Findings past the target's
  current position are demoted to `info` before `--strict`. Axis values
  read from the register by the adapter, never declared in the profile.
  New codes: `AXIS_UNRESOLVED`, `AXIS_INVALID`, `TRIGGER_OFF_AXIS`.
- `lattice --version`.
- **TOML consumer adapter.** Reads TOML registries (`registry`/`entry`
  nodes), an obligations ledger (`obligation`/`check`/`adr` nodes),
  and cross-registry / discharge / citation edges.
- `SOURCE_MISSING` issue code: a `file` reference that does not resolve
  on disk. Warning, profile-overridable.
- `adapter.exclude` in profiles: registry stems the reader skips.
- `CHECK_UNRESOLVED`, `OBLIGATION_UNBACKED`,
  `GROUP_CHILDREN_DISCHARGED` issue codes. `GROUP_CHILDREN_DISCHARGED`
  is **known unsound**; fix-or-drop slated for 0.3.0.
- `SOURCE_MISSING` for backtick-delimited paths in spec files that do
  not resolve. Profile-declared prefixes (`adapter.cited_path_prefixes`).

### Changed
- A profile may configure the same validation code more than once.
  Previously a second `COVERAGE` block silently replaced the first.
- **BREAKING (profile).** Example profile adds a second `COVERAGE` rule
  (req needs `fulfills` edge) and `adapter.cited_path_prefixes`.
- **BREAKING (profile).** Example profile adapter paths follow target
  register relocation. A target on the old layout needs a forked profile.

### Fixed
- `summary --format json` tests no longer parse concatenated
  stdout+stderr.
- Findings with no graph node (e.g. `DANGLING_REF` on a ghost source)
  land in `unattachable_findings` instead of being dropped.
- Profile severity overrides apply to adapter issue codes.
- `summary` and `validate` resolve severities through one shared
  operation; they can no longer disagree about the same finding.

## [0.1.0] — 2026-08-09

### Added
- Core graph engine: typed node/edge registers over networkx MultiDiGraph.
- Profile loader: YAML-declared node kinds, edge kinds, ID patterns, validations.
- Validation framework: severity-tagged issue codes, `--strict` promotion.
- Tri-format output: `plain`, `json`, `rich` via `output_result`.
- CLI commands: `validate`, `summary`, `trace`.
- Exit codes: 0 clean, 1 findings, 2 broken setup.
- Example adapter and profile: parses markdown requirement tables,
  spec headings, pytest `@mark.req` markers, and success criteria.
- Profile-driven adapter paths (`adapter.paths` in profile YAML).
- Trace report: per-node entries with kind, attrs, edges, provenance, findings.
- Coverage query: REQ nodes missing a `verifies` edge.
