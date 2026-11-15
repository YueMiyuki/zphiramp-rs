use std::sync::Arc;

pub use zphira_catalog::{BoxFuture, UpstreamError};
use zphira_phira::{GameRecord, PhiraClient, UserInfo};

pub trait GameUpstream: Send + Sync + 'static {
    fn user_by_token<'a>(
        &'a self,
        token: &'a str,
    ) -> BoxFuture<'a, Result<Arc<UserInfo>, UpstreamError>>;

    fn record(&self, id: i32) -> BoxFuture<'_, Result<Arc<GameRecord>, UpstreamError>>;

    fn recent_records(
        &self,
        player: i32,
    ) -> BoxFuture<'_, Result<Arc<Vec<GameRecord>>, UpstreamError>>;
}

impl GameUpstream for PhiraClient {
    fn user_by_token<'a>(
        &'a self,
        token: &'a str,
    ) -> BoxFuture<'a, Result<Arc<UserInfo>, UpstreamError>> {
        Box::pin(async move {
            PhiraClient::user_by_token(self, token)
                .await
                .map_err(UpstreamError::from)
        })
    }

    fn record(&self, id: i32) -> BoxFuture<'_, Result<Arc<GameRecord>, UpstreamError>> {
        Box::pin(async move {
            PhiraClient::record(self, id)
                .await
                .map_err(UpstreamError::from)
        })
    }

    fn recent_records(
        &self,
        player: i32,
    ) -> BoxFuture<'_, Result<Arc<Vec<GameRecord>>, UpstreamError>> {
        Box::pin(async move {
            PhiraClient::recent_records(self, player)
                .await
                .map_err(UpstreamError::from)
        })
    }
}
