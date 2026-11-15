use std::cell::RefCell;
use std::collections::HashSet;
use std::fmt;

use serde::de::{self, DeserializeSeed, IntoDeserializer, Visitor};

use super::error::{GsonError, GsonErrorKind, NullField};
use super::node::{Node, as_peeked_long};
use crate::java_fmt;

pub(crate) const ELEMENT_TOKEN: &str = "$zphira_common::gson::JsonElement";
pub(crate) const NUMBER_KEY: &str = "$zphira_common::gson::Number";

#[derive(Default)]
pub(crate) struct Ctx {
    learned: RefCell<HashSet<NullField>>,
}

pub(crate) fn from_node<T: de::DeserializeOwned>(node: &Node) -> Result<T, GsonError> {
    // A null in a non-Option struct field fails once per (struct, field); the retry treats
    // that field's nulls as absent so its default is kept.
    let ctx = Ctx::default();
    loop {
        let de = De {
            node,
            ctx: &ctx,
            path: Path::Root,
            field: None,
        };
        match T::deserialize(de) {
            Err(e) if e.null_field.is_some() => {
                let nf = e.null_field.clone().expect("checked");
                if !ctx.learned.borrow_mut().insert(nf) {
                    let mut e = e;
                    e.null_field = None;
                    return Err(e);
                }
            }
            other => return other,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Path<'p> {
    Root,
    Key(&'p Path<'p>, &'p str),
    Index(&'p Path<'p>, usize),
}

impl fmt::Display for Path<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Path::Root => f.write_str("$"),
            Path::Key(parent, key) => write!(f, "{parent}.{key}"),
            Path::Index(parent, i) => write!(f, "{parent}[{i}]"),
        }
    }
}

#[derive(Clone, Copy)]
struct FieldInfo<'p> {
    struct_name: &'static str,
    fields: usize,
    key: &'p str,
}

pub(crate) struct De<'n, 'p> {
    node: &'n Node,
    ctx: &'n Ctx,
    path: Path<'p>,
    field: Option<FieldInfo<'p>>,
}

const STATE_URL: &str =
    "https://github.com/google/gson/blob/main/Troubleshooting.md#unexpected-json-structure";

impl<'n, 'p> De<'n, 'p> {
    fn unexpected(&self, expected: &str) -> GsonError {
        GsonError::data(format!(
            "java.lang.IllegalStateException: Expected {expected} but was {} at path {}\nSee {STATE_URL}",
            self.node.token_name(),
            self.path
        ))
    }

    fn number_format(&self, message: &str) -> GsonError {
        GsonError::data(format!("java.lang.NumberFormatException: {message}"))
    }

    fn null_error(&self, expected: &str) -> GsonError {
        let mut e = GsonError::data(format!(
            "null is not allowed for {expected} at path {}",
            self.path
        ));
        if let Some(f) = self.field {
            e.null_field = Some(NullField {
                struct_name: f.struct_name,
                fields: f.fields,
                key: f.key.to_owned(),
            });
        }
        e
    }

    fn located(&self, message: impl fmt::Display) -> String {
        format!("{message} at path {}", self.path)
    }

    fn next_int(&self, what: &str) -> Result<i32, GsonError> {
        match self.node {
            Node::Number(text) => {
                if let Some(l) = as_peeked_long(text) {
                    let r = l as i32;
                    if r as i64 != l {
                        return Err(self.number_format(
                            &self.located(format_args!("Expected an int but was {l}")),
                        ));
                    }
                    return Ok(r);
                }
                self.int_from_double_text(text)
            }
            Node::String(s) => {
                if let Ok(v) = java_fmt::parse_int(s) {
                    return Ok(v);
                }
                self.int_from_double_text(s)
            }
            Node::Null => Err(self.null_error(what)),
            _ => Err(self.unexpected("an int")),
        }
    }

    fn int_from_double_text(&self, text: &str) -> Result<i32, GsonError> {
        let d = java_fmt::parse_double(text).map_err(|e| self.number_format(e.message()))?;
        let r = d as i32;
        if r as f64 != d {
            return Err(
                self.number_format(&self.located(format_args!("Expected an int but was {text}")))
            );
        }
        Ok(r)
    }

    fn next_long(&self, what: &str) -> Result<i64, GsonError> {
        match self.node {
            Node::Number(text) => {
                if let Some(l) = as_peeked_long(text) {
                    return Ok(l);
                }
                self.long_from_double_text(text)
            }
            Node::String(s) => {
                if let Ok(v) = java_fmt::parse_long(s) {
                    return Ok(v);
                }
                self.long_from_double_text(s)
            }
            Node::Null => Err(self.null_error(what)),
            _ => Err(self.unexpected("a long")),
        }
    }

