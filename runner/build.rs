//! Stamps the build's source revision into the binary as `RUNNER_BUILD_REF`.
//!
//! The runner reports it to the control plane in `hello` and logs it at
//! startup, so "is the deployed runner actually running the current code?"
//! is answerable from a log line. `CARGO_PKG_VERSION` alone cannot answer it:
//! it is a hardcoded `0.1.0`, identical in every build ever made, which is
//! exactly how a stale runner image goes unnoticed
//! (docs/fix-empty-workspace-stale-runner.md).
//!
//! Resolution order, and why both paths are needed:
//!   1. `OVERUP_BUILD_REF` — the Docker path. `.dockerignore` excludes `.git`,
//!      so git is NOT available inside the image build; `runner/Dockerfile`
//!      passes the ref in as a build arg instead.
//!   2. `git rev-parse --short HEAD` — the `cargo build` / systemd path.
//!   3. `"unknown"` — a tarball checkout with no git and no arg. Never fails
//!      the build: a missing stamp must not block compiling the runner.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=OVERUP_BUILD_REF");

    let build_ref = std::env::var("OVERUP_BUILD_REF")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(git_short_head)
        .unwrap_or_else(|| "unknown".to_string());

    // Rebuild when HEAD moves so the stamp cannot go stale in place. Both
    // paths are best-effort: outside a git checkout they simply do not exist.
    if let Some(git_dir) = git_dir() {
        println!("cargo:rerun-if-changed={git_dir}/HEAD");
        println!("cargo:rerun-if-changed={git_dir}/refs");
    }

    println!("cargo:rustc-env=RUNNER_BUILD_REF={build_ref}");
}

fn git_short_head() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn git_dir() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--absolute-git-dir"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!value.is_empty()).then_some(value)
}
