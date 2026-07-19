use meta_store::{
    EntityType, ResumeVersionId, MAX_ENTITY_MENTIONS_PER_VERSION, MAX_ENTITY_MENTION_VALUE_BYTES,
};

use super::entity_mentions_from_rules;

fn unique_closed_date_ranges(count: usize) -> String {
    (0..count)
        .map(|ordinal| {
            let start_year = 1980 + ordinal / 12;
            let start_month = ordinal % 12 + 1;
            let (end_year, end_month) = if start_month == 12 {
                (start_year + 1, 1)
            } else {
                (start_year, start_month + 1)
            };
            format!("{start_year}-{start_month:02} - {end_year}-{end_month:02}\n")
        })
        .collect()
}

#[test]
fn rule_mentions_fit_the_immutable_version_contract() {
    let text = format!(
        "Experience\n{}Education\nSchool: Synthetic University\nSkills\nRust\n",
        unique_closed_date_ranges(MAX_ENTITY_MENTIONS_PER_VERSION + 32)
    );
    let version_id = ResumeVersionId::from_non_secret_parts(&["bounded-rule-mentions"]);

    let mentions = entity_mentions_from_rules(&version_id, &text);

    assert_eq!(mentions.len(), MAX_ENTITY_MENTIONS_PER_VERSION);
    assert!(mentions.iter().all(|mention| {
        mention.raw_value.len() <= MAX_ENTITY_MENTION_VALUE_BYTES
            && mention
                .normalized_value
                .as_deref()
                .is_none_or(|value| value.len() <= MAX_ENTITY_MENTION_VALUE_BYTES)
    }));
    let years = mentions
        .iter()
        .find(|mention| mention.entity_type == EntityType::YearsExperience)
        .unwrap();
    assert_eq!(years.span_start, None);
    assert_eq!(years.span_end, None);
    assert_eq!(years.raw_value, years.normalized_value.as_deref().unwrap());
    assert_eq!(years.extractor, "rules-v2-derived");
    for entity_type in [EntityType::School, EntityType::Skill] {
        assert!(mentions
            .iter()
            .any(|mention| mention.entity_type == entity_type));
    }
}
