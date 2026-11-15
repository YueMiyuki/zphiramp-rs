use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use reqwest::redirect::Policy;
use serde::Serialize;
use zphira_common::gson;

use crate::PhiraError;
use crate::cache::TtlCache;
use crate::duration::http::ProbeHttp;
use crate::duration::probe;
use crate::error::Kind;
use crate::model::{ChartInfo, ChartListPage, GameRecord, LoginResult, UserInfo};

pub const DEFAULT_BASE_URL: &str = "https://phira.5wyxi.com/";

const USER_AGENT: &str = "JPhira/1";
const MAX_REDIRECTS: usize = 4;
const MAX_BODY: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Options {
    pub max_attempts: u32,
    pub retry_delay: Duration,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub probe_timeout: Duration,
    pub recent_record_ttl: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            max_attempts: 8,
            retry_delay: Duration::from_millis(150),
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(10),
            probe_timeout: Duration::from_secs(20),
            recent_record_ttl: Duration::from_secs(10),
        }
    }
}

#[derive(Clone)]
pub struct PhiraClient {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    probe_http: ProbeHttp,
    base_url: String,
    options: Options,
    token_cache: TtlCache<String, Arc<UserInfo>>,
    user_cache: TtlCache<i32, Arc<UserInfo>>,
    chart_cache: TtlCache<i32, Arc<ChartInfo>>,
    record_cache: TtlCache<i32, Arc<GameRecord>>,
    recent_record_cache: TtlCache<i32, Arc<Vec<GameRecord>>>,
    duration_cache: TtlCache<i32, i32>,
}

impl fmt::Debug for PhiraClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PhiraClient")
            .field("base_url", &self.inner.base_url)
            .finish_non_exhaustive()
    }
}

impl PhiraClient {
    pub fn new(base_url: impl Into<String>) -> Result<Self, PhiraError> {
        Self::with_options(base_url, Options::default())
    }

    pub fn with_options(base_url: impl Into<String>, options: Options) -> Result<Self, PhiraError> {
        let mut base_url = base_url.into();
        if !base_url.ends_with('/') {
            base_url.push('/');
        }
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(options.connect_timeout)
            .redirect(redirect_policy())
            // Environment proxy variables are deliberately ignored.
            .no_proxy()
            .build()
            .map_err(|error| PhiraError::new(Kind::InvalidRequest(error.to_string())))?;
        let probe_http = ProbeHttp::new(options.connect_timeout)?;
        let minute = Duration::from_secs(60);
        let inner = Inner {
            http,
            probe_http,
            base_url,
            token_cache: TtlCache::new(10 * minute, 10_000),
            user_cache: TtlCache::new(10 * minute, 5_000),
            chart_cache: TtlCache::new(30 * minute, 10_000),
            record_cache: TtlCache::new(30 * minute, 50_000),
            recent_record_cache: TtlCache::new(options.recent_record_ttl, 5_000),
            duration_cache: TtlCache::new(6 * 60 * minute, 20_000),
            options,
        };
        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    pub async fn user_by_token(&self, token: &str) -> Result<Arc<UserInfo>, PhiraError> {
        if let Some(user) = self.inner.token_cache.get(token) {
            return Ok(user);
        }
        let authorization = HeaderValue::from_str(&format!("Bearer {token}"))
            .map(|mut value| {
                value.set_sensitive(true);
                value
            })
            .map_err(|error| PhiraError::new(Kind::InvalidRequest(error.to_string())))?;
        let request = self.get("me")?.header(AUTHORIZATION, authorization);
        let user: UserInfo = self.fetch_json(request).await?;
        let user = Arc::new(user);
        self.inner
            .token_cache
            .insert(token.to_owned(), user.clone());
        Ok(user)
    }

    pub async fn user_by_id(&self, id: i32) -> Result<Arc<UserInfo>, PhiraError> {
        if let Some(user) = self.inner.user_cache.get(&id) {
            return Ok(user);
        }
        let user: UserInfo = self.fetch_json(self.get(&format!("user/{id}"))?).await?;
        let user = Arc::new(user);
        self.inner.user_cache.insert(id, user.clone());
        Ok(user)
    }

    pub async fn chart(&self, id: i32) -> Result<Arc<ChartInfo>, PhiraError> {
        if let Some(chart) = self.inner.chart_cache.get(&id) {
            return Ok(chart);
        }
        let chart: ChartInfo = self.fetch_json(self.get(&format!("chart/{id}"))?).await?;
        let chart = Arc::new(chart);
        self.inner.chart_cache.insert(id, chart.clone());
        Ok(chart)
    }

    pub async fn record(&self, id: i32) -> Result<Arc<GameRecord>, PhiraError> {
        if let Some(record) = self.inner.record_cache.get(&id) {
            return Ok(record);
        }
        let record: GameRecord = self.fetch_json(self.get(&format!("record/{id}"))?).await?;
        let record = Arc::new(record);
        self.inner.record_cache.insert(id, record.clone());
        Ok(record)
    }

    pub async fn recent_records(&self, player: i32) -> Result<Arc<Vec<GameRecord>>, PhiraError> {
        if let Some(records) = self.inner.recent_record_cache.get(&player) {
            return Ok(records);
        }
        let path = format!("record?player={player}&order=-time&pageNum=10");
        let body = self.execute(self.get(&path)?).await?;
        let records: Option<Vec<GameRecord>> = gson::from_str_opt(&body)?;
        let records = Arc::new(records.unwrap_or_default());
        self.inner
            .recent_record_cache
            .insert(player, records.clone());
        Ok(records)
    }

    pub async fn login(&self, email: &str, password: &str) -> Result<LoginResult, PhiraError> {
        #[derive(Serialize)]
        struct Body<'a> {
            email: &'a str,
            password: &'a str,
        }
        let body = gson::to_string_pretty(&Body { email, password })?;
        let request = self
            .request(reqwest::Method::POST, "login")?
            .header(CONTENT_TYPE, "application/json")
            .body(body);
        self.fetch_json(request).await
    }

