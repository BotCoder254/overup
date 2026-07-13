//! Notification mapping + delivery: turns one audit-ledger entry into
//! per-recipient notification rows.
//!
//! The mapping is deliberately a filter — routine successes, downloads, and
//! heartbeat-grade traffic never notify (the Activity Feed keeps the full
//! record). Every title/body is rendered HERE from static templates plus
//! database name lookups: runner/upstream text and secret values can never
//! reach a notification by construction. Recipient selection happens at
//! write time (permission fan-out, actor suppression, per-user preferences)
//! so the read API has nothing to re-filter.

use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::db;
use crate::db::notifications::{AuditTailRow, NewNotification};
use crate::services::authz;
use crate::services::notification_hub::NotificationEvent;
use crate::state::AppState;

/// Who receives a notification.
pub enum Recipients {
    /// Every workspace member holding this permission (minus the actor).
    Permission(&'static str),
    /// Exactly one user (e.g. the pipeline's triggering actor).
    Direct(Uuid),
}

/// A fully-rendered notification ready for per-recipient insertion.
pub struct NotificationSpec {
    pub category: &'static str,
    pub severity: &'static str,
    pub recipients: Recipients,
    pub suppress_actor: bool,
    pub title: String,
    pub body: String,
    pub subject_type: Option<String>,
    pub subject_id: Option<Uuid>,
    pub link: Value,
    pub dedup_key: Option<String>,
}

/// Pure routing decision for one audit action: `(category, severity,
/// permission, actor_suppressed)`, or `None` for events that never notify.
/// `pipeline.completed` consults its conclusion. Kept free of I/O so the
/// mapping table is unit-testable end to end.
pub fn classify_event(
    action: &str,
    metadata: &Value,
) -> Option<(&'static str, &'static str, &'static str, bool)> {
    let conclusion = metadata.get("conclusion").and_then(Value::as_str);
    match action {
        "pipeline.completed" => match conclusion {
            Some("failure") | Some("timed_out") | None => {
                Some(("pipeline", "error", authz::CONTENT_READ, false))
            }
            Some("partial") => Some(("pipeline", "warning", authz::CONTENT_READ, false)),
            // Success notifies the triggering actor only (handled by the
            // enrichment step); cancelled is covered by pipeline.cancelled.
            Some("success") => Some(("pipeline", "success", authz::CONTENT_READ, false)),
            Some(_) => None,
        },
        "pipeline.cancelled" => Some(("pipeline", "warning", authz::CONTENT_READ, true)),
        "runner.offline" => Some(("runner", "error", authz::CONTENT_READ, false)),
        "runner.recovered" => Some(("runner", "success", authz::CONTENT_READ, false)),
        "runner.provision_failed" => Some(("system", "error", authz::CONTENT_WRITE, true)),
        "repository.sync_failed" => Some(("repository", "error", authz::CONTENT_READ, true)),
        "workflow.invalid" => Some(("workflow", "warning", authz::CONTENT_READ, true)),
        "artifact.uploaded" => Some(("artifact", "info", authz::CONTENT_READ, true)),
        "secret.created" | "secret.updated" => {
            Some(("security", "warning", authz::SECRETS_READ, true))
        }
        "secret.deleted" => Some(("security", "error", authz::SECRETS_READ, true)),
        "installation.linked" => Some(("security", "warning", authz::CONTENT_WRITE, true)),
        "installation.unlinked" | "runner.revoked" | "runner.token_regenerated" => {
            Some(("security", "error", authz::CONTENT_WRITE, true))
        }
        "environment.created" | "environment.updated" => {
            Some(("environment", "info", authz::CONTENT_READ, true))
        }
        "environment.deleted" => Some(("environment", "warning", authz::CONTENT_READ, true)),
        // Everything else — routine successes, downloads, lifecycle noise,
        // and the notification.* actions themselves (no feedback loop) —
        // stays in the Activity Feed only.
        _ => None,
    }
}

/// Char-boundary-safe clamp so a rendered string can never trip the
/// migration's CHECK length constraints and abort a projector batch.
fn clamp(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn meta_str<'a>(metadata: &'a Value, key: &str) -> Option<&'a str> {
    metadata.get(key).and_then(Value::as_str)
}

#[derive(sqlx::FromRow)]
struct PipelineInfo {
    number: i32,
    workflow_name: String,
    git_ref: String,
    triggered_by: Option<Uuid>,
    repository_id: Uuid,
    repo_name: String,
}

