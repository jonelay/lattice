# Adapter configuration

Each adapter reads its configuration from the `adapter:` block in the profile. The core
passes this block through unchanged in the resolved document — it has no opinion about
its shape.

## Markdown table adapter

`crates/adapter-md/`. Single-table form — one schema for every table matched by
`adapter.paths.files`:

```yaml
adapter:
  paths:
    files: ["*.md"]
  table:
    id_column: "ID"
    column_map:
      "Description": summary
    edge_columns:
      "Traces To": traces_to
```

Multi-table form — `adapter.tables` dispatches by heading regex (first match wins,
heading text without the `##`, nearest preceding heading at any level):

```yaml
adapter:
  paths:
    files: ["**/*.md"]
  tables:
    - heading: '^Domain [0-9]+$'
      kind: requirement
      id_column: "ID"
      column_map:
        "Description": summary
        "Status": status
      edge_columns:
        "Traces To": traces_to

    - heading: '^Stakeholders$'
      kind: stakeholder
      id_column: "ID"
      column_map:
        "Name": name
        "Role": role
      edge_columns: {}
```

In multi-table mode an unmatched table produces `PARSE_ERROR`. Every `kind` named must
also be declared in `node_kinds`.

## TOML adapter

`crates/adapter-toml/`. `adapter.tables` maps TOML array-of-tables names to node kinds.
Each entry declares `kind`, `id_key`, `key_map`, and `edge_keys`. With
`id_prefix: file_stem`, IDs become `<file-stem>/<id>`.

```yaml
adapter:
  paths:
    files: ["registers/*.toml"]
  id_prefix: file_stem
  tables:
    item:
      kind: item
      id_key: id
      key_map:
        id: id
        stage: stage
      edge_keys:
        register: belongs_to
        refs: references
```

`adapter.mode: directory` treats each matched file's root table as one record (first
`tables` entry by key order describes every file). Default mode is `tables`.

Optional `adapter.header` reads one singleton table per file as a node (`table`, `kind`,
`id`, `key_map`). Set `id: file_stem` to derive the ID from the filename.

Optional `adapter.pathway` reads an ordering pathway from a file (`source_file`, `name`,
`order_key`, `current_key` as dotted paths).

See `profiles/toml.yaml` for a worked example.

## Entomologist adapter

`crates/adapter-entomologist/`. Reads a git-backed issue tracker's register from its
`entomologist-data` orphan branch via `git ls-tree` and `cat-file --batch`. No `adapter:`
config needed — the register format is fixed.

## GitHub Issues adapter

`crates/adapter-github/`. Reads issues via `gh api`. Derives repo from the target's
origin remote. `adapter.label_kind_map` maps issue labels to node kinds.

## GitLab Issues adapter

`crates/adapter-gitlab/`. Reads issues and issue links via `glab api`. Uses `iid`, not
instance-wide number. `adapter.link_type_map` maps link types to edge kinds.

## OpenSpec adapter

`adapters/openspec` (Python). Reads `openspec/specs/*/spec.md` and `Requirement:`
citation comments in test files. No `adapter:` block — paths are conventional.
