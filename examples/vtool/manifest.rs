//! Recipe manifest for the versioned-tool vertical: typed model + TOML parsing.
//!
//! Every law is enforced by the types as the document deserializes (`law_newtype!` values with
//! `FromStr` validation over the core `RemoteUrl`/`DigestValue` gates) — no post-parse validation.
use std::fmt;
use std::path::{Path, PathBuf};

use pulith::hash::DigestValue;
use pulith::net::RemoteUrl;
use serde::Deserialize;

/// A law-enforcing value: `FromStr` is the validation entry, `TryFrom<String>` is a thin serde
/// bridge (the `#[serde(try_from = "String")]` container attribute needs it).
macro_rules! law_newtype {
    ($name:ident, $inner:ty, $err:ty, $parse:expr) => {
        #[derive(Clone, Debug, PartialEq, Deserialize)]
        #[serde(try_from = "String")]
        pub struct $name($inner);

        impl std::str::FromStr for $name {
            type Err = $err;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Ok(Self($parse(value)?))
            }
        }

        impl TryFrom<String> for $name {
            type Error = $err;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                value.parse()
            }
        }
    };
}

// A single non-empty path component — the name/version law.
law_newtype!(Component, String, String, |value: &str| {
    if is_single_component(value) {
        Ok(value.to_string())
    } else {
        Err(format!(
            "value {value:?} must be a single non-empty path component"
        ))
    }
});

impl Component {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

// An absolute view path — the `link_at` law.
law_newtype!(ViewPath, PathBuf, String, |value: &str| {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(format!("value {value:?} must be an absolute path"))
    }
});

impl ViewPath {
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

fn is_single_component(value: &str) -> bool {
    let mut components = Path::new(value).components();
    matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(part)), None) if !part.is_empty()
    )
}

/// Caller-authored recipe for one versioned tool artifact.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Artifact identity; a single non-empty path component.
    pub name: Component,
    /// Resolved version; a single non-empty path component.
    pub version: Component,
    /// Subpath the active view links; the core link law validates it when a view is linked.
    #[serde(default)]
    pub expose: Option<PathBuf>,
    /// Absolute view path; absent means no view.
    #[serde(default)]
    pub link_at: Option<ViewPath>,
    /// Atomic source+hash pair per platform; absent means no artifact for that OS.
    pub windows: Option<PlatformSpec>,
    pub linux: Option<PlatformSpec>,
}

/// One platform's artifact: the bytes source and the digest expectation, atomically.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct PlatformSpec {
    pub source: Source,
    pub hash: DigestValue,
}

impl Manifest {
    /// Parse a TOML recipe; every manifest law is enforced by the types as it deserializes.
    /// A recipe with no running-OS platform pair is admitted by parse and refused by resolve.
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        toml::from_str(text).map_err(|error| ManifestError::Toml {
            message: error.to_string(),
        })
    }
}

/// Source of the artifact bytes.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Source {
    Url { url: RemoteUrl },
    Local { path: PathBuf },
}

/// The admitted plan: validated recipe plus the exact target, view, and selected OS pair.
#[derive(Debug)]
pub struct Resolved {
    pub manifest: Manifest,
    pub target: PathBuf,
    pub view: Option<PathBuf>,
    /// The running OS's artifact source (selected from the flattened platform pairs).
    pub source: Source,
    /// The running OS's digest expectation for that source.
    pub hash: DigestValue,
}

/// Failure of pure resolve; all failures name the offending field.
#[derive(Debug)]
pub enum ResolveError {
    NoSourceForPlatform { platform: &'static str },
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSourceForPlatform { platform } => {
                write!(
                    f,
                    "manifest declares no source+hash pair for platform {platform:?}"
                )
            }
        }
    }
}

impl std::error::Error for ResolveError {}

