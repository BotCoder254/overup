//! Translate a stored workflow into executable per-job plans.
//!
//! Structure and DAG validation reuse `workflow_parse` (same budgets, same
//! cycle detection); this module then extracts the executable details —
//! container image, env, `run:` steps — into a JSON snapshot persisted on
//! each pipeline_jobs row, so a pipeline keeps executing the revision it was
//! created from even if the workflow is resynced or deleted.
//!
//! Deliberately unsupported this phase (recorded in the plan, surfaced in
//! the UI and job logs, never silently dropped): `uses:` steps and reusable
//! workflows, and matrix/`strategy` expansion (the job runs once).

use std::collections::BTreeMap;

use serde_yaml_ng::Value;

use crate::services::workflow_parse;

/// Everything the scheduler persists per job at pipeline creation.
pub struct PlannedJob {
    pub key: String,
    pub name: Option<String>,
    pub runs_on: Vec<String>,
    pub needs: Vec<String>,
    /// `{ image, env, steps: [{name, run, shell, workingDirectory?}],
    /// environment, notices }` — `workingDirectory` is present only when the
    /// step declares one (workspace root stays implicit).
    pub plan: serde_json::Value,
    pub position: i32,
}

pub enum PlanError {
    /// The workflow has validation errors and can never execute.
    Invalid,
    /// The workflow parses but defines no runnable jobs.
    NoRunnableJobs,
}

const MAX_STEP_RUN_BYTES: usize = 32 * 1024;
const MAX_ENV_VALUE_BYTES: usize = 8 * 1024;

/// Byte-cap a string without panicking inside a multibyte character.
pub fn truncate_utf8(value: &mut String, max_bytes: usize) {
    if value.len() <= max_bytes {
        return;
    }
    let mut cut = max_bytes;
    while cut > 0 && !value.is_char_boundary(cut) {
        cut -= 1;
    }
    value.truncate(cut);
}