async fn pipeline_info(pool: &PgPool, pipeline_id: Uuid) -> sqlx::Result<Option<PipelineInfo>> {
    sqlx::query_as::<_, PipelineInfo>(
        r#"
        SELECT p.number, p.workflow_name, p.git_ref, p.triggered_by,
               p.repository_id, r.name AS repo_name
        FROM pipelines p
        JOIN repositories r ON r.id = p.repository_id
        WHERE p.id = $1
        "#,
    )
    .bind(pipeline_id)
    .fetch_optional(pool)
    .await
}

/// Map one ledger entry to a rendered spec, enriching with subject lookups.
/// `None` = the event doesn't notify (excluded action, or the subject row
/// vanished before the projector caught up — nothing useful to say then).
pub async fn route(pool: &PgPool, row: &AuditTailRow) -> sqlx::Result<Option<NotificationSpec>> {
    let Some(workspace_id) = row.workspace_id else {
        return Ok(None);
    };
    let Some((category, severity, permission, suppress_actor)) =
        classify_event(&row.action, &row.metadata)
    else {
        return Ok(None);
    };

    let subject_id = row.subject_id;
    let name = meta_str(&row.metadata, "name").unwrap_or("").to_string();

    let spec = match row.action.as_str() {
        "pipeline.completed" | "pipeline.cancelled" => {
            let Some(pipeline_id) = subject_id else {
                return Ok(None);
            };
            let Some(p) = pipeline_info(pool, pipeline_id).await? else {
                return Ok(None);
            };
            let conclusion = meta_str(&row.metadata, "conclusion").unwrap_or("cancelled");
            let (title, recipients, dedup_key) = if row.action == "pipeline.cancelled" {
                (
                    format!("Pipeline #{} cancelled — {}", p.number, p.repo_name),
                    Recipients::Permission(permission),
                    None,
                )
            } else {
                match conclusion {
                    "success" => {
                        // Success is personal: only the user who triggered
                        // the run cares, and push pipelines have no actor.
                        let Some(actor) = p.triggered_by else {
                            return Ok(None);
                        };
                        (
                            format!("Pipeline #{} succeeded — {}", p.number, p.repo_name),
                            Recipients::Direct(actor),
                            None,
                        )
                    }
                    "partial" => (
                        format!("Pipeline #{} partially succeeded — {}", p.number, p.repo_name),
                        Recipients::Permission(permission),
                        Some(format!(
                            "pipeline.failed:{}:{}:{}",
                            p.repository_id, p.workflow_name, p.git_ref
                        )),
                    ),
                    _ => (
                        format!("Pipeline #{} failed — {}", p.number, p.repo_name),
                        Recipients::Permission(permission),
                        Some(format!(
                            "pipeline.failed:{}:{}:{}",
                            p.repository_id, p.workflow_name, p.git_ref
                        )),
                    ),
                }
            };
            // Failure detail: the first failed job's STATIC error category
            // (step_failed, image_pull_failed, container_error, timeout, …)
            // — Docker-execution failures surface here, reported runner →
            // control plane → notification, never runner text verbatim.
            let mut body = format!("{} on {}", p.workflow_name, p.git_ref);
            if row.action == "pipeline.completed"
                && matches!(conclusion, "failure" | "timed_out")
            {
                let category_row: Option<(String,)> = sqlx::query_as(
                    r#"
                    SELECT error_category FROM pipeline_jobs
                    WHERE pipeline_id = $1 AND error_category IS NOT NULL
                    ORDER BY finished_at ASC NULLS LAST
                    LIMIT 1
                    "#,
                )
                .bind(pipeline_id)
                .fetch_optional(pool)
                .await?;
                if let Some((error_category,)) = category_row {
                    body = format!("{body} — {error_category}");
                }
            }
            NotificationSpec {
                category,
                severity,
                recipients,
                suppress_actor,
                title,
                body,
                subject_type: Some("pipeline".into()),
                subject_id: Some(pipeline_id),
                link: json!({
                    "kind": "pipeline",
                    "pipelineId": pipeline_id,
                    "repositoryId": p.repository_id,
                }),
                dedup_key,
            }
        }
        "runner.offline" => {
            // Escalate to critical when this was the workspace's last
            // connected runner: every queued pipeline is now stuck.
            let (online,): (i64,) = sqlx::query_as(
                r#"
                SELECT COUNT(*) FROM runners
                WHERE workspace_id = $1 AND status <> 'offline' AND revoked_at IS NULL
                "#,
            )
            .bind(workspace_id)
            .fetch_one(pool)
            .await?;
            let severity = if online == 0 { "critical" } else { severity };
            NotificationSpec {
                category,
                severity,
                recipients: Recipients::Permission(permission),
                suppress_actor,
                title: format!("Runner {name} went offline"),
                body: if online == 0 {
                    "No runners remain online — queued pipelines will not start.".into()
                } else {
                    String::new()
                },
                subject_type: Some("runner".into()),
                subject_id,
                link: json!({ "kind": "runners" }),
                dedup_key: subject_id.map(|id| format!("runner.offline:{id}")),
            }
        }
        "runner.recovered" => NotificationSpec {
            category,
            severity,
            recipients: Recipients::Permission(permission),
            suppress_actor,
            title: format!("Runner {name} is back online"),
            body: String::new(),
            subject_type: Some("runner".into()),
            subject_id,
            link: json!({ "kind": "runners" }),
            dedup_key: subject_id.map(|id| format!("runner.recovered:{id}")),
        },
        "runner.provision_failed" => NotificationSpec {
            category,
            severity,
            recipients: Recipients::Permission(permission),
            suppress_actor,
            title: "Hosted runner provisioning failed".into(),
            body: if name.is_empty() {
                String::new()
            } else {
                format!("Runner {name} could not be provisioned.")
            },
            subject_type: Some("runner".into()),
            subject_id,
            link: json!({ "kind": "runners" }),
            dedup_key: subject_id.map(|id| format!("runner.provision_failed:{id}")),
        },
        "repository.sync_failed" => NotificationSpec {
            category,
            severity,
            recipients: Recipients::Permission(permission),
            suppress_actor,
            title: format!("Repository sync failed — {name}"),
            // sync_error is a static category string by construction.
            body: meta_str(&row.metadata, "syncError")
                .map(|e| format!("Sync error: {e}"))
                .unwrap_or_default(),
            subject_type: Some("repository".into()),
            subject_id,
            link: json!({ "kind": "repository", "repositoryId": subject_id }),
            dedup_key: subject_id.map(|id| format!("repository.sync_failed:{id}")),
        },
        "workflow.invalid" => {
            let path = meta_str(&row.metadata, "path").unwrap_or("workflow");
            NotificationSpec {
                category,
                severity,
                recipients: Recipients::Permission(permission),
                suppress_actor,
                title: format!("Workflow validation failed — {path}"),
                body: if name.is_empty() {
                    String::new()
                } else {
                    format!("In repository {name}.")
                },
                subject_type: Some("workflow".into()),
                subject_id,
                link: json!({ "kind": "workflow", "workflowId": subject_id }),
                dedup_key: subject_id.map(|id| format!("workflow.invalid:{id}")),
            }
        }
        "artifact.uploaded" => {
            let Some(artifact_id) = subject_id else {
                return Ok(None);
            };
            // Personal event: only the triggering actor is told their
            // artifact is ready; push pipelines have nobody to tell.
            let info: Option<(Option<Uuid>, i32)> = sqlx::query_as(
                r#"
                SELECT p.triggered_by, p.number
                FROM artifacts a JOIN pipelines p ON p.id = a.pipeline_id
                WHERE a.id = $1
                "#,
            )
            .bind(artifact_id)
            .fetch_optional(pool)
            .await?;
            let Some((Some(actor), number)) = info else {
                return Ok(None);
            };
            NotificationSpec {
                category,
                severity,
                recipients: Recipients::Direct(actor),
                suppress_actor: false,
                title: format!("Artifact {name} uploaded — pipeline #{number}"),
                body: String::new(),
                subject_type: Some("artifact".into()),
                subject_id,
                link: json!({ "kind": "artifact", "artifactId": artifact_id }),
                dedup_key: None,
            }
        }
        "secret.created" | "secret.updated" | "secret.deleted" => {
            let verb = row.action.rsplit('.').next().unwrap_or("changed");
            NotificationSpec {
                category,
                severity,
                recipients: Recipients::Permission(permission),
                suppress_actor,
                title: format!("Secret {name} was {verb}"),
                body: String::new(),
                subject_type: Some("secret".into()),
                subject_id,
                link: if row.action == "secret.deleted" {
                    json!({ "kind": "secrets" })
                } else {
                    json!({ "kind": "secret", "secretId": subject_id })
                },
                dedup_key: None,
            }
        }
        "installation.linked" | "installation.unlinked" => {
            let account = meta_str(&row.metadata, "accountLogin").unwrap_or("GitHub");
            let verb = if row.action.ends_with("linked") && !row.action.ends_with("unlinked") {
                "linked"
            } else {
                "unlinked"
            };
            NotificationSpec {
                category,
                severity,
                recipients: Recipients::Permission(permission),
                suppress_actor,
                title: format!("GitHub installation for {account} was {verb}"),
                body: String::new(),
                subject_type: Some("github_installation".into()),
                subject_id,
                link: json!({ "kind": "repositories" }),
                dedup_key: None,
            }
        }
        "runner.revoked" | "runner.token_regenerated" => {
            let verb = if row.action == "runner.revoked" {
                "revoked"
            } else {
                "issued a new registration token"
            };
            NotificationSpec {
                category,
                severity,
                recipients: Recipients::Permission(permission),
                suppress_actor,
                title: format!("Runner {name} was {verb}"),
                body: String::new(),
                subject_type: Some("runner".into()),
                subject_id,
                link: json!({ "kind": "runners" }),
                dedup_key: None,
            }
        }
        "environment.created" | "environment.updated" | "environment.deleted" => {
            let verb = row.action.rsplit('.').next().unwrap_or("changed");
            NotificationSpec {
                category,
                severity,
                recipients: Recipients::Permission(permission),
                suppress_actor,
                title: format!("Environment {name} was {verb}"),
                body: String::new(),
                subject_type: Some("environment".into()),
                subject_id,
                link: if row.action == "environment.deleted" {
                    json!({ "kind": "environments" })
                } else {
                    json!({ "kind": "environment", "environmentId": subject_id })
                },
                dedup_key: None,
            }
        }
        _ => return Ok(None),
    };

    Ok(Some(spec))
}

