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
}

impl GuiRunner {
    pub fn new(
        dispatcher: Arc<dyn CommandDispatcher + Send + Sync>,
        snapshot_rx: shared_ui::watch::Receiver<CoreProcessingSnapshot>,
    ) -> Self {
        Self {
            dispatcher,
            snapshot_rx,
        }
    }

    pub fn run(&self) -> Result<(), slint::PlatformError> {
        let ui = MainWindow::new()?;
        let ui_handle = ui.as_weak();

        let state = WindowState::load();
        ui.window()
            .set_size(slint::LogicalSize::new(state.width, state.height));
        ui.window().set_position(
            slint::LogicalPosition::new(state.x, state.y).to_physical(ui.window().scale_factor()),
        );
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
        });

        let dispatcher_theme = self.dispatcher.clone();
        let ui_handle_theme = ui.as_weak();
        ui.on_change_theme(move |theme_str| {
            let theme_val = theme_str.to_string();
            let _ = dispatcher_theme.dispatch(UiCommand::UpdateSetting {
                key: "theme".to_string(),
                value: theme_val.clone(),
            });
            if let Some(ui) = ui_handle_theme.upgrade() {
                let is_dark = match theme_val.as_str() {
                    "Light" => false,
                    "Dark" => true,
                    _ => detect_system_dark_mode(),
                };
                ui.global::<Theme>().set_is_dark_mode(is_dark);
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
            if let Some(_ui) = ui_handle_resume.upgrade() {
                let _ = dispatcher_resume.dispatch(UiCommand::ResumeRun(id.to_string()));
            }
        });

        let ui_handle_delete = ui.as_weak();
        let dispatcher_delete = self.dispatcher.clone();
        ui.on_delete_run_clicked(move |id| {
            if let Some(_ui) = ui_handle_delete.upgrade() {
                let _ = dispatcher_delete.dispatch(UiCommand::DeleteRun(id.to_string()));
            }
        });

        let ui_handle_recover = ui.as_weak();
        let dispatcher_recover = self.dispatcher.clone();
        ui.on_recover_run_clicked(move |id| {
            if let Some(_ui) = ui_handle_recover.upgrade() {
                let _ = dispatcher_recover.dispatch(UiCommand::RecoverRun(id.to_string()));
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

        let dispatcher_setting = self.dispatcher.clone();
        ui.on_update_setting(move |k, v| {
            let _ = dispatcher_setting.dispatch(UiCommand::UpdateSetting {
                key: k.to_string(),
                value: v.to_string(),
            });
        });

        let dispatcher_start = self.dispatcher.clone();
        ui.on_start_processing(move || {
            let _ = dispatcher_start.dispatch(UiCommand::StartProcessing);
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
                ui.set_current_step(0); // Go back to Welcome page
            }
        });

        let ui_handle_open = ui.as_weak();
        ui.on_open_output_folder(move || {
            if let Some(ui) = ui_handle_open.upgrade() {
                let dest = ui.get_settings().destination_path;
                let dest_str = dest.to_string();

                #[cfg(target_os = "windows")]
                {
                    std::process::Command::new("explorer")
                        .arg(&dest_str)
                        .spawn()
                        .ok();
                }
                #[cfg(target_os = "macos")]
                {
                    std::process::Command::new("open")
                        .arg(&dest_str)
                        .spawn()
                        .ok();
                }
                #[cfg(target_os = "linux")]
                {
                    std::process::Command::new("xdg-open")
                        .arg(&dest_str)
                        .spawn()
                        .ok();
                }
            }
        });

        let ui_handle_copy = ui.as_weak();
        ui.on_copy_log_path(move || {
            if let Some(_ui) = ui_handle_copy.upgrade() {
                let log_dir = directories::ProjectDirs::from(
                    "",
                    "TakeoutRestorerTeam",
                    "GooglePhotosRestorer",
                )
                .map(|dirs| dirs.data_dir().join("logs").join("app.log"))
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default().join("app.log"));

                let log_path_str = log_dir.to_string_lossy().to_string();
                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                    let _ = clipboard.set_text(log_path_str);
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
                    state.x = pos.x as f32;
                    state.y = pos.y as f32;
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

pub fn detect_system_dark_mode() -> bool {
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        if let Ok(hkcu) = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize")
        {
            if let Ok(val) = hkcu.get_value::<u32, _>("AppsUseLightTheme") {
                return val == 0;
            }
        }
    }
    true
}
