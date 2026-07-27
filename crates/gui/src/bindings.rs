use shared_ui::ProcessingSnapshot as CoreProcessingSnapshot;
use slint::{ComponentHandle, ModelRc, VecModel, Weak};
use std::rc::Rc;

use crate::{MainWindow, Theme};
use shared_ui::view_models::DestinationValidation;

pub fn update_ui_from_snapshot(ui_handle: &Weak<MainWindow>, snapshot: &CoreProcessingSnapshot) {
    let current_phase_text = snapshot.current_phase_text.clone();
    let formatted_progress = snapshot.formatted_progress.clone();
    let eta_text = snapshot.eta_text.clone();
    let speed_text = snapshot.speed_text.clone();
    let is_finished = snapshot.is_finished;
    let is_paused = snapshot.is_paused;
    let total_files = snapshot.total_files;
    let completed_files = snapshot.completed_files;
    let _sequence_number = snapshot.sequence_number;
    let recent_runs = snapshot.recent_runs.clone();

    let settings = snapshot.settings.clone();

    let handle_clone = ui_handle.clone();
    let snapshot_clone = snapshot.clone();

    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = handle_clone.upgrade() {
            ui.set_processing(crate::ProcessingSnapshot {
                current_phase_text: current_phase_text.into(),
                formatted_progress: formatted_progress.into(),
                eta_text: eta_text.into(),
                speed_text: speed_text.into(),
                elapsed_text: snapshot_clone.elapsed_text.into(),
                current_filename: snapshot_clone.current_filename.into(),
                has_errors: snapshot_clone.has_errors,
                error_count: snapshot_clone.error_count as i32,
                ok_count: snapshot_clone.ok_count as i32,
                skipped_count: snapshot_clone.skipped_count as i32,
                unmatched_count: snapshot_clone.unmatched_count as i32,
                total_files: total_files as i32,
                completed_files: completed_files as i32,
                image_count: snapshot_clone.image_count as i32,
                video_count: snapshot_clone.video_count as i32,
                output_bytes: snapshot_clone.output_bytes as f32,
                is_finished,
                is_paused,
                is_processing: snapshot_clone.is_processing,
                results: {
                    let mut mapped = Vec::new();
                    for res in &snapshot_clone.results {
                        mapped.push(crate::FileResult {
                            filename: res.filename.clone().into(),
                            status: res.status.clone().into(),
                            destination: res.destination.clone().into(),
                            timestamp: res.timestamp.clone().into(),
                            media_type: res.media_type.clone().into(),
                            error: res.error.clone().into(),
                        });
                    }
                    slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(mapped)))
                },
                live_logs: {
                    let mut logs = Vec::new();
                    for log in &snapshot_clone.live_logs {
                        logs.push(log.clone().into());
                    }
                    slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(logs)))
                },
            });
            // Map recent runs
            let mut past_runs = Vec::new();
            for run in &recent_runs {
                past_runs.push(crate::PastRun {
                    id: run.id.clone().into(),
                    title: run.title.clone().into(),
                    last_active: run.last_active.clone().into(),
                    percent_complete: run.percent_complete as i32,
                    completed_files: run.completed_files as i32,
                    total_files: run.total_files as i32,
                    has_recovery_data: run.has_recovery_data,
                });
            }
            ui.set_past_runs(ModelRc::from(Rc::new(VecModel::from(past_runs))));

            // Keep settings logic since it's used elsewhere
            let dest_val = match settings.destination_validation.clone() {
                DestinationValidation::NotSet => crate::DestinationValidation {
                    kind: "not_set".into(),
                    free_gb: 0.0,
                    total_gb: 0.0,
                    pct_free: 0.0,
                    message: "".into(),
                },
                DestinationValidation::Valid {
                    free_gb,
                    total_gb,
                    pct_free,
                    message,
                } => crate::DestinationValidation {
                    kind: "valid".into(),
                    free_gb: free_gb as f32,
                    total_gb: total_gb as f32,
                    pct_free: pct_free as f32,
                    message: message.into(),
                },
                DestinationValidation::Warning {
                    free_gb,
                    total_gb,
                    pct_free,
                    message,
                } => crate::DestinationValidation {
                    kind: "warning".into(),
                    free_gb: free_gb as f32,
                    total_gb: total_gb as f32,
                    pct_free: pct_free as f32,
                    message: message.into(),
                },
                DestinationValidation::Error { message } => crate::DestinationValidation {
                    kind: "error".into(),
                    free_gb: 0.0,
                    total_gb: 0.0,
                    pct_free: 0.0,
                    message: message.into(),
                },
            };

            ui.set_settings(crate::SettingsSnapshot {
                destination_path: settings.destination_path.clone().unwrap_or_default().into(),
                destination_validation: dest_val,
                gps_enabled: settings.gps_enabled,
                timezone_enabled: settings.timezone_enabled,
                unmatched_enabled: settings.unmatched_enabled,
                anonymous_logging: settings.anonymous_logging,
                output_mode: settings.output_mode.clone().into(),
                high_performance: settings.high_performance,
                theme: settings.theme.clone().into(),
            });

            let is_dark = match settings.theme.as_str() {
                "Light" => false,
                "Dark" => true,
                _ => crate::detect_system_dark_mode(),
            };
            ui.global::<Theme>().set_is_dark_mode(is_dark);
        }
    });
}
