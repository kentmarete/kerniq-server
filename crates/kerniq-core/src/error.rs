//! The Kerniq error model.
//!
//! Every error produced by the framework — malformed input, missing routes,
//! timeouts, authentication failures — and every error returned by an
//! application handler is rendered with the same JSON envelope:
//!
//! ```json
//! {
//!   "error": {
//!     "code": "USER_NOT_FOUND",
//!     "message": "The requested user does not exist"
//!   }
//! }
//! ```
//!
//! An optional `details` field carries structured data such as field-level
//! validation failures.
//!
//! # Public messages versus diagnostic sources
//!
//! An [`Error`] has two kinds of information:
//!
//! * a **public** status, code, message and optional details, which are sent
//!   to the client, and
//! * an optional **source** error, which is *never* sent to the client. It is
//!   recorded through `tracing` when the response is produced (at `ERROR`
//!   level for 5xx responses, `DEBUG` otherwise).
//!
//! [`Error::internal`] uses a generic public message, so database errors,
//! file paths and similar implementation details cannot leak to clients.
//!
//! # Domain errors
//!
//! Application error types convert into [`Error`] with an ordinary `From`
//! implementation, which makes `?` work in handlers:
//!
//! ```
//! use kerniq_core::Error;
//!
//! #[derive(Debug, thiserror::Error)]
//! enum AccountError {
//!     #[error("account {0} not found")]
//!     NotFound(u64),
//!     #[error("insufficient funds")]
//!     InsufficientFunds,
//! }
//!
//! impl From<AccountError> for Error {
//!     fn from(err: AccountError) -> Self {
//!         match err {
//!             AccountError::NotFound(_) => Error::not_found("The requested account does not exist")
//!                 .with_code("ACCOUNT_NOT_FOUND"),
//!             AccountError::InsufficientFunds => {
//!                 Error::unprocessable("The account balance is too low").with_code("INSUFFICIENT_FUNDS")
//!             }
//!         }
//!     }
//! }
//! ```
//!
//! Kerniq deliberately does **not** provide a blanket `From<E: std::error::Error>`
//! conversion: it would make the implementation above impossible (it would
//! overlap), and it would silently turn every domain error into a 500.
//! Use [`ResultExt::or_internal`] when an unexpected failure should become a
//! 500 response.

use std::borrow::Cow;
use std::fmt;

use axum_core::response::{IntoResponse, Response};
use http::StatusCode;
use http::header::{self, HeaderName, HeaderValue};
use serde::Serialize;

/// A type-erased, thread-safe error used as an [`Error`] source.
pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// A `Result` whose error type defaults to the Kerniq [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// The public message used for errors whose details must not reach clients.
pub const INTERNAL_ERROR_MESSAGE: &str = "An internal server error occurred";

/// The framework error type.
///
/// `Error` is a single pointer wide, so `Result<T, Error>` stays small on the
/// success path. It implements [`IntoResponse`], so handlers can return
/// `Result<T>` directly.
///
/// See the [module documentation](self) for the response format and the
/// exposure policy.
pub struct Error {
    inner: Box<Inner>,
}

struct Inner {
    status: StatusCode,
    code: Cow<'static, str>,
    message: Cow<'static, str>,
    details: Option<serde_json::Value>,
    headers: Vec<(HeaderName, HeaderValue)>,
    source: Option<BoxError>,
}

