//! Live execution fan-out and the log ingest path.
//!
//! One broadcast channel per pipeline carries typed [`BrowserEvent`]s to
//! every subscribed browser WebSocket. Log chunks pass through here on their
//! way to Postgres: secret masking and size caps are applied server-side
//! BEFORE anything is persisted or broadcast — the runner is not trusted to
//! mask, and the browser never sees an unmasked byte.

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::Serialize;
use sqlx::PgPool;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::db;
use crate::models::artifact::ArtifactResponse;
use crate::models::pipeline::{PipelineEventResponse, PipelineJobResponse};

/// Per-pipeline broadcast depth. Slow browsers that lag receive a
/// `log_gap` and backfill over REST.
const CHANNEL_CAPACITY: usize = 512;

/// Longest single log line persisted; the rest of the line is dropped.
const MAX_LINE_BYTES: usize = 8 * 1024;

/// Largest chunk accepted from a runner message.
const MAX_CHUNK_BYTES: usize = 64 * 1024;

/// Sentinel sequence for the one overflow marker appended when a job hits
/// its log byte budget.
const OVERFLOW_MARKER_SEQ: i64 = i64::MAX;

/// Never mask values shorter than this — masking "a" would shred output.
const MIN_MASK_LEN: usize = 6;

/// How long mask values stay registered after a job finishes, so straggler
/// chunks that race the completion message are still masked.
const MASK_CLEAR_GRACE: std::time::Duration = std::time::Duration::from_secs(120);

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserEvent {
    #[serde(rename_all = "camelCase")]
    PipelineUpdate {
        id: Uuid,
        status: String,
        conclusion: Option<String>,
        started_at: Option<DateTime<Utc>>,
        finished_at: Option<DateTime<Utc>>,
    },
    JobUpdate {
        job: PipelineJobResponse,
    },
    Event {
        event: PipelineEventResponse,
    },
    #[serde(rename_all = "camelCase")]
    Log {
        job_id: Uuid,
        seq: i64,
        stream: &'static str,
        text: String,
        created_at: DateTime<Utc>,
    },
    /// The subscriber lagged behind the broadcast; it should refetch state
    /// and backfill logs over REST. job_id is None when the whole stream
    /// lagged rather than one job.
    #[serde(rename_all = "camelCase")]
    LogGap {
        job_id: Option<Uuid>,
    },
    Artifact {
        artifact: ArtifactResponse,
    },
}

/// Per-job masking state. Besides the secret values themselves, it keeps a
/// small carry-over tail (shorter than the longest secret) held back from
/// each chunk so a secret split across two runner chunks is still caught,
/// and the last seen seq so replayed chunks can't corrupt the carry.
struct MaskState {
    values: Vec<String>,
    max_len: usize,
    carry: String,
    carry_stream: &'static str,
    last_seq: i64,
    /// Bumped on every registration; delayed clears only remove the entry
    /// when the generation still matches, so a requeued job that was
    /// re-dispatched inside the grace window keeps its masks.
    generation: u64,
}

impl Default for MaskState {
    fn default() -> Self {
        Self {
            values: Vec::new(),
            max_len: 0,
            carry: String::new(),
            carry_stream: "stdout",
            last_seq: -1,
            generation: 0,
        }
    }
}

#[derive(Default)]
pub struct LogHub {
    channels: DashMap<Uuid, broadcast::Sender<BrowserEvent>>,
    /// job_id -> secret values that must never appear in logs (checkout
    /// tokens, confidential-looking env values). Registered at assignment,
    /// cleared (after a grace period) at completion. Memory only.
    masks: DashMap<Uuid, MaskState>,
}

impl LogHub {
    pub fn subscribe(&self, pipeline_id: Uuid) -> broadcast::Receiver<BrowserEvent> {
        self.channels
            .entry(pipeline_id)
            .or_insert_with(|| broadcast::channel(CHANNEL_CAPACITY).0)
            .subscribe()
    }

