use std::num::NonZeroUsize;

use rusqlite::{params_from_iter, types::Value};

use super::{
    BoundedFilterSelection, SearchFilterCase, SearchProjectionFilter, SearchProjectionPredicate,
    MAX_BOUNDED_FILTER_SELECTION,
};
use crate::{
    entity_type_to_storage, search_snapshot::SearchMetadataSnapshot, ActiveSearchProjection,
    EntityType, MetaStoreError, Result,
};

impl SearchMetadataSnapshot<'_> {
    /// Evaluates a bounded AND filter against the exact active mapping held by
    /// this metadata generation. Values inside one predicate use ANY semantics.
    /// `cap + 1` is read so an oversized result is rejected rather than
    /// silently truncated.
    pub fn bounded_filter_selection(
        &self,
        filter: &SearchProjectionFilter,
        cap: NonZeroUsize,
    ) -> Result<BoundedFilterSelection> {
        filter
            .validate()
            .map_err(|_| MetaStoreError::invalid_value("search_projection_filter"))?;
        let cap = cap.get();
        if cap > MAX_BOUNDED_FILTER_SELECTION {
            return Err(MetaStoreError::invalid_value(
                "search_projection_filter.cap",
            ));
        }
        let (sql, values) = filter_query(filter, &self.head.generation, cap)?;
        let mut statement = self
            .connection
            .prepare(&sql)
            .map_err(MetaStoreError::storage)?;
        let mut projections = statement
            .query_map(params_from_iter(values), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(MetaStoreError::storage)?
            .map(|row| {
                let (document_id, resume_version_id) = row.map_err(MetaStoreError::storage)?;
                Ok(ActiveSearchProjection {
                    document_id: document_id.parse().map_err(|_| {
                        MetaStoreError::invalid_value("active_search_projection.document_id")
                    })?,
                    resume_version_id: resume_version_id.parse().map_err(|_| {
                        MetaStoreError::invalid_value("active_search_projection.resume_version_id")
                    })?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        if projections.len() > cap {
            return Ok(BoundedFilterSelection::TooLarge { cap });
        }
        projections.shrink_to_fit();
        Ok(BoundedFilterSelection::Selected(projections))
    }
}

fn filter_query(
    filter: &SearchProjectionFilter,
    generation: &str,
    cap: usize,
) -> Result<(String, Vec<Value>)> {
    let mut sql = String::from(
        "SELECT projection.document_id, projection.resume_version_id\n\
         FROM active_search_projection AS projection\n\
         WHERE projection.generation = ?1",
    );
    let mut values = vec![Value::Text(generation.to_string())];
    for predicate in filter.predicates() {
        sql.push_str("\nAND ");
        append_predicate_sql(&mut sql, &mut values, predicate)?;
    }
    values.push(Value::Integer(
        i64::try_from(cap + 1).map_err(|_| MetaStoreError::storage_invariant())?,
    ));
    sql.push_str(&format!(
        "\nORDER BY projection.document_id\nLIMIT ?{}",
        values.len()
    ));
    Ok((sql, values))
}

fn append_predicate_sql(
    sql: &mut String,
    values: &mut Vec<Value>,
    predicate: &SearchProjectionPredicate,
) -> Result<()> {
    match predicate {
        SearchProjectionPredicate::EntityValuesAny {
            entity_type,
            normalized_values,
            min_confidence,
            case,
        } => append_entity_values_sql(
            sql,
            values,
            entity_type,
            normalized_values,
            *min_confidence,
            *case,
            false,
        ),
        SearchProjectionPredicate::EntityValuesAnyOrMissing {
            entity_type,
            normalized_values,
            min_confidence,
            case,
        } => append_entity_values_sql(
            sql,
            values,
            entity_type,
            normalized_values,
            *min_confidence,
            *case,
            true,
        ),
        SearchProjectionPredicate::NumericEntityMinimum {
            entity_type,
            minimum,
            min_confidence,
        } => {
            let entity = push_value(values, entity_type_to_storage(entity_type).to_string());
            let confidence = push_value(values, f64::from(*min_confidence));
            let minimum = push_value(values, f64::from(*minimum));
            sql.push_str(&format!(
                "EXISTS (SELECT 1 FROM entity_mention AS mention \
                 WHERE mention.resume_version_id = projection.resume_version_id \
                 AND mention.entity_type = ?{entity} \
                 AND mention.confidence >= ?{confidence} \
                 AND CAST(mention.normalized_value AS REAL) >= ?{minimum})"
            ));
        }
        SearchProjectionPredicate::DateRangeOverlap {
            start_month,
            end_month,
            min_confidence,
        } => {
            let confidence = push_value(values, f64::from(*min_confidence));
            let end = push_value(values, i64::from(end_month.unwrap_or(i32::MAX)));
            let start = push_value(values, i64::from(*start_month));
            sql.push_str(&format!(
                "EXISTS (SELECT 1 FROM entity_mention AS mention \
                 WHERE mention.resume_version_id = projection.resume_version_id \
                 AND mention.entity_type = 'date_range' \
                 AND mention.confidence >= ?{confidence} \
                 AND mention.normalized_value IS NOT NULL \
                 AND (mention.normalized_value GLOB \
                    '[0-9][0-9][0-9][0-9]-[0-9][0-9]/[0-9][0-9][0-9][0-9]-[0-9][0-9]' \
                    OR mention.normalized_value GLOB \
                    '[0-9][0-9][0-9][0-9]-[0-9][0-9]/PRESENT') \
                 AND (CAST(substr(mention.normalized_value, 1, 4) AS INTEGER) * 12 \
                    + CAST(substr(mention.normalized_value, 6, 2) AS INTEGER)) <= ?{end} \
                 AND (CASE WHEN substr(mention.normalized_value, 9) = 'PRESENT' \
                    THEN 2147483647 ELSE \
                    CAST(substr(mention.normalized_value, 9, 4) AS INTEGER) * 12 \
                    + CAST(substr(mention.normalized_value, 14, 2) AS INTEGER) END) >= ?{start})"
            ));
        }
        SearchProjectionPredicate::ContactHashesAny(contact_hashes) => {
            sql.push_str(
                "EXISTS (SELECT 1 FROM resume_version_candidate AS assignment \
                 JOIN candidate AS candidate ON candidate.id = assignment.candidate_id \
                 WHERE assignment.resume_version_id = projection.resume_version_id AND (",
            );
            let placeholders = contact_hashes
                .iter()
                .map(|hash| push_value(values, hash.as_str().to_string()))
                .map(|position| format!("?{position}"))
                .collect::<Vec<_>>()
                .join(", ");
            sql.push_str(&format!(
                "candidate.email_hash IN ({placeholders}) OR \
                 candidate.phone_hash IN ({placeholders})))"
            ));
        }
        SearchProjectionPredicate::MissingEntityType {
            entity_type,
            min_confidence,
        } => append_missing_entity_sql(sql, values, entity_type, *min_confidence),
    }
    Ok(())
}

fn append_entity_values_sql(
    sql: &mut String,
    values: &mut Vec<Value>,
    entity_type: &EntityType,
    normalized_values: &[String],
    min_confidence: f32,
    case: SearchFilterCase,
    include_missing: bool,
) {
    if include_missing {
        sql.push('(');
    }
    let entity = push_value(values, entity_type_to_storage(entity_type).to_string());
    let confidence = push_value(values, f64::from(min_confidence));
    let placeholders = normalized_values
        .iter()
        .map(|value| match case {
            SearchFilterCase::Exact => value.clone(),
            SearchFilterCase::AsciiInsensitive => value.to_ascii_lowercase(),
        })
        .map(|value| push_value(values, value))
        .map(|position| format!("?{position}"))
        .collect::<Vec<_>>()
        .join(", ");
    let expression = match case {
        SearchFilterCase::Exact => "mention.normalized_value",
        SearchFilterCase::AsciiInsensitive => "LOWER(mention.normalized_value)",
    };
    sql.push_str(&format!(
        "EXISTS (SELECT 1 FROM entity_mention AS mention \
         WHERE mention.resume_version_id = projection.resume_version_id \
         AND mention.entity_type = ?{entity} \
         AND mention.confidence >= ?{confidence} \
         AND {expression} IN ({placeholders}))"
    ));
    if include_missing {
        sql.push_str(" OR ");
        append_missing_entity_sql(sql, values, entity_type, min_confidence);
        sql.push(')');
    }
}

fn append_missing_entity_sql(
    sql: &mut String,
    values: &mut Vec<Value>,
    entity_type: &EntityType,
    min_confidence: f32,
) {
    let entity = push_value(values, entity_type_to_storage(entity_type).to_string());
    let confidence = push_value(values, f64::from(min_confidence));
    sql.push_str(&format!(
        "NOT EXISTS (SELECT 1 FROM entity_mention AS mention \
         WHERE mention.resume_version_id = projection.resume_version_id \
         AND mention.entity_type = ?{entity} \
         AND mention.confidence >= ?{confidence})"
    ));
}

fn push_value(values: &mut Vec<Value>, value: impl Into<Value>) -> usize {
    values.push(value.into());
    values.len()
}
