# builtin-adapter-dispatch Specification

## Purpose

Resolution of `--adapter` to either an external program or a named native adapter,
with profile-declared fallback when `--adapter` is omitted.
## Requirements
### Requirement: Adapter resolution precedence
The core SHALL resolve the adapter to use with this precedence:

1. If `--adapter` is supplied, check whether the value matches a known native
   adapter name. If it does, use the native adapter. Otherwise treat the value
   as an external program name or path, preserving the original OS string.
   The core SHALL delegate external lookup to command spawning, including PATH
   lookup for bare names; it SHALL NOT reject a name based on local existence.
   A bare name — one with no directory component — that is not found SHALL be
   retried once as `latt-adapter-<name>`, the program a release install puts on PATH
   beside `latt`; a value with a directory component, and a name already
   starting with `adapter-`, SHALL be run as given and never retried.
2. If `--adapter` is omitted, read the profile's `adapter.native` key. If
   present, use the named native adapter.
3. If neither is available, exit 2 with a message naming both resolution
   paths.

Resolution SHALL happen once and apply uniformly to every code path that runs
an adapter, including `query diff` which runs the adapter at two revisions.

Verified by: `cargo test --test native`, `cargo test --test dispatch`, and
`cargo test --test fuse manifest_order_paths_and_invalid_inputs`.

#### Scenario: Native adapter by name on the command line
- **WHEN** the user runs `latt validate --profile p.yaml --adapter md --target /repo`
- **THEN** latt resolves the md native adapter and ingests its document

#### Scenario: Explicit external adapter unchanged
- **WHEN** the user runs `latt validate --profile p.yaml --adapter ./adapters/md --target /repo`
- **THEN** lattice runs the external program as before

#### Scenario: Profile-declared native adapter fallback
- **WHEN** the user runs `latt validate --profile p.yaml --target /repo` without `--adapter`,
  and the profile declares `adapter: { native: toml }`
- **THEN** latt resolves the toml native adapter

#### Scenario: Neither adapter nor profile native
- **WHEN** the user omits `--adapter` and the profile has no `adapter.native` key
- **THEN** lattice exits 2 with a message naming `--adapter` and `adapter.native`

#### Scenario: External adapter installed only on PATH
- **WHEN** an explicit adapter name is not a compiled native name but is executable on PATH
- **THEN** the external adapter runs even though no matching file exists in the current directory

#### Scenario: Bare name falls back to the adapter-prefixed program
- **WHEN** the user runs `--adapter entomologist` from a release install, where no
  `entomologist` is on PATH but `latt-adapter-entomologist` is
- **THEN** lattice runs `latt-adapter-entomologist` with the same profile and target

#### Scenario: A path is never retried
- **WHEN** the user runs `--adapter ./foo` and `./foo` does not exist, while
  `./latt-adapter-foo` does
- **THEN** lattice exits 2 naming `./foo` and does not run `./latt-adapter-foo`

#### Scenario: Non-UTF8 external path
- **WHEN** an explicit external executable path contains non-UTF8 bytes on a supported platform
- **THEN** spawning preserves that path rather than using a lossy display string

#### Scenario: Native diff dispatch
- **WHEN** query diff selects md explicitly or through the profile at two distinct revisions
- **THEN** both materialized revisions use native dispatch and report their actual differences
  without reading uncommitted working-tree content

#### Scenario: Fuse manifest adapter is a native name or a relative program path
- **WHEN** a fuse source declares a bare adapter value
- **THEN** a compiled-in native name selects that native adapter, and any other value
  resolves against the manifest directory as an external path, never through PATH

### Requirement: Native adapter names
With the default `native-adapters` feature enabled, the core SHALL recognise
`md`, `toml` and `prose-xref` as native adapter names. A profile's `adapter.native` SHALL
remain native-only: an unknown or unavailable name SHALL exit 2 with native
availability guidance. Explicit CLI values that are not compiled native names
remain external programs. Actual spawn failure SHALL exit 2 with the program,
the `latt-adapter-<name>` spelling when one was tried, OS failure detail, and native
availability guidance.

