use std::sync::{Arc, Mutex};

use search_runtime::SearchArtifactFaultKey;

#[derive(Clone)]
pub(crate) struct ArtifactFaultReporter {
    latest: Arc<LatestValue<SearchArtifactFaultKey>>,
}

pub(crate) struct ArtifactFaultReceiver {
    latest: Arc<LatestValue<SearchArtifactFaultKey>>,
}

struct LatestValue<Value> {
    slot: Mutex<Option<Value>>,
}

impl<Value> Default for LatestValue<Value> {
    fn default() -> Self {
        Self {
            slot: Mutex::new(None),
        }
    }
}

pub(crate) fn artifact_fault_latch() -> (ArtifactFaultReporter, ArtifactFaultReceiver) {
    let latest = Arc::new(LatestValue::default());
    (
        ArtifactFaultReporter {
            latest: Arc::clone(&latest),
        },
        ArtifactFaultReceiver { latest },
    )
}

impl<Value: PartialEq> LatestValue<Value> {
    fn report(&self, value: Value) {
        let mut slot = self.slot.lock().expect("artifact fault latch");
        if slot.as_ref() != Some(&value) {
            *slot = Some(value);
        }
    }

    fn take(&self) -> Option<Value> {
        self.slot.lock().expect("artifact fault latch").take()
    }
}

impl ArtifactFaultReporter {
    /// Capacity-one latest-wins handoff. Repeated faults for the same immutable
    /// head are deduplicated; a newer head replaces a stale pending fault.
    pub(crate) fn report(&self, fault: SearchArtifactFaultKey) {
        self.latest.report(fault);
    }
}

impl ArtifactFaultReceiver {
    pub(crate) fn try_take(&self) -> Option<SearchArtifactFaultKey> {
        self.latest.take()
    }
}

#[cfg(test)]
mod tests {
    use super::LatestValue;

    #[test]
    fn pending_value_is_deduplicated_and_replaced_by_the_latest_key() {
        let latest = LatestValue::default();
        latest.report("generation-a".to_string());
        latest.report("generation-a".to_string());
        latest.report("generation-b".to_string());

        assert_eq!(latest.take().as_deref(), Some("generation-b"));
        assert_eq!(latest.take(), None);
    }
}
