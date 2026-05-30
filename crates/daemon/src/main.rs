use std::io;

fn main() {
    let mut stdout = io::stdout().lock();
    if let Err(error) = daemon::run_foreground_once(&mut stdout) {
        eprintln!("error: failed to run daemon foreground skeleton: {error}");
        std::process::exit(1);
    }
}
