//! Request extractors and response types for Kerniq Server.
//!
//! Kerniq handlers are ordinary Axum handlers: async functions whose
//! arguments implement `FromRequestParts`/`FromRequest` and whose return type
//! implements [`IntoResponse`]. This crate provides:
//!
//! * [`Path`], [`Query`], [`Json`] — Axum's extractors with Kerniq error
//!   responses (see [`extractors`] for the mapping),
//! * [`Header<T>`] — typed single-header extraction via [`FromHeader`],
//! * [`ValidatedJson`] / [`ValidatedQuery`] (feature `validation`),
//! * [`Created`] and [`NoContent`] responses,
//! * re-exports of the Axum types handlers commonly need ([`State`],
//!   [`Body`], [`Bytes`], [`Extension`], [`HeaderMap`], ...).

#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

pub mod extractors;
pub mod response;

pub use axum::body::{Body, Bytes};
pub use axum::extract::{ConnectInfo, Extension, MatchedPath, OriginalUri, Request, State};
pub use http::{HeaderMap, HeaderValue, Method, StatusCode, Uri};

#[cfg(feature = "json")]
pub use extractors::Json;
pub use extractors::{
    Authorization, BearerToken, ContentType, FromHeader, Header, Path, Query, UserAgent, XRequestId,
};
#[cfg(feature = "validation")]
pub use extractors::{ValidatedJson, ValidatedQuery, validation_error};
#[cfg(feature = "json")]
pub use response::Created;
pub use response::{Html, IntoResponse, NoContent, Redirect, Response};

/// WebSocket support (Axum's implementation), enabled by the `websocket`
/// feature.
///
/// ```
/// use kerniq_http::ws::{Message, WebSocket, WebSocketUpgrade};
/// use kerniq_http::Response;
///
/// async fn echo(ws: WebSocketUpgrade) -> Response {
///     ws.on_upgrade(|mut socket: WebSocket| async move {
///         while let Some(Ok(msg)) = socket.recv().await {
///             if matches!(msg, Message::Close(_)) || socket.send(msg).await.is_err() {
///                 break;
///             }
///         }
///     })
/// }
/// ```
#[cfg(feature = "websocket")]
pub mod ws {
    pub use axum::extract::ws::*;
}
