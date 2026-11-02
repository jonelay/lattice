# Fuse

`latt fuse` combines registers from different repos into one graph, so you can check
the references between them. Each source keeps its own profile and adapter. You add two
files:

- a **manifest** that lists the sources;
- a **fuse profile** that declares the edges allowed between them.

Only `fuse` reads a manifest. `validate`, `trace`, `summary`, `coverage` and `query`
each take one profile and see one source. To answer a cross-source question, filter the
`nodes[]` and `edges[]` of `fuse --format json` ([json.md](json.md#fuse)) by
source-qualified ID.

## Manifest

```yaml
manifest_version: "1.0.0"
name: my-project
version: "1.0.0"
fuse_profile: fuse-profile.yaml

sources:
  - name: reqs
    profile: ../requirements/profiles/md.yaml
    adapter: ../requirements/adapters/md
    target: ../requirements
  - name: specs
    profile: ../specs/profiles/openspec.yaml
    adapter: ../specs/adapters/openspec
    target: ../specs
  - name: tests
    profile: ../tests/profiles/md.yaml
    adapter: md
    target: ../tests
```

| Key | Holds |
|---|---|
| `manifest_version` | manifest format version, X.Y.Z; major `1` is read, any minor or patch |
| `name`, `version` | composed-graph identity, echoed in the report header |
| `fuse_profile` | path to the fuse profile |
| `sources` | ordered, non-empty list; each entry is one `validate`-shaped run |

Each source has four keys:

| Source key | Holds |
|---|---|
| `name` | unique among the sources; must not contain `:` |
| `profile` | the source's profile |
| `adapter` | a native adapter name (`md`, `toml`, `prose-xref`), run in process as `--adapter` does; any other value is a program path |
| `target` | the directory the source's adapter reads |

All paths are relative to the manifest file. A program path must be relative, and Lattice
does not look it up on PATH:

- `./md` names a program beside the manifest.
- An installed `latt-adapter-entomologist` is named by its path from the manifest
  (`../latt/bin/latt-adapter-entomologist`).
- A bare `entomologist` names a file beside the manifest.
- An absolute path is refused.

## Fuse profile

```yaml
name: my-project
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

| Key | Rule |
|---|---|
| `edge_kinds` | required; every endpoint is `source:kind` |
| `validations` | optional: severity overrides, `COVERAGE`/`COVERAGE_DEEP` blocks with `where:` and `source_where:`, and `SUPPRESS` entries |
| `node_kinds` | rejected; node kinds belong to source profiles |
| `annotations` | rejected; notes belong to source profiles, and fuse keeps the note each source's profile attached to its findings |

An endpoint must name a source the manifest lists and a kind that source's profile
declares. Fuse checks this before any source runs and exits 2 on a bad endpoint:

```text
ERROR FUSE_ERROR [] fuse.yaml:0 fuse profile edge kind 'cites' names 'ghost:issue', but the manifest has no source 'ghost' (sources: reqs)
```

A `COVERAGE` whose `edge_kind` is missing from `edge_kinds` produces `CONFIG_ERROR`;
valid entries beside it still run. A standard profile refuses to load on this fault. A
fuse profile reports it as a finding, because the composed kinds are known only once
the sources have run.

### Where edges come from

A fuse profile declares which edges may cross sources and judges them. It creates no
edges. Every edge comes from a source adapter, so a cross-source edge exists only when
that adapter emits it. A fuse edge kind no source emits is accepted and stays empty.

For example, the TOML adapter emits an edge only from a key the source profile maps
under `edge_keys` ([adapters.md](adapters.md#toml-adapter)). A row field that names
another source's IDs without that mapping cites nothing.

### Intra-source pairs

A kind declared in the fuse profile matches only its `allowed` pairs. Kinds the fuse
profile leaves out still resolve inside their own source. If you declare a kind that
also links within one source, add that pairing (e.g. `[tests:test, tests:test]`), or
those edges become `VACANCY`.

### Stub kinds in a source profile

A source profile's `allowed` pairs must name kinds that profile declares. An edge whose
target lives only in another source therefore needs a stub kind: a `node_kinds` entry
no row takes, named as the pair's target.

| Where | Pair | Result |
|---|---|---|
| source profile | `[test, requirement]`, with `requirement` declared and empty | the source run reports the edges as `VACANCY` hints (with `cross_source: true`) |
| fuse profile | `[tests:test, reqs:requirement]` | names the real kind and resolves the edges |

The md adapter's single-table form takes exactly one kind, so an md source declares its
stub through the multi-table `adapter.tables` form
([adapters.md](adapters.md#markdown-table-adapter)).

### Suppression

| Entries in | Apply to |
|---|---|
| source profile `SUPPRESS` | findings inside that source's trace run; they arrive at the merge already marked. `UNREFERENCED` and `UNTRACED` entries also apply to the composed graph's recomputed findings ([Pipeline](#pipeline)) |
| fuse profile `SUPPRESS` | cross-source and composed-graph findings; `node_ids` name composed IDs (`source:id`) |

Stale fuse-profile entries report as `SUPPRESS_UNUSED` against `<fuse-profile>`.

## Pipeline

| Step | What happens |
|---|---|
| 1. Source runs | `latt trace --format json` per source, in manifest order |
| 2. Qualification | every ID, kind and pathway gets its source prefix |
| 3. Cross-source resolution | edges of fuse-declared kinds are matched across sources |
| 4. Validation | standard validators run on the composed graph |

### 1. Source runs

Exit 0 or 1 is usable. These are `SOURCE_FAILURE`:

- exit 2, a crash, or unparseable output;
- a finding or location `line` that is negative or above 4294967295 (counts as
  unparseable);
- a source profile that cannot be loaded when composition reads its ID patterns.

After a failure, fuse finishes the remaining sources, then exits 2 without building a
graph. The report is still written, in the format asked, with the failure among its
findings.

### 2. Qualification

`REQ-014` from source `reqs` becomes `reqs:REQ-014`, of kind `reqs:requirement`.
Pathway `stage` from `specs` becomes `specs:stage`. The un-prefixed form is the **raw
ID**. A raw ID in more than one source produces `CROSS_SOURCE_DUPLICATE_ID`.

### 3. Cross-source resolution

For each edge whose kind the fuse profile declares, fuse looks for a node of an allowed
target kind with a matching raw ID:

| Matches | Result |
|---|---|
| one | the edge resolves |
| none | `VACANCY` |
| several | `AMBIGUOUS_CROSS_REF` |

A source's own run reports an edge into another source as `VACANCY`. Once fuse judges
the edge, it withdraws that source finding. A resolved edge leaves no finding; an
unresolved one leaves fuse's own `VACANCY` as the single report. A source `VACANCY` on a
kind the fuse profile does not declare stays, since fuse never judged it.

Mark cross-source edge kinds `cross_source: true` in the source profile
([profiles.md](profiles.md)). A source run on its own then reports them as hints
instead of errors.

### 4. Validation

Composed IDs keep their source's `id_pattern`. A kind named only in an edge declaration,
and never seen in a trace, accepts any ID and is `orphan_ok`.

`UNREFERENCED` and `UNTRACED` are recomputed here. Fuse withdraws every source's own
orphan findings, and the composed graph answers the orphan question instead:

- A resolved cross-source edge counts on its target.
- Each composed kind keeps its source profile's `orphan_ok`.
- A direction is checked when a source profile's own `allowed` pairs or the fuse
  profile's permit it ([profiles.md](profiles.md#node-kinds)). Edges that happened to
  arrive do not decide it.
- An edge fuse left unresolved counts as the source's own run counted it: it leaves its
  node, and reaches a same-source node its raw target names.
- Severity is the fuse profile's override for the code if it has one, else the source
  profile's.
- The finding is suppressed when the source profile's `SUPPRESS` entries would suppress
  it in the source's own run, matching `node_ids` against the raw ID (without
  `source:`). This holds even for a node the source run did not report: a self-named
  edge the source resolved to its own node, and fuse resolved to another source, makes
  an orphan only the composed graph sees.
- A source finding that arrived suppressed stays suppressed.

Every other source finding (`PARSE_ERROR`, `ID_FORMAT`, attribute checks, a `VACANCY`
fuse never judged) arrives as the source reported it.

## Output

```sh
latt fuse --manifest fuse.yaml
latt fuse --manifest fuse.yaml --format json
latt fuse --manifest fuse.yaml --strict
```

`--format`, `--strict` and the exit codes work as on the other commands
([cli.md](cli.md)). A source that cannot run is the exit-2 case
([Pipeline](#pipeline)).

**Plain** prints a header line, then nodes, edges, pathways and findings. Unresolved
edges print `(unresolved)`. Each finding carries its sources in brackets:

| Finding | Prints |
|---|---|
| a source's own finding | `[source]` |
| fuse's `VACANCY` for an edge it could not resolve | `[source]`, the source the edge belongs to |
| a finding on several sources | `[src, dst]` |
| a fuse-profile finding with no source, such as `COVERAGE_UNKNOWN` or `SUPPRESS_UNUSED` | `[]` |

A source finding's note ends its line as `| note: …`; rich prints it on the line below.

**JSON** has `header`, `nodes`, `edges`, `pathways` and `findings`:

- The header carries `name`, `version`, `manifest_version` and `fuse_version`, the
  report envelope's version (`"1"`).
- Resolved edges carry `target_kind` and `target_source`.
- A finding with several locations lists them under `locations`.
- A source finding with a note carries it as a `note` object.

## Troubleshooting

| Symptom | Cause |
|---|---|
| `source names must not contain ':'` | rename the source |
| `unsupported manifest version` / `is not a valid version` | `manifest_version` must be X.Y.Z with major `1` |
| `unknown field` | a manifest or source key is misspelled or not part of the format |
| `'profile' must be a non-empty relative path` | paths resolve from the manifest directory |
| `fuse profile must not declare 'node_kinds'` | move them to the source profile |
| `fuse profile must not declare 'annotations'` | name the notes file in the source profile |
| `expected a source-qualified kind` | every `allowed` endpoint needs `source:` prefix |
| `FUSE_ERROR` | the manifest, or a path in it, did not load; the message names which |
| `SOURCE_FAILURE` | run that source's `latt validate` alone |
| `VACANCY` on edges that resolved single-source | the kind is declared in the fuse profile but intra-source pairing is missing from `allowed` |
| `VACANCY` errors from a source run for edges into another source | mark the edge kind `cross_source: true` in that source profile |
| `AMBIGUOUS_CROSS_REF` | raw ID exists in two allowed target kinds; narrow the pairing |
| `COVERAGE` never fires | `edge_kind` is undeclared; look for `CONFIG_ERROR` |
