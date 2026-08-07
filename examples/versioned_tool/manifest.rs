//! Recipe manifest for the versioned-tool vertical: typed model + TOML parsing.
//!
//! Owns the caller-side recipe format (s2-11: no core data contract). `Manifest::parse` reads a
//! TOML document into the typed model and then validates the frozen laws: name and version are
//! single non-empty path components, an http(s) source URL, a well-formed hex digest of the right
//! length, an `expose` subpath that is relative and cannot escape the tree, and an absolute
//! `link_at` view path when one is declared. The materialization procedure is NOT declared by the
//! user: the vertical detects zip/tar magic and decompresses, otherwise copies — a compressed
//! package is never treated as an activatable view.
use std::fmt;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

/// Caller-authored recipe for one versioned tool artifact.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Artifact identity; a single non-empty path component.
    pub name: String,
    /// Resolved version; a single non-empty path component.
    pub version: String,
    /// Where the artifact bytes come from.
    pub source: Source,
    /// Digest expectation used by the verify step.
    pub hash: HashExpectation,
    /// Subpath of the materialized tree the active view links; default is the tree root.
    #[serde(default)]
    pub expose: Option<String>,
    /// Absolute path where the directory-symlink view is created; absent means no view.
    #[serde(default)]
    pub link_at: Option<PathBuf>,
}

impl Manifest {
    /// Parse a TOML recipe and validate the frozen manifest laws.
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        let manifest: Manifest = toml::from_str(text).map_err(|error| ManifestError::Toml {
            message: error.to_string(),
        })?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<(), ManifestError> {
        if !is_single_component(&self.name) {
            return Err(ManifestError::InvalidName {
                name: self.name.clone(),
            });
        }
        if !is_single_component(&self.version) {
            return Err(ManifestError::InvalidVersion {
                version: self.version.clone(),
            });
        }
        if let Source::Url { url } = &self.source
            && !(url.starts_with("https://") || url.starts_with("http://"))
        {
            return Err(ManifestError::InvalidSourceUrl { url: url.clone() });
        }
        match &self.hash {
            HashExpectation::Blake3 { hex } | HashExpectation::Sha2 { hex } => {
                if !valid_digest_hex(hex) {
                    return Err(ManifestError::InvalidHash {
                        kind: "blake3|sha2",
                        hex: hex.clone(),
                    });
                }
            }
        }
        if let Some(expose) = &self.expose
            && !valid_expose(expose)
        {
            return Err(ManifestError::InvalidExpose {
                expose: expose.clone(),
            });
        }
        if let Some(link_at) = &self.link_at
            && !link_at.is_absolute()
        {
            return Err(ManifestError::RelativeLinkAt {
                path: link_at.clone(),
            });
        }
        Ok(())
    }
}

/// A value is usable as one path component: exactly one non-empty `Normal` component.
pub(crate) fn is_single_component(value: &str) -> bool {
    let mut components = Path::new(value).components();
    matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(part)), None) if !part.is_empty()
    )
}

/// An `expose` subpath is relative, non-empty, and cannot escape the versioned tree.
fn valid_expose(expose: &str) -> bool {
    let path = Path::new(expose);
    if path.as_os_str().is_empty() || path.is_absolute() {
        return false;
    }
    path.components()
        .all(|component| matches!(component, Component::Normal(_)))
}

/// Both digest families are 32-byte digests, i.e. exactly 64 lowercase hex digits.
fn valid_digest_hex(hex: &str) -> bool {
    hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Source of the artifact bytes.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Source {
    Url { url: String },
    Local { path: PathBuf },
}

/// Digest expectation carried by the manifest.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum HashExpectation {
    Blake3 { hex: String },
    Sha2 { hex: String },
}

/// Failure of manifest parsing or validation; errors name the offending field.
#[derive(Debug)]
pub enum ManifestError {
    Toml { message: String },
    InvalidName { name: String },
    InvalidVersion { version: String },
    InvalidSourceUrl { url: String },
    InvalidHash { kind: &'static str, hex: String },
    InvalidExpose { expose: String },
    RelativeLinkAt { path: PathBuf },
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Toml { message } => write!(f, "invalid manifest: {message}"),
            Self::InvalidName { name } => write!(
                f,
                "manifest name {name:?} must be a single non-empty path component"
            ),
            Self::InvalidVersion { version } => write!(
                f,
                "manifest version {version:?} must be a single non-empty path component"
            ),
            Self::InvalidSourceUrl { url } => {
                write!(f, "manifest source url {url:?} must be http(s)")
            }
            Self::InvalidHash { kind, hex } => {
                write!(f, "manifest {kind} digest {hex:?} must be 64 hex digits")
            }
            Self::InvalidExpose { expose } => write!(
                f,
                "manifest expose {expose:?} must be a relative path that stays inside the tree"
            ),
            Self::RelativeLinkAt { path } => {
                write!(f, "manifest link_at {path:?} must be an absolute path")
            }
        }
    }
}