    fn long_from_double_text(&self, text: &str) -> Result<i64, GsonError> {
        let d = java_fmt::parse_double(text).map_err(|e| self.number_format(e.message()))?;
        let r = d as i64;
        if r as f64 != d {
            return Err(
                self.number_format(&self.located(format_args!("Expected a long but was {text}")))
            );
        }
        Ok(r)
    }

    fn next_double(&self, what: &str) -> Result<f64, GsonError> {
        match self.node {
            Node::Number(text) => {
                if let Some(l) = as_peeked_long(text) {
                    return Ok(l as f64);
                }
                java_fmt::parse_double(text)
                    .map_err(|e| GsonError::new(GsonErrorKind::NumberFormat, e.message()))
            }
            Node::String(s) => java_fmt::parse_double(s)
                .map_err(|e| GsonError::new(GsonErrorKind::NumberFormat, e.message())),
            Node::Null => Err(self.null_error(what)),
            _ => Err(self.unexpected("a double")),
        }
    }

    fn string_adapter(&self) -> Result<String, GsonError> {
        match self.node {
            Node::String(s) => Ok(s.clone()),
            Node::Number(text) => Ok(text.clone()),
            Node::Bool(b) => Ok(b.to_string()),
            Node::Null => Err(self.null_error("a string")),
            _ => Err(self.unexpected("a string")),
        }
    }

    fn reader_next_string(&self, what: &str) -> Result<&'n str, GsonError> {
        match self.node {
            Node::String(s) => Ok(s),
            Node::Number(text) => Ok(text),
            Node::Null => Err(self.null_error(what)),
            _ => Err(self.unexpected("a string")),
        }
    }

    fn boolean_adapter(&self) -> Result<bool, GsonError> {
        match self.node {
            Node::Bool(b) => Ok(*b),
            Node::String(s) => Ok(java_fmt::parse_boolean(s)),
            Node::Null => Err(self.null_error("a boolean")),
            _ => Err(self.unexpected("a boolean")),
        }
    }

    fn lossy(&self, v: i64, ty: &str) -> GsonError {
        GsonError::data(format!(
            "Lossy conversion from {v} to {ty}; at path {}",
            self.path
        ))
    }

    fn child<'c>(&self, node: &'n Node, path: Path<'c>) -> De<'n, 'c> {
        De {
            node,
            ctx: self.ctx,
            path,
            field: None,
        }
    }
}

macro_rules! int_via {
    ($method:ident, $visit:ident, $ty:ty, $next:ident, $name:literal, $lo:expr, $hi:expr) => {
        fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, GsonError> {
            let v = self.$next($name)? as i64;
            if v < $lo as i64 || v > $hi as i64 {
                return Err(self.lossy(v, $name));
            }
            visitor.$visit(v as $ty)
        }
    };
}

