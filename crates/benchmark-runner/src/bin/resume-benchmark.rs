use std::fs;
use std::path::PathBuf;

use benchmark_runner::{
    evaluate_benchmark_gate_json, evaluate_field_quality_gate_json,
    evaluate_ocr_throughput_gate_json, run_field_quality_jsonl,
    run_synthetic_ocr_throughput_benchmark, run_synthetic_query_benchmark, BenchmarkError,
    BenchmarkGateConfig, BenchmarkGateError, FieldQualityGateConfig, OcrThroughputGateConfig,
    SyntheticBenchmarkConfig, SyntheticOcrBenchmarkConfig, SyntheticOcrBenchmarkEngine,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("resume-benchmark: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), CliError> {
    match parse_command(std::env::args().skip(1))? {
        CliCommand::SyntheticQuery(args) => run_synthetic_query(args),
        CliCommand::Gate(args) => run_gate(args),
        CliCommand::FieldQuality(args) => run_field_quality(args),
        CliCommand::FieldGate(args) => run_field_gate(args),
        CliCommand::OcrThroughput(args) => run_ocr_throughput(args),
        CliCommand::OcrGate(args) => run_ocr_gate(args),
    }
}

fn run_synthetic_query(args: SyntheticQueryArgs) -> Result<(), CliError> {
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

fn run_gate(args: GateArgs) -> Result<(), CliError> {
    let report_json = fs::read_to_string(&args.report)
        .map_err(|_| CliError::user("unable to read benchmark report"))?;
    let mut config =
        BenchmarkGateConfig::new(args.min_documents, args.min_queries, args.max_p95_ms)
            .with_max_zero_result_queries(args.max_zero_result_queries);
    if args.allow_synthetic {
        config = config.allow_synthetic();
    }
    evaluate_benchmark_gate_json(&report_json, config).map_err(CliError::gate)?;
    println!("benchmark gate passed");
    Ok(())
}

fn run_field_quality(args: FieldQualityArgs) -> Result<(), CliError> {
    let dataset_jsonl = fs::read_to_string(&args.dataset)
        .map_err(|_| CliError::user("unable to read field quality dataset"))?;
    let report = run_field_quality_jsonl(&dataset_jsonl).map_err(CliError::benchmark)?;
    println!("{}", report.to_redacted_json());
    Ok(())
}

fn run_field_gate(args: FieldGateArgs) -> Result<(), CliError> {
    let report_json = fs::read_to_string(&args.report)
        .map_err(|_| CliError::user("unable to read field quality report"))?;
    let config = FieldQualityGateConfig::new(args.min_precision, args.min_recall, args.min_f1)
        .with_min_samples(args.min_samples);
    evaluate_field_quality_gate_json(&report_json, config).map_err(CliError::gate)?;
    println!("field quality gate passed");
    Ok(())
}

fn run_ocr_throughput(args: OcrThroughputArgs) -> Result<(), CliError> {
    let config = SyntheticOcrBenchmarkConfig::new(args.pages, args.page_timeout_ms)
        .map_err(CliError::benchmark)?
        .with_render_dpi(args.render_dpi)
        .map_err(CliError::benchmark)?;
    let engine = match (args.command, args.tesseract_command) {
        (Some(command), None) => {
            SyntheticOcrBenchmarkEngine::local_command(command).map_err(CliError::benchmark)?
        }
        (None, Some(command)) => {
            SyntheticOcrBenchmarkEngine::tesseract(command).map_err(CliError::benchmark)?
        }
        _ => return Err(CliError::usage()),
    };
    let report =
        run_synthetic_ocr_throughput_benchmark(engine, config).map_err(CliError::benchmark)?;
    println!("{}", report.to_redacted_json());
    Ok(())
}

fn run_ocr_gate(args: OcrGateArgs) -> Result<(), CliError> {
    let report_json = fs::read_to_string(&args.report)
        .map_err(|_| CliError::user("unable to read OCR throughput report"))?;
    let mut config =
        OcrThroughputGateConfig::new(args.min_pages, args.max_p95_ms, args.min_pages_per_second);
    if args.allow_synthetic {
        config = config.allow_synthetic();
    }
    evaluate_ocr_throughput_gate_json(&report_json, config).map_err(CliError::gate)?;
    println!("OCR throughput gate passed");
    Ok(())
}

fn parse_command<I>(args: I) -> Result<CliCommand, CliError>
where
    I: IntoIterator<Item = String>,
{
    let args = args.into_iter().collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("field-quality") => parse_field_quality_args(&args[1..]).map(CliCommand::FieldQuality),
        Some("field-gate") => parse_field_gate_args(&args[1..]).map(CliCommand::FieldGate),
        Some("ocr-throughput") => {
            parse_ocr_throughput_args(&args[1..]).map(CliCommand::OcrThroughput)
        }
        Some("ocr-gate") => parse_ocr_gate_args(&args[1..]).map(CliCommand::OcrGate),
        Some("gate") => parse_gate_args(&args[1..]).map(CliCommand::Gate),
        _ => parse_synthetic_query_args(&args).map(CliCommand::SyntheticQuery),
    }
}

fn parse_synthetic_query_args(args: &[String]) -> Result<SyntheticQueryArgs, CliError> {
    let mut data_dir = PathBuf::from("local-data");
    let mut index_dir = None;
    let mut documents = 1_000_usize;
    let mut queries = 100_usize;
    let mut top_k = 10_usize;
    let mut index = usize::from(args.first().map(String::as_str) == Some("synthetic-query"));

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

    Ok(SyntheticQueryArgs {
        data_dir,
        cleanup_after_run: index_dir.is_none(),
        index_dir,
        documents,
        queries,
        top_k,
    })
}

fn parse_gate_args(args: &[String]) -> Result<GateArgs, CliError> {
    let mut report = None;
    let mut allow_synthetic = false;
    let mut min_documents = 100_000_usize;
    let mut min_queries = 100_usize;
    let mut max_p95_ms = 200.0_f64;
    let mut max_zero_result_queries = 0_usize;
    let mut index = 0_usize;

    while index < args.len() {
        match args[index].as_str() {
            "--report" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                report = Some(PathBuf::from(value));
                index += 2;
            }
            "--allow-synthetic" => {
                allow_synthetic = true;
                index += 1;
            }
            "--min-documents" => {
                min_documents = parse_positive_usize(args.get(index + 1))?;
                index += 2;
            }
            "--min-queries" => {
                min_queries = parse_positive_usize(args.get(index + 1))?;
                index += 2;
            }
            "--max-p95-ms" => {
                max_p95_ms = parse_positive_f64(args.get(index + 1))?;
                index += 2;
            }
            "--max-zero-result-queries" => {
                max_zero_result_queries = parse_nonnegative_usize(args.get(index + 1))?;
                index += 2;
            }
            "--help" | "-h" => {
                return Err(CliError::user(usage()));
            }
            _ => return Err(CliError::usage()),
        }
    }

    Ok(GateArgs {
        report: report.ok_or_else(CliError::usage)?,
        allow_synthetic,
        min_documents,
        min_queries,
        max_p95_ms,
        max_zero_result_queries,
    })
}

