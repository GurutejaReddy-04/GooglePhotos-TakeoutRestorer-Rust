use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn ensure_small_dataset() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/small_dataset");
    if path.exists() {
        return path;
    }
    fs::create_dir_all(&path).unwrap();

    // 100 images
    for i in 1..=100 {
        let name = format!("IMG_{:04}.JPG", i);
        let file_path = path.join(&name);
        fs::write(&file_path, b"fake_jpeg_content").unwrap();

        let json_name = format!("{}.json", name);
        let json_path = path.join(&json_name);
        let json = format!(
            r#"{{
            "title": "{}",
            "description": "",
            "imageViews": "0",
            "creationTime": {{
                "timestamp": "1620000000",
                "formatted": "May 3, 2021, 12:00:00 AM UTC"
            }},
            "photoTakenTime": {{
                "timestamp": "1620000000",
                "formatted": "May 3, 2021, 12:00:00 AM UTC"
            }},
            "geoData": {{
                "latitude": 0.0,
                "longitude": 0.0,
                "altitude": 0.0,
                "latitudeSpan": 0.0,
                "longitudeSpan": 0.0
            }},
            "geoDataExif": {{
                "latitude": 0.0,
                "longitude": 0.0,
                "altitude": 0.0,
                "latitudeSpan": 0.0,
                "longitudeSpan": 0.0
            }}
        }}"#,
            name
        );
        fs::write(&json_path, json).unwrap();
    }

    // 20 videos
    for i in 1..=20 {
        let name = format!("VID_{:04}.MP4", i);
        let file_path = path.join(&name);
        fs::write(&file_path, b"fake_mp4_content").unwrap();

        let json_name = format!("{}.json", name);
        let json_path = path.join(&json_name);
        let json = format!(
            r#"{{
            "title": "{}",
            "creationTime": {{ "timestamp": "1620000000" }},
            "photoTakenTime": {{ "timestamp": "1620000000" }}
        }}"#,
            name
        );
        fs::write(&json_path, json).unwrap();
    }

    path
}

pub fn ensure_medium_dataset() -> PathBuf {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/medium_dataset");
    if path.exists() {
        return path;
    }
    fs::create_dir_all(&path).unwrap();

    for i in 1..=2000 {
        let name = format!("IMG_{:04}.JPG", i);
        let file_path = path.join(&name);
        fs::write(&file_path, b"fake_jpeg_content").unwrap();

        let json_name = format!("{}.json", name);
        let json_path = path.join(&json_name);
        let json = format!(
            r#"{{
            "title": "{}",
            "photoTakenTime": {{ "timestamp": "1620000000" }}
        }}"#,
            name
        );
        fs::write(&json_path, json).unwrap();
    }

    path
}

pub fn ensure_edge_case_dataset() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/edge_cases");
    if path.exists() {
        return path;
    }
    fs::create_dir_all(&path).unwrap();

    // 1. Corrupted JSON
    fs::write(path.join("corrupt.JPG"), b"img").unwrap();
    fs::write(path.join("corrupt.JPG.json"), b"{{ invalid_json").unwrap();

    // 2. Missing JSON
    fs::write(path.join("missing_meta.JPG"), b"img").unwrap();

    // 3. Duplicate filenames (in subfolders)
    let sub1 = path.join("folder1");
    let sub2 = path.join("folder2");
    fs::create_dir_all(&sub1).unwrap();
    fs::create_dir_all(&sub2).unwrap();

    fs::write(sub1.join("duplicate.JPG"), b"img").unwrap();
    fs::write(
        sub1.join("duplicate.JPG.json"),
        r#"{"title": "duplicate.JPG", "photoTakenTime": {"timestamp": "1620000000"}}"#,
    )
    .unwrap();

    fs::write(sub2.join("duplicate.JPG"), b"img2").unwrap();
    fs::write(
        sub2.join("duplicate.JPG.json"),
        r#"{"title": "duplicate.JPG", "photoTakenTime": {"timestamp": "1620000000"}}"#,
    )
    .unwrap();

    // 4. Non-English filenames
    fs::write(path.join("写真.JPG"), b"img").unwrap();
    fs::write(
        path.join("写真.JPG.json"),
        r#"{"title": "写真.JPG", "photoTakenTime": {"timestamp": "1620000000"}}"#,
    )
    .unwrap();

    path
}
