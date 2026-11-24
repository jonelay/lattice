# Adapter templates

Three adapters, in Python, Go and Rust, that read the same small register format and print
the same interface document. Copy the one in your language when no shipped adapter reads
your format, then change how it reads a line. The contract they follow is
[Writing your own adapter](../../docs/adapters.md#writing-your-own-adapter).

| Path | Holds |
|---|---|
| `python/adapter.py` | the Python template: one file, standard library only, Python 3.10 or later |
| `go/` | the Go template: one module, standard library only |
| `rust/` | the Rust template: one crate built on `latt-adapter`, Lattice's document types |
| `profile.yaml` | the profile the fixture is read with |
| `fixture/` | a target holding a small register, with one example of each kind of bad input |
| `expected.json` | the document every template prints for `fixture/` |

## The format

The register is the regular files directly inside one directory whose names end in one
extension. The profile's `adapter:` block names both:

```yaml
adapter:
  directory: register
  extension: .reg
```

Each line of a file declares one node and its outgoing edges:

```text
# comments and blank lines are skipped
requirement REQ-1: The pump stops on overpressure
test T-2 verifies REQ-2 verifies REQ-9: Relay opens
```

The words before the first `:` are the node's kind, its ID, then pairs of edge kind and
target ID. The rest of the line is the node's `text` attr. Kinds, ID shapes and targets
are left to the core, which checks them against the profile.

## What each template does

All three follow the same steps, in the same order:

1. Read `--profile` and `--target`. Anything else on the command line is exit 2.
2. Load the resolved profile (the JSON `latt show --profile` prints, which the core
   passes as `--profile`) and read the `adapter:` block. An unknown key, or a missing
   `directory` or `extension`, is exit 2. The core's own keys (`native`,
   `timeout_seconds`, `command_timeout_seconds`) are accepted.
3. List the directory, sorted by name, and read each file that ends in the extension:
   strip one leading BOM, split on `\n`, drop a trailing `\r`.
4. Turn each declaration into a node, and each pair into an edge that records, in
   `declared_by`, the index of the node whose line declared it.
5. Report what cannot be read as a finding and keep reading. A malformed line, a line
   that is not UTF-8, a file that cannot be opened, an entry that is not a regular file,
   or a missing directory each becomes a `PARSE_ERROR`. A directory with no matching
   file gives `NO_MATCHING_FILES`.
6. Print the document and exit 0. Exit 2, with one line on stderr and nothing on stdout,
   only when no document can be produced.

A finding is how the core and the user learn that part of the register went unread. An
adapter that skips a bad line without one reports a register as clean that it did not
fully read.

## Run one

```sh
latt validate --profile examples/adapters/profile.yaml \
    --adapter examples/adapters/python/adapter.py --target examples/adapters/fixture
```

The fixture is broken on purpose, so this exits 1 and lists four `PARSE_ERROR`s and a
`VACANCY` for `REQ-9`, which nothing declares. For the Go and Rust templates, build first
(`go build -o lines-adapter .` in `go/`, `cargo build --release` in `rust/`) and pass the
binary to `--adapter`.

## Copy one

Each template has a test that builds the fixture and compares the result with
`expected.json`. Keep the test when you copy, and point it at a fixture of your own.

**Python.** Copy `adapter.py` and `test_adapter.py`. Change `read_line`, and `build` if your
files are selected differently. Run the test with `python3 -m unittest`.

**Go.** Copy `go/`, rename the module in `go.mod`, and change `readLine`. Run the test with
`go test ./...`.

**Rust.** Copy `rust/` and replace the `latt-adapter` path dependency in `Cargo.toml` with
the release tag:

```toml
latt-adapter = { git = "https://github.com/jonelay/lattice", tag = "v0.8.1" }
```

The `[lints.clippy]` table answers a lint setting of this repository and can go. Change
`read_line`. Run the test with `cargo test`. `latt-adapter` also has the subprocess
runner and the file-selection helpers the shipped adapters use; its crate docs list them.

## Keep a copy honest

Run your profile through `latt validate` with the new adapter over a register you know,
and compare the node and edge counts with what the register holds. Zero findings over
zero edges also describes an adapter that built no edges at all.
