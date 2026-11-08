//! OTLP exporter configuration and construction.
//!
//! The exporters are `opentelemetry-otlp`'s; this module only maps Kerniq's
//! configuration onto their builders. Unset options fall back to the
//! standard `OTEL_EXPORTER_OTLP_*` environment variables.

use std::collections::HashMap;
use std::time::Duration;

use opentelemetry_otlp::{
    Compression, RetryPolicy, WithExportConfig, WithHttpConfig, WithTonicConfig,
};

use crate::Error;

/// OTLP transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// OTLP/gRPC (default port 4317). Runs on the application's Tokio
    /// runtime.
    Grpc,
    /// OTLP/HTTP with protobuf payloads (default port 4318). Uses a blocking
    /// HTTP client on the export thread.
    HttpProtobuf,
}

/// TLS settings for OTLP/gRPC (feature `otlp-tls`). OTLP/HTTP uses rustls
/// with the platform roots for `https://` endpoints.
#[cfg(feature = "otlp-tls")]
#[derive(Debug, Clone, Default)]
pub struct TlsConfig {
    /// Expected server name, when it differs from the endpoint host.
    pub domain: Option<String>,
    /// PEM CA certificate(s) to trust in addition to the platform roots.
    pub ca_pem: Option<Vec<u8>>,
    /// PEM client certificate and key, for mutual TLS.
    pub client_identity_pem: Option<(Vec<u8>, Vec<u8>)>,
}

/// Retry policy for failed exports (exponential backoff with jitter).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retry {
    /// Retries after the first attempt (0 disables retrying).
    pub max_retries: usize,
    /// First backoff delay.
    pub initial_delay: Duration,
    /// Backoff ceiling.
    pub max_delay: Duration,
}

impl Retry {
    /// `opentelemetry-otlp`'s recommended policy: 3 retries, 100 ms to 1.6 s.
    pub const RECOMMENDED: Self = Self {
        max_retries: 3,
        initial_delay: Duration::from_millis(100),
        max_delay: Duration::from_millis(1600),
    };
    /// A single attempt.
    pub const DISABLED: Self = Self {
        max_retries: 0,
        initial_delay: Duration::ZERO,
        max_delay: Duration::ZERO,
    };

    fn policy(self) -> RetryPolicy {
        if self.max_retries == 0 {
            return RetryPolicy::disabled();
        }
        RetryPolicy::recommended()
            .with_max_retries(self.max_retries)
            .with_initial_delay(self.initial_delay)
            .with_max_delay(self.max_delay)
    }
}

/// One OTLP destination for one signal.
///
/// ```
/// use std::time::Duration;
/// use kerniq_observability::OtlpExporter;
///
/// let exporter = OtlpExporter::grpc()
///     .endpoint("http://otel-collector:4317")
///     .header("authorization", "Bearer ${TOKEN}")
///     .timeout(Duration::from_secs(5))
///     .gzip();
/// ```
#[derive(Clone)]
#[must_use]
pub struct OtlpExporter {
    pub(crate) protocol: Protocol,
    pub(crate) endpoint: Option<String>,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) gzip: bool,
    pub(crate) retry: Retry,
    #[cfg(feature = "otlp-tls")]
    pub(crate) tls: Option<TlsConfig>,
}

impl std::fmt::Debug for OtlpExporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Header values are credentials: never print them.
        let header_names: Vec<&str> = self.headers.iter().map(|(k, _)| k.as_str()).collect();
        f.debug_struct("OtlpExporter")
            .field("protocol", &self.protocol)
            .field("endpoint", &self.endpoint)
            .field("headers", &header_names)
            .field("timeout", &self.timeout)
            .field("gzip", &self.gzip)
            .field("retry", &self.retry)
            .finish_non_exhaustive()
    }
}

impl Default for OtlpExporter {
    fn default() -> Self {
        Self::grpc()
    }
}

impl OtlpExporter {
    /// OTLP/gRPC.
    pub fn grpc() -> Self {
        Self::with_protocol(Protocol::Grpc)
    }

    /// OTLP/HTTP (protobuf).
    pub fn http() -> Self {
        Self::with_protocol(Protocol::HttpProtobuf)
    }

    fn with_protocol(protocol: Protocol) -> Self {
        Self {
            protocol,
            endpoint: None,
            headers: Vec::new(),
            timeout: None,
            gzip: false,
            retry: Retry::RECOMMENDED,
            #[cfg(feature = "otlp-tls")]
            tls: None,
        }
    }

