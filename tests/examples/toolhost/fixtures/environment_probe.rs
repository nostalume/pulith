fn main() {
    let output = std::env::args_os().nth(1).unwrap();
    std::fs::write(
        output,
        format!(
            "{}\n{}",
            std::env::var("TOOLHOST_HOME").unwrap(),
            std::env::var("PATH").unwrap()
        ),
    )
    .unwrap();
}
