# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1] - 2026-10-14

### Fixed
- **Brand Name & Trademark Alignment**: Renamed product to **Takeout Restorer** across desktop GUI, CLI descriptions, and package configurations, and added legal trademark disclaimer for Google Photos and Google Takeout.
- **Custom NSIS Installer Branding**: Configured custom application `.ico` icon and dedicated branding header/sidebar bitmaps for Windows installer setup wizards, replacing the default NSIS logo.
- **Multi-Resolution Linux App Icons**: Added multi-resolution PNG icons (`32x32`, `64x64`, `128x128`, `256x256`, `512x512`) for FreeDesktop standard compliance.
- **Data Integrity & Move Finalization**: Ensured staging file moves return `Result<PathBuf, AppError>` and verify final destination validity before recording a file as Completed; staging files are preserved upon filesystem failures.
- **ExifTool Error Propagation**: Fixed an issue where ExifTool write failures (`ExifToolResult::Failed`) were swallowed, now correctly updating state and surfacing error diagnostics.
- **MSRV Compatibility**: Retained full compatibility down to Rust 1.75+ by removing dependencies on Rust 1.83+ APIs.

## [0.2.0] - 2026-10-13

### Added
- **Continuous Streaming Pipeline**: Producer-consumer architecture with backpressure channel buffers, eliminating full upfront directory scan delays before media restoration begins.
- **Lock-Free Atomic Destination Handling**: Replaced static global file move mutex with atomic filesystem operations and per-destination isolation, unlocking maximum parallel throughput for multi-core Rayon workers.
- **In-Memory Microsecond Sidecar Processing**: Replaced recursive disk staging folder roundtrips with direct in-memory sidecar payload streaming, saving hundreds of thousands of filesystem syscalls on large libraries.
- **Cross-Platform Anti-Throttling Execution Governor & Native Sleep Inhibitor**:
  - Automatically inhibits OS sleep, display sleep, and CPU energy throttling when High Performance Mode is active.
  - Native Windows `SetThreadExecutionState` (`ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_AWAYMODE_REQUIRED`).
  - Native macOS `IOPMAssertionCreateWithName` (`kIOPMAssertionTypePreventUserIdleSystemSleep`).
  - Native Linux FreeDesktop ScreenSaver / systemd-inhibit D-Bus support.
- **Windows Job Object Child Process Supervisor**: Windows Job Object configuration with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, guaranteeing that spawned ExifTool worker processes terminate immediately with zero orphaned or zombie processes on application exit or unexpected crashes.
- **Multi-Row SQLite WAL Batching**: High-throughput database update batching (`write_batch`), reducing disk sync operations by over 80% while preserving crash resilience.
- **Comprehensive UI/UX & Typography Overhaul**:
  - Upgraded global typography scale with strict `12px` minimum font size (`text_xs: 12px`, `text_sm: 13px`, `text_base: 14px`, `text_lg: 17px`, `text_2xl: 28px`), eliminating all small/unreadable text.
  - Scaled control dimensions (`control_height: 36px`, `compact_control_height: 32px`).
  - Expanded left configuration sidebar to `380px` for comfortable setting label fit without text clipping.
  - Added dedicated `⚡ Turbo` indicator badge on header bar reflecting High Performance Mode status.
  - Modernized Output Mode Selector (`64px` min-height) with crisp radio indicators.
  - Modernized Metric Tiles (`80px` height) with high-contrast label and value hierarchy.
  - Embedded real-time `LogPanel` telemetry into the Live Monitor tab with 1-click clipboard path copying.
  - Enhanced Results Inspector with `56px` row heights, `14px` filename text, and distinct status dot indicators.

### Changed
- Increased buffered file I/O streams to 1MB chunks for large video and high-resolution photo transfers across storage volumes.
- Enhanced match logging to output explicit counts of indexed sidecars and matched pairs in real time.

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
