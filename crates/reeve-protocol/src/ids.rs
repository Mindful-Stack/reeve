use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

/// `PREFIX-n`: uppercase letters, a dash, a positive integer without leading zeros.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key {
    prefix: String,
    number: u64,
}

// utoipa's derive cannot override the schema of a named-field struct, so the string schema is written by hand.
impl utoipa::PartialSchema for Key {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::SchemaType::Type(
                utoipa::openapi::schema::Type::String,
            ))
            .pattern(Some("^[A-Z]+-[1-9][0-9]*$"))
            .examples(["ABC-412"])
            .into()
    }
}
impl utoipa::ToSchema for Key {}

impl Key {
    pub fn new(prefix: &str, number: u64) -> Result<Self, KeyError> {
        if number == 0
            || number > i64::MAX as u64
            || prefix.is_empty()
            || !prefix.chars().all(|c| c.is_ascii_uppercase())
        {
            return Err(KeyError(format!("{prefix}-{number}")));
        }
        Ok(Self {
            prefix: prefix.to_string(),
            number,
        })
    }
    pub fn prefix(&self) -> &str {
        &self.prefix
    }
    pub fn number(&self) -> u64 {
        self.number
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid key: {0}")]
pub struct KeyError(pub String);

impl FromStr for Key {
    type Err = KeyError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (p, n) = s.split_once('-').ok_or_else(|| KeyError(s.into()))?;
        if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) || n.starts_with('0') {
            return Err(KeyError(s.into()));
        }
        let number: u64 = n.parse().map_err(|_| KeyError(s.into()))?;
        Key::new(p, number)
    }
}
impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.prefix, self.number)
    }
}
impl Serialize for Key {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}
impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// A key of this instance or an external reference (`SCRUM-11`, `S2-4`): `^[A-Z][A-Z0-9]*-[0-9]+$`.
/// Only the server knows which; it resolves every `KeyRef` through `Handlers::resolve_ref`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyRef(String);

impl KeyRef {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl FromStr for KeyRef {
    type Err = KeyError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let ok = s.split_once('-').is_some_and(|(p, n)| {
            p.starts_with(|c: char| c.is_ascii_uppercase())
                && p.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                && !n.is_empty()
                && n.bytes().all(|b| b.is_ascii_digit())
        });
        if ok {
            Ok(Self(s.to_string()))
        } else {
            Err(KeyError(s.into()))
        }
    }
}
impl fmt::Display for KeyRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl From<Key> for KeyRef {
    fn from(k: Key) -> Self {
        Self(k.to_string())
    }
}
impl Serialize for KeyRef {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}
impl<'de> Deserialize<'de> for KeyRef {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}
impl utoipa::PartialSchema for KeyRef {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::SchemaType::Type(
                utoipa::openapi::schema::Type::String,
            ))
            .pattern(Some("^[A-Z][A-Z0-9]*-[0-9]+$"))
            .examples(["SCRUM-11"])
            .into()
    }
}
impl utoipa::ToSchema for KeyRef {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn key_parses_and_displays() {
        let k: Key = "ABC-412".parse().unwrap();
        assert_eq!(k.prefix(), "ABC");
        assert_eq!(k.number(), 412);
        assert_eq!(k.to_string(), "ABC-412");
    }
    #[test]
    fn key_rejects_bad_shapes() {
        for bad in [
            "ABC", "abc-1", "ABC-0", "ABC-01", "ABC-x", "-1", "G-1-2", "ABC-+1", "ABC-", " ABC-1",
            "ABC-1\n",
        ] {
            assert!(bad.parse::<Key>().is_err(), "{bad} should not parse");
        }
    }
    #[test]
    fn lowercase_key_rejected_on_deserialise_and_huge_number_on_new() {
        assert!(serde_json::from_str::<Key>("\"abc-1\"").is_err());
        assert!(Key::new("ABC", i64::MAX as u64 + 1).is_err());
        assert!(Key::new("ABC", i64::MAX as u64).is_ok());
    }
    #[test]
    fn key_serde_roundtrip() {
        let k: Key = "ABC-7".parse().unwrap();
        let s = serde_json::to_string(&k).unwrap();
        assert_eq!(s, "\"ABC-7\"");
        assert_eq!(serde_json::from_str::<Key>(&s).unwrap(), k);
    }
    #[test]
    fn key_ref_accepts_the_loose_grammar() {
        for ok in ["SCRUM-11", "S2-4", "TST-12"] {
            assert_eq!(ok.parse::<KeyRef>().unwrap().as_str(), ok);
        }
        for bad in ["not-a-key", "scrum-11", "2S-4", "SCRUM-", "SCRUM-1a"] {
            assert!(bad.parse::<KeyRef>().is_err(), "{bad} should not parse");
        }
        assert!(serde_json::from_str::<KeyRef>("\"scrum-11\"").is_err());
        assert_eq!(
            serde_json::to_string(&"S2-4".parse::<KeyRef>().unwrap()).unwrap(),
            "\"S2-4\""
        );
    }
}
