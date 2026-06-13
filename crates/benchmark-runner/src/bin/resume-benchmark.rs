use std::fs;
use std::path::PathBuf;

use benchmark_runner::{
    evaluate_benchmark_gate_json, evaluate_dedupe_quality_gate_json,
    evaluate_field_quality_gate_json, evaluate_ocr_throughput_gate_json,
    evaluate_vector_quality_gate_json, run_dedupe_quality_jsonl, run_field_quality_jsonl,
    run_private_business_dedupe_quality_jsonl, run_private_business_field_quality_jsonl,
    run_private_business_vector_quality_jsonl, run_private_ocr_throughput_benchmark,
    run_private_query_benchmark, run_synthetic_ocr_throughput_benchmark,
    run_synthetic_query_benchmark, run_vector_quality_jsonl, BenchmarkError, BenchmarkGateConfig,
    BenchmarkGateError, DedupeQualityGateConfig, FieldQualityGateConfig, OcrThroughputGateConfig,
    PrivateDedupeQualityManifestDigests, PrivateFieldQualityManifestDigests,
    PrivateOcrBenchmarkEngine, PrivateOcrManifestDigests, PrivateOcrThroughputConfig,
    PrivatePdfRenderEngine, PrivateQueryBenchmarkCommand, PrivateQueryBenchmarkConfig,
    PrivateQueryCorpusSummary, PrivateQueryManifestDigests, PrivateVectorQualityManifestDigests,
    SyntheticBenchmarkConfig, SyntheticOcrBenchmarkConfig, SyntheticOcrBenchmarkEngine,
    VectorQualityConfig, VectorQualityGateConfig,
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
        CliCommand::PrivateQuery(args) => run_private_query(args),
        CliCommand::Gate(args) => run_gate(args),
        CliCommand::FieldQuality(args) => run_field_quality(args),
        CliCommand::FieldGate(args) => run_field_gate(args),
        CliCommand::DedupeQuality(args) => run_dedupe_quality(args),
        CliCommand::DedupeGate(args) => run_dedupe_gate(args),
        CliCommand::OcrThroughput(args) => run_ocr_throughput(args),
        CliCommand::PrivateOcrThroughput(args) => run_private_ocr_throughput(args),
        CliCommand::OcrGate(args) => run_ocr_gate(args),
        CliCommand::VectorQuality(args) => run_vector_quality(args),
        CliCommand::VectorGate(args) => run_vector_gate(args),
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

fn run_private_query(args: PrivateQueryArgs) -> Result<(), CliError> {
    let manifests = PrivateQueryManifestDigests::new(
        args.dataset_manifest_sha256,
        args.query_set_sha256,
        args.model_manifest_sha256,
    )
    .map_err(CliError::benchmark)?;
    let command =
        PrivateQueryBenchmarkCommand::local_command_with_args(args.command, args.command_args)
            .map_err(CliError::benchmark)?;
    let corpus_summary = if args.allow_partial_hot_index_for_smoke {
        PrivateQueryCorpusSummary::from_redacted_json_file_allowing_partial_hot_index_for_smoke(
            args.corpus_summary,
        )
    } else {
        PrivateQueryCorpusSummary::from_redacted_json_file(args.corpus_summary)
    }
    .map_err(CliError::benchmark)?;
    let config =
        PrivateQueryBenchmarkConfig::new(args.query_set, command, corpus_summary, manifests)
            .and_then(|config| config.with_max_queries(args.max_queries))
            .and_then(|config| config.with_top_k(args.top_k))
            .and_then(|config| config.with_timeout_ms(args.timeout_ms))
            .map(|config| config.with_index_size_bytes(args.index_size_bytes))
            .map_err(CliError::benchmark)?;
    let report = run_private_query_benchmark(config).map_err(CliError::benchmark)?;
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
    if args.allow_smoke_confidence {
        config = config.allow_smoke_confidence();
    }
    if args.require_private_real_corpus {
        config = config.require_private_real_corpus();
    }
    if args.require_million_scale {
        config = config.require_million_scale();
    }
    evaluate_benchmark_gate_json(&report_json, config).map_err(CliError::gate)?;
    println!("benchmark gate passed");
    Ok(())
}

fn run_field_quality(args: FieldQualityArgs) -> Result<(), CliError> {
    let dataset_jsonl = fs::read_to_string(&args.dataset)
        .map_err(|_| CliError::user("unable to read field quality dataset"))?;
    let report = if args.private_business_labeled {
        let manifests = PrivateFieldQualityManifestDigests::new(
            args.dataset_manifest_sha256.ok_or_else(CliError::usage)?,
            args.annotation_manifest_sha256
                .ok_or_else(CliError::usage)?,
        )
        .map_err(CliError::benchmark)?;
        run_private_business_field_quality_jsonl(&dataset_jsonl, manifests)
            .map_err(CliError::benchmark)?
    } else {
        if args.dataset_manifest_sha256.is_some() || args.annotation_manifest_sha256.is_some() {
            return Err(CliError::usage());
        }
        run_field_quality_jsonl(&dataset_jsonl).map_err(CliError::benchmark)?
    };
    println!("{}", report.to_redacted_json());
    Ok(())
}

fn run_field_gate(args: FieldGateArgs) -> Result<(), CliError> {
    let report_json = fs::read_to_string(&args.report)
        .map_err(|_| CliError::user("unable to read field quality report"))?;
    let mut config = FieldQualityGateConfig::new(args.min_precision, args.min_recall, args.min_f1)
        .with_min_samples(args.min_samples);
    if args.require_private_business_labeled {
        config = config.require_private_business_labeled();
    }
    evaluate_field_quality_gate_json(&report_json, config).map_err(CliError::gate)?;
    println!("field quality gate passed");
    Ok(())
}

fn run_dedupe_quality(args: DedupeQualityArgs) -> Result<(), CliError> {
    let dataset_jsonl = fs::read_to_string(&args.dataset)
        .map_err(|_| CliError::user("unable to read dedupe quality dataset"))?;
    let report = if args.private_business_labeled {
        let manifests = PrivateDedupeQualityManifestDigests::new(
            args.dataset_manifest_sha256.ok_or_else(CliError::usage)?,
            args.annotation_manifest_sha256
                .ok_or_else(CliError::usage)?,
        )
        .map_err(CliError::benchmark)?;
        run_private_business_dedupe_quality_jsonl(&dataset_jsonl, manifests)
            .map_err(CliError::benchmark)?
    } else {
        if args.dataset_manifest_sha256.is_some() || args.annotation_manifest_sha256.is_some() {
            return Err(CliError::usage());
        }
        run_dedupe_quality_jsonl(&dataset_jsonl).map_err(CliError::benchmark)?
    };
    println!("{}", report.to_redacted_json());
    Ok(())
}

fn run_dedupe_gate(args: DedupeGateArgs) -> Result<(), CliError> {
    let report_json = fs::read_to_string(&args.report)
        .map_err(|_| CliError::user("unable to read dedupe quality report"))?;
    let mut config = DedupeQualityGateConfig::new(args.min_precision, args.min_recall, args.min_f1)
        .with_min_pairs(args.min_pairs)
        .with_min_positive_pairs(args.min_positive_pairs);
    if args.require_private_business_labeled {
        config = config.require_private_business_labeled();
    }
    evaluate_dedupe_quality_gate_json(&report_json, config).map_err(CliError::gate)?;
    println!("dedupe quality gate passed");
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

fn run_private_ocr_throughput(args: PrivateOcrThroughputArgs) -> Result<(), CliError> {
    let ocr_engine = match (args.command, args.tesseract_command) {
        (Some(command), None) => {
            PrivateOcrBenchmarkEngine::local_command(command).map_err(CliError::benchmark)?
        }
        (None, Some(command)) => {
            PrivateOcrBenchmarkEngine::tesseract(command).map_err(CliError::benchmark)?
        }
        _ => return Err(CliError::usage()),
    };
    let renderer = match (args.renderer_command, args.pdftoppm_command) {
        (Some(command), None) => {
            PrivatePdfRenderEngine::local_command(command).map_err(CliError::benchmark)?
        }
        (None, Some(command)) => {
            PrivatePdfRenderEngine::pdftoppm(command).map_err(CliError::benchmark)?
        }
        _ => return Err(CliError::usage()),
    };
    let manifests = PrivateOcrManifestDigests::new(
        args.dataset_manifest_sha256,
        args.ocr_runtime_manifest_sha256,
        args.renderer_manifest_sha256,
        args.language_pack_manifest_sha256,
    )
    .map_err(CliError::benchmark)?;
    let config = PrivateOcrThroughputConfig::new(args.root, ocr_engine, renderer, manifests)
        .and_then(|config| config.with_max_documents(args.max_documents))
        .and_then(|config| config.with_max_pages(args.max_pages))
        .and_then(|config| config.with_pages_per_document(args.pages_per_document))
        .and_then(|config| config.with_page_timeout_ms(args.page_timeout_ms))
        .and_then(|config| match args.max_run_ms {
            Some(max_run_ms) => config.with_max_run_ms(max_run_ms),
            None => Ok(config),
        })
        .and_then(|config| config.with_render_dpi(args.render_dpi))
        .and_then(|config| config.with_ocr_lang(args.ocr_lang))
        .and_then(|config| config.with_engine_profile(args.engine_profile))
        .map_err(CliError::benchmark)?;
    let report = run_private_ocr_throughput_benchmark(config).map_err(CliError::benchmark)?;
    println!("{}", report.to_redacted_json());
    Ok(())
}

fn run_ocr_gate(args: OcrGateArgs) -> Result<(), CliError> {
    let report_json = fs::read_to_string(&args.report)
        .map_err(|_| CliError::user("unable to read OCR throughput report"))?;
    let mut config = if args.current_stage_baseline {
        OcrThroughputGateConfig::current_stage_baseline(args.min_pages)
    } else {
        OcrThroughputGateConfig::new(args.min_pages, args.max_p95_ms, args.min_pages_per_second)
    };
    if args.allow_synthetic {
        config = config.allow_synthetic();
    }
    if args.require_private_real_corpus {
        config = config.require_private_real_corpus();
    }
    evaluate_ocr_throughput_gate_json(&report_json, config).map_err(CliError::gate)?;
    println!("OCR throughput gate passed");
    Ok(())
}

fn run_vector_quality(args: VectorQualityArgs) -> Result<(), CliError> {
    let dataset_jsonl = fs::read_to_string(&args.dataset)
        .map_err(|_| CliError::user("unable to read vector quality dataset"))?;
    let config = VectorQualityConfig::new(&args.command, args.model_id, args.dimension)
        .map_err(CliError::benchmark)?
        .with_top_k(args.top_k)
        .with_timeout_ms(args.timeout_ms)
        .map_err(CliError::benchmark)?
        .with_max_text_bytes(args.max_text_bytes)
        .map_err(CliError::benchmark)?;
    let report = if args.private_business_labeled {
        let manifests = PrivateVectorQualityManifestDigests::new(
            args.dataset_manifest_sha256.ok_or_else(CliError::usage)?,
            args.annotation_manifest_sha256
                .ok_or_else(CliError::usage)?,
            args.model_manifest_sha256.ok_or_else(CliError::usage)?,
        )
        .map_err(CliError::benchmark)?;
        run_private_business_vector_quality_jsonl(&dataset_jsonl, config, manifests)
            .map_err(CliError::benchmark)?
    } else {
        if args.dataset_manifest_sha256.is_some()
            || args.annotation_manifest_sha256.is_some()
            || args.model_manifest_sha256.is_some()
        {
            return Err(CliError::usage());
        }
        run_vector_quality_jsonl(&dataset_jsonl, config).map_err(CliError::benchmark)?
    };
    println!("{}", report.to_redacted_json());
    Ok(())
}

fn run_vector_gate(args: VectorGateArgs) -> Result<(), CliError> {
    let report_json = fs::read_to_string(&args.report)
        .map_err(|_| CliError::user("unable to read vector quality report"))?;
    let mut config = VectorQualityGateConfig::new(
        args.min_samples,
        args.min_recall_at_k,
        args.min_mrr,
        args.min_ndcg_at_k,
    )
    .with_max_zero_recall_queries(args.max_zero_recall_queries);
    if args.require_private_business_labeled {
        config = config.require_private_business_labeled();
    }
    evaluate_vector_quality_gate_json(&report_json, config).map_err(CliError::gate)?;
    println!("vector quality gate passed");
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
        Some("dedupe-quality") => {
            parse_dedupe_quality_args(&args[1..]).map(CliCommand::DedupeQuality)
        }
        Some("dedupe-gate") => parse_dedupe_gate_args(&args[1..]).map(CliCommand::DedupeGate),
        Some("ocr-throughput") => {
            parse_ocr_throughput_args(&args[1..]).map(CliCommand::OcrThroughput)
        }
        Some("private-query") => parse_private_query_args(&args[1..]).map(CliCommand::PrivateQuery),
        Some("private-ocr-throughput") => {
            parse_private_ocr_throughput_args(&args[1..]).map(CliCommand::PrivateOcrThroughput)
        }
        Some("ocr-gate") => parse_ocr_gate_args(&args[1..]).map(CliCommand::OcrGate),
        Some("vector-quality") => {
            parse_vector_quality_args(&args[1..]).map(CliCommand::VectorQuality)
        }
        Some("vector-gate") => parse_vector_gate_args(&args[1..]).map(CliCommand::VectorGate),
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

fn parse_private_query_args(args: &[String]) -> Result<PrivateQueryArgs, CliError> {
    let mut query_set = None;
    let mut command = None;
    let mut corpus_summary = None;
    let mut max_queries = 500_usize;
    let mut top_k = 10_usize;
    let mut timeout_ms = 5_000_u64;
    let mut index_size_bytes = 0_u64;
    let mut dataset_manifest_sha256 = None;
    let mut query_set_sha256 = None;
    let mut model_manifest_sha256 = None;
    let mut allow_partial_hot_index_for_smoke = false;
    let mut command_args = Vec::new();
    let mut index = 0_usize;

    while index < args.len() {
        match args[index].as_str() {
            "--query-set" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                query_set = Some(PathBuf::from(value));
                index += 2;
            }
            "--command" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                command = Some(PathBuf::from(value));
                index += 2;
            }
            "--command-arg" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                command_args.push(value.to_string());
                index += 2;
            }
            "--corpus-summary" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                corpus_summary = Some(PathBuf::from(value));
                index += 2;
            }
            "--max-queries" => {
                max_queries = parse_positive_usize(args.get(index + 1))?;
                index += 2;
            }
            "--top-k" => {
                top_k = parse_positive_usize(args.get(index + 1))?;
                index += 2;
            }
            "--timeout-ms" => {
                timeout_ms = parse_positive_u64(args.get(index + 1))?;
                index += 2;
            }
            "--index-size-bytes" => {
                index_size_bytes = parse_nonnegative_u64(args.get(index + 1))?;
                index += 2;
            }
            "--dataset-manifest-sha256" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                dataset_manifest_sha256 = Some(value.to_string());
                index += 2;
            }
            "--query-set-sha256" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                query_set_sha256 = Some(value.to_string());
                index += 2;
            }
            "--model-manifest-sha256" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                model_manifest_sha256 = Some(value.to_string());
                index += 2;
            }
            "--allow-partial-hot-index-for-smoke" => {
                allow_partial_hot_index_for_smoke = true;
                index += 1;
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

    Ok(PrivateQueryArgs {
        query_set: query_set.ok_or_else(CliError::usage)?,
        command: command.ok_or_else(CliError::usage)?,
        command_args,
        corpus_summary: corpus_summary.ok_or_else(CliError::usage)?,
        max_queries,
        top_k,
        timeout_ms,
        index_size_bytes,
        dataset_manifest_sha256: dataset_manifest_sha256.ok_or_else(CliError::usage)?,
        query_set_sha256: query_set_sha256.ok_or_else(CliError::usage)?,
        model_manifest_sha256: model_manifest_sha256.ok_or_else(CliError::usage)?,
        allow_partial_hot_index_for_smoke,
    })
}

