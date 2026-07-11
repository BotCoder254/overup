//! Per-kind artifact retention policies. Resolution order at upload time:
//! exact-kind row -> the workspace 'default' row -> the global
//! ARTIFACT_RETENTION_DAYS env fallback (applied by the caller). The
//! computed expires_at is stored immutably on the artifact row.

use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct RetentionPolicy {
    pub kind: String,
    pub retention_days: i32,
}

pub async fn list_for_workspace(
    pool: &PgPool,
    workspace_id: Uuid,
) -> sqlx::Result<Vec<RetentionPolicy>> {
    sqlx::query_as::<_, RetentionPolicy>(
        r#"
        SELECT kind, retention_days
        FROM artifact_retention_policies
        WHERE workspace_id = $1
        ORDER BY (kind = 'default') DESC, kind
        "#,
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await
}

/// Replace the workspace's whole policy set and record an audit entry in
/// one transaction. The caller has already validated kinds and day ranges.
pub async fn replace_for_workspace(
    pool: &PgPool,
    workspace_id: Uuid,
    policies: &[(String, i32)],
    actor: Uuid,
    request_id: Option<&str>,
) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM artifact_retention_policies WHERE workspace_id = $1")
        .bind(workspace_id)
        .execute(&mut *tx)
        .await?;

    for (kind, days) in policies {
        sqlx::query(
            r#"
            INSERT INTO artifact_retention_policies (workspace_id, kind, retention_days)
            VALUES ($1, $2, $3)
            "#,
        )
        .bind(workspace_id)
        .bind(kind)
        .bind(days)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query(
        r#"
        INSERT INTO audit_logs
            (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
        VALUES ($1, $2, 'artifact.retention_updated', 'workspace', $1, $3, $4)
        "#,
    )
    .bind(workspace_id)
    .bind(actor)
    .bind(serde_json::json!({
        "policies": policies
            .iter()
            .map(|(kind, days)| serde_json::json!({ "kind": kind, "retentionDays": days }))
            .collect::<Vec<_>>(),
    }))
    .bind(request_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Resolve the retention window for one artifact kind. `None` means no
/// workspace policy applies and the caller falls back to the env default.
pub async fn resolve_days(
    pool: &PgPool,
    workspace_id: Uuid,
    kind: &str,
) -> sqlx::Result<Option<i32>> {
    let row: Option<(i32,)> = sqlx::query_as(
        r#"
        SELECT retention_days
        FROM artifact_retention_policies
        WHERE workspace_id = $1 AND kind IN ($2, 'default')
        ORDER BY (kind = $2) DESC
        LIMIT 1
        "#,
    )
    .bind(workspace_id)
    .bind(kind)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(days,)| days))
}
