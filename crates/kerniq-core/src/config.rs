//! Server configuration.
//!
//! [`ServerConfig`] can be built programmatically, read from `KERNIQ_*`
//! environment variables, or (with the `config-file` feature) loaded from a
//! TOML file. Sources can be layered:
//!
//! ```no_run
//! # #[cfg(feature = "config-file")]
//! # fn demo() -> Result<(), kerniq_core::ConfigError> {
//! use kerniq_core::ServerConfig;
//!
//! // Defaults, overridden by a file, overridden by the environment.
//! let config = ServerConfig::from_file("kerniq.toml")?.apply_env()?;
//! # Ok(()) }
//! ```
//!
//! # What each setting controls
//!
//! The three concurrency-related settings are deliberately separate because
//! they are enforced at different layers:
//!
//! | Setting | Enforced by | Meaning |
//! |---|---|---|
//! | `workers` | Tokio | Worker threads of the runtime. Only honoured by `run_blocking`, because with `#[tokio::main]` *you* own the runtime. |
//! | `max_connections` | Accept loop | Open TCP connections. When reached, the server stops accepting until one closes. |
//! | `max_concurrent_requests` | Tower middleware | Requests being handled at once, across all connections. Excess requests wait. |
//!
//! Every setting listed here is honoured by the server; there are no
//! placeholder options.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Errors produced while loading or validating configuration.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// A value could not be parsed or is out of range.
    #[error("invalid value {value:?} for `{key}`: {reason}")]
    InvalidValue {
        /// The setting or environment variable name.
        key: String,
        /// The offending value.
        value: String,
        /// Why it was rejected.
        reason: String,
    },
    /// A configuration file could not be read.
    #[error("failed to read configuration file `{path}`: {source}")]
    Read {
        /// The file path.
        path: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// A configuration file could not be parsed.
    #[error("failed to parse configuration file `{path}`: {message}")]
    Parse {
        /// The file path.
        path: String,
        /// The parser's message.
        message: String,
    },
}

/// Configuration for the HTTP server.
///
/// Fields are public for reading; the same-named builder methods consume and
/// return the config for fluent construction:
///
/// ```
/// use std::time::Duration;
/// use kerniq_core::ServerConfig;
///
/// let config = ServerConfig::new()
///     .host("0.0.0.0")
///     .port(3000)
///     .workers(4)
///     .request_timeout(Duration::from_secs(30))
///     .max_body_size(10 * 1024 * 1024);
///
/// assert_eq!(config.address(), "0.0.0.0:3000");
/// assert_eq!(config.workers, Some(4));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    /// Interface to bind to. Default: `127.0.0.1`.
    pub host: String,
    /// Port to bind to. `0` selects an ephemeral port. Default: `3000`.
    pub port: u16,
    /// Tokio worker threads used by `run_blocking`. Default: one per CPU core.
    pub workers: Option<usize>,
    /// Maximum time to produce a response. Exceeding it yields
    /// `408 REQUEST_TIMEOUT`. Default: no timeout.
    #[serde(with = "secs::option")]
    pub request_timeout: Option<Duration>,
    /// Maximum request body size in bytes accepted by body extractors
    /// (`Json`, `String`, `Bytes`, ...). Larger bodies yield
    /// `413 PAYLOAD_TOO_LARGE`. Default: 2 MiB.
    pub max_body_size: usize,
    /// Maximum number of open TCP connections. Default: unlimited.
    pub max_connections: Option<usize>,
    /// Maximum number of requests processed concurrently. Default: unlimited.
    pub max_concurrent_requests: Option<usize>,
    /// How long graceful shutdown waits for in-flight requests and background
    /// tasks before giving up. Default: 30 seconds.
    #[serde(with = "secs")]
    pub shutdown_timeout: Duration,
    /// Maximum time for a client to send the request headers (protects
    /// against slow-header attacks). `0` disables it, which removes a
    /// per-request timer (a few percent of throughput on trivial endpoints)
    /// at the cost of that protection. Default: 30 seconds.
    #[serde(with = "secs")]
    pub header_read_timeout: Duration,
    /// Disable Nagle's algorithm on accepted sockets. Default: `true`.
    pub tcp_nodelay: bool,
    /// Make the peer address available to handlers through
    /// `ConnectInfo<SocketAddr>`. Costs one small allocation per request.
    /// Default: `true`.
    pub connect_info: bool,
}

