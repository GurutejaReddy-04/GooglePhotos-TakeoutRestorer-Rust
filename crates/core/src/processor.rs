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
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{debug, error, info};

/// Output subdirectory names for organized output.
const DIR_COMPLETED: &str = "Completed";
const DIR_UNMATCHED: &str = "Unmatched";
const DIR_ERRORS: &str = "Errors";
use std::sync::atomic::AtomicU64;

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
        let batch_size = 1000;
        let mut last_id = None;

        loop {
            if cancel.load(Ordering::Relaxed) {
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

            let total_ready = ready_files.len() as u64;
            // P0.2 / P1.2 Fix: Use std::thread::scope to pipeline extraction and EXIF processing
            let extracted_bytes_clone = std::sync::Arc::clone(&extracted_bytes);
            std::thread::scope(|s| {
                let (tx, rx) = std::sync::mpsc::sync_channel(100);

                // Producer thread: Extraction
                s.spawn(move || {
                    let extracted_bytes = extracted_bytes_clone;
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
                                let mut file_name =
                                    dest.file_name().unwrap_or_default().to_os_string();
                                file_name.push(".partial");
                                let temp_dest = dest.with_file_name(file_name);
                                let copy_result = fs::copy(p, &temp_dest)
                                    .and_then(|_| fs::rename(&temp_dest, &dest));
                                if let Err(e) = copy_result {
                                    let _ = fs::remove_file(&temp_dest);
                                    if let Err(db_err) =
                                        self.db.enqueue_status_update(StatusUpdate::Error(
                                            media.id,
                                            format!("Copy failed: {}", e),
                                        ))
                                    {
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

                    for (archive_path, files) in archive_map {
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }

                        let file_result = fs::File::open(&archive_path);
                        if let Ok(file) = file_result {
                            if let Ok(mut zip) = zip::ZipArchive::new(file) {
                                for media in files {
                                    if cancel.load(Ordering::Relaxed) {
                                        break;
                                    }

                                    let dest = self.resolve_staging_path(media);
                                    if let Ok(true) = self.db.try_mark_processing(media.id) {
                                        if let FilePath::Zip { internal, .. } = &media.path {
                                            let extract_res = (|| -> Result<u64, AppError> {
                                                let mut zip_file = zip.by_name(internal)?;
                                                // Ensure parent dir exists (redundant with resolve_target_path but safe)
                                                if let Some(p) = dest.parent() {
                                                    let _ = fs::create_dir_all(p);
                                                }
                                                // TEMPORARY EXTENSION STRATEGY
                                                let mut file_name = dest
                                                    .file_name()
                                                    .unwrap_or_default()
                                                    .to_os_string();
                                                file_name.push(".partial");
                                                let temp_dest = dest.with_file_name(file_name);

                                                let mut out_file = fs::File::create(&temp_dest)
                                                    .map_err(AppError::Io)?;
                                                let size =
                                                    std::io::copy(&mut zip_file, &mut out_file)
                                                        .map_err(|e| {
                                                            let _ = fs::remove_file(&temp_dest);
                                                            AppError::Io(e)
                                                        })?;
                                                drop(out_file);
                                                fs::rename(&temp_dest, &dest).map_err(|e| {
                                                    let _ = fs::remove_file(&temp_dest);
                                                    AppError::Io(e)
                                                })?;
                                                Ok(size)
                                            })(
                                            );

                                            match extract_res {
                                                Ok(size) => {
                                                    let prev = extracted_bytes
                                                        .fetch_add(size, Ordering::Relaxed);
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
                                                    self.publisher.publish(
                                                        AppEvent::ProgressStats {
                                                            completed: 0, // Publisher stats can be refined
                                                            total: total_ready,
                                                            eta_seconds: None,
                                                            speed_bps: size,
                                                        },
                                                    );
                                                    let _ = tx.send((media.clone(), dest));
                                                }
                                                Err(e) => {
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
                                                        cancel.store(true, Ordering::Relaxed);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                });

                // Consumer thread pool: EXIF processing
                rx.into_iter()
                    .par_bridge()
                    .for_each(|(media, original_target_path)| {
                        if cancel.load(Ordering::Relaxed) {
                            return;
                        }
                        while pause.load(Ordering::Relaxed) && !cancel.load(Ordering::Relaxed) {
                            std::thread::sleep(std::time::Duration::from_millis(100));
                        }

                        let mut target_path = original_target_path.clone();

                        // Auto-heal: detect and fix mismatched file extensions
                        if let Some(true_ext) =
                            crate::auto_heal::get_correction(&target_path, &media.extension)
                        {
                            let mut proposed_path = target_path.clone();
                            proposed_path.set_extension(true_ext.trim_start_matches('.'));

                            let _lock = FILE_MOVE_MUTEX.lock().unwrap();
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
                                    self.set_file_timestamps(&target_path, meta.taken_timestamp);
                                }

                                // Move file to appropriate output subdirectory
                                let final_path = self.move_to_output_subdir(
                                    &target_path,
                                    &media,
                                    parsed_meta.is_some(),
                                );

                                let bytes_written =
                                    std::fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0);
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
                                let final_path = self.move_to_error_subdir(&target_path, &media);

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
                    });
            });
        }

        self.db.flush()?;
        info!("Processing phase complete.");
        Ok(())
    }

    /// Creates the output subdirectories if they don't exist.
    fn ensure_output_dirs(&self) -> Result<(), AppError> {
        if self.config.processing.output_mode == OutputMode::InPlace {
            return Ok(());
        }

        for dir_name in [DIR_COMPLETED, DIR_UNMATCHED, DIR_ERRORS] {
            let dir = self.output_dir.join(dir_name);
            if !dir.exists() {
                fs::create_dir_all(&dir).map_err(AppError::Io)?;
            }
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
        if !dest_dir.exists() {
            let _ = fs::create_dir_all(&dest_dir);
        }

        let _lock = FILE_MOVE_MUTEX.lock().unwrap();
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

        let _lock = FILE_MOVE_MUTEX.lock().unwrap();
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
                let file = std::fs::File::open(archive)?;
                let mut zip = zip::ZipArchive::new(file)?;
                let mut zf = zip.by_name(internal)?;
                let mut s = String::new();
                std::io::Read::read_to_string(&mut zf, &mut s)?;
                s
            }
        };

        let parsed = parse(json_content.as_bytes())?;

        self.pool
            .execute(|engine| engine.update_metadata(target_path, &parsed))?;

        Ok(Some(parsed))
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
}
