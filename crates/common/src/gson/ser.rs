use std::io::{self, Write};

use serde::ser::{self, Serialize};

use super::error::{GsonError, GsonErrorKind};
use crate::java_fmt;

pub(crate) const NUMBER_TOKEN: &str = "$zphira_common::gson::Number";

#[derive(Debug, Clone, Copy)]
pub(crate) struct Options {
    pub pretty: bool,
    pub serialize_nulls: bool,
    pub html_safe: bool,
}

impl Options {
    pub(crate) const PRETTY: Options = Options {
        pretty: true,
        serialize_nulls: false,
        html_safe: true,
    };
    pub(crate) const COMPACT: Options = Options {
        pretty: false,
        serialize_nulls: true,
        html_safe: true,
    };
    pub(crate) const ELEMENT_TO_STRING: Options = Options {
        pretty: false,
        serialize_nulls: true,
        html_safe: false,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    EmptyDocument,
    NonemptyDocument,
    EmptyArray,
    NonemptyArray,
    EmptyObject,
    NonemptyObject,
    DanglingName,
}

pub(crate) struct JsonWriter<W: Write> {
    out: W,
    stack: Vec<Scope>,
    deferred_name: Option<String>,
    opts: Options,
}

fn nesting_problem() -> GsonError {
    GsonError::new(GsonErrorKind::Serialize, "Nesting problem.")
}

impl<W: Write> JsonWriter<W> {
    pub(crate) fn new(out: W, opts: Options) -> Self {
        Self {
            out,
            stack: vec![Scope::EmptyDocument],
            deferred_name: None,
            opts,
        }
    }

    pub(crate) fn into_inner(self) -> W {
        self.out
    }

    fn top(&self) -> Scope {
        *self.stack.last().expect("writer stack")
    }

    fn replace_top(&mut self, s: Scope) {
        *self.stack.last_mut().expect("writer stack") = s;
    }

    fn newline(&mut self) -> io::Result<()> {
        if !self.opts.pretty {
            return Ok(());
        }
        self.out.write_all(b"\n")?;
        for _ in 1..self.stack.len() {
            self.out.write_all(b"  ")?;
        }
        Ok(())
    }

    fn before_name(&mut self) -> Result<(), GsonError> {
        match self.top() {
            Scope::NonemptyObject => self.out.write_all(b",")?,
            Scope::EmptyObject => {}
            _ => return Err(nesting_problem()),
        }
        self.newline()?;
        self.replace_top(Scope::DanglingName);
        Ok(())
    }

    fn before_value(&mut self) -> Result<(), GsonError> {
        match self.top() {
            Scope::NonemptyDocument | Scope::EmptyDocument => {
                self.replace_top(Scope::NonemptyDocument)
            }
            Scope::EmptyArray => {
                self.replace_top(Scope::NonemptyArray);
                self.newline()?;
            }
            Scope::NonemptyArray => {
                self.out.write_all(b",")?;
                self.newline()?;
            }
            Scope::DanglingName => {
                self.out
                    .write_all(if self.opts.pretty { b": " } else { b":" })?;
                self.replace_top(Scope::NonemptyObject);
            }
            _ => return Err(nesting_problem()),
        }
        Ok(())
    }

    fn write_deferred_name(&mut self) -> Result<(), GsonError> {
        if let Some(name) = self.deferred_name.take() {
            self.before_name()?;
            self.string(&name)?;
        }
        Ok(())
    }

    pub(crate) fn name(&mut self, name: String) -> Result<(), GsonError> {
        if self.deferred_name.is_some() {
            return Err(GsonError::new(
                GsonErrorKind::Serialize,
                "Already wrote a name, expecting a value.",
            ));
        }
        match self.top() {
            Scope::EmptyObject | Scope::NonemptyObject => {}
            _ => {
                return Err(GsonError::new(
                    GsonErrorKind::Serialize,
                    "Please begin an object before writing a name.",
                ));
            }
        }
        self.deferred_name = Some(name);
        Ok(())
    }

    pub(crate) fn begin_array(&mut self) -> Result<(), GsonError> {
        self.write_deferred_name()?;
        self.before_value()?;
        self.stack.push(Scope::EmptyArray);
        self.out.write_all(b"[")?;
        Ok(())
    }

