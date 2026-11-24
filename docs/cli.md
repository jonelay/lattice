# Command line

What each `latt` command does, the output formats, and what the exit code means.

## Commands

| Command | What it answers |
|---|---|
| `validate` | Where does the register break the profile's rules? |
| `trace` | Every node, with its edges and findings. It takes no node ID; filter `--format json` for one |
| `summary` | How many nodes are in each status? (structural counts when no status attr is configured) |
| `coverage` | Per kind, how many nodes have incoming and outgoing edges of any kind? Use `--rule N` for one configured `COVERAGE` rule, its missing links and uncertainty. |
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

`query diff` reads each revision from its committed blobs (`git ls-tree` and
`git cat-file`) into a private scratch directory: `.gitattributes` settings
(`export-ignore`, `export-subst`, `eol`, `text`, `filter`) do not apply, no smudge
filter runs, and a submodule is an empty directory. git runs in the `--target`
repository on the stored objects: an inherited `GIT_DIR` (or another variable git lists
as repository-local) and `refs/replace/*` replacement objects do not apply. A revision, or a tree entry, that
cannot be read safely exits 2.

The scratch directory is created private (mode 0700) with an unpredictable name under
`TMPDIR`. `latt` checks that it is still the directory it created before clearing or
writing it, but a process running as the same user that can rename entries in `TMPDIR`
while extraction runs is outside what that check defends against.

