use crate::config::{Config, OutputMode};
use crate::error::AppError;
use crate::events::{AppEvent, EventPublisher};
use crate::exiftool::ExifToolPool;
use crate::matcher::Matcher;
use crate::parser::{parse, ParsedMetadata};
use crate::state_db::{FilePath, FileStatus, MediaFile, StateDatabase, StatusUpdate};
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use tracing::{debug, error, info};

const DIR_COMPLETED: &str = "Completed";
const DIR_UNMATCHED: &str = "Unmatched";
const DIR_ERRORS: &str = "Errors";
const DIR_LOGS: &str = "Logs";

static FILE_MOVE_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub trait DiskSpaceChecker: Send + Sync {
    fn available_bytes(&self, path: &Path) -> u64;
}

pub struct SysinfoDiskChecker {
    _sys: std::sync::Mutex<sysinfo::System>,
}

impl SysinfoDiskChecker {
    pub fn new() -> Self {
        Self {
            _sys: std::sync::Mutex::new(sysinfo::System::new_all()),
        }
    }
}

impl Default for SysinfoDiskChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl DiskSpaceChecker for SysinfoDiskChecker {
    fn available_bytes(&self, path: &Path) -> u64 {
        use sysinfo::Disks;
        let disks = Disks::new_with_refreshed_list();
        // Find the disk that contains the path
        let mut best_match: Option<&sysinfo::Disk> = None;
        let mut best_len = 0;

        for disk in &disks {
            let mount_point = disk.mount_point();
            if path.starts_with(mount_point) {
                let len = mount_point.as_os_str().len();
                if len > best_len {
                    best_len = len;
                    best_match = Some(disk);
                }
            }
        }

        if let Some(disk) = best_match {
            disk.available_space()
        } else {
            u64::MAX // Fallback if we can't determine the disk
        }
    }
}

pub struct Processor<'a> {
    db: &'a StateDatabase,
    config: &'a Config,
    pool: &'a ExifToolPool,
    output_dir: PathBuf,
    publisher: &'a dyn EventPublisher,
    disk_checker: Box<dyn DiskSpaceChecker>,
    run_id: String,
    zip_json_index_cache: Mutex<HashMap<PathBuf, HashMap<String, usize>>>,
}