    pub(crate) fn end_array(&mut self) -> Result<(), GsonError> {
        self.close_scope(Scope::EmptyArray, Scope::NonemptyArray, b"]")
    }

    pub(crate) fn begin_object(&mut self) -> Result<(), GsonError> {
        self.write_deferred_name()?;
        self.before_value()?;
        self.stack.push(Scope::EmptyObject);
        self.out.write_all(b"{")?;
        Ok(())
    }

    pub(crate) fn end_object(&mut self) -> Result<(), GsonError> {
        self.close_scope(Scope::EmptyObject, Scope::NonemptyObject, b"}")
    }

    fn close_scope(
        &mut self,
        empty: Scope,
        nonempty: Scope,
        bracket: &[u8],
    ) -> Result<(), GsonError> {
        let context = self.top();
        if context != nonempty && context != empty {
            return Err(nesting_problem());
        }
        if let Some(name) = &self.deferred_name {
            return Err(GsonError::new(
                GsonErrorKind::Serialize,
                format!("Dangling name: {name}"),
            ));
        }
        self.stack.pop();
        if context == nonempty {
            self.newline()?;
        }
        self.out.write_all(bracket)?;
        Ok(())
    }

    pub(crate) fn value_str(&mut self, value: &str) -> Result<(), GsonError> {
        self.write_deferred_name()?;
        self.before_value()?;
        self.string(value)?;
        Ok(())
    }

    pub(crate) fn value_bool(&mut self, value: bool) -> Result<(), GsonError> {
        self.write_deferred_name()?;
        self.before_value()?;
        self.out.write_all(if value { b"true" } else { b"false" })?;
        Ok(())
    }

    pub(crate) fn value_raw(&mut self, text: &str) -> Result<(), GsonError> {
        self.write_deferred_name()?;
        self.before_value()?;
        self.out.write_all(text.as_bytes())?;
        Ok(())
    }

    pub(crate) fn null_value(&mut self) -> Result<(), GsonError> {
        if self.deferred_name.is_some() {
            if self.opts.serialize_nulls {
                self.write_deferred_name()?;
            } else {
                // Without serialize_nulls a null member is dropped together with its name.
                self.deferred_name = None;
                return Ok(());
            }
        }
        self.before_value()?;
        self.out.write_all(b"null")?;
        Ok(())
    }

    fn string(&mut self, value: &str) -> io::Result<()> {
        write_escaped(&mut self.out, value, self.opts.html_safe)
    }
}

pub(crate) fn write_escaped<W: Write>(out: &mut W, value: &str, html_safe: bool) -> io::Result<()> {
    out.write_all(b"\"")?;
    let bytes = value.as_bytes();
    let mut last = 0;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        let replacement: Option<&[u8]> = if b < 0x80 {
            match b {
                b'"' => Some(b"\\\""),
                b'\\' => Some(b"\\\\"),
                b'\t' => Some(b"\\t"),
                0x08 => Some(b"\\b"),
                b'\n' => Some(b"\\n"),
                b'\r' => Some(b"\\r"),
                0x0c => Some(b"\\f"),
                0x00..=0x1f => None,
                b'<' if html_safe => Some(b"\\u003c"),
                b'>' if html_safe => Some(b"\\u003e"),
                b'&' if html_safe => Some(b"\\u0026"),
                b'=' if html_safe => Some(b"\\u003d"),
                b'\'' if html_safe => Some(b"\\u0027"),
                _ => {
                    i += 1;
                    continue;
                }
            }
        } else if bytes[i..].starts_with("\u{2028}".as_bytes()) {
            Some(b"\\u2028")
        } else if bytes[i..].starts_with("\u{2029}".as_bytes()) {
            Some(b"\\u2029")
        } else {
            i += 1;
            continue;
        };
        out.write_all(&bytes[last..i])?;
        match replacement {
            Some(r) => out.write_all(r)?,
            None => write!(out, "\\u{b:04x}")?,
        }
        i += if b < 0x80 { 1 } else { 3 };
        last = i;
    }
    out.write_all(&bytes[last..])?;
    out.write_all(b"\"")
}