/// Build executable plans from raw workflow YAML.
pub fn build_plans(raw_content: &str, default_image: &str) -> Result<Vec<PlannedJob>, PlanError> {
    let parsed = workflow_parse::parse_and_validate(raw_content);
    if parsed.status() == "errors" {
        return Err(PlanError::Invalid);
    }
    if parsed.jobs.is_empty() {
        return Err(PlanError::NoRunnableJobs);
    }

    // Second pass over the (already budget-checked) document for the
    // executable details workflow_parse doesn't normalize.
    let doc: Value = serde_yaml_ng::from_str(raw_content).unwrap_or(Value::Null);
    let root_env = env_map(doc.get("env"));

    let mut plans = Vec::with_capacity(parsed.jobs.len());
    for job in &parsed.jobs {
        let job_node = doc
            .get("jobs")
            .and_then(|jobs| jobs.get(job.key.as_str()))
            .cloned()
            .unwrap_or(Value::Null);

        let mut notices: Vec<String> = Vec::new();
        if job.strategy.is_some() {
            notices.push("matrix strategy is not expanded yet; the job runs once".into());
        }

        // `container:` wins, but only when it is a plausible image reference
        // — the string lands verbatim in the signed job payload, so garbage
        // (whitespace, control chars, oversized values) is rejected here
        // with a visible notice rather than shipped to a runner.
        let image = match container_image(&job_node) {
            Some(reference) if is_valid_image_reference(&reference) => reference,
            Some(reference) => {
                notices.push(format!(
                    "container image `{}` is not a valid image reference; using the image for `runs-on` instead",
                    sanitize_for_notice(&reference)
                ));
                image_for_runs_on(&job.runs_on).unwrap_or_else(|| default_image.to_string())
            }
            None => image_for_runs_on(&job.runs_on).unwrap_or_else(|| default_image.to_string()),
        };

        let mut env = root_env.clone();
        env.extend(env_map(job_node.get("env")));

        // GitHub precedence: step `working-directory` > job
        // `defaults.run.working-directory` > workflow `defaults.run.
        // working-directory`. Invalid values fall back to the workspace root
        // with a visible notice; the value itself never ships unvalidated.
        let job_default_workdir = match defaults_working_directory(&job_node)
            .or_else(|| defaults_working_directory(&doc))
        {
            None => None,
            Some(raw) => match plan_working_directory(raw) {
                Ok(value) => Some(value),
                Err(bad) => {
                    push_workdir_notice(&mut notices, &bad);
                    None
                }
            },
        };

        let mut steps = Vec::new();
        if let Some(uses) = &job.uses {
            notices.push(format!(
                "reusable workflow `{uses}` is not supported yet; job produces no steps"
            ));
        } else if let Some(list) = job_node.get("steps").and_then(Value::as_sequence) {
            for (index, step) in list.iter().enumerate() {
                if let Some(uses) = step.get("uses").and_then(Value::as_str) {
                    let uses = sanitize_for_notice(uses);
                    if is_checkout_action(&uses) {
                        // Checkout is NOT skipped — the runner fetches the
                        // repository automatically before the job runs. Saying
                        // "not supported; skipped" wrongly implies no source.
                        notices.push(format!(
                            "step `uses: {uses}` is a no-op here — your repository is checked out automatically before the job runs"
                        ));
                    } else if let Some(hint) = setup_action_hint(&uses) {
                        notices.push(format!(
                            "step `uses: {uses}` is not executed; the default job image ships common toolchains — pin this one by adding `{hint}` to the job"
                        ));
                    } else {
                        notices
                            .push(format!("step `uses: {uses}` is not supported yet; skipped"));
                    }
                    continue;
                }
                let Some(run) = step.get("run").and_then(Value::as_str) else {
                    continue;
                };
                let mut run = run.to_string();
                truncate_utf8(&mut run, MAX_STEP_RUN_BYTES);
                let name = step
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("step {}", index + 1));
                let shell = match step.get("shell").and_then(Value::as_str) {
                    Some("bash") => "bash",
                    _ => "sh",
                };
                let working_directory = match step.get("working-directory").and_then(Value::as_str)
                {
                    Some(raw) => match plan_working_directory(raw) {
                        Ok(value) => Some(value),
                        Err(bad) => {
                            push_workdir_notice(&mut notices, &bad);
                            None
                        }
                    },
                    None => job_default_workdir.clone(),
                };
                let mut step_json = serde_json::json!({
                    "name": name,
                    "run": run,
                    "shell": shell,
                });
                if let Some(workdir) = working_directory {
                    step_json["workingDirectory"] = serde_json::Value::String(workdir);
                }
                steps.push(step_json);
            }
        }

        plans.push(PlannedJob {
            key: job.key.clone(),
            name: job.name.clone(),
            runs_on: job.runs_on.clone(),
            needs: job.needs.clone(),
            plan: serde_json::json!({
                "image": image,
                "env": env,
                "steps": steps,
                // Name only — the scheduler resolves it to an environment
                // (and its secrets) live at dispatch, so rotated values and
                // renames apply to reruns automatically.
                "environment": job.environment,
                "notices": notices,
            }),
            position: job.position,
        });
    }

    if plans.iter().all(|p| {
        p.plan["steps"]
            .as_array()
            .map(|s| s.is_empty())
            .unwrap_or(true)
    }) {
        return Err(PlanError::NoRunnableJobs);
    }

    Ok(plans)
}

/// String scalars only; everything else is stringified conservatively.
/// Values are length-capped so a hostile workflow can't bloat the payload.
fn env_map(node: Option<&Value>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(map) = node.and_then(Value::as_mapping) else {
        return out;
    };
    for (key, value) in map {
        let Some(key) = key.as_str() else { continue };
        let mut value = match value {
            Value::String(s) => s.clone(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            _ => continue,
        };
        truncate_utf8(&mut value, MAX_ENV_VALUE_BYTES);
        out.insert(key.to_string(), value);
    }
    out
}

/// `container:` as a plain string or `container: { image: ... }`.
fn container_image(job_node: &Value) -> Option<String> {
    let container = job_node.get("container")?;
    match container {
        Value::String(image) => Some(image.clone()),
        Value::Mapping(_) => container
            .get("image")
            .and_then(Value::as_str)
            .map(str::to_string),
        _ => None,
    }
}

/// Conservative `runs-on` → image map for GitHub-style labels; anything
/// unrecognized falls through to the configured default image. The targets
/// are the `catthehacker/ubuntu:act-*` family — the GitHub-runner-compatible
/// medium images nektos/act and Gitea Actions default to (Node/npm/yarn,
/// Python, git, build-essential, …) — because bare `ubuntu:*` images have no
/// toolchains and every real workflow immediately fails with `npm: not
/// found`. Heavier toolchains pin an image per job via `container:`.
fn image_for_runs_on(runs_on: &[String]) -> Option<String> {
    for label in runs_on {
        let image = match label.trim().to_ascii_lowercase().as_str() {
            "ubuntu-latest" => Some("catthehacker/ubuntu:act-latest"),
            "ubuntu-24.04" => Some("catthehacker/ubuntu:act-24.04"),
            "ubuntu-22.04" => Some("catthehacker/ubuntu:act-22.04"),
            "ubuntu-20.04" => Some("catthehacker/ubuntu:act-20.04"),
            _ => None,
        };
        if let Some(image) = image {
            return Some(image.to_string());
        }
    }
    None
}

/// A plausible Docker image reference: registry/repo/name plus optional
/// tag/digest. Deliberately conservative — start alphanumeric, then only the
/// reference charset, hard length cap. Anything else is rejected at plan
/// time so the signed job payload never carries an arbitrary string.
fn is_valid_image_reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | ':' | '@'))
}