macro_rules! constructors {
    ($( $(#[$doc:meta])* $name:ident => $status:ident, $code:literal; )*) => {
        $(
            $(#[$doc])*
            pub fn $name(message: impl Into<Cow<'static, str>>) -> Self {
                Self::new(StatusCode::$status, $code, message)
            }
        )*
    };
}

impl Error {
    /// Creates an error with an explicit status, machine-readable code and
    /// public message.
    pub fn new(
        status: StatusCode,
        code: impl Into<Cow<'static, str>>,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self {
            inner: Box::new(Inner {
                status,
                code: code.into(),
                message: message.into(),
                details: None,
                headers: Vec::new(),
                source: None,
            }),
        }
    }

    constructors! {
        /// `400 Bad Request` with code `BAD_REQUEST`.
        bad_request => BAD_REQUEST, "BAD_REQUEST";
        /// `401 Unauthorized` with code `UNAUTHORIZED`.
        unauthorized => UNAUTHORIZED, "UNAUTHORIZED";
        /// `403 Forbidden` with code `FORBIDDEN`.
        forbidden => FORBIDDEN, "FORBIDDEN";
        /// `404 Not Found` with code `NOT_FOUND`.
        not_found => NOT_FOUND, "NOT_FOUND";
        /// `405 Method Not Allowed` with code `METHOD_NOT_ALLOWED`.
        method_not_allowed => METHOD_NOT_ALLOWED, "METHOD_NOT_ALLOWED";
        /// `408 Request Timeout` with code `REQUEST_TIMEOUT`.
        timeout => REQUEST_TIMEOUT, "REQUEST_TIMEOUT";
        /// `409 Conflict` with code `CONFLICT`.
        conflict => CONFLICT, "CONFLICT";
        /// `413 Payload Too Large` with code `PAYLOAD_TOO_LARGE`.
        payload_too_large => PAYLOAD_TOO_LARGE, "PAYLOAD_TOO_LARGE";
        /// `415 Unsupported Media Type` with code `UNSUPPORTED_MEDIA_TYPE`.
        unsupported_media_type => UNSUPPORTED_MEDIA_TYPE, "UNSUPPORTED_MEDIA_TYPE";
        /// `422 Unprocessable Entity` with code `UNPROCESSABLE_ENTITY`.
        unprocessable => UNPROCESSABLE_ENTITY, "UNPROCESSABLE_ENTITY";
        /// `429 Too Many Requests` with code `TOO_MANY_REQUESTS`.
        too_many_requests => TOO_MANY_REQUESTS, "TOO_MANY_REQUESTS";
        /// `503 Service Unavailable` with code `SERVICE_UNAVAILABLE`.
        service_unavailable => SERVICE_UNAVAILABLE, "SERVICE_UNAVAILABLE";
    }

    /// `422 Unprocessable Entity` with code `VALIDATION_FAILED` and the given
    /// structured details (typically a map of field names to messages).
    pub fn validation(details: impl Serialize) -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "VALIDATION_FAILED",
            "The request failed validation",
        )
        .with_details(details)
    }

    /// `500 Internal Server Error` with a generic public message.
    ///
    /// The `source` is logged when the response is produced but is never sent
    /// to the client.
    pub fn internal(source: impl Into<BoxError>) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL_ERROR",
            INTERNAL_ERROR_MESSAGE,
        )
        .with_source(source)
    }

    /// Replaces the machine-readable error code.
    #[must_use]
    pub fn with_code(mut self, code: impl Into<Cow<'static, str>>) -> Self {
        self.inner.code = code.into();
        self
    }

    /// Replaces the public message.
    #[must_use]
    pub fn with_message(mut self, message: impl Into<Cow<'static, str>>) -> Self {
        self.inner.message = message.into();
        self
    }

    /// Attaches structured, client-visible details.
    ///
    /// If `details` fails to serialize, the error is kept and the details are
    /// omitted (the failure is logged).
    #[must_use]
    pub fn with_details(mut self, details: impl Serialize) -> Self {
        match serde_json::to_value(details) {
            Ok(value) => self.inner.details = Some(value),
            Err(err) => tracing::warn!(error = %err, "failed to serialize error details"),
        }
        self
    }

    /// Attaches a diagnostic source error. It is logged, never sent to clients.
    #[must_use]
    pub fn with_source(mut self, source: impl Into<BoxError>) -> Self {
        self.inner.source = Some(source.into());
        self
    }

    /// Adds a response header (for example `Retry-After` or `WWW-Authenticate`).
    #[must_use]
    pub fn with_header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.inner.headers.push((name, value));
        self
    }

    /// The HTTP status code.
    pub fn status(&self) -> StatusCode {
        self.inner.status
    }

    /// The machine-readable error code.
    pub fn code(&self) -> &str {
        &self.inner.code
    }

    /// The public, client-visible message.
    pub fn message(&self) -> &str {
        &self.inner.message
    }

    /// Client-visible structured details, if any.
    pub fn details(&self) -> Option<&serde_json::Value> {
        self.inner.details.as_ref()
    }

    /// Extra response headers attached to this error.
    pub fn headers(&self) -> &[(HeaderName, HeaderValue)] {
        &self.inner.headers
    }

    /// The diagnostic source, if any.
    pub fn source_ref(&self) -> Option<&(dyn std::error::Error + Send + Sync + 'static)> {
        self.inner.source.as_deref()
    }

    /// Serializes the client-visible JSON body.
    pub fn to_json_body(&self) -> Vec<u8> {
        #[derive(Serialize)]
        struct Envelope<'a> {
            error: Body<'a>,
        }
        #[derive(Serialize)]
        struct Body<'a> {
            code: &'a str,
            message: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            details: Option<&'a serde_json::Value>,
        }
        let envelope = Envelope {
            error: Body {
                code: &self.inner.code,
                message: &self.inner.message,
                details: self.inner.details.as_ref(),
            },
        };
        // Serializing strings and an existing `Value` cannot fail.
        serde_json::to_vec(&envelope).unwrap_or_default()
    }

    fn log(&self) {
        let status = self.inner.status.as_u16();
        let code: &str = &self.inner.code;
        match (&self.inner.source, self.inner.status.is_server_error()) {
            (Some(source), true) => {
                tracing::error!(status, code, error = %ErrorChain(source.as_ref()), "request failed")
            }
            (None, true) => {
                tracing::error!(status, code, message = %self.inner.message, "request failed")
            }
            (Some(source), false) => {
                tracing::debug!(status, code, error = %ErrorChain(source.as_ref()), "request rejected")
            }
            (None, false) => tracing::debug!(status, code, "request rejected"),
        }
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        self.log();
        let body = self.to_json_body();
        let Inner {
            status, headers, ..
        } = *self.inner;
        let mut response = (status, body).into_response();
        let map = response.headers_mut();
        map.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        for (name, value) in headers {
            map.append(name, value);
        }
        response
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}: {}",
            self.inner.status.as_u16(),
            self.inner.code,
            self.inner.message
        )?;
        if let Some(source) = &self.inner.source {
            write!(f, ": {}", ErrorChain(source.as_ref()))?;
        }
        Ok(())
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `Debug` is what `fn main() -> Result<()>` prints, so keep it readable.
        if f.alternate() {
            f.debug_struct("Error")
                .field("status", &self.inner.status)
                .field("code", &self.inner.code)
                .field("message", &self.inner.message)
                .field("details", &self.inner.details)
                .field("source", &self.inner.source)
                .finish()
        } else {
            fmt::Display::fmt(self, f)
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.inner
            .source
            .as_ref()
            .map(|s| s.as_ref() as &(dyn std::error::Error + 'static))
    }
}

impl From<StatusCode> for Error {
    /// Creates an error from a bare status code, using the canonical reason
    /// phrase as the message and an upper-snake-case code.
    fn from(status: StatusCode) -> Self {
        let reason = status.canonical_reason().unwrap_or("Error");
        let code = reason.to_ascii_uppercase().replace([' ', '-'], "_");
        Self::new(status, code, reason)
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Self::internal(err)
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Self::internal(err)
    }
}

impl From<http::Error> for Error {
    fn from(err: http::Error) -> Self {
        Self::internal(err)
    }
}

impl From<crate::ServerError> for Error {
    fn from(err: crate::ServerError) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "SERVER_ERROR",
            INTERNAL_ERROR_MESSAGE,
        )
        .with_source(err)
    }
}

