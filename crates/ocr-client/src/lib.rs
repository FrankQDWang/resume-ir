pub fn crate_name() -> &'static str {
    "ocr-client"
}

use std::{
    fmt,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::{
    fs::{DirBuilderExt, OpenOptionsExt},
    process::CommandExt,
};

const OCR_OUTPUT_MAX_BYTES: usize = 4 * 1024 * 1024;
const OCR_RENDER_OUTPUT_MAX_BYTES: usize = 32 * 1024 * 1024;
const OCR_POLL_INTERVAL_MS: u64 = 10;

pub trait OcrClient {
    fn recognize_page(
        &self,
        request: OcrPageRequest,
        budget: OcrWorkerBudget,
        cancellation: &CancellationToken,
    ) -> Result<OcrPage, OcrError>;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisabledOcrWorkerClient;

impl OcrClient for DisabledOcrWorkerClient {
    fn recognize_page(
        &self,
        _request: OcrPageRequest,
        _budget: OcrWorkerBudget,
        cancellation: &CancellationToken,
    ) -> Result<OcrPage, OcrError> {
        if cancellation.is_cancelled() {
            return Err(OcrError::new(OcrErrorKind::Cancelled));
        }

        Err(OcrError::new(OcrErrorKind::Disabled))
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LocalOcrCommandSpec {
    program: PathBuf,
    args: Vec<String>,
    engine_profile: String,
}

impl LocalOcrCommandSpec {
    pub fn new<I, S>(
        program: impl Into<PathBuf>,
        args: I,
        engine_profile: impl Into<String>,
    ) -> Result<Self, OcrError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let program = program.into();
        let engine_profile = engine_profile.into();
        if program.as_os_str().is_empty() || engine_profile.trim().is_empty() {
            return Err(OcrError::new(OcrErrorKind::InvalidRequest));
        }

        Ok(Self {
            program,
            args: args.into_iter().map(Into::into).collect(),
            engine_profile,
        })
    }

    pub fn engine_profile(&self) -> &str {
        &self.engine_profile
    }
}

impl fmt::Debug for LocalOcrCommandSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalOcrCommandSpec")
            .field("program", &"<redacted>")
            .field("args_count", &self.args.len())
            .field("engine_profile", &self.engine_profile)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalOcrCommandClient {
    spec: LocalOcrCommandSpec,
}

impl LocalOcrCommandClient {
    pub fn new(spec: LocalOcrCommandSpec) -> Self {
        Self { spec }
    }
}

impl OcrClient for LocalOcrCommandClient {
    fn recognize_page(
        &self,
        request: OcrPageRequest,
        budget: OcrWorkerBudget,
        cancellation: &CancellationToken,
    ) -> Result<OcrPage, OcrError> {
        if cancellation.is_cancelled() {
            return Err(OcrError::new(OcrErrorKind::Cancelled));
        }

        let input = OcrTempInput::write(request.page().bytes())?;
        let started_at = Instant::now();
        let mut child = spawn_ocr_command(&self.spec, &request, input.path())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| OcrError::new(OcrErrorKind::EngineFailed))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| OcrError::new(OcrErrorKind::EngineFailed))?;
        let stdout_reader = spawn_output_reader(stdout, OCR_OUTPUT_MAX_BYTES);
        let stderr_reader = spawn_output_reader(stderr, OCR_OUTPUT_MAX_BYTES);

        let status = match wait_for_ocr_child(&mut child, budget, cancellation) {
            Ok(status) => status,
            Err(error) => {
                let _ = join_output_reader(stdout_reader);
                let _ = join_output_reader(stderr_reader);
                return Err(error);
            }
        };
        #[cfg(unix)]
        terminate_process_group(child.id());
        let stdout = join_output_reader(stdout_reader)?;
        let _stderr = join_output_reader(stderr_reader)?;
        if !status.success() {
            return Err(OcrError::new(OcrErrorKind::EngineFailed));
        }

        parse_ocr_output(
            request.page().page_no(),
            &stdout,
            self.spec.engine_profile(),
            elapsed_millis(started_at),
        )
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct TesseractOcrSpec {
    program: PathBuf,
    engine_profile: String,
    page_segmentation_mode: u8,
}

impl TesseractOcrSpec {
    pub fn new(
        program: impl Into<PathBuf>,
        engine_profile: impl Into<String>,
    ) -> Result<Self, OcrError> {
        let program = program.into();
        let engine_profile = engine_profile.into();
        if program.as_os_str().is_empty() || engine_profile.trim().is_empty() {
            return Err(OcrError::new(OcrErrorKind::InvalidRequest));
        }

        Ok(Self {
            program,
            engine_profile,
            page_segmentation_mode: 6,
        })
    }

    pub fn engine_profile(&self) -> &str {
        &self.engine_profile
    }
}

impl fmt::Debug for TesseractOcrSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TesseractOcrSpec")
            .field("program", &"<redacted>")
            .field("engine_profile", &self.engine_profile)
            .field("page_segmentation_mode", &self.page_segmentation_mode)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TesseractOcrClient {
    spec: TesseractOcrSpec,
}

impl TesseractOcrClient {
    pub fn new(spec: TesseractOcrSpec) -> Self {
        Self { spec }
    }
}

impl OcrClient for TesseractOcrClient {
    fn recognize_page(
        &self,
        request: OcrPageRequest,
        budget: OcrWorkerBudget,
        cancellation: &CancellationToken,
    ) -> Result<OcrPage, OcrError> {
        if cancellation.is_cancelled() {
            return Err(OcrError::new(OcrErrorKind::Cancelled));
        }

        let input = OcrTempInput::write_named(request.page().bytes(), "page-image.ppm")?;
        let started_at = Instant::now();
        let mut child = spawn_tesseract_command(&self.spec, &request, input.path())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| OcrError::new(OcrErrorKind::EngineFailed))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| OcrError::new(OcrErrorKind::EngineFailed))?;
        let stdout_reader = spawn_output_reader(stdout, OCR_OUTPUT_MAX_BYTES);
        let stderr_reader = spawn_output_reader(stderr, OCR_OUTPUT_MAX_BYTES);

        let status = match wait_for_ocr_child(&mut child, budget, cancellation) {
            Ok(status) => status,
            Err(error) => {
                let _ = join_output_reader(stdout_reader);
                let _ = join_output_reader(stderr_reader);
                return Err(error);
            }
        };
        #[cfg(unix)]
        terminate_process_group(child.id());
        let stdout = join_output_reader(stdout_reader)?;
        let _stderr = join_output_reader(stderr_reader)?;
        if !status.success() {
            return Err(OcrError::new(OcrErrorKind::EngineFailed));
        }

        parse_tesseract_tsv(
            request.page().page_no(),
            &stdout,
            self.spec.engine_profile(),
            elapsed_millis(started_at),
        )
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LocalPdfRenderCommandSpec {
    program: PathBuf,
    args: Vec<String>,
}

impl LocalPdfRenderCommandSpec {
    pub fn new<I, S>(program: impl Into<PathBuf>, args: I) -> Result<Self, OcrError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let program = program.into();
        if program.as_os_str().is_empty() {
            return Err(OcrError::new(OcrErrorKind::InvalidRequest));
        }

        Ok(Self {
            program,
            args: args.into_iter().map(Into::into).collect(),
        })
    }
}

impl fmt::Debug for LocalPdfRenderCommandSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalPdfRenderCommandSpec")
            .field("program", &"<redacted>")
            .field("args_count", &self.args.len())
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalPdfRenderCommandClient {
    spec: LocalPdfRenderCommandSpec,
}

impl LocalPdfRenderCommandClient {
    pub fn new(spec: LocalPdfRenderCommandSpec) -> Self {
        Self { spec }
    }

    pub fn render_page(
        &self,
        document_bytes: &[u8],
        page_no: u32,
        render_dpi: u32,
        budget: OcrWorkerBudget,
        cancellation: &CancellationToken,
    ) -> Result<RenderedPage, OcrError> {
        if cancellation.is_cancelled() {
            return Err(OcrError::new(OcrErrorKind::Cancelled));
        }

        let input = OcrTempInput::write(document_bytes)?;
        let mut child = spawn_pdf_render_command(&self.spec, page_no, render_dpi, input.path())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| OcrError::new(OcrErrorKind::EngineFailed))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| OcrError::new(OcrErrorKind::EngineFailed))?;
        let stdout_reader = spawn_output_reader(stdout, OCR_RENDER_OUTPUT_MAX_BYTES);
        let stderr_reader = spawn_output_reader(stderr, OCR_OUTPUT_MAX_BYTES);

        let status = match wait_for_ocr_child(&mut child, budget, cancellation) {
            Ok(status) => status,
            Err(error) => {
                let _ = join_output_reader(stdout_reader);
                let _ = join_output_reader(stderr_reader);
                return Err(error);
            }
        };
        #[cfg(unix)]
        terminate_process_group(child.id());
        let page_bytes = join_output_reader(stdout_reader)?;
        let _stderr = join_output_reader(stderr_reader)?;
        if !status.success() || page_bytes.is_empty() {
            return Err(OcrError::new(OcrErrorKind::EngineFailed));
        }

        RenderedPage::new(page_no, render_dpi, page_bytes)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct PdftoppmRenderSpec {
    program: PathBuf,
}

impl PdftoppmRenderSpec {
    pub fn new(program: impl Into<PathBuf>) -> Result<Self, OcrError> {
        let program = program.into();
        if program.as_os_str().is_empty() {
            return Err(OcrError::new(OcrErrorKind::InvalidRequest));
        }

        Ok(Self { program })
    }
}

impl fmt::Debug for PdftoppmRenderSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PdftoppmRenderSpec")
            .field("program", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdftoppmPdfRenderer {
    spec: PdftoppmRenderSpec,
}

impl PdftoppmPdfRenderer {
    pub fn new(spec: PdftoppmRenderSpec) -> Self {
        Self { spec }
    }

    pub fn render_page(
        &self,
        document_bytes: &[u8],
        page_no: u32,
        render_dpi: u32,
        budget: OcrWorkerBudget,
        cancellation: &CancellationToken,
    ) -> Result<RenderedPage, OcrError> {
        if page_no == 0 || render_dpi == 0 {
            return Err(OcrError::new(OcrErrorKind::InvalidRequest));
        }
        if cancellation.is_cancelled() {
            return Err(OcrError::new(OcrErrorKind::Cancelled));
        }

        let input = OcrTempInput::write(document_bytes)?;
        let output = OcrTempOutputPrefix::new()?;
        let mut child = spawn_pdftoppm_command(
            &self.spec,
            page_no,
            render_dpi,
            input.path(),
            output.prefix(),
        )?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| OcrError::new(OcrErrorKind::EngineFailed))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| OcrError::new(OcrErrorKind::EngineFailed))?;
        let stdout_reader = spawn_output_reader(stdout, OCR_OUTPUT_MAX_BYTES);
        let stderr_reader = spawn_output_reader(stderr, OCR_OUTPUT_MAX_BYTES);

        let status = match wait_for_ocr_child(&mut child, budget, cancellation) {
            Ok(status) => status,
            Err(error) => {
                let _ = join_output_reader(stdout_reader);
                let _ = join_output_reader(stderr_reader);
                return Err(error);
            }
        };
        #[cfg(unix)]
        terminate_process_group(child.id());
        let _stdout = join_output_reader(stdout_reader)?;
        let _stderr = join_output_reader(stderr_reader)?;
        if !status.success() {
            return Err(OcrError::new(OcrErrorKind::EngineFailed));
        }

        let page_bytes = read_rendered_ppm(&output.ppm_path())?;
        RenderedPage::new(page_no, render_dpi, page_bytes)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct OcrCacheKey {
    file_content_hash: String,
    page_no: u32,
    render_dpi: u32,
    ocr_lang: String,
    ocr_profile: String,
}

impl OcrCacheKey {
    pub fn new(
        file_content_hash: impl Into<String>,
        page_no: u32,
        render_dpi: u32,
        ocr_lang: impl Into<String>,
        ocr_profile: impl Into<String>,
    ) -> Result<Self, OcrError> {
        let file_content_hash = file_content_hash.into();
        let ocr_lang = ocr_lang.into();
        let ocr_profile = ocr_profile.into();
        if file_content_hash.trim().is_empty()
            || page_no == 0
            || render_dpi == 0
            || ocr_lang.trim().is_empty()
            || ocr_profile.trim().is_empty()
        {
            return Err(OcrError::new(OcrErrorKind::InvalidRequest));
        }

        Ok(Self {
            file_content_hash,
            page_no,
            render_dpi,
            ocr_lang,
            ocr_profile,
        })
    }

    pub fn page_no(&self) -> u32 {
        self.page_no
    }

    pub fn render_dpi(&self) -> u32 {
        self.render_dpi
    }

    pub fn ocr_lang(&self) -> &str {
        &self.ocr_lang
    }

    pub fn ocr_profile(&self) -> &str {
        &self.ocr_profile
    }
}

impl fmt::Debug for OcrCacheKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OcrCacheKey")
            .field("file_content_hash", &"<redacted>")
            .field("page_no", &self.page_no)
            .field("render_dpi", &self.render_dpi)
            .field("ocr_lang", &self.ocr_lang)
            .field("ocr_profile", &self.ocr_profile)
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub struct OcrPage {
    page_no: u32,
    text: String,
    confidence: f32,
    engine_profile: String,
    duration_ms: u64,
}

impl OcrPage {
    pub fn new(
        page_no: u32,
        text: impl Into<String>,
        confidence: f32,
        engine_profile: impl Into<String>,
        duration_ms: u64,
    ) -> Result<Self, OcrError> {
        if page_no == 0 || !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
            return Err(OcrError::new(OcrErrorKind::InvalidRequest));
        }

        Ok(Self {
            page_no,
            text: text.into(),
            confidence: confidence.clamp(0.0, 1.0),
            engine_profile: engine_profile.into(),
            duration_ms,
        })
    }

    pub fn page_no(&self) -> u32 {
        self.page_no
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn confidence(&self) -> f32 {
        self.confidence
    }

    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    pub fn engine_profile(&self) -> &str {
        &self.engine_profile
    }
}

impl fmt::Debug for OcrPage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OcrPage")
            .field("page_no", &self.page_no)
            .field("text", &"<redacted>")
            .field("text_bytes", &self.text.len())
            .field("confidence", &self.confidence)
            .field("engine_profile", &self.engine_profile)
            .field("duration_ms", &self.duration_ms)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct RenderedPage {
    page_no: u32,
    render_dpi: u32,
    bytes: Vec<u8>,
}

impl RenderedPage {
    pub fn new(page_no: u32, render_dpi: u32, bytes: Vec<u8>) -> Result<Self, OcrError> {
        if page_no == 0 || render_dpi == 0 || bytes.is_empty() {
            return Err(OcrError::new(OcrErrorKind::InvalidRequest));
        }

        Ok(Self {
            page_no,
            render_dpi,
            bytes,
        })
    }

    pub fn page_no(&self) -> u32 {
        self.page_no
    }

    pub fn render_dpi(&self) -> u32 {
        self.render_dpi
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl fmt::Debug for RenderedPage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RenderedPage")
            .field("page_no", &self.page_no)
            .field("render_dpi", &self.render_dpi)
            .field("bytes", &"<redacted>")
            .field("byte_len", &self.bytes.len())
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OcrOptions {
    lang: String,
    profile: String,
}

impl OcrOptions {
    pub fn new(lang: impl Into<String>, profile: impl Into<String>) -> Result<Self, OcrError> {
        let lang = lang.into();
        let profile = profile.into();
        if lang.trim().is_empty() || profile.trim().is_empty() {
            return Err(OcrError::new(OcrErrorKind::InvalidRequest));
        }

        Ok(Self { lang, profile })
    }

    pub fn lang(&self) -> &str {
        &self.lang
    }

    pub fn profile(&self) -> &str {
        &self.profile
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OcrPageRequest {
    page: RenderedPage,
    options: OcrOptions,
}

impl OcrPageRequest {
    pub fn new(page: RenderedPage, options: OcrOptions) -> Result<Self, OcrError> {
        Ok(Self { page, options })
    }

    pub fn page(&self) -> &RenderedPage {
        &self.page
    }

    pub fn options(&self) -> &OcrOptions {
        &self.options
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OcrWorkerBudget {
    page_timeout_ms: u64,
}

impl OcrWorkerBudget {
    pub fn new(page_timeout_ms: u64) -> Result<Self, OcrError> {
        if page_timeout_ms == 0 {
            return Err(OcrError::new(OcrErrorKind::InvalidRequest));
        }

        Ok(Self { page_timeout_ms })
    }

    pub fn page_timeout_ms(self) -> u64 {
        self.page_timeout_ms
    }
}

#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn new_cancelled() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

impl PartialEq for CancellationToken {
    fn eq(&self, other: &Self) -> bool {
        self.is_cancelled() == other.is_cancelled()
    }
}

impl Eq for CancellationToken {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrErrorKind {
    Disabled,
    Cancelled,
    Timeout,
    InvalidRequest,
    WorkerUnavailable,
    EngineFailed,
}

#[derive(Clone, PartialEq, Eq)]
pub struct OcrError {
    kind: OcrErrorKind,
}

impl OcrError {
    pub fn new(kind: OcrErrorKind) -> Self {
        Self { kind }
    }

    pub fn kind(&self) -> OcrErrorKind {
        self.kind
    }
}

impl fmt::Debug for OcrError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OcrError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for OcrError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            OcrErrorKind::Disabled => formatter.write_str("OCR worker is disabled"),
            OcrErrorKind::Cancelled => formatter.write_str("OCR request was cancelled"),
            OcrErrorKind::Timeout => formatter.write_str("OCR request timed out"),
            OcrErrorKind::InvalidRequest => formatter.write_str("OCR request is invalid"),
            OcrErrorKind::WorkerUnavailable => formatter.write_str("OCR worker is unavailable"),
            OcrErrorKind::EngineFailed => formatter.write_str("OCR engine failed"),
        }
    }
}

impl std::error::Error for OcrError {}

fn spawn_ocr_command(
    spec: &LocalOcrCommandSpec,
    request: &OcrPageRequest,
    input_path: &Path,
) -> Result<Child, OcrError> {
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .env("RESUME_IR_OCR_INPUT_PATH", input_path.as_os_str())
        .env(
            "RESUME_IR_OCR_PAGE_NO",
            request.page().page_no().to_string(),
        )
        .env(
            "RESUME_IR_OCR_RENDER_DPI",
            request.page().render_dpi().to_string(),
        )
        .env("RESUME_IR_OCR_LANG", request.options().lang())
        .env("RESUME_IR_OCR_PROFILE", request.options().profile())
        .env("RESUME_IR_OCR_ENGINE_PROFILE", spec.engine_profile())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_process_isolation(&mut command);

    command.spawn().map_err(|error| match error.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied => {
            OcrError::new(OcrErrorKind::WorkerUnavailable)
        }
        _ => OcrError::new(OcrErrorKind::EngineFailed),
    })
}

fn spawn_tesseract_command(
    spec: &TesseractOcrSpec,
    request: &OcrPageRequest,
    input_path: &Path,
) -> Result<Child, OcrError> {
    let mut command = Command::new(&spec.program);
    command
        .arg(input_path.as_os_str())
        .arg("stdout")
        .arg("--psm")
        .arg(spec.page_segmentation_mode.to_string())
        .arg("-l")
        .arg(request.options().lang())
        .arg("tsv")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_process_isolation(&mut command);

    command.spawn().map_err(|error| match error.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied => {
            OcrError::new(OcrErrorKind::WorkerUnavailable)
        }
        _ => OcrError::new(OcrErrorKind::EngineFailed),
    })
}

fn spawn_pdf_render_command(
    spec: &LocalPdfRenderCommandSpec,
    page_no: u32,
    render_dpi: u32,
    input_path: &Path,
) -> Result<Child, OcrError> {
    if page_no == 0 || render_dpi == 0 {
        return Err(OcrError::new(OcrErrorKind::InvalidRequest));
    }

    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .env("RESUME_IR_PDF_RENDER_INPUT_PATH", input_path.as_os_str())
        .env("RESUME_IR_PDF_RENDER_PAGE_NO", page_no.to_string())
        .env("RESUME_IR_PDF_RENDER_DPI", render_dpi.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_process_isolation(&mut command);

    command.spawn().map_err(|error| match error.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied => {
            OcrError::new(OcrErrorKind::WorkerUnavailable)
        }
        _ => OcrError::new(OcrErrorKind::EngineFailed),
    })
}

fn spawn_pdftoppm_command(
    spec: &PdftoppmRenderSpec,
    page_no: u32,
    render_dpi: u32,
    input_path: &Path,
    output_prefix: &Path,
) -> Result<Child, OcrError> {
    let mut command = Command::new(&spec.program);
    command
        .arg("-q")
        .arg("-f")
        .arg(page_no.to_string())
        .arg("-l")
        .arg(page_no.to_string())
        .arg("-r")
        .arg(render_dpi.to_string())
        .arg("-singlefile")
        .arg(input_path.as_os_str())
        .arg(output_prefix.as_os_str())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_process_isolation(&mut command);

    command.spawn().map_err(|error| match error.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied => {
            OcrError::new(OcrErrorKind::WorkerUnavailable)
        }
        _ => OcrError::new(OcrErrorKind::EngineFailed),
    })
}

#[cfg(unix)]
fn configure_process_isolation(command: &mut Command) {
    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_process_isolation(_command: &mut Command) {}

fn wait_for_ocr_child(
    child: &mut Child,
    budget: OcrWorkerBudget,
    cancellation: &CancellationToken,
) -> Result<std::process::ExitStatus, OcrError> {
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(budget.page_timeout_ms()))
        .ok_or_else(|| OcrError::new(OcrErrorKind::InvalidRequest))?;
    loop {
        if cancellation.is_cancelled() {
            terminate_child(child);
            return Err(OcrError::new(OcrErrorKind::Cancelled));
        }

        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(_) => {
                terminate_child(child);
                return Err(OcrError::new(OcrErrorKind::EngineFailed));
            }
        }

        let now = Instant::now();
        if now >= deadline {
            terminate_child(child);
            return Err(OcrError::new(OcrErrorKind::Timeout));
        }

        thread::sleep(Duration::from_millis(OCR_POLL_INTERVAL_MS).min(deadline - now));
    }
}

fn terminate_child(child: &mut Child) {
    #[cfg(unix)]
    {
        let process_id = child.id();
        signal_process_group(process_id, UnixSignal::Term);
        thread::sleep(Duration::from_millis(10));
        signal_process_group(process_id, UnixSignal::Kill);
        if wait_for_child_exit(child, Duration::from_millis(100)) {
            return;
        }
    }

    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(unix)]
fn terminate_process_group(process_group_id: u32) {
    signal_process_group(process_group_id, UnixSignal::Term);
    thread::sleep(Duration::from_millis(10));
    signal_process_group(process_group_id, UnixSignal::Kill);
}

#[cfg(unix)]
#[derive(Clone, Copy)]
enum UnixSignal {
    Term,
    Kill,
}

#[cfg(unix)]
impl UnixSignal {
    fn as_kill_arg(self) -> &'static str {
        match self {
            Self::Term => "-TERM",
            Self::Kill => "-KILL",
        }
    }
}

#[cfg(unix)]
fn signal_process_group(process_group_id: u32, signal: UnixSignal) {
    let _ = Command::new("/bin/kill")
        .arg(signal.as_kill_arg())
        .arg("--")
        .arg(format!("-{process_group_id}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(unix)]
fn wait_for_child_exit(child: &mut Child, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) => {}
            Err(_) => return true,
        }

        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(OCR_POLL_INTERVAL_MS).min(deadline - now));
    }
}

fn spawn_output_reader<R>(reader: R, max_bytes: usize) -> JoinHandle<io::Result<Vec<u8>>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || read_all_limited(reader, max_bytes))
}

