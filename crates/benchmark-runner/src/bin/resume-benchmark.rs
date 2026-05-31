use std::fs;
use std::path::PathBuf;

use benchmark_runner::{run_synthetic_query_benchmark, BenchmarkError, SyntheticBenchmarkConfig};

fn main() {
    if let Err(error) = run() {
        eprintln!("resume-benchmark: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), CliError> {
    let args = parse_args(std::env::args().skip(1))?;
    let index_dir = args.index_dir.unwrap_or_else(|| {
        args.data_dir
            .join("benchmark-scratch")
            .join(format!("synthetic-query-{}", std::process::id()))
    });
    let cleanup_after_run = args.cleanup_after_run;
    let _ = fs::remove_dir_all(&index_dir);
    fs::create_dir_all(&index_dir)
        .map_err(|_| CliError::user("unable to prepare benchmark scratch directory"))?;

    let config = SyntheticBenchmarkConfig::new(args.documents, args.queries)
        .map_err(CliError::benchmark)?
        .with_top_k(args.top_k);
    let result = run_synthetic_query_benchmark(&index_dir, config).map_err(CliError::benchmark);

    let report = result?;
    if cleanup_after_run {
        fs::remove_dir_all(&index_dir)
            .map_err(|_| CliError::user("unable to clean benchmark scratch directory"))?;
    }
    println!("{}", report.to_redacted_json());
    Ok(())
}

fn parse_args<I>(args: I) -> Result<CliArgs, CliError>
where
    I: IntoIterator<Item = String>,
{
    let mut data_dir = PathBuf::from("local-data");
    let mut index_dir = None;
    let mut documents = 1_000_usize;
    let mut queries = 100_usize;
    let mut top_k = 10_usize;
    let args = args.into_iter().collect::<Vec<_>>();
    let mut index = 0_usize;

    if args.first().map(String::as_str) == Some("synthetic-query") {
        index = 1;
    }

    while index < args.len() {
        match args[index].as_str() {
            "--data-dir" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                data_dir = PathBuf::from(value);
                index += 2;
            }
            "--index-dir" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                index_dir = Some(PathBuf::from(value));
                index += 2;
            }
            "--documents" => {
                documents = parse_positive_usize(args.get(index + 1))?;
                index += 2;
            }
            "--queries" => {
                queries = parse_positive_usize(args.get(index + 1))?;
                index += 2;
            }
            "--top-k" => {
                top_k = parse_positive_usize(args.get(index + 1))?;
                index += 2;
            }
            "--json" => {
                index += 1;
            }
            "--help" | "-h" => {
                return Err(CliError::user(usage()));
            }
            _ => return Err(CliError::usage()),
        }
    }

    Ok(CliArgs {
        data_dir,
        cleanup_after_run: index_dir.is_none(),
        index_dir,
        documents,
        queries,
        top_k,
    })
}

fn parse_positive_usize(value: Option<&String>) -> Result<usize, CliError> {
    value
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .ok_or_else(CliError::usage)
}

fn usage() -> &'static str {
    "usage: resume-benchmark [synthetic-query] [--data-dir <path> | --index-dir <path>] [--documents <n>] [--queries <n>] [--top-k <n>] [--json]"
}

#[derive(Clone, Debug)]
struct CliArgs {
    data_dir: PathBuf,
    index_dir: Option<PathBuf>,
    cleanup_after_run: bool,
    documents: usize,
    queries: usize,
    top_k: usize,
}

#[derive(Clone, Debug)]
struct CliError {
    message: String,
}

impl CliError {
    fn usage() -> Self {
        Self::user(usage())
    }

    fn user(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn benchmark(error: BenchmarkError) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}
