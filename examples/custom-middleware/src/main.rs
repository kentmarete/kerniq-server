//! Writing middleware with plain Tower traits, plus Kerniq's `Middleware`
//! trait for bundling layers.
//!
//! ```sh
//! cargo run -p example-custom-middleware
//! curl -i localhost:3000/            # x-response-time header
//! curl -i localhost:3000/admin        # 403 without the header
//! curl -i -H 'x-admin: yes' localhost:3000/admin
//! ```

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

use kerniq_server::axum;
use kerniq_server::http::{HeaderValue, Request, Response};
use kerniq_server::prelude::*;
use kerniq_server::tower::{Layer, Service};

// ---------------------------------------------------------------------------
// 1. A Tower Layer + Service: adds an `x-response-time` header.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct ResponseTimeLayer;

impl<S> Layer<S> for ResponseTimeLayer {
    type Service = ResponseTime<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ResponseTime { inner }
    }
}

#[derive(Clone)]
struct ResponseTime<S> {
    inner: S,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for ResponseTime<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<ReqBody>) -> Self::Future {
        let start = Instant::now();
        let future = self.inner.call(request);
        Box::pin(async move {
            let mut response = future.await?;
            let micros = start.elapsed().as_micros().to_string();
            if let Ok(value) = HeaderValue::from_str(&format!("{micros}us")) {
                response.headers_mut().insert("x-response-time", value);
            }
            Ok(response)
        })
    }
}

// ---------------------------------------------------------------------------
// 2. Axum's `from_fn`: middleware as an async function.
// ---------------------------------------------------------------------------

async fn require_admin(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> std::result::Result<axum::response::Response, Error> {
    if request.headers().get("x-admin").is_some_and(|v| v == "yes") {
        Ok(next.run(request).await)
    } else {
        Err(Error::forbidden("Administrators only"))
    }
}

// ---------------------------------------------------------------------------
// 3. A Kerniq `Middleware`: one configurable unit made of several layers.
// ---------------------------------------------------------------------------

/// Hardening for internal endpoints: security headers plus a strict rate
/// limit, configured as a single item.
struct InternalEndpoint {
    requests_per_minute: u32,
}

impl Middleware for InternalEndpoint {
    fn apply<S>(self, router: axum::Router<S>) -> axum::Router<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        let router = SecurityHeaders::new().apply(router);
        RateLimit::per_minute(self.requests_per_minute)
            .by_ip()
            .apply(router)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let admin = Router::new()
        .layer(axum::middleware::from_fn(require_admin))
        .middleware(InternalEndpoint {
            requests_per_minute: 30,
        })
        .get("/", || async { "admin area" });

    let app = App::new()
        .layer(ResponseTimeLayer)
        .get("/", || async { "public" })
        .mount("/admin", admin);

    println!("listening on http://127.0.0.1:3000");
    app.run("127.0.0.1:3000").await
}
