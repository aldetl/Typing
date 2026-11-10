#![cfg_attr(not(test), windows_subsystem = "windows")]
#[cfg(not(all(
    target_os = "windows",
    any(target_arch = "x86_64", target_arch = "x86")
)))]
compile_error!("The Windows host requires Windows x86 or x64.");

mod compatibility;
mod desktop;
mod inputsim;
mod logging;
mod output;
mod startup;
mod ui;

use inputsim::{Character, DOWN, Event, InputSim, UP};
use output::{Backend, Output};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use typing_core::{
    Action, Chord, Config, Direction, HostControl, Key, KeyEvent, LocalDateTime, Modifiers, Outcome,
};
use typing_host::{DiagnosticOutput, Lifecycle, Session, Work as HostWork};
use windows_sys::Win32::{
    Foundation::*,
    Globalization::*,
    System::{LibraryLoader::GetModuleHandleW, SystemInformation::GetLocalTime},
    UI::{Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
};

const WM_RPC: u32 = WM_APP + 1;
const WM_STATUS: u32 = WM_APP + 2;
const WM_TRAY: u32 = WM_APP + 3;
const ENABLE: usize = 1001;
const DISABLE: usize = 1002;
const QUIT: usize = 1003;
const SHOW: usize = 1004;
const TOGGLE: usize = 1005;
const WM_DESKTOP: u32 = WM_APP + 5;
const MODIFIER_CODES: [u16; 8] = [160, 161, 162, 163, 164, 165, 91, 92];
fn modifier_keys() -> [Key; 8] {
    MODIFIER_CODES.map(|code| Key::from_code(code).unwrap())
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
fn modifier(key: Key) -> bool {
    MODIFIER_CODES.contains(&key.code())
}

struct State {
    host: Session,
    profiles: typing_host::profiles::Activation,
    recorder: Option<typing_host::profiles::Recorder>,
    recorded_downs: BTreeSet<Key>,
    hotkey_registered: bool,
    physical: BTreeSet<Key>,
    visible: BTreeSet<Key>,
    effective: BTreeSet<Key>,
    command_modifiers: BTreeSet<Key>,
    modifier_events: BTreeMap<Key, Event>,
    physical_version: u64,
    left_shift_pending: bool,
    caps_on: bool,
    caps_down: bool,
    caps_restore: Option<bool>,
    hook: HHOOK,
}

impl std::ops::Deref for State {
    type Target = Session;
    fn deref(&self) -> &Session {
        &self.host
    }
}
impl std::ops::DerefMut for State {
    fn deref_mut(&mut self) -> &mut Session {
        &mut self.host
    }
}

#[derive(Clone, Debug)]
struct DiagnosticEvent {
    key: u16,
    down: bool,
    shift: bool,
    alt: bool,
    control: bool,
}

struct Shared {
    state: Mutex<State>,
    ready: Condvar,
    generation: AtomicU64,
    started: Instant,
    tag: usize,
    main_window: AtomicU64,
    hook_window: AtomicU64,
    diagnostic_events: std::sync::mpsc::SyncSender<DiagnosticEvent>,
    editor_active: AtomicBool,
    record_owner: AtomicU64,
}

impl Shared {
    fn lifecycle(&self, next: Lifecycle) {
        let mut state = self.state.lock().unwrap();
        if next == Lifecycle::Quit {
            state
                .profiles
                .cancel("Profile activation cancelled by Quit");
        }
        if matches!(
            next,
            Lifecycle::Enable | Lifecycle::Disable | Lifecycle::Quit
        ) {
            state.recorder = None;
            self.record_owner.store(0, Ordering::SeqCst);
        }
        if next == Lifecycle::Enable && !state.profiles.valid && state.profiles.pending.is_none() {
            drop(state);
            self.status(
                "Open or create a valid profile before enabling keyboard commands",
                false,
                true,
            );
            return;
        }
        if !state.host.request(next) {
            return;
        }
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.ready.notify_one();
        drop(state);
        logging::info("lifecycle", format_args!("requested {next:?}"));
    }
    fn pause(&self, paused: bool) {
        self.recover(paused, false);
    }
    fn recover(&self, paused: bool, new_cycle: bool) {
        let mut state = self.state.lock().unwrap();
        let changed = state.host.recover(paused, new_cycle);
        if paused || changed {
            state
                .profiles
                .cancel("Profile activation cancelled by desktop pause");
            state.recorder = None;
            self.record_owner.store(0, Ordering::SeqCst);
        }
        if changed {
            self.generation.fetch_add(1, Ordering::SeqCst);
            self.ready.notify_one();
            logging::info(
                "lifecycle",
                format_args!("recovery paused={paused} new_cycle={new_cycle}"),
            );
        }
    }
    fn status(&self, text: &str, quit: bool, failed: bool) {
        if failed {
            logging::error("lifecycle", format_args!("{text}"));
        } else {
            logging::info("lifecycle", format_args!("{text}"));
        }
        let notice = Box::into_raw(Box::new((text.to_string(), quit, failed)));
        if unsafe {
            PostMessageW(
                self.main_window.load(Ordering::SeqCst) as HWND,
                WM_STATUS,
                0,
                notice as LPARAM,
            )
        } == 0
        {
            unsafe {
                drop(Box::from_raw(notice));
            }
        }
    }
}

static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();

type Work = Box<dyn FnOnce() -> Result<(), String> + Send>;
struct Call {
    work: Option<Work>,
    result: Option<Result<(), String>>,
}

fn on_window_thread(
    window: HWND,
    work: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    let mut call = Call {
        work: Some(Box::new(work)),
        result: None,
    };
    unsafe {
        SendMessageW(window, WM_RPC, 0, &mut call as *mut Call as LPARAM);
    }
    call.result
        .unwrap_or_else(|| Err("native window dispatcher unavailable".into()))
}

unsafe fn dispatch_call(l: LPARAM) -> LRESULT {
    let call = unsafe { &mut *(l as *mut Call) };
    call.result = Some(call.work.take().unwrap()());
    0
}

unsafe extern "system" fn hook_window(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if message == WM_RPC {
        return unsafe { dispatch_call(l) };
    }
    unsafe { DefWindowProcW(window, message, w, l) }
}

fn normalize_native(event: &KBDLLHOOKSTRUCT) -> Option<Key> {
    let key = match event.vkCode as u16 {
        VK_SHIFT => {
            if event.scanCode == 0x36 {
                VK_RSHIFT
            } else {
                VK_LSHIFT
            }
        }
        VK_CONTROL => {
            if event.flags & LLKHF_EXTENDED != 0 {
                VK_RCONTROL
            } else {
                VK_LCONTROL
            }
        }
        VK_MENU => {
            if event.flags & LLKHF_EXTENDED != 0 {
                VK_RMENU
            } else {
                VK_LMENU
            }
        }
        key => key,
    };
    Key::from_code(key)
}

fn hotkey_modifiers(chord: Chord) -> u32 {
    let mut flags = 0;
    for (generic, left, right, native) in [
        (
            Modifiers::CONTROL,
            Modifiers::LEFT_CONTROL,
            Modifiers::RIGHT_CONTROL,
            MOD_CONTROL,
        ),
        (
            Modifiers::ALT,
            Modifiers::LEFT_ALT,
            Modifiers::RIGHT_ALT,
            MOD_ALT,
        ),
        (
            Modifiers::SHIFT,
            Modifiers::LEFT_SHIFT,
            Modifiers::RIGHT_SHIFT,
            MOD_SHIFT,
        ),
    ] {
        if chord.modifiers.contains(generic)
            || chord.modifiers.contains(left)
            || chord.modifiers.contains(right)
        {
            flags |= native;
        }
    }
    flags
}

fn held_hotkey(state: &State, key: Key) -> bool {
    let Some(chord) = state.core.config().global_hotkey else {
        return false;
    };
    if key != chord.key {
        return false;
    }
    let held = &state.physical;
    let mut flags = 0;
    if held.contains(&Key::LEFT_CONTROL) || held.contains(&Key::RIGHT_CONTROL) {
        flags |= MOD_CONTROL;
    }
    if held.contains(&Key::LEFT_ALT) || held.contains(&Key::RIGHT_ALT) {
        flags |= MOD_ALT;
    }
    if held.contains(&Key::LEFT_SHIFT) || held.contains(&Key::RIGHT_SHIFT) {
        flags |= MOD_SHIFT;
    }
    flags == hotkey_modifiers(chord)
}

fn visible_caps(state: &mut State, key: Key, down: bool, own: bool) {
    if key != Key::CAPS_LOCK {
        return;
    }
    if down && !state.caps_down {
        state.caps_on = !state.caps_on;
        if !own && let Some(target) = state.caps_restore.as_mut() {
            *target = !*target;
        }
    }
    state.caps_down = down;
}
unsafe extern "system" fn keyboard_hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code != HC_ACTION as i32 {
        return unsafe { CallNextHookEx(0, code, w, l) };
    }
    let event = unsafe { &*(l as *const KBDLLHOOKSTRUCT) };
    let Some(key) = normalize_native(event) else {
        return unsafe { CallNextHookEx(0, code, w, l) };
    };
    let shared = SHARED.get().unwrap();
    let down = event.flags & LLKHF_UP == 0;
    let mut state = shared.state.lock().unwrap();
    if event.flags & LLKHF_INJECTED != 0 && event.dwExtraInfo == shared.tag {
        visible_caps(&mut state, key, down, true);
        if modifier(key) {
            if down {
                state.effective.insert(key);
                if state.processing {
                    state.command_modifiers.insert(key);
                }
            } else {
                state.effective.remove(&key);
                state.command_modifiers.remove(&key);
            }
        }
        drop(state);
        return unsafe { CallNextHookEx(0, code, w, l) };
    }
    let repeated = state.physical.contains(&key);
    state.physical_version += 1;
    if down {
        state.physical.insert(key);
    } else {
        state.physical.remove(&key);
    }
    let was_visible = state.visible.contains(&key);
    let mut consume = false;
    let foreground = unsafe { GetForegroundWindow() };
    let owner = shared.record_owner.load(Ordering::SeqCst) as HWND;
    let record = owner != 0 && foreground == owner && state.recorder.is_some();
    let mut foreground_process = 0;
    unsafe {
        GetWindowThreadProcessId(foreground, &mut foreground_process);
    }
    let editing = (shared.editor_active.load(Ordering::Relaxed) || owner != 0)
        && foreground_process
            == unsafe { windows_sys::Win32::System::Threading::GetCurrentProcessId() };
    if owner != 0 && !record {
        state.recorder = None;
        shared.record_owner.store(0, Ordering::SeqCst);
    }
    let input = KeyEvent {
        key,
        direction: if down { Direction::Down } else { Direction::Up },
    };
    if record {
        state.core.observe_unbound(input);
        state.recorder.as_mut().unwrap().input(key, down);
        if down {
            state.recorded_downs.insert(key);
            consume = true;
        } else {
            consume = state.recorded_downs.remove(&key);
        }
    } else if state.recorded_downs.contains(&key) {
        state.core.observe_unbound(input);
        if !down {
            state.recorded_downs.remove(&key);
        }
        consume = true;
    } else if down && held_hotkey(&state, key) {
        if !repeated {
            unsafe {
                PostMessageW(
                    shared.main_window.load(Ordering::SeqCst) as HWND,
                    WM_HOTKEY,
                    1,
                    0,
                );
            }
        }
        consume = true;
    } else if editing {
        state.core.observe_unbound(input);
    } else if state.admission {
        let (handled, admitted) = state.host.input(
            KeyEvent {
                key,
                direction: if down { Direction::Down } else { Direction::Up },
            },
            shared.started.elapsed(),
        );
        consume = handled;
        if admitted {
            shared.ready.notify_one();
        }
    }
    if modifier(key) {
        if down && !consume {
            state.visible.insert(key);
        }
        if !down {
            state.visible.remove(&key);
        }
        if state.processing {
            let hardware_left_release = !down
                && key == Key::LEFT_SHIFT
                && event.flags & LLKHF_INJECTED == 0
                && was_visible
                && !state.command_modifiers.contains(&key)
                && !state.left_shift_pending;
            consume = !hardware_left_release;
        } else if !down && !was_visible {
            consume = true;
        }
        if !consume {
            if down {
                state.effective.insert(key);
            } else {
                state.effective.remove(&key);
            }
        }
    }
    if !consume {
        visible_caps(&mut state, key, down, false);
    }
    let _ = shared.diagnostic_events.try_send(DiagnosticEvent {
        key: key.code(),
        down,
        shift: state.physical.contains(&Key::LEFT_SHIFT)
            || state.physical.contains(&Key::RIGHT_SHIFT),
        alt: state.physical.contains(&Key::LEFT_ALT) || state.physical.contains(&Key::RIGHT_ALT),
        control: state.physical.contains(&Key::LEFT_CONTROL)
            || state.physical.contains(&Key::RIGHT_CONTROL),
    });
    drop(state);
    if consume {
        1
    } else {
        unsafe { CallNextHookEx(0, code, w, l) }
    }
}

#[derive(Clone)]
struct WindowsBackend {
    window: HWND,
    api: Arc<InputSim>,
    shared: Arc<Shared>,
}

impl Backend for WindowsBackend {
    fn key(&mut self, key: Key) -> Result<Event, String> {
        self.api.key(key.code())
    }
    fn character(&mut self, character: char) -> Result<Character, String> {
        self.api.character(character)
    }
    fn submit(&mut self, event: Event, direction: u32) -> Result<(), String> {
        let shared = &self.shared;
        let (version, logical) = {
            let mut state = shared.state.lock().unwrap();
            let logical = state
                .modifier_events
                .iter()
                .find(|(_, value)| **value == event)
                .map(|(key, _)| *key);
            state.left_shift_pending = direction == DOWN && logical == Some(Key::LEFT_SHIFT);
            (state.physical_version, logical)
        };
        // Native submission runs on the worker while capture keeps pumping.
        let result = self.api.submit(event, direction, shared.tag);
        let mut state = shared.state.lock().unwrap();
        state.left_shift_pending = false;
        if result.is_ok()
            && version == state.physical_version
            && let Some(key) = logical
        {
            if direction == DOWN {
                state.effective.insert(key);
            } else {
                state.effective.remove(&key);
            }
        }
        result
    }
}

fn begin_output(output: &mut Output<WindowsBackend>) -> Result<(), String> {
    output.begin()?;
    let backend = &mut output.backend;
    let mut identities = BTreeMap::new();
    for key in modifier_keys() {
        identities.insert(key, backend.key(key)?);
    }
    let suppress = {
        let mut state = backend.shared.state.lock().unwrap();
        state.processing = true;
        state.modifier_events = identities.clone();
        state.command_modifiers.clear();
        state.effective.iter().copied().collect::<Vec<_>>()
    };
    for key in suppress {
        if let Some(event) = identities.get(&key) {
            backend.submit(*event, UP)?;
        }
    }
    Ok(())
}

fn reconcile(output: &mut Output<WindowsBackend>) -> Result<(), String> {
    let mut failed = BTreeSet::new();
    let blocked = output.held_events().to_vec();
    let mut errors = Vec::new();
    loop {
        let next = {
            let state = output.backend.shared.state.lock().unwrap();
            modifier_keys().into_iter().find_map(|key| {
                let desired = state.physical.contains(&key) && state.visible.contains(&key);
                (desired != state.effective.contains(&key)
                    && !failed.contains(&key)
                    && !state
                        .modifier_events
                        .get(&key)
                        .is_some_and(|event| blocked.contains(event)))
                .then_some((key, desired, state.modifier_events.get(&key).copied()))
            })
        };
        let Some((key, down, event)) = next else {
            break;
        };
        let result = event
            .ok_or_else(|| "modifier identity unavailable".to_string())
            .and_then(|event| output.backend.submit(event, if down { DOWN } else { UP }));
        if let Err(error) = result {
            failed.insert(key);
            errors.push(error);
        }
    }
    let mut state = output.backend.shared.state.lock().unwrap();
    state.processing = false;
    state.command_modifiers.clear();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join(", "))
    }
}

