# Roadmap & Future Performance Architecture

This document tracks verified architectural bottlenecks, platform-specific runtime behaviors, and future optimization milestones for the Takeout Restorer codebase.

---

---

## 1. Delivered Reliability & Concurrency Optimizations (v0.2.0 Milestone)

The following architectural optimizations and safety invariants were delivered and verified in **v0.2.0**:
- **ExifTool STDIN Batching:** Formats all per-file parameters into a single memory buffer with `-execute\n`, eliminating per-argument OS write syscalls.
- **Job Object Process Lifecycle:** Attaches persistent ExifTool child processes to an OS Job Object (`JobObjectSupervisor`), guaranteeing zero orphan processes on Windows exit.
- **Strict Process Registry:** Replaced unbounded PID vectors with a `HashSet<u32>` and explicit process unregistration upon worker drop, preventing PID reuse collateral terminations.
- **Output Finalization Safety:** Verifies physical existence of destination files before enqueuing `Completed` status, preserving staging directories if movement fails.
- **ExifTool Result Validation:** Inspects inner `ExifToolResult::Failed` responses, converting write failures into recoverable errors rather than false successes.
- **Levenshtein Tie Rejection:** Rejects ambiguous matches where two or more candidates have the exact same edit distance.
- **Auto-Heal Extension Correction:** Inspects 12-byte container magic bytes across all scanned media files, renaming misnamed containers (e.g. JPEG disguised as PNG).
- **Stream-Bounded JSON Readers:** Enforces a strict 10 MB limit before and during sidecar reading, preventing memory exhaustion attacks or OOMs.

---

## 2. Delivered Hardening & Audit Remediations (v0.2.2 Milestone)

The following fixes and reliability hardening measures were delivered and verified in **v0.2.2** following an independent system audit:
- **ExifTool Archive Extraction:** Fixed extraction prefix stripping so official standalone executables (`exiftool(-k).exe`) with no enclosing directory are extracted cleanly (`CRIT-02`).
- **Deadlock-Free Watch Channels:** Added channel closure state tracking and wake-up notifications on sender drop in `shared_ui::watch`, eliminating worker thread hangs (`CRIT-03`).
- **In-Place ZIP Relocation:** Relocated ZIP media extraction to the configured output directory in in-place mode to prevent files remaining trapped in temporary staging directories (`CRIT-01`).
- **Cross-Volume Timestamp Preservation:** Enforced `set_file_timestamps` directly on destination paths across different physical disk drives (`HIGH-01`).
- **Non-Blocking Telemetry Broadcaster:** Ensured event bus fan-out never deadlocks on saturated channel buffers while preserving active subscribers (`CRIT-04`).
- **Cooperative 2-Stage Process Supervision:** Added graceful SIGINT (Ctrl+C) handling in CLI mode with immediate-abort fallback, ensuring active write operations flush safely without database corruption (`CRIT-05`).
- **Engine Concurrency Guard:** Guarded against duplicate pipeline launches with RAII `RunningGuard` (`HIGH-04`) and ensured worker thread pool joining (`HIGH-05`).
- **Telemetry & View Model Binding:** Synchronized rolling event logs to the Slint activity panel (`MED-01`) and updated video extension classifications (`MED-02`).
- **Multi-Monitor Window Geometry:** Added multi-monitor coordinate clamping and non-maximized window geometry restoration (`LOW-01`, `LOW-02`).

---

## 3. Future Architectural & Throughput Milestones (v0.3.0 Milestone)

The following items represent confirmed throughput governors identified during concurrency profiling, scheduled for **v0.3.0**:

### 2.1 Remove Global `FILE_MOVE_MUTEX` Contention (Planned for v0.3.0)
- **Location:** [`crates/core/src/processor.rs`](../crates/core/src/processor.rs)
- **Current Behavior:** A single static global mutex (`static FILE_MOVE_MUTEX: Mutex<()>`) is acquired by worker threads when resolving filename collisions. Inside this critical section, `resolve_collision()` executes multiple filesystem `exists()` stat calls.
- **Impact:** Rayon worker threads serialize on disk I/O when completing files, bottlenecking multi-threaded throughput on high-speed NVMe drives.
- **Planned Fix:** Replace the global mutex with lock-free atomic file destination handling (e.g. `fs::OpenOptions::new().create_new(true)`) or per-directory partition locks.

### 2.2 Eliminate Sidecar JSON Disk I/O Roundtrip (Planned for v0.3.0)
- **Location:** [`crates/core/src/processor.rs`](../crates/core/src/processor.rs)
- **Current Behavior:**
  1. The producer thread extracts matched sidecar JSON files to `.staging/<run_id>/<media_id>/sidecar.json` on disk.
  2. The consumer thread reads `sidecar.json` from disk with `fs::read_to_string()`.
  3. The consumer then deletes the per-file staging directory with `fs::remove_dir_all()`.
