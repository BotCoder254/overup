//! Wire protocol shared by the overup control plane and runners.
//!
//! Every message is a JSON object tagged by `type`. Job assignments are
//! integrity-protected: the control plane signs the exact transmitted
//! payload string with HMAC-SHA256 and the runner verifies it (constant
//! time) before parsing — a tampered or replayed payload is never executed.

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// Bump when messages change incompatibly; exchanged in hello/hello_ack.
pub const PROTOCOL_VERSION: u32 = 1;

/// Fine-grained job stages, in lifecycle order. Pure telemetry — scheduling
/// correctness depends only on the coarse status/conclusion columns.
pub const STAGES: &[&str] = &[
    "queued",
    "preparing",
    "waiting_for_runner",
    "assigned",
    "pulling_image",
    "starting_container",
    "running",
    "uploading_artifacts",
    "cleaning_workspace",
    "done",
];

/// Execution phases a log chunk can be attributed to, in lifecycle order.
/// Presentation-only section metadata — the server allow-lists inbound
/// values against this set and drops anything else (field, not chunk).
pub const LOG_PHASES: &[&str] = &[
    "checkout",
    "image_pull",
    "container",
    "steps",
    "artifacts",
    "cleanup",
];

// ---------------------------------------------------------------------------
// Runner -> server
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunnerMsg {
    /// First frame after the WebSocket opens.
    Hello {
        name: String,
        version: String,
        labels: Vec<String>,
        docker_available: bool,
    },
    Heartbeat {
        busy_job_id: Option<Uuid>,
        /// Ambient host telemetry. Older runners omit it and newer servers
        /// default it to `None`, so this addition never bumps
        /// [`PROTOCOL_VERSION`].
        #[serde(default)]
        health: Option<RunnerHealth>,
    },
    /// Acknowledges a job_assign; the scheduler reverts unacked assignments.
    JobAck {
        job_id: Uuid,
    },
    JobStage {
        job_id: Uuid,
        stage: String,
        detail: Option<String>,
        /// Structured per-step progress riding the stage channel. Older
        /// runners omit it and newer servers default it to `None`, so this
        /// addition never bumps [`PROTOCOL_VERSION`].
        #[serde(default)]
        step: Option<StepProgress>,
    },
    /// One chunk of output. `seq` is runner-monotonic per job so the server
    /// can dedupe and browsers can detect gaps.
    Log {
        job_id: Uuid,
        seq: u64,
        stream: LogStream,
        text: String,
        /// 0-based index into the signed plan's steps this chunk belongs to.
        /// Optional section attribution — same forward-compat convention as
        /// `Heartbeat.health`; never bumps [`PROTOCOL_VERSION`].
        #[serde(default)]
        step: Option<u32>,
        /// One of [`LOG_PHASES`]; the server re-validates before persisting.
        #[serde(default)]
        phase: Option<String>,
    },
    /// Ask for a presigned upload URL for one artifact.
    ArtifactRequest {
        job_id: Uuid,
        name: String,
        size_bytes: u64,
        content_type: String,
    },
    ArtifactDone {
        job_id: Uuid,
        name: String,
        size_bytes: u64,
        checksum_sha256: String,
        /// Archive introspection. Older runners omit all three and newer
        /// servers default them to `None`, so this addition never bumps
        /// [`PROTOCOL_VERSION`]. The runner caps `entries` well below the
        /// server's 128 KB inbound frame limit; the server re-validates.
        #[serde(default)]
        uncompressed_bytes: Option<u64>,
        #[serde(default)]
        file_count: Option<u32>,
        #[serde(default)]
        entries: Option<Vec<ArtifactEntry>>,
    },
    JobResult {
        job_id: Uuid,
        conclusion: JobConclusion,
        exit_code: Option<i32>,
        /// Static category strings only (step_failed, image_pull_failed, ...).
        error_category: Option<String>,
        metrics: JobMetrics,
    },
}