impl Manifest {
    /// Admit identity and policy: compute the exact target and view paths and select the OS
    /// artifact. No I/O; the manifest pins the source (deliberately not dependency resolution).
    pub fn resolve(self, root: &Path) -> Result<Resolved, ResolveError> {
        let target = root
            .join("artifacts")
            .join(self.name.as_str())
            .join(self.version.as_str());
        let view = self
            .link_at
            .as_ref()
            .map(|view| view.as_path().to_path_buf());
        let (platform, spec) = if cfg!(windows) {
            ("windows", &self.windows)
        } else {
            ("linux", &self.linux)
        };
        let spec = spec
            .as_ref()
            .ok_or(ResolveError::NoSourceForPlatform { platform })?;
        Ok(Resolved {
            source: spec.source.clone(),
            hash: spec.hash.clone(),
            manifest: self,
            target,
            view,
        })
    }
}

/// Failure of manifest parsing; the typed laws surface inside the TOML error message.
#[derive(Debug)]
pub enum ManifestError {
    Toml { message: String },
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self::Toml { message } = self;
        write!(f, "invalid manifest: {message}")
    }
}

impl std::error::Error for ManifestError {}

// --- Durable intent journal (s2-12, caller-owned) -------------------------------------------
// Append-only `journal.jsonl` under `.pulith-state/`: one `Record` per line, fsync'd before the
// caller acknowledges. The record is the caller's own intent (name@version + phase + the
// superseding generation) — the manifest carries the artifact content, so the journal never
// duplicates it. A crash before the fsync loses the record; recovery re-observes the paths.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};

use serde::Serialize;

/// One committed intent for a `name@version` address (the expectation repair reconciles
/// against — never derived from observation).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub name: String,
    pub version: String,
    pub phase: Phase,
    pub generation: u64,
}

/// The caller's committed lifecycle phase for an address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Installed,
    Deactivated,
}

/// Append-only journal with a behavior invariant: every append is one JSON line, fsync'd before
/// the caller acknowledges (an invariant-holding object, not a path wrapper).
pub struct Journal {
    path: PathBuf,
}

impl Journal {
    /// Open (creating `.pulith-state/` as needed) the journal for the layout root.
    pub fn open(root: &Path) -> Result<Self, StateError> {
        let dir = root.join(".pulith-state");
        fs::create_dir_all(&dir)
            .map_err(|source| StateError::io("create state directory", &dir, source))?;
        Ok(Self {
            path: dir.join("journal.jsonl"),
        })
    }

    /// Append one record: write the JSON line and fsync before returning. Test crash hooks
    /// abort right before/after the fsync (`PULITH_VT_CRASH_AFTER=journal-append|journal-fsync`).
    pub fn append(&mut self, record: &Record) -> Result<(), StateError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|source| StateError::io("open journal for append", &self.path, source))?;
        serde_json::to_writer(&mut file, record)
            .map_err(|error| StateError::encode(&self.path, error))?;
        file.write_all(b"\n")
            .map_err(|source| StateError::io("write journal line", &self.path, source))?;
        // Crash hooks (test-only by convention): abort before/after the fsync.
        if std::env::var("PULITH_VT_CRASH_AFTER").as_deref() == Ok("journal-append") {
            std::process::abort();
        }
        file.sync_all()
            .map_err(|source| StateError::io("fsync journal", &self.path, source))?;
        if std::env::var("PULITH_VT_CRASH_AFTER").as_deref() == Ok("journal-fsync") {
            std::process::abort();
        }
        Ok(())
    }

    /// Read every record in append order; a missing journal is empty, a corrupt line is an
    /// error (recovery must not silently drop committed intent).
    pub fn read(&self) -> Result<Vec<Record>, StateError> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(StateError::io("open journal for read", &self.path, source)),
        };
        let mut records = Vec::new();
        for (index, line) in BufReader::new(file).lines().enumerate() {
            let line =
                line.map_err(|source| StateError::io("read journal line", &self.path, source))?;
            if line.trim().is_empty() {
                continue;
            }
            records.push(
                serde_json::from_str(&line).map_err(|error| StateError::Decode {
                    path: self.path.clone(),
                    line: index + 1,
                    message: error.to_string(),
                })?,
            );
        }
        Ok(records)
    }
}

