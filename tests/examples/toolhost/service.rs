use super::*;

const VALID: &str = r#"
schema = 1
id = "indexer"
payload = "indexer"
args = ["serve", "--foreground"]

[environment]
INDEX_DIR = "data"
RUST_LOG = "info"
"#;

#[test]
fn declaration_normalizes_to_one_exact_form() {
    let declaration = ServiceDecl::parse(VALID).unwrap().normalize().unwrap();
    assert_eq!(declaration.id.as_str(), "indexer");
    assert_eq!(declaration.payload(), "indexer");
    assert_eq!(
        declaration.bytes(),
        b"schema = 1\nid = \"indexer\"\npayload = \"indexer\"\nargs = [\"serve\", \"--foreground\"]\n\n[environment]\nINDEX_DIR = \"data\"\nRUST_LOG = \"info\"\n"
    );
}

#[test]
fn declaration_rejects_schema_shape_and_identity_ambiguity() {
    for invalid in [
        VALID.replace("schema = 1", "schema = 2"),
        format!("{VALID}\nunknown = true\n"),
        VALID
            .replace("indexer\"", "Bad_Id\"")
            .replacen("Bad_Id", "bad/id", 1),
        VALID.replace("payload = \"indexer\"", "payload = \"../indexer\""),
    ] {
        assert!(
            ServiceDecl::parse(&invalid)
                .and_then(ServiceDecl::normalize)
                .is_err()
        );
    }
    for id in ["", "A", "a_", "-a", &"a".repeat(64)] {
        let invalid = VALID.replacen("indexer", id, 1);
        assert!(
            ServiceDecl::parse(&invalid)
                .and_then(ServiceDecl::normalize)
                .is_err()
        );
    }
}

#[test]
fn declaration_rejects_ambiguous_or_reserved_environment() {
    for environment in [
        "PATH = \"x\"",
        "toolhost_home = \"x\"",
        "LD_LIBRARY_PATH = \"x\"",
        "A = \"x\\u0000y\"",
        "\"A=B\" = \"x\"",
        "Name = \"one\"\nNAME = \"two\"",
    ] {
        let head = VALID.split("[environment]").next().unwrap();
        let invalid = format!("{head}[environment]\n{environment}\n");
        assert!(
            ServiceDecl::parse(&invalid)
                .and_then(ServiceDecl::normalize)
                .is_err()
        );
    }
}

#[test]
fn observations_have_one_stable_cross_platform_rendering() {
    let observation = Observation {
        definition: Definition::Exact,
        registration: Registration::Exact,
        boot: Boot::Enabled,
        runtime: Runtime::Running,
    };
    assert_eq!(
        observation.to_string(),
        "definition=exact registration=exact boot=enabled runtime=running"
    );
    assert_eq!(
        Change {
            changed: true,
            observation
        }
        .to_string(),
        "changed=true definition=exact registration=exact boot=enabled runtime=running"
    );
}

#[test]
fn service_errors_preserve_conflict_and_accepted_progress() {
    let observation = Observation {
        definition: Definition::Exact,
        registration: Registration::Removing,
        boot: Boot::Conflict,
        runtime: Runtime::Stopping,
    };
    let conflict = ServiceError::conflict("registration changed", observation);
    assert!(matches!(
        conflict.0,
        Failure::Conflict(observed) if observed == observation
    ));

    let partial = ServiceError::partial(
        AcceptedEffect::DeletionRequested,
        observation,
        "await service deletion",
        std::io::Error::new(std::io::ErrorKind::TimedOut, "deadline exceeded"),
    );
    assert!(matches!(
        partial.0,
        Failure::Partial(AcceptedEffect::DeletionRequested, observed)
            if observed == observation
    ));

    assert!(matches!(
        ServiceError::os("open service manager", std::io::Error::from_raw_os_error(5),).0,
        Failure::Authority
    ));
    assert!(matches!(
        ServiceError::operation("read service declaration", "missing").0,
        Failure::Operation
    ));
    assert_eq!(
        AcceptedEffect::BindingChangeRequested.to_string(),
        "binding-change-requested"
    );
}

