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

/// Stamped into every workflow's `metadata` JSONB. Bump whenever the parse
/// output or metadata shape changes: repo sync re-parses any stored workflow
/// whose stamped version is older, even when its blob sha is unchanged —
/// otherwise new metadata (e.g. the detected-requirements ref arrays) would
/// never materialize for files that don't change on GitHub.
pub const PARSER_VERSION: i64 = 3;

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
    /// GitHub-style deployment environment name (`environment: prod` or
    /// `environment: { name: prod }`); resolved live by name at dispatch to
    /// scope environment secrets.
    pub environment: Option<String>,
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
            // Version-stamped even on failure so sync doesn't re-parse a
            // persistently broken file on every run.
            metadata: serde_json::json!({ "parserVersion": PARSER_VERSION }),
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
        "parserVersion": PARSER_VERSION,
        "permissions": get(root, "permissions").map(value_to_json),
        "concurrency": get(root, "concurrency").map(value_to_json),
        "envKeys": get(root, "env")
            .and_then(Value::as_mapping)
            .map(|m| m.keys().filter_map(key_name).collect::<Vec<_>>())
            .unwrap_or_default(),
        "secretRefs": scan_secret_refs(content),
        "varRefs": scan_var_refs(content),
        "environments": distinct_environments(&jobs),
        "dispatchInputs": extract_dispatch_inputs(root, &mut diagnostics),
        "triggerFilters": extract_trigger_filters(root, &mut diagnostics),
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

/// Caps for trigger filter pattern lists (`on.push.branches`, …). A workflow
/// with more patterns than this is hostile or broken, not "large".
const MAX_FILTER_PATTERNS: usize = 50;
const MAX_FILTER_PATTERN_LEN: usize = 256;

/// The `pull_request` activity types GitHub defines. Unknown values are
/// dropped with a warning — a typo'd type must never silently widen or
/// narrow trigger matching at dispatch time.
const PR_ACTIVITY_TYPES: &[&str] = &[
    "opened",
    "synchronize",
    "reopened",
    "closed",
    "edited",
    "assigned",
    "unassigned",
    "labeled",
    "unlabeled",
    "ready_for_review",
    "converted_to_draft",
    "review_requested",
    "review_request_removed",
    "locked",
    "unlocked",
];

