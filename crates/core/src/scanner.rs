use crate::config::Config;
use crate::error::AppError;
use crate::events::EventPublisher;
use crate::state_db::{FilePath, JsonEntry, MediaFileInsert, StateDatabase};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Statistics representing the results of a file scan operation.
#[derive(Debug, Default)]
pub struct ScanStats {
    /// The number of valid media files (images/videos) discovered.
    pub media_count: usize,
    /// The number of valid JSON sidecar metadata files discovered.
    pub json_count: usize,
}

/// Recursively scans input directories and ZIP archives to identify media files and JSON metadata sidecars.
/// It validates zip archives for compression bombs and directly inserts discovered files into the SQLite database.
///
/// # Arguments
/// * `inputs` - A list of paths to directories or `.zip` files to scan.
/// * `db` - The SQLite state database where entries are inserted.
/// * `config` - Application configuration containing supported extensions.
/// * `cancel` - Atomic flag to abort the scan midway.
/// * `publisher` - Event publisher for progress updates.
pub fn scan_inputs(
    inputs: &[PathBuf],
    db: &StateDatabase,
    config: &Config,
    cancel: &AtomicBool,
    publisher: &dyn EventPublisher,
) -> Result<ScanStats, AppError> {
    let mut stats = ScanStats::default();
    let mut media_batch = Vec::with_capacity(5000);
    let mut json_batch = Vec::with_capacity(5000);

    let mut total_scanned = 0;

    for input in inputs {
        if cancel.load(Ordering::Relaxed) {
            break;
        }

        if input.is_dir() {
            let base_components = input.components().count();
            for entry in walkdir::WalkDir::new(input)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }

                if entry.file_type().is_dir() {
                    continue;
                }

                let path = entry.path();
                process_file(
                    path,
                    &mut stats,
                    &mut media_batch,
                    &mut json_batch,
                    db,
                    config,
                    || FilePath::Real {
                        base_components,
                        abs: path.to_path_buf(),
                    },
                )?;

                total_scanned += 1;
                if total_scanned % 1000 == 0 {
                    // publisher.publish(AppEvent::ProgressStats { ... }); // Optional: emit scan speed
                }
            }
        } else if input
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_lowercase())
            .as_deref()
            == Some("zip")
        {
            let file = fs::File::open(input).map_err(AppError::Io)?;
            let mut archive = zip::ZipArchive::new(file)?;

            for i in 0..archive.len() {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }

                let file_in_zip = archive.by_index(i)?;
                if file_in_zip.is_dir() {
                    continue;
                }

                let internal_path = file_in_zip.name().to_string();
                if internal_path.contains("..") {
                    continue; // Skip traversal attempts
                }

                let size = file_in_zip.size() as i64;
                let path = Path::new(&internal_path);

                // We mock the fs metadata size for zip entries by using the uncompressed size
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| format!(".{}", e.to_lowercase()))
                    .unwrap_or_default();

                let is_json = ext == ".json";

                // Prevent zip bombs during scan by checking compression ratio for media files
                let uncompressed = file_in_zip.size();
                let compressed = file_in_zip.compressed_size();
                if !is_json && uncompressed > 10_000_000 && compressed > 0 {
                    let ratio = uncompressed as f64 / compressed as f64;
                    if ratio > 10.0 {
                        return Err(AppError::SecurityThreat(format!(
                            "Zip Bomb detected in archive: {}",
                            input.display()
                        )));
                    }
                }

                let filename = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();

                let is_media = config.supported_image_extensions.contains(&ext)
                    || config.supported_video_extensions.contains(&ext);

                let fp = FilePath::zip(input.to_path_buf(), internal_path.clone())?;

                if is_media {
                    media_batch.push(MediaFileInsert {
                        path: fp,
                        filename,
                        extension: ext,
                        size,
                    });
                    stats.media_count += 1;
                } else if is_json {
                    json_batch.push(JsonEntry {
                        id: 0, // Ignored on insert
                        path: fp,
                        filename,
                    });
                    stats.json_count += 1;
                }

                if media_batch.len() >= 5000 {
                    db.insert_media_batch(&media_batch)?;
                    media_batch.clear();
                }

                if json_batch.len() >= 5000 {
                    db.insert_json_batch(&json_batch)?;
                    json_batch.clear();
                }

                total_scanned += 1;
                if total_scanned % 500 == 0 {
                    publisher.publish(crate::events::AppEvent::ProgressStats {
                        completed: stats.media_count as u64,
                        total: stats.media_count as u64,
                        eta_seconds: None,
                        speed_bps: 0,
                    });
                }
            }
        }
    }

    if !media_batch.is_empty() {
        db.insert_media_batch(&media_batch)?;
    }

    if !json_batch.is_empty() {
        db.insert_json_batch(&json_batch)?;
    }

    Ok(stats)
}

fn process_file<F>(
    path: &Path,
    stats: &mut ScanStats,
    media_batch: &mut Vec<MediaFileInsert>,
    json_batch: &mut Vec<JsonEntry>,
    db: &StateDatabase,
    config: &Config,
    fp_builder: F,
) -> Result<(), AppError>
where
    F: FnOnce() -> FilePath,
{
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e.to_lowercase()))
        .unwrap_or_default();

    let is_media = config.supported_image_extensions.contains(&ext)
        || config.supported_video_extensions.contains(&ext);
    let is_json = ext == ".json";

    if is_media || is_json {
        let size = path.metadata().map(|m| m.len() as i64).unwrap_or(0);
        let filename = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let fp = fp_builder();

        if is_media {
            media_batch.push(MediaFileInsert {
                path: fp,
                filename,
                extension: ext,
                size,
            });
            stats.media_count += 1;

            if media_batch.len() >= 5000 {
                db.insert_media_batch(media_batch)?;
                media_batch.clear();
            }
        } else if is_json {
            json_batch.push(JsonEntry {
                id: 0,
                path: fp,
                filename,
            });
            stats.json_count += 1;

            if json_batch.len() >= 5000 {
                db.insert_json_batch(json_batch)?;
                json_batch.clear();
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use tempfile::tempdir;

    #[test]
    fn test_scanner_directory() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("photo.jpg");
        fs::write(&file_path, "fake image data").unwrap();

        let db_path = dir.path().join("test.db");
        let db = StateDatabase::open(&db_path).unwrap();
        let config = Config::default();
        let cancel = AtomicBool::new(false);

        let publisher = crate::events::Broadcaster::new();

        let inputs = vec![dir.path().to_path_buf()];
        let stats = scan_inputs(&inputs, &db, &config, &cancel, &publisher).unwrap();

        assert_eq!(stats.media_count, 1);
        assert_eq!(stats.json_count, 0);
    }
}
