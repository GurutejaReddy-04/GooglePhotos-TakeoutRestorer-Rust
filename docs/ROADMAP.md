# Roadmap & Future Performance Architecture

This document tracks verified architectural bottlenecks, platform-specific runtime behaviors, and future optimization milestones for the Google Photos Takeout Restorer codebase.

---

## 1. High-Priority Architectural & Throughput Optimizations (v0.2.0 Milestone)

The following items represent confirmed throughput governors identified during deep concurrency and I/O profiling. While the current release (v0.1.8) is functionally stable and free of deadlocks, these optimizations will unlock substantial throughput gains (2x–4x on fast NVMe/SSD setups).

### 1.1 Remove Global `FILE_MOVE_MUTEX` Contention
- **Location:** [`crates/core/src/processor.rs:21, 768-771, 796-799`](../crates/core/src/processor.rs)
- **Current Behavior:** A single static global mutex (`static FILE_MOVE_MUTEX: Mutex<()>`) is acquired by every worker thread whenever moving files to `Completed/`, `Unmatched/`, or `Errors/`. Inside this critical section, `resolve_collision()` executes multiple filesystem `exists()` stat calls.
- **Impact:** All Rayon worker threads serialize on disk I/O when completing files, bottlenecking multi-threaded throughput on high-speed drives.
- **Planned Fix:** Replace the global mutex with lock-free atomic file destination handling (e.g. `fs::OpenOptions::new().create_new(true)`) or per-directory partition locks.

### 1.2 Eliminate Sidecar JSON Disk I/O Roundtrip
- **Location:** [`crates/core/src/processor.rs:550-558, 841-851, 380-386`](../crates/core/src/processor.rs)
- **Current Behavior:**
  1. The producer thread extracts matched sidecar JSON files to `.staging/<run_id>/<media_id>/sidecar.json` on disk.
  2. The consumer thread reads `sidecar.json` from disk with `fs::read_to_string()`.
  3. The consumer then deletes the per-file staging directory with `fs::remove_dir_all()`.
- **Impact:** Every file triggers 1 directory creation, 1 JSON disk write, 1 JSON disk read, and 1 recursive directory deletion (4 filesystem operations per file, generating ~200,000 metadata operations on a 50,000 file library, causing severe NTFS MFT and filesystem lock contention).
- **Planned Fix:** Pass pre-extracted sidecar bytes in memory directly through the pipeline channel payload: `(MediaFile, PathBuf, Option<Vec<u8>>)`. Avoid creating disk directories for sidecars entirely.

### 1.3 Separate SQLite Read/Write Connections
- **Location:** [`crates/core/src/state_db.rs:224, 365, 487`](../crates/core/src/state_db.rs)
- **Current Behavior:** `StateDatabase` wraps a single SQLite connection in `Arc<Mutex<Connection>>`. The background `writer_loop` holds this lock during 200-item batch transactions (`conn.lock()`), forcing reader queries and `try_mark_processing` in worker threads to wait.
- **Impact:** Even though SQLite operates in WAL mode (where readers do not block writers at the database engine level), the single Rust mutex forces full single-threaded serialization in application code.
- **Planned Fix:** Maintain a dedicated write connection for `writer_loop` and a separate read-only connection pool for worker threads and UI queries.

---

## 2. Secondary Performance Improvements

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
