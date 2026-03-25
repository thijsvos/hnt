# Contributing to HNT

Thanks for your interest in contributing! Here's how to get started.

## Development Setup

### Prerequisites

- [Rust](https://rustup.rs/) 1.88+

### Building

```bash
git clone https://github.com/thijsvos/hnt.git
cd hnt
cargo build
```

### Running

```bash
cargo run
```

Alternatively, use Docker:

```bash
docker compose run --rm -it dev cargo run
```

## Code Style

- Run `cargo fmt --all` before committing
- Run `cargo clippy -- -D warnings` and fix any warnings
- CI enforces both — your PR will fail if either has issues

## Submitting Changes

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/my-feature`)
3. Make your changes
4. Run `cargo fmt --all && cargo clippy -- -D warnings && cargo test`
5. Commit with a clear message
6. Open a pull request against `main`

## Reporting Issues

Open an issue at [github.com/thijsvos/hnt/issues](https://github.com/thijsvos/hnt/issues). Include:

- Steps to reproduce
- Expected vs actual behavior
- Terminal emulator and OS