/// Notices land in the plan JSONB and the UI: strip control characters and
/// cap the echoed fragment so a hostile workflow can't bloat or garble them.
fn sanitize_for_notice(value: &str) -> String {
    value.chars().filter(|c| !c.is_control()).take(100).collect()
}

/// The `defaults.run.working-directory` string under a workflow root or job
/// node, if present. Non-string values read as absent — the parser already
/// emitted a warning for them.
fn defaults_working_directory(node: &Value) -> Option<&str> {
    node.get("defaults")?
        .get("run")?
        .get("working-directory")?
        .as_str()
}

/// Classify a `working-directory` value for plan emission. `Ok` is the value
/// to persist: a normalized static path, or a `${{ … }}` expression carried
/// verbatim (the scheduler substitutes and re-validates it at dispatch).
/// `Err` carries the sanitized fragment for a notice — the step falls back
/// to the workspace root and the raw value never ships.
fn plan_working_directory(raw: &str) -> Result<String, String> {
    if raw.contains("${{") {
        // Never truncate a path — a cut expression changes meaning.
        if raw.len() <= protocol::MAX_RELATIVE_PATH_BYTES && !raw.chars().any(char::is_control) {
            return Ok(raw.to_string());
        }
        return Err(sanitize_for_notice(raw));
    }
    protocol::normalize_relative_path(raw).ok_or_else(|| sanitize_for_notice(raw))
}

/// One notice per distinct rejected working-directory value per job — a bad
/// job-level default must not repeat for every step.
fn push_workdir_notice(notices: &mut Vec<String>, bad: &str) {
    let notice = format!(
        "working-directory `{bad}` is not a supported workspace-relative path; \
         the affected steps run at the workspace root"
    );
    if !notices.contains(&notice) {
        notices.push(notice);
    }
}

/// Whether a `uses:` step is a repository checkout. These are a no-op on
/// overup because the runner checks the repository out automatically before
/// the job runs — so the notice must not imply the source is missing.
fn is_checkout_action(uses: &str) -> bool {
    let action = uses.split('@').next().unwrap_or(uses);
    matches!(action, "actions/checkout")
}

