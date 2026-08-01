//! Google Photos Takeout Restorer - App Dispatcher Crate
//! Bridges UI command dispatching with core pipeline orchestration.
//!
//! Author: Guruteja Reddy Nallachi (<https://github.com/GurutejaReddy-04>)
//! Open Source Software released under the MIT License.

use app_core::config::{Config, OutputMode};
use app_core::events::{AppEvent, Broadcaster, EventPublisher};
use app_core::exiftool::ExifToolPool;
use app_core::processor::Processor;
use app_core::scanner::scan_inputs;
use app_core::state_db::StateDatabase;
use downloader::ExifToolManager;
use fs2::FileExt;
use shared_ui::{CommandDispatcher, UiCommand};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

/// CoreDispatcher bridges UI Commands to Core Execution for both CLI and GUI.
pub struct CoreDispatcher {
    pub cancel_token: Arc<AtomicBool>,
    pub pause_token: Arc<AtomicBool>,
    pub publisher: Arc<Broadcaster>,

    // Mutable state for GUI inputs before starting
    pub input_dirs: Arc<Mutex<Vec<PathBuf>>>,
    pub output_dir: Arc<Mutex<Option<PathBuf>>>,
    pub db_path: Arc<Mutex<Option<PathBuf>>>,
    pub use_system_exiftool: Arc<Mutex<bool>>,
    pub concurrency_limit: Arc<Mutex<usize>>,

    // Core Configuration
    pub config: Arc<Mutex<Config>>,
}

impl CoreDispatcher {
    /// Single canonical conversion point for run identifiers.
    /// Strips the "state_" prefix if present, returning the bare identifier.
    fn canonical_run_id(raw_id: &str) -> String {
        raw_id.strip_prefix("state_").unwrap_or(raw_id).to_string()
    }

    fn recent_run_db_path(config_dir: &Path, run_id: &str) -> PathBuf {
        let normalized = Self::canonical_run_id(run_id);
        config_dir.join(format!("state_{}.db", normalized))
    }

    fn set_input_paths(&self, paths: Vec<String>) {
        let mut inputs = self.input_dirs.lock().unwrap_or_else(|e| e.into_inner());
        *inputs = paths.into_iter().map(PathBuf::from).collect();

        let out_dir = self
            .output_dir
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(dest) = out_dir {
            let result = app_core::validation::validate_destination(&dest, &inputs);
            self.publisher
                .publish(AppEvent::DestinationValidated(result));
        }
    }

    fn resume_recent_run(&self, run_id: &str) -> Result<(), String> {
        let config_dir =
            directories::ProjectDirs::from("", "TakeoutRestorerTeam", "GooglePhotosRestorer")
                .map(|dirs| dirs.config_dir().to_path_buf())
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        let db_path = Self::recent_run_db_path(&config_dir, run_id);
        if !db_path.exists() {
            return Err(format!("No saved session found for {}", run_id));
        }

        let lock_path = db_path.with_extension("lock");
        if lock_path.exists() {
            if let Ok(file) = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&lock_path)
            {
                if file.try_lock_exclusive().is_err() {
                    return Err(
                        "Cannot resume an active run. The run is already being processed."
                            .to_string(),
                    );
                }
            }
        }

        let mut active_db = self.db_path.lock().unwrap_or_else(|e| e.into_inner());
        *active_db = Some(db_path.clone());
        drop(active_db);
        self.cancel_token.store(false, Ordering::SeqCst);
        self.pause_token.store(false, Ordering::SeqCst);

        if let Ok(db) = app_core::state_db::StateDatabase::open(&db_path) {
            let mut config = app_core::config::Config::default();
            if let Ok(Some(persisted_dest)) = db.load_execution_contract(&mut config) {
                *self.output_dir.lock().unwrap_or_else(|e| e.into_inner()) = Some(persisted_dest);
            }
        }

        self.publisher.publish(AppEvent::RecentRunsLoaded(
            app_core::state_db::get_recent_runs(&config_dir),
        ));
        self.publisher.publish(AppEvent::ProcessingPhaseStarted {
            name: "Resuming".to_string(),
            total_files: None,
        });