impl<'a> Processor<'a> {
    pub fn new(
        db: &'a StateDatabase,
        config: &'a Config,
        pool: &'a ExifToolPool,
        output_dir: PathBuf,
        publisher: &'a dyn EventPublisher,
        run_id: String,
    ) -> Self {
        Self {
            db,
            config,
            pool,
            output_dir,
            publisher,
            disk_checker: Box::new(SysinfoDiskChecker::new()),
            run_id,
            zip_json_index_cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_disk_checker(mut self, checker: Box<dyn DiskSpaceChecker>) -> Self {
        self.disk_checker = checker;
        self
    }

    /// Runs the matching phase: loads all JSON sidecar files, builds the matcher
    /// index, and iterates through pending media files in batches to find matches.
    pub fn run_matching_phase(&self) -> Result<(), AppError> {
        info!("Loading JSON entries for matching...");
        let all_json = self.db.load_all_json()?;
        let matcher = Matcher::new(&all_json, self.config);

        let mut last_id = None;
        let batch_size = 5000;

        loop {
            let pending = self.db.load_pending_media_batch(last_id, batch_size)?;
            if pending.is_empty() {
                break;
            }
            last_id = pending.last().map(|m| m.id);

            debug!("Matching batch of {} files...", pending.len());
            let results = matcher.match_batch(&pending);
            self.db.apply_match_batch(&results)?;
        }

        info!("Matching phase complete.");
        Ok(())
    }

    /// Runs the processing phase: extracts files from archives, applies metadata
    /// via ExifTool persistent pool, sets filesystem timestamps, and organizes
    /// output into Completed/Unmatched/Errors subdirectories.
    pub fn run_processing_phase(
        &self,
        cancel: &AtomicBool,
        pause: &AtomicBool,
    ) -> Result<(), AppError> {
        info!("Starting processing phase...");

        // Create output subdirectories
        self.ensure_output_dirs()?;

        // Eager startup sweep for this run's staging dir
        if self.config.processing.output_mode != OutputMode::InPlace {
            let run_staging_dir = self.output_dir.join(".staging").join(&self.run_id);
            if run_staging_dir.exists() {
                let _ = fs::remove_dir_all(&run_staging_dir);
            }
        }

        // SEC-002: Pre-extraction validation
        let pending_size = self.db.get_total_pending_size().unwrap_or(0);
        let available = self.disk_checker.available_bytes(&self.output_dir);
        let safe_buffer = 5_000_000_000; // 5 GB
        if pending_size + safe_buffer > available && available != u64::MAX {
            self.publisher.publish(AppEvent::Error {
                file_id: None,
                fatal: true,
                message: format!(
                    "Insufficient disk space. Needed ~{} bytes, available {} bytes.",
                    pending_size + safe_buffer,
                    available
                ),
            });
            return Err(AppError::DiskFull);
        }

        let extracted_bytes = std::sync::Arc::new(AtomicU64::new(0));
        let completed_extracted = std::sync::Arc::new(AtomicU64::new(0));
        let restoration_started = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let batch_size = 1000;
        let mut last_id = None;

        loop {
            if cancel.load(Ordering::Relaxed) {
                self.cleanup_staging();
                self.publisher.publish(AppEvent::CancellationAcknowledged);
                return Ok(());
            }

            let ready_files = self.load_ready_media_batch(last_id, batch_size)?;
            if ready_files.is_empty() {
                break;
            }
            last_id = ready_files.last().map(|m| m.id);

            let mut archive_map: HashMap<PathBuf, Vec<&MediaFile>> = HashMap::new();
            let mut real_files = Vec::new();

            for media in &ready_files {
                match &media.path {
                    FilePath::Real { .. } => real_files.push(media),
                    FilePath::Zip { archive, .. } => {
                        archive_map.entry(archive.clone()).or_default().push(media)
                    }
                }
            }

            let grand_total = self
                .db
                .get_total_media_count()
                .unwrap_or(ready_files.len() as u64);
            let restoration_started = std::sync::Arc::clone(&restoration_started);
            let extracted_bytes_clone = std::sync::Arc::clone(&extracted_bytes);
            // ============================================================
            // DEBUG INSTRUMENTATION: Atomic counters for monitoring
            // ============================================================
            let dbg_producer_extracted = std::sync::Arc::new(AtomicUsize::new(0));
            let dbg_producer_send_ok = std::sync::Arc::new(AtomicUsize::new(0));
            let dbg_producer_send_blocked = std::sync::Arc::new(AtomicUsize::new(0));
            let dbg_consumer_received = std::sync::Arc::new(AtomicUsize::new(0));
            let dbg_consumer_completed = std::sync::Arc::new(AtomicUsize::new(0));
            let dbg_producer_done = std::sync::Arc::new(AtomicBool::new(false));
            let dbg_consumer_done = std::sync::Arc::new(AtomicBool::new(false));
            let dbg_last_completed_file = std::sync::Arc::new(Mutex::new(String::new()));
            let dbg_last_completed_id = std::sync::Arc::new(AtomicUsize::new(0));
            let dbg_outstanding_files =
                std::sync::Arc::new(Mutex::new(Vec::<(i64, String)>::new()));

            std::thread::scope(|s| {
                let (tx, rx) = std::sync::mpsc::sync_channel(100);

                let pool_size = self.pool.total_size();

                // ============================================================
                // DEBUG: Monitor thread - prints stats every second
                // ============================================================
                let mon_producer_extracted = std::sync::Arc::clone(&dbg_producer_extracted);
                let mon_producer_send_ok = std::sync::Arc::clone(&dbg_producer_send_ok);
                let mon_producer_send_blocked = std::sync::Arc::clone(&dbg_producer_send_blocked);
                let mon_consumer_received = std::sync::Arc::clone(&dbg_consumer_received);
                let mon_consumer_completed = std::sync::Arc::clone(&dbg_consumer_completed);
                let mon_producer_done = std::sync::Arc::clone(&dbg_producer_done);
                let mon_consumer_done = std::sync::Arc::clone(&dbg_consumer_done);
                let mon_last_completed_file = std::sync::Arc::clone(&dbg_last_completed_file);
                let mon_last_completed_id = std::sync::Arc::clone(&dbg_last_completed_id);
                let mon_outstanding = std::sync::Arc::clone(&dbg_outstanding_files);
                let mon_pool = self.pool;
                s.spawn(move || {
                    let mut last_completed_count = 0usize;
                    let mut stall_start: Option<std::time::Instant> = None;
                    let mut stall_dumped = false;

                    loop {
                        std::thread::sleep(std::time::Duration::from_secs(1));

                        let p_extracted = mon_producer_extracted.load(Ordering::Relaxed);
                        let p_sent = mon_producer_send_ok.load(Ordering::Relaxed);
                        let p_blocked = mon_producer_send_blocked.load(Ordering::Relaxed);
                        let c_received = mon_consumer_received.load(Ordering::Relaxed);
                        let c_completed = mon_consumer_completed.load(Ordering::Relaxed);
                        let p_done = mon_producer_done.load(Ordering::Relaxed);
                        let c_done = mon_consumer_done.load(Ordering::Relaxed);
                        let last_file = mon_last_completed_file.lock().unwrap_or_else(|e| e.into_inner()).clone();
                        let last_id = mon_last_completed_id.load(Ordering::Relaxed);
                        let exiftool_available = mon_pool.available_count();
                        let exiftool_total = mon_pool.total_size();
                        let exiftool_busy = exiftool_total.saturating_sub(exiftool_available);
                        let channel_pending = p_sent.saturating_sub(c_received);
                        let rayon_threads = rayon::current_num_threads();

                        eprintln!(
                            "\n[DEBUG][MONITOR] ========== 1-SECOND STATS ==========\n\
                             [DEBUG][MONITOR] Producer: extracted={} | sent={} | blocked_sends={} | done={}\n\
                             [DEBUG][MONITOR] Channel: pending_in_channel=~{} (sent-received)\n\
                             [DEBUG][MONITOR] Consumer: received={} | completed={} | done={}\n\
                             [DEBUG][MONITOR] ExifTool: available={} | busy={} | pool_size={}\n\
                             [DEBUG][MONITOR] Rayon: global_thread_count={}\n\
                             [DEBUG][MONITOR] Last completed: id={} file='{}'\n\
                             [DEBUG][MONITOR] ==============================================",
                            p_extracted, p_sent, p_blocked, p_done,
                            channel_pending,
                            c_received, c_completed, c_done,
                            exiftool_available, exiftool_busy, exiftool_total,
                            rayon_threads,
                            last_id, last_file
                        );

                        // Stall detection (Requirement 8)
                        if c_completed == last_completed_count && c_completed > 0 && !c_done {
                            if stall_start.is_none() {
                                stall_start = Some(std::time::Instant::now());
                            }
                            if let Some(start) = stall_start {
                                let stall_secs = start.elapsed().as_secs();
                                if stall_secs >= 10 && !stall_dumped {
                                    stall_dumped = true;
                                    let outstanding = mon_outstanding.lock().unwrap_or_else(|e| e.into_inner()).clone();
                                    eprintln!(
                                        "\n[DEBUG][STALL] !!! PROCESSING STALLED FOR {} SECONDS !!!\n\
                                         [DEBUG][STALL] ========== FULL STATE DUMP ==========\n\
                                         [DEBUG][STALL] Producer: extracted={} | sent={} | blocked_sends={} | done={}\n\
                                         [DEBUG][STALL] Channel: pending_in_channel=~{}\n\
                                         [DEBUG][STALL] Consumer: received={} | completed={} | done={}\n\
                                         [DEBUG][STALL] ExifTool: available={} | busy={} | pool_size={}\n\
                                         [DEBUG][STALL] Rayon: global_thread_count={}\n\
                                         [DEBUG][STALL] Last completed file: id={} file='{}'\n\
                                         [DEBUG][STALL] Outstanding files ({} total):",
                                        stall_secs,
                                        p_extracted, p_sent, p_blocked, p_done,
                                        channel_pending,
                                        c_received, c_completed, c_done,
                                        exiftool_available, exiftool_busy, exiftool_total,
                                        rayon_threads,
                                        last_id, last_file,
                                        outstanding.len()
                                    );
                                    for (oid, oname) in &outstanding {
                                        eprintln!("[DEBUG][STALL]   id={} file='{}'", oid, oname);
                                    }
                                    eprintln!("[DEBUG][STALL] ========== END STATE DUMP ==========");
                                }
                            }
                        } else {
                            last_completed_count = c_completed;
                            stall_start = None;
                            stall_dumped = false;
                        }

                        // Exit when both producer and consumer are done
                        if p_done && c_done {
                            eprintln!("[DEBUG][MONITOR] Both producer and consumer done. Monitor exiting.");
                            break;
                        }
                    }
                });

                // Consumer thread pool: EXIF processing
                let con_consumer_received = std::sync::Arc::clone(&dbg_consumer_received);
                let con_consumer_completed = std::sync::Arc::clone(&dbg_consumer_completed);
                let con_consumer_done = std::sync::Arc::clone(&dbg_consumer_done);
                let con_last_completed_file = std::sync::Arc::clone(&dbg_last_completed_file);
                let con_last_completed_id = std::sync::Arc::clone(&dbg_last_completed_id);
                let con_outstanding = std::sync::Arc::clone(&dbg_outstanding_files);
                s.spawn(move || {
                    rx.into_iter().par_bridge().for_each(
                        |(media, original_target_path): (MediaFile, PathBuf)| {
                            // DEBUG: Log every item consumed (Requirement 2)
                            let recv_seq = con_consumer_received.fetch_add(1, Ordering::Relaxed) + 1;
                            eprintln!(
                                "[DEBUG][CONSUMER] RECEIVED #{} | id={} | file='{}' | thread={:?}",
                                recv_seq, media.id, media.filename, std::thread::current().id()
                            );
                            // Track outstanding files
                            {
                                let mut outstanding = con_outstanding.lock().unwrap_or_else(|e| e.into_inner());
                                outstanding.push((media.id, media.filename.clone()));
                            }

                            if cancel.load(Ordering::Relaxed) {
                                return;
                            }
                            while pause.load(Ordering::Relaxed) && !cancel.load(Ordering::Relaxed) {
                                std::thread::sleep(std::time::Duration::from_millis(100));
                            }

                            if !restoration_started.swap(true, Ordering::Relaxed) {
                                self.publisher.publish(AppEvent::ProcessingPhaseStarted {
                                    name: format!("Restoring Metadata ({} Workers)", pool_size),
                                    total_files: Some(grand_total),
                                });
                            }

                            let mut target_path = original_target_path.clone();

                            // Auto-heal: detect and fix mismatched file extensions
                            if let Some(true_ext) =
                                crate::auto_heal::get_correction(&target_path, &media.extension)
                            {
                                let mut proposed_path = target_path.clone();
                                proposed_path.set_extension(true_ext.trim_start_matches('.'));

                                let _lock =
                                    FILE_MOVE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
                                let new_path = resolve_collision(
                                    proposed_path.parent().unwrap_or(Path::new("")),
                                    &proposed_path,
                                );

                                if std::fs::rename(&target_path, &new_path).is_ok() {
                                    self.publisher.publish(AppEvent::Warning {
                                        message: format!(
                                            "Auto-Heal: Renamed {} to match true extension {}",
                                            media.filename, true_ext
                                        ),
                                    });
                                    target_path = new_path;
                                }
                                drop(_lock);
                            }

                            let process_start = std::time::Instant::now();
                            match self.process_metadata(&media, &target_path) {
                                Ok(parsed_meta) => {
                                    // Set filesystem timestamps to the photo taken time
                                    if let Some(meta) = &parsed_meta {
                                        self.set_file_timestamps(
                                            &target_path,
                                            meta.taken_timestamp,
                                        );
                                    }

                                    // Move file to appropriate output subdirectory
                                    let final_path = self.move_to_output_subdir(
                                        &target_path,
                                        &media,
                                        parsed_meta.is_some(),
                                    );

                                    let bytes_written = if media.size > 0 {
                                         media.size as u64
                                     } else {
                                         std::fs::metadata(&final_path)
                                             .map(|m| m.len())
                                             .unwrap_or(0)
                                     };
                                    if let Err(db_err) = self
                                        .db
                                        .enqueue_status_update(StatusUpdate::Completed(media.id))
                                    {
                                        self.publisher.publish(AppEvent::Error {
                                            file_id: None,
                                            fatal: true,
                                            message: format!("Fatal persistence error: {}", db_err),
                                        });
                                        cancel.store(true, Ordering::Relaxed);
                                    }
                                    self.publisher.publish(AppEvent::FileProcessed {
                                        file_id: media.id,
                                        status: FileStatus::Completed,
                                        bytes_written,
                                    });

                                    // DEBUG: Log completion (Requirements 4, 5)
                                    let elapsed = process_start.elapsed();
                                    let completed_seq = con_consumer_completed.fetch_add(1, Ordering::Relaxed) + 1;
                                    {
                                        let mut last = con_last_completed_file.lock().unwrap_or_else(|e| e.into_inner());
                                        *last = media.filename.clone();
                                    }
                                    con_last_completed_id.store(media.id as usize, Ordering::Relaxed);
                                    eprintln!(
                                        "[DEBUG][CONSUMER] COMPLETED #{} | id={} | file='{}' | elapsed={:?} | thread={:?}",
                                        completed_seq, media.id, media.filename, elapsed, std::thread::current().id()
                                    );
                                    // Remove from outstanding
                                    {
                                        let mut outstanding = con_outstanding.lock().unwrap_or_else(|e| e.into_inner());
                                        outstanding.retain(|(id, _)| *id != media.id);
                                    }
                                }
                                Err(e) => {
                                    error!(
                                        "Error processing metadata for file {}: {}",
                                        media.filename, e
                                    );

                                    // Move to Errors subdirectory
                                    let final_path =
                                        self.move_to_error_subdir(&target_path, &media);

                                    if let Err(db_err) = self.db.enqueue_status_update(
                                        StatusUpdate::Error(media.id, e.to_string()),
                                    ) {
                                        self.publisher.publish(AppEvent::Error {
                                            file_id: None,
                                            fatal: true,
                                            message: format!("Fatal persistence error: {}", db_err),
                                        });
                                        cancel.store(true, Ordering::Relaxed);
                                    }
                                    self.publisher.publish(AppEvent::Error {
                                        file_id: Some(media.id),
                                        fatal: false,
                                        message: format!("{}: {}", final_path.display(), e),
                                    });
                                    self.publisher.publish(AppEvent::FileProcessed {
                                        file_id: media.id,
                                        status: FileStatus::Error,
                                        bytes_written: 0,
                                    });

                                    // DEBUG: Also count errors as completed for monitor
                                    let elapsed = process_start.elapsed();
                                    let completed_seq = con_consumer_completed.fetch_add(1, Ordering::Relaxed) + 1;
                                    eprintln!(
                                        "[DEBUG][CONSUMER] ERROR #{} | id={} | file='{}' | error='{}' | elapsed={:?}",
                                        completed_seq, media.id, media.filename, e, elapsed
                                    );
                                    // Remove from outstanding
                                    {
                                        let mut outstanding = con_outstanding.lock().unwrap_or_else(|e| e.into_inner());
                                        outstanding.retain(|(id, _)| *id != media.id);
                                    }
                                }
                            }

                            // Per-file sweep
                            if self.config.processing.output_mode != OutputMode::InPlace {
                                let staging_path = self.resolve_staging_path(&media);
                                if let Some(parent) = staging_path.parent() {
                                    if parent.exists() {
                                        let _ = fs::remove_dir_all(parent);
                                    }
                                }
                            }
                        },
                    );
                    con_consumer_done.store(true, Ordering::Relaxed);
                    eprintln!("[DEBUG][CONSUMER] par_bridge iterator exhausted. Consumer thread exiting.");
                });

                // Producer thread: Extraction
                let extracted_bytes = extracted_bytes_clone;
                let prod_extracted = std::sync::Arc::clone(&dbg_producer_extracted);
                let prod_send_ok = std::sync::Arc::clone(&dbg_producer_send_ok);
                let prod_send_blocked = std::sync::Arc::clone(&dbg_producer_send_blocked);
                for media in real_files {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let dest = self.resolve_staging_path(media);
                    if let Ok(true) = self.db.try_mark_processing(media.id) {
                        if self.config.processing.output_mode == OutputMode::InPlace {
                            if let FilePath::Real { abs: p, .. } = &media.path {
                                // DEBUG: Timed send (Requirement 1)
                                let send_start = std::time::Instant::now();
                                let _ = tx.send((media.clone(), p.clone()));
                                let send_elapsed = send_start.elapsed();
                                prod_send_ok.fetch_add(1, Ordering::Relaxed);
                                if send_elapsed > std::time::Duration::from_millis(100) {
                                    prod_send_blocked.fetch_add(1, Ordering::Relaxed);
                                    eprintln!(
                                        "[DEBUG][PRODUCER] SEND BLOCKED {:?} | id={} | file='{}' | thread={:?}",
                                        send_elapsed, media.id, media.filename, std::thread::current().id()
                                    );
                                } else {
                                    eprintln!(
                                        "[DEBUG][PRODUCER] SEND OK {:?} | id={} | file='{}'",
                                        send_elapsed, media.id, media.filename
                                    );
                                }
                            }
                        } else if let FilePath::Real { abs: p, .. } = &media.path {
                            if let Some(parent) = dest.parent() {
                                let _ = fs::create_dir_all(parent);
                            }
                            let mut file_name = dest.file_name().unwrap_or_default().to_os_string();
                            file_name.push(".partial");
                            let temp_dest = dest.with_file_name(file_name);
                            let copy_result =
                                fs::copy(p, &temp_dest).and_then(|_| fs::rename(&temp_dest, &dest));
                            if let Err(e) = copy_result {
                                let _ = fs::remove_file(&temp_dest);
                                if let Err(db_err) = self.db.enqueue_status_update(
                                    StatusUpdate::Error(media.id, format!("Copy failed: {}", e)),
                                ) {
                                    self.publisher.publish(AppEvent::Error {
                                        file_id: None,
                                        fatal: true,
                                        message: format!("Fatal persistence error: {}", db_err),
                                    });
                                    cancel.store(true, Ordering::Relaxed);
                                }
                            } else {
                                prod_extracted.fetch_add(1, Ordering::Relaxed);
                                // DEBUG: Timed send (Requirement 1)
                                let send_start = std::time::Instant::now();
                                let _ = tx.send((media.clone(), dest));
                                let send_elapsed = send_start.elapsed();
                                prod_send_ok.fetch_add(1, Ordering::Relaxed);
                                if send_elapsed > std::time::Duration::from_millis(100) {
                                    prod_send_blocked.fetch_add(1, Ordering::Relaxed);
                                    eprintln!(
                                        "[DEBUG][PRODUCER] SEND BLOCKED {:?} | id={} | file='{}' | thread={:?}",
                                        send_elapsed, media.id, media.filename, std::thread::current().id()
                                    );
                                } else {
                                    eprintln!(
                                        "[DEBUG][PRODUCER] SEND OK {:?} | id={} | file='{}'",
                                        send_elapsed, media.id, media.filename
                                    );
                                }
                            }
                        }
                    }
                }

                let completed_extracted = std::sync::Arc::clone(&completed_extracted);
                let prod_extracted_zip = std::sync::Arc::clone(&dbg_producer_extracted);
                let prod_send_ok_zip = std::sync::Arc::clone(&dbg_producer_send_ok);
                let prod_send_blocked_zip = std::sync::Arc::clone(&dbg_producer_send_blocked);
                for (archive_path, files) in archive_map {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }

                    eprintln!(
                        "[DEBUG][PRODUCER] Processing archive: {} ({} files)",
                        archive_path.display(),
                        files.len()
                    );

                    let file_result = fs::File::open(&archive_path);
                    if let Ok(file) = file_result {
                        if let Ok(mut zip) = zip::ZipArchive::new(file) {
                            let name_to_index: std::collections::HashMap<String, usize> = (0..zip
                                .len())
                                .filter_map(|idx| {
                                    zip.by_index(idx).ok().map(|f| (f.name().to_string(), idx))
                                })
                                .collect();

                            files.iter().for_each(|media| {
                                // DEBUG: Log iter entry
                                eprintln!(
                                    "[DEBUG][PRODUCER] ITER ENTERED | id={} | file='{}' | thread={:?}",
                                    media.id, media.filename, std::thread::current().id()
                                );

                                if cancel.load(Ordering::Relaxed) {
                                    return;
                                }

                                let dest = self.resolve_staging_path(media);
                                let is_claimed = self.db.try_mark_processing(media.id).unwrap_or(false);
                                if !is_claimed {
                                    eprintln!(
                                        "[DEBUG][PRODUCER] SKIPPED (not claimed) | id={} | file='{}'",
                                        media.id, media.filename
                                    );
                                    let current_cnt = completed_extracted.fetch_add(1, Ordering::Relaxed) + 1;
                                    if current_cnt.is_multiple_of(25) || current_cnt == grand_total {
                                        self.publisher.publish(AppEvent::ProgressStats {
                                            completed: current_cnt,
                                            total: grand_total,
                                            eta_seconds: None,
                                            speed_bps: 0,
                                        });
                                    }
                                    return;
                                }

                                if let FilePath::Zip { internal, .. } = &media.path {
                                    let extract_res = (|| -> Result<u64, AppError> {
                                        let idx = *name_to_index.get(internal).ok_or_else(|| {
                                            AppError::Io(std::io::Error::other(format!(
                                                "Zip entry not found: {}",
                                                internal
                                            )))
                                        })?;
                                        let file = fs::File::open(&archive_path)
                                            .map_err(AppError::Io)?;
                                         let mut zip = zip::ZipArchive::new(file)?;
                                        let mut zip_file = zip.by_index(idx)?;

                                        if let Some(p) = dest.parent() {
                                            let _ = fs::create_dir_all(p);
                                        }
                                        let mut file_name = dest
                                            .file_name()
                                            .unwrap_or_default()
                                            .to_os_string();
                                        file_name.push(".partial");
                                        let temp_dest = dest.with_file_name(file_name);

                                        let raw_file = fs::File::create(&temp_dest)
                                            .map_err(AppError::Io)?;
                                        let mut out_file = std::io::BufWriter::with_capacity(64 * 1024, raw_file);
                                        use std::io::{Read, Write};
                                        const MAX_SAFE_FILE_SIZE: u64 = 20_000_000_000;
                                        let mut bounded_reader = zip_file.by_ref().take(MAX_SAFE_FILE_SIZE + 1);
                                        let size =
                                            std::io::copy(&mut bounded_reader, &mut out_file)
                                                .map_err(|e| {
                                                    let _ = fs::remove_file(&temp_dest);
                                                    AppError::Io(e)
                                                })?;
                                        out_file.flush().map_err(|e| {
                                            let _ = fs::remove_file(&temp_dest);
                                            AppError::Io(e)
                                        })?;
                                        drop(out_file);

                                        if size > MAX_SAFE_FILE_SIZE {
                                            let _ = fs::remove_file(&temp_dest);
                                            return Err(AppError::SecurityThreat(
                                                "Extracted ZIP entry exceeded maximum safe size threshold".into()
                                            ));
                                        }

                                        fs::rename(&temp_dest, &dest).map_err(|e| {
                                            let _ = fs::remove_file(&temp_dest);
                                            AppError::Io(e)
                                        })?;
                                        Ok(size)
                                    })();

                                    match extract_res {
                                        Ok(size) => {
                                            prod_extracted_zip.fetch_add(1, Ordering::Relaxed);
                                            let prev = extracted_bytes
                                                .fetch_add(size, Ordering::Relaxed);
                                            let current_cnt = completed_extracted
                                                .fetch_add(1, Ordering::Relaxed)
                                                + 1;
                                            if (prev + size) / 500_000_000
                                                > prev / 500_000_000
                                            {
                                                let avail = self
                                                    .disk_checker
                                                    .available_bytes(&self.output_dir);
                                                if avail < 5_000_000_000
                                                    && avail != u64::MAX
                                                {
                                                    self.publisher
                                                        .publish(AppEvent::Error {
                                                        file_id: None,
                                                        fatal: true,
                                                        message:
                                                            "Disk full during extraction"
                                                                .into(),
                                                     });
                                                     cancel.store(true, Ordering::Relaxed);
                                                }
                                            }
                                            if current_cnt.is_multiple_of(25) || current_cnt == grand_total {
                                                self.publisher.publish(
                                                    AppEvent::ProgressStats {
                                                        completed: current_cnt,
                                                        total: grand_total,
                                                        eta_seconds: None,
                                                        speed_bps: size,
                                                    },
                                                );
                                            }
                                            // DEBUG: Timed send (Requirement 1)
                                            let send_start = std::time::Instant::now();
                                            let _ = tx.send(((*media).clone(), dest));
                                            let send_elapsed = send_start.elapsed();
                                            prod_send_ok_zip.fetch_add(1, Ordering::Relaxed);
                                            if send_elapsed > std::time::Duration::from_millis(100) {
                                                prod_send_blocked_zip.fetch_add(1, Ordering::Relaxed);
                                                eprintln!(
                                                    "[DEBUG][PRODUCER] SEND BLOCKED {:?} | id={} | file='{}' | archive='{}' | thread={:?}",
                                                    send_elapsed, media.id, media.filename, archive_path.display(), std::thread::current().id()
                                                );
                                            } else {
                                                eprintln!(
                                                    "[DEBUG][PRODUCER] SEND OK {:?} | id={} | file='{}'",
                                                    send_elapsed, media.id, media.filename
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            let current_cnt = completed_extracted.fetch_add(1, Ordering::Relaxed) + 1;
                                            if current_cnt.is_multiple_of(25) || current_cnt == grand_total {
                                                self.publisher.publish(
                                                    AppEvent::ProgressStats {
                                                        completed: current_cnt,
                                                        total: grand_total,
                                                        eta_seconds: None,
                                                        speed_bps: 0,
                                                    },
                                                );
                                            }
                                            if let Err(db_err) = self
                                                .db
                                                .enqueue_status_update(StatusUpdate::Error(
                                                    media.id,
                                                    e.to_string(),
                                                ))
                                            {
                                                self.publisher.publish(AppEvent::Error {
                                                    file_id: None,
                                                    fatal: true,
                                                    message: format!(
                                                        "Fatal persistence error: {}",
                                                        db_err
                                                    ),
                                                });
                                            }
                                            self.publisher.publish(AppEvent::FileProcessed {
                                                file_id: media.id,
                                                status: FileStatus::Error,
                                                bytes_written: 0,
                                            });
                                        }
                                    }
                                }
                            });
                        }
                    }
                }
                dbg_producer_done.store(true, Ordering::Relaxed);
                eprintln!(
                    "[DEBUG][PRODUCER] All archives processed. Dropping tx (channel sender). producer_done=true"
                );
                drop(tx);
            });
        }

        self.cleanup_staging();
        self.db.flush()?;
        info!("Processing phase complete.");
        Ok(())
    }

    pub fn cleanup_staging(&self) {
        if self.config.processing.output_mode != OutputMode::InPlace {
            let run_staging_dir = self.output_dir.join(".staging").join(&self.run_id);
            if run_staging_dir.exists() {
                let _ = fs::remove_dir_all(&run_staging_dir);
            }
            let root_staging_dir = self.output_dir.join(".staging");
            if root_staging_dir.exists() {
                // Use remove_dir instead of remove_dir_all to only delete .staging if it is completely empty,
                // preserving active staging subdirectories of concurrent runs.
                let _ = fs::remove_dir(&root_staging_dir);
            }
        }
    }

    /// Creates the output subdirectories if they don't exist.
    fn ensure_output_dirs(&self) -> Result<(), AppError> {
        if self.config.processing.output_mode == OutputMode::InPlace {
            return Ok(());
        }

        for dir_name in [DIR_COMPLETED, DIR_UNMATCHED, DIR_ERRORS, DIR_LOGS] {
            let dir = self.output_dir.join(dir_name);
            if !dir.exists() {
                fs::create_dir_all(&dir).map_err(AppError::Io)?;
            }
        }

        let log_file = self.output_dir.join(DIR_LOGS).join("restoration.log");
        if !log_file.exists() {
            let _ = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&log_file);
        }
        Ok(())
    }

    fn load_ready_media_batch(
        &self,
        last_id: Option<i64>,
        limit: usize,
    ) -> Result<Vec<MediaFile>, AppError> {
        self.db.load_ready_media_batch(last_id, limit)
    }

    /// Resolves the target path for a media file within the appropriate output
    /// subdirectory. Initially places files in the base output dir for extraction,
    /// before they get moved to their final subdirectory after processing.
    fn resolve_staging_path(&self, media: &MediaFile) -> PathBuf {
        self.output_dir
            .join(".staging")
            .join(&self.run_id)
            .join(media.id.to_string())
            .join(&media.filename)
    }

    fn resolve_rel_path(media: &MediaFile) -> PathBuf {
        let mut rel_path = PathBuf::new();
        match &media.path {
            FilePath::Real {
                base_components,
                abs,
            } => {
                if let Some(parent) = abs.parent() {
                    for c in parent.components().skip(*base_components) {
                        if let std::path::Component::Normal(s) = c {
                            rel_path.push(s);
                        }
                    }
                }
            }
            FilePath::Zip { internal, .. } => {
                if let Some(parent) = Path::new(internal).parent() {
                    for c in parent.components() {
                        if let std::path::Component::Normal(s) = c {
                            rel_path.push(s);
                        }
                    }
                }
            }
        }
        rel_path
    }

    fn resolve_final_output_path(&self, media: &MediaFile, has_json_match: bool) -> PathBuf {
        let subdir = if has_json_match {
            DIR_COMPLETED
        } else {
            DIR_UNMATCHED
        };
        self.output_dir
            .join(subdir)
            .join(Self::resolve_rel_path(media))
    }

    fn resolve_error_output_path(&self, media: &MediaFile) -> PathBuf {
        self.output_dir
            .join(DIR_ERRORS)
            .join(Self::resolve_rel_path(media))
    }

    /// Moves a successfully processed file to the appropriate output subdirectory.
    /// Returns the final path (or original path if move fails or mode is in-place).
    fn move_to_output_subdir(
        &self,
        current_path: &Path,
        media: &MediaFile,
        has_json_match: bool,
    ) -> PathBuf {
        if self.config.processing.output_mode == OutputMode::InPlace {
            return current_path.to_path_buf();
        }

        let dest_dir = self.resolve_final_output_path(media, has_json_match);
        let _ = fs::create_dir_all(&dest_dir);

        let _lock = FILE_MOVE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let dest_path = resolve_collision(&dest_dir, current_path);

        match fs::rename(current_path, &dest_path) {
            Ok(_) => dest_path,
            Err(_e) => match fs::copy(current_path, &dest_path) {
                Ok(_) => {
                    let _ = fs::remove_file(current_path);
                    dest_path
                }
                Err(_copy_err) => current_path.to_path_buf(),
            },
        }
    }

    /// Moves a file that failed processing to the Errors subdirectory.
    fn move_to_error_subdir(&self, current_path: &Path, media: &MediaFile) -> PathBuf {
        if self.config.processing.output_mode == OutputMode::InPlace {
            return current_path.to_path_buf();
        }

        let dest_dir = self.resolve_error_output_path(media);
        if !dest_dir.exists() {
            let _ = fs::create_dir_all(&dest_dir);
        }

        let _lock = FILE_MOVE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let dest_path = resolve_collision(&dest_dir, current_path);

        match fs::rename(current_path, &dest_path) {
            Ok(_) => dest_path,
            Err(_e) => match fs::copy(current_path, &dest_path) {
                Ok(_) => {
                    let _ = fs::remove_file(current_path);
                    dest_path
                }
                Err(_copy_err) => current_path.to_path_buf(),
            },
        }
    }

    /// Sets the file's modification and access times
    /// Processes metadata for a single file: reads the matched JSON sidecar,
    /// parses it, and writes EXIF metadata using an ExifTool engine from the pool.
    /// Returns the parsed metadata if a JSON match existed (for timestamp setting).
    fn set_file_timestamps(&self, path: &Path, taken_timestamp: i64) {
        let ft = filetime::FileTime::from_unix_time(taken_timestamp, 0);
        if let Err(e) = filetime::set_file_times(path, ft, ft) {
            tracing::warn!(
                "Failed to set file timestamps for {}: {}",
                path.display(),
                e
            );
        }
    }

    /// Processes metadata for a single file: reads the matched JSON sidecar,
    /// parses it, and writes EXIF metadata using an ExifTool engine from the pool.
    /// Returns the parsed metadata if a JSON match existed (for timestamp setting).
    fn process_metadata(
        &self,
        media: &MediaFile,
        target_path: &Path,
    ) -> Result<Option<ParsedMetadata>, AppError> {
        let json_path = match &media.json_path {
            Some(p) => p,
            None => return Ok(None),
        };

        let json_content = match json_path {
            FilePath::Real { abs, .. } => std::fs::read_to_string(abs).map_err(AppError::Io)?,
            FilePath::Zip { archive, internal } => {
                let idx = {
                    let cached_idx = {
                        let cache = self
                            .zip_json_index_cache
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        cache.get(archive).and_then(|m| m.get(internal).copied())
                    };

                    if let Some(i) = cached_idx {
                        Some(i)
                    } else {
                        // Build index outside the lock to prevent worker thread starvation
                        let map_opt = if let Ok(file) = std::fs::File::open(archive) {
                            if let Ok(mut zip) = zip::ZipArchive::new(file) {
                                let map: HashMap<String, usize> = (0..zip.len())
                                    .filter_map(|i| {
                                        zip.by_index(i).ok().map(|f| (f.name().to_string(), i))
                                    })
                                    .collect();
                                Some(map)
                            } else {
                                None
                            }
                        } else {
                            None
                        };

                        if let Some(map) = map_opt {
                            let idx = map.get(internal).copied();
                            let mut cache = self
                                .zip_json_index_cache
                                .lock()
                                .unwrap_or_else(|e| e.into_inner());
                            cache.insert(archive.clone(), map);
                            idx
                        } else {
                            None
                        }
                    }
                };

                if let Some(i) = idx {
                    let file = std::fs::File::open(archive).map_err(AppError::Io)?;
                    let mut zip = zip::ZipArchive::new(file)?;
                    let mut zf = zip.by_index(i)?;
                    let mut s = String::new();
                    std::io::Read::read_to_string(&mut zf, &mut s).map_err(AppError::Io)?;
                    s
                } else {
                    let file = std::fs::File::open(archive).map_err(AppError::Io)?;
                    let mut zip = zip::ZipArchive::new(file)?;
                    let mut zf = zip.by_name(internal)?;
                    let mut s = String::new();
                    std::io::Read::read_to_string(&mut zf, &mut s).map_err(AppError::Io)?;
                    s
                }
            }
        };

        let parsed = parse(json_content.as_bytes())?;

        self.pool
            .execute(|engine| engine.update_metadata(target_path, &parsed))?;

        Ok(Some(parsed))
    }
}

impl<'a> Drop for Processor<'a> {
    fn drop(&mut self) {
        self.cleanup_staging();
    }
}

fn resolve_collision(dest_dir: &Path, current_path: &Path) -> PathBuf {
    let filename = current_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let ext = current_path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let stem = filename.strip_suffix(&ext).unwrap_or(&filename);

    let mut dest_path = dest_dir.join(current_path.file_name().unwrap_or_default());

    let mut counter = 1;
    while dest_path.exists() {
        if counter > 10000 {
            break;
        }
        let new_name = format!("{}({}){}", stem, counter, ext);
        dest_path = dest_dir.join(new_name);
        counter += 1;
    }
    dest_path
}

#[cfg(test)]
mod tests {
    use super::*;
    use filetime::FileTime;
    use tempfile::tempdir;

    #[test]
    fn test_resolve_collision_auto_heal() {
        let dir = tempdir().unwrap();
        let dest_dir = dir.path().to_path_buf();

        let original = dest_dir.join("photo.jpg");
        fs::write(&original, "data").unwrap();

        let mut auto_healed = original.clone();
        auto_healed.set_extension("png");

        // 1. Normal auto-heal (no collision)
        let resolved = resolve_collision(&dest_dir, &auto_healed);
        assert_eq!(resolved.file_name().unwrap(), "photo.png");

        // 2. Collision after extension correction
        fs::write(&resolved, "png data").unwrap();
        let resolved_1 = resolve_collision(&dest_dir, &auto_healed);
        assert_eq!(resolved_1.file_name().unwrap(), "photo(1).png");

        // 3. Multiple collisions
        fs::write(&resolved_1, "png data").unwrap();
        let resolved_2 = resolve_collision(&dest_dir, &auto_healed);
        assert_eq!(resolved_2.file_name().unwrap(), "photo(2).png");
    }

    #[test]
    fn test_set_file_timestamps() {
        let dir = tempdir().unwrap();
        let test_file = dir.path().join("test.jpg");
        fs::write(&test_file, "test data").unwrap();

        // Set to a known timestamp (2023-01-01 00:00:00 UTC)
        let timestamp = 1672531200i64;
        let ft = FileTime::from_unix_time(timestamp, 0);
        filetime::set_file_times(&test_file, ft, ft).unwrap();

        let metadata = fs::metadata(&test_file).unwrap();
        let mtime = FileTime::from_last_modification_time(&metadata);
        assert_eq!(mtime.unix_seconds(), timestamp);
    }

    #[test]
    fn test_output_dir_names() {
        assert_eq!(DIR_COMPLETED, "Completed");
        assert_eq!(DIR_UNMATCHED, "Unmatched");
        assert_eq!(DIR_ERRORS, "Errors");
    }

    #[test]
    fn test_zip_by_name_perf() {
        use std::time::Instant;
        use zip::write::SimpleFileOptions;

        let dir = tempdir().unwrap();
        let zip_path = dir.path().join("test.zip");
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

        for i in 0..50000 {
            zip.start_file(format!("file_{}.txt", i), options).unwrap();
        }
        zip.finish().unwrap();

        let file = std::fs::File::open(&zip_path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();

        let start = Instant::now();
        for i in 0..1000 {
            let _ = archive.by_name(&format!("file_{}.txt", i)).unwrap();
        }
        let elapsed = start.elapsed();
        println!("Time for 1000 by_name lookups: {:?}", elapsed);
    }

    #[test]
    fn test_p0_003_cleanup_staging_does_not_delete_other_runs() {
        let dir = tempdir().unwrap();
        let output_dir = dir.path().to_path_buf();
        let run_1_dir = output_dir.join(".staging").join("run_1");
        let run_2_dir = output_dir.join(".staging").join("run_2");

        fs::create_dir_all(&run_1_dir).unwrap();
        fs::create_dir_all(&run_2_dir).unwrap();
        fs::write(run_1_dir.join("file1.tmp"), "run 1 data").unwrap();
        fs::write(run_2_dir.join("file2.tmp"), "run 2 data").unwrap();

        let db_path = dir.path().join("test.db");
        let db = StateDatabase::open(&db_path).unwrap();
        let config = Config::default();

        #[cfg(windows)]
        let mock_bin = dir.path().join("mock.bat");
        #[cfg(not(windows))]
        let mock_bin = dir.path().join("mock.sh");

        #[cfg(windows)]
        fs::write(
            &mock_bin,
            "@echo off\n:loop\nset /p line=\nif \"%line%\"==\"-execute\" echo {ready}\ngoto loop\n",
        )
        .unwrap();

        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::write(&mock_bin, "#!/bin/sh\nwhile read line; do [ \"$line\" = \"-execute\" ] && echo \"{ready}\"; done\n").unwrap();
            let mut perms = fs::metadata(&mock_bin).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&mock_bin, perms).unwrap();
        }

        let pool = ExifToolPool::new(mock_bin, 1).unwrap();
        let publisher = crate::events::Broadcaster::new();

        let processor = Processor {
            db: &db,
            config: &config,
            pool: &pool,
            output_dir: output_dir.clone(),
            publisher: &publisher,
            disk_checker: Box::new(SysinfoDiskChecker::new()),
            run_id: "run_1".to_string(),
            zip_json_index_cache: Mutex::new(HashMap::new()),
        };

        processor.cleanup_staging();

        // run_1 staging should be deleted, but run_2 staging must be untouched!
        assert!(!run_1_dir.exists(), "run_1 staging dir must be deleted");
        assert!(
            run_2_dir.exists(),
            "run_2 staging dir MUST NOT be deleted by run_1 cleanup"
        );
        assert!(run_2_dir.join("file2.tmp").exists());
    }
}