    /// The Collector or backend URL. For OTLP/HTTP a URL without a path
    /// (`http://collector:4318`) gets the signal path (`/v1/traces`,
    /// `/v1/metrics`, `/v1/logs`) appended; a URL with a path is used as-is.
    /// Default: `OTEL_EXPORTER_OTLP_[SIGNAL_]ENDPOINT`, else localhost.
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    /// Adds a header (gRPC metadata) sent with every export, e.g. an API key.
    /// Values are never logged.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers
            .push((name.into().to_ascii_lowercase(), value.into()));
        self
    }

    /// Per-export timeout. Default `OTEL_EXPORTER_OTLP_TIMEOUT`, else 10 s.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Compresses payloads with gzip.
    pub fn gzip(mut self) -> Self {
        self.gzip = true;
        self
    }

    /// Retry policy. Default [`Retry::RECOMMENDED`].
    pub fn retry(mut self, retry: Retry) -> Self {
        self.retry = retry;
        self
    }

    /// TLS settings (gRPC).
    #[cfg(feature = "otlp-tls")]
    pub fn tls(mut self, tls: TlsConfig) -> Self {
        self.tls = Some(tls);
        self
    }

    #[cfg_attr(not(any(feature = "tracing", feature = "logs")), allow(dead_code))]
    pub(crate) fn with_default_timeout(&self, timeout: Duration) -> Self {
        let mut this = self.clone();
        this.timeout.get_or_insert(timeout);
        this
    }

    /// The configured protocol.
    pub fn protocol(&self) -> Protocol {
        self.protocol
    }

    fn http_endpoint(&self, signal_path: &str) -> Option<String> {
        let endpoint = self.endpoint.as_deref()?;
        let trimmed = endpoint.trim_end_matches('/');
        let has_path = trimmed
            .split_once("://")
            .is_some_and(|(_, rest)| rest.contains('/'));
        Some(if has_path {
            endpoint.to_owned()
        } else {
            format!("{trimmed}{signal_path}")
        })
    }

    fn metadata(&self) -> Result<opentelemetry_otlp::tonic_types::metadata::MetadataMap, Error> {
        let mut map = HashMap::with_capacity(self.headers.len());
        for (k, v) in &self.headers {
            map.insert(k.clone(), v.clone());
        }
        let headers = http_header_map(&map)?;
        Ok(opentelemetry_otlp::tonic_types::metadata::MetadataMap::from_headers(headers))
    }

    pub(crate) fn validate(&self) -> Result<(), Error> {
        if let Some(endpoint) = &self.endpoint
            && !(endpoint.starts_with("http://") || endpoint.starts_with("https://"))
        {
            return Err(Error::Config(format!(
                "OTLP endpoint `{endpoint}` must start with http:// or https://"
            )));
        }
        for (k, _) in &self.headers {
            if http::HeaderName::from_bytes(k.as_bytes()).is_err() {
                return Err(Error::Config(format!("invalid OTLP header name `{k}`")));
            }
        }
        Ok(())
    }
}

fn http_header_map(headers: &HashMap<String, String>) -> Result<http::HeaderMap, Error> {
    let mut map = http::HeaderMap::with_capacity(headers.len());
    for (k, v) in headers {
        let name = http::HeaderName::from_bytes(k.as_bytes())
            .map_err(|_| Error::Config(format!("invalid OTLP header name `{k}`")))?;
        let value = http::HeaderValue::from_str(v)
            .map_err(|_| Error::Config(format!("invalid value for OTLP header `{k}`")))?;
        map.insert(name, value);
    }
    Ok(map)
}

