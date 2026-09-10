# Fuse: multi-source composition

Status: implemented as `lattice fuse` (core subcommand). The normative
contract is `openspec/specs/program-composition/`.

## Architecture

Fuse layers cross-source validation over existing single-source lattice runs.
The interface is:

```sh
lattice fuse --manifest fuse.yaml [--format plain|json|rich] [--strict]
```

The subcommand runs `lattice trace --format json` per source, ingests the
traces into one graph with source-qualified IDs and kinds, runs the standard
validators on the composed graph, and outputs the fuse report through the
standard tri-format dispatcher.

## Manifest

The YAML manifest declares fuse identity, a fuse profile, and an ordered list
of sources:

```yaml
manifest_version: "1.0.0"
name: example-fuse
version: "1.0.0"
fuse_profile: fuse-profile.yaml
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

`manifest_version`, `name`, `version`, `fuse_profile`, and a non-empty
`sources` list are required. Each source has a unique `name` (must not contain
`:`) plus its `profile`, `adapter`, and `target`. Paths are relative to the
manifest.

The fuse profile declares `edge_kinds`. Each edge kind contains `allowed` pairs
of source-qualified kinds using colon syntax: `[compliance:clause,
product:requirement]`. It may also declare fuse-level `validations`; source
node kinds remain owned by their source profiles.

## Qualification

Source qualification uses the colon separator: a node with raw ID `REQ-1` in
source `product` becomes `product:REQ-1`; its kind `requirement` becomes
`product:requirement`. Pathways are qualified the same way:
`product:stage_name`. The colon distinguishes source qualification from the
`file_stem/raw_id` pattern used by `id_prefix` in source profiles.

## Validation pipeline

Validation runs in three phases:

1. **Source runs.** Fuse invokes `lattice trace --format json` for every source,
   preserving its source-local findings with source attribution.
2. **Merge.** Healthy trace payloads are combined in manifest order. Node IDs
   and kinds are source-qualified with `:`. Cross-source duplicate raw IDs are
   reported as `CROSS_SOURCE_DUPLICATE_ID`.
3. **Cross-source resolution.** Edges declared by the fuse profile are resolved
   using their allowed source-qualified endpoint pairs. The standard validators
   run on the composed graph. Missing or ambiguous targets and fuse-level
   validations produce findings in the report.

`--strict` promotes warnings to errors after collection, consistent with
single-source runs.

## Exit codes

Fuse follows lattice's three-valued convention:

- `0`: no error-severity findings in any phase.
- `1`: error-severity findings in source or cross-source validation.
- `2`: could not run: bad manifest, bad fuse profile, adapter failure, or
  unparseable trace output.


