---
name: latt-author
description: Write or repair a Lattice profile, and an adapter when no shipped one reads the register's format, so that `latt validate` reads a repository's requirements, specs, tests or issues. Use when asked to set up Lattice for a register, write a lattice.yaml, or make `latt` read a file format it does not read yet. Covers choosing a shipped adapter or a template, the adapter contract's obligations, tight profile rules, and how to report a run whose input was partly unreadable. For running checks and answering questions over an existing profile, use the `latt` skill.
---

# latt-author

A register needs a profile, and an adapter for its format. Lattice ships adapters for
Markdown tables, TOML, prose cross-references, entomologist and OpenSpec. A format none
of them reads needs an adapter of your own, copied from a template.

The documentation sits beside the install's `bin/`, at
`"$(dirname "$(command -v latt)")/.."`: `docs/profiles.md` (kinds, IDs, edges,
validations), `docs/adapters.md` (each adapter's `adapter:` block, and "Writing your own
adapter"), `profiles/` (one profile per shipped adapter) and `examples/adapters/` (the
templates). The pages are long: print the section you need, such as
`awk '/^## Writing your own adapter/,0' docs/adapters.md`, rather than the whole page.

## A shipped format: write the profile

1. Copy the shipped profile for the format from `profiles/` (`md.yaml`, `toml.yaml`,
   `prose-xref.yaml`, `openspec.yaml`, `entomologist.yaml`). In `adapter.paths.files`,
   `*` stays within one directory and `**` crosses them. `md.yaml` and `toml.yaml` name
   a built-in adapter; the others need `--adapter` on every run: `--adapter prose-xref`,
   `--adapter entomologist`, or `--adapter "$(dirname "$(command -v latt)")/../adapters/openspec"`.
2. `latt show --profile p.yaml` loads it; exit 2 names the key at fault.
3. Run `latt describe` and `latt validate` (see Check the result).

## A new format: copy a template

Copy the template in the language the repository uses, or Python when it has none:
`examples/adapters/python/adapter.py` (standard library only), `go/` or `rust/`. Read
`examples/adapters/README.md` and the "Writing your own adapter" section of
`docs/adapters.md` before changing it.

Change how the template reads a line, and keep everything else. Its other functions are
the contract: the `resolved_schema` check and unknown-key refusal on the profile, the
leading-BOM strip, per-line UTF-8 decoding, and a finding for a missing directory, an
unreadable or non-regular file and every line it cannot read. When you rewrite a
function, carry its guards into the new one. The template's test checks the template's
own fixture: point it at a fixture in your format, with the counts and findings you
expect, before you rely on it, and report its result as it ran. An adapter at a path
without a `.py` suffix, such as `tools/latt-adapter`, cannot be imported by name: build the
module with `importlib.machinery.SourceFileLoader("adapter", path)` and call its
`exec_module` on a `types.ModuleType`.

Read the register's own description of its format, and handle every form it allows
(comment markers, repeated keys, continuation lines). A form you do not handle becomes a
false `PARSE_ERROR` on a valid line or, worse, a silently dropped row.

## Profile rules worth deciding

- `id_pattern`: the shape the register really uses (`^REQ-[0-9]{3}$` for three digits,
  not `[0-9]+`), so a malformed ID is reported.
- `allowed` pairs for each edge kind: only the directions the register means.
- Any key your adapter reads from the `adapter:` block, declared there rather than
  hard-coded.
- Which findings gate: `validations:` entries, a `COVERAGE` block when every requirement
  needs a test, severity overrides. `docs/profiles.md` lists them.
- A per-item obligation (this design needs a test, that one does not) is a bool attr
  plus `COVERAGE` and `EDGE_WHERE` entries: see `Per-item obligations` in
  `docs/profiles.md`. Declare each flag `required: true`, or an omitted flag exempts
  the item silently.

## Check the result

```sh
latt describe --profile lattice.yaml --adapter ./<adapter> --target . --format plain
latt validate --profile lattice.yaml --adapter ./<adapter> --target . --format plain; echo "exit $?"
```

For a shipped format, pass the `--adapter` from step 1, or none for `md` and `toml`.

- Compare `describe`'s counts per kind and per edge kind with the register. A kind at 0
  means a pattern, the target or the adapter is wrong. Zero findings over zero edges
  means no edge was built.
- Run the adapter on inputs outside the given files before you report: a missing
  directory, a broken symlink, a file starting with a BOM, an empty file, and each
  comment and syntax form the format allows. Each must give the nodes it holds or a
  `PARSE_ERROR`; a crash or a silent skip is a defect.

## Report a partly unreadable run

The adapter reports a line it cannot read as `PARSE_ERROR` and reads the rest. A
`PARSE_ERROR` is an error unless the adapter emits it at a lower severity (the openspec
adapter does for an unreadable scope) or the profile demotes or suppresses it, so such a run
usually exits 1; exit 1 also follows any other unsuppressed error, and a suppressed
`PARSE_ERROR` exits 0 with a `Partial input:` line on stderr. Report the unread lines
whatever the exit code, exit 0 included. Say:

- each unreadable line, as file and line, with what is wrong with it;
- what that line appears to have declared, such as the test whose `verifies` line it seems
  to be;
- what is unknown as a result. A node whose verifying line went unread may carry
  `state: unverified` in `coverage --rule` and `validate` JSON: that state describes the
  lines that were read. Quote it with `input_status: partial`, and say the run could not
  establish whether the node is verified rather than that it is unverified.
