//! Minimal CLI binary entry point.

fn main() {
    let mut stdout = std::io::stdout();
    if let Err(error) = resume_cli::run_with_args(std::env::args(), &mut stdout) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
