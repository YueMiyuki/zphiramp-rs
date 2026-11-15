mod de;
mod element;
mod error;
mod node;
mod reader;
mod ser;

use std::io::Write;

use serde::Serialize;
use serde::de::{DeserializeOwned, IntoDeserializer};

pub use element::{JsonElement, JsonNumber, JsonObject};
pub use error::{GsonError, GsonErrorKind};

use node::Node;
use reader::{Document, Trailing, read_document};
use ser::{Options, write_value};

/// Members whose value is null are omitted; `to_string_compact` writes them.
pub fn to_string_pretty<T: ?Sized + Serialize>(value: &T) -> Result<String, GsonError> {
    to_vec_pretty(value).map(into_string)
}

pub fn to_string_compact<T: ?Sized + Serialize>(value: &T) -> Result<String, GsonError> {
    to_vec_compact(value).map(into_string)
}

pub fn to_vec_pretty<T: ?Sized + Serialize>(value: &T) -> Result<Vec<u8>, GsonError> {
    write_value(Vec::with_capacity(128), value, Options::PRETTY)
}

pub fn to_vec_compact<T: ?Sized + Serialize>(value: &T) -> Result<Vec<u8>, GsonError> {
    write_value(Vec::with_capacity(128), value, Options::COMPACT)
}

pub fn to_writer_pretty<W: Write, T: ?Sized + Serialize>(
    writer: W,
    value: &T,
) -> Result<(), GsonError> {
    write_value(writer, value, Options::PRETTY).map(drop)
}

pub fn to_writer_compact<W: Write, T: ?Sized + Serialize>(
    writer: W,
    value: &T,
) -> Result<(), GsonError> {
    write_value(writer, value, Options::COMPACT).map(drop)
}

fn into_string(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).expect("serializer writes UTF-8")
}

/// `Ok(None)` for an empty document or one whose first token is `null`.
pub fn from_str_opt<T: DeserializeOwned>(json: &str) -> Result<Option<T>, GsonError> {
    match read_document(json, false).map_err(|e| e.into_syntax_error())? {
        Document::Empty | Document::Null => Ok(None),
        Document::Value { node, trailing } => {
            let value = de::from_node::<T>(&node)?;
            match trailing {
                Trailing::None => Ok(Some(value)),
                Trailing::Value => Err(GsonError::syntax("JSON document was not fully consumed.")),
                Trailing::Error(e) => Err(e.into_syntax_error()),
            }
        }
    }
}

pub fn from_str<T: DeserializeOwned>(json: &str) -> Result<T, GsonError> {
    match from_str_opt::<T>(json)? {
        Some(v) => Ok(v),
        None => de::from_node::<T>(&Node::Null).map_err(|_| {
            GsonError::new(
                GsonErrorKind::Null,
                "the JSON document is empty or null (Gson returns null)",
            )
        }),
    }
}

pub fn from_element<T: DeserializeOwned>(element: &JsonElement) -> Result<T, GsonError> {
    de::from_node::<T>(&element_to_node(element))
}

fn element_to_node(e: &JsonElement) -> Node {
    match e {
        JsonElement::Null => Node::Null,
        JsonElement::Bool(b) => Node::Bool(*b),
        JsonElement::Number(n) => Node::Number(n.as_str().to_owned()),
        JsonElement::String(s) => Node::String(s.clone()),
        JsonElement::Array(items) => Node::Array(items.iter().map(element_to_node).collect()),
        JsonElement::Object(o) => Node::Object(
            o.iter()
                .map(|(k, v)| (k.clone(), element_to_node(v)))
                .collect(),
        ),
    }
}

/// JSON key for a Java field name: `roundsPerStay` gives `rounds_per_stay`, `userID` gives `user_i_d`.
pub fn field_name(java_name: &str) -> String {
    let mut out = String::with_capacity(java_name.len() + 4);
    for c in java_name.chars() {
        if c.is_uppercase() && !out.is_empty() {
            out.push('_');
        }
        out.push(c);
    }
    out.to_lowercase()
}

/// For `deserialize_with` on `Option<E>`: an unknown name gives `None` instead of an error.
pub fn enum_or_none<'de, D, E>(deserializer: D) -> Result<Option<E>, D::Error>
where
    D: serde::Deserializer<'de>,
    E: DeserializeOwned,
{
    let name = deserializer.deserialize_any(EnumNameVisitor)?;
    Ok(name.and_then(|n| {
        E::deserialize(IntoDeserializer::<serde::de::value::Error>::into_deserializer(n.as_str()))
            .ok()
    }))
}

struct EnumNameVisitor;

impl<'de> serde::de::Visitor<'de> for EnumNameVisitor {
    type Value = Option<String>;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("an enum constant name")
    }

    fn visit_unit<Er: serde::de::Error>(self) -> Result<Self::Value, Er> {
        Ok(None)
    }

    fn visit_none<Er: serde::de::Error>(self) -> Result<Self::Value, Er> {
        Ok(None)
    }

    fn visit_some<D: serde::Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_any(EnumNameVisitor)
    }

    fn visit_str<Er: serde::de::Error>(self, v: &str) -> Result<Self::Value, Er> {
        Ok(Some(v.to_owned()))
    }

    fn visit_i64<Er: serde::de::Error>(self, v: i64) -> Result<Self::Value, Er> {
        Ok(Some(v.to_string()))
    }

    fn visit_u64<Er: serde::de::Error>(self, v: u64) -> Result<Self::Value, Er> {
        Ok(Some(v.to_string()))
    }

    fn visit_f64<Er: serde::de::Error>(self, v: f64) -> Result<Self::Value, Er> {
        Ok(Some(crate::java_fmt::double_to_string(v)))
    }
}
