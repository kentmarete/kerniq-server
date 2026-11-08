use std::collections::BTreeMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::error::TaskError;
use crate::models::task::{NewTask, Task};

/// Task business logic. No HTTP types here: it takes and returns domain
/// values and fails with `TaskError`.
#[derive(Debug, Clone, Default)]
pub struct TaskService {
    tasks: Arc<RwLock<BTreeMap<u64, Task>>>,
}

impl TaskService {
    pub async fn list(&self) -> Vec<Task> {
        self.tasks.read().await.values().cloned().collect()
    }

    pub async fn get(&self, id: u64) -> Result<Task, TaskError> {
        self.tasks
            .read()
            .await
            .get(&id)
            .cloned()
            .ok_or(TaskError::NotFound(id))
    }

    pub async fn create(&self, input: NewTask) -> Result<Task, TaskError> {
        let title = input.title.trim();
        if title.is_empty() {
            return Err(TaskError::EmptyTitle);
        }
        let mut tasks = self.tasks.write().await;
        let id = tasks.keys().next_back().copied().unwrap_or(0) + 1;
        let task = Task {
            id,
            title: title.to_owned(),
            done: false,
        };
        tasks.insert(id, task.clone());
        Ok(task)
    }

    pub async fn complete(&self, id: u64) -> Result<Task, TaskError> {
        let mut tasks = self.tasks.write().await;
        let task = tasks.get_mut(&id).ok_or(TaskError::NotFound(id))?;
        task.done = true;
        Ok(task.clone())
    }

    pub async fn delete(&self, id: u64) -> Result<(), TaskError> {
        self.tasks
            .write()
            .await
            .remove(&id)
            .map(|_| ())
            .ok_or(TaskError::NotFound(id))
    }
}
