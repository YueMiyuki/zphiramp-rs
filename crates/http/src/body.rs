use serde::de::{DeserializeOwned, Error as _};
use serde::{Deserialize, Deserializer};
use zphira_catalog::PoolCategory;
use zphira_common::gson::{self, JsonElement};

use crate::java;
use crate::reply::Failure;
use crate::request::Ctx;

pub(crate) async fn body_or_null<T: DeserializeOwned>(ctx: &mut Ctx) -> Result<Option<T>, Failure> {
    let body = ctx.body_string().await?;
    if java::is_blank(&body) {
        return Ok(None);
    }
    gson::from_str_opt::<T>(&body).map_err(|_| Failure::bad_format())
}

pub(crate) fn primitive<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)?.ok_or_else(|| {
        D::Error::custom("null is not allowed as value for record component of primitive type")
    })
}

pub(crate) fn category(raw: Option<&JsonElement>) -> Result<Option<PoolCategory>, Failure> {
    let name = match raw {
        None | Some(JsonElement::Null) => return Ok(None),
        Some(JsonElement::String(s)) => s.clone(),
        Some(JsonElement::Number(n)) => n.as_str().to_owned(),
        Some(_) => return Err(Failure::bad_format()),
    };
    PoolCategory::parse_ignore_case(&name)
        .map(Some)
        .ok_or_else(|| Failure::bad_request(format!("未知类别：{name}")))
}

pub(crate) fn int_list(raw: Option<Vec<Option<i32>>>) -> Result<Option<Vec<i32>>, Failure> {
    match raw {
        None => Ok(None),
        Some(items) => items
            .into_iter()
            .map(|v| v.ok_or_else(Failure::bad_format))
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
    }
}
