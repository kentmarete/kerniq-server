//! Error handling: domain errors, `?`, error codes, details, internal errors
//! and a custom fallback. Every failure uses the same JSON envelope:
//! `{"error": {"code": "...", "message": "...", "details": ...}}`.
//!
//! ```sh
//! cargo run -p example-error-handling
//! curl -i localhost:3000/accounts/1                 # 200
//! curl -i localhost:3000/accounts/9                 # 404 ACCOUNT_NOT_FOUND
//! curl -i localhost:3000/accounts/abc               # 400 INVALID_PATH_PARAMETER
//! curl -i -X POST localhost:3000/accounts/1/withdraw \
//!      -H 'content-type: application/json' -d '{"amount":500}'   # 422 INSUFFICIENT_FUNDS
//! curl -i -X POST localhost:3000/accounts/2/withdraw \
//!      -H 'content-type: application/json' -d '{"amount":5}'     # 403 ACCOUNT_FROZEN
//! curl -i -X POST localhost:3000/accounts/1/withdraw \
//!      -H 'content-type: application/json' -d '{"amount":0}'     # 400 with details
//! curl -i -X POST localhost:3000/accounts/1/withdraw \
//!      -H 'content-type: application/json' -d 'not json'        # 400 MALFORMED_JSON
//! curl -i localhost:3000/report                     # 500, source logged, never sent
//! curl -i localhost:3000/limited                    # 429 with Retry-After
//! curl -i localhost:3000/nope                       # 404 from the custom fallback
//! ```

use kerniq_server::http::{HeaderValue, Uri, header};
use kerniq_server::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize)]
struct Account {
    id: u64,
    balance: i64,
    frozen: bool,
}

#[derive(Deserialize)]
struct Withdraw {
    amount: i64,
}

/// Domain errors know nothing about HTTP...
#[derive(Debug)]
enum AccountError {
    NotFound(u64),
    Frozen,
    InsufficientFunds { balance: i64, requested: i64 },
}

/// ...and are mapped to HTTP in exactly one place.
impl From<AccountError> for Error {
    fn from(err: AccountError) -> Self {
        match err {
            AccountError::NotFound(id) => Error::not_found(format!("Account {id} does not exist"))
                .with_code("ACCOUNT_NOT_FOUND"),
            AccountError::Frozen => {
                Error::forbidden("The account is frozen").with_code("ACCOUNT_FROZEN")
            }
            AccountError::InsufficientFunds { balance, requested } => {
                Error::unprocessable("The account balance is too low")
                    .with_code("INSUFFICIENT_FUNDS")
                    .with_details(serde_json::json!({ "balance": balance, "requested": requested }))
            }
        }
    }
}

/// Plain domain logic returning a domain error.
fn find(id: u64) -> std::result::Result<Account, AccountError> {
    match id {
        1 => Ok(Account {
            id,
            balance: 100,
            frozen: false,
        }),
        2 => Ok(Account {
            id,
            balance: 100,
            frozen: true,
        }),
        _ => Err(AccountError::NotFound(id)),
    }
}

fn withdraw(mut account: Account, amount: i64) -> std::result::Result<Account, AccountError> {
    if account.frozen {
        return Err(AccountError::Frozen);
    }
    if amount > account.balance {
        return Err(AccountError::InsufficientFunds {
            balance: account.balance,
            requested: amount,
        });
    }
    account.balance -= amount;
    Ok(account)
}

/// `?` converts `AccountError` into `Error` through the `From` impl above.
/// A non-numeric `id` is rejected by the `Path` extractor before this runs.
async fn get_account(Path(id): Path<u64>) -> Result<Json<Account>> {
    Ok(Json(find(id)?))
}

async fn post_withdraw(Path(id): Path<u64>, Json(body): Json<Withdraw>) -> Result<Json<Account>> {
    // Input validation with a machine-readable code and structured details.
    if body.amount <= 0 {
        return Err(Error::bad_request("`amount` must be positive")
            .with_code("INVALID_AMOUNT")
            .with_details(serde_json::json!({ "field": "amount", "value": body.amount })));
    }
    let account = withdraw(find(id)?, body.amount)?;
    Ok(Json(account))
}

/// Unexpected failures become a generic 500. The source error is logged at
/// `ERROR` level and never reaches the client.
async fn report() -> Result<String> {
    let text = std::fs::read_to_string("/definitely/missing/report.csv").or_internal()?;
    Ok(text)
}

/// Any status, with extra headers.
async fn limited() -> Result<NoContent> {
    Err(Error::too_many_requests("Slow down")
        .with_header(header::RETRY_AFTER, HeaderValue::from_static("30")))
}

/// Replaces the default JSON 404 for unknown routes.
async fn not_found(uri: Uri) -> Error {
    Error::not_found(format!("No route for {}", uri.path())).with_code("ROUTE_NOT_FOUND")
}

#[tokio::main]
async fn main() -> Result<()> {
    let app = App::new()
        .get("/accounts/:id", get_account)
        .post("/accounts/:id/withdraw", post_withdraw)
        .get("/report", report)
        .get("/limited", limited)
        .fallback(not_found);

    println!("listening on http://127.0.0.1:3000");
    app.run("127.0.0.1:3000").await
}
