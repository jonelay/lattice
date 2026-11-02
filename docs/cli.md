# Command line

What each `latt` command does, the output formats, and what the exit code means.

## Commands

| Command | What it answers |
|---|---|
| `validate` | Where does the register break the profile's rules? |
| `trace` | Every node, with its edges and findings. It takes no node ID; filter `--format json` for one |
| `summary` | How many nodes are in each status? (structural counts when no status attr is configured) |
| `coverage` | Per kind, how many nodes have incoming and outgoing edges of any kind? (Link incidence: test coverage is a `COVERAGE` validation.) |
| `describe` | What does the profile declare, and what did the adapter read under it? Each kind with its ID pattern, count and source files; the validation entries; the adapter and its `adapter:` block. Validates nothing and exits `0` whenever it ran |
| `query reaches/reached-by` | What can this node reach, and what reaches it? (`--check-resolved` flags nodes reachable only through undeclared endpoints) |
| `query path` | What is the shortest path between two nodes? |
| `query orphans` | Which nodes have no edge at all? |
| `query counts` | How many nodes and edges of each kind? |
| `query at` | What does this source file declare, and which findings start there? |
| `query diff` | What changed between two git revisions? (`query diff v1.0 HEAD …`) |
| `fuse` | Do the references between several registers resolve? ([fuse.md](fuse.md)) |
| `browse` | Interactive navigation of nodes, edges and findings ([browse.md](browse.md)) |
| `show` | What profile document does the adapter receive? |
| `adapter` | What interface document does a built-in adapter (`adapter md`) produce? ([adapters.md](adapters.md)) |

