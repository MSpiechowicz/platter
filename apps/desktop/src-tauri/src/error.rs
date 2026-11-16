//! The error every IPC command returns: the contract's `ApiError { code, message }`.

use std::io;

use platter_core::QueryError;
use platter_core::ScanError;
use platter_core::dto::{ApiError, ApiErrorCode};
use serde::Serialize;

/// Wraps [`ApiError`] so `?` works on the errors commands run into. Serializes as the bare `ApiError`
/// object, which is what the frontend receives as the rejection value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct CommandError(ApiError);

pub type CommandResult<T> = Result<T, CommandError>;

impl CommandError {
    pub fn new(code: ApiErrorCode, message: impl Into<String>) -> Self {
        Self(ApiError::new(code, message))
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ApiErrorCode::NotFound, message)
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ApiErrorCode::Invalid, message)
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self::new(ApiErrorCode::Io, message)
    }

    #[cfg(test)]
    pub fn code(&self) -> ApiErrorCode {
        self.0.code
    }
}

impl From<ApiError> for CommandError {
    fn from(error: ApiError) -> Self {
        Self(error)
    }
}

impl From<QueryError> for CommandError {
    fn from(error: QueryError) -> Self {
        Self(ApiError::from(error))
    }
}

impl From<&ScanError> for CommandError {
    fn from(error: &ScanError) -> Self {
        Self(ApiError::from(error))
    }
}

impl From<io::Error> for CommandError {
    fn from(error: io::Error) -> Self {
        Self::io(error.to_string())
    }
}

/// Covers a background task that panicked or was aborted (`spawn_blocking` rejects with `tauri::Error`).
impl From<tauri::Error> for CommandError {
    fn from(error: tauri::Error) -> Self {
        Self::io(format!("background task failed: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_the_contract_error_object() {
        let error = CommandError::not_found("node 7 does not exist");

        let json = serde_json::to_value(&error).unwrap();

        assert_eq!(
            json,
            serde_json::json!({ "code": "notFound", "message": "node 7 does not exist" })
        );
    }

    #[test]
    fn query_errors_are_not_found() {
        let error = CommandError::from(QueryError::NotFound(platter_core::NodeId(9)));

        assert_eq!(error.code(), ApiErrorCode::NotFound);
    }
}