Every command but two takes `--profile <file>` and `--target <dir>`. `fuse` takes
`--manifest <file>`, which names each register's profile and target
([fuse.md](fuse.md#manifest)), and `show` takes `--profile` alone. The other flags:

- `--adapter` names the adapter: a built-in name (`md`, `toml`, `prose-xref`), a program
  on PATH (`entomologist` from an install) or a path to one. Without it, the profile's
  `adapter.native` decides. From a checkout, name an external adapter by its shim path,
  `./adapters/<name>` (`./adapters/openspec`, `./adapters/entomologist`). `adapter` takes
  no `--adapter`, and takes the resolved profile `show` prints, not the YAML.
- `--profile-version <X.Y.Z>` pins the profile on every command that reads profile YAML
  ([profiles.md](profiles.md)). `adapter` reads the resolved JSON, so pin the `show` that
  wrote it; `fuse` takes no pin.
- `--edge-kind <kind>`, repeatable, on `query reaches`, `reached-by` and `path`: traversal
  follows any listed kind, and without it every kind.
- `--suggestions <file>`, repeatable, on `validate`: renders each document's hints
  ([adapters.md](adapters.md#suggestion-document)).

`latt <command> --help` lists the rest.

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

### Rule coverage

`coverage --rule N` selects the Nth `COVERAGE` entry in the resolved profile,
counting from 1 in declaration order. `describe` lists the validation entries.
For this repository, use:

```sh
target/debug/latt coverage --rule 1 --profile profiles/openspec.yaml \
    --adapter ./adapters/openspec --target . --format json
```

The report names the incoming edge kind, target kind, and resolved `where` and
`source_where` filters. The denominator is explicit target-kind nodes matching
`where`. A target counts as covered when at least one selected edge enters it
from an explicit source matching `source_where`. Missing nodes carry IDs,
locations and `unknown` or `unverified` attribution state. An empty denominator
has a null percentage. This measures recorded evidence; it makes no claim that
linked tests pass or that a requirement is met.

The source population and unattributed count explain attribution uncertainty.
An outgoing edge to an absent target still attributes its source. Adapter findings
remain beside the aggregate on stdout in all three formats, even with `SUPPRESS`;
there is no duplicate findings block on stderr. `input_status: partial` means a
parse finding or host context identifies incomplete input; `unknown` leaves
completeness unproven. Quote a percentage with that status.

The selector covers direct `COVERAGE` rules; use `validate` for `COVERAGE_DEEP`.
A missing, zero, out-of-range or severity-only selection exits 2. A completed
report exits 0 even with missing links or adapter errors; zero nodes with an
error-severity adapter finding exits 2. Without `--rule`, coverage keeps its
existing per-kind incidence output and separate stderr findings.

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
  `2`. An attr that no node carries is named on stderr; `=`, `~=` and the comparisons match nothing on it,
  while `!=` matches every node that lacks it.

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
command; `json` carries the same as its `scope` key. `fuse` prints a header line with
the node, edge and pathway counts, then its findings, then the composed graph: every
node, every edge (`(unresolved)` when its target is not declared) and every pathway, so
its `plain` output grows with the registers while its findings stay at the top. The other
commands print their answer in every format, an empty one included (`No orphans.`).
Anything that parses the output, a script or an agent alike, reads `json` and the exit
code: `rich` and `plain` may change between releases, and `json` lists every finding
a `SUPPRESS` entry silenced (of the text views, only `coverage --rule` and `browse` show
them). An agent reading one answer to report it may use `plain`, as
the [agent skill](../skills/latt/SKILL.md) does. `show` prints the resolved profile
document, and `browse` owns the terminal.

In `rich` and `plain`, every batch command writes a control character from register or
profile text (a node ID, a kind, a message, a file name, a note) as a visible escape: `\n` for a newline, `\r` for a
carriage return, `\u{1b}` and the like for the other C0 and C1 controls, DEL, the bidi
controls and U+2028/U+2029. Tab stays a tab. Each finding is then one line, a TOML parse
error's caret snippet included, and an ID cannot carry a terminal escape or a link onto
the screen. A batch command's `--format json` carries the text unchanged. The documents
`show` and `adapter` print write DEL, the C1 and bidi controls and U+2028/U+2029 as JSON
escapes (`\u009b`), so they parse to the same value; JSON escapes the C0 controls itself.

The escape is for reading by eye. A register value holding the four
characters `a\nb` prints the same as one holding a newline, so `json` is the channel that
tells them apart. Stderr notices escape the same characters but keep a newline, so a path,
a node-ID argument, a revision or an adapter's stderr quoted in an error can still start a
new stderr line. That is a known limitation: a stderr line that reads like a finding
(`ERROR …`) may be quoted text. Read findings from the exit code and `--format json`.
A usage error escapes the argument it quotes, newline included, and so does a rejected
`--filter`'s own message. `rich` keeps its own colour sequences and escapes any other
(`ESC [ 8 m` among them). Invisible characters that
do not move the cursor or reorder text (zero-width spaces and joiners, the byte-order
mark, the soft hyphen, tag characters, variation selectors) stay as they are, so emoji
sequences and CJK text print intact.

Rich output and `browse` honour `NO_COLOR`. Rich output also drops colour under
`TERM=dumb`, and keeps it on a pipe when `FORCE_COLOR` is set to any non-empty value;
`NO_COLOR` wins over `FORCE_COLOR`.

With a screen reader, use `--format plain` rather than `browse`, which redraws the whole
screen. Plain output reads line by line, and `trace`, `query at` and `coverage` name each
field (`kind:req total:4`); `--format json` carries the whole structure.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Command ran; whether findings gate depends on the command |
| `1` | Command ran and an error-severity finding gates under that command |
| `2` | could not run: a profile that does not load, an adapter that cannot start, a bad flag, a panic on the main thread |

A failed write of required output to stdout or stderr exits `2`, including a
diagnostic or scope line that the destination could not accept.

The commands apply those codes as follows:

| Command | `0` | `1` | `2` |
|---|---|---|---|
| `validate`, `trace`, `fuse` | No unsuppressed error-severity finding | Unsuppressed error-severity finding | Could not run |
| `summary` | No unsuppressed error-severity adapter finding | Unsuppressed error-severity adapter finding | Could not run |
| `coverage`, `query` | Answered | Never | Could not run or could not pose the question |
| `describe` | Ran, even with adapter findings | Never | Could not run |
| `show`, `adapter` | Ran | Never | Could not run |

`fuse` at `2` also names each cause on stderr, a `fuse:` line per failed source or
unreadable file with the stage it failed at and the next step (a findings object in
`json`; [fuse.md](fuse.md#1-source-runs)). `json` stdout gains only the optional `stage` key on the failure findings fuse raises.

A `--profile` that names no file is a `2`; the message lists the `.yaml`/`.yml` files
beside it that declare `node_kinds`, or says none was found.

A `--target` that does not exist is not one of the `2` cases: the adapter reports it as a
`PARSE_ERROR` finding, so `validate` exits `1`. A `query` over it has no register to
answer from and exits `2`, as below.

Keep `1` and `2` apart in a script: `1` is an answer about the register, `2` says the
setup is broken. `--strict` (on `validate`, `trace`, `fuse` and `browse`) promotes
warnings to errors before the exit code is decided; hints are never promoted.
`SUPPRESS` entries in a profile silence expected findings without hiding them from JSON
output ([profiles.md](profiles.md#suppress)). A suppressed `PARSE_ERROR` still leaves
`Partial input: the profile suppresses N PARSE_ERROR; --format json lists it` on stderr in
`plain` and `rich`, one line per run (per revision for `query diff`, prefixed with it);
the exit code is unchanged. `coverage --rule` instead keeps the parse findings in its
report with `input_status: partial` ([Rule coverage](#rule-coverage)), and `browse`
lists suppressed findings when `.` is pressed.

`query` asks a question rather than auditing the register, so it never exits `1`: an
answer, an empty one included, is `0`, and only a question that cannot be asked (an
unknown node, or, except for `query at`, a register from which the adapter read no node while
reporting an error-severity finding) is `2`; a readable but empty register answers. An adapter's own
findings still print after the answer, on stderr ([json.md](json.md#adapter-findings-on-stderr)
says what stderr carries in `json` mode).
`coverage` without `--rule` does the same; `--rule` keeps findings in its stdout
report. `summary` prints
them too, but exits `1` when one is an unsuppressed error. Without a `SUMMARY` configuration its
table tallies the unsuppressed findings, validation findings included. Only adapter
findings set the exit code, so an error-severity finding can print there with exit `0`.
