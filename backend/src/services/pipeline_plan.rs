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
    /// `{ image, env, steps: [{name, run, shell}], notices: [..] }`
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

        let image = container_image(&job_node)
            .or_else(|| image_for_runs_on(&job.runs_on))
            .unwrap_or_else(|| default_image.to_string());

        let mut env = root_env.clone();
        env.extend(env_map(job_node.get("env")));

        let mut steps = Vec::new();
        if let Some(uses) = &job.uses {
            notices.push(format!(
                "reusable workflow `{uses}` is not supported yet; job produces no steps"
            ));
        } else if let Some(list) = job_node.get("steps").and_then(Value::as_sequence) {
            for (index, step) in list.iter().enumerate() {
                if let Some(uses) = step.get("uses").and_then(Value::as_str) {
                    notices.push(format!("step `uses: {uses}` is not supported yet; skipped"));
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
                steps.push(serde_json::json!({
                    "name": name,
                    "run": run,
                    "shell": shell,
                }));
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
/// unrecognized falls through to the configured default image.
fn image_for_runs_on(runs_on: &[String]) -> Option<String> {
    for label in runs_on {
        let image = match label.as_str() {
            "ubuntu-latest" | "ubuntu-24.04" => Some("ubuntu:24.04"),
            "ubuntu-22.04" => Some("ubuntu:22.04"),
            "ubuntu-20.04" => Some("ubuntu:20.04"),
            _ => None,
        };
        if let Some(image) = image {
            return Some(image.to_string());
        }
    }
    None
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
        assert_eq!(build.plan["image"], "ubuntu:24.04");
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
        assert_eq!(test.plan["image"], "ubuntu:22.04");
        assert_eq!(test.plan["steps"][0]["shell"], "bash");
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
}
