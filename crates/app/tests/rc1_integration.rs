mod common;

use app::CoreDispatcher;
use app_core::config::Config;
use app_core::events::{AppEvent, Broadcaster};
use shared_ui::{CommandDispatcher, SnapshotPolicy, UiCommand, ViewModelUpdater};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::tempdir;

fn setup_fixtures_path() {
    let current_path = std::env::var_os("PATH").unwrap_or_default();
    let fixtures_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut paths = std::env::split_paths(&current_path).collect::<Vec<_>>();
    if !paths.contains(&fixtures_dir) {
        paths.insert(0, fixtures_dir);
        if let Ok(new_path) = std::env::join_paths(paths) {
            std::env::set_var("PATH", new_path);
        }
    }
}

#[test]
fn test_small_dataset_pipeline() {
    setup_fixtures_path();
    let input_dir = common::ensure_small_dataset();
    let temp_out = tempdir().unwrap();
    let out_dir = temp_out.path().to_path_buf();
    let db_path = out_dir.join("small_dataset.db");

    let publisher = Arc::new(Broadcaster::new());
    let event_rx = publisher.subscribe();

    let snapshot_rx = ViewModelUpdater::spawn(
        event_rx,
        SnapshotPolicy::Debounced(Duration::from_millis(10)),
    );

    let config = Config::default();
    let error_rx = publisher.subscribe();

    let dispatcher = Arc::new(CoreDispatcher {
        cancel_token: Arc::new(AtomicBool::new(false)),
        pause_token: Arc::new(AtomicBool::new(false)),
        publisher: Arc::clone(&publisher),
        input_dirs: Arc::new(Mutex::new(vec![input_dir])),
        output_dir: Arc::new(Mutex::new(Some(out_dir))),
        db_path: Arc::new(Mutex::new(Some(db_path))),
        use_system_exiftool: Arc::new(Mutex::new(true)),
        concurrency_limit: Arc::new(Mutex::new(4)),
        config: Arc::new(Mutex::new(config)),
    });

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
    setup_fixtures_path();
    let input_dir = common::ensure_edge_case_dataset();
    let temp_out = tempdir().unwrap();
    let out_dir = temp_out.path().to_path_buf();
    let db_path = out_dir.join("edge_cases.db");

    let publisher = Arc::new(Broadcaster::new());
    let event_rx = publisher.subscribe();

    let snapshot_rx = ViewModelUpdater::spawn(
        event_rx,
        SnapshotPolicy::Debounced(Duration::from_millis(10)),
    );

    let mut config = Config::default();
    config.processing.unmatched_enabled = true;

    let error_rx = publisher.subscribe();

    let dispatcher = Arc::new(CoreDispatcher {
        cancel_token: Arc::new(AtomicBool::new(false)),
        pause_token: Arc::new(AtomicBool::new(false)),
        publisher: Arc::clone(&publisher),
        input_dirs: Arc::new(Mutex::new(vec![input_dir])),
        output_dir: Arc::new(Mutex::new(Some(out_dir))),
        db_path: Arc::new(Mutex::new(Some(db_path))),
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

    assert_eq!(
        final_snapshot.results.len(),
        5,
        "Should process all 5 media files"
    );
}

#[test]
fn test_interrupted_processing_recovery() {
    use app_core::state_db::{FilePath, FileStatus, MediaFileInsert, StateDatabase};

    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test_recovery.db");
    let file_id;

    {
        let db = StateDatabase::open(&db_path).unwrap();
        db.insert_media_batch(&[MediaFileInsert {
            path: FilePath::Real {
                base_components: 1,
                abs: temp_dir.path().join("photo.jpg"),
            },
            filename: "photo.jpg".to_string(),
            extension: ".jpg".to_string(),
            size: 100,
        }])
        .unwrap();

        let ready = db.load_pending_media_batch(None, 10).unwrap();
        assert_eq!(ready.len(), 1);
        file_id = ready[0].id;

        db.apply_match_batch(&[app_core::state_db::MatchResult {
            id: file_id,
            json_path: None,
            match_confidence: Some(100),
            match_tier: Some(1),
            status: FileStatus::Matched,
        }])
        .unwrap();

        assert!(db.try_mark_processing(file_id).unwrap());
        let conn = db.conn.lock().unwrap();
        let status: i64 = conn
            .query_row(
                "SELECT status FROM media_files WHERE id = ?1",
                [file_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, FileStatus::Processing as i64);
    }

    let db = StateDatabase::open(&db_path).unwrap();
    let conn = db.conn.lock().unwrap();
    let restored_status: i64 = conn
        .query_row(
            "SELECT status FROM media_files WHERE id = ?1",
            [file_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        restored_status,
        FileStatus::Matched as i64,
        "Interrupted Processing status should be auto-recovered to Matched"
    );
}

#[test]
fn test_staging_directory_cleanup() {
    setup_fixtures_path();
    let input_dir = common::ensure_small_dataset();
    let temp_out = tempdir().unwrap();
    let out_dir = temp_out.path().to_path_buf();
    let db_path = out_dir.join("staging_cleanup.db");

    let publisher = Arc::new(Broadcaster::new());
    let event_rx = publisher.subscribe();

    let snapshot_rx = ViewModelUpdater::spawn(
        event_rx,
        SnapshotPolicy::Debounced(Duration::from_millis(10)),
    );

    let config = Config::default();

    let dispatcher = Arc::new(CoreDispatcher {
        cancel_token: Arc::new(AtomicBool::new(false)),
        pause_token: Arc::new(AtomicBool::new(false)),
        publisher: Arc::clone(&publisher),
        input_dirs: Arc::new(Mutex::new(vec![input_dir])),
        output_dir: Arc::new(Mutex::new(Some(out_dir.clone()))),
        db_path: Arc::new(Mutex::new(Some(db_path))),
        use_system_exiftool: Arc::new(Mutex::new(true)),
        concurrency_limit: Arc::new(Mutex::new(4)),
        config: Arc::new(Mutex::new(config)),
    });

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

    let staging_dir = out_dir.join(".staging");
    assert!(
        !staging_dir.exists(),
        "Staging directory should be completely cleaned up after run completion"
    );
}
