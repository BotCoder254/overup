//! Secrets Management API: write-only encrypted credentials. Values arrive
//! once over TLS, are validated, envelope-encrypted (services::
//! secrets_crypto), and stored as ciphertext — there is deliberately NO
//! endpoint that returns a value, and no error, trace, or audit entry ever
//! contains one. Metadata reads need `secrets.read`; every mutation needs
//! `secrets.manage` and the configured master key (clean denial without it,
//! the R2 pattern).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::middleware::auth::CurrentUser;
use crate::models::secret::SecretResponse;
use crate::services::authz;
use crate::services::secrets_crypto::SecretsCrypto;
use crate::services::workspace_hub::WorkspaceEvent;
use crate::state::AppState;

use super::pipelines::{escape_like, format_cursor, parse_cursor};

const DEFAULT_PAGE: i64 = 25;
const NAME_MAX: usize = 200;
/// Above the log hub's MIN_MASK_LEN (6): every stored value is maskable.
const VALUE_MIN: usize = 8;
/// Fits comfortably inside the 64 KB browser body budget with JSON overhead.
const VALUE_MAX: usize = 32 * 1024;
const DESCRIPTION_MAX: usize = 500;

/// Names a secret may not take: prefixes the platform owns (a secret could
/// otherwise clobber runner- or checkout-critical environment) ...
const RESERVED_PREFIXES: &[&str] = &["OVERUP_", "GITHUB_", "RUNNER_", "DOCKER_"];
/// ... and well-known environment variables that alter process behavior.
const RESERVED_NAMES: &[&str] = &[
    "CI",
    "PATH",
    "HOME",
    "SHELL",
    "HOSTNAME",
    "LANG",
    "PWD",
    "USER",
    "TMPDIR",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
];

/// Normalize and validate a secret name. NFC first so visually identical
/// Unicode input can't dodge the ASCII allow-list, then strict
/// `^[A-Z_][A-Z0-9_]*$`. Lowercase is rejected (not silently folded) so
/// the name shown is always exactly the name stored.
fn validate_name(raw: &str) -> AppResult<String> {
    let name: String = raw.trim().nfc().collect();
    if name.is_empty() {
        return Err(AppError::Validation("secret name is required".into()));
    }
    if name.len() > NAME_MAX {
        return Err(AppError::Validation(format!(
            "secret name must be at most {NAME_MAX} characters"
        )));
    }
    let mut chars = name.chars();
    let first_ok = matches!(chars.next(), Some(c) if c.is_ascii_uppercase() || c == '_');
    let rest_ok = chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    if !first_ok || !rest_ok {
        return Err(AppError::Validation(
            "secret names are UPPER_SNAKE_CASE: letters A-Z, digits, and underscores, \
             not starting with a digit"
                .into(),
        ));
    }
    if let Some(prefix) = RESERVED_PREFIXES.iter().find(|p| name.starts_with(*p)) {
        return Err(AppError::Validation(format!(
            "the {prefix} prefix is reserved for the platform"
        )));
    }
    if RESERVED_NAMES.contains(&name.as_str()) {
        return Err(AppError::Validation(format!(
            "{name} is a reserved environment variable name"
        )));
    }
    Ok(name)
}

/// Validate a secret value without ever echoing it back. Length bounds and
/// no NUL (it becomes a container environment variable); UTF-8 is already
/// guaranteed by JSON deserialization.
fn validate_value(value: &str) -> AppResult<()> {
    if value.len() < VALUE_MIN {
        return Err(AppError::Validation(format!(
            "secret values must be at least {VALUE_MIN} bytes"
        )));
    }
    if value.len() > VALUE_MAX {
        return Err(AppError::Validation(format!(
            "secret values must be at most {VALUE_MAX} bytes"
        )));
    }
    if value.contains('\0') {
        return Err(AppError::Validation(
            "secret values must not contain NUL bytes".into(),
        ));
    }
    Ok(())
}

fn validate_description(raw: Option<&str>) -> AppResult<Option<String>> {
    match raw.map(str::trim) {
        None | Some("") => Ok(None),
        Some(d) if d.len() <= DESCRIPTION_MAX => Ok(Some(d.to_string())),
        Some(_) => Err(AppError::Validation(format!(
            "description must be at most {DESCRIPTION_MAX} characters"
        ))),
    }
}

