use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::models::workspace::WorkspaceSummary;

/// Full database row. Only ever serialized outward through [`MeResponse`].
#[derive(Debug, Clone, sqlx::FromRow)]
#[allow(dead_code)] // audit columns mapped ahead of first read
pub struct User {
    pub id: Uuid,
    pub github_id: i64,
    pub username: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub onboarded_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Non-sensitive profile returned by `GET /api/me`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeResponse {
    pub id: Uuid,
    pub username: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub onboarded: bool,
    pub workspace: Option<WorkspaceSummary>,
}

impl MeResponse {
    pub fn from_user(user: User, workspace: Option<WorkspaceSummary>) -> Self {
        Self {
            id: user.id,
            username: user.username,
            display_name: user.display_name,
            email: user.email,
            avatar_url: user.avatar_url,
            onboarded: user.onboarded_at.is_some(),
            workspace,
        }
    }
}
