# adapter-entomologist Specification

## Purpose
Reads an entomologist ("ent") issue register — one directory per issue, one plain
file per field, stored on a git orphan branch — into an interface document, so
lattice can answer questions over a register that has no files in the target's
worktree at all.

## Requirements
### Requirement: Read the register from the data branch, not the worktree

The adapter SHALL read issues from the target repository's data branch, resolving
`refs/heads/entomologist-data` and falling back to
`refs/remotes/origin/entomologist-data` — fully qualified, so a tag or any other
ref that happens to carry the name is never read. The chosen ref SHALL be
resolved to a single commit before any content is read, and all reads SHALL go
through that commit, so the graph comes from one snapshot even while a sync is
rewriting the branch. It SHALL NOT read the worktree: ent stores issues on an
orphan branch precisely so they never appear there.

The target SHALL be the root of the repository that is read: the adapter
compares the target path against git's resolved top-level directory (both
canonicalized), so a target nested under some other checkout cannot silently
read that ancestor's register. A target that is not that root, a target in no
git repository at all, and a repository with neither ref SHALL each produce a
`PARSE_ERROR` issue saying which, an otherwise empty document, and exit 0 — a
register that is absent or mislocated is a finding, not an adapter that broke.
A `git` binary that cannot be run at all is a broken setup: the adapter fails
rather than reporting, and the core surfaces that as exit 2.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_entomologist.py -k branch`

#### Scenario: Local branch preferred

- **WHEN** the target has both `entomologist-data` and `origin/entomologist-data`
- **THEN** the adapter reads the local branch

#### Scenario: Remote fallback

- **WHEN** the target has only `origin/entomologist-data`
- **THEN** the adapter reads the remote-tracking ref and builds the same graph

#### Scenario: Data branch absent

- **WHEN** the target is a git repository with no `entomologist-data` ref at all
- **THEN** the adapter emits a `PARSE_ERROR` naming the branch, emits no nodes,
  and exits 0

#### Scenario: A tag carries the branch name

- **WHEN** the target has a tag named `entomologist-data` and no branch or
  remote-tracking ref of that name
- **THEN** the adapter reports the branch as absent and never reads the tag

#### Scenario: Target is in no git repository

- **WHEN** the target directory is not inside any git repository
- **THEN** the adapter emits a `PARSE_ERROR` saying so and exits 0

#### Scenario: The resolved ref cannot be listed

- **WHEN** the data branch ref resolves but `git ls-tree` on the resolved
  commit fails — for example the ref names an object that does not exist
- **THEN** the adapter emits a `PARSE_ERROR` naming the failed listing, emits
  no nodes, and exits 0

#### Scenario: Target is nested inside some other repository

- **WHEN** the target directory is not itself a repository root but sits under a
  checkout that has an `entomologist-data` branch
- **THEN** the adapter emits a `PARSE_ERROR` naming the mismatch and does not
  read the ancestor's register

### Requirement: Build one issue node per issue directory

The adapter SHALL emit one `issue` node per top-level directory on the data
branch whose name is a 32-hex issue id. The node ID SHALL be the directory name —
it is what a `dependencies/` entry in another issue names, and so is the join key
that must resolve.

The node carries, when the file exists and decodes, `summary` (the first line of
the `description` file, which ent documents as git-commit style), `author`,
`assignee` and `tags` (one list element per file under `tags/`); `state` is
always carried, per its own requirement below.

Requiredness is profile data, not adapter code: the profile SHALL mark
`summary`, `author` and `state` required, and the adapter SHALL emit the node
with whatever fields exist. An issue missing one of those files then surfaces as
`ATTR_REQUIRED` at validation — one finding, owned by the layer that owns the
vocabulary — rather than as an adapter that either invents a value or drops the
node.

The profile marks the kind `orphan_ok`: an issue with no dependency edges is
the normal shape of a flat tracker, not a finding, and `query orphans` still
surfaces them.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_entomologist.py -k issue_node`

#### Scenario: Issue node with field attrs

- **WHEN** the branch holds an issue directory with `description`, `author`,
  `state`, `assignee` and two files under `tags/`
- **THEN** the graph contains one `issue` node whose ID is the directory name,
  with `summary` set to the description's first line, and `author`, `state`,
  `assignee` and a two-element `tags` list carried as attrs

#### Scenario: Description absent

- **WHEN** an issue directory has no `description` file
- **THEN** the adapter emits the node without a `summary` and no issue of its
  own, and validation reports `ATTR_REQUIRED` for it

### Requirement: State is passed through, with ent's own default