    /// Fan an event out to subscribers; idle channels are pruned lazily.
    pub fn publish(&self, pipeline_id: Uuid, event: BrowserEvent) {
        if let Some(tx) = self.channels.get(&pipeline_id)
            && tx.send(event).is_err()
        {
            drop(tx);
            // Nobody is listening — reclaim the entry.
            self.channels
                .remove_if(&pipeline_id, |_, tx| tx.receiver_count() == 0);
        }
    }

    pub fn register_masks(&self, job_id: Uuid, values: Vec<String>) {
        let values: Vec<String> = values
            .into_iter()
            .filter(|v| v.len() >= MIN_MASK_LEN)
            .collect();
        if !values.is_empty() {
            let mut state = self.masks.entry(job_id).or_default();
            state.max_len = state
                .max_len
                .max(values.iter().map(|v| v.len()).max().unwrap_or(0));
            state.values.extend(values);
            state.generation += 1;
        }
    }

    /// Immediate removal — only for the failed-dispatch unwind where nothing
    /// was ever logged. Completed jobs go through [`LogHub::finish_job`].
    pub fn clear_masks(&self, job_id: Uuid) {
        self.masks.remove(&job_id);
    }

    /// Mask one chunk, carrying a tail shorter than the longest secret over
    /// to the next chunk so boundary-spanning secrets are caught. Returns
    /// None for a replayed seq (the DB would drop it anyway; skipping keeps
    /// the carry from being corrupted by duplicates).
    fn mask_chunk(&self, job_id: Uuid, seq: i64, stream: &'static str, text: &str) -> Option<(String, &'static str)> {
        let Some(mut state) = self.masks.get_mut(&job_id) else {
            // No masks registered: pass through untouched (zero-copy path).
            return Some((text.to_string(), stream));
        };
        if seq <= state.last_seq {
            return None;
        }
        state.last_seq = seq;

        let out_stream = if state.carry.is_empty() {
            stream
        } else {
            state.carry_stream
        };
        let mut combined = std::mem::take(&mut state.carry);
        combined.push_str(text);
        let mut masked = apply_masks(combined, &state.values);

        // Hold back up to max_len - 1 trailing bytes; anything before the
        // last newline can flush eagerly (mask values never contain '\n' —
        // they come from single-line tokens/env values).
        let holdback = state.max_len.saturating_sub(1);
        let mut cut = floor_char_boundary(&masked, masked.len().saturating_sub(holdback));
        if let Some(nl) = masked[cut..].rfind('\n') {
            cut = cut + nl + 1;
        }
        state.carry = masked.split_off(cut);
        state.carry_stream = stream;
        Some((masked, out_stream))
    }

    /// The single write path for job output: mask (with cross-chunk carry)
    /// -> cap -> persist -> broadcast. Masking runs FIRST so the line cap
    /// can never bisect a secret and leak its prefix. Chunks beyond the
    /// per-job byte budget are dropped after a single overflow marker.
    #[allow(clippy::too_many_arguments)]
    pub async fn ingest_log(
        &self,
        pool: &PgPool,
        pipeline_id: Uuid,
        job_id: Uuid,
        seq: i64,
        stream: protocol::LogStream,
        text: &str,
        max_log_bytes: i64,
    ) -> sqlx::Result<()> {
        if seq < 0 || seq == OVERFLOW_MARKER_SEQ {
            return Ok(());
        }

        let Some((masked, out_stream)) = self.mask_chunk(job_id, seq, stream.as_str(), text)
        else {
            return Ok(());
        };
        let masked = cap_chunk(&masked);
        if masked.is_empty() {
            // The whole chunk was held back as carry; it will flush with the
            // next chunk or at job completion.
            return Ok(());
        }
        self.write_chunk(pool, pipeline_id, job_id, seq, out_stream, masked, max_log_bytes)
            .await
    }