/// Resolve a spec's recipients: permission fan-out (with actor suppression
/// and write-time preference filtering) or the single direct target (whose
/// preferences are honored via the same SQL path for consistency).
pub async fn recipients_for(
    pool: &PgPool,
    workspace_id: Uuid,
    actor: Option<Uuid>,
    spec: &NotificationSpec,
) -> sqlx::Result<Vec<Uuid>> {
    match spec.recipients {
        Recipients::Permission(permission) => {
            let exclude = if spec.suppress_actor { actor } else { None };
            db::notifications::fan_out_recipients(
                pool,
                workspace_id,
                permission,
                exclude,
                spec.category,
                spec.severity,
            )
            .await
        }
        Recipients::Direct(user_id) => {
            // Direct targets still get preference filtering: run the same
            // fan-out query and keep the row only if the target survives it.
            let all = db::notifications::fan_out_recipients(
                pool,
                workspace_id,
                authz::CONTENT_READ,
                None,
                spec.category,
                spec.severity,
            )
            .await?;
            Ok(all.into_iter().filter(|id| *id == user_id).collect())
        }
    }
}

/// Insert one notification per recipient inside the caller's transaction
/// and return the outcomes for post-commit hub publishing.
pub async fn insert_for_recipients(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    workspace_id: Uuid,
    action: &str,
    spec: &NotificationSpec,
    recipients: &[Uuid],
) -> sqlx::Result<Vec<(Uuid, db::notifications::UpsertOutcome)>> {
    let title = clamp(&spec.title, 300);
    let body = clamp(&spec.body, 1000);
    let dedup_key = spec.dedup_key.as_deref().map(|k| clamp(k, 200));

    let mut outcomes = Vec::with_capacity(recipients.len());
    for user_id in recipients {
        let outcome = db::notifications::upsert(
            &mut **tx,
            &NewNotification {
                workspace_id,
                user_id: *user_id,
                action,
                category: spec.category,
                severity: spec.severity,
                title: &title,
                body: &body,
                subject_type: spec.subject_type.as_deref(),
                subject_id: spec.subject_id,
                link: &spec.link,
                dedup_key: dedup_key.as_deref(),
            },
        )
        .await?;
        outcomes.push((*user_id, outcome));
    }
    Ok(outcomes)
}

