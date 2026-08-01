# Performance & IPC Benchmarking Guidelines

This document outlines key performance-critical architectures, IPC protocols, and benchmarking guidelines for the file processing pipeline.

---

## 1. Load-Bearing ExifTool STDIN Command Batching

In [`crates/core/src/exiftool.rs`](../crates/core/src/exiftool.rs), `ExifToolEngine::execute` constructs and formats all per-file command arguments into a single string buffer terminating with `-execute\n`, writing the entire batch to process `ChildStdin` in a single `write_all()` call before calling `flush()`:

```rust
let mut batch = String::with_capacity(args.len() * 64 + 16);
for arg in args {
    let sanitized_arg = arg.replace(['\r', '\n'], " ");
    batch.push_str(&sanitized_arg);
    batch.push('\n');
}
batch.push_str("-execute\n");

process_state.stdin.write_all(batch.as_bytes())?;
process_state.stdin.flush()?;
```

> [!WARNING]
> **Load-Bearing Optimization (Commit `292244b`):**
> Do **not** revert or "simplify" this loop back to individual per-line `writeln!` calls. Writing line-by-line over raw `ChildStdin` issues 5–8 separate OS `write` syscalls per file across the IPC pipe, introducing heavy context-switch overhead in the hottest pipeline stage (~75–85% of per-file execution time).

---

## 2. Windows Line-Ending Protocol Quirk (`LF` vs `CRLF`)

ExifTool's persistent `-stay_open` IPC protocol strictly requires **`\n` (LF)** line terminators between argument tokens, `-execute`, and `-stay_open` commands.

- **Issue:** Rust's standard `writeln!` macro appends platform-native line endings (`\r\n` on Windows).
- **Rule:** When modifying STDIN handling in `exiftool.rs`, never use `writeln!`. Always use explicit `\n` line endings (e.g., `write_all(b"-ver\n-execute\n")` or `write!(stdin, "{}\n", arg)`).
- **Impact:** Sending `\r\n` over STDIN causes ExifTool (and test fixture mock binaries) on Windows to parse `\r` as part of the command string, leading to process warmup crashes (`WARMUP DISCONNECTED`).

---

## 3. Benchmarking Guidelines & Test Fixture Overhead

- **Harness Overhead:** The small integration test fixture (`tests/fixtures/small_dataset`, 120 files) has **~80% fixed harness overhead** (process startup, Rayon thread pool initialization, in-memory SQLite setup, tempdir creation, and mock process execution).
- **Micro-Optimization Benchmarking:** Small fixture runs (120 files) are intended for integration correctness verification and cannot reliably isolate micro-optimizations below the OS noise floor (~100ms variance).
- **Accurate Profiling:** Anyone measuring micro-optimization performance must:
  1. Use larger test datasets containing **1,000+ real media files**.
  2. Benchmark against a real `exiftool` binary using the cargo benchmark suite (`cargo bench --bench exiftool_bench`).
