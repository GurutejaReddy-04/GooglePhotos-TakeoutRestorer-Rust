# Build Guide

## Release Builds
The official releases are generated automatically via GitHub Actions upon tagging a commit with a `v*` prefix.
We utilize `cargo-packager` to generate OS-native installer bundles.

## Manual Build
To manually generate a release artifact on your host OS:
1. Ensure `rustup` is installed. The repository's `rust-toolchain.toml` will automatically select `1.75.0`.
2. Install the packager: `cargo install cargo-packager --locked`
3. Build the binary: `cargo build --release`
4. Bundle: `cargo packager --release`

Artifacts will be located in `target/release/bundle`.
