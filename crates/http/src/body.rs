use serde::de::DeserializeOwned;
use zphira_catalog::PoolCategory;
use zphira_common::json;

use crate::reply::Failure;
use crate::request::Ctx;

pub(crate) async fn body_or_null<T: DeserializeOwned>(ctx: &mut Ctx) -> Result<Option<T>, Failure> {
    let body = ctx.body_string().await?;
    json::from_str_opt::<T>(&body).map_err(|_| Failure::bad_format())
}

pub(crate) fn category(name: Option<&str>) -> Result<Option<PoolCategory>, Failure> {
    name.map(|name| {
        PoolCategory::parse_ignore_case(name)
            .ok_or_else(|| Failure::bad_request(format!("未知类别：{name}")))
    })
    .transpose()
}
