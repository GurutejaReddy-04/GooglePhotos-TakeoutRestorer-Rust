//! Google Photos Takeout Restorer - GUI Crate
//! Slint-based graphical user interface for cross-platform desktop execution.
//!
//! Author: Guruteja Reddy Nallachi (<https://github.com/GurutejaReddy-04>)
//! Open Source Software released under the MIT License.

slint::include_modules!();

pub mod bindings;
pub mod window_state;

use std::sync::Arc;
use std::thread;

use rfd::FileDialog;
use shared_ui::{CommandDispatcher, ProcessingSnapshot as CoreProcessingSnapshot, UiCommand};
use slint::{ComponentHandle, Model, Weak};

use crate::bindings::update_ui_from_snapshot;
use crate::window_state::WindowState;

pub struct GuiRunner {
    dispatcher: Arc<dyn CommandDispatcher + Send + Sync>,
    snapshot_rx: shared_ui::watch::Receiver<CoreProcessingSnapshot>,
    startup_theme: String,
}

impl GuiRunner {
    pub fn new(
        dispatcher: Arc<dyn CommandDispatcher + Send + Sync>,
        snapshot_rx: shared_ui::watch::Receiver<CoreProcessingSnapshot>,
        startup_theme: String,
    ) -> Self {
        Self {
            dispatcher,
            snapshot_rx,
            startup_theme,
        }
    }

    pub fn run(&self) -> Result<(), slint::PlatformError> {
        let ui = MainWindow::new()?;
        ui.global::<Theme>()
            .set_preference(normalize_theme_preference(&self.startup_theme).into());
        let ui_handle = ui.as_weak();

        let state = WindowState::load();
        ui.window().set_size(slint::LogicalSize::new(
            state.width.max(800.0),
            state.height.max(600.0),
        ));
        if state.x >= 0.0 && state.y >= 0.0 && state.x < 3000.0 && state.y < 2000.0 {
            ui.window()
                .set_position(slint::LogicalPosition::new(state.x, state.y));
        }
        if state.is_maximized {
            ui.window().set_maximized(true);
        }

        self.setup_callbacks(&ui, state);
        self.spawn_snapshot_listener(&ui_handle);

        ui.run()
    }

