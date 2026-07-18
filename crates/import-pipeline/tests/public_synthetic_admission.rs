use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use import_pipeline::{import_root_with_options, ImportOptions};
use index_fulltext::{FullTextIndex, SearchQuery, SnapshotReadLease};
use index_vector::{VectorModelContract, VectorSnapshotRoot, VECTOR_SNAPSHOT_SCHEMA_V3};
use meta_store::{
    ClassificationStatus, Document, ImportTask, ImportTaskId, ImportTaskStatus, MetaStore,
    SearchSelection, SearchSelectionDetailsResolution, SourceRevision, UnixTimestamp,
    VectorSnapshotMode, CLASSIFIER_EPOCH,
};
use serde::Deserialize;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

const FROZEN_FIXTURE: &str =
    include_str!("../../../perf/fixtures/mixed-import/public-synthetic-benchmark.json");

#[derive(Deserialize)]
struct FrozenFixture {
    schema_version: String,
    synthetic_only: bool,
    freeze: FreezeContract,
    samples: Vec<FrozenSample>,
}

#[derive(Deserialize)]
struct FreezeContract {
    frozen: bool,
    mutation_after_freeze_allowed: bool,
}

#[derive(Deserialize)]
struct FrozenSample {
    virtual_relative_path: String,
    extension: String,
    ground_truth: String,
    parser_outcome: String,
    expected_status: String,
    content: String,
}

