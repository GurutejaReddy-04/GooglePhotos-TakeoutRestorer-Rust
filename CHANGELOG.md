# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.9] - 2026-08-10

### Fixed
- **GPS & Timezone Restoration Toggle Enforcement**: Fixed an issue where `gps_enabled` and `timezone_enabled` settings in `config.toml` and GUI were ignored during processing (always restored GPS/localized timezone regardless of user preference).
  > **Note**: These toggles control whether metadata is restored **from** the Takeout sidecar; they do not strip pre-existing camera-embedded EXIF data.

## [0.1.8] - 2026-08-09

### Changed
- **Zip Extraction & Pipeline Throughput**: Single-pass `ZipArchive` handle reuse per batch in producer loop, eliminating central directory re-parsing. Increased pipeline channel capacity to 2,000 items.
- **$O(1)$ Hash Matcher Indexing**: Refactored `Matcher` candidate index to nested `HashMap` for $O(1)$ candidate lookups across all 7 matching tiers.
- **Sidecar JSON Staging Fast-Path**: Pre-extracts JSON sidecars during producer pass for sub-millisecond consumer disk reads.
- **Lock Scope Narrowing**: Restricted `FILE_MOVE_MUTEX` scope to collision resolution and added 1MB buffered cross-volume file copying for cross-drive transfers.
- **CLI Progress UX**: Updated CLI progress loop to render status in-place using carriage returns (`\r`) and `stdout().flush()`.
- **Code Style**: Applied consistent `rustfmt` formatting across core pipeline modules.

### Fixed
- **Windows System Drive Hangs**: Replaced system-wide `sysinfo` drive list enumeration with Win32 `GetDiskFreeSpaceExW` API (`windows-sys` v0.61) on Windows.
- **GUI Font Glyph Rendering**: Replaced missing Unicode Dingbats character `U+2715` (`✕`) with `U+00D7` (`×`) in `MainWindow.slint` to resolve box placeholder (`tofu`) rendering on default Windows system fonts.
- **Log Cleanup**: Stripped verbose diagnostic debug `eprintln!` statements from ExifTool process engine pool.

## [0.1.7] - 2026-08-02
### Fixed
- Renamed binary from `app` to `TakeoutRestorer` and fixed Packager configurations.
- Corrected `SECURITY.md` supported version table.
- Removed unused dependencies (`thiserror`, `tracing`) from `app` and `downloader` crates to reduce compilation bloat.

## [0.1.0] - 2026-08-01
### Added
- Core pipeline with ExifTool metadata resolution.
- Intelligent Auto-Heal for mismatched file extensions.
- Timezone correction leveraging GPS coordinates.
- Slint-based GUI implementation.
- Real-time logging and progress virtualization.
- Windows, macOS, and Linux multi-architecture support.
