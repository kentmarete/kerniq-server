use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct Task {
    pub id: u64,
    pub title: String,
    pub done: bool,
}

#[derive(Debug, Deserialize)]
pub struct NewTask {
    pub title: String,
}
