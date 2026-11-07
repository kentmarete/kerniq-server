# Routing

Kerniq registers routes into Axum's router. Matching is done by Axum
(`matchit`), so routing performance is Axum's.

## HTTP methods

```rust
use kerniq_server::prelude::*;
# async fn get_users() {} async fn create_user() {} async fn update_user() {}
# async fn patch_user() {} async fn delete_user() {}

let app = App::new()
    .get("/users", get_users)
    .post("/users", create_user)
    .put("/users/{id}", update_user)
    .patch("/users/{id}", patch_user)
    .delete("/users/{id}", delete_user);
```

| Method | Registration |
|---|---|
| GET (also answers HEAD) | `.get(path, handler)` |
| POST, PUT, PATCH, DELETE | `.post`, `.put`, `.patch`, `.delete` |
| OPTIONS, HEAD, TRACE | `.options`, `.head`, `.trace` |
| Every method | `.any(path, handler)` |
| Any standard method | `.on(Method::CONNECT, path, handler)` |
| Several methods / per-route layers | `.route(path, methods::get(a).post(b))` |

`on` panics at startup for extension methods (for example `PURGE`), which
Axum's method router cannot match. Register those with `any` and inspect the
`Method` in the handler.

Requests for a known path with the wrong method get `405 METHOD_NOT_ALLOWED`
with an `Allow` header. Unknown paths get `404 NOT_FOUND`. Both use the JSON
error envelope; replace the 404 with `App::fallback(handler)`.

## Path syntax

The canonical syntax is Axum's:

| Pattern | Matches | Extract with |
|---|---|---|
| `/users/{id}` | `/users/42` | `Path<u64>` |
| `/users/{user_id}/accounts/{account_id}` | `/users/1/accounts/a` | `Path<(u64, String)>` or a struct |
| `/files/{*path}` | `/files/a/b.txt` | `Path<String>` (`"a/b.txt"`) |

The older `:id` and `*path` forms are accepted and converted when the route is
registered, so `.get("/users/:id", h)` and `.get("/users/{id}", h)` are
identical.

Trailing slashes are significant: `/users` and `/users/` are different routes.
A `"/"` route on a router mounted at `/users` answers `/users`.

## Route groups

Register related routes in a block. Prefixes apply to every route in the
router, whether registered before or after `prefix`:

```rust
use kerniq_server::prelude::*;
# async fn get_users() {} async fn create_user() {} async fn get_accounts() {}

let api = Router::new()
    .prefix("/api/v1")
    .group(|router| {
        router.get("/users", get_users);
        router.post("/users", create_user);
        router.get("/accounts", get_accounts);
    });

let app = App::new().merge(api);
```

## Independent routers

Define routers per feature, then mount them:

```rust
use kerniq_server::prelude::*;
# async fn get_users() {} async fn create_user() {} async fn get_accounts() {}

let users = Router::new()
    .get("/", get_users)
    .post("/", create_user);

let accounts = Router::new()
    .get("/", get_accounts);

let app = App::new()
    .mount("/users", users)
    .mount("/accounts", accounts);
```

* `mount(path, router)` serves `router` under `path`, combined with the
  router's own prefix.
* `merge(router)` serves it at its own prefix.

Routers nest arbitrarily and prefixes compose:

```rust
use kerniq_server::prelude::*;

let v1 = Router::new().prefix("/v1").get("/ping", || async { "pong" });
let api = Router::new().mount("/api", v1);
let app = App::new().mount("/svc", api);

assert_eq!(app.routes()[0].path, "/svc/api/v1/ping");
```

Mounting copies routes into the parent's table: composition costs nothing per
request.

## Router fallbacks

A mounted router can have its own fallback for unmatched paths under its
prefix:

```rust
use kerniq_server::prelude::*;

let api = Router::new()
    .get("/status", || async { "ok" })
    .fallback(|| async { Error::not_found("Unknown API endpoint").with_code("UNKNOWN_ENDPOINT") });

let app = App::new().mount("/api", api);
```

## Introspection

`App::routes()` and `Router::routes()` list registered routes, which is
useful for startup logs and tests:

```rust
use kerniq_server::prelude::*;

let app = App::new().get("/a", || async { "a" }).post("/b", || async { "b" });
for route in app.routes() {
    println!("{} {}", route.method, route.path);
}
```

Mounted Axum routers appear as a single opaque `*` entry.
