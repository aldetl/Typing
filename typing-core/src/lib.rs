//! Shared Typing logic. Hosts serialize calls to [`Core`] and execute output
//! without holding access to it. Own tagged events bypass [`Core::input`].
//!
//! A host queues the requests returned by `input` and `check_inactivity` in FIFO
//! order. Its single worker calls `resolve`, executes the returned action, then
//! calls `complete`, including on failure or cancellation. Mode gestures may
//! occur between those calls. Pause closes admission. Before `restart`, the host
//! discards its queue and completes active output and cleanup.
//!
//! ```
//! use std::{collections::VecDeque, time::Duration};
//! use typing_core::*;
//!
//! let config = Config::parse("[modekeys]\nEscape=mode\nZ=Left\n").unwrap();
//! let mut core = Core::new(config);
//! let mut queue = VecDeque::new();
//! for (name, direction) in [("Escape", Direction::Down), ("Escape", Direction::Up),
//!                           ("Z", Direction::Down), ("Z", Direction::Up)] {
//!     let decision = core.input(KeyEvent { key: name.parse().unwrap(), direction }, Duration::ZERO);
//!     if let Some(request) = decision.request { queue.push_back(request); }
//! }
//! let clock = LocalDateTime { year: 2026, month: 11, day: 7, hour: 12, minute: 0,
//!                             am: "AM", pm: "PM" };
//! let execution = core.resolve(queue.pop_front().unwrap(), clock).unwrap();
//! assert_eq!(execution.action(), &Action::Keys {
//!     keys: vec![Key::LEFT], count: 1, delay: Duration::ZERO,
//! });
//! // The host submits this output, then supplies the actual outcome.
//! core.complete(execution, Outcome::Success).unwrap();
//! ```

mod config;
mod engine;
mod keys;
mod message;
pub use message::{JoinMessages, Message, MessageValue};

pub use config::{Binding, BindingGroup, Command, Config, ConfigError, validate_binding};
pub use engine::{
    Action, Core, CoreError, Decision, Direction, Execution, HostControl, KeyEvent, LocalDateTime,
    Outcome, Request,
};
pub use keys::{Chord, Key, KeyParseError, Modifiers};
