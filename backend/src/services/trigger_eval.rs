//! Dispatch-time trigger evaluation: does a repository event satisfy a
//! workflow's `on:` filter conditions? Pure functions over the parser's
//! stored `metadata.triggerFilters` — no I/O, no expression evaluation.
//! Semantics mirror GitHub Actions' documented filter rules:
//!
//! - glob patterns support `*` (no `/`), `**` (crosses `/`), `?` / `+`
//!   (zero-or-one / one-or-more of the preceding character), `[abc]` /
//!   `[a-z]` classes, and `\` escapes;
//! - filter lists are ORDERED: a matching `!`-negated pattern after a
//!   positive match excludes the value, a later positive match re-includes
//!   it;
//! - with only `tags` defined a branch push never runs (and vice versa);
//! - branch AND path dimensions must BOTH pass; paths never apply to tag
//!   pushes;
//! - `pull_request` defaults to the `opened|synchronize|reopened` activity
//!   types, and its branch filters match the BASE branch;
//! - unavailable changed-file data (`changed_paths: None`) fails OPEN: path
//!   filters pass, matching GitHub's behavior when a diff can't be computed.
//!
//! Everything here is bounded: patterns are capped at parse time (50 per
//! list, 256 bytes each) and re-capped defensively on read, values at 1 KB,
//! and the matcher is iterative (position-set DP, no recursion, no regex).

use serde_json::Value;

/// Defensive read-time caps mirroring the parser's write-time caps
/// (`workflow_parse::MAX_FILTER_PATTERNS`) — hand-edited metadata must not
/// widen the budget.
const MAX_PATTERNS: usize = 50;
const MAX_PATTERN_LEN: usize = 256;
const MAX_VALUE_LEN: usize = 1024;

