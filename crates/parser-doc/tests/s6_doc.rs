use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use parser_common::{ParseInput, ParseStatus, Parser, ResourceBudget, SupportLevel};
use parser_doc::DocParser;

#[test]
fn exposes_parser_doc_crate_identity() {
    assert_eq!(parser_doc::crate_name(), "parser-doc");
}

#[cfg(unix)]
#[test]
fn extracts_legacy_doc_text_with_local_converter_without_output_leakage() {
    let temp = TestDir::new("parser-doc-converter");
    let converter = write_converter(temp.path());
    let parser = DocParser::with_converter(converter);
    let bytes = synthetic_ole_doc();
    let input = ParseInput::from_bytes(Some("doc"), &bytes);

    assert_eq!(parser.supports(input.probe()), SupportLevel::Supported);

    let output = parser.parse(input, ResourceBudget::default()).unwrap();

    assert_eq!(output.status(), ParseStatus::TextExtracted);
    assert_eq!(output.text(), "Synthetic Legacy Candidate\nRust Search");
    assert!(!format!("{output:?}").contains("Synthetic Legacy Candidate"));
}

fn synthetic_ole_doc() -> Vec<u8> {
    let mut bytes = b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1".to_vec();
    bytes.extend_from_slice(b"SYNTHETIC PRIVATE LEGACY DOC BODY");
    bytes
}

#[cfg(unix)]
fn write_converter(directory: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = directory.join("fixture-doc-converter");
    fs::write(
        &path,
        r#"#!/bin/sh
out=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-output" ]; then
    shift
    out="$1"
  fi
  shift
done
if [ -z "$out" ]; then
  exit 9
fi
printf 'Synthetic Legacy Candidate\nRust Search\n' > "$out"
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(label: &str) -> Self {
        let unique = format!(
            "{}-{}-{}",
            label,
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
