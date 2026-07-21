# Pomodoro App in Rust

A Pomodoro timer for Linux, built as a hands-on Rust learning project.

The project currently provides a terminal user interface built with Ratatui. The timer state machine lives in the UI-independent `pomodoro-core` crate so it can later be reused by an egui/eframe application targeting both native Linux and WebAssembly.

> [!NOTE]
> The current TUI text and desktop notifications are in Japanese. Internationalization can be added without changing the core timer logic.

## Current features

- 25-minute focus sessions, 5-minute short breaks, and 15-minute long breaks
- Long break after every four completed focus sessions
- Start, pause, resume, reset, and skip controls
- In-app editing for session durations and the number of focus sessions per round
- Deadline-based timing that remains accurate after delayed redraws or system sleep
- Persistent timer state and lightweight daily history
- Linux desktop notifications through `notify-send`
- Platform-independent timer logic with deterministic unit tests

See the [implementation plan](docs/IMPLEMENTATION_PLAN.md) for the architecture, accepted specifications, and roadmap.

## Requirements

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
| `Space` | Start, pause, or resume |
| `r` | Reset the current session |
| `n` | Skip to the next session |
| `s` | Open settings while the timer is idle |
| `?` | Toggle help |
| `q` | Save and quit |

Timer state and history are stored in `pomodoro-app-rs/state.json` under the user's XDG state directory.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## License

Licensed under the [MIT License](LICENSE).