fn read_all_limited<R>(mut reader: R, max_bytes: usize) -> io::Result<Vec<u8>>
where
    R: Read,
{
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    let mut exceeded = false;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }

        let remaining = max_bytes.saturating_sub(output.len());
        let retained = remaining.min(read);
        output.extend_from_slice(&buffer[..retained]);
        if retained < read {
            exceeded = true;
        }
    }

    if exceeded {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "OCR output exceeded configured byte limit",
        ))
    } else {
        Ok(output)
    }
}

fn join_output_reader(handle: JoinHandle<io::Result<Vec<u8>>>) -> Result<Vec<u8>, OcrError> {
    handle
        .join()
        .map_err(|_| OcrError::new(OcrErrorKind::EngineFailed))?
        .map_err(|_| OcrError::new(OcrErrorKind::EngineFailed))
}

fn read_rendered_ppm(path: &Path) -> Result<Vec<u8>, OcrError> {
    let file = fs::File::open(path).map_err(|_| OcrError::new(OcrErrorKind::EngineFailed))?;
    let bytes = read_all_limited(file, OCR_RENDER_OUTPUT_MAX_BYTES)
        .map_err(|_| OcrError::new(OcrErrorKind::EngineFailed))?;
    if bytes.is_empty() {
        Err(OcrError::new(OcrErrorKind::EngineFailed))
    } else {
        Ok(bytes)
    }
}

