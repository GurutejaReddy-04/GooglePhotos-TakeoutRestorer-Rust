use crate::error::AppError;
use crate::parser::ParsedMetadata;
use chrono::{DateTime, Utc};
use crossbeam_channel::{unbounded, Receiver, Sender};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;
use tracing::{debug, error, info, warn};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x08000000;
const TIMEOUT_SECS: u64 = 600;

#[derive(Debug, PartialEq, Eq)]
pub enum ExifToolResult {
    Updated(usize),
    Unchanged,
    Failed(String),
}

pub fn parse_exiftool_output(output: &str) -> ExifToolResult {
    let out_lower = output.to_lowercase();

    let has_error = out_lower.contains("error")
        || out_lower.contains("weren't updated")
        || out_lower.contains("failed");

    if out_lower.contains("unchanged") && !has_error {
        return ExifToolResult::Unchanged;
    }

    if has_error {
        return ExifToolResult::Failed(output.trim().to_string());
    }

    if out_lower.contains("image file") && out_lower.contains("updated") {
        let mut num = 1;
        for word in out_lower.split_whitespace() {
            if let Ok(n) = word.parse::<usize>() {
                num = n;
                break;
            }
        }
        return ExifToolResult::Updated(num);
    }

    ExifToolResult::Failed(output.trim().to_string())
}

pub struct ExifToolEngine {
    binary_path: PathBuf,
    process: Mutex<Option<ProcessState>>,
}

struct ProcessState {
    child: Child,
    stdin: ChildStdin,
    stdout_receiver: Receiver<String>,
}

impl ExifToolEngine {
    pub fn new(binary_path: PathBuf) -> Self {
        Self {
            binary_path,
            process: Mutex::new(None),
        }
    }

    fn ensure_running(&self) -> Result<(), AppError> {
        let mut process_guard = self.process.lock().unwrap_or_else(|e| e.into_inner());
        if process_guard.is_some() {
            return Ok(());
        }

        debug!("Starting ExifTool persistent process");

        let mut cmd = if cfg!(windows) {
            let mut c = Command::new(&self.binary_path);
            #[cfg(windows)]
            c.creation_flags(CREATE_NO_WINDOW);
            c
        } else {
            let mut c = Command::new("perl");
            c.arg(&self.binary_path);
            c
        };

        cmd.args(["-stay_open", "True", "-@", "-", "-common_args"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());

        let mut child = cmd
            .spawn()
            .map_err(|e| AppError::ExifToolLaunchFailed(e.to_string()))?;

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();

        let (tx, rx) = unbounded();

        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                if let Ok(line) = line {
                    if tx.send(line).is_err() {
                        break;
                    }
                } else {
                    break;
                }
            }
        });

        *process_guard = Some(ProcessState {
            child,
            stdin,
            stdout_receiver: rx,
        });

        Ok(())
    }

    pub fn execute(&self, args: &[&str]) -> Result<String, AppError> {
        self.ensure_running()?;

        let mut process_guard = self.process.lock().unwrap_or_else(|e| e.into_inner());
        let process_state = match process_guard.as_mut() {
            Some(ps) => ps,
            None => return Err(AppError::ExifToolCrashed),
        };

        for arg in args {
            if let Err(e) = writeln!(process_state.stdin, "{}", arg) {
                error!("Failed to write to ExifTool stdin: {}", e);
                *process_guard = None; // Force restart next time
                return Err(AppError::Io(e));
            }
        }
        if let Err(e) = writeln!(process_state.stdin, "-execute") {
            error!("Failed to write execute command to ExifTool stdin: {}", e);
            *process_guard = None;
            return Err(AppError::Io(e));
        }
        if let Err(e) = process_state.stdin.flush() {
            error!("Failed to flush ExifTool stdin: {}", e);
            *process_guard = None;
            return Err(AppError::Io(e));
        }

        let mut output = String::new();
        let timeout = Duration::from_secs(TIMEOUT_SECS);

        loop {
            match process_state.stdout_receiver.recv_timeout(timeout) {
                Ok(line) => {
                    let trimmed = line.trim();
                    if trimmed == "{ready}" {
                        break;
                    }
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output.push_str(&line);
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    error!("ExifTool read timeout");
                    *process_guard = None;
                    return Err(AppError::ExifToolLaunchFailed(
                        "ExifTool timeout".to_string(),
                    ));
                }
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                    error!("ExifTool process disconnected");
                    *process_guard = None;
                    return Err(AppError::ExifToolCrashed);
                }
            }
        }

        Ok(output)
    }

    pub fn update_metadata(
        &self,
        target_file: &Path,
        metadata: &ParsedMetadata,
    ) -> Result<ExifToolResult, AppError> {
        let args = build_metadata_args(target_file, metadata);
        let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let output = self.execute(&args_ref)?;

        Ok(parse_exiftool_output(&output))
    }
}

