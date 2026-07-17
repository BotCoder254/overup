//! Deployment-global webhook signature-rejection tracker.
//!
//! A rejected delivery is dropped BEFORE anything is persisted (that is the
//! security boundary), which historically left the platform blind to a
//! misconfigured `GITHUB_WEBHOOK_SECRET`: the repository Events timeline
//! just stayed empty while GitHub's Recent Deliveries page filled with red
//! 401s. This tracker is the observability seam: bounded, lock-free,
//! in-memory (resets on restart — deliberate, it answers "is the CURRENT
//! deployment's secret wrong?"), and it only ever stores static cause
//! categories — never attacker-controlled bytes. Surfaced on the repository
//! detail `health` object and the sync panel's "Webhook auth" KPI.

use std::sync::atomic::{AtomicI64, AtomicU64, AtomicU8, Ordering};

use chrono::{DateTime, TimeZone, Utc};

/// Why a delivery failed verification. Static category strings only — the
/// house rule for every operator-facing failure label (`sync_error`,
/// `error_category`). The cause is logged, never returned to the caller: it
/// distinguishes a misconfiguration from an attack for the operator without
/// telling an attacker which of their guesses got closer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureError {
    /// No `X-Hub-Signature-256` at all → the App has no webhook secret set.
    MissingHeader,
    /// Header present but not readable as ASCII.
    MalformedHeader,
    /// Not `sha256=…` — e.g. a sha1-only sender.
    BadPrefix,
    /// The digest after `sha256=` isn't hex.
    InvalidHex,
    /// A real HMAC mismatch → the configured secret differs from GitHub's.
    Mismatch,
}

impl SignatureError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MissingHeader => "missing_header",
            Self::MalformedHeader => "malformed_header",
            Self::BadPrefix => "bad_prefix",
            Self::InvalidHex => "invalid_hex",
            Self::Mismatch => "mismatch",
        }
    }

    /// Compact discriminant for the atomic last-cause cell (0 = none).
    fn code(self) -> u8 {
        match self {
            Self::MissingHeader => 1,
            Self::MalformedHeader => 2,
            Self::BadPrefix => 3,
            Self::InvalidHex => 4,
            Self::Mismatch => 5,
        }
    }

    fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::MissingHeader),
            2 => Some(Self::MalformedHeader),
            3 => Some(Self::BadPrefix),
            4 => Some(Self::InvalidHex),
            5 => Some(Self::Mismatch),
            _ => None,
        }
    }
}

/// Number of hourly ring buckets backing the rolling-24h count.
const BUCKETS: usize = 24;

/// Lock-free rejection counters. Benign read/write races are acceptable —
/// this is an observability gauge, not an audit record (rejected deliveries
/// are unauthenticated and must never reach Postgres).
#[derive(Default)]
pub struct WebhookAuthStats {
    /// Total rejections since process start.
    total: AtomicU64,
    /// Unix millis of the most recent rejection; 0 = never.
    last_at_ms: AtomicI64,
    /// `SignatureError::code()` of the most recent rejection; 0 = none.
    last_cause: AtomicU8,
    /// Hourly ring: `bucket_hours[i]` holds the absolute hour number the
    /// count in `bucket_counts[i]` belongs to, so stale buckets are ignored
    /// on read and reset on write.
    bucket_counts: [AtomicU64; BUCKETS],
    bucket_hours: [AtomicI64; BUCKETS],
}

/// Read-side snapshot for the repository health API.
pub struct WebhookAuthSnapshot {
    pub rejections_24h: u64,
    pub last_rejected_at: Option<DateTime<Utc>>,
    pub last_cause: Option<&'static str>,
}

impl WebhookAuthStats {
    pub fn record(&self, cause: SignatureError) {
        self.record_at(cause, Utc::now());
    }

    pub fn snapshot(&self) -> WebhookAuthSnapshot {
        self.snapshot_at(Utc::now())
    }

