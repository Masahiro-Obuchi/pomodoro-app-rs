//! UI- and operating-system-independent Pomodoro timer logic.

mod config;
mod domain;
pub mod legacy;
mod model;
mod observation;
mod reflection;
mod validation;

pub use config::{ConfigError, TimerConfig};
pub use domain::{Command, DomainState};
pub use model::*;
pub use observation::Observation;

#[cfg(test)]
mod domain_tests;
