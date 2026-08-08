# Contributing to Google Photos Takeout Restorer

Thank you for your interest in contributing! We welcome bug reports, feature requests, documentation improvements, and code contributions.

---

## 🛠️ Development Environment Setup

1. **Install Rust:** Ensure you have Rust 1.75+ installed via [rustup](https://rustup.rs):
   ```bash
   rustup update stable
   ```
2. **Clone the Repository:**
   ```bash
   git clone https://github.com/GurutejaReddy-04/GooglePhotos-TakeoutRestorer-Rust.git
   cd GooglePhotos-TakeoutRestorer-Rust
   ```
3. **Install ExifTool:** Refer to the ExifTool installation steps in [README.md](README.md#%EF%B8%8F-system-requirements--dependencies) or allow the application to download it automatically.

---

## 🧪 Running Tests & Quality Checks

Before submitting a Pull Request, ensure all tests and quality checks pass:

> **Note: Independent Verification Status**
> - **Verified by audit:** The build, formatting, linting, and testing commands below have been independently verified to execute cleanly across the entire workspace on v0.1.8.

```bash
# 1. Format Check
cargo fmt --all -- --check

# 2. Clippy Lints (Must remain clean with zero warnings)
cargo clippy --workspace --all-targets --all-features -- -D warnings

# 3. Test Suite
cargo test --workspace
```

---

## 📐 Architecture & Coding Expectations

- **Clean Architecture Boundaries:**
  - `crates/core`: Core pipeline, matching algorithms, database, timezone, auto-heal, and ExifTool IPC logic. No UI dependencies allowed.
  - `crates/shared_ui`: ViewModels, UI commands, and event bridges.
  - `crates/gui`: Slint GUI templates and window lifecycle logic.
  - `crates/downloader`: ExifTool binary downloading, SHA-256 verification, and archive extraction.
  - `crates/app`: Core dispatcher integration and main CLI/GUI entry binary.
- **Performance & Protocol Guidelines:** Read [docs/PERFORMANCE.md](docs/PERFORMANCE.md) before modifying `exiftool.rs` or STDIN IPC handling.

---

## 📬 Submitting Pull Requests

1. **Create a Feature Branch:** `git checkout -b feature/my-cool-feature`
2. **Commit Changes:** Write clear, descriptive commit messages.
3. **Verify Safety:** Ensure no personal machine paths or secrets are committed.
4. **Submit PR:** Push to your fork and open a Pull Request against `master`.
