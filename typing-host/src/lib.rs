//! Platform-independent desktop host policy. Native adapters own capture,
//! output, resources and synchronization; views consume observations.
use std::{collections::VecDeque, time::Duration};
use typing_core::{Config, Core, CoreError, Execution, KeyEvent, LocalDateTime, Request};

pub mod help;
pub mod profiles;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lifecycle {
    Enable,
    Disable,
    Pause,
    Quit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticOutput {
    Down(typing_core::Key),
    Up(typing_core::Key),
}

#[derive(Debug)]
pub enum Work {
    Command(Request),
    Diagnostic(DiagnosticOutput),
}

pub struct Session {
    pub core: Core,
    pub queue: VecDeque<Work>,
    pub desired: Option<Lifecycle>,
    pub target: Lifecycle,
    pub admission: bool,
    pub processing: bool,
    pub paused: bool,
    pub transitioning: bool,
    pub failed: bool,
    recovery_pending: bool,
}

impl Session {
    pub fn new(config: Config) -> Self {
        Self {
            core: Core::new(config),
            queue: VecDeque::new(),
            desired: None,
            target: Lifecycle::Disable,
            admission: false,
            processing: false,
            paused: false,
            transitioning: false,
            failed: false,
            recovery_pending: false,
        }
    }
    /// Returns false after Quit; no later request may restart that lifetime.
    pub fn request(&mut self, next: Lifecycle) -> bool {
        if self.target == Lifecycle::Quit {
            return false;
        }
        self.recovery_pending = false;
        if next != Lifecycle::Pause {
            self.target = next;
            self.failed = false;
        }
        self.close_admission();
        self.desired = Some(if self.paused && next != Lifecycle::Quit {
            Lifecycle::Pause
        } else {
            next
        });
        true
    }
    pub fn toggle_target(&self) -> Lifecycle {
        if self.target == Lifecycle::Enable {
            Lifecycle::Disable
        } else {
            Lifecycle::Enable
        }
    }
    pub fn set_paused(&mut self, paused: bool) -> bool {
        if self.paused == paused || self.target == Lifecycle::Quit {
            return false;
        }
        self.paused = paused;
        if self.failed {
            return false;
        }
        let changed = self.request(if paused {
            Lifecycle::Pause
        } else {
            self.target
        });
        if changed && !paused {
            self.recovery_pending = true;
        }
        changed
    }
    /// A wake may arrive without a preceding pause, even during a manual
    /// restart whose baseline may predate the wake. Only wake-driven work can
    /// coalesce that observation; otherwise invalidate the old transition.
    pub fn recover(&mut self, paused: bool, new_cycle: bool) -> bool {
        if self.paused != paused {
            return self.set_paused(paused);
        }
        if !new_cycle || paused || self.failed || self.recovery_pending {
            return false;
        }
        let changed = self.request(self.target);
        if changed {
            self.recovery_pending = true;
        }
        changed
    }
    pub fn take_transition(&mut self) -> Option<Lifecycle> {
        let next = self.desired.take()?;
        self.transitioning = true;
        Some(next)
    }
    pub fn finish_transition(&mut self, failed: bool) {
        self.transitioning = false;
        self.failed = failed;
        if failed || self.desired.is_none() {
            self.recovery_pending = false;
        }
        if failed {
            self.close_admission();
            // Quit must still observe retained releases and show its warning.
            if self.desired != Some(Lifecycle::Quit) {
                self.desired = None;
            }
        }
    }
    pub fn close_admission(&mut self) {
        self.admission = false;
        self.core.pause();
        self.queue.clear();
    }
    /// Binding admission and queue insertion stay together under the host lock.
    pub fn input(&mut self, event: KeyEvent, now: Duration) -> (bool, bool) {
        if !self.admission {
            return (false, false);
        }
        let decision = self.core.input(event, now);
        let admitted = decision.request.is_some();
        if let Some(request) = decision.request {
            self.queue.push_back(Work::Command(request));
        }
        (decision.consume, admitted)
    }
    pub fn check_inactivity(&mut self, now: Duration) -> bool {
        if self.admission
            && let Some(request) = self.core.check_inactivity(now)
        {
            self.queue.push_back(Work::Command(request));
            true
        } else {
            false
        }
    }
    pub fn resolve(
        &mut self,
        request: Request,
        local: LocalDateTime<'_>,
    ) -> Result<Execution, CoreError> {
        self.core.resolve(request, local)
    }
    pub fn diagnostic(&mut self, action: DiagnosticOutput) -> bool {
        if !self.admission {
            return false;
        }
        self.queue.push_back(Work::Diagnostic(action));
        true
    }
    pub fn status(&self) -> &'static str {
        if self.target == Lifecycle::Quit {
            "Exiting"
        } else if self.paused {
            "Paused"
        } else if self.failed {
            "Failed"
        } else if self.desired.is_some() || self.transitioning {
            "Changing state"
        } else if self.admission {
            "Enabled"
        } else {
            "Disabled"
        }
    }
    pub fn icon_state(&self) -> usize {
        if !self.admission {
            0
        } else if self.core.command_mode() {
            2
        } else {
            1
        }
    }
}