/// Failure of the durable-state machinery (action + path named).
#[derive(Debug)]
pub enum StateError {
    Io {
        action: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    Decode {
        path: PathBuf,
        line: usize,
        message: String,
    },
    Encode {
        path: PathBuf,
        message: String,
    },
}

impl StateError {
    fn io(action: &'static str, path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            action,
            path: path.to_path_buf(),
            source,
        }
    }

    fn encode(path: &Path, error: serde_json::Error) -> Self {
        Self::Encode {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    }
}

impl std::fmt::Display for StateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io {
                action,
                path,
                source,
            } => write!(f, "{action} `{}`: {source}", path.display()),
            Self::Decode {
                path,
                line,
                message,
            } => {
                write!(
                    f,
                    "corrupt journal line {line} in `{}`: {message}",
                    path.display()
                )
            }
            Self::Encode { path, message } => {
                write!(f, "encode journal record `{}`: {message}", path.display())
            }
        }
    }
}

impl std::error::Error for StateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    const LINK_AT: &str = "C:/opt/demo-tool";
    #[cfg(not(windows))]
    const LINK_AT: &str = "/opt/demo-tool";

    const WIN_HEX: &str = "75c251814644ed2e7b0048a25d396699851338bfb21cca3f0f1270f8952bb226";
    const LINUX_HEX: &str = "7f86e631a80af88c0c1e724d29c4828448ec7d4b99e6b19570f81eb9733e85ee";

    fn valid() -> String {
        format!(
            r#"
name = "demo-tool"
version = "1.2.0"
expose = "bin"
link_at = "{LINK_AT}"

[windows.source]
kind = "url"
url = "https://example.com/demo-tool/1.2.0/demo-tool-win.zip"

[windows.hash]
kind = "blake3"
hex = "{WIN_HEX}"

[linux.source]
kind = "url"
url = "https://example.com/demo-tool/1.2.0/demo-tool-linux.tar.gz"

[linux.hash]
kind = "blake3"
hex = "{LINUX_HEX}"
"#
        )
    }

    #[test]
    fn parses_a_full_valid_manifest() {
        let manifest = Manifest::parse(&valid()).unwrap();
        assert_eq!(manifest.name.as_str(), "demo-tool");
        assert_eq!(manifest.version.as_str(), "1.2.0");
        let windows = manifest.windows.as_ref().unwrap();
        match &windows.source {
            Source::Url { url } => {
                assert_eq!(
                    url.as_str(),
                    "https://example.com/demo-tool/1.2.0/demo-tool-win.zip"
                )
            }
            Source::Local { .. } => panic!("expected a url source"),
        }
        use pulith::hash::DigestAlgorithmKind;
        assert_eq!(windows.hash.algorithm(), DigestAlgorithmKind::Blake3);
        assert_eq!(windows.hash.as_str().len(), 64);
        assert!(manifest.linux.is_some());
        assert_eq!(manifest.expose.as_deref(), Some(Path::new("bin")));
        assert_eq!(
            manifest
                .link_at
                .as_ref()
                .map(|view| view.as_path().to_path_buf()),
            Some(PathBuf::from(LINK_AT))
        );
    }

    #[test]
    fn parses_a_local_source_with_sha2_and_no_view() {
        let manifest = Manifest::parse(
            r#"
name = "local-tool"
version = "0.4.1"

[windows.source]
kind = "local"
path = "vendor/local-tool-win.tar"

[windows.hash]
kind = "sha2"
hex = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"

[linux.source]
kind = "local"
path = "vendor/local-tool-linux.tar"

[linux.hash]
kind = "sha2"
hex = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
"#,
        )
        .unwrap();
        assert!(matches!(
            manifest.windows.as_ref().unwrap().source,
            Source::Local { .. }
        ));
        use pulith::hash::DigestAlgorithmKind;
        assert_eq!(
            manifest.windows.as_ref().unwrap().hash.algorithm(),
            DigestAlgorithmKind::Sha256
        );
        assert!(manifest.expose.is_none());
        assert!(manifest.link_at.is_none());
    }

    #[test]
    fn digest_hash_deserializes_directly_as_the_core_value() {
        use pulith::hash::DigestAlgorithmKind;
        let digest: DigestValue = toml::from_str(
            "kind = \"sha2\"
hex = \"1111111111111111111111111111111111111111111111111111111111111111\"
",
        )
        .unwrap();
        assert_eq!(digest.algorithm(), DigestAlgorithmKind::Sha256);
        assert_eq!(digest.as_str(), "11".repeat(32));
    }

    #[test]
    fn rejects_an_unknown_field() {
        let manifest = "name = \"demo-tool\"\nversion = \"1.0.0\"\nunknown = true\n";
        let error = Manifest::parse(manifest).unwrap_err();
        assert!(matches!(error, ManifestError::Toml { .. }));
    }

    #[test]
    fn rejects_a_partial_platform_pair() {
        let manifest = r#"
name = "demo-tool"
version = "1.0.0"

[windows.source]
kind = "url"
url = "https://example.com/tool-win.zip"

[linux.source]
kind = "url"
url = "https://example.com/tool-linux.zip"

[linux.hash]
kind = "blake3"
hex = "0000000000000000000000000000000000000000000000000000000000000000"
"#;
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

[windows.source]
kind = "url"
url = "https://example.com/tool-win.zip"

[windows.hash]
kind = "blake3"
hex = "0000000000000000000000000000000000000000000000000000000000000000"

[linux.source]
kind = "url"
url = "https://example.com/tool-linux.zip"

[linux.hash]
kind = "blake3"
hex = "0000000000000000000000000000000000000000000000000000000000000000"
"#
            );
            let error = Manifest::parse(&manifest).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("single non-empty path component"),
                "name {name:?} should be rejected: {error}"
            );
        }
    }

    #[test]
    fn rejects_an_invalid_version() {
        let manifest = r#"
name = "demo-tool"
version = "1.0/.."

[windows.source]
kind = "url"
url = "https://example.com/tool-win.zip"

[windows.hash]
kind = "blake3"
hex = "0000000000000000000000000000000000000000000000000000000000000000"

[linux.source]
kind = "url"
url = "https://example.com/tool-linux.zip"

[linux.hash]
kind = "blake3"
hex = "0000000000000000000000000000000000000000000000000000000000000000"
"#;
        let error = Manifest::parse(manifest).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("single non-empty path component")
        );
    }

    #[test]
    fn rejects_a_non_http_source_url() {
        let manifest = r#"
name = "demo-tool"
version = "1.0.0"

[windows.source]
kind = "url"
url = "ftp://example.com/tool-win.zip"

[windows.hash]
kind = "blake3"
hex = "0000000000000000000000000000000000000000000000000000000000000000"

[linux.source]
kind = "url"
url = "https://example.com/tool-linux.zip"

[linux.hash]
kind = "blake3"
hex = "0000000000000000000000000000000000000000000000000000000000000000"
"#;
        let error = Manifest::parse(manifest).unwrap_err();
        assert!(
            error.to_string().contains("unsupported remote URL scheme"),
            "ftp should be rejected: {error}"
        );
    }

    #[test]
    fn rejects_a_malformed_hash() {
        for hex in ["not-hex", "abcdef", "zz", &"0".repeat(63)] {
            let manifest = format!(
                r#"
name = "demo-tool"
version = "1.0.0"

[windows.source]
kind = "url"
url = "https://example.com/tool-win.zip"

[windows.hash]
kind = "blake3"
hex = "{hex}"

[linux.source]
kind = "url"
url = "https://example.com/tool-linux.zip"

[linux.hash]
kind = "blake3"
hex = "0000000000000000000000000000000000000000000000000000000000000000"
"#
            );
            let error = Manifest::parse(&manifest).unwrap_err();
            assert!(
                error.to_string().contains("64 hex digits"),
                "hash {hex:?} should be rejected: {error}"
            );
        }
    }

    #[test]
    fn rejects_an_unknown_digest_kind() {
        let manifest = r#"
name = "demo-tool"
version = "1.0.0"

[windows.source]
kind = "url"
url = "https://example.com/tool-win.zip"

[windows.hash]
kind = "sha3"
hex = "0000000000000000000000000000000000000000000000000000000000000000"

[linux.source]
kind = "url"
url = "https://example.com/tool-linux.zip"

[linux.hash]
kind = "blake3"
hex = "0000000000000000000000000000000000000000000000000000000000000000"
"#;
        let error = Manifest::parse(manifest).unwrap_err();
        assert!(error.to_string().contains("unknown digest kind"));
    }

    #[test]
    fn accepts_a_nested_expose() {
        let manifest = format!(
            r#"
name = "demo-tool"
version = "1.0.0"
expose = "opt/demo-tool/bin"

[windows.source]
kind = "url"
url = "https://example.com/tool-win.zip"

[windows.hash]
kind = "blake3"
hex = "0000000000000000000000000000000000000000000000000000000000000000"

[linux.source]
kind = "url"
url = "https://example.com/tool-linux.zip"

[linux.hash]
kind = "blake3"
hex = "0000000000000000000000000000000000000000000000000000000000000000"
"#
        );
        let manifest = Manifest::parse(&manifest).unwrap();
        assert_eq!(
            manifest.expose.as_deref(),
            Some(Path::new("opt/demo-tool/bin"))
        );
    }

    #[test]
    fn rejects_a_relative_link_at() {
        let manifest = r#"
name = "demo-tool"
version = "1.0.0"
link_at = "opt/demo-tool"

[windows.source]
kind = "url"
url = "https://example.com/tool-win.zip"

[windows.hash]
kind = "blake3"
hex = "0000000000000000000000000000000000000000000000000000000000000000"

[linux.source]
kind = "url"
url = "https://example.com/tool-linux.zip"

[linux.hash]
kind = "blake3"
hex = "0000000000000000000000000000000000000000000000000000000000000000"
"#;
        let error = Manifest::parse(manifest).unwrap_err();
        assert!(error.to_string().contains("must be an absolute path"));
    }

    #[cfg(windows)]
    fn absolute_view() -> PathBuf {
        PathBuf::from("C:/opt/demo-tool")
    }
    #[cfg(not(windows))]
    fn absolute_view() -> PathBuf {
        PathBuf::from("/opt/demo-tool")
    }

    fn manifest() -> Manifest {
        Manifest {
            name: "demo-tool".parse().unwrap(),
            version: "1.2.0".parse().unwrap(),
            expose: Some(PathBuf::from("bin")),
            link_at: Some(absolute_view().display().to_string().parse().unwrap()),
            windows: Some(PlatformSpec {
                source: Source::Url {
                    url: "https://example.com/demo-tool-win.zip".parse().unwrap(),
                },
                hash: DigestValue::new(pulith::hash::DigestAlgorithmKind::Blake3, "0".repeat(64))
                    .unwrap(),
            }),
            linux: Some(PlatformSpec {
                source: Source::Url {
                    url: "https://example.com/demo-tool-linux.tar.gz"
                        .parse()
                        .unwrap(),
                },
                hash: DigestValue::new(pulith::hash::DigestAlgorithmKind::Blake3, "1".repeat(64))
                    .unwrap(),
            }),
        }
    }

    #[test]
    fn resolves_the_versioned_target_path() {
        let resolved = manifest().resolve(Path::new("/opt/tools")).unwrap();
        assert_eq!(
            resolved.target,
            PathBuf::from("/opt/tools/artifacts/demo-tool/1.2.0")
        );
    }

    #[test]
    fn resolves_the_view_path_only_when_link_at_is_declared() {
        let with_view = manifest().resolve(Path::new("/opt/tools")).unwrap();
        assert_eq!(with_view.view, Some(absolute_view()));

        let mut no_view = manifest();
        no_view.link_at = None;
        let without_view = no_view.resolve(Path::new("/opt/tools")).unwrap();
        assert_eq!(without_view.view, None);
    }

    #[test]
    fn selects_the_running_platforms_artifact_pair() {
        let resolved = manifest().resolve(Path::new("/opt/tools")).unwrap();
        if cfg!(windows) {
            assert_eq!(
                resolved.hash,
                DigestValue::new(pulith::hash::DigestAlgorithmKind::Blake3, "0".repeat(64),)
                    .unwrap()
            );
        } else {
            assert_eq!(
                resolved.hash,
                DigestValue::new(pulith::hash::DigestAlgorithmKind::Blake3, "1".repeat(64),)
                    .unwrap()
            );
        }
    }

    #[test]
    fn rejects_a_platform_without_a_pair() {
        let mut bad = manifest();
        if cfg!(windows) {
            bad.windows = None;
        } else {
            bad.linux = None;
        }
        let error = bad.resolve(Path::new("/opt/tools")).unwrap_err();
        assert!(matches!(error, ResolveError::NoSourceForPlatform { .. }));
    }

    // --- journal tests (the state machinery lives with the manifest types) ---

    fn record(name: &str, version: &str, generation: u64) -> Record {
        Record {
            name: name.to_string(),
            version: version.to_string(),
            phase: Phase::Installed,
            generation,
        }
    }

    use std::collections::HashMap;

    /// Keep the highest generation per `name@version` (supersede), first-appearance order.
    /// The production paths (`commit`, `repair`) query the address directly; this is the
    /// fold semantics under test.
    fn fold(records: &[Record]) -> Vec<Record> {
        let mut best: HashMap<(&str, &str), usize> = HashMap::new();
        let mut folded: Vec<Record> = Vec::new();
        for record in records {
            let key = (record.name.as_str(), record.version.as_str());
            match best.get(&key) {
                Some(&index) if folded[index].generation >= record.generation => {}
                Some(&index) => folded[index] = record.clone(),
                None => {
                    best.insert(key, folded.len());
                    folded.push(record.clone());
                }
            }
        }
        folded
    }

    fn temp_root(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pulith-vtool-manifest-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn append_persists_across_reopen() {
        let root = temp_root("persist");
        let mut journal = Journal::open(&root).unwrap();
        journal.append(&record("demo", "1.0.0", 1)).unwrap();
        drop(journal);

        let reopened = Journal::open(&root).unwrap();
        let records = reopened.read().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].name, "demo");
        assert_eq!(records[0].generation, 1);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn read_of_missing_journal_is_empty() {
        let root = temp_root("missing");
        let journal = Journal::open(&root).unwrap();
        assert!(journal.read().unwrap().is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fold_keeps_highest_generation_per_address() {
        let records = vec![
            record("demo", "1.0.0", 1),
            record("demo", "1.0.0", 2),
            record("other", "2.0.0", 1),
            record("demo", "1.0.0", 3),
        ];
        let folded = fold(&records);
        assert_eq!(folded.len(), 2);
        assert_eq!(folded[0].name, "demo");
        assert_eq!(folded[0].generation, 3);
        assert_eq!(folded[1].name, "other");
        assert_eq!(folded[1].generation, 1);
    }

    #[test]
    fn older_generation_does_not_replace_newer() {
        let records = vec![record("demo", "1.0.0", 5), record("demo", "1.0.0", 3)];
        let folded = fold(&records);
        assert_eq!(folded[0].generation, 5);
    }

    #[test]
    fn a_corrupt_line_is_an_error_not_silent_drop() {
        let root = temp_root("corrupt");
        let mut journal = Journal::open(&root).unwrap();
        journal.append(&record("demo", "1.0.0", 1)).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(root.join(".pulith-state/journal.jsonl"))
            .unwrap()
            .write_all(b"not-json\n")
            .unwrap();
        let error = journal.read().unwrap_err();
        assert!(matches!(error, StateError::Decode { line: 2, .. }));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
