//! Language-independent diagnostic data. Display retains English for logs;
//! presentation supplies a catalog without making the core depend on it.
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Message {
    Literal(String),
    Template {
        id: &'static str,
        english: &'static str,
        arguments: Vec<(&'static str, Message)>,
    },
    Joined(Vec<Message>, String),
}

impl Message {
    pub const fn new(
        id: &'static str,
        english: &'static str,
        arguments: Vec<(&'static str, Self)>,
    ) -> Self {
        Self::Template {
            id,
            english,
            arguments,
        }
    }
    pub fn join(values: impl IntoIterator<Item = Self>, separator: &str) -> Self {
        Self::Joined(values.into_iter().collect(), separator.into())
    }
    pub fn value(value: &(impl MessageValue + ?Sized)) -> Self {
        value.message_value()
    }
    pub fn render_with<'a>(&self, lookup: &impl Fn(&str) -> Option<&'a str>) -> String {
        match self {
            Self::Literal(value) => value.clone(),
            Self::Joined(values, separator) => values
                .iter()
                .map(|v| v.render_with(lookup))
                .collect::<Vec<_>>()
                .join(separator),
            Self::Template {
                id,
                english,
                arguments,
            } => {
                let template = lookup(id).unwrap_or(english);
                let mut result = String::new();
                let mut remaining = template;
                while !remaining.is_empty() {
                    if let Some(tail) = remaining.strip_prefix("{{") {
                        result.push('{');
                        remaining = tail;
                    } else if let Some(tail) = remaining.strip_prefix("}}") {
                        result.push('}');
                        remaining = tail;
                    } else if let Some(tail) = remaining.strip_prefix('{') {
                        if let Some(end) = tail.find('}') {
                            let name = &tail[..end];
                            if let Some((_, value)) = arguments.iter().find(|(key, _)| *key == name)
                            {
                                result.push_str(&value.render_with(lookup));
                            } else {
                                result.push('{');
                                result.push_str(name);
                                result.push('}');
                            }
                            remaining = &tail[end + 1..];
                        } else {
                            result.push_str(remaining);
                            break;
                        }
                    } else {
                        let character = remaining.chars().next().unwrap();
                        result.push(character);
                        remaining = &remaining[character.len_utf8()..];
                    }
                }
                result
            }
        }
    }
    pub fn contains(&self, pattern: &str) -> bool {
        self.to_string().contains(pattern)
    }
    pub fn starts_with(&self, pattern: &str) -> bool {
        self.to_string().starts_with(pattern)
    }
    pub fn is_empty(&self) -> bool {
        self.to_string().is_empty()
    }
    pub fn argument(&self, name: &str) -> Option<&Message> {
        match self {
            Self::Template { arguments, .. } => arguments
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value),
            _ => None,
        }
    }
    pub fn has_id(&self, wanted: &str) -> bool {
        match self {
            Self::Template { id, arguments, .. } => {
                *id == wanted || arguments.iter().any(|(_, value)| value.has_id(wanted))
            }
            Self::Joined(values, _) => values.iter().any(|value| value.has_id(wanted)),
            Self::Literal(_) => false,
        }
    }
}
impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render_with(&|_| None))
    }
}
impl std::error::Error for Message {}
impl Default for Message {
    fn default() -> Self {
        Self::Literal(String::new())
    }
}
impl From<String> for Message {
    fn from(value: String) -> Self {
        Self::Literal(value)
    }
}
impl From<&str> for Message {
    fn from(value: &str) -> Self {
        Self::Literal(value.into())
    }
}
impl From<&String> for Message {
    fn from(value: &String) -> Self {
        Self::Literal(value.clone())
    }
}
impl From<&Message> for Message {
    fn from(value: &Message) -> Self {
        value.clone()
    }
}
impl PartialEq<&str> for Message {
    fn eq(&self, other: &&str) -> bool {
        self.render_with(&|_| None).as_str() == *other
    }
}
pub trait JoinMessages {
    fn join(&self, separator: &str) -> Message;
}
impl JoinMessages for Vec<Message> {
    fn join(&self, separator: &str) -> Message {
        Message::join(self.iter().cloned(), separator)
    }
}

pub trait MessageValue {
    fn message_value(&self) -> Message;
}
impl MessageValue for Message {
    fn message_value(&self) -> Message {
        self.clone()
    }
}
impl<T: MessageValue + ?Sized> MessageValue for &T {
    fn message_value(&self) -> Message {
        (*self).message_value()
    }
}
macro_rules! literal_values { ($($kind:ty),*) => { $(impl MessageValue for $kind { fn message_value(&self) -> Message { self.to_string().into() } })* }; }
literal_values!(
    str,
    String,
    bool,
    char,
    usize,
    isize,
    u8,
    u16,
    u32,
    u64,
    i8,
    i16,
    i32,
    i64,
    std::io::Error,
    std::string::FromUtf8Error,
    std::str::Utf8Error
);
impl MessageValue for std::path::Display<'_> {
    fn message_value(&self) -> Message {
        self.to_string().into()
    }
}
impl MessageValue for std::borrow::Cow<'_, str> {
    fn message_value(&self) -> Message {
        self.to_string().into()
    }
}

/// Stable identity plus literal/nested arguments; no selected language is read.
#[macro_export]
macro_rules! msg {
    ($id:literal, $english:literal $(, $name:ident = $value:expr)* $(,)?) => {
        $crate::Message::new($id, $english, vec![$((stringify!($name), $crate::Message::value(&$value))),*])
    };
}
