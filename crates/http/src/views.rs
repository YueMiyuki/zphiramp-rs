use serde::Serialize;
use zphira_catalog::{ChartIndex, PoolSnapshot, SubmissionView};
use zphira_common::time::JavaOffsetDateTime;
use zphira_phira::ChartInfo;

#[derive(Serialize)]
pub(crate) struct PoolJson<'a> {
    id: i32,
    #[serde(rename = "favoriteId")]
    favorite_id: Option<i32>,
    default: bool,
    #[serde(rename = "chartIds")]
    chart_ids: &'a [i32],
    category: Option<&'static str>,
    #[serde(rename = "sizeLimit")]
    size_limit: Option<i32>,
    #[serde(rename = "roundsPerStay")]
    rounds_per_stay: Option<i32>,
    order: i32,
    #[serde(rename = "submissionOpen")]
    submission_open: bool,
}

impl<'a> From<&'a PoolSnapshot> for PoolJson<'a> {
    fn from(pool: &'a PoolSnapshot) -> Self {
        Self {
            id: pool.id,
            favorite_id: pool.favorite_id,
            default: pool.default_flag,
            chart_ids: &pool.chart_ids,
            category: Some(pool.category.name()),
            size_limit: pool.size_limit,
            rounds_per_stay: pool.rounds_per_stay,
            order: pool.order,
            submission_open: pool.submission_open,
        }
    }
}

#[derive(Serialize)]
pub(crate) struct ChartJson<'a> {
    id: i32,
    name: Option<&'a str>,
    level: Option<&'a str>,
    difficulty: f32,
    charter: Option<&'a str>,
    composer: Option<&'a str>,
    illustrator: Option<&'a str>,
    description: Option<&'a str>,
    ranked: bool,
    reviewed: bool,
    stable: bool,
    #[serde(rename = "stableRequest")]
    stable_request: bool,
    illustration: Option<&'a str>,
    preview: Option<&'a str>,
    file: Option<&'a str>,
    uploader: i32,
    tags: &'a [String],
    rating: f32,
    #[serde(rename = "ratingCount")]
    rating_count: i32,
    created: Option<&'a JavaOffsetDateTime>,
    updated: Option<&'a JavaOffsetDateTime>,
    #[serde(rename = "chartUpdated")]
    chart_updated: Option<&'a JavaOffsetDateTime>,
    #[serde(rename = "durationSeconds")]
    duration_seconds: Option<i32>,
}

impl<'a> From<&'a ChartInfo> for ChartJson<'a> {
    fn from(chart: &'a ChartInfo) -> Self {
        Self {
            id: chart.id,
            name: chart.name.as_deref(),
            level: chart.level.as_deref(),
            difficulty: chart.difficulty,
            charter: chart.charter.as_deref(),
            composer: chart.composer.as_deref(),
            illustrator: chart.illustrator.as_deref(),
            description: chart.description.as_deref(),
            ranked: chart.ranked,
            reviewed: chart.reviewed,
            stable: chart.stable,
            stable_request: chart.stable_request,
            illustration: chart.illustration.as_deref(),
            preview: chart.preview.as_deref(),
            file: chart.file.as_deref(),
            uploader: chart.uploader,
            tags: chart.tags.as_deref().unwrap_or(&[]),
            rating: chart.rating,
            rating_count: chart.rating_count,
            created: chart.created.as_ref(),
            updated: chart.updated.as_ref(),
            chart_updated: chart.chart_updated.as_ref(),
            duration_seconds: chart.duration_seconds(),
        }
    }
}

#[derive(Serialize)]
struct SubmitterJson {
    #[serde(rename = "userId")]
    user_id: i32,
    name: String,
    at: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct SubmissionJson {
    #[serde(rename = "poolId")]
    pool_id: i32,
    #[serde(rename = "chartId")]
    chart_id: i32,
    status: &'static str,
    chart: Option<OwnedChart>,
    #[serde(rename = "submitterCount")]
    submitter_count: usize,
    submitters: Vec<SubmitterJson>,
    #[serde(rename = "createdAt")]
    created_at: Option<String>,
    #[serde(rename = "reviewedAt")]
    reviewed_at: Option<String>,
    #[serde(rename = "reviewerId")]
    reviewer_id: Option<i32>,
    reason: Option<String>,
}

pub(crate) struct OwnedChart(std::sync::Arc<ChartInfo>);

impl Serialize for OwnedChart {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ChartJson::from(self.0.as_ref()).serialize(serializer)
    }
}

impl SubmissionJson {
    pub(crate) fn new(view: SubmissionView, index: &ChartIndex) -> Self {
        Self {
            pool_id: view.pool_id,
            chart_id: view.chart_id,
            status: view.status.name(),
            chart: index.get(view.chart_id).map(OwnedChart),
            submitter_count: view.submitters.len(),
            submitters: view
                .submitters
                .into_iter()
                .map(|s| SubmitterJson {
                    user_id: s.user_id,
                    name: s.name.unwrap_or_default(),
                    at: s.at.map(|at| at.to_string()),
                })
                .collect(),
            created_at: view.created_at.map(|t| t.to_string()),
            reviewed_at: view.reviewed_at.map(|t| t.to_string()),
            reviewer_id: view.reviewer_id,
            reason: view.reason,
        }
    }
}
