use tokenizers::Encoding;

use super::{RuntimeError, DIMENSION, MAX_INPUTS};

pub(super) struct TokenizedBatch {
    pub(super) input_ids: Vec<i64>,
    pub(super) attention_masks: Vec<Vec<i64>>,
    pub(super) token_type_ids: Vec<i64>,
    pub(super) sequence_length: usize,
}

pub(super) fn tokenized_batch(encodings: &[Encoding]) -> Result<TokenizedBatch, RuntimeError> {
    if encodings.is_empty() || encodings.len() > MAX_INPUTS {
        return Err(RuntimeError::InferenceFailed);
    }
    let sequence_length = encodings[0].get_ids().len();
    if sequence_length == 0 {
        return Err(RuntimeError::InferenceFailed);
    }
    let capacity = encodings
        .len()
        .checked_mul(sequence_length)
        .ok_or(RuntimeError::InferenceFailed)?;
    let mut input_ids = Vec::with_capacity(capacity);
    let mut attention_masks = Vec::with_capacity(encodings.len());
    let mut token_type_ids = Vec::with_capacity(capacity);
    for encoding in encodings {
        let ids = encoding.get_ids();
        let mask = encoding.get_attention_mask();
        let types = encoding.get_type_ids();
        if ids.len() != sequence_length
            || mask.len() != sequence_length
            || types.len() != sequence_length
        {
            return Err(RuntimeError::InferenceFailed);
        }
        input_ids.extend(ids.iter().map(|value| i64::from(*value)));
        attention_masks.push(mask.iter().map(|value| i64::from(*value)).collect());
        token_type_ids.extend(types.iter().map(|value| i64::from(*value)));
    }
    Ok(TokenizedBatch {
        input_ids,
        attention_masks,
        token_type_ids,
        sequence_length,
    })
}

pub(super) fn mean_pool_batch(
    shape: &[i64],
    values: &[f32],
    attention_masks: &[Vec<i64>],
) -> Result<Vec<Vec<f32>>, RuntimeError> {
    if attention_masks.is_empty() || attention_masks.len() > MAX_INPUTS {
        return Err(RuntimeError::OutputInvalid);
    }
    let batch_size = attention_masks.len();
    let batch_size_i64 = i64::try_from(batch_size).map_err(|_| RuntimeError::OutputInvalid)?;
    if shape.len() == 2
        && shape[0] == batch_size_i64
        && shape[1] == DIMENSION as i64
        && values.len()
            == batch_size
                .checked_mul(DIMENSION)
                .ok_or(RuntimeError::OutputInvalid)?
    {
        return Ok(values
            .chunks_exact(DIMENSION)
            .map(ToOwned::to_owned)
            .collect());
    }

    let sequence_length = shape
        .get(1)
        .copied()
        .filter(|value| *value > 0)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(RuntimeError::OutputInvalid)?;
    if shape.len() != 3
        || shape[0] != batch_size_i64
        || shape[2] != DIMENSION as i64
        || attention_masks
            .iter()
            .any(|mask| mask.len() != sequence_length)
    {
        return Err(RuntimeError::OutputInvalid);
    }
    let row_width = sequence_length
        .checked_mul(DIMENSION)
        .ok_or(RuntimeError::OutputInvalid)?;
    let expected_values = batch_size
        .checked_mul(row_width)
        .ok_or(RuntimeError::OutputInvalid)?;
    if values.len() != expected_values {
        return Err(RuntimeError::OutputInvalid);
    }

    let mut pooled_batch = Vec::with_capacity(batch_size);
    for (row, attention_mask) in values.chunks_exact(row_width).zip(attention_masks) {
        let mut pooled = vec![0.0_f32; DIMENSION];
        let mut included = 0_u32;
        for (token, mask) in row.chunks_exact(DIMENSION).zip(attention_mask) {
            if *mask == 0 {
                continue;
            }
            included = included.saturating_add(1);
            for (output, value) in pooled.iter_mut().zip(token) {
                *output += value;
            }
        }
        if included == 0 {
            return Err(RuntimeError::OutputInvalid);
        }
        for value in &mut pooled {
            *value /= included as f32;
        }
        pooled_batch.push(pooled);
    }
    Ok(pooled_batch)
}