Without `native-adapters`, availability guidance SHALL explicitly name the disabled
feature. An explicit CLI value such as `md` MAY still resolve to an external program
on PATH in that build; profile fallback SHALL NOT resolve through PATH.

Verified by: `cargo test --test native --test dispatch`,
`cargo test --test fuse mini_fuse_native_names_match_the_adapter_programs` and
`cargo test --no-default-features --test dispatch`

#### Scenario: Unknown native adapter name
- **WHEN** the user runs `--adapter yaml` and `yaml` is not a known native adapter
  and neither `yaml` nor `adapter-yaml` can be spawned as an external program
- **THEN** lattice exits 2 naming `yaml` and `adapter-yaml` and listing the available
  native adapters

#### Scenario: Disabled native profile fallback
- **WHEN** a build without native-adapters reads a profile declaring adapter.native
- **THEN** resolution fails and the message names the disabled native-adapters feature

#### Scenario: External adapter with a native spelling in a disabled build
- **WHEN** native-adapters is disabled and an explicit --adapter md names an executable on PATH
- **THEN** the external executable runs

### Requirement: Native execution produces the same document
A native adapter run in-process SHALL produce the same JSON interface document as the
`adapters/<name>` shim produces when `--adapter ./adapters/<name>` runs it as an external
adapter with the same resolved profile and target. The shim runs `latt adapter <name>`;
the external path spawns `<program> --profile … --target …` and cannot pass a
subcommand itself. The document SHALL undergo the same schema validation, version
checking, and duplicate-ID detection as a parsed external interface document.

The built-in adapters SHALL live in `latt-core` as `latt_core::adapters::{md, toml,
prose_xref}`, behind the `native-adapters` feature, and SHALL build a
`latt_adapter::Document` like any other Rust adapter. The module SHALL be public but
`#[doc(hidden)]`: an unstable item a host may call, with no promise. Within
`latt-core`, the native dispatch in `document` SHALL be the only module that calls
into `adapters`, so a built-in adapter reaches the graph through ingest alone.

Each built-in adapter SHALL write its document's struct fields in their declared order
and its maps with keys sorted, so the bytes `latt adapter <name>` prints do not depend
on whether the embedding host enables `serde_json/preserve_order`.

Verified by: `cargo build` followed by `cargo test --test native` (trace parity,
fixture exit/count/content assertions, `native_interface_documents_match_shim_values`,
and `native_documents_are_the_same_bytes_in_every_build`, also run with
`--features serde_json/preserve_order`), and
`cargo test -p latt-core --locked --test adapters_boundary`.

#### Scenario: A core module calls an adapter directly
- **WHEN** a `latt-core` source file outside `src/document.rs` and `src/adapters/` names
  `adapters::`
- **THEN** the boundary test fails naming the file and line

#### Scenario: Native md parity
- **WHEN** the md native adapter runs on a fixture, and `--adapter ./adapters/md` runs on
  the same fixture with the same profile
- **THEN** both produce equivalent interface documents and byte-identical JSON trace output

#### Scenario: Native toml parity
- **WHEN** the toml native adapter runs on a fixture, and `--adapter ./adapters/toml` runs
  on the same fixture with the same profile
- **THEN** both produce equivalent interface documents and byte-identical JSON trace output

#### Scenario: Native prose-xref parity
- **WHEN** the prose-xref native adapter runs on a fixture, and
  `--adapter ./adapters/prose-xref` runs on the same fixture with the same profile
- **THEN** both produce equivalent interface documents and byte-identical JSON trace output

#### Scenario: A host enables preserve_order
- **WHEN** `latt-core` is built with `serde_json/preserve_order` and each built-in adapter
  writes its document over its fixture
- **THEN** the bytes equal the committed copy the default build writes

### Requirement: Native adapter failures are exit 2
A native adapter that fails to produce a document — profile unreadable by the
adapter or serialization failure — SHALL cause exit 2 with a diagnostic
message, the same as an external adapter that exits non-zero.

