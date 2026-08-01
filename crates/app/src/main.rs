use app_core::config::Config;
use app_core::events::{AppEvent, Broadcaster, EventPublisher};
use app_core::logger::init_logging;
use clap::Parser;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use shared_ui::{CommandDispatcher, SnapshotPolicy, UiCommand, ViewModelUpdater};

#[cfg(feature = "gui")]
use gui::GuiRunner;

#[derive(Parser, Debug)]
#[command(author, version, about = "Google Photos Takeout Restorer", long_about = None)]
struct Cli {
    #[arg(long, help = "Launch the graphical user interface")]
    gui: bool,

    #[arg(required = false)]
    inputs: Vec<PathBuf>,

    #[arg(short, long, required = false)]
    output: Option<PathBuf>,

    #[arg(long)]
    db_path: Option<PathBuf>,

    #[arg(long)]
    use_system_exiftool: bool,
}

use app::CoreDispatcher;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();
    let cli = Cli::parse();

    let publisher = Arc::new(Broadcaster::new());
    let event_rx = publisher.subscribe();

    let snapshot_rx = ViewModelUpdater::spawn(
        event_rx,
        SnapshotPolicy::Debounced(std::time::Duration::from_millis(1000)),
    );

    // Pre-load recent runs for Welcome page
    let config_dir =
        directories::ProjectDirs::from("", "TakeoutRestorerTeam", "GooglePhotosRestorer")
            .map(|dirs| dirs.config_dir().to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let recent_runs = app_core::state_db::get_recent_runs(&config_dir);
    publisher.publish(AppEvent::RecentRunsLoaded(recent_runs));

    let config = Config::load().unwrap_or_default();
    publisher.publish(AppEvent::ConfigChanged(config.clone()));

    // Apply CLI overrides here if needed, omitted for brevity

    let cancel_token = Arc::new(AtomicBool::new(false));
    let pause_token = Arc::new(AtomicBool::new(false));

    let dispatcher = Arc::new(CoreDispatcher {
        cancel_token: Arc::clone(&cancel_token),
        pause_token: Arc::clone(&pause_token),
        publisher: Arc::clone(&publisher),
        input_dirs: Arc::new(Mutex::new(cli.inputs.clone())),
        output_dir: Arc::new(Mutex::new(cli.output.clone())),
        db_path: Arc::new(Mutex::new(cli.db_path.clone())),
        use_system_exiftool: Arc::new(Mutex::new(cli.use_system_exiftool)),
        concurrency_limit: Arc::new(Mutex::new(config.processing.max_workers)),
        config: Arc::new(Mutex::new(config.clone())),
    });

    #[cfg(feature = "gui")]
    if cli.gui || (cli.inputs.is_empty() && cli.output.is_none()) {
        // Let Slint use the default hardware-accelerated backend (winit with OpenGL) to restore native OS drag-and-drop.
        let runner = GuiRunner::new(dispatcher, snapshot_rx, config.ui.theme.clone());
        let res = runner.run();

        // Ensure all ExifTool zombie processes are forcefully killed on exit
        app_core::exiftool::cleanup_all_processes();

        res?;
        return Ok(());
    }

    // CLI Mode
    if cli.inputs.is_empty() || cli.output.is_none() {
        eprintln!("Error: CLI mode requires input and output arguments. Or use --gui.");
        std::process::exit(1);
    }

    let dispatcher_ctrlc = dispatcher.clone();
    ctrlc::set_handler(move || {
        let _ = dispatcher_ctrlc.dispatch(UiCommand::CancelProcessing);
    })
    .map_err(|e| app_core::error::AppError::Io(std::io::Error::other(e.to_string())))?;

    // In CLI mode, just start processing
    dispatcher.dispatch(UiCommand::StartProcessing)?;

    println!("--- Google Photos Takeout Restorer ---");
    loop {
        let snapshot = snapshot_rx.wait_changed();

        println!(
            "[{}] Progress: {} | ETA: {} | Speed: {} | Phase: {}",
            snapshot.generation_timestamp,
            snapshot.formatted_progress,
            snapshot.eta_text,
            snapshot.speed_text,
            snapshot.current_phase_text
        );

        if snapshot.is_finished {
            break;
        }
    }

    println!("Restoration pipeline shut down cleanly.");
    Ok(())
}
