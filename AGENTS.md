# AGENTS.md

Lattice (`latt`) checks and queries the records a repository keeps in its own files
(requirements, specs, tests, issues) and the links between them. An adapter reads each
format; a YAML profile declares the kinds, IDs and rules.

- **Skill:** [`skills/latt/`](skills/latt/SKILL.md). Copy it into your agent's skills
  directory.
- **Docs:** [README.md](README.md), then [cli.md](docs/cli.md) for commands and exit
  codes, [profiles.md](docs/profiles.md), [adapters.md](docs/adapters.md) and
  [json.md](docs/json.md).

## Using it

1. Run the repository's own `latt` line, from its CI job, hook or AGENTS.md. Otherwise:
   `latt validate --profile <profile.yaml> --target . --format plain`.
2. Read the exit code first. `0`: no error-severity findings. `1`: error-severity
   findings, each at a file and line; fix the register there. `2`: Lattice could not
   run; fix the setup, since the run says nothing about the register.
3. Answer from the output. Open a register file only for what a finding does not carry.
4. Lattice only reads. Fix a finding in the register's files or with its own tool.
5. A script parses `--format json`; plain output may change between releases.
6. A `PARSE_ERROR` is input that went unread. Report it with any count over that file.
