use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant, UNIX_EPOCH};

use meta_store::{
    ImportProcessingContract, OwnedMetaStore, ScanTrigger, SourceRoot, SourceRootState,
    SourceWatcherState, UnixTimestamp,
};
use notify::{
    event::EventKind as NotifyEventKind, Config as NotifyConfig, Event as NotifyEvent,
    RecommendedWatcher, RecursiveMode, Watcher,
};

use crate::daemon_error::{DaemonError, Result};

const WATCH_EVENT_DEBOUNCE: Duration = Duration::from_millis(750);
const WATCH_EVENT_MAX_DELAY: Duration = Duration::from_secs(2);
const WATCH_RETRY_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone, Copy)]
struct PendingRoot {
    quiet_due: Instant,
    max_due: Instant,
}

impl PendingRoot {
    fn new(observed_at: Instant) -> Self {
        Self {
            quiet_due: observed_at + WATCH_EVENT_DEBOUNCE,
            max_due: observed_at + WATCH_EVENT_MAX_DELAY,
        }
    }

    fn observe_again(&mut self, observed_at: Instant) {
        self.quiet_due = observed_at + WATCH_EVENT_DEBOUNCE;
    }

    fn is_due(self, now: Instant) -> bool {
        self.quiet_due <= now || self.max_due <= now
    }
}

pub(crate) struct ImportWatcher {
    watcher: RecommendedWatcher,
    receiver: Receiver<notify::Result<NotifyEvent>>,
    watched_roots: BTreeSet<String>,
    watched_root_mtimes: BTreeMap<String, Option<u128>>,
    watch_retry_due: BTreeMap<String, Instant>,
    pending_roots: BTreeMap<String, PendingRoot>,
}

impl ImportWatcher {
    pub(crate) fn new() -> Result<Self> {
        let (sender, receiver) = mpsc::channel();
        let watcher = RecommendedWatcher::new(
            move |event| {
                let _ = sender.send(event);
            },
            NotifyConfig::default(),
        )
        .map_err(|_| {
            DaemonError::recoverable_dependency("import watcher blocked: local watcher unavailable")
        })?;

        Ok(Self {
            watcher,
            receiver,
            watched_roots: BTreeSet::new(),
            watched_root_mtimes: BTreeMap::new(),
            watch_retry_due: BTreeMap::new(),
            pending_roots: BTreeMap::new(),
        })
    }

    pub(crate) fn sync_and_requeue(
        &mut self,
        store: &OwnedMetaStore,
        processing_contract: &ImportProcessingContract,
        now: UnixTimestamp,
    ) -> Result<ImportWatcherSummary> {
        let (roots, mut summary) = reconcile_root_availability(store, processing_contract, now)?;
        let mut roots_by_path = BTreeMap::new();
        for root in roots {
            if root.state != SourceRootState::Active
                || root.watcher_state == SourceWatcherState::Paused
                || store
                    .latest_scan_snapshot(&root.id)
                    .map_err(DaemonError::store)?
                    .is_none()
            {
                continue;
            }
            roots_by_path.insert(root.canonical_path.clone(), root);
        }
        summary.extend(self.sync_watched_roots(store, &roots_by_path, now)?);
        self.drain_events(&roots_by_path, &mut summary);
        self.poll_changed_roots(&roots_by_path, &mut summary);
        let debounce_cutoff = Instant::now();
        let pending_roots = self
            .pending_roots
            .iter()
            .filter_map(|(root, pending)| pending.is_due(debounce_cutoff).then_some(root.clone()))
            .collect::<Vec<_>>();

        for root_path in pending_roots {
            self.pending_roots.remove(&root_path);
            let Some(root) = roots_by_path.get(&root_path) else {
                continue;
            };
            match crate::source_scan_coordinator::enqueue(
                store,
                processing_contract,
                root,
                ScanTrigger::Watcher,
                now,
            ) {
                Ok(_) => summary.requeued += 1,
                Err(_) => summary.event_errors += 1,
            }
        }

        Ok(summary)
    }

