use core_domain::SectionType;
use fs_crawler::DiscoveredFile;
use index_fulltext::IndexSection;
use meta_store::{Document, DocumentStatus, FileExtension, UnixTimestamp};
use sectionizer::SectionChunk;

pub(crate) fn document_from_discovered_file(
    file: &DiscoveredFile,
    now: UnixTimestamp,
    status: DocumentStatus,
) -> Document {
    Document {
        id: file.document_id.clone(),
        source_uri: format!("file://{}", file.normalized_path.as_str()),
        normalized_path: file.normalized_path.as_str().to_string(),
        file_name: file.file_name.clone(),
        extension: file.extension.clone(),
        byte_size: file.byte_size,
        mtime: file.mtime,
        content_hash: None,
        text_hash: None,
        is_deleted: false,
        created_at: now,
        updated_at: now,
        status,
    }
}

pub(crate) fn sections_to_index(sections: Vec<SectionChunk>) -> Vec<IndexSection> {
    sections
        .into_iter()
        .map(|section| IndexSection {
            section_type: section_type_label(&section.section_type).to_string(),
            text: section.text,
        })
        .collect()
}

pub(crate) fn section_type_label(section_type: &SectionType) -> &str {
    match section_type {
        SectionType::Profile => "profile",
        SectionType::Contact => "contact",
        SectionType::Education => "education",
        SectionType::Experience => "experience",
        SectionType::Project => "project",
        SectionType::Skill => "skill",
        SectionType::Certificate => "certificate",
        SectionType::Other(_) => "other",
    }
}

pub(crate) fn file_extension_label(extension: &FileExtension) -> &'static str {
    match extension {
        FileExtension::Docx => "docx",
        FileExtension::Pdf => "pdf",
        FileExtension::Doc => "doc",
        FileExtension::Txt => "txt",
        FileExtension::Image => "image",
        FileExtension::Other(_) => "other",
    }
}

pub(crate) fn language_set(text: &str) -> Vec<String> {
    classify_language_set(text)
        .into_iter()
        .map(str::to_string)
        .collect()
}

pub(crate) fn classify_language_set(text: &str) -> Vec<&'static str> {
    let mut has_english = false;
    let mut has_chinese = false;
    for character in text.chars() {
        has_english |= character.is_ascii_alphabetic();
        has_chinese |= is_cjk_character(character);
        if has_english && has_chinese {
            break;
        }
    }

    let mut languages = Vec::new();
    if has_english {
        languages.push("en");
    }
    if has_chinese {
        languages.push("zh");
    }
    if languages.is_empty() {
        languages.push("unknown");
    }
    languages
}

pub(crate) fn is_cjk_character(character: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&character) || ('\u{3400}'..='\u{4dbf}').contains(&character)
}