fn invalid_float(v: f64) -> GsonError {
    GsonError::new(
        GsonErrorKind::Serialize,
        format!(
            "{} is not a valid double value as per JSON specification. To override this behavior, use GsonBuilder.serializeSpecialFloatingPointValues() method.",
            java_fmt::double_to_string(v)
        ),
    )
}

pub(crate) struct GsonSerializer<W: Write> {
    pub(crate) writer: JsonWriter<W>,
}

type Res = Result<(), GsonError>;

impl<'a, W: Write> ser::Serializer for &'a mut GsonSerializer<W> {
    type Ok = ();
    type Error = GsonError;
    type SerializeSeq = Compound<'a, W>;
    type SerializeTuple = Compound<'a, W>;
    type SerializeTupleStruct = Compound<'a, W>;
    type SerializeTupleVariant = Compound<'a, W>;
    type SerializeMap = Compound<'a, W>;
    type SerializeStruct = Compound<'a, W>;
    type SerializeStructVariant = Compound<'a, W>;

    fn serialize_bool(self, v: bool) -> Res {
        self.writer.value_bool(v)
    }

    fn serialize_i8(self, v: i8) -> Res {
        self.writer.value_raw(&v.to_string())
    }

    fn serialize_i16(self, v: i16) -> Res {
        self.writer.value_raw(&v.to_string())
    }

    fn serialize_i32(self, v: i32) -> Res {
        self.writer.value_raw(&v.to_string())
    }

    fn serialize_i64(self, v: i64) -> Res {
        self.writer.value_raw(&v.to_string())
    }

    fn serialize_i128(self, v: i128) -> Res {
        self.writer.value_raw(&v.to_string())
    }

    fn serialize_u8(self, v: u8) -> Res {
        self.writer.value_raw(&v.to_string())
    }

    fn serialize_u16(self, v: u16) -> Res {
        self.writer.value_raw(&v.to_string())
    }

    fn serialize_u32(self, v: u32) -> Res {
        self.writer.value_raw(&v.to_string())
    }

    fn serialize_u64(self, v: u64) -> Res {
        self.writer.value_raw(&v.to_string())
    }

    fn serialize_u128(self, v: u128) -> Res {
        self.writer.value_raw(&v.to_string())
    }

    fn serialize_f32(self, v: f32) -> Res {
        if !v.is_finite() {
            return Err(invalid_float(v as f64));
        }
        self.writer.value_raw(&java_fmt::float_to_string(v))
    }

    fn serialize_f64(self, v: f64) -> Res {
        if !v.is_finite() {
            return Err(invalid_float(v));
        }
        self.writer.value_raw(&java_fmt::double_to_string(v))
    }

    fn serialize_char(self, v: char) -> Res {
        let mut buf = [0u8; 4];
        self.writer.value_str(v.encode_utf8(&mut buf))
    }

    fn serialize_str(self, v: &str) -> Res {
        self.writer.value_str(v)
    }

    fn serialize_bytes(self, v: &[u8]) -> Res {
        self.writer.begin_array()?;
        for &b in v {
            self.writer.value_raw(&(b as i8).to_string())?;
        }
        self.writer.end_array()
    }

