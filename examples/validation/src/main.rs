//! Declarative request validation with the `validator` crate.
//!
//! ```sh
//! cargo run -p example-validation
//! curl -i -X POST localhost:3000/transfers -H 'content-type: application/json' \
//!   -d '{"from_account":"","to_account":"acc-2","amount_minor":0,"reference":"x"}'
//! ```
//!
//! Invalid input yields `422 VALIDATION_FAILED` with per-field messages.

use kerniq_server::prelude::*;
use serde::{Deserialize, Serialize};
use validator::Validate;

#[derive(Deserialize, Validate)]
struct Transfer {
    #[validate(length(min = 1, message = "is required"))]
    from_account: String,
    #[validate(length(min = 1, message = "is required"))]
    to_account: String,
    /// Minor units; at most 1,000,000.00.
    #[validate(range(
        min = 1,
        max = 100_000_000,
        message = "must be between 1 and 100000000"
    ))]
    amount_minor: i64,
    #[validate(length(min = 3, max = 140))]
    reference: String,
}

#[derive(Serialize)]
struct Accepted {
    status: &'static str,
    amount_minor: i64,
}

async fn create_transfer(
    ValidatedJson(transfer): ValidatedJson<Transfer>,
) -> Result<Created<Accepted>> {
    if transfer.from_account == transfer.to_account {
        return Err(
            Error::unprocessable("Source and destination must differ").with_code("SAME_ACCOUNT")
        );
    }
    Ok(Created(Accepted {
        status: "accepted",
        amount_minor: transfer.amount_minor,
    }))
}

#[tokio::main]
async fn main() -> Result<()> {
    let app = App::new().post("/transfers", create_transfer);
    println!("listening on http://127.0.0.1:3000");
    app.run("127.0.0.1:3000").await
}
