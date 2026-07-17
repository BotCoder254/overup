//! Central catalog of the language-toolchain images overup understands.
//!
//! Every job runs in a Docker image. The base `catthehacker/ubuntu:act-*`
//! family ships the common GitHub-runner toolchains (Node, Python, git,
//! build-essential), but the `catthehacker/ubuntu` repository also publishes
//! specialized tags with a language's tooling pre-built (Rust with
//! rustfmt/clippy, a JS image with yarn/pnpm/nvm, Go, .NET, Java, PowerShell,
//! the GitHub CLI). This module is the single source of truth for that set:
//! the planner resolves short `container:` aliases against it, the read
//! endpoint serves it to the UI, and the allow-list check trusts it.
//!
//! Everything here is static and side-effect free — the image strings are
//! `&'static str`, so no user input ever flows into a resolved image.

/// One toolchain image family (three Ubuntu flavors of the same tooling).
pub struct Toolchain {
    /// Short alias an author types as `container: <key>`.
    pub key: &'static str,
    /// Human label for the UI.
    pub label: &'static str,
    /// Primary language / ecosystem the image targets.
    pub language: &'static str,
    /// One-line description of what the image provides.
    pub description: &'static str,
    /// Notable pre-installed tools, for the UI.
    pub tools: &'static [&'static str],
    pub image_latest: &'static str,
    pub image_2204: &'static str,
    pub image_2404: &'static str,
    /// `full-*` is a ~60 GB filesystem dump of the GitHub-hosted runner — far
    /// too big to prewarm by default; flagged so the UI can warn.
    pub large: bool,
}

/// The catalog. `act` is the base/default family; the rest add a language
/// toolchain on top of it. `java` maps to catthehacker's `java-tools-*` tag.
const CATALOG: &[Toolchain] = &[
    Toolchain {
        key: "act",
        label: "Base (act)",
        language: "General",
        description:
            "GitHub Actions-compatible base image and the default for `runs-on: ubuntu-*`. \
             Ships Node, Python, git, and build-essential.",
        tools: &["node", "python3", "git", "build-essential", "curl"],
        image_latest: "catthehacker/ubuntu:act-latest",
        image_2204: "catthehacker/ubuntu:act-22.04",
        image_2404: "catthehacker/ubuntu:act-24.04",
        large: false,
    },
    Toolchain {
        key: "rust",
        label: "Rust",
        language: "Rust",
        description: "Rust toolchain with cargo, rustfmt, clippy, and cbindgen pre-installed.",
        tools: &["rustc", "cargo", "rustfmt", "clippy", "cbindgen"],
        image_latest: "catthehacker/ubuntu:rust-latest",
        image_2204: "catthehacker/ubuntu:rust-22.04",
        image_2404: "catthehacker/ubuntu:rust-24.04",
        large: false,
    },
    Toolchain {
        key: "js",
        label: "JavaScript / Node.js",
        language: "JavaScript",
        description: "Heavy JS tooling: Node 20/24 via nvm, plus yarn, pnpm, and grunt.",
        tools: &["node", "npm", "yarn", "pnpm", "nvm", "grunt"],
        image_latest: "catthehacker/ubuntu:js-latest",
        image_2204: "catthehacker/ubuntu:js-22.04",
        image_2404: "catthehacker/ubuntu:js-24.04",
        large: false,
    },
    Toolchain {
        key: "go",
        label: "Go",
        language: "Go",
        description: "Go toolchain with the standard build tools pre-installed.",
        tools: &["go", "gofmt"],
        image_latest: "catthehacker/ubuntu:go-latest",
        image_2204: "catthehacker/ubuntu:go-22.04",
        image_2404: "catthehacker/ubuntu:go-24.04",
        large: false,
    },
    Toolchain {
        key: "dotnet",
        label: ".NET",
        language: ".NET",
        description: ".NET SDK and runtime tooling pre-installed.",
        tools: &["dotnet"],
        image_latest: "catthehacker/ubuntu:dotnet-latest",
        image_2204: "catthehacker/ubuntu:dotnet-22.04",
        image_2404: "catthehacker/ubuntu:dotnet-24.04",
        large: false,
    },
    Toolchain {
        key: "java",
        label: "Java",
        language: "Java",
        description: "JDK with the common Java build tools (Maven, Gradle) pre-installed.",
        tools: &["java", "maven", "gradle"],
        image_latest: "catthehacker/ubuntu:java-tools-latest",
        image_2204: "catthehacker/ubuntu:java-tools-22.04",
        image_2404: "catthehacker/ubuntu:java-tools-24.04",
        large: false,
    },
    Toolchain {
        key: "pwsh",
        label: "PowerShell",
        language: "PowerShell",
        description: "PowerShell (pwsh) with common modules pre-installed.",
        tools: &["pwsh"],
        image_latest: "catthehacker/ubuntu:pwsh-latest",
        image_2204: "catthehacker/ubuntu:pwsh-22.04",
        image_2404: "catthehacker/ubuntu:pwsh-24.04",
        large: false,
    },
    Toolchain {
        key: "gh",
        label: "GitHub CLI",
        language: "GitHub CLI",
        description: "The GitHub CLI (`gh`) and git on top of the base image.",
        tools: &["gh", "git"],
        image_latest: "catthehacker/ubuntu:gh-latest",
        image_2204: "catthehacker/ubuntu:gh-22.04",
        image_2404: "catthehacker/ubuntu:gh-24.04",
        large: false,
    },
    Toolchain {
        key: "full",
        label: "Full runner",
        language: "All",
        description:
            "A complete dump of the GitHub-hosted runner filesystem — every toolchain, but \
             very large (~60 GB extracted). Prefer a language-specific image where possible.",
        tools: &["(full GitHub-hosted runner toolset)"],
        image_latest: "catthehacker/ubuntu:full-latest",
        image_2204: "catthehacker/ubuntu:full-22.04",
        image_2404: "catthehacker/ubuntu:full-24.04",
        large: true,
    },
];