#[test]
fn frozen_public_synthetic_fixture_matches_production_admission() {
    let fixture: FrozenFixture = serde_json::from_str(FROZEN_FIXTURE).unwrap();
    assert_eq!(
        fixture.schema_version,
        "resume-ir.public-synthetic-mixed-benchmark.v1"
    );
    assert!(fixture.synthetic_only);
    assert!(fixture.freeze.frozen);
    assert!(!fixture.freeze.mutation_after_freeze_allowed);
    assert_eq!(fixture.samples.len(), 9);

    let temp = TestDir::new("public-synthetic-production-admission");
    let root = temp.path().join("mixed");
    let data_dir = temp.path().join("data");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&data_dir).unwrap();
    materialize_fixture(&root, &fixture.samples);

    let store = MetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let now = UnixTimestamp::from_unix_seconds(1_800_159_000);
    let task = ImportTask {
        id: ImportTaskId::from_non_secret_parts(&["public-synthetic-production-admission"]),
        root_path: root.to_string_lossy().into_owned(),
        status: ImportTaskStatus::Running,
        queued_at: now,
        started_at: Some(now),
        finished_at: None,
        updated_at: now,
    };
    store.insert_import_task(&task).unwrap();

    let summary = import_root_with_options(
        &data_dir,
        &store,
        &task,
        &root,
        now,
        ImportOptions::default(),
    )
    .unwrap();

    let counts = store.classification_counts(CLASSIFIER_EPOCH).unwrap();
    assert_eq!(
        (
            counts.resume_candidate,
            counts.non_resume,
            counts.needs_review,
            counts.ocr_backlog,
            counts.failed,
        ),
        (3, 3, 1, 1, 1)
    );
    assert_eq!(summary.searchable_documents, 3);
    assert_eq!(summary.ocr_required_documents, 1);
    assert_eq!(summary.failed_documents, 1);

    let fulltext_root = data_dir.join("search-index");
    let fulltext_lease = SnapshotReadLease::acquire(&fulltext_root)
        .unwrap()
        .expect("published full-text root has a read lease");
    let vector_root = VectorSnapshotRoot::new(data_dir.join("vector-index")).unwrap();
    let vector_lease = vector_root.acquire_read_lease().unwrap();
    let (head, projections, index) = store
        .with_search_metadata_snapshot(move |snapshot| {
            let head = snapshot.head().clone();
            let projections = snapshot.validated_active_projections().unwrap();
            assert_eq!(projections.len(), 3);

            for projection in &projections {
                let selection = SearchSelection {
                    document_id: projection.document_id.clone(),
                    resume_version_id: projection.resume_version_id.clone(),
                    visible_epoch: head.visible_epoch,
                };
                match snapshot.selection_details(&selection).unwrap() {
                    SearchSelectionDetailsResolution::Current(details) => {
                        assert_eq!(details.selection, selection);
                    }
                    other => panic!("active projection did not resolve exactly: {other:?}"),
                }
            }

            let fulltext_descriptor = head
                .publication
                .fulltext
                .as_ref()
                .expect("ready publication has full-text descriptor");
            let vector_descriptor = head
                .publication
                .vector
                .as_ref()
                .expect("ready publication has vector descriptor");
            assert_eq!(fulltext_descriptor.generation(), head.generation);
            assert_eq!(vector_descriptor.generation(), head.generation);
            assert_eq!(
                fulltext_descriptor.projection_digest(),
                &head.publication.projection_digest
            );
            assert_eq!(
                vector_descriptor.projection_digest(),
                &head.publication.projection_digest
            );
            assert_eq!(vector_descriptor.mode(), &VectorSnapshotMode::Disabled);

            let index = FullTextIndex::open_snapshot_with_lease(
                &fulltext_root,
                &head.generation,
                fulltext_lease,
            )
            .unwrap()
            .expect("ready full-text generation exists");
            let fulltext_metadata = index
                .snapshot_metadata()
                .expect("exact full-text reader has validated metadata");
            assert_eq!(fulltext_metadata.generation(), head.generation);
            assert_eq!(
                fulltext_metadata.projection_digest(),
                &head.publication.projection_digest
            );
            assert_eq!(
                fulltext_metadata.document_count(),
                fulltext_descriptor.document_count() as usize
            );
            assert_eq!(
                fulltext_metadata.logical_content_digest(),
                fulltext_descriptor.logical_content_digest()
            );
            let expected_pairs = projections
                .iter()
                .map(|projection| {
                    (
                        projection.document_id.to_string(),
                        projection.resume_version_id.to_string(),
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(index.exact_identity_pairs().unwrap(), expected_pairs);

            let vector = vector_root
                .open_generation_with_lease(
                    &head.generation,
                    &VectorModelContract::Disabled,
                    vector_lease,
                )
                .unwrap();
            assert_eq!(vector.summary().generation(), head.generation);
            assert_eq!(vector.summary().schema(), VECTOR_SNAPSHOT_SCHEMA_V3);
            assert_eq!(
                vector.summary().projection_digest(),
                &head.publication.projection_digest
            );
            assert_eq!(
                vector.summary().projection_digest(),
                fulltext_metadata.projection_digest()
            );
            assert_eq!(
                vector.summary().model_contract(),
                &VectorModelContract::Disabled
            );
            assert_eq!(
                vector.summary().projection_count(),
                vector_descriptor.projection_count() as usize
            );
            assert_eq!(vector.summary().projection_count(), projections.len());
            assert_eq!(
                vector.summary().vector_count(),
                vector_descriptor.vector_count() as usize
            );
            assert_eq!(vector.summary().vector_count(), 0);
            assert_eq!(
                vector.summary().vector_document_count(),
                vector_descriptor.document_count() as usize
            );
            assert_eq!(vector.summary().vector_document_count(), 0);
            assert_eq!(
                vector.summary().coverage_digest(),
                vector_descriptor.coverage_digest()
            );
            assert_eq!(
                vector.summary().logical_content_digest(),
                vector_descriptor.logical_content_digest()
            );
            assert_eq!(vector.exact_projection(), projections);

            Ok::<_, std::convert::Infallible>((head, projections, index))
        })
        .unwrap();
    assert_eq!(head.generation, head.publication.generation);

    let documents = store.visible_documents().unwrap();
    assert_eq!(documents.len(), fixture.samples.len());
    for document in &documents {
        let sample = fixture
            .samples
            .iter()
            .find(|sample| file_name(&sample.virtual_relative_path) == document.file_name)
            .unwrap();
        let classification_status = current_classification_status(&store, document).unwrap();
        assert_eq!(classification_status.as_str(), sample.expected_status);
        let active_projection = projections
            .iter()
            .find(|projection| projection.document_id == document.id);
        assert_eq!(
            active_projection.is_some(),
            classification_status == ClassificationStatus::ResumeCandidate
        );
        if classification_status != ClassificationStatus::ResumeCandidate {
            for version in store.resume_versions_for_document(&document.id).unwrap() {
                assert!(store
                    .entity_mentions_for_version(&version.id)
                    .unwrap()
                    .is_empty());
            }
        }
    }

    for sample in fixture
        .samples
        .iter()
        .filter(|sample| sample.parser_outcome == "text_extracted")
    {
        let probe = sample
            .content
            .lines()
            .nth(1)
            .or_else(|| sample.content.lines().next())
            .unwrap();
        let required_probe = probe
            .split_whitespace()
            .map(|term| term.trim_matches(|character: char| !character.is_alphanumeric()))
            .filter(|term| !term.is_empty())
            .map(|term| format!("+{term}"))
            .collect::<Vec<_>>()
            .join(" ");
        let hits = index.search(SearchQuery::new(required_probe)).unwrap();
        if sample.expected_status == "resume_candidate" {
            assert_eq!(hits.len(), 1);
            let hit_document = documents
                .iter()
                .find(|document| document.id.as_str() == hits[0].doc_id)
                .unwrap();
            assert_eq!(
                current_classification_status(&store, hit_document).unwrap(),
                ClassificationStatus::ResumeCandidate
            );
        } else {
            assert!(hits.is_empty());
        }
    }
    let status = store.status_summary().unwrap();
    assert!(status.entity_mentions > 0);
    assert_eq!(status.embedding_queue_depth, 0);
    assert!(data_dir.join("vector-index").exists());

    let expected_resumes = fixture
        .samples
        .iter()
        .filter(|sample| sample.ground_truth == "resume")
        .count();
    let indexed_true_resumes = fixture
        .samples
        .iter()
        .filter(|sample| {
            sample.ground_truth == "resume" && sample.expected_status == "resume_candidate"
        })
        .count();
    let indexed_total = fixture
        .samples
        .iter()
        .filter(|sample| sample.expected_status == "resume_candidate")
        .count();
    assert_eq!(
        (indexed_true_resumes, indexed_total, expected_resumes),
        (3, 3, 4)
    );
}

fn current_classification_status(
    store: &MetaStore,
    document: &Document,
) -> Option<ClassificationStatus> {
    let content_hash = document.content_hash.as_deref()?.parse().ok()?;
    let revision =
        SourceRevision::for_content(document.id.clone(), content_hash, document.byte_size);
    if let Some(triage) = store
        .source_revision_triage(&revision.id, CLASSIFIER_EPOCH)
        .unwrap()
    {
        return Some(triage.status);
    }
    let mut statuses = store
        .resume_versions_for_document(&document.id)
        .unwrap()
        .into_iter()
        .filter(|version| version.source_revision_id == revision.id)
        .filter_map(|version| {
            store
                .resume_version_classification(&version.id, CLASSIFIER_EPOCH)
                .unwrap()
                .map(|classification| classification.status)
        });
    let status = statuses.next()?;
    assert_eq!(
        statuses.next(),
        None,
        "one final classification per fixture"
    );
    Some(status)
}

fn materialize_fixture(root: &Path, samples: &[FrozenSample]) {
    for sample in samples {
        let path = root.join(&sample.virtual_relative_path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let bytes = match (sample.parser_outcome.as_str(), sample.extension.as_str()) {
            ("text_extracted", "txt") => sample.content.as_bytes().to_vec(),
            ("text_extracted", "docx") => synthetic_docx(&sample.content),
            ("text_extracted", "pdf") => text_layer_pdf(&sample.content),
            ("ocr_required", "pdf") => scanned_pdf(),
            ("failed", "doc") => Vec::new(),
            combination => panic!("unsupported frozen synthetic combination: {combination:?}"),
        };
        fs::write(path, bytes).unwrap();
    }
}

fn synthetic_docx(text: &str) -> Vec<u8> {
    let paragraphs = text
        .lines()
        .map(|line| format!("<w:p><w:r><w:t>{}</w:t></w:r></w:p>", xml_escape(line)))
        .collect::<String>();
    let document = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{paragraphs}</w:body></w:document>"#
    );
    let mut buffer = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut buffer);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    writer.start_file("word/document.xml", options).unwrap();
    writer.write_all(document.as_bytes()).unwrap();
    writer.finish().unwrap();
    buffer.into_inner()
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn text_layer_pdf(text: &str) -> Vec<u8> {
    let operations = text
        .lines()
        .map(|line| format!("({}) Tj T* ", pdf_escape(line)))
        .collect::<String>();
    let content = format!("BT /F1 12 Tf 72 720 Td {operations}ET\n").into_bytes();
    build_pdf(vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> >> /MediaBox [0 0 612 792] /Contents 5 0 R >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        [
            format!("<< /Length {} >>\nstream\n", content.len()).into_bytes(),
            content,
            b"endstream".to_vec(),
        ]
        .concat(),
    ])
}

fn pdf_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

fn scanned_pdf() -> Vec<u8> {
    build_pdf(vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Resources << /XObject << /Im1 4 0 R >> >> /MediaBox [0 0 612 792] /Contents 5 0 R >>".to_vec(),
        b"<< /Type /XObject /Subtype /Image /Width 100 /Height 100 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 11 >>\nstream\nimage bytes\nendstream".to_vec(),
        b"<< /Length 24 >>\nstream\nq 100 0 0 100 0 0 cm /Im1 Do Q\nendstream".to_vec(),
    ])
}

fn build_pdf(objects: Vec<Vec<u8>>) -> Vec<u8> {
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        pdf.extend_from_slice(object);
        if !object.ends_with(b"\n") {
            pdf.push(b'\n');
        }
        pdf.extend_from_slice(b"endobj\n");
    }
    let xref_offset = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Root 1 0 R /Size {} >>\nstartxref\n{xref_offset}\n%%EOF",
            objects.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap()
}

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("resume-ir-{label}-{unique}"));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