    pub async fn chart_page(
        &self,
        page: i32,
        division: Option<&str>,
    ) -> Result<ChartListPage, PhiraError> {
        let mut path = format!("chart?page={page}");
        if let Some(division) = division.filter(|d| !d.trim().is_empty()) {
            path.push_str("&division=");
            path.push_str(division);
        }
        self.fetch_json(self.get(&path)?).await
    }

    pub async fn probe_duration(&self, chart_id: i32) -> Result<i32, PhiraError> {
        if let Some(seconds) = self.inner.duration_cache.get(&chart_id) {
            return Ok(seconds);
        }
        let chart = self.chart(chart_id).await?;
        let file = chart.file.as_deref().unwrap_or("");
        if file.trim().is_empty() {
            return Err(PhiraError::probe(format!(
                "Chart has no archive: {chart_id}"
            )));
        }
        let seconds = probe::probe(
            &self.inner.probe_http,
            file,
            self.inner.options.probe_timeout,
        )
        .await?
        .ok_or_else(|| PhiraError::probe(format!("Unable to read audio duration: {chart_id}")))?;
        self.inner.duration_cache.insert(chart_id, seconds);
        Ok(seconds)
    }

    fn get(&self, path: &str) -> Result<reqwest::RequestBuilder, PhiraError> {
        self.request(reqwest::Method::GET, path)
    }

    fn request(
        &self,
        method: reqwest::Method,
        path: &str,
    ) -> Result<reqwest::RequestBuilder, PhiraError> {
        let url = format!("{}{path}", self.inner.base_url);
        let url = reqwest::Url::parse(&url)
            .map_err(|error| PhiraError::new(Kind::InvalidUrl(format!("{url}: {error}"))))?;
        Ok(self
            .inner
            .http
            .request(method, url)
            .timeout(self.inner.options.request_timeout)
            .header(ACCEPT, "application/json"))
    }

    async fn fetch_json<T: serde::de::DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, PhiraError> {
        let body = self.execute(request).await?;
        gson::from_str_opt(&body)?.ok_or_else(|| PhiraError::new(Kind::EmptyBody))
    }

