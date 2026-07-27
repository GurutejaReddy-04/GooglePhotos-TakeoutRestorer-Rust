use app::{run_core_pipeline, CoreDispatcher};
use app_core::config::Config;
use app_core::events::Broadcaster;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Instant;

fn generate_large_dataset(count: usize) -> PathBuf {
    let target = PathBuf::from("target/tmp/fixtures/large_dataset");
    if target.exists() {
        let _ = std::fs::remove_dir_all(&target);
    }
    std::fs::create_dir_all(&target).unwrap();

    // Create 'count' image files and 'count' json files
    for i in 0..count {
        let img_name = format!("IMG_{:05}.JPG", i);
        let json_name = format!("{}.json", img_name);

        let img_path = target.join(&img_name);
        let json_path = target.join(&json_name);

        std::fs::write(&img_path, b"dummy image data").unwrap();

        let json_content = format!(
            r#"{{
  "title": "{}",
  "description": "",
  "imageViews": "0",
  "creationTime": {{
    "timestamp": "1612137600",
    "formatted": "Feb 1, 2021, 12:00:00 AM UTC"
  }},
  "photoTakenTime": {{
    "timestamp": "1612137600",
    "formatted": "Feb 1, 2021, 12:00:00 AM UTC"
  }},
  "geoData": {{
    "latitude": 37.4220,
    "longitude": -122.0841,
    "altitude": 0.0,
    "latitudeSpan": 0.0,
    "longitudeSpan": 0.0
  }},
  "geoDataExif": {{
    "latitude": 37.4220,
    "longitude": -122.0841,
    "altitude": 0.0,
    "latitudeSpan": 0.0,
    "longitudeSpan": 0.0
  }},
  "url": "https://example.com/photo.jpg",
  "googlePhotosOrigin": {{
    "mobileUpload": {{
      "deviceFolder": {{
        "localFolderName": ""
      }},
      "deviceType": "ANDROID_PHONE"
    }}
  }}
}}"#,
            img_name
        );

        std::fs::write(&json_path, json_content).unwrap();
    }

    target
}

fn main() {
    println!("Generating 10,000 files large dataset...");
    let start_gen = Instant::now();
    let input_dir = generate_large_dataset(10000);
    println!("Generated in {:?}", start_gen.elapsed());

    let out_dir = PathBuf::from("target/tmp/fixtures/large_output");
    if out_dir.exists() {
        let _ = std::fs::remove_dir_all(&out_dir);
    }
    std::fs::create_dir_all(&out_dir).unwrap();

    let publisher = Arc::new(Broadcaster::new());
    let mut config = Config::default();
    config.processing.max_workers = 12; // Simulate high core count

    // We MUST use our fake exiftool.exe to bypass actual IO bottlenecks and network downloads
    // otherwise 10k files will take hours to process.
    let current_path = std::env::var_os("PATH").unwrap_or_default();
    let fixtures_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut new_path = std::ffi::OsString::new();
    new_path.push(&fixtures_dir);
    new_path.push(";");
    new_path.push(&current_path);
    std::env::set_var("PATH", new_path);

    let dispatcher = Arc::new(CoreDispatcher {
        cancel_token: Arc::new(AtomicBool::new(false)),
        pause_token: Arc::new(AtomicBool::new(false)),
        publisher: Arc::clone(&publisher),
        input_dirs: Arc::new(Mutex::new(vec![input_dir])),
        output_dir: Arc::new(Mutex::new(Some(out_dir))),
        db_path: Arc::new(Mutex::new(None)),
        use_system_exiftool: Arc::new(Mutex::new(true)),
        concurrency_limit: Arc::new(Mutex::new(12)),
        config: Arc::new(Mutex::new(config)),
    });

    let cancel = Arc::clone(&dispatcher.cancel_token);
    let pause = Arc::clone(&dispatcher.pause_token);
    let inputs = dispatcher.input_dirs.lock().unwrap().clone();
    let output = dispatcher.output_dir.lock().unwrap().clone().unwrap();
    let use_sys = *dispatcher.use_system_exiftool.lock().unwrap();

    let start = Instant::now();
    println!("Starting core pipeline...");

    run_core_pipeline(inputs, output, None, use_sys, cancel, pause, publisher).unwrap();

    let duration = start.elapsed();
    let throughput = 10000.0 / duration.as_secs_f64();
    println!("Finished in {:?}", duration);
    println!("Throughput: {:.2} files/sec", throughput);
}