The adapter SHALL carry the `state` file's content as the `state` attr verbatim
(trimmed), and SHALL apply ent's documented default of `new` when the file is
absent — ent's own reader does exactly that. The default applies to absence
only: an *undecodable* `state` file is treated like any other unreadable fetched
field — `PARSE_ERROR`, attr omitted, `ATTR_REQUIRED` at validation — because
defaulting it would rewrite a value that exists but cannot be read. The adapter
SHALL NOT validate the value against a state vocabulary: the vocabulary is profile data (an `enum`
attr), so a state ent adds later surfaces as a profile finding, not as an adapter
that silently rejects or rewrites it.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_entomologist.py -k state`

#### Scenario: Missing state file defaults to new

- **WHEN** an issue directory has no `state` file
- **THEN** the node carries `state: new`

#### Scenario: Unknown state passes through

- **WHEN** an issue's `state` file holds a value the profile's enum does not list
- **THEN** the adapter emits the node with that value unchanged, and validation —
  not the adapter — reports it as `ATTR_ENUM`

### Requirement: Dependencies become depends_on edges

The adapter SHALL emit one `depends_on` edge per file under an issue's
`dependencies/` directory, from that issue to the issue named by the file name.
The adapter SHALL NOT check that the target exists: an unresolvable dependency is
a `VACANCY` finding at validation, and an adapter that dropped the edge
would hide it.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_entomologist.py -k edge`

#### Scenario: Dependency becomes an edge

- **WHEN** issue `aaaa…` has a file `dependencies/bbbb…`
- **THEN** the graph contains a `depends_on` edge from `aaaa…` to `bbbb…`

#### Scenario: Dependency on an absent issue

- **WHEN** a `dependencies/` file names an id no issue directory declares
- **THEN** the adapter still emits the edge, and `lattice validate` reports
  `VACANCY` for it

### Requirement: Recognized-but-unrepresented content is deliberate, unrecognized content is reported

The adapter recognizes, and deliberately does not represent, `creation_time`,
`done_time`, the branch-root `README.md`, and an issue's comment files —
`comments/<32-hex>/(author|creation_time|description)` exactly. Comments are
ent's conversation surface, not register structure: the adapter SHALL pass over
recognized-unrepresented paths without an issue and SHALL NOT fetch their
content, since a file that contributes nothing to the graph cannot shrink it.

Any other file on the branch — a field name the adapter has no reader for, a
stray file inside a `comments/` tree, or a top-level entry that is neither an
issue directory nor the README — SHALL produce an issue naming the path, so a
shape ent adds later cannot shrink the graph in silence.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_entomologist.py -k unread`

#### Scenario: Comments pass in silence

- **WHEN** an issue directory holds a well-formed `comments/` tree
- **THEN** the adapter emits no issue and no node for it, and fetches none of
  its blobs

#### Scenario: Stray file inside a comments tree

- **WHEN** a comment directory holds a file `priority` outside the recognized
  comment fields
- **THEN** the adapter emits an issue naming that path

#### Scenario: Unrecognized field is reported

- **WHEN** an issue directory holds a file `priority` the adapter has no reader
  for
- **THEN** the adapter emits an issue naming that path, and still emits the
  issue node from the fields it does read

### Requirement: Unreadable content is reported, never raised

The adapter fetches only the blobs of paths it represents. A fetched blob that
cannot be decoded as UTF-8, or a git read that fails for a fetched path, SHALL
produce a `PARSE_ERROR` naming the path, and the adapter SHALL continue with the
remaining issues and exit 0. An undecodable required field yields two findings —
the adapter's `PARSE_ERROR` naming the path and validation's `ATTR_REQUIRED`
naming the node — which is deliberate: different layers, different information.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_entomologist.py -k unreadable`

#### Scenario: Undecodable blob among sound issues

- **WHEN** one issue's `description` is not valid UTF-8 and other issues are
  sound
- **THEN** the adapter emits a `PARSE_ERROR` naming that path, emits the nodes
  from the sound issues, and exits 0

### Requirement: Serve as the branch-read gate

`lattice validate` run with the entomologist profile and adapter against the
builder-constructed mini-ent fixture repository SHALL exit 1 — the fixture's
malformed shapes make that deterministic — and the interface document SHALL carry
exactly the node set, edge set, and finding code+path multiset the fixture
declares. The exactness is normative in both directions: a missing node or edge
is a partial read the batch framing hid, and a finding outside the multiset is a
regression hiding behind the expected exit 1. The enumeration lives in the gate
test, beside the fixture that defines it.

The fixture content is synthesized, shaped like ent's data branch but copied from
nowhere — ent is unlicensed, so its real issue text cannot be republished here.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_entomologist.py -k gate`

#### Scenario: Gate run over the fixture

- **WHEN** `lattice validate` runs with the entomologist profile and adapter
  against the fixture repository the test builder constructs
- **THEN** it exits 1, and the interface document carries exactly the fixture's
  declared nodes, edges, and finding multiset — nothing missing, nothing extra

#### Scenario: Adapter is a program that exits 0

- **WHEN** the adapter is invoked directly with `--profile` and `--target`
  against a fixture containing an issue with unreadable content
- **THEN** it writes an interface document to stdout and exits 0, carrying the
  unreadable content as findings rather than as a non-zero exit
