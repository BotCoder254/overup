//! API shapes for the language-toolchain catalog (`GET …/toolchains`). The
//! catalog itself is static (`services::toolchain_images`); these DTOs project
//! it, tagging each entry with whether its `-latest` image is prewarmed and its
//! install state (pulled onto the hosted-runner daemon from the UI). Static
//! strings only — no user or runner text ever reaches here.

use serde::Serialize;

use crate::db::toolchain_images::InstalledToolchainRow;
use crate::services::toolchain_images;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolchainResponse {
    pub key: &'static str,
    pub label: &'static str,
    pub language: &'static str,
    pub description: &'static str,
    pub tools: Vec<&'static str>,
    pub image_latest: &'static str,
    pub image2204: &'static str,
    pub image2404: &'static str,
    /// Very large image (`full-*`) — the UI warns against prewarming it.
    pub large: bool,
    /// The `-latest` image is warmed on runner connect (env prepull OR install).
    pub prewarmed: bool,
    /// Install lifecycle: 'none' | 'pending' | 'installed' | 'failed'.
    pub install_status: &'static str,
    /// Static failure category when `install_status == "failed"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolchainsResponse {
    pub toolchains: Vec<ToolchainResponse>,
    /// Image a job falls back to (`DEFAULT_JOB_IMAGE`).
    pub default_image: String,
    /// Whether `RUNNER_IMAGE_ALLOWLIST` restricts which images may run.
    pub allowlist_enabled: bool,
    /// Whether install/uninstall is possible right now (the hosted-runner
    /// provisioner is configured AND its Docker daemon is reachable).
    pub install_supported: bool,
}

/// Normalize a stored status string onto the fixed vocabulary the UI expects.
fn status_str(raw: &str) -> &'static str {
    match raw {
        "pending" => "pending",
        "installed" => "installed",
        "failed" => "failed",
        _ => "none",
    }
}

impl ToolchainsResponse {
    /// Build the response from the static catalog plus deployment/install state.
    pub fn build(
        default_image: String,
        prepull: &[String],
        allowlist_enabled: bool,
        install_supported: bool,
        installed: &[InstalledToolchainRow],
    ) -> Self {
        let toolchains = toolchain_images::all()
            .iter()
            .map(|t| {
                let row = installed.iter().find(|r| r.toolchain_key == t.key);
                let install_status = row.map(|r| status_str(&r.status)).unwrap_or("none");
                // Warmed when in the env prepull list OR actively installed.
                let prewarmed = prepull.iter().any(|p| p == t.image_latest)
                    || matches!(install_status, "installed" | "pending");
                ToolchainResponse {
                    key: t.key,
                    label: t.label,
                    language: t.language,
                    description: t.description,
                    tools: t.tools.to_vec(),
                    image_latest: t.image_latest,
                    image2204: t.image_2204,
                    image2404: t.image_2404,
                    large: t.large,
                    prewarmed,
                    install_status,
                    install_error: row.and_then(|r| {
                        (install_status == "failed").then(|| r.error.clone()).flatten()
                    }),
                }
            })
            .collect();
        Self {
            toolchains,
            default_image,
            allowlist_enabled,
            install_supported,
        }
    }
}