fn parse_ocr_output(
    page_no: u32,
    stdout: &[u8],
    engine_profile: &str,
    duration_ms: u64,
) -> Result<OcrPage, OcrError> {
    let output = String::from_utf8(stdout.to_vec())
        .map_err(|_| OcrError::new(OcrErrorKind::EngineFailed))?;
    let output = output.replace("\r\n", "\n");
    let structured = output
        .strip_prefix("resume-ir-ocr-v1\n")
        .ok_or_else(|| OcrError::new(OcrErrorKind::EngineFailed))?;
    let (metadata, text) = structured
        .split_once("text:\n")
        .ok_or_else(|| OcrError::new(OcrErrorKind::EngineFailed))?;
    let confidence = parse_confidence(metadata)?;
    OcrPage::new(
        page_no,
        text.to_owned(),
        confidence,
        engine_profile,
        duration_ms,
    )
}

fn parse_tesseract_tsv(
    page_no: u32,
    stdout: &[u8],
    engine_profile: &str,
    duration_ms: u64,
) -> Result<OcrPage, OcrError> {
    let output = String::from_utf8(stdout.to_vec())
        .map_err(|_| OcrError::new(OcrErrorKind::EngineFailed))?
        .replace("\r\n", "\n");
    let mut words = Vec::new();
    let mut confidence_sum = 0.0_f32;
    let mut confidence_count = 0_usize;

    for line in output.lines().skip(1) {
        let columns: Vec<&str> = line.split('\t').collect();
        if columns.len() < 12 || columns[0] != "5" {
            continue;
        }
        let word = columns[11..].join("\t");
        let word = word.trim();
        if word.is_empty() {
            continue;
        }
        let confidence = columns[10]
            .parse::<f32>()
            .map_err(|_| OcrError::new(OcrErrorKind::EngineFailed))?;
        if confidence >= 0.0 {
            confidence_sum += (confidence / 100.0).clamp(0.0, 1.0);
            confidence_count += 1;
        }
        words.push(word.to_string());
    }

    let text = if words.is_empty() {
        String::new()
    } else {
        format!("{}\n", words.join(" "))
    };
    let confidence = if confidence_count == 0 {
        0.0
    } else {
        confidence_sum / confidence_count as f32
    };
    OcrPage::new(page_no, text, confidence, engine_profile, duration_ms)
}

