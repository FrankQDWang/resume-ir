//! Bounded one-page PDF rendering protocol used by the desktop OCR worker.

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;

#[cfg(all(windows, feature = "windows-static-pdfium"))]
const INPUT_MAX_BYTES: u64 = 64 * 1024 * 1024;
#[cfg(any(all(windows, feature = "windows-static-pdfium"), test))]
const OUTPUT_MAX_BYTES: usize = 32 * 1024 * 1024;
const PATH_MAX_UTF16_UNITS: usize = 32_767;
const PAGE_MAX: u32 = 512;
const DPI_MIN: u32 = 72;
const DPI_MAX: u32 = 600;
#[cfg(any(all(windows, feature = "windows-static-pdfium"), test))]
const DIMENSION_MAX: u32 = 10_000;
#[cfg(any(all(windows, feature = "windows-static-pdfium"), test))]
const PIXEL_MAX: u64 = 10_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(any(all(windows, feature = "windows-static-pdfium"), test)),
    allow(dead_code)
)]
enum RenderFailure {
    Unavailable,
    Invalid,
    Budget,
}

impl RenderFailure {
    fn exit_code(self) -> i32 {
        match self {
            Self::Unavailable => 1,
            Self::Invalid => 2,
            Self::Budget => 3,
        }
    }

