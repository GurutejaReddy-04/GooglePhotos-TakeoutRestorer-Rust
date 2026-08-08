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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use tracing::{debug, error, info};

const DIR_COMPLETED: &str = "Completed";
const DIR_UNMATCHED: &str = "Unmatched";
const DIR_ERRORS: &str = "Errors";
const DIR_LOGS: &str = "Logs";

static FILE_MOVE_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Trait representing a checker for available disk space.
/// Useful for mocking during testing or providing custom OS-specific implementations.
pub trait DiskSpaceChecker: Send + Sync {
    /// Returns the available bytes on the disk partition where `path` is located.
    fn available_bytes(&self, path: &Path) -> u64;
}

/// A concrete implementation of `DiskSpaceChecker` that queries only the target drive.
/// This avoids `sysinfo::Disks::new_with_refreshed_list()` which enumerates ALL system
/// drives and can hang indefinitely on Windows when network/USB/optical drives are unresponsive.
pub struct SysinfoDiskChecker;

impl SysinfoDiskChecker {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SysinfoDiskChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl DiskSpaceChecker for SysinfoDiskChecker {
    fn available_bytes(&self, path: &Path) -> u64 {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            // Use GetDiskFreeSpaceExW to query only the target path's drive,
            // avoiding full disk enumeration that can hang on unresponsive drives.
            let wide_path: Vec<u16> = path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let mut free_bytes_available: u64 = 0;
            let ret = unsafe {
                windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                    wide_path.as_ptr(),
                    &mut free_bytes_available as *mut u64,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if ret != 0 {
                free_bytes_available
            } else {
                u64::MAX // Fallback if API call fails
            }
        }
        #[cfg(not(windows))]
        {
            // On non-Windows, use sysinfo as a fallback
            use sysinfo::Disks;
            let disks = Disks::new_with_refreshed_list();
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
                u64::MAX
            }
        }
    }
}

