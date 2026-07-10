use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

/// Database row for a registered runner. The registration token itself is
/// never stored — only its SHA-256 hash, and that column is never selected
/// into this struct.
#[derive(Debug, sqlx::FromRow)]
pub struct Runner {
    pub id: Uuid,
    #[allow(dead_code)] // scope column, filtered in queries
    pub workspace_id: Uuid,
    pub name: String,
    pub labels: Vec<String>,
    pub status: String,
    pub version: Option<String>,
    pub last_seen_at: Option<DateTime<Utc>>,
    #[allow(dead_code)] // audit column mapped ahead of first read
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnerResponse {
    pub id: Uuid,
    pub name: String,
    pub labels: Vec<String>,
    pub status: String,
    pub version: Option<String>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub revoked: bool,
}

impl From<Runner> for RunnerResponse {
    fn from(row: Runner) -> Self {
        Self {
            id: row.id,
            name: row.name,
            labels: row.labels,
            status: row.status,
            version: row.version,
            last_seen_at: row.last_seen_at,
            created_at: row.created_at,
            revoked: row.revoked_at.is_some(),
        }
    }
}