    fn public_message(self) -> &'static str {
        match self {
            Self::Unavailable => "pdf-render-runtime: unavailable",
            Self::Invalid => "pdf-render-runtime: invalid request",
            Self::Budget => "pdf-render-runtime: resource limit exceeded",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct RenderRequest {
    input_path: PathBuf,
    page_no: u32,
    render_dpi: u32,
}

fn parse_bounded_integer(
    value: Option<OsString>,
    minimum: u32,
    maximum: u32,
) -> Result<u32, RenderFailure> {
    let value = value
        .and_then(|value| value.into_string().ok())
        .ok_or(RenderFailure::Invalid)?;
    if value.is_empty() || value.len() > 3 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(RenderFailure::Invalid);
    }
    let parsed = value.parse::<u32>().map_err(|_| RenderFailure::Invalid)?;
    if !(minimum..=maximum).contains(&parsed) {
        return Err(RenderFailure::Invalid);
    }
    Ok(parsed)
}

fn parse_request(
    arguments: Vec<OsString>,
    input_path: Option<OsString>,
    page_no: Option<OsString>,
    render_dpi: Option<OsString>,
) -> Result<RenderRequest, RenderFailure> {
    if arguments.len() != 1 {
        return Err(RenderFailure::Invalid);
    }
    let input_path = input_path
        .and_then(|value| value.into_string().ok())
        .filter(|value| !value.is_empty() && value.encode_utf16().count() <= PATH_MAX_UTF16_UNITS)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or(RenderFailure::Invalid)?;
    Ok(RenderRequest {
        input_path,
        page_no: parse_bounded_integer(page_no, 1, PAGE_MAX)?,
        render_dpi: parse_bounded_integer(render_dpi, DPI_MIN, DPI_MAX)?,
    })
}

#[cfg(any(all(windows, feature = "windows-static-pdfium"), test))]
fn render_dimensions(
    width_points: f32,
    height_points: f32,
    render_dpi: u32,
) -> Result<(u32, u32), RenderFailure> {
    if !width_points.is_finite()
        || !height_points.is_finite()
        || width_points <= 0.0
        || height_points <= 0.0
        || !(DPI_MIN..=DPI_MAX).contains(&render_dpi)
    {
        return Err(RenderFailure::Invalid);
    }
    let scale = f64::from(render_dpi) / 72.0;
    let ceil_dimension = |points: f32| {
        let scaled = f64::from(points) * scale;
        let nearest = scaled.round();
        if (scaled - nearest).abs() <= 0.000_001 {
            nearest
        } else {
            scaled.ceil()
        }
    };
    let width = ceil_dimension(width_points);
    let height = ceil_dimension(height_points);
    if !width.is_finite()
        || !height.is_finite()
        || width < 1.0
        || height < 1.0
        || width > f64::from(DIMENSION_MAX)
        || height > f64::from(DIMENSION_MAX)
    {
        return Err(RenderFailure::Budget);
    }
    let width = width as u32;
    let height = height as u32;
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(RenderFailure::Budget)?;
    let ppm_bytes = pixels
        .checked_mul(3)
        .and_then(|bytes| bytes.checked_add(32))
        .ok_or(RenderFailure::Budget)?;
    if pixels > PIXEL_MAX || ppm_bytes > OUTPUT_MAX_BYTES as u64 {
        return Err(RenderFailure::Budget);
    }
    Ok((width, height))
}

#[cfg(any(all(windows, feature = "windows-static-pdfium"), test))]
fn write_ppm(
    writer: &mut impl Write,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<(), RenderFailure> {
    let pixels = usize::try_from(
        u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or(RenderFailure::Budget)?,
    )
    .map_err(|_| RenderFailure::Budget)?;
    if pixels == 0
        || pixels > PIXEL_MAX as usize
        || rgba.len() != pixels.checked_mul(4).ok_or(RenderFailure::Budget)?
    {
        return Err(RenderFailure::Invalid);
    }
    write!(writer, "P6\n{width} {height}\n255\n").map_err(|_| RenderFailure::Unavailable)?;
    let mut rgb = Vec::with_capacity(4096 * 3);
    for chunk in rgba.chunks(4096 * 4) {
        rgb.clear();
        for pixel in chunk.chunks_exact(4) {
            rgb.extend_from_slice(&pixel[..3]);
        }
        writer
            .write_all(&rgb)
            .map_err(|_| RenderFailure::Unavailable)?;
    }
    Ok(())
}

#[cfg(all(windows, feature = "windows-static-pdfium"))]
fn render(request: RenderRequest, output: &mut impl Write) -> Result<(), RenderFailure> {
    use pdfium_render::prelude::{PdfRenderConfig, Pdfium};
    use std::fs::{self, File};
    use std::io::Read;

    let metadata = fs::symlink_metadata(&request.input_path).map_err(|_| RenderFailure::Invalid)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() > INPUT_MAX_BYTES
    {
        return Err(RenderFailure::Invalid);
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(&request.input_path)
        .map_err(|_| RenderFailure::Invalid)?
        .take(INPUT_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| RenderFailure::Invalid)?;
    if bytes.len() as u64 > INPUT_MAX_BYTES || !bytes.starts_with(b"%PDF-") {
        return Err(RenderFailure::Invalid);
    }

    let bindings =
        Pdfium::bind_to_statically_linked_library().map_err(|_| RenderFailure::Unavailable)?;
    let pdfium = Pdfium::new(bindings);
    let document = pdfium
        .load_pdf_from_byte_vec(bytes, None)
        .map_err(|_| RenderFailure::Invalid)?;
    let page_count = document.pages().len();
    if page_count <= 0 || page_count > PAGE_MAX as i32 || request.page_no as i32 > page_count {
        return Err(RenderFailure::Invalid);
    }
    let page = document
        .pages()
        .get(request.page_no as i32 - 1)
        .map_err(|_| RenderFailure::Invalid)?;
    let (width, height) =
        render_dimensions(page.width().value, page.height().value, request.render_dpi)?;
    let bitmap = page
        .render_with_config(&PdfRenderConfig::new().set_fixed_size(width as i32, height as i32))
        .map_err(|_| RenderFailure::Unavailable)?;
    write_ppm(output, width, height, &bitmap.as_rgba_bytes())
}

#[cfg(not(all(windows, feature = "windows-static-pdfium")))]
fn render(_request: RenderRequest, _output: &mut impl Write) -> Result<(), RenderFailure> {
    Err(RenderFailure::Unavailable)
}

/// Runs the bounded one-shot renderer and returns its stable process exit code.
pub fn run() -> i32 {
    let request = parse_request(
        std::env::args_os().collect(),
        std::env::var_os("RESUME_IR_PDF_RENDER_INPUT_PATH"),
        std::env::var_os("RESUME_IR_PDF_RENDER_PAGE_NO"),
        std::env::var_os("RESUME_IR_PDF_RENDER_DPI"),
    );
    let result = request.and_then(|request| render(request, &mut io::stdout().lock()));
    match result {
        Ok(()) => 0,
        Err(failure) => {
            let _ = writeln!(io::stderr().lock(), "{}", failure.public_message());
            failure.exit_code()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_the_bounded_one_shot_protocol() {
        let path = std::env::temp_dir().join("synthetic-document.pdf");
        assert_eq!(
            parse_request(
                vec![OsString::from("resume-pdf-render-runtime")],
                Some(path.clone().into_os_string()),
                Some(OsString::from("512")),
                Some(OsString::from("600")),
            ),
            Ok(RenderRequest {
                input_path: path,
                page_no: 512,
                render_dpi: 600,
            })
        );
        assert_eq!(
            parse_request(
                vec![OsString::from("runtime"), OsString::from("unexpected")],
                Some(std::env::temp_dir().into_os_string()),
                Some(OsString::from("1")),
                Some(OsString::from("300")),
            ),
            Err(RenderFailure::Invalid)
        );
        assert_eq!(
            parse_bounded_integer(Some(OsString::from("0")), 1, PAGE_MAX),
            Err(RenderFailure::Invalid)
        );
        assert_eq!(
            parse_bounded_integer(Some(OsString::from(" 1")), 1, PAGE_MAX),
            Err(RenderFailure::Invalid)
        );
    }

    #[test]
    fn derives_exact_dpi_dimensions_and_rejects_resource_excess() {
        assert_eq!(render_dimensions(612.0, 792.0, 72), Ok((612, 792)));
        assert_eq!(render_dimensions(612.0, 792.0, 300), Ok((2550, 3300)));
        assert_eq!(
            render_dimensions(10_000.0, 10_000.0, 600),
            Err(RenderFailure::Budget)
        );
        assert_eq!(
            render_dimensions(f32::NAN, 792.0, 300),
            Err(RenderFailure::Invalid)
        );
    }

    #[test]
    fn emits_bounded_binary_ppm_without_alpha() {
        let mut output = Vec::new();
        write_ppm(&mut output, 2, 1, &[255, 0, 0, 7, 0, 255, 0, 9]).unwrap();
        assert_eq!(output, b"P6\n2 1\n255\n\xff\x00\x00\x00\xff\x00");
        assert_eq!(
            write_ppm(&mut Vec::new(), 2, 1, &[0; 7]),
            Err(RenderFailure::Invalid)
        );
    }

    #[test]
    fn stable_failure_codes_do_not_include_private_context() {
        assert_eq!(RenderFailure::Unavailable.exit_code(), 1);
        assert_eq!(RenderFailure::Invalid.exit_code(), 2);
        assert_eq!(RenderFailure::Budget.exit_code(), 3);
        assert_eq!(
            RenderFailure::Invalid.public_message(),
            "pdf-render-runtime: invalid request"
        );
    }
}
