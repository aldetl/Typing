//! Native views and desktop resources. Shared policy lives in typing-host.
use super::*;
use std::{cell::RefCell, sync::mpsc::Receiver};
use typing_host::{Feedback, WindowSize};
use windows_sys::Win32::{
    Graphics::Gdi::*,
    System::{Registry::*, RemoteDesktop::*, StationsAndDesktops::*},
    UI::{Accessibility::*, Controls::*},
};

#[path = "about_ui.rs"]
mod about;
#[path = "guide_ui.rs"]
mod guide;
pub use about::show as show_about;
#[path = "movement_picture.rs"]
mod movement_picture;
#[path = "profile_ui.rs"]
mod profiles;
pub use profiles::{confirm_exit, profile_status, show_profile_recovery};

fn help_view(parent: HWND) -> Result<HWND, Message> {
    // Rich Edit's window procedure must remain loaded until all child windows
    // are destroyed. The OS releases this process-lifetime module at exit.
    static MODULE: OnceLock<Result<isize, Message>> = OnceLock::new();
    MODULE
        .get_or_init(|| {
            let mut directory = vec![0u16; 32768];
            let length = unsafe {
                windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW(
                    directory.as_mut_ptr(),
                    directory.len() as u32,
                )
            } as usize;
            if length == 0 || length >= directory.len() {
                return Err(last_error(&msg!(
                    "windows.operation.locate_system_rich_edit",
                    "locate system Rich Edit"
                )));
            }
            let path =
                PathBuf::from(String::from_utf16_lossy(&directory[..length])).join("Msftedit.dll");
            let module = unsafe {
                windows_sys::Win32::System::LibraryLoader::LoadLibraryW(
                    wide(&path.to_string_lossy()).as_ptr(),
                )
            };
            if module == 0 {
                Err(last_error(&msg!(
                    "windows.operation.load_system_rich_edit",
                    "load system Rich Edit"
                )))
            } else {
                Ok(module)
            }
        })
        .as_ref()
        .map_err(Clone::clone)?;
    child(
        parent,
        "RICHEDIT50W",
        "",
        WS_TABSTOP
            | WS_BORDER
            | WS_VSCROLL
            | WS_HSCROLL
            | ES_MULTILINE as u32
            | ES_AUTOVSCROLL as u32
            | ES_READONLY as u32,
        GUIDE,
    )
}

fn render_help(view: HWND, font: HFONT) {
    #[repr(C)]
    struct SetText {
        flags: u32,
        codepage: u32,
    }
    #[repr(C)]
    struct CharFormat {
        size: u32,
        mask: u32,
        effects: u32,
        height: i32,
        offset: i32,
        color: u32,
        charset: u8,
        pitch: u8,
        face: [u16; 32],
    }
    let mut label_format: CharFormat = unsafe { std::mem::zeroed() };
    label_format.size = std::mem::size_of::<CharFormat>() as u32;
    let mut text_rect: RECT = unsafe { std::mem::zeroed() };
    let mut selection = [0i32; 2];
    let mut scroll: POINT = unsafe { std::mem::zeroed() };
    unsafe {
        SendMessageW(view, WM_USER + 52, 0, selection.as_mut_ptr() as isize); // EM_EXGETSEL
        SendMessageW(view, WM_USER + 221, 0, &mut scroll as *mut _ as isize); // EM_GETSCROLLPOS
        SendMessageW(view, WM_SETREDRAW, 0, 0);
        SendMessageW(view, WM_SETFONT, font as usize, 0);
        SendMessageW(view, WM_USER + 225, 0, 0); // EM_SETZOOM: reset to 100%
        SendMessageW(view, WM_USER + 72, 0, 0); // EM_SETTARGETDEVICE: wrap to view
        SendMessageW(view, WM_USER + 58, 0, &mut label_format as *mut _ as isize); // EM_GETCHARFORMAT, SCF_DEFAULT
        SendMessageW(view, EM_GETRECT, 0, &mut text_rect as *mut _ as isize);
    }
    let native_dpi = compatibility::window_dpi(view).max(96);
    let width_twips =
        ((text_rect.right - text_rect.left - 2).max(1) as i64 * 1440 / native_dpi as i64) as i32;
    let font_size = (label_format.height + 5) / 10;
    let mut rtf =
        typing_host::help::markdown_rtf(&language::snapshot().guide, width_twips, font_size)
            .into_bytes();
    rtf.push(0);
    let format = SetText {
        flags: 0,
        codepage: 1252,
    };
    unsafe {
        SendMessageW(
            view,
            WM_USER + 97,
            &format as *const _ as usize,
            rtf.as_ptr() as isize,
        ); // EM_SETTEXTEX
        // Apply the actual label font/size to all text, including headings,
        // code and tables, while retaining bold, italic and paragraph styles.
        label_format.mask = 0x80000000 | 0x20000000 | 0x08000000; // CFM_SIZE | CFM_FACE | CFM_CHARSET
        SendMessageW(view, WM_USER + 68, 4, &label_format as *const _ as isize); // EM_SETCHARFORMAT, SCF_ALL
        SendMessageW(view, WM_USER + 55, 0, selection.as_ptr() as isize); // EM_EXSETSEL
        SendMessageW(view, WM_USER + 222, 0, &scroll as *const _ as isize); // EM_SETSCROLLPOS
        SendMessageW(view, WM_SETREDRAW, 1, 0);
        InvalidateRect(view, std::ptr::null(), 1);
    }
}

const TAB: usize = 2000;
const KEY_EVENTS: usize = 2005;
const ABOUT: usize = 2007;
const SHIFT_DOWN: usize = 2008;
const SHIFT_UP: usize = 2009;
const LEFT_DOWN: usize = 2010;
const TIMER: usize = 1;
const BINDINGS_PAGE: usize = 0;
const EVENTS_PAGE: usize = 1;
const SETTINGS_PAGE: usize = 2;
const HELP_PAGE: usize = 3;
const LANGUAGE: usize = 2020;
const LANGUAGE_LABEL: usize = 2021;
const LANGUAGE_ERROR: usize = 2022;

fn tab_titles() -> [Message; 4] {
    [
        msg!("ui.mapping_profile", "Mapping Profile"),
        msg!("ui.event_diagnostics", "Event diagnostics"),
        msg!("ui.settings", "Settings"),
        msg!("ui.help", "Help"),
    ]
}
const GUIDE: usize = 2011;
const OPEN_GUIDE: usize = 2012;
const INTRODUCTION: usize = 2013;
const MOVEMENT_LABEL: usize = 2023;
const MOVEMENT_PICTURE: usize = 2024;
const CLEAR_EVENTS: usize = 2018;
const START_WITH_WINDOWS: usize = 2019;
const WM_LAYOUT_CONTENT: u32 = WM_APP + 84;
const BASE_SPACING: i32 = 10;

thread_local! {
    // The pinned build-std macro emits a nested TLS static for Drop types.
    // Its initializer triggers this lint even though this declaration is const.
    #[cfg_attr(target_vendor = "win7", allow(clippy::missing_const_for_thread_local))]
    static UI:RefCell<Option<Desktop>>=const { RefCell::new(None) };
}

struct Page {
    window: HWND,
    controls: Vec<HWND>,
    scroll: i32,
    total: i32,
}
struct Overlay {
    window: HWND,
    state: u8,
    x: i32,
    y: i32,
}
struct Desktop {
    window: HWND,
    tabs: HWND,
    pages: Vec<Page>,
    selected: usize,
    font: HFONT,
    bold_font: HFONT,
    font_height: i32,
    dpi: u32,
    text_scale: u32,
    normal_size: WindowSize,
    preference_established: bool,
    user_resizing: bool,
    preference: PathBuf,
    first_show: bool,
    changing: bool,
    feedback: Feedback,
    overlay: Overlay,
    icons: [HICON; 3],
    icon: usize,
    events: Receiver<DiagnosticEvent>,
    displayed_events: std::collections::VecDeque<DiagnosticEvent>,
    key_list: HWND,
    keyboard_events: HWND,
    startup_checked: Option<bool>,
    pause_reasons: u8,
    resume_seen: bool,
    power_notifications: Option<compatibility::PowerNotification>,
    session_notifications: bool,
    desktop_notifications: HWINEVENTHOOK,
}

