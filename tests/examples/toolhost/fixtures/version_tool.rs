fn main() {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        print!("tool/1\n");
    }
}
