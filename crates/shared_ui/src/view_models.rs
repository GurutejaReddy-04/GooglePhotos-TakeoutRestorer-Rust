use core::events::AppEvent;
use core::state_db::{FileStatus, RecentRun};
use core::validation::DestinationValidationKind;
use serde::Serialize;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Serialize, PartialEq, Default)]
pub enum DestinationValidation {
    #[default]
    NotSet,
    Valid {
        free_gb: f64,
        total_gb: f64,
        pct_free: f64,
        message: String,
    },
    Warning {
        free_gb: f64,
        total_gb: f64,
        pct_free: f64,
        message: String,
    },
    Error {
        message: String,
    },
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct SettingsSnapshot {
    pub destination_path: Option<String>,
    pub destination_validation: DestinationValidation,
    pub gps_enabled: bool,
    pub timezone_enabled: bool,
    pub unmatched_enabled: bool,
    pub anonymous_logging: bool,
    pub output_mode: String,
    pub high_performance: bool,
    pub theme: String,
}

impl Default for SettingsSnapshot {
    fn default() -> Self {
        Self {
            destination_path: None,
            destination_validation: DestinationValidation::NotSet,
            gps_enabled: true,
            timezone_enabled: true,
            unmatched_enabled: true,
            anonymous_logging: false,
            output_mode: "copy".to_string(),
            high_performance: false,
            theme: "System".to_string(),
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Default)]
pub struct FileResult {
    pub filename: String,
    pub status: String,
    pub destination: String,
    pub timestamp: String,
    pub media_type: String,
    pub error: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ProcessingSnapshot {
    pub sequence_number: u64,
    pub generation_timestamp: u128,

    pub current_phase_text: String,
    pub formatted_progress: String,
    pub eta_text: String,
    pub speed_text: String,
    pub elapsed_text: String,
    pub current_filename: String,
    pub has_errors: bool,
    pub error_count: usize,
    pub ok_count: usize,
    pub skipped_count: usize,
    pub unmatched_count: usize,

    pub total_files: usize,
    pub completed_files: usize,
    pub image_count: usize,
    pub video_count: usize,
    pub output_bytes: u64,

    pub is_finished: bool,
    pub is_paused: bool,
    pub is_processing: bool,
    pub terminal_state: String,
    pub fatal_error_message: String,

    pub recent_runs: Vec<RecentRun>,
    pub settings: SettingsSnapshot,
    pub results: Vec<FileResult>,
}

#[derive(Clone, Debug, Serialize)]
pub struct FileResultViewModel {
    pub filename: String,
    pub status: String,
    pub destination: String,
    pub timestamp: String,
}

impl Default for ProcessingSnapshot {
    fn default() -> Self {
        Self {
            sequence_number: 0,
            generation_timestamp: current_timestamp(),
            completed_files: 0,
            total_files: 0,
            current_phase_text: "Initializing".to_string(),
            formatted_progress: "0.0%".to_string(),
            eta_text: "Calculating...".to_string(),
            speed_text: "0 B/s".to_string(),
            has_errors: false,
            error_count: 0,
            ok_count: 0,
            skipped_count: 0,
            unmatched_count: 0,
            elapsed_text: "0s".to_string(),
            current_filename: "".to_string(),
            image_count: 0,
            video_count: 0,
            output_bytes: 0,
            is_finished: false,
            is_paused: false,
            is_processing: false,
            terminal_state: "idle".to_string(),
            fatal_error_message: String::new(),
            recent_runs: Vec::new(),
            settings: SettingsSnapshot::default(),
            results: Vec::new(),
        }
    }
}

pub enum SnapshotPolicy {
    Immediate,
    Debounced(Duration),
    Manual,
}

pub struct ProcessingViewModelBuilder {
    sequence_number: u64,
    completed_files: usize,
    total_files: usize,
    current_phase_text: String,
    eta_text: String,
    speed_text: String,
    has_errors: bool,
    error_count: usize,
    ok_count: usize,
    skipped_count: usize,
    unmatched_count: usize,
    image_count: usize,
    video_count: usize,
    output_bytes: u64,
    start_time: Option<std::time::Instant>,
    current_filename: String,
    pub is_finished: bool,
    pub is_paused: bool,
    pub is_processing: bool,
    pub terminal_state: String,
    pub fatal_error_message: String,
    pub last_snapshot_time: Option<std::time::Instant>,
    pub last_completed_files: usize,
    pub ema_speed: Option<f64>,

    pub recent_runs: Vec<RecentRun>,
    pub settings: SettingsSnapshot,

