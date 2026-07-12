//! Global Search API: one authenticated, permission-aware endpoint over the
//! `search_documents` index (see `services/search_indexer.rs` for how the
//! index is maintained). Every input is validated before SQL — the tsquery
//! is built from quoted prefix lexemes and bound as a single parameter, so
//! user text can never be parsed as tsquery syntax, and the ILIKE boost
//! pattern rides the existing `escape_like` helper. Results are filtered by
//! the caller's actual permission set BEFORE ranking, so objects a member
//! cannot see never influence scores, counts, or suggestions.

use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::middleware::auth::CurrentUser;
use crate::models::search::SearchResultResponse;
use crate::services::authz;
use crate::state::AppState;

use super::pipelines::escape_like;

const DEFAULT_PAGE: i64 = 20;
/// Palette mode returns the top N per category.
const GROUP_PER_CATEGORY: i64 = 5;
const MAX_QUERY_CHARS: usize = 200;
const MAX_TOKENS: usize = 8;

/// The indexed entity types — the category filter allow-list.
const CATEGORIES: &[&str] = &[
    "repository",
    "workflow",
    "pipeline",
    "runner",
    "artifact",
    "environment",
    "secret",
    "activity",
];

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    q: Option<String>,
    category: Option<String>,
    cursor: Option<String>,
    limit: Option<i64>,
    group: Option<bool>,
}

/// GET /api/workspaces/{workspace_id}/search
pub async fn query(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
    Query(query): Query<SearchQuery>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    let group = query.group.unwrap_or(false);
    if group && (query.cursor.is_some() || query.category.is_some()) {
        return Err(AppError::Validation(
            "group mode does not accept cursor or category".into(),
        ));
    }

    let category = match query.category.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(c) if CATEGORIES.contains(&c) => Some(c.to_string()),
        Some(_) => return Err(AppError::Validation("invalid category filter".into())),
    };

    let empty = || {
        Json(json!({ "results": [], "counts": {}, "nextCursor": null }))
    };
    let Some((tsquery, normalized)) = build_tsquery(query.q.as_deref().unwrap_or(""))? else {
        return Ok(empty());
    };

    // The caller's live permission set gates which document categories can
    // match at all (content.read is guaranteed by the endpoint gate above).
    let permissions = db::search::caller_permissions(&state.pool, user.id, workspace_id).await?;
    if permissions.is_empty() {
        return Ok(empty());
    }

    let cursor = match &query.cursor {
        None => None,
        Some(raw) => Some(parse_score_cursor(raw)?),
    };
    let limit = query.limit.unwrap_or(DEFAULT_PAGE).clamp(1, 50);

    let filter = db::search::SearchFilter {
        tsquery,
        prefix_pattern: format!("{}%", escape_like(&normalized)),
        raw_query: normalized,
        permissions,
        category,
        cursor,
        limit,
    };

    if group {
        let rows =
            db::search::query_grouped(&state.pool, workspace_id, &filter, GROUP_PER_CATEGORY)
                .await?;
        let counts = fetch_counts(&state, workspace_id, &filter).await?;
        let results: Vec<SearchResultResponse> =
            rows.into_iter().map(SearchResultResponse::from).collect();
        return Ok(Json(json!({ "results": results, "counts": counts, "nextCursor": null })));
    }

    let rows = db::search::query_flat(&state.pool, workspace_id, &filter).await?;
    let next_cursor = (rows.len() as i64 == limit)
        .then(|| rows.last())
        .flatten()
        .map(|row| format_score_cursor(row.score, row.source_updated_at, row.id));
    // Counts render the category tabs once — first page only, so pagination
    // doesn't churn them.
    let counts = if filter.cursor.is_none() {
        fetch_counts(&state, workspace_id, &filter).await?
    } else {
        serde_json::Value::Null
    };
    let results: Vec<SearchResultResponse> =
        rows.into_iter().map(SearchResultResponse::from).collect();

    Ok(Json(json!({ "results": results, "counts": counts, "nextCursor": next_cursor })))
}

async fn fetch_counts(
    state: &AppState,
    workspace_id: Uuid,
    filter: &db::search::SearchFilter,
) -> AppResult<serde_json::Value> {
    let rows = db::search::counts(&state.pool, workspace_id, filter).await?;
    let mut map = serde_json::Map::new();
    for row in rows {
        map.insert(row.entity_type, json!(row.total));
    }
    Ok(serde_json::Value::Object(map))
}

/// Turn raw user text into a safe tsquery string: NFC-normalize, strip
/// control characters, cap length, tokenize on whitespace, and emit each
/// token as a QUOTED prefix lexeme (`'tok':*`) joined with `&`. Quoting
/// (`'` → `''`) means tsquery operators (`& | ! ( ) : <->`) in user text are
/// matched literally, never parsed — and the result is still passed as one
/// bind parameter, so nothing is ever interpolated into SQL either.
///
/// Returns `None` for a query with no searchable tokens (the handler
/// responds with empty results, not an error) alongside the normalized text
/// used for the exact/prefix ranking boosts.
fn build_tsquery(raw: &str) -> AppResult<Option<(String, String)>> {
    let normalized: String = raw.trim().nfc().filter(|c| !c.is_control()).collect();
    if normalized.chars().count() > MAX_QUERY_CHARS {
        return Err(AppError::Validation("search query too long".into()));
    }
    let tokens: Vec<String> = normalized
        .split_whitespace()
        .filter(|t| t.chars().any(char::is_alphanumeric))
        .take(MAX_TOKENS)
        .map(|t| format!("'{}':*", t.replace('\\', "\\\\").replace('\'', "''")))
        .collect();
    if tokens.is_empty() {
        return Ok(None);
    }
    Ok(Some((tokens.join(" & "), normalized)))
}

