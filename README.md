# Pomodoro App in Rust

A Rust application for helping people start work, stay oriented, return after distractions, and review their work. The current implementation is a Pomodoro timer for Linux, built as a hands-on Rust learning project.

The project currently provides a terminal user interface built with Ratatui. The timer state machine lives in the UI-independent `pomodoro-core` crate. The TUI remains the initial frontend for validating the expanded product and domain model.

> [!NOTE]
> The current TUI text and desktop notifications are in English. Task names may contain Japanese or other Unicode text. Language switching is not currently implemented.

## Current features

- 25-minute focus sessions, 5-minute short breaks, and 15-minute long breaks
- Long break after every four completed focus sessions
- Start, pause, resume, cancel, reset, and skip controls
- Optional Current Task entry before Focus or Quick Start, saved with the session
- Two-minute Quick Start with an explicit finish or continue choice
- Distraction reporting and Return, including after a restart
- In-app editing for session durations and the number of focus sessions per round
- Monotonic timing that stops during application closure and observation gaps, including system sleep
- V1 JSON persistence with atomic saves, backup recovery, and single-process locking
- Save retry and explicit unsaved-exit controls
- Cumulative work time, focus completions, distractions, and returns derived from stored records
- Linux desktop notifications through `notify-send`
- Platform-independent timer logic with deterministic unit tests

## Accepted specifications

- [Product Spec](docs/PRODUCT_SPEC.md): product goal, MVP scope, behavior, and data preservation requirements.
- [Domain Model](docs/DOMAIN_MODEL.md): state ownership, session and interruption models, invariants, and transitions.
- [Persistence Schema](docs/PERSISTENCE_SCHEMA.md): single-JSON format, validation, atomic saves, recovery, and single-process protection.

The normal executable now uses the V1 domain, storage, and save-confirmed controller. On restart, interrupted sessions wait for manual resumption; application downtime and observation gaps are not added to work or break time. Current Tasks can be entered and restored, distractions can be reported and returned from even after a restart, and restored Quick Start decisions can be finished or continued.

The expanded MVP is implemented in the TUI. The Product Spec also records ideas outside the MVP, including Later Box, AI assistance, and a GUI.

The target format starts with fresh data and does not import existing settings, summaries, or timer state. Unsupported or invalid files must not be overwritten automatically.

The [implementation plan](docs/IMPLEMENTATION_PLAN.md) defines phase goals, dependencies, completion criteria, and phase-level progress. Detailed behavior belongs in the specifications; implementation details and verification evidence belong in code, tests, and PRs. The original plan remains available in Git history.

The completed [Phase 2 plan](docs/archive/PHASE2_PLAN.md), [Phase 3 plan](docs/archive/PHASE3_PLAN.md), and [Phase 4 plan](docs/archive/PHASE4_PLAN.md) are archived as records of their implementation. The [documentation index](docs/README.md) separates current documents from past plans. Phase-level progress remains in the implementation plan.

The [Windows and macOS plan](docs/WINDOWS_MACOS_PLAN.md) tracks a separate future expansion of the TUI. The current executable remains Linux-only.

## Requirements

- Linux with a local filesystem for saved state
- Rust 1.86 or later
- A terminal supported by Crossterm
- `notify-send` for Linux desktop notifications (optional)

## Run the TUI

```bash
cargo run -p pomodoro-tui
```

### Controls

| Key | Action |
| --- | --- |
| `Space` | Start, pause, resume, or return from a distraction |
| `t` | Edit the optional Current Task while waiting to start Focus or Quick Start |
| `2` | Start a two-minute Quick Start while waiting to start Focus |
| `d` | Report a distraction while Focus or Quick Start is running |
| `r` | Reset the current session to the same type and keep its task for restart |
| `n` | Skip to the next scheduled session |
| `x` | Cancel an active session and return to Focus start |
| `s` | Open settings while Ready (waiting to start) |
| `h` | Open the History view during normal operation; `h` or `Esc` returns |
| `?` | Toggle help |
| `q` | Save and quit |

