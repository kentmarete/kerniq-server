use kerniq_server::prelude::*;

use crate::models::task::{NewTask, Task};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .get("/", list)
        .post("/", create)
        .get("/{id}", show)
}

async fn list(State(state): State<AppState>) -> Json<Vec<Task>> {
    Json(state.tasks.list().await)
}

async fn show(State(state): State<AppState>, Path(id): Path<u64>) -> Result<Json<Task>> {
    Ok(Json(state.tasks.get(id).await?))
}

async fn create(
    State(state): State<AppState>,
    Json(input): Json<NewTask>,
) -> Result<Created<Task>> {
    Ok(Created(state.tasks.create(input).await?))
}
