# Running Lattice in CI and hooks

Recipes for running `latt` in your own repository's pre-commit hook and CI jobs,
from a release install with `latt` on PATH. This repository's tests run every recipe
below against a clean register, one with a warning, one with an error and one whose
profile cannot load, and check the outcome each recipe states.

The examples assume a profile at `lattice.yaml` in the repository root and the built-in
`md` adapter. Substitute your own `--profile`, `--adapter` and `--target`.

## Exit codes decide everything

`latt validate` exits 0, 1 or 2 ([cli.md](cli.md#exit-codes)). A recipe has to keep
1 and 2 apart: 1 is a finding to read, and 2 is a broken setup (an unreadable profile, a
missing adapter) that tells you nothing about the register. A shell also has its own
statuses. 127 means `latt` is not on PATH, and 126 means it is not executable.
Treating any of these like a finding hides a broken setup, and treating them like
success is worse.

`query` commands never exit 1, so their only failure is 2.

## Choose a posture

| Posture | Findings | Lattice could not run | Use it when |
|---|---|---|---|
| advisory | printed, pass | printed, pass | your blocking gates must stay hermetic: Lattice reports and passes |
| setup gate | printed, pass | stops | you want findings visible and a broken setup noticed |
| error gate | errors stop, warnings pass | stops | dangling references and parse errors should block |
| strict gate | warnings stop too | stops | the register is clean and should stay so |
| profile test | any change to the pinned codes and node IDs stops | stops | you want your profile's behaviour on a fixture register pinned |

The pre-commit hook and the GitLab job below are setup gates. Every posture that stops on
something is a gate you opt into, and a repository whose blocking checks must be hermetic
uses the advisory form.

## Pre-commit hook

A setup gate. Save it as `.git/hooks/pre-commit` and make it executable. Findings print
and the commit goes ahead. If Lattice cannot run, the hook refuses the commit and prints
why: a refused `git commit` exits 1 whatever code the hook returned, so the hook's message
is the only place the cause shows. `git commit --no-verify` skips the hook.

The hook validates the working tree, not what is staged. Git runs hooks with the
committer's PATH, which some GUI clients truncate. If `latt` is not found there, the
hook refuses the commit and says so.

<!-- recipe: pre-commit-hook posture=setup kind=hook clean=commit warning=commit broken=commit unloadable=reject -->
```sh
#!/bin/sh
latt validate --profile lattice.yaml --adapter md --target .
status=$?
case $status in
    0|1) exit 0 ;;
esac
echo "lattice could not run (exit $status); fix the setup or commit with --no-verify" >&2
exit 1
```

## GitLab CI job

A setup gate. Exit 1 is allowed to fail, so findings show as a warning on the pipeline
without failing it, and any other non-zero exit fails the job. GitLab runs `script` lines
with `set -eo pipefail`. The job needs `latt` on PATH in its image, for example an
image with a release install's `bin/` added.

<!-- recipe: gitlab-job posture=setup kind=gitlab clean=pass warning=pass broken=warn unloadable=fail -->
```yaml
lattice:
  script:
    - latt validate --profile lattice.yaml --adapter md --target .
  allow_failure:
    exit_codes: [1]
```

## Advisory

Never fails. It prints findings, and it also prints a line saying Lattice could not run,
so a broken setup is visible even though nothing stops. The `|| status=$?` keeps a shell
that runs with `set -e`, as GitLab's does, from stopping at Lattice's exit status.

<!-- recipe: advisory posture=advisory kind=sh clean=0 warning=0 broken=0 unloadable=0 -->
```sh
status=0
latt validate --profile lattice.yaml --adapter md --target . || status=$?
if [ "$status" -gt 1 ]; then echo "lattice could not run (exit $status); findings unknown"; fi
exit 0
```

## Error gate

Error-severity findings fail; warnings print and pass. This is `latt validate` as it
is, with its exit status passed through.

<!-- recipe: error-gate posture=error kind=sh clean=0 warning=0 broken=1 unloadable=2 -->
```sh
latt validate --profile lattice.yaml --adapter md --target .
```

## Strict gate

`--strict` makes warnings fail too ([cli.md](cli.md#exit-codes)). Hints never do.

<!-- recipe: strict-gate posture=strict kind=sh clean=0 warning=1 broken=1 unloadable=2 -->
```sh
latt validate --strict --profile lattice.yaml --adapter md --target .
```

## JSON check

Read the findings as JSON ([json.md](json.md) lists every key) with nothing beyond
`python3`. Do not pipe Lattice straight into the parser. At exit 2 `validate` writes
nothing to stdout, the parser fails on the empty input, and the pipeline reports the
parser's status, which turns "could not run" into an ordinary failure. Capture the output and the
status first:

<!-- recipe: json-check posture=error kind=sh clean=0 warning=0 broken=1 unloadable=2 -->
```sh
status=0
out=$(latt validate --profile lattice.yaml --adapter md --target . --format json) || status=$?
if [ "$status" -gt 1 ]; then exit "$status"; fi
printf '%s' "$out" | python3 -c 'import json, sys; errors = [f for f in json.load(sys.stdin)["findings"] if f["severity"] == "error" and not f.get("suppressed")]; print(len(errors), "error(s)"); sys.exit(1 if errors else 0)'
```

A suppressed finding stays in the JSON with `"suppressed": true`, and the parser skips it
as Lattice's own exit code does ([profiles.md](profiles.md#suppress)).

## Profile test

Test your own profile the way this repository tests Lattice: keep a small fixture
register beside the profile, run `validate --format json` over it, project the findings
to their codes and node IDs, and compare with an expected file committed beside the
fixture (`expected-findings.txt`, one `CODE NODE_ID` per line, `-` for a finding with no
node, sorted). Message text is left out of the projection on purpose, so a wording change
in a Lattice release does not fail it; a new or vanished finding does. Regenerate the
expected file when the profile or the fixture changes deliberately, and read the diff.

<!-- recipe: profile-test posture=test kind=sh clean=0 warning=1 broken=1 unloadable=2 -->
```sh
status=0
out=$(latt validate --profile lattice.yaml --adapter md --target . --format json) || status=$?
if [ "$status" -gt 1 ]; then exit "$status"; fi
printf '%s' "$out" | python3 -c 'import json, sys; got = sorted(f["code"] + " " + (f.get("node_id") or "-") for f in json.load(sys.stdin)["findings"]); want = open("expected-findings.txt").read().split("\n"); new = [g for g in got if g not in want]; gone = [w for w in want if w and w not in got]; [print("new:", g) for g in new]; [print("gone:", w) for w in gone]; sys.exit(1 if new or gone else 0)'
```

## Beside cargo-semver-checks

A Rust project often runs Lattice next to `cargo semver-checks`. Their exit codes do not
line up: Lattice exits 0 clean, 1 on error-severity findings, 2 when it could not run;
cargo-semver-checks exits 0 on pass, 100 on a lint failure, 101 when it could not
complete. Map each tool's "could not run" to a stopped job separately rather than
through one shared threshold. The severity words differ too. A cargo-semver-checks
`warn` lint never stops its run, while a Lattice `warning` stops a `--strict` run. A
profile that mirrors a `warn` level with `severity: warning` therefore gates under
`--strict` where the lint would not.

## Merge-request report

What the change does to the register: nodes and edges added, removed and changed between
the merge request's base and its head. `query diff` never exits 1, so this reports and
stops only when Lattice could not run. Pair it with one of the gates above if findings
should block. The job needs the base commit, so in GitLab set `GIT_DEPTH: 0` or fetch
the base before running it. The profile and adapter are the current checkout's, applied
to both revisions. A diff does not say whether an added edge resolves: a reference that
broke shows as one edge removed and one added whose target names no node. Each
revision's findings are its adapter's own, so the `VACANCY` for that target comes from
the gate; the diff shows only the edge.

<!-- recipe: mr-report posture=report kind=mr clean=0 warning=0 broken=0 unloadable=2 -->
```sh
latt query diff "$CI_MERGE_REQUEST_DIFF_BASE_SHA" HEAD --profile lattice.yaml --adapter md --target .
```

## Python

The `--format json` recipe and the Python programs (the `openspec` adapter and the
`latt-suggest` sidecar) need Python 3.12 or later. They use the standard library only
and run with `python3` from PATH.