    /// Flush any held-back carry as a final chunk, then drop the mask state
    /// after a grace period (generation-guarded so a requeued job that was
    /// re-dispatched in the meantime keeps its fresh masks).
    pub async fn finish_job(
        self: &std::sync::Arc<Self>,
        pool: &PgPool,
        pipeline_id: Uuid,
        job_id: Uuid,
        max_log_bytes: i64,
    ) -> sqlx::Result<()> {
        let flush = self.masks.get_mut(&job_id).and_then(|mut state| {
            if state.carry.is_empty() {
                return None;
            }
            let text = std::mem::take(&mut state.carry);
            state.last_seq += 1;
            Some((state.last_seq, state.carry_stream, text, state.generation))
        });

        let generation = if let Some((seq, stream, text, generation)) = flush {
            let text = cap_chunk(&text);
            if !text.is_empty() {
                self.write_chunk(pool, pipeline_id, job_id, seq, stream, text, max_log_bytes)
                    .await?;
            }
            Some(generation)
        } else {
            self.masks.get(&job_id).map(|state| state.generation)
        };

        if let Some(generation) = generation {
            let hub = std::sync::Arc::clone(self);
            tokio::spawn(async move {
                tokio::time::sleep(MASK_CLEAR_GRACE).await;
                hub.masks
                    .remove_if(&job_id, |_, state| state.generation == generation);
            });
        }
        Ok(())
    }

    /// Persist one already-masked, already-capped chunk and broadcast it,
    /// enforcing the per-job byte budget with a single overflow marker.
    #[allow(clippy::too_many_arguments)]
    async fn write_chunk(
        &self,
        pool: &PgPool,
        pipeline_id: Uuid,
        job_id: Uuid,
        seq: i64,
        stream_name: &'static str,
        masked: String,
        max_log_bytes: i64,
    ) -> sqlx::Result<()> {
        let byte_len = masked.len().min(i32::MAX as usize) as i32;

        let total_after = db::pipeline_jobs::add_log_bytes(pool, job_id, byte_len as i64).await?;
        let total_before = total_after - byte_len as i64;

        if total_before >= max_log_bytes {
            // Budget already exhausted; the marker chunk was written when we
            // crossed the line. Drop silently (log_bytes keeps counting).
            return Ok(());
        }

        db::pipeline_logs::insert_chunks(
            pool,
            job_id,
            &[db::pipeline_logs::NewChunk {
                seq,
                stream: stream_name,
                content: masked.clone(),
                byte_len,
            }],
        )
        .await?;

        self.publish(
            pipeline_id,
            BrowserEvent::Log {
                job_id,
                seq,
                stream: stream_name,
                text: masked,
                created_at: Utc::now(),
            },
        );

        if total_after >= max_log_bytes {
            let marker = "[log output truncated: per-job log limit reached]".to_string();
            db::pipeline_logs::insert_chunks(
                pool,
                job_id,
                &[db::pipeline_logs::NewChunk {
                    seq: OVERFLOW_MARKER_SEQ,
                    stream: "system",
                    content: marker.clone(),
                    byte_len: marker.len() as i32,
                }],
            )
            .await?;
            self.publish(
                pipeline_id,
                BrowserEvent::Log {
                    job_id,
                    seq: OVERFLOW_MARKER_SEQ,
                    stream: "system",
                    text: marker,
                    created_at: Utc::now(),
                },
            );
        }

        Ok(())
    }
}

/// Replace every registered secret with `***`.
fn apply_masks(text: String, values: &[String]) -> String {
    let mut masked = text;
    for value in values {
        if masked.contains(value.as_str()) {
            masked = masked.replace(value.as_str(), "***");
        }
    }
    masked
}

