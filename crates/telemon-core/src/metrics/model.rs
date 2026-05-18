use std::collections::BTreeMap;

use thiserror::Error;

use crate::metrics::names;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricKind {
    Gauge,
    Counter,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MetricSample {
    pub name: String,
    pub help: String,
    pub kind: MetricKind,
    pub labels: BTreeMap<String, String>,
    pub value: f64,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MetricError {
    #[error("project metric name must start with telemon_: {0}")]
    InvalidProjectMetricName(String),
}

impl MetricKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MetricKind::Gauge => "gauge",
            MetricKind::Counter => "counter",
        }
    }
}

impl MetricSample {
    pub fn new(
        name: impl Into<String>,
        help: impl Into<String>,
        kind: MetricKind,
        labels: BTreeMap<String, String>,
        value: f64,
    ) -> Result<Self, MetricError> {
        let name = name.into();
        if !name.starts_with(names::PREFIX) {
            return Err(MetricError::InvalidProjectMetricName(name));
        }

        Ok(Self {
            name,
            help: help.into(),
            kind,
            labels,
            value,
        })
    }

    pub fn gauge(
        name: impl Into<String>,
        help: impl Into<String>,
        labels: BTreeMap<String, String>,
        value: f64,
    ) -> Self {
        Self::new(name, help, MetricKind::Gauge, labels, value)
            .expect("project metric constants must use telemon_ prefix")
    }

    pub fn counter(
        name: impl Into<String>,
        help: impl Into<String>,
        labels: BTreeMap<String, String>,
        value: f64,
    ) -> Self {
        Self::new(name, help, MetricKind::Counter, labels, value)
            .expect("project metric constants must use telemon_ prefix")
    }
}

pub fn labels(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_project_metric_names() {
        let result = MetricSample::new(
            "other_metric",
            "help",
            MetricKind::Gauge,
            BTreeMap::new(),
            1.0,
        );

        assert_eq!(
            result.unwrap_err(),
            MetricError::InvalidProjectMetricName("other_metric".to_string())
        );
    }
}
