use axum::extract::Request;
use axum::http::Method;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::error::AppError;

/// Defense-in-depth CSRF guard on top of `SameSite=Lax` cookies: every
/// state-changing request must carry `X-Requested-With: XMLHttpRequest`.
/// Cross-site JavaScript cannot attach that header without passing our
/// CORS preflight, and plain form posts cannot attach headers at all.
pub async fn require_xhr_header(request: Request, next: Next) -> Response {
    let safe = matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );

    if !safe {
        let is_xhr = request
            .headers()
            .get("x-requested-with")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case("XMLHttpRequest"));
        if !is_xhr {
            return AppError::Forbidden.into_response();
        }
    }

    next.run(request).await
}
