//! Optional APIs are resolved at runtime so the x86 image can load on Windows 7.
use super::*;
use typing_core::{Message, msg};
use windows_sys::Win32::{Graphics::Gdi::*, System::SystemInformation::*, UI::HiDpi::*};

fn user32(name: &'static [u8]) -> Option<unsafe extern "system" fn() -> isize> {
    unsafe {
        windows_sys::Win32::System::LibraryLoader::GetProcAddress(
            GetModuleHandleW(wide("user32.dll").as_ptr()),
            name.as_ptr(),
        )
    }
}

pub fn prepare_process() {
    unsafe {
        if let Some(address) = user32(b"SetProcessDpiAwarenessContext\0") {
            let set: unsafe extern "system" fn(isize) -> BOOL = std::mem::transmute(address);
            if set(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) != 0 {
                return;
            }
        }
        // Windows 7 has one effective DPI for the application.
        SetProcessDPIAware();
    }
}

pub fn window_dpi(window: HWND) -> u32 {
    unsafe {
        if let Some(address) = user32(b"GetDpiForWindow\0") {
            let get: unsafe extern "system" fn(HWND) -> u32 = std::mem::transmute(address);
            return get(window).max(96);
        }
        let dc = GetDC(window);
        let dpi = GetDeviceCaps(dc, LOGPIXELSX as i32);
        ReleaseDC(window, dc);
        dpi.max(96) as u32
    }
}

pub fn window_frame(rect: &mut RECT, dpi: u32) {
    unsafe {
        if let Some(address) = user32(b"AdjustWindowRectExForDpi\0") {
            let adjust: unsafe extern "system" fn(*mut RECT, u32, BOOL, u32, u32) -> BOOL =
                std::mem::transmute(address);
            adjust(rect, WS_OVERLAPPEDWINDOW, 0, 0, dpi);
        } else {
            AdjustWindowRectEx(rect, WS_OVERLAPPEDWINDOW, 0, 0);
        }
    }
}

pub struct PowerNotification {
    handle: isize,
    unregister: unsafe extern "system" fn(isize) -> BOOL,
}
impl PowerNotification {
    pub fn register(window: HWND) -> Result<Option<Self>, Message> {
        match (
            user32(b"RegisterSuspendResumeNotification\0"),
            user32(b"UnregisterSuspendResumeNotification\0"),
        ) {
            (Some(register), Some(unregister)) => unsafe {
                let register: unsafe extern "system" fn(HANDLE, u32) -> isize =
                    std::mem::transmute(register);
                let handle = register(window, DEVICE_NOTIFY_WINDOW_HANDLE);
                if handle == 0 {
                    return Err(msg!(
                        "compatibility.register_sleep_wake_notifications",
                        "register sleep/wake notifications: {arg0}",
                        arg0 = std::io::Error::last_os_error()
                    ));
                }
                Ok(Some(Self {
                    handle,
                    unregister: std::mem::transmute::<
                        unsafe extern "system" fn() -> isize,
                        unsafe extern "system" fn(isize) -> BOOL,
                    >(unregister),
                }))
            },
            _ => {
                let mut version: OSVERSIONINFOW = unsafe { std::mem::zeroed() };
                version.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;
                if unsafe { GetVersionExW(&mut version) } == 0
                    || (version.dwMajorVersion, version.dwMinorVersion) >= (6, 2)
                {
                    Err(msg!(
                        "compatibility.sleep_wake_registration_apis_are_unavailable",
                        "Sleep/wake registration APIs are unavailable on this Windows version."
                    ))
                } else {
                    Ok(None)
                } // Windows 7 sends legacy WM_POWERBROADCAST notifications.
            }
        }
    }
}
impl Drop for PowerNotification {
    fn drop(&mut self) {
        if unsafe { (self.unregister)(self.handle) } == 0 {
            logging::warning(
                "desktop",
                format_args!(
                    "unregister sleep/wake notifications: {}",
                    std::io::Error::last_os_error()
                ),
            );
        }
    }
}
