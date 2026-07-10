//! Safe, deterministic parsing and validation of GitHub Actions workflow
//! YAML. Nothing here executes expressions or shell — files are parsed into
//! plain data, walked under strict resource budgets, and reduced to
//! normalized metadata plus a diagnostics list. Parse failures are
//! diagnostics, never 500s. Shared verbatim by repository sync and the
//! interactive /validate endpoint so the editor sees exactly what the
//! catalog stores.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use serde_yaml_ng::{Mapping, Value};

use super::github_app::MAX_WORKFLOW_FILE_BYTES;

/// Resource budgets: a workflow file that exceeds these is hostile or
/// broken, not "large". They bound both memory and walk time.
const MAX_NODES: usize = 20_000;
const MAX_DEPTH: usize = 32;
pub const MAX_JOBS: usize = 100;
const MAX_STEPS_PER_JOB: usize = 100;

/// Top-level keys GitHub's workflow schema defines; anything else is a
/// warning (typo detection), never a hard failure.
const KNOWN_TOP_LEVEL_KEYS: &[&str] = &[
    "name",
    "run-name",
    "on",
    "permissions",
    "env",
    "defaults",
    "concurrency",
    "jobs",
];

const KNOWN_JOB_KEYS: &[&str] = &[
    "name",
    "permissions",
    "needs",
    "if",
    "runs-on",
    "environment",
    "concurrency",
    "outputs",
    "env",
    "defaults",
    "steps",
    "timeout-minutes",
    "strategy",
    "continue-on-error",
    "container",
    "services",
    "uses",
    "with",
    "secrets",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

impl Diagnostic {
    fn error(message: impl Into<String>, path: Option<String>) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
            path,
            line: None,
        }
    }

    fn warning(message: impl Into<String>, path: Option<String>) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
            path,
            line: None,
        }
    }
}

#[derive(Debug)]
pub struct ParsedJob {
    pub key: String,
    pub name: Option<String>,
    pub runs_on: Vec<String>,
    pub needs: Vec<String>,
    pub uses: Option<String>,
    pub strategy: Option<serde_json::Value>,
    pub step_count: i32,
    pub position: i32,
}

#[derive(Debug)]
pub struct ParsedWorkflow {
    /// YAML `name`, if present; callers fall back to the file name.
    pub name: Option<String>,
    pub triggers: Vec<String>,
    pub jobs: Vec<ParsedJob>,
    /// Normalized extras: permissions, concurrency, env keys, secret refs.
    pub metadata: serde_json::Value,
    pub diagnostics: Vec<Diagnostic>,
}

impl ParsedWorkflow {
    /// valid | warnings | errors — matches the workflows.validation_status
    /// CHECK constraint.
    pub fn status(&self) -> &'static str {
        if self
            .diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
        {
            "errors"
        } else if !self.diagnostics.is_empty() {
            "warnings"
        } else {
            "valid"
        }
    }

    fn failed(diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            name: None,
            triggers: Vec::new(),
            jobs: Vec::new(),
            metadata: serde_json::json!({}),
            diagnostics,
        }
    }
}

