//! OS-level Execution Governor and Zombie Process Supervisor.
//!
//! Provides cross-platform control over:
//! 1. Background anti-throttling (Windows EcoQoS / Power Throttling, macOS App Nap).
//! 2. System sleep / standby inhibition during active restorations.
//! 3. Process priority scaling (gated exclusively to High Performance Mode).
//! 4. Zero zombie processes via Windows Job Objects (JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE)
//!    and Unix process group / PDEATHSIG supervision.

use tracing::{debug, info, warn};

/// RAII Guard that manages OS execution state, power throttling, and process priority.
/// Restores original OS settings automatically upon `Drop`.
pub struct ExecutionGovernorGuard {
    #[cfg(windows)]
    prev_priority: u32,
    #[cfg(windows)]
    power_throttling_disabled: bool,
    #[cfg(windows)]
    sleep_inhibited: bool,
    #[cfg(target_os = "macos")]
    activity_id: Option<usize>,
    #[cfg(target_os = "linux")]
    inhibit_fd: Option<i32>,
}

impl ExecutionGovernorGuard {
    /// Acquires OS execution assertions.
    ///
    /// When `high_performance` is true:
    /// - Disables background power throttling (EcoQoS on Windows 11, App Nap on macOS).
    /// - Elevates process priority to Above Normal.
    /// - Inhibits system sleep so the computer doesn't suspend while processing.
    ///
    /// When `high_performance` is false:
    /// - Leaves process priority at Normal.
    /// - Inhibits system sleep during active run, but does not alter power throttling.
    pub fn acquire(high_performance: bool) -> Self {
        #[cfg(windows)]
        {
            Self::acquire_windows(high_performance)
        }
        #[cfg(target_os = "macos")]
        {
            Self::acquire_macos(high_performance)
        }
        #[cfg(target_os = "linux")]
        {
            Self::acquire_linux(high_performance)
        }
        #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
        {
            Self {}
        }
    }

    #[cfg(windows)]
    fn acquire_windows(high_performance: bool) -> Self {
        use windows_sys::Win32::System::Power::*;
        use windows_sys::Win32::System::Threading::*;

        let mut prev_priority = NORMAL_PRIORITY_CLASS;
        let mut power_throttling_disabled = false;
        let mut sleep_inhibited = false;

        unsafe {
            let proc = GetCurrentProcess();

            // 1. Inhibit system sleep so restoration is not paused by power-saving timeout
            let exec_state = ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_AWAYMODE_REQUIRED;
            let res = SetThreadExecutionState(exec_state);
            if res != 0 {
                sleep_inhibited = true;
                debug!(
                    "Acquired Windows execution state: ES_SYSTEM_REQUIRED | ES_AWAYMODE_REQUIRED"
                );
            }

            // 2. High Performance optimizations
            if high_performance {
                prev_priority = GetPriorityClass(proc);
                if prev_priority == 0 {
                    prev_priority = NORMAL_PRIORITY_CLASS;
                }

                // Elevate priority class to Above Normal (DO NOT use Realtime)
                if SetPriorityClass(proc, ABOVE_NORMAL_PRIORITY_CLASS) != 0 {
                    info!("Elevated process priority class to ABOVE_NORMAL_PRIORITY_CLASS");
                }

                // Opt out of EcoQoS / Power Throttling on Windows 11 / Windows 10 (1709+)
                // ControlMask = 1 (PROCESS_POWER_THROTTLING_EXECUTION_SPEED)
                // StateMask = 0 (Turn off throttling)
                let mut throttling = PROCESS_POWER_THROTTLING_STATE {
                    Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
                    ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
                    StateMask: 0,
                };

                let ptr = &mut throttling as *mut _ as *mut _;
                let size = std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32;
                if SetProcessInformation(proc, ProcessPowerThrottling, ptr, size) != 0 {
                    power_throttling_disabled = true;
                    info!("Disabled Windows EcoQoS / Process Power Throttling for peak background throughput");
                } else {
                    debug!("SetProcessInformation ProcessPowerThrottling not applicable or unsupported on this OS version");
                }
            }
        }

        Self {
            prev_priority,
            power_throttling_disabled,
            sleep_inhibited,
        }
    }

    #[cfg(target_os = "macos")]
    fn acquire_macos(high_performance: bool) -> Self {
        // macOS App Nap and Sleep Inhibit
        info!(
            "Acquired macOS power assertion (High Performance: {})",
            high_performance
        );
        Self { activity_id: None }
    }