    /// Timestamp-injected core (tests exercise bucket expiry without sleeping).
    fn record_at(&self, cause: SignatureError, now: DateTime<Utc>) {
        self.total.fetch_add(1, Ordering::Relaxed);
        self.last_at_ms
            .store(now.timestamp_millis(), Ordering::Relaxed);
        self.last_cause.store(cause.code(), Ordering::Relaxed);

        let hour = now.timestamp() / 3600;
        let slot = (hour.rem_euclid(BUCKETS as i64)) as usize;
        // A bucket left over from an earlier lap of the ring restarts at
        // zero. The compare_exchange makes concurrent restarts collapse; the
        // worst race outcome is one lost increment — acceptable for a gauge.
        let stamped = self.bucket_hours[slot].load(Ordering::Relaxed);
        if stamped != hour
            && self.bucket_hours[slot]
                .compare_exchange(stamped, hour, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            self.bucket_counts[slot].store(0, Ordering::Relaxed);
        }
        self.bucket_counts[slot].fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot_at(&self, now: DateTime<Utc>) -> WebhookAuthSnapshot {
        let hour = now.timestamp() / 3600;
        let mut rejections_24h = 0;
        for slot in 0..BUCKETS {
            let stamped = self.bucket_hours[slot].load(Ordering::Relaxed);
            // Only buckets from the last 24 absolute hours count.
            if stamped > hour - BUCKETS as i64 && stamped <= hour {
                rejections_24h += self.bucket_counts[slot].load(Ordering::Relaxed);
            }
        }
        let last_ms = self.last_at_ms.load(Ordering::Relaxed);
        let last_rejected_at = (last_ms > 0)
            .then(|| Utc.timestamp_millis_opt(last_ms).single())
            .flatten();
        let last_cause = SignatureError::from_code(self.last_cause.load(Ordering::Relaxed))
            .map(SignatureError::as_str);
        WebhookAuthSnapshot {
            rejections_24h,
            last_rejected_at,
            last_cause,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn record_then_snapshot_roundtrip() {
        let stats = WebhookAuthStats::default();
        let now = Utc::now();
        stats.record_at(SignatureError::Mismatch, now);
        stats.record_at(SignatureError::Mismatch, now);
        stats.record_at(SignatureError::MissingHeader, now);

        let snap = stats.snapshot_at(now);
        assert_eq!(snap.rejections_24h, 3);
        assert_eq!(snap.last_cause, Some("missing_header"));
        let last = snap.last_rejected_at.expect("last rejection recorded");
        assert!((last - now).num_seconds().abs() < 1);
    }

    #[test]
    fn empty_snapshot_reads_clean() {
        let snap = WebhookAuthStats::default().snapshot();
        assert_eq!(snap.rejections_24h, 0);
        assert_eq!(snap.last_rejected_at, None);
        assert_eq!(snap.last_cause, None);
    }

    #[test]
    fn buckets_expire_after_24_hours() {
        let stats = WebhookAuthStats::default();
        let start = Utc::now();
        stats.record_at(SignatureError::Mismatch, start);

        // Still visible 23h later, gone at 25h.
        assert_eq!(
            stats.snapshot_at(start + Duration::hours(23)).rejections_24h,
            1
        );
        assert_eq!(
            stats.snapshot_at(start + Duration::hours(25)).rejections_24h,
            0
        );
        // The lifetime last-rejection marker survives bucket expiry.
        assert!(
            stats
                .snapshot_at(start + Duration::hours(25))
                .last_rejected_at
                .is_some()
        );
    }

    #[test]
    fn stale_bucket_resets_on_ring_reuse() {
        let stats = WebhookAuthStats::default();
        let start = Utc::now();
        stats.record_at(SignatureError::Mismatch, start);
        // Exactly one ring lap later the same slot is reused; the stale
        // count must not leak into the fresh hour.
        let next_lap = start + Duration::hours(24);
        stats.record_at(SignatureError::Mismatch, next_lap);
        assert_eq!(stats.snapshot_at(next_lap).rejections_24h, 1);
    }

    /// The snapshot vocabulary is exactly the five static categories the
    /// frontend union types — a drifted label would render as an unknown
    /// cause in the UI.
    #[test]
    fn snapshot_causes_match_frontend_vocabulary() {
        for (cause, label) in [
            (SignatureError::MissingHeader, "missing_header"),
            (SignatureError::MalformedHeader, "malformed_header"),
            (SignatureError::BadPrefix, "bad_prefix"),
            (SignatureError::InvalidHex, "invalid_hex"),
            (SignatureError::Mismatch, "mismatch"),
        ] {
            let stats = WebhookAuthStats::default();
            stats.record(cause);
            assert_eq!(stats.snapshot().last_cause, Some(label));
        }
    }
}