/// Well-known `setup-*` actions mapped to the per-job `container:` image
/// that provides the same toolchain. `uses:` steps are not executed on
/// overup, so the notice points authors at the mechanism that is.
fn setup_action_hint(uses: &str) -> Option<&'static str> {
    let action = uses.split('@').next().unwrap_or(uses);
    match action {
        "actions/setup-node" => Some("container: node:22"),
        "actions/setup-python" => Some("container: python:3.12"),
        "actions/setup-go" => Some("container: golang:1.23"),
        "actions/setup-java" => Some("container: eclipse-temurin:21"),
        "actions/setup-dotnet" => Some("container: mcr.microsoft.com/dotnet/sdk:8.0"),
        "ruby/setup-ruby" => Some("container: ruby:3.3"),
        "dtolnay/rust-toolchain"
        | "actions-rust-lang/setup-rust-toolchain"
        | "actions-rs/toolchain" => Some("container: rust:1"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE: &str = r#"
name: CI
on: push
env:
  CI: "true"
jobs:
  build:
    runs-on: ubuntu-latest
    env:
      MODE: release
    steps:
      - name: compile
        run: echo compiling
      - uses: actions/checkout@v4
      - run: echo done
  test:
    needs: build
    runs-on: ubuntu-22.04
    steps:
      - run: echo testing
        shell: bash
"#;

    #[test]
    fn plans_extract_steps_env_and_image() {
        let plans = build_plans(SIMPLE, "ubuntu:24.04").ok().unwrap();
        assert_eq!(plans.len(), 2);

        let build = &plans[0];
        assert_eq!(build.key, "build");
        assert_eq!(build.plan["image"], "catthehacker/ubuntu:act-latest");
        assert_eq!(build.plan["env"]["CI"], "true");
        assert_eq!(build.plan["env"]["MODE"], "release");
        let steps = build.plan["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 2); // the uses: step is skipped
        assert_eq!(steps[0]["name"], "compile");
        assert_eq!(steps[1]["shell"], "sh");
        let notices = build.plan["notices"].as_array().unwrap();
        assert!(notices.iter().any(|n| n.as_str().unwrap().contains("uses")));

        let test = &plans[1];
        assert_eq!(test.needs, vec!["build".to_string()]);
        assert_eq!(test.plan["image"], "catthehacker/ubuntu:act-22.04");
        assert_eq!(test.plan["steps"][0]["shell"], "bash");
    }

    #[test]
    fn working_directory_precedence_is_step_over_job_over_workflow() {
        let workflow = r#"
on: push
defaults:
  run:
    working-directory: root-dir
jobs:
  a:
    runs-on: ubuntu-latest
    defaults:
      run:
        working-directory: ./job-dir/
    steps:
      - run: echo job default
      - run: echo step override
        working-directory: step-dir
  b:
    runs-on: ubuntu-latest
    steps:
      - run: echo workflow default
"#;
        let plans = build_plans(workflow, "ubuntu:24.04").ok().unwrap();
        let a = plans[0].plan["steps"].as_array().unwrap();
        // Job default (normalized: `./` and trailing `/` stripped).
        assert_eq!(a[0]["workingDirectory"], "job-dir");
        assert_eq!(a[1]["workingDirectory"], "step-dir");
        let b = plans[1].plan["steps"].as_array().unwrap();
        assert_eq!(b[0]["workingDirectory"], "root-dir");
    }

    #[test]
    fn absent_working_directory_emits_no_key() {
        let plans = build_plans(SIMPLE, "ubuntu:24.04").ok().unwrap();
        let step = &plans[0].plan["steps"][0];
        assert!(step.get("workingDirectory").is_none());
        // Exact step shape stays pinned for plans without a workdir.
        assert_eq!(
            step.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["name", "run", "shell"]
        );
    }

    #[test]
    fn invalid_working_directory_becomes_a_notice_not_a_field() {
        let workflow = r#"
on: push
jobs:
  a:
    runs-on: ubuntu-latest
    defaults:
      run:
        working-directory: ../escape
    steps:
      - run: echo one
      - run: echo two
"#;
        let plans = build_plans(workflow, "ubuntu:24.04").ok().unwrap();
        let steps = plans[0].plan["steps"].as_array().unwrap();
        assert!(steps.iter().all(|s| s.get("workingDirectory").is_none()));
        let notices = plans[0].plan["notices"].as_array().unwrap();
        let workdir_notices: Vec<_> = notices
            .iter()
            .filter(|n| n.as_str().unwrap().contains("workspace-relative"))
            .collect();
        // Deduped: one notice for the shared bad job default, not per step.
        assert_eq!(workdir_notices.len(), 1);
    }

    #[test]
    fn expression_working_directory_ships_verbatim() {
        let workflow = r#"
on: push
jobs:
  a:
    runs-on: ubuntu-latest
    steps:
      - run: echo hi
        working-directory: ${{ vars.TARGET_DIR }}
"#;
        let plans = build_plans(workflow, "ubuntu:24.04").ok().unwrap();
        assert_eq!(
            plans[0].plan["steps"][0]["workingDirectory"],
            "${{ vars.TARGET_DIR }}"
        );
    }

    #[test]
    fn environment_snapshots_into_plan() {
        // GitHub's two forms: bare string and map-with-name.
        let with_env = r#"
on: push
jobs:
  deploy:
    runs-on: ubuntu-latest
    environment: production
    steps: [{ run: echo deploy }]
  stage:
    runs-on: ubuntu-latest
    environment:
      name: staging
      url: https://stage.example.com
    steps: [{ run: echo stage }]
  plain:
    runs-on: ubuntu-latest
    steps: [{ run: echo plain }]
"#;
        let plans = build_plans(with_env, "ubuntu:24.04").ok().unwrap();
        assert_eq!(plans[0].plan["environment"], "production");
        assert_eq!(plans[1].plan["environment"], "staging");
        assert!(plans[2].plan["environment"].is_null());
    }

    #[test]
    fn malformed_environment_is_a_warning_not_an_error() {
        let malformed = r#"
on: push
jobs:
  deploy:
    runs-on: ubuntu-latest
    environment: [not, a, string]
    steps: [{ run: echo deploy }]
"#;
        // Still plans (warning severity), with no environment captured.
        let plans = build_plans(malformed, "ubuntu:24.04").ok().unwrap();
        assert!(plans[0].plan["environment"].is_null());
    }

    #[test]
    fn cyclic_workflow_is_rejected() {
        let cyclic = r#"
on: push
jobs:
  a:
    runs-on: ubuntu-latest
    needs: b
    steps: [{ run: echo a }]
  b:
    runs-on: ubuntu-latest
    needs: a
    steps: [{ run: echo b }]
"#;
        assert!(matches!(
            build_plans(cyclic, "ubuntu:24.04"),
            Err(PlanError::Invalid)
        ));
    }

    #[test]
    fn uses_only_workflow_has_no_runnable_jobs() {
        let uses_only = r#"
on: push
jobs:
  deploy:
    uses: org/repo/.github/workflows/deploy.yml@main
"#;
        assert!(matches!(
            build_plans(uses_only, "ubuntu:24.04"),
            Err(PlanError::NoRunnableJobs)
        ));
    }

    #[test]
    fn container_image_wins_over_runs_on() {
        let with_container = r#"
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    container: node:22
    steps: [{ run: node --version }]
"#;
        let plans = build_plans(with_container, "ubuntu:24.04").ok().unwrap();
        assert_eq!(plans[0].plan["image"], "node:22");
    }

    #[test]
    fn invalid_container_image_falls_back_with_notice() {
        let bad_container = r#"
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    container: "node:22 && rm -rf /"
    steps: [{ run: node --version }]
"#;
        let plans = build_plans(bad_container, "ubuntu:24.04").ok().unwrap();
        assert_eq!(plans[0].plan["image"], "catthehacker/ubuntu:act-latest");
        let notices = plans[0].plan["notices"].as_array().unwrap();
        assert!(
            notices
                .iter()
                .any(|n| n.as_str().unwrap().contains("not a valid image reference"))
        );
    }

    #[test]
    fn image_reference_validation() {
        assert!(is_valid_image_reference("node:22"));
        assert!(is_valid_image_reference("rust:1"));
        assert!(is_valid_image_reference("catthehacker/ubuntu:act-latest"));
        assert!(is_valid_image_reference(
            "ghcr.io/org/image:tag@sha256:0123456789abcdef"
        ));
        assert!(!is_valid_image_reference(""));
        assert!(!is_valid_image_reference("node:22 extra"));
        assert!(!is_valid_image_reference("-leading-dash"));
        assert!(!is_valid_image_reference("bad\nimage"));
        assert!(!is_valid_image_reference(&"a".repeat(257)));
    }

    #[test]
    fn runs_on_image_map_is_case_insensitive() {
        assert_eq!(
            image_for_runs_on(&["Ubuntu-Latest".to_string()]).as_deref(),
            Some("catthehacker/ubuntu:act-latest")
        );
        assert_eq!(image_for_runs_on(&["windows-latest".to_string()]), None);
    }

    #[test]
    fn setup_actions_get_container_hints() {
        let with_setup = r#"
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/setup-node@v4
      - run: npm test
"#;
        let plans = build_plans(with_setup, "ubuntu:24.04").ok().unwrap();
        let notices = plans[0].plan["notices"].as_array().unwrap();
        assert!(
            notices
                .iter()
                .any(|n| n.as_str().unwrap().contains("container: node:22"))
        );
        assert_eq!(setup_action_hint("dtolnay/rust-toolchain@stable"), Some("container: rust:1"));
        assert_eq!(setup_action_hint("actions/checkout@v4"), None);
    }

    #[test]
    fn checkout_notice_is_reassuring_not_a_skip_warning() {
        // The SIMPLE workflow uses `actions/checkout@v4`; its notice must say
        // the repo is checked out automatically, NOT "not supported; skipped".
        let plans = build_plans(SIMPLE, "ubuntu:24.04").ok().unwrap();
        let notices = plans[0].plan["notices"].as_array().unwrap();
        let checkout = notices
            .iter()
            .filter_map(|n| n.as_str())
            .find(|n| n.contains("actions/checkout@v4"))
            .expect("a checkout notice");
        assert!(checkout.contains("checked out automatically"), "{checkout}");
        assert!(!checkout.contains("not supported"), "{checkout}");

        assert!(is_checkout_action("actions/checkout@v4"));
        assert!(!is_checkout_action("actions/setup-node@v4"));
    }
}
