use parser_common::{
    FileProbe, ParseInput, ParseOutput, ParseStatus, Parser, ParserError, ResourceBudget,
    SupportLevel,
};

pub fn crate_name() -> &'static str {
    "parser-text"
}

pub const DEFAULT_MAX_BYTES: usize = 16 * 1024 * 1024;

pub struct TxtParser;

impl Parser for TxtParser {
    fn supports(&self, probe: &FileProbe) -> SupportLevel {
        match probe.extension() {
            Some("txt") => SupportLevel::Supported,
            _ => SupportLevel::Unsupported,
        }
    }

    fn parse(
        &self,
        input: ParseInput<'_>,
        budget: ResourceBudget,
    ) -> parser_common::Result<ParseOutput> {
        let parse_budget = budget.begin(input.probe().byte_len())?;
        if self.supports(input.probe()) == SupportLevel::Unsupported {
            return Err(ParserError::unsupported(
                "txt parser received unsupported probe",
            ));
        }

        parse_budget.check_deadline()?;
        let text = decode_text(input.bytes())?;
        parse_budget.check_deadline()?;

        Ok(ParseOutput::new(
            ParseStatus::TextExtracted,
            normalize_newlines(text),
        ))
    }
}

fn decode_text(bytes: &[u8]) -> parser_common::Result<String> {
    if let Some(bytes) = bytes.strip_prefix(b"\xef\xbb\xbf") {
        return String::from_utf8(bytes.to_vec())
            .map_err(|_| ParserError::corrupted("txt parser received invalid utf-8"));
    }

    if let Some(bytes) = bytes.strip_prefix(b"\xff\xfe") {
        return decode_utf16(bytes, Endian::Little);
    }

    if let Some(bytes) = bytes.strip_prefix(b"\xfe\xff") {
        return decode_utf16(bytes, Endian::Big);
    }

    String::from_utf8(bytes.to_vec())
        .map_err(|_| ParserError::corrupted("txt parser received invalid utf-8"))
}

fn decode_utf16(bytes: &[u8], endian: Endian) -> parser_common::Result<String> {
    if !bytes.len().is_multiple_of(2) {
        return Err(ParserError::corrupted(
            "txt parser received truncated utf-16 input",
        ));
    }

    let units = bytes.chunks_exact(2).map(|chunk| match endian {
        Endian::Little => u16::from_le_bytes([chunk[0], chunk[1]]),
        Endian::Big => u16::from_be_bytes([chunk[0], chunk[1]]),
    });

    std::char::decode_utf16(units)
        .collect::<std::result::Result<String, _>>()
        .map_err(|_| ParserError::corrupted("txt parser received invalid utf-16"))
}

fn normalize_newlines(text: String) -> String {
    let text = text.replace("\r\n", "\n");
    text.replace('\r', "\n")
}

#[derive(Clone, Copy)]
enum Endian {
    Little,
    Big,
}
