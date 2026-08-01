# Contributing

We welcome contributions!

## Process
1. Fork the repository.
2. Create a feature branch.
3. Submit a Pull Request.

## Rules
- All code must pass `cargo fmt` and `cargo clippy`.
- Any UI logic must reside exclusively in `crates/gui` or `crates/shared_ui`.
- Write tests for core behavior modifications.
- For performance architectures, IPC STDIN batching rules, and benchmarking guidelines, see [docs/PERFORMANCE.md](docs/PERFORMANCE.md).
