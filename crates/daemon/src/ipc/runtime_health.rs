use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};

use super::{OptionalRuntimeHealth, OptionalRuntimeReason};

const RUNTIME_HEALTH_CHANNEL_CAPACITY: usize = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeHealthUpdate {
    Ocr(OptionalRuntimeHealth),
}

#[derive(Clone)]
pub(crate) struct RuntimeHealthReporter {
    sender: SyncSender<RuntimeHealthUpdate>,
}

pub(crate) struct RuntimeHealthReceiver {
    receiver: Receiver<RuntimeHealthUpdate>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeHealthChannelClosed;

pub(crate) fn runtime_health_channel() -> (RuntimeHealthReporter, RuntimeHealthReceiver) {
    let (sender, receiver) = mpsc::sync_channel(RUNTIME_HEALTH_CHANNEL_CAPACITY);
    (
        RuntimeHealthReporter { sender },
        RuntimeHealthReceiver { receiver },
    )
}

impl RuntimeHealthReporter {
    pub(crate) fn ocr_unavailable(
        &self,
        reason: OptionalRuntimeReason,
    ) -> Result<(), RuntimeHealthChannelClosed> {
        let update = RuntimeHealthUpdate::Ocr(OptionalRuntimeHealth::unavailable(reason));
        self.sender
            .send(update)
            .map_err(|_| RuntimeHealthChannelClosed)
    }
}

impl RuntimeHealthReceiver {
    pub(crate) fn try_recv(
        &self,
    ) -> Result<Option<RuntimeHealthUpdate>, RuntimeHealthChannelClosed> {
        match self.receiver.try_recv() {
            Ok(update) => Ok(Some(update)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(RuntimeHealthChannelClosed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{runtime_health_channel, RuntimeHealthUpdate};
    use crate::ipc::{OptionalRuntimeReason, OptionalRuntimeState};

    #[test]
    fn bounded_runtime_health_channel_carries_typed_degradation() {
        let (reporter, receiver) = runtime_health_channel();
        reporter
            .ocr_unavailable(OptionalRuntimeReason::Invalid)
            .unwrap();

        let update = receiver.try_recv().unwrap().unwrap();
        let RuntimeHealthUpdate::Ocr(health) = update;
        assert_eq!(health.state, OptionalRuntimeState::Unavailable);
        assert_eq!(health.reason, Some(OptionalRuntimeReason::Invalid));
        assert!(receiver.try_recv().unwrap().is_none());
    }
}
