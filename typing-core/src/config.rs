use crate::{Message, msg};
use std::collections::{BTreeMap, BTreeSet};

use crate::{Chord, Key, Modifiers};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindingGroup {
    CommandMode,
    Always,
}

impl crate::MessageValue for BindingGroup {
    fn message_value(&self) -> Message {
        match self {
            Self::CommandMode => msg!("binding.command_mode", "command mode"),
            Self::Always => msg!("binding.both_modes", "both enabled modes"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    ModeKey,
    Keys(Chord),
    Movement(Chord),
    Step(u32),
    Exit,
    Undo,
    ToggleSelection,
    Date,
    Time,
    Enable,
    Disable,
    SwitchWindow(u32),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Binding {
    pub group: BindingGroup,
    pub command: Command,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub(crate) mode_keys: BTreeSet<Key>,
    pub(crate) bindings: BTreeMap<Chord, Binding>,
    pub auto_exit: bool,
    pub auto_exit_seconds: i32,
    pub global_hotkey: Option<Chord>,
    /// Original common settings, including UI values interpreted by the host.
    pub common: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigError {
    pub line: usize,
    pub message: Message,
}

struct Entry {
    key: String,
    value: String,
    line: usize,
}

impl Config {
    /// Parses text only. File access and the user-facing response to diagnostics
    /// belong to the host. A failed parse publishes no partial configuration.
    pub fn parse(text: &str) -> Result<Self, Vec<ConfigError>> {
        Self::parse_with_defaults(text, true)
    }

    /// Explicit bindings: an empty document never introduces legacy defaults.
    pub fn parse_explicit(text: &str) -> Result<Self, Vec<ConfigError>> {
        Self::parse_with_defaults(text, false)
    }

    fn parse_with_defaults(text: &str, legacy: bool) -> Result<Self, Vec<ConfigError>> {
        let mut sections: BTreeMap<String, Vec<Entry>> = BTreeMap::new();
        let mut section = String::new();
        let mut errors = Vec::new();
        for (index, source) in text.trim_start_matches('\u{feff}').lines().enumerate() {
            let line = source.trim();
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = line[1..line.len() - 1].to_owned();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                errors.push(ConfigError {
                    line: index + 1,
                    message: msg!("profiles.expected_key_value", "expected key=value"),
                });
                continue;
            };
            let entries = sections.entry(section.clone()).or_default();
            let entry = Entry {
                key: key.trim().into(),
                value: value.trim().into(),
                line: index + 1,
            };
            // Aldetl's section reader replaces duplicate text in its original slot.
            if let Some(existing) = entries.iter_mut().find(|old| old.key == entry.key) {
                *existing = entry;
            } else {
                entries.push(entry);
            }
        }

        let mut config = Self {
            mode_keys: BTreeSet::new(),
            bindings: BTreeMap::new(),
            auto_exit: true,
            auto_exit_seconds: 5,
            global_hotkey: None,
            common: BTreeMap::new(),
        };
        if let Some(entries) = sections.get("common") {
            for entry in entries {
                config.common.insert(entry.key.clone(), entry.value.clone());
                match entry.key.as_str() {
                    "mode_auto_exit" => match entry.value.as_str() {
                        "on" => config.auto_exit = true,
                        "off" => config.auto_exit = false,
                        _ => errors.push(config_error(
                            entry,
                            msg!("config.expected_on_or_off", "expected on or off"),
                        )),
                    },
                    "mode_auto_exit_time" => match entry.value.parse() {
                        Ok(seconds) => config.auto_exit_seconds = seconds,
                        Err(_) => errors.push(config_error(
                            entry,
                            msg!(
                                "config.expected_a_signed_integer_number_of",
                                "expected a signed integer number of seconds"
                            ),
                        )),
                    },
                    "hotkey" => match entry.value.parse() {
                        Ok(chord) => config.global_hotkey = Some(chord),
                        Err(error) => errors.push(config_error(entry, error.diagnostic())),
                    },
                    _ => {}
                }
            }
        }

        // Separate command maps preserve Aldetl's simple-command-first dispatch,
        // including collisions when one binding group overwrites another.
        let mut simple = BTreeMap::new();
        let mut movement = BTreeMap::new();
        let mut steps = BTreeMap::new();
        let mut registry = BTreeMap::new();
        if legacy && sections.get("modekeys").is_none_or(Vec::is_empty) {
            config.mode_keys.insert(Key::CAPS_LOCK);
            for (key, command) in [
                ("G", "exit"),
                ("Q", "undo"),
                ("Y", "date"),
                ("T", "time"),
                ("E", "delete"),
                ("R", "backspace"),
                ("U", "home"),
                ("O", "end"),
            ] {
                // Constructor defaults remain separate from the active registry.
                simple.insert(key.parse().unwrap(), named_command(command).unwrap());
            }
            for (key, command) in [
                ("I", "up"),
                ("K", "down"),
                ("J", "left"),
                ("L", "right"),
                ("N", "pageup"),
                ("M", "pagedown"),
            ] {
                movement.insert(
                    key.parse().unwrap(),
                    Command::Movement(movement_name(command).unwrap()),
                );
            }
            for (key, step) in [("F", 1), ("D", 2), ("S", 5), ("A", 10)] {
                steps.insert(key.parse().unwrap(), Command::Step(step));
            }
        }
        for (section, group) in [
            ("modekeys", BindingGroup::CommandMode),
            ("hotkeys", BindingGroup::Always),
        ] {
            for entry in sections.get(section).into_iter().flatten() {
                let result = parse_binding(entry);
                let (input, command) = match result {
                    Ok(binding) => binding,
                    Err(message) => {
                        errors.push(config_error(entry, &message));
                        continue;
                    }
                };
                match command {
                    Command::ModeKey => {
                        config.mode_keys.insert(input.key);
                    }
                    Command::Movement(_) => {
                        movement.insert(input, command);
                    }
                    Command::Step(_) => {
                        steps.insert(input, command);
                    }
                    _ => {
                        simple.insert(input, command);
                    }
                }
                registry.insert(input, group);
            }
        }
        for (input, group) in registry {
            let command = simple
                .get(&input)
                .or_else(|| movement.get(&input))
                .or_else(|| steps.get(&input))
                .cloned()
                .unwrap_or(Command::ModeKey);
            config.bindings.insert(input, Binding { group, command });
        }
        if errors.is_empty() {
            Ok(config)
        } else {
            Err(errors)
        }
    }

    pub fn mode_keys(&self) -> &BTreeSet<Key> {
        &self.mode_keys
    }

    pub fn binding(&self, chord: Chord) -> Option<&Binding> {
        self.bindings.get(&chord)
    }

    pub fn bindings(&self) -> impl Iterator<Item = (Chord, &Binding)> {
        self.bindings.iter().map(|(key, value)| (*key, value))
    }

    /// Keep application settings while replacing the complete binding set.
    pub fn with_bindings(&self, bindings: &Config) -> Self {
        let mut composed = self.clone();
        composed.mode_keys = bindings.mode_keys.clone();
        composed.bindings = bindings.bindings.clone();
        composed
    }

    pub fn same_bindings(&self, other: &Self) -> bool {
        self.mode_keys == other.mode_keys && self.bindings == other.bindings
    }
}

pub fn validate_binding(input: &str, action: &str) -> Result<(Chord, Command), Message> {
    fn explicit_chord(text: &str) -> Result<(), Message> {
        let parts = text.split('+').collect::<Vec<_>>();
        if parts.iter().any(|part| part.is_empty()) {
            return Err(msg!(
                "config.a_shortcut_cannot_contain_an_empty",
                "A shortcut cannot contain an empty key name"
            ));
        }
        for part in &parts[..parts.len().saturating_sub(1)] {
            if ["Control", "Alt", "Shift"].contains(part) {
                continue;
            }
            let key: Key = part
                .parse()
                .map_err(|error: crate::KeyParseError| error.diagnostic())?;
            if ![
                Key::CAPS_LOCK,
                Key::LEFT_ALT,
                Key::RIGHT_ALT,
                Key::LEFT_CONTROL,
                Key::RIGHT_CONTROL,
                Key::LEFT_SHIFT,
                Key::RIGHT_SHIFT,
            ]
            .contains(&key)
            {
                return Err(msg!(
                    "config.only_supported_modifiers_may_precede_the",
                    "Only supported modifiers may precede the final key"
                ));
            }
        }
        Ok(())
    }
    explicit_chord(input)?;
    let binding = parse_binding(&Entry {
        key: input.into(),
        value: action.into(),
        line: 0,
    })?;
    if action.parse::<Chord>().is_ok() {
        explicit_chord(action)?;
    }
    Ok(binding)
}

impl Command {
    pub fn configuration_value(&self) -> String {
        match self {
            Self::ModeKey => "mode".into(),
            Self::Keys(chord) | Self::Movement(chord) => chord.to_string(),
            Self::Step(count) => format!("step{count}"),
            Self::Exit => "exit".into(),
            Self::Undo => "undo".into(),
            Self::ToggleSelection => "shift_onoff".into(),
            Self::Date => "date".into(),
            Self::Time => "time".into(),
            Self::Enable => "enable".into(),
            Self::Disable => "disable".into(),
            Self::SwitchWindow(number) => format!("switch_win{number}"),
        }
    }

    pub fn label(&self) -> Message {
        match self {
            Self::ModeKey => msg!("command.toggle_command_mode", "Toggle command mode"),
            Self::Undo => msg!("command.inverse_movement", "Inverse movement"),
            Self::ToggleSelection => msg!("command.toggle_selection", "Toggle selection"),
            Self::Exit => msg!("command.exit_command_mode", "Exit command mode"),
            Self::Movement(chord) => msg!("config.move", "Move: {chord}", chord = chord),
            Self::Keys(chord) => msg!("config.output", "Output: {chord}", chord = chord),
            Self::Step(count) => msg!("config.step_count", "Step count: {count}", count = count),
            Self::Date => msg!("command.date", "Date"),
            Self::Time => msg!("command.time", "Time"),
            Self::Enable => msg!("command.enable", "Enable"),
            Self::Disable => msg!("command.disable", "Disable"),
            Self::SwitchWindow(number) => msg!(
                "command.switch_window",
                "Switch window {number}",
                number = number
            ),
        }
    }
}

fn config_error(entry: &Entry, message: impl Into<Message>) -> ConfigError {
    ConfigError {
        line: entry.line,
        message: message.into(),
    }
}

fn parse_binding(entry: &Entry) -> Result<(Chord, Command), Message> {
    let input: Chord = entry
        .key
        .parse()
        .map_err(|error: crate::KeyParseError| error.diagnostic())?;
    if input.key == Key::NONE {
        return Err(msg!(
            "config.a_mapping_trigger_requires_a_final",
            "a mapping trigger requires a final key"
        ));
    }
    let command = if entry.value == "mode" {
        if input.modifiers != Modifiers::default() {
            return Err(msg!(
                "config.mode_requires_one_key",
                "mode requires one key"
            ));
        }
        Command::ModeKey
    } else if let Some(chord) = movement_name(&entry.value) {
        Command::Movement(chord)
    } else if let Some(command) = named_command(&entry.value) {
        command
    } else if let Some(step) = entry
        .value
        .strip_prefix("step")
        .and_then(|n| n.parse::<i32>().ok())
        .filter(|n| *n > 0)
    {
        Command::Step(step as u32)
    } else {
        let output: Chord = entry
            .value
            .parse()
            .map_err(|error: crate::KeyParseError| error.diagnostic())?;
        output.output_keys().map_err(|error| error.diagnostic())?;
        if is_movement(output) {
            Command::Movement(output)
        } else {
            Command::Keys(output)
        }
    };
    Ok((input, command))
}

fn named_command(name: &str) -> Option<Command> {
    Some(match name {
        "exit" => Command::Exit,
        "undo" => Command::Undo,
        "date" => Command::Date,
        "time" => Command::Time,
        "shift_onoff" => Command::ToggleSelection,
        "enable" => Command::Enable,
        "disable" => Command::Disable,
        "switch_win1" => Command::SwitchWindow(1),
        "switch_win2" => Command::SwitchWindow(2),
        "delete" => Command::Keys(Chord::single(Key::DELETE)),
        "backspace" => Command::Keys(Chord::single(Key::BACK)),
        "home" => Command::Keys(Chord::single(Key::HOME)),
        "end" => Command::Keys(Chord::single(Key::END)),
        "capslock" => Command::Keys(Chord::single(Key::CAPS_LOCK)),
        "ctrl_delete" => Command::Keys(Chord {
            key: Key::DELETE,
            modifiers: Modifiers::CONTROL,
        }),
        "ctrl_backspace" => Command::Keys(Chord {
            key: Key::BACK,
            modifiers: Modifiers::CONTROL,
        }),
        _ => return None,
    })
}

fn movement_name(name: &str) -> Option<Chord> {
    let key = match name {
        "left" | "left_word" => Key::LEFT,
        "right" | "right_word" => Key::RIGHT,
        "up" => Key::UP,
        "down" => Key::DOWN,
        "pageup" => Key::PAGE_UP,
        "pagedown" => Key::PAGE_DOWN,
        "home" => Key::HOME,
        "end" => Key::END,
        _ => return None,
    };
    let modifiers = if name.ends_with("_word") {
        Modifiers::CONTROL
    } else {
        Modifiers::default()
    };
    Some(Chord { key, modifiers })
}

fn is_movement(chord: Chord) -> bool {
    if chord.modifiers == Modifiers::CONTROL {
        return matches!(chord.key, Key::LEFT | Key::RIGHT);
    }
    chord.modifiers == Modifiers::default()
        && matches!(
            chord.key,
            Key::LEFT
                | Key::RIGHT
                | Key::UP
                | Key::DOWN
                | Key::PAGE_UP
                | Key::PAGE_DOWN
                | Key::HOME
                | Key::END
        )
}