The adapter reads a scratch copy of the resolved profile, which is not a file the user
wrote. A profile-load failure reported through the CLI SHALL name the user's `--profile`
path and the adapter, keeping the adapter's detail (serde's field and position) as is:
`could not load profile <path> for adapter <name>: <detail>`. The scratch path SHALL
NOT appear in the message. Through the library, where no profile path is known, the
message names "the resolved profile" in its place. An external adapter that exits
non-zero gets the same substitution in the stderr the core reports: every occurrence
of the scratch path becomes the user's `--profile` path, or "the resolved profile".

A missing or unreadable target is not an adapter failure: per the adapter
contract, the adapter reports it as a `PARSE_ERROR` finding inside a valid
document. The exit code depends on the finding's severity, not on the adapter
dispatch path.

Verified by: `cargo test --test native native_configuration_errors`,
`cargo test --test native native_profile_error_names_the_users_profile`,
`cargo test --test dispatch external_profile_error_names_the_users_profile`,
`cargo test --test native native_unreadable_target`, and
`.venv/bin/python -m pytest tests/test_adapter_md.py tests/test_adapter_toml.py`.

#### Scenario: Native adapter profile error
- **WHEN** `--adapter md` runs with a resolved profile the adapter cannot parse
- **THEN** lattice exits 2 with a diagnostic message

#### Scenario: Profile error names the user's profile
- **WHEN** `latt validate --profile profiles/x.yaml` runs an md profile whose `table`
  lacks `id_column`
- **THEN** lattice exits 2 with `could not load profile profiles/x.yaml for adapter md:
  missing field `id_column` …`, and no `lattice-profile-` scratch path appears

#### Scenario: External profile error names the user's profile
- **WHEN** an external adapter exits non-zero with stderr naming the scratch profile path
- **THEN** lattice exits 2, the message names the user's `--profile` path in its place,
  and no `lattice-profile-` scratch path appears

#### Scenario: Unreadable native target
- **WHEN** md or toml receives a missing target with otherwise valid adapter configuration
- **THEN** it produces a valid document containing a PARSE_ERROR finding, and trace exits 1
  under the default severity rather than reporting execution failure

### Requirement: Adapter shims run from an install
`adapters/md`, `adapters/prose-xref` and `adapters/toml` SHALL be copies of one native-shim
template, differing only in the adapter name. A native shim SHALL run the first of `latt`
on PATH, `$CARGO_TARGET_DIR/debug/latt` and `<shim dir>/../target/debug/latt` that exists,
as `latt adapter <name>` followed by its own arguments unchanged.

`adapters/entomologist` SHALL run the first of `latt-adapter-entomologist` on PATH,
`$CARGO_TARGET_DIR/debug/latt-adapter-entomologist` and
`<shim dir>/../target/debug/latt-adapter-entomologist` that exists, passing its arguments
through unchanged.

When a shim finds no program, it SHALL exit 127 with a message on stderr naming the
program and all three places, and write nothing to stdout. A release install without its
binaries then never reads as an empty register.

Verified by: `.venv/bin/python -m pytest tests/test_shims.py`, which runs each shim
from a copy of `adapters/` with stub binaries and no `target/` tree.

#### Scenario: Install with the binaries on PATH
- **WHEN** `adapters/entomologist` runs with `latt-adapter-entomologist` on PATH and no
  `target/` tree
- **THEN** `latt-adapter-entomologist` runs with the shim's `--profile` and `--target` arguments

#### Scenario: Native adapter shim finds the install
- **WHEN** `adapters/md` runs with `latt` on PATH and no `target/` tree
- **THEN** `latt adapter md` runs with the shim's arguments

#### Scenario: Checkout without an install
- **WHEN** `adapters/entomologist` runs with no `latt-adapter-entomologist` on PATH and
  `../target/debug/latt-adapter-entomologist` built
- **THEN** that binary runs

#### Scenario: Neither
- **WHEN** `adapters/md` runs with no `latt` on PATH, no `$CARGO_TARGET_DIR`, and no
  `../target/debug/latt`
- **THEN** the shim exits 127, writes nothing to stdout, and names the three places