    // Raw results and filter state
    pub raw_results: Vec<FileResult>,
    pub search_query: String,
    pub status_filter: String,
}

impl ProcessingViewModelBuilder {
    pub fn new() -> Self {
        Self {
            sequence_number: 0,
            current_phase_text: "Initializing...".to_string(),
            completed_files: 0,
            total_files: 0,
            eta_text: "Calculating...".to_string(),
            speed_text: "0 B/s".to_string(),
            has_errors: false,
            error_count: 0,
            ok_count: 0,
            skipped_count: 0,
            unmatched_count: 0,
            image_count: 0,
            video_count: 0,
            output_bytes: 0,
            start_time: None,
            current_filename: "".to_string(),
            is_finished: false,
            is_paused: false,
            is_processing: false,
            terminal_state: "idle".to_string(),
            fatal_error_message: String::new(),
            last_snapshot_time: None,
            last_completed_files: 0,
            ema_speed: None,
            recent_runs: Vec::new(),
            settings: SettingsSnapshot::default(),
            raw_results: Vec::new(),
            search_query: "".to_string(),
            status_filter: "All Files".to_string(),
        }
    }
}

impl Default for ProcessingViewModelBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessingViewModelBuilder {
    pub fn apply_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::ProcessingPhaseStarted { name, total_files } => {
                self.is_processing = true;
                self.is_finished = false;
                if name.starts_with("Restoring") && self.start_time.is_none() {
                    self.completed_files = 0;
                }
                self.current_phase_text = name;
                self.terminal_state = "running".to_string();
                self.fatal_error_message.clear();
                if self.start_time.is_none() {
                    self.start_time = Some(std::time::Instant::now());
                }
                if let Some(t) = total_files {
                    self.total_files = (t as usize).max(self.total_files);
                }
            }
            AppEvent::FileProcessed {
                status,
                file_id,
                bytes_written,
            } => {
                self.completed_files += 1;
                self.total_files = self.total_files.max(self.completed_files);
                self.output_bytes += bytes_written;
                self.current_filename = format!("File ID: {}", file_id);
                match status {
                    FileStatus::Completed => {
                        self.ok_count += 1;
                    }
                    FileStatus::Error => {
                        self.has_errors = true;
                        self.error_count += 1;
                    }
                    FileStatus::Skipped => self.skipped_count += 1,
                    FileStatus::Unmatched => self.unmatched_count += 1,
                    _ => {}
                }
                let total_done =
                    self.ok_count + self.error_count + self.skipped_count + self.unmatched_count;
                self.completed_files = total_done.min(self.total_files);
            }
            AppEvent::Warning { .. } => {
                self.has_errors = true;
                self.terminal_state = "completed_with_issues".to_string();
            }
            AppEvent::Error { fatal, message, .. } => {
                self.has_errors = true;
                self.terminal_state = "failed".to_string();
                if fatal {
                    self.is_processing = false;
                    self.is_finished = true;
                    self.current_phase_text = "Failed".to_string();
                    self.terminal_state = "failed".to_string();
                    self.fatal_error_message = message;
                }
            }
            AppEvent::ProgressStats {
                completed,
                total,
                eta_seconds: _,
                speed_bps: _,
            } => {
                let grand = (total as usize)
                    .max(completed as usize)
                    .max(self.total_files);
                self.total_files = grand;
                let total_done =
                    self.ok_count + self.error_count + self.skipped_count + self.unmatched_count;
                let cnt = (completed as usize).max(total_done);
                self.completed_files = cnt.min(self.total_files);
            }
            AppEvent::RecentRunsLoaded(runs) => {
                self.recent_runs = runs;
            }
            AppEvent::ConfigChanged(config) => {
                self.settings.gps_enabled = config.processing.gps_enabled;
                self.settings.timezone_enabled = config.processing.timezone_enabled;
                self.settings.unmatched_enabled = config.processing.unmatched_enabled;
                self.settings.anonymous_logging = config.processing.anonymous_logging;

                self.settings.output_mode = match config.processing.output_mode {
                    core::config::OutputMode::Copy => "copy".to_string(),
                    core::config::OutputMode::InPlace => "in-place".to_string(),
                };

                self.settings.high_performance = config.processing.high_performance;
                self.settings.theme = config.ui.theme.clone();
            }
            AppEvent::DestinationValidated(res) => {
                self.settings.destination_path = Some(res.path.to_string_lossy().to_string());
                let msg = res.message.clone();
                let free_gb = res.free_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                let total_gb = res.total_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                let pct_free = if total_gb > 0.0 {
                    (free_gb / total_gb) * 100.0
                } else {
                    0.0
                };

                self.settings.destination_validation = match res.kind {
                    DestinationValidationKind::Valid => DestinationValidation::Valid {
                        free_gb,
                        total_gb,
                        pct_free,
                        message: msg,
                    },
                    DestinationValidationKind::Warning => DestinationValidation::Warning {
                        free_gb,
                        total_gb,
                        pct_free,
                        message: msg,
                    },
                    DestinationValidationKind::Error => {
                        DestinationValidation::Error { message: msg }
                    }
                };
            }
            AppEvent::RunCompleted { results } => {
                self.is_processing = false;
                self.is_finished = true;
                self.completed_files = self.total_files;
                if self.terminal_state != "failed" {
                    self.current_phase_text = "Finished".to_string();
                    self.terminal_state = if self.has_errors {
                        "completed_with_issues".to_string()
                    } else {
                        "completed".to_string()
                    };
                }

                let mut image_cnt = 0;
                let mut video_cnt = 0;

                self.raw_results = results
                    .into_iter()
                    .map(|m| {
                        let ext = m.extension.to_lowercase();
                        let is_video = m.has_live_video
                            || ext == ".mp4"
                            || ext == ".mov"
                            || ext == ".avi"
                            || ext == ".mkv";

                        if m.status.is_terminal() && m.status != FileStatus::Skipped {
                            if is_video {
                                video_cnt += 1;
                            } else {
                                image_cnt += 1;
                            }
                        }

                        FileResult {
                            filename: m.filename,
                            status: match m.status {
                                FileStatus::Pending => "Pending".to_string(),
                                FileStatus::Matched | FileStatus::MatchedLowConfidence => {
                                    "Matched".to_string()
                                }
                                FileStatus::Unmatched => "Unmatched".to_string(),
                                FileStatus::Processing => "Processing".to_string(),
                                FileStatus::Completed => "Success".to_string(),
                                FileStatus::Error => "Error".to_string(),
                                FileStatus::Skipped => "Skipped".to_string(),
                            },
                            destination: match m.path {
                                core::state_db::FilePath::Real { abs: p, .. } => {
                                    p.to_string_lossy().to_string()
                                }
                                core::state_db::FilePath::Zip { archive, internal } => {
                                    format!("{}|{}", archive.to_string_lossy(), internal)
                                }
                            },
                            timestamp: "".to_string(), // MediaFile doesn't store completion time directly
                            media_type: if m.has_live_video {
                                "Live Photo".to_string()
                            } else if is_video {
                                "Video".to_string()
                            } else {
                                "Image".to_string()
                            },
                            error: m.error_message.unwrap_or_default(),
                        }
                    })
                    .collect();

                self.image_count = image_cnt;
                self.video_count = video_cnt;
            }
            AppEvent::CancellationAcknowledged => {
                self.is_processing = false;
                self.is_finished = true;
                self.current_phase_text = "Cancelled".to_string();
                self.terminal_state = "cancelled".to_string();
            }
            AppEvent::ResultsFilterChanged {
                search,
                status_filter,
            } => {
                self.search_query = search.to_lowercase();
                self.status_filter = status_filter;
            }
            AppEvent::ExifToolDownloadProgress {
                downloaded_bytes,
                total_bytes,
            } => {
                if total_bytes > 0 {
                    let pct = (downloaded_bytes as f64 / total_bytes as f64) * 100.0;
                    self.current_phase_text = format!("Downloading ExifTool... {:.0}%", pct);
                } else {
                    self.current_phase_text =
                        format!("Downloading ExifTool... {} KB", downloaded_bytes / 1024);
                }
            }
            AppEvent::StateReset => {
                // Reset all processing state to default but keep settings and recent_runs
                let settings = self.settings.clone();
                let recent_runs = self.recent_runs.clone();
                *self = ProcessingViewModelBuilder::new();
                self.settings = settings;
                self.recent_runs = recent_runs;
            }
        }
    }

