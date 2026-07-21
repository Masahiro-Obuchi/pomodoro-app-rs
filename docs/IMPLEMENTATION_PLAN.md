# Pomodoro App Implementation Plan

## Implementation status

- [x] M1: Project foundation and core library
- [ ] M2: TUI (basic screen, controls, persistence, and notifications are implemented; settings screen remains)
- [ ] M3: Native Linux GUI
- [ ] M4: WebAssembly GUI
- [ ] M5: Packaging and extensions

## 1. Purpose

Build a Pomodoro timer suitable for daily use on Linux while learning Rust through a practical project. Complete the TUI first, then provide native Linux and WebAssembly GUIs that reuse the same core timer logic.

## 2. Technical direction

- Keep timer state transitions in `pomodoro-core`, independent of the UI and operating system.
- Use Ratatui for the first frontend.
- Use egui/eframe to share GUI code between native Linux and WebAssembly.
- Keep GTK 4 as a low-priority future frontend and outside the MVP.
- Isolate platform-specific time, persistence, and notification behavior behind adapters.
- Run `cargo fmt`, `cargo clippy`, and `cargo test` at every milestone.

## 3. MVP specification

### 3.1 Timer

| Setting | Default |
| --- | ---: |
| Focus session | 25 minutes |
| Short break | 5 minutes |
| Long break | 15 minutes |
| Focus sessions before a long break | 4 |

The timer supports starting, pausing, resuming, resetting, and skipping to the next session.

- A completed focus session is followed by a short break, except that every fourth completed focus is followed by a long break.
- Completing a session sends a notification but does not automatically start the next session.
- Only naturally completed focus sessions count toward history.
- Skipped, reset, and interrupted sessions do not count toward history.
- Time spent paused does not count toward the session duration.
- The timer continues across system sleep. If its deadline has passed when the system resumes, the session completes at that point.
- Running state is persisted and restored after an application restart or web page reload.
- Notifications are not guaranteed while the native application or web page is completely closed.

### 3.2 History

The MVP stores daily aggregates only.

```json
{
  "schema_version": 1,
  "days": {
    "2026-07-21": {
      "completed_focus_sessions": 4,
      "focused_seconds": 6000
    }
  }
}
```

- Use the local date corresponding to the scheduled completion time.
- For each naturally completed focus session, increment the count and add its configured duration.
- Use the same logical data format on native and web targets.
- Include `schema_version` to support future migrations.
- Add detailed event history in a separate storage area without changing the daily aggregate format.

### 3.3 Notifications and persistence

- Use desktop notifications on native Linux.
- Use the Notifications API on the web after the user grants permission.
- Fall back to an in-app message where system notifications are unavailable.
- Follow the XDG Base Directory specification for native settings, state, and history.
- Use browser local storage on the web.
- Write native state through a temporary file and replace the destination to reduce corruption from interrupted writes.

## 4. Architecture

```text
pomodoro-app-rs/
├── Cargo.toml
├── crates/
│   ├── pomodoro-core/
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── config.rs
│   │       ├── history.rs
│   │       ├── session.rs
│   │       └── timer.rs
│   └── pomodoro-platform/
│       └── src/
│           ├── storage.rs
│           └── notification.rs
└── apps/
    ├── pomodoro-tui/
    └── pomodoro-gui/
```

### 4.1 `pomodoro-core`

This crate is independent of the operating system, UI toolkit, filesystem, and asynchronous runtime.

- `TimerConfig`: session durations and focus count before a long break
- `SessionKind`: focus, short break, or long break
- `TimerState`: idle, running, or paused, including persisted timing data
- `PomodoroTimer`: state transitions and completed focus count
- `TimerEvent`: events passed to UI, history, and notification layers
- `History`: versioned daily aggregates

The frontend supplies the current Unix time in milliseconds. A running timer calculates its remaining time from a stored deadline instead of decrementing a counter every second. This prevents drift after delayed redraws, background-tab throttling, or system sleep.

### 4.2 `pomodoro-platform`

This crate provides native time, persistence, and notification adapters. WebAssembly-specific implementations can live in conditionally compiled `wasm32` modules in the GUI crate when needed.

### 4.3 `pomodoro-tui`

The Ratatui frontend displays:

- Current session kind
- Remaining time
- Timer status
- Progress within the current round
- Today's completed focus count and total focus time
- Keyboard help

Initial key bindings:

| Key | Action |
| --- | --- |
| `Space` | Start, pause, or resume |
| `r` | Reset |
| `n` | Skip to the next session |
| `s` | Open settings |
| `?` | Toggle help |
| `q` | Save and quit |

### 4.4 `pomodoro-gui`

Share the main screen between native Linux and WebAssembly with egui/eframe. Implement persistence, notifications, and current-time access separately for each target. On the web, recompute state when the tab becomes visible and after a page reload.

## 5. Milestones

### M1: Project foundation and core

- Create the Cargo workspace.
- Define configuration, state, event, and history types.
- Implement start, pause, resume, reset, skip, and time advancement.
- Add deterministic tests that supply timestamps directly.

Completion criteria:

- All transitions can be tested without waiting for real time.
- The fourth completed focus session transitions to a long break.
- A large time jump emits exactly one completion event.
- History can be serialized to and restored from JSON.

### M2: TUI

- Implement the event loop and screen.
- Implement keyboard controls and settings.
- Persist configuration, timer state, and history.
- Send Linux desktop notifications.

Completion criteria:

- A short test configuration can complete a focus-and-break sequence.
- The application restores the terminal correctly when it exits.
- Timer state and history survive a restart.

### M3: Native Linux GUI

- Implement the egui/eframe screen.
- Reuse the core, persistence, and notification layers from the TUI.
- Add a `.desktop` file and application icon.

Completion criteria:

- The GUI uses the same transitions, settings, and history as the TUI.
- It can be launched from the Linux desktop and send notifications.

### M4: WebAssembly GUI

- Add a `wasm32-unknown-unknown` build.
- Implement browser persistence, notification permission handling, and visibility restoration.
- Generate artifacts suitable for static hosting.

Completion criteria:

- The native GUI screen and transitions work in a browser.
- Remaining time is corrected after returning from a background tab.
- Running state and history survive a page reload.

### M5: Packaging and extensions

- Add PWA support, offline startup, and installation instructions.
- Select and implement a Linux packaging format.
- Add detailed history, statistics, sound, and automatic transitions as needed.
- Reassess the priority of a GTK 4 frontend.
- Add internationalization for UI text and notifications.

## 6. Quality policy

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- Core tests must not sleep or wait for real time.
- Invalid configuration, timestamp overflow, and invalid transitions must return errors.
- UI layers must not duplicate timer business rules.

## 7. Risks and mitigations

| Risk | Mitigation |
| --- | --- |
| Browsers throttle background timers | Recompute remaining time from the stored deadline |
| Closed pages and processes cannot notify reliably | Apply completion on restoration and document the limitation |
| TUI and GUI behavior diverges | Keep all state transitions in `pomodoro-core` |
| Future formats cannot read existing data | Version stored data and provide migrations |
| Too many targets delay a usable result | Complete the core, TUI, native GUI, and WebAssembly target in that order |
