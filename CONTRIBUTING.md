# Contributing

## Setup

```bash
git clone https://github.com/onblueroses/claude-code-wrapped
cd claude-code-wrapped
cargo build --release
```

Requires Rust 1.70+. No other dependencies — the build is fully offline-capable via the vendored `glob` crate.

## Running tests

```bash
cargo test                      # unit + integration tests
cargo clippy -- -W clippy::all  # lints (CI enforces this)
```

The integration tests in `tests/integration_test.rs` use in-memory fixtures — no real Claude Code history required.

## Adding a feature

- Readers live in `src/readers/` — anything that touches the filesystem
- Analyzers live in `src/analyzers/` — pure computation over the parsed data
- Renderers live in `src/renderers/` — turn the report struct into output

Keep the share card (`renderers/share_card.rs`) free of project names and file paths. It's designed to be screenshot-safe.

## PRs

- One logical change per PR
- `cargo clippy` must pass cleanly before opening
- If you're adding a new output flag, default it to off and gate all file writes behind `!cli.json`