    pub fn build_snapshot(&mut self) -> ProcessingSnapshot {
        self.sequence_number += 1;

        let grand_total = self.total_files.max(self.completed_files);
        let percentage = if grand_total > 0 {
            ((self.completed_files as f64 / grand_total as f64) * 100.0).min(100.0)
        } else {
            0.0
        };

        let is_meaningful_progress =
            self.total_files > 0 && !self.current_phase_text.contains("Initializing");
        let formatted_progress = if is_meaningful_progress {
            format!("{:.1}%", percentage)
        } else {
            "---".to_string()
        };

        let filtered_results: Vec<FileResult> = self
            .raw_results
            .iter()
            .filter(|r| {
                let matches_search = self.search_query.is_empty()
                    || r.filename.to_lowercase().contains(&self.search_query);
                let matches_status =
                    self.status_filter == "All Files" || r.status == self.status_filter;
                matches_search && matches_status
            })
            .cloned()
            .collect();

        let elapsed_text = if let Some(start) = self.start_time {
            let elapsed_secs = start.elapsed().as_secs();
            format_time(elapsed_secs)
        } else {
            "0s".to_string()
        };

        if let Some(start) = self.start_time {
            let elapsed_f64 = start.elapsed().as_secs_f64();

            // Calculate EMA
            let now = std::time::Instant::now();
            if let Some(last_time) = self.last_snapshot_time {
                let dt = now.duration_since(last_time).as_secs_f64();
                if dt > 0.1 {
                    let d_files = self
                        .completed_files
                        .saturating_sub(self.last_completed_files);
                    let current_speed = d_files as f64 / dt;
                    let alpha = 0.3; // Smoothing factor
                    let new_ema = self
                        .ema_speed
                        .map(|e| alpha * current_speed + (1.0 - alpha) * e)
                        .unwrap_or(current_speed);
                    self.ema_speed = Some(new_ema);
                }
            }
            self.last_snapshot_time = Some(now);
            self.last_completed_files = self.completed_files;

            if elapsed_f64 > 1.0 && self.completed_files > 0 {
                let lifetime_speed = self.completed_files as f64 / elapsed_f64;
                let blended_speed = self
                    .ema_speed
                    .map(|ema| (ema + lifetime_speed) / 2.0)
                    .unwrap_or(lifetime_speed);

                self.speed_text = format!("{:.1} files/s", blended_speed);

                let remaining = self.total_files.saturating_sub(self.completed_files);
                if remaining > 0 && blended_speed > 0.0 {
                    let eta_secs = (remaining as f64 / blended_speed) as u64;
                    self.eta_text = format_time(eta_secs);
                } else {
                    self.eta_text = "0s".to_string();
                }
            } else if elapsed_f64 <= 1.0 {
                self.eta_text = "Calculating...".to_string();
                self.speed_text = "Calculating...".to_string();
            }
        }

        ProcessingSnapshot {
            sequence_number: self.sequence_number,
            generation_timestamp: current_timestamp(),
            completed_files: self.completed_files,
            total_files: self.total_files,
            current_phase_text: self.current_phase_text.clone(),
            formatted_progress,
            eta_text: self.eta_text.clone(),
            speed_text: self.speed_text.clone(),
            has_errors: self.has_errors,
            elapsed_text,
            error_count: self.error_count,
            ok_count: self.ok_count,
            skipped_count: self.skipped_count,
            unmatched_count: self.unmatched_count,
            current_filename: self.current_filename.clone(),
            image_count: self.image_count,
            video_count: self.video_count,
            output_bytes: self.output_bytes,
            is_finished: self.is_finished,
            is_paused: self.is_paused,
            is_processing: self.is_processing,
            terminal_state: self.terminal_state.clone(),
            fatal_error_message: self.fatal_error_message.clone(),
            recent_runs: self.recent_runs.clone(),
            settings: self.settings.clone(),
            results: filtered_results,
        }
    }
}

fn current_timestamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn format_time(seconds: u64) -> String {
    let m = seconds / 60;
    let s = seconds % 60;
    if m > 60 {
        let h = m / 60;
        let rem_m = m % 60;
        format!("{}h {}m", h, rem_m)
    } else if m > 0 {
        format!("{}m {}s", m, s)
    } else {
        format!("{}s", s)
    }
}

#[cfg(test)]
mod tests {
    use super::ProcessingViewModelBuilder;
    use core::events::AppEvent;

    #[test]
    fn fatal_errors_remain_failed_after_the_terminal_event() {
        let mut builder = ProcessingViewModelBuilder::new();
        builder.apply_event(AppEvent::Error {
            file_id: None,
            fatal: true,
            message: "ExifTool could not start".to_string(),
        });
        builder.apply_event(AppEvent::RunCompleted {
            results: Vec::new(),
        });

        let snapshot = builder.build_snapshot();
        assert!(snapshot.is_finished);
        assert_eq!(snapshot.terminal_state, "failed");
        assert_eq!(snapshot.current_phase_text, "Failed");
        assert_eq!(snapshot.fatal_error_message, "ExifTool could not start");
    }

    #[test]
    fn cancelled_runs_have_a_distinct_terminal_state() {
        let mut builder = ProcessingViewModelBuilder::new();
        builder.apply_event(AppEvent::CancellationAcknowledged);

        let snapshot = builder.build_snapshot();
        assert!(snapshot.is_finished);
        assert_eq!(snapshot.terminal_state, "cancelled");
        assert_eq!(snapshot.current_phase_text, "Cancelled");
    }
}