macro_rules! build_exporter {
    ($config:expr, $builder:expr, $signal:literal, $path:literal) => {{
        let config: &OtlpExporter = $config;
        config.validate()?;
        let map_err = |source: opentelemetry_otlp::ExporterBuildError| Error::Exporter {
            signal: $signal,
            source: Box::new(source),
        };
        match config.protocol {
            Protocol::Grpc => {
                let mut b = $builder.with_tonic();
                if let Some(endpoint) = &config.endpoint {
                    b = b.with_endpoint(endpoint.clone());
                }
                if let Some(timeout) = config.timeout {
                    b = b.with_timeout(timeout);
                }
                if !config.headers.is_empty() {
                    b = b.with_metadata(config.metadata()?);
                }
                if config.gzip {
                    b = b.with_compression(Compression::Gzip);
                }
                b = b.with_retry_policy(config.retry.policy());
                #[cfg(feature = "otlp-tls")]
                if let Some(tls) = &config.tls {
                    b = b.with_tls_config(tls_config(tls));
                }
                b.build().map_err(map_err)
            }
            Protocol::HttpProtobuf => {
                let mut b = $builder.with_http();
                if let Some(endpoint) = config.http_endpoint($path) {
                    b = b.with_endpoint(endpoint);
                }
                if let Some(timeout) = config.timeout {
                    b = b.with_timeout(timeout);
                }
                if !config.headers.is_empty() {
                    b = b.with_headers(config.headers.iter().cloned().collect());
                }
                if config.gzip {
                    b = b.with_compression(Compression::Gzip);
                }
                b = b.with_retry_policy(config.retry.policy());
                b.build().map_err(map_err)
            }
        }
    }};
}

#[cfg(feature = "otlp-tls")]
fn tls_config(tls: &TlsConfig) -> opentelemetry_otlp::tonic_types::transport::ClientTlsConfig {
    use opentelemetry_otlp::tonic_types::transport::{Certificate, ClientTlsConfig, Identity};
    let mut config = ClientTlsConfig::new().with_native_roots();
    if let Some(domain) = &tls.domain {
        config = config.domain_name(domain.clone());
    }
    if let Some(ca) = &tls.ca_pem {
        config = config.ca_certificate(Certificate::from_pem(ca));
    }
    if let Some((cert, key)) = &tls.client_identity_pem {
        config = config.identity(Identity::from_pem(cert, key));
    }
    config
}

#[cfg(feature = "tracing")]
pub(crate) fn span_exporter(
    config: &OtlpExporter,
) -> Result<opentelemetry_otlp::SpanExporter, Error> {
    build_exporter!(
        config,
        opentelemetry_otlp::SpanExporter::builder(),
        "traces",
        "/v1/traces"
    )
}

#[cfg(feature = "logs")]
pub(crate) fn log_exporter(
    config: &OtlpExporter,
) -> Result<opentelemetry_otlp::LogExporter, Error> {
    build_exporter!(
        config,
        opentelemetry_otlp::LogExporter::builder(),
        "logs",
        "/v1/logs"
    )
}

#[cfg(feature = "metrics")]
pub(crate) fn metric_exporter(
    config: &OtlpExporter,
    delta: bool,
) -> Result<opentelemetry_otlp::MetricExporter, Error> {
    let temporality = if delta {
        opentelemetry_sdk::metrics::Temporality::Delta
    } else {
        opentelemetry_sdk::metrics::Temporality::Cumulative
    };
    build_exporter!(
        config,
        opentelemetry_otlp::MetricExporter::builder().with_temporality(temporality),
        "metrics",
        "/v1/metrics"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_endpoint_gets_signal_path() {
        let e = OtlpExporter::http().endpoint("http://collector:4318");
        assert_eq!(
            e.http_endpoint("/v1/traces").unwrap(),
            "http://collector:4318/v1/traces"
        );
        let e = OtlpExporter::http().endpoint("http://collector:4318/");
        assert_eq!(
            e.http_endpoint("/v1/logs").unwrap(),
            "http://collector:4318/v1/logs"
        );
        let e = OtlpExporter::http().endpoint("https://ingest.example.com/otlp/v1/traces");
        assert_eq!(
            e.http_endpoint("/v1/traces").unwrap(),
            "https://ingest.example.com/otlp/v1/traces"
        );
        assert!(OtlpExporter::http().http_endpoint("/v1/traces").is_none());
    }

    #[test]
    fn validation() {
        assert!(
            OtlpExporter::grpc()
                .endpoint("collector:4317")
                .validate()
                .is_err()
        );
        assert!(
            OtlpExporter::grpc()
                .header("bad header", "x")
                .validate()
                .is_err()
        );
        assert!(
            OtlpExporter::grpc()
                .endpoint("http://c:4317")
                .header("x-api-key", "k")
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn debug_hides_header_values() {
        let e = OtlpExporter::grpc().header("authorization", "Bearer secret-token");
        let debug = format!("{e:?}");
        assert!(debug.contains("authorization"));
        assert!(!debug.contains("secret-token"));
    }
}
