# finding-notes Specification

## Purpose

Lets the people who own a register record their judgement about a finding — why it
stands, who said so, until when — in a file the profile names, and shows that judgement
wherever the finding is read, without hiding or downgrading the finding.

## Requirements

### Requirement: Notes file location
A profile MAY declare a top-level `annotations` key whose value is a string path to a
notes file. The path SHALL be resolved relative to the directory of the profile file that
declares it, before `extends` merging, so a parent profile's `annotations` names a file
beside the parent. Under `extends` the key is a scalar: the nearest declaration in the
chain wins. A non-string value SHALL be a profile load error naming the key.

A notes file that cannot be parsed as YAML, or whose top level is neither a list nor
empty, SHALL be a profile load error naming the file (exit 2). A notes file that does
not exist SHALL NOT be a load error: validation SHALL report `NOTE_FILE_MISSING` naming
the resolved path, so a mistyped `annotations` never reads as "no notes". An empty file
holds no entries.

The resolved profile document handed to adapters SHALL NOT carry `annotations`:
notes are the core's input, and the resolved path is specific to one machine. No lattice
command SHALL write, create or append to the notes file.

The notes file SHALL carry no version key of its own. Its shape is part of the profile
schema and follows the `profile_version` major of the profile that names it: a change
that makes a formerly valid notes file invalid is a profile-schema major change.

Verified by: `cargo test -p latt --locked --test finding_notes location`.

#### Scenario: Path resolved beside the declaring profile
- **WHEN** `profiles/base.yaml` declares `annotations: notes.yaml` and
  `repo/lattice.yaml` declares `extends: ../profiles/base.yaml`
- **THEN** the notes file read is `profiles/notes.yaml`

#### Scenario: Child declaration wins
- **WHEN** a parent declares `annotations: a.yaml` and its child declares
  `annotations: b.yaml`
- **THEN** only the child's `b.yaml` is read

#### Scenario: Missing notes file is reported
- **WHEN** `annotations` names a file that does not exist
- **THEN** validation reports one `NOTE_FILE_MISSING` warning naming the resolved path,
  and every other finding is reported as if no `annotations` were declared

#### Scenario: Resolved document omits annotations
- **WHEN** a profile declaring `annotations` is resolved for an adapter
- **THEN** the resolved document has no `annotations` key

#### Scenario: Unparseable notes file
- **WHEN** the notes file holds `- finding: {code: VACANCY` (unterminated)
- **THEN** lattice exits 2 naming the notes file

### Requirement: Note entry schema
Each list item in the notes file SHALL be a mapping with:

- `finding` (required): a mapping of `code` (string) and `node` (string).
- `note` (required): non-empty text.
- `until` (optional): an ISO calendar date `YYYY-MM-DD`, or a mapping of `pathway`
  (string naming one of the profile's `pathways`) and `position` (string).
- `by` (optional): a string.
- `at` (optional): an ISO calendar date `YYYY-MM-DD`.

An item that breaks the schema — a missing required key, an unknown key, a value of the
wrong type, a date that is not a real calendar date, a pathway the profile does not
declare — SHALL be reported as `NOTE_INVALID` naming the item and the fault, and SHALL
NOT attach. The other items SHALL still apply. A second item with the same `code` and
`node` as an earlier one SHALL be `NOTE_INVALID` naming the earlier item's line; the
earlier item applies.

`NOTE_*` findings SHALL carry the notes file as provenance, with the line on which the
item's top-level `-` begins, and no node. Items of a flow-style list (`[...]`) carry
line 0, the file as a whole.

Verified by: `cargo test -p latt --locked --test finding_notes schema`.

#### Scenario: A complete entry
- **WHEN** an item declares `finding: {code: VACANCY, node: UN-3}`,
  `until: 2026-12-31`, `by: jone`, `at: 2026-10-01` and `note: Lands with M2.`
- **THEN** it loads with no `NOTE_INVALID`

#### Scenario: Missing note text
- **WHEN** an item at line 7 declares `finding` and no `note`
- **THEN** one `NOTE_INVALID` error at the notes file, line 7, names the missing `note`,
  and the item attaches to nothing

#### Scenario: Unknown key
- **WHEN** an item declares `status: accepted`
- **THEN** `NOTE_INVALID` names `status` and lists the accepted keys

#### Scenario: Impossible date
- **WHEN** an item declares `until: 2026-02-30`
- **THEN** `NOTE_INVALID` names the value as not a calendar date

#### Scenario: Duplicate entry
- **WHEN** items at lines 1 and 6 both declare `finding: {code: VACANCY, node: UN-3}`
- **THEN** the line-6 item is `NOTE_INVALID` naming line 1, and the line-1 item's note
  attaches

### Requirement: Notes attach to findings
After validation has produced its findings and before strict promotion and suppression
are applied, every finding whose `code` and `node_id` equal an entry's `finding.code`
and `finding.node` SHALL carry that entry's note. One entry SHALL attach to every
finding it matches. A note SHALL NOT change a finding's severity, its suppression or the
exit code. A note SHALL attach to a suppressed finding as to any other. A finding with
no node SHALL never carry a note.

An entry that matches no finding, suppressed or not, SHALL be reported as `NOTE_UNUSED`
naming its code and node.

Verified by: `cargo test -p latt --locked --test finding_notes attach`.

#### Scenario: A note on a dangling reference
- **WHEN** `UN-3` has a `VACANCY` finding and an entry names `{code: VACANCY, node: UN-3}`
- **THEN** the finding carries the entry's note and is still an error

#### Scenario: One entry, several findings
- **WHEN** `EDGE_WHERE` reports three findings on `REQ-9` and one entry names
  `{code: EDGE_WHERE, node: REQ-9}`
- **THEN** all three carry the note

#### Scenario: Note on a suppressed finding
- **WHEN** a finding is both suppressed by a `SUPPRESS` entry and matched by a note
- **THEN** it stays suppressed, carries the note, and the entry is not `NOTE_UNUSED`

#### Scenario: Stale entry
- **WHEN** an entry names `{code: VACANCY, node: UN-3}` and `UN-3` has no `VACANCY`
- **THEN** one `NOTE_UNUSED` info finding names `VACANCY` and `UN-3`

### Requirement: Note expiry
An entry whose `until` is a date SHALL be expired when the validation date is after that
date; on the date itself it is not expired. The validation date SHALL be the current UTC
date unless the caller supplies one (see the graph-service capability).

An entry whose `until` names a pathway SHALL be expired when the register's current
position on that pathway is the named position or after it in the pathway's order.
The position SHALL be read from the register on every run. A position that is not in the
pathway's order, or a pathway the register supplies no position for, SHALL make the
entry `NOTE_INVALID` naming the pathway.

An expired entry that attaches SHALL be reported as `NOTE_EXPIRED` naming its code, node
and `until`, and its note SHALL still attach. An expired entry that attaches to nothing
is `NOTE_UNUSED` only.

Verified by: `cargo test -p latt --locked --test finding_notes expiry`.

#### Scenario: Date passed
- **WHEN** an attached entry declares `until: 2026-09-30` and the validation date is
  2026-10-01
- **THEN** one `NOTE_EXPIRED` warning names the entry, and the finding still carries
  the note

#### Scenario: On the date
- **WHEN** an attached entry declares `until: 2026-10-01` and the validation date is
  2026-10-01
- **THEN** no `NOTE_EXPIRED` is reported

#### Scenario: Pathway position reached
- **WHEN** an attached entry declares `until: {pathway: stage, position: M2}` and the
  register's current `stage` is `M2`
- **THEN** one `NOTE_EXPIRED` warning names the entry

#### Scenario: Pathway position ahead
- **WHEN** the same entry is read while the register's current `stage` is `M1`, before
  `M2` in the order
- **THEN** no `NOTE_EXPIRED` is reported

#### Scenario: Position not on the pathway
- **WHEN** an entry declares `until: {pathway: stage, position: M9}` and `M9` is not in
  the register's `stage` order
- **THEN** `NOTE_INVALID` names `stage` and `M9`

### Requirement: Note finding codes
The note codes SHALL have these default severities, and a profile MAY override each one
as it overrides any built-in code: `NOTE_FILE_MISSING` warning, `NOTE_INVALID` error,
`NOTE_UNUSED` info, `NOTE_EXPIRED` warning. `--strict` SHALL promote the warnings as it
promotes any warning. A `SUPPRESS` entry MAY suppress a note code. None of them SHALL
end the run: the exit code is decided from findings as for any other code.

Verified by: `cargo test -p latt --locked --test finding_notes codes`.

#### Scenario: Severity override
- **WHEN** a profile declares `- NOTE_EXPIRED: {severity: error}` and an entry has expired
- **THEN** the `NOTE_EXPIRED` finding is an error and the run exits 1

#### Scenario: Strict promotes a missing file
- **WHEN** `validate --strict` runs with a missing notes file
- **THEN** `NOTE_FILE_MISSING` is an error and the run exits 1

#### Scenario: Stale notes do not gate
- **WHEN** the only finding is `NOTE_UNUSED`
- **THEN** `validate` exits 0
