use std::sync::{Arc, Condvar, Mutex};

struct Shared<T> {
    state: Mutex<(u64, T)>,
    cvar: Condvar,
}

pub struct Sender<T> {
    shared: Arc<Shared<T>>,
}

pub struct Receiver<T> {
    shared: Arc<Shared<T>>,
    last_seen: Mutex<u64>,
}

/// A lightweight, synchronous watch channel similar to `tokio::sync::watch`.
/// Always retains the latest value. Receivers can wait for updates.
pub fn channel<T>(initial: T) -> (Sender<T>, Receiver<T>) {
    let shared = Arc::new(Shared {
        state: Mutex::new((0, initial)),
        cvar: Condvar::new(),
    });

    (
        Sender {
            shared: Arc::clone(&shared),
        },
        Receiver {
            shared,
            last_seen: Mutex::new(0),
        },
    )
}

impl<T> Sender<T> {
    /// Pushes a new value and notifies all waiting receivers.
    pub fn send(&self, value: T) {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        state.0 += 1;
        state.1 = value;
        self.shared.cvar.notify_all();
    }
}

impl<T: Clone> Receiver<T> {
    /// Borrows the current value immediately (without blocking).
    pub fn borrow(&self) -> T {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .1
            .clone()
    }

    /// Blocks until the value is updated by the sender.
    pub fn wait_changed(&self) -> T {
        let mut last_seen = self.last_seen.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        while state.0 == *last_seen {
            state = self
                .shared
                .cvar
                .wait(state)
                .unwrap_or_else(|e| e.into_inner());
        }
        *last_seen = state.0;
        state.1.clone()
    }
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        let last_seen = *self.last_seen.lock().unwrap_or_else(|e| e.into_inner());
        Self {
            shared: Arc::clone(&self.shared),
            last_seen: Mutex::new(last_seen),
        }
    }
}
