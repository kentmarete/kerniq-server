//! Typed request extractors.
//!
//! Kerniq's [`Path`], [`Query`] and [`Json`] extractors delegate parsing to
//! Axum's extractors of the same name and differ in exactly one respect:
//! their rejections are Kerniq [`Error`]s, so malformed input produces the
//! same JSON error envelope as every other error in the application:
//!
//! | Problem | Status | Code |
//! |---|---|---|
//! | Path parameter does not parse (`/users/abc` for `Path<u64>`) | 400 | `INVALID_PATH_PARAMETER` |
//! | Query string does not deserialize | 400 | `INVALID_QUERY` |
//! | Body is not syntactically valid JSON | 400 | `MALFORMED_JSON` |
//! | JSON does not match the target type | 422 | `INVALID_JSON` |
//! | Missing `Content-Type: application/json` | 415 | `UNSUPPORTED_MEDIA_TYPE` |
//! | Body exceeds the configured limit | 413 | `PAYLOAD_TOO_LARGE` |
//! | Required header missing / malformed | 400 | `MISSING_HEADER` / `INVALID_HEADER` |
//!
//! Because Kerniq handlers *are* Axum handlers, any Axum extractor
//! (`axum::extract::Path`, `HeaderMap`, `axum_extra` types, ...) can be used
//! alongside these.

use axum::extract::{FromRequestParts, OptionalFromRequestParts};
use http::StatusCode;
use http::request::Parts;
use kerniq_core::Error;
use serde::de::DeserializeOwned;

/// Converts an Axum rejection (status + text) into a Kerniq error.
pub(crate) fn rejection(status: StatusCode, text: String, code_for_400: &'static str) -> Error {
    let code = match status {
        StatusCode::BAD_REQUEST => code_for_400,
        StatusCode::UNPROCESSABLE_ENTITY => "INVALID_JSON",
        StatusCode::UNSUPPORTED_MEDIA_TYPE => "UNSUPPORTED_MEDIA_TYPE",
        StatusCode::PAYLOAD_TOO_LARGE => "PAYLOAD_TOO_LARGE",
        status if status.is_server_error() => {
            // A server-side extraction failure is a programming error (for
            // example `Path<T>` on a route without parameters). Do not leak it.
            return Error::internal(text);
        }
        _ => "BAD_REQUEST",
    };
    Error::new(status, code, text)
}

/// Extracts typed path parameters.
///
/// Routes declare parameters as `{name}` (or `:name`, see the routing
/// documentation):
///
/// ```
/// use kerniq_http::Path;
///
/// // GET /users/{id}
/// async fn get_user(Path(id): Path<u64>) -> String {
///     format!("user {id}")
/// }
///
/// // GET /users/{user_id}/accounts/{account_id}
/// async fn get_account(Path((user, account)): Path<(u64, String)>) -> String {
///     format!("{user}/{account}")
/// }
/// ```
///
/// A struct deriving `Deserialize` can be used to extract parameters by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Path<T>(pub T);

impl<T, S> FromRequestParts<S> for Path<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match <axum::extract::Path<T> as FromRequestParts<S>>::from_request_parts(parts, state)
            .await
        {
            Ok(axum::extract::Path(value)) => Ok(Path(value)),
            Err(err) => Err(rejection(
                err.status(),
                err.body_text(),
                "INVALID_PATH_PARAMETER",
            )),
        }
    }
}

/// Extracts and deserializes the query string.
///
/// ```
/// use kerniq_http::Query;
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct Page {
///     page: Option<u32>,
///     limit: Option<u32>,
/// }
///
/// async fn list(Query(page): Query<Page>) -> String {
///     format!("page {} (limit {})", page.page.unwrap_or(1), page.limit.unwrap_or(20))
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Query<T>(pub T);

impl<T, S> FromRequestParts<S> for Query<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        axum::extract::Query::<T>::try_from_uri(&parts.uri)
            .map(|axum::extract::Query(value)| Query(value))
            .map_err(|err| rejection(err.status(), err.body_text(), "INVALID_QUERY"))
    }
}

