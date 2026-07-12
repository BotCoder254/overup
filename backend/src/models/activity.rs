//! Activity Feed rows and DTOs: a read-time presentation layer over the
//! immutable `audit_logs` ledger. Severity, category, and the security flag
//! are DERIVED from the action string here — never stored — so the ledger
//! itself stays append-only and schema-stable. Category keys off the action
//! prefix rather than `subject_type` because the two disagree
//! (`artifact.retention_updated` is written with subject_type `workspace`).

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

/// Every action string the platform writes to `audit_logs`. Doubles as the
/// allow-list for the feed's `action` filter — anything else is rejected
/// before SQL.
pub const ACTIONS: &[&str] = &[
    "workspace.created",
    "installation.linked",
    "installation.unlinked",
    "repository.imported",
    "repository.removed",
    "repository.synced",
    "pipeline.created",
    "pipeline.completed",
    "pipeline.cancelled",
    "job.cancelled",
    "runner.created",
    "runner.revoked",
    "runner.updated",
    "runner.token_regenerated",
    "runner.drained",
    "runner.disabled",
    "runner.resumed",
    "runner.provisioned",
    "runner.provision_failed",
    "secret.created",
    "secret.updated",
    "secret.deleted",
    "environment.created",
    "environment.updated",
    "environment.deleted",
    "artifact.uploaded",
    "artifact.downloaded",
    "artifact.deleted",
    "artifact.retention_updated",
];

/// Presentation severity for a ledger entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Success,
    Warning,
    Danger,
}

impl Severity {
    /// The wire/CSV spelling — identical to the serde `lowercase` rename.
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Success => "success",
            Severity::Warning => "warning",
            Severity::Danger => "danger",
        }
    }
}

/// Feed category derived from the action prefix.
pub fn category(action: &str) -> &'static str {
    match action.split('.').next().unwrap_or("") {
        "workspace" => "workspace",
        "installation" => "integration",
        "repository" => "repository",
        // Job-level entries roll up into the pipeline category.
        "pipeline" | "job" => "pipeline",
        "runner" => "runner",
        "secret" => "secret",
        "environment" => "environment",
        "artifact" => "artifact",
        _ => "other",
    }
}

/// Derive `(severity, category, security)` for one ledger entry. Metadata
/// only participates for `pipeline.completed`, whose outcome rides in
/// `metadata.conclusion`; every value is treated as untrusted JSON.
pub fn classify(action: &str, metadata: &serde_json::Value) -> (Severity, &'static str, bool) {
    let security = action.starts_with("secret.")
        || matches!(
            action,
            "runner.token_regenerated"
                | "runner.revoked"
                | "installation.linked"
                | "installation.unlinked"
        );

    let severity = match action {
        "workspace.created" | "installation.linked" | "repository.imported"
        | "runner.created" | "runner.provisioned" | "runner.resumed" | "secret.created"
        | "environment.created" => Severity::Success,
        "pipeline.completed" => {
            if metadata.get("conclusion").and_then(serde_json::Value::as_str) == Some("success") {
                Severity::Success
            } else {
                Severity::Danger
            }
        }
        "pipeline.cancelled" | "job.cancelled" | "runner.drained" | "runner.disabled"
        | "artifact.deleted" => Severity::Warning,
        "installation.unlinked" | "repository.removed" | "runner.revoked"
        | "runner.token_regenerated" | "runner.provision_failed" | "secret.deleted"
        | "environment.deleted" => Severity::Danger,
        _ => Severity::Info,
    };

    (severity, category(action), security)
}