/// One file inside an archive artifact, reported with `artifact_done` so
/// browsers can list archive contents without server-side extraction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ArtifactEntry {
    pub path: String,
    pub size_bytes: u64,
}

/// Ambient host telemetry reported with `heartbeat`. Every field is
/// optional so this can grow without ever bumping [`PROTOCOL_VERSION`] —
/// same forward-compat convention as [`JobMetrics`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RunnerHealth {
    /// Current CPU load in permille of one core (2500 = 2.5 cores).
    #[serde(default)]
    pub cpu_permille: Option<u64>,
    #[serde(default)]
    pub mem_used_bytes: Option<u64>,
    #[serde(default)]
    pub mem_total_bytes: Option<u64>,
    #[serde(default)]
    pub disk_used_bytes: Option<u64>,
    #[serde(default)]
    pub disk_total_bytes: Option<u64>,
    #[serde(default)]
    pub docker_version: Option<String>,
    #[serde(default)]
    pub os: Option<String>,
    #[serde(default)]
    pub uptime_secs: Option<u64>,
}

/// Structured progress for one plan step, reported with `job_stage`. The
/// server validates `index`/`total` against the signed plan and allow-lists
/// `status` before recording anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StepProgress {
    /// 0-based index into the signed plan's steps.
    pub index: u32,
    /// Total step count from the signed plan (cross-checked server-side).
    pub total: u32,
    /// "started" | "succeeded" | "failed" — the server allow-lists.
    pub status: String,
    #[serde(default)]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogStream {
    Stdout,
    Stderr,
    System,
}

impl LogStream {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogStream::Stdout => "stdout",
            LogStream::Stderr => "stderr",
            LogStream::System => "system",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobConclusion {
    Success,
    Failure,
    Cancelled,
    TimedOut,
}

impl JobConclusion {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobConclusion::Success => "success",
            JobConclusion::Failure => "failure",
            JobConclusion::Cancelled => "cancelled",
            JobConclusion::TimedOut => "timed_out",
        }
    }
}

/// Execution telemetry reported with `job_result`. Every field is optional:
/// older runners omit the resource fields and newer servers default them to
/// `None`, so additions here never bump [`PROTOCOL_VERSION`]. Resource values
/// are integers (permille, bytes) so they round-trip exactly through JSONB.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct JobMetrics {
    pub image_pull_ms: Option<u64>,
    pub exec_ms: Option<u64>,
    /// Peak CPU usage in permille of one core (2500 = 2.5 cores).
    #[serde(default)]
    pub cpu_peak_permille: Option<u64>,
    /// Mean CPU usage across samples, same unit as the peak.
    #[serde(default)]
    pub cpu_avg_permille: Option<u64>,
    #[serde(default)]
    pub mem_peak_bytes: Option<u64>,
    #[serde(default)]
    pub net_rx_bytes: Option<u64>,
    #[serde(default)]
    pub net_tx_bytes: Option<u64>,
    #[serde(default)]
    pub blkio_read_bytes: Option<u64>,
    #[serde(default)]
    pub blkio_write_bytes: Option<u64>,
    /// Number of stats samples the aggregates were computed from.
    #[serde(default)]
    pub sample_count: Option<u32>,
}