/// Score-ordered keyset cursor: `<score>~<rfc3339>~<uuid>`. The score is a
/// finite float64 the server computed; garbage is rejected before SQL.
fn parse_score_cursor(raw: &str) -> AppResult<(f64, DateTime<Utc>, Uuid)> {
    let invalid = || AppError::Validation("invalid cursor".into());
    if raw.len() > 160 {
        return Err(invalid());
    }
    let mut parts = raw.splitn(3, '~');
    let (Some(score), Some(at), Some(id)) = (parts.next(), parts.next(), parts.next()) else {
        return Err(invalid());
    };
    let score: f64 = score.parse().map_err(|_| invalid())?;
    if !score.is_finite() {
        return Err(invalid());
    }
    let at = DateTime::parse_from_rfc3339(at)
        .map_err(|_| invalid())?
        .with_timezone(&Utc);
    let id = Uuid::parse_str(id).map_err(|_| invalid())?;
    Ok((score, at, id))
}

fn format_score_cursor(score: f64, at: DateTime<Utc>, id: Uuid) -> String {
    format!("{score}~{}~{id}", at.to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tsquery_quotes_plain_tokens_as_prefix_lexemes() {
        let (q, raw) = build_tsquery("deploy prod").unwrap().unwrap();
        assert_eq!(q, "'deploy':* & 'prod':*");
        assert_eq!(raw, "deploy prod");
    }

    #[test]
    fn tsquery_neutralizes_operator_injection() {
        // tsquery syntax in user text must be matched literally, never parsed.
        let (q, _) = build_tsquery("a&b !c:* (d|e)").unwrap().unwrap();
        assert_eq!(q, "'a&b':* & '!c:*':* & '(d|e)':*");

        // Embedded quotes cannot break out of the quoted lexeme.
        let (q, _) = build_tsquery("it's & !injection").unwrap().unwrap();
        assert!(q.starts_with("'it''s':*"));
        // "&" alone has no alphanumeric — dropped; "!injection" survives quoted.
        assert!(q.ends_with("'!injection':*"));
        assert_eq!(q.matches(" & ").count(), 1);
    }

    #[test]
    fn tsquery_drops_pure_punctuation_and_empty_input() {
        assert!(build_tsquery("").unwrap().is_none());
        assert!(build_tsquery("   ").unwrap().is_none());
        assert!(build_tsquery("&& || !! ()").unwrap().is_none());
    }

    #[test]
    fn tsquery_strips_control_characters() {
        let (q, raw) = build_tsquery("de\u{0007}ploy\u{001b}").unwrap().unwrap();
        assert_eq!(raw, "deploy");
        assert_eq!(q, "'deploy':*");
    }

    #[test]
    fn tsquery_caps_token_count() {
        let input = (0..20).map(|i| format!("t{i}")).collect::<Vec<_>>().join(" ");
        let (q, _) = build_tsquery(&input).unwrap().unwrap();
        assert_eq!(q.matches(":*").count(), MAX_TOKENS);
    }

    #[test]
    fn tsquery_rejects_over_length_queries() {
        let long = "a".repeat(MAX_QUERY_CHARS + 1);
        assert!(build_tsquery(&long).is_err());
        let exact = "a".repeat(MAX_QUERY_CHARS);
        assert!(build_tsquery(&exact).unwrap().is_some());
    }

    #[test]
    fn tsquery_passes_unicode_through_nfc() {
        // é as e + combining acute must normalize to the composed form.
        let (q, raw) = build_tsquery("caf\u{0065}\u{0301}").unwrap().unwrap();
        assert_eq!(raw, "café");
        assert_eq!(q, "'café':*");
    }

    #[test]
    fn score_cursor_round_trips() {
        let at = Utc::now();
        let id = Uuid::new_v4();
        let raw = format_score_cursor(0.123456789, at, id);
        let (score, parsed_at, parsed_id) = parse_score_cursor(&raw).unwrap();
        assert_eq!(score, 0.123456789);
        assert_eq!(parsed_at, at);
        assert_eq!(parsed_id, id);
    }

    #[test]
    fn score_cursor_rejects_garbage() {
        assert!(parse_score_cursor("").is_err());
        assert!(parse_score_cursor("1.0~notadate~xyz").is_err());
        assert!(parse_score_cursor("NaN~2026-01-01T00:00:00Z~00000000-0000-0000-0000-000000000000").is_err());
        assert!(parse_score_cursor("inf~2026-01-01T00:00:00Z~00000000-0000-0000-0000-000000000000").is_err());
        assert!(parse_score_cursor(&"x".repeat(200)).is_err());
    }
}
