//! Stable error codes for adapters; the human message is supplementary.
use serde::Serialize;
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    InvalidArguments,
    Forbidden,
    NotFound,
    Conflict,
}
#[derive(Debug)]
pub struct Error {
    pub code: Code,
    pub message: String,
}
impl Error {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Error {}
pub fn result(error: &anyhow::Error) -> serde_json::Value {
    if let Some(error) = error.downcast_ref::<crate::media::extractors::MediaError>() {
        serde_json::json!({"error":error.message,"error_code":error.code,"phase":error.phase,"retryable":error.retryable})
    } else if let Some(error) = error.downcast_ref::<Error>() {
        serde_json::json!({"error":error.message,"error_code":error.code})
    } else if matches!(
        error.downcast_ref::<rusqlite::Error>(),
        Some(rusqlite::Error::QueryReturnedNoRows)
    ) {
        serde_json::json!({"error":"未找到指定记录，请检查 ID 或先查看列表","error_code":"not_found"})
    } else {
        serde_json::json!({"error":error.to_string(),"error_code":"operation_failed"})
    }
}
