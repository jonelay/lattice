# JSON output

`--format json` is the output for scripts and agents. Its shape is a public surface: a
patch release never changes it; below 1.0 a minor release may, with the change listed
under Breaking in `CHANGELOG.md`. The tests fail when the binary and this page disagree.

For example, `latt validate --profile profiles/toml.yaml --adapter toml --target tests/fixtures/mini-toml --format json`
run from a checkout of this repository writes this, trimmed to two findings:

```json
{
  "findings": [
    {
      "code": "UNREFERENCED",
      "file": "registers/alpha.toml",
      "line": 19,
      "message": "node 'alpha/two' has no incoming edges",
      "node_id": "alpha/two",
      "severity": "info"
    },
    {
      "code": "PARSE_ERROR",
      "file": "registers/baditem.toml",
      "line": 13,
      "message": "registers/baditem.toml: item row 1 declares no string 'id' ID key",
      "node_id": null,
      "severity": "error"
    }
  ]
}
```

## Reading the key lists

| Notation | Means |
|----------|-------|
| `a.b` | key `b` inside object `a` |
| `a[]` | the elements of array `a` |
| trailing `?` | present only in some runs |
| `<kind>`, `<attr>`, `<status>`, `<group>` | a key the profile or register supplies |
| `<*>` | any key and everything beneath it |

