use super::*;

#[test]
fn missing_unknown_and_non_unicode_verbs_are_usage_errors() {
    assert!(Command::parse([]).is_err());
    let root = std::env::temp_dir();
    assert!(
        Command::parse([
            OsString::from("unknown"),
            "--root".into(),
            root.into_os_string(),
            "manifest".into()
        ])
        .is_err()
    );
    #[cfg(unix)]
    let invalid = {
        use std::os::unix::ffi::OsStringExt;
        OsString::from_vec(vec![0xff])
    };
    #[cfg(windows)]
    let invalid = {
        use std::os::windows::ffi::OsStringExt;
        OsString::from_wide(&[0xd800])
    };
    assert!(Command::parse([invalid]).is_err());
}
