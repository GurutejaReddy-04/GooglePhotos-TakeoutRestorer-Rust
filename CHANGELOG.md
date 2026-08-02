# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