/// A value that can be decoded from a single, well-known request header.
///
/// Implement this trait to make a header available through [`Header<T>`]:
///
/// ```
/// use kerniq_http::{FromHeader, Header};
/// use http::HeaderValue;
///
/// struct ApiKey(String);
///
/// impl FromHeader for ApiKey {
///     const NAME: &'static str = "x-api-key";
///
///     fn from_header_value(value: &HeaderValue) -> Option<Self> {
///         value.to_str().ok().map(|s| ApiKey(s.to_owned()))
///     }
/// }
///
/// async fn handler(Header(ApiKey(key)): Header<ApiKey>) -> String {
///     format!("key has {} chars", key.len())
/// }
/// ```
pub trait FromHeader: Sized {
    /// The lowercase header name.
    const NAME: &'static str;

    /// Decodes the header value, returning `None` if it is malformed.
    fn from_header_value(value: &http::HeaderValue) -> Option<Self>;
}

/// Extracts a typed header implementing [`FromHeader`].
///
/// A missing header is rejected with `400 MISSING_HEADER`; use
/// `Option<Header<T>>` for optional headers. A malformed value is always
/// rejected with `400 INVALID_HEADER`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header<T>(pub T);

fn decode_header<T: FromHeader>(parts: &Parts) -> Result<Option<T>, Error> {
    match parts.headers.get(T::NAME) {
        None => Ok(None),
        Some(value) => T::from_header_value(value).map(Some).ok_or_else(|| {
            Error::bad_request(format!("Header `{}` is malformed", T::NAME))
                .with_code("INVALID_HEADER")
        }),
    }
}

impl<T, S> FromRequestParts<S> for Header<T>
where
    T: FromHeader,
    S: Send + Sync,
{
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        decode_header::<T>(parts)?.map(Header).ok_or_else(|| {
            Error::bad_request(format!("Missing required header `{}`", T::NAME))
                .with_code("MISSING_HEADER")
        })
    }
}

impl<T, S> OptionalFromRequestParts<S> for Header<T>
where
    T: FromHeader,
    S: Send + Sync,
{
    type Rejection = Error;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> Result<Option<Self>, Self::Rejection> {
        decode_header::<T>(parts).map(|value| value.map(Header))
    }
}

macro_rules! string_header {
    ($(#[$doc:meta])* $name:ident, $header:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $name(pub String);

        impl FromHeader for $name {
            const NAME: &'static str = $header;

            fn from_header_value(value: &http::HeaderValue) -> Option<Self> {
                value.to_str().ok().map(|s| Self(s.to_owned()))
            }
        }
    };
}

string_header!(
    /// The `User-Agent` header.
    UserAgent,
    "user-agent"
);
string_header!(
    /// The `Content-Type` header.
    ContentType,
    "content-type"
);
string_header!(
    /// The raw `Authorization` header.
    Authorization,
    "authorization"
);
string_header!(
    /// The `X-Request-Id` header (set by the `RequestId` middleware when
    /// absent).
    XRequestId,
    "x-request-id"
);

/// The token from an `Authorization: Bearer <token>` header.
///
/// The scheme is matched case-insensitively. Any other scheme, or an empty
/// token, is treated as malformed (`400 INVALID_HEADER`).
#[derive(Clone, PartialEq, Eq)]
pub struct BearerToken(pub String);

impl std::fmt::Debug for BearerToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print credentials.
        f.write_str("BearerToken(<redacted>)")
    }
}

impl FromHeader for BearerToken {
    const NAME: &'static str = "authorization";

    fn from_header_value(value: &http::HeaderValue) -> Option<Self> {
        let value = value.to_str().ok()?;
        let (scheme, token) = value.split_once(' ')?;
        let token = token.trim();
        (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then(|| Self(token.to_owned()))
    }
}

#[cfg(feature = "json")]
pub use json::Json;

#[cfg(feature = "json")]
mod json {
    use axum::extract::{FromRequest, Request};
    use axum::response::{IntoResponse, Response};
    use bytes::{BufMut, BytesMut};
    use http::HeaderValue;
    use http::header::CONTENT_TYPE;
    use kerniq_core::Error;
    use serde::Serialize;
    use serde::de::DeserializeOwned;

