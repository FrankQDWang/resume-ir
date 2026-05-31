fn main() {
    let mut args = std::env::args().skip(1);

    if matches!(args.next().as_deref(), Some("--identity")) && args.next().is_none() {
        println!("resume-cli");
    } else {
        eprintln!("resume-cli: no commands are implemented in S1");
        std::process::exit(2);
    }
}
