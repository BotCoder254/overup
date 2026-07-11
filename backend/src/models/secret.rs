use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

/// Secret metadata row for the catalog and detail views, with creator /
/// updater logins and the repository name joined in. Deliberately contains
/// NO ciphertext columns — cipher material only travels on the dedicated
/// dispatch-path row type in `db::secrets`, and plaintext exists nowhere.
#[derive(Debug, sqlx::FromRow)]
pub struct SecretMeta {
    pub id: Uuid,
    #[allow(dead_code)]
    pub workspace_id: Uuid,
    pub repository_id: Option<Uuid>,
    pub repository_name: Option<String>,
    pub environment_id: Option<Uuid>,
    pub environment_name: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub creator_login: Option<String>,
    pub updater_login: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// When the value was last set (created or replaced) — the rotation
    /// clock; updated_at also moves on metadata edits.
    pub value_set_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub usage_count: i64,
}

/// API shape. The value is write-only by design: no response DTO in the
/// entire codebase carries a secret value field.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretResponse {
    pub id: Uuid,
    pub name: String,
    /// `workspace`, `repository`, or `environment`.
    pub scope: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator_login: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updater_login: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub value_set_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<DateTime<Utc>>,
    pub usage_count: i64,
}

impl From<SecretMeta> for SecretResponse {
    fn from(row: SecretMeta) -> Self {
        Self {
            id: row.id,
            name: row.name,
            scope: if row.repository_id.is_some() {
                "repository"
            } else if row.environment_id.is_some() {
                "environment"
            } else {
                "workspace"
            },
            repository_id: row.repository_id,
            repository_name: row.repository_name,
            environment_id: row.environment_id,
            environment_name: row.environment_name,
            description: row.description,
            creator_login: row.creator_login,
            updater_login: row.updater_login,
            created_at: row.created_at,
            updated_at: row.updated_at,
            value_set_at: row.value_set_at,
            last_used_at: row.last_used_at,
            usage_count: row.usage_count,
        }
    }
}
