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

The normal executable now uses the V1 domain, storage, and save-confirmed controller. On restart, interrupted sessions wait for manual resumption; application downtime and observation gaps are not added to work or break time. Saved Current Tasks are displayed, restored distractions can be returned from, and restored Quick Start decisions can be finished or continued.

These documents also define features that are still planned. Current Task entry, starting Quick Start, and reporting a new distraction through the TUI are Phase 3 work. The full reflection UI and MVP acceptance checks remain in Phase 4. Temporary legacy modules remain in the source until the next cleanup step, but the executable no longer uses them.

The target format starts with fresh data and does not import existing settings, summaries, or timer state. Unsupported or invalid files must not be overwritten automatically.

The [implementation plan](docs/IMPLEMENTATION_PLAN.md) defines phase goals, dependencies, completion criteria, and phase-level progress. Detailed behavior belongs in the specifications; implementation details and verification evidence belong in code, tests, and PRs. The original plan remains available in Git history.

The [Phase 2 plan](docs/PHASE2_PLAN.md) splits persistence and the V1 TUI cutover into independently reviewable steps, with compatibility work, tests, and exit criteria. Phase-level progress remains in the implementation plan.

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
| `Space` | Start, pause, or resume |
| `r` | End/reset the current session; the next start creates a new session |
| `n` | Skip to the next session |
| `s` | Open settings while Ready (waiting to start) |
| `?` | Toggle help |
| `q` | Save and quit |

For a restored distraction, `Space` records Return. At a restored Quick Start decision, `f` finishes and `c` starts a linked Focus session. Breaks and following sessions always wait for manual start.

### Storage, recovery, and save failures

State and history are stored in `pomodoro-app-rs/state.json` under the user's XDG state directory (`$XDG_STATE_HOME`, or normally `~/.local/state`). The same directory holds `state.json.bak` and `state.lock`. A second instance using that directory is rejected. The lock file remains after exit; its existence alone does not mean another instance is running.

Startup validates the entire V1 file before allowing normal operation. Old unversioned data and unsupported versions stop startup without being overwritten or automatically replaced from backup. To start fresh, stop all instances and explicitly move the old application state directory aside, or select a separate empty XDG state directory. Settings and history from the old format are not imported.

If the primary is missing or corrupt and `state.json.bak` is valid, a startup prompt shows the backup's UTC save time and warns that later records may be lost. Press `y` to recover, or `n`, `q`, or `Esc` to exit without changing saved data. Recovery preserves the backup and archives any existing primary in a uniquely named quarantine file before replacement. Temporary files are never automatically adopted; if no valid primary or backup is available, startup stops and preserves the files.

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