    fn sync_watched_roots(
        &mut self,
        store: &OwnedMetaStore,
        roots: &BTreeMap<String, SourceRoot>,
        now: UnixTimestamp,
    ) -> Result<ImportWatcherSummary> {
        let requested_roots = roots.keys().cloned().collect::<BTreeSet<_>>();
        let previous_roots = self.watched_roots.clone();
        let mut next_roots = BTreeSet::new();
        let mut summary = ImportWatcherSummary::default();
        self.watch_retry_due
            .retain(|root, _| requested_roots.contains(root));

        for root in previous_roots.difference(&requested_roots) {
            if self.watcher.unwatch(Path::new(root)).is_err() {
                summary.event_errors += 1;
            }
            self.watched_root_mtimes.remove(root);
            self.watch_retry_due.remove(root);
            self.pending_roots.remove(root);
        }

        for (root_path, root) in roots {
            if previous_roots.contains(root_path) {
                next_roots.insert(root_path.clone());
                self.watch_retry_due.remove(root_path);
                if root.watcher_state != SourceWatcherState::Active {
                    store
                        .set_source_root_state(
                            &root.id,
                            SourceRootState::Active,
                            SourceWatcherState::Active,
                            now,
                        )
                        .map_err(DaemonError::store)?;
                }
                continue;
            }
            if self
                .watch_retry_due
                .get(root_path)
                .is_some_and(|due| *due > Instant::now())
            {
                continue;
            }
            if self
                .watcher
                .watch(Path::new(root_path), RecursiveMode::Recursive)
                .is_ok()
            {
                self.watched_root_mtimes
                    .insert(root_path.clone(), import_watcher_root_mtime(root_path));
                self.watch_retry_due.remove(root_path);
                next_roots.insert(root_path.clone());
                if root.watcher_state != SourceWatcherState::Active {
                    store
                        .set_source_root_state(
                            &root.id,
                            SourceRootState::Active,
                            SourceWatcherState::Active,
                            now,
                        )
                        .map_err(DaemonError::store)?;
                }
            } else {
                summary.event_errors += 1;
                self.watch_retry_due
                    .insert(root_path.clone(), Instant::now() + WATCH_RETRY_INTERVAL);
                if root.watcher_state != SourceWatcherState::Unavailable {
                    store
                        .set_source_root_state(
                            &root.id,
                            SourceRootState::Active,
                            SourceWatcherState::Unavailable,
                            now,
                        )
                        .map_err(DaemonError::store)?;
                }
            }
        }

        self.watched_roots = next_roots;
        summary.active_roots =
            (self.watched_roots != previous_roots).then_some(self.watched_roots.len());
        Ok(summary)
    }

    fn drain_events(
        &mut self,
        roots_by_path: &BTreeMap<String, SourceRoot>,
        summary: &mut ImportWatcherSummary,
    ) {
        loop {
            match self.receiver.try_recv() {
                Ok(Ok(event)) => {
                    if !import_watcher_event_is_relevant(&event) {
                        continue;
                    }
                    summary.events += 1;
                    for path in event.paths {
                        if let Some(root) = import_watcher_root_for_path(roots_by_path, &path) {
                            self.schedule_pending_root(root.to_string(), Instant::now());
                        }
                    }
                }
                Ok(Err(_)) => summary.event_errors += 1,
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    summary.event_errors += 1;
                    return;
                }
            }
        }
    }

    fn poll_changed_roots(
        &mut self,
        roots: &BTreeMap<String, SourceRoot>,
        summary: &mut ImportWatcherSummary,
    ) {
        for root in roots.keys() {
            if !self.watched_roots.contains(root) {
                continue;
            }
            let previous_mtime = self.watched_root_mtimes.get(root).copied().flatten();
            let current_mtime = import_watcher_root_mtime(root);
            self.watched_root_mtimes.insert(root.clone(), current_mtime);
            if previous_mtime.is_some() && current_mtime != previous_mtime {
                summary.events += 1;
                self.schedule_pending_root(root.clone(), Instant::now());
            }
        }
    }

    fn schedule_pending_root(&mut self, root: String, observed_at: Instant) {
        self.pending_roots
            .entry(root)
            .and_modify(|pending| pending.observe_again(observed_at))
            .or_insert_with(|| PendingRoot::new(observed_at));
    }
}

