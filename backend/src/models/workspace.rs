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
    pub logo_key: Option<String>,
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

/// One row of the read-only members table in Settings → Workspace.
#[derive(Debug, sqlx::FromRow)]
pub struct WorkspaceMemberRow {
    pub user_id: Uuid,
    pub username: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub role_key: String,
    pub role_name: String,
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceMemberResponse {
    pub user_id: Uuid,
    pub username: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub role_key: String,
    pub role_name: String,
    pub joined_at: DateTime<Utc>,
}

impl From<WorkspaceMemberRow> for WorkspaceMemberResponse {
    fn from(row: WorkspaceMemberRow) -> Self {
        Self {
            user_id: row.user_id,
            username: row.username,
            display_name: row.display_name,
            email: row.email,
            avatar_url: row.avatar_url,
            role_key: row.role_key,
            role_name: row.role_name,
            joined_at: row.joined_at,
        }
    }
}
