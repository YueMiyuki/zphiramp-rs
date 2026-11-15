use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use zphira_phira::{ChartInfo, ChartListPage, PhiraClient, PhiraError};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait Upstream: Send + Sync + 'static {
    fn chart(&self, id: i32) -> BoxFuture<'_, Result<Arc<ChartInfo>, UpstreamError>>;

    fn chart_page<'a>(
        &'a self,
        page: i32,
        division: Option<&'a str>,
    ) -> BoxFuture<'a, Result<ChartListPage, UpstreamError>>;

    fn probe_duration(&self, chart_id: i32) -> BoxFuture<'_, Result<i32, UpstreamError>>;
}

impl Upstream for PhiraClient {
    fn chart(&self, id: i32) -> BoxFuture<'_, Result<Arc<ChartInfo>, UpstreamError>> {
        Box::pin(async move {
            PhiraClient::chart(self, id)
                .await
                .map_err(UpstreamError::from)
        })
    }

    fn chart_page<'a>(
        &'a self,
        page: i32,
        division: Option<&'a str>,
    ) -> BoxFuture<'a, Result<ChartListPage, UpstreamError>> {
        Box::pin(async move {
            PhiraClient::chart_page(self, page, division)
                .await
                .map_err(UpstreamError::from)
        })
    }

    fn probe_duration(&self, chart_id: i32) -> BoxFuture<'_, Result<i32, UpstreamError>> {
        Box::pin(async move {
            PhiraClient::probe_duration(self, chart_id)
                .await
                .map_err(UpstreamError::from)
        })
    }
}

pub(crate) async fn fetch_chart(
    upstream: &dyn Upstream,
    chart_id: i32,
) -> Result<Arc<ChartInfo>, UpstreamError> {
    let chart = upstream.chart(chart_id).await?;
    if chart.id != chart_id {
        return Err(UpstreamError::new(
            format!("Phira returned chart {} for request {chart_id}", chart.id),
            None,
        ));
    }
    Ok(chart)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamError {
    message: String,
    status: Option<u16>,
}

impl UpstreamError {
    pub fn new(message: impl Into<String>, status: Option<u16>) -> Self {
        Self {
            message: message.into(),
            status,
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn status(&self) -> Option<u16> {
        self.status
    }

    pub fn is_not_found(&self) -> bool {
        self.status == Some(404)
    }
}

impl From<PhiraError> for UpstreamError {
    fn from(e: PhiraError) -> Self {
        Self {
            message: e.to_string(),
            status: e.status(),
        }
    }
}

impl fmt::Display for UpstreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for UpstreamError {}
