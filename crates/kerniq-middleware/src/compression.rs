//! Response compression.

use kerniq_routing::Middleware;
use tower_http::compression::CompressionLayer;

/// Compresses responses according to the request's `Accept-Encoding`
/// (gzip, deflate, brotli, zstd), using tower-http's `CompressionLayer`.
///
/// Small bodies (under 32 bytes), images, gRPC and server-sent events are
/// not compressed (tower-http's default predicate).
///
/// Requires the `compression` feature.
#[derive(Clone, Default)]
pub struct Compression {
    layer: CompressionLayer,
}

impl std::fmt::Debug for Compression {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Compression").finish_non_exhaustive()
    }
}

impl Compression {
    /// Enables every supported algorithm.
    pub fn new() -> Self {
        Self::default()
    }

    /// The underlying `CompressionLayer`.
    pub fn into_layer(self) -> CompressionLayer {
        self.layer
    }
}

impl From<CompressionLayer> for Compression {
    fn from(layer: CompressionLayer) -> Self {
        Self { layer }
    }
}

impl Middleware for Compression {
    fn apply<S>(self, router: axum::Router<S>) -> axum::Router<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        router.layer(self.layer)
    }
}
