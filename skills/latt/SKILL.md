---
name: latt
description: Check and query the records a repository keeps in its own files (requirements, specs, tests, issues) and the links between them, with the Lattice CLI `latt`. An adapter per format reads the files where they live (Markdown tables, TOML, prose cross-references, entomologist, OpenSpec, or one the user wrote) and a YAML profile declares the kinds, IDs and rules. Use when the repository has a Lattice profile and you are asked which requirements have no recorded test link, whether references resolve, what a change touched or why `latt` failed in CI, or to write a profile for a register. Most useful when records span many files, rows hold syntax a text search misreads, or the check repeats on every revision; for a small register and a one-off question, reading the files may be quicker. It reports recorded links; it does not run tests or judge whether a requirement is met.
---

# latt

Lattice reads a repository's registers through an adapter, builds one graph of nodes
and edges, and checks it against a YAML profile. It only reads. To fix a finding, edit
the register file at the line the finding names, or use the register's own tool.

## Fast path

The profile is a YAML file with `node_kinds:` in it, usually `lattice.yaml` at the
repository root. When the question is about a CI job or hook, the command that job runs
is the setup in question: read it, and run what it runs. Otherwise answer from one call:

```sh
latt describe --profile <profile.yaml> --target . --format plain; latt validate --profile <profile.yaml> --target . --format plain; echo "exit $?"
```

`describe` prints what the profile declares and what was read under it: each node kind
with its ID pattern, count and files, each edge kind with its allowed pairs and count,
the validation entries, and the `adapter:` block. That replaces reading the profile.
`validate` prints the findings, then a `scope:` line on stderr. Its exit code says what
to do next:

| Exit | Meaning | Do |
|---|---|---|
| 0 | no error-severity findings | report warnings and hints if asked |
| 1 | error-severity findings | each names a file and line; fix the register there |
| 2 | Lattice could not run | the message names the profile, adapter or flag at fault; see Setup |

Answer from that output. Open a register file only for what the findings do not carry,
such as a requirement's wording, and go to the line they name. A question about one node
or one change takes one more call, from the table under Other commands.

Only `validate` grades the register: `describe`, `coverage` and `query` exit 0 whatever
the adapter found, and `summary` exits 1 on an error-severity adapter finding
(`docs/json.md`, "Adapter findings on stderr").

## Read the findings

- `VACANCY`: an edge names an ID nothing declares. Fix the reference or declare the target.
- `COVERAGE`: a node lacks the incoming edge its profile requires, such as a requirement
  no test verifies.
- `PARSE_ERROR`: the adapter could not read the input at that line, and read the rest.
  Fix the input. Until you do, counts and percentages from that run cover only what was
  read; say so when you quote them.
- `EMPTY_KIND` (info): a declared kind nothing populated, such as `node kind 'test' has
  no node` on a run that read no tests.
- Other codes: `docs/profiles.md`, "Finding codes".

Check the `scope:` line, or `describe`'s counts, before you report a clean result: zero
findings over zero edges means the adapter built no edges, so nothing could dangle.

## Setup

Use this when the fast path exits 2 or no profile is in sight.

- No profile at the root: `grep -rl --include='*.y*ml' '^node_kinds:' .` finds it. The
  repository's AGENTS.md, CI configuration or pre-commit hook usually holds the exact
  command; reuse it.
- A `--profile` that names no file exits 2 and lists the profiles beside it.
- `--target` is the directory the profile's file patterns are relative to, usually the
  repository root. A `NO_MATCHING_FILES` finding means it points elsewhere, and names
  the directory to target when one would match.
- Add `--adapter <name or path>` when the profile's `adapter:` block has no `native:` key.

`latt <command> --help` documents every flag. A release install keeps the full
documentation beside its `bin/`, at `"$(dirname "$(command -v latt)")/../docs"`;
otherwise read it at https://github.com/jonelay/lattice/tree/v0.8.0/docs.

## Other commands

Each takes the same `--profile` and `--target`. Use `--format json` when a script parses
the answer (`docs/json.md` lists every key).

| Command | Answers |
|---|---|
| `query reached-by <ID>` | what reaches a node, such as the tests that verify a requirement (`--edge-kind verifies`) |
| `query reaches <ID>`, `query path <A> <B>` | what a node reaches; how two nodes connect |
| `query diff <rev> <rev>` | what changed in the register between two git revisions |
| `query at <file>` | what a file declares, and the findings that start there |
| `coverage` | per kind, how many nodes have incoming and outgoing edges of any kind; a `contains` edge counts, so ask `validate` (`COVERAGE`) whether requirements have tests |
| `summary` | node and edge counts per kind, and findings per code |

`latt browse` is interactive and needs a terminal; suggest it to a person.

## Write a profile

1. Copy the shipped profile for the register's format from `profiles/` (`md.yaml`,
   `toml.yaml`, `prose-xref.yaml`, `openspec.yaml`, `entomologist.yaml`). In
   `adapter.paths.files`, `*` stays within one directory and `**` crosses them.
2. `latt show --profile p.yaml` loads it; exit 2 names the key at fault.
3. Run the fast path's call. Compare `describe`'s counts and files with the register; a
   kind at 0 means a pattern or the target is wrong.

`docs/profiles.md` covers kinds, ID patterns, edges and validations; `docs/adapters.md`
covers each adapter's `adapter:` block.
