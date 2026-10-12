use once_cell::sync::Lazy;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use tracing_subscriber::{fmt, EnvFilter};

static ACTIVE_LOG_FILE: Lazy<Arc<Mutex<Option<BufWriter<File>>>>> =
    Lazy::new(|| Arc::new(Mutex::new(None)));

/// A dual-destination writer that streams tracing events simultaneously to standard output
/// and to an active restoration log file (when configured).
#[derive(Clone, Copy, Debug, Default)]
pub struct DualWriter;

impl io::Write for DualWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = io::stdout().write(buf);
        if let Ok(mut guard) = ACTIVE_LOG_FILE.lock() {
            if let Some(ref mut writer) = *guard {
                let _ = writer.write(buf);
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let _ = io::stdout().flush();
        if let Ok(mut guard) = ACTIVE_LOG_FILE.lock() {
            if let Some(ref mut writer) = *guard {
                let _ = writer.flush();
            }
        }
        Ok(())
    }
}

/// Initializes global tracing telemetry. Supports safe re-initialization in test fixtures.
pub fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let _ = fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(|| DualWriter)
        .try_init();
}

/// Sets the active destination file for runtime restoration logs.
pub fn set_active_log_file<P: AsRef<Path>>(path: P) -> io::Result<()> {
    if let Some(parent) = path.as_ref().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    if let Ok(mut guard) = ACTIVE_LOG_FILE.lock() {
        *guard = Some(BufWriter::new(file));
    }
    Ok(())
}

/// Flushes pending bytes in the active restoration log file buffer to disk.
pub fn flush_active_log_file() {
    if let Ok(mut guard) = ACTIVE_LOG_FILE.lock() {
        if let Some(ref mut writer) = *guard {
            let _ = writer.flush();
        }
    }
}

/// Flushes and closes the active restoration log file.
pub fn close_active_log_file() {
    if let Ok(mut guard) = ACTIVE_LOG_FILE.lock() {
        if let Some(mut writer) = guard.take() {
            let _ = writer.flush();
        }
    }
}
