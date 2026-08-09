use std::time::Duration;

fn main() {
    let home = std::env::var_os("TOOLHOST_HOME").expect("TOOLHOST_HOME");
    if std::fs::write(std::path::PathBuf::from(home).join("runtime-write"), b"unsafe").is_ok() {
        std::process::exit(41);
    }
    if std::fs::write(std::env::temp_dir().join("toolhost-private-temp"), b"ok").is_err() {
        std::process::exit(42);
    }
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}
