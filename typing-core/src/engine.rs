use std::{collections::BTreeSet, fmt, sync::Arc, time::Duration};

use crate::{BindingGroup, Chord, Command, Config, Key, Modifiers};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Down,
    Up,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyEvent {
    pub key: Key,
    pub direction: Direction,
}

/// A request is admitted once and moved into the host's FIFO queue. Do not retain
/// requests across the host's pause/restart boundary.
#[derive(Debug)]
pub struct Request {
    command: Command,
}

impl Request {
    pub fn command(&self) -> &Command {
        &self.command
    }
}

#[derive(Debug)]
pub struct Decision {
    pub consume: bool,
    pub request: Option<Request>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostControl {
    Enable,
    Disable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    None,
    /// Hold all but the last key across `count` taps of the last key. Release
    /// owned keys on completion, cancellation, and failure in the host.
    Keys {
        keys: Vec<Key>,
        count: u32,
        delay: Duration,
    },
    Text(String),
    /// Post the lifecycle request, complete this core execution, then await
    /// shutdown or restart. Holding the execution while awaiting restart would
    /// prevent the host from completing its old output lifetime.
    HostControl(HostControl),
}

/// Host-supplied local wall time. `am` and `pm` carry the host culture's
/// designators, matching Aldetl's `hh:mm tt` formatting. Values are calendar
/// components from the host clock, with hours in 0..24 and minutes in 0..60.
#[derive(Clone, Copy, Debug)]
pub struct LocalDateTime<'a> {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub am: &'a str,
    pub pm: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Success,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoreError {
    Busy,
    Paused,
    ForeignExecution,
}

impl fmt::Display for CoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Busy => "an execution still needs completion",
            Self::Paused => "command admission is paused",
            Self::ForeignExecution => "execution does not belong to this core's active command",
        })
    }
}

impl std::error::Error for CoreError {}

impl CoreError {
    pub fn diagnostic(&self) -> crate::Message {
        match self {
            Self::Busy => crate::msg!("core.busy", "an execution still needs completion"),
            Self::Paused => crate::msg!("core.paused", "command admission is paused"),
            Self::ForeignExecution => crate::msg!(
                "core.foreign",
                "execution does not belong to this core's active command"
            ),
        }
    }
}

/// Must be returned to the same Core after the host finishes the action.
/// The private identity prevents duplicate or foreign completion bookkeeping.
#[derive(Debug)]
#[must_use = "execute the action and report completion, including on failure"]
pub struct Execution {
    action: Action,
    identity: Arc<()>,
    movement: Option<(Chord, u32)>,
    exit_after_output: bool,
}

impl Execution {
    pub fn action(&self) -> &Action {
        &self.action
    }
}

pub struct Core {
    config: Config,
    enabled: bool,
    command_mode: bool,
    held: BTreeSet<Key>,
    isolated_mode_key: bool,
    activity: Duration,
    counts: [u32; 4],
    pending_step: Option<u32>,
    selection: bool,
    history: Vec<(Chord, u32)>,
    active: Option<Arc<()>>,
}