impl std::error::Error for ManifestError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    const LINK_AT: &str = "C:/opt/demo-tool";
    #[cfg(not(windows))]
    const LINK_AT: &str = "/opt/demo-tool";

    fn valid() -> String {
        format!(
            r#"
name = "demo-tool"
version = "1.2.0"
expose = "bin"
link_at = "{LINK_AT}"

[source]
kind = "url"
url = "https://example.com/demo-tool/1.2.0/demo-tool.zip"

[hash]
kind = "blake3"
hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
"#
        )
    }

    #[test]
    fn parses_a_full_valid_manifest() {
        let manifest = Manifest::parse(&valid()).unwrap();
        assert_eq!(manifest.name, "demo-tool");
        assert_eq!(manifest.version, "1.2.0");
        match manifest.source {
            Source::Url { url } => {
                assert_eq!(url, "https://example.com/demo-tool/1.2.0/demo-tool.zip")
            }
            Source::Local { .. } => panic!("expected a url source"),
        }
        match manifest.hash {
            HashExpectation::Blake3 { hex } => assert_eq!(hex.len(), 64),
            HashExpectation::Sha2 { .. } => panic!("expected blake3"),
        }
        assert_eq!(manifest.expose.as_deref(), Some("bin"));
        assert_eq!(manifest.link_at, Some(PathBuf::from(LINK_AT)));
    }

    #[test]
    fn parses_a_local_source_with_sha2_and_no_view() {
        let manifest = Manifest::parse(
            r#"
name = "local-tool"
version = "0.4.1"

[source]
kind = "local"
path = "vendor/local-tool.tar"

[hash]
kind = "sha2"
hex = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
"#,
        )
        .unwrap();
        assert!(matches!(manifest.source, Source::Local { .. }));
        assert!(matches!(manifest.hash, HashExpectation::Sha2 { .. }));
        assert!(manifest.expose.is_none());
        assert!(manifest.link_at.is_none());
    }

    #[test]
    fn rejects_an_unknown_field() {
        let manifest = "name = \"demo-tool\"\nversion = \"1.0.0\"\nunknown = true\n";
        let error = Manifest::parse(manifest).unwrap_err();
        assert!(matches!(error, ManifestError::Toml { .. }));
    }

    #[test]
    fn rejects_a_missing_required_field() {
        let error = Manifest::parse("name = \"demo-tool\"\n").unwrap_err();
        assert!(matches!(error, ManifestError::Toml { .. }));
    }

    #[test]
    fn rejects_an_invalid_name() {
        for name in ["", "a/b", "../up", ".", ".."] {
            let manifest = format!(
                r#"
name = "{name}"
version = "1.0.0"

[source]
kind = "url"
url = "https://example.com/tool.zip"

[hash]
kind = "blake3"
hex = "0000000000000000000000000000000000000000000000000000000000000000"
"#
            );
            let error = Manifest::parse(&manifest).unwrap_err();
            assert!(
                matches!(error, ManifestError::InvalidName { .. }),
                "name {name:?} should be rejected"
            );
        }
    }

    #[test]
    fn rejects_an_invalid_version() {
        let manifest = r#"
name = "demo-tool"
version = "1.0/.."

[source]
kind = "url"
url = "https://example.com/tool.zip"

[hash]
kind = "blake3"
hex = "0000000000000000000000000000000000000000000000000000000000000000"
"#;
        let error = Manifest::parse(manifest).unwrap_err();
        assert!(matches!(error, ManifestError::InvalidVersion { .. }));
    }

    #[test]
    fn rejects_a_non_http_source_url() {
        let manifest = r#"
name = "demo-tool"
version = "1.0.0"

[source]
kind = "url"
url = "ftp://example.com/tool.zip"

[hash]
kind = "blake3"
hex = "0000000000000000000000000000000000000000000000000000000000000000"
"#;
        let error = Manifest::parse(manifest).unwrap_err();
        assert!(matches!(error, ManifestError::InvalidSourceUrl { .. }));
    }

    #[test]
    fn rejects_a_malformed_hash() {
        for hex in ["not-hex", "abcdef", "zz", &"0".repeat(63)] {
            let manifest = format!(
                r#"
name = "demo-tool"
version = "1.0.0"

[source]
kind = "url"
url = "https://example.com/tool.zip"

[hash]
kind = "blake3"
hex = "{hex}"
"#
            );
            let error = Manifest::parse(&manifest).unwrap_err();
            assert!(
                matches!(error, ManifestError::InvalidHash { .. }),
                "hash {hex:?} should be rejected"
            );
        }
    }

    #[test]
    fn rejects_an_invalid_expose() {
        for expose in ["", "/bin", "../bin", "a/../b", ".."] {
            let manifest = format!(
                r#"
name = "demo-tool"
version = "1.0.0"
expose = "{expose}"

[source]
kind = "url"
url = "https://example.com/tool.zip"

[hash]
kind = "blake3"
hex = "0000000000000000000000000000000000000000000000000000000000000000"
"#
            );
            let error = Manifest::parse(&manifest).unwrap_err();
            assert!(
                matches!(error, ManifestError::InvalidExpose { .. }),
                "expose {expose:?} should be rejected"
            );
        }
    }

    #[test]
    fn accepts_a_nested_expose() {
        let manifest = format!(
            r#"
name = "demo-tool"
version = "1.0.0"
expose = "opt/demo-tool/bin"

[source]
kind = "url"
url = "https://example.com/tool.zip"

[hash]
kind = "blake3"
hex = "0000000000000000000000000000000000000000000000000000000000000000"
"#
        );
        let manifest = Manifest::parse(&manifest).unwrap();
        assert_eq!(manifest.expose.as_deref(), Some("opt/demo-tool/bin"));
    }

    #[test]
    fn rejects_a_relative_link_at() {
        let manifest = r#"
name = "demo-tool"
version = "1.0.0"
link_at = "opt/demo-tool"

[source]
kind = "url"
url = "https://example.com/tool.zip"

[hash]
kind = "blake3"
hex = "0000000000000000000000000000000000000000000000000000000000000000"
"#;
        let error = Manifest::parse(manifest).unwrap_err();
        assert!(matches!(error, ManifestError::RelativeLinkAt { .. }));
    }
}
