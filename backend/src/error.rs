use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// Central application error. Internal details are logged with structured
/// tracing but NEVER leak into HTTP responses — clients only ever see a
/// stable machine-readable code and a generic message.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("conflict: {0}")]
    Conflict(&'static str),
    #[error("oauth flow failed")]
    OAuth(#[source] anyhow::Error),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    fn status_and_code(&self) -> (StatusCode, &'static str) {
        match self {
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            AppError::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            AppError::Validation(_) => (StatusCode::UNPROCESSABLE_ENTITY, "validation_failed"),
            AppError::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            AppError::OAuth(_) => (StatusCode::BAD_GATEWAY, "oauth_failed"),
            AppError::Database(_) | AppError::Internal(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
            }
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code) = self.status_and_code();

        // Full detail stays server-side.
        if status.is_server_error() || matches!(self, AppError::OAuth(_)) {
            tracing::error!(error = ?self, %status, "request failed");
        } else {
            tracing::debug!(error = ?self, %status, "request rejected");
        }

        let message = match &self {
            AppError::Validation(msg) => msg.clone(),
            // Static strings only — nothing internal can leak through here.
            AppError::Conflict(msg) => (*msg).to_string(),
            _ => code.replace('_', " "),
        };

        let body = Json(json!({ "error": { "code": code, "message": message } }));
        (status, body).into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;