impl From<crate::ConfigError> for Error {
    fn from(err: crate::ConfigError) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "CONFIG_ERROR",
            INTERNAL_ERROR_MESSAGE,
        )
        .with_source(err)
    }
}

/// Formats an error and its `source()` chain on one line.
struct ErrorChain<'a>(&'a (dyn std::error::Error + 'static));

impl fmt::Display for ErrorChain<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)?;
        let mut current = self.0.source();
        while let Some(source) = current {
            write!(f, ": {source}")?;
            current = source.source();
        }
        Ok(())
    }
}

/// Conversions from arbitrary `Result`s into Kerniq errors.
pub trait ResultExt<T> {
    /// Maps any error into a `500 Internal Server Error`, keeping it as the
    /// (logged, never exposed) source.
    fn or_internal(self) -> Result<T>;

    /// Replaces any error with `error`, keeping the original as its source.
    fn or_error(self, error: Error) -> Result<T>;
}

impl<T, E> ResultExt<T> for std::result::Result<T, E>
where
    E: Into<BoxError>,
{
    fn or_internal(self) -> Result<T> {
        self.map_err(Error::internal)
    }

    fn or_error(self, error: Error) -> Result<T> {
        self.map_err(|source| error.with_source(source))
    }
}

/// Conversions from `Option`s into Kerniq errors.
pub trait OptionExt<T> {
    /// Maps `None` to a `404 Not Found` with the given public message.
    fn or_not_found(self, message: impl Into<Cow<'static, str>>) -> Result<T>;

    /// Maps `None` to the given error.
    fn or_error(self, error: Error) -> Result<T>;
}

impl<T> OptionExt<T> for Option<T> {
    fn or_not_found(self, message: impl Into<Cow<'static, str>>) -> Result<T> {
        self.ok_or_else(|| Error::not_found(message))
    }