impl Core {
    /// Starts enabled with command mode off. Use `restart` with a sampled key
    /// baseline when capture begins with keys already held.
    pub fn new(config: Config) -> Self {
        Self {
            config,
            enabled: true,
            command_mode: false,
            held: BTreeSet::new(),
            isolated_mode_key: false,
            activity: Duration::ZERO,
            counts: [1; 4],
            pending_step: None,
            selection: false,
            history: Vec::new(),
            active: None,
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }
    pub fn command_mode(&self) -> bool {
        self.command_mode
    }
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
    pub fn selection(&self) -> bool {
        self.selection
    }

    pub fn input(&mut self, event: KeyEvent, now: Duration) -> Decision {
        let mut decision = Decision {
            consume: false,
            request: None,
        };
        if !self.enabled {
            return decision;
        }
        let is_mode_key = self.config.mode_keys.contains(&event.key);
        match event.direction {
            Direction::Up => {
                self.held.remove(&event.key);
                if self.isolated_mode_key && is_mode_key {
                    self.change_mode(!self.command_mode, now);
                }
            }
            Direction::Down => {
                self.held.insert(event.key);
                self.isolated_mode_key = false;
                // Mode-key Caps Lock must never leak a capitalization toggle
                // when another key prevents an isolated mode gesture.
                decision.consume = is_mode_key && event.key == Key::CAPS_LOCK;
                if is_mode_key && self.held.len() == 1 {
                    self.isolated_mode_key = true;
                    decision.consume = true;
                    return decision;
                }
                let modifiers = self.held.iter().fold(Modifiers::default(), |flags, key| {
                    flags.with(Modifiers::for_key(*key))
                });
                let chord = Chord {
                    key: event.key,
                    modifiers,
                };
                if let Some(binding) =
                    self.config.bindings.get(&chord).filter(|binding| {
                        binding.group == BindingGroup::Always || self.command_mode
                    })
                {
                    if binding.group == BindingGroup::CommandMode {
                        self.activity = now;
                    }
                    decision.consume = true;
                    decision.request = Some(Request {
                        command: binding.command.clone(),
                    });
                }
            }
        }
        decision
    }

    /// Maintain held-key identity while a host-owned editor receives input.
    /// No binding or isolated mode-key gesture is admitted on this path.
    pub fn observe_unbound(&mut self, event: KeyEvent) {
        self.isolated_mode_key = false;
        match event.direction {
            Direction::Down => {
                self.held.insert(event.key);
            }
            Direction::Up => {
                self.held.remove(&event.key);
            }
        }
    }

    pub fn check_inactivity(&mut self, now: Duration) -> Option<Request> {
        if !self.enabled || !self.command_mode || !self.config.auto_exit {
            return None;
        }
        let elapsed = now.saturating_sub(self.activity).as_secs();
        if self.config.auto_exit_seconds < 0 || elapsed > self.config.auto_exit_seconds as u64 {
            self.activity = now;
            return Some(Request {
                command: Command::Exit,
            });
        }
        None
    }

    /// The caller must finish each execution before dequeuing another request.
    /// Mode gestures and timer checks may occur while an execution is active.
    pub fn resolve(
        &mut self,
        request: Request,
        local: LocalDateTime<'_>,
    ) -> Result<Execution, CoreError> {
        if !self.enabled {
            return Err(CoreError::Paused);
        }
        if self.active.is_some() {
            return Err(CoreError::Busy);
        }
        let mut movement = None;
        let mut exit_after_output = false;
        let action = match request.command {
            Command::ModeKey => Action::None,
            Command::Keys(chord) => key_action(chord, 1),
            Command::Step(step) => {
                self.pending_step = Some(step);
                Action::None
            }
            Command::ToggleSelection => {
                self.selection = !self.selection;
                Action::None
            }
            Command::Exit => {
                self.change_mode(false, Duration::ZERO);
                Action::None
            }
            Command::Movement(chord) => {
                let group = movement_group(chord);
                if let (Some(step), Some(index)) = (self.pending_step.take(), group) {
                    self.counts[index] = step;
                }
                let count = group.map_or(1, |index| self.counts[index]);
                if group.is_some() {
                    movement = Some((chord, count));
                }
                let modifiers = if self.selection {
                    chord.modifiers.with(Modifiers::SHIFT)
                } else {
                    chord.modifiers
                };
                key_action(Chord { modifiers, ..chord }, count)
            }
            Command::Undo => match self.history.pop() {
                Some((chord, count)) => key_action(opposite(chord), count),
                None => Action::None,
            },
            Command::Date => Action::Text(format!(
                "{:04}-{:02}-{:02}",
                local.year, local.month, local.day
            )),
            Command::Time => {
                let hour = match local.hour % 12 {
                    0 => 12,
                    hour => hour,
                };
                let designator = if local.hour < 12 { local.am } else { local.pm };
                Action::Text(format!("{hour:02}:{:02} {designator}", local.minute))
            }
            Command::Enable => Action::HostControl(HostControl::Enable),
            Command::Disable => Action::HostControl(HostControl::Disable),
            Command::SwitchWindow(count) => {
                exit_after_output = true;
                Action::Keys {
                    keys: vec![Key::LEFT_ALT, Key::TAB],
                    count,
                    delay: Duration::from_millis(100),
                }
            }
        };
        let identity = Arc::new(());
        self.active = Some(identity.clone());
        Ok(Execution {
            action,
            identity,
            movement,
            exit_after_output,
        })
    }

    pub fn complete(&mut self, execution: Execution, outcome: Outcome) -> Result<(), CoreError> {
        if !self
            .active
            .as_ref()
            .is_some_and(|active| Arc::ptr_eq(active, &execution.identity))
        {
            return Err(CoreError::ForeignExecution);
        }
        self.active = None;
        if outcome == Outcome::Success {
            if let Some(movement) = execution.movement {
                self.history.push(movement);
            }
            if execution.exit_after_output && self.enabled {
                self.change_mode(false, Duration::ZERO);
            }
        }
        Ok(())
    }

    /// The host also discards queued requests and cancels/cleans active output.
    /// Completion remains permitted so the worker can release its lifetime.
    pub fn pause(&mut self) {
        self.enabled = false;
    }

    /// The host calls this only after its old queue is discarded and its output
    /// cleanup succeeds. Rejected output releases remain owned by the host.
    pub fn restart(&mut self, baseline: impl IntoIterator<Item = Key>) -> Result<(), CoreError> {
        if self.active.is_some() {
            return Err(CoreError::Busy);
        }
        self.held = baseline.into_iter().collect();
        self.isolated_mode_key = false;
        self.change_mode(false, Duration::ZERO);
        self.enabled = true;
        Ok(())
    }

    fn change_mode(&mut self, enabled: bool, now: Duration) {
        self.command_mode = enabled;
        self.counts = [1; 4];
        self.pending_step = None;
        self.selection = false;
        if enabled {
            self.history.clear();
            self.activity = now;
        }
    }
}

fn key_action(chord: Chord, count: u32) -> Action {
    Action::Keys {
        keys: chord
            .output_keys()
            .expect("configured output chords are validated"),
        count,
        delay: Duration::ZERO,
    }
}

fn movement_group(chord: Chord) -> Option<usize> {
    if chord.modifiers == Modifiers::CONTROL {
        return matches!(chord.key, Key::LEFT | Key::RIGHT).then_some(3);
    }
    match chord.key {
        Key::LEFT | Key::RIGHT => Some(0),
        Key::UP | Key::DOWN => Some(1),
        Key::PAGE_UP | Key::PAGE_DOWN => Some(2),
        _ => None,
    }
}

fn opposite(chord: Chord) -> Chord {
    let key = match chord.key {
        Key::LEFT => Key::RIGHT,
        Key::RIGHT => Key::LEFT,
        Key::UP => Key::DOWN,
        Key::DOWN => Key::UP,
        Key::PAGE_UP => Key::PAGE_DOWN,
        Key::PAGE_DOWN => Key::PAGE_UP,
        _ => unreachable!("only reversible movement enters history"),
    };
    Chord { key, ..chord }
}
