use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

/// Environment metadata row for the catalog and detail views, with creator/
/// updater logins and the count of secrets scoped to it joined in.
#[derive(Debug, sqlx::FromRow)]
pub struct EnvironmentMeta {
    pub id: Uuid,
    #[allow(dead_code)]
    pub workspace_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub creator_login: Option<String>,
    pub updater_login: Option<String>,
    pub secret_count: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// API shape (camelCase).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentResponse {
    pub id: Uuid,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator_login: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updater_login: Option<String>,
    pub secret_count: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<EnvironmentMeta> for EnvironmentResponse {
    fn from(row: EnvironmentMeta) -> Self {
        Self {
            id: row.id,
            name: row.name,
            description: row.description,
            creator_login: row.creator_login,
            updater_login: row.updater_login,
            secret_count: row.secret_count,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}
