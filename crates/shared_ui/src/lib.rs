//! Google Photos Takeout Restorer - Shared UI Crate
//! Platform-agnostic view models, commands, and snapshot event bridge for UI frontends.
//!
//! Author: Guruteja Reddy Nallachi (<https://github.com/GurutejaReddy-04>)
//! Open Source Software released under MIT / Apache-2.0 License.

pub mod commands;
pub mod updater;
pub mod view_models;
pub mod watch;

pub use commands::{CommandDispatcher, UiCommand};
pub use updater::ViewModelUpdater;
pub use view_models::{ProcessingSnapshot, ProcessingViewModelBuilder, SnapshotPolicy};