pub fn build_metadata_args(target_file: &Path, metadata: &ParsedMetadata) -> Vec<String> {
    let mut args = vec!["-overwrite_original".to_string()];

    let ext = target_file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let is_video = ext == "mp4" || ext == "mov";

    let formatted_time = if let Some(gps) = &metadata.gps {
        crate::timezone::format_localized_time(
            gps.latitude,
            gps.longitude,
            metadata.taken_timestamp,
        )
        .unwrap_or_else(|| format_fallback_timestamp(metadata.taken_timestamp, is_video))
    } else {
        format_fallback_timestamp(metadata.taken_timestamp, is_video)
    };

    if is_video {
        args.push("-api".to_string());
        args.push("QuickTimeUTC=1".to_string());
        args.push(format!("-QuickTime:CreateDate={}", formatted_time));
        args.push(format!("-QuickTime:ModifyDate={}", formatted_time));
    } else {
        args.push(format!("-AllDates={}", formatted_time));
    }

    if let Some(gps) = &metadata.gps {
        let lat_ref = if gps.latitude >= 0.0 { "N" } else { "S" };
        let lon_ref = if gps.longitude >= 0.0 { "E" } else { "W" };

        args.push(format!("-GPSLatitude={}", gps.latitude.abs()));
        args.push(format!("-GPSLatitudeRef={}", lat_ref));
        args.push(format!("-GPSLongitude={}", gps.longitude.abs()));
        args.push(format!("-GPSLongitudeRef={}", lon_ref));

        if let Some(alt) = gps.altitude {
            let alt_ref = if alt >= 0.0 { "0" } else { "1" };
            args.push(format!("-GPSAltitude={}", alt.abs()));
            args.push(format!("-GPSAltitudeRef={}", alt_ref));
        }
    }

    if let Some(desc) = &metadata.description {
        let safe_desc = desc.replace("\r", " ").replace("\n", " ");
        args.push(format!("-ImageDescription={}", safe_desc));
    }
    if let Some(title) = &metadata.title {
        let safe_title = title.replace("\r", " ").replace("\n", " ");
        args.push(format!("-XPTitle={}", safe_title));
        args.push(format!("-Title={}", safe_title));
    }

    args.push("--".to_string());
    args.push(target_file.to_string_lossy().to_string());

    args
}

fn format_fallback_timestamp(timestamp: i64, is_video: bool) -> String {
    let dt: DateTime<Utc> = DateTime::from_timestamp(timestamp, 0).unwrap_or_default();
    if is_video {
        dt.format("%Y:%m:%d %H:%M:%SZ").to_string()
    } else {
        dt.format("%Y:%m:%d %H:%M:%S").to_string()
    }
}

impl Drop for ExifToolEngine {
    fn drop(&mut self) {
        let mut process_guard = match self.process.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        if let Some(mut process_state) = process_guard.take() {
            debug!("Shutting down ExifTool engine");
            let _ = writeln!(process_state.stdin, "-stay_open");
            let _ = writeln!(process_state.stdin, "False");
            let _ = process_state.stdin.flush();
            drop(process_state.stdin);

            let start = std::time::Instant::now();
            let mut exited = false;
            while start.elapsed() < Duration::from_secs(2) {
                if let Ok(Some(_)) = process_state.child.try_wait() {
                    exited = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }

            if !exited {
                warn!("ExifTool did not exit gracefully, killing");
                let _ = process_state.child.kill();
                let _ = process_state.child.wait();
            }
        }
    }
}

pub struct ExifToolPool {
    sender: Sender<ExifToolEngine>,
    receiver: Receiver<ExifToolEngine>,
}

impl ExifToolPool {
    pub fn new(binary_path: PathBuf, pool_size: usize) -> Result<Self, AppError> {
        info!("Creating ExifToolPool with {} engines", pool_size);
        let (sender, receiver) = unbounded();

        for _ in 0..pool_size {
            let engine = ExifToolEngine::new(binary_path.clone());
            engine.ensure_running()?;
            sender.send(engine).unwrap();
        }

        Ok(Self { sender, receiver })
    }

