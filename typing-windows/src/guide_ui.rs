//! Native, read-only user-guide dialog. Formatting is shared with the host.
use super::*;

struct GuideDialog {
    view: HWND,
    close: HWND,
    font: HFONT,
    dpi: u32,
    render_key: Option<(i32, u32, u32, u64)>,
    done: bool,
}

fn layout(window: HWND, update_font: bool) {
    let pointer = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *mut GuideDialog;
    if pointer.is_null() {
        return;
    }
    let dialog = unsafe { &mut *pointer };
    let text_scale = text_scale();
    let height = ((11 * dialog.dpi * text_scale + 3600) / 7200) as i32;
    if update_font {
        let font = unsafe {
            CreateFontW(
                -height,
                0,
                0,
                0,
                FW_NORMAL as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                0,
                0,
                CLEARTYPE_QUALITY as u32,
                0,
                wide("Segoe UI").as_ptr(),
            )
        };
        if font != 0 {
            unsafe {
                SendMessageW(dialog.close, WM_SETFONT, font as usize, 1);
            }
            let old = std::mem::replace(&mut dialog.font, font);
            if old != 0 {
                unsafe {
                    DeleteObject(old);
                }
            }
        }
        dialog.render_key = None;
    }
    let mut bounds: RECT = unsafe { std::mem::zeroed() };
    unsafe {
        GetClientRect(window, &mut bounds);
    }
    let gap = ui_gap(dialog.dpi);
    let width = (bounds.right - gap * 2).max(1);
    let button_height = height + gap;
    let button_width = profiles::control_width(dialog.close, gap * 2, height * 4).min(width);
    let button_y = (bounds.bottom - gap - button_height).max(gap);
    unsafe {
        MoveWindow(dialog.view, gap, gap, width, (button_y - gap * 2).max(1), 1);
        MoveWindow(
            dialog.close,
            gap + width - button_width,
            button_y,
            button_width,
            button_height,
            1,
        );
    }
    let key = (width, dialog.dpi, text_scale, language::snapshot().revision);
    if dialog.render_key != Some(key) {
        render_help(dialog.view, dialog.font);
        dialog.render_key = Some(key);
    }
}

unsafe extern "system" fn dialog_proc(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let pointer = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *mut GuideDialog;
    if !pointer.is_null() {
        match message {
            WM_CLOSE | WM_COMMAND if message == WM_CLOSE || w & 0xffff == 2 => {
                unsafe {
                    (*pointer).done = true;
                }
                return 0;
            }
            WM_SIZE => {
                layout(window, false);
                return 0;
            }
            WM_DPICHANGED | WM_SETTINGCHANGE => {
                if message == WM_DPICHANGED {
                    unsafe {
                        (*pointer).dpi = (w & 0xffff) as u32;
                    }
                    if l != 0 {
                        let bounds = unsafe { &*(l as *const RECT) };
                        unsafe {
                            SetWindowPos(
                                window,
                                0,
                                bounds.left,
                                bounds.top,
                                bounds.right - bounds.left,
                                bounds.bottom - bounds.top,
                                SWP_NOZORDER | SWP_NOACTIVATE,
                            );
                        }
                    }
                }
                layout(window, true);
                return 0;
            }
            WM_GETMINMAXINFO if l != 0 => {
                let info = unsafe { &mut *(l as *mut MINMAXINFO) };
                let dpi = unsafe { (*pointer).dpi };
                info.ptMinTrackSize = POINT {
                    x: scale(320, dpi),
                    y: scale(240, dpi),
                };
                return 0;
            }
            _ => {}
        }
    }
    unsafe { DefWindowProcW(window, message, w, l) }
}

