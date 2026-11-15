use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;
use zphira_common::time::JavaOffsetDateTime;
use zphira_phira::{
    ChartInfo, ChartListPage, DEFAULT_BASE_URL, GameRecord, LoginResult, PhiraClient, PhiraError,
    UserInfo,
};

fn model<
    T: std::fmt::Debug + Clone + Serialize + DeserializeOwned + Default + Send + Sync + 'static,
>() {
}

#[test]
fn models_derive_what_the_api_promises() {
    model::<UserInfo>();
    model::<ChartInfo>();
    model::<GameRecord>();
    model::<LoginResult>();
    model::<ChartListPage>();
}

#[allow(dead_code)]
fn user_info_fields(u: &UserInfo) {
    let _: i32 = u.id;
    let _: &Option<String> = &u.name;
    let _: &Option<String> = &u.avatar;
    let _: &Option<String> = &u.language;
    let _: &Option<String> = &u.bio;
    let _: i32 = u.exp;
    let _: f64 = u.rks;
    let _: &Option<JavaOffsetDateTime> = &u.joined;
    let _: &Option<JavaOffsetDateTime> = &u.last_login;
    let _: i32 = u.roles;
    let _: bool = u.banned;
    let _: bool = u.login_banned;
    let _: i32 = u.follower_count;
    let _: i32 = u.following_count;
    let _: &Option<String> = &u.email;
    let _: &str = u.name_or_null();
}

#[allow(dead_code)]
fn chart_info_fields(c: &ChartInfo) {
    let _: i32 = c.id;
    let _: &Option<String> = &c.name;
    let _: &Option<String> = &c.level;
    let _: f32 = c.difficulty;
    let _: &Option<String> = &c.charter;
    let _: &Option<String> = &c.composer;
    let _: &Option<String> = &c.illustrator;
    let _: &Option<String> = &c.description;
    let _: bool = c.ranked;
    let _: bool = c.reviewed;
    let _: bool = c.stable;
    let _: bool = c.stable_request;
    let _: &Option<String> = &c.illustration;
    let _: &Option<String> = &c.preview;
    let _: &Option<String> = &c.file;
    let _: i32 = c.uploader;
    let _: &Option<Vec<String>> = &c.tags;
    let _: f32 = c.rating;
    let _: i32 = c.rating_count;
    let _: &Option<JavaOffsetDateTime> = &c.created;
    let _: &Option<JavaOffsetDateTime> = &c.updated;
    let _: &Option<JavaOffsetDateTime> = &c.chart_updated;
    let _: &str = c.name_or_null();
    let _: &str = c.level_or_null();
    let _: bool = c.has_tag("regular");
    let _: Option<i32> = c.duration_seconds();
    c.set_duration_seconds(Some(1));
}

#[allow(dead_code)]
fn game_record_fields(r: &GameRecord) {
    let _: i32 = r.id;
    let _: i32 = r.player;
    let _: i32 = r.chart;
    let _: i32 = r.score;
    let _: f32 = r.accuracy;
    let _: i32 = r.perfect;
    let _: i32 = r.good;
    let _: i32 = r.bad;
    let _: i32 = r.miss;
    let _: f32 = r.speed;
    let _: i32 = r.max_combo;
    let _: bool = r.best;
    let _: bool = r.best_std;
    let _: i32 = r.mods;
    let _: bool = r.full_combo;
    let _: &Option<JavaOffsetDateTime> = &r.time;
    let _: f32 = r.std;
    let _: f32 = r.std_score;
}

#[allow(dead_code)]
fn response_records(l: &LoginResult, p: &ChartListPage) {
    let _: i32 = l.id;
    let _: &Option<String> = &l.token;
    let _: &Option<String> = &l.refresh_token;
    let _: &Option<String> = &l.expire_at;
    let _: i32 = p.count;
    let _: &Option<Vec<Arc<ChartInfo>>> = &p.results;
}

fn assert_auto<T: Send + Sync + 'static>() {}

#[test]
fn the_client_is_cheap_to_clone_and_shareable() {
    assert_auto::<PhiraClient>();
    assert_auto::<PhiraError>();
    fn is_clone<T: Clone>() {}
    is_clone::<PhiraClient>();
    fn is_error<T: std::error::Error>() {}
    is_error::<PhiraError>();
    assert_eq!(DEFAULT_BASE_URL, "https://phira.5wyxi.com/");
}

#[tokio::test]
async fn signatures_match_the_api_document() {
    let client: Result<PhiraClient, PhiraError> = PhiraClient::new(DEFAULT_BASE_URL);
    let client = client.unwrap();
    let calls = async move {
        let _: Result<Arc<UserInfo>, PhiraError> = client.user_by_token("t").await;
        let _: Result<Arc<UserInfo>, PhiraError> = client.user_by_id(1).await;
        let _: Result<Arc<ChartInfo>, PhiraError> = client.chart(1).await;
        let _: Result<Arc<GameRecord>, PhiraError> = client.record(1).await;
        let _: Result<Arc<Vec<GameRecord>>, PhiraError> = client.recent_records(1).await;
        let _: Result<LoginResult, PhiraError> = client.login("e", "p").await;
        let _: Result<ChartListPage, PhiraError> = client.chart_page(1, Some("plain")).await;
        let _: Result<ChartListPage, PhiraError> = client.chart_page(1, None).await;
        let _: Result<i32, PhiraError> = client.probe_duration(1).await;
        let error: PhiraError = client.chart(1).await.unwrap_err();
        let _: Option<u16> = error.status();
    };
    fn requires_send<F: std::future::Future + Send>(_: &F) {}
    requires_send(&calls);
    drop(calls);
}
