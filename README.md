# Google Photos Takeout Restorer

[![Latest Release](https://img.shields.io/github/v/release/GurutejaReddy-04/GooglePhotos-TakeoutRestorer-Rust)](https://github.com/GurutejaReddy-04/GooglePhotos-TakeoutRestorer-Rust/releases)
[![Release Workflow](https://github.com/GurutejaReddy-04/GooglePhotos-TakeoutRestorer-Rust/actions/workflows/release.yml/badge.svg)](https://github.com/GurutejaReddy-04/GooglePhotos-TakeoutRestorer-Rust/actions)
[![CI Build](https://github.com/GurutejaReddy-04/GooglePhotos-TakeoutRestorer-Rust/actions/workflows/ci.yml/badge.svg)](https://github.com/GurutejaReddy-04/GooglePhotos-TakeoutRestorer-Rust/actions)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust: 1.75+](https://img.shields.io/badge/Rust-1.75%2B-orange.svg)](https://www.rust-lang.org)
[![Platform: Windows | macOS | Linux](https://img.shields.io/badge/Platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](#platform-compatibility)

> A multi-threaded, cross-platform Rust tool that re-embeds Google Photos Takeout metadata (EXIF, GPS, timestamps) back into the original media files.

---

## The Problem

When you download your photo library from **Google Photos Takeout**, Google separates metadata from your actual photos and videos:
1. **Separated Metadata:** Date taken, descriptions, titles, and GPS coordinates are stripped from media files and placed into separate `.json` sidecar files.
2. **Reset Timestamps:** File creation and modification dates are reset to the exact moment you downloaded the archive.
3. **Truncated & Duplicate Filenames:** Google Takeout truncates long filenames (e.g. `IMG_20210503_120000(1).jpg` becomes `IMG_20210503_120000(.json`), causing standard metadata fixers to fail.

**Google Photos Takeout Restorer** solves this by matching sidecar JSONs to media files (handling truncated filenames, duplicate indexing, and nested album directories), re-embedding EXIF metadata via ExifTool, correcting misnamed file extensions via magic-byte inspection, and restoring original filesystem timestamps.

---

## User Interface

![App Icon](assets/icon.png)



The app features both an intuitive, modern graphical interface (built with [Slint](https://slint.dev)) and a powerful headless CLI for automated/server environments.

---

## Platform Compatibility

We distinguish between automated CI test execution and pre-built release package availability:

| Platform & Target Architecture | Automated CI Suite | Pre-Built Release Asset | Host Verification | Classification & Usage Notes |
| :--- | :---: | :---: | :---: | :--- |
| **Windows (`x86_64`)** | ✅ (`windows-latest`) | ✅ (`.exe` setup, `.msi`) | ⚠️ (CI Runner only) | **CI-Tested & Packaged** — Automated test suite passes; release installers (`.exe`, `.msi`) generated. |
| **Ubuntu Linux (`x86_64`)** | ✅ (`ubuntu-latest`) | ✅ (`.deb`, `.AppImage`, `.tar.gz`) | ⚠️ (CI Container only) | **CI-Tested & Packaged** — Automated test suite passes; native packages generated per release. |
| **macOS Apple Silicon (`aarch64`)** | ✅ (`macos-latest` ARM64) | ❌ (No native ARM64 asset) | ⚠️ (CI Runner only) | **CI-Tested / Rosetta Run** — Native test suite passes in CI; pre-built Intel `.dmg` runs via macOS Rosetta 2; native build from source supported. |
| **macOS Intel (`x86_64`)** | ⚠️ (Cross-compiled on ARM runner) | ✅ (`.dmg` bundle) | ⚠️ (CI Runner only) | **Packaged Target** — Built via `x86_64-apple-darwin` target for universal Intel and Rosetta 2 execution. |
| **Linux Other (`aarch64`)** | ❌ | ❌ | ❌ | **Source Only** — Portable Rust codebase; compiles from source with local Rust and ExifTool. |

### Technical Definitions:
- **Automated CI Suite:** Unit, integration, and formatting tests execute successfully inside GitHub Actions runners.
- **Pre-Built Release Asset:** Pre-compiled installer or bundle attached to official GitHub Releases.
- **Host Verification:** Execution status on virtualized CI runners versus untracked bare-metal hardware.
- **Rosetta Run / Source Build:** Native pre-compiled binaries are not currently published; execution relies on OS binary translation (e.g., Apple Rosetta 2) or compiling from source.

---

## System Requirements

1. **Rust Toolchain:** Rust 1.75 or later (for building from source).
2. **ExifTool:** Required for writing metadata into image/video files.
   - **Automatic (Recommended):** The application includes an embedded downloader that can automatically fetch and set up the correct binary for your OS.
   - **Manual System Installation:**
     - **Windows:** `winget install ExifTool` or `choco install exiftool`
     - **macOS:** `brew install exiftool`
     - **Linux (Ubuntu/Debian):** `sudo apt update && sudo apt install libimage-exiftool-perl`
     - **Linux (Arch):** `sudo pacman -S perl-image-exiftool`
3. **Perl (macOS/Linux):** Required when running ExifTool on Unix-like operating systems.

---

## Installation

### Pre-Built Installers
Download the latest pre-built installers for Windows, macOS, or Linux from the [Latest Release](https://github.com/GurutejaReddy-04/GooglePhotos-TakeoutRestorer-Rust/releases/latest) page.

> [!NOTE]
> **Windows SmartScreen Notice**  
> Because this is a new open-source project and binaries are unsigned, Windows SmartScreen may display an *"unrecognized publisher"* or *"Windows protected your PC"* warning. This is standard behavior for new open-source software and is **not** an indication of malware.
>
> **To proceed with installation:**
> 1. Click **More info** on the SmartScreen prompt.
> 2. Click **Run anyway**.
>
> You can independently verify the safety and integrity of all release binaries by inspecting our open-source codebase and the automated build logs in our public [GitHub Actions CI pipeline](https://github.com/GurutejaReddy-04/GooglePhotos-TakeoutRestorer-Rust/actions).

### Building from Source
Clone the repository and build using Cargo:

```bash
# Clone repository
git clone https://github.com/GurutejaReddy-04/GooglePhotos-TakeoutRestorer-Rust.git
cd GooglePhotos-TakeoutRestorer-Rust

# Build release binaries (CLI + GUI)
cargo build --release
```

The compiled binaries will be located at `target/release/TakeoutRestorer`.

---

## 📖 Usage Guide

### 1. Graphical Interface (GUI Mode)
Launch the GUI by double-clicking the application executable or running:

```bash
cargo run --release -- --gui
```

**Workflow:**
1. **Select Input:** Choose your extracted Google Photos Takeout folder(s) or `.zip` archives.
2. **Select Output:** Choose the destination folder for restored media files.
3. **Configure Options:** Toggle options like Timezone resolution, GPS restoration (re-embeds coordinates from `.json` sidecars; does not remove pre-existing camera-embedded EXIF data), or Output Mode (`Copy` vs `In-Place`).
4. **Start Restoration:** Click **Start Processing** and monitor real-time progress and logs.

> [!NOTE]
> **Metadata Restoration Scope**  
> Google Photos Takeout Restorer is a metadata restoration tool designed to restore metadata separated by Google Takeout into `.json` sidecar files. The GPS restoration toggle controls whether location data from sidecars is re-embedded; it is not a privacy/EXIF scrubber and does not strip pre-existing GPS coordinates embedded by the camera at capture time.

### 2. Command Line Interface (CLI Mode)
Run headlessly in server or automated batch environments:

```bash
# Basic CLI invocation
./target/release/TakeoutRestorer --output "/path/to/output" "/path/to/Google Photos Takeout"

# Use system ExifTool binary
./target/release/TakeoutRestorer --use-system-exiftool --output "/path/to/output" "/path/to/Takeout"
```

---

## 🤝 Contributing

Contributions are welcome! Please review [CONTRIBUTING.md](CONTRIBUTING.md) for development environment setup, coding guidelines, testing instructions, and performance architectures ([docs/PERFORMANCE.md](docs/PERFORMANCE.md)).

---

## 📜 License

This project is licensed under the **MIT License**. See [LICENSE](LICENSE) for details.

## 📚 Project Documentation & Governance

For in-depth guides on architecture, performance, configuration, and release procedures:
- 📋 **[CHANGELOG.md](CHANGELOG.md)** — Chronological release history and upcoming milestones.
- 🗺️ **[ROADMAP.md](docs/ROADMAP.md)** — Architectural bottlenecks, performance audit findings, and v0.2.0 milestones.
- 📦 **[Build & Verification Guide](docs/build_guide.md)** — Deterministic build steps, SHA-256 verification, and GitHub Artifact Attestation checks.
- 🏷️ **[Versioning Policy](docs/versioning_policy.md)** — Semantic Versioning guidelines and release channels.
- 🚀 **[Release Process](docs/release_process.md)** — Step-by-step CI/CD release workflow and QA checklist.
- ⚡ **[Performance & Benchmarking](docs/PERFORMANCE.md)** — IPC STDIN protocols, line-ending quirks, and benchmark datasets.
- 🏗️ **[Architecture Guide](docs/architecture_guide.md)** — Event-driven MVVM design and crate structure.

---

## 📜 Project Origins & Architectural Evolution

This Rust implementation is the high-performance successor to the original Python desktop prototype:
👉 **[GooglePhotos-TakeoutRestorer (Python)](https://github.com/GurutejaReddy-04/GooglePhotos-TakeoutRestorer)**

### Systems-Engineering Comparison: Python Reference vs Rust Implementation

The following table contrasts the architectural design choices between the original Python prototype and the Rust rewrite. Qualitative claims are grounded in repository code and build configurations; empirical benchmarks follow the methodology defined in [`docs/PERFORMANCE.md`](docs/PERFORMANCE.md):

| Dimension | Python Reference (`GooglePhotos-TakeoutRestorer`) | Rust Implementation (`GooglePhotos-TakeoutRestorer-Rust`) | Evidence / Technical Basis |
| :--- | :--- | :--- | :--- |
| **Concurrency Model** | `ThreadPoolExecutor` worker pool; subject to Python Global Interpreter Lock (GIL) contention during CPU-bound string normalization. | Multi-threaded work distribution via `Rayon` with `crossbeam-channel` pipeline orchestration. | Code (`crates/core/src/processor.rs` vs Python `core/processor.py`) |
| **IPC Protocol & Process Lifecycle** | Python subprocess pipe with per-file command invocations. | Single-pass pre-buffered stdin command batches terminating with `-execute\n` and strict LF line terminators. | Code (`crates/core/src/exiftool.rs`, documented in `docs/PERFORMANCE.md`) |
| **Packaging & Distribution** | PyInstaller single-file archive; extracts Python runtime, DLLs, and bytecode to `%TEMP%` on each execution. | Single self-contained native binary packaged into native platform installers (`.exe`/`.msi` via NSIS, `.deb`, `.dmg`). | Build config (`cargo-packager` vs `GooglePhotosTakeoutRestorer.spec`) |
| **UI Architecture** | CustomTkinter with Python main thread polling. | Declarative Slint UI compiled directly to native machine code with GPU/software rendering fallback. | Code (`crates/gui` vs Python `ui/`) |
| **Sidecar Matching Engine** | Linear file system scans and regex evaluations. | Multi-tier candidate index ($O(1)$ nested `HashMap` lookups across 7 fallback tiers) with rapid Levenshtein distance fallback. | Code (`crates/core/src/matcher.rs`) |
| **Startup Latency** | Includes PyInstaller decompression and CPython module initialization. | Instant native OS binary load (zero runtime unpack overhead). | *Empirical Benchmark Pending (per docs/PERFORMANCE.md)* |
| **Memory Footprint** | CPython runtime heap, garbage collection structures, and loaded Python libraries. | Compact native memory layout with statically typed structures, eliminating CPython runtime and cyclic GC overhead. | *Empirical Benchmark Pending (per docs/PERFORMANCE.md)* |
| **Processing Throughput** | Bounded by GIL and subprocess dispatch serialization. | Bounded by disk I/O and ExifTool worker pool saturation. | *Empirical Benchmark Pending (1,000+ file dataset required)* |

Both repositories are publicly maintained to document this technical evolution.

---

## 👤 Author & Credits

Created and maintained by **Guruteja Reddy Nallachi**:
- **GitHub:** [@GurutejaReddy-04](https://github.com/GurutejaReddy-04)
- **Email:** `159574479+GurutejaReddy-04@users.noreply.github.com`