fn last_error(operation: &(impl MessageValue + ?Sized)) -> Message {
    msg!(
        "startup.detail",
        "{operation}: {arg0}",
        operation = operation,
        arg0 = std::io::Error::last_os_error()
    )
}

pub fn desktop_ready() -> bool {
    unsafe {
        let own = GetThreadDesktop(windows_sys::Win32::System::Threading::GetCurrentThreadId());
        let mut receiving_input = 0i32;
        let mut needed = 0;
        let checked = GetUserObjectInformationW(
            own,
            UOI_IO,
            (&mut receiving_input as *mut i32).cast(),
            4,
            &mut needed,
        );
        if checked == 0 {
            logging::error(
                "desktop",
                format_args!(
                    "{}",
                    last_error(&msg!(
                        "windows.operation.inspect_input_desktop",
                        "inspect input desktop"
                    ))
                ),
            );
        }
        checked != 0 && receiving_input != 0
    }
}

unsafe extern "system" fn desktop_event(
    _: HWINEVENTHOOK,
    _: u32,
    _: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    if let Some(shared) = SHARED.get() {
        unsafe {
            PostMessageW(
                shared.main_window.load(Ordering::SeqCst) as HWND,
                WM_DESKTOP,
                0,
                0,
            );
        }
    }
}
fn scale(value: i32, dpi: u32) -> i32 {
    (value as i64 * dpi as i64 / 96) as i32
}
fn ui_gap(dpi: u32) -> i32 {
    scale(BASE_SPACING, dpi)
}

fn text_block(parent: HWND, text: impl MessageValue, id: usize) -> Result<HWND, Message> {
    child(
        parent,
        "EDIT",
        text,
        WS_TABSTOP | ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32 | ES_READONLY as u32,
        id,
    )
}

