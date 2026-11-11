use std::{fmt, str::FromStr};

/// Shared named key identity. Numeric values retain the legacy virtual-key
/// vocabulary, allowing a Windows adapter to translate without OS dependencies.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct Key(u16);

impl Key {
    pub const NONE: Self = Self(0);
    pub const BACK: Self = Self(8);
    pub const TAB: Self = Self(9);
    pub const ENTER: Self = Self(13);
    pub const CAPS_LOCK: Self = Self(20);
    pub const PAGE_UP: Self = Self(33);
    pub const PAGE_DOWN: Self = Self(34);
    pub const END: Self = Self(35);
    pub const HOME: Self = Self(36);
    pub const LEFT: Self = Self(37);
    pub const UP: Self = Self(38);
    pub const RIGHT: Self = Self(39);
    pub const DOWN: Self = Self(40);
    pub const DELETE: Self = Self(46);
    pub const LEFT_SHIFT: Self = Self(160);
    pub const RIGHT_SHIFT: Self = Self(161);
    pub const LEFT_CONTROL: Self = Self(162);
    pub const RIGHT_CONTROL: Self = Self(163);
    pub const LEFT_ALT: Self = Self(164);
    pub const RIGHT_ALT: Self = Self(165);

    pub fn code(self) -> u16 {
        self.0
    }

    pub fn from_code(code: u16) -> Option<Self> {
        (code <= 254).then_some(Self(code))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyParseError(pub String);

impl fmt::Display for KeyParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown key or chord {:?}", self.0)
    }
}

impl std::error::Error for KeyParseError {}

impl KeyParseError {
    pub fn diagnostic(&self) -> crate::Message {
        crate::msg!(
            "keys.unknown",
            "unknown key or chord {key}",
            key = format!("{:?}", self.0)
        )
    }
}
impl crate::MessageValue for KeyParseError {
    fn message_value(&self) -> crate::Message {
        self.diagnostic()
    }
}
impl crate::MessageValue for Key {
    fn message_value(&self) -> crate::Message {
        self.to_string().into()
    }
}
impl crate::MessageValue for Chord {
    fn message_value(&self) -> crate::Message {
        self.to_string().into()
    }
}

impl FromStr for Key {
    type Err = KeyParseError;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        if let Ok(code) = name.parse::<u16>() {
            return Self::from_code(code).ok_or_else(|| KeyParseError(name.into()));
        }
        if name.len() == 1 && name.as_bytes()[0].is_ascii_uppercase() {
            return Ok(Self(u16::from(name.as_bytes()[0])));
        }
        if let Some(number) = name
            .strip_prefix('D')
            .and_then(|n| n.parse::<u16>().ok())
            .filter(|n| *n <= 9)
        {
            return Ok(Self(48 + number));
        }
        if let Some(number) = name
            .strip_prefix("NumPad")
            .and_then(|n| n.parse::<u16>().ok())
            .filter(|n| *n <= 9)
        {
            return Ok(Self(96 + number));
        }
        if let Some(number) = name
            .strip_prefix('F')
            .and_then(|n| n.parse::<u16>().ok())
            .filter(|n| (1..=24).contains(n))
        {
            return Ok(Self(111 + number));
        }
        let code = match name {
            "None" => 0,
            "LButton" => 1,
            "RButton" => 2,
            "Cancel" => 3,
            "MButton" => 4,
            "XButton1" => 5,
            "XButton2" => 6,
            "Back" => 8,
            "Tab" => 9,
            "LineFeed" => 10,
            "Clear" => 12,
            "Return" | "Enter" => 13,
            "ShiftKey" => 16,
            "ControlKey" => 17,
            "Menu" => 18,
            "Pause" => 19,
            "Capital" | "CapsLock" => 20,
            "KanaMode" | "HanguelMode" | "HangulMode" => 21,
            "JunjaMode" => 23,
            "FinalMode" => 24,
            "HanjaMode" | "KanjiMode" => 25,
            "Escape" => 27,
            "IMEConvert" => 28,
            "IMENonconvert" => 29,
            "IMEAccept" | "IMEAceept" => 30,
            "IMEModeChange" => 31,
            "Space" => 32,
            "Prior" | "PageUp" => 33,
            "Next" | "PageDown" => 34,
            "End" => 35,
            "Home" => 36,
            "Left" => 37,
            "Up" => 38,
            "Right" => 39,
            "Down" => 40,
            "Select" => 41,
            "Print" => 42,
            "Execute" => 43,
            "Snapshot" | "PrintScreen" => 44,
            "Insert" => 45,
            "Delete" => 46,
            "Help" => 47,
            "LWin" => 91,
            "RWin" => 92,
            "Apps" => 93,
            "Sleep" => 95,
            "Multiply" => 106,
            "Add" => 107,
            "Separator" => 108,
            "Subtract" => 109,
            "Decimal" => 110,
            "Divide" => 111,
            "NumLock" => 144,
            "Scroll" => 145,
            "LShiftKey" => 160,
            "RShiftKey" => 161,
            "LControlKey" => 162,
            "RControlKey" => 163,
            "LMenu" => 164,
            "RMenu" => 165,
            "BrowserBack" => 166,
            "BrowserForward" => 167,
            "BrowserRefresh" => 168,
            "BrowserStop" => 169,
            "BrowserSearch" => 170,
            "BrowserFavorites" => 171,
            "BrowserHome" => 172,
            "VolumeMute" => 173,
            "VolumeDown" => 174,
            "VolumeUp" => 175,
            "MediaNextTrack" => 176,
            "MediaPreviousTrack" => 177,
            "MediaStop" => 178,
            "MediaPlayPause" => 179,
            "LaunchMail" => 180,
            "SelectMedia" => 181,
            "LaunchApplication1" => 182,
            "LaunchApplication2" => 183,
            "OemSemicolon" | "Oem1" => 186,
            "Oemplus" => 187,
            "Oemcomma" => 188,
            "OemMinus" => 189,
            "OemPeriod" => 190,
            "OemQuestion" | "Oem2" => 191,
            "Oemtilde" | "Oem3" => 192,
            "OemOpenBrackets" | "Oem4" => 219,
            "OemPipe" | "Oem5" => 220,
            "OemCloseBrackets" | "Oem6" => 221,
            "OemQuotes" | "Oem7" => 222,
            "Oem8" => 223,
            "OemBackslash" | "Oem102" => 226,
            "ProcessKey" => 229,
            "Packet" => 231,
            "Attn" => 246,
            "Crsel" => 247,
            "Exsel" => 248,
            "EraseEof" => 249,
            "Play" => 250,
            "Zoom" => 251,
            "NoName" => 252,
            "Pa1" => 253,
            "OemClear" => 254,
            _ => return Err(KeyParseError(name.into())),
        };
        Ok(Self(code))
    }
}

