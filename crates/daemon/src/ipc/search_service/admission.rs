use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

const TOTAL_IN_FLIGHT_LIMIT: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClientClass {
    InteractiveGui,
    CodexValidation,
    Benchmark,
    Background,
}

impl ClientClass {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "interactive_gui" => Some(Self::InteractiveGui),
            "codex_validation" => Some(Self::CodexValidation),
            "benchmark" => Some(Self::Benchmark),
            "background" => Some(Self::Background),
            _ => None,
        }
    }

    fn index(self) -> usize {
        match self {
            Self::InteractiveGui => 0,
            Self::CodexValidation => 1,
            Self::Benchmark => 2,
            Self::Background => 3,
        }
    }

    pub(super) fn in_flight_limit(self) -> usize {
        match self {
            Self::InteractiveGui => 8,
            Self::CodexValidation => 2,
            Self::Benchmark => 8,
            Self::Background => 4,
        }
    }
}

pub(super) struct AdmissionState {
    total: AtomicUsize,
    by_class: [AtomicUsize; 4],
}

impl AdmissionState {
    pub(super) fn new() -> Self {
        Self {
            total: AtomicUsize::new(0),
            by_class: std::array::from_fn(|_| AtomicUsize::new(0)),
        }
    }

    pub(super) fn acquire(self: &Arc<Self>, class: ClientClass) -> Option<AdmissionPermit> {
        let class_counter = &self.by_class[class.index()];
        class_counter
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < class.in_flight_limit()).then_some(current + 1)
            })
            .ok()?;
        if self
            .total
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < TOTAL_IN_FLIGHT_LIMIT).then_some(current + 1)
            })
            .is_err()
        {
            class_counter.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        Some(AdmissionPermit {
            inner: Arc::new(AdmissionPermitInner {
                state: Arc::clone(self),
                class,
                released: AtomicBool::new(false),
            }),
        })
    }
}

#[derive(Clone)]
pub(super) struct AdmissionPermit {
    inner: Arc<AdmissionPermitInner>,
}

struct AdmissionPermitInner {
    state: Arc<AdmissionState>,
    class: ClientClass,
    released: AtomicBool,
}

impl AdmissionPermit {
    pub(super) fn release(&self) {
        if self.inner.released.swap(true, Ordering::AcqRel) {
            return;
        }
        self.inner.state.total.fetch_sub(1, Ordering::AcqRel);
        self.inner.state.by_class[self.inner.class.index()].fetch_sub(1, Ordering::AcqRel);
    }
}

impl Drop for AdmissionPermitInner {
    fn drop(&mut self) {
        if self.released.swap(true, Ordering::AcqRel) {
            return;
        }
        self.state.total.fetch_sub(1, Ordering::AcqRel);
        self.state.by_class[self.class.index()].fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) struct BatchAdmissionPermit {
    pub(super) active: Arc<AtomicBool>,
}

impl Drop for BatchAdmissionPermit {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
    }
}

#[cfg(test)]
#[path = "admission_tests.rs"]
mod tests;
