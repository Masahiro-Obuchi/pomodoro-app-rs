use std::{collections::BTreeSet, error::Error, fmt};

use pomodoro_core::{DomainError, DomainState, Timestamp};
use serde::{
    Deserializer,
    de::{Error as _, IgnoredAny, MapAccess, Visitor},
};

use super::dto::Envelope;

/// A complete candidate, not proof that a file was written successfully.
/// The storage boundary owns generation advancement and fixed-candidate retries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PersistedStateV1 {
    pub save_generation: u64,
    pub saved_at: Timestamp,
    pub domain: DomainState,
}

impl PersistedStateV1 {
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        // IgnoredAny skips string UTF-8 validation, including in object values.
        // Check all bytes before classifying an unsupported version so corrupt
        // documents remain eligible for explicit backup recovery.
        std::str::from_utf8(bytes).map_err(serde_json::Error::custom)?;
        if bytes
            .iter()
            .find(|byte| !matches!(byte, b' ' | b'\n' | b'\r' | b'\t'))
            != Some(&b'{')
        {
            // A valid non-object document cannot carry a top-level version.
            // Validate the entire document first: malformed JSON must still be
            // reported as corruption, not as an unsupported unversioned file.
            serde_json::from_slice::<IgnoredAny>(bytes)?;
            return Err(CodecError::MissingVersion);
        }
        // Inspect the original token stream, not serde_json::Value: a map that
        // overwrites duplicate keys would erase evidence before V1 validation.
        let mut deserializer = serde_json::Deserializer::from_slice(bytes);
        let version = deserializer.deserialize_map(VersionVisitor)?;
        deserializer.end()?;
        match version {
            None => return Err(CodecError::MissingVersion),
            Some(1) => {}
            Some(version) => return Err(CodecError::UnsupportedVersion(version)),
        }
        let envelope: Envelope = serde_json::from_slice(bytes)?;
        if envelope.save_generation == 0 {
            return Err(CodecError::InvalidValue("save generation must be nonzero"));
        }
        let domain = DomainState::from_parts(
            envelope.snapshot.into(),
            envelope.history.into(),
            envelope.id_allocators.into(),
        )?;
        Ok(Self {
            save_generation: envelope.save_generation,
            saved_at: envelope.saved_at.into(),
            domain,
        })
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, CodecError> {
        self.domain.validate()?;
        if self.save_generation == 0 {
            return Err(CodecError::InvalidValue("save generation must be nonzero"));
        }
        let envelope = Envelope {
            schema_version: 1,
            save_generation: self.save_generation,
            saved_at: self.saved_at.try_into().map_err(CodecError::InvalidValue)?,
            id_allocators: self.domain.id_allocators().into(),
            snapshot: self.domain.snapshot().try_into()?,
            history: self.domain.history().try_into()?,
        };
        Ok(serde_json::to_vec_pretty(&envelope)?)
    }
}

struct VersionVisitor;

impl<'de> Visitor<'de> for VersionVisitor {
    type Value = Option<u32>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a versioned persistence object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut seen = BTreeSet::new();
        let mut version = None;
        while let Some(key) = map.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                return Err(A::Error::custom(format!("duplicate field {key}")));
            }
            if key == "schema_version" {
                version = Some(map.next_value::<u32>()?);
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        Ok(version)
    }
}

#[derive(Debug)]
pub(crate) enum CodecError {
    MissingVersion,
    UnsupportedVersion(u32),
    Json(serde_json::Error),
    InvalidDomain(DomainError),
    InvalidValue(&'static str),
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingVersion => {
                f.write_str("missing schema_version; unversioned data is unsupported")
            }
            Self::UnsupportedVersion(version) => write!(f, "unsupported schema version: {version}"),
            Self::Json(error) => write!(f, "invalid V1 JSON: {error}"),
            Self::InvalidDomain(error) => write!(f, "invalid V1 domain state: {error}"),
            Self::InvalidValue(reason) => write!(f, "invalid V1 value: {reason}"),
        }
    }
}

impl Error for CodecError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            Self::InvalidDomain(error) => Some(error),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for CodecError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<DomainError> for CodecError {
    fn from(value: DomainError) -> Self {
        Self::InvalidDomain(value)
    }
}

impl From<&'static str> for CodecError {
    fn from(value: &'static str) -> Self {
        Self::InvalidValue(value)
    }
}

#[cfg(test)]
#[path = "codec_tests.rs"]
mod tests;
