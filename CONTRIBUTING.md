# Contributing

## Setup

```bash
git clone https://github.com/onblueroses/claude-code-wrapped
cd claude-code-wrapped
cargo build --release
```

Requires Rust 1.85 or newer. `Cargo.toml` declares this minimum and CI runs
`cargo check --all-targets --locked` on Rust 1.85.0.

The complete compatibility-capture suite requires the repository-pinned Rust/Cargo 1.95.0
toolchain; CI installs that exact version so generated API and Serde artifacts stay comparable.

## Running tests

```bash
cargo check --all-targets --locked # with Rust 1.85.0
cargo build --release --locked
cargo test --all-targets --locked
cargo fmt --all -- --check
cargo clippy --manifest-path tests/support/report-source-audit/Cargo.toml --all-targets --locked -- -D warnings
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
shellcheck scripts/*.sh
scripts/generate-readme-assets.sh --verify-manifest
scripts/verify-readme-assets-container.sh
```

Run every command above before release. The release workflow repeats these gates, requires a
`vX.Y.Z` tag to match `package.version` exactly, and grants write access only to the job that
publishes artifacts after verification and cross-platform packaging succeed.

The integration and Phase 4 product tests use checked synthetic fixtures or temporary
synthetic homes—never real Claude Code history. Run
`cargo test --test phase4_product --locked --offline -- --test-threads=1` when changing a
renderer, privacy profile, output flag, JSON error, browser launcher, or file transaction.

The README screenshots come from the checked synthetic fixture and the production HTML/card
renderers. `scripts/generate-readme-assets.sh --verify-manifest` performs the fast drift
check. `scripts/verify-readme-assets-container.sh` performs the byte-for-byte recapture in
the digest-pinned browser/font environment; pass `--regenerate` to that wrapper for an
intentional refresh. The exact package and source hashes are recorded in
`assets/README-ASSETS.sha256`. Never use real Claude history for repository screenshots.

## Adding a feature

- Readers live in `src/readers/` — anything that touches the filesystem
- Analyzers live in `src/analyzers/` — pure computation over the parsed data
- Renderers live in `src/renderers/` — turn the report struct into output

Keep the share card (`renderers/share_card.rs`) behind its aggregate-only typed DTO. The
HTML template must never accept `Report`, project/session/request/account identifiers,
paths, prompts/content, commands, or diagnostic fields. Preserve the
`render_share_card(&Report)` compatibility wrapper as an immediate projection boundary.

## PRs

- One logical change per PR
- `cargo clippy` must pass cleanly before opening
- If you're adding a new output flag, default it to off, add it to JSON-mode conflicts,
  prove one-value stdout/no-side-effect errors, and keep browser/file work after transaction
  commit
