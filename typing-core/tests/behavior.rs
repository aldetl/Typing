use std::{collections::VecDeque, time::Duration};
use typing_core::*;

fn key(name: &str) -> Key {
    name.parse().unwrap()
}
fn chord(name: &str) -> Chord {
    name.parse().unwrap()
}
fn local() -> LocalDateTime<'static> {
    LocalDateTime {
        year: 2026,
        month: 11,
        day: 7,
        hour: 13,
        minute: 5,
        am: "AM",
        pm: "PM",
    }
}

struct Host {
    core: Core,
    queue: VecDeque<Request>,
    now: Duration,
}

impl Host {
    fn new(configuration: &str) -> Self {
        Self {
            core: Core::new(Config::parse(configuration).unwrap()),
            queue: VecDeque::new(),
            now: Duration::ZERO,
        }
    }
    fn event(&mut self, name: &str, direction: Direction) -> bool {
        let decision = self.core.input(
            KeyEvent {
                key: key(name),
                direction,
            },
            self.now,
        );
        if let Some(request) = decision.request {
            self.queue.push_back(request);
        }
        decision.consume
    }
    fn tap(&mut self, name: &str) {
        self.event(name, Direction::Down);
        self.event(name, Direction::Up);
    }
    fn run(&mut self) -> Action {
        let execution = self
            .core
            .resolve(self.queue.pop_front().unwrap(), local())
            .unwrap();
        let action = execution.action().clone();
        self.core.complete(execution, Outcome::Success).unwrap();
        action
    }
    fn timeout(&mut self, seconds: u64) {
        self.now = Duration::from_secs(seconds);
        if let Some(request) = self.core.check_inactivity(self.now) {
            self.queue.push_back(request);
        }
    }
}

const CONFIG: &str = "[common]\nmode_auto_exit=on\nmode_auto_exit_time=2\n[modekeys]\nCapsLock=mode\nLMenu=mode\nJ=Left\nL=Right\nI=Up\nA=step10\nF=step1\nV=shift_onoff\nQ=undo\nG=exit\nU=Home\nY=date\nT=time\nX=switch_win2\nB=disable\n[hotkeys]\nCapsLock+J=Left\nLMenu+C=Control+C\n";

fn expected_keys(names: &[&str], count: u32) -> Action {
    Action::Keys {
        keys: names.iter().map(|name| key(name)).collect(),
        count,
        delay: Duration::ZERO,
    }
}

#[test]
fn installed_configuration_and_host_settings_are_supported() {
    let config = Config::parse(include_str!("fixtures/aldetl-installed.ini")).unwrap();
    assert_eq!(config.global_hotkey, Some(chord("Control+U")));
    assert_eq!(config.auto_exit_seconds, 2);
    assert_eq!(config.common["tip_radius"], "15");
    assert!(config.mode_keys().contains(&Key::LEFT_ALT));
    assert_eq!(
        config.binding(chord("CapsLock+Q")).unwrap().command,
        Command::Undo
    );
    assert_eq!(
        config.binding(chord("LMenu+Oemcomma")).unwrap().group,
        BindingGroup::Always
    );
}

#[test]
fn keys_and_modifier_names_preserve_identity_and_output_order() {
    assert_eq!(chord("Enter"), chord("Return"));
    assert_eq!(chord("PageUp"), chord("Prior"));
    assert_ne!(chord("LShiftKey+J"), chord("RShiftKey+J"));
    assert_eq!(
        chord("Control+Shift+Alt+F24").output_keys().unwrap(),
        vec![
            Key::LEFT_CONTROL,
            Key::LEFT_ALT,
            Key::RIGHT_SHIFT,
            key("F24")
        ]
    );
    assert_eq!(
        chord("Shift+LShiftKey+Left").output_keys().unwrap(),
        vec![Key::LEFT_SHIFT, Key::LEFT]
    );
    assert_eq!(
        chord("Control").output_keys().unwrap(),
        vec![Key::LEFT_CONTROL]
    );
    assert!(chord("Control+Shift").output_keys().is_err());
    assert_eq!("65".parse::<Key>().unwrap(), key("A"));
    assert_eq!(chord("Oem102"), chord("OemBackslash"));
    assert!("j".parse::<Key>().is_err());
}

