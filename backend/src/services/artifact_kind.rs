//! Server-side artifact classification. The kind drives per-kind retention
//! policies and catalog filtering; it is derived from the (already
//! allow-list-validated) artifact name only — never from runner-supplied
//! metadata. Keep the suffix map in sync with the backfill CASE in
//! migrations/20260711100001_artifacts_metadata.sql.

/// Every value the classifier can return; also the filter allow-list.
pub const KINDS: &[&str] = &[
    "package", "report", "docs", "archive", "binary", "image", "log", "other",
];

const SUFFIX_MAP: &[(&str, &str)] = &[
    (".crate", "package"),
    (".whl", "package"),
    (".gem", "package"),
    (".jar", "package"),
    (".deb", "package"),
    (".rpm", "package"),
    (".nupkg", "package"),
    (".apk", "package"),
    // tar.gz/tar.bz2 must sort before plain extension checks; the map is
    // scanned in order and these longer suffixes appear first.
    (".tar.gz", "archive"),
    (".tar.bz2", "archive"),
    (".zip", "archive"),
    (".tar", "archive"),
    (".tgz", "archive"),
    (".7z", "archive"),
    (".png", "image"),
    (".jpg", "image"),
    (".jpeg", "image"),
    (".gif", "image"),
    (".svg", "image"),
    (".webp", "image"),
    (".ico", "image"),
    (".html", "docs"),
    (".htm", "docs"),
    (".pdf", "docs"),
    (".md", "docs"),
    (".xml", "report"),
    (".sarif", "report"),
    (".lcov", "report"),
    (".junit", "report"),
    (".log", "log"),
    (".txt", "log"),
    (".exe", "binary"),
    (".dll", "binary"),
    (".so", "binary"),
    (".dylib", "binary"),
    (".wasm", "binary"),
    (".bin", "binary"),
];

/// Classify an artifact name onto the static kind allow-list.
pub fn classify(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    for (suffix, kind) in SUFFIX_MAP {
        if lower.ends_with(suffix) {
            return kind;
        }
    }
    "other"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_each_kind() {
        assert_eq!(classify("app-1.0.0.crate"), "package");
        assert_eq!(classify("coverage.lcov"), "report");
        assert_eq!(classify("site.html"), "docs");
        assert_eq!(classify("dist.tar.gz"), "archive");
        assert_eq!(classify("server.exe"), "binary");
        assert_eq!(classify("screenshot.PNG"), "image");
        assert_eq!(classify("build.log"), "log");
        assert_eq!(classify("mystery"), "other");
        assert_eq!(classify("noextension."), "other");
    }

    #[test]
    fn compound_suffixes_win_over_plain_gz() {
        // .tar.gz is an archive even though bare .gz is unmapped.
        assert_eq!(classify("bundle.tar.gz"), "archive");
        assert_eq!(classify("bundle.tgz"), "archive");
    }

    #[test]
    fn every_result_is_on_the_allow_list() {
        for name in ["a.zip", "b.exe", "c.unknown", "d.md", "e.log"] {
            assert!(KINDS.contains(&classify(name)));
        }
    }
}
