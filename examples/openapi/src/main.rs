//! OpenAPI documentation with Utoipa and Swagger UI.
//!
//! ```sh
//! cargo run -p example-openapi
//! open http://127.0.0.1:3000/docs
//! curl localhost:3000/openapi.json
//! ```

use kerniq_server::openapi::OpenApiDocs;
use kerniq_server::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::{OpenApi, ToSchema};

#[derive(Serialize, ToSchema)]
struct Account {
    id: u64,
    owner: String,
    /// Balance in minor units (cents).
    balance_minor: i64,
}

#[derive(Deserialize, ToSchema)]
struct OpenAccount {
    owner: String,
}

/// Fetch an account.
#[utoipa::path(
    get,
    path = "/accounts/{id}",
    params(("id" = u64, Path, description = "Account id")),
    responses(
        (status = 200, description = "The account", body = Account),
        (status = 404, description = "No such account"),
    )
)]
async fn get_account(Path(id): Path<u64>) -> Result<Json<Account>> {
    if id != 1 {
        return Err(Error::not_found("No such account").with_code("ACCOUNT_NOT_FOUND"));
    }
    Ok(Json(Account {
        id,
        owner: "Ada".into(),
        balance_minor: 10_000,
    }))
}

/// Open an account.
#[utoipa::path(
    post,
    path = "/accounts",
    request_body = OpenAccount,
    responses((status = 201, description = "Account opened", body = Account))
)]
async fn open_account(Json(input): Json<OpenAccount>) -> Created<Account> {
    Created(Account {
        id: 2,
        owner: input.owner,
        balance_minor: 0,
    })
}

#[derive(OpenApi)]
#[openapi(
    info(title = "Accounts API", version = "1.0.0"),
    paths(get_account, open_account),
    components(schemas(Account, OpenAccount))
)]
struct ApiDoc;

#[tokio::main]
async fn main() -> Result<()> {
    let docs = OpenApiDocs::new(ApiDoc::openapi()).swagger_ui("/docs");

    let app = App::new()
        .get("/accounts/{id}", get_account)
        .post("/accounts", open_account)
        .merge(docs.into_router());

    println!("docs at http://127.0.0.1:3000/docs");
    app.run("127.0.0.1:3000").await
}
