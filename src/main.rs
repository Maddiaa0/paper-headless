fn main() {
    if let Err(error) = paper_headless::run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
