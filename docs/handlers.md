# Handlers, responses and errors

## Handlers

A handler is an `async fn` (or closure) whose arguments are extractors and
whose return type implements `IntoResponse`. Kerniq handlers *are* Axum
handlers; there is no Kerniq-specific handler trait.

```rust
use kerniq_server::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct User { id: u64, name: String }

#[derive(Deserialize)]
struct CreateUser { name: String }

async fn health() -> &'static str {
    "OK"
}

async fn get_users() -> Json<Vec<User>> {
    Json(vec![])
}

async fn create_user(Json(payload): Json<CreateUser>) -> Result<Json<User>> {
    Ok(Json(User { id: 1, name: payload.name }))
}
```

Up to 16 extractors are supported. Extractors that read the request body
(`Json`, `String`, `Bytes`, `Body`, ...) must come last.

## Responses

Return anything implementing `IntoResponse`:

| Return type | Response |
|---|---|
| `&'static str`, `String` | `200`, `text/plain` |
| `Json<T>` | `200`, `application/json` |
| `Created<T>` | `201`, JSON |
| `NoContent` | `204` |
| `StatusCode` | that status, empty body |
| `(StatusCode, T)` | `T` with that status |
| `([(HeaderName, &str); N], T)` | `T` with extra headers |
| `Html<T>`, `Redirect` | HTML / redirects |
| `Result<T, E>` | `T` or `E` (both `IntoResponse`) |
| `Error` | the JSON error envelope |
| `Response` | full control |

```rust
use kerniq_server::prelude::*;

async fn teapot() -> (StatusCode, [(&'static str, &'static str); 1], &'static str) {
    (StatusCode::IM_A_TEAPOT, [("x-brewed-by", "kerniq")], "short and stout")
}
```

## The error model

Every error, from handlers, extractors, middleware, 404/405 fallbacks,
timeouts, rate limits or authentication, is rendered the same way:

```json
{
  "error": {
    "code": "USER_NOT_FOUND",
    "message": "The requested user does not exist"
  }
}
```

An optional `details` field carries structured data (for example
validation failures).

### Creating errors

```rust
use kerniq_server::prelude::*;

let e = Error::not_found("The requested user does not exist").with_code("USER_NOT_FOUND");
assert_eq!(e.status(), StatusCode::NOT_FOUND);

let e = Error::validation(serde_json::json!({"email": ["is invalid"]}));
assert_eq!(e.code(), "VALIDATION_FAILED");

let e = Error::new(StatusCode::PAYMENT_REQUIRED, "LIMIT_EXCEEDED", "Monthly limit reached");
```

Constructors: `bad_request`, `unauthorized`, `forbidden`, `not_found`,
`method_not_allowed`, `timeout`, `conflict`, `payload_too_large`,
`unsupported_media_type`, `unprocessable`, `too_many_requests`,
`service_unavailable`, `validation`, `internal`, plus `From<StatusCode>`.
Builders: `with_code`, `with_message`, `with_details`, `with_header`,
`with_source`.

### Internal errors never leak

`Error::internal(source)` responds with a generic message. The source error
(database messages, file paths, connection strings) is logged through
`tracing` at `ERROR` level and is never sent to the client:

```rust
use kerniq_server::prelude::*;

async fn load() -> Result<String> {
    let text = std::fs::read_to_string("/etc/app/secret.conf").or_internal()?;
    Ok(text)
}
```

### Domain errors

Map your error types to HTTP once, with `From`, and use `?` everywhere:

```rust
use kerniq_server::prelude::*;

#[derive(Debug)]
enum TransferError {
    InsufficientFunds,
    AccountFrozen(u64),
}

impl From<TransferError> for Error {
    fn from(err: TransferError) -> Self {
        match err {
            TransferError::InsufficientFunds => {
                Error::unprocessable("The account balance is too low").with_code("INSUFFICIENT_FUNDS")
            }
            TransferError::AccountFrozen(_) => {
                Error::forbidden("The account is frozen").with_code("ACCOUNT_FROZEN")
            }
        }
    }
}

fn debit(balance: i64, amount: i64) -> std::result::Result<i64, TransferError> {
    if amount > balance { Err(TransferError::InsufficientFunds) } else { Ok(balance - amount) }
}

async fn transfer() -> Result<String> {
    let remaining = debit(100, 500)?; // -> 422 INSUFFICIENT_FUNDS
    Ok(remaining.to_string())
}
```

Kerniq intentionally has no blanket `From<E: std::error::Error>`: that would
make the implementation above impossible and silently turn every domain error
into a 500. Use `.or_internal()` when an error really is unexpected, and
`.or_not_found(..)` / `.or_error(..)` on `Option`s and `Result`s.
