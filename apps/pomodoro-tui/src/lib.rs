//! Application control independent of terminal input and rendering.
//! The legacy executable is switched to this boundary in a later Phase 2 unit.

#[cfg(target_os = "linux")]
pub mod controller;