impl<'de> de::Deserializer<'de> for De<'_, '_> {
    type Error = GsonError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, GsonError> {
        match self.node {
            Node::Null => visitor.visit_unit(),
            Node::Bool(b) => visitor.visit_bool(*b),
            Node::Number(text) => {
                if let Some(l) = as_peeked_long(text) {
                    visitor.visit_i64(l)
                } else if let Ok(u) = text.parse::<u64>() {
                    visitor.visit_u64(u)
                } else {
                    let d = java_fmt::parse_double(text)
                        .map_err(|e| GsonError::new(GsonErrorKind::NumberFormat, e.message()))?;
                    visitor.visit_f64(d)
                }
            }
            Node::String(s) => visitor.visit_str(s),
            Node::Array(items) => visitor.visit_seq(SeqAccess {
                items,
                index: 0,
                ctx: self.ctx,
                path: self.path,
            }),
            Node::Object(entries) => {
                visitor.visit_map(MapAccess::new(entries, self.ctx, self.path))
            }
        }
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, GsonError> {
        visitor.visit_bool(self.boolean_adapter()?)
    }

    fn deserialize_i8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, GsonError> {
        let v = self.next_int("byte")?;
        if !(-128..=255).contains(&v) {
            return Err(self.lossy(v as i64, "byte"));
        }
        visitor.visit_i8(v as i8)
    }

    fn deserialize_i16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, GsonError> {
        let v = self.next_int("short")?;
        if !(-32768..=65535).contains(&v) {
            return Err(self.lossy(v as i64, "short"));
        }
        visitor.visit_i16(v as i16)
    }

    fn deserialize_i32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, GsonError> {
        visitor.visit_i32(self.next_int("int")?)
    }

    fn deserialize_i64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, GsonError> {
        visitor.visit_i64(self.next_long("long")?)
    }

    fn deserialize_i128<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, GsonError> {
        visitor.visit_i128(self.next_long("long")? as i128)
    }

    int_via!(
        deserialize_u8,
        visit_u8,
        u8,
        next_int,
        "unsigned byte",
        0,
        u8::MAX
    );
    int_via!(
        deserialize_u16,
        visit_u16,
        u16,
        next_int,
        "unsigned short",
        0,
        u16::MAX
    );
    int_via!(
        deserialize_u32,
        visit_u32,
        u32,
        next_long,
        "unsigned int",
        0,
        u32::MAX
    );

    fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, GsonError> {
        let v = self.next_long("unsigned long")?;
        if v < 0 {
            return Err(self.lossy(v, "unsigned long"));
        }
        visitor.visit_u64(v as u64)
    }

    fn deserialize_u128<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, GsonError> {
        let v = self.next_long("unsigned long")?;
        if v < 0 {
            return Err(self.lossy(v, "unsigned long"));
        }
        visitor.visit_u128(v as u128)
    }

    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, GsonError> {
        visitor.visit_f32(self.next_double("float")? as f32)
    }

    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, GsonError> {
        visitor.visit_f64(self.next_double("double")?)
    }

    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, GsonError> {
        let s = self.reader_next_string("a char")?;
        let mut chars = s.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if c.len_utf16() == 1 => visitor.visit_char(c),
            _ => Err(GsonError::data(format!(
                "Expecting character, got: {s}; at {}",
                self.path
            ))),
        }
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, GsonError> {
        visitor.visit_string(self.string_adapter()?)
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, GsonError> {
        visitor.visit_string(self.string_adapter()?)
    }

    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, GsonError> {
        self.deserialize_byte_buf(visitor)
    }

    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, GsonError> {
        match self.node {
            Node::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for (i, item) in items.iter().enumerate() {
                    let path = Path::Index(&self.path, i);
                    let v: i8 = de::Deserialize::deserialize(self.child(item, path))?;
                    out.push(v as u8);
                }
                visitor.visit_byte_buf(out)
            }
            Node::Null => Err(self.null_error("an array")),
            _ => Err(self.unexpected("BEGIN_ARRAY")),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, GsonError> {
        match self.node {
            Node::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, GsonError> {
        match self.node {
            Node::Null => visitor.visit_unit(),
            _ => Err(self.unexpected("null")),
        }
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, GsonError> {
        match self.node {
            Node::Null | Node::Object(_) => visitor.visit_unit(),
            _ => Err(self.unexpected("BEGIN_OBJECT")),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, GsonError> {
        if name == ELEMENT_TOKEN {
            return visitor.visit_newtype_struct(RawDe { node: self.node });
        }
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, GsonError> {
        match self.node {
            Node::Array(items) => visitor.visit_seq(SeqAccess {
                items,
                index: 0,
                ctx: self.ctx,
                path: self.path,
            }),
            Node::Null => Err(self.null_error("an array")),
            _ => Err(self.unexpected("BEGIN_ARRAY")),
        }
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, GsonError> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, GsonError> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, GsonError> {
        match self.node {
            Node::Object(entries) => {
                let mut seen: std::collections::HashMap<&str, bool> =
                    std::collections::HashMap::new();
                for (key, value) in entries {
                    let non_null = !matches!(value, Node::Null);
                    if let Some(prev_non_null) = seen.insert(key.as_str(), non_null)
                        && prev_non_null
                    {
                        return Err(GsonError::data(format!("duplicate key: {key}")));
                    }
                }
                if seen.len() == entries.len() {
                    visitor.visit_map(MapAccess::new(entries, self.ctx, self.path))
                } else {
                    let mut last: std::collections::HashMap<&str, usize> =
                        std::collections::HashMap::new();
                    for (i, (key, _)) in entries.iter().enumerate() {
                        last.insert(key.as_str(), i);
                    }
                    let kept: Vec<(String, Node)> = entries
                        .iter()
                        .enumerate()
                        .filter(|(i, (key, _))| last[key.as_str()] == *i)
                        .map(|(_, e)| e.clone())
                        .collect();
                    visitor.visit_map(MapAccess::new(&kept, self.ctx, self.path))
                }
            }
            Node::Array(pairs) => visitor.visit_map(PairMapAccess {
                pairs,
                index: 0,
                ctx: self.ctx,
                path: self.path,
                value: None,
            }),
            Node::Null => Err(self.null_error("an object")),
            _ => Err(self.unexpected("BEGIN_OBJECT")),
        }
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, GsonError> {
        match self.node {
            Node::Object(entries) => {
                let fields_id = fields.as_ptr() as usize;
                let learned = self.ctx.learned.borrow();
                let mut slots: Vec<Option<&(String, Node)>> = Vec::with_capacity(entries.len());
                let mut position: std::collections::HashMap<&str, usize> =
                    std::collections::HashMap::new();
                for entry in entries {
                    if matches!(entry.1, Node::Null)
                        && !learned.is_empty()
                        && learned.contains(&NullField {
                            struct_name: name,
                            fields: fields_id,
                            key: entry.0.clone(),
                        })
                    {
                        continue;
                    }
                    if let Some(old) = position.insert(entry.0.as_str(), slots.len()) {
                        slots[old] = None;
                    }
                    slots.push(Some(entry));
                }
                drop(learned);
                let kept: Vec<&(String, Node)> = slots.into_iter().flatten().collect();
                visitor.visit_map(StructAccess {
                    entries: kept,
                    index: 0,
                    ctx: self.ctx,
                    path: self.path,
                    struct_name: name,
                    fields: fields_id,
                })
            }
            Node::Null => Err(self.null_error("an object")),
            _ => Err(self.unexpected("BEGIN_OBJECT")),
        }
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, GsonError> {
        match self.node {
            Node::String(s) | Node::Number(s) => {
                visitor.visit_enum(IntoDeserializer::<GsonError>::into_deserializer(s.as_str()))
            }
            Node::Object(entries) if entries.len() == 1 => visitor.visit_enum(VariantAccess {
                name: &entries[0].0,
                value: &entries[0].1,
                ctx: self.ctx,
                path: self.path,
            }),
            Node::Null => Err(self.null_error("an enum")),
            _ => Err(self.unexpected("a string")),
        }
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, GsonError> {
        self.deserialize_str(visitor)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, GsonError> {
        visitor.visit_unit()
    }
}

struct SeqAccess<'n, 'p> {
    items: &'n [Node],
    index: usize,
    ctx: &'n Ctx,
    path: Path<'p>,
}

impl<'de> de::SeqAccess<'de> for SeqAccess<'_, '_> {
    type Error = GsonError;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, GsonError> {
        let Some(item) = self.items.get(self.index) else {
            return Ok(None);
        };
        let i = self.index;
        self.index += 1;
        let de = De {
            node: item,
            ctx: self.ctx,
            path: Path::Index(&self.path, i),
            field: None,
        };
        seed.deserialize(de).map(Some)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.items.len() - self.index)
    }
}

struct MapAccess<'n, 'p> {
    entries: &'n [(String, Node)],
    index: usize,
    ctx: &'n Ctx,
    path: Path<'p>,
}

impl<'n, 'p> MapAccess<'n, 'p> {
    fn new(entries: &'n [(String, Node)], ctx: &'n Ctx, path: Path<'p>) -> Self {
        Self {
            entries,
            index: 0,
            ctx,
            path,
        }
    }
}

impl<'de> de::MapAccess<'de> for MapAccess<'_, '_> {
    type Error = GsonError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, GsonError> {
        let Some((key, _)) = self.entries.get(self.index) else {
            return Ok(None);
        };
        let key_node = Node::String(key.clone());
        let de = De {
            node: &key_node,
            ctx: self.ctx,
            path: Path::Key(&self.path, key),
            field: None,
        };
        seed.deserialize(de).map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, GsonError> {
        let (key, value) = &self.entries[self.index];
        self.index += 1;
        let de = De {
            node: value,
            ctx: self.ctx,
            path: Path::Key(&self.path, key),
            field: None,
        };
        seed.deserialize(de)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.entries.len() - self.index)
    }
}

struct PairMapAccess<'n, 'p> {
    pairs: &'n [Node],
    index: usize,
    ctx: &'n Ctx,
    path: Path<'p>,
    value: Option<&'n Node>,
}

impl<'de> de::MapAccess<'de> for PairMapAccess<'_, '_> {
    type Error = GsonError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, GsonError> {
        let Some(pair) = self.pairs.get(self.index) else {
            return Ok(None);
        };
        let i = self.index;
        let entry_path = Path::Index(&self.path, i);
        let items = match pair {
            Node::Array(items) => items,
            other => {
                return Err(GsonError::data(format!(
                    "java.lang.IllegalStateException: Expected BEGIN_ARRAY but was {} at path {entry_path}\nSee {STATE_URL}",
                    other.token_name()
                )));
            }
        };
        if items.len() < 2 {
            return Err(GsonError::data(format!(
                "map entry at path {entry_path} must be a [key, value] array"
            )));
        }
        if items.len() > 2 {
            let extra = Path::Index(&entry_path, 2);
            return Err(GsonError::data(format!(
                "java.lang.IllegalStateException: Expected END_ARRAY but was {} at path {extra}\nSee {STATE_URL}",
                items[2].token_name()
            )));
        }
        self.value = Some(&items[1]);
        let de = De {
            node: &items[0],
            ctx: self.ctx,
            path: Path::Index(&entry_path, 0),
            field: None,
        };
        seed.deserialize(de).map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, GsonError> {
        let i = self.index;
        self.index += 1;
        let entry_path = Path::Index(&self.path, i);
        let de = De {
            node: self.value.take().expect("key read first"),
            ctx: self.ctx,
            path: Path::Index(&entry_path, 1),
            field: None,
        };
        seed.deserialize(de)
    }
}

struct StructAccess<'n, 'p> {
    entries: Vec<&'n (String, Node)>,
    index: usize,
    ctx: &'n Ctx,
    path: Path<'p>,
    struct_name: &'static str,
    fields: usize,
}

impl<'de> de::MapAccess<'de> for StructAccess<'_, '_> {
    type Error = GsonError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, GsonError> {
        let Some(entry) = self.entries.get(self.index) else {
            return Ok(None);
        };
        seed.deserialize(IntoDeserializer::<GsonError>::into_deserializer(
            entry.0.as_str(),
        ))
        .map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, GsonError> {
        let entry = self.entries[self.index];
        self.index += 1;
        let de = De {
            node: &entry.1,
            ctx: self.ctx,
            path: Path::Key(&self.path, &entry.0),
            field: Some(FieldInfo {
                struct_name: self.struct_name,
                fields: self.fields,
                key: &entry.0,
            }),
        };
        seed.deserialize(de)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.entries.len() - self.index)
    }
}

struct VariantAccess<'n, 'p> {
    name: &'n str,
    value: &'n Node,
    ctx: &'n Ctx,
    path: Path<'p>,
}

impl<'de, 'n, 'p> de::EnumAccess<'de> for VariantAccess<'n, 'p> {
    type Error = GsonError;
    type Variant = Self;

    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Self), GsonError> {
        let v = seed.deserialize(IntoDeserializer::<GsonError>::into_deserializer(self.name))?;
        Ok((v, self))
    }
}

impl<'de> de::VariantAccess<'de> for VariantAccess<'_, '_> {
    type Error = GsonError;

    fn unit_variant(self) -> Result<(), GsonError> {
        Ok(())
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, GsonError> {
        seed.deserialize(De {
            node: self.value,
            ctx: self.ctx,
            path: Path::Key(&self.path, self.name),
            field: None,
        })
    }

    fn tuple_variant<V: Visitor<'de>>(
        self,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, GsonError> {
        de::Deserializer::deserialize_seq(
            De {
                node: self.value,
                ctx: self.ctx,
                path: Path::Key(&self.path, self.name),
                field: None,
            },
            visitor,
        )
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, GsonError> {
        de::Deserializer::deserialize_struct(
            De {
                node: self.value,
                ctx: self.ctx,
                path: Path::Key(&self.path, self.name),
                field: None,
            },
            "",
            fields,
            visitor,
        )
    }
}

struct RawDe<'n> {
    node: &'n Node,
}

impl<'de> de::Deserializer<'de> for RawDe<'_> {
    type Error = GsonError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, GsonError> {
        match self.node {
            Node::Null => visitor.visit_unit(),
            Node::Bool(b) => visitor.visit_bool(*b),
            Node::Number(text) => {
                visitor.visit_map(de::value::MapDeserializer::<_, GsonError>::new(
                    std::iter::once((NUMBER_KEY, text.as_str())),
                ))
            }
            Node::String(s) => visitor.visit_str(s),
            Node::Array(items) => {
                visitor.visit_seq(de::value::SeqDeserializer::<_, GsonError>::new(
                    items.iter().map(|node| RawDe { node }),
                ))
            }
            Node::Object(entries) => {
                visitor.visit_map(de::value::MapDeserializer::<_, GsonError>::new(
                    entries.iter().map(|(k, node)| (k.as_str(), RawDe { node })),
                ))
            }
        }
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum identifier ignored_any
    }
}

impl<'de> IntoDeserializer<'de, GsonError> for RawDe<'_> {
    type Deserializer = Self;

    fn into_deserializer(self) -> Self {
        self
    }
}
