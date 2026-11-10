//! Native profile controls. Document and interpretation policy are shared.
use super::*;
use std::path::Path;
use typing_host::profiles::{
    AvailableProfile, Document, Entry, Library, Profile, Recorder, import_document, same_file,
};
use windows_sys::Win32::UI::Controls::Dialogs::*;

const NEW: usize = 3000;
const PROFILE_LIST: usize = 3001;
const SAVE: usize = 3002;
const SAVE_AS: usize = 3003;
const COPY: usize = 3004;
const PROFILE_DELETE: usize = 3005;
const IMPORT: usize = 3006;
const ADD: usize = 3007;
const EDIT: usize = 3008;
const DELETE: usize = 3009;
const RENAME: usize = 3013;
const SET_ACTIVE: usize = 3014;
const RECORD_INPUT: usize = 3100;
const RECORD_OUTPUT: usize = 3101;
const ENTRY_ACCEPT: usize = 3104;
const ENTRY_CANCEL: usize = 3105;
const ACTION: usize = 3106;
const ENTRY_ENABLED: usize = 3107;
const WM_REFRESH_MAPPINGS: u32 = WM_APP + 83;
const WM_DIALOG_TICK: usize = 7;

thread_local! {
    // The custom Windows 7 std expands const TLS through nested statics that
    // the pinned Clippy misidentifies as non-const declarations.
    #[cfg_attr(target_vendor = "win7", allow(clippy::missing_const_for_thread_local))]
    static PROFILES: RefCell<Option<ProfileUi>> = const { RefCell::new(None) };
    #[cfg_attr(target_vendor = "win7", allow(clippy::missing_const_for_thread_local))]
    static DIALOG: RefCell<Option<EditDialog>> = const { RefCell::new(None) };
    #[cfg_attr(target_vendor = "win7", allow(clippy::missing_const_for_thread_local))]
    static RECORD_HOOK: std::cell::Cell<HHOOK> = const { std::cell::Cell::new(0) };
}