/// GitHub's default `pull_request` activity types when `types:` is absent.
const DEFAULT_PR_TYPES: &[&str] = &["opened", "synchronize", "reopened"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Run,
    /// Static skip category for the repository-event timeline.
    Skip(&'static str),
}

pub const SKIP_EVENT_NOT_DECLARED: &str = "event_not_declared";
pub const SKIP_BRANCH_FILTERED: &str = "branch_filtered";
pub const SKIP_TAG_FILTERED: &str = "tag_filtered";
pub const SKIP_PATH_FILTERED: &str = "path_filtered";
pub const SKIP_TYPE_FILTERED: &str = "type_filtered";

#[derive(Debug, Clone, Copy)]
pub enum EventKind<'a> {
    Push { branch: &'a str },
    TagPush { tag: &'a str },
    PullRequest { action: &'a str, base_branch: &'a str },
}

#[derive(Debug, Clone, Copy)]
pub struct EventContext<'a> {
    pub kind: EventKind<'a>,
    /// `None` means the changed-file set is unknown (truncated push payload,
    /// pull_request event) — path filters then pass (fail-open).
    pub changed_paths: Option<&'a [String]>,
}

/// Evaluate one workflow against one repository event. `triggers` is the
/// stored `workflows.triggers` event-name array; `metadata` the stored
/// `workflows.metadata` JSONB (any missing/malformed `triggerFilters` shape —
/// pre-v3 rows — degrades to "declared event runs unfiltered").
pub fn evaluate(triggers: &[String], metadata: &Value, ctx: &EventContext) -> Decision {
    let declared_event = match ctx.kind {
        EventKind::Push { .. } | EventKind::TagPush { .. } => "push",
        EventKind::PullRequest { .. } => "pull_request",
    };
    if !triggers.iter().any(|t| t == declared_event) {
        return Decision::Skip(SKIP_EVENT_NOT_DECLARED);
    }

    let filters = &metadata["triggerFilters"];

    match ctx.kind {
        EventKind::Push { branch } => {
            let push = &filters["push"];
            let branches = patterns(push, "branches");
            let branches_ignore = patterns(push, "branchesIgnore");
            let tags = patterns(push, "tags");
            let tags_ignore = patterns(push, "tagsIgnore");

            if !branches.is_empty() {
                // Defense-in-depth: when both forms are somehow present
                // (the parser flags it as an error), `branches` wins.
                if !match_ordered(&branches, branch) {
                    return Decision::Skip(SKIP_BRANCH_FILTERED);
                }
            } else if !branches_ignore.is_empty() {
                if match_ordered(&branches_ignore, branch) {
                    return Decision::Skip(SKIP_BRANCH_FILTERED);
                }
            } else if !tags.is_empty() || !tags_ignore.is_empty() {
                // Only the tag dimension is filtered: branch pushes never run.
                return Decision::Skip(SKIP_BRANCH_FILTERED);
            }

            evaluate_paths(push, ctx.changed_paths)
        }
        EventKind::TagPush { tag } => {
            let push = &filters["push"];
            let branches = patterns(push, "branches");
            let branches_ignore = patterns(push, "branchesIgnore");
            let tags = patterns(push, "tags");
            let tags_ignore = patterns(push, "tagsIgnore");

            if !tags.is_empty() {
                if !match_ordered(&tags, tag) {
                    return Decision::Skip(SKIP_TAG_FILTERED);
                }
            } else if !tags_ignore.is_empty() {
                if match_ordered(&tags_ignore, tag) {
                    return Decision::Skip(SKIP_TAG_FILTERED);
                }
            } else if !branches.is_empty() || !branches_ignore.is_empty() {
                // Only the branch dimension is filtered: tag pushes never run.
                return Decision::Skip(SKIP_TAG_FILTERED);
            }

            // Path filters deliberately never apply to tag pushes.
            Decision::Run
        }
        EventKind::PullRequest { action, base_branch } => {
            let pr = &filters["pullRequest"];

            let types = patterns(pr, "types");
            let type_matches = if types.is_empty() {
                DEFAULT_PR_TYPES.contains(&action)
            } else {
                types.iter().any(|t| t == action)
            };
            if !type_matches {
                return Decision::Skip(SKIP_TYPE_FILTERED);
            }

            let branches = patterns(pr, "branches");
            let branches_ignore = patterns(pr, "branchesIgnore");
            if !branches.is_empty() {
                if !match_ordered(&branches, base_branch) {
                    return Decision::Skip(SKIP_BRANCH_FILTERED);
                }
            } else if !branches_ignore.is_empty() && match_ordered(&branches_ignore, base_branch) {
                return Decision::Skip(SKIP_BRANCH_FILTERED);
            }

            evaluate_paths(pr, ctx.changed_paths)
        }
    }
}

/// The path dimension shared by branch pushes and pull requests: with
/// `paths`, at least one changed file must match; with `paths-ignore`, at
/// least one changed file must NOT match. Unknown changed files pass.
fn evaluate_paths(filters: &Value, changed_paths: Option<&[String]>) -> Decision {
    let paths = patterns(filters, "paths");
    let paths_ignore = patterns(filters, "pathsIgnore");
    if paths.is_empty() && paths_ignore.is_empty() {
        return Decision::Run;
    }
    let Some(changed) = changed_paths else {
        // Fail-open: the diff is unknown (truncated payload / PR event).
        return Decision::Run;
    };
    let run = if !paths.is_empty() {
        // Defense-in-depth: `paths` wins when both forms are present.
        changed.iter().any(|p| match_ordered(&paths, p))
    } else {
        changed.iter().any(|p| !match_ordered(&paths_ignore, p))
    };
    if run {
        Decision::Run
    } else {
        Decision::Skip(SKIP_PATH_FILTERED)
    }
}

/// One filter list out of stored metadata: strings only, re-capped.
fn patterns(filters: &Value, key: &str) -> Vec<String> {
    filters[key]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .filter(|p| !p.is_empty() && p.len() <= MAX_PATTERN_LEN)
                .map(str::to_string)
                .take(MAX_PATTERNS)
                .collect()
        })
        .unwrap_or_default()
}