#[test]
fn configured_mode_taps_and_bindings_work_after_remapping() {
    for (mode, movement, step, exit) in [("CapsLock", "J", "A", "G"), ("Escape", "Z", "D8", "F12")]
    {
        let config =
            format!("[modekeys]\n{mode}=mode\n{movement}=Left\n{step}=step7\n{exit}=exit\n");
        let mut host = Host::new(&config);
        assert!(!host.event(movement, Direction::Down));
        host.event(movement, Direction::Up);
        assert!(host.event(mode, Direction::Down));
        assert!(!host.event(mode, Direction::Up));
        assert!(host.core.command_mode());
        host.tap(step);
        host.run();
        host.tap(movement);
        assert_eq!(host.run(), expected_keys(&["Left"], 7));
        host.tap(exit);
        host.run();
        assert!(!host.core.command_mode());
        assert!(!host.event("B", Direction::Down));
    }
}

#[test]
fn a_chord_cancels_the_mode_gesture_and_repeat_remains_admissible() {
    let mut host = Host::new(CONFIG);
    assert!(host.event("CapsLock", Direction::Down));
    assert!(host.event("J", Direction::Down));
    assert!(host.event("J", Direction::Down));
    assert!(!host.event("J", Direction::Up));
    host.event("CapsLock", Direction::Up);
    assert!(!host.core.command_mode());
    assert_eq!(host.run(), expected_keys(&["Left"], 1));
    assert_eq!(host.run(), expected_keys(&["Left"], 1));
    assert!(host.queue.is_empty());
}

#[test]
fn another_held_key_prevents_an_isolated_mode_tap() {
    let mut host = Host::new(CONFIG);
    host.event("B", Direction::Down);
    host.tap("LMenu");
    assert!(!host.core.command_mode());
    host.event("B", Direction::Up);
    host.tap("LMenu");
    assert!(host.core.command_mode());
}

#[test]
fn mode_key_caps_lock_suppresses_unmatched_overlaps_and_stale_keys() {
    let default = include_str!("../../resources/profiles/default.typing.ini");
    for initially_on in [false, true] {
        for held in [
            "LControlKey",
            "RControlKey",
            "LShiftKey",
            "RShiftKey",
            "LMenu",
            "RMenu",
            "B",
            "Space",
        ] {
            let mut host = Host::new(default);
            if initially_on {
                host.tap("CapsLock");
            }
            host.event(held, Direction::Down);
            for _ in 0..2 {
                assert!(host.event("CapsLock", Direction::Down), "held={held}");
            }
            host.event("CapsLock", Direction::Up);
            assert_eq!(host.core.command_mode(), initially_on);
            // Omit the held key's release to exercise the same suppression
            // when the observation is stale rather than physically held.
            assert!(host.event("CapsLock", Direction::Down));
            host.event("CapsLock", Direction::Up);
            assert_eq!(host.core.command_mode(), initially_on);
            assert!(host.queue.is_empty());
            host.event(held, Direction::Up);
            host.tap("CapsLock");
            assert_eq!(host.core.command_mode(), !initially_on);

            if held != "Space" {
                let mut host = Host::new(default);
                if initially_on {
                    host.tap("CapsLock");
                }
                assert!(host.event("CapsLock", Direction::Down));
                host.event(held, Direction::Down);
                assert!(host.event("CapsLock", Direction::Down));
                host.event(held, Direction::Up);
                host.event("CapsLock", Direction::Up);
                assert_eq!(host.core.command_mode(), initially_on);
                assert!(host.queue.is_empty());
            }
        }
    }
}

#[test]
fn caps_space_output_toggles_caps_without_changing_command_mode() {
    for action in ["CapsLock", "capslock"] {
        for initially_on in [false, true] {
            let mut host = Host::new(&format!(
                "[modekeys]\nCapsLock=mode\n[hotkeys]\nCapsLock+Space={action}\n"
            ));
            if initially_on {
                host.tap("CapsLock");
            }
            assert!(host.event("CapsLock", Direction::Down));
            assert!(host.event("Space", Direction::Down));
            host.event("Space", Direction::Up);
            host.event("CapsLock", Direction::Up);
            assert_eq!(host.core.command_mode(), initially_on);
            assert_eq!(host.run(), expected_keys(&["CapsLock"], 1));
            assert_eq!(host.core.command_mode(), initially_on);
            assert!(host.queue.is_empty());
        }
    }
}