    fn or_error(self, error: Error) -> Result<T> {
        self.ok_or(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    async fn body_json(response: Response) -> serde_json::Value {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn error_is_pointer_sized() {
        assert_eq!(std::mem::size_of::<Error>(), std::mem::size_of::<usize>());
    }

    #[tokio::test]
    async fn renders_standard_envelope() {
        let err = Error::not_found("The requested user does not exist").with_code("USER_NOT_FOUND");
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        let body = body_json(response).await;
        assert_eq!(
            body,
            serde_json::json!({"error": {"code": "USER_NOT_FOUND", "message": "The requested user does not exist"}})
        );
    }

    #[tokio::test]
    async fn internal_errors_never_expose_their_source() {
        let source = std::io::Error::other("connection to db.internal:5432 refused");
        let response = Error::internal(source).into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let text = body_json(response).await.to_string();
        assert!(!text.contains("db.internal"));
        assert!(text.contains(INTERNAL_ERROR_MESSAGE));
    }

    #[tokio::test]
    async fn validation_details_and_headers() {
        let response = Error::validation(serde_json::json!({"email": ["invalid"]}))
            .with_header(header::RETRY_AFTER, HeaderValue::from_static("1"))
            .into_response();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(response.headers()[header::RETRY_AFTER], "1");
        let body = body_json(response).await;
        assert_eq!(body["error"]["code"], "VALIDATION_FAILED");
        assert_eq!(body["error"]["details"]["email"][0], "invalid");
    }

    #[test]
    fn from_status_code() {
        let err = Error::from(StatusCode::IM_A_TEAPOT);
        assert_eq!(err.code(), "I'M_A_TEAPOT");
        assert_eq!(err.status(), StatusCode::IM_A_TEAPOT);
        let err = Error::from(StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(err.code(), "GATEWAY_TIMEOUT");
    }

    #[test]
    fn result_and_option_extensions() {
        let r: std::result::Result<(), std::io::Error> = Err(std::io::Error::other("boom"));
        let err = r.or_internal().unwrap_err();
        assert_eq!(err.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(err.to_string().contains("boom"));

        let err = None::<u8>.or_not_found("missing").unwrap_err();
        assert_eq!(err.status(), StatusCode::NOT_FOUND);
        assert_eq!(err.message(), "missing");
    }

    #[test]
    fn domain_error_conversion_with_question_mark() {
        #[derive(Debug)]
        struct Missing;
        impl From<Missing> for Error {
            fn from(_: Missing) -> Self {
                Error::not_found("gone").with_code("GONE")
            }
        }
        fn handler() -> Result<()> {
            Err(Missing)?;
            Ok(())
        }
        assert_eq!(handler().unwrap_err().code(), "GONE");
    }
}