/// One `audit_logs` row with the actor's public profile joined in. Secret
/// values never appear here by construction: audit metadata is written
/// value-free at every call site.
#[derive(Debug, sqlx::FromRow)]
pub struct ActivityRow {
    pub id: Uuid,
    pub action: String,
    pub subject_type: String,
    pub subject_id: Option<Uuid>,
    pub actor_user_id: Option<Uuid>,
    pub actor_login: Option<String>,
    pub actor_avatar_url: Option<String>,
    pub metadata: serde_json::Value,
    pub request_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// API shape for one feed entry. Nullable fields serialize as explicit
/// `null` (the audit-list convention) — `NULL` actor means a system action.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEventResponse {
    pub id: Uuid,
    pub action: String,
    pub category: &'static str,
    pub severity: Severity,
    pub security: bool,
    pub subject_type: String,
    pub subject_id: Option<Uuid>,
    pub actor_id: Option<Uuid>,
    pub actor_login: Option<String>,
    pub actor_avatar_url: Option<String>,
    pub metadata: serde_json::Value,
    pub request_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl From<ActivityRow> for ActivityEventResponse {
    fn from(row: ActivityRow) -> Self {
        let (severity, category, security) = classify(&row.action, &row.metadata);
        Self {
            id: row.id,
            action: row.action,
            category,
            severity,
            security,
            subject_type: row.subject_type,
            subject_id: row.subject_id,
            actor_id: row.actor_user_id,
            actor_login: row.actor_login,
            actor_avatar_url: row.actor_avatar_url,
            metadata: row.metadata,
            request_id: row.request_id,
            created_at: row.created_at,
        }
    }
}

/// One aggregate pass over the workspace's ledger slice. `total` is
/// all-time; everything else is windowed so the numbers stay meaningful.
#[derive(Debug, sqlx::FromRow)]
pub struct ActivitySummaryRow {
    pub total: i64,
    pub last_24h: i64,
    pub security_30d: i64,
    pub failures_30d: i64,
    pub cat_pipeline: i64,
    pub cat_runner: i64,
    pub cat_repository: i64,
    pub cat_artifact: i64,
    pub cat_secret: i64,
    pub cat_environment: i64,
    pub cat_integration: i64,
    pub cat_workspace: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_known_action_maps_to_a_category() {
        for action in ACTIONS {
            let (_, category, _) = classify(action, &json!({}));
            assert_ne!(category, "other", "{action} must map to a real category");
        }
    }

    #[test]
    fn pipeline_completed_severity_follows_conclusion() {
        let (ok, _, _) = classify("pipeline.completed", &json!({ "conclusion": "success" }));
        assert_eq!(ok, Severity::Success);
        let (failed, _, _) = classify("pipeline.completed", &json!({ "conclusion": "failure" }));
        assert_eq!(failed, Severity::Danger);
        // Missing or non-string conclusion is not a success.
        let (missing, _, _) = classify("pipeline.completed", &json!({}));
        assert_eq!(missing, Severity::Danger);
    }

    #[test]
    fn security_flag_covers_credential_surfaces() {
        for action in [
            "secret.created",
            "secret.updated",
            "secret.deleted",
            "runner.token_regenerated",
            "runner.revoked",
            "installation.linked",
            "installation.unlinked",
        ] {
            let (_, _, security) = classify(action, &json!({}));
            assert!(security, "{action} must carry the security flag");
        }
        let (_, _, security) = classify("pipeline.created", &json!({}));
        assert!(!security);
    }

    #[test]
    fn retention_update_is_an_artifact_event_despite_workspace_subject() {
        // Written with subject_type 'workspace' — category must come from
        // the action prefix, which is exactly why classify ignores subjects.
        assert_eq!(category("artifact.retention_updated"), "artifact");
    }

    #[test]
    fn severity_table_spot_checks() {
        assert_eq!(classify("workspace.created", &json!({})).0, Severity::Success);
        assert_eq!(classify("repository.synced", &json!({})).0, Severity::Info);
        assert_eq!(classify("runner.drained", &json!({})).0, Severity::Warning);
        assert_eq!(classify("environment.deleted", &json!({})).0, Severity::Danger);
        assert_eq!(classify("artifact.downloaded", &json!({})).0, Severity::Info);
    }
}
