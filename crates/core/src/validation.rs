use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use sysinfo::Disks;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ValidationCode {
    Ok,
    OverlapsWithSource,
    DestInsideSource,
    SourceInsideDest,
    ReadOnly,
    InvalidPath,
    UnknownError,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DestinationValidationKind {
    Valid,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DestinationValidationResult {
    pub path: PathBuf,
    pub kind: DestinationValidationKind,
    pub free_bytes: u64,
    pub total_bytes: u64,
    pub code: ValidationCode,
    pub message: String,
}

pub fn validate_destination(dest: &Path, inputs: &[PathBuf]) -> DestinationValidationResult {
    let mut result = DestinationValidationResult {
        path: dest.to_path_buf(),
        kind: DestinationValidationKind::Valid,
        free_bytes: 0,
        total_bytes: 0,
        code: ValidationCode::Ok,
        message: String::new(),
    };

    // 1. Check write permissions and create dir if needed
    if !dest.exists() {
        if let Err(e) = fs::create_dir_all(dest) {
            result.kind = DestinationValidationKind::Error;
            result.code = ValidationCode::InvalidPath;
            result.message = format!("Cannot create folder: {}", e);
            return result;
        }
    }

    let test_file = dest.join(".takeout_fixer_write_test");
    if let Err(e) = fs::File::create(&test_file) {
        result.kind = DestinationValidationKind::Error;
        result.code = ValidationCode::ReadOnly;
        result.message = format!("Cannot write to this folder: {}", e);
        return result;
    }
    let _ = fs::remove_file(test_file);

    // 2. Disk Space
    let disks = Disks::new_with_refreshed_list();
    let mut best_match = None;
    let mut best_len = 0;
    let dest_str = dest.to_string_lossy().to_string();

    for disk in disks.iter() {
        let mnt = disk.mount_point().to_string_lossy().to_string();
        if dest_str.starts_with(&mnt) && mnt.len() > best_len {
            best_len = mnt.len();
            best_match = Some(disk);
        }
    }

    if let Some(disk) = best_match {
        result.free_bytes = disk.available_space();
        result.total_bytes = disk.total_space();
    }

    // 3. Overlap check
    let dest_resolved = fs::canonicalize(dest).unwrap_or_else(|_| dest.to_path_buf());

    for inp in inputs {
        let inp_resolved = fs::canonicalize(inp).unwrap_or_else(|_| inp.to_path_buf());
        if dest_resolved == inp_resolved {
            result.kind = DestinationValidationKind::Warning;
            result.code = ValidationCode::OverlapsWithSource;
            result.message =
                "Destination is the same as a source folder. In-place mode will modify originals."
                    .to_string();
            return result;
        }

        if dest_resolved.starts_with(&inp_resolved) {
            result.kind = DestinationValidationKind::Warning;
            result.code = ValidationCode::DestInsideSource;
            result.message =
                "Destination is inside a source folder. This may cause issues in copy mode."
                    .to_string();
            return result;
        }

        if inp_resolved.starts_with(&dest_resolved) {
            result.kind = DestinationValidationKind::Warning;
            result.code = ValidationCode::SourceInsideDest;
            result.message =
                "A source folder is inside the destination. Output may mix with source files."
                    .to_string();
            return result;
        }
    }

    result
}

/// Validates whether in-place mode is safe given the input files.
/// Returns a warning message if any input is a ZIP archive and the output mode is InPlace,
/// since ZIP contents cannot be modified in-place.
pub fn validate_inplace_zip(inputs: &[PathBuf]) -> Option<String> {
    let has_zip = inputs.iter().any(|p| {
        p.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("zip"))
            .unwrap_or(false)
    });

    if has_zip {
        Some(
            "ZIP archives cannot be modified in-place. \
             Files will be extracted and copied to the output directory instead."
                .to_string(),
        )
    } else {
        None
    }
}