    /// JSON request body extractor and JSON response.
    ///
    /// As an extractor it requires `Content-Type: application/json` and
    /// deserializes the body into `T`. As a response it serializes `T` with
    /// status `200 OK` (wrap it in `Created` or a `(StatusCode, Json<T>)`
    /// tuple for other statuses).
    ///
    /// ```
    /// use kerniq_http::Json;
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Deserialize)]
    /// struct CreateUser { name: String }
    ///
    /// #[derive(Serialize)]
    /// struct User { id: u64, name: String }
    ///
    /// async fn create(Json(input): Json<CreateUser>) -> Json<User> {
    ///     Json(User { id: 1, name: input.name })
    /// }
    /// ```
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct Json<T>(pub T);

    impl<T, S> FromRequest<S> for Json<T>
    where
        T: DeserializeOwned,
        S: Send + Sync,
    {
        type Rejection = Error;

        async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
            match axum::Json::<T>::from_request(req, state).await {
                Ok(axum::Json(value)) => Ok(Json(value)),
                Err(err) => Err(super::rejection(
                    err.status(),
                    err.body_text(),
                    "MALFORMED_JSON",
                )),
            }
        }
    }

    impl<T: Serialize> IntoResponse for Json<T> {
        fn into_response(self) -> Response {
            // Same strategy as Axum: serialize straight into a pre-sized
            // buffer and hand it over without copying.
            let mut buf = BytesMut::with_capacity(128).writer();
            match serde_json::to_writer(&mut buf, &self.0) {
                Ok(()) => (
                    [(CONTENT_TYPE, HeaderValue::from_static("application/json"))],
                    buf.into_inner().freeze(),
                )
                    .into_response(),
                Err(err) => Error::internal(err).into_response(),
            }
        }
    }

    impl<T> From<T> for Json<T> {
        fn from(value: T) -> Self {
            Json(value)
        }
    }
}

#[cfg(feature = "validation")]
pub use validation::{ValidatedJson, ValidatedQuery};

#[cfg(feature = "validation")]
mod validation {
    use std::collections::BTreeMap;

    use axum::extract::{FromRequest, FromRequestParts, Request};
    use http::request::Parts;
    use kerniq_core::Error;
    use serde::de::DeserializeOwned;
    use validator::{Validate, ValidationErrors, ValidationErrorsKind};

    /// Like [`Json`](super::Json), but also runs [`validator::Validate`].
    ///
    /// Validation failures produce `422 VALIDATION_FAILED` with a
    /// `details` object mapping each field path to its messages:
    ///
    /// ```json
    /// {"error": {"code": "VALIDATION_FAILED",
    ///            "message": "The request failed validation",
    ///            "details": {"email": ["must be a valid email address"]}}}
    /// ```
    ///
    /// Submitted values are never echoed back in the details.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct ValidatedJson<T>(pub T);

    /// Like [`Query`](super::Query), but also runs [`validator::Validate`].
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct ValidatedQuery<T>(pub T);

    impl<T, S> FromRequest<S> for ValidatedJson<T>
    where
        T: DeserializeOwned + Validate,
        S: Send + Sync,
    {
        type Rejection = Error;

        async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
            let super::Json(value) = super::Json::<T>::from_request(req, state).await?;
            value.validate().map_err(validation_error)?;
            Ok(Self(value))
        }
    }

    impl<T, S> FromRequestParts<S> for ValidatedQuery<T>
    where
        T: DeserializeOwned + Validate,
        S: Send + Sync,
    {
        type Rejection = Error;

        async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
            let super::Query(value) = super::Query::<T>::from_request_parts(parts, state).await?;
            value.validate().map_err(validation_error)?;
            Ok(Self(value))
        }
    }

    /// Converts `validator` errors into a `422 VALIDATION_FAILED` error whose
    /// details map field paths (`address.city`, `items[2].sku`) to messages.
    pub fn validation_error(errors: ValidationErrors) -> Error {
        let mut fields = BTreeMap::new();
        flatten(&errors, String::new(), &mut fields);
        Error::validation(fields)
    }

    fn flatten(errors: &ValidationErrors, prefix: String, out: &mut BTreeMap<String, Vec<String>>) {
        for (field, kind) in errors.errors() {
            let path = if prefix.is_empty() {
                field.to_string()
            } else {
                format!("{prefix}.{field}")
            };
            match kind {
                ValidationErrorsKind::Field(list) => {
                    let messages = out.entry(path).or_default();
                    for err in list {
                        messages.push(match &err.message {
                            Some(message) => message.to_string(),
                            None => format!("failed `{}` validation", err.code),
                        });
                    }
                }
                ValidationErrorsKind::Struct(inner) => flatten(inner, path, out),
                ValidationErrorsKind::List(items) => {
                    for (index, inner) in items {
                        flatten(inner, format!("{path}[{index}]"), out);
                    }
                }
            }
        }
    }
}

#[cfg(feature = "validation")]
pub use validation::validation_error;
