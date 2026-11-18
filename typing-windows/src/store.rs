//! Windows-provided WinRT integration. Only x64 links these Windows 8+ APIs;
//! the Windows 7 x86 portable image retains its existing imports.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(target_arch = "x86", expect(dead_code, reason = "MSIX is x64-only"))]
pub enum StartupState {
    Disabled,
    Enabled,
    DisabledByUser,
    DisabledByPolicy,
    EnabledByPolicy,
}

// The UI apartment must outlive package-service calls and host cleanup.
pub struct Runtime {
    initialized: bool,
    _thread: std::marker::PhantomData<*mut ()>,
}
impl Runtime {
    pub fn initialize(packaged: bool) -> Result<Self, Message> {
        #[cfg(target_arch = "x86_64")]
        if packaged {
            unsafe {
                windows::Win32::System::WinRT::RoInitialize(
                    windows::Win32::System::WinRT::RO_INIT_SINGLETHREADED,
                )
            }
            .map_err(|error| {
                msg!(
                    "store.runtime_error",
                    "Cannot initialize Windows package services: {error}",
                    error = error.to_string()
                )
            })?;
        }
        Ok(Self {
            initialized: packaged,
            _thread: std::marker::PhantomData,
        })
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        #[cfg(target_arch = "x86_64")]
        if self.initialized {
            unsafe {
                windows::Win32::System::WinRT::RoUninitialize();
            }
        }
        #[cfg(target_arch = "x86")]
        let _ = self.initialized;
    }
}

#[cfg(target_arch = "x86_64")]
mod native {
    use super::*;
    use windows::{
        ApplicationModel::{StartupTask, StartupTaskState},
        Storage::ApplicationData,
        Win32::System::WinRT::*,
    };

    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                RoUninitialize();
            }
        }
    }

    fn initialized(multithreaded: bool) -> windows::core::Result<Apartment> {
        unsafe {
            RoInitialize(if multithreaded {
                RO_INIT_MULTITHREADED
            } else {
                RO_INIT_SINGLETHREADED
            })?;
        }
        Ok(Apartment)
    }
    pub fn local_cache() -> windows::core::Result<PathBuf> {
        Ok(PathBuf::from(
            ApplicationData::Current()?
                .LocalCacheFolder()?
                .Path()?
                .to_os_string(),
        ))
    }
    pub fn startup(enabled: Option<bool>) -> windows::core::Result<StartupState> {
        let _apartment = initialized(true)?;
        let task = StartupTask::GetAsync(&"TypingStartup".into())?.get()?;
        if let Some(enabled) = enabled {
            if enabled {
                task.RequestEnableAsync()?.get()?;
            } else {
                task.Disable()?;
            }
        }
        let state = task.State()?;
        Ok(match state {
            StartupTaskState::Disabled => StartupState::Disabled,
            StartupTaskState::Enabled => StartupState::Enabled,
            StartupTaskState::DisabledByUser => StartupState::DisabledByUser,
            StartupTaskState::DisabledByPolicy => StartupState::DisabledByPolicy,
            StartupTaskState::EnabledByPolicy => StartupState::EnabledByPolicy,
            _ => {
                return Err(windows::core::Error::from(windows::core::HRESULT(
                    0x80004005u32 as i32,
                )));
            }
        })
    }
}

pub fn local_cache() -> Result<PathBuf, Message> {
    #[cfg(target_arch = "x86_64")]
    {
        native::local_cache().map_err(|e| {
            msg!(
                "store.cache_error",
                "Cannot locate the package's local cache folder: {error}",
                error = e.to_string()
            )
        })
    }
    #[cfg(target_arch = "x86")]
    {
        Err(msg!(
            "store.x64_required",
            "The installed MSIX edition requires Windows x64."
        ))
    }
}

pub fn startup(enabled: Option<bool>) -> Result<StartupState, Message> {
    #[cfg(target_arch = "x86_64")]
    {
        native::startup(enabled).map_err(|e| {
            msg!(
                "store.startup_error",
                "Windows startup task operation failed: {error}",
                error = e.to_string()
            )
        })
    }
    #[cfg(target_arch = "x86")]
    {
        let _ = enabled;
        Err(msg!(
            "store.x64_required",
            "The installed MSIX edition requires Windows x64."
        ))
    }
}