fn parse_gate_args(args: &[String]) -> Result<GateArgs, CliError> {
    let mut report = None;
    let mut allow_synthetic = false;
    let mut allow_smoke_confidence = false;
    let mut require_private_real_corpus = false;
    let mut require_million_scale = false;
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
            "--allow-smoke-confidence" => {
                allow_smoke_confidence = true;
                index += 1;
            }
            "--require-private-real-corpus" => {
                require_private_real_corpus = true;
                index += 1;
            }
            "--require-million-scale" => {
                require_million_scale = true;
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
        allow_smoke_confidence,
        require_private_real_corpus,
        require_million_scale,
        min_documents,
        min_queries,
        max_p95_ms,
        max_zero_result_queries,
    })
}

fn parse_field_quality_args(args: &[String]) -> Result<FieldQualityArgs, CliError> {
    let mut dataset = None;
    let mut private_business_labeled = false;
    let mut dataset_manifest_sha256 = None;
    let mut annotation_manifest_sha256 = None;
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
            "--private-business-labeled" => {
                private_business_labeled = true;
                index += 1;
            }
            "--dataset-manifest-sha256" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                dataset_manifest_sha256 = Some(value.clone());
                index += 2;
            }
            "--annotation-manifest-sha256" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                annotation_manifest_sha256 = Some(value.clone());
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
        private_business_labeled,
        dataset_manifest_sha256,
        annotation_manifest_sha256,
    })
}

