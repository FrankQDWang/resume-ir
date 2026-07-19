use std::collections::{HashMap, VecDeque};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::search_contract::SearchCancellation;

const CANCEL_HISTORY_LIMIT: usize = 128;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CancelStatus {
    Cancelled,
    CancelRequested,
    Complete,
}

impl CancelStatus {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::CancelRequested => "cancel_requested",
            Self::Complete => "complete",
        }
    }
}

pub(super) struct RequestControl {
    pub(super) completed: AtomicBool,
    pub(super) cancellation: SearchCancellation,
}

impl RequestControl {
    pub(super) fn new() -> Self {
        Self {
            completed: AtomicBool::new(false),
            cancellation: SearchCancellation::new(),
        }
    }
}

#[derive(Default)]
pub(super) struct CancellationRegistry {
    state: Mutex<CancellationRegistryState>,
}

#[derive(Default)]
struct CancellationRegistryState {
    active: HashMap<String, Arc<RequestControl>>,
    terminal: HashMap<String, CancelStatus>,
    terminal_order: VecDeque<String>,
}

impl CancellationRegistry {
    pub(super) fn register(&self, token: &str, control: Arc<RequestControl>) -> bool {
        let mut state = self.state.lock().expect("query cancellation registry");
        if state.active.contains_key(token) || state.terminal.contains_key(token) {
            return false;
        }
        state.active.insert(token.to_string(), control);
        true
    }

    pub(super) fn lookup(&self, token: &str) -> RegistryLookup {
        let state = self.state.lock().expect("query cancellation registry");
        if let Some(control) = state.active.get(token) {
            return RegistryLookup::Active(Arc::clone(control));
        }
        RegistryLookup::Terminal(
            state
                .terminal
                .get(token)
                .copied()
                .unwrap_or(CancelStatus::Complete),
        )
    }

    pub(super) fn complete(&self, token: Option<&str>, status: CancelStatus) {
        let Some(token) = token else {
            return;
        };
        let mut state = self.state.lock().expect("query cancellation registry");
        state.active.remove(token);
        state.terminal.insert(token.to_string(), status);
        state.terminal_order.push_back(token.to_string());
        while state.terminal_order.len() > CANCEL_HISTORY_LIMIT {
            if let Some(expired) = state.terminal_order.pop_front() {
                state.terminal.remove(&expired);
            }
        }
    }
}

pub(super) enum RegistryLookup {
    Active(Arc<RequestControl>),
    Terminal(CancelStatus),
}
