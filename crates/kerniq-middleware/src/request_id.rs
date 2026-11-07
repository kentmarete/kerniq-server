//! Request IDs.

use http::HeaderName;
use kerniq_routing::Middleware;
use tower::ServiceBuilder;
use tower::layer::util::{Identity, Stack};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};

/// Assigns every request an ID and echoes it in the response.
///
/// * If the request already carries the header (for example from a load
///   balancer or an upstream service), that ID is kept.
/// * Otherwise a random UUID v4 is generated.
/// * The ID is set on the request (so handlers, `Logger` and downstream
///   calls can read it) and copied to the response.
///
/// Handlers read it with `Header<XRequestId>`. Declare `RequestId` *before*
/// `Logger` so log spans include the ID.
///
/// ```
/// use kerniq_middleware::RequestId;
///
/// let default = RequestId::new();                    // x-request-id
/// let custom = RequestId::header("x-correlation-id");
/// ```
#[derive(Debug, Clone)]
pub struct RequestId {
    header: HeaderName,
}

impl Default for RequestId {
    fn default() -> Self {
        Self::new()
    }
}

impl RequestId {
    /// Uses the `x-request-id` header.
    pub fn new() -> Self {
        Self {
            header: HeaderName::from_static("x-request-id"),
        }
    }

    /// Uses a custom header name.
    ///
    /// # Panics
    ///
    /// Panics if `name` is not a valid header name.
    pub fn header(name: &str) -> Self {
        Self {
            header: HeaderName::try_from(name).expect("invalid request id header name"),
        }
    }
}

/// The Tower layer stack produced by [`RequestId::into_layer`].
pub type RequestIdLayer =
    Stack<PropagateRequestIdLayer, Stack<SetRequestIdLayer<MakeRequestUuid>, Identity>>;

impl RequestId {
    /// Converts into a Tower layer (set + propagate).
    pub fn into_layer(self) -> RequestIdLayer {
        ServiceBuilder::new()
            .layer(SetRequestIdLayer::new(self.header.clone(), MakeRequestUuid))
            .layer(PropagateRequestIdLayer::new(self.header))
            .into_inner()
    }
}

impl Middleware for RequestId {
    fn apply<S>(self, router: axum::Router<S>) -> axum::Router<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        router.layer(self.into_layer())
    }
}
