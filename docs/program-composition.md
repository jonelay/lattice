# Program composition

Status: implemented as the `tools/lattice-compose` Python shim. The originating
OpenSpec change is archived; `openspec/specs/program-composition/` is the
normative contract.

## Architecture

Program composition layers cross-source validation over existing single-source
Lattice runs. The interface is:

```sh
tools/lattice-compose <manifest.yaml>
```

The shim writes one JSON document to stdout. It leaves adapters and the native
core single-source model unchanged.

## Manifest

The YAML manifest declares program identity, a program profile, and an ordered
list of sources:

```yaml
manifest_version: "1.0.0"
program: example-program
program_version: "1.0.0"
program_profile: program-profile.yaml
sources:
  - name: product
    profile: profiles/product.yaml
    adapter: adapters/product
    target: repos/product
  - name: compliance
    profile: profiles/compliance.yaml
    adapter: adapters/compliance
    target: repos/compliance
```

`manifest_version`, `program`, `program_version`, `program_profile`, and a
non-empty `sources` list are required. Each source has a unique `name` plus its
`profile`, `adapter`, and `target`. Paths are relative to the manifest.

The program profile declares `edge_kinds`. Each edge kind contains `allowed`
pairs of source-qualified kinds such as
`[compliance/clause, product/requirement]`. It may also declare program-level
`validations`; source node kinds remain owned by their source profiles.

## Validation pipeline

Validation runs in three phases:

1. **Source runs.** The shim invokes `lattice trace --format json` for every
   source, preserving its source-local findings.
2. **Merge.** Healthy trace payloads are combined in manifest order. Node IDs
   remain unchanged, while kinds are qualified as `<source>/<kind>` and
   cross-source duplicate IDs are reported.
3. **Cross-source resolution.** Edges declared by the program profile are
   resolved using their allowed source-qualified endpoint pairs. Missing or
   ambiguous targets and optional program-level validations produce findings in
   the composed output.

The output includes merged nodes, edges, findings from all three phases, and an
empty `axes` collection.

## Exit codes

The shim follows Lattice conventions:

- `0`: the program ran with no error-severity findings.
- `1`: source or program validation produced an error-severity finding.
- `2`: the program could not run, such as from an invalid manifest or program
  profile, an unusable source result, or an adapter failure.

## Deferred work

- forwarding strict-mode behavior to source runs
- merging axes across sources
- native composition integration in the Rust core and CLI
