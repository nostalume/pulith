use super::*;

#[test]
fn host_binds_declaration_to_its_own_immutable_release() {
    let temporary = tempfile::tempdir().unwrap();
    let release = temporary.path().join("installs/tool/1");
    let service = release.join("service");
    std::fs::create_dir_all(&service).unwrap();
    let executable = service.join(format!("tool{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&executable, b"host").unwrap();
    let declaration = temporary.path().join("service.toml");
    std::fs::write(
        &declaration,
        "schema=1\nid='service'\npayload='tool'\nargs=[]\n",
    )
    .unwrap();
    let host = Host::load_from(executable, declaration).unwrap();
    assert_eq!(host.release, release.canonicalize().unwrap());
    assert_eq!(host.home, temporary.path().canonicalize().unwrap());
}

#[test]
fn host_rejects_a_companion_named_for_another_payload() {
    let temporary = tempfile::tempdir().unwrap();
    let release = temporary.path().join("installs/tool/1/service");
    std::fs::create_dir_all(&release).unwrap();
    let executable = release.join(format!("other{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&executable, b"host").unwrap();
    let declaration = temporary.path().join("service.toml");
    std::fs::write(
        &declaration,
        "schema=1\nid='service'\npayload='tool'\nargs=[]\n",
    )
    .unwrap();
    assert!(Host::load_from(executable, declaration).is_err());
}
