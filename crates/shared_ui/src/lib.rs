//! Platform-agnostic view models, commands, and snapshot event bridge for UI frontends.

pub mod commands;
pub mod updater;
pub mod view_models;
pub mod watch;

pub use commands::{CommandDispatcher, UiCommand};
pub use updater::ViewModelUpdater;
pub use view_models::{ProcessingSnapshot, ProcessingViewModelBuilder, SnapshotPolicy};