/// Janitor entry point for derived conditions (stale secrets, expiring
/// artifacts): resolve recipients, insert on the pool (no ledger entry —
/// these are states, not events), publish to connected sockets.
pub async fn deliver_direct(
    state: &AppState,
    workspace_id: Uuid,
    action: &str,
    spec: &NotificationSpec,
) -> sqlx::Result<u64> {
    let recipients = recipients_for(&state.pool, workspace_id, None, spec).await?;
    if recipients.is_empty() {
        return Ok(0);
    }
    let mut tx = state.pool.begin().await?;
    let outcomes = insert_for_recipients(&mut tx, workspace_id, action, spec, &recipients).await?;
    tx.commit().await?;
    let delivered = outcomes.len() as u64;
    for (user_id, outcome) in outcomes {
        state.notification_hub.publish(
            user_id,
            NotificationEvent::Notification {
                inserted: outcome.inserted,
                notification: Box::new(outcome.row.into()),
            },
        );
    }
    Ok(delivered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Actions that must never generate a notification.
    const EXCLUDED: &[&str] = &[
        "workspace.created",
        "repository.imported",
        "repository.removed",
        "repository.synced",
        "pipeline.created",
        "job.cancelled",
        "runner.created",
        "runner.updated",
        "runner.drained",
        "runner.disabled",
        "runner.resumed",
        "runner.provisioned",
        "artifact.downloaded",
        "artifact.deleted",
        "artifact.retention_updated",
        "notification.read_all",
        "notification.bulk_archived",
        "notification.preferences_updated",
    ];

    #[test]
    fn excluded_actions_never_notify() {
        for action in EXCLUDED {
            assert!(
                classify_event(action, &json!({})).is_none(),
                "{action} must not notify"
            );
        }
    }

    #[test]
    fn every_notifiable_action_uses_known_category_and_severity() {
        use crate::models::notification::{CATEGORIES, SEVERITIES};
        for action in crate::models::activity::ACTIONS {
            for conclusion in ["success", "failure", "timed_out", "partial", "cancelled"] {
                let meta = json!({ "conclusion": conclusion });
                if let Some((category, severity, _, _)) = classify_event(action, &meta) {
                    assert!(CATEGORIES.contains(&category), "{action}: bad {category}");
                    assert!(SEVERITIES.contains(&severity), "{action}: bad {severity}");
                }
            }
        }
    }

    #[test]
    fn pipeline_severity_follows_conclusion() {
        let failed = classify_event("pipeline.completed", &json!({"conclusion": "failure"}));
        assert_eq!(failed.unwrap().1, "error");
        let timed = classify_event("pipeline.completed", &json!({"conclusion": "timed_out"}));
        assert_eq!(timed.unwrap().1, "error");
        let partial = classify_event("pipeline.completed", &json!({"conclusion": "partial"}));
        assert_eq!(partial.unwrap().1, "warning");
        let success = classify_event("pipeline.completed", &json!({"conclusion": "success"}));
        assert_eq!(success.unwrap().1, "success");
        // Cancelled conclusion is covered by the pipeline.cancelled action —
        // notifying both would double-alert every member.
        assert!(classify_event("pipeline.completed", &json!({"conclusion": "cancelled"})).is_none());
    }

    #[test]
    fn security_events_gate_on_the_right_permissions() {
        let (cat, _, perm, suppressed) = classify_event("secret.deleted", &json!({})).unwrap();
        assert_eq!(cat, "security");
        assert_eq!(perm, authz::SECRETS_READ);
        assert!(suppressed, "the actor already knows what they did");

        let (_, _, perm, _) = classify_event("runner.revoked", &json!({})).unwrap();
        assert_eq!(perm, authz::CONTENT_WRITE);
    }

    #[test]
    fn janitor_delivered_kinds_never_ride_the_projector() {
        // These insert via deliver_direct from the janitor's scans; if they
        // ever routed through classify_event too, one condition could
        // notify twice through two paths.
        for kind in [
            "secret.stale",
            "artifact.expiring",
            "queue.congested",
            "runner.bootstrap_expired",
        ] {
            assert!(
                classify_event(kind, &json!({})).is_none(),
                "{kind} must be janitor-delivered only"
            );
        }
    }

    #[test]
    fn clamp_is_char_boundary_safe() {
        assert_eq!(clamp("short", 300), "short");
        let long = "é".repeat(400);
        let clamped = clamp(&long, 300);
        assert!(clamped.chars().count() <= 300);
        assert!(clamped.ends_with('…'));
    }
}
