# Developer Guide

## Prerequisites
- Rust 1.88.0 or later (configured automatically via `rust-toolchain.toml`).
- On Windows, Visual Studio Build Tools with the **Desktop development with C++** workload (provides `link.exe`).
- `cargo-packager` (for building OS installers).

## Building Locally
Run the app in CLI mode:
`cargo run --bin TakeoutRestorer -- /path/to/takeout --output /path/to/dest`

Run the app in GUI mode:
`cargo run --bin TakeoutRestorer --features gui -- --gui`

## Running Tests
Run the entire test suite:
`cargo test --workspace`

## Conventions
- Do not import `core` from `gui`.
- Use `UiCommand` to mutate state.
- Ensure all loops in `core` check the `cancel` and `pause` atomics.
