use std::fmt;

use indexmap::IndexMap;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::de::{ELEMENT_TOKEN, NUMBER_KEY};
use super::error::GsonError;
use super::node::{Node, as_peeked_long};
use super::reader::{Document, Trailing, read_document};
use super::ser::{NUMBER_TOKEN, Options, write_value};
use crate::java_fmt;

/// Serialize with this module only: numbers keep their literal text through a private newtype.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum JsonElement {
    #[default]
    Null,
    Bool(bool),
    Number(JsonNumber),
    String(String),
    Array(Vec<JsonElement>),
    Object(JsonObject),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JsonNumber(String);

#[derive(Debug, Clone, PartialEq, Default)]
pub struct JsonObject(IndexMap<String, JsonElement>);

impl JsonElement {
    pub fn parse(json: &str) -> Result<JsonElement, GsonError> {
        match read_document(json, true).map_err(|e| e.into_syntax_error())? {
            Document::Empty | Document::Null => Ok(JsonElement::Null),
            Document::Value { node, trailing } => match trailing {
                Trailing::None => Ok(JsonElement::from_node(node)),
                Trailing::Value => Err(GsonError::syntax("Did not consume the entire document.")),
                Trailing::Error(e) => Err(e.into_syntax_error()),
            },
        }
    }

    pub(crate) fn from_node(node: Node) -> JsonElement {
        match node {
            Node::Null => JsonElement::Null,
            Node::Bool(b) => JsonElement::Bool(b),
            Node::Number(text) => JsonElement::Number(JsonNumber(text)),
            Node::String(s) => JsonElement::String(s),
            Node::Array(items) => {
                JsonElement::Array(items.into_iter().map(JsonElement::from_node).collect())
            }
            Node::Object(entries) => {
                let mut obj = JsonObject::new();
                for (k, v) in entries {
                    obj.insert(k, JsonElement::from_node(v));
                }
                JsonElement::Object(obj)
            }
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, JsonElement::Null)
    }

    pub fn is_primitive(&self) -> bool {
        matches!(
            self,
            JsonElement::Bool(_) | JsonElement::Number(_) | JsonElement::String(_)
        )
    }

    pub fn is_array(&self) -> bool {
        matches!(self, JsonElement::Array(_))
    }

    pub fn is_object(&self) -> bool {
        matches!(self, JsonElement::Object(_))
    }

