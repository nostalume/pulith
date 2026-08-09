use super::*;

#[test]
fn dispatcher_selects_only_its_release_private_binary() {
    let own = if cfg!(windows) {
        Path::new(r"C:\root\current\shims\tool.exe")
    } else {
        Path::new("/root/current/shims/tool")
    };
    let expected = if cfg!(windows) {
        Path::new(r"C:\root\current\bin\tool.exe")
    } else {
        Path::new("/root/current/bin/tool")
    };
    assert_eq!(selected_target(own).unwrap(), expected);
}