pub(super) fn show(owner: HWND) -> Result<(), Message> {
    static CLASS: OnceLock<bool> = OnceLock::new();
    if !CLASS.get_or_init(|| {
        let name = wide("TypingGuideDialog");
        let class = WNDCLASSW {
            lpfnWndProc: Some(dialog_proc),
            hInstance: unsafe { GetModuleHandleW(std::ptr::null()) },
            hCursor: unsafe { LoadCursorW(0, IDC_ARROW) },
            hbrBackground: (COLOR_BTNFACE + 1) as isize,
            lpszClassName: name.as_ptr(),
            ..unsafe { std::mem::zeroed() }
        };
        unsafe { RegisterClassW(&class) != 0 }
    }) {
        return Err(last_error(&msg!(
            "windows.operation.register_user_guide_dialog",
            "register user guide dialog"
        )));
    }
    let dpi = compatibility::window_dpi(owner);
    let mut bounds: RECT = unsafe { std::mem::zeroed() };
    let mut monitor: MONITORINFO = unsafe { std::mem::zeroed() };
    monitor.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    unsafe {
        GetWindowRect(owner, &mut bounds);
        if GetMonitorInfoW(
            MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        ) == 0
        {
            SystemParametersInfoW(
                SPI_GETWORKAREA,
                0,
                (&mut monitor.rcWork as *mut RECT).cast(),
                0,
            );
        }
    }
    let work = monitor.rcWork;
    let width = scale(720, dpi).min(work.right - work.left).max(1);
    let height = scale(560, dpi).min(work.bottom - work.top).max(1);
    let x = (bounds.left + (bounds.right - bounds.left - width) / 2)
        .clamp(work.left, (work.right - width).max(work.left));
    let y = (bounds.top + (bounds.bottom - bounds.top - height) / 2)
        .clamp(work.top, (work.bottom - height).max(work.top));
    let window = unsafe {
        CreateWindowExW(
            WS_EX_CONTROLPARENT,
            wide("TypingGuideDialog").as_ptr(),
            wide(&msg!("ui.typing_user_guide", "Typing User Guide")).as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            x,
            y,
            width,
            height,
            owner,
            0,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        )
    };
    if window == 0 {
        return Err(last_error(&msg!(
            "windows.operation.create_user_guide_dialog",
            "create user guide dialog"
        )));
    }
    let controls = (|| {
        Ok::<_, Message>((
            help_view(window)?,
            child(
                window,
                "BUTTON",
                msg!("ui.close", "Close"),
                WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
                2,
            )?,
        ))
    })();
    let (view, close) = match controls {
        Ok(controls) => controls,
        Err(error) => {
            unsafe {
                DestroyWindow(window);
            }
            return Err(error);
        }
    };
    let mut dialog = Box::new(GuideDialog {
        view,
        close,
        font: 0,
        dpi,
        render_key: None,
        done: false,
    });
    unsafe {
        SetWindowLongPtrW(
            window,
            GWLP_USERDATA,
            (&mut *dialog as *mut GuideDialog) as isize as _,
        );
    }
    layout(window, true);
    let shared = SHARED.get().unwrap();
    let previous = shared.editor_active.swap(true, Ordering::SeqCst);
    let focus = unsafe { GetFocus() };
    unsafe {
        EnableWindow(owner, 0);
        ShowWindow(window, SW_SHOW);
        SetForegroundWindow(window);
        SetFocus(view);
    }
    let mut message: MSG = unsafe { std::mem::zeroed() };
    let mut result = Ok(());
    loop {
        if dialog.done {
            break;
        }
        let received = unsafe { GetMessageW(&mut message, 0, 0, 0) };
        if received <= 0 {
            if received == 0 {
                unsafe {
                    PostQuitMessage(message.wParam as i32);
                }
            } else {
                result = Err(last_error(&msg!(
                    "windows.operation.read_user_guide_messages",
                    "read user guide messages"
                )));
            }
            break;
        }
        if unsafe { IsDialogMessageW(window, &message) } == 0 {
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
    shared.editor_active.store(previous, Ordering::SeqCst);
    unsafe {
        SetWindowLongPtrW(window, GWLP_USERDATA, 0);
        DestroyWindow(window);
        if dialog.font != 0 {
            DeleteObject(dialog.font);
        }
        EnableWindow(owner, 1);
        SetForegroundWindow(owner);
        if IsWindow(focus) != 0 {
            SetFocus(focus);
        }
    }
    result
}
