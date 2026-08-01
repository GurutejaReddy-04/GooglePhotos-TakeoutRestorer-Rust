//! Google Photos Takeout Restorer - Downloader Crate
//! Automatic downloading and verification of platform-specific ExifTool binaries.
//!
//! Author: Guruteja Reddy Nallachi (<https://github.com/GurutejaReddy-04>)
//! Open Source Software released under the MIT License.

use core::error::AppError;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const EXIFTOOL_VERSION: &str = "13.59";

#[cfg(target_os = "windows")]
const EXIFTOOL_FILENAME: &str = "exiftool-13.59_64.zip";
#[cfg(target_os = "windows")]
const EXIFTOOL_URL: &str =
    "https://sourceforge.net/projects/exiftool/files/exiftool-13.59_64.zip/download";
#[cfg(target_os = "windows")]
const EXIFTOOL_SHA256: &str = "44b512b25af500724ba579d0a53c8fc5851628b692dd5e5d94ae4a15c2cba9ec";
#[cfg(target_os = "windows")]
const EXIFTOOL_EXECUTABLE: &str = "exiftool(-k).exe";
#[cfg(target_os = "windows")]
const EXIFTOOL_FINAL_EXECUTABLE: &str = "exiftool.exe";

#[cfg(not(target_os = "windows"))]
const EXIFTOOL_FILENAME: &str = "Image-ExifTool-13.59.tar.gz";
#[cfg(not(target_os = "windows"))]
const EXIFTOOL_URL: &str =
    "https://sourceforge.net/projects/exiftool/files/Image-ExifTool-13.59.tar.gz/download";
#[cfg(not(target_os = "windows"))]
const EXIFTOOL_SHA256: &str = "668ea3acececb7235fbd0f4900e72d5f12c9b07e5c778fd36cb1e9b5828fd65a";
#[cfg(not(target_os = "windows"))]
#[allow(dead_code)]
const EXIFTOOL_EXECUTABLE: &str = "exiftool";
#[cfg(not(target_os = "windows"))]
const EXIFTOOL_FINAL_EXECUTABLE: &str = "exiftool";

pub struct ExifToolManager {
    install_dir: PathBuf,
}

impl Default for ExifToolManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ExifToolManager {
    pub fn new() -> Self {
        let base_dir =
            directories::ProjectDirs::from("", "TakeoutRestorerTeam", "GooglePhotosRestorer")
                .map(|dirs| dirs.data_local_dir().to_path_buf())
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        let install_dir = base_dir.join("exiftool").join(EXIFTOOL_VERSION);

        Self { install_dir }
    }

    pub fn is_installed(&self) -> bool {
        self.exiftool_path().exists()
    }

    pub fn exiftool_path(&self) -> PathBuf {
        self.install_dir.join(EXIFTOOL_FINAL_EXECUTABLE)
    }

    pub fn check_perl(&self) -> Result<(), AppError> {
        #[cfg(target_os = "windows")]
        {
            // Windows standalone executable does not require Perl.
            Ok(())
        }
        #[cfg(not(target_os = "windows"))]
        {
            let status = std::process::Command::new("perl")
                .arg("-v")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();

            match status {
                Ok(s) if s.success() => Ok(()),
                _ => Err(AppError::ExifToolNotFound),
            }
        }
    }

    pub fn ensure_installed<F>(&self, progress: F) -> Result<(), AppError>
    where
        F: Fn(u64, u64),
    {
        if self.is_installed() {
            return Ok(());
        }

        std::fs::create_dir_all(&self.install_dir).map_err(AppError::Io)?;

        let temp_dir = tempfile::tempdir().map_err(AppError::Io)?;
        let archive_path = temp_dir.path().join(EXIFTOOL_FILENAME);

        self.download_file(EXIFTOOL_URL, &archive_path, progress)?;
        self.verify_checksum(&archive_path, EXIFTOOL_SHA256)?;

        #[cfg(target_os = "windows")]
        self.extract_zip(&archive_path)?;

        #[cfg(not(target_os = "windows"))]
        self.extract_tar_gz(&archive_path)?;

        Ok(())
    }

    fn download_file<F>(&self, url: &str, dest: &Path, progress: F) -> Result<(), AppError>
    where
        F: Fn(u64, u64),
    {
        let agent = ureq::AgentBuilder::new().build();

        let response = agent
            .get(url)
            .call()
            .map_err(|e| AppError::Io(std::io::Error::other(e.to_string())))?;

        let total_size = response
            .header("Content-Length")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        let mut reader = response.into_reader();
        let mut file = File::create(dest).map_err(AppError::Io)?;
        let mut downloaded = 0;
        let mut buffer = [0; 8192];

        loop {
            let bytes_read = reader.read(&mut buffer).map_err(AppError::Io)?;
            if bytes_read == 0 {
                break;
            }
            file.write_all(&buffer[..bytes_read])
                .map_err(AppError::Io)?;
            downloaded += bytes_read as u64;
            progress(downloaded, total_size);
        }

        Ok(())
    }

    fn verify_checksum(&self, path: &Path, expected_hex: &str) -> Result<(), AppError> {
        let mut file = File::open(path).map_err(AppError::Io)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0; 8192];

        loop {
            let bytes_read = file.read(&mut buffer).map_err(AppError::Io)?;
            if bytes_read == 0 {
                break;
            }
            hasher.update(&buffer[..bytes_read]);
        }

        let hash = hasher.finalize();
        let hash_hex = format!("{:x}", hash);