        self.dispatch(UiCommand::StartProcessing)?;
        Ok(())
    }

    fn delete_recent_run(&self, run_id: &str) -> Result<(), String> {
        let config_dir =
            directories::ProjectDirs::from("", "TakeoutRestorerTeam", "GooglePhotosRestorer")
                .map(|dirs| dirs.config_dir().to_path_buf())
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        let db_path = Self::recent_run_db_path(&config_dir, run_id);
        let lock_path = db_path.with_extension("lock");

        if lock_path.exists() {
            if let Ok(file) = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&lock_path)
            {
                if file.try_lock_exclusive().is_err() {
                    return Err(
                        "Cannot delete an active run. The run is currently being processed."
                            .to_string(),
                    );
                }
            }
        }

        if db_path.exists() {
            if let Ok(conn) = rusqlite::Connection::open(&db_path) {
                if let Ok(dest_str) = conn.query_row(
                    "SELECT value FROM run_config WHERE key = 'destination'",
                    [],
                    |row| row.get::<_, String>(0),
                ) {
                    let dest_path = std::path::PathBuf::from(dest_str);
                    let canonical_id = Self::canonical_run_id(run_id);
                    let _ = std::fs::remove_dir_all(dest_path.join(".staging").join(&canonical_id));
                }
            }
            let _ = std::fs::remove_file(&db_path);
        }
        if lock_path.exists() {
            let _ = std::fs::remove_file(&lock_path);
        }

        self.publisher.publish(AppEvent::RecentRunsLoaded(
            app_core::state_db::get_recent_runs(&config_dir),
        ));
        Ok(())
    }

    fn clear_all_recent_runs(&self) -> Result<(), String> {
        let config_dir =
            directories::ProjectDirs::from("", "TakeoutRestorerTeam", "GooglePhotosRestorer")
                .map(|dirs| dirs.config_dir().to_path_buf())
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        let recent = app_core::state_db::get_recent_runs(&config_dir);
        for run in recent {
            let _ = self.delete_recent_run(&run.id);
        }

        self.publisher.publish(AppEvent::RecentRunsLoaded(
            app_core::state_db::get_recent_runs(&config_dir),
        ));
        Ok(())
    }
    fn recover_recent_run(&self, run_id: &str) -> Result<(), String> {
        let config_dir =
            directories::ProjectDirs::from("", "TakeoutRestorerTeam", "GooglePhotosRestorer")
                .map(|dirs| dirs.config_dir().to_path_buf())
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        let db_path = Self::recent_run_db_path(&config_dir, run_id);
        if !db_path.exists() {
            return Err(format!("No saved session found for {}", run_id));
        }

        let lock_path = db_path.with_extension("lock");
        if lock_path.exists() {
            if let Ok(file) = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&lock_path)
            {
                if file.try_lock_exclusive().is_err() {
                    return Err(
                        "Cannot resume an active run. The run is already being processed."
                            .to_string(),
                    );
                }
            }
        }

        match StateDatabase::open(&db_path) {
            Ok(db) => {
                if let Err(e) = db.apply_recovery_data() {
                    return Err(format!("Failed to apply recovery log: {}", e));
                }
            }
            Err(e) => return Err(format!("Failed to open database: {}", e)),
        }

        self.publisher.publish(AppEvent::RecentRunsLoaded(
            app_core::state_db::get_recent_runs(&config_dir),
        ));

        Ok(())
    }
}

