use std::fmt;

use kerniq_server::prelude::*;

/// Errors raised by the task service, mapped to HTTP responses below.
#[derive(Debug)]
pub enum TaskError {
    EmptyTitle,
    NotFound(u64),
}

impl fmt::Display for TaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskError::EmptyTitle => f.write_str("task title is empty"),
            TaskError::NotFound(id) => write!(f, "task {id} not found"),
        }
    }
}

impl std::error::Error for TaskError {}

impl From<TaskError> for Error {
    fn from(err: TaskError) -> Self {
        match err {
            TaskError::EmptyTitle => {
                Error::unprocessable("`title` must not be empty").with_code("EMPTY_TITLE")
            }
            TaskError::NotFound(_) => {
                Error::not_found("The requested task does not exist").with_code("TASK_NOT_FOUND")
            }
        }
    }
}