fn parse_confidence(metadata: &str) -> Result<f32, OcrError> {
    let confidence = metadata
        .lines()
        .find_map(|line| line.strip_prefix("confidence="))
        .ok_or_else(|| OcrError::new(OcrErrorKind::EngineFailed))?
        .parse::<f32>()
        .map_err(|_| OcrError::new(OcrErrorKind::EngineFailed))?;
    if confidence.is_finite() && (0.0..=1.0).contains(&confidence) {
        Ok(confidence)
    } else {
        Err(OcrError::new(OcrErrorKind::EngineFailed))
    }
}

fn elapsed_millis(started_at: Instant) -> u64 {
    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
}

struct OcrTempInput {
    path: PathBuf,
}

impl OcrTempInput {
    fn write(bytes: &[u8]) -> Result<Self, OcrError> {
        Self::write_named(bytes, "page-image.bin")
    }

    fn write_named(bytes: &[u8], file_name: &str) -> Result<Self, OcrError> {
        for attempt in 0..32 {
            let directory = std::env::temp_dir().join(format!(
                "resume-ir-ocr-input-{}-{}-{attempt}.bin",
                std::process::id(),
                unique_nanos()
            ));
            match create_private_temp_dir(&directory) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(_) => return Err(OcrError::new(OcrErrorKind::WorkerUnavailable)),
            }

            let path = directory.join(file_name);
            match create_private_temp_file(&path) {
                Ok(mut file) => {
                    if file.write_all(bytes).is_ok() {
                        return Ok(Self { path });
                    }
                    let _ = fs::remove_dir_all(&directory);
                    return Err(OcrError::new(OcrErrorKind::WorkerUnavailable));
                }
                Err(_) => {
                    let _ = fs::remove_dir_all(&directory);
                    return Err(OcrError::new(OcrErrorKind::WorkerUnavailable));
                }
            }
        }

