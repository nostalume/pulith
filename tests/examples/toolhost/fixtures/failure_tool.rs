fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--fail") => std::process::exit(7),
        Some("--version") => print!("actual\n"),
        _ => {}
    }
}
