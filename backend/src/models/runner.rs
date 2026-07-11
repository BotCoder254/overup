use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

/// Database row for a registered runner. The registration token itself is
/// never stored — only its SHA-256 hash, and that column is never selected
/// into this struct.
#[derive(Debug, sqlx::FromRow)]
pub struct Runner {
    pub id: Uuid,
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
    pub last_health: Option<serde_json::Value>,
    #[allow(dead_code)] // reserved for the Runner Management dashboard
    pub last_health_at: Option<DateTime<Utc>>,
    pub draining_at: Option<DateTime<Utc>>,
    /// Hosted runner provisioned by the control plane itself.
    pub managed: bool,
    /// Docker container backing a managed runner; internal — never serialized.
    pub container_id: Option<String>,
    /// Static failure category when background provisioning failed
    /// (image_pull_failed, container_create_failed, ...); NULL otherwise.
    pub provision_error: Option<String>,
    /// Sizing preset for hosted runners (small|standard|large); NULL for
    /// self-hosted rows.
    pub resource_profile: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
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
    pub last_health: Option<serde_json::Value>,
    pub draining: bool,
    pub managed: bool,
    pub provision_error: Option<String>,
    pub resource_profile: Option<String>,
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
            last_health: row.last_health,
            draining: row.draining_at.is_some(),
            managed: row.managed,
            provision_error: row.provision_error,
            resource_profile: row.resource_profile,
        }
    }
}