/// Ordered filter-list matching with `!` negation: patterns apply in order
/// and the LAST matching pattern decides. A list with no positive pattern
/// matches nothing (GitHub rejects such lists outright; failing closed here
/// is the safe mirror).
pub fn match_ordered(patterns: &[String], value: &str) -> bool {
    let mut matched = false;
    for pattern in patterns {
        if let Some(negated) = pattern.strip_prefix('!') {
            if matched && glob_match(negated, value) {
                matched = false;
            }
        } else if glob_match(pattern, value) {
            matched = true;
        }
    }
    matched
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Lit(char),
    /// `[abc]` / `[a-z]`: inclusive ranges (single chars are (c, c)).
    Class(Vec<(char, char)>),
    /// `*`: any run not crossing `/`.
    AnyNoSlash,
    /// `**`: any run.
    AnyAll,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Quant {
    One,
    /// `?`: zero or one of the preceding token.
    ZeroOrOne,
    /// `+`: one or more of the preceding token.
    OneOrMore,
}

/// GitHub-flavored filter glob. Iterative position-set matching (the classic
/// NFA-simulation shape): O(pattern × value), bounded by the caps above —
/// pathological patterns cannot blow up.
pub fn glob_match(pattern: &str, value: &str) -> bool {
    if pattern.len() > MAX_PATTERN_LEN || value.len() > MAX_VALUE_LEN {
        return false;
    }
    let tokens = tokenize(pattern);
    let value: Vec<char> = value.chars().collect();

    // positions[i] == true → the tokens consumed so far can end at value[i].
    let mut positions = vec![false; value.len() + 1];
    positions[0] = true;

    for (tok, quant) in &tokens {
        let mut next = vec![false; value.len() + 1];
        match tok {
            Tok::AnyNoSlash | Tok::AnyAll => {
                // Quantifiers on a star are meaningless (a star is already a
                // closure) — treat as the star itself.
                let cross_slash = *tok == Tok::AnyAll;
                for start in 0..positions.len() {
                    if !positions[start] {
                        continue;
                    }
                    next[start] = true;
                    for (offset, c) in value[start..].iter().enumerate() {
                        if !cross_slash && *c == '/' {
                            break;
                        }
                        next[start + offset + 1] = true;
                    }
                }
            }
            Tok::Lit(_) | Tok::Class(_) => {
                let matches_at = |pos: usize| -> bool {
                    value.get(pos).is_some_and(|c| match tok {
                        Tok::Lit(l) => c == l,
                        Tok::Class(ranges) => ranges.iter().any(|(lo, hi)| *lo <= *c && *c <= *hi),
                        _ => unreachable!(),
                    })
                };
                for start in 0..positions.len() {
                    if !positions[start] {
                        continue;
                    }
                    match quant {
                        Quant::One => {
                            if matches_at(start) {
                                next[start + 1] = true;
                            }
                        }
                        Quant::ZeroOrOne => {
                            next[start] = true;
                            if matches_at(start) {
                                next[start + 1] = true;
                            }
                        }
                        Quant::OneOrMore => {
                            let mut pos = start;
                            while matches_at(pos) {
                                pos += 1;
                                next[pos] = true;
                            }
                        }
                    }
                }
            }
        }
        positions = next;
        if !positions.iter().any(|p| *p) {
            return false;
        }
    }

    positions[value.len()]
}

/// Pattern → token stream. Malformed constructs degrade to literals (an
/// unclosed `[` matches a literal `[`), matching glob conventions — never a
/// panic, never an error.
fn tokenize(pattern: &str) -> Vec<(Tok, Quant)> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut tokens: Vec<(Tok, Quant)> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                // Escape: next char is a literal; a trailing `\` is itself.
                if i + 1 < chars.len() {
                    tokens.push((Tok::Lit(chars[i + 1]), Quant::One));
                    i += 2;
                } else {
                    tokens.push((Tok::Lit('\\'), Quant::One));
                    i += 1;
                }
            }
            '*' => {
                if chars.get(i + 1) == Some(&'*') {
                    tokens.push((Tok::AnyAll, Quant::One));
                    i += 2;
                } else {
                    tokens.push((Tok::AnyNoSlash, Quant::One));
                    i += 1;
                }
            }
            '[' => match parse_class(&chars[i + 1..]) {
                Some((ranges, consumed)) => {
                    tokens.push((Tok::Class(ranges), Quant::One));
                    i += consumed + 1;
                }
                None => {
                    tokens.push((Tok::Lit('['), Quant::One));
                    i += 1;
                }
            },
            '?' => {
                // Quantifies the preceding token; leading `?` is a literal.
                match tokens.last_mut() {
                    Some(last) if last.1 == Quant::One => last.1 = Quant::ZeroOrOne,
                    _ => tokens.push((Tok::Lit('?'), Quant::One)),
                }
                i += 1;
            }
            '+' => {
                match tokens.last_mut() {
                    Some(last) if last.1 == Quant::One => last.1 = Quant::OneOrMore,
                    _ => tokens.push((Tok::Lit('+'), Quant::One)),
                }
                i += 1;
            }
            c => {
                tokens.push((Tok::Lit(c), Quant::One));
                i += 1;
            }
        }
    }
    tokens
}

