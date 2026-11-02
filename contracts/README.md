# Contract artifacts

Generated pins of the surfaces a release promises, read by
`crates/latt/tests/contracts.rs`.

| File | Holds |
|---|---|
| `cli.txt`, `cli-tui.txt` | every command, option and positional argument, read off `-h`: requiredness, default, possible values. `cli-tui.txt` is the `tui` build's, with `browse` |
| `codes.tsv` | every finding code, its owner, the core's default severity (`-` for an adapter code), and whether every finding of it carries a node (`no` for an adapter code: the core cannot promise one) |
| `retired-codes.tsv` | every finding code a release retired, the code that replaced it, and the release; a profile naming one is refused |
| `id-patterns.tsv` | every shipped profile's node kinds and `id_pattern` |
| `released/` | the same files frozen at the last release, with its `VERSION` |

A change to the build regenerates the top-level files with `LATTICE_REGEN_BASELINES=1`;
the diff is the review. A break from `released/` must be listed under Unreleased ›
Breaking, naming the contract lint and the item in backticks, and is refused on a
patch line. The lints are `cli_command_removed`, `cli_option_removed`,
`cli_argument_removed`, `cli_now_required`, `cli_default_changed`,
`cli_value_removed`, `code_removed`, `code_severity_changed`,
`retired_code_unretired`, `id_kind_removed`, and `id_pattern_changed`. An artifact the
release predates (`released/` lacks the file) has nothing to break from until the
next cut. Any change to a pattern counts, because narrowing
cannot be told from the text. `cli_now_required` also names a required input
added to a command the release already had: additions are not uniformly
compatible.

`released/` is refreshed at each release cut by the ignored test
`seed_released_contracts`, once per feature set, with `LATTICE_SEED_BINARY` and
`LATTICE_SEED_PROFILES` naming the release's binary and profiles, run from the
release commit: it writes all four files, taking `codes.tsv` and `VERSION` from the
tree it runs in, and refuses a binary whose version differs. `released/VERSION`
names the current seed. Run it with absolute paths: the test's working directory is
the crate, not the repository.