    fn serialize_none(self) -> Res {
        self.writer.null_value()
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Res {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Res {
        self.writer.null_value()
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Res {
        self.writer.begin_object()?;
        self.writer.end_object()
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> Res {
        self.writer.value_str(variant)
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(self, name: &'static str, value: &T) -> Res {
        if name == NUMBER_TOKEN {
            let text = value.serialize(RawTextCapture)?;
            return self.writer.value_raw(&text);
        }
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        value: &T,
    ) -> Res {
        self.writer.begin_object()?;
        self.writer.name(variant.to_owned())?;
        value.serialize(&mut *self)?;
        self.writer.end_object()
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, GsonError> {
        self.writer.begin_array()?;
        Ok(Compound {
            ser: self,
            variant: false,
        })
    }

    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, GsonError> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleStruct, GsonError> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, GsonError> {
        self.writer.begin_object()?;
        self.writer.name(variant.to_owned())?;
        self.writer.begin_array()?;
        Ok(Compound {
            ser: self,
            variant: true,
        })
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, GsonError> {
        self.writer.begin_object()?;
        Ok(Compound {
            ser: self,
            variant: false,
        })
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, GsonError> {
        self.writer.begin_object()?;
        Ok(Compound {
            ser: self,
            variant: false,
        })
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, GsonError> {
        self.writer.begin_object()?;
        self.writer.name(variant.to_owned())?;
        self.writer.begin_object()?;
        Ok(Compound {
            ser: self,
            variant: true,
        })
    }
}

pub(crate) struct Compound<'a, W: Write> {
    ser: &'a mut GsonSerializer<W>,
    variant: bool,
}

impl<W: Write> Compound<'_, W> {
    fn close_variant(&mut self) -> Res {
        if self.variant {
            self.ser.writer.end_object()?;
        }
        Ok(())
    }
}

impl<W: Write> ser::SerializeSeq for Compound<'_, W> {
    type Ok = ();
    type Error = GsonError;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Res {
        value.serialize(&mut *self.ser)
    }

    fn end(mut self) -> Res {
        self.ser.writer.end_array()?;
        self.close_variant()
    }
}

impl<W: Write> ser::SerializeTuple for Compound<'_, W> {
    type Ok = ();
    type Error = GsonError;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Res {
        value.serialize(&mut *self.ser)
    }

    fn end(self) -> Res {
        ser::SerializeSeq::end(self)
    }
}

impl<W: Write> ser::SerializeTupleStruct for Compound<'_, W> {
    type Ok = ();
    type Error = GsonError;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Res {
        value.serialize(&mut *self.ser)
    }

    fn end(self) -> Res {
        ser::SerializeSeq::end(self)
    }
}

impl<W: Write> ser::SerializeTupleVariant for Compound<'_, W> {
    type Ok = ();
    type Error = GsonError;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Res {
        value.serialize(&mut *self.ser)
    }

    fn end(self) -> Res {
        ser::SerializeSeq::end(self)
    }
}

impl<W: Write> ser::SerializeMap for Compound<'_, W> {
    type Ok = ();
    type Error = GsonError;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Res {
        let name = key.serialize(MapKeySerializer)?;
        self.ser.writer.name(name)
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Res {
        value.serialize(&mut *self.ser)
    }

    fn end(mut self) -> Res {
        self.ser.writer.end_object()?;
        self.close_variant()
    }
}

impl<W: Write> ser::SerializeStruct for Compound<'_, W> {
    type Ok = ();
    type Error = GsonError;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, key: &'static str, value: &T) -> Res {
        self.ser.writer.name(key.to_owned())?;
        value.serialize(&mut *self.ser)
    }

    fn end(mut self) -> Res {
        self.ser.writer.end_object()?;
        self.close_variant()
    }
}

impl<W: Write> ser::SerializeStructVariant for Compound<'_, W> {
    type Ok = ();
    type Error = GsonError;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, key: &'static str, value: &T) -> Res {
        ser::SerializeStruct::serialize_field(self, key, value)
    }

    fn end(self) -> Res {
        ser::SerializeStruct::end(self)
    }
}

fn key_error() -> GsonError {
    GsonError::new(
        GsonErrorKind::Serialize,
        "map key must be a string, number, boolean, char or unit enum variant",
    )
}

struct MapKeySerializer;

impl ser::Serializer for MapKeySerializer {
    type Ok = String;
    type Error = GsonError;
    type SerializeSeq = ser::Impossible<String, GsonError>;
    type SerializeTuple = ser::Impossible<String, GsonError>;
    type SerializeTupleStruct = ser::Impossible<String, GsonError>;
    type SerializeTupleVariant = ser::Impossible<String, GsonError>;
    type SerializeMap = ser::Impossible<String, GsonError>;
    type SerializeStruct = ser::Impossible<String, GsonError>;
    type SerializeStructVariant = ser::Impossible<String, GsonError>;