    pub fn as_array(&self) -> Option<&Vec<JsonElement>> {
        match self {
            JsonElement::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_array_mut(&mut self) -> Option<&mut Vec<JsonElement>> {
        match self {
            JsonElement::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&JsonObject> {
        match self {
            JsonElement::Object(o) => Some(o),
            _ => None,
        }
    }

    pub fn as_object_mut(&mut self) -> Option<&mut JsonObject> {
        match self {
            JsonElement::Object(o) => Some(o),
            _ => None,
        }
    }

    pub fn get_as_string(&self) -> Option<String> {
        match self {
            JsonElement::String(s) => Some(s.clone()),
            JsonElement::Number(n) => Some(n.0.clone()),
            JsonElement::Bool(b) => Some(b.to_string()),
            JsonElement::Array(a) if a.len() == 1 => a[0].get_as_string(),
            _ => None,
        }
    }

    pub fn get_as_int(&self) -> Option<i32> {
        match self {
            JsonElement::Number(n) => Some(n.int_value()),
            JsonElement::String(s) => java_fmt::parse_int(s).ok(),
            JsonElement::Bool(b) => java_fmt::parse_int(&b.to_string()).ok(),
            JsonElement::Array(a) if a.len() == 1 => a[0].get_as_int(),
            _ => None,
        }
    }

    pub fn get_as_long(&self) -> Option<i64> {
        match self {
            JsonElement::Number(n) => Some(n.long_value()),
            JsonElement::String(s) => java_fmt::parse_long(s).ok(),
            JsonElement::Array(a) if a.len() == 1 => a[0].get_as_long(),
            _ => None,
        }
    }

    pub fn get_as_double(&self) -> Option<f64> {
        match self {
            JsonElement::Number(n) => Some(n.double_value()),
            JsonElement::String(s) => java_fmt::parse_double(s).ok(),
            JsonElement::Array(a) if a.len() == 1 => a[0].get_as_double(),
            _ => None,
        }
    }

    pub fn get_as_boolean(&self) -> Option<bool> {
        match self {
            JsonElement::Bool(b) => Some(*b),
            JsonElement::String(_) | JsonElement::Number(_) => Some(java_fmt::parse_boolean(
                &self.get_as_string().unwrap_or_default(),
            )),
            JsonElement::Array(a) if a.len() == 1 => a[0].get_as_boolean(),
            _ => None,
        }
    }
}

impl JsonNumber {
    pub fn from_text(text: impl Into<String>) -> Self {
        JsonNumber(text.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn int_value(&self) -> i32 {
        if let Ok(v) = java_fmt::parse_int(&self.0) {
            return v;
        }
        if let Ok(v) = java_fmt::parse_long(&self.0) {
            return v as i32;
        }
        big_decimal_low_bits(&self.0) as i32
    }

    pub fn long_value(&self) -> i64 {
        if let Ok(v) = java_fmt::parse_long(&self.0) {
            return v;
        }
        big_decimal_low_bits(&self.0) as i64
    }

    pub fn double_value(&self) -> f64 {
        java_fmt::parse_double(&self.0).unwrap_or(f64::NAN)
    }
}

fn big_decimal_low_bits(text: &str) -> u64 {
    let (negative, rest) = match text.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let (mantissa, exp) = match rest.find(['e', 'E']) {
        Some(i) => (&rest[..i], rest[i + 1..].parse::<i64>().unwrap_or(0)),
        None => (rest, 0),
    };
    let (int_part, frac_part) = match mantissa.find('.') {
        Some(i) => (&mantissa[..i], &mantissa[i + 1..]),
        None => (mantissa, ""),
    };
    let digits: Vec<u8> = int_part
        .bytes()
        .chain(frac_part.bytes())
        .map(|b| b.wrapping_sub(b'0'))
        .collect();
    let scale = exp - frac_part.len() as i64;
    let keep = digits.len() as i64 + scale.min(0);
    let mut acc: u64 = 0;
    for &d in digits.iter().take(keep.max(0) as usize) {
        acc = acc.wrapping_mul(10).wrapping_add(d as u64);
    }
    for _ in 0..scale.clamp(0, 64) {
        acc = acc.wrapping_mul(10);
    }
    if negative { acc.wrapping_neg() } else { acc }
}

impl JsonObject {
    pub fn new() -> Self {
        Self(IndexMap::new())
    }

    pub fn insert(&mut self, key: impl Into<String>, value: JsonElement) -> Option<JsonElement> {
        self.0.insert(key.into(), value)
    }

    pub fn get(&self, key: &str) -> Option<&JsonElement> {
        self.0.get(key)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut JsonElement> {
        self.0.get_mut(key)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    pub fn remove(&mut self, key: &str) -> Option<JsonElement> {
        self.0.shift_remove(key)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &JsonElement)> {
        self.0.iter()
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.0.keys()
    }
}

impl FromIterator<(String, JsonElement)> for JsonObject {
    fn from_iter<I: IntoIterator<Item = (String, JsonElement)>>(iter: I) -> Self {
        let mut o = JsonObject::new();
        for (k, v) in iter {
            o.insert(k, v);
        }
        o
    }
}

impl IntoIterator for JsonObject {
    type Item = (String, JsonElement);
    type IntoIter = indexmap::map::IntoIter<String, JsonElement>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl fmt::Display for JsonElement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bytes =
            write_value(Vec::new(), self, Options::ELEMENT_TO_STRING).map_err(|_| fmt::Error)?;
        f.write_str(&String::from_utf8(bytes).map_err(|_| fmt::Error)?)
    }
}

impl Serialize for JsonNumber {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if let Some(l) = as_peeked_long(&self.0) {
            serializer.serialize_i64(l)
        } else {
            serializer.serialize_newtype_struct(NUMBER_TOKEN, &self.0)
        }
    }
}

impl Serialize for JsonElement {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            JsonElement::Null => serializer.serialize_unit(),
            JsonElement::Bool(b) => serializer.serialize_bool(*b),
            JsonElement::Number(n) => n.serialize(serializer),
            JsonElement::String(s) => serializer.serialize_str(s),
            JsonElement::Array(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            JsonElement::Object(obj) => obj.serialize(serializer),
        }
    }
}

impl Serialize for JsonObject {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (k, v) in &self.0 {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for JsonElement {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_newtype_struct(ELEMENT_TOKEN, ElementVisitor)
    }
}

struct ElementVisitor;

impl<'de> Visitor<'de> for ElementVisitor {
    type Value = JsonElement;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_newtype_struct<D: Deserializer<'de>>(self, d: D) -> Result<JsonElement, D::Error> {
        d.deserialize_any(ElementVisitor)
    }

    fn visit_unit<E: de::Error>(self) -> Result<JsonElement, E> {
        Ok(JsonElement::Null)
    }

    fn visit_none<E: de::Error>(self) -> Result<JsonElement, E> {
        Ok(JsonElement::Null)
    }

    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<JsonElement, D::Error> {
        d.deserialize_any(ElementVisitor)
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<JsonElement, E> {
        Ok(JsonElement::Bool(v))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<JsonElement, E> {
        Ok(JsonElement::Number(JsonNumber(v.to_string())))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<JsonElement, E> {
        Ok(JsonElement::Number(JsonNumber(v.to_string())))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<JsonElement, E> {
        Ok(JsonElement::Number(JsonNumber(java_fmt::double_to_string(
            v,
        ))))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<JsonElement, E> {
        Ok(JsonElement::String(v.to_owned()))
    }

    fn visit_string<E: de::Error>(self, v: String) -> Result<JsonElement, E> {
        Ok(JsonElement::String(v))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<JsonElement, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element_seed(ElementSeed)? {
            items.push(item);
        }
        Ok(JsonElement::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<JsonElement, A::Error> {
        let mut obj = JsonObject::new();
        let mut first = true;
        while let Some(key) = map.next_key::<String>()? {
            if first && key == NUMBER_KEY {
                let text: String = map.next_value()?;
                return Ok(JsonElement::Number(JsonNumber(text)));
            }
            first = false;
            let value = map.next_value_seed(ElementSeed)?;
            obj.insert(key, value);
        }
        Ok(JsonElement::Object(obj))
    }
}

struct ElementSeed;

impl<'de> de::DeserializeSeed<'de> for ElementSeed {
    type Value = JsonElement;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<JsonElement, D::Error> {
        d.deserialize_any(ElementVisitor)
    }
}
