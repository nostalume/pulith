use super::*;
use crate::service::ServiceDecl;

const DECLARATION: &str = r#"
schema = 1
id = "indexer"
payload = "indexer"
"#;

#[test]
fn receipt_validation_rejects_changed_or_escaping_grant_identity() {
    let temporary = tempfile::tempdir().unwrap();
    let root = ServiceRoot(temporary.path().canonicalize().unwrap());
    let declaration = ServiceDecl::parse(DECLARATION)
        .unwrap()
        .normalize()
        .unwrap();
    let release = root.0.join("installs/indexer/1");
    std::fs::create_dir_all(release.join("service")).unwrap();
    std::fs::create_dir_all(release.join("bin")).unwrap();
    std::fs::create_dir_all(root.directory(&declaration)).unwrap();
    let executable = format!("indexer{}", std::env::consts::EXE_SUFFIX);
    std::fs::write(release.join("service").join(&executable), b"host").unwrap();
    std::fs::write(release.join("bin").join(executable), b"payload").unwrap();
    std::fs::write(root.declaration(&declaration), declaration.bytes()).unwrap();
    let binding = Binding::admit(&root, release, &declaration).unwrap();
    let state = AccessState {
        root: &root,
        declaration: &declaration,
    };
    let plan = state.plan(&binding);
    let mut receipt = AccessReceipt {
        schema: 1,
        release: relative(&root, &binding.release).unwrap(),
        grants: [
            GrantReceipt {
                path: relative(&root, &plan[0].path).unwrap(),
                mask: plan[0].mask,
                inheritance: plan[0].inheritance,
                ownership: Ownership::Created,
            },
            GrantReceipt {
                path: relative(&root, &plan[1].path).unwrap(),
                mask: plan[1].mask,
                inheritance: plan[1].inheritance,
                ownership: Ownership::Preexisting,
            },
        ],
    };
    state.validate_receipt(&receipt).unwrap();
    receipt.grants[0].mask ^= 1;
    assert!(state.validate_receipt(&receipt).is_err());
    receipt.grants[0].mask ^= 1;
    receipt.release = root.0.join("outside").display().to_string();
    assert!(state.validate_receipt(&receipt).is_err());
}

#[test]
fn records_reject_unknown_fields() {
    let text = "schema = 1\nrelease = 'installs/indexer/1'\nunknown = true\ngrants = []\n";
    assert!(toml::from_str::<AccessReceipt>(text).is_err());
}

#[test]
fn rebind_intent_admits_source_and_target_retries_only() {
    let (temporary, root, declaration, bindings) = bindings();
    let state = AccessState {
        root: &root,
        declaration: &declaration,
    };
    let stable = receipt(&state, &bindings[0]);
    let intent = RebindIntent {
        schema: 1,
        from: receipt(&state, &bindings[0]),
        to: receipt(&state, &bindings[1]),
    };
    state
        .validate_intent(&intent, &stable, &bindings[0], &bindings[1])
        .unwrap();
    state
        .validate_intent(&intent, &stable, &bindings[1], &bindings[1])
        .unwrap();
    assert!(
        state
            .validate_intent(&intent, &stable, &bindings[2], &bindings[1])
            .is_err()
    );
    drop(temporary);
}

fn bindings() -> (
    tempfile::TempDir,
    ServiceRoot,
    crate::service::NormalizedDecl,
    [Binding; 3],
) {
    let temporary = tempfile::tempdir().unwrap();
    let root = ServiceRoot(temporary.path().canonicalize().unwrap());
    let declaration = ServiceDecl::parse(DECLARATION)
        .unwrap()
        .normalize()
        .unwrap();
    let releases = ["1", "2", "3"].map(|version| {
        let release = root.0.join("installs/indexer").join(version);
        std::fs::create_dir_all(release.join("service")).unwrap();
        std::fs::create_dir_all(release.join("bin")).unwrap();
        let executable = format!("indexer{}", std::env::consts::EXE_SUFFIX);
        std::fs::write(release.join("service").join(&executable), b"host").unwrap();
        std::fs::write(release.join("bin").join(executable), b"payload").unwrap();
        Binding::admit(&root, release, &declaration).unwrap()
    });
    (temporary, root, declaration, releases)
}

fn receipt(state: &AccessState<'_>, binding: &Binding) -> AccessReceipt {
    let plan = state.plan(binding);
    AccessReceipt {
        schema: 1,
        release: relative(state.root, &binding.release).unwrap(),
        grants: [0, 1].map(|index| GrantReceipt {
            path: relative(state.root, &plan[index].path).unwrap(),
            mask: plan[index].mask,
            inheritance: plan[index].inheritance,
            ownership: Ownership::Created,
        }),
    }
}
