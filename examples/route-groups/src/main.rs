//! Organizing routes: prefixes, groups, independent routers and nesting.
//!
//! ```sh
//! cargo run -p example-route-groups
//! curl localhost:3000/api/v1/users
//! curl localhost:3000/users/7
//! curl localhost:3000/accounts
//! curl localhost:3000/admin/v2/stats
//! ```

use kerniq_server::prelude::*;

async fn get_users() -> &'static str {
    "users"
}
async fn create_user() -> &'static str {
    "created"
}
async fn get_accounts() -> &'static str {
    "accounts"
}

#[tokio::main]
async fn main() -> Result<()> {
    // 1. A router with its own prefix, routes registered in a block.
    let api = Router::new().prefix("/api/v1").group(|router| {
        router.get("/users", get_users);
        router.post("/users", create_user);
        router.get("/accounts", get_accounts);
    });

    // 2. Independent routers, mounted at a path by the app.
    let users = Router::new()
        .get("/", get_users)
        .post("/", create_user)
        .get("/{id}", |Path(id): Path<u64>| async move {
            format!("user {id}")
        });
    let accounts = Router::new().get("/", get_accounts);

    // 3. Routers nest arbitrarily; prefixes compose.
    let admin = Router::new().mount(
        "/v2",
        Router::new().get("/stats", || async { "admin stats" }),
    );

    let app = App::new()
        .merge(api)
        .mount("/users", users)
        .mount("/accounts", accounts)
        .mount("/admin", admin);

    for route in app.routes() {
        println!("{:>6} {}", route.method, route.path);
    }
    app.run("127.0.0.1:3000").await
}
