use super::*;

#[test]
fn remaining_time_uses_one_clock_and_help_shows_session_duration() {
    let mut h = harness(DomainState::new(TimerConfig::default()).unwrap(), 0);
    press(&mut h.app, ' ');
    h.at.set(1_000);
    h.app.tick();
    for (width, height, large) in [(80, 24, true), (24, 17, false)] {
        let screen = render(&h.app, width, height);
        assert_eq!(screen.contains("REMAINING / 24:59"), !large, "{screen}");
        assert!(!screen.contains("25:00"), "{screen}");
    }
    press(&mut h.app, '?');
    let screen = render(&h.app, 80, 24);
    assert!(!screen.contains("REMAINING"), "{screen}");
    assert!(screen.contains("Session duration: 25:00"), "{screen}");
}

#[test]
fn initial_help_explains_start_task_quick_start_and_auto_start_with_a_visible_gap() {
    let mut h = harness(DomainState::new(TimerConfig::default()).unwrap(), 0);
    press(&mut h.app, '?');
    let screen = render(&h.app, 80, 24);
    for phrase in [
        "HELP · Getting started",
        "Space starts Focus",
        "t edits an optional task; Enter saves, Esc cancels",
        "2 starts two minutes; then f finishes or c starts a full Focus",
        "Completed Focus/breaks auto-start while open",
        "s: Settings",
        "?: Hide help",
        "Session duration: 25:00",
    ] {
        assert!(screen.contains(phrase), "missing {phrase}: {screen}");
    }
    let rows = screen.lines().collect::<Vec<_>>();
    let help = rows.iter().position(|row| row.contains("HELP ·")).unwrap();
    assert!(rows[help - 1].trim_matches(['│', ' ']).is_empty());
    press(&mut h.app, '?');
    let closed = render(&h.app, 80, 24);
    assert!(closed.contains("?: Help"));
    assert!(!closed.contains("HELP ·"));
}

#[test]
fn running_help_explains_the_current_session_without_advertising_unavailable_actions() {
    for (kind, title, note) in [
        (
            SessionKind::Focus,
            "Focus controls",
            "The next break starts automatically",
        ),
        (
            SessionKind::QuickStart,
            "Quick Start controls",
            "At zero: f finishes; c starts a full Focus",
        ),
        (
            SessionKind::ShortBreak,
            "Break controls",
            "Break time is excluded from Work total",
        ),
        (
            SessionKind::LongBreak,
            "Break controls",
            "The next Focus starts automatically",
        ),
    ] {
        let mut h = harness(started(kind), 0);
        press(&mut h.app, '?');
        let screen = render(&h.app, 80, 24);
        assert!(screen.contains(&format!("HELP · {title}")), "{screen}");
        assert!(screen.contains(note), "{screen}");
        assert_eq!(
            screen.contains("d reports a distraction"),
            kind.is_work(),
            "{screen}"
        );
        assert!(!screen.contains("s: Settings"), "{screen}");
    }
}

#[test]
fn interruption_help_explains_return_resume_and_the_time_that_is_excluded() {
    for (kind, note) in [
        (InterruptionKind::Pause, "Paused time is not counted"),
        (InterruptionKind::Distraction, "Space records a Return"),
        (
            InterruptionKind::AppExit,
            "Time while closed is not counted",
        ),
        (
            InterruptionKind::ObservationGap,
            "The timing gap is not counted",
        ),
    ] {
        let mut domain = started(SessionKind::Focus);
        match kind {
            InterruptionKind::Pause => domain
                .apply(Command::Pause(SessionId(1)), Timestamp(0))
                .unwrap(),
            InterruptionKind::Distraction => domain
                .apply(Command::Distraction(SessionId(1)), Timestamp(0))
                .unwrap(),
            InterruptionKind::AppExit => domain.apply(Command::CloseApp, Timestamp(0)).unwrap(),
            InterruptionKind::ObservationGap => domain
                .observe(Observation {
                    previous_at: Timestamp(0),
                    at: Timestamp(10),
                    monotonic_elapsed_ms: None,
                })
                .unwrap(),
        }
        domain.apply(Command::RestoreApp, Timestamp(100)).unwrap();
        let mut h = harness(domain, 100);
        press(&mut h.app, '?');
        let screen = render(&h.app, 80, 24);
        assert!(screen.contains(note), "{screen}");
        assert_eq!(
            screen.contains("Space: Return"),
            kind == InterruptionKind::Distraction
        );
        assert!(!screen.contains("d: Report distraction"), "{screen}");
    }
}

#[test]
fn reset_quick_start_help_explains_restart_instead_of_the_unavailable_two_key() {
    let mut h = harness(started(SessionKind::QuickStart), 0);
    press(&mut h.app, 'r');
    press(&mut h.app, '?');
    let screen = render(&h.app, 80, 24);
    assert!(screen.contains("HELP · Restart Quick Start"), "{screen}");
    assert!(screen.contains("Space restarts two minutes"), "{screen}");
    assert!(!screen.contains("2 starts"), "{screen}");
    assert!(!screen.contains("2: Quick Start"), "{screen}");
}