/// Largest index `<= at` that lands on a UTF-8 character boundary.
fn floor_char_boundary(text: &str, at: usize) -> usize {
    if at >= text.len() {
        return text.len();
    }
    let mut at = at;
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Enforce the chunk and per-line byte caps without splitting UTF-8.
fn cap_chunk(text: &str) -> String {
    let mut text = text.to_string();
    crate::services::pipeline_plan::truncate_utf8(&mut text, MAX_CHUNK_BYTES);
    if text.lines().all(|line| line.len() <= MAX_LINE_BYTES) {
        return text;
    }
    let capped: Vec<String> = text
        .lines()
        .map(|line| {
            let mut line = line.to_string();
            crate::services::pipeline_plan::truncate_utf8(&mut line, MAX_LINE_BYTES);
            line
        })
        .collect();
    capped.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run a chunk through mask_chunk and return the emitted text (the
    /// held-back carry stays inside the hub).
    fn masked(hub: &LogHub, job: Uuid, seq: i64, text: &str) -> Option<String> {
        hub.mask_chunk(job, seq, "stdout", text).map(|(t, _)| t)
    }

    #[test]
    fn masking_replaces_registered_values() {
        let hub = LogHub::default();
        let job = Uuid::new_v4();
        hub.register_masks(job, vec!["ghs_supersecrettoken".into(), "x".into()]);
        // Trailing newline lets the whole line flush (no carry held back).
        let out = masked(&hub, job, 1, "auth with ghs_supersecrettoken done\n").unwrap();
        assert_eq!(out, "auth with *** done\n");
        // Too-short values are never registered.
        let out = masked(&hub, job, 2, "x marks the spot\n").unwrap();
        assert_eq!(out, "x marks the spot\n");
        hub.clear_masks(job);
        let out = masked(&hub, job, 3, "ghs_supersecrettoken").unwrap();
        assert_eq!(out, "ghs_supersecrettoken");
    }

    #[test]
    fn secret_split_across_chunks_is_masked() {
        let hub = LogHub::default();
        let job = Uuid::new_v4();
        hub.register_masks(job, vec!["ghs_supersecrettoken".into()]);
        // The secret is bisected by the chunk boundary.
        let first = masked(&hub, job, 1, "token is ghs_super").unwrap();
        assert!(
            !first.contains("ghs_super"),
            "partial secret must be held back, got {first:?}"
        );
        let second = masked(&hub, job, 2, "secrettoken end\n").unwrap();
        let combined = format!("{first}{second}");
        assert!(!combined.contains("ghs_supersecrettoken"));
        assert!(combined.contains("token is ***"));
        assert!(combined.ends_with("end\n"));
    }

    #[test]
    fn replayed_seq_is_dropped_and_carry_survives() {
        let hub = LogHub::default();
        let job = Uuid::new_v4();
        hub.register_masks(job, vec!["ghs_supersecrettoken".into()]);
        let _ = masked(&hub, job, 1, "prefix ghs_super");
        // A reconnecting runner re-sends seq 1; it must not disturb carry.
        assert!(masked(&hub, job, 1, "prefix ghs_super").is_none());
        let second = masked(&hub, job, 2, "secrettoken\n").unwrap();
        assert!(!second.contains("secrettoken"));
    }

    #[test]
    fn carry_is_flushed_by_newline() {
        let hub = LogHub::default();
        let job = Uuid::new_v4();
        hub.register_masks(job, vec!["ghs_supersecrettoken".into()]);
        // Ends with newline: everything flushes, nothing held back.
        let out = masked(&hub, job, 1, "line one\nline two\n").unwrap();
        assert_eq!(out, "line one\nline two\n");
        assert!(hub.masks.get(&job).unwrap().carry.is_empty());
    }

    #[test]
    fn no_mask_path_passes_through() {
        let hub = LogHub::default();
        let job = Uuid::new_v4();
        // No masks registered: chunks pass through verbatim, replay
        // protection is left to the database unique index.
        let out = masked(&hub, job, 1, "anything at all").unwrap();
        assert_eq!(out, "anything at all");
        let out = masked(&hub, job, 1, "anything at all").unwrap();
        assert_eq!(out, "anything at all");
    }

    #[test]
    fn floor_char_boundary_respects_utf8() {
        let text = "héllo"; // 'é' is two bytes (1..3)
        assert_eq!(floor_char_boundary(text, 2), 1);
        assert_eq!(floor_char_boundary(text, 3), 3);
        assert_eq!(floor_char_boundary(text, 99), text.len());
    }

    #[test]
    fn cap_chunk_truncates_long_lines() {
        let long = "a".repeat(MAX_LINE_BYTES + 100);
        let capped = cap_chunk(&format!("short\n{long}"));
        let lines: Vec<&str> = capped.lines().collect();
        assert_eq!(lines[0], "short");
        assert_eq!(lines[1].len(), MAX_LINE_BYTES);
    }
}
