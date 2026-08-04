use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

const DESKTOP_IMPORT_RESCAN_MIN_AGE_SECONDS: i64 = 300;
#[cfg(any(not(debug_assertions), test))]
const PACKAGED_EMBEDDING_MODEL_ID: &str = "intfloat-multilingual-e5-small-qint8-r1";
#[cfg(any(not(debug_assertions), test))]
const PACKAGED_COREML_MODEL_ID: &str = "intfloat-multilingual-e5-small-coreml-fp16-r1";
#[cfg(any(not(debug_assertions), test))]
const PACKAGED_EMBEDDING_DIMENSION: usize = 384;
const PACKAGED_OCR_LANG: &str = "eng+chi_sim";
const OCR_JOBS_PER_TICK: usize = 1;
const DAEMON_IPC_PROTOCOL: &str = "resume-ir.daemon-ipc.v5";
const CANDIDATE_PATH_MAX_BYTES: usize = 4096;
const MODEL_ID_MAX_BYTES: usize = 128;

/// Candidate runtime coordinates passed to the daemon for authoritative
/// validation after its control plane has been published.
pub(super) struct EmbeddingRuntime {
    command: PathBuf,
    model_id: String,
    dimension: usize,
    resource_dir: Option<PathBuf>,
    coreml_worker: Option<PathBuf>,
    coreml_runtime_dir: Option<PathBuf>,
}

impl EmbeddingRuntime {
    pub(super) fn configure_command(&self, command: &mut Command) {
        command
            .env("TRANSFORMERS_OFFLINE", "1")
            .env("HF_HUB_OFFLINE", "1");
        if let Some(resource_dir) = &self.resource_dir {
            command.env("RESUME_IR_EMBEDDING_RUNTIME_DIR", resource_dir);
        }
        if let (Some(worker), Some(runtime_dir)) = (&self.coreml_worker, &self.coreml_runtime_dir) {
            command
                .env("RESUME_IR_COREML_WORKER_BIN", worker)
                .env("RESUME_IR_COREML_RUNTIME_DIR", runtime_dir);
        }
    }
}

/// Candidate OCR coordinates. Existence, digest, executable bits, and model
/// contents are deliberately validated by the daemon bootstrap worker.
pub(super) struct OcrRuntime {
    engine_command: PathBuf,
    tessdata_dir: PathBuf,
}

impl OcrRuntime {
    pub(super) fn configure_command(&self, command: &mut Command) {
        command
            .env("TESSDATA_PREFIX", &self.tessdata_dir)
            .env("OMP_THREAD_LIMIT", "1");
    }
}

pub(super) struct PdfiumRuntime {
    renderer_command: PathBuf,
    resource_dir: PathBuf,
}

impl PdfiumRuntime {
    pub(super) fn configure_command(&self, command: &mut Command) {
        command.env("RESUME_IR_PDFIUM_RUNTIME_DIR", &self.resource_dir);
    }
}

pub(super) struct ClassifierRuntime {
    model_path: PathBuf,
}

#[cfg(debug_assertions)]
pub(super) fn configured_embedding_runtime(
    _current_exe: &Path,
    _resource_dir: &Path,
) -> Option<EmbeddingRuntime> {
    let command = bounded_env_path("RESUME_IR_EMBEDDING_COMMAND")?;
    let model_id = std::env::var("RESUME_IR_EMBEDDING_MODEL_ID")
        .ok()
        .filter(|value| !value.is_empty() && value.len() <= MODEL_ID_MAX_BYTES)?;
    let dimension = std::env::var("RESUME_IR_EMBEDDING_DIMENSION")
        .ok()?
        .parse::<usize>()
        .ok()
        .filter(|value| (1..=4096).contains(value))?;
    Some(EmbeddingRuntime {
        command,
        model_id,
        dimension,
        resource_dir: None,
        coreml_worker: bounded_env_path("RESUME_IR_COREML_WORKER_BIN"),
        coreml_runtime_dir: bounded_env_path("RESUME_IR_COREML_RUNTIME_DIR"),
    })
}

#[cfg(not(debug_assertions))]
pub(super) fn configured_embedding_runtime(
    current_exe: &Path,
    resource_dir: &Path,
) -> Option<EmbeddingRuntime> {
    let coreml = cfg!(all(
        target_os = "macos",
        target_arch = "aarch64",
        not(feature = "macos-onnx-edition")
    ));
    let binary_dir = current_exe.parent()?;
    Some(EmbeddingRuntime {
        command: binary_dir.join(embedding_binary_name()),
        model_id: if coreml {
            PACKAGED_COREML_MODEL_ID
        } else {
            PACKAGED_EMBEDDING_MODEL_ID
        }
        .to_string(),
        dimension: PACKAGED_EMBEDDING_DIMENSION,
        resource_dir: Some(resource_dir.to_path_buf()),
        coreml_worker: coreml.then(|| binary_dir.join(coreml_worker_name())),
        coreml_runtime_dir: coreml.then(|| resource_dir.join("coreml")),
    })
}