fn text(window: HWND) -> String {
    let count = unsafe { GetWindowTextLengthW(window) }.max(0) as usize;
    let mut buffer = vec![0; count + 1];
    let len =
        unsafe { GetWindowTextW(window, buffer.as_mut_ptr(), buffer.len() as i32) }.max(0) as usize;
    String::from_utf16_lossy(&buffer[..len])
}
fn set_text(window: HWND, value: &str) {
    if text(window) != value {
        unsafe {
            SetWindowTextW(window, wide(value).as_ptr());
        }
    }
}
fn alert(window: HWND, message: &str) {
    unsafe {
        MessageBoxW(
            window,
            wide(message).as_ptr(),
            wide("Typing profiles").as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}
fn ask(window: HWND, message: &str, style: u32) -> i32 {
    unsafe {
        MessageBoxW(
            window,
            wide(message).as_ptr(),
            wide("Typing profiles").as_ptr(),
            style,
        )
    }
}
fn combo(parent: HWND, id: usize, values: &[&str]) -> Result<HWND, String> {
    let control = child(
        parent,
        "COMBOBOX",
        "",
        WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST as u32,
        id,
    )?;
    for value in values {
        unsafe {
            SendMessageW(control, CB_ADDSTRING, 0, wide(value).as_ptr() as isize);
        }
    }
    unsafe {
        SendMessageW(control, CB_SETCURSEL, 0, 0);
    }
    Ok(control)
}
struct NamePrompt {
    controls: Vec<HWND>,
    font: HFONT,
    dropdown: bool,
    done: bool,
    result: Option<String>,
}

fn layout_name_prompt(window: HWND, update_font: bool) {
    let pointer = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *mut NamePrompt;
    if pointer.is_null() {
        return;
    }
    let controls = unsafe { (*pointer).controls.clone() };
    if update_font {
        let font = dialog_font(window);
        if font != 0 {
            for &control in &controls {
                unsafe {
                    SendMessageW(control, WM_SETFONT, font as usize, 1);
                }
            }
            let old = unsafe { std::mem::replace(&mut (*pointer).font, font) };
            if old != 0 {
                unsafe {
                    DeleteObject(old);
                }
            }
        }
    }
    let dpi = compatibility::window_dpi(window);
    let gap = ui_gap(dpi);
    let mut font: LOGFONTW = unsafe { std::mem::zeroed() };
    let mut bounds: RECT = unsafe { std::mem::zeroed() };
    unsafe {
        GetObjectW(
            (*pointer).font,
            std::mem::size_of::<LOGFONTW>() as i32,
            (&mut font as *mut LOGFONTW).cast(),
        );
        GetClientRect(window, &mut bounds);
    }
    let height = font.lfHeight.abs() + gap;
    let width = (bounds.right - gap * 2).max(1);
    let required = height * 3 + gap * 4;
    if bounds.bottom < required {
        let mut outer: RECT = unsafe { std::mem::zeroed() };
        unsafe {
            GetWindowRect(window, &mut outer);
            SetWindowPos(
                window,
                0,
                0,
                0,
                outer.right - outer.left,
                outer.bottom - outer.top + required - bounds.bottom,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
    let button = control_width(controls[2], gap * 2, height * 2);
    for (control, x, y, w) in [
        (controls[0], gap, gap, width),
        (controls[1], gap, height + gap * 2, width),
        (controls[2], gap, height * 2 + gap * 3, button.min(width)),
        (
            controls[3],
            gap * 2 + button,
            height * 2 + gap * 3,
            button.min(width),
        ),
    ] {
        unsafe {
            MoveWindow(
                control,
                x,
                y,
                w,
                if (*pointer).dropdown && control == controls[1] {
                    height * 10
                } else {
                    height
                },
                1,
            );
        }
    }
}

unsafe extern "system" fn name_prompt_proc(
    window: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
) -> LRESULT {
    let pointer = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *mut NamePrompt;
    if !pointer.is_null() {
        match message {
            WM_COMMAND if [1, 3201].contains(&(w & 0xffff)) => {
                let prompt = unsafe { &mut *pointer };
                prompt.result = Some(text(prompt.controls[1]));
                prompt.done = true;
                return 0;
            }
            WM_CLOSE | WM_COMMAND if message == WM_CLOSE || [2, 3202].contains(&(w & 0xffff)) => {
                unsafe {
                    (*pointer).done = true;
                }
                return 0;
            }
            WM_SIZE => {
                layout_name_prompt(window, false);
                return 0;
            }
            WM_DPICHANGED | WM_SETTINGCHANGE => {
                if message == WM_DPICHANGED && l != 0 {
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
                layout_name_prompt(window, true);
                return 0;
            }
            _ => {}
        }
    }
    unsafe { DefWindowProcW(window, message, w, l) }
}

fn name_prompt(
    owner: HWND,
    title: &str,
    initial: &str,
    choices: Option<&[&str]>,
) -> Result<Option<String>, String> {
    static CLASS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !CLASS.get_or_init(|| {
        let name = wide("TypingProfileNameDialog");
        let class = WNDCLASSW {
            lpfnWndProc: Some(name_prompt_proc),
            hInstance: unsafe { GetModuleHandleW(std::ptr::null()) },
            hCursor: unsafe { LoadCursorW(0, IDC_ARROW) },
            hbrBackground: (COLOR_WINDOW + 1) as isize,
            lpszClassName: name.as_ptr(),
            ..unsafe { std::mem::zeroed() }
        };
        unsafe { RegisterClassW(&class) != 0 }
    }) {
        return Err(last_error("register profile name dialog"));
    }
    let dpi = compatibility::window_dpi(owner);
    let mut owner_bounds: RECT = unsafe { std::mem::zeroed() };
    unsafe {
        GetWindowRect(owner, &mut owner_bounds);
    }
    let window = unsafe {
        CreateWindowExW(
            WS_EX_CONTROLPARENT,
            wide("TypingProfileNameDialog").as_ptr(),
            wide(title).as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
            owner_bounds.left + scale(30, dpi),
            owner_bounds.top + scale(30, dpi),
            scale(360, dpi),
            scale(170, dpi),
            owner,
            0,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        )
    };
    if window == 0 {
        return Err(last_error("create profile name dialog"));
    }
    let controls = (|| -> Result<Vec<HWND>, String> {
        Ok(vec![
            child(
                window,
                "STATIC",
                if choices.is_some() {
                    "Profile"
                } else {
                    "Profile name"
                },
                0,
                0,
            )?,
            if let Some(choices) = choices {
                let selector = combo(window, 3200, choices)?;
                let index = choices
                    .iter()
                    .position(|&name| name == initial)
                    .unwrap_or(0);
                unsafe {
                    SendMessageW(selector, CB_SETCURSEL, index, 0);
                }
                selector
            } else {
                child(
                    window,
                    "EDIT",
                    initial,
                    WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
                    3200,
                )?
            },
            child(
                window,
                "BUTTON",
                "OK",
                WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
                3201,
            )?,
            child(window, "BUTTON", "Cancel", WS_TABSTOP, 3202)?,
        ])
    })();
    let controls = match controls {
        Ok(controls) => controls,
        Err(error) => {
            unsafe {
                DestroyWindow(window);
            }
            return Err(error);
        }
    };
    let mut prompt = Box::new(NamePrompt {
        controls,
        font: 0,
        dropdown: choices.is_some(),
        done: false,
        result: None,
    });
    unsafe {
        SetWindowLongPtrW(
            window,
            GWLP_USERDATA,
            (&mut *prompt as *mut NamePrompt) as isize as _,
        );
    }
    layout_name_prompt(window, true);
    let shared = SHARED.get().unwrap();
    let previous = shared.editor_active.swap(true, Ordering::SeqCst);
    unsafe {
        EnableWindow(owner, 0);
        ShowWindow(window, SW_SHOW);
        SetForegroundWindow(window);
        SetFocus(prompt.controls[1]);
        SendMessageW(prompt.controls[1], EM_SETSEL, 0, -1);
    }
    let mut message: MSG = unsafe { std::mem::zeroed() };
    loop {
        if prompt.done {
            break;
        }
        let result = unsafe { GetMessageW(&mut message, 0, 0, 0) };
        if result <= 0 {
            if result == 0 {
                unsafe {
                    PostQuitMessage(message.wParam as i32);
                }
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
        if prompt.font != 0 {
            DeleteObject(prompt.font);
        }
        EnableWindow(owner, 1);
        SetForegroundWindow(owner);
    }
    Ok(prompt.result.take())
}

fn file_dialog(owner: HWND, save: bool, legacy: bool) -> Result<Option<PathBuf>, String> {
    let mut filename = vec![0u16; 32768];
    let filter = if legacy {
        "Profiles and legacy INI\0*.ini\0All files\0*.*\0\0"
    } else {
        "Typing profiles\0*.typing.ini\0INI files\0*.ini\0All files\0*.*\0\0"
    };
    let filter: Vec<u16> = filter.encode_utf16().collect();
    let extension = wide("typing.ini");
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: filter.as_ptr(),
        lpstrFile: filename.as_mut_ptr(),
        nMaxFile: filename.len() as u32,
        lpstrDefExt: extension.as_ptr(),
        Flags: OFN_EXPLORER
            | OFN_NOCHANGEDIR
            | OFN_PATHMUSTEXIST
            | if save {
                OFN_OVERWRITEPROMPT
            } else {
                OFN_FILEMUSTEXIST
            },
        ..unsafe { std::mem::zeroed() }
    };
    let result = unsafe {
        if save {
            GetSaveFileNameW(&mut dialog)
        } else {
            GetOpenFileNameW(&mut dialog)
        }
    };
    if result == 0 {
        let code = unsafe { CommDlgExtendedError() };
        return if code == 0 {
            Ok(None)
        } else {
            Err(format!("File dialog failed: {code}"))
        };
    }
    use std::os::windows::ffi::OsStringExt;
    let length = filename
        .iter()
        .position(|c| *c == 0)
        .unwrap_or(filename.len());
    Ok(Some(PathBuf::from(std::ffi::OsString::from_wide(
        &filename[..length],
    ))))
}

struct ProfileUi {
    owner: HWND,
    status: HWND,
    selector: HWND,
    library: Library,
    available: Vec<AvailableProfile>,
    discovery_errors: Vec<String>,
    list: HWND,
    document: Option<Document>,
    visible: Vec<usize>,
    sort: Option<(usize, bool)>,
    controls: Vec<HWND>,
}

fn mapping_cell_text(list: HWND, row: usize, column: usize) -> String {
    let mut buffer = vec![0u16; 128];
    loop {
        let mut item = LVITEMW {
            iSubItem: column as i32,
            pszText: buffer.as_mut_ptr(),
            cchTextMax: buffer.len() as i32,
            ..unsafe { std::mem::zeroed() }
        };
        let length =
            unsafe { SendMessageW(list, LVM_GETITEMTEXTW, row, &mut item as *mut _ as isize) }
                as usize;
        if length < buffer.len() - 1 {
            return String::from_utf16_lossy(&buffer[..length]);
        }
        buffer.resize(buffer.len() * 2, 0);
    }
}

unsafe extern "system" fn compare_mapping_rank(first: LPARAM, second: LPARAM, _: LPARAM) -> i32 {
    match first.cmp(&second) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

pub fn initialize(owner: HWND, page: HWND) -> Result<Vec<HWND>, String> {
    let label = child(page, "STATIC", "Profile:", 0, 0)?;
    let selector = combo(page, PROFILE_LIST, &[])?;
    let set_active = child(page, "BUTTON", "Set as &active", WS_TABSTOP, SET_ACTIVE)?;
    let mut controls = vec![label, selector];
    for (label, id) in [
        ("Save", SAVE),
        ("Save As...", SAVE_AS),
        ("Rename...", RENAME),
        ("New...", NEW),
        ("Copy...", COPY),
        ("Delete", PROFILE_DELETE),
        ("Import...", IMPORT),
    ] {
        controls.push(child(
            page,
            "BUTTON",
            label,
            WS_TABSTOP | BS_PUSHBUTTON as u32,
            id,
        )?);
    }
    let status = text_block(page, "", 0)?;
    controls.push(status);
    controls.push(child(page, "STATIC", "Key mapping:", 0, 0)?);
    for (label, id) in [("Add", ADD), ("Edit", EDIT), ("Delete", DELETE)] {
        controls.push(child(
            page,
            "BUTTON",
            label,
            WS_TABSTOP | BS_PUSHBUTTON as u32,
            id,
        )?);
    }
    let list = list(
        page,
        &["Enabled", "Type", "Trigger", "Action", "Diagnostics"],
    )?;
    unsafe {
        SetWindowTextW(list, wide("Profile key mappings").as_ptr());
        SendMessageW(
            list,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            LVS_EX_CHECKBOXES as usize,
            LVS_EX_CHECKBOXES as isize,
        );
    }
    controls.push(list);
    controls.push(set_active);
    let document = SHARED
        .get()
        .unwrap()
        .state
        .lock()
        .unwrap()
        .profiles
        .active
        .as_ref()
        .map(|c| c.document.clone());
    let mut ui = ProfileUi {
        owner,
        status,
        selector,
        library: Library {
            directory: std::env::current_exe()
                .map_err(|e| e.to_string())?
                .parent()
                .unwrap()
                .join("profiles"),
        },
        available: Vec::new(),
        discovery_errors: Vec::new(),
        list,
        document,
        visible: Vec::new(),
        sort: None,
        controls: controls.clone(),
    };
    ui.refresh();
    PROFILES.with(|cell| *cell.borrow_mut() = Some(ui));
    Ok(controls)
}

type Positions = Vec<(HWND, i32, i32, i32, i32)>;
pub(super) fn fit_mapping_columns(list: HWND) {
    let mut bounds: RECT = unsafe { std::mem::zeroed() };
    let mut used = 0;
    unsafe {
        GetClientRect(list, &mut bounds);
        for column in 0..4 {
            SendMessageW(list, LVM_SETCOLUMNWIDTH, column, LVSCW_AUTOSIZE as isize);
            let content = SendMessageW(list, LVM_GETCOLUMNWIDTH, column, 0);
            SendMessageW(
                list,
                LVM_SETCOLUMNWIDTH,
                column,
                LVSCW_AUTOSIZE_USEHEADER as isize,
            );
            let header = SendMessageW(list, LVM_GETCOLUMNWIDTH, column, 0);
            let width = content.max(header);
            SendMessageW(list, LVM_SETCOLUMNWIDTH, column, width);
            used += width as i32;
        }
        SendMessageW(
            list,
            LVM_SETCOLUMNWIDTH,
            4,
            (bounds.right - used).max(scale(160, compatibility::window_dpi(list))) as isize,
        );
    }
}
pub(super) fn control_width(window: HWND, padding: i32, minimum: i32) -> i32 {
    let label = wide(&text(window));
    let mut size: SIZE = unsafe { std::mem::zeroed() };
    unsafe {
        let dc = GetDC(window);
        let font = SendMessageW(window, WM_GETFONT, 0, 0);
        let old = SelectObject(dc, font);
        GetTextExtentPoint32W(
            dc,
            label.as_ptr(),
            label.len().saturating_sub(1) as i32,
            &mut size,
        );
        SelectObject(dc, old);
        ReleaseDC(window, dc);
    }
    (size.cx + padding).max(minimum)
}

pub fn layout(page: &Page, width: i32, unit: i32, gap: i32) -> (Positions, i32) {
    let mut out = Vec::new();
    let mut y = gap;
    let height = unit + gap / 2;
    let label_width = control_width(page.controls[0], gap, unit * 3).min(width);
    out.push((page.controls[0], gap, y, label_width, height));
    if width < unit * 14 {
        y += height + gap;
    }
    let selector_x = if width < unit * 14 {
        gap
    } else {
        gap * 2 + label_width
    };
    let active = page.controls[15];
    let active_width = control_width(active, gap * 2, unit * 5).min(width);
    let available = (gap + width - selector_x).max(1);
    let same_row = available >= active_width + gap + unit * 6;
    let selector_width = (available - if same_row { active_width + gap } else { 0 })
        .max(1)
        .min(unit * 16);
    out.push((page.controls[1], selector_x, y, selector_width, unit * 12));
    let active_x = if same_row {
        selector_x + selector_width + gap
    } else {
        y += height + gap;
        gap
    };
    out.push((active, active_x, y, active_width, height));
    y += height + gap;
    let mut x = gap;
    for &control in &page.controls[2..9] {
        let button_width = control_width(control, gap * 2, unit * 3).min(width);
        if x > gap && x + button_width > width + gap {
            x = gap;
            y += height + gap;
        }
        out.push((control, x, y, button_width, height));
        x += button_width + gap;
    }
    y += height + gap;
    let status_height = text_block_height(page.controls[9], width);
    out.push((page.controls[9], gap, y, width, status_height));
    y += status_height + gap;
    x = gap;
    for &control in &page.controls[10..14] {
        let button_width = control_width(control, gap * 2, unit * 3).min(width);
        if x > gap && x + button_width > width + gap {
            x = gap;
            y += height + gap;
        }
        out.push((control, x, y, button_width, height));
        x += button_width + gap;
    }
    y += height + gap;
    let mut bounds: RECT = unsafe { std::mem::zeroed() };
    unsafe {
        GetClientRect(page.window, &mut bounds);
    }
    let list_height = (bounds.bottom - y - gap).max(unit * 9);
    let list = page.controls[14];
    out.push((list, gap, y, width, list_height));
    y += list_height + gap;
    (out, y)
}

impl ProfileUi {
    fn hotkey(&self) -> Option<Chord> {
        SHARED
            .get()
            .unwrap()
            .state
            .lock()
            .unwrap()
            .profiles
            .application
            .settings
            .global_hotkey
    }
    fn refresh_status(&self) {
        let state = SHARED.get().unwrap().state.lock().unwrap();
        let editor = self
            .document
            .as_ref()
            .and_then(|d| {
                Profile::parse(&d.text, state.profiles.application.settings.global_hotkey).ok()
            })
            .map(|p| p.name)
            .unwrap_or_else(|| "Choose a profile, New, or Import".into());
        let status = if let Some(document) = &self.document {
            if document.dirty() {
                "Unsaved changes"
            } else if state.profiles.is_active_revision(document) {
                "Saved; active"
            } else if state.profiles.is_current(document) {
                "Saved; changes not active — Set as active to apply"
            } else if document.path.is_none() {
                "Not saved"
            } else {
                "Saved; inactive — Set as active to use"
            }
        } else {
            ""
        };
        let presentation = format!(
            "Active: {}\r\nEditing: {editor} — {status}{}",
            state.profiles.name(),
            if state.profiles.pending.is_some() {
                " — Activating saved profile..."
            } else {
                ""
            }
        );
        if text(self.status) != presentation {
            set_text(self.status, &presentation);
            // Defer layout until profile/core borrows have been released.
            unsafe {
                PostMessageW(self.owner, WM_LAYOUT_CONTENT, BINDINGS_PAGE, 0);
            }
        }
        let busy = state.profiles.pending.is_some()
            || state.profiles.completed.is_some()
            || state.transitioning;
        unsafe {
            EnableWindow(self.list, i32::from(!busy && self.document.is_some()));
        }
        for &control in &self.controls {
            let id = unsafe { GetDlgCtrlID(control) } as usize;
            if (NEW..=SET_ACTIVE).contains(&id) {
                unsafe {
                    EnableWindow(control, i32::from(!busy));
                }
            }
        }
        for id in [SAVE, SAVE_AS, RENAME, COPY, ADD, EDIT, DELETE, SET_ACTIVE] {
            if let Some(&control) = self
                .controls
                .iter()
                .find(|&&c| unsafe { GetDlgCtrlID(c) } as usize == id)
            {
                unsafe {
                    EnableWindow(control, i32::from(!busy && self.document.is_some()));
                }
            }
        }
        let can_delete = self.available.iter().any(|p| {
            !state
                .profiles
                .active
                .as_ref()
                .and_then(|c| c.document.path.as_ref())
                .is_some_and(|active| same_file(active, &p.path))
        });
        if let Some(&control) = self
            .controls
            .iter()
            .find(|&&c| unsafe { GetDlgCtrlID(c) } as usize == PROFILE_DELETE)
        {
            unsafe {
                EnableWindow(control, i32::from(!busy && can_delete));
            }
        }
    }
    fn refresh_catalog(&mut self) {
        let catalog = self.library.scan(self.hotkey());
        self.available = catalog.profiles;
        self.discovery_errors = catalog.diagnostics;
        unsafe {
            SendMessageW(self.selector, CB_RESETCONTENT, 0, 0);
        }
        let mut selected = usize::MAX;
        for (index, profile) in self.available.iter().enumerate() {
            unsafe {
                SendMessageW(
                    self.selector,
                    CB_ADDSTRING,
                    0,
                    wide(&profile.name).as_ptr() as isize,
                );
            }
            if self
                .document
                .as_ref()
                .and_then(|d| d.path.as_ref())
                .is_some_and(|p| same_file(p, &profile.path))
            {
                selected = index;
            }
        }
        unsafe {
            SendMessageW(self.selector, CB_SETCURSEL, selected, 0);
        }
    }

    fn choose_name(
        &self,
        title: &str,
        initial: &str,
        except: Option<&Path>,
    ) -> Result<Option<String>, String> {
        let mut initial = initial.to_string();
        loop {
            let Some(name) = name_prompt(self.owner, title, &initial, None)? else {
                return Ok(None);
            };
            match self.library.check_name(&name, except, self.hotkey()) {
                Ok(name) => return Ok(Some(name)),
                Err(error) => {
                    alert(self.owner, &error);
                    initial = name;
                }
            }
        }
    }
    fn refresh(&mut self) {
        self.refresh_catalog();
        self.refresh_status();
        self.refresh_rows();
    }
    fn refresh_rows_preserving_selection(&mut self) {
        let mut selected = Vec::new();
        let mut index = usize::MAX;
        let focused = unsafe {
            SendMessageW(
                self.list,
                LVM_GETNEXTITEM,
                usize::MAX,
                LVNI_FOCUSED as isize,
            )
        };
        let focused = self.visible.get(focused as usize).copied();
        loop {
            let next =
                unsafe { SendMessageW(self.list, LVM_GETNEXTITEM, index, LVNI_SELECTED as isize) };
            if next < 0 {
                break;
            }
            index = next as usize;
            if let Some(line) = self.visible.get(index) {
                selected.push(*line);
            }
        }
        self.refresh_rows();
        for (index, line) in self.visible.iter().enumerate() {
            let state = if selected.contains(line) {
                LVIS_SELECTED
            } else {
                0
            } | if focused == Some(*line) {
                LVIS_FOCUSED
            } else {
                0
            };
            if state != 0 {
                let item = LVITEMW {
                    state,
                    stateMask: LVIS_SELECTED | LVIS_FOCUSED,
                    ..unsafe { std::mem::zeroed() }
                };
                unsafe {
                    SendMessageW(
                        self.list,
                        LVM_SETITEMSTATE,
                        index,
                        &item as *const _ as isize,
                    );
                    if focused == Some(*line) {
                        SendMessageW(self.list, LVM_ENSUREVISIBLE, index, 0);
                    }
                }
            }
        }
    }
    fn refresh_rows(&mut self) {
        self.visible.clear();
        unsafe {
            SendMessageW(self.list, LVM_DELETEALLITEMS, 0, 0);
        }
        if let Some(document) = &self.document {
            match Profile::parse(&document.text, self.hotkey()) {
                Ok(profile) => {
                    let mut messages: Vec<String> = profile
                        .warnings
                        .iter()
                        .filter(|warning| {
                            !profile
                                .entries
                                .iter()
                                .any(|entry| warning.starts_with(&format!("Line {}:", entry.line)))
                        })
                        .cloned()
                        .collect();
                    if let Some(error) = SHARED
                        .get()
                        .unwrap()
                        .state
                        .lock()
                        .unwrap()
                        .profiles
                        .error
                        .clone()
                    {
                        messages.push(error);
                    }
                    let mut rows = Vec::with_capacity(profile.entries.len());
                    for entry in profile.entries {
                        let mode = entry.command == typing_core::Command::ModeKey;
                        let scope = if mode {
                            "Mode key"
                        } else if entry.section == "modekeys" {
                            "Command mode"
                        } else {
                            "Hotkey"
                        };
                        let action = entry.command.label();
                        let warning = profile
                            .warnings
                            .iter()
                            .filter(|w| w.starts_with(&format!("Line {}:", entry.line)))
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("; ");
                        rows.push((
                            entry.line,
                            entry.enabled,
                            [String::new(), scope.into(), entry.input, action, warning],
                        ));
                    }
                    if let Some((column, ascending)) = self.sort {
                        if ascending {
                            rows.sort_by_cached_key(|(_, _, values)| values[column].to_lowercase());
                        } else {
                            rows.sort_by_cached_key(|(_, _, values)| {
                                std::cmp::Reverse(values[column].to_lowercase())
                            });
                        }
                    }
                    for (line, enabled, values) in rows {
                        row(self.list, &values, false);
                        self.set_check_state(self.visible.len(), if enabled { 2 } else { 1 });
                        self.visible.push(line);
                    }
                    for message in messages {
                        self.diagnostic(&message);
                    }
                }
                Err(error) => self.diagnostic(&error),
            }
        } else {
            let error = SHARED.get().unwrap().state.lock().unwrap().profiles.error.clone().unwrap_or_else(|| "Choose a profile, New, or Import to edit mappings. Legacy configuration can be copied with Import.".into());
            self.diagnostic(&error);
        }
        for error in &self.discovery_errors {
            self.diagnostic(error);
        }
        fit_mapping_columns(self.list);
    }
    fn sort_by_column(&mut self, column: usize) {
        let ascending = !matches!(self.sort, Some((previous, true)) if previous == column);
        let mut order: Vec<_> = self
            .visible
            .iter()
            .enumerate()
            .map(|(index, line)| {
                (
                    index,
                    *line,
                    mapping_cell_text(self.list, index, column).to_lowercase(),
                )
            })
            .collect();
        order.sort_by(|(_, first_line, first), (_, second_line, second)| {
            let comparison = first.cmp(second);
            (if ascending {
                comparison
            } else {
                comparison.reverse()
            })
            .then_with(|| first_line.cmp(second_line))
        });
        // Sort the native items themselves so selection, focus, check states,
        // scroll position, and column widths survive without an empty-table pass.
        let count = unsafe { SendMessageW(self.list, LVM_GETITEMCOUNT, 0, 0) } as usize;
        for (rank, index) in order
            .iter()
            .map(|(index, _, _)| *index)
            .chain(self.visible.len()..count)
            .enumerate()
        {
            let item = LVITEMW {
                mask: LVIF_PARAM,
                iItem: index as i32,
                lParam: rank as isize,
                ..unsafe { std::mem::zeroed() }
            };
            unsafe {
                SendMessageW(self.list, LVM_SETITEMW, 0, &item as *const _ as isize);
            }
        }
        if unsafe {
            SendMessageW(
                self.list,
                LVM_SORTITEMS,
                0,
                compare_mapping_rank as *const () as isize,
            )
        } == 0
        {
            return;
        }
        self.visible = order.into_iter().map(|(_, line, _)| line).collect();
        self.sort = Some((column, ascending));
        let header = unsafe { SendMessageW(self.list, LVM_GETHEADER, 0, 0) };
        for index in 1..4 {
            let mut item = HDITEMW {
                mask: HDI_FORMAT,
                ..unsafe { std::mem::zeroed() }
            };
            unsafe {
                if SendMessageW(header, HDM_GETITEMW, index, &mut item as *mut _ as isize) != 0 {
                    item.fmt &= !(HDF_SORTUP | HDF_SORTDOWN);
                    if index == column {
                        item.fmt |= if ascending { HDF_SORTUP } else { HDF_SORTDOWN };
                    }
                    SendMessageW(header, HDM_SETITEMW, index, &item as *const _ as isize);
                }
            }
        }
    }
    fn diagnostic(&self, message: &str) {
        let index = unsafe { SendMessageW(self.list, LVM_GETITEMCOUNT, 0, 0) } as usize;
        row(
            self.list,
            &[
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                message.into(),
            ],
            false,
        );
        self.set_check_state(index, 0);
    }
    fn set_check_state(&self, index: usize, state: u32) {
        let item = LVITEMW {
            state: state << 12,
            stateMask: LVIS_STATEIMAGEMASK,
            ..unsafe { std::mem::zeroed() }
        };
        unsafe {
            SendMessageW(
                self.list,
                LVM_SETITEMSTATE,
                index,
                &item as *const _ as isize,
            );
        }
    }
    fn selected(&self) -> Result<Entry, String> {
        let index = unsafe {
            SendMessageW(
                self.list,
                LVM_GETNEXTITEM,
                usize::MAX,
                LVNI_SELECTED as isize,
            )
        };
        let line = self
            .visible
            .get(index as usize)
            .ok_or("Select a mapping first")?;
        Profile::parse(
            &self
                .document
                .as_ref()
                .ok_or("Select or create a profile first")?
                .text,
            self.hotkey(),
        )?
        .entries
        .into_iter()
        .find(|e| e.line == *line)
        .ok_or_else(|| "Mapping selection is stale".into())
    }
    fn save(&mut self, save_as: bool) -> Result<bool, String> {
        let hotkey = self.hotkey();
        let document = self
            .document
            .as_mut()
            .ok_or("Select or create a profile first")?;
        Profile::parse(&document.text, hotkey)?;
        let chosen = save_as;
        let Some(path) = (if save_as {
            file_dialog(self.owner, true, false)?
        } else if let Some(path) = &document.path {
            Some(path.clone())
        } else {
            Some(
                self.library
                    .destination(&Profile::parse(&document.text, hotkey)?.name, hotkey)?,
            )
        }) else {
            return Ok(false);
        };
        let mut overwrite = chosen && path.exists();
        loop {
            match self.library.save(document, &path, overwrite, hotkey) {
                Ok(()) => return Ok(true),
                Err(error) if error.starts_with("External change") => {
                    match ask(
                        self.owner,
                        &format!(
                            "{error}\r\n\r\nYes: overwrite the file. No: choose Reload or Save As. Cancel: keep editing."
                        ),
                        MB_YESNOCANCEL | MB_ICONWARNING,
                    ) {
                        IDYES => overwrite = true,
                        IDNO => {
                            match ask(
                                self.owner,
                                "Reload the file and discard this working copy?\r\nYes: reload. No: Save As. Cancel: keep editing.",
                                MB_YESNOCANCEL | MB_ICONQUESTION,
                            ) {
                                IDYES => {
                                    *document = Document::open(&path, hotkey)?;
                                    return Ok(false);
                                }
                                IDNO => {
                                    let Some(other) = file_dialog(self.owner, true, false)? else {
                                        return Ok(false);
                                    };
                                    return self
                                        .library
                                        .save(document, &other, other.exists(), hotkey)
                                        .map(|()| true);
                                }
                                _ => return Ok(false),
                            }
                        }
                        _ => return Ok(false),
                    }
                }
                Err(error) => return Err(error),
            }
        }
    }
    fn abandon(&mut self) -> Result<bool, String> {
        if !self.document.as_ref().is_some_and(Document::dirty) {
            return Ok(true);
        }
        match ask(
            self.owner,
            "You have unsaved profile changes.\r\nSave them before continuing?\r\nYes: Save. No: Don't Save. Cancel: keep editing.",
            MB_YESNOCANCEL | MB_ICONQUESTION,
        ) {
            IDYES => self.save(false),
            IDNO => Ok(true),
            _ => Ok(false),
        }
    }
    fn activate(&mut self, document: Document, replace_editor: bool) -> Result<(), String> {
        let shared = SHARED.get().unwrap();
        let mut state = shared.state.lock().unwrap();
        if state.target == Lifecycle::Quit || state.paused || state.transitioning {
            return Err(
                "Wait for the current lifecycle operation to finish before activating a saved profile"
                    .into(),
            );
        }
        state.profiles.prepare(document, replace_editor)?;
        if let Some(candidate) = &state.profiles.pending
            && let Some(path) = &candidate.document.path
        {
            logging::info(
                "profile",
                format_args!("activation requested path={}", path.display()),
            );
        }
        let target = state.target;
        state.host.request(target);
        shared.generation.fetch_add(1, Ordering::SeqCst);
        shared.ready.notify_one();
        Ok(())
    }
    fn action(&mut self, id: usize) -> Result<(), String> {
        match id {
            NEW | COPY => {
                let source = if id == COPY {
                    Some(
                        self.document
                            .as_ref()
                            .ok_or("Create or select a profile first")?
                            .text
                            .clone(),
                    )
                } else {
                    None
                };
                let Some(name) = self.choose_name(
                    if id == COPY {
                        "Copy profile"
                    } else {
                        "New profile"
                    },
                    "",
                    None,
                )?
                else {
                    return Ok(());
                };
                if !self.abandon()? {
                    return Ok(());
                }
                let mut document = if let Some(source) = source {
                    Document::untitled(source)
                } else {
                    Document::new(&name)
                };
                document.rename(&name)?;
                document.baseline = document.text.clone();
                self.document = Some(document);
            }
            PROFILE_LIST => {
                let index = unsafe { SendMessageW(self.selector, CB_GETCURSEL, 0, 0) };
                let Some(profile) = self.available.get(index as usize) else {
                    return Ok(());
                };
                let path = profile.path.clone();
                Document::open(&path, self.hotkey())?;
                if !self.abandon()? {
                    return Ok(());
                }
                self.document = Some(Document::open(&path, self.hotkey())?);
            }
            SAVE | SAVE_AS => {
                self.save(id == SAVE_AS)?;
            }
            SET_ACTIVE if self.save(false)? => {
                self.activate(self.document.as_ref().unwrap().clone(), false)?;
            }
            IMPORT => {
                let Some(path) = file_dialog(self.owner, false, true)? else {
                    return Ok(());
                };
                let source =
                    std::fs::read_to_string(&path).map_err(|e| format!("Read import: {e}"))?;
                let explicit = path
                    .file_name()
                    .is_some_and(|s| s.to_string_lossy().to_lowercase().ends_with(".typing.ini"))
                    || source
                        .trim_start_matches('\u{feff}')
                        .lines()
                        .any(|line| line.trim().eq_ignore_ascii_case("[profile]"));
                let imported =
                    import_document(&source, explicit, "Imported profile", self.hotkey())?;
                let initial = if explicit {
                    Profile::parse(&source, self.hotkey())?.name
                } else {
                    "Imported profile".into()
                };
                let Some(name) = self.choose_name("Import profile", &initial, None)? else {
                    return Ok(());
                };
                if self.abandon()? {
                    let mut document = imported;
                    document.rename(&name)?;
                    document.baseline = document.text.clone();
                    self.document = Some(document);
                }
            }
            PROFILE_DELETE => {
                let active = SHARED
                    .get()
                    .unwrap()
                    .state
                    .lock()
                    .unwrap()
                    .profiles
                    .active
                    .as_ref()
                    .and_then(|c| c.document.path.clone());
                let inactive: Vec<_> = self
                    .available
                    .iter()
                    .filter(|p| {
                        !active
                            .as_ref()
                            .is_some_and(|active| same_file(active, &p.path))
                    })
                    .cloned()
                    .collect();
                if inactive.is_empty() {
                    return Err(
                        "Set another profile as active before deleting the active profile".into(),
                    );
                }
                let names: Vec<_> = inactive.iter().map(|p| p.name.as_str()).collect();
                let initial = self
                    .document
                    .as_ref()
                    .and_then(|d| d.path.as_ref())
                    .and_then(|path| inactive.iter().find(|p| same_file(&p.path, path)))
                    .map(|p| p.name.as_str())
                    .unwrap_or(names[0]);
                let Some(name) = name_prompt(self.owner, "Delete profile", initial, Some(&names))?
                else {
                    return Ok(());
                };
                let matching: Vec<_> = inactive.iter().filter(|p| p.name == name).collect();
                if matching.len() != 1 {
                    return Err(
                        "Rename duplicate profiles to distinguish them before deletion".into(),
                    );
                }
                let path = matching[0].path.clone();
                let mut document = Document::open(&path, self.hotkey())?;
                if ask(
                    self.owner,
                    &format!("Delete profile {name}? This removes its saved file."),
                    MB_YESNO | MB_ICONWARNING,
                ) != IDYES
                {
                    return Ok(());
                }
                if self
                    .document
                    .as_ref()
                    .and_then(|d| d.path.as_ref())
                    .is_some_and(|p| same_file(p, &path))
                {
                    if !self.abandon()? {
                        return Ok(());
                    }
                    if let Some(editor) = self
                        .document
                        .as_ref()
                        .filter(|d| d.path.as_ref().is_some_and(|p| same_file(p, &path)))
                    {
                        document = editor.clone();
                        document.discard();
                    }
                }
                let active = SHARED
                    .get()
                    .unwrap()
                    .state
                    .lock()
                    .unwrap()
                    .profiles
                    .active
                    .as_ref()
                    .and_then(|c| c.document.path.clone());
                self.library.remove(&document, active.as_deref())?;
                if self
                    .document
                    .as_ref()
                    .and_then(|d| d.path.as_ref())
                    .is_some_and(|p| same_file(p, &path))
                {
                    self.document = None;
                }
            }
            ADD | EDIT => {
                if self.document.is_none() {
                    return Err("Choose a profile, New, or Import first".into());
                }
                let old = if id == EDIT {
                    Some(self.selected()?)
                } else {
                    None
                };
                if let Some((scope, input, action, enabled)) =
                    edit_dialog(self.owner, old.as_ref())?
                {
                    let hotkey = self.hotkey();
                    let mut candidate = self.document.as_ref().unwrap().clone();
                    let line = candidate.edit(
                        old.as_ref().map(|e| e.line),
                        &scope,
                        &input,
                        &action,
                        hotkey,
                    )?;
                    candidate.set_enabled(line, enabled, hotkey)?;
                    self.document = Some(candidate);
                }
            }
            DELETE => {
                let entry = self.selected()?;
                self.document.as_mut().unwrap().delete(entry.line);
            }
            RENAME => {
                let document = self
                    .document
                    .as_ref()
                    .ok_or("Create or select a profile first")?;
                let initial = Profile::parse(&document.text, self.hotkey())?.name;
                if let Some(name) =
                    self.choose_name("Rename profile", &initial, document.path.as_deref())?
                {
                    self.document.as_mut().unwrap().rename(&name)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

pub fn confirm_exit() -> bool {
    confirm_leave(false)
}

pub fn confirm_close() -> bool {
    confirm_leave(true)
}

fn confirm_leave(closing_window: bool) -> bool {
    let model = PROFILES.with(|cell| cell.borrow_mut().take());
    let Some(mut ui) = model else {
        return false;
    };
    let result = match ui.abandon() {
        Ok(result) => result,
        Err(error) => {
            alert(ui.owner, &error);
            false
        }
    };
    if result
        && closing_window
        && let Some(document) = &mut ui.document
    {
        document.discard();
    }
    ui.refresh();
    PROFILES.with(|cell| *cell.borrow_mut() = Some(ui));
    result
}

pub fn profile_status() {
    let model = PROFILES.with(|cell| cell.borrow_mut().take());
    let Some(mut ui) = model else {
        return;
    };
    let completed = SHARED
        .get()
        .unwrap()
        .state
        .lock()
        .unwrap()
        .profiles
        .completed
        .take();
    if let Some(candidate) = completed {
        if candidate.replace_editor {
            ui.document = Some(candidate.document.clone());
        }
        let shared = SHARED.get().unwrap();
        let mut app = shared.state.lock().unwrap().profiles.application.clone();
        let path = candidate.document.path.as_ref().unwrap();
        let mut errors = Vec::new();
        if let Err(error) = app.remember(path) {
            errors.push(format!(
                "Profile is active for this run; remembering selection failed: {error}"
            ));
        } else {
            shared.state.lock().unwrap().profiles.application = app;
        }
        if std::fs::read(path).ok().as_deref() != candidate.document.fingerprint.as_deref() {
            errors.push("The profile source changed after validation. The exact validated snapshot remains active.".into());
        }
        if !errors.is_empty() {
            for error in &errors {
                logging::warning("profile", format_args!("{error}"));
            }
            alert(ui.owner, &errors.join("\r\n"));
        }
    }
    ui.refresh();
    PROFILES.with(|cell| *cell.borrow_mut() = Some(ui));
}

pub fn show_profile_recovery() {
    UI.with(|cell| {
        if let Some(ui) = cell.borrow_mut().as_mut() {
            ui.selected = BINDINGS_PAGE;
            unsafe {
                SendMessageW(ui.tabs, TCM_SETCURSEL, BINDINGS_PAGE, 0);
            }
            SHARED
                .get()
                .unwrap()
                .editor_active
                .store(true, Ordering::SeqCst);
            ui.show();
        }
    });
}

pub fn message(_: HWND, message: u32, w: WPARAM, l: LPARAM) -> Option<LRESULT> {
    if message == WM_REFRESH_MAPPINGS {
        PROFILES.with(|cell| {
            if let Ok(mut model) = cell.try_borrow_mut()
                && let Some(ui) = model.as_mut()
            {
                ui.refresh_rows_preserving_selection();
                ui.refresh_status();
            }
        });
        return Some(0);
    }
    if message == WM_NOTIFY && l != 0 {
        let header = unsafe { &*(l as *const NMHDR) };
        if header.code == LVN_ITEMCHANGING || header.code == LVN_ITEMCHANGED {
            return PROFILES.with(|cell| {
                let Ok(mut model) = cell.try_borrow_mut() else {
                    return None;
                };
                let ui = model.as_mut()?;
                if header.hwndFrom != ui.list {
                    return None;
                }
                let change = unsafe { &*(l as *const NMLISTVIEW) };
                let previous = change.uOldState & LVIS_STATEIMAGEMASK;
                let next = change.uNewState & LVIS_STATEIMAGEMASK;
                if change.uChanged & LVIF_STATE == 0 || previous == next {
                    return Some(0);
                }
                let line = ui.visible.get(change.iItem as usize).copied();
                if header.code == LVN_ITEMCHANGING {
                    let state = SHARED.get().unwrap().state.lock().unwrap();
                    return Some(isize::from(
                        line.is_none()
                            || state.profiles.pending.is_some()
                            || state.profiles.completed.is_some()
                            || state.transitioning,
                    ));
                }
                if let Some(line) = line {
                    let hotkey = ui.hotkey();
                    let enabled = next == 2 << 12;
                    if let Some(document) = ui.document.as_mut() {
                        if let Err(error) = document.set_enabled(line, enabled, hotkey) {
                            ui.set_check_state(change.iItem as usize, previous >> 12);
                            alert(ui.owner, &error);
                        }
                        // Refresh after the native control finishes this state
                        // change, so list rebuilding cannot invalidate its item.
                        unsafe {
                            PostMessageW(ui.owner, WM_REFRESH_MAPPINGS, 0, 0);
                        }
                    }
                }
                Some(0)
            });
        }
        if header.code == LVN_COLUMNCLICK {
            return PROFILES.with(|cell| {
                let Ok(mut model) = cell.try_borrow_mut() else {
                    return None;
                };
                let ui = model.as_mut()?;
                if header.hwndFrom != ui.list {
                    return None;
                }
                let click = unsafe { &*(l as *const NMLISTVIEW) };
                if (1..4).contains(&click.iSubItem) {
                    ui.sort_by_column(click.iSubItem as usize);
                }
                Some(0)
            });
        }
        return None;
    }
    if message == WM_CLOSE && !confirm_close() {
        return Some(0);
    }
    if message == WM_TIMER && w == TIMER {
        release_record_hook();
        if SHARED
            .get()
            .unwrap()
            .state
            .lock()
            .unwrap()
            .profiles
            .completed
            .is_some()
        {
            profile_status();
        }
        PROFILES.with(|cell| {
            if let Ok(model) = cell.try_borrow()
                && let Some(ui) = model.as_ref()
            {
                ui.refresh_status();
            }
        });
        return None;
    }
    if message != WM_COMMAND {
        return None;
    }
    let id = w & 0xffff;
    if ![
        NEW,
        PROFILE_LIST,
        SAVE,
        SAVE_AS,
        COPY,
        PROFILE_DELETE,
        IMPORT,
        ADD,
        EDIT,
        DELETE,
        RENAME,
        SET_ACTIVE,
    ]
    .contains(&id)
    {
        return None;
    }
    if id == PROFILE_LIST && (w >> 16) as u32 != CBN_SELCHANGE && (w >> 16) as u32 != CBN_DROPDOWN {
        return Some(0);
    }
    let model = PROFILES.with(|cell| cell.borrow_mut().take());
    if let Some(mut ui) = model {
        let busy = {
            let state = SHARED.get().unwrap().state.lock().unwrap();
            state.profiles.pending.is_some() || state.profiles.completed.is_some()
        };
        if !busy {
            let result = if id == PROFILE_LIST && (w >> 16) as u32 == CBN_DROPDOWN {
                ui.refresh_catalog();
                Ok(())
            } else {
                ui.action(id)
            };
            if let Err(error) = result {
                alert(ui.owner, &error);
            }
            ui.refresh();
        }
        PROFILES.with(|cell| *cell.borrow_mut() = Some(ui));
    }
    Some(0)
}

const ACTIONS: &[(&str, &str)] = &[
    ("Output shortcut", ""),
    ("Move left", "Left"),
    ("Move right", "Right"),
    ("Move up", "Up"),
    ("Move down", "Down"),
    ("Page up", "PageUp"),
    ("Page down", "PageDown"),
    ("Home", "Home"),
    ("End", "End"),
    ("Previous word", "Control+Left"),
    ("Next word", "Control+Right"),
    ("Step count", "step"),
    ("Inverse movement", "undo"),
    ("Toggle selection", "shift_onoff"),
    ("Exit command mode", "exit"),
    ("Date", "date"),
    ("Time", "time"),
    ("Enable", "enable"),
    ("Disable", "disable"),
    ("Switch window 1", "switch_win1"),
    ("Switch window 2", "switch_win2"),
    ("Delete", "Delete"),
    ("Backspace", "Back"),
    ("Caps Lock", "CapsLock"),
];

struct EditDialog {
    window: HWND,
    input: HWND,
    scope: HWND,
    enabled: HWND,
    action: HWND,
    parameter: HWND,
    status: HWND,
    controls: Vec<HWND>,
    target: HWND,
    result: Option<(String, String, String, bool)>,
    done: bool,
    font: HFONT,
    scroll: i32,
    total: i32,
    pending_record: Option<(usize, u64)>,
}

unsafe extern "system" fn recording_hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    let handles = if code == HC_ACTION as i32 {
        let event = unsafe { &*(l as *const KBDLLHOOKSTRUCT) };
        let shared = SHARED.get().unwrap();
        let mut state = shared.state.lock().unwrap();
        if state.recorder.is_some()
            && unsafe { GetForegroundWindow() }
                != shared.record_owner.load(Ordering::SeqCst) as HWND
        {
            state.recorder = None;
            shared.record_owner.store(0, Ordering::SeqCst);
        }
        state.recorder.is_some()
            || normalize_native(event).is_some_and(|key| state.recorded_downs.contains(&key))
    } else {
        false
    };
    if handles {
        unsafe { keyboard_hook(code, w, l) }
    } else {
        unsafe { CallNextHookEx(0, code, w, l) }
    }
}

fn release_record_hook() {
    let removable = {
        let state = SHARED.get().unwrap().state.lock().unwrap();
        state.recorder.is_none() && state.recorded_downs.is_empty()
    };
    if removable {
        RECORD_HOOK.with(|hook| {
            if hook.get() != 0 && unsafe { UnhookWindowsHookEx(hook.get()) } != 0 {
                hook.set(0);
            }
        });
    }
}

fn stop_recording() {
    let shared = SHARED.get().unwrap();
    shared.record_owner.store(0, Ordering::SeqCst);
    shared.state.lock().unwrap().recorder = None;
    DIALOG.with(|cell| {
        if let Some(dialog) = cell.borrow_mut().as_mut() {
            dialog.target = 0;
            dialog.pending_record = None;
        }
    });
}

fn complete_recording() -> Result<(), String> {
    let shared = SHARED.get().unwrap();
    let chord = {
        let state = shared.state.lock().unwrap();
        let recorder = state.recorder.as_ref().ok_or("Recording has stopped")?;
        if !recorder.released() {
            return Err("Release all recorded keys first".into());
        }
        if let Some(error) = &recorder.error {
            return Err(error.clone());
        }
        recorder
            .chord
            .clone()
            .ok_or("No valid shortcut was recorded")?
    };
    let (target, parameter, action, status) = DIALOG.with(|cell| {
        let holder = cell.borrow();
        let dialog = holder.as_ref().unwrap();
        (
            dialog.target,
            dialog.parameter,
            dialog.action,
            dialog.status,
        )
    });
    if target == 0 {
        return Err("No recording field is selected".into());
    }
    set_text(target, &chord);
    if target == parameter {
        unsafe {
            SendMessageW(action, CB_SETCURSEL, 0, 0);
            EnableWindow(parameter, 1);
        }
    }
    stop_recording();
    set_text(
        status,
        "Shortcut recorded. Confirm to update the working copy.",
    );
    Ok(())
}

fn dialog_action(id: usize) -> Result<(), String> {
    let snapshot = DIALOG.with(|cell| {
        cell.borrow().as_ref().map(|d| {
            (
                d.window,
                d.input,
                d.scope,
                d.action,
                d.parameter,
                d.status,
                d.target,
            )
        })
    });
    let Some((_window, input, scope, action, parameter, status, target)) = snapshot else {
        return Ok(());
    };
    match id {
        RECORD_INPUT | RECORD_OUTPUT => {
            if target != 0 {
                stop_recording();
                set_text(status, "Recording stopped; the field was not changed.");
                return Ok(());
            }
            let shared = SHARED.get().unwrap();
            if shared.record_owner.load(Ordering::SeqCst) != 0 {
                return Err("Stop the current recording first".into());
            }
            if !sample_baseline(|code| {
                (unsafe { GetAsyncKeyState(code as i32) }) as u16 & 0x8000 != 0
            })
            .is_empty()
            {
                return Err("Release all keyboard keys before recording".into());
            }
            let target = shared.state.lock().unwrap().host.target;
            shared.lifecycle(target);
            let generation = shared.generation.load(Ordering::SeqCst);
            DIALOG.with(|cell| {
                let mut holder = cell.borrow_mut();
                let dialog = holder.as_mut().unwrap();
                dialog.pending_record = Some((id, generation));
                dialog.target = if id == RECORD_INPUT { input } else { parameter };
            });
            set_text(
                status,
                "Preparing recording: finishing output cleanup. Release all keys and wait for Recording.",
            );
        }
        ENTRY_CANCEL => {
            stop_recording();
            DIALOG.with(|cell| cell.borrow_mut().as_mut().unwrap().done = true);
        }
        ENTRY_ACCEPT => {
            if target != 0 {
                return Err("Finish or stop recording first".into());
            }
            let scope = unsafe { SendMessageW(scope, CB_GETCURSEL, 0, 0) };
            let input_text = text(input);
            let selected = unsafe { SendMessageW(action, CB_GETCURSEL, 0, 0) } as usize;
            let value = if scope == 0 {
                "mode".into()
            } else {
                let (_, value) = ACTIONS.get(selected).ok_or("Choose an action")?;
                match *value {
                    "" => text(parameter),
                    "step" => format!("step{}", text(parameter)),
                    other => other.into(),
                }
            };
            typing_core::validate_binding(input_text.trim(), value.trim())?;
            let result = (
                if scope == 2 { "hotkeys" } else { "modekeys" }.into(),
                input_text,
                value,
                DIALOG.with(|cell| unsafe {
                    SendMessageW(cell.borrow().as_ref().unwrap().enabled, BM_GETCHECK, 0, 0)
                        == BST_CHECKED as isize
                }),
            );
            DIALOG.with(|cell| {
                let mut holder = cell.borrow_mut();
                let dialog = holder.as_mut().unwrap();
                dialog.result = Some(result);
                dialog.done = true;
            });
        }
        ACTION => {
            let index = unsafe { SendMessageW(action, CB_GETCURSEL, 0, 0) } as usize;
            unsafe {
                EnableWindow(parameter, i32::from(index == 0 || index == 11));
            }
        }
        _ => {}
    }
    Ok(())
}

fn prepare_recording() -> Result<(), String> {
    let pending = DIALOG.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|dialog| dialog.pending_record)
    });
    if let Some((_, generation)) = pending {
        let shared = SHARED.get().unwrap();
        let (ready, failed) = {
            let state = shared.state.lock().unwrap();
            (
                state.desired.is_none() && !state.transitioning && !state.processing,
                state.failed,
            )
        };
        if generation != shared.generation.load(Ordering::SeqCst) || failed {
            stop_recording();
            return Err("Recording cancelled by a lifecycle change or cleanup failure".into());
        }
        if ready
            && sample_baseline(|code| {
                (unsafe { GetAsyncKeyState(code as i32) }) as u16 & 0x8000 != 0
            })
            .is_empty()
        {
            let no_hook = RECORD_HOOK.with(|hook| hook.get() == 0);
            if no_hook {
                let temporary_hook = unsafe {
                    SetWindowsHookExW(
                        WH_KEYBOARD_LL,
                        Some(recording_hook),
                        GetModuleHandleW(std::ptr::null()),
                        0,
                    )
                };
                if temporary_hook == 0 {
                    return Err(last_error("start shortcut recording"));
                }
                RECORD_HOOK.with(|hook| hook.set(temporary_hook));
            }
            let (window, status) = DIALOG.with(|cell| {
                let mut holder = cell.borrow_mut();
                let dialog = holder.as_mut().unwrap();
                dialog.pending_record = None;
                (dialog.window, dialog.status)
            });
            shared.state.lock().unwrap().recorder = Some(Recorder::default());
            shared.record_owner.store(window as u64, Ordering::SeqCst);
            set_text(
                status,
                "Recording. Press and release a shortcut to fill the field, or click Stop recording to keep its current value.",
            );
        }
    }
    Ok(())
}

fn layout_dialog(window: HWND) {
    DIALOG.with(|cell| {
        let Ok(mut holder) = cell.try_borrow_mut() else {
            return;
        };
        let Some(dialog) = holder.as_mut() else {
            return;
        };
        let dpi = compatibility::window_dpi(window);
        let mut font: LOGFONTW = unsafe { std::mem::zeroed() };
        unsafe {
            GetObjectW(
                dialog.font,
                std::mem::size_of::<LOGFONTW>() as i32,
                (&mut font as *mut LOGFONTW).cast(),
            );
        }
        let unit = font.lfHeight.abs() + scale(5, dpi);
        let gap = ui_gap(dpi);
        let mut rect: RECT = unsafe { std::mem::zeroed() };
        unsafe {
            GetClientRect(window, &mut rect);
        }
        let width = (rect.right - gap * 2).max(1);
        let mut y = gap;
        let height = unit + gap / 2;
        let label = control_width(dialog.controls[0], gap, unit * 5).min(width / 3);
        let record_width = control_width(dialog.controls[2], gap * 2, unit * 6).min(width / 3);
        let mut positions = Vec::new();
        for (label_window, field, button) in [
            (dialog.controls[11], dialog.scope, None),
            (dialog.controls[0], dialog.input, Some(dialog.controls[2])),
            (dialog.controls[12], dialog.action, None),
            (
                dialog.controls[5],
                dialog.parameter,
                Some(dialog.controls[7]),
            ),
        ] {
            if field == dialog.input {
                positions.push((dialog.enabled, gap, y, width, height));
                y += height + gap;
            }
            if width < unit * 20 {
                positions.push((label_window, gap, y, width, unit));
                y += unit + gap / 2;
                positions.push((field, gap, y, width, height));
                y += height + gap;
                if let Some(button) = button {
                    positions.push((button, gap, y, record_width.max(width / 2), height));
                    y += height + gap;
                }
            } else {
                positions.push((label_window, gap, y, label, height));
                positions.push((
                    field,
                    gap * 2 + label,
                    y,
                    width
                        - label
                        - gap
                        - if button.is_some() {
                            record_width + gap
                        } else {
                            0
                        },
                    height,
                ));
                if let Some(button) = button {
                    positions.push((button, gap + width - record_width, y, record_width, height));
                }
                y += height + gap;
            }
        }
        let recording = dialog.target != 0;
        for (button, field, title) in [
            (dialog.controls[2], dialog.input, "Record trigger"),
            (dialog.controls[7], dialog.parameter, "Record output"),
        ] {
            set_text(
                button,
                if dialog.target == field {
                    "Stop recording"
                } else {
                    title
                },
            );
            unsafe {
                EnableWindow(button, i32::from(!recording || dialog.target == field));
            }
        }
        unsafe {
            EnableWindow(dialog.controls[9], i32::from(!recording));
        }
        let status_height = text_block_height(dialog.status, width);
        positions.push((dialog.status, gap, y, width, status_height));
        y += status_height + gap;
        let confirm = control_width(dialog.controls[9], gap * 2, unit * 6).min(width / 2 - gap);
        let cancel = control_width(dialog.controls[10], gap * 2, unit * 6).min(width / 2 - gap);
        positions.push((
            dialog.controls[9],
            gap + width - confirm - cancel - gap,
            y,
            confirm,
            height,
        ));
        positions.push((dialog.controls[10], gap + width - cancel, y, cancel, height));
        y += height + gap;
        dialog.total = y;
        dialog.scroll = dialog.scroll.clamp(0, (dialog.total - rect.bottom).max(0));
        let info = SCROLLINFO {
            cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
            fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
            nMin: 0,
            nMax: dialog.total - 1,
            nPage: rect.bottom.max(1) as u32,
            nPos: dialog.scroll,
            nTrackPos: 0,
        };
        unsafe {
            SetScrollInfo(window, SB_VERT, &info, 1);
        }
        for (control, x, y, width, height) in positions {
            let dropdown = control == dialog.action || control == dialog.scope;
            unsafe {
                MoveWindow(
                    control,
                    x,
                    y - dialog.scroll,
                    width,
                    if dropdown { unit * 10 } else { height },
                    1,
                );
            }
        }
    });
}

fn dialog_font(window: HWND) -> HFONT {
    let dpi = compatibility::window_dpi(window);
    let height = ((11 * dpi * text_scale() + 3600) / 7200) as i32;
    unsafe {
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
    }
}

fn refresh_dialog_font(window: HWND) {
    let font = dialog_font(window);
    if font == 0 {
        return;
    }
    let state = DIALOG.with(|cell| {
        let mut holder = cell.borrow_mut();
        holder.as_mut().map(|dialog| {
            let old = dialog.font;
            dialog.font = font;
            (old, dialog.controls.clone())
        })
    });
    if let Some((old, controls)) = state {
        for control in controls {
            unsafe {
                SendMessageW(control, WM_SETFONT, font as usize, 1);
            }
        }
        unsafe {
            DeleteObject(old);
        }
    } else {
        unsafe {
            DeleteObject(font);
        }
    }
}

unsafe extern "system" fn dialog_proc(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match message {
        WM_COMMAND => {
            let id = match w & 0xffff {
                1 => ENTRY_ACCEPT,
                2 => ENTRY_CANCEL,
                id => id,
            };
            let result = dialog_action(id);
            if let Err(error) = result {
                alert(window, &error);
            }
            layout_dialog(window);
            0
        }
        WM_CLOSE => {
            let _ = dialog_action(ENTRY_CANCEL);
            0
        }
        WM_ACTIVATE if w & 0xffff == WA_INACTIVE as usize => {
            stop_recording();
            layout_dialog(window);
            0
        }
        WM_SIZE => {
            layout_dialog(window);
            0
        }
        WM_DPICHANGED | WM_SETTINGCHANGE => {
            refresh_dialog_font(window);
            if message == WM_DPICHANGED && l != 0 {
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
            layout_dialog(window);
            0
        }
        WM_VSCROLL | WM_MOUSEWHEEL => {
            DIALOG.with(|cell| {
                if let Some(dialog) = cell.borrow_mut().as_mut() {
                    let step = scale(60, compatibility::window_dpi(window));
                    let mut info: SCROLLINFO = unsafe { std::mem::zeroed() };
                    info.cbSize = std::mem::size_of::<SCROLLINFO>() as u32;
                    info.fMask = SIF_ALL;
                    unsafe {
                        GetScrollInfo(window, SB_VERT, &mut info);
                    }
                    dialog.scroll = if message == WM_MOUSEWHEEL {
                        dialog.scroll - ((w >> 16) as u16 as i16 as i32) * step / 120
                    } else {
                        match w & 0xffff {
                            0 => dialog.scroll - step,
                            1 => dialog.scroll + step,
                            2 => dialog.scroll - info.nPage as i32,
                            3 => dialog.scroll + info.nPage as i32,
                            4 | 5 => info.nTrackPos,
                            _ => dialog.scroll,
                        }
                    };
                }
            });
            layout_dialog(window);
            0
        }
        WM_TIMER => {
            if let Err(error) = prepare_recording() {
                stop_recording();
                DIALOG.with(|cell| {
                    if let Some(dialog) = cell.borrow().as_ref() {
                        set_text(dialog.status, &error);
                    }
                });
            }
            let shared = SHARED.get().unwrap();
            let completed = {
                let state = shared.state.lock().unwrap();
                state.recorder.as_ref().is_some_and(|recorder| {
                    recorder.released() && (recorder.chord.is_some() || recorder.error.is_some())
                })
            };
            if shared.record_owner.load(Ordering::SeqCst) != 0
                && unsafe { GetForegroundWindow() } != window
            {
                stop_recording();
            }
            if completed
                && shared.record_owner.load(Ordering::SeqCst) != 0
                && let Err(error) = complete_recording()
            {
                stop_recording();
                DIALOG.with(|cell| {
                    if let Some(dialog) = cell.borrow().as_ref() {
                        set_text(dialog.status, &error);
                    }
                });
            }
            let cancelled = shared.state.lock().unwrap().recorder.is_none()
                && DIALOG.with(|cell| {
                    cell.borrow()
                        .as_ref()
                        .is_some_and(|dialog| dialog.target != 0 && dialog.pending_record.is_none())
                });
            if cancelled {
                stop_recording();
            }
            layout_dialog(window);
            release_record_hook();
            0
        }
        _ => unsafe { DefWindowProcW(window, message, w, l) },
    }
}

fn edit_dialog(
    owner: HWND,
    old: Option<&Entry>,
) -> Result<Option<(String, String, String, bool)>, String> {
    static CLASS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let registered = CLASS.get_or_init(|| {
        let name = wide("TypingBindingEditor");
        let class = WNDCLASSW {
            lpfnWndProc: Some(dialog_proc),
            hInstance: unsafe { GetModuleHandleW(std::ptr::null()) },
            hCursor: unsafe { LoadCursorW(0, IDC_ARROW) },
            hbrBackground: (COLOR_WINDOW + 1) as isize,
            lpszClassName: name.as_ptr(),
            ..unsafe { std::mem::zeroed() }
        };
        unsafe { RegisterClassW(&class) != 0 }
    });
    if !registered {
        return Err(last_error("register mapping editor"));
    }
    let dpi = compatibility::window_dpi(owner);
    let text_unit = ((11 * 96 * text_scale() + 3600) / 7200) as i32 + 5;
    let initial_width = scale((text_unit * 28 + 32).max(720), dpi);
    let initial_height = scale((text_unit * 10 + 80).max(440), dpi);
    let window = unsafe {
        CreateWindowExW(
            WS_EX_CONTROLPARENT,
            wide("TypingBindingEditor").as_ptr(),
            wide("Edit mapping").as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VSCROLL,
            50,
            30,
            initial_width,
            initial_height,
            owner,
            0,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        )
    };
    if window == 0 {
        return Err(last_error("create mapping editor"));
    }
    // Native accessibility associates a field with its preceding static label.
    // Creation order also defines keyboard navigation; vector order is layout.
    let scope_label = child(window, "STATIC", "Type", 0, 0)?;
    let scope = combo(window, 0, &["Mode key", "Command mode", "Hotkey"])?;
    let enabled = child(
        window,
        "BUTTON",
        "Enabled",
        WS_TABSTOP | BS_AUTOCHECKBOX as u32,
        ENTRY_ENABLED,
    )?;
    unsafe {
        SendMessageW(
            enabled,
            BM_SETCHECK,
            if old.is_none_or(|entry| entry.enabled) {
                BST_CHECKED
            } else {
                BST_UNCHECKED
            } as usize,
            0,
        );
    }
    let input_label = child(window, "STATIC", "Trigger", 0, 0)?;
    let input = child(
        window,
        "EDIT",
        old.map(|e| e.input.as_str()).unwrap_or(""),
        WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
        0,
    )?;
    let record_input = child(window, "BUTTON", "Record trigger", WS_TABSTOP, RECORD_INPUT)?;
    let action_names = ACTIONS.iter().map(|(name, _)| *name).collect::<Vec<_>>();
    let action_label = child(window, "STATIC", "Action", 0, 0)?;
    let action = combo(window, ACTION, &action_names)?;
    let parameter_label = child(window, "STATIC", "Output / step", 0, 0)?;
    let parameter = child(
        window,
        "EDIT",
        "",
        WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
        0,
    )?;
    let status = text_block(
        window,
        "Trigger is a key or shortcut. Output shortcut and Step count use the parameter field. Inverse movement reverses recorded movement; it is not document undo.",
        0,
    )?;
    let mut controls = vec![
        input_label,
        input,
        record_input,
        scope,
        action,
        parameter_label,
        parameter,
    ];
    controls.push(child(
        window,
        "BUTTON",
        "Record output",
        WS_TABSTOP,
        RECORD_OUTPUT,
    )?);
    controls.push(status);
    for (label, id) in [("Confirm", ENTRY_ACCEPT), ("Cancel", ENTRY_CANCEL)] {
        controls.push(child(window, "BUTTON", label, WS_TABSTOP, id)?);
    }
    controls.push(scope_label);
    controls.push(action_label);
    controls.push(enabled);
    let mut selected_action = 0;
    let mut selected_scope = 1;
    if let Some(entry) = old {
        selected_scope = if entry.command == typing_core::Command::ModeKey {
            0
        } else if entry.section == "hotkeys" {
            2
        } else {
            1
        };
        if let Some(index) = ACTIONS.iter().position(|(_, value)| *value == entry.action) {
            selected_action = index;
        } else if let typing_core::Command::Step(count) = entry.command {
            selected_action = 11;
            set_text(parameter, &count.to_string());
        } else {
            set_text(parameter, &entry.action);
        }
    }
    unsafe {
        SendMessageW(scope, CB_SETCURSEL, selected_scope, 0);
        SendMessageW(action, CB_SETCURSEL, selected_action, 0);
        EnableWindow(
            parameter,
            i32::from(selected_action == 0 || selected_action == 11),
        );
    }
    let font = dialog_font(window);
    if font == 0 {
        unsafe {
            DestroyWindow(window);
        }
        return Err(last_error("create mapping editor font"));
    }
    for &control in &controls {
        unsafe {
            SendMessageW(control, WM_SETFONT, font as usize, 1);
        }
    }
    DIALOG.with(|cell| {
        *cell.borrow_mut() = Some(EditDialog {
            window,
            input,
            scope,
            enabled,
            action,
            parameter,
            status,
            controls,
            target: 0,
            result: None,
            done: false,
            font,
            scroll: 0,
            total: 0,
            pending_record: None,
        })
    });
    layout_dialog(window);
    unsafe {
        let mut monitor: MONITORINFO = std::mem::zeroed();
        monitor.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(
            MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        ) != 0
        {
            let area = monitor.rcWork;
            let width = initial_width.min(area.right - area.left);
            let height = initial_height.min(area.bottom - area.top);
            SetWindowPos(
                window,
                0,
                area.left + (area.right - area.left - width) / 2,
                area.top + (area.bottom - area.top - height) / 2,
                width,
                height,
                SWP_NOZORDER,
            );
        }
    }
    let shared = SHARED.get().unwrap();
    let previous = shared.editor_active.swap(true, Ordering::SeqCst);
    unsafe {
        EnableWindow(owner, 0);
        ShowWindow(window, SW_SHOW);
        SetForegroundWindow(window);
        SetFocus(input);
        SetTimer(window, WM_DIALOG_TICK, 50, None);
    }
    let mut message: MSG = unsafe { std::mem::zeroed() };
    while !DIALOG.with(|cell| cell.borrow().as_ref().unwrap().done) {
        let result = unsafe { GetMessageW(&mut message, 0, 0, 0) };
        if result <= 0 {
            if result == 0 {
                unsafe {
                    PostQuitMessage(message.wParam as i32);
                }
            }
            break;
        }
        if unsafe { IsDialogMessageW(window, &message) } == 0 {
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        if message.message == WM_KEYDOWN && message.wParam == VK_TAB as usize {
            let focus = unsafe { GetFocus() };
            if unsafe { IsChild(window, focus) } != 0 {
                let mut bounds: RECT = unsafe { std::mem::zeroed() };
                let mut client: RECT = unsafe { std::mem::zeroed() };
                unsafe {
                    GetWindowRect(focus, &mut bounds);
                    MapWindowPoints(0, window, (&mut bounds as *mut RECT).cast(), 2);
                    GetClientRect(window, &mut client);
                }
                let shift = if bounds.top < 0 {
                    bounds.top
                } else if bounds.bottom > client.bottom {
                    bounds.bottom - client.bottom
                } else {
                    0
                };
                if shift != 0 {
                    DIALOG.with(|cell| {
                        if let Some(dialog) = cell.borrow_mut().as_mut() {
                            dialog.scroll += shift;
                        }
                    });
                    layout_dialog(window);
                }
            }
        }
    }
    stop_recording();
    shared.editor_active.store(previous, Ordering::SeqCst);
    let dialog = DIALOG.with(|cell| cell.borrow_mut().take().unwrap());
    unsafe {
        KillTimer(window, WM_DIALOG_TICK);
        EnableWindow(owner, 1);
        DestroyWindow(window);
        DeleteObject(dialog.font);
        SetForegroundWindow(owner);
    }
    release_record_hook();
    Ok(dialog.result)
}
