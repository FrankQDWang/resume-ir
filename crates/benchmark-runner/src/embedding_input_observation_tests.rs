use super::*;

fn observation(
    active_tokens: usize,
    family: Family,
    total: u64,
    retained: [u64; 3],
) -> DocumentObservation {
    DocumentObservation {
        active_tokens,
        families: [(family, FamilyTokens { total, retained })]
            .into_iter()
            .collect(),
    }
}

#[test]
fn conjunctive_trigger_requires_every_frozen_condition() {
    let mut passing = Accumulator::new(1_000).unwrap();
    for _ in 0..1_000 {
        passing
            .observe(observation(700, Family::Experience, 100, [40, 30, 20]))
            .unwrap();
    }
    let report: Value = serde_json::from_str(passing.finish().unwrap().to_redacted_json()).unwrap();
    assert_eq!(report["triggers"]["all"], true);
    assert_eq!(report["decision"], "l1_eligible");
    assert_eq!(report["documents"]["observed"], 1_000);
    assert_eq!(report["pre_truncation_active_tokens"]["p99"], 700);

    let mut insufficient = Accumulator::new(999).unwrap();
    for _ in 0..999 {
        insufficient
            .observe(observation(700, Family::Experience, 100, [40, 30, 20]))
            .unwrap();
    }
    let report: Value =
        serde_json::from_str(insufficient.finish().unwrap().to_redacted_json()).unwrap();
    assert_eq!(report["triggers"]["minimum_documents"], false);
    assert_eq!(report["triggers"]["all"], false);
    assert_eq!(report["decision"], "lost");
}

#[test]
fn coverage_buckets_and_document_reconciliation_are_exact() {
    let mut accumulator = Accumulator::new(6).unwrap();
    accumulator
        .observe(observation(600, Family::Skill, 10, [0, 0, 0]))
        .unwrap();
    accumulator
        .observe(observation(500, Family::Skill, 10, [4, 4, 4]))
        .unwrap();
    accumulator
        .observe(observation(400, Family::Skill, 10, [6, 6, 6]))
        .unwrap();
    accumulator
        .observe(observation(200, Family::Skill, 10, [10, 10, 10]))
        .unwrap();
    accumulator.exclude_oversize().unwrap();
    accumulator.fail_document().unwrap();
    let report: Value =
        serde_json::from_str(accumulator.finish().unwrap().to_redacted_json()).unwrap();
    assert_eq!(
        report["documents"],
        json!({
            "selected": 6,
            "observed": 4,
            "excluded_oversize": 1,
            "failed": 1
        })
    );
    assert_eq!(report["section_coverage"]["skill"]["present_documents"], 4);
    assert_eq!(
        report["section_coverage"]["skill"]["budgets"]["512"],
        json!({
            "complete_loss": 1,
            "partial_below_half": 1,
            "partial_at_least_half": 1,
            "complete_retained": 1
        })
    );
    assert_eq!(report["priority_coverage_512"]["documents_below_half"], 2);
}

#[test]
fn current_sectionizer_leaves_preamble_available_for_unassigned_attribution() {
    let text = "姓名 张三\nExperience\n构建 systems\nSkills\nRust";
    let spans = section_spans(text).unwrap();
    assert!(spans.first().unwrap().start > 0);
    assert_eq!(spans[0].family, Family::Experience);
    assert_eq!(spans[1].family, Family::Skill);
    assert_eq!(
        char_to_byte(text, text.chars().count()).unwrap(),
        text.len()
    );
}
