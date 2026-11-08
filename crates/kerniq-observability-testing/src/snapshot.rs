//! A flat, assertion-friendly view of collected metrics.
//!
//! This module depends only on `opentelemetry_sdk`, so other Kerniq crates
//! can include it in their own tests without a dependency cycle.

use std::collections::BTreeMap;

use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData, ResourceMetrics};

/// The value of one data point.
#[derive(Debug, Clone, PartialEq)]
pub enum PointValue {
    /// A counter or up-down counter value.
    Sum(f64),
    /// A gauge value.
    Gauge(f64),
    /// A histogram: number of measurements, their sum, and the per-bucket
    /// counts (one more than `bounds`, the last bucket is `+Inf`).
    Histogram {
        /// Number of recorded values.
        count: u64,
        /// Sum of recorded values.
        sum: f64,
        /// Explicit bucket upper bounds.
        bounds: Vec<f64>,
        /// Count per bucket.
        buckets: Vec<u64>,
    },
}

/// One data point of one metric.
#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    /// Instrument name.
    pub name: String,
    /// Instrument unit.
    pub unit: String,
    /// Attributes, stringified.
    pub attributes: BTreeMap<String, String>,
    /// The value.
    pub value: PointValue,
}

impl Point {
    /// The attribute value for `key`.
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attributes.get(key).map(String::as_str)
    }

    /// Counter/gauge value, or histogram count.
    pub fn number(&self) -> f64 {
        match &self.value {
            PointValue::Sum(v) | PointValue::Gauge(v) => *v,
            PointValue::Histogram { count, .. } => *count as f64,
        }
    }

    /// Histogram sum (0 for other kinds).
    pub fn sum(&self) -> f64 {
        match &self.value {
            PointValue::Histogram { sum, .. } => *sum,
            PointValue::Sum(v) | PointValue::Gauge(v) => *v,
        }
    }
}

/// All data points from one or more collections. When several collections
/// are included (cumulative temporality), use [`MetricSnapshot::latest`]
/// semantics: the last collection wins.
#[derive(Debug, Clone, Default)]
pub struct MetricSnapshot {
    /// Every data point, in collection order.
    pub points: Vec<Point>,
}

impl MetricSnapshot {
    /// Flattens the *last* `ResourceMetrics` in `collections` (cumulative
    /// temporality makes the last one complete).
    pub fn latest(collections: &[ResourceMetrics]) -> Self {
        collections
            .last()
            .map(Self::from_resource)
            .unwrap_or_default()
    }

    /// Flattens one collection.
    pub fn from_resource(rm: &ResourceMetrics) -> Self {
        let mut points = Vec::new();
        for scope in rm.scope_metrics() {
            for metric in scope.metrics() {
                let name = metric.name().to_owned();
                let unit = metric.unit().to_owned();
                let mut push = |attrs: BTreeMap<String, String>, value: PointValue| {
                    points.push(Point {
                        name: name.clone(),
                        unit: unit.clone(),
                        attributes: attrs,
                        value,
                    });
                };
                macro_rules! attrs {
                    ($p:expr) => {
                        $p.attributes()
                            .map(|kv| (kv.key.as_str().to_owned(), kv.value.as_str().into_owned()))
                            .collect::<BTreeMap<_, _>>()
                    };
                }
                macro_rules! data {
                    ($data:expr, $conv:expr) => {
                        match $data {
                            MetricData::Sum(s) => {
                                for p in s.data_points() {
                                    push(attrs!(p), PointValue::Sum($conv(p.value())));
                                }
                            }
                            MetricData::Gauge(g) => {
                                for p in g.data_points() {
                                    push(attrs!(p), PointValue::Gauge($conv(p.value())));
                                }
                            }
                            MetricData::Histogram(h) => {
                                for p in h.data_points() {
                                    push(
                                        attrs!(p),
                                        PointValue::Histogram {
                                            count: p.count(),
                                            sum: $conv(p.sum()),
                                            bounds: p.bounds().collect(),
                                            buckets: p.bucket_counts().collect(),
                                        },
                                    );
                                }
                            }
                            MetricData::ExponentialHistogram(_) => {}
                        }
                    };
                }
                match metric.data() {
                    AggregatedMetrics::F64(d) => data!(d, |v: f64| v),
                    AggregatedMetrics::U64(d) => data!(d, |v: u64| v as f64),
                    AggregatedMetrics::I64(d) => data!(d, |v: i64| v as f64),
                }
            }
        }
        Self { points }
    }

    /// Points of the metric `name`.
    pub fn metric<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Point> + 'a {
        self.points.iter().filter(move |p| p.name == name)
    }

    /// The single point of `name` whose attributes include all of `attrs`.
    pub fn find(&self, name: &str, attrs: &[(&str, &str)]) -> Option<&Point> {
        self.points
            .iter()
            .filter(|p| p.name == name)
            .find(|p| attrs.iter().all(|(k, v)| p.attr(k) == Some(*v)))
    }

    /// Sum of [`Point::number`] over every point of `name` matching `attrs`.
    pub fn total(&self, name: &str, attrs: &[(&str, &str)]) -> f64 {
        self.metric(name)
            .filter(|p| attrs.iter().all(|(k, v)| p.attr(k) == Some(*v)))
            .map(Point::number)
            .sum()
    }

    /// Every distinct value of attribute `key` on metric `name`.
    pub fn attribute_values(&self, name: &str, key: &str) -> Vec<String> {
        let mut values: Vec<String> = self
            .metric(name)
            .filter_map(|p| p.attr(key).map(str::to_owned))
            .collect();
        values.sort();
        values.dedup();
        values
    }

    /// Whether any metric named `name` was collected.
    pub fn contains(&self, name: &str) -> bool {
        self.metric(name).next().is_some()
    }
}
