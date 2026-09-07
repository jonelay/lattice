# Changelog

All notable changes to lattice are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

This project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
**While below 1.0.0, minor bumps (0.x.0) may include breaking changes to
public surfaces (node IDs, profile `id_pattern`s, trace-JSON payload schema).
Patch bumps (0.x.y) do not change public surfaces.**

See `openspec/specs/trace-report/spec.md` for which trace-JSON fields are
public and what each version axis governs.

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
- **Letter-suffix `spec-goal` IDs in the RM profile.** `3.6a` sits
  between `3.6` and `3.7` without renumbering. One lowercase letter on
  the last component only; a 27th insertion forces a decision. Profile-
  only change, no code. `requirements-rm` 1.11.0.
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
  entry; on a consumer target the 19 flat findings become 18 deep, and
  REQ-0708 (covered through children) correctly loses its finding.
- **Openspec adapter and profile: lattice audits its own register.**
  `adapters/openspec` reads `openspec/specs/*/spec.md` and `Requirement:`
  citation comments in test files. `profiles/openspec.yaml` keeps
  `DANGLING_REF` at `error` and demotes `COVERAGE` to `hint`. The
  self-audit exits 0.
- **Coverage evidence model, `orphan_ok`, and the `hint` tier.** The
  adapter emits every statically discovered test (unmarked
  ones with no edges). `COVERAGE` gains a `state` field (`unverified` /
  `unknown`). `COVERAGE_UNKNOWN` reports at severity `hint`: never
  promoted by `--strict`, never part of the exit code. `orphan_ok: true`
  on a node kind exempts it from `ORPHAN_NODE`. Contract version 1.1.
- **Openspec register carries summary text.** `title` attr on
  `requirement` nodes, `summary_attr` bindings on both kinds.
  `profiles/openspec.yaml` 1.1.0.
- **`tomlreg` adapter.** Generic TOML register as a standing non-markdown
  format gate. Axis named `stage` (not `phase`) to prove axis names are
  profile data. `profiles/tomlreg.yaml`, `tests/fixtures/mini-tomlreg`.
- **`summary_attr` on node kinds.** Moves the trace "Key Attr" column
  into the profile. The hardcoded `KEY_ATTRS` table is deleted.
  `requirements-rm` 1.6.0 → 1.7.0.
- Dual license: MIT or Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`).

### Changed
- **Trace gate baselines are synthetic.** Replaced consumer-captured
  consumer-captured fixtures with
  `synthetic.*`: 31 nodes, 27 edges, findings spanning every severity.
  Finding-code coverage is a superset of the old baselines'.
- `requirements-rm` declares `text_attrs: [function, docstring]` on its
  `test` kind (1.10.0). Migration only — the sidecar already read both.
- `profiles/openspec.yaml` declares body attrs and `text_attrs` (1.2.0).
- **BREAKING (provenance strings).** Registers read as UTF-8; provenance
  paths rendered relative to the target. On a consumer target: same
  findings multiset (56 findings, 1128 edges), different rendered bytes.
- Adapter file-walking deduplication: bipolaris `validate` 144 → 89 ms,
  consumer ~178 → ~163 ms.

### Fixed
- Suggestion sidecar ranks duplicated node IDs once, over the union of
  occurrences' text. Previously kept only the last occurrence.
- Loader rejects `text_attrs` entries whose attr type is not textual
  (`string` or `enum`). No ranked profile affected.

### Removed
- **BREAKING (bipolaris adapter consumers).** The bipolaris adapter,
  profile, tests, fixture, and capability spec moved to
  `bipolaris-world-runtime`. `--adapter ./adapters/bipolaris` no longer
  exists; callers name the program at its path in the consumer repo.
  Node IDs and the contract document are unchanged.

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
| consumer `validate` | 440 ms | 172 ms |
| bipolaris `validate` | 375 ms | 138 ms |

In-process core: 24.3 → 3.5 ms total, `validate` alone 11.3 → 0.39 ms.
At 20× the consumer's size (18,360 nodes, 22,560 edges) the core is
~95 ms and grows linearly.

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
- **Bipolaris adapter.** Reads TOML registries (`registry`/`entry`
  nodes), the obligations ledger (`obligation`/`check`/`adr` nodes),
  and cross-registry / discharge / citation edges. Profile
  `bipolaris-runtime` 1.0.0 → 1.1.0.
- `SOURCE_MISSING` issue code: a `file` reference that does not resolve
  on disk. Warning, profile-overridable.
- `adapter.exclude` in profiles: registry stems the reader skips.
- `CHECK_UNRESOLVED`, `OBLIGATION_UNBACKED`,
  `GROUP_CHILDREN_DISCHARGED` issue codes. `GROUP_CHILDREN_DISCHARGED`
  is **known unsound** — fix-or-drop slated for 0.3.0.
- The adapter emits `SOURCE_MISSING` for backtick-delimited
  paths in spec files that do not resolve. Profile-declared prefixes
  (`adapter.cited_path_prefixes`). 22 issues over 12
  stale paths.

### Changed
- A profile may configure the same validation code more than once.
  Previously a second `COVERAGE` block silently replaced the first.
- **BREAKING (profile)** — `requirements-rm` 1.5.0 → 1.6.0. Second
  `COVERAGE` rule (req needs `fulfills` edge) and
  `adapter.cited_path_prefixes`.
- **BREAKING (profile)** — `requirements-rm` 1.4.0 → 1.5.0. Adapter
  paths follow the target register relocation to `docs/internal/`.
  A target on the old layout needs a forked profile.

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
- Requirements-RM adapter: parses REQUIREMENTS.md tables, spec headings,
  pytest `@mark.req` markers, and success criteria.
- Requirements-RM profile with BN/UN/REQ/spec-goal/test/SC kinds.
- Profile-driven adapter paths (`adapter.paths` in profile YAML).
- Trace report: per-node entries with kind, attrs, edges, provenance, findings.
- Coverage query: REQ nodes missing a `verifies` edge.
