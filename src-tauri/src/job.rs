#[cfg(windows)]
mod platform {
    use std::mem::size_of;

    use tokio::process::Child;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        },
    };

    pub struct ProcessJob(HANDLE);

    unsafe impl Send for ProcessJob {}

    impl ProcessJob {
        pub fn assign(child: &Child) -> Result<Self, String> {
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() {
                    return Err(std::io::Error::last_os_error().to_string());
                }
                let mut information: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let configured = SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    std::ptr::addr_of!(information).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if configured == 0 {
                    CloseHandle(job);
                    return Err(std::io::Error::last_os_error().to_string());
                }
                let process = child
                    .raw_handle()
                    .ok_or_else(|| "The process handle is unavailable.".to_owned())?
                    as HANDLE;
                if AssignProcessToJobObject(job, process) == 0 {
                    CloseHandle(job);
                    return Err(std::io::Error::last_os_error().to_string());
                }
                Ok(Self(job))
            }
        }
    }

    impl Drop for ProcessJob {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use tokio::process::Child;

    pub struct ProcessJob;

    impl ProcessJob {
        pub fn assign(_child: &Child) -> Result<Self, String> {
            Ok(Self)
        }
    }
}

pub use platform::ProcessJob;
