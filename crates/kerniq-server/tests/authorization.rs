//! `#[authorize]`, `Authorized<T, P>`, `Policy` and `Authorities`.

mod common;

use axum::body::Body;
use common::*;
use http::{Request, StatusCode};
use kerniq_server::middleware::auth::{Auth, Authorities, Authorized, Identity, Policy};
use kerniq_server::prelude::*;

#[derive(Clone, Debug)]
struct User {
    name: String,
    roles: Vec<&'static str>,
    permissions: Vec<&'static str>,
}

impl Authorities for User {
    fn has_role(&self, role: &str) -> bool {
        self.roles.contains(&role)
    }
    fn has_permission(&self, permission: &str) -> bool {
        self.permissions.contains(&permission)
    }
}

fn user(name: &str, roles: &[&'static str], permissions: &[&'static str]) -> User {
    User {
        name: name.to_owned(),
        roles: roles.to_vec(),
        permissions: permissions.to_vec(),
    }
}

/// Tokens are user names: `guest` (no roles), `alice` (USER), `root`
/// (ADMIN), `auditor` (USER + reports:read).
fn auth() -> impl Middleware {
    Auth::bearer(|token: String| async move {
        match token.as_str() {
            "guest" => Ok(user("guest", &[], &[])),
            "alice" => Ok(user("alice", &["USER"], &[])),
            "root" => Ok(user("root", &["ADMIN"], &["users:delete"])),
            "auditor" => Ok(user("auditor", &["USER"], &["reports:read"])),
            _ => Err(Error::unauthorized("Invalid token")),
        }
    })
}

#[authorize(has_any_role("USER", "ADMIN"))]
async fn profile(Identity(user): Identity<User>) -> String {
    format!("profile of {}", user.name)
}

#[authorize(has_role("ADMIN") && has_permission("users:delete"), identity = User)]
async fn delete_user(Path(id): Path<u64>) -> String {
    format!("deleted {id}")
}

#[authorize(
    has_role("ADMIN") || (has_role("USER") && has_permission("reports:read")),
    identity = User,
    message = "Reports are restricted"
)]
async fn reports() -> &'static str {
    "reports"
}

fn is_named_alice(user: &User) -> bool {
    user.name == "alice"
}

#[authorize(authenticated() && !has_role("ADMIN") && check(is_named_alice), identity = User)]
async fn alice_only() -> &'static str {
    "hi alice"
}

#[authorize(has_role("ADMIN"), identity = User)]
async fn create(Json(body): Json<serde_json::Value>) -> String {
    body.to_string()
}

fn router() -> axum::Router {
    let api = Router::new()
        .middleware(auth())
        .get("/profile", profile)
        .delete("/users/{id}", delete_user)
        .get("/reports", reports)
        .get("/alice", alice_only)
        .post("/items", create);
    App::new()
        .get("/unprotected", profile)
        .mount("/api", api)
        .into_axum_router()
}

fn as_user(method: &str, uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

async fn status(router: &axum::Router, method: &str, uri: &str, token: &str) -> StatusCode {
    send(router, as_user(method, uri, token)).await.status
}

#[tokio::test]
async fn any_role() {
    let router = router();
    assert_eq!(
        get(&router, "/api/profile").await.status,
        StatusCode::UNAUTHORIZED
    );
    let res = send(&router, as_user("GET", "/api/profile", "guest")).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(res.error_code(), "FORBIDDEN");
    assert_eq!(res.json()["error"]["message"], "Insufficient permissions");

    let res = send(&router, as_user("GET", "/api/profile", "alice")).await;
    assert_eq!(res.body, "profile of alice");
    assert_eq!(
        status(&router, "GET", "/api/profile", "root").await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn role_and_permission_with_explicit_identity() {
    let router = router();
    assert_eq!(
        status(&router, "DELETE", "/api/users/7", "alice").await,
        StatusCode::FORBIDDEN
    );
    let res = send(&router, as_user("DELETE", "/api/users/7", "root")).await;
    assert_eq!(res.body, "deleted 7");
}

#[tokio::test]
async fn combinators_and_custom_message() {
    let router = router();
    let res = send(&router, as_user("GET", "/api/reports", "alice")).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(res.json()["error"]["message"], "Reports are restricted");
    for token in ["root", "auditor"] {
        assert_eq!(
            status(&router, "GET", "/api/reports", token).await,
            StatusCode::OK
        );
    }
}

#[tokio::test]
async fn negation_and_custom_check() {
    let router = router();
    assert_eq!(
        status(&router, "GET", "/api/alice", "alice").await,
        StatusCode::OK
    );
    for token in ["root", "auditor", "guest"] {
        assert_eq!(
            status(&router, "GET", "/api/alice", token).await,
            StatusCode::FORBIDDEN
        );
    }
}

#[tokio::test]
async fn authorization_runs_before_body_extraction() {
    let router = router();
    let malformed = |token: &str| {
        Request::post("/api/items")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from("{not json"))
            .unwrap()
    };
    assert_eq!(
        send(&router, malformed("alice")).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(&router, malformed("root")).await.status,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn fails_closed_without_auth_middleware() {
    assert_eq!(
        get(&router(), "/unprotected").await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn hand_written_policy_without_macro() {
    struct AdminOnly;
    impl Policy<User> for AdminOnly {
        fn check(user: &User) -> Result<()> {
            if user.has_role("ADMIN") {
                Ok(())
            } else {
                Err(Error::forbidden("Admins only"))
            }
        }
    }

    let router = App::new()
        .middleware(auth())
        .get("/", |admin: Authorized<User, AdminOnly>| async move {
            admin.name.clone()
        })
        .into_axum_router();
    assert_eq!(
        status(&router, "GET", "/", "alice").await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(&router, as_user("GET", "/", "root")).await.body,
        "root"
    );
}
