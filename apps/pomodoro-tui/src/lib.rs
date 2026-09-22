//! V1 application controller, terminal interaction, and snapshot rendering.
//! The legacy executable is switched to this boundary in a later Phase 2 unit.

#[cfg(target_os = "linux")]
pub mod controller;

#[cfg(target_os = "linux")]
pub mod app;
mod settings;
#[cfg(target_os = "linux")]
pub mod ui;
mod ui_settings;
