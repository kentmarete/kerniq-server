use kerniq_server::prelude::*;

use crate::models::user::{NewUser, User};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .get("/", list)
        .post("/", create)
        .get("/{id}", show)
}

async fn list(State(state): State<AppState>) -> Json<Vec<User>> {
    Json(state.users.list().await)
}

async fn show(State(state): State<AppState>, Path(id): Path<u64>) -> Result<Json<User>> {
    Ok(Json(state.users.get(id).await?))
}

async fn create(
    State(state): State<AppState>,
    Json(input): Json<NewUser>,
) -> Result<Created<User>> {
    Ok(Created(state.users.create(input).await?))
}