/// The full toolchain catalog.
pub fn all() -> &'static [Toolchain] {
    CATALOG
}

/// Resolve a short `container:` alias (`rust`, `rust-latest`, `rust-22.04`, …)
/// to a concrete catalog image. Returns `None` for anything that is not a bare
/// alias so full image references (which contain `/`, `:`, or `@`) fall
/// through to the normal reference-validation path unchanged.
///
/// An explicit version suffix on the alias wins; otherwise the job's
/// `runs-on` Ubuntu version (`os_version`, "22.04"/"24.04") is used; otherwise
/// `-latest`.
pub fn resolve_alias(value: &str, os_version: Option<&str>) -> Option<&'static str> {
    let v = value.trim().to_ascii_lowercase();
    // A short alias is a bare token — never a full image reference.
    if v.is_empty() || v.contains(['/', ':', '@']) {
        return None;
    }

    let (key, explicit_ver) = if let Some(k) = v.strip_suffix("-latest") {
        (k.to_string(), Some("latest"))
    } else if let Some(k) = v.strip_suffix("-24.04") {
        (k.to_string(), Some("24.04"))
    } else if let Some(k) = v.strip_suffix("-22.04") {
        (k.to_string(), Some("22.04"))
    } else {
        (v.clone(), None)
    };

    let toolchain = CATALOG.iter().find(|t| t.key == key)?;
    let version = explicit_ver.or(os_version).unwrap_or("latest");
    Some(match version {
        "24.04" => toolchain.image_2404,
        "22.04" => toolchain.image_2204,
        _ => toolchain.image_latest,
    })
}

/// Whether `image` is one of the exact catalog image references (any flavor).
/// Used so catalog images always pass the optional image allow-list.
pub fn is_catalog_image(image: &str) -> bool {
    CATALOG.iter().any(|t| {
        image == t.image_latest || image == t.image_2204 || image == t.image_2404
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_bare_alias_to_latest() {
        assert_eq!(
            resolve_alias("rust", None),
            Some("catthehacker/ubuntu:rust-latest")
        );
        assert_eq!(
            resolve_alias("JS", None),
            Some("catthehacker/ubuntu:js-latest")
        );
    }

    #[test]
    fn java_alias_maps_to_java_tools_tag() {
        assert_eq!(
            resolve_alias("java", None),
            Some("catthehacker/ubuntu:java-tools-latest")
        );
    }

    #[test]
    fn explicit_version_suffix_wins_over_runs_on() {
        assert_eq!(
            resolve_alias("rust-22.04", Some("24.04")),
            Some("catthehacker/ubuntu:rust-22.04")
        );
    }

    #[test]
    fn runs_on_version_pairs_when_no_suffix() {
        assert_eq!(
            resolve_alias("go", Some("24.04")),
            Some("catthehacker/ubuntu:go-24.04")
        );
        assert_eq!(
            resolve_alias("go", Some("22.04")),
            Some("catthehacker/ubuntu:go-22.04")
        );
    }

    #[test]
    fn unknown_alias_is_none() {
        assert_eq!(resolve_alias("ruby", None), None);
        assert_eq!(resolve_alias("", None), None);
    }

    #[test]
    fn full_reference_is_not_treated_as_alias() {
        // Already a valid reference — must not be swallowed by the resolver.
        assert_eq!(resolve_alias("catthehacker/ubuntu:rust-latest", None), None);
        assert_eq!(resolve_alias("node:22", None), None);
    }

    #[test]
    fn catalog_image_recognized() {
        assert!(is_catalog_image("catthehacker/ubuntu:rust-latest"));
        assert!(is_catalog_image("catthehacker/ubuntu:java-tools-24.04"));
        assert!(!is_catalog_image("node:22"));
    }
}
