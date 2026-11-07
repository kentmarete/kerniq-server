//! OpenAPI documentation for Kerniq Server, powered by
//! [Utoipa](https://docs.rs/utoipa).
//!
//! Kerniq does not generate the OpenAPI document itself: Utoipa's derive
//! macros do that at compile time from your types and handler annotations.
//! This crate serves the document and an optional Swagger UI page:
//!
//! ```
//! use kerniq_openapi::OpenApiDocs;
//! use kerniq_routing::Router;
//! use utoipa::OpenApi;
//!
//! #[derive(OpenApi)]
//! #[openapi(info(title = "Customer API", version = "1.0.0"))]
//! struct ApiDoc;
//!
//! let docs: Router = OpenApiDocs::new(ApiDoc::openapi())
//!     .json_path("/openapi.json")   // default
//!     .swagger_ui("/docs")          // optional
//!     .into_router();
//! // app.merge(docs)
//! ```
//!
//! The document is serialized once, when the router is built; requests are
//! answered from the cached bytes.
//!
//! The Swagger UI page loads `swagger-ui-dist` from the unpkg CDN, which
//! keeps this crate free of the large embedded asset bundle. For air-gapped
//! deployments, mount `utoipa-swagger-ui` instead (it embeds the assets) via
//! `App::mount_axum`.

#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

use axum::response::{Html, IntoResponse};
use bytes::Bytes;
use http::HeaderValue;
use http::header::CONTENT_TYPE;
use kerniq_routing::Router;
use kerniq_routing::path::normalize;
pub use utoipa;
use utoipa::openapi::OpenApi;

/// The pinned `swagger-ui-dist` version loaded by the Swagger UI page.
pub const SWAGGER_UI_VERSION: &str = "5.17.14";

/// Serves an OpenAPI document and, optionally, Swagger UI.
#[derive(Clone)]
pub struct OpenApiDocs {
    document: OpenApi,
    json_path: String,
    swagger_ui: Option<String>,
}

impl std::fmt::Debug for OpenApiDocs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenApiDocs")
            .field("title", &self.document.info.title)
            .field("json_path", &self.json_path)
            .field("swagger_ui", &self.swagger_ui)
            .finish()
    }
}

impl OpenApiDocs {
    /// Serves `document` at `/openapi.json`, without Swagger UI.
    pub fn new(document: OpenApi) -> Self {
        Self {
            document,
            json_path: "/openapi.json".to_owned(),
            swagger_ui: None,
        }
    }

    /// Changes where the JSON document is served.
    ///
    /// # Panics
    ///
    /// Panics if `path` contains characters other than ASCII letters,
    /// digits, `/`, `-`, `_` and `.`.
    pub fn json_path(mut self, path: &str) -> Self {
        self.json_path = checked_path(path);
        self
    }

    /// Serves a Swagger UI page at `path`.
    ///
    /// # Panics
    ///
    /// Panics under the same conditions as [`json_path`](Self::json_path).
    pub fn swagger_ui(mut self, path: &str) -> Self {
        self.swagger_ui = Some(checked_path(path));
        self
    }

    /// The document being served.
    pub fn document(&self) -> &OpenApi {
        &self.document
    }

    /// Builds a router serving the document (and UI). Merge it into an app
    /// with `App::merge`, or mount it under a prefix with `App::mount` (the
    /// UI then references the JSON path relative to the mount point, so
    /// prefer `merge` with absolute paths).
    ///
    /// # Panics
    ///
    /// Panics if the document cannot be serialized to JSON (which would be
    /// a bug in the document's construction).
    pub fn into_router<S>(self) -> Router<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        let json = Bytes::from(
            self.document
                .to_json()
                .expect("OpenAPI document must serialize to JSON"),
        );
        let mut router = Router::new().get(&self.json_path, move || {
            let json = json.clone();
            async move {
                (
                    [(CONTENT_TYPE, HeaderValue::from_static("application/json"))],
                    json,
                )
                    .into_response()
            }
        });
        if let Some(ui_path) = self.swagger_ui {
            let page = Html(swagger_page(&self.json_path, &self.document.info.title));
            router = router.get(&ui_path, move || {
                let page = page.clone();
                async move { page }
            });
        }
        router
    }
}

fn checked_path(path: &str) -> String {
    let path = normalize(path).into_owned();
    assert!(
        path.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.')),
        "OpenAPI paths may only contain ASCII letters, digits, '/', '-', '_' and '.': {path:?}"
    );
    path
}

fn html_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn swagger_page(json_path: &str, title: &str) -> String {
    // `json_path` is restricted to URL-safe characters by `checked_path`, so
    // it can be embedded in the script directly.
    format!(
        r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} — API documentation</title>
<link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@{v}/swagger-ui.css">
</head>
<body>
<div id="swagger-ui"></div>
<script src="https://unpkg.com/swagger-ui-dist@{v}/swagger-ui-bundle.js" crossorigin></script>
<script>
window.onload = () => {{
  window.ui = SwaggerUIBundle({{ url: "{json_path}", dom_id: "#swagger-ui" }});
}};
</script>
</body>
</html>
"##,
        title = html_escape(title),
        v = SWAGGER_UI_VERSION,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    use utoipa::OpenApi as _;

    #[derive(utoipa::ToSchema, serde::Serialize)]
    #[allow(dead_code)]
    struct User {
        id: u64,
        name: String,
    }

    #[utoipa::path(get, path = "/users/{id}", responses((status = 200, body = User)))]
    #[allow(dead_code)]
    async fn get_user() {}

    #[derive(utoipa::OpenApi)]
    #[openapi(
        info(title = "Test <API>", version = "1.0.0"),
        paths(get_user),
        components(schemas(User))
    )]
    struct ApiDoc;

    async fn get(router: axum::Router, path: &str) -> (http::StatusCode, String, String) {
        let response = router
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .map(|v| v.to_str().unwrap().to_owned())
            .unwrap_or_default();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            content_type,
            String::from_utf8(body.to_vec()).unwrap(),
        )
    }

    #[tokio::test]
    async fn serves_document_and_ui() {
        let router: Router = OpenApiDocs::new(ApiDoc::openapi())
            .swagger_ui("/docs")
            .into_router();
        let router = router.into_axum();

        let (status, content_type, body) = get(router.clone(), "/openapi.json").await;
        assert_eq!(status, 200);
        assert_eq!(content_type, "application/json");
        let doc: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(doc["info"]["title"], "Test <API>");
        assert!(doc["paths"]["/users/{id}"]["get"].is_object());
        assert!(doc["components"]["schemas"]["User"].is_object());

        let (status, content_type, body) = get(router, "/docs").await;
        assert_eq!(status, 200);
        assert!(content_type.starts_with("text/html"));
        assert!(body.contains("url: \"/openapi.json\""));
        assert!(body.contains("Test &lt;API&gt;"));
    }

    #[test]
    #[should_panic(expected = "OpenAPI paths may only contain")]
    fn rejects_unsafe_paths() {
        let _ = OpenApiDocs::new(ApiDoc::openapi()).json_path("/x\"<script>");
    }
}