#[test]
fn caps_suppression_respects_mode_configuration_pause_and_explicit_bindings() {
    let mut ordinary = Host::new("[modekeys]\nEscape=mode\n");
    assert!(!ordinary.event("CapsLock", Direction::Down));
    ordinary.event("CapsLock", Direction::Up);
    ordinary.event("LControlKey", Direction::Down);
    assert!(!ordinary.event("CapsLock", Direction::Down));
    let mut paused = Host::new(CONFIG);
    paused.core.pause();
    assert!(!paused.event("CapsLock", Direction::Down));
    let mut custom = Host::new("[modekeys]\nCapsLock=mode\n[hotkeys]\nLControlKey+CapsLock=Left\n");
    custom.event("LControlKey", Direction::Down);
    assert!(custom.event("CapsLock", Direction::Down));
    custom.event("CapsLock", Direction::Up);
    custom.event("LControlKey", Direction::Up);
    assert!(!custom.core.command_mode());
    assert_eq!(custom.run(), expected_keys(&["Left"], 1));
}

#[test]
fn movement_pair_counts_history_and_selection_match_aldetl() {
    let mut host = Host::new(CONFIG);
    host.tap("CapsLock");
    host.tap("A");
    host.tap("J");
    host.tap("L");
    assert_eq!(host.run(), Action::None);
    assert_eq!(host.run(), expected_keys(&["Left"], 10));
    assert_eq!(host.run(), expected_keys(&["Right"], 10));
    host.tap("Q");
    assert_eq!(host.run(), expected_keys(&["Left"], 10));
    host.tap("V");
    host.run();
    host.tap("I");
    assert_eq!(host.run(), expected_keys(&["RShiftKey", "Up"], 1));
    host.tap("Q");
    assert_eq!(host.run(), expected_keys(&["Down"], 1));
}

#[test]
fn home_consumes_pending_step_and_generic_chords_ignore_selection() {
    let config = format!("{CONFIG}\n[modekeys]\nR=RShiftKey+Left\n");
    let mut host = Host::new(&config);
    host.tap("CapsLock");
    host.tap("A");
    host.run();
    host.tap("U");
    assert_eq!(host.run(), expected_keys(&["Home"], 1));
    host.tap("J");
    assert_eq!(host.run(), expected_keys(&["Left"], 1));
    host.tap("V");
    host.run();
    host.tap("R");
    assert_eq!(host.run(), expected_keys(&["RShiftKey", "Left"], 1));
}

#[test]
fn mode_exit_resets_queued_movement_at_resolution() {
    let mut host = Host::new(CONFIG);
    host.tap("CapsLock");
    host.tap("A");
    host.run();
    host.tap("J");
    host.tap("CapsLock");
    assert!(!host.core.command_mode());
    assert_eq!(host.run(), expected_keys(&["Left"], 1));
}

#[test]
fn active_movement_retains_its_plan_and_completes_into_current_history() {
    let mut host = Host::new(CONFIG);
    host.tap("CapsLock");
    host.tap("A");
    host.run();
    host.tap("J");
    let execution = host
        .core
        .resolve(host.queue.pop_front().unwrap(), local())
        .unwrap();
    assert_eq!(execution.action(), &expected_keys(&["Left"], 10));
    host.tap("CapsLock");
    host.tap("CapsLock"); // Exit and re-enter while output runs.
    host.core.complete(execution, Outcome::Success).unwrap();
    host.tap("Q");
    assert_eq!(host.run(), expected_keys(&["Right"], 10));
    host.tap("J");
    assert_eq!(host.run(), expected_keys(&["Left"], 1));
}

#[test]
fn queued_exit_and_later_admitted_command_both_execute() {
    let mut host = Host::new(CONFIG);
    host.tap("CapsLock");
    host.tap("A");
    host.run();
    host.tap("J");
    host.timeout(3);
    host.tap("L");
    assert!(host.core.command_mode());
    assert_eq!(host.run(), expected_keys(&["Left"], 10));
    assert_eq!(host.run(), Action::None);
    assert!(!host.core.command_mode());
    assert_eq!(host.run(), expected_keys(&["Right"], 1));
    assert!(!host.event("J", Direction::Down));
}

