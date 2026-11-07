//! Everything most applications need: `use kerniq_server::prelude::*;`

pub use crate::App;
#[cfg(feature = "macros")]
pub use crate::authorize;
pub use kerniq_core::{Error, OptionExt, Result, ResultExt, ServerConfig};

pub use kerniq_routing::{Middleware, Router};

pub use kerniq_http::{
    Body, Bytes, Extension, FromHeader, Header, HeaderMap, Html, IntoResponse, Method, NoContent,
    Path, Query, Redirect, Response, State, StatusCode,
};
#[cfg(feature = "json")]
pub use kerniq_http::{Created, Json};
#[cfg(feature = "validation")]
pub use kerniq_http::{ValidatedJson, ValidatedQuery};

#[cfg(feature = "compression")]
pub use kerniq_middleware::Compression;
#[cfg(feature = "cors")]
pub use kerniq_middleware::Cors;
#[cfg(feature = "tracing")]
pub use kerniq_middleware::Logger;
pub use kerniq_middleware::{
    BodyLimit, ConcurrencyLimit, Metrics, RateLimit, RequestId, SecurityHeaders, Timeout,
};

pub use kerniq_runtime::Shutdown;
