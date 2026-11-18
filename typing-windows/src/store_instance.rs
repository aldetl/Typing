//! Per-user/package ownership spans sessions; only same-session Show is forwarded.
use super::*;
use windows_sys::Win32::{
    Security::{Authorization::*, *},
    System::Threading::*,
};

const WAIT_MS: u32 = 5_000;
const ACKNOWLEDGED: usize = 0x54595049;

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

struct Security(*mut std::ffi::c_void);
impl Security {
    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0,
            bInheritHandle: 0,
        }
    }
}
impl Drop for Security {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}

fn os_error() -> Message {
    msg!(
        "store.instance_error",
        "Installed TypingVE activation failed: {error}",
        error = std::io::Error::last_os_error()
    )
}

fn identity() -> Result<(String, Security), Message> {
    unsafe {
        let mut token = 0;
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(os_error());
        }
        let token = Handle(token);
        let mut size = 0;
        GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut size);
        if size == 0 {
            return Err(os_error());
        }
        let mut buffer = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
        if GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            size,
            &mut size,
        ) == 0
        {
            return Err(os_error());
        }
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut sid = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut sid) == 0 {
            return Err(os_error());
        }
        let mut length = 0;
        while *sid.add(length) != 0 {
            length += 1;
        }
        let name = String::from_utf16_lossy(std::slice::from_raw_parts(sid, length));
        LocalFree(sid.cast());
        let mut descriptor = std::ptr::null_mut();
        let sddl = wide(&format!("D:P(A;;GA;;;SY)(A;;GA;;;{name})"));
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(os_error());
        }
        Ok((name, Security(descriptor)))
    }
}

fn show_message() -> u32 {
    static MESSAGE: OnceLock<u32> = OnceLock::new();
    *MESSAGE
        .get_or_init(|| unsafe { RegisterWindowMessageW(wide("Typing.Store.Show.v1").as_ptr()) })
}

pub struct Instance {
    mutex: Handle,
    ready: Handle,
    class: Vec<u16>,
    window: HWND,
}

impl Instance {
    pub fn enter(family: &str, show: bool) -> Result<Option<Self>, Message> {
        let (user, security) = identity()?;
        let key = format!("Typing.Store.{user}.{family}");
        let class = wide(&key);
        unsafe {
            let attributes = security.attributes();
            let mutex = CreateMutexW(&attributes, 0, wide(&format!(r"Global\{key}")).as_ptr());
            if mutex == 0 {
                return Err(os_error());
            }
            let mutex = Handle(mutex);
            let ready = CreateEventW(
                &attributes,
                1,
                0,
                wide(&format!(r"Global\{key}.Ready")).as_ptr(),
            );
            if ready == 0 {
                return Err(os_error());
            }
            let ready = Handle(ready);
            match WaitForSingleObject(mutex.0, 0) {
                WAIT_OBJECT_0 | WAIT_ABANDONED => {
                    if ResetEvent(ready.0) == 0 {
                        ReleaseMutex(mutex.0);
                        return Err(os_error());
                    }
                    Ok(Some(Self {
                        mutex,
                        ready,
                        class,
                        window: 0,
                    }))
                }
                WAIT_TIMEOUT if !show => Ok(None),
                WAIT_TIMEOUT => {
                    let handles = [mutex.0, ready.0];
                    let result = WaitForMultipleObjects(2, handles.as_ptr(), 0, WAIT_MS);
                    if result == WAIT_OBJECT_0 || result == WAIT_ABANDONED_0 {
                        ReleaseMutex(mutex.0);
                    }
                    if result != WAIT_OBJECT_0 + 1 {
                        return Err(msg!(
                            "store.activation_unavailable",
                            "The installed TypingVE instance could not accept this launch. Try again after it finishes starting or exits."
                        ));
                    }
                    let window = FindWindowExW(HWND_MESSAGE, 0, class.as_ptr(), std::ptr::null());
                    if window == 0 {
                        return Err(msg!(
                            "store.other_session",
                            "Installed TypingVE is already running in another Windows session or its activation window is unavailable. Quit that instance before starting another."
                        ));
                    }
                    let mut owner = 0;
                    GetWindowThreadProcessId(window, &mut owner);
                    AllowSetForegroundWindow(owner);
                    let mut reply = 0;
                    if show_message() == 0
                        || SendMessageTimeoutW(
                            window,
                            show_message(),
                            0,
                            0,
                            SMTO_ABORTIFHUNG | SMTO_BLOCK,
                            WAIT_MS,
                            &mut reply,
                        ) == 0
                        || reply != ACKNOWLEDGED
                    {
                        return Err(msg!(
                            "store.activation_unavailable",
                            "The installed TypingVE instance could not accept this launch. Try again after it finishes starting or exits."
                        ));
                    }
                    Ok(None)
                }
                _ => Err(os_error()),
            }
        }
    }

    pub fn publish(&mut self, main: HWND) -> Result<(), Message> {
        unsafe {
            if show_message() == 0 {
                return Err(os_error());
            }
            let mut class: WNDCLASSW = std::mem::zeroed();
            class.hInstance = GetModuleHandleW(std::ptr::null());
            class.lpfnWndProc = Some(activation_window);
            class.lpszClassName = self.class.as_ptr();
            if RegisterClassW(&class) == 0 {
                return Err(os_error());
            }
            self.window = CreateWindowExW(
                0,
                self.class.as_ptr(),
                self.class.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                0,
                class.hInstance,
                main as *const std::ffi::c_void,
            );
            if self.window == 0 {
                return Err(os_error());
            }
            if SetEvent(self.ready.0) == 0 {
                return Err(os_error());
            }
            Ok(())
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            ResetEvent(self.ready.0);
            if self.window != 0 {
                DestroyWindow(self.window);
            }
            ReleaseMutex(self.mutex.0);
        }
    }
}

unsafe extern "system" fn activation_window(
    window: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
) -> LRESULT {
    unsafe {
        if message == WM_NCCREATE {
            SetWindowLongPtrW(
                window,
                GWLP_USERDATA,
                (*(l as *const CREATESTRUCTW)).lpCreateParams as _,
            );
        }
        if message == show_message() && w == 0 && l == 0 {
            let main = GetWindowLongPtrW(window, GWLP_USERDATA) as HWND;
            if main != 0 && PostMessageW(main, WM_STORE_SHOW, 0, 0) != 0 {
                return ACKNOWLEDGED as isize;
            }
            return 0;
        }
        DefWindowProcW(window, message, w, l)
    }
}