fn parse_field_gate_args(args: &[String]) -> Result<FieldGateArgs, CliError> {
    let mut report = None;
    let mut require_private_business_labeled = false;
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
            "--require-private-business-labeled" => {
                require_private_business_labeled = true;
                index += 1;
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
        require_private_business_labeled,
        min_samples,
        min_precision,
        min_recall,
        min_f1,
    })
}

fn parse_dedupe_quality_args(args: &[String]) -> Result<DedupeQualityArgs, CliError> {
    let mut dataset = None;
    let mut private_business_labeled = false;
    let mut dataset_manifest_sha256 = None;
    let mut annotation_manifest_sha256 = None;
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
            "--private-business-labeled" => {
                private_business_labeled = true;
                index += 1;
            }
            "--dataset-manifest-sha256" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                dataset_manifest_sha256 = Some(value.clone());
                index += 2;
            }
            "--annotation-manifest-sha256" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                annotation_manifest_sha256 = Some(value.clone());
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

    Ok(DedupeQualityArgs {
        dataset: dataset.ok_or_else(CliError::usage)?,
        private_business_labeled,
        dataset_manifest_sha256,
        annotation_manifest_sha256,
    })
}

fn parse_dedupe_gate_args(args: &[String]) -> Result<DedupeGateArgs, CliError> {
    let mut report = None;
    let mut require_private_business_labeled = false;
    let mut min_pairs = 1_usize;
    let mut min_positive_pairs = 1_usize;
    let mut min_precision = 0.90_f64;
    let mut min_recall = 0.90_f64;
    let mut min_f1 = 0.90_f64;
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
            "--require-private-business-labeled" => {
                require_private_business_labeled = true;
                index += 1;
            }
            "--min-pairs" => {
                min_pairs = parse_positive_usize(args.get(index + 1))?;
                index += 2;
            }
            "--min-positive-pairs" => {
                min_positive_pairs = parse_positive_usize(args.get(index + 1))?;
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

    Ok(DedupeGateArgs {
        report: report.ok_or_else(CliError::usage)?,
        require_private_business_labeled,
        min_pairs,
        min_positive_pairs,
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

fn parse_private_ocr_throughput_args(
    args: &[String],
) -> Result<PrivateOcrThroughputArgs, CliError> {
    let mut root = None;
    let mut command = None;
    let mut tesseract_command = None;
    let mut renderer_command = None;
    let mut pdftoppm_command = None;
    let mut max_documents = 100_usize;
    let mut max_pages = 500_usize;
    let mut pages_per_document = 1_usize;
    let mut page_timeout_ms = 30_000_u64;
    let mut max_run_ms = None;
    let mut render_dpi = 150_u32;
    let mut ocr_lang = "eng".to_string();
    let mut engine_profile = "private-real-corpus".to_string();
    let mut dataset_manifest_sha256 = None;
    let mut ocr_runtime_manifest_sha256 = None;
    let mut renderer_manifest_sha256 = None;
    let mut language_pack_manifest_sha256 = None;
    let mut index = 0_usize;

    while index < args.len() {
        match args[index].as_str() {
            "--root" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                root = Some(PathBuf::from(value));
                index += 2;
            }
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
            "--renderer-command" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                if renderer_command.is_some() {
                    return Err(CliError::usage());
                }
                renderer_command = Some(PathBuf::from(value));
                index += 2;
            }
            "--pdftoppm-command" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                if pdftoppm_command.is_some() {
                    return Err(CliError::usage());
                }
                pdftoppm_command = Some(PathBuf::from(value));
                index += 2;
            }
            "--max-documents" => {
                max_documents = parse_positive_usize(args.get(index + 1))?;
                index += 2;
            }
            "--max-pages" => {
                max_pages = parse_positive_usize(args.get(index + 1))?;
                index += 2;
            }
            "--pages-per-document" => {
                pages_per_document = parse_positive_usize(args.get(index + 1))?;
                index += 2;
            }
            "--page-timeout-ms" => {
                page_timeout_ms = parse_positive_u64(args.get(index + 1))?;
                index += 2;
            }
            "--max-run-ms" => {
                max_run_ms = Some(parse_positive_u64(args.get(index + 1))?);
                index += 2;
            }
            "--render-dpi" => {
                render_dpi = parse_positive_u32(args.get(index + 1))?;
                index += 2;
            }
            "--ocr-lang" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                ocr_lang = value.to_string();
                index += 2;
            }
            "--engine-profile" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                engine_profile = value.to_string();
                index += 2;
            }
            "--dataset-manifest-sha256" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                dataset_manifest_sha256 = Some(value.to_string());
                index += 2;
            }
            "--ocr-runtime-manifest-sha256" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                ocr_runtime_manifest_sha256 = Some(value.to_string());
                index += 2;
            }
            "--renderer-manifest-sha256" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                renderer_manifest_sha256 = Some(value.to_string());
                index += 2;
            }
            "--language-pack-manifest-sha256" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                language_pack_manifest_sha256 = Some(value.to_string());
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

    Ok(PrivateOcrThroughputArgs {
        root: root.ok_or_else(CliError::usage)?,
        command,
        tesseract_command,
        renderer_command,
        pdftoppm_command,
        max_documents,
        max_pages,
        pages_per_document,
        page_timeout_ms,
        max_run_ms,
        render_dpi,
        ocr_lang,
        engine_profile,
        dataset_manifest_sha256: dataset_manifest_sha256.ok_or_else(CliError::usage)?,
        ocr_runtime_manifest_sha256: ocr_runtime_manifest_sha256.ok_or_else(CliError::usage)?,
        renderer_manifest_sha256: renderer_manifest_sha256.ok_or_else(CliError::usage)?,
        language_pack_manifest_sha256: language_pack_manifest_sha256.ok_or_else(CliError::usage)?,
    })
}

