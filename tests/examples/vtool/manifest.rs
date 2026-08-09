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
                url: Box::new("https://example.com/demo-tool-win.zip".parse().unwrap()),
            },
            hash: DigestValue::new(pulith::hash::DigestAlgorithmKind::Blake3, "0".repeat(64))
                .unwrap(),
        }),
        linux: Some(PlatformSpec {
            source: Source::Url {
                url: Box::new(
                    "https://example.com/demo-tool-linux.tar.gz"
                        .parse()
                        .unwrap(),
                ),
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
            DigestValue::new(pulith::hash::DigestAlgorithmKind::Blake3, "0".repeat(64),).unwrap()
        );
    } else {
        assert_eq!(
            resolved.hash,
            DigestValue::new(pulith::hash::DigestAlgorithmKind::Blake3, "1".repeat(64),).unwrap()
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
