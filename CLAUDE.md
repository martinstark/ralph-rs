# ralph-rs

Autonomous AI agent loop CLI. Orchestrates Claude CLI to iteratively work on features defined in a PRD (JSON5), tracking progress until completion.

## Toolchain

- Rust 2021 edition
- tokio async runtime
- clap (CLI), serde + json5 (config), anyhow (errors)

## Commands

```bash
cargo check                      # type check
cargo test                       # run tests
cargo clippy -- -D warnings      # lint (strict)
cargo run -- --help              # CLI help
cargo run -- --init              # generate template PRD
```

## Git

Conventional commits: `feat:`, `fix:`, `refactor:`, `docs:`, `chore:`

## Code Style

Terse, idiomatic Rust. Prefer borrowing over cloning, functional over imperative, enums for state. Minimal comments.
