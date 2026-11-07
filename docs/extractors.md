# Extractors

Extractors are handler arguments that parse part of the request. Kerniq's
extractors are in the prelude and in `kerniq_server::extract`.

| Extractor | Extracts | Rejection |
|---|---|---|
| `Path<T>` | path parameters | `400 INVALID_PATH_PARAMETER` |
| `Query<T>` | query string | `400 INVALID_QUERY` |
| `Json<T>` | JSON body | `400 MALFORMED_JSON`, `422 INVALID_JSON`, `415 UNSUPPORTED_MEDIA_TYPE`, `413 PAYLOAD_TOO_LARGE` |
| `State<T>` | application state | — (checked at compile time) |
| `Header<T>` | one typed header | `400 MISSING_HEADER` / `INVALID_HEADER` |
| `Body` | the raw body stream | — |
| `Bytes`, `String` | the whole body | `413` over the body limit |
| `HeaderMap`, `Method`, `Uri` | request head | — |
| `ValidatedJson<T>`, `ValidatedQuery<T>` | parsed and validated input | `422 VALIDATION_FAILED` (feature `validation`) |

`Path`, `Query` and `Json` delegate to Axum's extractors of the same name; the
only difference is that their rejections use Kerniq's JSON error envelope.
Axum's versions (`axum::Json`, ...) also work in Kerniq handlers, with Axum's
plain-text rejections.

## Path parameters

```rust
use kerniq_server::prelude::*;
use serde::Deserialize;

// GET /users/{id}
async fn get_user(Path(id): Path<u64>) -> String {
    format!("user {id}")
}

// GET /users/{user_id}/accounts/{account_id}
async fn get_account(Path((user_id, account_id)): Path<(u64, String)>) -> String {
    format!("{user_id}/{account_id}")
}

// Or by name
#[derive(Deserialize)]
struct AccountPath {
    user_id: u64,
    account_id: String,
}

async fn get_account_named(Path(p): Path<AccountPath>) -> String {
    format!("{}/{}", p.user_id, p.account_id)
}
```

`GET /users/abc` with `Path<u64>` responds:

```json
{"error":{"code":"INVALID_PATH_PARAMETER","message":"Invalid URL: Cannot parse `abc` to a `u64`"}}
```

## Query parameters

```rust
use kerniq_server::prelude::*;
use serde::Deserialize;
# #[derive(serde::Serialize)] struct User;

#[derive(Deserialize)]
struct UserQuery {
    page: Option<u32>,
    limit: Option<u32>,
}

async fn list_users(Query(query): Query<UserQuery>) -> Result<Json<Vec<User>>> {
    let _page = query.page.unwrap_or(1);
    let _limit = query.limit.unwrap_or(20).min(100);
    Ok(Json(vec![]))
}
```

## JSON bodies

```rust
use kerniq_server::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct CreateUser { name: String }

#[derive(Serialize)]
struct User { id: u64, name: String }

async fn create_user(Json(payload): Json<CreateUser>) -> Created<User> {
    Created(User { id: 1, name: payload.name })
}
```

The request must have `Content-Type: application/json`. Bodies larger than
`ServerConfig::max_body_size` (default 2 MiB) are rejected with `413`.

## Typed headers

`Header<T>` extracts one header implementing `FromHeader`. Built-in:
`UserAgent`, `ContentType`, `Authorization`, `BearerToken`, `XRequestId`.

```rust
use kerniq_server::prelude::*;
use kerniq_server::extract::{BearerToken, UserAgent};

async fn whoami(
    Header(UserAgent(agent)): Header<UserAgent>,
    token: Option<Header<BearerToken>>,
) -> String {
    match token {
        Some(Header(BearerToken(_))) => format!("authenticated via {agent}"),
        None => format!("anonymous via {agent}"),
    }
}
```

Define your own:

```rust
use kerniq_server::prelude::*;
use kerniq_server::http::HeaderValue;

struct TenantId(u32);

impl FromHeader for TenantId {
    const NAME: &'static str = "x-tenant-id";

    fn from_header_value(value: &HeaderValue) -> Option<Self> {
        value.to_str().ok()?.parse().ok().map(TenantId)
    }
}

async fn tenant(Header(TenantId(id)): Header<TenantId>) -> String {
    format!("tenant {id}")
}
```

## Validation

With the `validation` feature, `ValidatedJson<T>` and `ValidatedQuery<T>` run
[`validator`](https://docs.rs/validator) rules after parsing:

```rust
use kerniq_server::prelude::*;
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate)]
struct Signup {
    #[validate(email(message = "must be a valid email address"))]
    email: String,
    #[validate(length(min = 12))]
    password: String,
}

async fn signup(ValidatedJson(input): ValidatedJson<Signup>) -> Result<NoContent> {
    let _ = input;
    Ok(NoContent)
}
```

```json
{"error":{"code":"VALIDATION_FAILED","message":"The request failed validation",
  "details":{"email":["must be a valid email address"],"password":["failed `length` validation"]}}}
```

Nested fields are reported as `address.city` and `items[2].sku`. Submitted
values are never echoed back.

## Raw bodies and other Axum extractors

```rust
use kerniq_server::prelude::*;

async fn upload(headers: HeaderMap, body: Bytes) -> String {
    format!("{} bytes, content-type {:?}", body.len(), headers.get("content-type"))
}
```

Any type implementing Axum's `FromRequestParts` / `FromRequest` is an
extractor, including those from `axum-extra` and other crates.