/// Structured `on.<event>` filter lists consumed by the dispatch-time trigger
/// evaluation engine (`services/trigger_eval.rs`). Only the mapping form of
/// `on:` carries filters; string/sequence forms yield the all-empty shape
/// (= unfiltered). Every list is capped and length-bounded — patterns are
/// stored verbatim (they are matched, never executed or interpolated).
/// Combining `branches` with `branches-ignore` (or `tags` with `tags-ignore`)
/// on the same event is an Error, matching GitHub's own rejection.
fn extract_trigger_filters(root: &Mapping, diagnostics: &mut Vec<Diagnostic>) -> serde_json::Value {
    // Same YAML 1.1 quirk as extract_triggers: `on` may be the bool key.
    let events = root
        .get(Value::String("on".into()))
        .or_else(|| root.get(Value::Bool(true)))
        .and_then(Value::as_mapping);

    let mut filters = serde_json::json!({
        "push": {
            "branches": [], "branchesIgnore": [],
            "tags": [], "tagsIgnore": [],
            "paths": [], "pathsIgnore": [],
        },
        "pullRequest": {
            "types": [],
            "branches": [], "branchesIgnore": [],
            "paths": [], "pathsIgnore": [],
        },
    });
    let Some(events) = events else {
        return filters;
    };

    for (event, yaml_key, keys) in [
        (
            "push",
            "push",
            &[
                ("branches", "branches"),
                ("branches-ignore", "branchesIgnore"),
                ("tags", "tags"),
                ("tags-ignore", "tagsIgnore"),
                ("paths", "paths"),
                ("paths-ignore", "pathsIgnore"),
            ][..],
        ),
        (
            "pullRequest",
            "pull_request",
            &[
                ("branches", "branches"),
                ("branches-ignore", "branchesIgnore"),
                ("paths", "paths"),
                ("paths-ignore", "pathsIgnore"),
            ][..],
        ),
    ] {
        let Some(body) = get(events, yaml_key).and_then(Value::as_mapping) else {
            continue;
        };
        for (yaml_name, json_name) in keys {
            filters[event][*json_name] = serde_json::json!(filter_patterns(
                get(body, yaml_name),
                &format!("on.{yaml_key}.{yaml_name}"),
                diagnostics,
            ));
        }
        // GitHub rejects a workflow that combines the include and ignore
        // forms of the same dimension on one event.
        for (include, ignore) in [("branches", "branches-ignore"), ("tags", "tags-ignore")] {
            if get(body, include).is_some() && get(body, ignore).is_some() {
                diagnostics.push(Diagnostic::error(
                    format!("`on.{yaml_key}` cannot combine `{include}` with `{ignore}`"),
                    Some(format!("on.{yaml_key}.{ignore}")),
                ));
            }
        }
        if get(body, "paths").is_some() && get(body, "paths-ignore").is_some() {
            diagnostics.push(Diagnostic::error(
                format!("`on.{yaml_key}` cannot combine `paths` with `paths-ignore`"),
                Some(format!("on.{yaml_key}.paths-ignore")),
            ));
        }
    }

    // pull_request activity types: allow-listed, deduped, capped.
    if let Some(body) = get(events, "pull_request").and_then(Value::as_mapping) {
        let mut types: Vec<String> = Vec::new();
        for value in string_or_list(get(body, "types")) {
            if !PR_ACTIVITY_TYPES.contains(&value.as_str()) {
                diagnostics.push(Diagnostic::warning(
                    format!(
                        "unknown pull_request activity type `{}`",
                        value.chars().take(50).collect::<String>()
                    ),
                    Some("on.pull_request.types".into()),
                ));
                continue;
            }
            if !types.contains(&value) {
                types.push(value);
            }
        }
        filters["pullRequest"]["types"] = serde_json::json!(types);
    }

    filters
}

/// One filter pattern list: string-or-sequence, non-empty, byte-capped,
/// count-capped. Oversized patterns and overflow are dropped with a warning
/// — a filter that can't be stored must never silently match everything.
fn filter_patterns(
    value: Option<&Value>,
    path: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<String> {
    let raw = match value {
        None => return Vec::new(),
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Sequence(seq)) => seq
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(_) => {
            diagnostics.push(Diagnostic::warning(
                format!("`{path}` must be a string or a list of patterns"),
                Some(path.to_string()),
            ));
            return Vec::new();
        }
    };
    if raw.len() > MAX_FILTER_PATTERNS {
        diagnostics.push(Diagnostic::warning(
            format!("`{path}` lists more than {MAX_FILTER_PATTERNS} patterns; extras are ignored"),
            Some(path.to_string()),
        ));
    }
    let mut out = Vec::new();
    for pattern in raw.into_iter().take(MAX_FILTER_PATTERNS) {
        if pattern.is_empty() {
            continue;
        }
        if pattern.len() > MAX_FILTER_PATTERN_LEN {
            diagnostics.push(Diagnostic::warning(
                format!("`{path}` contains a pattern longer than {MAX_FILTER_PATTERN_LEN} bytes; it was ignored"),
                Some(path.to_string()),
            ));
            continue;
        }
        out.push(pattern);
    }
    out
}

/// Caps for `on.workflow_dispatch.inputs` extraction. GitHub itself allows
/// at most 25 top-level inputs; the string caps bound stored metadata.
const MAX_DISPATCH_INPUTS: usize = 25;
const MAX_INPUT_NAME_LEN: usize = 64;
const MAX_INPUT_DESCRIPTION_LEN: usize = 500;
const MAX_INPUT_DEFAULT_LEN: usize = 1024;
const MAX_INPUT_OPTIONS: usize = 50;
const MAX_INPUT_OPTION_LEN: usize = 200;