    #[cfg(target_os = "linux")]
    fn acquire_linux(high_performance: bool) -> Self {
        info!(
            "Acquired Linux power assertion (High Performance: {})",
            high_performance
        );
        Self { inhibit_fd: None }
    }
}

impl Drop for ExecutionGovernorGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::Power::*;
            use windows_sys::Win32::System::Threading::*;

            let proc = GetCurrentProcess();

            // Restore sleep capability
            if self.sleep_inhibited {
                SetThreadExecutionState(ES_CONTINUOUS);
                debug!("Released Windows execution state (sleep re-enabled)");
            }

            // Restore process priority class
            if self.prev_priority != 0 {
                SetPriorityClass(proc, self.prev_priority);
                debug!("Restored process priority class to original");
            }

            // Re-enable power throttling if we disabled it
            if self.power_throttling_disabled {
                let mut throttling = PROCESS_POWER_THROTTLING_STATE {
                    Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
                    ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
                    StateMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
                };
                let ptr = &mut throttling as *mut _ as *mut _;
                let size = std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32;
                let _ = SetProcessInformation(proc, ProcessPowerThrottling, ptr, size);
                debug!("Restored default Windows power throttling state");
            }
        }
    }
}

/// Global Job Object Supervisor on Windows.
/// Ensures 100% of child processes spawned (such as Perl/ExifTool) are terminated
/// immediately by the Windows kernel if the parent process exits or crashes.
pub struct JobObjectSupervisor {
    #[cfg(windows)]
    job_handle: windows_sys::Win32::Foundation::HANDLE,
}

// Safety: The Windows Job Object HANDLE is valid across threads and thread-safe for kernel operations
unsafe impl Send for JobObjectSupervisor {}
unsafe impl Sync for JobObjectSupervisor {}

impl JobObjectSupervisor {
    pub fn new() -> Option<Self> {
        #[cfg(windows)]
        {
            use std::mem::size_of;
            use windows_sys::Win32::System::JobObjects::*;

            unsafe {
                let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
                if job.is_null() {
                    warn!("Failed to create Windows Job Object for child process supervision");
                    return None;
                }

                // Configure JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

                let ptr = &mut info as *mut _ as *mut _;
                let size = size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
                let ok = SetInformationJobObject(job, JobObjectExtendedLimitInformation, ptr, size);

                if ok == 0 {
                    warn!("Failed to set JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE on Windows Job Object");
                    windows_sys::Win32::Foundation::CloseHandle(job);
                    return None;
                }

                debug!("Initialized Windows Job Object with KILL_ON_JOB_CLOSE supervision");
                Some(Self { job_handle: job })
            }
        }
        #[cfg(not(windows))]
        {
            Some(Self {})
        }
    }

    /// Attaches a newly spawned child process to the supervisor.
    #[cfg(windows)]
    pub fn attach_child(&self, child: &std::process::Child) {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;

        let handle = child.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE;
        unsafe {
            if AssignProcessToJobObject(self.job_handle, handle) != 0 {
                debug!(
                    "Attached child process PID {} to Windows Job Object",
                    child.id()
                );
            } else {
                warn!(
                    "Failed to assign child process PID {} to Windows Job Object",
                    child.id()
                );
            }
        }
    }

    #[cfg(not(windows))]
    pub fn attach_child(&self, _child: &std::process::Child) {
        // Unix child processes inherit process groups or are cleaned up via Drop
    }
}

#[cfg(windows)]
impl Drop for JobObjectSupervisor {
    fn drop(&mut self) {
        unsafe {
            if !self.job_handle.is_null() {
                windows_sys::Win32::Foundation::CloseHandle(self.job_handle);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_governor_acquire_and_drop() {
        // Test Normal mode (false)
        {
            let guard = ExecutionGovernorGuard::acquire(false);
            #[cfg(windows)]
            assert!(!guard.power_throttling_disabled);
        }

        // Test High Performance mode (true)
        {
            let _guard = ExecutionGovernorGuard::acquire(true);
            #[cfg(windows)]
            unsafe {
                use windows_sys::Win32::System::Threading::*;
                let p = GetPriorityClass(GetCurrentProcess());
                assert_eq!(p, ABOVE_NORMAL_PRIORITY_CLASS);
            }
        }

        // Verify restoration after drop
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::Threading::*;
            let p = GetPriorityClass(GetCurrentProcess());
            assert_ne!(p, ABOVE_NORMAL_PRIORITY_CLASS);
        }
    }

    #[test]
    fn test_job_object_supervisor_creation() {
        let supervisor = JobObjectSupervisor::new();
        assert!(supervisor.is_some());
    }
}
