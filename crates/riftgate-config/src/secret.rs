//! `Secret<T>` — a newtype that redacts at every leak surface.
//!
//! Wraps an inner value; `Debug` / `Display` always print `***`. Callers
//! that genuinely need the inner value call [`Secret::expose`].
//!
//! Per [ADR 0012](../../../docs/06-adrs/0012-static-toml-env-override-v01.md):
//! every field marked `Secret<T>` cannot leak through any logging or
//! error-message path.

use core::fmt;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Newtype that hides its inner value at every `Debug` / `Display` /
/// `Serialize` surface.
///
/// `expose` and `into_inner` are the only ways to read the underlying
/// value; any other path (`println!("{:?}", ...)`, `tracing::info!(?cfg, ...)`,
/// the `--dry-run` config dump) prints `***`.
#[derive(Clone, Eq, PartialEq, Hash)]
pub struct Secret<T>(T);

impl<T> Secret<T> {
    /// Wrap a value in `Secret`.
    pub const fn new(value: T) -> Self {
        Self(value)
    }

    /// Borrow the inner value. Use sparingly; every call site is a
    /// potential leak source.
    pub fn expose(&self) -> &T {
        &self.0
    }

    /// Move the inner value out of the wrapper.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Secret(***)")
    }
}

impl<T> fmt::Display for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "***")
    }
}

impl<T: Default> Default for Secret<T> {
    fn default() -> Self {
        Self(T::default())
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Secret<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        T::deserialize(d).map(Secret)
    }
}

impl<T: Serialize> Serialize for Secret<T> {
    /// Serialises as the literal string `"***"`. The serialise surface is
    /// for the `--dry-run` output; secret material must never round-trip
    /// through it.
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str("***")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts() {
        let s = Secret::new("super-secret-key".to_string());
        assert_eq!(format!("{s:?}"), "Secret(***)");
    }

    #[test]
    fn display_redacts() {
        let s = Secret::new("super-secret-key".to_string());
        assert_eq!(format!("{s}"), "***");
    }

    #[test]
    fn expose_returns_inner() {
        let s = Secret::new("inner".to_string());
        assert_eq!(s.expose(), "inner");
    }

    #[test]
    fn deserializes_through_string() {
        // Use TOML to avoid an extra serde_json dep in unit tests
        // (serde_json is in dev-dependencies for the integration tests
        // in `tests/`).
        #[derive(serde::Deserialize)]
        struct Wrapper {
            value: Secret<String>,
        }
        let w: Wrapper = toml::from_str(r#"value = "hidden""#).unwrap();
        assert_eq!(w.value.expose(), "hidden");
        assert_eq!(format!("{:?}", w.value), "Secret(***)");
    }
}
