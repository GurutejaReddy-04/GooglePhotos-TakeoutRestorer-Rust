# Google Photos Takeout Restorer

[![CI Build](https://github.com/GurutejaReddy-04/Google-Photos-Takeout-Restorer-Rust/workflows/CI/badge.svg)](https://github.com/GurutejaReddy-04/Google-Photos-Takeout-Restorer-Rust/actions)
[![License: MIT / Apache-2.0](https://img.shields.io/badge/License-MIT%20%2F%20Apache--2.0-blue.svg)](LICENSE)
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

---

## 🎨 User Interface

![App Icon](assets/icon.png)

*(Slint UI Screenshots & Demo GIF: TODO)*

The app features both an intuitive, modern graphical interface (built with [Slint](https://slint.dev)) and a powerful headless CLI for automated/server environments.

---

## 💻 Supported Platforms

| Platform | Status | Package / Installer Availability | Recommended Installation |
| :--- | :--- | :--- | :--- |
| **Windows** (x86_64) | Tested (Primary) | Pre-built Installer (`.exe` / `.msi`) / Portable | Releases / Build from source |
| **macOS** (x86_64 / ARM64) | Supported | Build from source *(Packager installer in progress)* | `cargo build --release` |
| **Linux** (x86_64) | Supported | Build from source *(Packager installer in progress)* | `cargo build --release` |

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
Check the [Releases](https://github.com/GurutejaReddy-04/Google-Photos-Takeout-Restorer-Rust/releases) section for official releases and installers.

### Building from Source
Clone the repository and build using Cargo:

```bash
# Clone repository
git clone https://github.com/GurutejaReddy-04/Google-Photos-Takeout-Restorer-Rust.git
cd Google-Photos-Takeout-Restorer-Rust

# Build release binaries (CLI + GUI)
cargo build --release
```

The compiled binaries will be located at `target/release/app`.

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
./target/release/app --output "/path/to/output" "/path/to/Google Photos Takeout"

# Use system ExifTool binary
./target/release/app --use-system-exiftool --output "/path/to/output" "/path/to/Takeout"
```

---

## 🤝 Contributing

Contributions are welcome! Please review [CONTRIBUTING.md](CONTRIBUTING.md) for development environment setup, coding guidelines, testing instructions, and performance architectures ([docs/PERFORMANCE.md](docs/PERFORMANCE.md)).

---

## 📜 License

This project is dual-licensed under either the **MIT License** or **Apache License (Version 2.0)** at your option. See [LICENSE](LICENSE) for details.

---

## 👤 Author & Credits

Created and maintained by **Guruteja Reddy Nallachi**:
- **GitHub:** [@GurutejaReddy-04](https://github.com/GurutejaReddy-04)
- **Email:** `159574479+GurutejaReddy-04@users.noreply.github.com`