    fn setup_callbacks(&self, ui: &MainWindow, _state: WindowState) {
        let ui_handle_next = ui.as_weak();
        ui.on_go_next(move || {
            if let Some(ui) = ui_handle_next.upgrade() {
                ui.set_action_feedback("".into());
                let mut step = ui.get_current_step();
                if step < 5 {
                    step += 1;
                    ui.set_current_step(step);
                }
            }
        });

        let ui_handle_back = ui.as_weak();
        ui.on_go_back(move || {
            if let Some(ui) = ui_handle_back.upgrade() {
                if !ui.get_processing().is_processing {
                    ui.set_action_feedback("".into());
                    let mut step = ui.get_current_step();
                    if step > 0 {
                        step -= 1;
                        ui.set_current_step(step);
                    }
                }
            }
        });

        let dispatcher = Arc::clone(&self.dispatcher);
        ui.on_shutdown_requested(move || {
            let _ = dispatcher.dispatch(UiCommand::Shutdown);
            let _ = slint::quit_event_loop();
        });

        let dispatcher_theme = self.dispatcher.clone();
        let ui_handle_theme = ui.as_weak();
        ui.on_change_theme(move |theme_str| {
            let theme_val = normalize_theme_preference(&theme_str).to_string();
            let result = dispatcher_theme.dispatch(UiCommand::UpdateSetting {
                key: "theme".to_string(),
                value: theme_val.clone(),
            });
            if let Some(ui) = ui_handle_theme.upgrade() {
                match result {
                    Ok(()) => ui.global::<Theme>().set_preference(theme_val.into()),
                    Err(error) => {
                        ui.set_action_feedback_is_error(true);
                        ui.set_action_feedback(
                            format!("Could not update appearance preference: {error}").into(),
                        );
                    }
                }
            }
        });

        let ui_handle_zip = ui.as_weak();
        let dispatcher_inputs = self.dispatcher.clone();
        ui.on_select_zip_files(move || {
            if let Some(ui) = ui_handle_zip.upgrade() {
                ui.set_import_error("".into());
                if let Some(files) = FileDialog::new().add_filter("ZIP", &["zip"]).pick_files() {
                    let current = ui.get_inputs().iter().collect::<Vec<_>>();
                    let mut input_paths = current.iter().map(|s| s.to_string()).collect::<Vec<_>>();
                    let mut skipped = Vec::new();

                    for file in files {
                        let path_str = file.to_string_lossy().to_string();
                        let filename = file
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();

                        if input_paths.iter().any(|s| s == &path_str) {
                            skipped.push(format!("Duplicate: {}", filename));
                            continue;
                        }

                        match std::fs::metadata(&file) {
                            Ok(meta) => {
                                if !meta.is_file()
                                    || file
                                        .extension()
                                        .is_none_or(|ext| !ext.eq_ignore_ascii_case("zip"))
                                {
                                    skipped.push(format!("Invalid ZIP: {}", filename));
                                } else {
                                    input_paths.push(path_str);
                                }
                            }
                            Err(_) => {
                                skipped.push(format!("Unreadable: {}", filename));
                            }
                        }
                    }

                    if !skipped.is_empty() {
                        ui.set_import_error(slint::SharedString::from(format!(
                            "Skipped {} invalid/duplicate items:\n{}",
                            skipped.len(),
                            skipped.join(", ")
                        )));
                    }

                    let model = slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(
                        input_paths.iter().map(|s| s.into()).collect::<Vec<_>>(),
                    )));
                    ui.set_inputs(model);
                    let _ = dispatcher_inputs.dispatch(UiCommand::SetInputPaths(input_paths));
                }
            }
        });

        let ui_handle_folder = ui.as_weak();
        let dispatcher_inputs_folder = self.dispatcher.clone();
        ui.on_select_folders(move || {
            if let Some(ui) = ui_handle_folder.upgrade() {
                ui.set_import_error("".into());
                let folders = FileDialog::new()
                    .pick_folders()
                    .or_else(|| FileDialog::new().pick_folder().map(|f| vec![f]));
                if let Some(folders) = folders {
                    let current = ui.get_inputs().iter().collect::<Vec<_>>();
                    let mut input_paths = current.iter().map(|s| s.to_string()).collect::<Vec<_>>();
                    let mut skipped = Vec::new();

                    for folder in folders {
                        let path_str = folder.to_string_lossy().to_string();
                        let folder_name = folder
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();

                        if input_paths.iter().any(|s| s == &path_str) {
                            skipped.push(format!("Duplicate: {}", folder_name));
                            continue;
                        }

                        match std::fs::metadata(&folder) {
                            Ok(meta) => {
                                if !meta.is_dir() {
                                    skipped.push(format!("Not a folder: {}", folder_name));
                                } else {
                                    input_paths.push(path_str);
                                }
                            }
                            Err(_) => {
                                skipped.push(format!("Unreadable: {}", folder_name));
                            }
                        }
                    }

                    if !skipped.is_empty() {
                        ui.set_import_error(slint::SharedString::from(format!(
                            "Skipped {} invalid/duplicate items:\n{}",
                            skipped.len(),
                            skipped.join(", ")
                        )));
                    }

                    let model = slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(
                        input_paths.iter().map(|s| s.into()).collect::<Vec<_>>(),
                    )));
                    ui.set_inputs(model);
                    let _ =
                        dispatcher_inputs_folder.dispatch(UiCommand::SetInputPaths(input_paths));
                }
            }
        });

        let dispatcher_remove = self.dispatcher.clone();
        let ui_handle_remove = ui.as_weak();
        ui.on_remove_input(move |path| {
            if let Some(ui) = ui_handle_remove.upgrade() {
                let current = ui.get_inputs().iter().collect::<Vec<_>>();
                let mut input_paths = current.iter().map(|s| s.to_string()).collect::<Vec<_>>();
                input_paths.retain(|s| s != &path.to_string());
                let model = slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(
                    input_paths.iter().map(|s| s.into()).collect::<Vec<_>>(),
                )));
                ui.set_inputs(model);
                let _ = dispatcher_remove.dispatch(UiCommand::SetInputPaths(input_paths));
            }
        });

        let dispatcher_clear = self.dispatcher.clone();
        let ui_handle_clear = ui.as_weak();
        ui.on_clear_inputs(move || {
            if let Some(ui) = ui_handle_clear.upgrade() {
                let input_paths = Vec::new();
                let model = slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(
                    input_paths
                        .iter()
                        .map(|s: &String| s.into())
                        .collect::<Vec<_>>(),
                )));
                ui.set_inputs(model);
                let _ = dispatcher_clear.dispatch(UiCommand::SetInputPaths(input_paths));
            }
        });

        // Welcome Page Actions (Sprint 2)
        let ui_handle_resume = ui.as_weak();
        let dispatcher_resume = self.dispatcher.clone();
        ui.on_resume_run_clicked(move |id| {
            if let Some(ui) = ui_handle_resume.upgrade() {
                match dispatcher_resume.dispatch(UiCommand::ResumeRun(id.to_string())) {
                    Ok(()) => {
                        ui.set_action_feedback_is_error(false);
                        ui.set_action_feedback("Resuming saved session…".into());
                        ui.set_active_tab(1);
                    }
                    Err(error) => {
                        ui.set_action_feedback_is_error(true);
                        ui.set_action_feedback(
                            format!("Could not resume saved session: {error}").into(),
                        );
                    }
                }
            }
        });

        let ui_handle_delete = ui.as_weak();
        let dispatcher_delete = self.dispatcher.clone();
        ui.on_delete_run_confirmed(move |id| {
            if let Some(ui) = ui_handle_delete.upgrade() {
                match dispatcher_delete.dispatch(UiCommand::DeleteRun(id.to_string())) {
                    Ok(()) => {
                        ui.set_action_feedback_is_error(false);
                        ui.set_action_feedback("Saved session deleted.".into());
                    }
                    Err(error) => {
                        ui.set_action_feedback_is_error(true);
                        ui.set_action_feedback(
                            format!("Could not delete saved session: {error}").into(),
                        );
                    }
                }
            }
        });

        let ui_handle_clear_all = ui.as_weak();
        let dispatcher_clear_all = self.dispatcher.clone();
        ui.on_clear_all_runs_confirmed(move || {
            if let Some(ui) = ui_handle_clear_all.upgrade() {
                match dispatcher_clear_all.dispatch(UiCommand::ClearAllRuns) {
                    Ok(()) => {
                        ui.set_action_feedback_is_error(false);
                        ui.set_action_feedback("All restore history cleared.".into());
                    }
                    Err(error) => {
                        ui.set_action_feedback_is_error(true);
                        ui.set_action_feedback(
                            format!("Could not clear restore history: {error}").into(),
                        );
                    }
                }
            }
        });

        let ui_handle_recover = ui.as_weak();
        let dispatcher_recover = self.dispatcher.clone();
        ui.on_recover_run_clicked(move |id| {
            if let Some(ui) = ui_handle_recover.upgrade() {
                match dispatcher_recover.dispatch(UiCommand::RecoverRun(id.to_string())) {
                    Ok(()) => {
                        ui.set_action_feedback_is_error(false);
                        ui.set_action_feedback(
                            "Recovery data applied. Resume the session when you are ready.".into(),
                        );
                    }
                    Err(error) => {
                        ui.set_action_feedback_is_error(true);
                        ui.set_action_feedback(
                            format!("Could not recover saved session: {error}").into(),
                        );
                    }
                }
            }
        });

        let dispatcher_dest = self.dispatcher.clone();
        ui.on_select_destination(move || {
            if let Some(folder) = FileDialog::new().pick_folder() {
                let _ = dispatcher_dest.dispatch(UiCommand::SelectOutputDirectory(
                    folder.to_string_lossy().to_string(),
                ));
            }
        });

        let ui_handle_setting = ui.as_weak();
        let dispatcher_setting = self.dispatcher.clone();
        ui.on_update_setting(move |k, v| {
            if let Some(ui) = ui_handle_setting.upgrade() {
                let mut settings = ui.get_settings();
                match k.as_str() {
                    "gps_enabled" => settings.gps_enabled = v == "true",
                    "timezone_enabled" => settings.timezone_enabled = v == "true",
                    "unmatched_enabled" => settings.unmatched_enabled = v == "true",
                    "high_performance" => settings.high_performance = v == "true",
                    "anonymous_logging" => settings.anonymous_logging = v == "true",
                    "output_mode" => settings.output_mode = v.clone(),
                    _ => {}
                }
                ui.set_settings(settings);
            }
            let _ = dispatcher_setting.dispatch(UiCommand::UpdateSetting {
                key: k.to_string(),
                value: v.to_string(),
            });
        });

        let dispatcher_start = self.dispatcher.clone();
        let ui_handle_start = ui.as_weak();
        ui.on_start_processing(move || {
            let _ = dispatcher_start.dispatch(UiCommand::StartProcessing);
            if let Some(ui) = ui_handle_start.upgrade() {
                ui.set_active_tab(1);
            }
        });

        let dispatcher_stop = self.dispatcher.clone();
        ui.on_stop_processing(move || {
            let _ = dispatcher_stop.dispatch(UiCommand::CancelProcessing);
        });

        let dispatcher_again = self.dispatcher.clone();
        let ui_handle_again = ui.as_weak();
        ui.on_export_again(move || {
            let _ = dispatcher_again.dispatch(UiCommand::ResetState);
            if let Some(ui) = ui_handle_again.upgrade() {
                ui.set_inputs(slint::ModelRc::from(std::rc::Rc::new(
                    slint::VecModel::from(Vec::<slint::SharedString>::new()),
                )));
                ui.set_active_tab(0);
            }
        });

        let ui_handle_open = ui.as_weak();
        ui.on_open_output_folder(move || {
            if let Some(ui) = ui_handle_open.upgrade() {
                let dest = ui.get_settings().destination_path;
                let dest_str = dest.to_string();

                if dest_str.is_empty() {
                    ui.set_action_feedback_is_error(true);
                    ui.set_action_feedback("No output folder is available for this run.".into());
                    return;
                }

                #[cfg(target_os = "windows")]
                let open_result = std::process::Command::new("explorer")
                    .arg(dest_str.replace('/', "\\"))
                    .spawn();
                #[cfg(target_os = "macos")]
                let open_result = std::process::Command::new("open").arg(&dest_str).spawn();
                #[cfg(target_os = "linux")]
                let open_result = std::process::Command::new("xdg-open")
                    .arg(&dest_str)
                    .spawn();
                #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
                let open_result = Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "Unsupported operating system",
                ));

                match open_result {
                    Ok(_) => {
                        ui.set_action_feedback_is_error(false);
                        ui.set_action_feedback("Opened output folder.".into());
                    }
                    Err(error) => {
                        ui.set_action_feedback_is_error(true);
                        ui.set_action_feedback(
                            format!("Could not open output folder: {error}").into(),
                        );
                    }
                }
            }
        });

        let ui_handle_copy = ui.as_weak();
        ui.on_copy_log_path(move || {
            if let Some(ui) = ui_handle_copy.upgrade() {
                let log_dir = directories::ProjectDirs::from(
                    "",
                    "TakeoutRestorerTeam",
                    "GooglePhotosRestorer",
                )
                .map(|dirs| dirs.data_dir().join("logs").join("app.log"))
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default().join("app.log"));

                let log_path_str = log_dir.to_string_lossy().to_string();
                match arboard::Clipboard::new()
                    .and_then(|mut clipboard| clipboard.set_text(log_path_str))
                {
                    Ok(()) => {
                        ui.set_action_feedback_is_error(false);
                        ui.set_action_feedback("Log path copied to the clipboard.".into());
                    }
                    Err(error) => {
                        ui.set_action_feedback_is_error(true);
                        ui.set_action_feedback(
                            format!("Could not copy the log path: {error}").into(),
                        );
                    }
                }
            }
        });

        let dispatcher_filter = self.dispatcher.clone();
        ui.on_update_results_filter(move |search, filter| {
            let _ = dispatcher_filter.dispatch(UiCommand::UpdateResultsFilter {
                search: search.to_string(),
                status_filter: filter.to_string(),
            });
        });

        let dispatcher_close = self.dispatcher.clone();
        let ui_handle_close = ui.as_weak();
        ui.window()
            .on_close_requested(move || -> slint::CloseRequestResponse {
                if let Some(ui) = ui_handle_close.upgrade() {
                    let size = ui.window().size();
                    let pos = ui.window().position();
                    let width = size.width as f32 / ui.window().scale_factor();
                    let height = size.height as f32 / ui.window().scale_factor();

                    let mut state = WindowState::load();
                    state.width = width;
                    state.height = height;
                    state.x = pos.x as f32 / ui.window().scale_factor();
                    state.y = pos.y as f32 / ui.window().scale_factor();
                    state.is_maximized = ui.window().is_maximized();
                    state.save();

                    let _ = dispatcher_close.dispatch(UiCommand::Shutdown);
                }
                slint::CloseRequestResponse::default()
            });
    }

    fn spawn_snapshot_listener(&self, ui_handle: &Weak<MainWindow>) {
        let snapshot_rx = self.snapshot_rx.clone();
        let ui_handle_clone = ui_handle.clone();

        thread::spawn(move || loop {
            let snapshot = snapshot_rx.wait_changed();
            update_ui_from_snapshot(&ui_handle_clone, &snapshot);
        });
    }
}

pub fn normalize_theme_preference(theme: &str) -> &'static str {
    match theme {
        "Light" => "Light",
        "Dark" => "Dark",
        _ => "System",
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_theme_preference;

    #[test]
    fn normalizes_theme_preferences_to_the_supported_set() {
        assert_eq!(normalize_theme_preference("Light"), "Light");
        assert_eq!(normalize_theme_preference("Dark"), "Dark");
        assert_eq!(normalize_theme_preference("System"), "System");
        assert_eq!(normalize_theme_preference("unexpected"), "System");
    }
}
