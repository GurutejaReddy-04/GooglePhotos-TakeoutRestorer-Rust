use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Inspects leading magic bytes to identify true container and image formats.
pub fn detect_magic_bytes(path: &Path) -> Option<&'static str> {
    let mut file = File::open(path).ok()?;
    let mut buffer = [0u8; 12];

    let bytes_read = file.read(&mut buffer).ok()?;
    if bytes_read < 4 {
        return None;
    }

    let slice = &buffer[..bytes_read];

    if slice.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(".jpg");
    }
    if slice.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some(".png");
    }
    if slice.starts_with(b"GIF87a") || slice.starts_with(b"GIF89a") {
        return Some(".gif");
    }
    if slice.starts_with(&[0x49, 0x49, 0x2A, 0x00])
        || slice.starts_with(&[0x4D, 0x4D, 0x00, 0x2A])
    {
        return Some(".tiff");
    }
    if bytes_read >= 12 && &slice[0..4] == b"RIFF" && &slice[8..12] == b"WEBP" {
        return Some(".webp");
    }
    if bytes_read >= 12 && &slice[4..8] == b"ftyp" {
        let brand = &slice[8..12];
        match brand {
            b"mp41" | b"mp42" | b"isom" | b"iso2" => return Some(".mp4"),
            b"qt  " => return Some(".mov"),
            b"mif1" | b"msf1" | b"heic" | b"heix" => return Some(".heic"),
            _ => return None,
        }
    }

    None
}

/// Returns a corrected file extension if the file's binary signature contradicts its filename extension.
pub fn get_correction(path: &Path, expected_ext: &str) -> Option<&'static str> {
    let true_ext = detect_magic_bytes(path)?;
    let expected_lower = expected_ext.to_lowercase();
    let expected_ext_normalized = if expected_lower.starts_with('.') {
        expected_lower
    } else {
        format!(".{}", expected_lower)
    };

    let is_jpeg_synonym = (true_ext == ".jpg")
        && (expected_ext_normalized == ".jpeg" || expected_ext_normalized == ".jpg");
    let is_tiff_synonym = (true_ext == ".tiff")
        && (expected_ext_normalized == ".tif" || expected_ext_normalized == ".tiff");

    if is_jpeg_synonym || is_tiff_synonym || true_ext == expected_ext_normalized {
        None
    } else {
        Some(true_ext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_temp_file(bytes: &[u8]) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(bytes).unwrap();
        file
    }

    #[test]
    fn test_detect_jpeg() {
        let file = write_temp_file(&[0xFF, 0xD8, 0xFF, 0xDB, 0x00]);
        assert_eq!(detect_magic_bytes(file.path()), Some(".jpg"));
    }

    #[test]
    fn test_detect_png() {
        let file = write_temp_file(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00]);
        assert_eq!(detect_magic_bytes(file.path()), Some(".png"));
    }

    #[test]
    fn test_detect_heic() {
        let file = write_temp_file(&[
            0x00, 0x00, 0x00, 0x18, b'f', b't', b'y', b'p', b'h', b'e', b'i', b'c',
        ]);
        assert_eq!(detect_magic_bytes(file.path()), Some(".heic"));
    }

    #[test]
    fn test_get_correction() {
        let file = write_temp_file(&[0xFF, 0xD8, 0xFF, 0xDB]);
        // JPEG correctly named .jpg -> No correction
        assert_eq!(get_correction(file.path(), ".jpg"), None);
        // JPEG correctly named .jpeg -> No correction
        assert_eq!(get_correction(file.path(), ".jpeg"), None);
        // JPEG incorrectly named .png -> Needs correction
        assert_eq!(get_correction(file.path(), ".png"), Some(".jpg"));

        let empty_file = write_temp_file(&[]);
        assert_eq!(get_correction(empty_file.path(), ".jpg"), None);
    }
}
