use kerniq_server::prelude::*;
use serde::Serialize;

use crate::state::AppState;

#[derive(Serialize)]
pub struct Health {
    service: String,
    status: &'static str,
}

pub async fn health(State(state): State<AppState>) -> Json<Health> {
    Json(Health {
        service: state.service_name.to_string(),
        status: "healthy",
    })
}