fn parse_field_quality_args(args: &[String]) -> Result<FieldQualityArgs, CliError> {
    let mut dataset = None;
    let mut index = 0_usize;

    while index < args.len() {
        match args[index].as_str() {
            "--dataset" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                dataset = Some(PathBuf::from(value));
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

    Ok(FieldQualityArgs {
        dataset: dataset.ok_or_else(CliError::usage)?,
    })
}

fn parse_field_gate_args(args: &[String]) -> Result<FieldGateArgs, CliError> {
    let mut report = None;
    let mut min_samples = 1_usize;
    let mut min_precision = 0.95_f64;
    let mut min_recall = 0.95_f64;
    let mut min_f1 = 0.95_f64;
    let mut index = 0_usize;

    while index < args.len() {
        match args[index].as_str() {
            "--report" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                report = Some(PathBuf::from(value));
                index += 2;
            }
            "--min-samples" => {
                min_samples = parse_positive_usize(args.get(index + 1))?;
                index += 2;
            }
            "--min-precision" => {
                min_precision = parse_positive_f64(args.get(index + 1))?;
                index += 2;
            }
            "--min-recall" => {
                min_recall = parse_positive_f64(args.get(index + 1))?;
                index += 2;
            }
            "--min-f1" => {
                min_f1 = parse_positive_f64(args.get(index + 1))?;
                index += 2;
            }
            "--help" | "-h" => {
                return Err(CliError::user(usage()));
            }
            _ => return Err(CliError::usage()),
        }
    }

    Ok(FieldGateArgs {
        report: report.ok_or_else(CliError::usage)?,
        min_samples,
        min_precision,
        min_recall,
        min_f1,
    })
}

fn parse_ocr_throughput_args(args: &[String]) -> Result<OcrThroughputArgs, CliError> {
    let mut command = None;
    let mut tesseract_command = None;
    let mut pages = 10_usize;
    let mut page_timeout_ms = 30_000_u64;
    let mut render_dpi = 150_u32;
    let mut index = 0_usize;

    while index < args.len() {
        match args[index].as_str() {
            "--command" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                if command.is_some() {
                    return Err(CliError::usage());
                }
                command = Some(PathBuf::from(value));
                index += 2;
            }
            "--tesseract-command" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                if tesseract_command.is_some() {
                    return Err(CliError::usage());
                }
                tesseract_command = Some(PathBuf::from(value));
                index += 2;
            }
            "--pages" => {
                pages = parse_positive_usize(args.get(index + 1))?;
                index += 2;
            }
            "--page-timeout-ms" => {
                page_timeout_ms = parse_positive_u64(args.get(index + 1))?;
                index += 2;
            }
            "--render-dpi" => {
                render_dpi = parse_positive_u32(args.get(index + 1))?;
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

    if command.is_some() == tesseract_command.is_some() {
        return Err(CliError::usage());
    }

    Ok(OcrThroughputArgs {
        command,
        tesseract_command,
        pages,
        page_timeout_ms,
        render_dpi,
    })
}

fn parse_ocr_gate_args(args: &[String]) -> Result<OcrGateArgs, CliError> {
    let mut report = None;
    let mut allow_synthetic = false;
    let mut min_pages = 200_usize;
    let mut max_p95_ms = 30_000.0_f64;
    let mut min_pages_per_second = 0.1_f64;
    let mut index = 0_usize;

    while index < args.len() {
        match args[index].as_str() {
            "--report" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                report = Some(PathBuf::from(value));
                index += 2;
            }
            "--allow-synthetic" => {
                allow_synthetic = true;
                index += 1;
            }
            "--min-pages" => {
                min_pages = parse_positive_usize(args.get(index + 1))?;
                index += 2;
            }
            "--max-p95-ms" => {
                max_p95_ms = parse_positive_f64(args.get(index + 1))?;
                index += 2;
            }
            "--min-pages-per-second" => {
                min_pages_per_second = parse_positive_f64(args.get(index + 1))?;
                index += 2;
            }
            "--help" | "-h" => {
                return Err(CliError::user(usage()));
            }
            _ => return Err(CliError::usage()),
        }
    }

    Ok(OcrGateArgs {
        report: report.ok_or_else(CliError::usage)?,
        allow_synthetic,
        min_pages,
        max_p95_ms,
        min_pages_per_second,
    })
}

fn parse_positive_usize(value: Option<&String>) -> Result<usize, CliError> {
    value
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .ok_or_else(CliError::usage)
}

fn parse_positive_u64(value: Option<&String>) -> Result<u64, CliError> {
    value
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .ok_or_else(CliError::usage)
}

fn parse_positive_u32(value: Option<&String>) -> Result<u32, CliError> {
    value
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .ok_or_else(CliError::usage)
}

fn parse_nonnegative_usize(value: Option<&String>) -> Result<usize, CliError> {
    value
        .and_then(|value| value.parse::<usize>().ok())
        .ok_or_else(CliError::usage)
}

fn parse_positive_f64(value: Option<&String>) -> Result<f64, CliError> {
    value
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| *value > 0.0)
        .ok_or_else(CliError::usage)
}