// Measure the native Edit's wrapped lines and formatting margins. The same
// control renders the text, so long words and paragraph breaks wrap exactly as
// measured, including at enlarged Windows text sizes.
fn text_block_height(window: HWND, width: i32) -> i32 {
    if unsafe { GetWindowTextLengthW(window) } == 0 {
        return 0;
    }
    let mut metrics: TEXTMETRICW = unsafe { std::mem::zeroed() };
    let mut bounds: RECT = unsafe { std::mem::zeroed() };
    let mut formatting: RECT = unsafe { std::mem::zeroed() };
    unsafe {
        let dc = GetDC(window);
        let old = SelectObject(dc, SendMessageW(window, WM_GETFONT, 0, 0));
        GetTextMetricsW(dc, &mut metrics);
        SelectObject(dc, old);
        ReleaseDC(window, dc);
        GetClientRect(window, &mut bounds);
        let measure_height = bounds
            .bottom
            .max(metrics.tmHeight + ui_gap(compatibility::window_dpi(window)));
        SetWindowPos(
            window,
            0,
            0,
            0,
            width.max(1),
            measure_height,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW,
        );
        SendMessageW(window, EM_GETRECT, 0, &mut formatting as *mut RECT as isize);
        let lines = SendMessageW(window, EM_GETLINECOUNT, 0, 0).max(1) as i32;
        lines * metrics.tmHeight.max(1)
            + formatting.top.max(0)
            + (measure_height - formatting.bottom).max(0)
    }
}
fn text_scale() -> u32 {
    let mut value = 100u32;
    let mut bytes = 4;
    unsafe {
        let _ = RegGetValueW(
            HKEY_CURRENT_USER,
            wide("Software\\Microsoft\\Accessibility").as_ptr(),
            wide("TextScaleFactor").as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut value as *mut u32).cast(),
            &mut bytes,
        );
    }
    value.clamp(100, 225)
}
fn child(
    parent: HWND,
    class: &str,
    text: impl MessageValue,
    style: u32,
    id: usize,
) -> Result<HWND, Message> {
    let text = Message::value(&text);
    let window = unsafe {
        CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(&language::control_text(class, &text)).as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | style,
            0,
            0,
            1,
            1,
            parent,
            id as isize,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        )
    };
    if window == 0 {
        Err(last_error(&msg!(
            "windows.operation.create_control",
            "create control"
        )))
    } else {
        language::remember(window, &text);
        Ok(window)
    }
}
fn list(parent: HWND, labels: &[impl MessageValue]) -> Result<HWND, Message> {
    let window = child(
        parent,
        "SysListView32",
        "",
        WS_TABSTOP | WS_BORDER | LVS_REPORT | LVS_SHOWSELALWAYS,
        0,
    )?;
    unsafe {
        SendMessageW(
            window,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            0,
            (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as isize,
        );
    }
    for (index, label) in labels.iter().enumerate() {
        let mut text = wide(label);
        let column = LVCOLUMNW {
            mask: LVCF_TEXT | LVCF_WIDTH | LVCF_SUBITEM,
            cx: 150,
            pszText: text.as_mut_ptr(),
            iSubItem: index as i32,
            ..unsafe { std::mem::zeroed() }
        };
        unsafe {
            SendMessageW(
                window,
                LVM_INSERTCOLUMNW,
                index,
                &column as *const _ as isize,
            );
        }
    }
    language::columns(window, labels.iter().map(Message::value).collect());
    Ok(window)
}
fn row(window: HWND, values: &[String], newest: bool) {
    let count = unsafe { SendMessageW(window, LVM_GETITEMCOUNT, 0, 0) } as i32;
    let index = if newest { 0 } else { count };
    for (column, value) in values.iter().enumerate() {
        let mut text = wide(value);
        let item = LVITEMW {
            mask: LVIF_TEXT,
            iItem: index,
            iSubItem: column as i32,
            pszText: text.as_mut_ptr(),
            ..unsafe { std::mem::zeroed() }
        };
        unsafe {
            SendMessageW(
                window,
                if column == 0 {
                    LVM_INSERTITEMW
                } else {
                    LVM_SETITEMW
                },
                0,
                &item as *const _ as isize,
            );
        }
    }
    if newest && count >= 100 {
        unsafe {
            SendMessageW(window, LVM_DELETEITEM, 100, 0);
        }
    }
}

pub fn initialize(
    window: HWND,
    _text: &str,
    feedback: Feedback,
    preference: PathBuf,
    events: Receiver<DiagnosticEvent>,
) -> Result<(), Message> {
    unsafe {
        let controls = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_TAB_CLASSES | ICC_LISTVIEW_CLASSES,
        };
        if InitCommonControlsEx(&controls) == 0 {
            return Err(last_error(&msg!(
                "windows.operation.initialize_common_controls",
                "initialize common controls"
            )));
        }
    }
    let class_name = wide("TypingPage");
    let class = WNDCLASSW {
        lpfnWndProc: Some(page_proc),
        hInstance: unsafe { GetModuleHandleW(std::ptr::null()) },
        hCursor: unsafe { LoadCursorW(0, IDC_ARROW) },
        hbrBackground: (COLOR_WINDOW + 1) as isize,
        lpszClassName: class_name.as_ptr(),
        ..unsafe { std::mem::zeroed() }
    };
    if unsafe { RegisterClassW(&class) } == 0 {
        return Err(last_error(&msg!(
            "windows.operation.register_page_class",
            "register page class"
        )));
    }
    let overlay_name = wide("TypingCursorFeedback");
    let overlay_class = WNDCLASSW {
        lpfnWndProc: Some(overlay_proc),
        hInstance: class.hInstance,
        lpszClassName: overlay_name.as_ptr(),
        ..unsafe { std::mem::zeroed() }
    };
    if unsafe { RegisterClassW(&overlay_class) } == 0 {
        return Err(last_error(&msg!(
            "windows.operation.register_overlay_class",
            "register overlay class"
        )));
    }
    let overlay = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            overlay_name.as_ptr(),
            wide(&msg!("ui.cursor_feedback", "Typing cursor feedback")).as_ptr(),
            WS_POPUP,
            0,
            0,
            1,
            1,
            0,
            0,
            class.hInstance,
            std::ptr::null(),
        )
    };
    if overlay == 0 {
        return Err(last_error(&msg!(
            "windows.operation.create_cursor_feedback",
            "create cursor feedback"
        )));
    }
    let tabs = child(
        window,
        "SysTabControl32",
        "",
        WS_TABSTOP | TCS_OWNERDRAWFIXED,
        TAB,
    )?;
    // Keep native tab geometry and interaction; paint just the headers.
    unsafe {
        SetWindowTheme(tabs, wide("").as_ptr(), wide("").as_ptr());
    }
    let mut pages = Vec::new();
    for title in tab_titles() {
        let index = pages.len();
        let mut name = wide(&title);
        let tab = TCITEMW {
            mask: TCIF_TEXT,
            pszText: name.as_mut_ptr(),
            ..unsafe { std::mem::zeroed() }
        };
        unsafe {
            SendMessageW(tabs, TCM_INSERTITEMW, index, &tab as *const _ as isize);
        }
        let page = child(window, "TypingPage", title, WS_VSCROLL | WS_CLIPCHILDREN, 0)?;
        unsafe {
            SetWindowLongPtrW(page, GWL_EXSTYLE, WS_EX_CONTROLPARENT as _);
        }
        pages.push(Page {
            window: page,
            controls: Vec::new(),
            scroll: 0,
            total: 0,
        });
    }
    let help_page = pages[HELP_PAGE].window;
    let movement_label = text_block(
        help_page,
        msg!(
            "ui.default_cursor_movement",
            "Default cursor movement key mapping"
        ),
        MOVEMENT_LABEL,
    )?;
    let movement_picture = movement_picture::create(help_page, MOVEMENT_PICTURE)?;
    let about = child(
        help_page,
        "BUTTON",
        msg!("ui.about", "About"),
        WS_TABSTOP | BS_PUSHBUTTON as u32,
        ABOUT,
    )?;
    let open_guide = child(
        pages[HELP_PAGE].window,
        "BUTTON",
        msg!("ui.open_user_guide", "Open User Guide…"),
        WS_TABSTOP | BS_PUSHBUTTON as u32,
        OPEN_GUIDE,
    )?;
    let introduction = text_block(
        help_page,
        typing_host::help::introduction_message(),
        INTRODUCTION,
    )?;
    pages[HELP_PAGE].controls.extend([
        introduction,
        movement_label,
        movement_picture,
        open_guide,
        about,
    ]);
    let keyboard_events = child(
        pages[EVENTS_PAGE].window,
        "BUTTON",
        msg!("ui.show_keyboard_events", "Show keyboard events"),
        WS_TABSTOP | BS_AUTOCHECKBOX as u32,
        KEY_EVENTS,
    )?;
    unsafe {
        SendMessageW(keyboard_events, BM_SETCHECK, BST_CHECKED as usize, 0);
    }
    let key_list = list(
        pages[EVENTS_PAGE].window,
        &[
            msg!("ui.event", "Event"),
            msg!("ui.key_code", "Key code"),
            msg!("ui.shift", "Shift"),
            msg!("ui.alt", "Alt"),
            msg!("ui.control", "Control"),
        ],
    )?;
    let clear_events = child(
        pages[EVENTS_PAGE].window,
        "BUTTON",
        msg!("ui.clear", "Clear"),
        WS_TABSTOP,
        CLEAR_EVENTS,
    )?;
    pages[EVENTS_PAGE]
        .controls
        .extend([keyboard_events, key_list, clear_events]);
    if cfg!(debug_assertions) {
        let parent = pages[EVENTS_PAGE].window;
        for (title, id) in [
            (msg!("ui.start_keyboard", "Start keyboard"), ENABLE),
            (msg!("ui.stop_keyboard", "Stop keyboard"), DISABLE),
            (msg!("ui.right_shift_down", "Right Shift down"), SHIFT_DOWN),
            (msg!("ui.right_shift_up", "Right Shift up"), SHIFT_UP),
            (msg!("ui.left_key_down", "Left key down"), LEFT_DOWN),
        ] {
            pages[EVENTS_PAGE].controls.push(child(
                parent,
                "BUTTON",
                title,
                WS_TABSTOP | BS_PUSHBUTTON as u32,
                id,
            )?);
        }
    }
    pages[BINDINGS_PAGE].controls = profiles::initialize(window, pages[BINDINGS_PAGE].window)?;
    let settings_page = pages[SETTINGS_PAGE].window;
    let language_label = child(
        settings_page,
        "STATIC",
        msg!("language.label", "Language:"),
        0,
        LANGUAGE_LABEL,
    )?;
    let language_choice = child(
        settings_page,
        "COMBOBOX",
        "",
        WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST as u32,
        LANGUAGE,
    )?;
    pages[SETTINGS_PAGE].controls.push(child(
        settings_page,
        "BUTTON",
        msg!("ui.start_with_windows", "Start with Windows"),
        WS_TABSTOP | BS_CHECKBOX as u32 | BS_MULTILINE as u32,
        START_WITH_WINDOWS,
    )?);
    pages[SETTINGS_PAGE].controls.push(text_block(
        settings_page,
        msg!("ui.start_typing_in_the_system_tray", "Start Typing in the system tray when you sign in.\r\nIf you move the Typing folder, enable this option again from its new location."),
        0,
    )?);
    pages[SETTINGS_PAGE]
        .controls
        .push(text_block(settings_page, "", 0)?);
    let language_error = text_block(settings_page, "", LANGUAGE_ERROR)?;
    pages[SETTINGS_PAGE]
        .controls
        .extend([language_label, language_choice, language_error]);

    let saved_size = std::fs::read_to_string(&preference)
        .ok()
        .and_then(|s| WindowSize::parse(&s));
    let normal_size = saved_size.unwrap_or_default();
    let mut desktop = Desktop {
        window,
        tabs,
        pages,
        selected: BINDINGS_PAGE,
        font: 0,
        bold_font: 0,
        font_height: 16,
        dpi: compatibility::window_dpi(window),
        text_scale: text_scale(),
        normal_size,
        preference_established: saved_size.is_some(),
        user_resizing: false,
        preference,
        first_show: true,
        changing: false,
        feedback,
        overlay: Overlay {
            window: overlay,
            state: 0,
            x: i32::MIN,
            y: i32::MIN,
        },
        icons: [0; 3],
        icon: usize::MAX,
        events,
        displayed_events: std::collections::VecDeque::new(),
        key_list,
        keyboard_events,
        startup_checked: None,
        pause_reasons: 0,
        resume_seen: false,
        power_notifications: None,
        session_notifications: false,
        desktop_notifications: 0,
    };
    // Notification-area icons use the small system metric, not LoadIconW's
    // large-icon metric. Prefer the matching embedded frame before scaling.
    for (index, id) in [101, 102, 103].into_iter().enumerate() {
        let result = unsafe {
            LoadIconMetric(
                class.hInstance,
                id as *const u16,
                LIM_SMALL,
                &mut desktop.icons[index],
            )
        };
        if result < 0 {
            return Err(msg!(
                "ui.load_embedded_tray_icon_hresult",
                "load embedded tray icon {id}: HRESULT {result}",
                id = id,
                result = format!("{value:#010x}", value = result)
            ));
        }
    }
    desktop.set_font();
    desktop.refresh_startup();
    desktop.refresh_languages(true);
    desktop.power_notifications = compatibility::PowerNotification::register(window)?;
    desktop.session_notifications =
        unsafe { WTSRegisterSessionNotification(window, NOTIFY_FOR_THIS_SESSION) } != 0;
    if !desktop.session_notifications {
        return Err(last_error(&msg!(
            "windows.operation.register_session_notifications",
            "register session notifications"
        )));
    }
    desktop.desktop_notifications = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_DESKTOPSWITCH,
            EVENT_SYSTEM_DESKTOPSWITCH,
            0,
            Some(desktop_event),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    if desktop.desktop_notifications == 0 {
        return Err(last_error(&msg!(
            "windows.operation.register_desktop_notifications",
            "register desktop notifications"
        )));
    }
    if !desktop_ready() {
        desktop.pause_reasons |= 4;
        SHARED.get().unwrap().pause(true);
    }
    let mut icon = desktop.tray_data();
    icon.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    icon.uCallbackMessage = WM_TRAY;
    icon.hIcon = desktop.icons[0];
    if unsafe { Shell_NotifyIconW(NIM_ADD, &icon) } == 0 {
        return Err(last_error(&msg!(
            "windows.operation.create_tray_icon",
            "create tray icon"
        )));
    }
    if unsafe { SetTimer(window, TIMER, 50, None) } == 0 {
        return Err(last_error(&msg!(
            "windows.operation.create_feedback_timer",
            "create feedback timer"
        )));
    }
    UI.with(|cell| *cell.borrow_mut() = Some(desktop));
    Ok(())
}

