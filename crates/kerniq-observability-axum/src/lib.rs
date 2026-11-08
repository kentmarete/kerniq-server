//! HTTP instrumentation for Kerniq Server and Axum.
//!
//! [`HttpInstrumentation`] is a Tower layer that records the OpenTelemetry
//! HTTP server metrics and, with the `tracing` feature, opens one `tracing`
//! span per request whose parent is taken from the incoming W3C
//! `traceparent`/`baggage` headers. It depends only on the OpenTelemetry
//! *API*: whichever SDK (or none) the application configured receives the
//! measurements. `kerniq-observability` wires it up automatically.
//!
//! ```
//! use kerniq_observability_axum::HttpInstrumentation;
//! use opentelemetry::metrics::MeterProvider;
//! # let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder().build();
//!
//! let layer = HttpInstrumentation::new()
//!     .meter(&provider.meter("kerniq.http"))
//!     .exclude("/health")
//!     .into_layer();
//! let app: axum::Router = axum::Router::new()
//!     .route("/users/{id}", axum::routing::get(|| async { "user" }))
//!     .layer(layer); // `Router::layer`: per route, so `http.route` is known
//! ```
//!
//! # What is measured
//!
//! | Metric | Instrument | Attributes |
//! |---|---|---|
//! | `http.server.request.duration` (s) | Histogram | method, scheme, protocol, route, status, error.type |
//! | `http.server.active_requests` | UpDownCounter | method, scheme |
//! | `http.server.request.body.size` (By) | Histogram | as duration; only when `Content-Length` is present |
//! | `http.server.response.body.size` (By) | Histogram | as duration; bytes actually written |
//! | `kerniq.http.server.time_to_first_byte` (s) | Histogram | as duration; responses with a body |
//!
//! # Streaming
//!
//! The response body is wrapped, never buffered: each frame is passed
//! through as soon as the inner body yields it, and the wrapper only adds
//! the frame length to a counter. A request is *active* until its response
//! body ends, so a server-sent-events stream counts as active for its whole
//! lifetime, and its duration is the stream lifetime. Responses that end
//! early are labeled with `error.type`:
//!
//! * `kerniq.aborted` — the client disconnected, or the request was
//!   cancelled, before the response finished,
//! * `kerniq.body_error` — the body returned an error mid-stream,
//! * `kerniq.service_error` — the inner service failed,
//! * `500`, `503`, ... — a 5xx status.
//!
//! Active *connections* are a different quantity (one HTTP/2 connection
//! carries many requests; a keep-alive HTTP/1 connection carries zero or
//! one) and are not measured by this layer.
//!
//! # Cardinality
//!
//! `http.route` is the route *template* from Axum's `MatchedPath`
//! (`/users/{id}`, never `/users/42`). Requests that match no route carry no
//! `http.route`. Methods outside the standard set become `_OTHER`. Raw
//! paths, query strings, request ids and user agents only ever appear on
//! spans.

#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]
// Reduced feature sets compile out the code that reads some fields.
#![cfg_attr(
    not(all(feature = "metrics", feature = "tracing")),
    allow(dead_code, unused_imports)
)]

mod body;
mod layer;
#[cfg(feature = "metrics")]
mod metrics;
#[cfg(feature = "tracing")]
pub mod propagation;

pub use body::ObservedBody;
pub use layer::{HttpInstrumentation, HttpLayer, HttpService, ResponseFuture};
#[cfg(feature = "metrics")]
pub use metrics::HttpMetrics;