#[cfg(debug_assertions)]
pub(super) fn configured_ocr_runtime(
    _current_exe: &Path,
    _resource_dir: &Path,
) -> Option<OcrRuntime> {
    None
}

#[cfg(not(debug_assertions))]
pub(super) fn configured_ocr_runtime(
    _current_exe: &Path,
    resource_dir: &Path,
) -> Option<OcrRuntime> {
    Some(OcrRuntime {
        engine_command: resource_dir.join("tesseract"),
        tessdata_dir: resource_dir.join("tessdata"),
    })
}

#[cfg(debug_assertions)]
pub(super) fn configured_pdfium_runtime(
    _current_exe: &Path,
    resource_dir: &Path,
) -> Option<PdfiumRuntime> {
    Some(PdfiumRuntime {
        renderer_command: bounded_env_path("RESUME_IR_PDF_RENDER_COMMAND")?,
        resource_dir: resource_dir.to_path_buf(),
    })
}

#[cfg(not(debug_assertions))]
pub(super) fn configured_pdfium_runtime(
    current_exe: &Path,
    resource_dir: &Path,
) -> Option<PdfiumRuntime> {
    Some(PdfiumRuntime {
        renderer_command: current_exe.parent()?.join(pdf_renderer_binary_name()),
        resource_dir: resource_dir.to_path_buf(),
    })
}

#[cfg(debug_assertions)]
pub(super) fn configured_classifier_runtime(_resource_dir: &Path) -> Option<ClassifierRuntime> {
    None
}

#[cfg(not(debug_assertions))]
pub(super) fn configured_classifier_runtime(resource_dir: &Path) -> Option<ClassifierRuntime> {
    Some(ClassifierRuntime {
        model_path: resource_dir.join("linear-promotion-model.json"),
    })
}

pub(super) fn daemon_arguments(
    data_dir: &Path,
    launch_id: &str,
    embedding: Option<&EmbeddingRuntime>,
    ocr: Option<&OcrRuntime>,
    classifier: Option<&ClassifierRuntime>,
    pdfium: Option<&PdfiumRuntime>,
) -> Vec<OsString> {
    let mut arguments: Vec<OsString> = [
        OsString::from("--data-dir"),
        data_dir.as_os_str().to_os_string(),
        OsString::from("run"),
        OsString::from("--foreground"),
        OsString::from("--parent-lifecycle-stdin"),
        OsString::from("--launch-id"),
        OsString::from(launch_id),
        OsString::from("--work-imports"),
        OsString::from("--work-index"),
        OsString::from("--rescan-completed-imports"),
        OsString::from("--watch-import-roots"),
        OsString::from("--import-rescan-min-age-seconds"),
        OsString::from(DESKTOP_IMPORT_RESCAN_MIN_AGE_SECONDS.to_string()),
        OsString::from("--expected-ipc-protocol"),
        OsString::from(DAEMON_IPC_PROTOCOL),
        OsString::from("--ipc-listen"),
        OsString::from("127.0.0.1:0"),
    ]
    .into_iter()
    .collect();
    if let Some(embedding) = embedding {
        arguments.extend([
            OsString::from("--embedding-command"),
            embedding.command.as_os_str().to_os_string(),
            OsString::from("--embedding-model-id"),
            OsString::from(&embedding.model_id),
            OsString::from("--embedding-dimension"),
            OsString::from(embedding.dimension.to_string()),
        ]);
    }
    if let Some(ocr) = ocr {
        arguments.extend([
            OsString::from("--work-ocr"),
            OsString::from("--ocr-tesseract-command"),
            ocr.engine_command.as_os_str().to_os_string(),
            OsString::from("--ocr-lang"),
            OsString::from(PACKAGED_OCR_LANG),
            OsString::from("--ocr-jobs-per-tick"),
            OsString::from(OCR_JOBS_PER_TICK.to_string()),
        ]);
    }
    if let Some(pdfium) = pdfium {
        arguments.extend([
            OsString::from("--pdf-render-command"),
            pdfium.renderer_command.as_os_str().to_os_string(),
        ]);
    }
    if let Some(classifier) = classifier {
        arguments.extend([
            OsString::from("--resume-classifier-model"),
            classifier.model_path.as_os_str().to_os_string(),
        ]);
    }
    arguments
}

