use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;
use std::thread;
use std::time::Duration;

fn main() {
    assert_eq!(env::args().nth(1).as_deref(), Some("--resident"));
    let executable = env::current_exe().unwrap();
    let name = executable.file_stem().unwrap().to_string_lossy();
    let model_id = env::var("RESUME_IR_EMBEDDING_MODEL_ID").unwrap();
    let dimension = env::var("RESUME_IR_EMBEDDING_DIMENSION")
        .unwrap()
        .parse::<usize>()
        .unwrap();
    append(&executable.with_extension("spawns"), b"spawn\n");
    write_frame(&format!(
        "{{\"type\":\"ready\",\"schema_version\":\"resume-ir.embedding-stream.v1\",\"model_id\":\"{model_id}\",\"dimension\":{dimension}}}"
    ));
    append(&executable.with_extension("ready"), b"ready\n");

    while let Some(payload) = read_frame() {
        append(&executable.with_extension("requests"), b"request\n");
        let order = if payload.contains("\"role\":\"query\"") {
            b"query\n".as_slice()
        } else {
            b"passage\n".as_slice()
        };
        append(&executable.with_extension("order"), order);
        if name.contains("crash_always") {
            std::process::exit(3);
        }
        if name.contains("crash_once") {
            let marker = executable.with_extension("crashed");
            if !marker.exists() {
                fs::write(marker, b"crashed").unwrap();
                std::process::exit(3);
            }
        }
        if name.contains("slow") {
            thread::sleep(Duration::from_millis(300));
        }
        let request_id = json_u64(&payload, "\"request_id\":").unwrap();
        let count = payload.matches("\"role\":").count();
        let vector = std::iter::once("1")
            .chain(std::iter::repeat_n("0", dimension.saturating_sub(1)))
            .collect::<Vec<_>>()
            .join(",");
        let vectors = std::iter::repeat_n(format!("[{vector}]"), count)
            .collect::<Vec<_>>()
            .join(",");
        write_frame(&format!(
            "{{\"type\":\"result\",\"schema_version\":\"resume-ir.embedding-stream.v1\",\"request_id\":{request_id},\"vectors\":[{vectors}]}}"
        ));
    }
}

fn append(path: &Path, bytes: &[u8]) {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap()
        .write_all(bytes)
        .unwrap();
}

fn read_frame() -> Option<String> {
    let mut prefix = [0_u8; 4];
    match io::stdin().read_exact(&mut prefix) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return None,
        Err(error) => panic!("{error}"),
    }
    let mut payload = vec![0_u8; u32::from_be_bytes(prefix) as usize];
    io::stdin().read_exact(&mut payload).unwrap();
    Some(String::from_utf8(payload).unwrap())
}

fn write_frame(payload: &str) {
    let mut stdout = io::stdout().lock();
    stdout
        .write_all(&(payload.len() as u32).to_be_bytes())
        .unwrap();
    stdout.write_all(payload.as_bytes()).unwrap();
    stdout.flush().unwrap();
}

fn json_u64(payload: &str, key: &str) -> Option<u64> {
    let value = payload.split_once(key)?.1;
    value
        .bytes()
        .take_while(u8::is_ascii_digit)
        .map(char::from)
        .collect::<String>()
        .parse()
        .ok()
}
