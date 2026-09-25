//! V1 application controller, terminal interaction, and snapshot rendering.

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub mod controller;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub mod app;
mod settings;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub mod startup_gate;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub mod terminal;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub mod ui;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
mod ui_history;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
mod ui_settings;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
mod ui_task;
