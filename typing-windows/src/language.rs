//! Windows resource, locale, preference and caption adapters. Shared policy is
//! in typing-host; no keyboard state or lifecycle is owned here.
use super::*;
use std::{cell::RefCell, collections::BTreeMap, io::Write, path::Path};
use typing_host::languages::{Platform, Service, Snapshot};

pub struct Windows {
    directory: PathBuf,
    preference: Option<PathBuf>,
}
impl Windows {
    fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            preference: std::env::var_os("LOCALAPPDATA")
                .filter(|p| !p.is_empty())
                .map(|p| PathBuf::from(p).join("Typing/language.txt")),
        }
    }
}
impl Platform for Windows {
    fn language_directory(&self) -> &Path {
        &self.directory
    }
    fn preferred_languages(&self) -> Vec<String> {
        let mut count = 0;
        let mut length = 0;
        unsafe {
            GetUserPreferredUILanguages(
                MUI_LANGUAGE_NAME,
                &mut count,
                std::ptr::null_mut(),
                &mut length,
            );
        }
        if length == 0 {
            return Vec::new();
        }
        let mut names = vec![0u16; length as usize];
        if unsafe {
            GetUserPreferredUILanguages(
                MUI_LANGUAGE_NAME,
                &mut count,
                names.as_mut_ptr(),
                &mut length,
            )
        } == 0
        {
            return Vec::new();
        }
        names
            .split(|c| *c == 0)
            .filter(|s| !s.is_empty())
            .map(String::from_utf16_lossy)
            .collect()
    }
    fn read_selection(&self) -> Result<Option<String>, Message> {
        let path = self.preference.as_ref().ok_or_else(|| {
            msg!(
                "language.preference_location",
                "Cannot locate the per-user language preference."
            )
        })?;
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(msg!(
                "language.preference_read",
                "Cannot read language preference {path}: {error}",
                path = path.display(),
                error = error
            )),
        }
    }
    fn write_selection(&self, selection: &str) -> Result<(), Message> {
        let path = self.preference.as_ref().ok_or_else(|| {
            msg!(
                "language.preference_location",
                "Cannot locate the per-user language preference."
            )
        })?;
        let parent = path.parent().unwrap();
        let operation = || -> Result<(), std::io::Error> {
            std::fs::create_dir_all(parent)?;
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let temp = parent.join(format!(
                "language-{}-{}.tmp",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp)?;
            let result = (|| {
                writeln!(file, "{selection}")?;
                file.sync_all()?;
                drop(file);
                std::fs::rename(&temp, path)
            })();
            if result.is_err() {
                let _ = std::fs::remove_file(&temp);
            }
            result
        };
        operation().map_err(|error| {
            msg!(
                "language.preference_write",
                "Cannot save language preference {path}: {error}",
                path = path.display(),
                error = error
            )
        })
    }
}
struct State {
    platform: Windows,
    service: Service,
}
thread_local! {
    #[cfg_attr(target_vendor = "win7", allow(clippy::missing_const_for_thread_local))]
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    #[cfg_attr(target_vendor = "win7", allow(clippy::missing_const_for_thread_local))]
    static CAPTIONS: RefCell<BTreeMap<HWND, Message>> = const { RefCell::new(BTreeMap::new()) };
    #[cfg_attr(target_vendor = "win7", allow(clippy::missing_const_for_thread_local))]
    static COLUMNS: RefCell<BTreeMap<HWND, Vec<Message>>> = const { RefCell::new(BTreeMap::new()) };
    #[cfg_attr(target_vendor = "win7", allow(clippy::missing_const_for_thread_local))]
    static ROOT: std::cell::Cell<HWND> = const { std::cell::Cell::new(0) };
}
pub fn initialize(directory: PathBuf) {
    let platform = Windows::new(directory.join("languages"));
    let service = Service::initialize(&platform);
    STATE.with(|cell| *cell.borrow_mut() = Some(State { platform, service }));
}
pub fn log_diagnostics() {
    STATE.with(|cell| {
        if let Some(state) = cell.borrow().as_ref() {
            for diagnostic in state
                .service
                .notice
                .iter()
                .chain(&state.service.catalog.diagnostics)
            {
                logging::warning("languages", format_args!("{diagnostic}"));
            }
        }
    });
}
pub fn root(window: HWND) {
    ROOT.with(|root| root.set(window));
}
pub fn snapshot() -> Arc<Snapshot> {
    STATE.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|s| s.service.snapshot.clone())
            .unwrap_or_else(|| Arc::new(Snapshot::english(0)))
    })
}
pub fn render(message: &Message) -> String {
    if let Message::Literal(value) = message {
        return value.clone();
    }
    snapshot().render(message)
}
pub fn remember(window: HWND, message: &Message) {
    if !ROOT.with(|root| root.get() != 0 && unsafe { GetAncestor(window, GA_ROOT) } == root.get()) {
        return;
    }
    CAPTIONS.with(|cell| {
        let mut captions = cell.borrow_mut();
        if matches!(message, Message::Literal(_)) {
            captions.remove(&window);
        } else {
            captions.insert(window, message.clone());
        }
    });
}
pub fn set_caption(window: HWND, message: &Message) {
    remember(window, message);
    unsafe {
        SetWindowTextW(window, wide(&window_text(window, message)).as_ptr());
    }
}
pub fn columns(window: HWND, messages: Vec<Message>) {
    COLUMNS.with(|cell| {
        cell.borrow_mut().insert(window, messages);
    });
}
pub fn refresh_captions() {
    let captions = CAPTIONS.with(|cell| cell.borrow().clone());
    for (window, message) in captions {
        if unsafe { IsWindow(window) } != 0 {
            unsafe {
                SetWindowTextW(window, wide(&window_text(window, &message)).as_ptr());
            }
        }
    }
    let columns = COLUMNS.with(|cell| cell.borrow().clone());
    for (window, messages) in columns {
        if unsafe { IsWindow(window) } == 0 {
            continue;
        }
        for (index, message) in messages.iter().enumerate() {
            let mut text = wide(message);
            let column = windows_sys::Win32::UI::Controls::LVCOLUMNW {
                mask: windows_sys::Win32::UI::Controls::LVCF_TEXT,
                pszText: text.as_mut_ptr(),
                ..unsafe { std::mem::zeroed() }
            };
            unsafe {
                SendMessageW(
                    window,
                    windows_sys::Win32::UI::Controls::LVM_SETCOLUMNW,
                    index,
                    &column as *const _ as isize,
                );
            }
        }
    }
}
pub fn choices(rescan: bool) -> (Vec<String>, usize, Option<Message>) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        let state = state.as_mut().unwrap();
        if rescan {
            state.service.rescan(&state.platform);
            for diagnostic in &state.service.catalog.diagnostics {
                logging::warning("languages", format_args!("{diagnostic}"));
            }
        }
        let service = &state.service;
        let current = &service.snapshot;
        let mut labels = vec![if service.requested.eq_ignore_ascii_case("system") {
            current.render(&msg!(
                "language.system",
                "System default — {name}",
                name = current.name
            ))
        } else {
            current.render(&msg!("language.system_default", "System default"))
        }];
        for choice in &service.catalog.choices {
            let duplicate = service
                .catalog
                .choices
                .iter()
                .filter(|c| c.name == choice.name)
                .count()
                > 1;
            labels.push(if duplicate {
                format!("{} ({})", choice.name, choice.id)
            } else {
                choice.name.clone()
            });
        }
        let selected = if service.requested.eq_ignore_ascii_case("system") {
            0
        } else if let Some(index) = service
            .catalog
            .choices
            .iter()
            .position(|c| c.id.eq_ignore_ascii_case(&service.requested))
        {
            index + 1
        } else {
            labels.push(current.render(&msg!(
                "language.retained",
                "{name} — saved profile {id} unavailable",
                name = current.name,
                id = service.requested
            )));
            labels.len() - 1
        };
        let messages = service
            .notice
            .iter()
            .cloned()
            .chain(service.catalog.diagnostics.iter().cloned())
            .collect::<Vec<_>>();
        (
            labels,
            selected,
            if messages.is_empty() {
                None
            } else {
                Some(Message::join(messages, "\n\n"))
            },
        )
    })
}
pub fn select(index: usize) -> Result<(), Message> {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        let state = state.as_mut().unwrap();
        let requested = if index == 0 {
            "system".to_string()
        } else {
            state
                .service
                .catalog
                .choices
                .get(index - 1)
                .ok_or_else(|| {
                    msg!(
                        "language.choice_changed",
                        "Available languages changed. Reopen Settings and try again."
                    )
                })?
                .id
                .clone()
        };
        let result = state
            .service
            .prepare(&requested, &state.platform)
            .and_then(|p| state.service.commit(p, &state.platform));
        if let Err(error) = &result {
            state.service.notice = Some(error.clone());
        }
        result
    })
}

