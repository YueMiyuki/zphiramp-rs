use std::fmt;

use serde::de::{self, DeserializeOwned, IntoDeserializer, Visitor};

/// `Ok(None)` for a blank document or `null`; data files and request bodies treat both as absent.
pub fn from_str_opt<T: DeserializeOwned>(text: &str) -> serde_json::Result<Option<T>> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    serde_json::from_str::<Option<T>>(text)
}

/// For `deserialize_with` on `Option<E>`: an unknown or null name gives `None`.
pub fn enum_or_none<'de, D, E>(deserializer: D) -> Result<Option<E>, D::Error>
where
    D: de::Deserializer<'de>,
    E: DeserializeOwned,
{
    let name = deserializer.deserialize_any(EnumName)?;
    Ok(name.and_then(|n| {
        E::deserialize(IntoDeserializer::<de::value::Error>::into_deserializer(
            n.as_str(),
        ))
        .ok()
    }))
}

struct EnumName;

impl<'de> Visitor<'de> for EnumName {
    type Value = Option<String>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an enum name")
    }

    fn visit_unit<Er: de::Error>(self) -> Result<Self::Value, Er> {
        Ok(None)
    }

    fn visit_none<Er: de::Error>(self) -> Result<Self::Value, Er> {
        Ok(None)
    }

    fn visit_some<D: de::Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_any(EnumName)
    }

    fn visit_str<Er: de::Error>(self, v: &str) -> Result<Self::Value, Er> {
        Ok(Some(v.to_owned()))
    }

    fn visit_i64<Er: de::Error>(self, v: i64) -> Result<Self::Value, Er> {
        Ok(Some(v.to_string()))
    }

    fn visit_u64<Er: de::Error>(self, v: u64) -> Result<Self::Value, Er> {
        Ok(Some(v.to_string()))
    }

    fn visit_f64<Er: de::Error>(self, v: f64) -> Result<Self::Value, Er> {
        Ok(Some(v.to_string()))
    }
}
