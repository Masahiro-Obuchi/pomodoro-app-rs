# Pomodoro App in Rust

A Rust application for helping people start work, stay oriented, return after distractions, and review their work. The current implementation is a Pomodoro timer for Linux, built as a hands-on Rust learning project.

The project currently provides a terminal user interface built with Ratatui. The timer state machine lives in the UI-independent `pomodoro-core` crate. The TUI remains the initial frontend for validating the expanded product and domain model.

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

## Accepted specifications

- [Product Spec](docs/PRODUCT_SPEC.md): product goal, MVP scope, behavior, and data preservation requirements.
- [Domain Model](docs/DOMAIN_MODEL.md): state ownership, session and interruption models, invariants, and transitions.
- [Persistence Schema](docs/PERSISTENCE_SCHEMA.md): single-JSON format, validation, atomic saves, recovery, and single-process protection.

These documents define the accepted target behavior; they do not imply that the new features are implemented. In particular, the target specification stops timing during application closure and observation gaps, unlike the current deadline-based restoration behavior described above.

The new domain API is available in `pomodoro-core`, but the TUI and existing storage still use the explicitly isolated `pomodoro_core::legacy` API until the V1 storage cutover. Quick Start, interruption/recovery history, and the new timing rules are not yet available through the TUI. This temporary boundary does not migrate or synchronize old data.

The target format starts with fresh data and does not import existing settings, summaries, or timer state. Unsupported or invalid files must not be overwritten automatically.

The [implementation plan](docs/IMPLEMENTATION_PLAN.md) defines phase goals, dependencies, completion criteria, and phase-level progress. Detailed behavior belongs in the specifications; implementation details and verification evidence belong in code, tests, and PRs. The original plan remains available in Git history.

The [Phase 2 plan](docs/PHASE2_PLAN.md) splits persistence and the V1 TUI cutover into independently reviewable steps, with compatibility work, tests, and exit criteria. Phase-level progress remains in the implementation plan.

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
