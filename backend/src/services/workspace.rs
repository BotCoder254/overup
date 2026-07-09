//! Centralized validation and canonical slug generation for workspaces.
//!
//! All rules follow the OWASP input-validation guidance: normalize to a
//! canonical encoding first, validate against character-category
//! allow-lists (never deny-lists), enforce length bounds, and reject
//! invalid input outright instead of silently repairing it. The client's
//! slug preview is cosmetic — everything here is the authority.

use unicode_general_category::{GeneralCategory, get_general_category};
use unicode_normalization::UnicodeNormalization;

use crate::error::AppError;

pub const NAME_MIN_CHARS: usize = 2;
pub const NAME_MAX_CHARS: usize = 80;
pub const DESCRIPTION_MAX_CHARS: usize = 500;
pub const SLUG_MAX_LEN: usize = 50;
pub const MAX_SLUG_ATTEMPTS: u32 = 10;

/// Route and infrastructure names a workspace slug may never shadow.
pub const RESERVED_SLUGS: &[&str] = &[
    "admin",
    "api",
    "app",
    "assets",
    "auth",
    "callback",
    "dashboard",
    "docs",
    "health",
    "healthz",
    "help",
    "login",
    "logout",
    "me",
    "new",
    "null",
    "onboarding",
    "root",
    "settings",
    "static",
    "support",
    "system",
    "undefined",
    "w",
    "workspace",
    "workspaces",
    "www",
];

/// True for the printable Unicode general categories we accept in names:
/// letters (L*), marks (M*), numbers (N*), punctuation (P*), symbols (S*).
/// Everything else — controls (Cc), format chars like bidi overrides and
/// zero-width joiners (Cf), surrogates (Cs), private use (Co), unassigned
/// (Cn), and line/paragraph separators (Zl/Zp) — is rejected.
fn is_allowed_name_char(ch: char) -> bool {
    use GeneralCategory as G;
    matches!(
        get_general_category(ch),
        G::UppercaseLetter
            | G::LowercaseLetter
            | G::TitlecaseLetter
            | G::ModifierLetter
            | G::OtherLetter
            | G::NonspacingMark
            | G::SpacingMark
            | G::EnclosingMark
            | G::DecimalNumber
            | G::LetterNumber
            | G::OtherNumber
            | G::ConnectorPunctuation
            | G::DashPunctuation
            | G::OpenPunctuation
            | G::ClosePunctuation
            | G::InitialPunctuation
            | G::FinalPunctuation
            | G::OtherPunctuation
            | G::MathSymbol
            | G::CurrencySymbol
            | G::ModifierSymbol
            | G::OtherSymbol
    )
}

/// NFC-normalize, trim, collapse internal whitespace runs to single spaces,
/// then enforce length bounds and the character allow-list.
fn normalize_and_validate_text(
    raw: &str,
    field: &str,
    min_chars: usize,
    max_chars: usize,
) -> Result<String, AppError> {
    let normalized: String = raw.nfc().collect();
    // split_whitespace uses char::is_whitespace, so tabs, newlines and
    // Unicode spaces (NBSP & friends) are trimmed and collapsed alike.
    let collapsed = normalized.split_whitespace().collect::<Vec<_>>().join(" ");

    let len = collapsed.chars().count();
    if len < min_chars {
        return Err(AppError::Validation(format!(
            "{field} must be at least {min_chars} characters"
        )));
    }
    if len > max_chars {
        return Err(AppError::Validation(format!(
            "{field} must be at most {max_chars} characters"
        )));
    }

    for ch in collapsed.chars() {
        if ch != ' ' && !is_allowed_name_char(ch) {
            return Err(AppError::Validation(format!(
                "{field} contains unsupported characters"
            )));
        }
    }

    Ok(collapsed)
}

pub fn normalize_and_validate_name(raw: &str) -> Result<String, AppError> {
    normalize_and_validate_text(raw, "workspace name", NAME_MIN_CHARS, NAME_MAX_CHARS)
}

/// Same pipeline as the name, but optional: empty or whitespace-only input
/// becomes `None` rather than an error.
pub fn normalize_and_validate_description(
    raw: Option<&str>,
) -> Result<Option<String>, AppError> {
    match raw {
        None => Ok(None),
        Some(text) if text.trim().is_empty() => Ok(None),
        Some(text) => {
            normalize_and_validate_text(text, "description", 0, DESCRIPTION_MAX_CHARS).map(Some)
        }
    }
}

