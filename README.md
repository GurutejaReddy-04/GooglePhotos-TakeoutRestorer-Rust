# Google Photos Takeout Restorer

[![Latest Release](https://img.shields.io/github/v/release/GurutejaReddy-04/GooglePhotos-TakeoutRestorer-Rust)](https://github.com/GurutejaReddy-04/GooglePhotos-TakeoutRestorer-Rust/releases)
[![Release Workflow](https://github.com/GurutejaReddy-04/GooglePhotos-TakeoutRestorer-Rust/actions/workflows/release.yml/badge.svg)](https://github.com/GurutejaReddy-04/GooglePhotos-TakeoutRestorer-Rust/actions)
[![CI Build](https://github.com/GurutejaReddy-04/GooglePhotos-TakeoutRestorer-Rust/actions/workflows/ci.yml/badge.svg)](https://github.com/GurutejaReddy-04/GooglePhotos-TakeoutRestorer-Rust/actions)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust: 1.75+](https://img.shields.io/badge/Rust-1.75%2B-orange.svg)](https://www.rust-lang.org)
[![Platform: Windows | macOS | Linux](https://img.shields.io/badge/Platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](#supported-platforms)

> An ultra-fast, multi-threaded cross-platform Rust tool to seamlessly re-embed Google Photos Takeout metadata (EXIF, GPS, timestamps) back into original media files.

---

## 💡 The Problem

When you download your photo library from **Google Photos Takeout**, Google separates metadata from your actual photos and videos:
1. **Separated Metadata:** Date taken, descriptions, titles, and GPS coordinates are stripped from media files and placed into separate `.json` sidecar files.
2. **Reset Timestamps:** File creation and modification dates are reset to the exact moment you downloaded the archive.
3. **Truncated & Duplicate Filenames:** Google Takeout truncates long filenames (e.g. `IMG_20210503_120000(1).jpg` becomes `IMG_20210503_120000(.json`), causing standard metadata fixers to fail.

**Google Photos Takeout Restorer** solves this completely by intelligently matching sidecar JSONs to media files (even across truncated names, unicode titles, and subfolders), re-embedding EXIF metadata via ExifTool, fixing misnamed file extensions ("auto-healing"), and setting filesystem creation/modification timestamps.

> **Note: Independent Verification Status**
> - **Verified by audit:** Timestamp setting (OS filesystem times), Auto-healing logic, CLI flags, `.zip` archive extraction (including zip-bomb protection).
> - **Not independently verified:** While Unicode title matching and truncation handling are fully covered by the test suite, they have not been independently tested against real-world Google Takeout edge-cases beyond the included fixtures.

---

## 🎨 User Interface

![App Icon](assets/icon.png)



The app features both an intuitive, modern graphical interface (built with [Slint](https://slint.dev)) and a powerful headless CLI for automated/server environments.

---

## 💻 Supported Platforms

| Platform | Status | Package / Installer Availability | Recommended Installation |
| :--- | :--- | :--- | :--- |
| **Windows** (x86_64) | Tested (Primary) | Pre-built Installer (`.exe` / `.msi` / `.nsis`) | Download from Releases |
| **macOS** (x86_64 / ARM64) | Supported | Pre-built App Bundle (`.app` / `.dmg`) | Download from Releases |
| **Linux** (x86_64) | Supported | Pre-built Package (`.deb` / `.AppImage`) | Download from Releases |

---

## 🛠️ System Requirements & Dependencies

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

## 🚀 Installation & Building

### Pre-Built Installers
Download the latest pre-built installers for Windows, macOS, or Linux from the [Latest Release](https://github.com/GurutejaReddy-04/GooglePhotos-TakeoutRestorer-Rust/releases/latest) page.

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
3. **Configure Options:** Toggle options like Timezone resolution, GPS preservation, or Output Mode (`Copy` vs `In-Place`).
4. **Start Restoration:** Click **Start Processing** and monitor real-time progress and logs.

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

---

## 👤 Author & Credits

Created and maintained by **Guruteja Reddy Nallachi**:
- **GitHub:** [@GurutejaReddy-04](https://github.com/GurutejaReddy-04)
- **Email:** `159574479+GurutejaReddy-04@users.noreply.github.com`
