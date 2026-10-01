# Build & Verification Guide

## Release Builds
The official releases are generated automatically via GitHub Actions upon tagging a commit with a `v*` prefix. We utilize `cargo-packager` to generate OS-native installer bundles (`.exe`/`.msi` on Windows, `.dmg` on macOS, and `.deb`/`.AppImage` on Linux).

## Manual Build from Source
To manually compile and generate release artifacts on your host OS:
1. Ensure `rustup` is installed. The repository's `rust-toolchain.toml` tracks `stable`, while CI builds against MSRV `1.75.0`.
2. Install the packager tool:
   ```bash
   cargo install cargo-packager --locked
   ```
3. Build the release binary:
   ```bash
   cargo build --release --locked
   ```
4. Bundle platform installers:
   ```bash
   cargo packager --release
   ```

Artifacts will be located in `target/release/` and `target/release/bundle/`.

---

## Release Artifact Verification

### 1. File Integrity Verification (SHA-256 Checksums)
Every published release includes a consolidated `SHA256SUMS` manifest containing cryptographic hashes of all platform packages:

- **Windows (PowerShell):**
  ```powershell
  Get-FileHash -Algorithm SHA256 .\TakeoutRestorer_0.1.9_x64-setup.exe
  ```
- **Linux:**
  ```bash
  sha256sum -c SHA256SUMS --ignore-missing
  ```
- **macOS:**
  ```bash
  shasum -a 256 TakeoutRestorer_0.1.9_x86_64.AppImage
  ```

### 2. Supply-Chain Provenance (GitHub Artifact Attestations)
Official release assets have Sigstore-backed GitHub Artifact Attestations establishing their build provenance (`actions/attest@v4`). You can cryptographically verify that a downloaded binary was produced by this repository's automated CI workflow without tampering:
```bash
gh attestation verify TakeoutRestorer_0.1.9_x64-setup.exe --repo GurutejaReddy-04/GooglePhotos-TakeoutRestorer-Rust
```

---

## Deterministic Source Build Procedure
To reproduce binaries from source under controlled conditions:
1. Ensure the Rust compiler is installed.
2. Build with Cargo's locked dependency resolution to strictly adhere to `Cargo.lock`:
   ```bash
   cargo build --release --locked
   ```