impl CommandDispatcher for CoreDispatcher {
    fn dispatch(&self, cmd: UiCommand) -> Result<(), String> {
        match cmd {
            UiCommand::StartProcessing => {
                let inputs = self
                    .input_dirs
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                let output = self
                    .output_dir
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();

                let is_resume = self
                    .db_path
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .is_some();

                if inputs.is_empty() && !is_resume {
                    return Err("No input directories selected".to_string());
                }

                let out_dir = match output {
                    Some(o) => o,
                    None => {
                        if !is_resume {
                            let is_inplace = self
                                .config
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .processing
                                .output_mode
                                == app_core::config::OutputMode::InPlace;
                            if is_inplace && !inputs.is_empty() {
                                inputs[0].clone()
                            } else {
                                return Err("No output directory selected".to_string());
                            }
                        } else {
                            PathBuf::new()
                        }
                    }
                };

                let db_path = self
                    .db_path
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                let use_system_exiftool = *self
                    .use_system_exiftool
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                let cancel = Arc::clone(&self.cancel_token);
                let pause = Arc::clone(&self.pause_token);
                let publisher = Arc::clone(&self.publisher);

                // Offload Core execution to a background orchestrator thread
                thread::spawn(move || {
                    if let Err(e) = run_core_pipeline(
                        inputs,
                        out_dir,
                        db_path,
                        use_system_exiftool,
                        cancel,
                        pause,
                        publisher.clone(),
                    ) {
                        publisher.publish(AppEvent::Error {
                            file_id: None,
                            fatal: true,
                            message: e.to_string(),
                        });
                        publisher.publish(AppEvent::RunCompleted {
                            results: Vec::new(),
                        });
                    }
                });

                Ok(())
            }
            UiCommand::CancelProcessing => {
                self.cancel_token.store(true, Ordering::SeqCst);
                self.publisher.publish(AppEvent::CancellationAcknowledged);
                Ok(())
            }
            UiCommand::PauseProcessing => {
                self.pause_token.store(true, Ordering::SeqCst);
                self.publisher.publish(AppEvent::ProcessingPhaseStarted {
                    name: "Paused".to_string(),
                    total_files: None,
                });
                Ok(())
            }
            UiCommand::ResumeProcessing => {
                self.pause_token.store(false, Ordering::SeqCst);
                self.publisher.publish(AppEvent::ProcessingPhaseStarted {
                    name: "Processing".to_string(),
                    total_files: None,
                });
                Ok(())
            }
            UiCommand::SelectInputDirectory(path) => {
                self.set_input_paths(vec![path]);
                Ok(())
            }
            UiCommand::SelectInputDirectories(paths) => {
                self.set_input_paths(paths);
                Ok(())
            }
            UiCommand::SetInputPaths(paths) => {
                self.set_input_paths(paths);
                Ok(())
            }
            UiCommand::SelectOutputDirectory(path) => {
                let dest = PathBuf::from(path);
                *self.output_dir.lock().unwrap_or_else(|e| e.into_inner()) = Some(dest.clone());

                let inputs = self
                    .input_dirs
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                let result = app_core::validation::validate_destination(&dest, &inputs);
                self.publisher
                    .publish(AppEvent::DestinationValidated(result));
                Ok(())
            }
            UiCommand::UpdateSetting { key, value } => {
                let mut config = self.config.lock().unwrap_or_else(|e| e.into_inner());
                match key.as_str() {
                    "use_system_exiftool" => {
                        *self
                            .use_system_exiftool
                            .lock()
                            .unwrap_or_else(|e| e.into_inner()) = value == "true"
                    }
                    "concurrency_limit" => {
                        if let Ok(limit) = value.parse::<usize>() {
                            *self
                                .concurrency_limit
                                .lock()
                                .unwrap_or_else(|e| e.into_inner()) = limit;
                            config.processing.max_workers = limit;
                        }
                    }
                    "gps_enabled" => config.processing.gps_enabled = value == "true",
                    "timezone_enabled" => config.processing.timezone_enabled = value == "true",
                    "unmatched_enabled" => config.processing.unmatched_enabled = value == "true",
                    "anonymous_logging" => config.processing.anonymous_logging = value == "true",
                    "output_mode" => {
                        config.processing.output_mode = match value.as_str() {
                            "in-place" => app_core::config::OutputMode::InPlace,
                            _ => app_core::config::OutputMode::Copy,
                        };
                    }
                    "high_performance" => {
                        let is_enabled = value == "true";
                        config.processing.high_performance = is_enabled;
                        if is_enabled {
                            let avail = std::thread::available_parallelism()
                                .map(|n| n.get() * 2)
                                .unwrap_or(8);
                            config.processing.max_workers = avail;
                            *self
                                .concurrency_limit
                                .lock()
                                .unwrap_or_else(|e| e.into_inner()) = avail;
                        }
                    }
                    "theme" => config.ui.theme = value,
                    _ => {}
                }
                // Save config and publish
                let _ = config.save();
                self.publisher
                    .publish(AppEvent::ConfigChanged(config.clone()));
                Ok(())
            }
            UiCommand::UpdateResultsFilter {
                search,
                status_filter,
            } => {
                self.publisher.publish(AppEvent::ResultsFilterChanged {
                    search,
                    status_filter,
                });
                Ok(())
            }
            UiCommand::ResetState => {
                self.cancel_token.store(false, Ordering::SeqCst);
                self.pause_token.store(false, Ordering::SeqCst);
                *self.db_path.lock().unwrap_or_else(|e| e.into_inner()) = None;
                *self.input_dirs.lock().unwrap_or_else(|e| e.into_inner()) = Vec::new();
                *self.output_dir.lock().unwrap_or_else(|e| e.into_inner()) = None;
                self.publisher.publish(AppEvent::StateReset);
                Ok(())
            }
            UiCommand::ResumeRun(run_id) => self.resume_recent_run(&run_id),
            UiCommand::DeleteRun(run_id) => self.delete_recent_run(&run_id),
            UiCommand::ClearAllRuns => self.clear_all_recent_runs(),
            UiCommand::RecoverRun(run_id) => self.recover_recent_run(&run_id),

            UiCommand::Shutdown => {
                self.cancel_token.store(true, Ordering::SeqCst);
                // Exit will be handled by the UI lifecycle
                Ok(())
            }
        }
    }
}

