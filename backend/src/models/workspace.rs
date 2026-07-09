use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

/// Full database row. Only ever serialized outward through
/// [`WorkspaceResponse`] / [`WorkspaceSummary`].
#[derive(Debug, Clone, sqlx::FromRow)]
#[allow(dead_code)] // audit columns mapped ahead of first read
pub struct Workspace {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Payload returned by `POST /api/workspaces`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceResponse {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
}

impl From<Workspace> for WorkspaceResponse {
    fn from(workspace: Workspace) -> Self {
        Self {
            id: workspace.id,
            name: workspace.name,
            slug: workspace.slug,
            description: workspace.description,
        }
    }
}

/// Compact shape embedded in `GET /api/me` for routing decisions.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSummary {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
}
