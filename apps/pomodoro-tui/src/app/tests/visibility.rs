use super::*;

fn has(screen: &str, phrase: &str) -> bool {
    let compact = |text: &str| {
        text.chars()
            .filter(|ch| !ch.is_whitespace() && !matches!(ch, '│' | '─' | '┌' | '┐' | '└' | '┘'))
            .collect::<String>()
    };
    compact(screen).contains(&compact(phrase))
}

#[test]
fn narrow_screen_shows_state_specific_controls_and_save_failure_choices() {
    let mut h = harness(ready(), 0);
    let ready = render(&h.app, 30, 25);
    assert!(has(&ready, "History"), "{ready}");
    for phrase in [
        "Space: Start",
        "t: Edit task",
        "2: Quick Start",
        "s: Settings",
    ] {
        assert!(has(&ready, phrase), "missing {phrase}: {ready}");
    }
    press(&mut h.app, '2');
    let running = render(&h.app, 30, 25);
    for phrase in [
        "Space: Pause",
        "d: Report distraction",
        "x: Cancel",
        "q: Save & quit",
    ] {
        assert!(has(&running, phrase), "missing {phrase}: {running}");
    }
    h.at.set(100);
    press(&mut h.app, 'd');
    let distracted = render(&h.app, 30, 25);
    for phrase in [
        "Distracted",
        "Space: Return",
        "x: Cancel",
        "Distraction saved",
    ] {
        assert!(has(&distracted, phrase), "missing {phrase}: {distracted}");
    }

    h.failures.set(1);
    press(&mut h.app, ' ');
    let blocked = render(&h.app, 30, 25);
    for phrase in [
        "Save pending",
        "r: Retry save",
        "Q: Confirm unsaved exit",
        "Could not confirm save",
    ] {
        assert!(has(&blocked, phrase), "missing {phrase}: {blocked}");
    }
    press(&mut h.app, 'Q');
    let confirming = render(&h.app, 30, 25);
    for phrase in ["Exit without saving", "y: Exit unsaved", "n / Esc: Back"] {
        assert!(has(&confirming, phrase), "missing {phrase}: {confirming}");
    }
}

#[test]
fn narrow_quick_start_decision_shows_choices_and_completion() {
    let mut h = harness(ready(), 0);
    press(&mut h.app, '2');
    for at in (1_000..=120_000).step_by(1_000) {
        h.at.set(at);
        h.app.tick();
    }
    let decision = render(&h.app, 30, 25);
    for phrase in [
        "Quick Start · Choose finish",
        "f: Finish Quick Start",
        "c: Continue to Focus",
        "Quick Start complete",
    ] {
        assert!(has(&decision, phrase), "missing {phrase}: {decision}");
    }
    press(&mut h.app, '?');
    let help = render(&h.app, 30, 25);
    assert!(has(&help, "Settings unavailable"), "{help}");
    assert!(has(&help, "Quick Start complete"), "{help}");
}

#[test]
fn tight_terminal_keeps_operation_and_save_recovery_controls_visible() {
    for (width, height) in [(30, 17), (24, 17), (24, 20)] {
        let mut h = harness(ready(), 0);
        press(&mut h.app, '2');
        let running = render(&h.app, width, height);
        for phrase in ["Space: Pause", "d: Report distraction", "x: Cancel"] {
            assert!(has(&running, phrase), "missing {phrase}: {running}");
        }
        h.at.set(100);
        h.failures.set(1);
        press(&mut h.app, 'd');
        let blocked = render(&h.app, width, height);
        for phrase in [
            "r: Retry save",
            "Q: Confirm unsaved exit",
            "Could not confirm save",
        ] {
            assert!(has(&blocked, phrase), "missing {phrase}: {blocked}");
        }
        if (width, height) == (24, 20) {
            assert!(!has(&blocked, "History"), "{blocked}");
            assert!(has(&blocked, "state.json"), "{blocked}");
        }
        press(&mut h.app, 'Q');
        let confirming = render(&h.app, width, height);
        assert!(has(&confirming, "y: Exit unsaved"), "{confirming}");
    }
}

#[test]
fn short_terminal_shows_quick_start_choices_and_help() {
    let mut h = harness(awaiting(), 120_000);
    press(&mut h.app, '?');
    let choice = render(&h.app, 24, 17);
    for phrase in [
        "f: Finish Quick Start",
        "c: Continue to Focus",
        "Settings unavailable",
    ] {
        assert!(has(&choice, phrase), "missing {phrase}: {choice}");
    }
}

#[test]
fn short_terminal_keeps_completion_message_with_help() {
    let mut h = harness(ready(), 0);
    press(&mut h.app, '2');
    for at in (1_000..=120_000).step_by(1_000) {
        h.at.set(at);
        h.app.tick();
    }
    press(&mut h.app, '?');
    let screen = render(&h.app, 24, 17);
    for phrase in [
        "f: Finish Quick Start",
        "c: Continue to Focus",
        "Settings unavailable",
        "Quick Start complete",
    ] {
        assert!(has(&screen, phrase), "missing {phrase}: {screen}");
    }
}
