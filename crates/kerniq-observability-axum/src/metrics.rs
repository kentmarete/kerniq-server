//! The HTTP server instruments.

use opentelemetry::metrics::{Histogram, Meter, UpDownCounter};

use kerniq_telemetry::semconv::{buckets, http, kerniq};

/// The HTTP server instruments, created once per application and shared by
/// every route (cloning is a few `Arc` bumps).
#[derive(Debug, Clone)]
pub struct HttpMetrics {
    pub(crate) duration: Histogram<f64>,
    pub(crate) active: UpDownCounter<i64>,
    pub(crate) request_size: Histogram<u64>,
    pub(crate) response_size: Histogram<u64>,
    pub(crate) time_to_first_byte: Histogram<f64>,
}

impl HttpMetrics {
    /// Creates the instruments on `meter`.
    pub fn new(meter: &Meter) -> Self {
        Self {
            duration: meter
                .f64_histogram(http::SERVER_REQUEST_DURATION)
                .with_unit("s")
                .with_description(
                    "Duration of HTTP server requests, until the response body ended.",
                )
                .with_boundaries(buckets::DURATION_SECONDS.to_vec())
                .build(),
            active: meter
                .i64_up_down_counter(http::SERVER_ACTIVE_REQUESTS)
                .with_unit("{request}")
                .with_description("Number of HTTP server requests whose response has not finished.")
                .build(),
            request_size: meter
                .u64_histogram(http::SERVER_REQUEST_BODY_SIZE)
                .with_unit("By")
                .with_description("Size of HTTP server request bodies (from Content-Length).")
                .with_boundaries(buckets::BODY_SIZE_BYTES.to_vec())
                .build(),
            response_size: meter
                .u64_histogram(http::SERVER_RESPONSE_BODY_SIZE)
                .with_unit("By")
                .with_description("Size of HTTP server response bodies, as written.")
                .with_boundaries(buckets::BODY_SIZE_BYTES.to_vec())
                .build(),
            time_to_first_byte: meter
                .f64_histogram(kerniq::SERVER_TIME_TO_FIRST_BYTE)
                .with_unit("s")
                .with_description("Time until the first response body frame was produced.")
                .with_boundaries(buckets::DURATION_SECONDS.to_vec())
                .build(),
        }
    }
}
