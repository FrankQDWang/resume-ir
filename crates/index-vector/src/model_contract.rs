use crate::model::{validate_model_id, VectorIndexError};

pub const MAX_VECTOR_DIMENSION: usize = 65_536;

/// Exact semantic model contract bound to an immutable vector generation.
///
/// A disabled generation still carries the complete active search projection,
/// but cannot contain vectors or invent a model identity or dimension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VectorModelContract {
    Disabled,
    Enabled { model_id: String, dimension: usize },
}

impl VectorModelContract {
    pub fn enabled(
        model_id: impl Into<String>,
        dimension: usize,
    ) -> Result<Self, VectorIndexError> {
        let contract = Self::Enabled {
            model_id: model_id.into(),
            dimension,
        };
        contract.validate()?;
        Ok(contract)
    }

    pub fn model_id(&self) -> Option<&str> {
        match self {
            Self::Disabled => None,
            Self::Enabled { model_id, .. } => Some(model_id),
        }
    }

    pub fn dimension(&self) -> Option<usize> {
        match self {
            Self::Disabled => None,
            Self::Enabled { dimension, .. } => Some(*dimension),
        }
    }

    pub(crate) fn validate(&self) -> Result<(), VectorIndexError> {
        match self {
            Self::Disabled => Ok(()),
            Self::Enabled {
                model_id,
                dimension,
            } => {
                validate_model_id(model_id)?;
                if *dimension == 0 {
                    return Err(VectorIndexError::InvalidDimension {
                        expected: 1,
                        actual: 0,
                    });
                }
                if *dimension > MAX_VECTOR_DIMENSION {
                    return Err(VectorIndexError::InvalidDimension {
                        expected: MAX_VECTOR_DIMENSION,
                        actual: *dimension,
                    });
                }
                Ok(())
            }
        }
    }
}
