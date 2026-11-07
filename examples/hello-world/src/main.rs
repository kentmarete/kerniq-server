//! The smallest Kerniq application.
//!
//! ```sh
//! cargo run -p example-hello-world
//! curl localhost:3000/
//! ```

use kerniq_server::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let app = App::new()
        .get("/", || async { "Hello from Kerniq" })
        .get("/json", || async {
            Json(std::collections::BTreeMap::from([(
                "message",
                "Hello from Kerniq",
            )]))
        });

    println!("listening on http://127.0.0.1:3000");
    app.run("127.0.0.1:3000").await
}
