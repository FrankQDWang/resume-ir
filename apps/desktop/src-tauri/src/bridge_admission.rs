use std::sync::Mutex;

use crate::daemon_client::DesktopError;
use crate::daemon_request::Operation;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BridgeLane {
    Lifecycle,
    NativeDialog,
    Import,
    Control,
    Diagnostics,
    Interactive,
    Status,
    Cancel,
}

impl BridgeLane {
    const COUNT: usize = 8;

    fn index(self) -> usize {
        self as usize
    }

    fn capacity(self) -> usize {
        match self {
            Self::Lifecycle | Self::NativeDialog | Self::Import | Self::Diagnostics => 1,
            Self::Control => 2,
            Self::Interactive => 4,
            Self::Status | Self::Cancel => 2,
        }
    }
}

pub(crate) struct BridgeAdmissionState {
    active: Mutex<[usize; BridgeLane::COUNT]>,
}

impl Default for BridgeAdmissionState {
    fn default() -> Self {
        Self {
            active: Mutex::new([0; BridgeLane::COUNT]),
        }
    }
}

impl BridgeAdmissionState {
    pub(crate) fn try_acquire(&self, lane: BridgeLane) -> Result<BridgePermit<'_>, DesktopError> {
        let mut active = self.active.lock().map_err(|_| DesktopError::internal())?;
        let count = &mut active[lane.index()];
        if *count >= lane.capacity() {
            return Err(DesktopError::new(
                "bridge_overloaded",
                "桌面请求繁忙，请稍后重试",
            ));
        }
        *count += 1;
        Ok(BridgePermit {
            admission: self,
            lane,
        })
    }

    fn release(&self, lane: BridgeLane) {
        if let Ok(mut active) = self.active.lock() {
            active[lane.index()] = active[lane.index()].saturating_sub(1);
        }
    }
}

pub(crate) struct BridgePermit<'a> {
    admission: &'a BridgeAdmissionState,
    lane: BridgeLane,
}

impl Drop for BridgePermit<'_> {
    fn drop(&mut self) {
        self.admission.release(self.lane);
    }
}

pub(crate) fn lane_for_operation(operation: Operation) -> BridgeLane {
    match operation {
        Operation::Status => BridgeLane::Status,
        Operation::Diagnostics => BridgeLane::Diagnostics,
        Operation::Import => BridgeLane::Import,
        Operation::RootControl => BridgeLane::Control,
        Operation::Search | Operation::Detail | Operation::Hydrate => BridgeLane::Interactive,
        Operation::Cancel => BridgeLane::Cancel,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn saturated_work_lanes_leave_status_and_cancel_capacity_available() {
        let admission = BridgeAdmissionState::default();
        let interactive = (0..4)
            .map(|_| admission.try_acquire(BridgeLane::Interactive).unwrap())
            .collect::<Vec<_>>();
        let import = admission.try_acquire(BridgeLane::Import).unwrap();
        let control = (0..2)
            .map(|_| admission.try_acquire(BridgeLane::Control).unwrap())
            .collect::<Vec<_>>();

        let interactive_error = admission
            .try_acquire(BridgeLane::Interactive)
            .err()
            .unwrap();
        let import_error = admission.try_acquire(BridgeLane::Import).err().unwrap();
        assert!(admission.try_acquire(BridgeLane::Control).is_err());
        assert_eq!(
            serde_json::to_value(interactive_error).unwrap(),
            json!({"code": "bridge_overloaded", "message": "桌面请求繁忙，请稍后重试"})
        );
        assert_eq!(
            serde_json::to_value(import_error).unwrap(),
            json!({"code": "bridge_overloaded", "message": "桌面请求繁忙，请稍后重试"})
        );

        let status = (0..2)
            .map(|_| admission.try_acquire(BridgeLane::Status).unwrap())
            .collect::<Vec<_>>();
        let cancel = (0..2)
            .map(|_| admission.try_acquire(BridgeLane::Cancel).unwrap())
            .collect::<Vec<_>>();
        assert!(admission.try_acquire(BridgeLane::Status).is_err());
        assert!(admission.try_acquire(BridgeLane::Cancel).is_err());

        drop(interactive);
        drop(import);
        drop(control);
        drop(status);
        drop(cancel);
        assert!(admission.try_acquire(BridgeLane::Interactive).is_ok());
        assert!(admission.try_acquire(BridgeLane::Import).is_ok());
        assert!(admission.try_acquire(BridgeLane::Control).is_ok());
        assert!(admission.try_acquire(BridgeLane::Status).is_ok());
        assert!(admission.try_acquire(BridgeLane::Cancel).is_ok());
    }

    #[test]
    fn status_and_cancel_use_distinct_lanes_from_every_longer_daemon_operation() {
        assert_eq!(lane_for_operation(Operation::Status), BridgeLane::Status);
        assert_eq!(lane_for_operation(Operation::Cancel), BridgeLane::Cancel);
        assert_eq!(
            lane_for_operation(Operation::Diagnostics),
            BridgeLane::Diagnostics
        );
        for operation in [Operation::Search, Operation::Detail, Operation::Hydrate] {
            assert_eq!(lane_for_operation(operation), BridgeLane::Interactive);
        }
        assert_eq!(lane_for_operation(Operation::Import), BridgeLane::Import);
        assert_eq!(
            lane_for_operation(Operation::RootControl),
            BridgeLane::Control
        );
    }

    #[test]
    fn low_frequency_native_command_lanes_are_independently_bounded() {
        let admission = BridgeAdmissionState::default();
        for lane in [
            BridgeLane::Lifecycle,
            BridgeLane::NativeDialog,
            BridgeLane::Diagnostics,
        ] {
            let permit = admission.try_acquire(lane).unwrap();
            assert!(admission.try_acquire(lane).is_err());
            drop(permit);
            assert!(admission.try_acquire(lane).is_ok());
        }
    }
}