impl Desktop {
    fn refresh_languages(&mut self, rescan: bool) {
        let (labels, selected, notice) = language::choices(rescan);
        let combo = self.pages[SETTINGS_PAGE].controls[4];
        unsafe {
            SendMessageW(combo, CB_RESETCONTENT, 0, 0);
            for label in labels {
                SendMessageW(combo, CB_ADDSTRING, 0, wide(&label).as_ptr() as isize);
            }
            SendMessageW(combo, CB_SETCURSEL, selected, 0);
        }
        language::set_caption(
            self.pages[SETTINGS_PAGE].controls[5],
            &notice.unwrap_or_default(),
        );
    }
    fn finish_language_change(&mut self, changed: bool) {
        let focus = unsafe { GetFocus() };
        if changed {
            language::set_caption(
                self.overlay.window,
                &msg!("ui.cursor_feedback", "Typing cursor feedback"),
            );
            for (index, title) in tab_titles().iter().enumerate() {
                let mut text = wide(title);
                let item = TCITEMW {
                    mask: TCIF_TEXT,
                    pszText: text.as_mut_ptr(),
                    ..unsafe { std::mem::zeroed() }
                };
                unsafe {
                    SendMessageW(self.tabs, TCM_SETITEMW, index, &item as *const _ as isize);
                }
            }
            for (index, event) in self.displayed_events.iter().enumerate() {
                let text = if event.down {
                    msg!("ui.down", "Down")
                } else {
                    msg!("ui.up", "Up")
                };
                let mut text = wide(&text);
                let item = LVITEMW {
                    iSubItem: 0,
                    pszText: text.as_mut_ptr(),
                    ..unsafe { std::mem::zeroed() }
                };
                unsafe {
                    SendMessageW(
                        self.key_list,
                        LVM_SETITEMTEXTW,
                        index,
                        &item as *const _ as isize,
                    );
                }
            }
        }
        self.refresh_languages(false);
        self.layout();
        unsafe {
            if IsWindow(focus) != 0 {
                SetFocus(focus);
            }
            InvalidateRect(self.window, std::ptr::null(), 1);
        }
    }
    fn draw_tab(&self, item: &DRAWITEMSTRUCT) {
        let selected =
            item.itemID as isize == unsafe { SendMessageW(self.tabs, TCM_GETCURSEL, 0, 0) };
        let mut high_contrast: HIGHCONTRASTW = unsafe { std::mem::zeroed() };
        high_contrast.cbSize = std::mem::size_of::<HIGHCONTRASTW>() as u32;
        unsafe {
            SystemParametersInfoW(
                SPI_GETHIGHCONTRAST,
                high_contrast.cbSize,
                (&mut high_contrast as *mut HIGHCONTRASTW).cast(),
                0,
            );
            let contrast = high_contrast.dwFlags & HCF_HIGHCONTRASTON != 0;
            let background = if selected && contrast {
                GetSysColor(COLOR_HIGHLIGHT)
            } else if selected {
                0x00feeadb
            } else {
                GetSysColor(COLOR_BTNFACE)
            };
            let text_color = GetSysColor(if selected && contrast {
                COLOR_HIGHLIGHTTEXT
            } else {
                COLOR_BTNTEXT
            });
            let saved = SaveDC(item.hDC);
            let brush = CreateSolidBrush(background);
            FillRect(item.hDC, &item.rcItem, brush);
            DeleteObject(brush);
            SelectObject(item.hDC, if selected { self.bold_font } else { self.font });
            SetTextColor(item.hDC, text_color);
            SetBkMode(item.hDC, TRANSPARENT as i32);
            let mut label = [0u16; 128];
            let mut tab: TCITEMW = std::mem::zeroed();
            tab.mask = TCIF_TEXT;
            tab.pszText = label.as_mut_ptr();
            tab.cchTextMax = label.len() as i32;
            SendMessageW(
                self.tabs,
                TCM_GETITEMW,
                item.itemID as usize,
                &mut tab as *mut _ as isize,
            );
            let mut bounds = item.rcItem;
            let inset = scale(4, self.dpi);
            InflateRect(&mut bounds, -inset, 0);
            DrawTextW(
                item.hDC,
                label.as_ptr(),
                -1,
                &mut bounds,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
            );
            if item.itemState & ODS_FOCUS != 0 && item.itemState & ODS_NOFOCUSRECT == 0 {
                InflateRect(&mut bounds, 0, -scale(3, self.dpi));
                DrawFocusRect(item.hDC, &bounds);
            }
            RestoreDC(item.hDC, saved);
        }
    }

    fn startup_status(&self, text: &(impl MessageValue + ?Sized)) {
        language::set_caption(self.pages[SETTINGS_PAGE].controls[2], &Message::value(text));
    }

    fn refresh_startup(&mut self) {
        let checkbox = self.pages[SETTINGS_PAGE].controls[0];
        match startup::is_registered() {
            Ok(checked) => {
                self.startup_checked = Some(checked);
                unsafe {
                    EnableWindow(checkbox, 1);
                    SendMessageW(checkbox, BM_SETCHECK, usize::from(checked), 0);
                }
                self.startup_status("");
            }
            Err(error) => {
                self.startup_checked = None;
                unsafe {
                    EnableWindow(checkbox, 0);
                    SendMessageW(checkbox, BM_SETCHECK, BST_UNCHECKED as usize, 0);
                }
                self.startup_status(&msg!(
                    "ui.could_not_read_start_with_windows",
                    "Could not read Start with Windows. {error} Reopen Settings to try again.",
                    error = error
                ));
            }
        }
    }

    fn change_startup(&mut self) {
        let Some(checked) = self.startup_checked else {
            return;
        };
        match startup::set_enabled(!checked) {
            Ok(()) => {
                self.startup_checked = Some(!checked);
                unsafe {
                    SendMessageW(
                        self.pages[SETTINGS_PAGE].controls[0],
                        BM_SETCHECK,
                        usize::from(!checked),
                        0,
                    );
                }
                self.startup_status("");
            }
            Err(error) => {
                // BS_CHECKBOX is manual: the confirmed state remains visible
                // until the registry operation succeeds.
                self.startup_status(&msg!(
                    "ui.could_not_change_start_with_windows",
                    "Could not change Start with Windows. {error}",
                    error = error
                ));
            }
        }
        self.layout_page(SETTINGS_PAGE);
    }

