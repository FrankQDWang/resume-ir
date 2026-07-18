use super::{SearchProjectionFilterError, SearchProjectionPredicate, MAX_SEARCH_FILTER_VALUES};

const MAX_SEARCH_FILTER_VALUE_BYTES: usize = 4 * 1024;

pub(super) fn validate_predicate(
    predicate: &SearchProjectionPredicate,
) -> std::result::Result<(), SearchProjectionFilterError> {
    match predicate {
        SearchProjectionPredicate::EntityValuesAny {
            normalized_values,
            min_confidence,
            ..
        }
        | SearchProjectionPredicate::EntityValuesAnyOrMissing {
            normalized_values,
            min_confidence,
            ..
        } => {
            validate_values(normalized_values)?;
            validate_confidence(*min_confidence)
        }
        SearchProjectionPredicate::NumericEntityMinimum {
            minimum,
            min_confidence,
            ..
        } => {
            if !minimum.is_finite() {
                return Err(SearchProjectionFilterError::InvalidNumericMinimum);
            }
            validate_confidence(*min_confidence)
        }
        SearchProjectionPredicate::DateRangeOverlap {
            start_month,
            end_month,
            min_confidence,
        } => {
            let end_month = end_month.unwrap_or(i32::MAX);
            if *start_month < 1900 * 12 + 1 || end_month < *start_month {
                return Err(SearchProjectionFilterError::InvalidDateRange);
            }
            validate_confidence(*min_confidence)
        }
        SearchProjectionPredicate::ContactHashesAny(values) => {
            if values.is_empty() {
                return Err(SearchProjectionFilterError::EmptyValues);
            }
            if values.len() > MAX_SEARCH_FILTER_VALUES {
                return Err(SearchProjectionFilterError::TooManyValues);
            }
            Ok(())
        }
        SearchProjectionPredicate::MissingEntityType { min_confidence, .. } => {
            validate_confidence(*min_confidence)
        }
    }
}

fn validate_values(values: &[String]) -> std::result::Result<(), SearchProjectionFilterError> {
    if values.is_empty() {
        return Err(SearchProjectionFilterError::EmptyValues);
    }
    if values.len() > MAX_SEARCH_FILTER_VALUES {
        return Err(SearchProjectionFilterError::TooManyValues);
    }
    if values
        .iter()
        .any(|value| value.is_empty() || value.len() > MAX_SEARCH_FILTER_VALUE_BYTES)
    {
        return Err(SearchProjectionFilterError::ValueTooLarge);
    }
    Ok(())
}

fn validate_confidence(value: f32) -> std::result::Result<(), SearchProjectionFilterError> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(SearchProjectionFilterError::InvalidConfidence)
    }
}
