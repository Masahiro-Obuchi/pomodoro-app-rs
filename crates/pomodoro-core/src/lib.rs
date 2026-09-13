//! UI- and operating-system-independent Pomodoro timer logic.

mod config;
pub mod legacy;
mod model;

pub use config::{ConfigError, TimerConfig};
pub use model::*;
