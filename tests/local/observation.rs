#![cfg(feature = "local")]

use pulith::Inspect;
use pulith::local::{LocalInspectEvidence, LocalObservation, LocalTarget};

#[test]
fn cloned_local_target_observes_the_same_facts_without_mutating_the_target() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("artifact.txt");
    std::fs::write(&path, b"pulith").unwrap();

    let original = LocalTarget::new(path.as_path()).unwrap();
    let clone = original.clone();

    let (original_observation, original_evidence) = original.inspect(()).unwrap();
    let (clone_observation, clone_evidence) = clone.inspect(()).unwrap();

    assert_eq!(original_observation, LocalObservation::File { bytes: 6 });
    assert_eq!(original_observation, clone_observation);
    assert_eq!(original_evidence, clone_evidence);
    assert_eq!(original_evidence.path, path);
    assert_eq!(std::fs::read(&path).unwrap(), b"pulith".to_vec());
}

#[test]
fn observe_convenience_matches_inspect_with_the_same_evidence() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("tree");
    std::fs::create_dir(&directory).unwrap();

    let (via_trait, trait_evidence) = LocalTarget::new(directory.as_path())
        .unwrap()
        .inspect(())
        .unwrap();
    let (via_convenience, convenience_evidence) = LocalTarget::new(directory.as_path())
        .unwrap()
        .observe()
        .unwrap();

    assert_eq!(via_trait, LocalObservation::Directory);
    assert_eq!(via_trait, via_convenience);
    assert_eq!(
        trait_evidence,
        LocalInspectEvidence {
            path: directory.clone()
        }
    );
    assert_eq!(trait_evidence, convenience_evidence);
    assert!(directory.is_dir());
}
