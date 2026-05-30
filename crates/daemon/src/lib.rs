use std::io::{self, Write};

pub fn run_foreground_once<W>(stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    writeln!(stdout, "resume-daemon foreground lifecycle skeleton ready")
}

#[must_use]
pub fn crate_name() -> &'static str {
    "daemon"
}

#[must_use]
pub fn binary_name() -> &'static str {
    "resume-daemon"
}