/// Mutations require the envelope-encryption engine; without
/// SECRETS_MASTER_KEY the feature is cleanly denied (the R2 pattern).
fn require_crypto(state: &AppState) -> AppResult<&SecretsCrypto> {
    state
        .secrets_crypto
        .as_deref()
        .ok_or_else(|| {
            AppError::Validation(
                "secrets storage is not configured on this deployment (set SECRETS_MASTER_KEY)"
                    .into(),
            )
        })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogQuery {
    q: Option<String>,
    scope: Option<String>,
    repository_id: Option<Uuid>,
    environment_id: Option<Uuid>,
    cursor: Option<String>,
    limit: Option<i64>,
}

/// GET /api/workspaces/{workspace_id}/secrets
pub async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
    Query(query): Query<CatalogQuery>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::SECRETS_READ).await?;

    let scope = match query.scope.as_deref() {
        None | Some("") => None,
        Some(s @ ("workspace" | "repository" | "environment")) => Some(s.to_string()),
        Some(_) => return Err(AppError::Validation("invalid scope filter".into())),
    };
    let search_pattern = match query.q.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(q) if q.len() <= 200 => Some(format!("%{}%", escape_like(q))),
        Some(_) => return Err(AppError::Validation("search query too long".into())),
    };
    let cursor = match &query.cursor {
        None => None,
        Some(raw) => Some(parse_cursor(raw)?),
    };
    let limit = query.limit.unwrap_or(DEFAULT_PAGE).clamp(1, 50);

    let filter = db::secrets::CatalogFilter {
        scope,
        repository_id: query.repository_id,
        environment_id: query.environment_id,
        search_pattern,
        cursor,
        limit,
    };
    let rows = db::secrets::list_catalog(&state.pool, workspace_id, &filter).await?;
    let next_cursor = (rows.len() as i64 == limit)
        .then(|| rows.last())
        .flatten()
        .map(|row| format_cursor(row.created_at, row.id));
    let secrets: Vec<SecretResponse> = rows.into_iter().map(SecretResponse::from).collect();

    Ok(Json(json!({ "secrets": secrets, "nextCursor": next_cursor })))
}

/// GET /api/workspaces/{workspace_id}/secrets/summary
pub async fn summary(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::SECRETS_READ).await?;

    let summary = db::secrets::summary(&state.pool, workspace_id).await?;
    Ok(Json(json!({
        "total": summary.total,
        "workspaceScoped": summary.workspace_scoped,
        "repositoryScoped": summary.repository_scoped,
        "environmentScoped": summary.environment_scoped,
        "usedLast30d": summary.used_last_30d,
        "neverUsed": summary.never_used,
        "createdLast30d": summary.created_last_30d,
        "distinctRepositories": summary.distinct_repositories,
        "totalInjections": summary.total_injections,
        // Values not rotated within the stale window — rotation nudge.
        "stale": summary.stale,
        "staleAfterDays": db::secrets::SECRET_STALE_DAYS,
        // The UI's posture card states plainly whether values can be stored.
        "encryptionConfigured": state.secrets_crypto.is_some(),
    })))
}

