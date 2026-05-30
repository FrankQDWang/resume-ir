use std::io;

fn main() {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    let code = resume_cli::run(std::env::args(), &mut stdout, &mut stderr);
    std::process::exit(code);
}
