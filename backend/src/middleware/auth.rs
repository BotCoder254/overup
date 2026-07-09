use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum_extra::extract::cookie::CookieJar;

use crate::db;
use crate::error::AppError;
use crate::models::user::User;
use crate::services::session;
use crate::state::AppState;

/// Extractor for authenticated routes: resolves the session cookie to a
/// live user or rejects with 401. Possession of a cookie alone is not
/// enough — the hashed token must match an unexpired session row.
pub struct CurrentUser(pub User);

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        let jar = CookieJar::from_headers(&parts.headers);
        let token = jar
            .get(&state.config.cookie_name)
            .map(|cookie| cookie.value().to_string())
            .ok_or(AppError::Unauthorized)?;

        let user = db::sessions::find_valid_user(&state.pool, &session::hash_token(&token))
            .await?
            .ok_or(AppError::Unauthorized)?;

        Ok(CurrentUser(user))
    }
}