- Every `line` is an integer from 0 to 4294967295.
- Map-shaped objects sort their keys; arrays keep the plain output's order.
- A finding's keys mean the same in every command; `validate` describes them once.
- The answer is one pretty-printed JSON object on stdout. Stderr can mix adapter findings,
  labels and plain text ([below](#adapter-findings-on-stderr)).
- Exit codes do not change with the format ([cli.md](cli.md#exit-codes)).

## validate

```text
findings[]              in output order
findings[].severity     error | warning | info | hint, after --strict
findings[].code         see profiles.md#finding-codes
findings[].file         target-relative path, or a reserved name (below)
findings[].line         1-based; 0 when unknown
findings[].message      the plain-format text
findings[].node_id      null when none
findings[].state?       a qualifier such as unknown
findings[].suppressed?  true when a SUPPRESS entry matched
findings[].note?        the profile's note, when one is attached
findings[].note.text
findings[].note.at?     date written
findings[].note.by?     author
findings[].note.until?  a date string, or {pathway, position}
findings[].note.until.pathway?
findings[].note.until.position?
findings[].note.expired? true once until has passed
scope                   what the graph held
scope.nodes.<kind>      per declared node kind; 0 when none
scope.edges.<kind>      per declared edge kind; 0 when none
scope.files             distinct files that declared a node or edge
```

A finding about a whole input sits at line 0 under a reserved `file` name:

| `file` | The input |
|--------|-----------|
| `<profile>` | the profile's configuration |
| `<fuse-profile>` | a fuse profile |
| `<source>` | a fuse source that could not run |
| `<suggestions>` | a suggestion document |

Fuse reads these back from a source's trace; a Rust host sees them as `Origin` variants
([embedding.md](embedding.md)). Finding codes and default severities:
[profiles.md](profiles.md#finding-codes).

- With no findings the output is `{"findings": [], "scope": {…}}`.
- `--suggestions` adds `SUGGESTED_EDGE` findings, and one `SUGGESTION_UNRANKED` for a
  positive `unranked` count; no new keys.
- `scope` counts only kinds the profile declares; an undeclared kind is an
  `UNKNOWN_KIND` finding.
- `scope.files` leaves out a matched file that held no row. `NO_MATCHING_FILES` and
  `EMPTY_KIND` report when nothing was read.

## trace

```text
header
header.profile
header.profile_version  the register's vocabulary version
header.lattice_version  the binary's version
header.trace_version    the report shape version
entries[]               one per node
entries[].id
entries[].kind
entries[].attrs         an md cell is always a string, "" when empty
entries[].attrs.<attr>
entries[].provenance.file
entries[].provenance.line
entries[].edges[]       outgoing edges
entries[].edges[].kind
entries[].edges[].tgt   declared or not
entries[].edges[].attrs usually empty
entries[].edges[].provenance.file
entries[].edges[].provenance.line
entries[].findings[]    findings on this node, without node_id
entries[].findings[].severity
entries[].findings[].code
entries[].findings[].file
entries[].findings[].line
entries[].findings[].message
entries[].findings[].state?
entries[].findings[].suppressed?
entries[].findings[].note?
entries[].findings[].note.text
entries[].findings[].note.at?
entries[].findings[].note.by?
entries[].findings[].note.until?
entries[].findings[].note.until.pathway?
entries[].findings[].note.until.position?
entries[].findings[].note.expired?
pathways[]
pathways[].name
pathways[].order[]      positions, in order
pathways[].current      where the register stands
unattachable_findings[] findings naming no node, or an undeclared one
unattachable_findings[].severity
unattachable_findings[].code
unattachable_findings[].file
unattachable_findings[].line
unattachable_findings[].message
unattachable_findings[].state?
unattachable_findings[].suppressed?
unattachable_findings[].note?
unattachable_findings[].note.text
unattachable_findings[].note.at?
unattachable_findings[].note.by?
unattachable_findings[].note.until?
unattachable_findings[].note.until.pathway?
unattachable_findings[].note.until.position?
unattachable_findings[].note.expired?
```

## summary

Without a `SUMMARY` block, structural counts:

```text
node_counts             per declared node kind, zero included
node_counts.<kind>
edge_counts             per declared edge kind
edge_counts.<kind>
finding_counts[]        per (code, severity) seen; empty when clean
finding_counts[].code
finding_counts[].severity
finding_counts[].count
```

## summary with a SUMMARY block

With `SUMMARY` configured ([profiles.md](profiles.md#validations)), the status table:

```text
groups[]                one per group_by_attr value
groups[].<group>        key is the group_by_attr name; value is the group
groups[].<status>       count per status_attr value
groups[].total
totals                  summed over every group
totals.<status>
totals.total
```

The group key is the attr's name (`file` for `group_by_attr: file`), so read the profile
to know it. A status named `total` is refused at profile load.

## coverage

```text
kinds[]                 one per declared node kind
kinds[].kind
kinds[].total
kinds[].incoming        nodes with an incoming edge of any kind
kinds[].incoming_pct    incoming / total × 100; 0.0 when total is 0
kinds[].outgoing        nodes with an outgoing edge of any kind
kinds[].outgoing_pct
```

Any edge counts, structural `contains`-style edges included. For one edge kind, use a
`COVERAGE` validation.

## describe

```text
profile.name
profile.version
adapter.name            native adapter name, or the external program as named
adapter.config          the adapter: block as `latt show` prints it; null when absent
adapter.config.<*>?     defined by the adapter (adapters.md)
node_kinds[]            in name order
node_kinds[].kind
node_kinds[].id_pattern as written
node_kinds[].count
node_kinds[].files[]    distinct source files, in path order
edge_kinds[]            in name order
edge_kinds[].kind
edge_kinds[].allowed[]  [source, target] kind pairs
edge_kinds[].allowed[][]
edge_kinds[].count
validations[]           one per validations: entry, by code
validations[].code
validations[].severity? only when the entry sets one
findings[]              the adapter's own findings
findings[].severity
findings[].code
findings[].file
findings[].line
findings[].message
findings[].node_id
findings[].state?
findings[].suppressed?
findings[].note?
findings[].note.text
findings[].note.at?
findings[].note.by?
findings[].note.until?
findings[].note.until.pathway?
findings[].note.until.position?
findings[].note.expired?
```

`describe` validates nothing: it exits 0 whenever it ran, whatever the findings, and 2
when it could not run.

## query reaches, query reached-by

```text
origin
direction               reaches | reached-by
edge_kinds[]            --edge-kind filters; empty for all
nodes[]                 every node reached, origin excluded
nodes[].id
nodes[].kind
nodes[].tainted?        with --check-resolved: true when every path passes an undeclared endpoint
```

## query path

```text
src
tgt
found
nodes[]                 IDs along one shortest path, src first; empty when not found
edges[]                 edge kinds between them
```

## query orphans

```text
kind_filter             the --kind given, or null
orphans[]               nodes with no edge in or out
orphans[].id
orphans[].kind
orphans[].file
orphans[].line
```

## query counts

```text
nodes                   per declared node kind, zero included
nodes.<kind>
edges                   per declared edge kind
edges.<kind>
```

## query at

```text
path                    target-relative
entries[]               trace entries from that path
entries[].id
entries[].kind
entries[].attrs
entries[].attrs.<attr>
entries[].provenance.file
entries[].provenance.line
entries[].edges[]
entries[].edges[].kind
entries[].edges[].tgt
entries[].edges[].attrs
entries[].edges[].provenance.file
entries[].edges[].provenance.line
entries[].findings[]
entries[].findings[].severity
entries[].findings[].code
entries[].findings[].file
entries[].findings[].line
entries[].findings[].message
entries[].findings[].state?
entries[].findings[].suppressed?
entries[].findings[].note?
entries[].findings[].note.text
entries[].findings[].note.at?
entries[].findings[].note.by?
entries[].findings[].note.until?
entries[].findings[].note.until.pathway?
entries[].findings[].note.until.position?
entries[].findings[].note.expired?
findings[]              findings at that path outside every entry; no node_id
findings[].severity
findings[].code
findings[].file
findings[].line
findings[].message
findings[].state?
findings[].suppressed?
findings[].note?
findings[].note.text
findings[].note.at?
findings[].note.by?
findings[].note.until?
findings[].note.until.pathway?
findings[].note.until.position?
findings[].note.expired?
```

`findings[]` holds the findings at the path that attach to a node declared elsewhere, or
to none.

## query diff

```text
rev_a
rev_b
nodes_added[]
nodes_added[].id
nodes_added[].kind
nodes_removed[]
nodes_removed[].id
nodes_removed[].kind
nodes_changed[]         same ID, different kind or attrs
nodes_changed[].id
nodes_changed[].kind    as at rev_b
nodes_changed[].attrs_a
nodes_changed[].attrs_a.<attr>
nodes_changed[].attrs_b
nodes_changed[].attrs_b.<attr>
edges_added[]
edges_added[].src
edges_added[].tgt
edges_added[].kind
edges_removed[]
edges_removed[].src
edges_removed[].tgt
edges_removed[].kind
edges_changed[]         same endpoints and kind, different attrs
edges_changed[].src
edges_changed[].tgt
edges_changed[].kind
edges_changed[].attrs_a
edges_changed[].attrs_a.<attr>
edges_changed[].attrs_b
edges_changed[].attrs_b.<attr>
pathways_changed[]      names of pathways added, removed, or with a new order or current
```

Provenance is ignored: a node or edge that only moved is unchanged.

## fuse

The composed graph ([fuse.md](fuse.md)), and the only cross-source graph Lattice writes.
`trace`, `query` and `coverage` take one profile, so for a per-node answer across sources,
filter `nodes[]` and `edges[]` here.

- Node IDs and edge endpoints are `source:id`.
- A node's `kind` and an edge's `source_kind` and `target_kind` are `source:kind`; an
  edge's own `kind` stays unqualified.
- Unlike `validate`, fuse writes this object at exit 2 too: the graph is empty and the
  `FUSE_ERROR` or `SOURCE_FAILURE` that stopped it is among the findings.

```text
header.name             the manifest's name
header.version          the manifest's version
header.manifest_version manifest format version
header.fuse_version     report envelope version
nodes[]
nodes[].id
nodes[].kind
nodes[].attrs
nodes[].attrs.<attr>
nodes[].provenance.file
nodes[].provenance.line
nodes[].provenance.source
edges[]
edges[].src
edges[].tgt             qualified by the source fuse resolved it in, else the edge's own
edges[].kind
edges[].source
edges[].source_kind     kind of src
edges[].target_kind?    kind of tgt; absent when unresolved
edges[].target_source?  source of tgt; absent when unresolved
edges[].attrs
edges[].attrs.<attr>
edges[].provenance.file
edges[].provenance.line
edges[].provenance.source
pathways[]              source-qualified names
pathways[].name
pathways[].order[]
pathways[].current
findings[]
findings[].severity
findings[].code
findings[].file
findings[].line
findings[].message
findings[].node_id
findings[].source?      absent on fuse-profile-level and multi-location findings
findings[].sources?     every source a multi-location finding names
findings[].locations?   one per location, e.g. CROSS_SOURCE_DUPLICATE_ID
findings[].locations[].file
findings[].locations[].line
findings[].locations[].source
findings[].state?
findings[].suppressed?
findings[].note?
findings[].note.text
findings[].note.at?
findings[].note.by?
findings[].note.until?
findings[].note.until.pathway?
findings[].note.until.position?
findings[].note.expired?
```

Fuse-profile-level findings include `COVERAGE_UNKNOWN` and `SUPPRESS_UNUSED`.

## Adapter findings on stderr

`query`, `summary` and `coverage` keep the adapter's own findings (`PARSE_ERROR` and the
like) out of the answer. In `json` mode they go to stderr after the answer, as one object
in the `validate` shape:

```text
findings[]
findings[].severity
findings[].code
findings[].file
findings[].line
findings[].message
findings[].node_id
findings[].state?
findings[].suppressed?  plain and rich omit it
findings[].note?
findings[].note.text
findings[].note.at?
findings[].note.by?
findings[].note.until?
findings[].note.until.pathway?
findings[].note.until.position?
findings[].note.expired?
```

| Command | Exit code with adapter findings |
|---------|-------------------------------|
| `query`, `coverage` | 0 |
| `summary` | 1 when one is an unsuppressed error |
| any | 2 when the adapter read nothing from the register |

With no adapter findings, nothing goes to stderr. Stderr can also carry plain text, so
read it as lines:

- `query diff` runs the adapter at both revisions and writes one findings object per
  revision that has any, each after a line naming the revision (`HEAD:`).
- A `--filter` on an attr no node carries prints a `Warning:` line; the answer is unchanged.
- At exit 2 the reason is a plain-text `Error:` line.