Every command but two takes `--profile <file>` and `--target <dir>`: `fuse` takes
`--manifest <file>`, which names each register's profile and target
([fuse.md](fuse.md#manifest)), and `show` takes `--profile` alone. `adapter` takes the
resolved profile `show` prints, not the YAML, and no `--adapter`. `--adapter` names the
adapter: a built-in name (`md`, `toml`, `prose-xref`), a program on PATH (`entomologist`
from an install) or a path to one. Without it, the profile's
`adapter.native` decides. From a checkout, name an external adapter by its shim path,
`./adapters/<name>` (`./adapters/openspec`, `./adapters/entomologist`).
`--profile-version <X.Y.Z>` pins the profile on every command that reads profile YAML
([profiles.md](profiles.md)); `adapter`, which reads the resolved JSON, takes no pin, so
pin the `show` that wrote it. `query reaches`, `reached-by` and `path` take
`--edge-kind <kind>`, repeatable: traversal follows any listed kind, and without it every
kind. `validate --suggestions <file>` is repeatable too, rendering each document's hints
([adapters.md](adapters.md#suggestion-document)). `latt <command> --help` lists the rest.

### Try them on this repository

From a checkout, after `cargo build --locked`, this repository's own requirements are a
register:

```sh
target/debug/latt validate --profile profiles/openspec.yaml \
    --adapter ./adapters/openspec --target .

target/debug/latt trace --profile profiles/openspec.yaml \
    --adapter ./adapters/openspec --target . --format plain

target/debug/latt query reaches trace-report --profile profiles/openspec.yaml \
    --adapter ./adapters/openspec --target .

target/debug/latt coverage --profile profiles/openspec.yaml \
    --adapter ./adapters/openspec --target .
```

### Filters

`query reaches`, `query reached-by`, `query orphans`, `query counts` and `query at` take
`--filter <attr><op><value>`, which keeps only the nodes whose attrs match:

| Operator | Matches when the attr |
|---|---|
| `=`, `!=` | equals, or does not equal, the value |
| `>`, `>=`, `<`, `<=` | orders after or before the value |
| `~=` | matches the value as a regular expression |

- The condition splits at the first operator in it, so the attr name never holds one and
  the value may: `status=a>b` compares `status` with the string `a>b`.
- For `=` and `!=` the value is read as an integer if it is one, else as `true` or
  `false`, else as a string. For the ordering operators it is an integer or a string.
- Repeat `--filter` to require every condition at once.
- On `reaches` and `reached-by` a filter trims the nodes reported, not the walk: a node
  that does not match still leads on to those beyond it.
- On `counts` it filters the node tallies; the edge tallies stay totals over the whole
  graph.
- A filter with no operator, an empty attr name or an invalid regular expression exits
  `2`. An attr that no node carries is named on stderr and matches nothing; the exit
  code is unchanged.

Quote each filter so the shell does not read `>` or `<` as a redirect:

```sh
target/debug/latt query counts --profile profiles/openspec.yaml \
    --adapter ./adapters/openspec --target . \
    --filter 'scenarios>=3' --filter 'title~=^Query'
```

## Output formats

All commands except `show`, `adapter` and `browse` take `--format plain|json|rich`.

| Format | For | What it is |
|---|---|---|
| `rich` | people at a terminal (the default on a TTY) | colour and grouping; `validate` prints `No findings.` when there are none |
| `plain` | logs, grep, screen readers (the default when stdout is not a TTY) | uncoloured lines; `validate` prints nothing to stdout when there are no findings |
| `json` | scripts and agents | the one format whose shape is a public surface; every key, and what may change it, is in [json.md](json.md) |

In `rich` and `plain`, `validate` ends with one line on stderr saying what it read —
`scope: 2 files; nodes requirement 3, test 3; edges verifies 3` — so a clean run that
read nothing is told apart from a clean run over a populated register without a second
command; `json` carries the same as its `scope` key. The other commands print their
answer in every format, an empty one included (`No orphans.`). Anything that parses the
output, a script or an agent alike, reads `json` and the exit code: `rich` and `plain`
may change between releases, and only `json` lists the findings a `SUPPRESS` entry
silenced. An agent reading one answer to report it may use `plain`, as the
[agent skill](../skills/latt/SKILL.md) does. `show` prints the resolved profile document as it
is, and `browse`
owns the terminal.

Rich output and `browse` honour `NO_COLOR`. Rich output also drops colour under
`TERM=dumb`, and keeps it on a pipe when `FORCE_COLOR` is set to any non-empty value;
`NO_COLOR` wins over `FORCE_COLOR`.

With a screen reader, use `--format plain` rather than `browse`, which redraws the whole
screen. Plain output reads line by line, and `trace`, `query at` and `coverage` name each
field (`kind:req total:4`); `--format json` carries the whole structure.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | ran, and found nothing at error severity |
| `1` | ran, and found at least one error-severity finding |
| `2` | could not run: a profile that does not load, an adapter that cannot start, a bad flag |

A `--profile` that names no file is a `2` whose message lists the `.yaml`/`.yml` files
beside it that declare `node_kinds` — a misnamed profile is told what was probably
meant — or says that no profile was found in that directory. Only that directory is
read.

A `--target` that does not exist is not one of the `2` cases: the adapter reports it as a
`PARSE_ERROR` finding, so `validate` exits `1`. A `query` over it has no register to
answer from and exits `2`, as below.

Keep `1` and `2` apart in a script: `1` is an answer about the register, `2` says the
setup is broken. `--strict` (on `validate`, `trace`, `fuse` and `browse`) promotes
warnings to errors before the exit code is decided; hints are never promoted.
`SUPPRESS` entries in a profile silence expected findings without hiding them from JSON
output ([profiles.md](profiles.md#suppress)).

`query` asks a question rather than auditing the register, so it never exits `1`: an
answer, an empty one included, is `0`, and only a question that cannot be asked (an
unknown node, a register the adapter could read nothing from) is `2`. An adapter's own
findings still print after the answer, on stderr ([json.md](json.md#adapter-findings-on-stderr)
says what stderr carries in `json` mode). `coverage` does the same. `summary` prints
them too, but exits `1` when one is an unsuppressed error, since its table would count a
register it could not read.