    pub fn execute<F, R>(&self, f: F) -> Result<R, AppError>
    where
        F: FnOnce(&ExifToolEngine) -> Result<R, AppError>,
    {
        let timeout = Duration::from_secs(120);
        let engine = self.receiver.recv_timeout(timeout).map_err(|_| {
            AppError::ExifToolLaunchFailed(
                "Timeout waiting for ExifTool engine from pool".to_string(),
            )
        })?;

        let result = f(&engine);

        let _ = self.sender.send(engine);

        result
    }

    pub fn shutdown(&self) {
        info!("Shutting down ExifToolPool");
        while let Ok(engine) = self.receiver.try_recv() {
            drop(engine);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{GpsCoordinate, ParsedMetadata};

    #[test]
    fn test_gps_refs_logic() {
        let gps = GpsCoordinate {
            latitude: -40.0,
            longitude: 74.0,
            altitude: Some(-10.0),
        };

        let lat_ref = if gps.latitude >= 0.0 { "N" } else { "S" };
        let lon_ref = if gps.longitude >= 0.0 { "E" } else { "W" };
        let alt_ref = if gps.altitude.unwrap() >= 0.0 {
            "0"
        } else {
            "1"
        };

        assert_eq!(lat_ref, "S");
        assert_eq!(lon_ref, "E");
        assert_eq!(alt_ref, "1");
    }

    #[test]
    fn test_output_parsing_updated() {
        assert_eq!(
            parse_exiftool_output("1 image files updated"),
            ExifToolResult::Updated(1)
        );
        assert_eq!(
            parse_exiftool_output("    2 image files updated\n"),
            ExifToolResult::Updated(2)
        );
        assert_eq!(
            parse_exiftool_output("1 image file updated"),
            ExifToolResult::Updated(1)
        );
    }

    #[test]
    fn test_output_parsing_unchanged() {
        assert_eq!(
            parse_exiftool_output("1 image files unchanged"),
            ExifToolResult::Unchanged
        );
        assert_eq!(
            parse_exiftool_output("0 image files updated\n1 image files unchanged"),
            ExifToolResult::Unchanged
        );
    }

    #[test]
    fn test_output_parsing_error() {
        assert_eq!(
            parse_exiftool_output("Error: File not found"),
            ExifToolResult::Failed("Error: File not found".to_string())
        );
        assert_eq!(
            parse_exiftool_output("1 files weren't updated due to errors"),
            ExifToolResult::Failed("1 files weren't updated due to errors".to_string())
        );
    }

    #[test]
    fn test_text_metadata_args() {
        let desc = "My Description";
        let title = "My Title";
        assert!(format!("-ImageDescription={}", desc).contains("My Description"));
        assert!(format!("-XPTitle={}", title).contains("My Title"));
        assert!(format!("-Title={}", title).contains("My Title"));
    }

    #[test]
    fn test_timestamp_formatting_image() {
        let meta = ParsedMetadata {
            title: None,
            description: None,
            taken_timestamp: 1672531200,
            gps: None,
        };
        let target = Path::new("photo.jpg");
        let args = build_metadata_args(target, &meta);

        assert!(args.contains(&"-AllDates=2023:01:01 00:00:00".to_string()));
        assert!(!args.iter().any(|a| a.contains("QuickTime")));
    }

    #[test]
    fn test_timestamp_formatting_video() {
        let meta = ParsedMetadata {
            title: None,
            description: None,
            taken_timestamp: 1672531200,
            gps: None,
        };
        let target = Path::new("video.mp4");
        let args = build_metadata_args(target, &meta);

        assert!(args.contains(&"-api".to_string()));
        assert!(args.contains(&"QuickTimeUTC=1".to_string()));
        assert!(args.contains(&"-QuickTime:CreateDate=2023:01:01 00:00:00Z".to_string()));
        assert!(args.contains(&"-QuickTime:ModifyDate=2023:01:01 00:00:00Z".to_string()));
    }

    #[test]
    fn test_sec_001_command_injection_sanitization() {
        let meta = ParsedMetadata {
            title: Some("Title\n-execute\n-malicious".to_string()),
            description: Some("Desc\r\n-execute".to_string()),
            taken_timestamp: 1672531200,
            gps: None,
        };
        let target = Path::new("photo.jpg");
        let args = build_metadata_args(target, &meta);

        assert!(args.contains(&"-ImageDescription=Desc  -execute".to_string()));
        assert!(args.contains(&"-Title=Title -execute -malicious".to_string()));
        for arg in args {
            assert!(!arg.contains('\n'));
            assert!(!arg.contains('\r'));
        }
    }
}
