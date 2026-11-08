//! Shared telemetry conventions for the Kerniq observability crates.
//!
//! This crate deliberately contains no telemetry engine. It depends only on
//! the OpenTelemetry *API* crate and provides:
//!
//! * [`semconv`] — the metric and attribute names Kerniq emits, following the
//!   OpenTelemetry HTTP semantic conventions, plus Kerniq's documented
//!   extensions. Dashboards and tests use these constants, so names cannot
//!   drift between the implementation and the assets that consume it.
//! * [`attributes`] — helpers that keep metric attributes bounded
//!   (normalized HTTP methods, protocol versions, allow-listed labels).
//! * [`resource`] — service identity (`service.name`, `service.version`,
//!   `deployment.environment.name`, `service.instance.id`).
//! * [`inference`] — reusable instruments and a request guard for
//!   long-running AI inference workloads.
//!
//! Application metrics are created directly with the OpenTelemetry
//! [`Meter`](opentelemetry::metrics::Meter) API; Kerniq does not wrap it.

#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod attributes;
pub mod inference;
pub mod resource;
pub mod semconv;

pub use attributes::BoundedLabel;
pub use resource::ServiceInfo;

/// Re-export of the OpenTelemetry API crate at the version Kerniq uses.
pub use opentelemetry;
pub use opentelemetry::KeyValue;
