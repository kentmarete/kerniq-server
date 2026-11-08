//! Bounded attribute values.
//!
//! Every distinct combination of metric attribute values is a separate time
//! series held in memory by the SDK and stored by the backend. Attribute
//! values therefore have to come from small, known sets.
//!
//! **Never** use these as metric attributes: user, customer or account
//! identifiers, transaction or request ids, raw URLs or paths, e-mail
//! addresses, exception messages, prompts or generated text, or free-form
//! model names. Put them on spans or log events instead, where each value is
//! stored once per event rather than as a new series.
//!
//! The helpers here return `&'static str`, so building attributes from them
//! does not allocate.

use http::{Method, Version};

/// The OpenTelemetry value for non-standard HTTP methods.
pub const OTHER: &str = "_OTHER";

/// Normalizes an HTTP method for `http.request.method`. Standard methods
/// map to themselves; extension methods map to `_OTHER`, as the semantic
/// conventions require, so a client cannot create new series by sending
/// arbitrary method names.
pub fn http_method(method: &Method) -> &'static str {
    match *method {
        Method::GET => "GET",
        Method::POST => "POST",
        Method::PUT => "PUT",
        Method::DELETE => "DELETE",
        Method::PATCH => "PATCH",
        Method::HEAD => "HEAD",
        Method::OPTIONS => "OPTIONS",
        Method::TRACE => "TRACE",
        Method::CONNECT => "CONNECT",
        _ => OTHER,
    }
}

/// `network.protocol.version` for an HTTP version.
pub fn protocol_version(version: Version) -> Option<&'static str> {
    match version {
        Version::HTTP_09 => Some("0.9"),
        Version::HTTP_10 => Some("1.0"),
        Version::HTTP_11 => Some("1.1"),
        Version::HTTP_2 => Some("2"),
        Version::HTTP_3 => Some("3"),
        _ => None,
    }
}

/// `error.type` for a status code: the code itself for 5xx responses
/// (as the HTTP conventions specify), `None` otherwise.
pub fn status_error_type(status: u16) -> Option<&'static str> {
    Some(match status {
        500 => "500",
        501 => "501",
        502 => "502",
        503 => "503",
        504 => "504",
        505 => "505",
        506 => "506",
        507 => "507",
        508 => "508",
        510 => "510",
        511 => "511",
        _ if (500..600).contains(&status) => "5xx",
        _ => return None,
    })
}

/// Maps arbitrary input onto a fixed allow-list, so it can be used as a
/// metric attribute without unbounded cardinality.
///
/// ```
/// use kerniq_telemetry::BoundedLabel;
///
/// static MODELS: BoundedLabel = BoundedLabel::new(&["small", "large"], "other");
///
/// assert_eq!(MODELS.get("small"), "small");
/// assert_eq!(MODELS.get("customer-42-finetune"), "other");
/// ```
#[derive(Debug, Clone, Copy)]
pub struct BoundedLabel {
    allowed: &'static [&'static str],
    fallback: &'static str,
}

impl BoundedLabel {
    /// An allow-list with a fallback value for everything else.
    pub const fn new(allowed: &'static [&'static str], fallback: &'static str) -> Self {
        Self { allowed, fallback }
    }

    /// The allow-listed value equal to `value`, or the fallback.
    pub fn get(&self, value: &str) -> &'static str {
        self.allowed
            .iter()
            .copied()
            .find(|allowed| *allowed == value)
            .unwrap_or(self.fallback)
    }

    /// Every value this label can produce (allow-list plus fallback), useful
    /// for documenting the cardinality of a metric.
    pub fn values(&self) -> impl Iterator<Item = &'static str> {
        self.allowed
            .iter()
            .copied()
            .chain(std::iter::once(self.fallback))
    }
}

/// Attribute keys that must never be used on metrics. Used by tests and
/// debug assertions; not a security boundary.
pub const FORBIDDEN_METRIC_KEYS: &[&str] = &[
    "user.id",
    "user.email",
    "enduser.id",
    "customer.id",
    "account.id",
    "account.number",
    "transaction.id",
    "request.id",
    "kerniq.request_id",
    "url.full",
    "url.path",
    "url.query",
    "exception.message",
    "gen_ai.prompt",
    "gen_ai.completion",
];

/// Whether `key` is on the [`FORBIDDEN_METRIC_KEYS`] list.
pub fn is_forbidden_metric_key(key: &str) -> bool {
    FORBIDDEN_METRIC_KEYS.contains(&key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn methods_are_bounded() {
        assert_eq!(http_method(&Method::GET), "GET");
        let custom = Method::from_bytes(b"PURGE").unwrap();
        assert_eq!(http_method(&custom), OTHER);
    }

    #[test]
    fn error_types() {
        assert_eq!(status_error_type(200), None);
        assert_eq!(status_error_type(404), None);
        assert_eq!(status_error_type(503), Some("503"));
        assert_eq!(status_error_type(599), Some("5xx"));
    }

    #[test]
    fn protocol_versions() {
        assert_eq!(protocol_version(Version::HTTP_11), Some("1.1"));
        assert_eq!(protocol_version(Version::HTTP_2), Some("2"));
    }

    #[test]
    fn bounded_label() {
        static L: BoundedLabel = BoundedLabel::new(&["a", "b"], "other");
        assert_eq!(L.get("a"), "a");
        assert_eq!(L.get("zzz"), "other");
        assert_eq!(L.values().collect::<Vec<_>>(), ["a", "b", "other"]);
    }

    #[test]
    fn forbidden_keys() {
        assert!(is_forbidden_metric_key("user.id"));
        assert!(!is_forbidden_metric_key("http.route"));
    }
}