fn usage() -> &'static str {
    "usage: resume-benchmark [synthetic-query] [--data-dir <path> | --index-dir <path>] [--documents <n>] [--queries <n>] [--top-k <n>] [--json] OR resume-benchmark gate --report <path> [--allow-synthetic] [--min-documents <n>] [--min-queries <n>] [--max-p95-ms <n>] [--max-zero-result-queries <n>] OR resume-benchmark field-quality --dataset <jsonl> [--json] OR resume-benchmark field-gate --report <path> [--min-samples <n>] [--min-precision <n>] [--min-recall <n>] [--min-f1 <n>] OR resume-benchmark ocr-throughput (--command <path>|--tesseract-command <path>) [--pages <n>] [--page-timeout-ms <n>] [--render-dpi <n>] [--json] OR resume-benchmark ocr-gate --report <path> [--allow-synthetic] [--min-pages <n>] [--max-p95-ms <n>] [--min-pages-per-second <n>]"
}

#[derive(Clone, Debug)]
enum CliCommand {
    SyntheticQuery(SyntheticQueryArgs),
    Gate(GateArgs),
    FieldQuality(FieldQualityArgs),
    FieldGate(FieldGateArgs),
    OcrThroughput(OcrThroughputArgs),
    OcrGate(OcrGateArgs),
}

#[derive(Clone, Debug)]
struct SyntheticQueryArgs {
    data_dir: PathBuf,
    index_dir: Option<PathBuf>,
    cleanup_after_run: bool,
    documents: usize,
    queries: usize,
    top_k: usize,
}

#[derive(Clone, Debug)]
struct GateArgs {
    report: PathBuf,
    allow_synthetic: bool,
    min_documents: usize,
    min_queries: usize,
    max_p95_ms: f64,
    max_zero_result_queries: usize,
}

#[derive(Clone, Debug)]
struct FieldQualityArgs {
    dataset: PathBuf,
}

#[derive(Clone, Debug)]
struct FieldGateArgs {
    report: PathBuf,
    min_samples: usize,
    min_precision: f64,
    min_recall: f64,
    min_f1: f64,
}

#[derive(Clone, Debug)]
struct OcrThroughputArgs {
    command: Option<PathBuf>,
    tesseract_command: Option<PathBuf>,
    pages: usize,
    page_timeout_ms: u64,
    render_dpi: u32,
}

#[derive(Clone, Debug)]
struct OcrGateArgs {
    report: PathBuf,
    allow_synthetic: bool,
    min_pages: usize,
    max_p95_ms: f64,
    min_pages_per_second: f64,
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

    fn gate(error: BenchmarkGateError) -> Self {
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
