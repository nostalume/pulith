#![cfg(all(feature = "local", feature = "blake3"))]

use std::path::PathBuf;

use pulith::hash::{DigestAlgorithmKind, DigestValue, HashVerify};
use pulith::local::{
    LocalApply, LocalExpectation, LocalMaterial, LocalObservation, LocalReconcile,
    LocalReconciliation,
};
use pulith::{
    Acquire, Acquired, EvidenceChain, Inspect, Inspected, Materialize, MaterializeMode, Reconcile,
    Verified, Verify,
};

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExternalSource(PathBuf);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExternalAcquire;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExternalEvidence;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExternalInspect;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExternalInspectEvidence;

impl Inspect<PathBuf> for ExternalInspect {
    type Error = std::convert::Infallible;
    type Output = Inspected<PathBuf, LocalObservation, ExternalInspectEvidence>;

    fn inspect(&self, input: PathBuf) -> Result<Self::Output, Self::Error> {
        Ok(Inspected {
            input,
            observation: LocalObservation::File { bytes: 6 },
            evidence: ExternalInspectEvidence,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExternalVerify;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExternalVerifyEvidence;

impl<I, E> Verify<Acquired<I, LocalMaterial, E>, ()> for ExternalVerify {
    type Error = std::convert::Infallible;
    type Output = Verified<I, LocalMaterial, EvidenceChain<E, ExternalVerifyEvidence>>;

    fn verify(
        &self,
        input: Acquired<I, LocalMaterial, E>,
        (): (),
    ) -> Result<Self::Output, Self::Error> {
        let Acquired {
            input,
            material,
            evidence,
        } = input;
        Ok(Verified {
            input,
            material,
            evidence: EvidenceChain {
                previous: evidence,
                current: ExternalVerifyEvidence,
            },
        })
    }
}

impl Acquire<Materialize<&'static str, ExternalSource, PathBuf>> for ExternalAcquire {
    type Error = std::io::Error;
    type Output = Acquired<
        Materialize<&'static str, ExternalSource, PathBuf>,
        LocalMaterial,
        ExternalEvidence,
    >;

    fn acquire(
        &self,
        input: Materialize<&'static str, ExternalSource, PathBuf>,
    ) -> Result<Self::Output, Self::Error> {
        let staged = tempfile::NamedTempFile::new()?;
        std::fs::copy(&input.source.0, staged.path())?;
        Ok(Acquired {
            input,
            material: LocalMaterial::StagedFile {
                path: staged.into_temp_path(),
            },
            evidence: ExternalEvidence,
        })
    }
}

#[test]
fn external_acquire_composes_with_builtin_verify_and_apply() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.bin");
    let target = temp.path().join("target.bin");
    std::fs::write(&source, b"pulith").unwrap();

    let request = Materialize::new(
        "external",
        ExternalSource(source.clone()),
        target.clone(),
        MaterializeMode::CreateNew,
    );
    let acquired = ExternalAcquire.acquire(request).unwrap();
    let staged_path = match &acquired.material {
        LocalMaterial::StagedFile { path } => path.to_path_buf(),
        _ => panic!("external acquire must return staged custody"),
    };
    let expected = DigestValue::new(
        DigestAlgorithmKind::Blake3,
        blake3::hash(b"pulith").to_hex().to_string(),
    )
    .unwrap();
    let verified = HashVerify::new(DigestAlgorithmKind::Blake3)
        .verify(acquired, expected)
        .unwrap();
    let applied = LocalApply.apply(verified).unwrap();

    assert_eq!(std::fs::read(target).unwrap(), b"pulith");
    assert_eq!(std::fs::read(source).unwrap(), b"pulith");
    assert!(!staged_path.exists());
    assert_eq!(applied.evidence.previous.previous, ExternalEvidence);
}

#[test]
fn external_inspect_composes_with_builtin_reconcile() {
    let inspected = ExternalInspect
        .inspect(PathBuf::from("external-target"))
        .unwrap();
    let reconciled = LocalReconcile
        .reconcile(inspected, LocalExpectation::FileSize(6))
        .unwrap();

    assert_eq!(reconciled.reconciliation, LocalReconciliation::Matches);
    assert_eq!(reconciled.evidence.previous, ExternalInspectEvidence);
}

#[test]
fn external_middle_transition_consumes_and_rebuilds_canonical_state() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.bin");
    let target = temp.path().join("target.bin");
    std::fs::write(&source, b"pulith").unwrap();
    let acquired = ExternalAcquire
        .acquire(Materialize::new(
            "external",
            ExternalSource(source),
            target.clone(),
            MaterializeMode::CreateNew,
        ))
        .unwrap();
    let staged_path = match &acquired.material {
        LocalMaterial::StagedFile { path } => path.to_path_buf(),
        _ => panic!("external acquire must return staged custody"),
    };

    let verified = ExternalVerify.verify(acquired, ()).unwrap();
    let applied = LocalApply.apply(verified).unwrap();

    assert_eq!(std::fs::read(target).unwrap(), b"pulith");
    assert!(!staged_path.exists());
    assert_eq!(applied.evidence.previous.previous, ExternalEvidence);
    assert_eq!(applied.evidence.previous.current, ExternalVerifyEvidence);
}