#[cfg(debug_assertions)]
fn bounded_env_path(name: &str) -> Option<PathBuf> {
    let value = std::env::var_os(name)?;
    let path = PathBuf::from(value);
    let encoded = path.to_str()?;
    (!encoded.is_empty() && encoded.len() <= CANDIDATE_PATH_MAX_BYTES).then_some(path)
}

#[cfg(not(debug_assertions))]
fn embedding_binary_name() -> &'static str {
    if cfg!(windows) {
        "resume-embedding-runtime.exe"
    } else {
        "resume-embedding-runtime"
    }
}

#[cfg(not(debug_assertions))]
fn coreml_worker_name() -> &'static str {
    "resume-coreml-embedding-worker"
}

#[cfg(not(debug_assertions))]
fn pdf_renderer_binary_name() -> &'static str {
    if cfg!(windows) {
        "resume-pdf-render-runtime.exe"
    } else {
        "resume-pdf-render-runtime"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAUNCH_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn daemon_command_is_bounded_to_owned_launch_and_loopback_control_plane() {
        let arguments = daemon_arguments(
            Path::new("synthetic-data"),
            LAUNCH_ID,
            None,
            None,
            None,
            None,
        );
        assert_eq!(
            arguments,
            [
                "--data-dir",
                "synthetic-data",
                "run",
                "--foreground",
                "--parent-lifecycle-stdin",
                "--launch-id",
                LAUNCH_ID,
                "--work-imports",
                "--work-index",
                "--rescan-completed-imports",
                "--watch-import-roots",
                "--import-rescan-min-age-seconds",
                "300",
                "--expected-ipc-protocol",
                "resume-ir.daemon-ipc.v5",
                "--ipc-listen",
                "127.0.0.1:0",
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn candidate_coordinates_are_forwarded_without_desktop_pack_validation() {
        let embedding = EmbeddingRuntime {
            command: PathBuf::from("/missing/embedding-runtime"),
            model_id: PACKAGED_EMBEDDING_MODEL_ID.to_string(),
            dimension: PACKAGED_EMBEDDING_DIMENSION,
            resource_dir: Some(PathBuf::from("/tampered/runtime-pack")),
            coreml_worker: None,
            coreml_runtime_dir: None,
        };
        let ocr = OcrRuntime {
            engine_command: PathBuf::from("/missing/tesseract"),
            tessdata_dir: PathBuf::from("/tampered/tessdata"),
        };
        let pdfium = PdfiumRuntime {
            renderer_command: PathBuf::from("/missing/renderer"),
            resource_dir: PathBuf::from("/tampered/pdfium-runtime-pack"),
        };
        let classifier = ClassifierRuntime {
            model_path: PathBuf::from("/tampered/classifier.json"),
        };
        let arguments = daemon_arguments(
            Path::new("synthetic-data"),
            LAUNCH_ID,
            Some(&embedding),
            Some(&ocr),
            Some(&classifier),
            Some(&pdfium),
        );
        assert!(arguments.contains(&OsString::from("/missing/embedding-runtime")));
        assert!(arguments.contains(&OsString::from("/missing/tesseract")));
        assert!(arguments.contains(&OsString::from("/tampered/classifier.json")));
    }

    #[test]
    fn coreml_default_coordinates_are_injected_into_the_daemon_environment() {
        let embedding = EmbeddingRuntime {
            command: PathBuf::from("/bundle/resume-embedding-runtime"),
            model_id: PACKAGED_COREML_MODEL_ID.to_string(),
            dimension: PACKAGED_EMBEDDING_DIMENSION,
            resource_dir: Some(PathBuf::from("/bundle/embedding/runtime-pack")),
            coreml_worker: Some(PathBuf::from("/bundle/resume-coreml-embedding-worker")),
            coreml_runtime_dir: Some(PathBuf::from("/bundle/embedding/runtime-pack/coreml")),
        };
        let mut command = Command::new("/bundle/resume-daemon");
        embedding.configure_command(&mut command);
        let environment = command
            .get_envs()
            .map(|(name, value)| (name.to_owned(), value.map(OsString::from)))
            .collect::<std::collections::BTreeMap<_, _>>();

        assert_eq!(
            environment.get(std::ffi::OsStr::new("RESUME_IR_COREML_WORKER_BIN")),
            Some(&Some(OsString::from("/bundle/resume-coreml-embedding-worker")))
        );
        assert_eq!(
            environment.get(std::ffi::OsStr::new("RESUME_IR_COREML_RUNTIME_DIR")),
            Some(&Some(OsString::from("/bundle/embedding/runtime-pack/coreml")))
        );
    }
}
