mod common;

use app::{run_core_pipeline, CoreDispatcher};
use app_core::config::Config;
use app_core::events::{AppEvent, Broadcaster};
use shared_ui::{CommandDispatcher, SnapshotPolicy, UiCommand, ViewModelUpdater};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::tempdir;

#[test]
fn test_small_dataset_pipeline() {
    let input_dir = common::ensure_small_dataset();
    let temp_out = tempdir().unwrap();
    let out_dir = temp_out.path().to_path_buf();

    let publisher = Arc::new(Broadcaster::new());
    let event_rx = publisher.subscribe();

    let snapshot_rx = ViewModelUpdater::spawn(
        event_rx,
        SnapshotPolicy::Debounced(Duration::from_millis(10)),
    );

    let config = Config::default();

    let error_rx = publisher.subscribe();

    // Set PATH to include our dummy exiftool.exe
    let mut current_path = std::env::var_os("PATH").unwrap_or_default();
    let fixtures_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut new_path = std::ffi::OsString::new();
    new_path.push(&fixtures_dir);
    new_path.push(";"); // Windows separator
    new_path.push(&current_path);
    std::env::set_var("PATH", new_path);

    let dispatcher = Arc::new(CoreDispatcher {
        cancel_token: Arc::new(AtomicBool::new(false)),
        pause_token: Arc::new(AtomicBool::new(false)),
        publisher: Arc::clone(&publisher),
        input_dirs: Arc::new(Mutex::new(vec![input_dir])),
        output_dir: Arc::new(Mutex::new(Some(out_dir.clone()))),
        db_path: Arc::new(Mutex::new(None)),
        use_system_exiftool: Arc::new(Mutex::new(true)), // Use our dummy exiftool
        concurrency_limit: Arc::new(Mutex::new(4)),
        config: Arc::new(Mutex::new(config)),
    });

    // We can directly call run_core_pipeline, but dispatching StartProcessing tests the UiCommand flow
    // Dispatch is async, so we'll wait for the finished state.
    dispatcher.dispatch(UiCommand::StartProcessing).unwrap();

    let mut is_finished = false;
    for _ in 0..300 {
        if snapshot_rx.borrow().is_finished {
            is_finished = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    assert!(is_finished, "Pipeline did not finish within timeout");

    let mut errors = Vec::new();
    while let Ok(event) = error_rx.try_recv() {
        if let AppEvent::Error { message, .. } = event {
            errors.push(message);
        }
    }
    if !errors.is_empty() {
        println!("Pipeline failed with errors: {:?}", errors);
    }

    let final_snapshot = snapshot_rx.borrow().clone();
    println!("Final snapshot total_files: {}", final_snapshot.total_files);
    println!(
        "Final snapshot results length: {}",
        final_snapshot.results.len()
    );

    // We expect 100 images + 20 videos = 120 total, plus 120 JSONs = 240 files scanned maybe.
    // Actually we only care about matched media.
    assert_eq!(
        final_snapshot.image_count, 100,
        "Should have processed 100 images. Phase: {}",
        final_snapshot.current_phase_text
    );
    assert_eq!(
        final_snapshot.video_count, 20,
        "Should have processed 20 videos"
    );
}

#[test]
fn test_edge_cases_pipeline() {
    let input_dir = common::ensure_edge_case_dataset();
    let temp_out = tempdir().unwrap();
    let out_dir = temp_out.path().to_path_buf();

    let publisher = Arc::new(Broadcaster::new());
    let event_rx = publisher.subscribe();

    let snapshot_rx = ViewModelUpdater::spawn(
        event_rx,
        SnapshotPolicy::Debounced(Duration::from_millis(10)),
    );

    let mut config = Config::default();
    config.processing.unmatched_enabled = true;

    // Set PATH to include our dummy exiftool.exe
    let mut current_path = std::env::var_os("PATH").unwrap_or_default();
    let fixtures_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut new_path = std::ffi::OsString::new();
    new_path.push(&fixtures_dir);
    new_path.push(";"); // Windows separator
    new_path.push(&current_path);
    std::env::set_var("PATH", new_path);

    let error_rx = publisher.subscribe();

    let dispatcher = Arc::new(CoreDispatcher {
        cancel_token: Arc::new(AtomicBool::new(false)),
        pause_token: Arc::new(AtomicBool::new(false)),
        publisher: Arc::clone(&publisher),
        input_dirs: Arc::new(Mutex::new(vec![input_dir])),
        output_dir: Arc::new(Mutex::new(Some(out_dir.clone()))),
        db_path: Arc::new(Mutex::new(None)),
        use_system_exiftool: Arc::new(Mutex::new(true)),
        concurrency_limit: Arc::new(Mutex::new(4)),
        config: Arc::new(Mutex::new(config)),
    });

    dispatcher.dispatch(UiCommand::StartProcessing).unwrap();

    let mut is_finished = false;
    for _ in 0..100 {
        if snapshot_rx.borrow().is_finished {
            is_finished = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    assert!(is_finished, "Pipeline did not finish within timeout");

    let mut errors = Vec::new();
    while let Ok(event) = error_rx.try_recv() {
        if let AppEvent::Error { message, .. } = event {
            errors.push(message);
        }
    }
    if !errors.is_empty() {
        println!("Pipeline failed with errors: {:?}", errors);
    }

    let final_snapshot = snapshot_rx.borrow().clone();

    // Ensure all edge cases are processed (some might fail or be unmatched, but it shouldn't crash)
    // 1 corrupt json (will fall back to unmatched), 1 missing json (unmatched), 2 duplicates (will rename), 1 non-english
    assert_eq!(
        final_snapshot.results.len(),
        5,
        "Should process all 5 media files"
    );
}
