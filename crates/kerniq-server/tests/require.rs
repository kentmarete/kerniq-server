//! Router-level authorization with `Require`.

mod common;

use axum::body::Body;
use common::*;
use http::{Request, StatusCode};
use kerniq_server::middleware::auth::{Auth, Authorities, Policy, Require};
use kerniq_server::prelude::*;

#[derive(Clone, Debug)]
struct User {
    name: &'static str,
    roles: &'static [&'static str],
    permissions: &'static [&'static str],
}

impl Authorities for User {
    fn has_role(&self, role: &str) -> bool {
        self.roles.contains(&role)
    }
    fn has_permission(&self, permission: &str) -> bool {
        self.permissions.contains(&permission)
    }
}

fn auth() -> impl Middleware {
    Auth::bearer(|token: String| async move {
        let user = |name, roles, permissions| User {
            name,
            roles,
            permissions,
        };
        match token.as_str() {
            "guest" => Ok(user("guest", &[], &[])),
            "alice" => Ok(user("alice", &["USER"], &[])),
            "root" => Ok(user("root", &["ADMIN"], &["READ_USER"])),
            "support" => Ok(user("support", &["USER"], &["READ_USER"])),
            _ => Err(Error::unauthorized("Invalid token")),
        }
    })
}

/// The Spring Security configuration
///
/// ```java
/// .requestMatchers("/api/auth/**", "/api/test/all").permitAll()
/// .requestMatchers("/api/test/user").hasRole("USER")
/// .requestMatchers("/api/test/admin").hasRole("ADMIN")
/// .requestMatchers("/api/test/users").hasAuthority("READ_USER")
/// .anyRequest().authenticated()
/// ```
///
/// expressed as Kerniq routers.
fn app() -> axum::Router {
    let public = Router::new()
        .get("/auth/login", || async { "login" })
        .get("/test/all", || async { "all" });

    let protected = Router::new()
        .middleware(auth()) // anyRequest().authenticated()
        .get("/test/me", || async { "me" })
        .merge(
            Router::new()
                .middleware(Require::<User>::role("USER"))
                .get("/test/user", || async { "user" }),
        )
        .merge(
            Router::new()
                .middleware(Require::<User>::role("ADMIN"))
                .get("/test/admin", || async { "admin" }),
        )
        .merge(
            Router::new()
                .middleware(Require::<User>::permission("READ_USER"))
                .get("/test/users", || async { "users" }),
        );

    App::new()
        .mount("/api", public)
        .mount("/api", protected)
        .into_axum_router()
}

fn as_user(uri: &str, token: &str) -> Request<Body> {
    Request::get(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

async fn status(router: &axum::Router, uri: &str, token: Option<&str>) -> StatusCode {
    match token {
        Some(token) => send(router, as_user(uri, token)).await.status,
        None => get(router, uri).await.status,
    }
}

#[tokio::test]
async fn spring_style_matrix() {
    use StatusCode as S;
    let router = app();
    let expectations: &[(&str, Option<&str>, StatusCode)] = &[
        ("/api/auth/login", None, S::OK),
        ("/api/test/all", None, S::OK),
        ("/api/test/me", None, S::UNAUTHORIZED),
        ("/api/test/me", Some("guest"), S::OK),
        ("/api/test/user", None, S::UNAUTHORIZED),
        ("/api/test/user", Some("guest"), S::FORBIDDEN),
        ("/api/test/user", Some("alice"), S::OK),
        ("/api/test/user", Some("root"), S::FORBIDDEN),
        ("/api/test/admin", Some("alice"), S::FORBIDDEN),
        ("/api/test/admin", Some("root"), S::OK),
        ("/api/test/users", Some("alice"), S::FORBIDDEN),
        ("/api/test/users", Some("support"), S::OK),
        ("/api/test/users", Some("root"), S::OK),
    ];
    for &(uri, token, expected) in expectations {
        assert_eq!(
            status(&router, uri, token).await,
            expected,
            "{uri} as {token:?}"
        );
    }
    let res = send(&router, as_user("/api/test/admin", "alice")).await;
    assert_eq!(res.error_code(), "FORBIDDEN");
    assert_eq!(res.json()["error"]["message"], "Insufficient permissions");
}

#[tokio::test]
async fn combinators_and_message() {
    let rule = Require::<User>::role("ADMIN")
        .or(Require::role("USER").and(Require::permission("READ_USER")))
        .message("Support staff only");
    let router = App::new()
        .middleware(auth())
        .middleware(rule)
        .get("/", || async { "ok" })
        .into_axum_router();
    for (token, expected) in [
        ("root", StatusCode::OK),
        ("support", StatusCode::OK),
        ("alice", StatusCode::FORBIDDEN),
    ] {
        assert_eq!(status(&router, "/", Some(token)).await, expected, "{token}");
    }
    let res = send(&router, as_user("/", "alice")).await;
    assert_eq!(res.json()["error"]["message"], "Support staff only");
}

#[tokio::test]
async fn any_and_all_roles_and_custom_check() {
    let build = |rule: Require<User>| {
        App::new()
            .middleware(auth())
            .middleware(rule)
            .get("/", || async { "ok" })
            .into_axum_router()
    };
    let any = build(Require::any_role(["USER", "ADMIN"]));
    assert_eq!(status(&any, "/", Some("root")).await, StatusCode::OK);
    assert_eq!(
        status(&any, "/", Some("guest")).await,
        StatusCode::FORBIDDEN
    );

    let all = build(Require::all_permissions(["READ_USER", "WRITE_USER"]));
    assert_eq!(status(&all, "/", Some("root")).await, StatusCode::FORBIDDEN);

    let named = build(Require::check(|user: &User| user.name.starts_with('a')));
    assert_eq!(status(&named, "/", Some("alice")).await, StatusCode::OK);
    assert_eq!(
        status(&named, "/", Some("root")).await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn reuses_policy_types_and_their_errors() {
    struct NotGuest;
    impl Policy<User> for NotGuest {
        fn check(user: &User) -> Result<()> {
            if user.name == "guest" {
                Err(Error::conflict("Finish sign-up first"))
            } else {
                Ok(())
            }
        }
    }
    let router = App::new()
        .middleware(auth())
        .middleware(Require::<User>::policy::<NotGuest>())
        .get("/", || async { "ok" })
        .into_axum_router();
    assert_eq!(status(&router, "/", Some("alice")).await, StatusCode::OK);
    assert_eq!(
        status(&router, "/", Some("guest")).await,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn fails_closed_without_auth() {
    // Require before (outside) Auth sees no identity yet.
    let router = App::new()
        .middleware(Require::<User>::authenticated())
        .middleware(auth())
        .get("/", || async { "ok" })
        .into_axum_router();
    assert_eq!(
        status(&router, "/", Some("root")).await,
        StatusCode::UNAUTHORIZED
    );
}