/// Input types GitHub's manual-trigger schema defines. Unknown types
/// degrade to `string` with a warning rather than failing the workflow.
const DISPATCH_INPUT_TYPES: &[&str] = &["string", "number", "boolean", "choice", "environment"];

fn is_valid_input_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && name.len() <= MAX_INPUT_NAME_LEN
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Structured `on.workflow_dispatch.inputs` — the definitions that drive the
/// Run Workflow form and the server-side validation of submitted inputs.
/// Everything is capped and allow-listed; malformed entries become warnings
/// and are dropped, never hard failures (the workflow still runs without
/// them). Returns an empty array when the trigger or its inputs are absent.
fn extract_dispatch_inputs(root: &Mapping, diagnostics: &mut Vec<Diagnostic>) -> serde_json::Value {
    // Same YAML 1.1 quirk as extract_triggers: `on` may be the bool key.
    let on = root
        .get(Value::String("on".into()))
        .or_else(|| root.get(Value::Bool(true)));
    let inputs = on
        .and_then(Value::as_mapping)
        .and_then(|events| get(events, "workflow_dispatch"))
        .and_then(Value::as_mapping)
        .and_then(|dispatch| get(dispatch, "inputs"))
        .and_then(Value::as_mapping);
    let Some(inputs) = inputs else {
        return serde_json::json!([]);
    };

    if inputs.len() > MAX_DISPATCH_INPUTS {
        diagnostics.push(Diagnostic::warning(
            format!(
                "workflow_dispatch defines more than {MAX_DISPATCH_INPUTS} inputs; \
                 extras are ignored"
            ),
            Some("on.workflow_dispatch.inputs".into()),
        ));
    }

    let mut out = Vec::new();
    for (key, body) in inputs.iter().take(MAX_DISPATCH_INPUTS) {
        let path = "on.workflow_dispatch.inputs".to_string();
        let Some(name) = key_name(key).filter(|n| is_valid_input_name(n)) else {
            diagnostics.push(Diagnostic::warning(
                "workflow_dispatch input with an invalid name was ignored".to_string(),
                Some(path),
            ));
            continue;
        };
        // GitHub allows a bare `input_name:` (null body) — all defaults.
        let empty = Mapping::new();
        let body = match body {
            Value::Null => &empty,
            other => match other.as_mapping() {
                Some(map) => map,
                None => {
                    diagnostics.push(Diagnostic::warning(
                        format!("workflow_dispatch input `{name}` must be a mapping"),
                        Some(format!("{path}.{name}")),
                    ));
                    continue;
                }
            },
        };

        let declared_type = get(body, "type").and_then(Value::as_str).unwrap_or("string");
        let input_type = if DISPATCH_INPUT_TYPES.contains(&declared_type) {
            declared_type
        } else {
            diagnostics.push(Diagnostic::warning(
                format!(
                    "workflow_dispatch input `{name}` has unknown type `{}`; treated as string",
                    declared_type.chars().take(50).collect::<String>()
                ),
                Some(format!("{path}.{name}.type")),
            ));
            "string"
        };

        let required = get(body, "required").and_then(Value::as_bool).unwrap_or(false);
        let description = get(body, "description")
            .and_then(Value::as_str)
            .map(|s| s.chars().take(MAX_INPUT_DESCRIPTION_LEN).collect::<String>());
        // Defaults are stringified: booleans/numbers arrive as scalars but
        // the execution env is string-typed anyway.
        let default = get(body, "default").and_then(|v| match v {
            Value::String(s) => Some(s.chars().take(MAX_INPUT_DEFAULT_LEN).collect::<String>()),
            Value::Bool(b) => Some(b.to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        });
        let options: Vec<String> = get(body, "options")
            .and_then(Value::as_sequence)
            .map(|seq| {
                seq.iter()
                    .filter_map(Value::as_str)
                    .filter(|s| !s.is_empty() && s.len() <= MAX_INPUT_OPTION_LEN)
                    .map(str::to_string)
                    .take(MAX_INPUT_OPTIONS)
                    .collect()
            })
            .unwrap_or_default();

        if input_type == "choice" && options.is_empty() {
            diagnostics.push(Diagnostic::warning(
                format!("workflow_dispatch choice input `{name}` defines no options"),
                Some(format!("{path}.{name}.options")),
            ));
        }

        let mut entry = serde_json::json!({
            "name": name,
            "type": input_type,
            "required": required,
        });
        if let Some(description) = description {
            entry["description"] = serde_json::Value::String(description);
        }
        if let Some(default) = default {
            entry["default"] = serde_json::Value::String(default);
        }
        if !options.is_empty() {
            entry["options"] = serde_json::json!(options);
        }
        out.push(entry);
    }
    serde_json::Value::Array(out)
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

        // GitHub's two `environment:` forms — a bare string or a map with
        // `name` (whose `url` we deliberately ignore). Anything else is a
        // warning, never an error: the job still runs, just without
        // environment secrets.
        let environment = match get(job, "environment") {
            None => None,
            Some(value) => {
                let name = value.as_str().or_else(|| {
                    value
                        .as_mapping()
                        .and_then(|m| get(m, "name"))
                        .and_then(Value::as_str)
                });
                match name.map(str::trim) {
                    Some(name) if !name.is_empty() => {
                        Some(name.chars().take(100).collect::<String>())
                    }
                    _ => {
                        diagnostics.push(Diagnostic::warning(
                            format!(
                                "job `{job_key}` has a malformed `environment` \
                                 (expected a string or a map with `name`)"
                            ),
                            Some(format!("{path}.environment")),
                        ));
                        None
                    }
                }
            }
        };

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
            environment,
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

/// Names of secrets referenced as `${{ secrets.NAME }}` (or the bracket form
/// `secrets['NAME']`) — metadata only, never values.
pub fn scan_secret_refs(content: &str) -> Vec<String> {
    scan_context_refs(content, "secrets", &["GITHUB_TOKEN"])
}

/// Names of configuration variables referenced as `${{ vars.NAME }}` — the
/// platform doesn't manage plain variables yet, so these are surfaced purely
/// as detected requirements.
pub fn scan_var_refs(content: &str) -> Vec<String> {
    scan_context_refs(content, "vars", &[])
}

/// Names referenced as `${{ <context>.NAME }}` or `${{ <context>['NAME'] }}`
/// inside expression blocks. Hand-rolled scan; no expression evaluation.
/// Identifiers follow GitHub's rule (`[A-Za-z_][A-Za-z0-9_]*`, here capped at
/// 200 bytes); deduped, capped at 100, sorted.
fn scan_context_refs(content: &str, context: &str, exclude: &[&str]) -> Vec<String> {
    let mut refs: Vec<String> = Vec::new();
    let mut rest = content;
    'blocks: while let Some(start) = rest.find("${{") {
        rest = &rest[start + 3..];
        let Some(end) = rest.find("}}") else { break };
        let expr = &rest[..end];
        let mut offset = 0;
        while let Some(pos) = expr[offset..].find(context) {
            let at = offset + pos;
            let after = &expr[at + context.len()..];
            offset = at + context.len();
            // Word boundary: `mysecrets.X` or `x.vars.Y` must not match.
            let bounded = at == 0 || {
                let prev = expr.as_bytes()[at - 1];
                !(prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'.')
            };
            if !bounded {
                continue;
            }
            if let Some(name) = extract_ref_name(after)
                && !exclude.contains(&name.as_str())
                && !refs.contains(&name)
            {
                refs.push(name);
                if refs.len() >= 100 {
                    break 'blocks;
                }
            }
        }
        rest = &rest[end + 2..];
    }
    refs.sort();
    refs
}

/// A well-formed reference identifier: `[A-Za-z_][A-Za-z0-9_]*`, ≤200 bytes.
fn is_ref_ident(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && name.len() <= 200
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The identifier after a context word: `.NAME`, or `['NAME']` / `["NAME"]`.
/// Anything not matching `is_ref_ident` is dropped — dynamic or malformed
/// references never become metadata.
fn extract_ref_name(after: &str) -> Option<String> {
    let name: String = if let Some(rest) = after.strip_prefix('.') {
        rest.chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect()
    } else if let Some(rest) = after.strip_prefix('[') {
        let rest = rest.trim_start();
        let quote = rest.chars().next().filter(|c| matches!(c, '\'' | '"'))?;
        let inner = &rest[1..];
        let end = inner.find(quote)?;
        let tail = inner[end + 1..].trim_start();
        if !tail.starts_with(']') {
            return None;
        }
        inner[..end].to_string()
    } else {
        return None;
    };
    is_ref_ident(&name).then_some(name)
}

/// A trimmed expression that is EXACTLY one `secrets.NAME` / `vars.NAME`
/// reference (dot or bracket form, nothing before or after) — the only
/// shape `substitute_context_refs` will resolve. Compound expressions are
/// deliberately not evaluated.
fn parse_single_ref(expr: &str) -> Option<(&'static str, String)> {
    for context in ["secrets", "vars"] {
        let Some(rest) = expr.strip_prefix(context) else {
            continue;
        };
        if let Some(ident) = rest.strip_prefix('.') {
            if is_ref_ident(ident) {
                return Some((context, ident.to_string()));
            }
        } else if let Some(bracket) = rest.strip_prefix('[') {
            let bracket = bracket.trim_start();
            let Some(quote) = bracket.chars().next().filter(|c| matches!(c, '\'' | '"')) else {
                continue;
            };
            let inner = &bracket[1..];
            let Some(end) = inner.find(quote) else {
                continue;
            };
            let name = &inner[..end];
            if inner[end + 1..].trim() == "]" && is_ref_ident(name) {
                return Some((context, name.to_string()));
            }
        }
    }
    None
}

/// Replace `${{ secrets.NAME }}` / `${{ vars.NAME }}` blocks with
/// `lookup(context, name)` — GitHub-style expression resolution restricted
/// to those two contexts. A block whose (trimmed) inner expression is
/// anything else — another context, a compound expression, a dynamic name —
/// passes through untouched: this is a substitutor, not an evaluator.
/// `lookup` returning `None` also leaves the block untouched; to match
/// GitHub's unset-secret semantics the caller returns an empty string for
/// unknown names instead.
pub fn substitute_context_refs(
    input: &str,
    lookup: &dyn Fn(&str, &str) -> Option<String>,
) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${{") {
        let Some(end_rel) = rest[start + 3..].find("}}") else {
            break;
        };
        let expr = &rest[start + 3..start + 3 + end_rel];
        let block_end = start + 3 + end_rel + 2;
        match parse_single_ref(expr.trim()).and_then(|(context, name)| lookup(context, &name)) {
            Some(value) => {
                out.push_str(&rest[..start]);
                out.push_str(&value);
            }
            None => out.push_str(&rest[..block_end]),
        }
        rest = &rest[block_end..];
    }
    out.push_str(rest);
    out
}

/// Distinct `environment:` names across jobs, deduped case-insensitively
/// (first-seen casing kept) and filtered to the slug charset an environment
/// row can actually take — dynamic `${{ … }}` names are dropped, never stored.
fn distinct_environments(jobs: &[ParsedJob]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for job in jobs {
        let Some(name) = job.environment.as_deref() else {
            continue;
        };
        if !valid_environment_name(name) {
            continue;
        }
        if !names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
            names.push(name.to_string());
        }
    }
    names.sort_by_key(|n| n.to_ascii_lowercase());
    names
}

/// Mirrors the environments handler's name rule
/// (`^[A-Za-z0-9][A-Za-z0-9._-]*$`, ≤100 chars).
fn valid_environment_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphanumeric())
        && name.len() <= 100
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
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
    fn extracts_workflow_dispatch_inputs() {
        let parsed = parse_and_validate(
            r#"
on:
  push:
  workflow_dispatch:
    inputs:
      environment:
        type: choice
        description: Target environment
        required: true
        options: [staging, production]
      dry_run:
        type: boolean
        default: true
      note:
jobs:
  a:
    runs-on: x
    steps: []
"#,
        );
        assert_eq!(parsed.status(), "valid");
        let inputs = parsed.metadata["dispatchInputs"].as_array().unwrap();
        assert_eq!(inputs.len(), 3);
        assert_eq!(inputs[0]["name"], "environment");
        assert_eq!(inputs[0]["type"], "choice");
        assert_eq!(inputs[0]["required"], true);
        assert_eq!(inputs[0]["options"][1], "production");
        assert_eq!(inputs[1]["name"], "dry_run");
        assert_eq!(inputs[1]["default"], "true");
        assert_eq!(inputs[2]["name"], "note");
        assert_eq!(inputs[2]["type"], "string");
        assert_eq!(inputs[2]["required"], false);
    }

    #[test]
    fn dispatch_inputs_absent_or_bool_on_yield_empty_array() {
        let parsed = parse_and_validate("on: [push]\njobs:\n  a:\n    steps: []\n    runs-on: x\n");
        assert_eq!(parsed.metadata["dispatchInputs"], serde_json::json!([]));
        // Bare workflow_dispatch with no inputs.
        let parsed = parse_and_validate(
            "on:\n  workflow_dispatch:\njobs:\n  a:\n    steps: []\n    runs-on: x\n",
        );
        assert_eq!(parsed.metadata["dispatchInputs"], serde_json::json!([]));
    }

    #[test]
    fn dispatch_inputs_enforce_caps_and_type_allow_list() {
        let mut yaml = String::from("on:\n  workflow_dispatch:\n    inputs:\n");
        for i in 0..30 {
            yaml.push_str(&format!("      input_{i}:\n        type: strange\n"));
        }
        yaml.push_str("jobs:\n  a:\n    steps: []\n    runs-on: x\n");
        let parsed = parse_and_validate(&yaml);
        let inputs = parsed.metadata["dispatchInputs"].as_array().unwrap();
        assert_eq!(inputs.len(), MAX_DISPATCH_INPUTS);
        assert!(inputs.iter().all(|i| i["type"] == "string"));
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|d| d.message.contains("more than"))
        );
    }

    #[test]
    fn secret_scan_skips_github_token_and_dedupes() {
        let refs = scan_secret_refs(
            "a ${{ secrets.API_KEY }} b ${{ secrets.GITHUB_TOKEN }} c ${{ secrets.API_KEY }}",
        );
        assert_eq!(refs, vec!["API_KEY"]);
    }

    #[test]
    fn secret_scan_handles_bracket_form_and_word_boundaries() {
        let refs = scan_secret_refs(
            "a ${{ secrets['DEPLOY_KEY'] }} b ${{ secrets[\"OTHER\"] }} \
             c ${{ secrets['GITHUB_TOKEN'] }} d ${{ mysecrets.NOPE }} \
             e ${{ github.secrets.ALSO_NOPE }} f ${{ secrets['1BAD'] }}",
        );
        assert_eq!(refs, vec!["DEPLOY_KEY", "OTHER"]);
    }

    #[test]
    fn scans_var_refs() {
        let refs = scan_var_refs(
            "x ${{ vars.NODE_ENV }} y ${{ vars['REGION'] }} z ${{ vars.NODE_ENV }} \
             w ${{ canvars.NOPE }}",
        );
        assert_eq!(refs, vec!["NODE_ENV", "REGION"]);
    }

    #[test]
    fn aggregates_environment_names_case_insensitively() {
        let parsed = parse_and_validate(
            r#"
on: push
jobs:
  a:
    runs-on: ubuntu-latest
    environment: production
    steps: []
  b:
    runs-on: ubuntu-latest
    environment:
      name: Production
    steps: []
  c:
    runs-on: ubuntu-latest
    environment: staging
    steps: []
"#,
        );
        assert_eq!(
            parsed.metadata["environments"],
            serde_json::json!(["production", "staging"])
        );
    }

    #[test]
    fn drops_dynamic_environment_names_from_metadata() {
        let parsed = parse_and_validate(
            "on: push\njobs:\n  a:\n    runs-on: ubuntu-latest\n    environment: ${{ inputs.env }}\n    steps: []\n",
        );
        assert_eq!(parsed.metadata["environments"], serde_json::json!([]));
    }

    #[test]
    fn metadata_carries_empty_ref_arrays_when_nothing_is_referenced() {
        let parsed = parse_and_validate(
            "on: push\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps: []\n",
        );
        assert_eq!(parsed.metadata["secretRefs"], serde_json::json!([]));
        assert_eq!(parsed.metadata["varRefs"], serde_json::json!([]));
        assert_eq!(parsed.metadata["environments"], serde_json::json!([]));
    }

    fn test_lookup(context: &str, name: &str) -> Option<String> {
        match (context, name) {
            ("secrets", "API_KEY") => Some("s3cr3t-value".into()),
            ("secrets", _) => Some(String::new()),
            ("vars", _) => Some(String::new()),
            _ => None,
        }
    }

    #[test]
    fn substitutes_secret_and_var_refs() {
        assert_eq!(
            substitute_context_refs("x=${{ secrets.API_KEY }};", &test_lookup),
            "x=s3cr3t-value;"
        );
        assert_eq!(
            substitute_context_refs("${{secrets['API_KEY']}} ${{ vars.REGION }}", &test_lookup),
            "s3cr3t-value "
        );
        // Unknown secret resolves to empty (GitHub semantics via the lookup).
        assert_eq!(
            substitute_context_refs("v=${{ secrets.UNKNOWN }}!", &test_lookup),
            "v=!"
        );
    }

    #[test]
    fn substitution_leaves_other_expressions_untouched() {
        for input in [
            "sha=${{ github.sha }}",
            "m=${{ matrix.os }}",
            "c=${{ secrets.A || secrets.B }}",
            "f=${{ format('{0}', secrets.A) }}",
            "d=${{ secrets[github.ref] }}",
            "open=${{ secrets.NEVER_CLOSED",
        ] {
            assert_eq!(substitute_context_refs(input, &test_lookup), input);
        }
    }

    #[test]
    fn substitution_handles_multiple_blocks() {
        assert_eq!(
            substitute_context_refs(
                "a=${{ secrets.API_KEY }} b=${{ github.ref }} c=${{ vars.X }}",
                &test_lookup
            ),
            "a=s3cr3t-value b=${{ github.ref }} c="
        );
    }

    #[test]
    fn extracts_trigger_filters_from_mapping_form() {
        let parsed = parse_and_validate(
            r#"
on:
  push:
    branches: [main, 'releases/**']
    paths-ignore:
      - 'docs/**'
  pull_request:
    types: [opened, labeled]
    branches: [main]
jobs:
  a:
    runs-on: x
    steps: []
"#,
        );
        assert_eq!(parsed.status(), "valid");
        let filters = &parsed.metadata["triggerFilters"];
        assert_eq!(
            filters["push"]["branches"],
            serde_json::json!(["main", "releases/**"])
        );
        assert_eq!(
            filters["push"]["pathsIgnore"],
            serde_json::json!(["docs/**"])
        );
        assert_eq!(filters["push"]["tags"], serde_json::json!([]));
        assert_eq!(
            filters["pullRequest"]["types"],
            serde_json::json!(["opened", "labeled"])
        );
        assert_eq!(
            filters["pullRequest"]["branches"],
            serde_json::json!(["main"])
        );
    }

    #[test]
    fn string_and_sequence_on_forms_yield_empty_filters() {
        for yaml in [
            "on: push\njobs:\n  a:\n    runs-on: x\n    steps: []\n",
            "on: [push, pull_request]\njobs:\n  a:\n    runs-on: x\n    steps: []\n",
        ] {
            let parsed = parse_and_validate(yaml);
            let filters = &parsed.metadata["triggerFilters"];
            assert_eq!(filters["push"]["branches"], serde_json::json!([]));
            assert_eq!(filters["pullRequest"]["types"], serde_json::json!([]));
        }
    }

    #[test]
    fn combining_branches_with_branches_ignore_is_an_error() {
        let parsed = parse_and_validate(
            "on:\n  push:\n    branches: [main]\n    branches-ignore: [dev]\n\
             jobs:\n  a:\n    runs-on: x\n    steps: []\n",
        );
        assert_eq!(parsed.status(), "errors");
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|d| d.message.contains("cannot combine `branches` with `branches-ignore`"))
        );
    }

    #[test]
    fn combining_paths_with_paths_ignore_is_an_error() {
        let parsed = parse_and_validate(
            "on:\n  pull_request:\n    paths: [src/**]\n    paths-ignore: [docs/**]\n\
             jobs:\n  a:\n    runs-on: x\n    steps: []\n",
        );
        assert_eq!(parsed.status(), "errors");
    }

    #[test]
    fn unknown_pr_types_are_dropped_with_a_warning() {
        let parsed = parse_and_validate(
            "on:\n  pull_request:\n    types: [opened, bogus_type]\n\
             jobs:\n  a:\n    runs-on: x\n    steps: []\n",
        );
        assert_eq!(parsed.status(), "warnings");
        assert_eq!(
            parsed.metadata["triggerFilters"]["pullRequest"]["types"],
            serde_json::json!(["opened"])
        );
    }

    #[test]
    fn filter_patterns_enforce_caps() {
        let mut yaml = String::from("on:\n  push:\n    branches:\n");
        for i in 0..60 {
            yaml.push_str(&format!("      - branch-{i}\n"));
        }
        yaml.push_str(&format!("      - '{}'\n", "x".repeat(300)));
        yaml.push_str("jobs:\n  a:\n    runs-on: x\n    steps: []\n");
        let parsed = parse_and_validate(&yaml);
        let branches = parsed.metadata["triggerFilters"]["push"]["branches"]
            .as_array()
            .unwrap();
        assert_eq!(branches.len(), MAX_FILTER_PATTERNS);
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|d| d.message.contains("more than"))
        );
    }

    #[test]
    fn bool_on_key_still_extracts_filters() {
        // Unquoted `on` resolves to boolean true in YAML 1.1; the mapping
        // form must still be found under the bool key.
        let parsed = parse_and_validate(
            "on:\n  push:\n    branches: [main]\njobs:\n  a:\n    runs-on: x\n    steps: []\n",
        );
        assert_eq!(
            parsed.metadata["triggerFilters"]["push"]["branches"],
            serde_json::json!(["main"])
        );
    }

    #[test]
    fn metadata_is_parser_version_stamped_on_success_and_failure() {
        let parsed = parse_and_validate(
            "on: push\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps: []\n",
        );
        assert_eq!(
            parsed.metadata["parserVersion"],
            serde_json::json!(PARSER_VERSION)
        );
        let broken = parse_and_validate("{not yaml");
        assert_eq!(
            broken.metadata["parserVersion"],
            serde_json::json!(PARSER_VERSION)
        );
    }
}
