use std::collections::HashMap;

use sqlx::PgPool;
use uuid::Uuid;

use crate::models::workspace::{Workspace, WorkspaceSummary};

/// Result of one provisioning attempt. Both non-`Created` outcomes are
/// detected via named unique constraints so races lose cleanly.
pub enum ProvisionOutcome {
    Created(Workspace),
    SlugTaken,
    AlreadyMember,
}

const OWNER_PERMISSIONS: &[&str] = &[
    "workspace.manage",
    "workspace.delete",
    "members.invite",
    "members.remove",
    "members.manage_roles",
    "settings.manage",
    "audit.read",
    "content.read",
    "content.write",
];

const ADMIN_PERMISSIONS: &[&str] = &[
    "workspace.manage",
    "members.invite",
    "members.remove",
    "settings.manage",
    "audit.read",
    "content.read",
    "content.write",
];

const MEMBER_PERMISSIONS: &[&str] = &["content.read", "content.write"];

fn is_unique_violation(err: &sqlx::Error, constraint: &str) -> bool {
    matches!(
        err,
        sqlx::Error::Database(db)
            if db.is_unique_violation() && db.constraint() == Some(constraint)
    )
}

/// Atomically provision a workspace and everything the spec requires:
/// the workspace row, default roles with their permission mappings, the
/// owner membership, default settings, an audit entry, and the user's
/// onboarding completion — all or nothing. Dropping the transaction on
/// any early return rolls back automatically, so partial provisioning
/// cannot exist.
pub async fn provision(
    pool: &PgPool,
    user_id: Uuid,
    name: &str,
    description: Option<&str>,
    slug: &str,
    request_id: Option<&str>,
) -> sqlx::Result<ProvisionOutcome> {
    let mut tx = pool.begin().await?;

    let workspace = match sqlx::query_as::<_, Workspace>(
        r#"
        INSERT INTO workspaces (name, slug, description, created_by)
        VALUES ($1, $2, $3, $4)
        RETURNING *
        "#,
    )
    .bind(name)
    .bind(slug)
    .bind(description)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(workspace) => workspace,
        Err(err) if is_unique_violation(&err, "workspaces_slug_key") => {
            return Ok(ProvisionOutcome::SlugTaken);
        }
        Err(err) => return Err(err),
    };

    let mut role_ids: HashMap<&str, Uuid> = HashMap::new();
    for (key, display) in [("owner", "Owner"), ("admin", "Admin"), ("member", "Member")] {
        let (role_id,): (Uuid,) = sqlx::query_as(
            r#"
            INSERT INTO workspace_roles (workspace_id, key, name, is_system)
            VALUES ($1, $2, $3, true)
            RETURNING id
            "#,
        )
        .bind(workspace.id)
        .bind(key)
        .bind(display)
        .fetch_one(&mut *tx)
        .await?;
        role_ids.insert(key, role_id);
    }

    for (key, permissions) in [
        ("owner", OWNER_PERMISSIONS),
        ("admin", ADMIN_PERMISSIONS),
        ("member", MEMBER_PERMISSIONS),
    ] {
        sqlx::query(
            "INSERT INTO role_permissions (role_id, permission) SELECT $1, unnest($2::text[])",
        )
        .bind(role_ids[key])
        .bind(permissions)
        .execute(&mut *tx)
        .await?;
    }

    match sqlx::query(
        "INSERT INTO workspace_members (workspace_id, user_id, role_id) VALUES ($1, $2, $3)",
    )
    .bind(workspace.id)
    .bind(user_id)
    .bind(role_ids["owner"])
    .execute(&mut *tx)
    .await
    {
        Ok(_) => {}
        // Deny-by-default holds even when two requests race past the
        // handler's fast-path check: the unique index is the authority.
        Err(err) if is_unique_violation(&err, "workspace_members_user_id_key") => {
            return Ok(ProvisionOutcome::AlreadyMember);
        }
        Err(err) => return Err(err),
    }

    sqlx::query("INSERT INTO workspace_settings (workspace_id) VALUES ($1)")
        .bind(workspace.id)
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        r#"
        INSERT INTO audit_logs
            (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
        VALUES ($1, $2, 'workspace.created', 'workspace', $1, $3, $4)
        "#,
    )
    .bind(workspace.id)
    .bind(user_id)
    .bind(serde_json::json!({ "name": name, "slug": slug }))
    .bind(request_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        UPDATE users
        SET onboarded_at = COALESCE(onboarded_at, now()), updated_at = now()
        WHERE id = $1
        "#,
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(ProvisionOutcome::Created(workspace))
}

/// The workspace the user belongs to, if any. Membership — not slug — is
/// what authorization keys off; the summary exists for routing only.
pub async fn find_summary_for_user(
    pool: &PgPool,
    user_id: Uuid,
) -> sqlx::Result<Option<WorkspaceSummary>> {
    sqlx::query_as::<_, WorkspaceSummary>(
        r#"
        SELECT w.id, w.name, w.slug
        FROM workspaces w
        JOIN workspace_members m ON m.workspace_id = w.id
        WHERE m.user_id = $1
        "#,
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
}