    fn tray_data(&self) -> NOTIFYICONDATAW {
        let mut icon: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        icon.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        icon.hWnd = self.window;
        icon.uID = 1;
        let tip = wide("Typing");
        let mut buffer = [0u16; 128];
        buffer[..tip.len()].copy_from_slice(&tip);
        icon.szTip = buffer;
        icon
    }
    fn set_font(&mut self) {
        let height = ((11 * self.dpi * self.text_scale + 3600) / 7200) as i32;
        let new_font = unsafe {
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
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                DEFAULT_PITCH as u32,
                wide("Segoe UI").as_ptr(),
            )
        };
        if new_font == 0 {
            return;
        }
        let mut bold: LOGFONTW = unsafe { std::mem::zeroed() };
        unsafe {
            GetObjectW(
                new_font,
                std::mem::size_of::<LOGFONTW>() as i32,
                (&mut bold as *mut LOGFONTW).cast(),
            );
        }
        bold.lfWeight = FW_BOLD as i32;
        let bold_font = unsafe { CreateFontIndirectW(&bold) };
        if bold_font == 0 {
            unsafe {
                DeleteObject(new_font);
            }
            return;
        }
        let old = self.font;
        let old_bold = self.bold_font;
        self.font = new_font;
        self.bold_font = bold_font;
        self.font_height = height + scale(5, self.dpi);
        let padding_x = scale(18, self.dpi) * self.text_scale as i32 / 100;
        let padding_y = scale(6, self.dpi) * self.text_scale as i32 / 100;
        unsafe {
            SendMessageW(
                self.tabs,
                TCM_SETPADDING,
                0,
                ((padding_y << 16) | (padding_x & 0xffff)) as isize,
            );
        }
        // Reserve the bold label width for every header, keeping tab geometry
        // stable when the active page changes.
        unsafe {
            SendMessageW(self.tabs, WM_SETFONT, bold_font as usize, 1);
        }
        for window in self.pages.iter().flat_map(|p| p.controls.iter().copied()) {
            unsafe {
                SendMessageW(window, WM_SETFONT, new_font as usize, 1);
            }
        }
        if old != 0 {
            unsafe {
                DeleteObject(old);
            }
        }
        if old_bold != 0 {
            unsafe {
                DeleteObject(old_bold);
            }
        }
    }
    fn save(&self) {
        if !self.preference_established {
            return;
        }
        if let Some(parent) = self.preference.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(error) = std::fs::write(&self.preference, self.normal_size.encode()) {
            logging::warning("preferences", format_args!("window size: {error}"));
        }
    }
    fn show(&mut self) {
        if self.selected == SETTINGS_PAGE {
            self.refresh_startup();
            self.refresh_languages(true);
        }
        SHARED.get().unwrap().editor_active.store(
            matches!(self.selected, BINDINGS_PAGE | SETTINGS_PAGE | HELP_PAGE),
            Ordering::SeqCst,
        );
        if self.first_show {
            self.changing = true;
            let size = self.normal_size.physical(self.dpi);
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: size.width,
                bottom: size.height,
            };
            let mut work: RECT = unsafe { std::mem::zeroed() };
            unsafe {
                compatibility::window_frame(&mut rect, self.dpi);
                let monitor = MonitorFromWindow(self.window, MONITOR_DEFAULTTONEAREST);
                let mut info = MONITORINFO {
                    cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                    ..std::mem::zeroed()
                };
                if GetMonitorInfoW(monitor, &mut info) != 0 {
                    work = info.rcWork;
                } else {
                    SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut work as *mut RECT).cast(), 0);
                }
                let w = (rect.right - rect.left).min(work.right - work.left);
                let h = (rect.bottom - rect.top).min(work.bottom - work.top);
                SetWindowPos(
                    self.window,
                    0,
                    work.left + (work.right - work.left - w) / 2,
                    work.top + (work.bottom - work.top - h) / 2,
                    w,
                    h,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            self.first_show = false;
            self.changing = false;
        }
        // Explicit user display must not inherit STARTUPINFO's SW_HIDE hint.
        // Window placement also restores a previously minimized/maximized view.
        unsafe {
            let mut placement: WINDOWPLACEMENT = std::mem::zeroed();
            placement.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
            if GetWindowPlacement(self.window, &mut placement) != 0 {
                placement.showCmd = SW_SHOWNORMAL as u32;
                placement.flags = 0;
                if SetWindowPlacement(self.window, &placement) == 0 {
                    logging::error(
                        "window",
                        format_args!(
                            "{}",
                            last_error(&msg!(
                                "windows.operation.show_main_window",
                                "show main window"
                            ))
                        ),
                    );
                }
            } else {
                logging::error(
                    "window",
                    format_args!(
                        "{}",
                        last_error(&msg!(
                            "windows.operation.read_window_placement",
                            "read window placement"
                        ))
                    ),
                );
            }
            // Visibility is an explicit user request, independent of the
            // launcher's initial show hint used by window-placement APIs.
            if SetWindowPos(
                self.window,
                0,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            ) == 0
            {
                logging::error(
                    "window",
                    format_args!(
                        "{}",
                        last_error(&msg!(
                            "windows.operation.display_main_window",
                            "display main window"
                        ))
                    ),
                );
            }
            SetForegroundWindow(self.window);
        }
        self.layout();
    }
    fn layout(&mut self) {
        let mut rect: RECT = unsafe { std::mem::zeroed() };
        unsafe {
            GetClientRect(self.window, &mut rect);
        }
        let margin = ui_gap(self.dpi);
        let width = (rect.right - 2 * margin).max(1);
        let tab_height = (rect.bottom - 2 * margin).max(1);
        unsafe {
            // Pages are siblings of the tab control. Keep its background behind
            // them; WS_CLIPSIBLINGS then prevents either from overpainting the other.
            SetWindowPos(
                self.tabs,
                HWND_BOTTOM,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            MoveWindow(self.tabs, margin, margin, width, tab_height, 1);
        }
        let mut content = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: tab_height,
        };
        unsafe {
            SendMessageW(
                self.tabs,
                TCM_ADJUSTRECT,
                0,
                &mut content as *mut _ as isize,
            );
        }
        for index in 0..self.pages.len() {
            let page = &self.pages[index];
            unsafe {
                MoveWindow(
                    page.window,
                    margin + content.left,
                    margin + content.top,
                    (content.right - content.left).max(1),
                    (content.bottom - content.top).max(1),
                    1,
                );
                ShowWindow(
                    page.window,
                    if index == self.selected {
                        SW_SHOW
                    } else {
                        SW_HIDE
                    },
                );
            }
            self.layout_page(index);
        }
    }
    fn layout_page(&mut self, index: usize) {
        let page = &mut self.pages[index];
        let mut rect: RECT = unsafe { std::mem::zeroed() };
        unsafe {
            GetClientRect(page.window, &mut rect);
        }
        let gap = ui_gap(self.dpi);
        let width = (rect.right - 2 * gap).max(1);
        let unit = self.font_height;
        let control_height = unit + gap / 2;
        let mut positions: Vec<(HWND, i32, i32, i32, i32)> = Vec::new();
        let mut y = gap;
        if index == HELP_PAGE {
            let introduction = page.controls[0];
            let label = page.controls[1];
            let picture_width = width.min(scale(285, self.dpi) * self.text_scale as i32 / 100);
            let label_height = text_block_height(label, picture_width);
            positions.push((label, gap, y, picture_width, label_height));
            y += label_height + gap;
            let picture_height = movement_picture::height(picture_width);
            positions.push((page.controls[2], gap, y, picture_width, picture_height));
            let button_width = page.controls[3..]
                .iter()
                .map(|&window| profiles::control_width(window, gap * 2, unit * 3))
                .max()
                .unwrap_or(1)
                .min(width);
            let button_height = 2 * control_height + gap;
            let beside_picture = picture_width + gap + button_width <= width;
            let button_y = if beside_picture {
                y
            } else {
                y + picture_height + gap
            };
            let button_x = gap + width - button_width;
            positions.push((
                page.controls[4],
                button_x,
                button_y,
                button_width,
                control_height,
            ));
            positions.push((
                page.controls[3],
                button_x,
                button_y + control_height + gap,
                button_width,
                control_height,
            ));
            y = (y + picture_height).max(button_y + button_height) + gap;
            let height = text_block_height(introduction, width);
            positions.push((introduction, gap, y, width, height));
            y += height + gap;
        } else if index == BINDINGS_PAGE {
            (positions, y) = profiles::layout(page, width, unit, gap);
        } else if index == SETTINGS_PAGE {
            let label = page.controls[3];
            let combo = page.controls[4];
            let label_width = profiles::control_width(label, 0, unit).min(width);
            let mut combo_width = unit * 10;
            let count = unsafe { SendMessageW(combo, CB_GETCOUNT, 0, 0) };
            for item in 0..count.max(0) {
                let length = unsafe { SendMessageW(combo, CB_GETLBTEXTLEN, item as usize, 0) };
                if length < 0 {
                    continue;
                }
                let mut text = vec![0u16; length as usize + 1];
                unsafe {
                    SendMessageW(
                        combo,
                        CB_GETLBTEXT,
                        item as usize,
                        text.as_mut_ptr() as isize,
                    );
                }
                let mut size: SIZE = unsafe { std::mem::zeroed() };
                unsafe {
                    let dc = GetDC(combo);
                    let old = SelectObject(dc, self.font);
                    GetTextExtentPoint32W(dc, text.as_ptr(), length as i32, &mut size);
                    SelectObject(dc, old);
                    ReleaseDC(combo, dc);
                }
                combo_width = combo_width.max(size.cx + unit * 2 + gap * 2);
            }
            combo_width = combo_width.min(width);
            positions.push((label, gap, y, label_width, control_height));
            let combo_x = if label_width + gap + combo_width <= width {
                gap + label_width + gap
            } else {
                y += control_height + gap;
                gap
            };
            positions.push((combo, combo_x, y, combo_width, control_height * 8));
            y += control_height + gap;
            let error = page.controls[5];
            let error_height = text_block_height(error, width);
            unsafe {
                ShowWindow(
                    error,
                    if error_height == 0 {
                        SW_HIDE
                    } else {
                        SW_SHOWNA
                    },
                );
            }
            positions.push((error, gap, y, width, error_height));
            if error_height > 0 {
                y += error_height + gap;
            }
            for (index, &window) in page.controls[..3].iter().enumerate() {
                if index != 0 {
                    let height = text_block_height(window, width);
                    unsafe {
                        ShowWindow(window, if height == 0 { SW_HIDE } else { SW_SHOWNA });
                    }
                    positions.push((window, gap, y, width, height));
                    if height > 0 {
                        y += height + gap;
                    }
                    continue;
                }
                let mut text =
                    vec![0u16; unsafe { GetWindowTextLengthW(window) }.max(0) as usize + 1];
                let length =
                    unsafe { GetWindowTextW(window, text.as_mut_ptr(), text.len() as i32) };
                if length == 0 {
                    continue;
                }
                let mut bounds = RECT {
                    right: (width - if index == 0 { unit * 2 } else { 0 }).max(1),
                    ..unsafe { std::mem::zeroed() }
                };
                unsafe {
                    let dc = GetDC(window);
                    let old = SelectObject(dc, self.font);
                    DrawTextW(
                        dc,
                        text.as_ptr(),
                        length,
                        &mut bounds,
                        DT_CALCRECT | DT_WORDBREAK,
                    );
                    SelectObject(dc, old);
                    ReleaseDC(window, dc);
                }
                let height = (bounds.bottom + gap / 2).max(control_height);
                positions.push((window, gap, y, width, height));
                y += height + gap;
            }
        } else {
            let mut row_x = gap;
            for &window in &[self.keyboard_events, page.controls[2]] {
                let control_width = profiles::control_width(window, gap * 2, unit * 3).min(width);
                if row_x > gap && row_x + control_width > rect.right - gap {
                    row_x = gap;
                    y += control_height + gap;
                }
                positions.push((window, row_x, y, control_width, control_height));
                row_x += control_width + gap;
            }
            y += control_height + gap;
            let mut buttons = Vec::new();
            let mut x = gap;
            let mut button_y = 0;
            for &window in &page.controls[3..] {
                let button_width = profiles::control_width(window, gap * 2, unit * 4).min(width);
                if x > gap && x + button_width > rect.right - gap {
                    x = gap;
                    button_y += control_height + gap;
                }
                buttons.push((window, x, button_y, button_width, control_height));
                x += button_width + gap;
            }
            let button_height = if buttons.is_empty() {
                0
            } else {
                button_y + control_height + gap
            };
            let height = (rect.bottom - y - gap - button_height).max(unit * 4);
            positions.push((self.key_list, gap, y, width, height));
            y += height + gap;
            positions.extend(
                buttons
                    .into_iter()
                    .map(|(window, x, offset, width, height)| {
                        (window, x, y + offset, width, height)
                    }),
            );
            y += button_height;
        }
        page.total = y;
        page.scroll = page.scroll.clamp(0, (y - rect.bottom).max(0));
        let info = SCROLLINFO {
            cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
            fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
            nMin: 0,
            nMax: y - 1,
            nPage: rect.bottom.max(1) as u32,
            nPos: page.scroll,
            nTrackPos: 0,
        };
        unsafe {
            SetScrollInfo(page.window, SB_VERT, &info, 1);
        }
        for (window, x, y, w, h) in positions {
            unsafe {
                MoveWindow(window, x, y - page.scroll, w, h, 1);
            }
        }
        if index == BINDINGS_PAGE {
            profiles::fit_mapping_columns(page.controls[14]);
        }
        if [HELP_PAGE, SETTINGS_PAGE, BINDINGS_PAGE].contains(&index) {
            // Measuring changes the Edit bounds without painting. Repaint the
            // exposed parent area too, so its old background cannot remain
            // behind or below the buttons after the text shrinks.
            unsafe {
                RedrawWindow(
                    page.window,
                    std::ptr::null(),
                    0,
                    RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN,
                );
            }
        }
        {
            let window = self.key_list;
            let mut bounds: RECT = unsafe { std::mem::zeroed() };
            unsafe {
                GetClientRect(window, &mut bounds);
            }
            let columns = unsafe { SendMessageW(window, LVM_GETHEADER, 0, 0) };
            let count = unsafe { SendMessageW(columns, HDM_GETITEMCOUNT, 0, 0) } as usize;
            for column in 0..count {
                unsafe {
                    SendMessageW(
                        window,
                        LVM_SETCOLUMNWIDTH,
                        column,
                        (bounds.right * (column + 1) as i32 / count as i32
                            - bounds.right * column as i32 / count as i32)
                            .max(scale(80, self.dpi)) as isize,
                    );
                }
            }
        }
    }
    fn tick(&mut self) {
        let shared = SHARED.get().unwrap();
        let (icon, feedback) = {
            let state = shared.state.lock().unwrap();
            (
                state.host.icon_state(),
                self.feedback.visible_state(
                    state.admission,
                    state.core.command_mode(),
                    state.caps_on,
                ),
            )
        };
        if icon != self.icon {
            let mut data = self.tray_data();
            data.uFlags = NIF_ICON;
            data.hIcon = self.icons[icon];
            if unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) } == 0 {
                logging::error(
                    "tray",
                    format_args!(
                        "{}",
                        last_error(&msg!(
                            "windows.operation.update_tray_icon",
                            "update tray icon"
                        ))
                    ),
                );
            }
            self.icon = icon;
        }
        self.update_overlay(feedback);
        let active =
            self.selected == EVENTS_PAGE && unsafe { GetForegroundWindow() } == self.window;
        while let Ok(event) = self.events.try_recv() {
            if !active {
                continue;
            }
            if unsafe { SendMessageW(self.keyboard_events, BM_GETCHECK, 0, 0) }
                == BST_CHECKED as isize
            {
                self.displayed_events.push_front(event.clone());
                self.displayed_events.truncate(100);
                let DiagnosticEvent {
                    key,
                    down,
                    shift,
                    alt,
                    control,
                } = event;
                row(
                    self.key_list,
                    &[
                        language::render(&if down {
                            msg!("ui.down", "Down")
                        } else {
                            msg!("ui.up", "Up")
                        }),
                        key.to_string(),
                        shift.to_string(),
                        alt.to_string(),
                        control.to_string(),
                    ],
                    true,
                )
            }
        }
    }
    fn update_overlay(&mut self, state: u8) {
        if state == 0 {
            if self.overlay.state != 0 {
                unsafe {
                    ShowWindow(self.overlay.window, SW_HIDE);
                }
            }
            self.overlay.state = 0;
            return;
        }
        let mut point: POINT = unsafe { std::mem::zeroed() };
        if unsafe { GetPhysicalCursorPos(&mut point) } == 0 {
            return;
        }
        if state != self.overlay.state {
            let radius = self.feedback.radius;
            let side = radius * 2;
            let screen = unsafe { GetDC(0) };
            let dc = unsafe { CreateCompatibleDC(screen) };
            let mut info: BITMAPINFO = unsafe { std::mem::zeroed() };
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: side,
                biHeight: -side,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..unsafe { std::mem::zeroed() }
            };
            let mut bits = std::ptr::null_mut();
            let bitmap =
                unsafe { CreateDIBSection(screen, &info, DIB_RGB_COLORS, &mut bits, 0, 0) };
            if dc != 0 && bitmap != 0 && !bits.is_null() {
                let pixels = unsafe {
                    std::slice::from_raw_parts_mut(bits as *mut u32, (side * side) as usize)
                };
                for y in 0..side {
                    for x in 0..side {
                        let distance = ((x as f64 + 0.5 - radius as f64).powi(2)
                            + (y as f64 + 0.5 - radius as f64).powi(2))
                        .sqrt();
                        let alpha = ((radius as f64 - distance + 0.5).clamp(0.0, 1.0)
                            * self.feedback.opacity as f64
                            * 255.0
                            / 100.0) as u32;
                        let color = if state == 2 || (state == 3 && y < radius) {
                            self.feedback.caps_color
                        } else {
                            self.feedback.command_color
                        };
                        pixels[(y * side + x) as usize] = (alpha << 24)
                            | ((color[0] as u32 * alpha / 255) << 16)
                            | ((color[1] as u32 * alpha / 255) << 8)
                            | (color[2] as u32 * alpha / 255);
                    }
                }
                let old = unsafe { SelectObject(dc, bitmap) };
                let size = SIZE { cx: side, cy: side };
                let origin = POINT { x: 0, y: 0 };
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };
                let updated = unsafe {
                    UpdateLayeredWindow(
                        self.overlay.window,
                        screen,
                        &point,
                        &size,
                        dc,
                        &origin,
                        0,
                        &blend,
                        ULW_ALPHA,
                    )
                };
                unsafe {
                    SelectObject(dc, old);
                }
                if updated == 0 {
                    logging::error(
                        "overlay",
                        format_args!(
                            "{}",
                            last_error(&msg!(
                                "windows.operation.draw_cursor_overlay",
                                "draw cursor overlay"
                            ))
                        ),
                    );
                }
            }
            unsafe {
                if bitmap != 0 {
                    DeleteObject(bitmap);
                }
                if dc != 0 {
                    DeleteDC(dc);
                }
                ReleaseDC(0, screen);
            }
        }
        if state != self.overlay.state || point.x != self.overlay.x || point.y != self.overlay.y {
            unsafe {
                SetWindowPos(
                    self.overlay.window,
                    HWND_TOPMOST,
                    point.x,
                    point.y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            }
            self.overlay.x = point.x;
            self.overlay.y = point.y;
        }
        self.overlay.state = state;
    }
}