    fn serialize_bool(self, v: bool) -> Result<String, GsonError> {
        Ok(v.to_string())
    }
    fn serialize_i8(self, v: i8) -> Result<String, GsonError> {
        Ok(v.to_string())
    }
    fn serialize_i16(self, v: i16) -> Result<String, GsonError> {
        Ok(v.to_string())
    }
    fn serialize_i32(self, v: i32) -> Result<String, GsonError> {
        Ok(v.to_string())
    }
    fn serialize_i64(self, v: i64) -> Result<String, GsonError> {
        Ok(v.to_string())
    }
    fn serialize_i128(self, v: i128) -> Result<String, GsonError> {
        Ok(v.to_string())
    }
    fn serialize_u8(self, v: u8) -> Result<String, GsonError> {
        Ok(v.to_string())
    }
    fn serialize_u16(self, v: u16) -> Result<String, GsonError> {
        Ok(v.to_string())
    }
    fn serialize_u32(self, v: u32) -> Result<String, GsonError> {
        Ok(v.to_string())
    }
    fn serialize_u64(self, v: u64) -> Result<String, GsonError> {
        Ok(v.to_string())
    }
    fn serialize_u128(self, v: u128) -> Result<String, GsonError> {
        Ok(v.to_string())
    }
    fn serialize_f32(self, v: f32) -> Result<String, GsonError> {
        Ok(java_fmt::float_to_string(v))
    }
    fn serialize_f64(self, v: f64) -> Result<String, GsonError> {
        Ok(java_fmt::double_to_string(v))
    }
    fn serialize_char(self, v: char) -> Result<String, GsonError> {
        Ok(v.to_string())
    }
    fn serialize_str(self, v: &str) -> Result<String, GsonError> {
        Ok(v.to_owned())
    }
    fn serialize_bytes(self, _v: &[u8]) -> Result<String, GsonError> {
        Err(key_error())
    }
    fn serialize_none(self) -> Result<String, GsonError> {
        Ok("null".to_owned())
    }
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<String, GsonError> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Result<String, GsonError> {
        Ok("null".to_owned())
    }
    fn serialize_unit_struct(self, _name: &'static str) -> Result<String, GsonError> {
        Err(key_error())
    }
    fn serialize_unit_variant(
        self,
        _n: &'static str,
        _i: u32,
        variant: &'static str,
    ) -> Result<String, GsonError> {
        Ok(variant.to_owned())
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _n: &'static str,
        value: &T,
    ) -> Result<String, GsonError> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _n: &'static str,
        _i: u32,
        _v: &'static str,
        _value: &T,
    ) -> Result<String, GsonError> {
        Err(key_error())
    }
    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, GsonError> {
        Err(key_error())
    }
    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, GsonError> {
        Err(key_error())
    }
    fn serialize_tuple_struct(
        self,
        _n: &'static str,
        _l: usize,
    ) -> Result<Self::SerializeTupleStruct, GsonError> {
        Err(key_error())
    }
    fn serialize_tuple_variant(
        self,
        _n: &'static str,
        _i: u32,
        _v: &'static str,
        _l: usize,
    ) -> Result<Self::SerializeTupleVariant, GsonError> {
        Err(key_error())
    }
    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, GsonError> {
        Err(key_error())
    }
    fn serialize_struct(
        self,
        _n: &'static str,
        _l: usize,
    ) -> Result<Self::SerializeStruct, GsonError> {
        Err(key_error())
    }
    fn serialize_struct_variant(
        self,
        _n: &'static str,
        _i: u32,
        _v: &'static str,
        _l: usize,
    ) -> Result<Self::SerializeStructVariant, GsonError> {
        Err(key_error())
    }
}

struct RawTextCapture;

impl ser::Serializer for RawTextCapture {
    type Ok = String;
    type Error = GsonError;
    type SerializeSeq = ser::Impossible<String, GsonError>;
    type SerializeTuple = ser::Impossible<String, GsonError>;
    type SerializeTupleStruct = ser::Impossible<String, GsonError>;
    type SerializeTupleVariant = ser::Impossible<String, GsonError>;
    type SerializeMap = ser::Impossible<String, GsonError>;
    type SerializeStruct = ser::Impossible<String, GsonError>;
    type SerializeStructVariant = ser::Impossible<String, GsonError>;

