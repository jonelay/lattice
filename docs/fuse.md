# Fuse

`lattice fuse` composes several single-source registers into one graph and validates
the references that cross between them. Each source keeps its own profile and adapter;
fuse adds a manifest naming the sources and a fuse profile declaring the cross-source
edges.

`openspec/specs/program-composition/spec.md` is the normative contract.

## Manifest

```yaml
manifest_version: "1.0.0"
name: phase-sweep
version: "1.0.0"
fuse_profile: fuse-profile.yaml

sources:
  - name: reqs
    profile: ../requirements/profiles/md.yaml
    adapter: ../requirements/adapters/md
    target: ../requirements
  - name: specs
    profile: ../phase-sweep/profiles/openspec.yaml
    adapter: ../phase-sweep/adapters/openspec
    target: ../phase-sweep
```

| Key | Holds |
|---|---|
| `manifest_version` | schema version; `"1.0.0"` |
| `name`, `version` | composed-graph identity, echoed in the report header |
| `fuse_profile` | path to the fuse profile |
| `sources` | ordered, non-empty list; each is one `validate`-shaped run |

Each source declares `name`, `profile`, `adapter`, `target`. All paths are relative to
the manifest file. Source names must be unique and must not contain `:`.

## Fuse profile

```yaml
name: phase-sweep
profile_version: "1.0.0"

edge_kinds:
  satisfies:
    allowed:
      - [specs:spec, reqs:requirement]
  verifies:
    allowed:
      - [tests:test, reqs:requirement]
      - [tests:test, specs:spec]

validations:
  - VACANCY:
      severity: error
  - COVERAGE:
      target_kind: reqs:requirement
      edge_kind: verifies
      severity: warning
      where:
        status: { in: [approved, baselined] }
```

`edge_kinds` is required. Every endpoint is `source:kind`. `node_kinds` is rejected —
node kinds belong to source profiles.

`validations` is optional: severity overrides, `COVERAGE`/`COVERAGE_DEEP` blocks with
`where:`, and `SUPPRESS` entries. A `COVERAGE` whose `edge_kind` is not declared under
`edge_kinds` produces `CONFIG_ERROR`.

Declaring an edge kind here changes how it resolves: undeclared kinds resolve inside
their source as before; declared kinds resolve only against their `allowed` pairs. Add
intra-source pairings (e.g. `[tests:test, tests:test]`) or those edges become `VACANCY`.

### Suppression

Source profile `SUPPRESS` entries apply inside that source's trace run — findings arrive
at the merge already marked. Fuse profile entries apply to cross-source and composed-graph
findings. `node_ids` here name composed IDs (`source:id`). Stale entries report as
`SUPPRESS_UNUSED` against `<fuse-profile>`.

## Pipeline

**1. Source runs.** `lattice trace --format json` per source, in manifest order. Exit 0
or 1 is usable. Exit 2, crash, or unparseable output is `SOURCE_FAILURE` — fuse finishes
the remaining sources, then exits 2 without building a graph.

**2. Qualification.** `REQ-014` from source `reqs` becomes `reqs:REQ-014` of kind
`reqs:requirement`; pathway `stage` from `specs` becomes `specs:stage`. A raw ID in more
than one source produces `CROSS_SOURCE_DUPLICATE_ID`.

**3. Cross-source resolution.** For each edge whose kind the fuse profile declares, fuse
finds a node whose qualified kind is an allowed target and whose raw ID matches. One
match resolves the edge. No match: `VACANCY`. Multiple matches: `AMBIGUOUS_CROSS_REF`.

**4. Validation.** Standard validators run on the composed graph. Composed IDs keep their
source's `id_pattern`. A kind named only in an edge declaration (never seen in a trace)
accepts any ID.

## Output

```sh
lattice fuse --manifest fuse.yaml
lattice fuse --manifest fuse.yaml --format json
lattice fuse --manifest fuse.yaml --strict
```

`--format plain|json|rich` (default: rich on TTY). `--strict` promotes warnings to errors.

Plain: header line, then nodes, edges, pathways, findings. Unresolved edges print
`(unresolved)`. Findings carry `[source]`; fuse-level findings print `[]`.

JSON: `header`, `nodes`, `edges`, `pathways`, `findings`. Resolved edges carry
`target_kind` and `target_source`. Multi-location findings list each under `locations`.

Exit codes: 0 clean, 1 error-severity findings, 2 could not run.

## Troubleshooting

| Symptom | Cause |
|---|---|
| `source names must not contain ':'` | rename the source |
| `'profile' must be a non-empty relative path` | paths resolve from the manifest directory |
| `fuse profile must not declare 'node_kinds'` | move them to the source profile |
| `expected a source-qualified kind` | every `allowed` endpoint needs `source:` prefix |
| `SOURCE_FAILURE` | run that source's `lattice validate` alone |
| `VACANCY` on edges that resolved single-source | the kind is declared in the fuse profile but intra-source pairing is missing from `allowed` |
| `AMBIGUOUS_CROSS_REF` | raw ID exists in two allowed target kinds; narrow the pairing |
| `COVERAGE` never fires | `edge_kind` is undeclared — look for `CONFIG_ERROR` |

A working fixture lives in `tests/fixtures/mini-fuse/`.
