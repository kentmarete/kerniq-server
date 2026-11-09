use std::sync::Arc;

use kerniq_server::http::request::Parts;
use kerniq_server::middleware::auth::{Auth, Authenticator};
use kerniq_server::prelude::*;

pub const API_KEY_HEADER: &str = "x-api-key";

/// Identity attached to requests that presented a valid API key.
#[derive(Clone, Debug)]
pub struct ApiClient;

/// Authenticates requests by comparing the `x-api-key` header with the
/// configured key.
#[derive(Clone)]
pub struct ApiKeyAuth {
    key: Arc<str>,
}

impl ApiKeyAuth {
    pub fn layer(key: &str) -> Auth<Self> {
        Auth::new(Self { key: key.into() })
    }
}

impl Authenticator for ApiKeyAuth {
    type Identity = ApiClient;

    async fn authenticate(&self, parts: &Parts) -> Result<ApiClient> {
        let presented = parts
            .headers
            .get(API_KEY_HEADER)
            .ok_or_else(|| Error::unauthorized("Missing x-api-key header"))?;
        if constant_time_eq(presented.as_bytes(), self.key.as_bytes()) {
            Ok(ApiClient)
        } else {
            Err(Error::unauthorized("Invalid API key"))
        }
    }
}

/// Compares two byte strings without short-circuiting on the first mismatch.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