/// The default `max_body_size` (2 MiB, the same default as Axum).
pub const DEFAULT_MAX_BODY_SIZE: usize = 2 * 1024 * 1024;

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_owned(),
            port: 3000,
            workers: None,
            request_timeout: None,
            max_body_size: DEFAULT_MAX_BODY_SIZE,
            max_connections: None,
            max_concurrent_requests: None,
            shutdown_timeout: Duration::from_secs(30),
            header_read_timeout: Duration::from_secs(30),
            tcp_nodelay: true,
            connect_info: true,
        }
    }
}

impl ServerConfig {
    /// Creates a configuration with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the interface to bind to.
    #[must_use]
    pub fn host(mut self, host: impl Into<String>) -> Self {
        self.host = host.into();
        self
    }

    /// Sets the port to bind to.
    #[must_use]
    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Sets the number of Tokio worker threads used by `run_blocking`.
    #[must_use]
    pub fn workers(mut self, workers: usize) -> Self {
        self.workers = Some(workers);
        self
    }

    /// Sets the per-request timeout.
    #[must_use]
    pub fn request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = Some(timeout);
        self
    }

    /// Sets the maximum request body size in bytes.
    #[must_use]
    pub fn max_body_size(mut self, bytes: usize) -> Self {
        self.max_body_size = bytes;
        self
    }

    /// Limits the number of open TCP connections.
    #[must_use]
    pub fn max_connections(mut self, max: usize) -> Self {
        self.max_connections = Some(max);
        self
    }

    /// Limits the number of requests processed concurrently.
    #[must_use]
    pub fn max_concurrent_requests(mut self, max: usize) -> Self {
        self.max_concurrent_requests = Some(max);
        self
    }

    /// Sets the graceful shutdown timeout.
    #[must_use]
    pub fn shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.shutdown_timeout = timeout;
        self
    }

    /// Sets the request header read timeout (`Duration::ZERO` disables it).
    #[must_use]
    pub fn header_read_timeout(mut self, timeout: Duration) -> Self {
        self.header_read_timeout = timeout;
        self
    }

    /// Enables or disables `TCP_NODELAY`.
    #[must_use]
    pub fn tcp_nodelay(mut self, enabled: bool) -> Self {
        self.tcp_nodelay = enabled;
        self
    }

    /// Enables or disables `ConnectInfo<SocketAddr>` for handlers.
    #[must_use]
    pub fn connect_info(mut self, enabled: bool) -> Self {
        self.connect_info = enabled;
        self
    }

    /// The `host:port` address, with brackets around IPv6 hosts.
    pub fn address(&self) -> String {
        if self.host.contains(':') && !self.host.starts_with('[') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    /// Builds a configuration from defaults overridden by `KERNIQ_*`
    /// environment variables. See [`ServerConfig::apply_env_vars`].
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::default().apply_env()
    }

    /// Overrides values from the process environment. See
    /// [`ServerConfig::apply_env_vars`].
    pub fn apply_env(self) -> Result<Self, ConfigError> {
        self.apply_env_vars(std::env::vars())
    }

    /// Overrides values from the given `(name, value)` pairs, using these
    /// variables (durations are in seconds, sizes in bytes):
    ///
    /// | Variable | Setting |
    /// |---|---|
    /// | `KERNIQ_HOST` | `host` |
    /// | `KERNIQ_PORT` | `port` |
    /// | `KERNIQ_WORKERS` | `workers` |
    /// | `KERNIQ_REQUEST_TIMEOUT` | `request_timeout` |
    /// | `KERNIQ_MAX_BODY_SIZE` | `max_body_size` |
    /// | `KERNIQ_MAX_CONNECTIONS` | `max_connections` |
    /// | `KERNIQ_MAX_CONCURRENT_REQUESTS` | `max_concurrent_requests` |
    /// | `KERNIQ_SHUTDOWN_TIMEOUT` | `shutdown_timeout` |
    /// | `KERNIQ_HEADER_READ_TIMEOUT` | `header_read_timeout` |
    /// | `KERNIQ_TCP_NODELAY` | `tcp_nodelay` |
    /// | `KERNIQ_CONNECT_INFO` | `connect_info` |
    ///
    /// Unrelated variables are ignored. The result is validated.
    pub fn apply_env_vars<I, K, V>(mut self, vars: I) -> Result<Self, ConfigError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        for (key, value) in vars {
            let (key, value) = (key.as_ref(), value.as_ref().trim());
            let Some(name) = key.strip_prefix("KERNIQ_") else {
                continue;
            };
            match name {
                "HOST" => self.host = value.to_owned(),
                "PORT" => self.port = parse(key, value)?,
                "WORKERS" => self.workers = Some(parse(key, value)?),
                "REQUEST_TIMEOUT" => self.request_timeout = Some(parse_secs(key, value)?),
                "MAX_BODY_SIZE" => self.max_body_size = parse(key, value)?,
                "MAX_CONNECTIONS" => self.max_connections = Some(parse(key, value)?),
                "MAX_CONCURRENT_REQUESTS" => {
                    self.max_concurrent_requests = Some(parse(key, value)?)
                }
                "SHUTDOWN_TIMEOUT" => self.shutdown_timeout = parse_secs(key, value)?,
                "HEADER_READ_TIMEOUT" => self.header_read_timeout = parse_secs(key, value)?,
                "TCP_NODELAY" => self.tcp_nodelay = parse_bool(key, value)?,
                "CONNECT_INFO" => self.connect_info = parse_bool(key, value)?,
                _ => {}
            }
        }
        self.validate()?;
        Ok(self)
    }

    /// Loads a configuration from a TOML file. Missing keys use defaults;
    /// unknown keys are rejected. Durations are given in seconds.
    ///
    /// ```toml
    /// host = "0.0.0.0"
    /// port = 8080
    /// request_timeout = 30
    /// max_concurrent_requests = 1024
    /// ```
    #[cfg(feature = "config-file")]
    pub fn from_file(path: impl AsRef<std::path::Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let display = path.display().to_string();
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: display.clone(),
            source,
        })?;
        Self::from_toml_str(&text).map_err(|err| match err {
            ConfigError::Parse { message, .. } => ConfigError::Parse {
                path: display,
                message,
            },
            other => other,
        })
    }

    /// Parses a configuration from a TOML string. See [`ServerConfig::from_file`].
    #[cfg(feature = "config-file")]
    pub fn from_toml_str(text: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(text).map_err(|err| ConfigError::Parse {
            path: "<string>".to_owned(),
            message: err.message().to_owned(),
        })?;
        config.validate()?;
        Ok(config)
    }

    /// Checks that values are within range.
    pub fn validate(&self) -> Result<(), ConfigError> {
        fn non_zero(key: &str, value: Option<usize>) -> Result<(), ConfigError> {
            match value {
                Some(0) => Err(ConfigError::InvalidValue {
                    key: key.to_owned(),
                    value: "0".to_owned(),
                    reason: "must be greater than zero".to_owned(),
                }),
                _ => Ok(()),
            }
        }
        if self.host.trim().is_empty() {
            return Err(ConfigError::InvalidValue {
                key: "host".to_owned(),
                value: self.host.clone(),
                reason: "must not be empty".to_owned(),
            });
        }
        non_zero("workers", self.workers)?;
        non_zero("max_connections", self.max_connections)?;
        non_zero("max_concurrent_requests", self.max_concurrent_requests)?;
        if self.request_timeout == Some(Duration::ZERO) {
            return Err(ConfigError::InvalidValue {
                key: "request_timeout".to_owned(),
                value: "0".to_owned(),
                reason: "must be greater than zero".to_owned(),
            });
        }
        Ok(())
    }
}