const ACCESS_KEYS: &[(&str, char, &str)] = &[
    ("ui.open_user_guide", 'G', "Guide"),
    ("ui.about", 'A', "About"),
    ("ui.show_keyboard_events", 'k', "keyboard"),
    ("ui.clear", 'C', "Clear"),
    ("ui.start_with_windows", 'W', "Windows"),
    ("ui.set_as_active", 'a', "active"),
    ("ui.close", 'C', "Close"),
];
pub fn control_text(class: &str, message: &Message) -> String {
    let text = render(message);
    if class.eq_ignore_ascii_case("EDIT") {
        return text
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\n', "\r\n");
    }
    if !class.eq_ignore_ascii_case("BUTTON") && !class.eq_ignore_ascii_case("STATIC") {
        return text;
    }
    let mut text = text.replace('&', "&&");
    if let Message::Template { id, .. } = message
        && let Some((_, key, word)) = ACCESS_KEYS.iter().find(|(known, _, _)| known == id)
    {
        if let Some(index) = text.find(word).or_else(|| {
            text.char_indices()
                .find(|(_, c)| c.eq_ignore_ascii_case(key))
                .map(|(index, _)| index)
        }) {
            text.insert(index, '&');
        } else {
            text.push_str(&format!(" (&{key})"));
        }
    }
    text
}
pub fn window_text(window: HWND, message: &Message) -> String {
    let mut name = [0u16; 64];
    let len = unsafe { GetClassNameW(window, name.as_mut_ptr(), name.len() as i32) };
    control_text(
        &String::from_utf16_lossy(&name[..len.max(0) as usize]),
        message,
    )
}