        Err(OcrError::new(OcrErrorKind::WorkerUnavailable))
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for OcrTempInput {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        if let Some(directory) = self.path.parent() {
            let _ = fs::remove_dir(directory);
        }
    }
}

struct OcrTempOutputPrefix {
    directory: PathBuf,
    prefix: PathBuf,
}

impl OcrTempOutputPrefix {
    fn new() -> Result<Self, OcrError> {
        for attempt in 0..32 {
            let directory = std::env::temp_dir().join(format!(
                "resume-ir-pdf-render-output-{}-{}-{attempt}",
                std::process::id(),
                unique_nanos()
            ));
            match create_private_temp_dir(&directory) {
                Ok(()) => {
                    let prefix = directory.join("page");
                    return Ok(Self { directory, prefix });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(_) => return Err(OcrError::new(OcrErrorKind::WorkerUnavailable)),
            }
        }

        Err(OcrError::new(OcrErrorKind::WorkerUnavailable))
    }

    fn prefix(&self) -> &Path {
        &self.prefix
    }

    fn ppm_path(&self) -> PathBuf {
        self.directory.join("page.ppm")
    }
}

impl Drop for OcrTempOutputPrefix {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn create_private_temp_dir(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    configure_private_dir_builder(&mut builder);
    builder.create(path)
}

#[cfg(unix)]
fn configure_private_dir_builder(builder: &mut fs::DirBuilder) {
    builder.mode(0o700);
}

#[cfg(not(unix))]
fn configure_private_dir_builder(_builder: &mut fs::DirBuilder) {}

fn create_private_temp_file(path: &Path) -> io::Result<fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    configure_private_file_options(&mut options);
    options.open(path)
}

#[cfg(unix)]
fn configure_private_file_options(options: &mut OpenOptions) {
    options.mode(0o600);
}

#[cfg(not(unix))]
fn configure_private_file_options(_options: &mut OpenOptions) {}

fn unique_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos())
}