        if hash_hex != expected_hex {
            return Err(AppError::ChecksumMismatch);
        }

        Ok(())
    }

    #[cfg(target_os = "windows")]
    fn extract_zip(&self, archive_path: &Path) -> Result<(), AppError> {
        let file = File::open(archive_path).map_err(AppError::Io)?;
        let mut archive = zip::ZipArchive::new(file)?;

        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;

            // Prevent zip bomb
            if file.size() > 100_000_000 {
                return Err(AppError::SecurityThreat(
                    "Downloaded zip file too large".into(),
                ));
            }

            let enclosed_name = match file.enclosed_name() {
                Some(path) => path.to_owned(),
                None => continue,
            };

            let rel_path = match enclosed_name.strip_prefix(
                enclosed_name
                    .components()
                    .next()
                    .map(|c| c.as_os_str())
                    .unwrap_or_default(),
            ) {
                Ok(p) => p.to_owned(),
                Err(_) => enclosed_name.clone(),
            };

            if rel_path.as_os_str().is_empty() {
                continue;
            }

            let outpath = self.install_dir.join(&rel_path);

            if file.is_dir() {
                std::fs::create_dir_all(&outpath).map_err(AppError::Io)?;
            } else {
                if let Some(p) = outpath.parent() {
                    std::fs::create_dir_all(p).map_err(AppError::Io)?;
                }
                let mut outfile = File::create(&outpath).map_err(AppError::Io)?;
                std::io::copy(&mut file, &mut outfile).map_err(AppError::Io)?;
            }
        }

        // Rename `exiftool(-k).exe` to `exiftool.exe`
        let original_exe = self.install_dir.join(EXIFTOOL_EXECUTABLE);
        if original_exe.exists() {
            let _ = std::fs::rename(&original_exe, self.exiftool_path());
        }

        // Fallback search for nested executable if extracted inside a subfolder
        if !self.exiftool_path().exists() {
            if let Ok(entries) = std::fs::read_dir(&self.install_dir) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        let sub_exe = entry.path().join(EXIFTOOL_EXECUTABLE);
                        let sub_final = entry.path().join(EXIFTOOL_FINAL_EXECUTABLE);
                        if sub_exe.exists() {
                            let _ = std::fs::rename(&sub_exe, self.exiftool_path());
                        } else if sub_final.exists() {
                            let _ = std::fs::rename(&sub_final, self.exiftool_path());
                        }
                    }
                }
            }
        }

        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    fn extract_tar_gz(&self, archive_path: &Path) -> Result<(), AppError> {
        let file = File::open(archive_path).map_err(AppError::Io)?;
        let tar = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(tar);

        archive.unpack(&self.install_dir).map_err(AppError::Io)?;

        let extracted_root = self
            .install_dir
            .join(format!("Image-ExifTool-{}", EXIFTOOL_VERSION));
        if extracted_root.exists() {
            if let Ok(entries) = std::fs::read_dir(&extracted_root) {
                for entry in entries.flatten() {
                    let src = entry.path();
                    let dest = self.install_dir.join(entry.file_name());
                    let _ = std::fs::rename(&src, &dest);
                }
            }
            let _ = std::fs::remove_dir_all(&extracted_root);
        }

        let exe_path = self.exiftool_path();
        if exe_path.exists() {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&exe_path)
                .map_err(AppError::Io)?
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&exe_path, perms).map_err(AppError::Io)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(target_os = "windows"))]
    use tempfile::tempdir;

    #[test]
    fn test_exiftool_manager_paths() {
        let manager = ExifToolManager::new();
        assert!(manager
            .exiftool_path()
            .to_string_lossy()
            .contains("exiftool"));
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn test_p0_001_extract_tar_gz_preserves_lib_directory() {
        let dir = tempdir().unwrap();
        let manager = ExifToolManager {
            install_dir: dir.path().to_path_buf(),
        };

        // Create a mock tar.gz archive representing Image-ExifTool-13.59
        let archive_path = dir.path().join("test.tar.gz");
        let tar_file = File::create(&archive_path).unwrap();
        let gz = flate2::write::GzEncoder::new(tar_file, flate2::Compression::default());
        let mut builder = tar::Builder::new(gz);

        let root = format!("Image-ExifTool-{}", EXIFTOOL_VERSION);

        // Add exiftool binary
        let mut header = tar::Header::new_gnu();
        header.set_size(10);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(
                &mut header,
                format!("{}/exiftool", root),
                &b"#!/bin/sh\n"[..],
            )
            .unwrap();

        // Add lib/Image/ExifTool.pm
        let mut header_lib = tar::Header::new_gnu();
        header_lib.set_size(12);
        header_lib.set_mode(0o644);
        header_lib.set_cksum();
        builder
            .append_data(
                &mut header_lib,
                format!("{}/lib/Image/ExifTool.pm", root),
                &b"package PM;\n"[..],
            )
            .unwrap();

        builder.finish().unwrap();

        // Perform extraction
        manager.extract_tar_gz(&archive_path).unwrap();

        // Verify that both exiftool AND lib/ exist at root of install_dir
        assert!(
            dir.path().join("exiftool").exists(),
            "exiftool executable must exist at root of install_dir"
        );
        assert!(
            dir.path()
                .join("lib")
                .join("Image")
                .join("ExifTool.pm")
                .exists(),
            "lib/ directory must be moved to root of install_dir alongside exiftool"
        );
    }
}