#[test]
fn queued_timeout_survives_new_activity_and_mode_reentry() {
    let mut host = Host::new(CONFIG);
    host.tap("CapsLock");
    host.timeout(3);
    host.tap("CapsLock");
    host.tap("CapsLock");
    host.tap("J");
    assert!(host.core.command_mode());
    host.run();
    assert!(!host.core.command_mode());
    assert_eq!(host.run(), expected_keys(&["Left"], 1));
}

#[test]
fn whole_second_timeout_and_binding_group_refresh_are_preserved() {
    let mut host = Host::new(CONFIG);
    host.tap("CapsLock");
    host.timeout(2);
    assert!(host.queue.is_empty());
    assert!(
        host.core
            .check_inactivity(Duration::from_millis(2999))
            .is_none()
    );
    host.now = Duration::from_secs(2);
    host.tap("J");
    host.run(); // Refreshes to 2.
    host.event("CapsLock", Direction::Down);
    host.event("J", Direction::Down); // Always binding does not refresh.
    host.event("J", Direction::Up);
    host.event("CapsLock", Direction::Up);
    host.timeout(4);
    assert_eq!(host.queue.len(), 1);
    host.timeout(5);
    host.timeout(8); // A second timeout can queue before the first runs.
    assert_eq!(host.queue.len(), 3);
}

#[test]
fn failed_and_cancelled_movements_do_not_enter_history() {
    for outcome in [Outcome::Failed, Outcome::Cancelled] {
        let mut host = Host::new(CONFIG);
        host.tap("CapsLock");
        host.tap("A");
        host.run();
        host.tap("J");
        let execution = host
            .core
            .resolve(host.queue.pop_front().unwrap(), local())
            .unwrap();
        host.core.complete(execution, outcome).unwrap();
        host.tap("Q");
        assert_eq!(host.run(), Action::None);
        host.tap("L");
        assert_eq!(host.run(), expected_keys(&["Right"], 10));
    }
}

#[test]
fn pause_and_restart_gate_work_and_discard_stale_mode_observations() {
    let mut host = Host::new(CONFIG);
    host.tap("CapsLock");
    host.tap("J");
    host.tap("L");
    let execution = host
        .core
        .resolve(host.queue.pop_front().unwrap(), local())
        .unwrap();
    host.event("CapsLock", Direction::Down);
    host.core.pause();
    host.queue.clear();
    assert_eq!(host.core.restart([]), Err(CoreError::Busy));
    assert!(!host.event("J", Direction::Down));
    host.core.complete(execution, Outcome::Cancelled).unwrap();
    host.core.restart([Key::LEFT_CONTROL]).unwrap();
    host.event("CapsLock", Direction::Up);
    host.tap("CapsLock"); // Another baseline key is still held.
    assert!(!host.core.command_mode());
    host.event("LControlKey", Direction::Up);
    host.tap("CapsLock");
    assert!(host.core.command_mode());
}

#[test]
fn date_time_and_host_controls_have_executable_actions() {
    let mut host = Host::new(CONFIG);
    host.tap("CapsLock");
    host.tap("Y");
    host.tap("T");
    host.tap("B");
    assert_eq!(host.run(), Action::Text("2026-11-07".into()));
    assert_eq!(host.run(), Action::Text("01:05 PM".into()));
    assert_eq!(host.run(), Action::HostControl(HostControl::Disable));
    host.tap("X");
    let execution = host
        .core
        .resolve(host.queue.pop_front().unwrap(), local())
        .unwrap();
    assert_eq!(
        execution.action(),
        &Action::Keys {
            keys: vec![Key::LEFT_ALT, Key::TAB],
            count: 2,
            delay: Duration::from_millis(100)
        }
    );
    assert!(host.core.command_mode());
    host.core.complete(execution, Outcome::Success).unwrap();
    assert!(!host.core.command_mode());
}

#[test]
fn configuration_precedence_includes_simple_command_dispatch() {
    let config = Config::parse("[hotkeys]\nJ=step3\n[modekeys]\nCapsLock=mode\nJ=date\n").unwrap();
    assert_eq!(
        config.binding(chord("J")).unwrap().group,
        BindingGroup::Always
    );
    assert_eq!(config.binding(chord("J")).unwrap().command, Command::Date);
    let config = Config::parse("[modekeys]\nCapsLock=mode\nJ=Left\nJ=Right\n").unwrap();
    assert_eq!(
        config.binding(chord("J")).unwrap().command,
        Command::Movement(chord("Right"))
    );
}

