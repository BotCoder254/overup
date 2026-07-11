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
    },
    /// One chunk of output. `seq` is runner-monotonic per job so the server
    /// can dedupe and browsers can detect gaps.
    Log {
        job_id: Uuid,
        seq: u64,
        stream: LogStream,
        text: String,
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
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"job_stage\""));
        let parsed: RunnerMsg = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, RunnerMsg::JobStage { .. }));
    }
}
