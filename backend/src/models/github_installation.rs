use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

/// Database row: a GitHub App installation linked to a workspace.
#[derive(Debug, sqlx::FromRow)]
pub struct GitHubInstallation {
    pub id: Uuid,
    #[allow(dead_code)] // audit column mapped ahead of first read
    pub workspace_id: Uuid,
    pub installation_id: i64,
    pub account_login: String,
    pub account_type: String,
    pub account_avatar_url: Option<String>,
    #[allow(dead_code)]
    pub linked_by: Option<Uuid>,
    pub suspended_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    #[allow(dead_code)]
    pub updated_at: DateTime<Utc>,
}

/// API shape. The numeric GitHub installation id is intentionally included —
/// it is not a secret (it appears in GitHub UI URLs) and the frontend needs
/// it only for display; all API routes key off the internal UUID.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallationResponse {
    pub id: Uuid,
    pub account_login: String,
    pub account_type: String,
    pub account_avatar_url: Option<String>,
    pub suspended: bool,
    pub created_at: DateTime<Utc>,
}

impl From<GitHubInstallation> for InstallationResponse {
    fn from(row: GitHubInstallation) -> Self {
        Self {
            id: row.id,
            account_login: row.account_login,
            account_type: row.account_type,
            account_avatar_url: row.account_avatar_url,
            suspended: row.suspended_at.is_some(),
            created_at: row.created_at,
        }
    }
}