/// Generic family flags and explicit modifier sides remain distinct, as in Aldetl.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct Modifiers(u32);

impl Modifiers {
    pub const SHIFT: Self = Self(1 << 0);
    pub const CONTROL: Self = Self(1 << 1);
    pub const ALT: Self = Self(1 << 2);
    pub const CAPS_LOCK: Self = Self(1 << 3);
    pub const LEFT_ALT: Self = Self(1 << 4);
    pub const RIGHT_ALT: Self = Self(1 << 5);
    pub const LEFT_CONTROL: Self = Self(1 << 6);
    pub const RIGHT_CONTROL: Self = Self(1 << 7);
    pub const LEFT_SHIFT: Self = Self(1 << 8);
    pub const RIGHT_SHIFT: Self = Self(1 << 9);

    pub fn contains(self, flag: Self) -> bool {
        self.0 & flag.0 == flag.0
    }

    pub fn with(self, flag: Self) -> Self {
        Self(self.0 | flag.0)
    }

    pub(crate) fn for_key(key: Key) -> Self {
        match key {
            Key::CAPS_LOCK => Self::CAPS_LOCK,
            Key::LEFT_ALT => Self::LEFT_ALT,
            Key::RIGHT_ALT => Self::RIGHT_ALT,
            Key::LEFT_CONTROL => Self::LEFT_CONTROL,
            Key::RIGHT_CONTROL => Self::RIGHT_CONTROL,
            Key::LEFT_SHIFT => Self::LEFT_SHIFT,
            Key::RIGHT_SHIFT => Self::RIGHT_SHIFT,
            _ => Self::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Chord {
    pub key: Key,
    pub modifiers: Modifiers,
}

impl Chord {
    pub fn single(key: Key) -> Self {
        Self {
            key,
            modifiers: Modifiers::default(),
        }
    }

    /// Output modifier order and default sides match Aldetl's KeyAdapter.
    pub fn output_keys(self) -> Result<Vec<Key>, KeyParseError> {
        if self.key == Key::NONE
            && self.modifiers != Modifiers::default()
            && ![Modifiers::CONTROL, Modifiers::ALT, Modifiers::SHIFT].contains(&self.modifiers)
        {
            return Err(KeyParseError("output shortcut requires a final key".into()));
        }
        let mut keys = Vec::new();
        for (generic, left, right, left_key, right_key, default_key) in [
            (
                Modifiers::CONTROL,
                Modifiers::LEFT_CONTROL,
                Modifiers::RIGHT_CONTROL,
                Key::LEFT_CONTROL,
                Key::RIGHT_CONTROL,
                Key::LEFT_CONTROL,
            ),
            (
                Modifiers::ALT,
                Modifiers::LEFT_ALT,
                Modifiers::RIGHT_ALT,
                Key::LEFT_ALT,
                Key::RIGHT_ALT,
                Key::LEFT_ALT,
            ),
            (
                Modifiers::SHIFT,
                Modifiers::LEFT_SHIFT,
                Modifiers::RIGHT_SHIFT,
                Key::LEFT_SHIFT,
                Key::RIGHT_SHIFT,
                Key::RIGHT_SHIFT,
            ),
        ] {
            let has_left = self.modifiers.contains(left);
            let has_right = self.modifiers.contains(right);
            if has_left {
                keys.push(left_key);
            }
            if has_right {
                keys.push(right_key);
            }
            if !has_left && !has_right && self.modifiers.contains(generic) {
                keys.push(default_key);
            }
        }
        if self.key != Key::NONE {
            keys.push(self.key);
        }
        Ok(keys)
    }
}

impl FromStr for Chord {
    type Err = KeyParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let names: Vec<_> = text.split('+').filter(|name| !name.is_empty()).collect();
        let mut chord = Self::single(Key::NONE);
        for name in &names {
            let generic = match *name {
                "Shift" => Some(Modifiers::SHIFT),
                "Control" => Some(Modifiers::CONTROL),
                "Alt" => Some(Modifiers::ALT),
                _ => None,
            };
            chord.key = Key::NONE;
            if let Some(flag) = generic {
                chord.modifiers = chord.modifiers.with(flag);
            } else {
                chord.key = name.parse()?;
                if names.len() > 1 {
                    chord.modifiers = chord.modifiers.with(Modifiers::for_key(chord.key));
                }
            }
        }
        if names.is_empty() {
            return Err(KeyParseError(text.into()));
        }
        Ok(chord)
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self.0 {
            0 => "None",
            8 => "Back",
            9 => "Tab",
            13 => "Enter",
            20 => "CapsLock",
            27 => "Escape",
            32 => "Space",
            33 => "PageUp",
            34 => "PageDown",
            35 => "End",
            36 => "Home",
            37 => "Left",
            38 => "Up",
            39 => "Right",
            40 => "Down",
            45 => "Insert",
            46 => "Delete",
            91 => "LWin",
            92 => "RWin",
            160 => "LShiftKey",
            161 => "RShiftKey",
            162 => "LControlKey",
            163 => "RControlKey",
            164 => "LMenu",
            165 => "RMenu",
            186 => "OemSemicolon",
            187 => "Oemplus",
            188 => "Oemcomma",
            189 => "OemMinus",
            190 => "OemPeriod",
            191 => "OemQuestion",
            192 => "Oemtilde",
            219 => "OemOpenBrackets",
            220 => "OemPipe",
            221 => "OemCloseBrackets",
            222 => "OemQuotes",
            48..=57 => return write!(f, "D{}", self.0 - 48),
            65..=90 => return write!(f, "{}", char::from(self.0 as u8)),
            96..=105 => return write!(f, "NumPad{}", self.0 - 96),
            112..=135 => return write!(f, "F{}", self.0 - 111),
            _ => return write!(f, "{}", self.0),
        };
        f.write_str(name)
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut names = Vec::new();
        for (flag, name) in [
            (Modifiers::CONTROL, "Control"),
            (Modifiers::ALT, "Alt"),
            (Modifiers::SHIFT, "Shift"),
            (Modifiers::CAPS_LOCK, "CapsLock"),
            (Modifiers::LEFT_CONTROL, "LControlKey"),
            (Modifiers::RIGHT_CONTROL, "RControlKey"),
            (Modifiers::LEFT_ALT, "LMenu"),
            (Modifiers::RIGHT_ALT, "RMenu"),
            (Modifiers::LEFT_SHIFT, "LShiftKey"),
            (Modifiers::RIGHT_SHIFT, "RShiftKey"),
        ] {
            if self.modifiers.contains(flag) && flag != Modifiers::for_key(self.key) {
                names.push(name.to_owned());
            }
        }
        if self.key != Key::NONE {
            names.push(self.key.to_string());
        }
        f.write_str(&names.join("+"))
    }
}

#[cfg(test)]
mod formatting_tests {
    use super::*;
    #[test]
    fn key_and_shortcut_names_round_trip_for_profile_persistence() {
        for code in 0..=254 {
            let key = Key::from_code(code).unwrap();
            assert_eq!(key.to_string().parse::<Key>().unwrap(), key);
        }
        for source in [
            "CapsLock",
            "CapsLock+J",
            "Control+Shift+F11",
            "RControlKey+Right",
            "LMenu+LShiftKey+Back",
            "Control",
            "Shift",
            "CapsLock+RShiftKey",
            "RShiftKey+CapsLock",
            "LControlKey+RControlKey+Left",
        ] {
            let chord: Chord = source.parse().unwrap();
            assert_eq!(
                chord.to_string().parse::<Chord>().unwrap(),
                chord,
                "{source}"
            );
        }
    }
}