- **Impact:** Every file triggers directory creation, JSON disk write, JSON disk read, and directory deletion (4 filesystem operations per file, generating ~200,000 metadata operations on a 50,000 file library).
- **Planned Fix:** Pass pre-extracted sidecar bytes in memory directly through the pipeline channel payload: `(MediaFile, PathBuf, Option<Vec<u8>>)`. Avoid creating disk directories for sidecars entirely.

### 2.3 Separate SQLite Read/Write Connections (Planned for v0.3.0)
- **Location:** [`crates/core/src/state_db.rs`](../crates/core/src/state_db.rs)
- **Current Behavior:** `StateDatabase` wraps a single SQLite connection in `Arc<Mutex<Connection>>`. The background `writer_loop` holds this lock during batch transactions, forcing reader queries and `try_mark_processing` in worker threads to wait.
- **Impact:** Even though SQLite operates in WAL mode (where readers do not block writers at the database engine level), the single Rust mutex forces single-threaded serialization in application code.
- **Planned Fix:** Maintain a dedicated write connection for `writer_loop` and a separate read-only connection pool for worker threads and UI queries.

---

## 3. Secondary Performance Improvements

| Component | Location | Issue | Proposed Optimization |
| :--- | :--- | :--- | :--- |
| **Processor** | `processor.rs:441-456` | Re-scans `(0..zip.len())` on every 1,000-file batch without checking `zip_json_index_cache`. | Check `zip_json_index_cache.get(&archive_path)` before parsing central directory. |
| **Telemetry** | `events.rs:103` | `Broadcaster::publish` takes exclusive `RwLock::write()` on every single `FileProcessed` event. | Acquire `RwLock::read()` for publishing; upgrade to `write()` only when removing disconnected subscribers. |
| **Database** | `state_db.rs:411` | Missing composite index on `(status, id)`. | Add `CREATE INDEX IF NOT EXISTS idx_media_status_id ON media_files(status, id);` for $O(\log N)$ keyset pagination. |
| **I/O Buffering** | `scanner.rs:85`, `processor.rs:438` | `File::open` passed unbuffered to `ZipArchive::new`. | Wrap in `BufReader::with_capacity(128 * 1024, file)` to reduce small OS read syscalls. |
| **Scanner** | `scanner.rs:213` | Discards `walkdir::DirEntry` cached metadata and calls `path.metadata()` stat syscall per file. | Reuse `entry.metadata()` directly during directory traversal. |
| **Matcher** | `matcher.rs:73-200` | Transient `String`/`Vec` heap allocations across 7 matching tiers (~150 allocations/file). | Use stack-allocated scratch buffers (`smallvec` / borrowed `&str` slicing) for candidate lookup keys. |

---

## 3. Platform & OS Runtime Verification Evidence

### 3.1 macOS Perl Runtime Availability (Dispelled Concern)
- **Investigation:** Explored whether macOS 12.3+ (Monterey) removed `/usr/bin/perl`, potentially breaking ExifTool process execution.
- **Verified Finding:** **Perl remains available on macOS.** Apple specifically removed **Python 2.7** (`/usr/bin/python`) in macOS 12.3, not Perl.
- **Citations & References:**
  - *Apple Developer Documentation:* [macOS Monterey 12.3 Release Notes](https://developer.apple.com/documentation/macos-release-notes/macos-12_3-release-notes)
    > *"Python 2.7 was removed from macOS in this release. Developers should use Python 3 or an alternative language instead."*
  - *CI Runner Verification:* GitHub Actions Run `31274695175` on `macos-latest` ran all 64 unit and integration tests to completion in 48.44s. Post-job runner logs confirmed ExifTool child processes executed under `perl5.34`:
    ```text
    Build & Test (macos-latest) Terminate orphan process: pid (15397) (perl5.34)
    Build & Test (macos-latest) Terminate orphan process: pid (15378) (perl5.34)
    ```

### 3.2 APFS Unicode Normalization (Verified & Calibrated as Low Impact)
- **Investigation:** Explored whether macOS APFS decomposes filenames to NFD, causing in-memory Rust `HashMap` lookups in `matcher.rs` to fail Tier 1/2 exact matching.
- **Verified Finding:**
  - Unlike legacy HFS+ (which was *normalization-modifying* to NFD), **APFS is normalization-preserving and normalization-insensitive**.
  - When Takeout ZIP archives are extracted on APFS, files are written and preserved in NFC (standard ZIP encoding).
  - Normal Takeout libraries match in Tier 1 ($O(1)$) with zero issues.
  - *Edge Case:* If files originate from legacy HFS+ drives or macOS Cocoa UI text inputs (which emit NFD), Rust's byte-exact `HashMap` misses Tier 1/2 and gracefully falls back to Tier 6 (Levenshtein fuzzy matching).
- **Citations & References:**
  - *Apple Developer Documentation:* [Apple File System Reference](https://developer.apple.com/documentation/foundation/filemanager/apple_file_system_guide)
  - *Codebase Test Audit:* [`crates/core/src/matcher.rs:494-521`](../crates/core/src/matcher.rs) covers multi-byte Unicode and Emoji truncation (`test_tier_5_unicode_progressive_truncation`), but uses byte-identical NFC string literals.