/// Derive the base slug from a validated name: NFKD-decompose so accented
/// letters contribute their ASCII base, keep lowercased ASCII alphanumerics,
/// fold every other run into a single hyphen, trim edges, cap the length.
/// Names with no ASCII representation (all-CJK, emoji) fall back to
/// "workspace" so the suffix ladder still produces a valid slug.
pub fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.nfkd() {
        // NFKD splits accented letters into base + combining marks; the
        // marks must vanish, not act as separators ("É" → "e", not "e-").
        if matches!(
            get_general_category(ch),
            GeneralCategory::NonspacingMark
                | GeneralCategory::SpacingMark
                | GeneralCategory::EnclosingMark
        ) {
            continue;
        }
        let ch = ch.to_ascii_lowercase();
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    let mut capped: String = trimmed.chars().take(SLUG_MAX_LEN).collect();
    while capped.ends_with('-') {
        capped.pop();
    }
    if capped.is_empty() {
        "workspace".to_string()
    } else {
        capped
    }
}

/// Deterministic collision ladder: `base`, `base-2`, `base-3`, ...
/// Reserved bases never appear bare — attempt 0 already carries a suffix.
/// The result always fits within [`SLUG_MAX_LEN`].
pub fn slug_candidate(base: &str, attempt: u32) -> String {
    if attempt == 0 && !RESERVED_SLUGS.contains(&base) {
        return base.to_string();
    }
    let suffix = format!("-{}", attempt + 1);
    let keep = SLUG_MAX_LEN.saturating_sub(suffix.len());
    let head: String = base.chars().take(keep).collect();
    format!("{}{}", head.trim_end_matches('-'), suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_trims_and_collapses_whitespace() {
        let name = normalize_and_validate_name("  My\t Automation \u{00a0} Platform  ").unwrap();
        assert_eq!(name, "My Automation Platform");
    }

    #[test]
    fn name_rejects_control_characters() {
        assert!(matches!(
            normalize_and_validate_name("a\u{0007}b"),
            Err(AppError::Validation(_))
        ));
    }

    #[test]
    fn name_rejects_bidi_override() {
        assert!(matches!(
            normalize_and_validate_name("evil\u{202e}name"),
            Err(AppError::Validation(_))
        ));
    }

    #[test]
    fn name_rejects_too_short_and_too_long() {
        assert!(normalize_and_validate_name("a").is_err());
        assert!(normalize_and_validate_name("   ").is_err());
        let max = "x".repeat(NAME_MAX_CHARS);
        assert!(normalize_and_validate_name(&max).is_ok());
        let over = "x".repeat(NAME_MAX_CHARS + 1);
        assert!(normalize_and_validate_name(&over).is_err());
    }

    #[test]
    fn name_accepts_unicode_letters_and_symbols() {
        assert!(normalize_and_validate_name("Émile's Café — Team 1").is_ok());
        assert!(normalize_and_validate_name("日本語ワークスペース").is_ok());
        assert!(normalize_and_validate_name("Rockets 🚀").is_ok());
    }

    #[test]
    fn description_empty_becomes_none() {
        assert_eq!(normalize_and_validate_description(None).unwrap(), None);
        assert_eq!(normalize_and_validate_description(Some("   ")).unwrap(), None);
        assert_eq!(
            normalize_and_validate_description(Some(" builds things ")).unwrap(),
            Some("builds things".to_string())
        );
    }

    #[test]
    fn slugify_transliterates_diacritics() {
        assert_eq!(slugify("Émile's Café"), "emile-s-cafe");
        assert_eq!(slugify("My Automation Platform"), "my-automation-platform");
    }

    #[test]
    fn slugify_falls_back_for_non_ascii_names() {
        assert_eq!(slugify("日本語"), "workspace");
        assert_eq!(slugify("🚀🚀"), "workspace");
    }

    #[test]
    fn slugify_caps_length_without_trailing_hyphen() {
        let long = "word ".repeat(30);
        let slug = slugify(&long);
        assert!(slug.chars().count() <= SLUG_MAX_LEN);
        assert!(!slug.ends_with('-'));
        assert!(!slug.starts_with('-'));
    }

    #[test]
    fn reserved_slug_never_appears_bare() {
        assert_eq!(slug_candidate("admin", 0), "admin-1");
        assert_eq!(slug_candidate("admin", 1), "admin-2");
    }

    #[test]
    fn candidate_ladder_is_deterministic() {
        assert_eq!(slug_candidate("acme", 0), "acme");
        assert_eq!(slug_candidate("acme", 1), "acme-2");
        assert_eq!(slug_candidate("acme", 2), "acme-3");
    }

    #[test]
    fn candidate_fits_length_cap() {
        let base = "b".repeat(SLUG_MAX_LEN);
        let candidate = slug_candidate(&base, 9);
        assert!(candidate.chars().count() <= SLUG_MAX_LEN);
        assert!(candidate.ends_with("-10"));
    }
}
