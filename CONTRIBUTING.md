# Contributing to lattice

Thanks for your interest in contributing. This guide covers the basics.

## Reporting bugs and requesting features

Open an issue on [GitHub Issues](https://github.com/jonelay/lattice/issues).
Include enough detail to reproduce the problem: the profile, register
format, adapter invocation, and the full output or traceback.

## Development setup

Requires Rust 1.95+ and Python 3.12+.

```bash
git clone https://github.com/jonelay/lattice.git
cd lattice
cargo build
python -m venv .venv
.venv/bin/pip install -e ".[test]"
```

## Running tests

Build the Rust binary first. The Python adapter suite needs it and
silently skips core-facing tests when it is absent.

```bash
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
.venv/bin/python -m pytest -q
```

The spec contract is also gated:

```bash
npx @fission-ai/openspec@1.7.0 validate --specs --strict
```

## Submitting changes

1. Open an issue first for anything beyond a small bug fix.
2. Fork the repo and create a branch from `main`.
3. Make your changes, add or update tests as needed.
4. Make sure all checks above pass.
5. Open a pull request against `main`.

Keep PRs focused - one logical change per PR.

## AI-assisted development

Contributors may use AI tools provided they review, understand, and accept
responsibility for the resulting contribution. Disclose material assistance
in the pull-request description or with a commit trailer:

```text
Assisted-by: <tool>
```

## License

By contributing you agree that your contributions are dual licensed under
the [Apache License, Version 2.0](LICENSE-APACHE) and the
[MIT license](LICENSE-MIT), without any additional terms or conditions.