fn parse_ocr_gate_args(args: &[String]) -> Result<OcrGateArgs, CliError> {
    let mut report = None;
    let mut allow_synthetic = false;
    let mut require_private_real_corpus = false;
    let mut current_stage_baseline = false;
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
            "--require-private-real-corpus" => {
                require_private_real_corpus = true;
                index += 1;
            }
            "--current-stage-baseline" => {
                current_stage_baseline = true;
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
        require_private_real_corpus,
        current_stage_baseline,
        min_pages,
        max_p95_ms,
        min_pages_per_second,
    })
}

fn parse_vector_quality_args(args: &[String]) -> Result<VectorQualityArgs, CliError> {
    let mut dataset = None;
    let mut command = None;
    let mut model_id = None;
    let mut dimension = None;
    let mut private_business_labeled = false;
    let mut dataset_manifest_sha256 = None;
    let mut annotation_manifest_sha256 = None;
    let mut model_manifest_sha256 = None;
    let mut top_k = 10_usize;
    let mut timeout_ms = 30_000_u64;
    let mut max_text_bytes = 128 * 1024_usize;
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
            "--command" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                command = Some(PathBuf::from(value));
                index += 2;
            }
            "--model-id" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                model_id = Some(value.clone());
                index += 2;
            }
            "--dimension" => {
                dimension = Some(parse_positive_usize(args.get(index + 1))?);
                index += 2;
            }
            "--private-business-labeled" => {
                private_business_labeled = true;
                index += 1;
            }
            "--dataset-manifest-sha256" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                dataset_manifest_sha256 = Some(value.clone());
                index += 2;
            }
            "--annotation-manifest-sha256" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                annotation_manifest_sha256 = Some(value.clone());
                index += 2;
            }
            "--model-manifest-sha256" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage());
                };
                model_manifest_sha256 = Some(value.clone());
                index += 2;
            }
            "--top-k" => {
                top_k = parse_positive_usize(args.get(index + 1))?;
                index += 2;
            }
            "--timeout-ms" => {
                timeout_ms = parse_positive_u64(args.get(index + 1))?;
                index += 2;
            }
            "--max-text-bytes" => {
                max_text_bytes = parse_positive_usize(args.get(index + 1))?;
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

    Ok(VectorQualityArgs {
        dataset: dataset.ok_or_else(CliError::usage)?,
        command: command.ok_or_else(CliError::usage)?,
        model_id: model_id.ok_or_else(CliError::usage)?,
        dimension: dimension.ok_or_else(CliError::usage)?,
        private_business_labeled,
        dataset_manifest_sha256,
        annotation_manifest_sha256,
        model_manifest_sha256,
        top_k,
        timeout_ms,
        max_text_bytes,
    })
}