impl Drop for Desktop {
    fn drop(&mut self) {
        unsafe {
            self.save();
            KillTimer(self.window, TIMER);
            self.power_notifications.take();
            if self.session_notifications && WTSUnRegisterSessionNotification(self.window) == 0 {
                logging::warning(
                    "desktop",
                    format_args!(
                        "{}",
                        last_error(&msg!(
                            "windows.operation.unregister_session_notifications",
                            "unregister session notifications"
                        ))
                    ),
                );
            }
            if self.desktop_notifications != 0 && UnhookWinEvent(self.desktop_notifications) == 0 {
                logging::warning(
                    "desktop",
                    format_args!(
                        "{}",
                        last_error(&msg!(
                            "windows.operation.unregister_desktop_notifications",
                            "unregister desktop notifications"
                        ))
                    ),
                );
            }
            let icon = self.tray_data();
            Shell_NotifyIconW(NIM_DELETE, &icon);
            for icon in self.icons {
                if icon != 0 {
                    DestroyIcon(icon);
                }
            }
            DestroyWindow(self.overlay.window);
            if self.font != 0 {
                DeleteObject(self.font);
            }
            if self.bold_font != 0 {
                DeleteObject(self.bold_font);
            }
        }
    }
}

pub fn refresh() {
    UI.with(|cell| {
        if let Some(ui) = cell.borrow_mut().as_mut() {
            ui.tick();
        }
    });
}
pub fn shutdown() {
    UI.with(|cell| {
        cell.borrow_mut().take();
    });
}