/// Parse and validate one workflow file. Total: every input produces a
/// `ParsedWorkflow`, with problems reported as diagnostics.
pub fn parse_and_validate(content: &str) -> ParsedWorkflow {
    if content.len() > MAX_WORKFLOW_FILE_BYTES {
        return ParsedWorkflow::failed(vec![Diagnostic::error(
            format!("workflow file exceeds the {MAX_WORKFLOW_FILE_BYTES} byte limit"),
            None,
        )]);
    }

    let value: Value = match serde_yaml_ng::from_str(content) {
        Ok(value) => value,
        Err(err) => {
            // serde_yaml_ng rejects duplicate mapping keys and malformed
            // syntax here; its message is safe (derived from the document,
            // which the caller already holds).
            let line = err.location().map(|l| l.line() as u32);
            return ParsedWorkflow::failed(vec![Diagnostic {
                severity: Severity::Error,
                message: format!("YAML parse error: {err}"),
                path: None,
                line,
            }]);
        }
    };

    if let Err(diagnostic) = check_budget(&value) {
        return ParsedWorkflow::failed(vec![diagnostic]);
    }

    let mut diagnostics = Vec::new();

    let Some(root) = value.as_mapping() else {
        return ParsedWorkflow::failed(vec![Diagnostic::error(
            "workflow root must be a mapping",
            None,
        )]);
    };

    for key in root.keys() {
        if let Some(key) = key_name(key)
            && !KNOWN_TOP_LEVEL_KEYS.contains(&key.as_str())
        {
            diagnostics.push(Diagnostic::warning(
                format!("unknown top-level key `{key}`"),
                Some(key.clone()),
            ));
        }
    }

    let name = get(root, "name")
        .and_then(Value::as_str)
        .map(|s| s.chars().take(200).collect::<String>());

    let triggers = extract_triggers(root, &mut diagnostics);
    let jobs = extract_jobs(root, &mut diagnostics);
    validate_needs(&jobs, &mut diagnostics);

    let metadata = serde_json::json!({
        "permissions": get(root, "permissions").map(value_to_json),
        "concurrency": get(root, "concurrency").map(value_to_json),
        "envKeys": get(root, "env")
            .and_then(Value::as_mapping)
            .map(|m| m.keys().filter_map(key_name).collect::<Vec<_>>())
            .unwrap_or_default(),
        "secretRefs": scan_secret_refs(content),
    });

    ParsedWorkflow {
        name,
        triggers,
        jobs,
        metadata,
        diagnostics,
    }
}

