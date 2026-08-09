fn main() {
    let shim = std::path::Path::new("shims").join(format!(
        "tool{}",
        std::env::consts::EXE_SUFFIX
    ));
    let _ = std::fs::remove_file(shim);
}
