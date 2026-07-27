//! # Telemetry & Event Architecture
//!
//! This module defines the `AppEvent` system that replaces direct progress polling.
//!
//! ## Event Bus Contract
//! - **Delivery Guarantees:** At-most-once delivery. Events are broadcast to all currently connected subscribers.
//! - **Ordering Guarantees:** Strict FIFO (First-In, First-Out) per publisher thread.
//! - **Failure Behavior:** If a subscriber panics or drops its receiver, the bus automatically detects the disconnect and removes the subscriber on the next publish attempt.
//! - **Queue Limits:** The bus uses a bounded `sync_channel` with a capacity of 10,000 events per subscriber to prevent Out-Of-Memory (OOM) conditions.
//! - **Slow Subscriber Policy:** If a subscriber's queue fills up (i.e., it lags behind by 10,000 events), the bus will proactively disconnect and drop that slow subscriber to prevent backpressuring the Core Engine worker threads. The Core Engine must never be blocked by a slow UI.
//! - **Thread-Safety:** The `EventPublisher` is `Send + Sync`. Internal state is protected by a `RwLock`, ensuring parallel processes (like Rayon workers) can publish concurrently without data races.

use crate::state_db::FileStatus;
use std::sync::{mpsc, RwLock};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub enum AppEvent {
    ProcessingPhaseStarted {
        name: String,
        total_files: Option<u64>,
    },
    FileProcessed {
        file_id: i64,
        status: FileStatus,
        bytes_written: u64,
    },
    Warning {
        message: String,
    },
    Error {
        file_id: Option<i64>,
        fatal: bool,
        message: String,
    },
    ProgressStats {
        completed: u64,
        total: u64,
        eta_seconds: Option<u64>,
        speed_bps: u64,
    },
    RecentRunsLoaded(Vec<crate::state_db::RecentRun>),
    ConfigChanged(crate::config::Config),
    DestinationValidated(crate::validation::DestinationValidationResult),
    CancellationAcknowledged,
    ExifToolDownloadProgress {
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    RunCompleted {
        results: Vec<crate::state_db::MediaFile>,
    },
    ResultsFilterChanged {
        search: String,
        status_filter: String,
    },
    StateReset,
}

pub trait EventPublisher: Send + Sync {
    fn publish(&self, event: AppEvent);
}

pub trait EventSubscriber: Send {
    fn try_recv(&self) -> Result<AppEvent, mpsc::TryRecvError>;
    fn recv_timeout(&self, timeout: Duration) -> Result<AppEvent, mpsc::RecvTimeoutError>;
}

/// A fan-out broadcaster that sends events to multiple subscribers.
pub struct Broadcaster {
    senders: RwLock<Vec<mpsc::SyncSender<AppEvent>>>,
}

impl Broadcaster {
    pub fn new() -> Self {
        Self {
            senders: RwLock::new(Vec::new()),
        }
    }
}

impl Default for Broadcaster {
    fn default() -> Self {
        Self::new()
    }
}

impl Broadcaster {
    /// Subscribes to the event bus.
    /// Returns a receiver that will buffer up to 10,000 events.
    pub fn subscribe(&self) -> mpsc::Receiver<AppEvent> {
        let (tx, rx) = mpsc::sync_channel(10000);
        self.senders
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .push(tx);
        rx
    }
}

impl EventPublisher for Broadcaster {
    fn publish(&self, event: AppEvent) {
        let mut senders = self.senders.write().unwrap_or_else(|e| e.into_inner());

        senders.retain(|tx| {
            match tx.try_send(event.clone()) {
                Ok(_) => true,
                Err(mpsc::TrySendError::Full(_)) => {
                    // Slow subscriber policy: Disconnect if the 10,000 event buffer is full.
                    // This prevents backpressure from halting the Core Engine.
                    tracing::warn!(
                        "Slow subscriber detected (queue full). Disconnecting subscriber."
                    );
                    false
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    // Normal disconnect
                    false
                }
            }
        });
    }
}

pub struct Subscriber(mpsc::Receiver<AppEvent>);

impl Subscriber {
    pub fn new(rx: mpsc::Receiver<AppEvent>) -> Self {
        Self(rx)
    }
}

impl EventSubscriber for Subscriber {
    fn try_recv(&self) -> Result<AppEvent, mpsc::TryRecvError> {
        self.0.try_recv()
    }

    fn recv_timeout(&self, timeout: Duration) -> Result<AppEvent, mpsc::RecvTimeoutError> {
        self.0.recv_timeout(timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_multi_subscriber_delivery() {
        let bus = Broadcaster::new();
        let sub1 = bus.subscribe();
        let sub2 = bus.subscribe();

        bus.publish(AppEvent::RunCompleted {
            results: Vec::new(),
        });

        assert_eq!(
            sub1.try_recv().unwrap(),
            AppEvent::RunCompleted {
                results: Vec::new()
            }
        );
        assert_eq!(
            sub2.try_recv().unwrap(),
            AppEvent::RunCompleted {
                results: Vec::new()
            }
        );
    }

    #[test]
    fn test_subscriber_disconnect() {
        let bus = Broadcaster::new();
        let sub1 = bus.subscribe();

        {
            let _sub2 = bus.subscribe();
            assert_eq!(bus.senders.read().unwrap().len(), 2);
        } // sub2 dropped here

        // Publishing forces the bus to prune disconnected senders
        bus.publish(AppEvent::RunCompleted {
            results: Vec::new(),
        });

        assert_eq!(bus.senders.read().unwrap().len(), 1);
        assert_eq!(
            sub1.try_recv().unwrap(),
            AppEvent::RunCompleted {
                results: Vec::new()
            }
        );
    }

    #[test]
    fn test_event_ordering() {
        let bus = Broadcaster::new();
        let sub = bus.subscribe();

        bus.publish(AppEvent::ProcessingPhaseStarted {
            name: "A".to_string(),
            total_files: None,
        });
        bus.publish(AppEvent::RunCompleted {
            results: Vec::new(),
        });

        assert_eq!(
            sub.try_recv().unwrap(),
            AppEvent::ProcessingPhaseStarted {
                name: "A".to_string(),
                total_files: None
            }
        );
        assert_eq!(
            sub.try_recv().unwrap(),
            AppEvent::RunCompleted {
                results: Vec::new()
            }
        );
    }

    #[test]
    fn test_concurrent_publishing() {
        let bus = Arc::new(Broadcaster::new());
        let sub = bus.subscribe();

        let mut handles = vec![];
        for _ in 0..10 {
            let bus_clone = Arc::clone(&bus);
            handles.push(thread::spawn(move || {
                bus_clone.publish(AppEvent::RunCompleted {
                    results: Vec::new(),
                });
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        let mut count = 0;
        while sub.try_recv().is_ok() {
            count += 1;
        }
        assert_eq!(count, 10);
    }

    #[test]
    fn test_slow_subscriber_policy() {
        let bus = Broadcaster::new();

        // We can't easily mock the 10000 limit without changing the code,
        // but we can manually force it if we had a configurable limit.
        // For testing, we just trust the implementation or we can fill the queue.
        let sub = bus.subscribe();

        for _ in 0..10000 {
            bus.publish(AppEvent::RunCompleted {
                results: Vec::new(),
            });
        }

        // The queue is now full. The next publish should disconnect `sub`.
        bus.publish(AppEvent::CancellationAcknowledged);

        // Check that the bus pruned the sender
        assert_eq!(bus.senders.read().unwrap().len(), 0);

        // `sub` should still have the original 10000 events
        let mut count = 0;
        while sub.try_recv().is_ok() {
            count += 1;
        }
        assert_eq!(count, 10000);
    }
}
