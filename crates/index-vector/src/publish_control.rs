use crate::VectorIndexError;

pub(crate) const SNAPSHOT_PUBLISH_RECORD_INTERVAL: usize = 64;

/// Cooperative cancellation control for immutable vector publication work.
///
/// Checks run only between records, fixed-size batches, or crash-safe phases.
/// The final generation rename and durability sequence remains indivisible.
#[derive(Clone, Copy)]
pub struct VectorSnapshotPublishControl<'a> {
    cancel_check: Option<&'a dyn Fn() -> bool>,
}

impl<'a> VectorSnapshotPublishControl<'a> {
    /// Disables cancellation while retaining the same publication phase path.
    pub fn disabled() -> Self {
        Self { cancel_check: None }
    }

    /// Observes a caller-owned cancellation predicate at cooperative boundaries.
    pub fn from_cancel_check(cancel_check: &'a dyn Fn() -> bool) -> Self {
        Self {
            cancel_check: Some(cancel_check),
        }
    }

    pub(crate) fn check(self) -> Result<(), VectorIndexError> {
        if self.cancel_check.is_some_and(|cancel_check| cancel_check()) {
            Err(VectorIndexError::Cancelled)
        } else {
            Ok(())
        }
    }

    pub(crate) fn check_after_record(self, completed: usize) -> Result<(), VectorIndexError> {
        if completed.is_multiple_of(SNAPSHOT_PUBLISH_RECORD_INTERVAL) {
            self.check()?;
        }
        Ok(())
    }
}