// ---------------------------------------------------------------------------
// Server -> runner
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    HelloAck {
        runner_id: Uuid,
        heartbeat_interval_secs: u64,
        protocol_version: u32,
        /// Present exactly once: when this connection authenticated with a
        /// short-lived bootstrap token, this is the permanent credential the
        /// runner must persist and use for every future reconnect. Older
        /// runners that don't look for this field simply ignore it, so this
        /// addition never bumps [`PROTOCOL_VERSION`].
        #[serde(default)]
        permanent_token: Option<String>,
        /// Job-payload HMAC verification key, delivered to every
        /// authenticated runner over the (TLS) socket so operators no longer
        /// have to copy `RUNNER_JOB_SIGNING_KEY` by hand. Runners hold it in
        /// memory only and re-receive it on every connect; a locally
        /// configured key always takes precedence. Optional for the same
        /// forward-compat reason as `permanent_token`.
        #[serde(default)]
        job_signing_key: Option<String>,
    },
    Ping,
    /// Signed job descriptor: `payload_json` parses to [`JobPayload`] only
    /// after `signature_hex` verifies over its exact bytes.
    JobAssign {
        payload_json: String,
        signature_hex: String,
    },
    JobCancel {
        job_id: Uuid,
        reason: CancelReason,
    },
    ArtifactGrant {
        job_id: Uuid,
        name: String,
        put_url: String,
        key: String,
        expires_at: DateTime<Utc>,
    },
    ArtifactDeny {
        job_id: Uuid,
        name: String,
        reason: String,
    },
    Error {
        code: String,
    },
    /// An operator changed this runner's scheduling eligibility. Since job
    /// assignment is always server-initiated (the runner never polls for
    /// work), this is purely informational on the runner side — display
    /// and logging only, never an accept/reject gate.
    LifecycleChanged {
        status: RunnerLifecycle,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelReason {
    User,
    Timeout,
    PipelineCancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerLifecycle {
    /// Stops taking new work immediately; reversible via `Resumed`.
    Disabled,
    /// Finishing the current job, then will not take new work.
    Draining,
    Resumed,
}

/// The signed job descriptor. Everything a runner needs to execute one job;
/// bound to a single runner and a short validity window.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct JobPayload {
    pub job_id: Uuid,
    pub pipeline_id: Uuid,
    /// The runner this payload was issued to — runners refuse others'.
    pub runner_id: Uuid,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub image: String,
    pub env: std::collections::BTreeMap<String, String>,
    pub steps: Vec<JobStep>,
    pub checkout: Option<Checkout>,
    pub timeout_seconds: u64,
    pub caps: JobCaps,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct JobStep {
    pub name: String,
    pub run: String,
    pub shell: String,
    /// Workspace-relative directory the step executes in (already
    /// normalized via [`normalize_relative_path`]); `None` = workspace root.
    #[serde(default)]
    pub working_dir: Option<String>,
}

/// Read-only, short-lived checkout credentials. The token is registered as
/// a mask pattern server-side before this payload is ever sent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Checkout {
    pub tarball_url: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct JobCaps {
    pub max_log_bytes: u64,
    pub max_artifact_bytes: u64,
    pub max_artifacts: u32,
}

// ---------------------------------------------------------------------------
// Path validation
// ---------------------------------------------------------------------------

/// Maximum byte length of a workspace-relative path (e.g. a step
/// `working-directory`).
pub const MAX_RELATIVE_PATH_BYTES: usize = 255;
/// Maximum number of `/`-separated segments in a workspace-relative path.
pub const MAX_RELATIVE_PATH_SEGMENTS: usize = 32;

/// Normalize and validate a workspace-relative path.
///
/// Lives in this crate so the control plane (parser, planner, dispatch) and
/// the runner validate with ONE implementation — the runner re-checks
/// independently and never trusts the payload for path safety.
///
/// Accepts only paths that stay inside the workspace by construction:
/// - one leading `./` and any trailing `/` are stripped (`./scripts/` → `scripts`)
/// - non-empty after normalization, ≤ 255 bytes, ≤ 32 segments
/// - no leading `/` (absolute), no `\`, no control characters
/// - every `/`-separated segment is non-empty (rejects `//`), not `.` or
///   `..`, and matches `[A-Za-z0-9._-]+` (ASCII only)
///
/// Returns the normalized path, or `None` when the value is unsafe. A plain
/// join of the result onto the workspace root cannot escape it.
pub fn normalize_relative_path(value: &str) -> Option<String> {
    let mut path = value.strip_prefix("./").unwrap_or(value);
    while let Some(stripped) = path.strip_suffix('/') {
        path = stripped;
    }
    if path.is_empty() || path.len() > MAX_RELATIVE_PATH_BYTES {
        return None;
    }
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() > MAX_RELATIVE_PATH_SEGMENTS {
        return None;
    }
    for segment in &segments {
        if segment.is_empty() || *segment == "." || *segment == ".." {
            return None;
        }
        if !segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        {
            return None;
        }
    }
    Some(path.to_string())
}

// ---------------------------------------------------------------------------
// Signing
// ---------------------------------------------------------------------------

/// HMAC-SHA256 over the exact bytes, hex-encoded.
pub fn sign(key: &[u8], payload: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(payload);
    hex::encode(mac.finalize().into_bytes())
}

/// Constant-time verification of a hex signature over the exact bytes.
pub fn verify(key: &[u8], payload: &[u8], signature_hex: &str) -> bool {
    let Ok(expected) = hex::decode(signature_hex) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(key) else {
        return false;
    };
    mac.update(payload);
    // verify_slice is constant-time.
    mac.verify_slice(&expected).is_ok()
}

/// Sign a job payload for transmission: serialize once, sign those bytes.
pub fn sign_job_payload(key: &[u8], payload: &JobPayload) -> serde_json::Result<ServerMsg> {
    let payload_json = serde_json::to_string(payload)?;
    let signature_hex = sign(key, payload_json.as_bytes());
    Ok(ServerMsg::JobAssign {
        payload_json,
        signature_hex,
    })
}

/// Verify and parse a received job payload. Returns None when the signature
/// is invalid, the payload is malformed, or the validity window has lapsed.
pub fn verify_job_payload(
    key: &[u8],
    payload_json: &str,
    signature_hex: &str,
    now: DateTime<Utc>,
) -> Option<JobPayload> {
    if !verify(key, payload_json.as_bytes(), signature_hex) {
        return None;
    }
    let payload: JobPayload = serde_json::from_str(payload_json).ok()?;
    if now < payload.issued_at - chrono::Duration::seconds(60) || now > payload.expires_at {
        return None;
    }
    Some(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_payload() -> JobPayload {
        JobPayload {
            job_id: Uuid::new_v4(),
            pipeline_id: Uuid::new_v4(),
            runner_id: Uuid::new_v4(),
            issued_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::minutes(5),
            image: "ubuntu:24.04".into(),
            env: [("CI".to_string(), "true".to_string())].into_iter().collect(),
            steps: vec![JobStep {
                name: "build".into(),
                run: "echo hello".into(),
                shell: "sh".into(),
                working_dir: None,
            }],
            checkout: None,
            timeout_seconds: 3600,
            caps: JobCaps {
                max_log_bytes: 10 * 1024 * 1024,
                max_artifact_bytes: 100 * 1024 * 1024,
                max_artifacts: 10,
            },
        }
    }

    #[test]
    fn sign_verify_round_trip() {
        let key = b"0123456789abcdef0123456789abcdef";
        let payload = sample_payload();
        let ServerMsg::JobAssign {
            payload_json,
            signature_hex,
        } = sign_job_payload(key, &payload).unwrap()
        else {
            panic!("expected JobAssign");
        };
        let verified = verify_job_payload(key, &payload_json, &signature_hex, Utc::now())
            .expect("valid signature must verify");
        assert_eq!(verified.job_id, payload.job_id);
        assert_eq!(verified.runner_id, payload.runner_id);
    }

    #[test]
    fn tampered_payload_is_rejected() {
        let key = b"0123456789abcdef0123456789abcdef";
        let ServerMsg::JobAssign {
            payload_json,
            signature_hex,
        } = sign_job_payload(key, &sample_payload()).unwrap()
        else {
            panic!("expected JobAssign");
        };
        // Flip one byte of the payload.
        let tampered = payload_json.replacen("echo", "Echo", 1);
        assert_ne!(tampered, payload_json);
        assert!(verify_job_payload(key, &tampered, &signature_hex, Utc::now()).is_none());
    }

    #[test]
    fn wrong_key_is_rejected() {
        let ServerMsg::JobAssign {
            payload_json,
            signature_hex,
        } = sign_job_payload(b"key-a-key-a-key-a-key-a-key-a-32", &sample_payload()).unwrap()
        else {
            panic!("expected JobAssign");
        };
        assert!(
            verify_job_payload(
                b"key-b-key-b-key-b-key-b-key-b-32",
                &payload_json,
                &signature_hex,
                Utc::now()
            )
            .is_none()
        );
    }

    #[test]
    fn expired_payload_is_rejected() {
        let key = b"0123456789abcdef0123456789abcdef";
        let mut payload = sample_payload();
        payload.issued_at = Utc::now() - chrono::Duration::minutes(20);
        payload.expires_at = Utc::now() - chrono::Duration::minutes(10);
        let ServerMsg::JobAssign {
            payload_json,
            signature_hex,
        } = sign_job_payload(key, &payload).unwrap()
        else {
            panic!("expected JobAssign");
        };
        assert!(verify_job_payload(key, &payload_json, &signature_hex, Utc::now()).is_none());
    }

    #[test]
    fn old_shape_job_result_still_parses() {
        // A job_result emitted by a pre-metrics-expansion runner: only the
        // original two metric fields. New fields must default to None.
        let json = format!(
            r#"{{"type":"job_result","job_id":"{}","conclusion":"success","exit_code":0,"error_category":null,"metrics":{{"image_pull_ms":1200,"exec_ms":45000}}}}"#,
            Uuid::nil()
        );
        let parsed: RunnerMsg = serde_json::from_str(&json).unwrap();
        let RunnerMsg::JobResult { metrics, .. } = parsed else {
            panic!("expected JobResult");
        };
        assert_eq!(metrics.image_pull_ms, Some(1200));
        assert_eq!(metrics.exec_ms, Some(45000));
        assert_eq!(metrics.cpu_peak_permille, None);
        assert_eq!(metrics.mem_peak_bytes, None);
        assert_eq!(metrics.sample_count, None);
    }

    #[test]
    fn old_shape_artifact_done_still_parses() {
        // An artifact_done emitted by a pre-manifest runner: only the
        // original four fields. The manifest fields must default to None.
        let json = format!(
            r#"{{"type":"artifact_done","job_id":"{}","name":"dist.tar.gz","size_bytes":1024,"checksum_sha256":"{}"}}"#,
            Uuid::nil(),
            "a".repeat(64)
        );
        let parsed: RunnerMsg = serde_json::from_str(&json).unwrap();
        let RunnerMsg::ArtifactDone {
            uncompressed_bytes,
            file_count,
            entries,
            ..
        } = parsed
        else {
            panic!("expected ArtifactDone");
        };
        assert_eq!(uncompressed_bytes, None);
        assert_eq!(file_count, None);
        assert!(entries.is_none());
    }

    #[test]
    fn artifact_done_round_trips_manifest() {
        let msg = RunnerMsg::ArtifactDone {
            job_id: Uuid::nil(),
            name: "dist.tar.gz".into(),
            size_bytes: 1024,
            checksum_sha256: "b".repeat(64),
            uncompressed_bytes: Some(4096),
            file_count: Some(2),
            entries: Some(vec![ArtifactEntry {
                path: "bin/app".into(),
                size_bytes: 4000,
            }]),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: RunnerMsg = serde_json::from_str(&json).unwrap();
        let RunnerMsg::ArtifactDone {
            uncompressed_bytes,
            file_count,
            entries,
            ..
        } = parsed
        else {
            panic!("expected ArtifactDone");
        };
        assert_eq!(uncompressed_bytes, Some(4096));
        assert_eq!(file_count, Some(2));
        assert_eq!(
            entries.as_deref(),
            Some(
                &[ArtifactEntry {
                    path: "bin/app".into(),
                    size_bytes: 4000,
                }][..]
            )
        );
    }

    #[test]
    fn old_shape_job_step_still_parses() {
        // A step serialized by a pre-working-directory control plane: only
        // the original three fields. `working_dir` must default to None.
        let json = r#"{"name":"build","run":"echo hello","shell":"sh"}"#;
        let step: JobStep = serde_json::from_str(json).unwrap();
        assert_eq!(step.working_dir, None);
    }

    #[test]
    fn working_dir_survives_sign_verify() {
        let key = b"0123456789abcdef0123456789abcdef";
        let mut payload = sample_payload();
        payload.steps[0].working_dir = Some("scripts/build".into());
        let ServerMsg::JobAssign {
            payload_json,
            signature_hex,
        } = sign_job_payload(key, &payload).unwrap()
        else {
            panic!("expected JobAssign");
        };
        let verified = verify_job_payload(key, &payload_json, &signature_hex, Utc::now())
            .expect("valid signature must verify");
        assert_eq!(verified.steps[0].working_dir.as_deref(), Some("scripts/build"));
    }

    #[test]
    fn normalize_relative_path_accepts_safe_paths() {
        assert_eq!(normalize_relative_path("scripts").as_deref(), Some("scripts"));
        assert_eq!(normalize_relative_path("a/b/c").as_deref(), Some("a/b/c"));
        assert_eq!(
            normalize_relative_path("sub.dir/x_y-z").as_deref(),
            Some("sub.dir/x_y-z")
        );
        // Normalization: one leading `./`, trailing slashes.
        assert_eq!(normalize_relative_path("./scripts").as_deref(), Some("scripts"));
        assert_eq!(normalize_relative_path("scripts/").as_deref(), Some("scripts"));
        assert_eq!(normalize_relative_path("./a/b/").as_deref(), Some("a/b"));
    }

    #[test]
    fn normalize_relative_path_rejects_unsafe_paths() {
        for bad in [
            "",
            ".",
            "./",
            "..",
            "a/../b",
            "../escape",
            "/etc",
            "/",
            "a//b",
            "a\\b",
            "a/б",       // non-ASCII
            "a b",       // space
            "~root",     // charset
            "a/\x07b",   // control char
            "${{ x }}",  // expression left unresolved
        ] {
            assert!(
                normalize_relative_path(bad).is_none(),
                "expected rejection of {bad:?}"
            );
        }
        // Length and segment budgets.
        assert!(normalize_relative_path(&"a".repeat(256)).is_none());
        assert!(normalize_relative_path(&["a"; 33].join("/")).is_none());
        assert!(normalize_relative_path(&["a"; 32].join("/")).is_some());
    }

    #[test]
    fn old_shape_heartbeat_still_parses() {
        // A heartbeat emitted by a pre-health-expansion runner: no `health`
        // field at all. It must default to None, not fail to parse.
        let json = r#"{"type":"heartbeat","busy_job_id":null}"#;
        let parsed: RunnerMsg = serde_json::from_str(json).unwrap();
        let RunnerMsg::Heartbeat { busy_job_id, health } = parsed else {
            panic!("expected Heartbeat");
        };
        assert_eq!(busy_job_id, None);
        assert_eq!(health, None);
    }

    #[test]
    fn old_shape_hello_ack_still_parses() {
        // A hello_ack emitted by a pre-key-delivery server: no
        // `job_signing_key` (and no `permanent_token`). Both must default to
        // None, not fail to parse.
        let json = format!(
            r#"{{"type":"hello_ack","runner_id":"{}","heartbeat_interval_secs":30,"protocol_version":1}}"#,
            Uuid::nil()
        );
        let parsed: ServerMsg = serde_json::from_str(&json).unwrap();
        let ServerMsg::HelloAck {
            permanent_token,
            job_signing_key,
            ..
        } = parsed
        else {
            panic!("expected HelloAck");
        };
        assert_eq!(permanent_token, None);
        assert_eq!(job_signing_key, None);
    }

    #[test]
    fn hello_ack_round_trips_job_signing_key() {
        let msg = ServerMsg::HelloAck {
            runner_id: Uuid::nil(),
            heartbeat_interval_secs: 30,
            protocol_version: PROTOCOL_VERSION,
            permanent_token: None,
            job_signing_key: Some("0123456789abcdef0123456789abcdef".into()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: ServerMsg = serde_json::from_str(&json).unwrap();
        let ServerMsg::HelloAck { job_signing_key, .. } = parsed else {
            panic!("expected HelloAck");
        };
        assert_eq!(
            job_signing_key.as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
    }

    #[test]
    fn message_wire_format_is_snake_case_tagged() {
        let msg = RunnerMsg::JobStage {
            job_id: Uuid::nil(),
            stage: "pulling_image".into(),
            detail: None,
            step: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"job_stage\""));
        let parsed: RunnerMsg = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, RunnerMsg::JobStage { .. }));
    }

    #[test]
    fn old_shape_log_still_parses() {
        // A log emitted by a pre-section runner: no `step`/`phase` fields.
        // Both must default to None, not fail to parse.
        let json = format!(
            r#"{{"type":"log","job_id":"{}","seq":7,"stream":"stdout","text":"hello"}}"#,
            Uuid::nil()
        );
        let parsed: RunnerMsg = serde_json::from_str(&json).unwrap();
        let RunnerMsg::Log { seq, step, phase, .. } = parsed else {
            panic!("expected Log");
        };
        assert_eq!(seq, 7);
        assert_eq!(step, None);
        assert_eq!(phase, None);
    }

    #[test]
    fn log_round_trips_section_attribution() {
        let msg = RunnerMsg::Log {
            job_id: Uuid::nil(),
            seq: 3,
            stream: LogStream::Stderr,
            text: "npm ERR!".into(),
            step: Some(2),
            phase: Some("steps".into()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: RunnerMsg = serde_json::from_str(&json).unwrap();
        let RunnerMsg::Log { step, phase, .. } = parsed else {
            panic!("expected Log");
        };
        assert_eq!(step, Some(2));
        assert_eq!(phase.as_deref(), Some("steps"));
    }

    #[test]
    fn old_shape_job_stage_still_parses() {
        // A job_stage emitted by a pre-step-progress runner: no `step`
        // object. It must default to None, not fail to parse.
        let json = format!(
            r#"{{"type":"job_stage","job_id":"{}","stage":"running","detail":null}}"#,
            Uuid::nil()
        );
        let parsed: RunnerMsg = serde_json::from_str(&json).unwrap();
        let RunnerMsg::JobStage { stage, step, .. } = parsed else {
            panic!("expected JobStage");
        };
        assert_eq!(stage, "running");
        assert_eq!(step, None);
    }

    #[test]
    fn job_stage_round_trips_step_progress() {
        let msg = RunnerMsg::JobStage {
            job_id: Uuid::nil(),
            stage: "running".into(),
            detail: None,
            step: Some(StepProgress {
                index: 1,
                total: 4,
                status: "failed".into(),
                exit_code: Some(2),
            }),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: RunnerMsg = serde_json::from_str(&json).unwrap();
        let RunnerMsg::JobStage { step, .. } = parsed else {
            panic!("expected JobStage");
        };
        assert_eq!(
            step,
            Some(StepProgress {
                index: 1,
                total: 4,
                status: "failed".into(),
                exit_code: Some(2),
            })
        );
    }

    #[test]
    fn step_progress_without_exit_code_still_parses() {
        let json = r#"{"index":0,"total":3,"status":"started"}"#;
        let parsed: StepProgress = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.exit_code, None);
        assert_eq!(parsed.status, "started");
    }
}
