use config::{Profile, RuntimeProfile};
use meta_store::{DocumentRecord, MetaStore};
use std::io::Write;
use std::path::Path;

pub fn run<I, S, W, E>(args: I, stdout: &mut W, stderr: &mut E) -> i32
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
    W: Write,
    E: Write,
{
    let args: Vec<String> = args
        .into_iter()
        .map(|arg| arg.as_ref().to_owned())
        .collect();
    let Some(command) = args.get(1).map(String::as_str) else {
        return write_stderr(stderr, "usage: resume-cli <status|import|search>");
    };

    match command {
        "status" => status(stdout, stderr),
        "import" => import_root(&args[2..], stdout, stderr),
        "search" => search(&args[2..], stdout, stderr),
        _ => write_stderr(
            stderr,
            "unknown command; expected status, import, or search",
        ),
    }
}

fn status<W, E>(stdout: &mut W, stderr: &mut E) -> i32
where
    W: Write,
    E: Write,
{
    let profile = RuntimeProfile::default();
    let output = format!(
        "health: ok\nindexed_documents: 0\nsearchable_documents: 0\nactive_profile: {}\n",
        profile_name(profile.profile)
    );
    write_stdout(stdout, stderr, &output)
}

fn import_root<W, E>(args: &[String], stdout: &mut W, stderr: &mut E) -> i32
where
    W: Write,
    E: Write,
{
    let Some(root) = flag_value(args, "--root") else {
        return write_stderr(stderr, "usage: resume-cli import --root <path>");
    };
    let root_path = Path::new(root);
    if !root_path.is_dir() {
        return write_stderr(stderr, "root is not a readable directory");
    }

    let result = queue_import(root_path);
    match result {
        Ok(job_id) => {
            let output = format!("import_job: queued\njob_id: {}\n", job_id.as_str());
            write_stdout(stdout, stderr, &output)
        }
        Err(error) => write_stderr(stderr, &format!("failed to queue import job: {error}")),
    }
}

fn search<W, E>(args: &[String], stdout: &mut W, stderr: &mut E) -> i32
where
    W: Write,
    E: Write,
{
    if args.is_empty() {
        return write_stderr(stderr, "usage: resume-cli search <query>");
    }
    let query = args.join(" ");
    let output =
        format!("query: {query}\nresults: 0\nmessage: full-text index is not available yet\n");
    write_stdout(stdout, stderr, &output)
}

fn queue_import(root_path: &Path) -> meta_store::StoreResult<meta_store::JobId> {
    let store = MetaStore::open_in_memory()?;
    store.apply_migrations()?;

    let normalized_path = root_path.to_string_lossy().into_owned();
    let file_name = root_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("root")
        .to_owned();

    store.upsert_document(&DocumentRecord {
        doc_id: "doc_import_root".to_owned(),
        source_uri: normalized_path.clone(),
        normalized_path,
        file_name,
        extension: "directory".to_owned(),
        byte_size: 0,
        mtime_unix_ms: 0,
        is_deleted: false,
    })?;
    store.create_ingest_job("doc_import_root", 3)
}

fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|window| window[0] == flag)
        .map(|window| window[1].as_str())
}

fn profile_name(profile: Profile) -> &'static str {
    match profile {
        Profile::Economy => "economy",
        Profile::Balanced => "balanced",
        Profile::Turbo => "turbo",
    }
}

fn write_stdout<W, E>(stdout: &mut W, stderr: &mut E, message: &str) -> i32
where
    W: Write,
    E: Write,
{
    match stdout.write_all(message.as_bytes()) {
        Ok(()) => 0,
        Err(error) => write_stderr(stderr, &format!("failed to write output: {error}")),
    }
}

fn write_stderr<E>(stderr: &mut E, message: &str) -> i32
where
    E: Write,
{
    match writeln!(stderr, "error: {message}") {
        Ok(()) => 1,
        Err(_) => 1,
    }
}

#[must_use]
pub fn crate_name() -> &'static str {
    "resume-cli"
}

#[must_use]
pub fn binary_name() -> &'static str {
    "resume-cli"
}