#[derive(Clone, Debug)]
pub struct Feedback {
    pub command_tip: bool,
    pub radius: i32,
    pub opacity: u8,
    pub command_color: [u8; 3],
    pub caps_color: [u8; 3],
}
impl Feedback {
    pub fn from_config(config: &Config) -> Result<Self, String> {
        let value = |name: &str, fallback: i32, low: i32, high: i32| -> Result<i32, String> {
            match config.common.get(name) {
                None => Ok(fallback),
                Some(text) => text
                    .parse::<i32>()
                    .ok()
                    .filter(|v| (low..=high).contains(v))
                    .ok_or_else(|| format!("Invalid {name}: expected {low}..{high}")),
            }
        };
        let switch = |name: &str| -> Result<Option<bool>, String> {
            config
                .common
                .get(name)
                .map(|text| match text.as_str() {
                    "on" => Ok(true),
                    "off" => Ok(false),
                    _ => Err(format!(
                        "Invalid {name}: expected on or off, found {text:?}"
                    )),
                })
                .transpose()
        };
        let preferred = switch("mode_cursor_tip")?;
        let alias = switch("mode_mouse_tip")?;
        Ok(Self {
            command_tip: preferred.or(alias).unwrap_or(true),
            radius: value("tip_radius", 15, 1, 128)?,
            opacity: value("tip_opacity", 50, 0, 100)? as u8,
            command_color: [
                value("tip_color_r", 255, 0, 255)? as u8,
                value("tip_color_g", 224, 0, 255)? as u8,
                value("tip_color_b", 192, 0, 255)? as u8,
            ],
            caps_color: [192, 224, 255],
        })
    }
    pub fn visible_state(&self, admitted: bool, command: bool, caps: bool) -> u8 {
        if !admitted {
            0
        } else {
            u8::from(command && self.command_tip) | (u8::from(caps) << 1)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowSize {
    pub width: i32,
    pub height: i32,
}
impl Default for WindowSize {
    fn default() -> Self {
        Self {
            width: 640,
            height: 440,
        }
    }
}
impl WindowSize {
    pub fn parse(text: &str) -> Option<Self> {
        let (w, h) = text.trim().split_once(',')?;
        let size = Self {
            width: w.parse().ok()?,
            height: h.parse().ok()?,
        };
        (size.width > 0 && size.height > 0 && size.width <= 32768 && size.height <= 32768)
            .then_some(size)
    }
    pub fn encode(self) -> String {
        format!("{},{}\n", self.width, self.height)
    }
    pub fn physical(self, dpi: u32) -> Self {
        Self {
            width: (self.width as i64 * dpi as i64 / 96) as i32,
            height: (self.height as i64 * dpi as i64 / 96) as i32,
        }
    }
    pub fn logical(self, dpi: u32) -> Self {
        Self {
            width: (self.width as i64 * 96 / dpi.max(1) as i64) as i32,
            height: (self.height as i64 * 96 / dpi.max(1) as i64) as i32,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BindingRow {
    pub group: &'static str,
    pub input: String,
    pub action: String,
}

/// Presentation of configured names only. Interpretation/validation is exclusively Config's job.
pub fn binding_rows(text: &str) -> Vec<BindingRow> {
    let mut group = "";
    let mut rows: Vec<BindingRow> = Vec::new();
    for line in text.trim_start_matches('\u{feff}').lines().map(str::trim) {
        if line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            group = match line {
                "[modekeys]" => "Command mode",
                "[hotkeys]" => "Both enabled modes",
                _ => "",
            };
        } else if !group.is_empty()
            && let Some((input, action)) = line.split_once('=')
        {
            if let Some(row) = rows
                .iter_mut()
                .find(|r| r.group == group && r.input == input.trim())
            {
                row.action = action.trim().into();
            } else {
                rows.push(BindingRow {
                    group,
                    input: input.trim().into(),
                    action: action.trim().into(),
                });
            }
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use typing_core::{Direction, Key};
    fn session() -> Session {
        Session::new(Config::parse("[modekeys]\nCapsLock=mode\nJ=Left\n").unwrap())
    }
    #[test]
    fn latest_target_and_pause_preserve_user_choice_and_quit_is_terminal() {
        let mut s = session();
        s.request(Lifecycle::Enable);
        assert!(s.set_paused(true));
        assert_eq!(s.target, Lifecycle::Enable);
        s.request(Lifecycle::Disable);
        assert_eq!(s.desired, Some(Lifecycle::Pause));
        assert!(s.set_paused(false));
        assert_eq!(s.desired, Some(Lifecycle::Disable));
        s.request(Lifecycle::Quit);
        assert!(!s.request(Lifecycle::Enable));
        assert!(!s.set_paused(true));
    }
    #[test]
    fn pause_discards_admitted_work_and_cannot_finish_a_stale_mode_tap() {
        let mut s = session();
        s.core.restart([]).unwrap();
        s.admission = true;
        s.input(
            KeyEvent {
                key: Key::CAPS_LOCK,
                direction: Direction::Down,
            },
            Duration::ZERO,
        );
        s.request(Lifecycle::Disable);
        assert!(!s.admission);
        assert!(s.queue.is_empty());
        s.core.restart([]).unwrap();
        s.admission = true;
        s.input(
            KeyEvent {
                key: Key::CAPS_LOCK,
                direction: Direction::Up,
            },
            Duration::ZERO,
        );
        assert!(!s.core.command_mode());
    }
    #[test]
    fn feedback_obeys_admission_and_legacy_setting_name() {
        let f = Feedback::from_config(
            &Config::parse("[common]\nmode_cursor_tip=off\nmode_mouse_tip=on\ntip_radius=20\n")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(f.radius, 20);
        assert_eq!(f.visible_state(true, true, true), 2);
        assert_eq!(f.visible_state(false, true, true), 0);
    }
    #[test]
    fn missed_suspend_starts_recovery_and_duplicate_notifications_coalesce() {
        let mut s = session();
        s.request(Lifecycle::Enable);
        assert_eq!(s.take_transition(), Some(Lifecycle::Enable));
        s.core.restart([]).unwrap();
        s.admission = true;
        s.finish_transition(false);
        assert!(s.recover(false, true));
        assert!(!s.admission);
        assert!(!s.recover(false, true));
        assert_eq!(s.take_transition(), Some(Lifecycle::Enable));
        assert!(!s.recover(false, true));
        s.finish_transition(false);
        assert!(!s.recover(false, false));
    }
    #[test]
    fn wake_during_manual_restart_invalidates_the_old_baseline() {
        let mut s = session();
        s.request(Lifecycle::Enable);
        assert_eq!(s.take_transition(), Some(Lifecycle::Enable));
        assert!(s.recover(false, true));
        assert_eq!(s.desired, Some(Lifecycle::Enable));
        assert!(!s.admission);
        assert!(!s.recover(false, true));
        s.finish_transition(false); // Complete the superseded manual restart.
        assert_eq!(s.take_transition(), Some(Lifecycle::Enable));
        assert!(!s.recover(false, true)); // This operation began after wake.
        s.finish_transition(false);
        assert!(!s.recover(false, false)); // Paired user-resume notification.
        assert!(s.recover(false, true)); // A later wake can miss suspend again.
    }
    #[test]
    fn automatic_notifications_cannot_retry_failure_but_explicit_controls_can() {
        for next in [Lifecycle::Enable, Lifecycle::Disable] {
            let mut s = session();
            s.request(Lifecycle::Enable);
            s.take_transition();
            s.finish_transition(true);
            assert!(!s.recover(false, true));
            assert!(!s.set_paused(true));
            assert!(!s.set_paused(false));
            assert_eq!(s.desired, None);
            assert!(s.failed);
            assert!(s.request(next));
            assert!(!s.failed);
            assert_eq!(s.take_transition(), Some(next));
        }
    }
    #[test]
    fn failure_preserves_quit_and_later_suspend_blocks_pending_enable() {
        let mut s = session();
        s.request(Lifecycle::Enable);
        s.take_transition();
        assert!(s.set_paused(true));
        assert_eq!(s.desired, Some(Lifecycle::Pause));
        assert!(!s.admission);
        s.request(Lifecycle::Disable);
        assert_eq!(s.target, Lifecycle::Disable);
        assert!(s.set_paused(false));
        assert_eq!(s.desired, Some(Lifecycle::Disable));
        s.request(Lifecycle::Quit);
        s.finish_transition(true);
        assert_eq!(s.desired, Some(Lifecycle::Quit));
        assert!(!s.recover(false, true));
        assert!(!s.request(Lifecycle::Enable));
    }
    #[test]
    fn feedback_alias_validation_precedence_and_all_states() {
        for (settings, expected) in [
            ("", true),
            ("mode_mouse_tip=off", false),
            ("mode_mouse_tip=on", true),
            ("mode_cursor_tip=off\nmode_mouse_tip=on", false),
            ("mode_cursor_tip=on\nmode_mouse_tip=off", true),
        ] {
            let f =
                Feedback::from_config(&Config::parse(&format!("[common]\n{settings}")).unwrap())
                    .unwrap();
            assert_eq!(f.command_tip, expected);
            assert_eq!(f.visible_state(true, false, false), 0);
            assert_eq!(f.visible_state(true, true, false), u8::from(expected));
            assert_eq!(f.visible_state(true, false, true), 2);
            assert_eq!(f.visible_state(true, true, true), 2 | u8::from(expected));
        }
        for settings in [
            "mode_mouse_tip=yes",
            "mode_cursor_tip=off\nmode_mouse_tip=bad",
            "tip_radius=0",
            "tip_opacity=101",
            "tip_color_r=256",
        ] {
            assert!(
                Feedback::from_config(&Config::parse(&format!("[common]\n{settings}")).unwrap())
                    .is_err()
            );
        }
    }
    #[test]
    fn preferences_roundtrip_logical_size_without_cumulative_scaling() {
        let size = WindowSize {
            width: 700,
            height: 500,
        };
        assert_eq!(WindowSize::parse(&size.encode()), Some(size));
        assert_eq!(size.physical(144).logical(144), size);
        assert_eq!(WindowSize::parse("-1,500"), None);
    }
}
