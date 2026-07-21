use std::{error::Error, fmt, time::SystemTime};

use chrono::{DateTime, Local, Utc};

/// Returns the current Unix timestamp in milliseconds.
///
/// # Errors
///
/// Returns [`TimeError`] if the system clock is before the Unix epoch or the timestamp
/// does not fit in a `u64`.
pub fn unix_time_millis() -> Result<u64, TimeError> {
    let millis = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|_| TimeError::BeforeUnixEpoch)?
        .as_millis();
    u64::try_from(millis).map_err(|_| TimeError::OutOfRange)
}

/// Converts a Unix timestamp to the environment's local date in `YYYY-MM-DD` format.
///
/// # Errors
///
/// Returns [`TimeError::OutOfRange`] if `chrono` cannot represent the timestamp.
pub fn local_date_at(timestamp_ms: u64) -> Result<String, TimeError> {
    let timestamp = i64::try_from(timestamp_ms).map_err(|_| TimeError::OutOfRange)?;
    let utc = DateTime::<Utc>::from_timestamp_millis(timestamp).ok_or(TimeError::OutOfRange)?;
    Ok(utc.with_timezone(&Local).format("%Y-%m-%d").to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeError {
    BeforeUnixEpoch,
    OutOfRange,
}

impl fmt::Display for TimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::BeforeUnixEpoch => "system time is before the Unix epoch",
            Self::OutOfRange => "timestamp is outside the supported range",
        })
    }
}

impl Error for TimeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_a_timestamp_as_a_date() {
        let date = local_date_at(0).unwrap();

        assert_eq!(date.len(), 10);
        assert_eq!(date.as_bytes()[4], b'-');
        assert_eq!(date.as_bytes()[7], b'-');
    }
}
