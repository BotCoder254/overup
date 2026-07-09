use axum::Json;
use axum::extract::State;

use crate::db;
use crate::error::AppResult;
use crate::middleware::auth::CurrentUser;
use crate::models::user::MeResponse;
use crate::state::AppState;

pub async fn get_me(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> AppResult<Json<MeResponse>> {
    let workspace = db::workspaces::find_summary_for_user(&state.pool, user.id).await?;
    Ok(Json(MeResponse::from_user(user, workspace)))
}