fn parse<T>(key: &str, value: &str) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    value
        .parse()
        .map_err(|err: T::Err| ConfigError::InvalidValue {
            key: key.to_owned(),
            value: value.to_owned(),
            reason: err.to_string(),
        })
}

fn parse_secs(key: &str, value: &str) -> Result<Duration, ConfigError> {
    let secs: f64 = parse(key, value)?;
    Duration::try_from_secs_f64(secs).map_err(|err| ConfigError::InvalidValue {
        key: key.to_owned(),
        value: value.to_owned(),
        reason: err.to_string(),
    })
}

fn parse_bool(key: &str, value: &str) -> Result<bool, ConfigError> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(ConfigError::InvalidValue {
            key: key.to_owned(),
            value: value.to_owned(),
            reason: "expected a boolean (true/false/1/0)".to_owned(),
        }),
    }
}

/// Serde helpers representing durations as (fractional) seconds.
mod secs {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_f64(d.as_secs_f64())
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let secs = f64::deserialize(d)?;
        Duration::try_from_secs_f64(secs).map_err(serde::de::Error::custom)
    }

    pub(super) mod option {
        use super::*;

        pub(crate) fn serialize<S: Serializer>(
            d: &Option<Duration>,
            s: S,
        ) -> Result<S::Ok, S::Error> {
            match d {
                Some(d) => s.serialize_some(&d.as_secs_f64()),
                None => s.serialize_none(),
            }
        }

        pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
            d: D,
        ) -> Result<Option<Duration>, D::Error> {
            Option::<f64>::deserialize(d)?
                .map(|secs| Duration::try_from_secs_f64(secs).map_err(serde::de::Error::custom))
                .transpose()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_sets_fields() {
        let c = ServerConfig::new()
            .host("0.0.0.0")
            .port(8080)
            .workers(4)
            .request_timeout(Duration::from_secs(30))
            .max_body_size(1024)
            .max_connections(10)
            .max_concurrent_requests(5)
            .shutdown_timeout(Duration::from_secs(5));
        assert_eq!(c.address(), "0.0.0.0:8080");
        assert_eq!(c.workers, Some(4));
        assert_eq!(c.request_timeout, Some(Duration::from_secs(30)));
        assert_eq!(c.max_body_size, 1024);
        assert_eq!(c.max_connections, Some(10));
        assert_eq!(c.max_concurrent_requests, Some(5));
        assert_eq!(c.shutdown_timeout, Duration::from_secs(5));
        c.validate().unwrap();
    }

    #[test]
    fn ipv6_address_is_bracketed() {
        assert_eq!(
            ServerConfig::new().host("::1").port(80).address(),
            "[::1]:80"
        );
    }

    #[test]
    fn env_overrides() {
        let c = ServerConfig::default()
            .apply_env_vars([
                ("KERNIQ_HOST", "0.0.0.0"),
                ("KERNIQ_PORT", "9000"),
                ("KERNIQ_REQUEST_TIMEOUT", "2.5"),
                ("KERNIQ_TCP_NODELAY", "false"),
                ("KERNIQ_MAX_CONCURRENT_REQUESTS", "64"),
                ("PATH", "/usr/bin"),
                ("KERNIQ_UNKNOWN", "ignored"),
            ])
            .unwrap();
        assert_eq!(c.address(), "0.0.0.0:9000");
        assert_eq!(c.request_timeout, Some(Duration::from_millis(2500)));
        assert!(!c.tcp_nodelay);
        assert_eq!(c.max_concurrent_requests, Some(64));
    }

    #[test]
    fn env_rejects_bad_values() {
        let err = ServerConfig::default()
            .apply_env_vars([("KERNIQ_PORT", "seventy")])
            .unwrap_err();
        assert!(err.to_string().contains("KERNIQ_PORT"));
        let err = ServerConfig::default()
            .apply_env_vars([("KERNIQ_WORKERS", "0")])
            .unwrap_err();
        assert!(err.to_string().contains("workers"));
    }

    #[cfg(feature = "config-file")]
    #[test]
    fn toml_parsing() {
        let c = ServerConfig::from_toml_str(
            "host = \"0.0.0.0\"\nport = 8080\nrequest_timeout = 30\nmax_connections = 100\n",
        )
        .unwrap();
        assert_eq!(c.port, 8080);
        assert_eq!(c.request_timeout, Some(Duration::from_secs(30)));
        assert_eq!(c.max_connections, Some(100));
        assert_eq!(c.shutdown_timeout, Duration::from_secs(30));

        let err = ServerConfig::from_toml_str("prot = 1").unwrap_err();
        assert!(matches!(err, ConfigError::Parse { .. }));
    }
}