fn parse_vector_gate_args(args: &[String]) -> Result<VectorGateArgs, CliError> {
    let mut report = None;
    let mut min_samples = 100_usize;
    let mut min_recall_at_k = 0.80_f64;
    let mut min_mrr = 0.70_f64;
    let mut min_ndcg_at_k = 0.80_f64;
    let mut max_zero_recall_queries = 0_usize;
    let mut require_private_business_labeled = false;
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
            "--min-recall-at-k" => {
                min_recall_at_k = parse_positive_f64(args.get(index + 1))?;
                index += 2;
            }
            "--min-mrr" => {
                min_mrr = parse_positive_f64(args.get(index + 1))?;
                index += 2;
            }
            "--min-ndcg-at-k" => {
                min_ndcg_at_k = parse_positive_f64(args.get(index + 1))?;
                index += 2;
            }
            "--max-zero-recall-queries" => {
                max_zero_recall_queries = parse_nonnegative_usize(args.get(index + 1))?;
                index += 2;
            }
            "--require-private-business-labeled" => {
                require_private_business_labeled = true;
                index += 1;
            }
            "--help" | "-h" => {
                return Err(CliError::user(usage()));
            }
            _ => return Err(CliError::usage()),
        }
    }

    Ok(VectorGateArgs {
        report: report.ok_or_else(CliError::usage)?,
        require_private_business_labeled,
        min_samples,
        min_recall_at_k,
        min_mrr,
        min_ndcg_at_k,
        max_zero_recall_queries,
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

fn parse_nonnegative_u64(value: Option<&String>) -> Result<u64, CliError> {
    value
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(CliError::usage)
}

fn parse_positive_f64(value: Option<&String>) -> Result<f64, CliError> {
    value
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| *value > 0.0)
        .ok_or_else(CliError::usage)
}

fn usage() -> &'static str {
    "usage: resume-benchmark [synthetic-query] [--data-dir <path> | --index-dir <path>] [--documents <n>] [--queries <n>] [--top-k <n>] [--json] OR resume-benchmark private-query --query-set <jsonl> --command <path> [--command-arg <arg> ...] --corpus-summary <json> --dataset-manifest-sha256 <sha256> --query-set-sha256 <sha256> --model-manifest-sha256 <sha256> [--allow-partial-hot-index-for-smoke] [--max-queries <n>] [--top-k <n>] [--timeout-ms <n>] [--index-size-bytes <n>] [--json] OR resume-benchmark gate --report <path> [--allow-synthetic] [--allow-smoke-confidence] [--require-private-real-corpus] [--require-million-scale] [--min-documents <n>] [--min-queries <n>] [--max-p95-ms <n>] [--max-zero-result-queries <n>] OR resume-benchmark field-quality --dataset <jsonl> [--private-business-labeled --dataset-manifest-sha256 <sha256> --annotation-manifest-sha256 <sha256>] [--json] OR resume-benchmark field-gate --report <path> [--require-private-business-labeled] [--min-samples <n>] [--min-precision <n>] [--min-recall <n>] [--min-f1 <n>] OR resume-benchmark dedupe-quality --dataset <jsonl> [--private-business-labeled --dataset-manifest-sha256 <sha256> --annotation-manifest-sha256 <sha256>] [--json] OR resume-benchmark dedupe-gate --report <path> [--require-private-business-labeled] [--min-pairs <n>] [--min-positive-pairs <n>] [--min-precision <n>] [--min-recall <n>] [--min-f1 <n>] OR resume-benchmark ocr-throughput (--command <path>|--tesseract-command <path>) [--pages <n>] [--page-timeout-ms <n>] [--render-dpi <n>] [--json] OR resume-benchmark private-ocr-throughput --root <path> (--renderer-command <path>|--pdftoppm-command <path>) (--command <path>|--tesseract-command <path>) --dataset-manifest-sha256 <sha256> --ocr-runtime-manifest-sha256 <sha256> --renderer-manifest-sha256 <sha256> --language-pack-manifest-sha256 <sha256> [--max-documents <n>] [--max-pages <n>] [--pages-per-document <n>] [--page-timeout-ms <n>] [--max-run-ms <n>] [--render-dpi <n>] [--ocr-lang <lang>] [--engine-profile <id>] [--json] OR resume-benchmark ocr-gate --report <path> [--allow-synthetic] [--require-private-real-corpus] [--current-stage-baseline] [--min-pages <n>] [--max-p95-ms <n>] [--min-pages-per-second <n>] OR resume-benchmark vector-quality --dataset <jsonl> --command <path> --model-id <id> --dimension <n> [--private-business-labeled --dataset-manifest-sha256 <sha256> --annotation-manifest-sha256 <sha256> --model-manifest-sha256 <sha256>] [--top-k <n>] [--timeout-ms <n>] [--max-text-bytes <n>] [--json] OR resume-benchmark vector-gate --report <path> [--require-private-business-labeled] [--min-samples <n>] [--min-recall-at-k <n>] [--min-mrr <n>] [--min-ndcg-at-k <n>] [--max-zero-recall-queries <n>]"
}