/// A well-formed `${{ … }}` reference identifier (`[A-Za-z_][A-Za-z0-9_]*`,
/// ≤200 bytes). The parser only writes such names, but requirements are
/// re-filtered at read time so legacy or hand-edited metadata can never
/// smuggle arbitrary strings into a response.
pub(crate) fn valid_ref_ident(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && name.len() <= 200
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Fold requirement rows into per-(repository, name) entries — the shape the
/// repo-grouped detection cards render directly. Sorted by repository then
/// name (BTreeMap key), capped at 200 entries and 20 workflow references
/// each with `referenceCount` carrying the true total. `keep` is the
/// read-time charset allow-list. `configuredId` is the covering secret's or
/// matching environment's id (null when missing). Names and ids only, never
/// values.
pub(crate) fn group_requirements(
    rows: Vec<db::workflows::RequirementRefRow>,
    keep: impl Fn(&str) -> bool,
) -> Vec<serde_json::Value> {
    const MAX_ENTRIES: usize = 200;
    const MAX_REFS_PER_ENTRY: usize = 20;

    struct Entry {
        repository_id: Uuid,
        configured_id: Option<Uuid>,
        count: i64,
        references: Vec<serde_json::Value>,
    }
    let mut grouped: std::collections::BTreeMap<(String, String), Entry> =
        std::collections::BTreeMap::new();
    for row in rows {
        if !keep(&row.name) {
            continue;
        }
        let key = (row.repository_name.clone(), row.name.clone());
        if !grouped.contains_key(&key) && grouped.len() >= MAX_ENTRIES {
            continue;
        }
        let entry = grouped.entry(key).or_insert_with(|| Entry {
            repository_id: row.repository_id,
            configured_id: None,
            count: 0,
            references: Vec::new(),
        });
        entry.count += 1;
        if entry.configured_id.is_none() {
            entry.configured_id = row.configured_id;
        }
        if entry.references.len() < MAX_REFS_PER_ENTRY {
            entry.references.push(json!({
                "workflowId": row.workflow_id,
                "workflowPath": row.workflow_path,
            }));
        }
    }
    grouped
        .into_iter()
        .map(|((repository_name, name), entry)| {
            json!({
                "name": name,
                "repositoryId": entry.repository_id,
                "repositoryName": repository_name,
                "configured": entry.configured_id.is_some(),
                "configuredId": entry.configured_id,
                "referenceCount": entry.count,
                "references": entry.references,
            })
        })
        .collect()
}

/// GET /api/workspaces/{workspace_id}/secrets/requirements
///
/// Workflow-declared requirements detected at sync time: every secret name
/// the YAML references (`${{ secrets.X }}`) with its configured state and,
/// when satisfied, the id of the covering secret; plus `${{ vars.X }}`
/// references (informational — the platform doesn't manage plain
/// variables). The charset filter is deliberately loose (`valid_ref_ident`,
/// not the configurable-name rule) so reserved-prefix refs like
/// DOCKER_TOKEN stay VISIBLE — the client marks them unconfigurable instead
/// of silently hiding them. Names and ids only, never values.
pub async fn requirements(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::SECRETS_READ).await?;

    let refs = db::workflows::secret_ref_states(&state.pool, workspace_id).await?;
    let vars = db::workflows::workspace_var_refs(&state.pool, workspace_id).await?;

    Ok(Json(json!({
        "secrets": group_requirements(refs, valid_ref_ident),
        "vars": group_requirements(vars, valid_ref_ident),
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditQuery {
    limit: Option<i64>,
}

fn audit_events(rows: Vec<db::secrets::SecretAuditRow>) -> Vec<serde_json::Value> {
    rows.into_iter()
        .map(|row| {
            json!({
                "action": row.action,
                "actorLogin": row.actor_login,
                "subjectId": row.subject_id,
                "metadata": row.metadata,
                "createdAt": row.created_at,
            })
        })
        .collect()
}

/// GET /api/workspaces/{workspace_id}/secrets/audit
pub async fn audit(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
    Query(query): Query<AuditQuery>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::SECRETS_READ).await?;

    let limit = query.limit.unwrap_or(20).clamp(1, 50);
    let rows = db::secrets::list_audit(&state.pool, workspace_id, None, limit).await?;
    Ok(Json(json!({ "events": audit_events(rows) })))
}

/// GET /api/workspaces/{workspace_id}/secrets/{secret_id}
pub async fn detail(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, secret_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::SECRETS_READ).await?;

    let meta = db::secrets::find_meta(&state.pool, workspace_id, secret_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let audit = db::secrets::list_audit(&state.pool, workspace_id, Some(secret_id), 20).await?;
    Ok(Json(json!({
        "secret": SecretResponse::from(meta),
        "audit": audit_events(audit),
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBody {
    name: String,
    // Wrapped in Zeroizing immediately after validation; never logged,
    // never echoed. (No Debug derive on this struct — the value must not
    // appear in any formatting path.)
    value: String,
    description: Option<String>,
    repository_id: Option<Uuid>,
    environment_id: Option<Uuid>,
}

/// POST /api/workspaces/{workspace_id}/secrets
pub async fn create(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
    headers: HeaderMap,
    Json(mut body): Json<CreateBody>,
) -> AppResult<Response> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::SECRETS_MANAGE).await?;
    let crypto = require_crypto(&state)?;

    // Take ownership of the plaintext into a self-wiping buffer before
    // anything else can fail and drop it unwiped.
    let value = Zeroizing::new(std::mem::take(&mut body.value));

    let name = validate_name(&body.name)?;
    validate_value(&value)?;
    let description = validate_description(body.description.as_deref())?;

    // Scopes are mutually exclusive: a secret is workspace-, repository-,
    // OR environment-scoped (matches the DB CHECK constraint).
    if body.repository_id.is_some() && body.environment_id.is_some() {
        return Err(AppError::Validation(
            "a secret is scoped to a repository or an environment, not both".into(),
        ));
    }

    // Repository scope must point at a repository inside this workspace.
    if let Some(repository_id) = body.repository_id
        && db::repositories::find_for_workspace(&state.pool, workspace_id, repository_id)
            .await?
            .is_none()
    {
        return Err(AppError::Validation(
            "repository does not belong to this workspace".into(),
        ));
    }

    // Environment scope: same workspace-membership check, same flat error
    // shape — existence in another workspace must not be distinguishable.
    if let Some(environment_id) = body.environment_id
        && db::environments::find_meta(&state.pool, workspace_id, environment_id)
            .await?
            .is_none()
    {
        return Err(AppError::Validation(
            "environment does not belong to this workspace".into(),
        ));
    }

    // The id doubles as AEAD associated data, binding the ciphertext to
    // exactly this row.
    let id = Uuid::new_v4();
    let enc = crypto.encrypt(value.as_bytes(), id.as_bytes())?;
    drop(value);

    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    let outcome = db::secrets::insert(
        &state.pool,
        id,
        workspace_id,
        body.repository_id,
        body.environment_id,
        &name,
        description.as_deref(),
        &enc,
        user.id,
        request_id,
    )
    .await?;

    match outcome {
        db::secrets::InsertOutcome::Created(meta) => {
            tracing::info!(%workspace_id, secret_id = %id, "secret created");
            state.workspace_hub.publish(
                workspace_id,
                WorkspaceEvent::ActivityUpdate { category: "secret".into() },
            );
            Ok((
                StatusCode::CREATED,
                Json(json!({ "secret": SecretResponse::from(*meta) })),
            )
                .into_response())
        }
        db::secrets::InsertOutcome::DuplicateName => Err(AppError::Conflict(
            "a secret with this name already exists in this scope",
        )),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceValueBody {
    // Same handling as CreateBody::value — no Debug derive.
    value: String,
}

/// PUT /api/workspaces/{workspace_id}/secrets/{secret_id}/value
///
/// Replaces the encrypted value wholesale. The previous value is not — and
/// cannot be — retrieved; name and scope are immutable.
pub async fn replace_value(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, secret_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    Json(mut body): Json<ReplaceValueBody>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::SECRETS_MANAGE).await?;
    let crypto = require_crypto(&state)?;

    let value = Zeroizing::new(std::mem::take(&mut body.value));
    validate_value(&value)?;

    let enc = crypto.encrypt(value.as_bytes(), secret_id.as_bytes())?;
    drop(value);

    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    if !db::secrets::replace_value(&state.pool, workspace_id, secret_id, &enc, user.id, request_id)
        .await?
    {
        return Err(AppError::NotFound);
    }

    let meta = db::secrets::find_meta(&state.pool, workspace_id, secret_id)
        .await?
        .ok_or(AppError::NotFound)?;
    tracing::info!(%workspace_id, %secret_id, "secret value replaced");
    state.workspace_hub.publish(
        workspace_id,
        WorkspaceEvent::ActivityUpdate { category: "secret".into() },
    );
    Ok(Json(json!({ "secret": SecretResponse::from(meta) })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateBody {
    description: Option<String>,
}

/// PATCH /api/workspaces/{workspace_id}/secrets/{secret_id}
///
/// Metadata-only update (description). The value has its own endpoint so
/// its handling stays a narrow, auditable path.
pub async fn update(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, secret_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    Json(body): Json<UpdateBody>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::SECRETS_MANAGE).await?;

    let description = validate_description(body.description.as_deref())?;
    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    if !db::secrets::update_description(
        &state.pool,
        workspace_id,
        secret_id,
        description.as_deref(),
        user.id,
        request_id,
    )
    .await?
    {
        return Err(AppError::NotFound);
    }

    let meta = db::secrets::find_meta(&state.pool, workspace_id, secret_id)
        .await?
        .ok_or(AppError::NotFound)?;
    state.workspace_hub.publish(
        workspace_id,
        WorkspaceEvent::ActivityUpdate { category: "secret".into() },
    );
    Ok(Json(json!({ "secret": SecretResponse::from(meta) })))
}

/// DELETE /api/workspaces/{workspace_id}/secrets/{secret_id}
pub async fn remove(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, secret_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> AppResult<Response> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::SECRETS_MANAGE).await?;

    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    if db::secrets::delete(&state.pool, workspace_id, secret_id, user.id, request_id)
        .await?
        .is_none()
    {
        return Err(AppError::NotFound);
    }

    tracing::info!(%workspace_id, %secret_id, "secret deleted");
    state.workspace_hub.publish(
        workspace_id,
        WorkspaceEvent::ActivityUpdate { category: "secret".into() },
    );
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_accepts_upper_snake_case() {
        assert_eq!(validate_name("DEPLOY_TOKEN").unwrap(), "DEPLOY_TOKEN");
        assert_eq!(validate_name("_INTERNAL").unwrap(), "_INTERNAL");
        assert_eq!(validate_name("A1_B2").unwrap(), "A1_B2");
        assert_eq!(validate_name("  TRIMMED  ").unwrap(), "TRIMMED");
    }

    #[test]
    fn name_rejects_lowercase_and_bad_shapes() {
        assert!(validate_name("deploy_token").is_err());
        assert!(validate_name("Deploy_Token").is_err());
        assert!(validate_name("1STARTS_WITH_DIGIT").is_err());
        assert!(validate_name("HAS-DASH").is_err());
        assert!(validate_name("HAS SPACE").is_err());
        assert!(validate_name("").is_err());
        assert!(validate_name(&"A".repeat(201)).is_err());
    }

    #[test]
    fn name_rejects_unicode_confusables() {
        // Cyrillic А (U+0410) survives NFC but is not ASCII A.
        assert!(validate_name("\u{0410}TOKEN").is_err());
        // Fullwidth latin letters normalize away from ASCII too.
        assert!(validate_name("ＴOKEN").is_err());
    }

    #[test]
    fn name_rejects_reserved() {
        assert!(validate_name("OVERUP_ANYTHING").is_err());
        assert!(validate_name("GITHUB_TOKEN").is_err());
        assert!(validate_name("RUNNER_LABELS").is_err());
        assert!(validate_name("DOCKER_HOST").is_err());
        assert!(validate_name("PATH").is_err());
        assert!(validate_name("LD_PRELOAD").is_err());
        // ...but names merely containing a reserved word are fine.
        assert!(validate_name("MY_GITHUB_ISH").is_ok());
        assert!(validate_name("APP_PATH").is_ok());
    }

    #[test]
    fn value_bounds() {
        assert!(validate_value("1234567").is_err()); // 7 bytes: below minimum
        assert!(validate_value("12345678").is_ok()); // exactly the minimum
        assert!(validate_value(&"x".repeat(VALUE_MAX)).is_ok());
        assert!(validate_value(&"x".repeat(VALUE_MAX + 1)).is_err());
        assert!(validate_value("with\0nul-byte").is_err());
    }

    #[test]
    fn description_bounds() {
        assert_eq!(validate_description(None).unwrap(), None);
        assert_eq!(validate_description(Some("  ")).unwrap(), None);
        assert_eq!(
            validate_description(Some(" ok ")).unwrap().as_deref(),
            Some("ok")
        );
        assert!(validate_description(Some(&"d".repeat(501))).is_err());
    }
}
