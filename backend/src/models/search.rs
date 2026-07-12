use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

/// One ranked hit off the `search_documents` index, with the score the
/// query layer computed (ts_rank_cd + exact/prefix title boosts).
#[derive(Debug, sqlx::FromRow)]
pub struct SearchHit {
    pub id: Uuid,
    pub entity_type: String,
    pub entity_id: Uuid,
    pub title: String,
    pub subtitle: String,
    pub meta: serde_json::Value,
    pub source_updated_at: DateTime<Utc>,
    pub score: f64,
}

/// Per-category totals over the whole matched, permission-filtered set —
/// drives the category tabs / palette group headers.
#[derive(Debug, sqlx::FromRow)]
pub struct SearchCategoryCount {
    pub entity_type: String,
    pub total: i64,
}

/// API shape (camelCase). `meta` is display-only chips (status, kind,
/// repo full_name, ...) written by the indexer — never free-form input.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResultResponse {
    pub id: Uuid,
    pub category: String,
    pub entity_id: Uuid,
    pub title: String,
    pub subtitle: String,
    pub meta: serde_json::Value,
    pub updated_at: DateTime<Utc>,
    pub score: f64,
}

impl From<SearchHit> for SearchResultResponse {
    fn from(row: SearchHit) -> Self {
        Self {
            id: row.id,
            category: row.entity_type,
            entity_id: row.entity_id,
            title: row.title,
            subtitle: row.subtitle,
            meta: row.meta,
            updated_at: row.source_updated_at,
            score: row.score,
        }
    }
}