/// `[...]` class body: single chars and `a-z` ranges up to the closing `]`.
/// Returns the ranges and how many chars (incl. `]`) were consumed, or None
/// when unclosed/empty.
fn parse_class(rest: &[char]) -> Option<(Vec<(char, char)>, usize)> {
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        match rest[i] {
            ']' => {
                return if ranges.is_empty() {
                    None
                } else {
                    Some((ranges, i + 1))
                };
            }
            c => {
                if rest.get(i + 1) == Some(&'-')
                    && rest.get(i + 2).is_some_and(|end| *end != ']')
                {
                    let end = rest[i + 2];
                    if c <= end {
                        ranges.push((c, end));
                    }
                    i += 3;
                } else {
                    ranges.push((c, c));
                    i += 1;
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(patterns: &[&str]) -> Vec<String> {
        patterns.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn glob_matches_github_documented_examples() {
        // feature/* — `*` does not cross `/`.
        assert!(glob_match("feature/*", "feature/my-branch"));
        assert!(glob_match("feature/*", "feature/your-branch"));
        assert!(!glob_match("feature/*", "feature/beta-a/my-branch"));
        // feature/** — `**` crosses `/`.
        assert!(glob_match("feature/**", "feature/beta-a/my-branch"));
        assert!(glob_match("feature/**", "feature/my-branch"));
        // v2.* — dot is literal.
        assert!(glob_match("v2.*", "v2.0"));
        assert!(glob_match("v2.*", "v2.9"));
        assert!(!glob_match("v2.*", "v3.0"));
        // Character classes with `+`.
        assert!(glob_match("v[12].[0-9]+.[0-9]+", "v1.10.1"));
        assert!(glob_match("v[12].[0-9]+.[0-9]+", "v2.0.0"));
        assert!(!glob_match("v[12].[0-9]+.[0-9]+", "v3.0.0"));
        assert!(!glob_match("v[12].[0-9]+.[0-9]+", "v1..0"));
        // Bare stars.
        assert!(glob_match("*", "main"));
        assert!(!glob_match("*", "releases/x"));
        assert!(glob_match("**", "releases/x/y"));
        // Path globs.
        assert!(glob_match("*.js", "app.js"));
        assert!(!glob_match("*.js", "src/app.js"));
        assert!(glob_match("**.js", "src/app.js"));
        assert!(glob_match("src/**", "src/a/b.rs"));
    }

    #[test]
    fn glob_quantifiers_apply_to_preceding_char() {
        // `?`: zero or one of the preceding character.
        assert!(glob_match("releases/v?", "releases/v"));
        assert!(glob_match("releases/v?", "releases/"));
        assert!(!glob_match("releases/v?", "releases/vv"));
        // `+`: one or more of the preceding character.
        assert!(glob_match("ab+", "ab"));
        assert!(glob_match("ab+", "abbb"));
        assert!(!glob_match("ab+", "a"));
        // Leading `?` / `+` degrade to literals.
        assert!(glob_match("?x", "?x"));
        assert!(glob_match("+y", "+y"));
    }

    #[test]
    fn glob_escapes_and_malformed_classes() {
        assert!(glob_match("\\*literal", "*literal"));
        assert!(!glob_match("\\*literal", "xliteral"));
        assert!(glob_match("a\\[b", "a[b"));
        // Unclosed class is a literal `[`.
        assert!(glob_match("a[bc", "a[bc"));
        // Trailing escape is a literal backslash.
        assert!(glob_match("end\\", "end\\"));
    }

    #[test]
    fn glob_rejects_oversized_inputs() {
        let long_value = "x".repeat(MAX_VALUE_LEN + 1);
        assert!(!glob_match("**", &long_value));
        let long_pattern = "x".repeat(MAX_PATTERN_LEN + 1);
        assert!(!glob_match(&long_pattern, "x"));
    }

    #[test]
    fn ordered_negation_last_match_wins() {
        let patterns = strs(&["releases/**", "!releases/**-alpha"]);
        assert!(match_ordered(&patterns, "releases/10"));
        assert!(match_ordered(&patterns, "releases/beta/mona"));
        assert!(!match_ordered(&patterns, "releases/10-alpha"));
        // A later positive re-includes.
        let patterns = strs(&["releases/**", "!releases/**-alpha", "releases/v1-alpha"]);
        assert!(match_ordered(&patterns, "releases/v1-alpha"));
        // Only-negative lists match nothing (GitHub rejects them; fail closed).
        assert!(!match_ordered(&strs(&["!main"]), "dev"));
    }

    fn meta(filters: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "triggerFilters": filters })
    }

    fn push_ctx<'a>(branch: &'a str, changed: Option<&'a [String]>) -> EventContext<'a> {
        EventContext {
            kind: EventKind::Push { branch },
            changed_paths: changed,
        }
    }

    #[test]
    fn evaluate_requires_declared_event() {
        let triggers = vec!["pull_request".to_string()];
        assert_eq!(
            evaluate(&triggers, &meta(serde_json::json!({})), &push_ctx("main", None)),
            Decision::Skip(SKIP_EVENT_NOT_DECLARED)
        );
    }

    #[test]
    fn evaluate_pre_v3_metadata_runs_unfiltered() {
        let triggers = vec!["push".to_string()];
        // No triggerFilters key at all (pre-v3 rows).
        assert_eq!(
            evaluate(&triggers, &serde_json::json!({}), &push_ctx("anything", None)),
            Decision::Run
        );
        // Malformed shape.
        assert_eq!(
            evaluate(
                &triggers,
                &serde_json::json!({ "triggerFilters": "bogus" }),
                &push_ctx("anything", None)
            ),
            Decision::Run
        );
    }

    #[test]
    fn evaluate_push_branch_filters() {
        let triggers = vec!["push".to_string()];
        let m = meta(serde_json::json!({
            "push": { "branches": ["main", "releases/**", "!releases/**-alpha"] }
        }));
        assert_eq!(evaluate(&triggers, &m, &push_ctx("main", None)), Decision::Run);
        assert_eq!(
            evaluate(&triggers, &m, &push_ctx("releases/10", None)),
            Decision::Run
        );
        assert_eq!(
            evaluate(&triggers, &m, &push_ctx("releases/10-alpha", None)),
            Decision::Skip(SKIP_BRANCH_FILTERED)
        );
        assert_eq!(
            evaluate(&triggers, &m, &push_ctx("dev", None)),
            Decision::Skip(SKIP_BRANCH_FILTERED)
        );

        let m = meta(serde_json::json!({ "push": { "branchesIgnore": ["dev/*"] } }));
        assert_eq!(
            evaluate(&triggers, &m, &push_ctx("dev/x", None)),
            Decision::Skip(SKIP_BRANCH_FILTERED)
        );
        assert_eq!(evaluate(&triggers, &m, &push_ctx("main", None)), Decision::Run);
    }

    #[test]
    fn evaluate_only_tags_blocks_branch_pushes_and_vice_versa() {
        let triggers = vec!["push".to_string()];
        let tags_only = meta(serde_json::json!({ "push": { "tags": ["v*"] } }));
        assert_eq!(
            evaluate(&triggers, &tags_only, &push_ctx("main", None)),
            Decision::Skip(SKIP_BRANCH_FILTERED)
        );
        let tag_ctx = EventContext {
            kind: EventKind::TagPush { tag: "v1.2" },
            changed_paths: None,
        };
        assert_eq!(evaluate(&triggers, &tags_only, &tag_ctx), Decision::Run);

        let branches_only = meta(serde_json::json!({ "push": { "branches": ["main"] } }));
        assert_eq!(
            evaluate(&triggers, &branches_only, &tag_ctx),
            Decision::Skip(SKIP_TAG_FILTERED)
        );
        // Neither dimension filtered: both run.
        let unfiltered = meta(serde_json::json!({ "push": {} }));
        assert_eq!(evaluate(&triggers, &unfiltered, &tag_ctx), Decision::Run);
        assert_eq!(
            evaluate(&triggers, &unfiltered, &push_ctx("main", None)),
            Decision::Run
        );
    }

    #[test]
    fn evaluate_tag_filters() {
        let triggers = vec!["push".to_string()];
        let m = meta(serde_json::json!({ "push": { "tags": ["v[0-9]+.*"] } }));
        let ctx = |tag| EventContext {
            kind: EventKind::TagPush { tag },
            changed_paths: None,
        };
        assert_eq!(evaluate(&triggers, &m, &ctx("v1.0")), Decision::Run);
        assert_eq!(
            evaluate(&triggers, &m, &ctx("latest")),
            Decision::Skip(SKIP_TAG_FILTERED)
        );
        // tags-ignore.
        let m = meta(serde_json::json!({ "push": { "tagsIgnore": ["nightly-*"] } }));
        assert_eq!(
            evaluate(&triggers, &m, &ctx("nightly-2026")),
            Decision::Skip(SKIP_TAG_FILTERED)
        );
        assert_eq!(evaluate(&triggers, &m, &ctx("v1.0")), Decision::Run);
    }

    #[test]
    fn evaluate_paths_require_a_matching_changed_file() {
        let triggers = vec!["push".to_string()];
        let m = meta(serde_json::json!({
            "push": { "branches": ["main"], "paths": ["src/**", "*.toml"] }
        }));
        let changed = vec!["src/lib.rs".to_string(), "README.md".to_string()];
        assert_eq!(
            evaluate(&triggers, &m, &push_ctx("main", Some(&changed))),
            Decision::Run
        );
        let docs_only = vec!["docs/guide.md".to_string()];
        assert_eq!(
            evaluate(&triggers, &m, &push_ctx("main", Some(&docs_only))),
            Decision::Skip(SKIP_PATH_FILTERED)
        );
        // Branch AND path must both pass.
        assert_eq!(
            evaluate(&triggers, &m, &push_ctx("dev", Some(&changed))),
            Decision::Skip(SKIP_BRANCH_FILTERED)
        );
        // Unknown diff fails open.
        assert_eq!(
            evaluate(&triggers, &m, &push_ctx("main", None)),
            Decision::Run
        );
        // Empty diff with a paths filter skips.
        let empty: Vec<String> = Vec::new();
        assert_eq!(
            evaluate(&triggers, &m, &push_ctx("main", Some(&empty))),
            Decision::Skip(SKIP_PATH_FILTERED)
        );
    }

    #[test]
    fn evaluate_paths_ignore_needs_one_unignored_file() {
        let triggers = vec!["push".to_string()];
        let m = meta(serde_json::json!({ "push": { "pathsIgnore": ["docs/**"] } }));
        let docs_only = vec!["docs/a.md".to_string(), "docs/b.md".to_string()];
        assert_eq!(
            evaluate(&triggers, &m, &push_ctx("main", Some(&docs_only))),
            Decision::Skip(SKIP_PATH_FILTERED)
        );
        let mixed = vec!["docs/a.md".to_string(), "src/main.rs".to_string()];
        assert_eq!(
            evaluate(&triggers, &m, &push_ctx("main", Some(&mixed))),
            Decision::Run
        );
    }

    #[test]
    fn evaluate_pull_request_types_and_base_branch() {
        let triggers = vec!["pull_request".to_string()];
        let pr_ctx = |action, base| EventContext {
            kind: EventKind::PullRequest {
                action,
                base_branch: base,
            },
            changed_paths: None,
        };
        // Default types.
        let m = meta(serde_json::json!({ "pullRequest": {} }));
        assert_eq!(evaluate(&triggers, &m, &pr_ctx("opened", "main")), Decision::Run);
        assert_eq!(
            evaluate(&triggers, &m, &pr_ctx("synchronize", "main")),
            Decision::Run
        );
        assert_eq!(
            evaluate(&triggers, &m, &pr_ctx("labeled", "main")),
            Decision::Skip(SKIP_TYPE_FILTERED)
        );
        // Explicit types replace the defaults.
        let m = meta(serde_json::json!({ "pullRequest": { "types": ["closed"] } }));
        assert_eq!(
            evaluate(&triggers, &m, &pr_ctx("opened", "main")),
            Decision::Skip(SKIP_TYPE_FILTERED)
        );
        assert_eq!(evaluate(&triggers, &m, &pr_ctx("closed", "main")), Decision::Run);
        // Branch filters match the BASE branch.
        let m = meta(serde_json::json!({ "pullRequest": { "branches": ["main"] } }));
        assert_eq!(evaluate(&triggers, &m, &pr_ctx("opened", "main")), Decision::Run);
        assert_eq!(
            evaluate(&triggers, &m, &pr_ctx("opened", "dev")),
            Decision::Skip(SKIP_BRANCH_FILTERED)
        );
    }
}