/// Iterative walk enforcing the node budget and depth cap — the layered
/// defense against alias/anchor expansion bombs on top of the parser's own
/// recursion limit.
fn check_budget(value: &Value) -> Result<(), Diagnostic> {
    let mut stack: Vec<(&Value, usize)> = vec![(value, 0)];
    let mut nodes = 0usize;
    while let Some((value, depth)) = stack.pop() {
        nodes += 1;
        if nodes > MAX_NODES {
            return Err(Diagnostic::error(
                format!("workflow document exceeds the {MAX_NODES} node budget"),
                None,
            ));
        }
        if depth > MAX_DEPTH {
            return Err(Diagnostic::error(
                format!("workflow document exceeds the nesting depth limit of {MAX_DEPTH}"),
                None,
            ));
        }
        match value {
            Value::Sequence(seq) => stack.extend(seq.iter().map(|v| (v, depth + 1))),
            Value::Mapping(map) => {
                for (k, v) in map {
                    stack.push((k, depth + 1));
                    stack.push((v, depth + 1));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// The `on` key. YAML 1.1 resolves the unquoted scalar `on` to boolean
/// `true`, so the key may arrive as either a string or a bool.
fn extract_triggers(root: &Mapping, diagnostics: &mut Vec<Diagnostic>) -> Vec<String> {
    let on = root
        .get(Value::String("on".into()))
        .or_else(|| root.get(Value::Bool(true)));

    let Some(on) = on else {
        diagnostics.push(Diagnostic::error(
            "workflow has no `on` trigger definition",
            Some("on".into()),
        ));
        return Vec::new();
    };

    let mut triggers: Vec<String> = match on {
        Value::String(event) => vec![event.clone()],
        Value::Sequence(events) => events
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Value::Mapping(events) => events.keys().filter_map(key_name).collect(),
        _ => Vec::new(),
    };
    triggers.retain(|t| is_valid_identifier(t));
    triggers.truncate(50);

    if triggers.is_empty() {
        diagnostics.push(Diagnostic::error(
            "`on` defines no valid trigger events",
            Some("on".into()),
        ));
    }
    triggers
}

fn extract_jobs(root: &Mapping, diagnostics: &mut Vec<Diagnostic>) -> Vec<ParsedJob> {
    let Some(jobs_value) = get(root, "jobs") else {
        diagnostics.push(Diagnostic::error(
            "workflow has no `jobs` section",
            Some("jobs".into()),
        ));
        return Vec::new();
    };
    let Some(jobs_map) = jobs_value.as_mapping() else {
        diagnostics.push(Diagnostic::error(
            "`jobs` must be a mapping of job ids",
            Some("jobs".into()),
        ));
        return Vec::new();
    };
    if jobs_map.is_empty() {
        diagnostics.push(Diagnostic::error(
            "`jobs` defines no jobs",
            Some("jobs".into()),
        ));
        return Vec::new();
    }
    if jobs_map.len() > MAX_JOBS {
        diagnostics.push(Diagnostic::error(
            format!("workflow defines more than {MAX_JOBS} jobs"),
            Some("jobs".into()),
        ));
        return Vec::new();
    }

    let mut jobs = Vec::with_capacity(jobs_map.len());
    for (position, (key, body)) in jobs_map.iter().enumerate() {
        let Some(job_key) = key_name(key) else {
            diagnostics.push(Diagnostic::error(
                "job id must be a string",
                Some("jobs".into()),
            ));
            continue;
        };
        let path = format!("jobs.{job_key}");

        // GitHub: must start with a letter or `_`, then alphanumerics,
        // `-`, or `_`.
        if !is_valid_job_id(&job_key) {
            diagnostics.push(Diagnostic::error(
                format!("invalid job id `{job_key}`"),
                Some(path.clone()),
            ));
            continue;
        }

        let Some(job) = body.as_mapping() else {
            diagnostics.push(Diagnostic::error(
                format!("job `{job_key}` must be a mapping"),
                Some(path),
            ));
            continue;
        };

        for key in job.keys() {
            if let Some(key) = key_name(key)
                && !KNOWN_JOB_KEYS.contains(&key.as_str())
            {
                diagnostics.push(Diagnostic::warning(
                    format!("unknown key `{key}` in job `{job_key}`"),
                    Some(format!("{path}.{key}")),
                ));
            }
        }

        let uses = get(job, "uses").and_then(Value::as_str).map(str::to_string);
        let steps = get(job, "steps").and_then(Value::as_sequence);
        let step_count = steps.map(|s| s.len()).unwrap_or(0);

        if uses.is_none() && steps.is_none() {
            diagnostics.push(Diagnostic::error(
                format!("job `{job_key}` has neither `steps` nor `uses`"),
                Some(path.clone()),
            ));
        }
        if uses.is_some() && steps.is_some() {
            diagnostics.push(Diagnostic::error(
                format!("job `{job_key}` cannot combine `uses` with `steps`"),
                Some(path.clone()),
            ));
        }
        if step_count > MAX_STEPS_PER_JOB {
            diagnostics.push(Diagnostic::error(
                format!("job `{job_key}` exceeds {MAX_STEPS_PER_JOB} steps"),
                Some(format!("{path}.steps")),
            ));
        }
        if uses.is_none() && get(job, "runs-on").is_none() {
            diagnostics.push(Diagnostic::warning(
                format!("job `{job_key}` does not declare `runs-on`"),
                Some(format!("{path}.runs-on")),
            ));
        }

        let strategy = get(job, "strategy").map(value_to_json);
        if let Some(matrix) = get(job, "strategy")
            .and_then(Value::as_mapping)
            .and_then(|s| get(s, "matrix"))
            && matrix.as_mapping().is_none()
            && matrix.as_str().is_none()
        {
            diagnostics.push(Diagnostic::error(
                format!("job `{job_key}` has a malformed `strategy.matrix`"),
                Some(format!("{path}.strategy.matrix")),
            ));
        }

        jobs.push(ParsedJob {
            name: get(job, "name")
                .and_then(Value::as_str)
                .map(|s| s.chars().take(200).collect()),
            runs_on: string_or_list(get(job, "runs-on")),
            needs: string_or_list(get(job, "needs")),
            uses,
            strategy,
            step_count: step_count as i32,
            position: position as i32,
            key: job_key,
        });
    }
    jobs
}

/// Unresolved `needs` references and dependency cycles (iterative DFS).
fn validate_needs(jobs: &[ParsedJob], diagnostics: &mut Vec<Diagnostic>) {
    let ids: HashSet<&str> = jobs.iter().map(|j| j.key.as_str()).collect();
    let mut edges: HashMap<&str, Vec<&str>> = HashMap::new();

    for job in jobs {
        for need in &job.needs {
            if job.key == *need {
                diagnostics.push(Diagnostic::error(
                    format!("job `{}` depends on itself", job.key),
                    Some(format!("jobs.{}.needs", job.key)),
                ));
            } else if !ids.contains(need.as_str()) {
                diagnostics.push(Diagnostic::error(
                    format!("job `{}` needs unknown job `{need}`", job.key),
                    Some(format!("jobs.{}.needs", job.key)),
                ));
            } else {
                edges.entry(job.key.as_str()).or_default().push(need);
            }
        }
    }

    // Iterative three-color DFS: 1 = on stack, 2 = done.
    let mut color: HashMap<&str, u8> = HashMap::new();
    for job in jobs {
        if color.contains_key(job.key.as_str()) {
            continue;
        }
        let mut stack: Vec<(&str, bool)> = vec![(job.key.as_str(), false)];
        while let Some((node, leaving)) = stack.pop() {
            if leaving {
                color.insert(node, 2);
                continue;
            }
            match color.get(node) {
                Some(2) => continue,
                Some(1) => {
                    diagnostics.push(Diagnostic::error(
                        format!("circular `needs` dependency involving job `{node}`"),
                        Some(format!("jobs.{node}.needs")),
                    ));
                    return; // one cycle report is enough
                }
                _ => {}
            }
            color.insert(node, 1);
            stack.push((node, true));
            if let Some(next) = edges.get(node) {
                for &n in next {
                    if color.get(n) != Some(&2) {
                        stack.push((n, false));
                    }
                }
            }
        }
    }
}

/// Names of secrets referenced as `${{ secrets.NAME }}` — metadata only,
/// never values. Hand-rolled scan; no expression evaluation.
pub fn scan_secret_refs(content: &str) -> Vec<String> {
    let mut refs: Vec<String> = Vec::new();
    let mut rest = content;
    while let Some(start) = rest.find("${{") {
        rest = &rest[start + 3..];
        let Some(end) = rest.find("}}") else { break };
        let expr = &rest[..end];
        let mut scan = expr;
        while let Some(pos) = scan.find("secrets.") {
            let after = &scan[pos + "secrets.".len()..];
            let name: String = after
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() && name != "GITHUB_TOKEN" && !refs.contains(&name) {
                refs.push(name);
            }
            scan = after;
        }
        rest = &rest[end + 2..];
        if refs.len() >= 100 {
            break;
        }
    }
    refs.sort();
    refs
}

fn get<'a>(map: &'a Mapping, key: &str) -> Option<&'a Value> {
    map.get(Value::String(key.to_string()))
}

/// Mapping keys as display strings; bools included because of the YAML 1.1
/// `on` quirk.
fn key_name(key: &Value) -> Option<String> {
    match key {
        Value::String(s) => Some(s.clone()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn string_or_list(value: Option<&Value>) -> Vec<String> {
    let mut out = match value {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Sequence(seq)) => seq
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    };
    out.retain(|s| !s.is_empty() && s.len() <= 200);
    out.truncate(50);
    out
}

fn is_valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn is_valid_job_id(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && value.len() <= 100
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Lossy YAML → JSON for storing strategy/permissions/concurrency blobs.
fn value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Number(n) => serde_json::to_value(n).unwrap_or(serde_json::Value::Null),
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::Sequence(seq) => serde_json::Value::Array(seq.iter().map(value_to_json).collect()),
        Value::Mapping(map) => serde_json::Value::Object(
            map.iter()
                .filter_map(|(k, v)| key_name(k).map(|k| (k, value_to_json(v))))
                .collect(),
        ),
        Value::Tagged(tagged) => value_to_json(&tagged.value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_valid_workflow() {
        let parsed = parse_and_validate(
            r#"
name: CI
on:
  push:
    branches: [main]
  pull_request:
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo test
  deploy:
    needs: build
    runs-on: ubuntu-latest
    steps:
      - run: echo deploy ${{ secrets.DEPLOY_KEY }}
"#,
        );
        assert_eq!(parsed.status(), "valid");
        assert_eq!(parsed.name.as_deref(), Some("CI"));
        assert_eq!(parsed.triggers, vec!["push", "pull_request"]);
        assert_eq!(parsed.jobs.len(), 2);
        assert_eq!(parsed.jobs[1].needs, vec!["build"]);
        assert_eq!(parsed.metadata["secretRefs"][0], "DEPLOY_KEY");
    }

    #[test]
    fn handles_yaml_11_on_bool_key() {
        // Unquoted `on` resolves to boolean true in YAML 1.1.
        let parsed = parse_and_validate("on: push\njobs:\n  a:\n    steps: []\n");
        assert_eq!(parsed.triggers, vec!["push"]);
    }

    #[test]
    fn reports_circular_needs() {
        let parsed = parse_and_validate(
            "on: push\njobs:\n  a:\n    needs: b\n    steps: []\n  b:\n    needs: a\n    steps: []\n",
        );
        assert_eq!(parsed.status(), "errors");
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|d| d.message.contains("circular"))
        );
    }

    #[test]
    fn reports_unresolved_needs() {
        let parsed = parse_and_validate("on: push\njobs:\n  a:\n    needs: ghost\n    steps: []\n");
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|d| d.message.contains("unknown job `ghost`"))
        );
    }

    #[test]
    fn duplicate_job_ids_become_a_diagnostic_not_a_panic() {
        let parsed =
            parse_and_validate("on: push\njobs:\n  a:\n    steps: []\n  a:\n    steps: []\n");
        assert_eq!(parsed.status(), "errors");
        assert!(
            parsed.diagnostics[0].message.contains("YAML parse error"),
            "duplicate mapping keys are rejected by the parser"
        );
    }

    #[test]
    fn missing_on_and_jobs_are_errors() {
        let parsed = parse_and_validate("name: broken\n");
        let messages: Vec<_> = parsed.diagnostics.iter().map(|d| &d.message).collect();
        assert!(messages.iter().any(|m| m.contains("`on`")));
        assert!(messages.iter().any(|m| m.contains("`jobs`")));
    }

    #[test]
    fn alias_bomb_is_rejected_by_node_budget() {
        // Anchor expansion multiplies nodes past the budget without a
        // large source document.
        let bomb = r#"
a: &a ["x","x","x","x","x","x","x","x","x","x"]
b: &b [*a,*a,*a,*a,*a,*a,*a,*a,*a,*a]
c: &c [*b,*b,*b,*b,*b,*b,*b,*b,*b,*b]
d: &d [*c,*c,*c,*c,*c,*c,*c,*c,*c,*c]
e: [*d,*d,*d,*d,*d,*d,*d,*d,*d,*d]
"#;
        let parsed = parse_and_validate(bomb);
        assert_eq!(parsed.status(), "errors");
    }

    #[test]
    fn oversized_file_is_rejected_before_parse() {
        let big = "x".repeat(MAX_WORKFLOW_FILE_BYTES + 1);
        let parsed = parse_and_validate(&big);
        assert_eq!(parsed.status(), "errors");
        assert!(parsed.diagnostics[0].message.contains("byte limit"));
    }

    #[test]
    fn job_without_steps_or_uses_is_an_error() {
        let parsed = parse_and_validate("on: push\njobs:\n  a:\n    runs-on: ubuntu-latest\n");
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|d| d.message.contains("neither `steps` nor `uses`"))
        );
    }

    #[test]
    fn reusable_workflow_job_is_valid() {
        let parsed = parse_and_validate(
            "on: push\njobs:\n  call:\n    uses: octo/repo/.github/workflows/ci.yml@main\n",
        );
        assert_eq!(parsed.status(), "valid");
        assert!(parsed.jobs[0].uses.is_some());
    }

    #[test]
    fn unknown_keys_are_warnings() {
        let parsed = parse_and_validate(
            "on: push\ntypo_key: 1\njobs:\n  a:\n    steps: []\n    runs-on: x\n",
        );
        assert_eq!(parsed.status(), "warnings");
    }

    #[test]
    fn secret_scan_skips_github_token_and_dedupes() {
        let refs = scan_secret_refs(
            "a ${{ secrets.API_KEY }} b ${{ secrets.GITHUB_TOKEN }} c ${{ secrets.API_KEY }}",
        );
        assert_eq!(refs, vec!["API_KEY"]);
    }
}