    async fn execute(&self, request: reqwest::RequestBuilder) -> Result<String, PhiraError> {
        let options = &self.inner.options;
        let attempts = options.max_attempts.max(1);
        let mut attempt = 1;
        loop {
            let Some(this_try) = request.try_clone() else {
                return Err(PhiraError::new(Kind::InvalidRequest(
                    "request body cannot be replayed".to_owned(),
                )));
            };
            let error = match self.execute_once(this_try).await {
                Ok(body) => return Ok(body),
                Err(error) => error,
            };
            if !error.is_retryable() || attempt >= attempts {
                return Err(error);
            }
            tracing::debug!(attempt, attempts, %error, "Phira request failed, retrying");
            tokio::time::sleep(backoff(attempt, options.retry_delay)).await;
            attempt += 1;
        }
    }

    async fn execute_once(&self, request: reqwest::RequestBuilder) -> Result<String, PhiraError> {
        let mut response = request
            .send()
            .await
            .map_err(|error| PhiraError::transport(&error))?;
        let status = response.status();
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| PhiraError::transport(&error))?
        {
            if body.len() + chunk.len() > MAX_BODY {
                return Err(PhiraError::new(Kind::TooLarge(MAX_BODY)));
            }
            body.extend_from_slice(&chunk);
        }
        let text = String::from_utf8_lossy(&body).into_owned();
        if status.is_success() {
            Ok(text)
        } else {
            Err(PhiraError::status_error(status.as_u16(), text))
        }
    }
}

fn backoff(failed_attempt: u32, unit: Duration) -> Duration {
    unit.saturating_mul(failed_attempt)
}

fn redirect_policy() -> Policy {
    Policy::custom(|attempt| {
        let downgrade = attempt
            .previous()
            .last()
            .is_some_and(|from| from.scheme() == "https" && attempt.url().scheme() == "http");
        // `previous()` includes the original request, so this follows MAX_REDIRECTS redirects.
        if attempt.previous().len() > MAX_REDIRECTS || downgrade {
            attempt.stop()
        } else {
            attempt.follow()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_java_constants() {
        let o = Options::default();
        assert_eq!(o.max_attempts, 8);
        assert_eq!(o.retry_delay, Duration::from_millis(150));
        assert_eq!(o.connect_timeout, Duration::from_secs(5));
        assert_eq!(o.request_timeout, Duration::from_secs(10));
        assert_eq!(o.probe_timeout, Duration::from_secs(20));
        assert_eq!(o.recent_record_ttl, Duration::from_secs(10));
        assert_eq!(DEFAULT_BASE_URL, "https://phira.5wyxi.com/");
        assert_eq!(USER_AGENT, "JPhira/1");
    }

    #[test]
    fn backoff_is_linear_and_has_no_wait_after_the_last_attempt() {
        let unit = Duration::from_millis(150);
        let waits: Vec<u128> = (1..8).map(|n| backoff(n, unit).as_millis()).collect();
        assert_eq!(waits, [150, 300, 450, 600, 750, 900, 1050]);
        assert_eq!(waits.iter().sum::<u128>(), 4200);
        assert_eq!(backoff(u32::MAX, Duration::MAX), Duration::MAX);
    }

    #[test]
    fn caches_have_the_java_sizes_and_lifetimes() {
        let client = PhiraClient::new("http://127.0.0.1:1/").unwrap();
        let minute = Duration::from_secs(60);
        let inner = &client.inner;
        assert_eq!(inner.token_cache.limits(), (10 * minute, 10_000));
        assert_eq!(inner.user_cache.limits(), (10 * minute, 5_000));
        assert_eq!(inner.chart_cache.limits(), (30 * minute, 10_000));
        assert_eq!(inner.record_cache.limits(), (30 * minute, 50_000));
        assert_eq!(
            inner.recent_record_cache.limits(),
            (Duration::from_secs(10), 5_000)
        );
        assert_eq!(inner.duration_cache.limits(), (6 * 60 * minute, 20_000));
    }

    #[test]
    fn base_url_gets_its_slash() {
        assert_eq!(
            PhiraClient::new("http://h").unwrap().inner.base_url,
            "http://h/"
        );
        assert_eq!(
            PhiraClient::new("http://h/").unwrap().inner.base_url,
            "http://h/"
        );
    }

    #[test]
    fn debug_shows_no_credentials() {
        let client = PhiraClient::new("http://h/").unwrap();
        let text = format!("{client:?}");
        assert!(
            text.contains("http://h/") && !text.contains("cache"),
            "{text}"
        );
    }
}
