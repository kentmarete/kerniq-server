# Authorization

[Authentication](middleware.md#authentication) establishes *who* is calling;
authorization decides *what* they may do. Kerniq's authorization is opt-in
per handler and builds on the identity produced by the `Auth` middleware.
There are two equivalent ways to use it:

* the `#[authorize(..)]` attribute (feature `macros`), for declarative rules
  such as `has_any_role("USER", "ADMIN")`,
* the `Authorized<T, P>` extractor with a hand-written `Policy`, for rules
  that need code,
* the `Require<T>` middleware, for one rule shared by every route of a
  router (no feature needed).

Routes without any of them are unaffected.

## Enabling

```toml
[dependencies]
kerniq-server = { version = "0.1", features = ["macros"] }
```

## Exposing roles and permissions

Implement `Authorities` for the identity type your `Authenticator` returns.
How roles are stored is up to you: claims in a JWT, a database row, a
cached lookup.

```rust
use kerniq_server::middleware::auth::Authorities;

#[derive(Clone)]
struct User {
    name: String,
    roles: Vec<String>,
    permissions: Vec<String>,
}

impl Authorities for User {
    fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
    }

    // Optional: defaults to `false` for role-only identities.
    fn has_permission(&self, permission: &str) -> bool {
        self.permissions.iter().any(|p| p == permission)
    }
}
```

## `#[authorize]`

```rust
use kerniq_server::prelude::*;
use kerniq_server::middleware::auth::{Auth, Authorities, Identity};
use serde::Serialize;

#[derive(Clone)]
struct User { name: String, roles: Vec<String> }

impl Authorities for User {
    fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
    }
}

#[derive(Serialize)]
struct Profile { name: String }

// The identity type is inferred from the `Identity<User>` parameter.
#[authorize(has_any_role("USER", "ADMIN"))]
async fn profile(Identity(user): Identity<User>) -> Json<Profile> {
    Json(Profile { name: user.name })
}

// Without an `Identity<T>` parameter, name the type.
#[authorize(has_role("ADMIN"), identity = User)]
async fn delete_user(Path(id): Path<u64>) -> NoContent {
    let _ = id;
    NoContent
}

let auth = Auth::bearer(|token: String| async move {
    // Validate the token and load the user's roles.
    match token.as_str() {
        "t-admin" => Ok(User { name: "root".into(), roles: vec!["ADMIN".into()] }),
        _ => Err(Error::unauthorized("Invalid token")),
    }
});

let api = Router::new()
    .middleware(auth)
    .get("/profile", profile)
    .delete("/users/{id}", delete_user);
let app = App::new().mount("/api", api);
```

Responses:

| Situation | Status | Error code |
|---|---|---|
| No identity (no `Auth`, or authentication failed) | `401` | `UNAUTHORIZED` |
| Identity does not satisfy the rule | `403` | `FORBIDDEN` |

The check runs before the handler's other extractors, so an unauthorized
caller gets `403` without its request body being read or validated.

### Rules

| Rule | Passes when the identity |
|---|---|
| `authenticated()` | exists |
| `has_role("A")` | has role `A` |
| `has_any_role("A", "B", ..)` | has at least one of the roles |
| `has_all_roles("A", "B", ..)` | has every role |
| `has_permission("p")` | has permission `p` |
| `has_any_permission("p", "q", ..)` | has at least one of the permissions |
| `has_all_permissions("p", "q", ..)` | has every permission |
| `check(path::to::function)` | makes `fn(&T) -> bool` return `true` |

Combine rules with `&&`, `||`, `!` and parentheses:

```rust
use kerniq_server::prelude::*;
use kerniq_server::middleware::auth::Authorities;

#[derive(Clone)]
struct User { roles: Vec<String>, permissions: Vec<String>, suspended: bool }

impl Authorities for User {
    fn has_role(&self, role: &str) -> bool { self.roles.iter().any(|r| r == role) }
    fn has_permission(&self, p: &str) -> bool { self.permissions.iter().any(|x| x == p) }
}

fn not_suspended(user: &User) -> bool {
    !user.suspended
}

#[authorize(
    (has_role("ADMIN") || has_permission("reports:read")) && check(not_suspended),
    identity = User,
    message = "Reports are restricted",
)]
async fn reports() -> &'static str {
    "..."
}
```

Role and permission names are compared exactly as written: there is no
implicit `ROLE_` prefix as in Spring.

### Options

| Option | Meaning |
|---|---|
| `identity = Type` | Identity type; inferred from an `Identity<T>` parameter when omitted |
| `message = "..."` | Message of the `403` error (default `"Insufficient permissions"`) |
| `crate = path` | Path to `kerniq_server`, if the dependency was renamed |

### Limitations

`#[authorize]` adds a hidden `Authorized<T, P>` parameter and emits the
policy type `P` next to the function. It therefore supports free `async fn`
handlers (not methods or generic functions) and uses one of Axum's 16
extractor slots. Invalid rules, unknown rule names and identity types that
do not implement `Authorities` are compile errors.

## Protecting route groups: `Require`

Spring Security usually centralizes rules as path matchers:

```java
.requestMatchers("/api/auth/**", "/api/test/all").permitAll()
.requestMatchers("/api/test/user").hasRole("USER")
.requestMatchers("/api/test/admin").hasRole("ADMIN")
.requestMatchers("/api/test/users").hasAuthority("READ_USER")
.anyRequest().authenticated()
```

In Kerniq, middleware is scoped to routers, so the router structure is the
matcher list: routes that share a requirement live in one router, and
`Require` is applied to it. There are no path patterns to keep in sync with
the routes, and a new route gets exactly the protection of the router it is
added to.

```rust
use kerniq_server::prelude::*;
use kerniq_server::middleware::auth::{Auth, Authorities, Require};

#[derive(Clone)]
struct User { roles: Vec<String>, permissions: Vec<String> }

impl Authorities for User {
    fn has_role(&self, role: &str) -> bool { self.roles.iter().any(|r| r == role) }
    fn has_permission(&self, p: &str) -> bool { self.permissions.iter().any(|x| x == p) }
}

# async fn login() {} async fn all() {} async fn me() {}
# async fn user_board() {} async fn admin_board() {} async fn list_users() {}
let auth = Auth::bearer(|token: String| async move {
    // Validate the token and load roles and permissions.
    let _ = token;
    Ok::<_, Error>(User { roles: vec!["USER".into()], permissions: vec![] })
});

// permitAll()
let public = Router::new()
    .get("/auth/login", login)
    .get("/test/all", all);

// anyRequest().authenticated(): everything below requires `Auth`.
let protected = Router::new()
    .middleware(auth)
    .get("/test/me", me)
    .merge(
        Router::new()
            .middleware(Require::<User>::role("USER"))
            .get("/test/user", user_board),
    )
    .merge(
        Router::new()
            .middleware(Require::<User>::role("ADMIN"))
            .get("/test/admin", admin_board),
    )
    .merge(
        Router::new()
            .middleware(Require::<User>::permission("READ_USER"))
            .get("/test/users", list_users),
    );

let app = App::new()
    .mount("/api", public)
    .mount("/api", protected);
```

`Require` reads the identity that `Auth` stored, so `Auth` must run first:
declare it earlier on the same router (first declared is outermost) or on
an enclosing router, as above. If no identity is present, `Require` responds
`401`, so a misordered stack fails closed.

| Constructor | Passes when the identity |
|---|---|
| `Require::authenticated()` | exists |
| `Require::role("A")` | has role `A` |
| `Require::any_role(["A", "B"])` | has at least one of the roles |
| `Require::all_roles(["A", "B"])` | has every role |
| `Require::permission("p")` | has permission `p` |
| `Require::any_permission(["p", "q"])` | has at least one of the permissions |
| `Require::all_permissions(["p", "q"])` | has every permission |
| `Require::check(\|user: &User\| ..)` | makes the closure return `true` |
| `Require::policy::<P>()` | satisfies `Policy` `P` (its error is sent as-is) |

Rules combine with `.and(..)` and `.or(..)`, and `.message(..)` sets the
`403` message:

```rust
use kerniq_server::middleware::auth::{Authorities, Require};

# #[derive(Clone)] struct User;
# impl Authorities for User { fn has_role(&self, _: &str) -> bool { true } }
let support = Require::<User>::role("ADMIN")
    .or(Require::role("USER").and(Require::permission("READ_USER")))
    .message("Support staff only");
```

`Require` and `#[authorize]` compose: a router can require `USER` for all
its routes while one handler additionally requires `ADMIN`.

## Without macros: `Policy` and `Authorized`

The attribute is sugar over a public API. A `Policy` is a type whose
`check` decides whether an identity may proceed; it can return any error,
not just `403`:

```rust
use kerniq_server::prelude::*;
use kerniq_server::middleware::auth::{Authorized, Policy};

#[derive(Clone)]
struct User { id: u64, plan: &'static str }

struct PaidPlan;

impl Policy<User> for PaidPlan {
    fn check(user: &User) -> Result<()> {
        match user.plan {
            "pro" | "team" => Ok(()),
            _ => Err(Error::forbidden("A paid plan is required")
                .with_details(serde_json::json!({ "upgrade": "/billing" }))),
        }
    }
}

// `Authorized<T, P>` dereferences to `T`.
async fn export(user: Authorized<User, PaidPlan>) -> String {
    format!("export for {}", user.id)
}
```

Reusing a policy type across handlers keeps the rule in one place, and
`Authorized<User, PaidPlan>` in a signature documents the requirement.