#[derive(Clone, Debug)]
enum CliCommand {
    SyntheticQuery(SyntheticQueryArgs),
    PrivateQuery(PrivateQueryArgs),
    Gate(GateArgs),
    FieldQuality(FieldQualityArgs),
    FieldGate(FieldGateArgs),
    DedupeQuality(DedupeQualityArgs),
    DedupeGate(DedupeGateArgs),
    OcrThroughput(OcrThroughputArgs),
    PrivateOcrThroughput(PrivateOcrThroughputArgs),
    OcrGate(OcrGateArgs),
    VectorQuality(VectorQualityArgs),
    VectorGate(VectorGateArgs),
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
struct PrivateQueryArgs {
    query_set: PathBuf,
    command: PathBuf,
    command_args: Vec<String>,
    corpus_summary: PathBuf,
    max_queries: usize,
    top_k: usize,
    timeout_ms: u64,
    index_size_bytes: u64,
    dataset_manifest_sha256: String,
    query_set_sha256: String,
    model_manifest_sha256: String,
    allow_partial_hot_index_for_smoke: bool,
}

#[derive(Clone, Debug)]
struct GateArgs {
    report: PathBuf,
    allow_synthetic: bool,
    allow_smoke_confidence: bool,
    require_private_real_corpus: bool,
    require_million_scale: bool,
    min_documents: usize,
    min_queries: usize,
    max_p95_ms: f64,
    max_zero_result_queries: usize,
}

#[derive(Clone, Debug)]
struct FieldQualityArgs {
    dataset: PathBuf,
    private_business_labeled: bool,
    dataset_manifest_sha256: Option<String>,
    annotation_manifest_sha256: Option<String>,
}

#[derive(Clone, Debug)]
struct FieldGateArgs {
    report: PathBuf,
    require_private_business_labeled: bool,
    min_samples: usize,
    min_precision: f64,
    min_recall: f64,
    min_f1: f64,
}

#[derive(Clone, Debug)]
struct DedupeQualityArgs {
    dataset: PathBuf,
    private_business_labeled: bool,
    dataset_manifest_sha256: Option<String>,
    annotation_manifest_sha256: Option<String>,
}

#[derive(Clone, Debug)]
struct DedupeGateArgs {
    report: PathBuf,
    require_private_business_labeled: bool,
    min_pairs: usize,
    min_positive_pairs: usize,
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
struct PrivateOcrThroughputArgs {
    root: PathBuf,
    command: Option<PathBuf>,
    tesseract_command: Option<PathBuf>,
    renderer_command: Option<PathBuf>,
    pdftoppm_command: Option<PathBuf>,
    max_documents: usize,
    max_pages: usize,
    pages_per_document: usize,
    page_timeout_ms: u64,
    max_run_ms: Option<u64>,
    render_dpi: u32,
    ocr_lang: String,
    engine_profile: String,
    dataset_manifest_sha256: String,
    ocr_runtime_manifest_sha256: String,
    renderer_manifest_sha256: String,
    language_pack_manifest_sha256: String,
}

#[derive(Clone, Debug)]
struct OcrGateArgs {
    report: PathBuf,
    allow_synthetic: bool,
    require_private_real_corpus: bool,
    current_stage_baseline: bool,
    min_pages: usize,
    max_p95_ms: f64,
    min_pages_per_second: f64,
}

#[derive(Clone, Debug)]
struct VectorQualityArgs {
    dataset: PathBuf,
    command: PathBuf,
    model_id: String,
    dimension: usize,
    private_business_labeled: bool,
    dataset_manifest_sha256: Option<String>,
    annotation_manifest_sha256: Option<String>,
    model_manifest_sha256: Option<String>,
    top_k: usize,
    timeout_ms: u64,
    max_text_bytes: usize,
}

#[derive(Clone, Debug)]
struct VectorGateArgs {
    report: PathBuf,
    require_private_business_labeled: bool,
    min_samples: usize,
    min_recall_at_k: f64,
    min_mrr: f64,
    min_ndcg_at_k: f64,
    max_zero_recall_queries: usize,
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
