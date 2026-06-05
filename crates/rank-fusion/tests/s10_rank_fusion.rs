use rank_fusion::{
    fold_by_candidate, reciprocal_rank_fusion, soft_dedupe_score, DedupeProfile, DegreeLevel,
    RankedHit, ResumeProfile, SchoolTier, SearchFilters,
};

#[test]
fn exposes_rank_fusion_crate_identity() {
    assert_eq!(rank_fusion::crate_name(), "rank-fusion");
}

#[test]
fn field_filters_require_degree_skill_and_year_thresholds() {
    let filters = SearchFilters::default()
        .with_degree_min(DegreeLevel::Bachelor)
        .with_skills_any(["java", "spring cloud"])
        .with_years_experience_min(3.0);

    let matching = ResumeProfile::new("doc_java")
        .with_degree(DegreeLevel::Master)
        .with_skills(["Rust", "Java"])
        .with_years_experience(4.2);
    let low_degree = ResumeProfile::new("doc_low_degree")
        .with_degree(DegreeLevel::Associate)
        .with_skills(["Java"])
        .with_years_experience(8.0);
    let missing_skill = ResumeProfile::new("doc_missing_skill")
        .with_degree(DegreeLevel::Bachelor)
        .with_skills(["Python"])
        .with_years_experience(5.0);
    let junior = ResumeProfile::new("doc_junior")
        .with_degree(DegreeLevel::Bachelor)
        .with_skills(["Java"])
        .with_years_experience(1.5);

    assert!(filters.matches(&matching));
    assert!(!filters.matches(&low_degree));
    assert!(!filters.matches(&missing_skill));
    assert!(!filters.matches(&junior));
}

#[test]
fn field_filters_match_any_school_tier() {
    let filters = SearchFilters::default().with_school_tiers_any([SchoolTier::Tier985]);
    let matching = ResumeProfile::new("doc_elite")
        .with_school_tiers([SchoolTier::Tier211, SchoolTier::Tier985]);
    let other_tier = ResumeProfile::new("doc_other").with_school_tiers([SchoolTier::Overseas]);
    let missing_tier = ResumeProfile::new("doc_missing");

    assert!(filters.matches(&matching));
    assert!(!filters.matches(&other_tier));
    assert!(!filters.matches(&missing_tier));
    assert_eq!(filters.school_tiers_any()[0].canonical(), "985");
    assert_eq!(
        SchoolTier::parse("双一流").unwrap(),
        SchoolTier::DoubleFirstClass
    );
}

#[test]
fn field_filters_match_unknown_school_tier_when_no_tier_evidence_exists() {
    let filters = SearchFilters::default().with_school_tiers_any([SchoolTier::Unknown]);
    let missing_tier = ResumeProfile::new("doc_missing");
    let known_tier = ResumeProfile::new("doc_known").with_school_tiers([SchoolTier::Regular]);

    assert!(filters.matches(&missing_tier));
    assert!(!filters.matches(&known_tier));
    assert_eq!(filters.school_tiers_any()[0].canonical(), "unknown");
}

#[test]
fn field_filters_match_any_certificate() {
    let filters = SearchFilters::default().with_certificates_any(["PMP", "CKA"]);
    let matching = ResumeProfile::new("doc_certified").with_certificates(["pmp", "cissp"]);
    let other_certificate = ResumeProfile::new("doc_other").with_certificates(["cpa"]);
    let missing_certificate = ResumeProfile::new("doc_missing");

    assert!(filters.matches(&matching));
    assert!(!filters.matches(&other_certificate));
    assert!(!filters.matches(&missing_certificate));
    assert_eq!(filters.certificates_any(), &["cka", "pmp"]);
}

#[test]
fn candidate_fold_keeps_best_ranked_version_per_candidate() {
    let hits = vec![
        RankedHit::new("doc_old", 1, 9.5).with_candidate_key("cand_same"),
        RankedHit::new("doc_new", 2, 8.5).with_candidate_key("cand_same"),
        RankedHit::new("doc_other", 3, 7.0).with_candidate_key("cand_other"),
    ];

    let folded = fold_by_candidate(hits);

    assert_eq!(
        folded.iter().map(|hit| hit.doc_id()).collect::<Vec<_>>(),
        vec!["doc_old", "doc_other"]
    );
}

#[test]
fn soft_dedupe_scores_same_name_with_correlated_non_contact_evidence() {
    let first = DedupeProfile::new("doc_first")
        .with_name("Synthetic Candidate")
        .with_schools(["Synthetic University"])
        .with_companies(["Example Labs"])
        .with_skills(["Java", "Payments"]);
    let second = DedupeProfile::new("doc_second")
        .with_name(" synthetic candidate ")
        .with_schools(["synthetic university"])
        .with_skills(["Java", "Search"]);
    let different_name = DedupeProfile::new("doc_other")
        .with_name("Different Candidate")
        .with_schools(["Synthetic University"])
        .with_companies(["Example Labs"])
        .with_skills(["Java", "Payments"]);

    let score = soft_dedupe_score(&first, &second).expect("same name plus school/skill evidence");

    assert!(score.confidence() > 0.70);
    assert_eq!(score.left_doc_id(), "doc_first");
    assert_eq!(score.right_doc_id(), "doc_second");
    assert_eq!(score.shared_school_count(), 1);
    assert_eq!(score.shared_skill_count(), 1);
    assert!(soft_dedupe_score(&first, &different_name).is_none());
    assert!(!format!("{score:?}").contains("Synthetic Candidate"));
    assert!(!format!("{first:?}").contains("Synthetic University"));
}

#[test]
fn reciprocal_rank_fusion_combines_independent_channels() {
    let fused = reciprocal_rank_fusion(
        [
            vec!["doc_a".to_string(), "doc_b".to_string()],
            vec!["doc_b".to_string(), "doc_c".to_string()],
        ],
        60.0,
    );

    assert_eq!(fused[0].doc_id(), "doc_b");
    assert!(fused[0].score() > fused[1].score());
}
