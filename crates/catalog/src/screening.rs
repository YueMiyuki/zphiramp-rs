use std::collections::BTreeSet;

use zphira_phira::ChartInfo;

use crate::category::PoolCategory;

#[derive(Debug, Clone, PartialEq)]
pub struct ScreeningRule {
    pub category: PoolCategory,
    pub any_tags: BTreeSet<String>,
    pub min_rating: f32,
    pub min_rating_count: i32,
    pub min_difficulty: Option<f64>,
    pub min_duration_seconds: Option<i32>,
}

impl ScreeningRule {
    pub fn new<I, S>(
        category: PoolCategory,
        any_tags: I,
        min_rating: f32,
        min_rating_count: i32,
        min_difficulty: Option<f64>,
        min_duration_seconds: Option<i32>,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            category,
            any_tags: any_tags.into_iter().map(Into::into).collect(),
            min_rating,
            min_rating_count,
            min_difficulty,
            min_duration_seconds,
        }
    }

    pub fn regular() -> Self {
        Self::new(
            PoolCategory::Regular,
            ["regular"],
            0.84,
            5,
            Some(17.0),
            None,
        )
    }

    pub fn configured() -> Self {
        Self::new(PoolCategory::Configured, ["plain"], 0.80, 5, None, None)
    }

    pub fn tb() -> Self {
        Self::new(
            PoolCategory::Tb,
            ["regular", "plain"],
            0.84,
            5,
            None,
            Some(360),
        )
    }

    pub fn of(category: PoolCategory) -> Option<Self> {
        match category {
            PoolCategory::Regular => Some(Self::regular()),
            PoolCategory::Configured => Some(Self::configured()),
            PoolCategory::Tb => Some(Self::tb()),
            PoolCategory::Manual => None,
        }
    }

    pub fn without_duration(&self) -> Self {
        Self {
            min_duration_seconds: None,
            ..self.clone()
        }
    }

    pub fn matches(&self, chart: &ChartInfo) -> bool {
        if !self.any_tags.is_empty() && !self.any_tags.iter().any(|tag| chart.has_tag(tag)) {
            return false;
        }
        // Compared in f32 on purpose: in f64 a chart exactly on the bound would pass.
        if chart.rating <= self.min_rating || chart.rating_count < self.min_rating_count {
            return false;
        }
        if let Some(min) = self.min_difficulty
            && f64::from(chart.difficulty) < min
        {
            return false;
        }
        self.test_duration(chart)
    }

    pub fn test_duration(&self, chart: &ChartInfo) -> bool {
        match self.min_duration_seconds {
            None => true,
            Some(min) => chart.duration_seconds().is_some_and(|d| d >= min),
        }
    }
}
