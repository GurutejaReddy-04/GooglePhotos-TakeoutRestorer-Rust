use crate::view_models::{ProcessingSnapshot, ProcessingViewModelBuilder, SnapshotPolicy};
use crate::watch;
use core::events::AppEvent;
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::Instant;

/// Background worker that consumes raw `AppEvent`s from the Core Engine,
/// applies them to a `ProcessingViewModelBuilder`, and publishes
/// `ProcessingSnapshot`s according to the specified `SnapshotPolicy`.
pub struct ViewModelUpdater {
    // We only expose a handle to optionally join the thread
    // The actual state is pushed to the `watch::Receiver`.
}

impl ViewModelUpdater {
    pub fn spawn(
        event_rx: Receiver<AppEvent>,
        policy: SnapshotPolicy,
    ) -> watch::Receiver<ProcessingSnapshot> {
        let (tx, rx) = watch::channel(ProcessingSnapshot::default());

        thread::spawn(move || {
            let mut builder = ProcessingViewModelBuilder::new();
            let mut last_publish = Instant::now();

            for event in event_rx {
                let is_terminal = matches!(
                    event,
                    AppEvent::RunCompleted { .. } | AppEvent::CancellationAcknowledged
                );
                let is_phase_change = matches!(event, AppEvent::ProcessingPhaseStarted { .. });

                builder.apply_event(event);

                let should_publish = match policy {
                    SnapshotPolicy::Immediate => true,
                    SnapshotPolicy::Debounced(duration) => {
                        is_terminal || is_phase_change || last_publish.elapsed() >= duration
                    }
                    SnapshotPolicy::Manual => is_terminal,
                };

                if should_publish {
                    let snapshot = builder.build_snapshot();
                    tx.send(snapshot);
                    last_publish = Instant::now();
                }

                // Note: The loop will naturally exit when the `event_rx` channel is closed,
                // which happens on application shutdown when the Broadcaster is dropped.
            }
        });

        rx
    }
}
