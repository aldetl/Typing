use crate::inputsim::{Character, DOWN, Event, UP};
use typing_core::Key;

pub trait Backend {
    fn key(&mut self, key: Key) -> Result<Event, String>;
    fn character(&mut self, character: char) -> Result<Character, String>;
    fn submit(&mut self, event: Event, direction: u32) -> Result<(), String>;
}

/// The worker retains these exact values until their releases succeed.
pub struct Output<B> {
    pub backend: B,
    held: Vec<Event>,
    failed: Vec<Event>,
}

impl<B: Backend> Output<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            held: Vec::new(),
            failed: Vec::new(),
        }
    }
    #[cfg(test)]
    pub fn pending(&self) -> usize {
        self.held.len()
    }
    pub fn held_events(&self) -> &[Event] {
        &self.held
    }
    pub fn begin(&mut self) -> Result<(), String> {
        if !self.held.is_empty() {
            return Err("outstanding releases prevent output".into());
        }
        self.failed.clear();
        Ok(())
    }
    pub fn down(&mut self, event: Event) -> Result<(), String> {
        if self.held.contains(&event) {
            return Ok(());
        }
        self.backend.submit(event, DOWN)?;
        self.held.push(event);
        Ok(())
    }
    pub fn up(&mut self, event: Event) -> Result<(), String> {
        let Some(index) = self.held.iter().rposition(|held| *held == event) else {
            return Ok(());
        };
        match self.backend.submit(event, UP) {
            Ok(()) => {
                self.held.remove(index);
                Ok(())
            }
            Err(error) => {
                self.failed.push(event);
                Err(error)
            }
        }
    }
    /// Cleanup attempts every remaining release once, preserving failed values.
    /// Only an explicit lifecycle retry includes a release already rejected.
    pub fn cleanup(&mut self, explicit_retry: bool) -> Result<(), String> {
        if explicit_retry {
            self.failed.clear();
        }
        let mut errors = Vec::new();
        for event in self.held.clone().into_iter().rev() {
            if self.failed.contains(&event) {
                continue;
            }
            if let Err(error) = self.up(event) {
                errors.push(error);
            }
        }
        if self.held.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "{} outstanding releases, {}",
                self.held.len(),
                errors.join(", ")
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    struct Fake {
        outcomes: VecDeque<bool>,
        calls: Vec<(Event, u32)>,
    }
    impl Backend for Fake {
        fn key(&mut self, _: Key) -> Result<Event, String> {
            unreachable!()
        }
        fn character(&mut self, _: char) -> Result<Character, String> {
            unreachable!()
        }
        fn submit(&mut self, event: Event, direction: u32) -> Result<(), String> {
            self.calls.push((event, direction));
            if self.outcomes.pop_front().unwrap_or(true) {
                Ok(())
            } else {
                Err("rejected".into())
            }
        }
    }
    fn event(code: u32) -> Event {
        Event {
            kind: 1,
            code,
            flags: 0,
            reserved: 0,
        }
    }
    #[test]
    fn rejected_down_creates_no_release() {
        let mut output = Output::new(Fake {
            outcomes: [false].into(),
            calls: vec![],
        });
        assert!(output.down(event(42)).is_err());
        assert_eq!(output.pending(), 0);
        output.cleanup(false).unwrap();
        assert_eq!(output.backend.calls.len(), 1);
    }
    #[test]
    fn failed_up_is_retained_and_only_explicitly_retried() {
        let key = event(42);
        let mut output = Output::new(Fake {
            outcomes: [true, false, true].into(),
            calls: vec![],
        });
        output.down(key).unwrap();
        assert!(output.up(key).is_err());
        assert_eq!(output.pending(), 1);
        assert!(output.cleanup(false).is_err());
        assert!(output.begin().is_err());
        assert_eq!(output.backend.calls.len(), 2);
        output.cleanup(true).unwrap();
        assert_eq!(output.pending(), 0);
        assert_eq!(output.backend.calls, [(key, DOWN), (key, UP), (key, UP)]);
    }
    #[test]
    fn partial_cleanup_attempts_all_keys_in_reverse_order() {
        let mut output = Output::new(Fake {
            outcomes: [true, true, false, true, true].into(),
            calls: vec![],
        });
        output.down(event(42)).unwrap();
        output.down(event(75)).unwrap();
        assert!(output.cleanup(false).is_err());
        assert_eq!(output.pending(), 1);
        assert_eq!(
            &output.backend.calls[2..],
            &[(event(75), UP), (event(42), UP)]
        );
        output.cleanup(true).unwrap();
        assert_eq!(output.pending(), 0);
    }
}
