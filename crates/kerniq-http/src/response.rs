//! Response types.
//!
//! Anything implementing [`IntoResponse`] can be returned from a handler:
//! `&'static str`, `String`, [`Json<T>`](crate::Json), `StatusCode`,
//! `(StatusCode, T)` tuples, [`Html`], [`Redirect`], `Result<T, E>` where both
//! sides implement `IntoResponse`, and Kerniq's [`Error`](kerniq_core::Error).
//!
//! This module adds two small conveniences for common REST responses.

pub use axum::response::{Html, IntoResponse, Redirect, Response};
pub use http::StatusCode;

/// `204 No Content` with an empty body.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoContent;

impl IntoResponse for NoContent {
    fn into_response(self) -> Response {
        StatusCode::NO_CONTENT.into_response()
    }
}

/// `201 Created` with a JSON body.
///
/// ```
/// use kerniq_http::Created;
/// # #[derive(serde::Serialize)] struct User { id: u64 }
///
/// async fn create_user() -> Created<User> {
///     Created(User { id: 7 })
/// }
/// ```
#[cfg(feature = "json")]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Created<T>(pub T);

#[cfg(feature = "json")]
impl<T: serde::Serialize> IntoResponse for Created<T> {
    fn into_response(self) -> Response {
        (StatusCode::CREATED, crate::Json(self.0)).into_response()
    }
}