#[test]
fn invalid_configuration_returns_line_diagnostics() {
    let errors =
        Config::parse("[modekeys]\nnot-a-key=Left\nJ=unknown-command\nA=step0\n").unwrap_err();
    assert_eq!(
        errors.iter().map(|error| error.line).collect::<Vec<_>>(),
        vec![2, 3, 4]
    );
}

#[test]
fn empty_mode_section_retains_legacy_constructor_dispatch_precedence() {
    let config = Config::parse("[modekeys]\n[hotkeys]\nJ=step3\n").unwrap();
    assert_eq!(
        config.binding(chord("J")).unwrap().command,
        Command::Movement(chord("Left"))
    );
    let config = Config::parse("[modekeys]\n[hotkeys]\nA=date\n").unwrap();
    assert_eq!(config.binding(chord("A")).unwrap().command, Command::Date);
}

#[test]
fn explicit_modifier_sides_match_exactly_like_aldetl() {
    let mut host = Host::new(
        "[modekeys]\nEscape=mode\n[hotkeys]\nLControlKey+Z=Control+Left\nRControlKey+Z=Control+Right\n",
    );
    host.event("LControlKey", Direction::Down);
    host.tap("Z");
    host.event("LControlKey", Direction::Up);
    host.event("RControlKey", Direction::Down);
    host.tap("Z");
    host.event("RControlKey", Direction::Up);
    assert_eq!(host.run(), expected_keys(&["LControlKey", "Left"], 1));
    assert_eq!(host.run(), expected_keys(&["LControlKey", "Right"], 1));
}

#[test]
fn failed_window_switch_keeps_mode_and_midnight_uses_twelve() {
    let mut host = Host::new(CONFIG);
    host.tap("CapsLock");
    host.tap("X");
    let execution = host
        .core
        .resolve(host.queue.pop_front().unwrap(), local())
        .unwrap();
    host.core.complete(execution, Outcome::Failed).unwrap();
    assert!(host.core.command_mode());
    host.tap("T");
    let midnight = LocalDateTime { hour: 0, ..local() };
    let execution = host
        .core
        .resolve(host.queue.pop_front().unwrap(), midnight)
        .unwrap();
    assert_eq!(execution.action(), &Action::Text("12:05 AM".into()));
    host.core.complete(execution, Outcome::Success).unwrap();
}

#[test]
fn completion_handles_belong_to_the_originating_core() {
    let mut first = Host::new(CONFIG);
    let mut second = Host::new(CONFIG);
    first.tap("CapsLock");
    first.tap("J");
    second.tap("CapsLock");
    second.tap("L");
    let first_execution = first
        .core
        .resolve(first.queue.pop_front().unwrap(), local())
        .unwrap();
    let second_execution = second
        .core
        .resolve(second.queue.pop_front().unwrap(), local())
        .unwrap();
    assert_eq!(
        first.core.complete(second_execution, Outcome::Success),
        Err(CoreError::ForeignExecution)
    );
    first
        .core
        .complete(first_execution, Outcome::Success)
        .unwrap();
    first.tap("Q");
    assert_eq!(first.run(), expected_keys(&["Right"], 1));
}

#[test]
fn word_and_page_pairs_retain_independent_counts() {
    let mut host = Host::new(
        "[modekeys]\nEscape=mode\nZ=step4\nX=left_word\nC=right_word\nB=PageUp\nN=PageDown\nV=undo\n",
    );
    host.tap("Escape");
    host.tap("Z");
    host.run();
    host.tap("X");
    host.tap("B");
    host.tap("C");
    host.tap("N");
    assert_eq!(host.run(), expected_keys(&["LControlKey", "Left"], 4));
    assert_eq!(host.run(), expected_keys(&["PageUp"], 1));
    assert_eq!(host.run(), expected_keys(&["LControlKey", "Right"], 4));
    assert_eq!(host.run(), expected_keys(&["PageDown"], 1));
    host.tap("V");
    assert_eq!(host.run(), expected_keys(&["PageUp"], 1));
}
