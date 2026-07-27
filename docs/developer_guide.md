# Developer Guide

## Prerequisites
- Rust 1.75.0 or later (configured automatically via `rust-toolchain.toml`).
- `cargo-packager` (for building OS installers).

## Building Locally
Run the app in CLI mode:
`cargo run --bin app -- /path/to/takeout --output /path/to/dest`

Run the app in GUI mode:
`cargo run --bin app --features gui -- --gui`

## Running Tests
Run the entire test suite:
`cargo test --workspace`

## Conventions
- Do not import `core` from `gui`.
- Use `UiCommand` to mutate state.
- Ensure all loops in `core` check the `cancel` and `pause` atomics.