pub fn run_core_pipeline(
    inputs: Vec<PathBuf>,
    mut output: PathBuf,
    db_path: Option<PathBuf>,
    use_system_exiftool: bool,
    cancel: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    publisher: Arc<Broadcaster>,
) -> Result<(), app_core::error::AppError> {
    let mut config = Config::load()?;

    // Validate ZIP + InPlace before any processing begins
    if config.processing.output_mode == OutputMode::InPlace {
        let has_zip = inputs.iter().any(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("zip"))
                .unwrap_or(false)
        });
        if has_zip {
            return Err(app_core::error::AppError::Config(
                "ZIP archives cannot be processed in InPlace mode. Please select Copy mode."
                    .to_string(),
            ));
        }
    }

    let exiftool_manager = ExifToolManager::new();
    let binary_path = if use_system_exiftool {
        PathBuf::from("exiftool")
    } else {
        let publisher_dl = Arc::clone(&publisher);
        exiftool_manager.ensure_installed(move |downloaded, total| {
            publisher_dl.publish(AppEvent::ExifToolDownloadProgress {
                downloaded_bytes: downloaded,
                total_bytes: total,
            });
        })?;
        exiftool_manager.check_perl()?;
        exiftool_manager.exiftool_path()
    };

    let pool_size = if config.processing.high_performance {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8)
    } else {
        let max_safe = std::thread::available_parallelism()
            .map(|n| (n.get() / 2).max(2))
            .unwrap_or(4);
        config.processing.max_workers.min(max_safe).max(1)
    };
    let pool = ExifToolPool::new(binary_path, pool_size)?;

    let is_resume = db_path.is_some();
    if config.processing.output_mode != app_core::config::OutputMode::InPlace
        && !is_resume
        && !output.ends_with("Google Photos Restored")
    {
        output = output.join("Google Photos Restored");
    }

    let resolved_db_path = match db_path {
        Some(p) => p,
        None => {
            if !output.exists() {
                std::fs::create_dir_all(&output).map_err(|e| {
                    app_core::error::AppError::Io(std::io::Error::other(format!(
                        "Failed to create output directory: {}",
                        e
                    )))
                })?;
            }
            let config_dir =
                directories::ProjectDirs::from("", "TakeoutRestorerTeam", "GooglePhotosRestorer")
                    .map(|dirs| dirs.config_dir().to_path_buf())
                    .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            std::fs::create_dir_all(&config_dir).map_err(|e| {
                app_core::error::AppError::Io(std::io::Error::other(format!(
                    "Failed to create config directory: {}",
                    e
                )))
            })?;
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            config_dir.join(format!("state_{}.db", ts))
        }
    };

    let lock_path = resolved_db_path.with_extension("lock");
    let _lock_file = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
    {
        Ok(file) => {
            if file.try_lock_exclusive().is_err() {
                return Err(app_core::error::AppError::Io(std::io::Error::other(
                    "Cannot start processing: Run is already active in another process."
                        .to_string(),
                )));
            }
            file
        }
        Err(e) => {
            return Err(app_core::error::AppError::Io(std::io::Error::other(
                format!("Failed to open lock file: {}", e),
            )))
        }
    };

    let run_id = CoreDispatcher::canonical_run_id(
        &resolved_db_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy(),
    );

    let db = StateDatabase::open(&resolved_db_path)?;

    if is_resume {
        if let Ok(Some(persisted_dest)) = db.load_execution_contract(&mut config) {
            output = persisted_dest;
        }
    } else {
        db.save_execution_contract(&config, &output)?;
    }

    publisher.publish(AppEvent::ProcessingPhaseStarted {
        name: "Scanning".to_string(),
        total_files: None,
    });

    let stats = scan_inputs(&inputs, &db, &config, &cancel, publisher.as_ref())?;

    if cancel.load(Ordering::SeqCst) {
        return Ok(());
    }

    let processor = Processor::new(&db, &config, &pool, output, publisher.as_ref(), run_id);
    processor.run_matching_phase()?;

    if cancel.load(Ordering::SeqCst) {
        return Ok(());
    }

    publisher.publish(AppEvent::ProcessingPhaseStarted {
        name: "Processing".to_string(),
        total_files: Some(stats.media_count as u64),
    });

    processor.run_processing_phase(&cancel, &pause)?;

    let results = db.get_all_terminal_results().unwrap_or_default();
    publisher.publish(AppEvent::RunCompleted { results });

    // Gracefully shut down ExifTool processes
    pool.shutdown();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_core::events::Broadcaster;
    use shared_ui::CommandDispatcher;
    use std::sync::atomic::Ordering;

    #[test]
    fn test_reset_state_clears_tokens_and_paths() {
        let publisher = Arc::new(Broadcaster::new());
        let dispatcher = CoreDispatcher {
            cancel_token: Arc::new(AtomicBool::new(true)),
            pause_token: Arc::new(AtomicBool::new(true)),
            publisher: Arc::clone(&publisher),
            input_dirs: Arc::new(Mutex::new(vec![PathBuf::from("/tmp/in")])),
            output_dir: Arc::new(Mutex::new(Some(PathBuf::from("/tmp/out")))),
            db_path: Arc::new(Mutex::new(Some(PathBuf::from("/tmp/db.sqlite")))),
            use_system_exiftool: Arc::new(Mutex::new(true)),
            concurrency_limit: Arc::new(Mutex::new(4)),
            config: Arc::new(Mutex::new(Config::default())),
        };

        dispatcher.dispatch(UiCommand::ResetState).unwrap();

        assert!(!dispatcher.cancel_token.load(Ordering::SeqCst));
        assert!(!dispatcher.pause_token.load(Ordering::SeqCst));
        assert!(dispatcher.db_path.lock().unwrap().is_none());
        assert!(dispatcher.input_dirs.lock().unwrap().is_empty());
        assert!(dispatcher.output_dir.lock().unwrap().is_none());
    }

    #[test]
    fn test_pause_processing_toggles_token_and_event() {
        let publisher = Arc::new(Broadcaster::new());
        let rx = publisher.subscribe();
        let dispatcher = CoreDispatcher {
            cancel_token: Arc::new(AtomicBool::new(false)),
            pause_token: Arc::new(AtomicBool::new(false)),
            publisher: Arc::clone(&publisher),
            input_dirs: Arc::new(Mutex::new(Vec::new())),
            output_dir: Arc::new(Mutex::new(None)),
            db_path: Arc::new(Mutex::new(None)),
            use_system_exiftool: Arc::new(Mutex::new(true)),
            concurrency_limit: Arc::new(Mutex::new(4)),
            config: Arc::new(Mutex::new(Config::default())),
        };

        dispatcher.dispatch(UiCommand::PauseProcessing).unwrap();
        assert!(dispatcher.pause_token.load(Ordering::SeqCst));

        let event = rx.try_recv().unwrap();
        if let AppEvent::ProcessingPhaseStarted { name, .. } = event {
            assert_eq!(name, "Paused");
        } else {
            panic!("Expected ProcessingPhaseStarted with name Paused");
        }
    }
}
#[test]
fn test_high_performance_off() {
    let mut config = app_core::config::Config::default();
    config.processing.high_performance = false;
    config.processing.max_workers = 2;
    assert!(!config.processing.high_performance);

    let pool_size = if config.processing.high_performance {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8)
    } else {
        config.processing.max_workers.max(1)
    };

    assert_eq!(pool_size, 2);
}