#[test]
fn removal_planning_preserves_manager_first_cleanup() {
    let observed = |definition, registration, boot, runtime| Observation {
        definition,
        registration,
        boot,
        runtime,
    };
    assert_eq!(
        removal_plan(observed(
            Definition::Missing,
            Registration::Missing,
            Boot::Disabled,
            Runtime::Stopped
        ))
        .unwrap(),
        RemovalPlan::Unchanged
    );
    assert_eq!(
        removal_plan(observed(
            Definition::Exact,
            Registration::Missing,
            Boot::Disabled,
            Runtime::Stopped
        ))
        .unwrap(),
        RemovalPlan::CleanupOnly
    );
    assert_eq!(
        removal_plan(observed(
            Definition::Exact,
            Registration::Removing,
            Boot::Conflict,
            Runtime::Stopping
        ))
        .unwrap(),
        RemovalPlan::AwaitDeletion
    );
    assert_eq!(
        removal_plan(observed(
            Definition::Exact,
            Registration::Exact,
            Boot::Disabled,
            Runtime::Stopped
        ))
        .unwrap(),
        RemovalPlan::Delete
    );
    assert!(
        removal_plan(observed(
            Definition::Exact,
            Registration::Conflict,
            Boot::Disabled,
            Runtime::Stopped
        ))
        .is_err()
    );
    assert!(
        removal_plan(observed(
            Definition::Exact,
            Registration::Exact,
            Boot::Enabled,
            Runtime::Stopped
        ))
        .is_err()
    );
}

#[test]
fn manager_definition_pins_one_immutable_release() {
    let temporary = tempfile::tempdir().unwrap();
    let root = ServiceRoot(temporary.path().canonicalize().unwrap());
    let release = root.0.join("installs/tool/1");
    std::fs::create_dir_all(release.join("service")).unwrap();
    std::fs::create_dir_all(release.join("bin")).unwrap();
    let declaration = ServiceDecl::parse(VALID).unwrap().normalize().unwrap();
    let executable = format!("indexer{}", std::env::consts::EXE_SUFFIX);
    std::fs::write(release.join("service").join(&executable), b"host").unwrap();
    std::fs::write(release.join("bin").join(executable), b"payload").unwrap();
    let binding = Binding::admit(&root, release.canonicalize().unwrap(), &declaration).unwrap();
    let rendered = platform::render_definition(&root, &declaration, &binding);
    assert!(rendered.contains(&binding.release.display().to_string()));
    assert!(!rendered.contains("current"));
    #[cfg(unix)]
    for directive in [
        "DynamicUser=yes",
        "NoNewPrivileges=yes",
        "CapabilityBoundingSet=",
        "AmbientCapabilities=",
        "ProtectSystem=strict",
        "ProtectHome=yes",
        "PrivateTmp=yes",
    ] {
        assert!(rendered.contains(directive), "{directive}");
    }
}

#[cfg(windows)]
#[test]
fn scm_install_policy_is_least_privilege() {
    assert_eq!(platform::ACCOUNT, "NT AUTHORITY\\LocalService");
    assert_eq!(platform::scm::SID_TYPE, 3);
    assert!(platform::scm::REQUIRED_PRIVILEGES.is_empty());
    assert_eq!(
        platform::scm::CREATED_ACCESS,
        windows_sys::Win32::System::Services::SERVICE_CHANGE_CONFIG
            | windows_sys::Win32::System::Services::SERVICE_QUERY_CONFIG
            | windows_sys::Win32::System::Services::SERVICE_QUERY_STATUS
    );
    assert_ne!(
        platform::scm::CREATED_ACCESS,
        windows_sys::Win32::System::Services::SERVICE_ALL_ACCESS
    );
}

#[cfg(windows)]
#[test]
fn scm_operation_rights_are_exact() {
    use windows_sys::Win32::Storage::FileSystem::DELETE;
    use windows_sys::Win32::System::Services::*;

    assert_eq!(
        platform::scm::OBSERVE_ACCESS,
        SERVICE_QUERY_CONFIG | SERVICE_QUERY_STATUS
    );
    assert_eq!(platform::scm::BINDING_ACCESS, SERVICE_QUERY_CONFIG);
    assert_eq!(
        platform::scm::REPAIR_ACCESS,
        SERVICE_CHANGE_CONFIG | SERVICE_QUERY_CONFIG | SERVICE_QUERY_STATUS
    );
    assert_eq!(platform::scm::CONFIGURE_ACCESS, SERVICE_CHANGE_CONFIG);
    assert_eq!(platform::scm::REBIND_ACCESS, SERVICE_CHANGE_CONFIG);
    assert_eq!(
        platform::scm::START_ACCESS,
        SERVICE_START | SERVICE_QUERY_STATUS
    );
    assert_eq!(
        platform::scm::STOP_ACCESS,
        SERVICE_STOP | SERVICE_QUERY_STATUS
    );
    assert_eq!(platform::scm::REMOVE_ACCESS, DELETE | SERVICE_QUERY_CONFIG);
}