At a Quick Start decision, `f` finishes and `c` starts a new, full-length linked Focus session. Time spent choosing is not counted, and the choice is still waiting after an exit and restart. A saved Current Task is carried into Quick Start and its continued Focus. Resetting Quick Start keeps its task; press `t` to edit it before restarting with `Space`. From a Break start screen, press `n` to return to Focus start before choosing Quick Start. During a running Focus or Quick Start, `d` records a distraction and stops work time. `Space` records Return and resumes the remaining work time, including after an exit and restart. While any session is running or interrupted, `x` ends that attempt and returns to Focus start. Credited work time remains in history; cancelling a distraction does not record a Return. Breaks and following sessions always wait for manual start.

The History view shows recorded work time from Focus and Quick Start, naturally completed Focus sessions, reported distractions, and explicit Returns across all records, including the current session. Pause, distraction, break, and Quick Start choice time do not add work time. Work time uses `H:MM:SS`; subsecond time is truncated only for display. While a session runs, the shown time may include progress since the latest saved checkpoint. Opening History does not pause the timer or save state. If a save fails while it is open, the recovery controls take priority. During a pending save, the main screen shows the latest confirmed saved state and keeps the unconfirmed candidate separate.

On short or narrow terminals, the timer hides the progress gauge and cumulative summary when their space is needed for operation and save recovery controls. Press `h` to view the totals when the summary is hidden. If the recovery keys or History view cannot fit, the screen asks you to enlarge the terminal. Enlarge the terminal to see the gauge and summary again.

In the Current Task editor, type a single line and press `Enter` to save, or `Esc` to cancel. `Backspace` removes the last visible character. The editor starts with the saved task, if any. An empty or whitespace-only line clears it; surrounding whitespace is trimmed when saved. Editing does not change the saved file until `Enter`. Ordinary letters, including `q`, `?`, and `2`, are task text while the editor is open. Terminals that send bracketed paste allow one-line paste; a paste containing line breaks or control characters is rejected as one input. A terminal that sends paste as ordinary keys cannot distinguish it from typing: the first newline can confirm the task and later characters may trigger normal controls. Paste a single-line task in that case.

### Storage, recovery, and save failures

State and history are stored in `pomodoro-app-rs/state.json` under the user's XDG state directory (`$XDG_STATE_HOME`, or normally `~/.local/state`). The same directory holds `state.json.bak` and `state.lock`. A second instance using that directory is rejected. The lock file remains after exit; its existence alone does not mean another instance is running.

Startup validates the entire V1 file before allowing normal operation. Old unversioned data and unsupported versions stop startup without being overwritten or automatically replaced from backup. To start fresh, stop all instances and explicitly move the old application state directory aside, or select a separate empty XDG state directory. Settings and history from the old format are not imported.

If the primary is missing or corrupt and `state.json.bak` is valid, a startup prompt shows the backup's UTC save time and warns that later records may be lost. Press `y` to recover, or `n`, `q`, or `Esc` to exit without changing saved data. If the terminal is narrower than 50 columns or shorter than 9 rows, enlarge it before confirming recovery; `y` is held until the warning and choices can be shown. Recovery preserves the backup and archives any existing primary in a uniquely named quarantine file before replacement. Temporary files are never automatically adopted; if no valid primary or backup is available, startup stops and preserves the files.

On a failed or uncertain save, timing and normal controls are held. Press `r` to retry the same candidate, or `Q` followed by `y` to exit without a confirmed save (`n` / `Esc` cancels that exit). Unsaved exit returns a nonzero process status. After recovery from a failed save, a running session waits for manual resumption. Completion notifications are sent only after the corresponding save succeeds.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p pomodoro-platform --lib crash_boundaries_in_isolated_process -- --ignored
```

The crash suite runs separately so subprocess creation cannot temporarily inherit locks held by parallel unit tests. It stops children at save boundaries and checks restart behavior; it does not simulate power loss. Linux TUI integration tests use pseudoterminals to exercise the executable's real input and exit paths.

## License

Licensed under the [MIT License](LICENSE).
