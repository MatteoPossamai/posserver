use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
#[derive(Debug)]
pub struct Error {
    pub status: u16,
    pub code: &'static str,
    pub details: Value,
    pub db_reason: Option<&'static str>,
}
pub type Result<T> = std::result::Result<T, Error>;
impl Error {
    pub fn new(status: u16, code: &'static str) -> Self {
        Self {
            status,
            code,
            details: json!({}),
            db_reason: None,
        }
    }
    pub fn detail(mut self, details: Value) -> Self {
        self.details = details;
        self
    }
    pub fn json(&self) -> Value {
        json!({"error":{"code":self.code,"message":self.code.replace('_', " "),"details":self.details}})
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        (
            StatusCode::from_u16(self.status).unwrap(),
            Json(self.json()),
        )
            .into_response()
    }
}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        use rusqlite::ErrorCode::*;
        let reason = match e.sqlite_error_code() {
            Some(DatabaseBusy | DatabaseLocked) => "busy",
            Some(DiskFull) => "disk_full",
            Some(SystemIoFailure | CannotOpen | ReadOnly | PermissionDenied) => "io",
            Some(ConstraintViolation) => "constraint",
            _ => "other",
        };
        let mut error = Self::new(503, "storage_unavailable");
        error.db_reason = Some(reason);
        error
    }
}
impl From<std::io::Error> for Error {
    fn from(_: std::io::Error) -> Self {
        Self::new(503, "storage_unavailable")
    }
}
pub fn invalid() -> Error {
    Error::new(422, "invalid_input")
}
