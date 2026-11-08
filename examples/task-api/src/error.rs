use std::fmt;

use kerniq_server::prelude::*;

/// Errors raised by the user service. Handlers return them with `?`; the
/// `From` implementation below decides how each one is presented over HTTP.
#[derive(Debug)]
pub enum UserError {
    NotFound(u64),
    EmailTaken,
    InvalidEmail,
}

impl fmt::Display for UserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UserError::NotFound(id) => write!(f, "user {id} not found"),
            UserError::EmailTaken => f.write_str("email already registered"),
            UserError::InvalidEmail => f.write_str("invalid email"),
        }
    }
}

impl std::error::Error for UserError {}

impl From<UserError> for Error {
    fn from(err: UserError) -> Self {
        match err {
            UserError::NotFound(_) => {
                Error::not_found("The requested user does not exist").with_code("USER_NOT_FOUND")
            }
            UserError::EmailTaken => {
                Error::conflict("A user with this email already exists").with_code("EMAIL_TAKEN")
            }
            UserError::InvalidEmail => {
                Error::unprocessable("`email` must be a valid email address")
                    .with_code("INVALID_EMAIL")
            }
        }
    }
}

/// Errors raised by the task service.
#[derive(Debug)]
pub enum TaskError {
    NotFound(u64),
    EmptyTitle,
}

impl fmt::Display for TaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskError::NotFound(id) => write!(f, "task {id} not found"),
            TaskError::EmptyTitle => f.write_str("task title is empty"),
        }
    }
}

impl std::error::Error for TaskError {}

impl From<TaskError> for Error {
    fn from(err: TaskError) -> Self {
        match err {
            TaskError::NotFound(_) => {
                Error::not_found("The requested task does not exist").with_code("TASK_NOT_FOUND")
            }
            TaskError::EmptyTitle => {
                Error::unprocessable("`title` must not be empty").with_code("EMPTY_TITLE")
            }
        }
    }
}
