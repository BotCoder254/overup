//! API shapes for the language-toolchain catalog (`GET …/toolchains`). The
//! catalog itself is static (`services::toolchain_images`); these DTOs project
//! it, tagging each entry with whether its `-latest` image is prewarmed by the
//! deployment's `RUNNER_PREPULL_IMAGES`. Static strings only — no user or
//! runner text ever reaches here.

use serde::Serialize;

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
    /// The `-latest` image is in `RUNNER_PREPULL_IMAGES` (warmed on connect).
    pub prewarmed: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolchainsResponse {
    pub toolchains: Vec<ToolchainResponse>,
    /// Image a job falls back to (`DEFAULT_JOB_IMAGE`).
    pub default_image: String,
    /// Whether `RUNNER_IMAGE_ALLOWLIST` restricts which images may run.
    pub allowlist_enabled: bool,
}

impl ToolchainsResponse {
    /// Build the response from the static catalog plus the deployment's
    /// prepull list and default image.
    pub fn build(default_image: String, prepull: &[String], allowlist_enabled: bool) -> Self {
        let toolchains = toolchain_images::all()
            .iter()
            .map(|t| ToolchainResponse {
                key: t.key,
                label: t.label,
                language: t.language,
                description: t.description,
                tools: t.tools.to_vec(),
                image_latest: t.image_latest,
                image2204: t.image_2204,
                image2404: t.image_2404,
                large: t.large,
                prewarmed: prepull.iter().any(|p| p == t.image_latest),
            })
            .collect();
        Self {
            toolchains,
            default_image,
            allowlist_enabled,
        }
    }
}
