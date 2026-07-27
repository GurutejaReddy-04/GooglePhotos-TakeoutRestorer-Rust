use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Instant;
use sysinfo::System;

fn main() {
    println!("--- EXIFTOOL EXECUTION STRATEGY BENCHMARK ---");
    println!("Recording hardware capabilities...");

    let mut sys = System::new_all();
    sys.refresh_all();

    let os_name = System::name().unwrap_or_else(|| "Unknown".to_owned());
    let os_ver = System::os_version().unwrap_or_else(|| "Unknown".to_owned());
    let cpu_brand = sys.cpus().first().map(|c| c.brand()).unwrap_or("Unknown");

    println!("OS: {} {}", os_name, os_ver);
    println!("CPU: {}", cpu_brand);
    println!("Memory: {} MB", sys.total_memory() / 1024 / 1024);
    println!("Storage: NVMe/SSD expected (not measurable from safe code directly)");

    let rustc_ver = Command::new("rustc")
        .arg("-V")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let exiftool_ver = Command::new("exiftool")
        .arg("-ver")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    println!("Rust Version: {}", rustc_ver);
    println!("ExifTool Version: {}", exiftool_ver);
    println!("----------------------------------------------\n");

    if exiftool_ver.is_empty() {
        println!("ExifTool not found in PATH. Exiting benchmark.");
        return;
    }

    let workloads = [100, 1000, 10000];

    for &count in &workloads {
        println!(">>> Workload: {} files (Mixed Media)", count);

        // 1. Spawning Approach
        let spawn_start = Instant::now();
        let mut spawn_sys = System::new();
        let mut max_mem = 0;
        let mut process_launches = 0;

        for i in 0..count {
            // Mock ExifTool launch for "-ver" just to simulate process overhead and CPU load
            let mut cmd = Command::new("exiftool");
            cmd.arg("-ver").stdout(Stdio::null()).stderr(Stdio::null());

            if let Ok(mut child) = cmd.spawn() {
                process_launches += 1;
                let _ = child.wait();
            }

            if i % 50 == 0 {
                spawn_sys.refresh_memory();
                max_mem = max_mem.max(spawn_sys.used_memory());
            }
        }

        let spawn_elapsed = spawn_start.elapsed();
        println!(
            "    [SPAWN] Elapsed: {:.2?} | Peak Mem: {} MB | Launches: {}",
            spawn_elapsed,
            max_mem / 1024 / 1024,
            process_launches
        );

        // 2. Persistent (-stay_open) Approach
        let stay_open_start = Instant::now();
        let mut stay_sys = System::new();
        let mut stay_max_mem = 0;
        let mut stay_launches = 0;

        if let Ok(mut child) = Command::new("exiftool")
            .arg("-stay_open")
            .arg("True")
            .arg("-@")
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            stay_launches += 1;
            if let Some(mut stdin) = child.stdin.take() {
                for i in 0..count {
                    // Send argument batch to ExifTool stdin
                    let _ = stdin.write_all(b"-ver\n-execute\n");

                    if i % 50 == 0 {
                        stay_sys.refresh_memory();
                        stay_max_mem = stay_max_mem.max(stay_sys.used_memory());
                    }
                }
                let _ = stdin.write_all(b"-stay_open\nFalse\n");
            }
            let _ = child.wait();
        }

        let stay_elapsed = stay_open_start.elapsed();
        println!(
            "    [STAY_OPEN] Elapsed: {:.2?} | Peak Mem: {} MB | Launches: {}",
            stay_elapsed,
            stay_max_mem / 1024 / 1024,
            stay_launches
        );
        println!();
    }

    println!("Benchmark Complete.");
    println!("Recommendation: Only migrate to -stay_open if STABLE elapsed time is >15% faster for the target media size.");
}
