use std::fmt;

use serde::{Deserialize, Serialize};

/// ポモドーロを構成するセッションの種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Focus,
    ShortBreak,
    LongBreak,
}

impl SessionKind {
    #[must_use]
    pub const fn is_focus(self) -> bool {
        matches!(self, Self::Focus)
    }
}

impl fmt::Display for SessionKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Focus => "focus",
            Self::ShortBreak => "short break",
            Self::LongBreak => "long break",
        })
    }
}