pub(crate) fn mark_watchers_unavailable(
    store: &OwnedMetaStore,
    now: UnixTimestamp,
) -> Result<usize> {
    let mut changed = 0;
    for root in store.source_roots().map_err(DaemonError::store)? {
        if root.watcher_state != SourceWatcherState::Active {
            continue;
        }
        store
            .set_source_root_state(&root.id, root.state, SourceWatcherState::Unavailable, now)
            .map_err(DaemonError::store)?;
        changed += 1;
    }
    Ok(changed)
}

fn reconcile_root_availability(
    store: &OwnedMetaStore,
    processing_contract: &ImportProcessingContract,
    now: UnixTimestamp,
) -> Result<(Vec<SourceRoot>, ImportWatcherSummary)> {
    let mut roots = Vec::new();
    let mut summary = ImportWatcherSummary::default();
    for root in store.source_roots().map_err(DaemonError::store)? {
        if store
            .source_root_deletion_in_progress(&root.id)
            .map_err(DaemonError::store)?
        {
            continue;
        }
        let available = crate::source_root_path::is_available(&root.canonical_path);
        match (root.state, available) {
            (state, false) if state != SourceRootState::Offline => {
                let watcher_state = if root.watcher_state == SourceWatcherState::Paused {
                    SourceWatcherState::Paused
                } else {
                    SourceWatcherState::Unavailable
                };
                roots.push(
                    store
                        .set_source_root_state(
                            &root.id,
                            SourceRootState::Offline,
                            watcher_state,
                            now,
                        )
                        .map_err(DaemonError::store)?,
                );
            }
            (SourceRootState::Offline, true) => {
                let watcher_state = if root.watcher_state == SourceWatcherState::Paused {
                    SourceWatcherState::Paused
                } else {
                    SourceWatcherState::Active
                };
                let recovered = store
                    .set_source_root_state(&root.id, SourceRootState::Active, watcher_state, now)
                    .map_err(DaemonError::store)?;
                let has_scan_history = store
                    .latest_scan_snapshot(&recovered.id)
                    .map_err(DaemonError::store)?
                    .is_some();
                if watcher_state == SourceWatcherState::Active && has_scan_history {
                    match crate::source_scan_coordinator::enqueue(
                        store,
                        processing_contract,
                        &recovered,
                        ScanTrigger::Recovery,
                        now,
                    ) {
                        Ok(_) => summary.requeued += 1,
                        Err(_) => summary.event_errors += 1,
                    }
                }
                roots.push(recovered);
            }
            _ => roots.push(root),
        }
    }
    Ok((roots, summary))
}

#[derive(Default)]
pub(crate) struct ImportWatcherSummary {
    pub(crate) active_roots: Option<usize>,
    pub(crate) events: usize,
    pub(crate) requeued: usize,
    pub(crate) event_errors: usize,
}

impl ImportWatcherSummary {
    fn extend(&mut self, other: Self) {
        if other.active_roots.is_some() {
            self.active_roots = other.active_roots;
        }
        self.events += other.events;
        self.requeued += other.requeued;
        self.event_errors += other.event_errors;
    }
}

fn import_watcher_event_is_relevant(event: &NotifyEvent) -> bool {
    matches!(
        event.kind,
        NotifyEventKind::Any
            | NotifyEventKind::Create(_)
            | NotifyEventKind::Modify(_)
            | NotifyEventKind::Remove(_)
    )
}

fn import_watcher_root_for_path<'a>(
    roots_by_path: &'a BTreeMap<String, SourceRoot>,
    event_path: &Path,
) -> Option<&'a str> {
    roots_by_path
        .keys()
        .find(|root| event_path.starts_with(Path::new(root.as_str())))
        .map(String::as_str)
}

fn import_watcher_root_mtime(root: &str) -> Option<u128> {
    std::fs::metadata(root)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_events_preserve_a_bounded_maximum_debounce_delay() {
        let first = Instant::now();
        let mut pending = PendingRoot::new(first);
        assert!(!pending.is_due(first + WATCH_EVENT_DEBOUNCE / 2));
        assert!(pending.is_due(first + WATCH_EVENT_DEBOUNCE));

        pending.observe_again(first + WATCH_EVENT_MAX_DELAY - WATCH_EVENT_DEBOUNCE / 2);
        assert!(!pending.is_due(first + WATCH_EVENT_MAX_DELAY - Duration::from_millis(1)));
        assert!(pending.is_due(first + WATCH_EVENT_MAX_DELAY));
    }
}
