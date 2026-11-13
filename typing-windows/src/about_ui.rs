//! Native About dialog and its fixed official hyperlinks.
use super::*;
use windows_sys::Win32::{System::Com::*, UI::Shell::*};

const WEBSITE: &str = "https://xdetail.org/typing";
const GITHUB: &str = "https://github.com/aldetl/Typing";

pub fn show(owner: HWND) {
    let title = wide(&msg!("ui.about_typing", "About Typing"));
    let message = msg!(
        "main.typing_fast_vim_and_emacs_cursor",
        "Typing {arg0}\r\nFast vim and emacs cursor moving and convenient key mapping\r\n\r\nWebsite: https://xdetail.org/typing\r\nGitHub: https://github.com/aldetl/Typing",
        arg0 = env!("CARGO_PKG_VERSION")
    );
    // The catalog supplies plain text; only the fixed official URLs become markup.
    let mut content = language::render(&message)
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    for target in [WEBSITE, GITHUB] {
        content = content.replace(target, &format!("<a href=\"{target}\">{target}</a>"));
    }
    let content: Vec<u16> = content.encode_utf16().chain(Some(0)).collect();
    let configuration = TASKDIALOGCONFIG {
        cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: owner,
        dwFlags: TDF_ENABLE_HYPERLINKS
            | TDF_ALLOW_DIALOG_CANCELLATION
            | TDF_POSITION_RELATIVE_TO_WINDOW,
        dwCommonButtons: TDCBF_OK_BUTTON,
        pszWindowTitle: title.as_ptr(),
        pszContent: content.as_ptr(),
        Anonymous1: TASKDIALOGCONFIG_0 {
            pszMainIcon: TD_INFORMATION_ICON,
        },
        pfCallback: Some(callback),
        ..unsafe { std::mem::zeroed() }
    };
    let result = unsafe {
        TaskDialogIndirect(
            &configuration,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if result < 0 {
        unsafe {
            MessageBoxW(
                owner,
                wide(&msg!(
                    "windows.about_dialog_failed",
                    "Unable to open the About dialog."
                ))
                .as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR,
            )
        };
    }
}

unsafe extern "system" fn callback(
    window: HWND,
    notification: u32,
    _: WPARAM,
    link: LPARAM,
    _: isize,
) -> i32 {
    if notification == TDN_HYPERLINK_CLICKED as u32 && link != 0 {
        // The notification owns a NUL-terminated UTF-16 URL for this call.
        // Compare through the terminator and dispatch only an exact official URL.
        let clicked = link as *const u16;
        let Some(target) = [WEBSITE, GITHUB].into_iter().find(|target| {
            target
                .encode_utf16()
                .chain(Some(0))
                .enumerate()
                .all(|(index, value)| unsafe { clicked.add(index).read() == value })
        }) else {
            return 0;
        };
        // Shell handlers can require STA COM. Keep it alive until the shell
        // has finished dispatching the open request, without changing the
        // user's browser association or accepting arbitrary translated URLs.
        let initialized = unsafe {
            CoInitializeEx(
                std::ptr::null(),
                (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
            )
        };
        let mut opened = false;
        if initialized >= 0 {
            let verb = wide("open");
            let url = wide(target);
            let mut execution = SHELLEXECUTEINFOW {
                cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
                fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
                hwnd: window,
                lpVerb: verb.as_ptr(),
                lpFile: url.as_ptr(),
                nShow: SW_SHOWNORMAL,
                ..unsafe { std::mem::zeroed() }
            };
            opened = unsafe { ShellExecuteExW(&mut execution) != 0 };
            unsafe { CoUninitialize() };
        }
        if !opened {
            unsafe {
                MessageBoxW(
                    window,
                    wide(&msg!(
                        "windows.website_open_failed",
                        "Unable to open the link in your default browser."
                    ))
                    .as_ptr(),
                    wide(&msg!("ui.about_typing", "About Typing")).as_ptr(),
                    MB_OK | MB_ICONERROR,
                )
            };
        }
    }
    0
}
