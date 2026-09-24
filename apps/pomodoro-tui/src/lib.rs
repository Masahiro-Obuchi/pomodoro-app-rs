//! V1 application controller, terminal interaction, and snapshot rendering.

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod controller;

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod app;
mod settings;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod startup_gate;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod ui;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod ui_history;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod ui_settings;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod ui_task;