pub fn message(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> Option<LRESULT> {
    if message == WM_APP + 85 {
        let selected = UI.with(|cell| {
            cell.borrow().as_ref().map(|ui| unsafe {
                SendMessageW(ui.pages[SETTINGS_PAGE].controls[4], CB_GETCURSEL, 0, 0)
            })
        });
        if let Some(selected) = selected.filter(|i| *i >= 0) {
            let changed = language::select(selected as usize).is_ok();
            if changed {
                language::refresh_captions();
                profiles::language_changed();
            }
            UI.with(|cell| {
                if let Some(ui) = cell.borrow_mut().as_mut() {
                    ui.finish_language_change(changed);
                }
            });
        }
        return Some(0);
    }
    if message == WM_APP + 41 {
        // The guide's modal loop must run without borrowing Desktop.
        if let Err(error) = guide::show(window) {
            unsafe {
                MessageBoxW(
                    window,
                    wide(&error).as_ptr(),
                    wide(&msg!("ui.typing_user_guide", "Typing User Guide")).as_ptr(),
                    MB_OK | MB_ICONERROR,
                );
            }
        }
        return Some(0);
    }
    if let Some(result) = profiles::message(window, message, w, l) {
        return Some(result);
    }
    UI.with(|cell| {
        let Ok(mut holder) = cell.try_borrow_mut() else {
            return None;
        };
        let ui = holder.as_mut()?;
        match message {
            WM_LAYOUT_CONTENT if w < ui.pages.len() => {
                ui.layout_page(w);
                Some(0)
            }
            WM_DRAWITEM if w == TAB && l != 0 => {
                let item = unsafe { &*(l as *const DRAWITEMSTRUCT) };
                if item.hwndItem == ui.tabs {
                    ui.draw_tab(item);
                    Some(1)
                } else {
                    None
                }
            }
            WM_SYSCOLORCHANGE => {
                unsafe {
                    InvalidateRect(ui.tabs, std::ptr::null(), 1);
                }
                Some(0)
            }
            WM_TIMER if w == TIMER => {
                ui.tick();
                Some(0)
            }
            WM_COMMAND if w & 0xffff == SHOW => {
                ui.show();
                Some(0)
            }
            WM_COMMAND if w & 0xffff == ABOUT => {
                // Post a separate request; the modal message loop must not hold the view borrow.
                unsafe {
                    PostMessageW(window, WM_APP + 40, 0, 0);
                }
                Some(0)
            }
            WM_COMMAND if w & 0xffff == OPEN_GUIDE => {
                unsafe {
                    PostMessageW(window, WM_APP + 41, 0, 0);
                }
                Some(0)
            }
            WM_COMMAND if w & 0xffff == CLEAR_EVENTS => {
                ui.displayed_events.clear();
                while ui.events.try_recv().is_ok() {}
                unsafe {
                    SendMessageW(ui.key_list, LVM_DELETEALLITEMS, 0, 0);
                }
                Some(0)
            }
            WM_COMMAND if w & 0xffff == LANGUAGE && w >> 16 == CBN_SELCHANGE as usize => {
                unsafe {
                    PostMessageW(window, WM_APP + 85, 0, 0);
                }
                Some(0)
            }
            WM_COMMAND
                if w & 0xffff == START_WITH_WINDOWS
                    && w >> 16 == BN_CLICKED as usize
                    && l == ui.pages[SETTINGS_PAGE].controls[0] =>
            {
                ui.change_startup();
                Some(0)
            }
            WM_COMMAND if [SHIFT_DOWN, SHIFT_UP, LEFT_DOWN].contains(&(w & 0xffff)) => {
                let action = match w & 0xffff {
                    SHIFT_DOWN => DiagnosticOutput::Down(Key::RIGHT_SHIFT),
                    SHIFT_UP => DiagnosticOutput::Up(Key::RIGHT_SHIFT),
                    _ => DiagnosticOutput::Down(Key::LEFT),
                };
                let shared = SHARED.get().unwrap();
                if shared.state.lock().unwrap().host.diagnostic(action) {
                    shared.ready.notify_one();
                }
                Some(0)
            }
            WM_NOTIFY if l != 0 => {
                let header = unsafe { &*(l as *const NMHDR) };
                if header.idFrom == TAB && header.code == TCN_SELCHANGE {
                    ui.selected = unsafe { SendMessageW(ui.tabs, TCM_GETCURSEL, 0, 0) } as usize;
                    if ui.selected == SETTINGS_PAGE {
                        ui.refresh_startup();
                        ui.refresh_languages(true);
                    }
                    SHARED.get().unwrap().editor_active.store(
                        matches!(ui.selected, BINDINGS_PAGE | SETTINGS_PAGE | HELP_PAGE),
                        Ordering::SeqCst,
                    );
                    ui.layout();
                    Some(0)
                } else {
                    None
                }
            }
            WM_SIZE => {
                if w == SIZE_MINIMIZED as usize {
                    ui.save();
                    unsafe {
                        ShowWindow(window, SW_HIDE);
                    }
                } else {
                    ui.layout();
                }
                Some(0)
            }
            WM_ENTERSIZEMOVE => {
                ui.user_resizing = false;
                Some(0)
            }
            WM_SIZING => {
                ui.user_resizing = true;
                Some(1)
            }
            WM_EXITSIZEMOVE => {
                let resized = std::mem::take(&mut ui.user_resizing);
                if resized
                    && !ui.changing
                    && unsafe { IsIconic(window) } == 0
                    && unsafe { IsZoomed(window) } == 0
                {
                    let mut rect: RECT = unsafe { std::mem::zeroed() };
                    unsafe {
                        GetClientRect(window, &mut rect);
                    }
                    let normal_size = WindowSize {
                        width: rect.right,
                        height: rect.bottom,
                    }
                    .logical(ui.dpi);
                    if normal_size != ui.normal_size {
                        ui.normal_size = normal_size;
                        ui.preference_established = true;
                    }
                    ui.save();
                }
                Some(0)
            }
            WM_CLOSE => {
                ui.save();
                unsafe {
                    ShowWindow(window, SW_HIDE);
                }
                Some(0)
            }
            WM_DPICHANGED => {
                ui.changing = true;
                ui.dpi = (w & 0xffff) as u32;
                ui.set_font();
                if l != 0 {
                    let rect = unsafe { &*(l as *const RECT) };
                    unsafe {
                        SetWindowPos(
                            window,
                            0,
                            rect.left,
                            rect.top,
                            rect.right - rect.left,
                            rect.bottom - rect.top,
                            SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                    }
                }
                ui.layout();
                ui.changing = false;
                Some(0)
            }
            WM_SETTINGCHANGE => {
                ui.text_scale = text_scale();
                ui.set_font();
                ui.layout();
                Some(0)
            }
            WM_POWERBROADCAST => {
                let mut new_cycle = false;
                match w as u32 {
                    PBT_APMSUSPEND => {
                        ui.pause_reasons |= 1;
                        ui.resume_seen = false;
                    }
                    PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => {
                        new_cycle = w as u32 == PBT_APMRESUMEAUTOMATIC || !ui.resume_seen;
                        ui.resume_seen = true;
                        ui.pause_reasons &= !1;
                    }
                    _ => return Some(1),
                }
                if desktop_ready() {
                    ui.pause_reasons &= !4;
                } else {
                    ui.pause_reasons |= 4;
                }
                SHARED
                    .get()
                    .unwrap()
                    .recover(ui.pause_reasons != 0, new_cycle);
                Some(1)
            }
            WM_WTSSESSION_CHANGE => {
                match w as u32 {
                    WTS_SESSION_LOCK | WTS_CONSOLE_DISCONNECT | WTS_REMOTE_DISCONNECT => {
                        ui.pause_reasons |= 2
                    }
                    WTS_SESSION_UNLOCK | WTS_CONSOLE_CONNECT | WTS_REMOTE_CONNECT => {
                        ui.pause_reasons &= !2
                    }
                    _ => {}
                }
                if desktop_ready() {
                    ui.pause_reasons &= !4;
                } else {
                    ui.pause_reasons |= 4;
                }
                SHARED.get().unwrap().pause(ui.pause_reasons != 0);
                Some(0)
            }
            WM_DESKTOP => {
                if desktop_ready() {
                    ui.pause_reasons &= !4;
                } else {
                    ui.pause_reasons |= 4;
                }
                SHARED.get().unwrap().pause(ui.pause_reasons != 0);
                Some(0)
            }
            _ => None,
        }
    })
}

unsafe extern "system" fn page_proc(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if message == WM_NOTIFY {
        // Notification data belongs to the sender and is valid only during this call.
        return unsafe { SendMessageW(GetParent(window), message, w, l) };
    }
    if message == WM_COMMAND {
        unsafe {
            PostMessageW(GetParent(window), message, w, l);
        }
        return 0;
    }
    if message == WM_VSCROLL || message == WM_MOUSEWHEEL {
        UI.with(|cell| {
            if let Ok(mut holder) = cell.try_borrow_mut()
                && let Some(ui) = holder.as_mut()
                && let Some(index) = ui.pages.iter().position(|p| p.window == window)
            {
                let page = &mut ui.pages[index];
                let step = ui.font_height * 3;
                let mut info: SCROLLINFO = unsafe { std::mem::zeroed() };
                info.cbSize = std::mem::size_of::<SCROLLINFO>() as u32;
                info.fMask = SIF_ALL;
                unsafe {
                    GetScrollInfo(window, SB_VERT, &mut info);
                }
                let next = if message == WM_MOUSEWHEEL {
                    page.scroll - ((w >> 16) as u16 as i16 as i32) * step / 120
                } else {
                    match w & 0xffff {
                        0 => page.scroll - step,
                        1 => page.scroll + step,
                        2 => page.scroll - info.nPage as i32,
                        3 => page.scroll + info.nPage as i32,
                        4 | 5 => info.nTrackPos,
                        _ => page.scroll,
                    }
                };
                page.scroll = next.clamp(0, (page.total - info.nPage as i32).max(0));
                ui.layout_page(index);
            }
        });
        return 0;
    }
    unsafe { DefWindowProcW(window, message, w, l) }
}
unsafe extern "system" fn overlay_proc(
    window: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
) -> LRESULT {
    match message {
        WM_NCHITTEST => HTTRANSPARENT as isize,
        WM_MOUSEACTIVATE => MA_NOACTIVATE as isize,
        _ => unsafe { DefWindowProcW(window, message, w, l) },
    }
}

pub fn prepare_process() {
    compatibility::prepare_process();
}
pub fn preference_path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("AppData/Local")))
        .unwrap_or_else(std::env::temp_dir)
        .join("Typing/window-size.txt")
}
pub fn translate_dialog(message: &MSG) -> bool {
    let window = UI
        .with(|cell| {
            cell.try_borrow()
                .ok()
                .and_then(|v| v.as_ref().map(|ui| ui.window))
        })
        .unwrap_or(0);
    window != 0
        && unsafe { IsWindowVisible(window) } != 0
        && unsafe { IsDialogMessageW(window, message) } != 0
}