/// The central processor that orchestrates the matching and metadata restoration phases.
/// It coordinates the `StateDatabase`, `Matcher`, and `ExifToolPool` to process files
/// concurrently while reporting progress and managing disk space.
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

            std::thread::scope(|s| {
                let (tx, rx) = std::sync::mpsc::sync_channel(2000);

                let pool_size = self.pool.total_size();

                // Consumer thread pool: EXIF processing
                s.spawn(move || {
                    rx.into_iter().par_bridge().for_each(
                        |(media, original_target_path): (MediaFile, PathBuf)| {
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
                            let is_non_standard = media.extension.is_empty()
                                || media.extension == "."
                                || !self
                                    .config
                                    .supported_image_extensions
                                    .contains(&media.extension)
                                    && !self
                                        .config
                                        .supported_video_extensions
                                        .contains(&media.extension);

                            let correction = if is_non_standard {
                                crate::auto_heal::get_correction(&target_path, &media.extension)
                            } else {
                                None
                            };

                            if let Some(true_ext) = correction {
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
                                        std::fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0)
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
                });

                // Producer thread: Extraction
                let extracted_bytes = std::sync::Arc::clone(&extracted_bytes);
                for media in real_files {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let dest = self.resolve_staging_path(media);
                    if let Ok(true) = self.db.try_mark_processing(media.id) {
                        if self.config.processing.output_mode == OutputMode::InPlace {
                            if let FilePath::Real { abs: p, .. } = &media.path {
                                let _ = tx.send((media.clone(), p.clone()));
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
                                let _ = tx.send((media.clone(), dest));
                            }
                        }
                    }
                }

                let completed_extracted = std::sync::Arc::clone(&completed_extracted);
                for (archive_path, files) in archive_map {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }

                    let file_result = fs::File::open(&archive_path);
                    if let Ok(file) = file_result {
                        if let Ok(mut zip) = zip::ZipArchive::new(file) {
                            let name_to_index: std::collections::HashMap<String, usize> = (0..zip
                                .len())
                                .filter_map(|idx| {
                                    zip.by_index(idx).ok().map(|f| (f.name().to_string(), idx))
                                })
                                .collect();

                            // Pre-populate the zip index cache so consumer threads
                            // don't redundantly rebuild it (thundering herd prevention).
                            {
                                let mut cache = self
                                    .zip_json_index_cache
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner());
                                cache.insert(archive_path.clone(), name_to_index.clone());
                            }

                            for media in files.iter() {
                                if cancel.load(Ordering::Relaxed) {
                                    continue;
                                }

                                let dest = self.resolve_staging_path(media);
                                let is_claimed =
                                    self.db.try_mark_processing(media.id).unwrap_or(false);
                                if !is_claimed {
                                    let current_cnt =
                                        completed_extracted.fetch_add(1, Ordering::Relaxed) + 1;
                                    if current_cnt.is_multiple_of(25) || current_cnt == grand_total
                                    {
                                        self.publisher.publish(AppEvent::ProgressStats {
                                            completed: current_cnt,
                                            total: grand_total,
                                            eta_seconds: None,
                                            speed_bps: 0,
                                        });
                                    }
                                    continue;
                                }

                                if let FilePath::Zip { internal, .. } = &media.path {
                                    let extract_res = (|| -> Result<u64, AppError> {
                                        let idx =
                                            *name_to_index.get(internal).ok_or_else(|| {
                                                AppError::Io(std::io::Error::other(format!(
                                                    "Zip entry not found: {}",
                                                    internal
                                                )))
                                            })?;
                                        // Reuse the outer zip handle — avoids re-reading
                                        // the central directory for each extraction.
                                        let mut zip_file = zip.by_index(idx)?;

                                        if let Some(p) = dest.parent() {
                                            let _ = fs::create_dir_all(p);
                                        }
                                        let mut file_name =
                                            dest.file_name().unwrap_or_default().to_os_string();
                                        file_name.push(".partial");
                                        let temp_dest = dest.with_file_name(file_name);

                                        let raw_file =
                                            fs::File::create(&temp_dest).map_err(AppError::Io)?;
                                        let mut out_file =
                                            std::io::BufWriter::with_capacity(64 * 1024, raw_file);
                                        use std::io::{Read, Write};
                                        const MAX_SAFE_FILE_SIZE: u64 = 20_000_000_000;
                                        let mut bounded_reader =
                                            zip_file.by_ref().take(MAX_SAFE_FILE_SIZE + 1);
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
                                        drop(zip_file);

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

                                        // Sidecar JSON Staging Optimization:
                                        // If media has a matched sidecar JSON in the same archive,
                                        // extract it directly to dest.parent()/sidecar.json using our open zip handle.
                                        if let Some(FilePath::Zip {
                                            archive: json_archive,
                                            internal: json_internal,
                                        }) = &media.json_path
                                        {
                                            if json_archive == &archive_path {
                                                if let Some(&json_idx) =
                                                    name_to_index.get(json_internal)
                                                {
                                                    if let Ok(mut json_zf) = zip.by_index(json_idx)
                                                    {
                                                        if let Some(p) = dest.parent() {
                                                            let sidecar_path =
                                                                p.join("sidecar.json");
                                                            if let Ok(mut sidecar_file) =
                                                                fs::File::create(&sidecar_path)
                                                            {
                                                                let _ = std::io::copy(
                                                                    &mut json_zf,
                                                                    &mut sidecar_file,
                                                                );
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }

                                        Ok(size)
                                    })();

                                    match extract_res {
                                        Ok(size) => {
                                            let prev =
                                                extracted_bytes.fetch_add(size, Ordering::Relaxed);
                                            let current_cnt = completed_extracted
                                                .fetch_add(1, Ordering::Relaxed)
                                                + 1;
                                            if (prev + size) / 500_000_000 > prev / 500_000_000 {
                                                let avail = self
                                                    .disk_checker
                                                    .available_bytes(&self.output_dir);
                                                if avail < 5_000_000_000 && avail != u64::MAX {
                                                    self.publisher.publish(AppEvent::Error {
                                                        file_id: None,
                                                        fatal: true,
                                                        message: "Disk full during extraction"
                                                            .into(),
                                                    });
                                                    cancel.store(true, Ordering::Relaxed);
                                                }
                                            }
                                            if current_cnt.is_multiple_of(25)
                                                || current_cnt == grand_total
                                            {
                                                self.publisher.publish(AppEvent::ProgressStats {
                                                    completed: current_cnt,
                                                    total: grand_total,
                                                    eta_seconds: None,
                                                    speed_bps: size,
                                                });
                                            }
                                            let _ = tx.send(((*media).clone(), dest));
                                        }
                                        Err(e) => {
                                            let current_cnt = completed_extracted
                                                .fetch_add(1, Ordering::Relaxed)
                                                + 1;
                                            if current_cnt.is_multiple_of(25)
                                                || current_cnt == grand_total
                                            {
                                                self.publisher.publish(AppEvent::ProgressStats {
                                                    completed: current_cnt,
                                                    total: grand_total,
                                                    eta_seconds: None,
                                                    speed_bps: 0,
                                                });
                                            }
                                            if let Err(db_err) = self.db.enqueue_status_update(
                                                StatusUpdate::Error(media.id, e.to_string()),
                                            ) {
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
                            }
                        }
                    }
                }
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

        let dest_path = {
            let _lock = FILE_MOVE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            resolve_collision(&dest_dir, current_path)
        };

        match fs::rename(current_path, &dest_path) {
            Ok(_) => dest_path,
            Err(_e) => match copy_buffered(current_path, &dest_path) {
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

        let dest_path = {
            let _lock = FILE_MOVE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            resolve_collision(&dest_dir, current_path)
        };

        match fs::rename(current_path, &dest_path) {
            Ok(_) => dest_path,
            Err(_e) => match copy_buffered(current_path, &dest_path) {
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

        // Fast-path: Check if producer pre-extracted sidecar.json into the staging directory
        let json_content = if let Some(parent) = target_path.parent() {
            let sidecar_file = parent.join("sidecar.json");
            if sidecar_file.exists() {
                std::fs::read_to_string(&sidecar_file).map_err(AppError::Io)?
            } else {
                self.read_json_from_source(json_path)?
            }
        } else {
            self.read_json_from_source(json_path)?
        };

        let parsed = parse(json_content.as_bytes())?;

        self.pool
            .execute(|engine| engine.update_metadata(target_path, &parsed))?;

        Ok(Some(parsed))
    }

    /// Reads JSON sidecar content from file system or zip archive fallback.
    fn read_json_from_source(&self, json_path: &FilePath) -> Result<String, AppError> {
        match json_path {
            FilePath::Real { abs, .. } => std::fs::read_to_string(abs).map_err(AppError::Io),
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
                        let mut cache = self
                            .zip_json_index_cache
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        if let Some(idx) = cache.get(archive).and_then(|m| m.get(internal).copied())
                        {
                            Some(idx)
                        } else {
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
                                cache.insert(archive.clone(), map);
                                idx
                            } else {
                                None
                            }
                        }
                    }
                };

                if let Some(i) = idx {
                    let file = std::fs::File::open(archive).map_err(AppError::Io)?;
                    let mut zip = zip::ZipArchive::new(file)?;
                    let mut zf = zip.by_index(i)?;
                    let mut s = String::new();
                    std::io::Read::read_to_string(&mut zf, &mut s).map_err(AppError::Io)?;
                    Ok(s)
                } else {
                    let file = std::fs::File::open(archive).map_err(AppError::Io)?;
                    let mut zip = zip::ZipArchive::new(file)?;
                    let mut zf = zip.by_name(internal)?;
                    let mut s = String::new();
                    std::io::Read::read_to_string(&mut zf, &mut s).map_err(AppError::Io)?;
                    Ok(s)
                }
            }
        }
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

fn copy_buffered(src: &Path, dst: &Path) -> std::io::Result<u64> {
    use std::io::Write;
    let mut reader = std::io::BufReader::with_capacity(1_048_576, std::fs::File::open(src)?);
    let mut writer = std::io::BufWriter::with_capacity(1_048_576, std::fs::File::create(dst)?);
    let size = std::io::copy(&mut reader, &mut writer)?;
    writer.flush()?;
    Ok(size)
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
            "@echo off\n:loop\nset /p line=\nif \"%line:~0,4%\"==\"-ver\" (echo 13.59\necho {ready})\nif \"%line:~0,8%\"==\"-execute\" echo {ready}\ngoto loop\n",
        )
        .unwrap();

        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::write(&mock_bin, "#!/bin/sh\nwhile read line; do [ \"$line\" = \"-ver\" ] && echo \"13.59\"; [ \"$line\" = \"-execute\" ] && echo \"{ready}\"; done\n").unwrap();
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
