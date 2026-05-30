use extractor_rules::extract_resume_fields;
use rank_fusion::{CandidateProfile, DegreeLevel, FieldFilter, filter_candidates, soft_dedupe_key};

#[test]
fn field_filters_keep_candidates_meeting_degree_skill_and_experience() {
    let fields = extract_resume_fields(
        "Education: Zhejiang University Bachelor of Computer Science 2018.09-2022.06 \
         Skills: Java, Spring, Redis Experience: 2022.07-2026.05 Java backend engineer",
    );
    let candidate = CandidateProfile {
        doc_id: "doc_java_backend".to_owned(),
        fields,
    };
    let filters = FieldFilter {
        degree_min: Some(DegreeLevel::Bachelor),
        skills_any: vec!["java".to_owned()],
        years_experience_min: Some(3.0),
    };

    let kept = filter_candidates(&[candidate], &filters);

    assert_eq!(kept, vec!["doc_java_backend".to_owned()]);
}

#[test]
fn field_filters_reject_candidates_missing_required_skill() {
    let fields = extract_resume_fields(
        "Education: Zhejiang University Bachelor Skills: Python Experience: 2022.07-2026.05",
    );
    let candidate = CandidateProfile {
        doc_id: "doc_python".to_owned(),
        fields,
    };
    let filters = FieldFilter {
        degree_min: Some(DegreeLevel::Bachelor),
        skills_any: vec!["java".to_owned()],
        years_experience_min: None,
    };

    assert!(filter_candidates(&[candidate], &filters).is_empty());
}

#[test]
fn soft_dedupe_key_uses_stable_non_contact_fields_when_available() {
    let fields = extract_resume_fields("Education: Zhejiang University Bachelor Skills: Java");

    let key = soft_dedupe_key("java_backend.docx", &fields);

    assert_eq!(key, "profile:zhejiang university:bachelor");
}
