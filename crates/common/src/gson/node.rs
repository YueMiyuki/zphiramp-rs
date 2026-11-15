#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Node {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<Node>),
    Object(Vec<(String, Node)>),
}

impl Node {
    pub(crate) fn token_name(&self) -> &'static str {
        match self {
            Node::Null => "NULL",
            Node::Bool(_) => "BOOLEAN",
            Node::Number(_) => "NUMBER",
            Node::String(_) => "STRING",
            Node::Array(_) => "BEGIN_ARRAY",
            Node::Object(_) => "BEGIN_OBJECT",
        }
    }
}

pub(crate) fn as_peeked_long(text: &str) -> Option<i64> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if text == "-0" {
        return None;
    }
    text.parse::<i64>().ok()
}
