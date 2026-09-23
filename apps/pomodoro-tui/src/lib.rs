//! V1 application controller, terminal interaction, and snapshot rendering.

#[cfg(target_os = "linux")]
pub mod controller;

#[cfg(target_os = "linux")]
pub mod app;
mod settings;
#[cfg(target_os = "linux")]
pub mod startup_gate;
#[cfg(target_os = "linux")]
pub mod ui;
mod ui_settings;
