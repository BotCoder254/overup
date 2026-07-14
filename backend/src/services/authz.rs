//! Workspace authorization. First real consumer of the RBAC tables seeded
//! at provisioning: membership alone is not enough — the member's role must
//! carry the specific permission. Deny by default; non-membership and a
//! missing permission are indistinguishable to the caller (a flat 403), so
//! workspace existence never leaks.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{AppError, AppResult};

pub const CONTENT_READ: &str = "content.read";
pub const CONTENT_WRITE: &str = "content.write";
/// Secret metadata (names, scopes, audit, usage) — never values.
pub const SECRETS_READ: &str = "secrets.read";
/// Create/replace/delete secrets. Owner + admin roles only.
pub const SECRETS_MANAGE: &str = "secrets.manage";
/// Create/rename/delete environments (listing rides content.read — the
/// metadata isn't sensitive). Owner + admin roles only.
pub const ENVIRONMENTS_MANAGE: &str = "environments.manage";
/// Read the workspace activity feed (the audit_logs ledger). All roles.
pub const AUDIT_READ: &str = "audit.read";
/// Rename the workspace / manage its logo. Owner + admin roles (seeded
/// since original provisioning — no backfill needed).
pub const WORKSPACE_MANAGE: &str = "workspace.manage";

pub async fn require_permission(
    pool: &PgPool,
    user_id: Uuid,
    workspace_id: Uuid,
    permission: &str,
) -> AppResult<()> {
    let allowed: Option<(i32,)> = sqlx::query_as(
        r#"
        SELECT 1
        FROM workspace_members m
        JOIN role_permissions rp ON rp.role_id = m.role_id
        WHERE m.user_id = $1 AND m.workspace_id = $2 AND rp.permission = $3
        LIMIT 1
        "#,
    )
    .bind(user_id)
    .bind(workspace_id)
    .bind(permission)
    .fetch_optional(pool)
    .await?;

    match allowed {
        Some(_) => Ok(()),
        None => Err(AppError::Forbidden),
    }
}
