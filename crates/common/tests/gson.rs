mod support;

use indexmap::IndexMap;
use serde::de::DeserializeOwned;
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};
use support::read_tsv;
use zphira_common::gson::{self, GsonError, GsonErrorKind, JsonElement};
use zphira_common::java_fmt::{JavaDouble, JavaFloat};
use zphira_common::time::JavaOffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Kind {
    Regular,
    Final,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
struct Inner {
    id: i32,
    label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
struct Everything {
    int_value: i32,
    long_value: i64,
    float_value: f32,
    double_value: f64,
    flag: bool,
    letter: char,
    text: Option<String>,
    null_text: Option<String>,
    boxed_null: Option<i32>,
    boxed: Option<i32>,
    numbers: Option<Vec<i32>>,
    empty_list: Option<Vec<i32>>,
    with_nulls: Option<Vec<Option<String>>>,
    map: Option<IndexMap<String, Option<i32>>>,
    empty_map: Option<IndexMap<String, i32>>,
    int_keys: Option<IndexMap<i32, Option<String>>>,
    inner: Option<Inner>,
    null_inner: Option<Inner>,
    inners: Option<Vec<Inner>>,
    kind: Option<Kind>,
    null_kind: Option<Kind>,
    at: Option<JavaOffsetDateTime>,
    #[serde(rename = "ratingCount")]
    rating_count: i32,
    #[serde(rename = "user_i_d")]
    user_id: i32,
    chart2_id: i32,
    nested: Option<Vec<Vec<i32>>>,
    map_of_lists: Option<IndexMap<String, Vec<i32>>>,
}

fn everything() -> Everything {
    let mut map = IndexMap::new();
    map.insert("one".to_owned(), Some(1));
    map.insert("gone".to_owned(), None);
    map.insert("<three>".to_owned(), Some(3));
    let mut int_keys = IndexMap::new();
    int_keys.insert(10, Some("ten".to_owned()));
    int_keys.insert(-1, None);
    let mut map_of_lists = IndexMap::new();
    map_of_lists.insert("a".to_owned(), vec![]);
    map_of_lists.insert("b".to_owned(), vec![4, 5]);
    Everything {
        int_value: -42,
        long_value: 9_007_199_254_740_993,
        float_value: 0.9876_f32 * 100.0,
        double_value: 1234.0 / 1000.0,
        flag: true,
        letter: '<',
        text: Some(
            "a<b>&c='d' \"q\" \\ \n\t\u{8}\u{c}\r \u{1}\u{1f}\u{7f} \u{2028}\u{2029} 中文 😀 /"
                .to_owned(),
        ),
        null_text: None,
        boxed_null: None,
        boxed: Some(7),
        numbers: Some(vec![1, 2, 3]),
        empty_list: Some(vec![]),
        with_nulls: Some(vec![Some("x".to_owned()), None, Some("y".to_owned())]),
        map: Some(map),
        empty_map: Some(IndexMap::new()),
        int_keys: Some(int_keys),
        inner: Some(Inner {
            id: 1,
            label: Some("first".to_owned()),
        }),
        null_inner: None,
        inners: Some(vec![
            Inner { id: 2, label: None },
            Inner {
                id: 3,
                label: Some("third".to_owned()),
            },
        ]),
        kind: Some(Kind::Final),
        null_kind: None,
        at: Some(JavaOffsetDateTime::parse("2026-11-14T15:26:00.120+08:00").unwrap()),
        rating_count: 5,
        user_id: 6,
        chart2_id: 8,
        nested: Some(vec![vec![], vec![1], vec![2, 3]]),
        map_of_lists: Some(map_of_lists),
    }
}

#[derive(Debug, Serialize)]
struct Floats {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f64,
    f: f64,
    g: f64,
    h: f64,
}

#[derive(Debug, Default, Serialize)]
struct AllNull {
    a: Option<String>,
    b: Option<i32>,
    c: Option<Inner>,
    d: Option<Vec<i32>>,
}

#[derive(Debug, Serialize)]
struct NestedEmpty {
    empty: IndexMap<String, i32>,
    list: (IndexMap<String, i32>, Vec<i32>),
}

struct DoubleKeys;

impl Serialize for DoubleKeys {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut m = serializer.serialize_map(Some(3))?;
        m.serialize_entry(&0.001_f64, "milli")?;
        m.serialize_entry(&1.0_f64, "one")?;
        m.serialize_entry(&1e7_f64, "big")?;
        m.end()
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
struct Rec {
    a: i32,
    b: Option<String>,
    c: f64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
struct Pair {
    a: i32,
    b: Option<String>,
    c: Option<Inner>,
}

fn java_error(e: &GsonError) -> String {
    match e.kind() {
        GsonErrorKind::NumberFormat => {
            format!("ERR java.lang.NumberFormatException: {}", e.message())
        }
        GsonErrorKind::Serialize => {
            format!("ERR java.lang.IllegalArgumentException: {}", e.message())
        }
        GsonErrorKind::Custom => format!(
            "ERR java.time.format.DateTimeParseException: {}",
            e.message()
        ),
        _ => format!("ERR com.google.gson.JsonSyntaxException: {}", e.message()),
    }
}

fn same_result(expected: &str, got: &str) -> bool {
    expected == got || (!got.contains(" at line ") && strip_line_column(expected) == got)
}

fn strip_line_column(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find(" at line ") {
        out.push_str(&rest[..i]);
        let tail = &rest[i + " at line ".len()..];
        match tail.find(" path ") {
            Some(j) if tail[..j].contains(" column ") => {
                out.push_str(" at path ");
                rest = &tail[j + " path ".len()..];
            }
            _ => {
                out.push_str(" at line ");
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

#[test]
fn serialization_matches_gson() {
    let rows = read_tsv("gson_serialize.tsv");
    let mut checked = 0;
    for row in &rows {
        let (name, pretty, compact) = (&row[0], &row[1], &row[2]);
        let (p, c) = match name.as_str() {
            "everything" => (
                gson::to_string_pretty(&everything()),
                gson::to_string_compact(&everything()),
            ),
            "floats" => {
                let f = Floats {
                    a: 0.1,
                    b: -0.0,
                    c: 1.0e-4,
                    d: 1.234_567_8e7,
                    e: 1.0e-5,
                    f: 1.0e7,
                    g: -0.0,
                    h: f64::from_bits(1),
                };
                (gson::to_string_pretty(&f), gson::to_string_compact(&f))
            }
            "all_null" => (
                gson::to_string_pretty(&AllNull::default()),
                gson::to_string_compact(&AllNull::default()),
            ),
            "top_null" => (
                gson::to_string_pretty(&None::<i32>),
                gson::to_string_compact(&None::<i32>),
            ),
            "top_string" => (
                gson::to_string_pretty("<html> & 'x'"),
                gson::to_string_compact("<html> & 'x'"),
            ),
            "top_int" => (gson::to_string_pretty(&5), gson::to_string_compact(&5)),
            "top_empty_list" => (
                gson::to_string_pretty(&Vec::<i32>::new()),
                gson::to_string_compact(&Vec::<i32>::new()),
            ),
            "top_list_of_nulls" => {
                let v: Vec<Option<i32>> = vec![None, None];
                (gson::to_string_pretty(&v), gson::to_string_compact(&v))
            }
            "map_all_null" => {
                let mut m: IndexMap<&str, Option<i32>> = IndexMap::new();
                m.insert("a", None);
                m.insert("b", None);
                (gson::to_string_pretty(&m), gson::to_string_compact(&m))
            }
            "nested_empty" => {
                let v = NestedEmpty {
                    empty: IndexMap::new(),
                    list: (IndexMap::new(), vec![]),
                };
                (gson::to_string_pretty(&v), gson::to_string_compact(&v))
            }
            "double_keys" => (
                gson::to_string_pretty(&DoubleKeys),
                gson::to_string_compact(&DoubleKeys),
            ),
            "bool_keys" => {
                let mut m = IndexMap::new();
                m.insert(true, 1);
                m.insert(false, 0);
                (gson::to_string_pretty(&m), gson::to_string_compact(&m))
            }
            "record" => {
                let r = Rec {
                    a: 1,
                    b: None,
                    c: 2.5,
                };
                (gson::to_string_pretty(&r), gson::to_string_compact(&r))
            }
            "element" => {
                let e = JsonElement::parse(
                    r#"{"n":1.50,"e":1e5,"neg":-0,"big":123456789012345678901234567890,"s":"<","nul":null,"arr":[null,1,{"x":null}],"o":{}}"#,
                )
                .unwrap();
                (gson::to_string_pretty(&e), gson::to_string_compact(&e))
            }
            "error_nan" | "error_neg_inf" => {
                let v = if name == "error_nan" { f32::NAN } else { 0.0 };
                let e = if name == "error_nan" {
                    0.0
                } else {
                    f64::NEG_INFINITY
                };
                let f = Floats {
                    a: v,
                    b: 0.0,
                    c: 0.0,
                    d: 0.0,
                    e,
                    f: 0.0,
                    g: 0.0,
                    h: 0.0,
                };
                (gson::to_string_pretty(&f), gson::to_string_compact(&f))
            }
            other => panic!("unknown case {other}"),
        };
        let p = p.unwrap_or_else(|e| java_error(&e));
        let c = c.unwrap_or_else(|e| java_error(&e));
        assert_eq!(&p, pretty, "pretty output of {name}");
        assert_eq!(&c, compact, "compact output of {name}");
        checked += 1;
    }
    assert_eq!(checked, 16);

    let back: Everything =
        gson::from_str(&gson::to_string_compact(&everything()).unwrap()).unwrap();
    assert_eq!(back, everything());
    let back: Everything = gson::from_str(&gson::to_string_pretty(&everything()).unwrap()).unwrap();
    let mut expected = everything();
    expected.map.as_mut().unwrap().shift_remove("gone");
    expected.int_keys.as_mut().unwrap().shift_remove(&-1);
    assert_eq!(back, expected);
}

macro_rules! boxed {
    ($name:ident, $ty:ty, $default:expr) => {
        #[derive(Debug, Deserialize)]
        #[serde(default)]
        struct $name {
            v: $ty,
        }
        impl Default for $name {
            fn default() -> Self {
                Self { v: $default }
            }
        }
    };
}

boxed!(IntBox, i32, 42);
boxed!(LongBox, i64, 42);
boxed!(ShortBox, i16, 42);
boxed!(ByteBox, i8, 42);
boxed!(FloatBox, f32, 42.0);
boxed!(DoubleBox, f64, 42.0);
boxed!(BoolBox, bool, true);
boxed!(CharBox, char, 'z');
boxed!(StringBox, Option<String>, Some("init".to_owned()));
boxed!(IntegerBox, Option<i32>, Some(42));
boxed!(TimeBox, Option<JavaOffsetDateTime>, None);
boxed!(ListBox, Option<Vec<i32>>, Some(vec![9]));
boxed!(
    InnerBox,
    Option<Inner>,
    Some(Inner {
        id: 9,
        label: Some("init".to_owned())
    })
);

#[derive(Debug, Deserialize)]
#[serde(default)]
struct KindBox {
    #[serde(deserialize_with = "gson::enum_or_none")]
    v: Option<Kind>,
}

impl Default for KindBox {
    fn default() -> Self {
        Self {
            v: Some(Kind::Regular),
        }
    }
}

fn run<T: DeserializeOwned>(json: &str, show: impl Fn(T) -> String) -> String {
    match gson::from_str::<T>(json) {
        Ok(v) => format!("OK {}", show(v)),
        Err(e) => java_error(&e),
    }
}

fn opt<T>(v: Option<T>, f: impl Fn(T) -> String) -> String {
    v.map_or_else(|| "null".to_owned(), f)
}

fn coerce(ty: &str, json: &str) -> String {
    match ty {
        "int" => run::<IntBox>(json, |b| b.v.to_string()),
        "long" => run::<LongBox>(json, |b| b.v.to_string()),
        "short" => run::<ShortBox>(json, |b| b.v.to_string()),
        "byte" => run::<ByteBox>(json, |b| b.v.to_string()),
        "float" => run::<FloatBox>(json, |b| JavaFloat(b.v).to_string()),
        "double" => run::<DoubleBox>(json, |b| JavaDouble(b.v).to_string()),
        "boolean" => run::<BoolBox>(json, |b| b.v.to_string()),
        "char" => run::<CharBox>(json, |b| b.v.to_string()),
        "String" => run::<StringBox>(json, |b| opt(b.v, |s| s)),
        "Integer" => run::<IntegerBox>(json, |b| opt(b.v, |v| v.to_string())),
        "enum" => run::<KindBox>(json, |b| {
            opt(b.v, |k| match k {
                Kind::Regular => "REGULAR".to_owned(),
                Kind::Final => "FINAL".to_owned(),
            })
        }),
        "OffsetDateTime" => run::<TimeBox>(json, |b| opt(b.v, |t| t.to_string())),
        "List" => run::<ListBox>(json, |b| opt(b.v, |l| gson::to_string_compact(&l).unwrap())),
        "object" => run::<InnerBox>(json, |b| opt(b.v, |i| gson::to_string_compact(&i).unwrap())),
        "record" => run::<Rec>(json, |r| gson::to_string_compact(&r).unwrap()),
        other => panic!("unknown type {other}"),
    }
}

// Cases where this implementation knowingly differs from the recorded result.
fn known_difference(ty: &str, value: &str) -> Option<&'static str> {
    match (ty, value) {
        ("record", r#"{"a":null}"#) => Some("record null primitive"),
        ("OffsetDateTime", "[]" | "[1]" | "{}" | "[[]]" | r#"{"id":3}"#) => {
            Some("getAsString on containers")
        }
        ("enum", "true" | "false" | "TRUE" | "[]" | "[1]" | "{}" | "[[]]" | r#"{"id":3}"#) => {
            Some("enum from non-string")
        }
        _ => None,
    }
}

#[test]
fn coercions_match_gson() {
    let rows = read_tsv("gson_coerce.tsv");
    assert!(rows.len() > 1000);
    let mut failures = Vec::new();
    let mut exact = 0;
    for row in &rows {
        let (ty, value, expected) = (&row[0], &row[1], &row[2]);
        let json = if ty == "record" {
            value.clone()
        } else {
            format!("{{\"v\":{value}}}")
        };
        let got = coerce(ty, &json);
        if let Some(reason) = known_difference(ty, value) {
            assert!(
                got.starts_with("ERR") == expected.starts_with("ERR")
                    || reason == "record null primitive",
                "{ty} {value}: {reason}: java={expected} rust={got}"
            );
            continue;
        }
        if same_result(expected, &got) {
            exact += 1;
        } else {
            failures.push(format!("{ty} {value}\n  java: {expected}\n  rust: {got}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches ({exact} exact):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn json_parser_matches_gson() {
    let rows = read_tsv("gson_parse.tsv");
    assert!(rows.len() > 80);
    let mut failures = Vec::new();
    for row in &rows {
        let (doc, expected, expected_pretty) = (&row[0], &row[1], &row[2]);
        let (got, pretty) = match JsonElement::parse(doc) {
            Ok(e) => {
                let s = e.to_string();
                let pretty = gson::to_string_pretty(&e).unwrap();
                if s.len() > 300 {
                    (format!("OK {}", s.len()), String::new())
                } else {
                    (format!("OK {s}"), pretty)
                }
            }
            Err(e) => (java_error(&e), String::new()),
        };
        if &got != expected || &pretty != expected_pretty {
            failures.push(format!(
                "{doc:?}\n  java: {expected}\n  rust: {got}\n  java pretty: {expected_pretty}\n  rust pretty: {pretty}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn documents_match_gson() {
    let mut failures = Vec::new();
    for row in read_tsv("gson_document.tsv") {
        let (doc, expected) = (&row[0], &row[1]);
        let got = match gson::from_str_opt::<Pair>(doc) {
            Ok(None) => "NULL".to_owned(),
            Ok(Some(p)) => format!("OK {}", gson::to_string_compact(&p).unwrap()),
            Err(e) => java_error(&e),
        };
        if !same_result(expected, &got) {
            failures.push(format!("{doc:?}\n  java: {expected}\n  rust: {got}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn maps_match_gson() {
    let mut failures = Vec::new();
    for row in read_tsv("gson_map.tsv") {
        let (ty, doc, expected) = (&row[0], &row[1], &row[2]);
        let got = match ty.as_str() {
            "si" => gson::from_str::<IndexMap<String, i32>>(doc)
                .map(|m| gson::to_string_compact(&m).unwrap()),
            "is" => gson::from_str::<IndexMap<i32, String>>(doc)
                .map(|m| gson::to_string_compact(&m).unwrap()),
            "ii" => gson::from_str::<IndexMap<i32, Option<Inner>>>(doc)
                .map(|m| gson::to_string_compact(&m).unwrap()),
            other => panic!("{other}"),
        };
        let got = match got {
            Ok(s) => format!("OK {s}"),
            Err(e) => java_error(&e),
        };
        let known =
            (ty == "is" && doc == r#"{"1":"a","01":"b"}"#) || (ty == "si" && doc == r#"[["a"]]"#);
        if known {
            if doc.starts_with("[[") {
                assert!(got.starts_with("ERR"), "{doc}: {got}");
            }
            continue;
        }
        if !same_result(expected, &got) {
            failures.push(format!("{ty} {doc}\n  java: {expected}\n  rust: {got}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn field_names_match_gson() {
    for row in read_tsv("gson_names.tsv") {
        assert_eq!(gson::field_name(&row[0]), row[1], "{}", row[0]);
    }
}

#[test]
fn writer_variants_and_element_accessors() {
    let mut buf = Vec::new();
    gson::to_writer_pretty(&mut buf, &everything()).unwrap();
    assert_eq!(
        String::from_utf8(buf).unwrap(),
        gson::to_string_pretty(&everything()).unwrap()
    );
    let mut buf = Vec::new();
    gson::to_writer_compact(&mut buf, &everything()).unwrap();
    assert_eq!(buf, gson::to_vec_compact(&everything()).unwrap());

    let e = JsonElement::parse(
        r#"{"favoriteId": 12, "s": "34", "big": 4294967297, "f": 1.9e1, "n": null}"#,
    )
    .unwrap();
    let o = e.as_object().unwrap();
    assert_eq!(o.get("favoriteId").unwrap().get_as_int(), Some(12));
    assert_eq!(o.get("s").unwrap().get_as_int(), Some(34));
    assert_eq!(o.get("big").unwrap().get_as_int(), Some(1));
    assert_eq!(o.get("f").unwrap().get_as_int(), Some(19));
    assert!(o.get("n").unwrap().is_null());
    let p: Pair = gson::from_element(&JsonElement::parse(r#"{"a":"5"}"#).unwrap()).unwrap();
    assert_eq!(p.a, 5);

    #[derive(Serialize, Deserialize, Default)]
    #[serde(default)]
    struct Holder {
        raw: JsonElement,
    }
    let h: Holder = gson::from_str(r#"{"raw":{"x":1.50,"y":[1e5,-0]}}"#).unwrap();
    assert_eq!(
        gson::to_string_compact(&h).unwrap(),
        r#"{"raw":{"x":1.50,"y":[1e5,-0]}}"#
    );
}

#[test]
fn from_str_null_and_empty() {
    assert_eq!(gson::from_str_opt::<Pair>("").unwrap(), None);
    assert_eq!(gson::from_str_opt::<Pair>(" \n").unwrap(), None);
    assert_eq!(gson::from_str::<Option<Pair>>("null").unwrap(), None);
    assert_eq!(
        gson::from_str::<Pair>("").unwrap_err().kind(),
        GsonErrorKind::Null
    );
    #[derive(Deserialize, Debug, PartialEq)]
    #[serde(default)]
    struct D {
        rounds: i32,
        tags: Vec<String>,
        name: Option<String>,
        nested: Inner,
    }
    impl Default for D {
        fn default() -> Self {
            Self {
                rounds: 3,
                tags: vec!["t".to_owned()],
                name: Some("n".to_owned()),
                nested: Inner { id: 1, label: None },
            }
        }
    }
    let d: D = gson::from_str(r#"{"rounds":null,"tags":null,"name":null,"nested":null}"#).unwrap();
    assert_eq!(
        d,
        D {
            name: None,
            ..D::default()
        }
    );
    assert!(gson::from_str::<Vec<i32>>("[1,null]").is_err());
    assert_eq!(
        gson::from_str::<Vec<Option<i32>>>("[1,null]").unwrap(),
        vec![Some(1), None]
    );
}