#[cfg(windows)]
#[test]
fn scm_access_plan_grants_only_exact_read_and_execute_inputs() {
    let temporary = tempfile::tempdir().unwrap();
    let root = ServiceRoot(temporary.path().canonicalize().unwrap());
    let release = root.0.join("installs/tool/1");
    std::fs::create_dir_all(release.join("service")).unwrap();
    std::fs::create_dir_all(release.join("bin")).unwrap();
    let declaration = ServiceDecl::parse(VALID).unwrap().normalize().unwrap();
    let executable = format!("indexer{}", std::env::consts::EXE_SUFFIX);
    std::fs::write(release.join("service").join(&executable), b"host").unwrap();
    std::fs::write(release.join("bin").join(executable), b"payload").unwrap();
    std::fs::create_dir_all(root.directory(&declaration)).unwrap();
    std::fs::write(root.declaration(&declaration), declaration.bytes()).unwrap();
    let binding = Binding::admit(&root, release, &declaration).unwrap();
    let plan = platform::access_plan(&root, &declaration, &binding);
    assert_eq!(plan.len(), 2);
    assert!(plan.iter().any(|grant| grant.path == binding.release));
    assert!(
        plan.iter()
            .any(|grant| grant.path == root.declaration(&declaration))
    );
    let writes = windows_sys::Win32::Storage::FileSystem::FILE_WRITE_DATA
        | windows_sys::Win32::Storage::FileSystem::FILE_APPEND_DATA
        | windows_sys::Win32::Storage::FileSystem::FILE_WRITE_EA
        | windows_sys::Win32::Storage::FileSystem::FILE_WRITE_ATTRIBUTES
        | windows_sys::Win32::Storage::FileSystem::DELETE
        | windows_sys::Win32::Storage::FileSystem::WRITE_DAC
        | windows_sys::Win32::Storage::FileSystem::WRITE_OWNER;
    assert!(plan.iter().all(|grant| grant.mask & writes == 0));
}

#[cfg(unix)]
#[test]
fn systemd_observation_is_independent_of_property_order() {
    let observation = super::platform::parse_observation(
        "ActiveState=active\nLoadState=loaded\nUnitFileState=enabled\n",
    );
    assert_eq!(observation.registration, Registration::Exact);
    assert_eq!(observation.boot, Boot::Enabled);
    assert_eq!(observation.runtime, Runtime::Running);
}

#[test]
fn definition_publication_adopts_exact_and_rejects_conflict_or_broken_shape() {
    let temporary = tempfile::tempdir().unwrap();
    let root = ServiceRoot(temporary.path().to_path_buf());
    let declaration = ServiceDecl::parse(VALID).unwrap().normalize().unwrap();
    assert_eq!(root.observe(&declaration).unwrap(), Definition::Missing);
    assert!(root.publish(&declaration).unwrap());
    assert_eq!(root.observe(&declaration).unwrap(), Definition::Exact);
    assert!(!root.publish(&declaration).unwrap());

    std::fs::write(root.declaration(&declaration), b"different").unwrap();
    assert_eq!(root.observe(&declaration).unwrap(), Definition::Conflict);
    std::fs::remove_file(root.declaration(&declaration)).unwrap();
    assert_eq!(root.observe(&declaration).unwrap(), Definition::Broken);
}

#[test]
fn service_root_admission_rejects_relative_and_missing_paths() {
    assert!(ServiceRoot::admit("relative".into()).is_err());
    let temporary = tempfile::tempdir().unwrap();
    assert!(ServiceRoot::admit(temporary.path().join("missing")).is_err());
}

#[cfg(unix)]
#[test]
fn exposure_rejects_a_writable_pinned_input() {
    use std::os::unix::fs::PermissionsExt;

    let temporary = tempfile::tempdir().unwrap();
    let root = ServiceRoot(temporary.path().canonicalize().unwrap());
    let release = root.0.join("installs/tool/1");
    std::fs::create_dir_all(release.join("service")).unwrap();
    std::fs::create_dir_all(release.join("bin")).unwrap();
    let declaration = ServiceDecl::parse(VALID).unwrap().normalize().unwrap();
    let executable = format!("indexer{}", std::env::consts::EXE_SUFFIX);
    std::fs::write(release.join("service").join(&executable), b"host").unwrap();
    let payload = release.join("bin").join(executable);
    std::fs::write(&payload, b"payload").unwrap();
    std::fs::set_permissions(&payload, std::fs::Permissions::from_mode(0o777)).unwrap();
    let binding = Binding::admit(&root, release, &declaration).unwrap();
    assert!(root.admit_exposure(&binding, &declaration).is_err());
}