fn set_caps(output: &mut Output<WindowsBackend>, wanted: bool) -> Result<(), String> {
    let current = output.backend.shared.state.lock().unwrap().caps_on;
    if current == wanted {
        return Ok(());
    }
    let event = output.backend.key(Key::CAPS_LOCK)?;
    if output.held_events().contains(&event) {
        return Err("Caps Lock release remains outstanding".into());
    }
    output.down(event)?;
    output.up(event)?;
    if output.backend.shared.state.lock().unwrap().caps_on == wanted {
        Ok(())
    } else {
        Err("Caps Lock changed during character setup".into())
    }
}

fn restore_caps(output: &mut Output<WindowsBackend>) -> Result<(), String> {
    loop {
        let target = output.backend.shared.state.lock().unwrap().caps_restore;
        let Some(target) = target else {
            return Ok(());
        };
        set_caps(output, target)?;
        let mut state = output.backend.shared.state.lock().unwrap();
        if state.caps_restore == Some(state.caps_on) {
            state.caps_restore = None;
            return Ok(());
        }
    }
}

fn finish_output(output: &mut Output<WindowsBackend>, retry: bool) -> Result<(), String> {
    let mut errors = Vec::new();
    for result in [
        output.cleanup(retry),
        reconcile(output),
        restore_caps(output),
    ] {
        if let Err(error) = result {
            errors.push(error);
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}
fn cancelled(shared: &Shared, generation: u64) -> bool {
    shared.generation.load(Ordering::SeqCst) != generation
}

fn tap(output: &mut Output<WindowsBackend>, event: Event, generation: u64) -> Result<(), String> {
    if cancelled(&output.backend.shared, generation) {
        return Err("cancelled".into());
    }
    output.down(event)?;
    output.up(event)
}

fn execute(
    output: &mut Output<WindowsBackend>,
    action: &Action,
    generation: u64,
) -> Result<(), String> {
    if cancelled(&output.backend.shared, generation) {
        return Err("cancelled".into());
    }
    begin_output(output)?;
    match action {
        Action::Keys { keys, count, delay } => {
            let identities = keys
                .iter()
                .map(|key| output.backend.key(*key))
                .collect::<Result<Vec<_>, _>>()?;
            let Some((last, modifiers)) = identities.split_last() else {
                return Ok(());
            };
            for event in modifiers {
                if cancelled(&output.backend.shared, generation) {
                    return Err("cancelled".into());
                }
                output.down(*event)?;
            }
            for _ in 0..*count {
                tap(output, *last, generation)?;
                if !delay.is_zero() {
                    let state = output.backend.shared.state.lock().unwrap();
                    let _ = output
                        .backend
                        .shared
                        .ready
                        .wait_timeout_while(state, *delay, |_| {
                            !cancelled(&output.backend.shared, generation)
                        })
                        .unwrap();
                }
            }
        }
        Action::Text(text) => {
            {
                let mut state = output.backend.shared.state.lock().unwrap();
                state.caps_restore = Some(state.caps_on);
            }
            for character in text.chars() {
                if cancelled(&output.backend.shared, generation) {
                    return Err("cancelled".into());
                }
                let plan = output.backend.character(character)?;
                set_caps(output, false)?;
                for event in &plan.modifiers[..plan.modifier_count as usize] {
                    if cancelled(&output.backend.shared, generation) {
                        return Err("cancelled".into());
                    }
                    output.down(*event)?;
                }
                tap(output, plan.key, generation)?;
                for event in plan.modifiers[..plan.modifier_count as usize].iter().rev() {
                    output.up(*event)?;
                }
            }
        }
        Action::None | Action::HostControl(_) => {}
    }
    Ok(())
}

fn locale_designator(kind: u32) -> String {
    let mut buffer = [0u16; 64];
    let count = unsafe {
        GetLocaleInfoEx(
            std::ptr::null(),
            kind,
            buffer.as_mut_ptr(),
            buffer.len() as i32,
        )
    };
    if count > 0 {
        String::from_utf16_lossy(&buffer[..count as usize - 1])
    } else {
        String::new()
    }
}

fn sample_baseline(is_down: impl Fn(u16) -> bool) -> BTreeSet<Key> {
    (1..=254)
        .filter(|code| !matches!(code, 1 | 2 | 4 | 5 | 6))
        .filter(|code| is_down(*code))
        .filter_map(Key::from_code)
        .collect()
}
fn worker(shared: Arc<Shared>, mut output: Output<WindowsBackend>) {
    let mut cancelled_execution = false;
    loop {
        let mut state = shared.state.lock().unwrap();
        while state.desired.is_none() && (!state.admission || state.queue.is_empty()) {
            if state.host.check_inactivity(shared.started.elapsed()) {
                break;
            }
            state = shared
                .ready
                .wait_timeout(state, Duration::from_millis(100))
                .unwrap()
                .0;
        }
        if let Some(lifecycle) = state.host.take_transition() {
            drop(state);
            let cleanup = finish_output(&mut output, !cancelled_execution);
            cancelled_execution = false;
            if let Err(error) = &cleanup {
                let mut state = shared.state.lock().unwrap();
                if state.profiles.pending.is_some() {
                    logging::error(
                        "profile",
                        format_args!("activation cleanup failed: {error}"),
                    );
                }
                state.profiles.cancel(format!("Cleanup failed: {error}"));
                state.host.finish_transition(true);
                drop(state);
                shared.status(
                    &format!("Cleanup failed: {error}"),
                    lifecycle == Lifecycle::Quit,
                    true,
                );
                if lifecycle == Lifecycle::Quit {
                    break;
                }
                continue;
            }
            let next = lifecycle;
            let hook_shared = shared.clone();
            let prepare = if next == Lifecycle::Enable {
                let prepare_shared = shared.clone();
                let window = shared.main_window.load(Ordering::SeqCst) as HWND;
                // This is native lifecycle preparation, not presentation. Remove
                // the fallback before capture can admit output matching it.
                on_window_thread(window, move || {
                    let needed = {
                        let state = prepare_shared.state.lock().unwrap();
                        state.target == Lifecycle::Enable
                            && !state.paused
                            && state.desired.is_none()
                    };
                    if needed {
                        configure_hotkey(window, &prepare_shared, false)
                    } else {
                        Ok(())
                    }
                })
            } else {
                Ok(())
            };
            let result = prepare.and_then(|()| on_window_thread(output.backend.window, move || {
                let mut state = hook_shared.state.lock().unwrap();
                desktop::replace_hook(&mut state.hook, false)?;
                if next == Lifecycle::Enable {
                    if state.desired.is_some() {
                        return Ok(());
                    }
                    if state.paused || !ui::desktop_ready() {
                        return Err("Input desktop is unavailable; capture remains stopped. Use Enable to retry when it is available.".into());
                    }
                    desktop::replace_hook(&mut state.hook, true)?;
                    let baseline = sample_baseline(|code| {
                        (unsafe { GetAsyncKeyState(code as i32) }) as u16 & 0x8000 != 0
                    });
                    state.recorded_downs.retain(|key| baseline.contains(key));
                    state.caps_on = unsafe { GetKeyState(VK_CAPITAL as i32) } & 1 != 0;
                    state.caps_down = baseline.contains(&Key::CAPS_LOCK);
                    state.physical = baseline.clone();
                    state.visible = baseline.clone();
                    state.effective = baseline
                        .iter()
                        .filter(|key| modifier(**key))
                        .copied()
                        .collect();
                    if let Some(config) = state.profiles.commit() {
                        logging::info("profile", format_args!("activated name={}", state.profiles.name()));
                        state.host.core = typing_core::Core::new(config);
                    }
                    state
                        .core
                        .restart(baseline)
                        .map_err(|error| error.to_string())?;
                    // A later lifecycle request must keep admission closed.
                    state.admission = state.desired.is_none()
                        && !state.paused
                        && state.target == Lifecycle::Enable;
                } else if next == Lifecycle::Disable
                    && !state.paused
                    && state.target == Lifecycle::Disable
                {
                    // Reconcile recording suppression after output cleanup on
                    // disabled resume too. An unavailable desktop cannot supply
                    // a replacement observation of still-held keys.
                    if ui::desktop_ready() {
                        state.recorded_downs.retain(|key| {
                            (unsafe { GetAsyncKeyState(key.code() as i32) }) as u16 & 0x8000 != 0
                        });
                    }
                    if let Some(config) = state.profiles.commit() {
                        logging::info("profile", format_args!("activated name={}", state.profiles.name()));
                        state.host.core = typing_core::Core::new(config);
                        state.host.core.pause();
                    }
                }
                Ok(())
            }));
            {
                let mut state = shared.state.lock().unwrap();
                if let Err(error) = &result {
                    if state.profiles.pending.is_some() {
                        logging::error(
                            "profile",
                            format_args!("activation native readiness failed: {error}"),
                        );
                    }
                    state.profiles.cancel(format!("Native readiness: {error}"));
                }
                state.host.finish_transition(result.is_err());
            }
            match result {
                Ok(()) => {
                    let actual = shared.state.lock().unwrap().host.status().to_owned();
                    shared.status(&actual, next == Lifecycle::Quit, false);
                }
                Err(error) => shared.status(&error, next == Lifecycle::Quit, true),
            }
            if next == Lifecycle::Quit {
                break;
            }
            continue;
        }
        let work = state.queue.pop_front().unwrap();
        let generation = shared.generation.load(Ordering::SeqCst);
        let request = match work {
            HostWork::Command(request) => request,
            HostWork::Diagnostic(action) => {
                drop(state);
                let result = (|| {
                    if !output.backend.shared.state.lock().unwrap().processing {
                        begin_output(&mut output)?;
                    }
                    let key = match action {
                        DiagnosticOutput::Down(key) | DiagnosticOutput::Up(key) => key,
                    };
                    let event = output.backend.key(key)?;
                    match action {
                        DiagnosticOutput::Down(_) => output.down(event),
                        DiagnosticOutput::Up(_) => {
                            output.up(event)?;
                            finish_output(&mut output, false)
                        }
                    }
                })();
                if let Err(error) = result {
                    let _ = finish_output(&mut output, false);
                    shared.status(&error, false, true);
                }
                continue;
            }
        };
        let am = locale_designator(LOCALE_S1159);
        let pm = locale_designator(LOCALE_S2359);
        let mut local: SYSTEMTIME = unsafe { std::mem::zeroed() };
        unsafe {
            GetLocalTime(&mut local);
        }
        let execution = match state.host.resolve(
            request,
            LocalDateTime {
                year: local.wYear,
                month: local.wMonth as u8,
                day: local.wDay as u8,
                hour: local.wHour as u8,
                minute: local.wMinute as u8,
                am: &am,
                pm: &pm,
            },
        ) {
            Ok(execution) => execution,
            Err(error) => {
                drop(state);
                shared.status(&error.to_string(), false, true);
                continue;
            }
        };
        drop(state);
        let action = execution.action().clone();
        let result = match action {
            Action::None | Action::HostControl(_) => Ok(()),
            _ => execute(&mut output, &action, generation),
        };
        let was_cancelled = cancelled(&shared, generation);
        cancelled_execution = was_cancelled;
        let cleanup = if was_cancelled {
            Ok(())
        } else {
            finish_output(&mut output, false)
        };
        let outcome = if was_cancelled {
            Outcome::Cancelled
        } else if result.is_ok() && cleanup.is_ok() {
            Outcome::Success
        } else {
            Outcome::Failed
        };
        let mut state = shared.state.lock().unwrap();
        if let Err(error) = state.core.complete(execution, outcome) {
            logging::error("core", format_args!("completion failed: {error}"));
        }
        if !was_cancelled && cleanup.is_err() {
            state.host.finish_transition(true);
        }
        drop(state);
        if let Err(error) = result {
            logging::error("output", format_args!("{error}"));
        }
        if let Err(error) = cleanup {
            shared.status(&error, false, true);
        }
        if let Action::HostControl(control) = action {
            let command = match control {
                HostControl::Enable => ENABLE,
                HostControl::Disable => DISABLE,
            };
            unsafe {
                PostMessageW(
                    shared.main_window.load(Ordering::SeqCst) as HWND,
                    WM_COMMAND,
                    command,
                    0,
                );
            }
        }
    }
}

fn configure_hotkey(window: HWND, shared: &Shared, registered: bool) -> Result<(), String> {
    let (chord, current) = {
        let state = shared.state.lock().unwrap();
        (state.core.config().global_hotkey, state.hotkey_registered)
    };
    let Some(chord) = chord else {
        return Ok(());
    };
    if current == registered {
        return Ok(());
    }
    let success = unsafe {
        if registered {
            RegisterHotKey(
                window,
                1,
                hotkey_modifiers(chord) | MOD_NOREPEAT,
                chord.key.code() as u32,
            )
        } else {
            UnregisterHotKey(window, 1)
        }
    };
    if success == 0 {
        return Err(format!(
            "global hotkey update failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    shared.state.lock().unwrap().hotkey_registered = registered;
    Ok(())
}
fn tray_menu(next: Lifecycle) -> Result<HMENU, String> {
    unsafe {
        let menu = CreatePopupMenu();
        if menu == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        for (id, label) in [
            (SHOW, "Show Window"),
            (
                TOGGLE,
                if next == Lifecycle::Enable {
                    "Enable"
                } else {
                    "Disable"
                },
            ),
            (QUIT, "Exit"),
        ] {
            if AppendMenuW(menu, MF_STRING, id, wide(label).as_ptr()) == 0 {
                let error = std::io::Error::last_os_error().to_string();
                DestroyMenu(menu);
                return Err(error);
            }
        }
        Ok(menu)
    }
}

unsafe extern "system" fn main_window(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if message == WM_RPC {
        return unsafe { dispatch_call(l) };
    }
    let shared = SHARED.get().unwrap();
    if let Some(result) = ui::message(window, message, w, l) {
        return result;
    }
    match message {
        value if value == WM_APP + 40 => {
            unsafe {
                MessageBoxW(
                    window,
                    wide(&format!(
                        "Typing {}\r\nFast vim and emacs cursor moving and powerful key mapping\r\n\r\nGitHub: https://github.com/aldetl/Typing\r\nWebsite: https://xdetail.org/typing",
                        env!("CARGO_PKG_VERSION")
                    ))
                    .as_ptr(),
                    wide("About Typing").as_ptr(),
                    MB_OK | MB_ICONINFORMATION,
                );
            }
            0
        }
        WM_COMMAND => {
            match w & 0xffff {
                ENABLE => shared.lifecycle(Lifecycle::Enable),
                DISABLE => shared.lifecycle(Lifecycle::Disable),
                TOGGLE => {
                    let next = shared.state.lock().unwrap().host.toggle_target();
                    shared.lifecycle(next);
                }
                QUIT if ui::confirm_exit() => shared.lifecycle(Lifecycle::Quit),
                _ => {}
            }
            0
        }
        WM_HOTKEY => {
            if shared.record_owner.load(Ordering::SeqCst) != 0 {
                return 0;
            }
            let next = shared.state.lock().unwrap().host.toggle_target();
            shared.lifecycle(next);
            0
        }
        WM_STATUS => {
            let (mut text, quit, failed) =
                *unsafe { Box::from_raw(l as *mut (String, bool, bool)) };
            ui::profile_status();
            if !quit {
                let fallback = {
                    let state = shared.state.lock().unwrap();
                    state.hook == 0
                        && (state.target != Lifecycle::Enable || state.paused || state.failed)
                };
                if let Err(error) = configure_hotkey(window, shared, fallback) {
                    logging::error("hotkey", format_args!("{error}"));
                    let mut state = shared.state.lock().unwrap();
                    state.host.close_admission();
                    state.host.failed = true;
                    drop(state);
                    text = format!("{text}\r\n{error}");
                }
            }
            ui::refresh();
            unsafe {
                SetWindowTextW(window, wide(&format!("Typing - {text}")).as_ptr());
            }
            if quit {
                if failed {
                    unsafe {
                        MessageBoxW(
                            window,
                            wide(&text).as_ptr(),
                            wide("Typing cleanup warning").as_ptr(),
                            MB_OK | MB_ICONWARNING,
                        );
                    }
                }
                unsafe {
                    DestroyWindow(window);
                }
            }
            0
        }
        WM_TRAY if l as u32 == WM_LBUTTONDBLCLK => {
            let next = shared.state.lock().unwrap().host.toggle_target();
            shared.lifecycle(next);
            0
        }
        WM_TRAY if l as u32 == WM_RBUTTONUP => {
            let next = shared.state.lock().unwrap().host.toggle_target();
            let menu = match tray_menu(next) {
                Ok(menu) => menu,
                Err(error) => {
                    logging::error("tray", format_args!("create menu: {error}"));
                    return 0;
                }
            };
            unsafe {
                let mut point: POINT = std::mem::zeroed();
                GetCursorPos(&mut point);
                SetForegroundWindow(window);
                let selected = TrackPopupMenu(
                    menu,
                    TPM_RETURNCMD | TPM_RIGHTBUTTON,
                    point.x,
                    point.y,
                    0,
                    window,
                    std::ptr::null(),
                );
                DestroyMenu(menu);
                if selected != 0 {
                    PostMessageW(window, WM_COMMAND, selected as usize, 0);
                }
            }
            0
        }
        WM_CLOSE => {
            shared.lifecycle(Lifecycle::Quit);
            0
        }
        WM_DESTROY => {
            ui::shutdown();
            unsafe {
                PostQuitMessage(0);
            }
            0
        }
        _ => unsafe { DefWindowProcW(window, message, w, l) },
    }
}

fn create_window(class: &str, title: &str, procedure: WNDPROC) -> Result<HWND, String> {
    let name = wide(class);
    let mut class: WNDCLASSW = unsafe { std::mem::zeroed() };
    class.lpfnWndProc = procedure;
    class.hInstance = unsafe { GetModuleHandleW(std::ptr::null()) };
    class.hIcon = unsafe { LoadIconW(class.hInstance, 100 as *const u16) };
    if class.hIcon == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    class.hCursor = unsafe { LoadCursorW(0, IDC_ARROW) };
    class.hbrBackground = (windows_sys::Win32::Graphics::Gdi::COLOR_WINDOW + 1) as isize;
    class.lpszClassName = name.as_ptr();
    if unsafe { RegisterClassW(&class) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let window = unsafe {
        CreateWindowExW(
            0,
            name.as_ptr(),
            wide(title).as_ptr(),
            if title.starts_with("Typing -") {
                WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN
            } else {
                0
            },
            0,
            0,
            0,
            0,
            0,
            0,
            class.hInstance,
            std::ptr::null(),
        )
    };
    if window == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(window)
    }
}

fn message_loop() {
    let mut message: MSG = unsafe { std::mem::zeroed() };
    while unsafe { GetMessageW(&mut message, 0, 0, 0) } > 0 {
        if ui::translate_dialog(&message) {
            continue;
        }
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn source_tag(process: u32) -> usize {
    #[cfg(target_pointer_width = "64")]
    {
        0x5459504900000000 | process as usize
    }
    #[cfg(target_pointer_width = "32")]
    {
        process as usize
    }
}

fn load_configuration(
    path: &std::path::Path,
) -> Result<
    (
        String,
        Config,
        typing_host::Feedback,
        typing_host::profiles::Activation,
    ),
    String,
> {
    let path = std::path::absolute(path)
        .map_err(|error| format!("Invalid configuration path: {error}"))?;
    let help = "Edit or restore configuration.ini, then restart Typing. No keyboard handling was started and the file was not changed.";
    let text = std::fs::read_to_string(&path).map_err(|error| {
        format!(
            "Cannot read configuration:\r\n{}\r\n{error}\r\n\r\n{help}",
            path.display()
        )
    })?;
    let application = typing_host::profiles::Application::parse(path.clone(), text.clone())
        .map_err(|error| {
            format!(
                "Invalid configuration {}:\r\n{error}\r\n{help}",
                path.display()
            )
        })?;
    let (profiles, config) =
        typing_host::profiles::Activation::load(application).map_err(|error| {
            format!(
                "Invalid configuration {}:\r\n{error}\r\n{help}",
                path.display()
            )
        })?;
    let feedback = typing_host::Feedback::from_config(&config).map_err(|error| {
        format!(
            "Invalid configuration:\r\n{}\r\n{error}\r\n\r\n{help}",
            path.display()
        )
    })?;
    Ok((text, config, feedback, profiles))
}

fn run() -> Result<(), String> {
    ui::prepare_process();
    let directory = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .parent()
        .unwrap()
        .to_path_buf();
    let mut config_path = directory.join("configuration.ini");
    let mut dll_path = directory.join("inputsim.dll");
    let mut preference_path = ui::preference_path();
    let mut log_path = preference_path.with_file_name("typing.log");
    let mut arguments = std::env::args_os().skip(1);
    while let Some(option) = arguments.next() {
        let value = arguments
            .next()
            .ok_or("expected a path after each option")?;
        match option.to_str() {
            Some("--config") => config_path = PathBuf::from(value),
            Some("--inputsim") => dll_path = PathBuf::from(value),
            Some("--log") => log_path = PathBuf::from(value),
            Some("--state") => preference_path = PathBuf::from(value),
            _ => return Err("options are --config, --inputsim, --log, and --state".into()),
        }
    }
    logging::start(log_path, env!("CARGO_PKG_VERSION"));
    let (text, config, feedback, profiles) = load_configuration(&config_path)?;
    let profile_recovery = !profiles.valid;
    let api = Arc::new(InputSim::load(&dll_path)?);
    let (event_sender, event_receiver) = std::sync::mpsc::sync_channel(256);
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            host: Session::new(config),
            profiles,
            recorder: None,
            recorded_downs: BTreeSet::new(),
            hotkey_registered: false,
            physical: BTreeSet::new(),
            visible: BTreeSet::new(),
            effective: BTreeSet::new(),
            command_modifiers: BTreeSet::new(),
            modifier_events: BTreeMap::new(),
            physical_version: 0,
            left_shift_pending: false,
            caps_on: false,
            caps_down: false,
            caps_restore: None,
            hook: 0,
        }),
        ready: Condvar::new(),
        generation: AtomicU64::new(0),
        started: Instant::now(),
        tag: source_tag(unsafe { windows_sys::Win32::System::Threading::GetCurrentProcessId() }),
        main_window: AtomicU64::new(0),
        hook_window: AtomicU64::new(0),
        diagnostic_events: event_sender,
        editor_active: AtomicBool::new(false),
        record_owner: AtomicU64::new(0),
    });
    SHARED
        .set(shared.clone())
        .map_err(|_| "host already initialized")?;
    logging::info(
        "startup",
        format_args!("configuration loaded; InputSim loaded"),
    );
    let main = create_window(
        "TypingWindowsClient",
        "Typing - Starting",
        Some(main_window),
    )?;
    shared.main_window.store(main as u64, Ordering::SeqCst);
    if let Err(error) = ui::initialize(main, &text, feedback, preference_path, event_receiver) {
        unsafe {
            DestroyWindow(main);
        }
        return Err(error);
    }
    if let Err(error) = configure_hotkey(main, &shared, true) {
        logging::error("hotkey", format_args!("{error}"));
        unsafe {
            DestroyWindow(main);
        }
        return Err(error);
    }
    logging::info("startup", format_args!("tray and hotkey ready"));
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let hook_thread = thread::spawn(move || {
        let result = create_window("TypingWindowsHook", "Typing Hook", Some(hook_window));
        let succeeded = result.is_ok();
        sender.send(result).unwrap();
        if succeeded {
            message_loop();
        }
    });
    let hook = receiver.recv().map_err(|error| error.to_string())??;
    shared.hook_window.store(hook as u64, Ordering::SeqCst);
    let worker_shared = shared.clone();
    let backend = WindowsBackend {
        window: hook,
        api,
        shared: shared.clone(),
    };
    let worker = thread::spawn(move || worker(worker_shared, Output::new(backend)));
    if profile_recovery {
        ui::show_profile_recovery();
    } else {
        shared.lifecycle(Lifecycle::Enable);
    }
    message_loop();
    unsafe {
        UnregisterHotKey(main, 1);
    }
    logging::info("lifecycle", format_args!("joining worker"));
    let _ = worker.join();
    logging::info("lifecycle", format_args!("worker stopped"));
    on_window_thread(hook, move || {
        let shared = SHARED.get().unwrap();
        let mut state = shared.state.lock().unwrap();
        desktop::replace_hook(&mut state.hook, false)?;
        unsafe {
            PostQuitMessage(0);
        }
        Ok(())
    })?;
    let _ = hook_thread.join();
    logging::info("session", format_args!("shutdown complete"));
    Ok(())
}

fn main() {
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(location) = info.location() {
            logging::error(
                "panic",
                format_args!("at {}:{}", location.file(), location.line()),
            );
        }
        default_panic(info);
    }));
    if let Err(error) = run() {
        logging::error("startup", format_args!("{error}"));
        unsafe {
            MessageBoxW(
                0,
                wide(&error).as_ptr(),
                wide("Typing startup error").as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
        logging::shutdown();
        std::process::exit(1);
    }
    logging::shutdown();
}

#[cfg(test)]
mod host_tests {
    use super::*;
    #[test]
    fn tray_menu_orders_show_toggle_exit_and_labels_the_next_action() {
        for (next, label) in [
            (Lifecycle::Enable, "Enable"),
            (Lifecycle::Disable, "Disable"),
        ] {
            let menu = tray_menu(next).unwrap();
            unsafe {
                assert_eq!(GetMenuItemCount(menu), 3);
                for (index, (id, title)) in [(SHOW, "Show Window"), (TOGGLE, label), (QUIT, "Exit")]
                    .into_iter()
                    .enumerate()
                {
                    let mut buffer = [0u16; 64];
                    assert_eq!(GetMenuItemID(menu, index as i32), id as u32);
                    let count = GetMenuStringW(
                        menu,
                        index as u32,
                        buffer.as_mut_ptr(),
                        buffer.len() as i32,
                        MF_BYPOSITION,
                    );
                    assert_eq!(String::from_utf16_lossy(&buffer[..count as usize]), title);
                }
                DestroyMenu(menu);
            }
        }
    }
    #[test]
    fn mouse_buttons_never_enter_the_keyboard_restart_baseline() {
        let baseline = sample_baseline(|code| [1, 2, 4, 5, 6, 65, 162].contains(&code));
        assert_eq!(
            baseline,
            [Key::from_code(65).unwrap(), Key::LEFT_CONTROL].into()
        );
    }
}