    fn serialize_str(self, v: &str) -> Result<String, GsonError> {
        Ok(v.to_owned())
    }

    fn serialize_bool(self, _v: bool) -> Result<String, GsonError> {
        Err(raw_error())
    }
    fn serialize_i8(self, _v: i8) -> Result<String, GsonError> {
        Err(raw_error())
    }
    fn serialize_i16(self, _v: i16) -> Result<String, GsonError> {
        Err(raw_error())
    }
    fn serialize_i32(self, _v: i32) -> Result<String, GsonError> {
        Err(raw_error())
    }
    fn serialize_i64(self, _v: i64) -> Result<String, GsonError> {
        Err(raw_error())
    }
    fn serialize_u8(self, _v: u8) -> Result<String, GsonError> {
        Err(raw_error())
    }
    fn serialize_u16(self, _v: u16) -> Result<String, GsonError> {
        Err(raw_error())
    }
    fn serialize_u32(self, _v: u32) -> Result<String, GsonError> {
        Err(raw_error())
    }
    fn serialize_u64(self, _v: u64) -> Result<String, GsonError> {
        Err(raw_error())
    }
    fn serialize_f32(self, _v: f32) -> Result<String, GsonError> {
        Err(raw_error())
    }
    fn serialize_f64(self, _v: f64) -> Result<String, GsonError> {
        Err(raw_error())
    }
    fn serialize_char(self, _v: char) -> Result<String, GsonError> {
        Err(raw_error())
    }
    fn serialize_bytes(self, _v: &[u8]) -> Result<String, GsonError> {
        Err(raw_error())
    }
    fn serialize_none(self) -> Result<String, GsonError> {
        Err(raw_error())
    }
    fn serialize_some<T: ?Sized + Serialize>(self, _value: &T) -> Result<String, GsonError> {
        Err(raw_error())
    }
    fn serialize_unit(self) -> Result<String, GsonError> {
        Err(raw_error())
    }
    fn serialize_unit_struct(self, _n: &'static str) -> Result<String, GsonError> {
        Err(raw_error())
    }
    fn serialize_unit_variant(
        self,
        _n: &'static str,
        _i: u32,
        _v: &'static str,
    ) -> Result<String, GsonError> {
        Err(raw_error())
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _n: &'static str,
        _v: &T,
    ) -> Result<String, GsonError> {
        Err(raw_error())
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _n: &'static str,
        _i: u32,
        _v: &'static str,
        _value: &T,
    ) -> Result<String, GsonError> {
        Err(raw_error())
    }
    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, GsonError> {
        Err(raw_error())
    }
    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, GsonError> {
        Err(raw_error())
    }
    fn serialize_tuple_struct(
        self,
        _n: &'static str,
        _l: usize,
    ) -> Result<Self::SerializeTupleStruct, GsonError> {
        Err(raw_error())
    }
    fn serialize_tuple_variant(
        self,
        _n: &'static str,
        _i: u32,
        _v: &'static str,
        _l: usize,
    ) -> Result<Self::SerializeTupleVariant, GsonError> {
        Err(raw_error())
    }
    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, GsonError> {
        Err(raw_error())
    }
    fn serialize_struct(
        self,
        _n: &'static str,
        _l: usize,
    ) -> Result<Self::SerializeStruct, GsonError> {
        Err(raw_error())
    }
    fn serialize_struct_variant(
        self,
        _n: &'static str,
        _i: u32,
        _v: &'static str,
        _l: usize,
    ) -> Result<Self::SerializeStructVariant, GsonError> {
        Err(raw_error())
    }
}

fn raw_error() -> GsonError {
    GsonError::new(
        GsonErrorKind::Serialize,
        "raw number token must wrap a string",
    )
}

pub(crate) fn write_value<W: Write, T: ?Sized + Serialize>(
    out: W,
    value: &T,
    opts: Options,
) -> Result<W, GsonError> {
    let mut ser = GsonSerializer {
        writer: JsonWriter::new(out, opts),
    };
    value.serialize(&mut ser)?;
    Ok(ser.writer.into_inner())
}
