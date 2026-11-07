//! A CRUD REST API with typed paths, queries, JSON bodies and domain errors.
//!
//! ```sh
//! cargo run -p example-rest-api
//! curl -X POST localhost:3000/users -H 'content-type: application/json' -d '{"name":"Ada"}'
//! curl 'localhost:3000/users?page=1&limit=10'
//! curl localhost:3000/users/1
//! curl -X PATCH localhost:3000/users/1 -H 'content-type: application/json' -d '{"name":"Ada L."}'
//! curl -X DELETE localhost:3000/users/1
//! curl localhost:3000/users/abc          # 400 INVALID_PATH_PARAMETER
//! ```

use std::collections::BTreeMap;
use std::sync::Arc;

use kerniq_server::prelude::*;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

#[derive(Clone, Serialize)]
struct User {
    id: u64,
    name: String,
}

#[derive(Deserialize)]
struct CreateUser {
    name: String,
}

#[derive(Deserialize)]
struct PatchUser {
    name: Option<String>,
}

#[derive(Deserialize)]
struct UserQuery {
    page: Option<u32>,
    limit: Option<u32>,
}

#[derive(Clone, Default)]
struct Db(Arc<RwLock<BTreeMap<u64, User>>>);

/// A domain error, mapped to HTTP in one place.
enum UserError {
    NotFound,
    EmptyName,
}

impl From<UserError> for Error {
    fn from(err: UserError) -> Self {
        match err {
            UserError::NotFound => {
                Error::not_found("The requested user does not exist").with_code("USER_NOT_FOUND")
            }
            UserError::EmptyName => {
                Error::unprocessable("`name` must not be empty").with_code("EMPTY_NAME")
            }
        }
    }
}

async fn list_users(State(db): State<Db>, Query(q): Query<UserQuery>) -> Json<Vec<User>> {
    let limit = q.limit.unwrap_or(20).min(100) as usize;
    let skip = (q.page.unwrap_or(1).max(1) as usize - 1) * limit;
    Json(
        db.0.read()
            .await
            .values()
            .skip(skip)
            .take(limit)
            .cloned()
            .collect(),
    )
}

async fn get_user(State(db): State<Db>, Path(id): Path<u64>) -> Result<Json<User>> {
    let users = db.0.read().await;
    Ok(Json(users.get(&id).cloned().ok_or(UserError::NotFound)?))
}

async fn create_user(State(db): State<Db>, Json(input): Json<CreateUser>) -> Result<Created<User>> {
    if input.name.trim().is_empty() {
        return Err(UserError::EmptyName.into());
    }
    let mut users = db.0.write().await;
    let id = users.keys().next_back().map_or(1, |last| last + 1);
    let user = User {
        id,
        name: input.name,
    };
    users.insert(id, user.clone());
    Ok(Created(user))
}

async fn patch_user(
    State(db): State<Db>,
    Path(id): Path<u64>,
    Json(input): Json<PatchUser>,
) -> Result<Json<User>> {
    let mut users = db.0.write().await;
    let user = users.get_mut(&id).ok_or(UserError::NotFound)?;
    if let Some(name) = input.name {
        user.name = name;
    }
    Ok(Json(user.clone()))
}

async fn delete_user(State(db): State<Db>, Path(id): Path<u64>) -> Result<NoContent> {
    db.0.write().await.remove(&id).ok_or(UserError::NotFound)?;
    Ok(NoContent)
}

#[tokio::main]
async fn main() -> Result<()> {
    let app = App::new()
        .state(Db::default())
        .get("/users", list_users)
        .post("/users", create_user)
        .get("/users/:id", get_user) // `:id` and `{id}` are equivalent
        .patch("/users/{id}", patch_user)
        .delete("/users/{id}", delete_user);

    println!("listening on http://127.0.0.1:3000");
    app.run("127.0.0.1:3000").await
}
