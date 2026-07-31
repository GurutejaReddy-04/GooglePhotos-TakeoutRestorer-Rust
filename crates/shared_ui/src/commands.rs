/// Commands expressing user intent.
/// Frontends dispatch these commands rather than invoking Core directly.
#[derive(Debug, Clone, PartialEq)]
pub enum UiCommand {
    StartProcessing,
    CancelProcessing,
    PauseProcessing,
    ResumeProcessing,
    SelectInputDirectory(String),
    SelectInputDirectories(Vec<String>),
    SelectOutputDirectory(String),
    SetInputPaths(Vec<String>),
    UpdateSetting {
        key: String,
        value: String,
    },
    UpdateResultsFilter {
        search: String,
        status_filter: String,
    },
    ResetState,
    ResumeRun(String),
    DeleteRun(String),
    ClearAllRuns,
    RecoverRun(String),

    Shutdown,
}

/// Dispatcher trait to allow frontends to send commands to the Core Engine
/// or to an orchestration layer (like `crates/app/src/main.rs`).
pub trait CommandDispatcher: Send + Sync {
    fn dispatch(&self, cmd: UiCommand) -> Result<(), String>;
}
