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
fn degree_level_parse_accepts_broader_engineering_degree_aliases() {
    assert_eq!(DegreeLevel::parse("MEng"), Some(DegreeLevel::Master));
    assert_eq!(DegreeLevel::parse("M.Tech"), Some(DegreeLevel::Master));
    assert_eq!(DegreeLevel::parse("MPhil"), Some(DegreeLevel::Master));
    assert_eq!(DegreeLevel::parse("B.Tech"), Some(DegreeLevel::Bachelor));
    assert_eq!(DegreeLevel::parse("B.E."), Some(DegreeLevel::Bachelor));
    assert_eq!(DegreeLevel::parse("BE"), None);
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
fn field_filters_match_overlapping_date_range() {
    let filters = SearchFilters::default().with_date_range_overlaps("2021-01/2021-12");
    let matching =
        ResumeProfile::new("doc_active").with_date_ranges(["2020-03/2022-06", "2024-01/PRESENT"]);
    let before = ResumeProfile::new("doc_before").with_date_ranges(["2018-01/2019-12"]);
    let after = ResumeProfile::new("doc_after").with_date_ranges(["2022-01/2023-12"]);
    let missing_range = ResumeProfile::new("doc_missing_range");

    assert!(filters.matches(&matching));
    assert!(!filters.matches(&before));
    assert!(!filters.matches(&after));
    assert!(!filters.matches(&missing_range));
    assert_eq!(
        filters.date_range_overlaps().unwrap().canonical(),
        "2021-01/2021-12"
    );
}

#[test]
fn field_filters_match_any_school() {
    let filters = SearchFilters::default()
        .with_schools_any(["Synthetic Institute of Technology", "Other University"]);
    let matching =
        ResumeProfile::new("doc_school").with_schools(["synthetic institute of technology"]);
    let other_school = ResumeProfile::new("doc_other_school").with_schools(["Synthetic College"]);
    let missing_school = ResumeProfile::new("doc_missing_school");

    assert!(filters.matches(&matching));
    assert!(!filters.matches(&other_school));
    assert!(!filters.matches(&missing_school));
    assert_eq!(
        filters.schools_any(),
        &["other university", "synthetic institute of technology"]
    );
}

#[test]
fn field_filters_match_any_major() {
    let filters =
        SearchFilters::default().with_majors_any(["Computer Science", "Software Engineering"]);
    let matching = ResumeProfile::new("doc_major").with_majors(["computer_science"]);
    let other_major = ResumeProfile::new("doc_other_major").with_majors(["data_science"]);
    let missing_major = ResumeProfile::new("doc_missing_major");

    assert!(filters.matches(&matching));
    assert!(!filters.matches(&other_major));
    assert!(!filters.matches(&missing_major));
    assert_eq!(
        filters.majors_any(),
        &["computer_science", "software_engineering"]
    );
}

#[test]
fn field_filters_normalize_broader_major_aliases() {
    let filters = SearchFilters::default().with_majors_any(["人工智能", "网络工程", "会计学"]);
    let artificial_intelligence =
        ResumeProfile::new("doc_ai").with_majors(["artificial_intelligence"]);
    let network_engineering =
        ResumeProfile::new("doc_network").with_majors(["network_engineering"]);
    let accounting = ResumeProfile::new("doc_accounting").with_majors(["accounting"]);
    let other_major = ResumeProfile::new("doc_other").with_majors(["computer_science"]);

    assert!(filters.matches(&artificial_intelligence));
    assert!(filters.matches(&network_engineering));
    assert!(filters.matches(&accounting));
    assert!(!filters.matches(&other_major));
    assert_eq!(
        filters.majors_any(),
        &[
            "accounting",
            "artificial_intelligence",
            "network_engineering"
        ]
    );
}

#[test]
fn field_filters_match_company_and_title() {
    let filters = SearchFilters::default()
        .with_companies_any(["Synthetic Payments Inc.", "Other Co"])
        .with_titles_any(["Backend Engineer"]);
    let matching = ResumeProfile::new("doc_backend")
        .with_companies(["synthetic payments"])
        .with_titles(["backend_engineer"]);
    let other_company = ResumeProfile::new("doc_other_company")
        .with_companies(["synthetic search"])
        .with_titles(["backend_engineer"]);
    let other_title = ResumeProfile::new("doc_other_title")
        .with_companies(["synthetic payments"])
        .with_titles(["product_manager"]);

    assert!(filters.matches(&matching));
    assert!(!filters.matches(&other_company));
    assert!(!filters.matches(&other_title));
    assert_eq!(filters.companies_any(), &["other", "synthetic payments"]);
    assert_eq!(filters.titles_any(), &["backend_engineer"]);

    let chinese_company_filter = SearchFilters::default().with_companies_any(["幻方股份有限公司"]);
    let chinese_company_profile =
        ResumeProfile::new("doc_chinese_company").with_companies(["幻方"]);
    assert!(chinese_company_filter.matches(&chinese_company_profile));
    assert_eq!(chinese_company_filter.companies_any(), &["幻方"]);
}

#[test]
fn field_filters_match_any_location() {
    let filters = SearchFilters::default().with_locations_any(["Shanghai", "杭州"]);
    let matching = ResumeProfile::new("doc_shanghai").with_locations(["上海"]);
    let other_location = ResumeProfile::new("doc_beijing").with_locations(["Beijing"]);
    let missing_location = ResumeProfile::new("doc_missing_location");

    assert!(filters.matches(&matching));
    assert!(!filters.matches(&other_location));
    assert!(!filters.matches(&missing_location));
    assert_eq!(filters.locations_any(), &["hangzhou", "shanghai"]);
}

#[test]
fn field_filters_normalize_broader_location_aliases() {
    let filters = SearchFilters::default().with_locations_any(["SF Bay Area", "纽约", "Hong Kong"]);
    let bay_area = ResumeProfile::new("doc_bay_area").with_locations(["San Francisco Bay Area"]);
    let new_york = ResumeProfile::new("doc_new_york").with_locations(["New York City"]);
    let hong_kong = ResumeProfile::new("doc_hong_kong").with_locations(["香港"]);
    let other_location = ResumeProfile::new("doc_seattle").with_locations(["Seattle"]);

    assert!(filters.matches(&bay_area));
    assert!(filters.matches(&new_york));
    assert!(filters.matches(&hong_kong));
    assert!(!filters.matches(&other_location));
    assert_eq!(
        filters.locations_any(),
        &["hong_kong", "new_york", "san_francisco"]
    );
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
