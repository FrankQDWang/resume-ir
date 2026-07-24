use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::UNIX_EPOCH;

use meta_store::{ImportProcessingContract, ImportScanScope, OwnedMetaStore, UnixTimestamp};
use notify::{
    event::EventKind as NotifyEventKind, Config as NotifyConfig, Event as NotifyEvent,
    RecommendedWatcher, RecursiveMode, Watcher,
};

use crate::daemon_error::{DaemonError, Result};
use crate::{import_command, rescan_schedule};

pub(crate) struct ImportWatcher {
    watcher: RecommendedWatcher,
    receiver: Receiver<notify::Result<NotifyEvent>>,
    watched_roots: BTreeSet<String>,
    watched_root_mtimes: BTreeMap<String, Option<u128>>,
    pending_roots: BTreeSet<String>,
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
            pending_roots: BTreeSet::new(),
        })
    }

    pub(crate) fn sync_and_requeue(
        &mut self,
        store: &OwnedMetaStore,
        processing_contract: &ImportProcessingContract,
        now: UnixTimestamp,
    ) -> Result<ImportWatcherSummary> {
        let scopes = store
            .completed_import_scan_scopes_due_for_requeue(now)
            .map_err(DaemonError::store)?;
        let scopes_by_root = scopes
            .into_iter()
            .map(|scope| (scope.canonical_root_path.clone(), scope))
            .collect::<BTreeMap<_, _>>();
        let roots = scopes_by_root.keys().cloned().collect::<BTreeSet<_>>();
        let mut summary = self.sync_watched_roots(&roots);
        self.drain_events(&scopes_by_root, &mut summary);
        self.poll_changed_roots(&roots, &mut summary);
        let pending_roots = std::mem::take(&mut self.pending_roots);

        for (index, root) in pending_roots.into_iter().enumerate() {
            let Some(scope) = scopes_by_root.get(&root).cloned() else {
                continue;
            };
            if rescan_schedule::enqueue_import_from_completed_scope(
                store,
                processing_contract,
                scope,
                import_command::new_task_id(index)
                    .map_err(|_| DaemonError::user("system clock is before unix epoch"))?,
                now,
            )? {
                summary.requeued += 1;
            }
        }

        Ok(summary)
    }

    fn sync_watched_roots(&mut self, roots: &BTreeSet<String>) -> ImportWatcherSummary {
        let previous_roots = self.watched_roots.clone();
        let mut next_roots = BTreeSet::new();
        let mut event_errors = 0_usize;

        for root in previous_roots.difference(roots) {
            if self.watcher.unwatch(Path::new(root)).is_err() {
                event_errors += 1;
            }
            self.watched_root_mtimes.remove(root);
        }

        for root in roots {
            if previous_roots.contains(root) {
                next_roots.insert(root.clone());
                continue;
            }
            if !Path::new(root).exists() {
                event_errors += 1;
                continue;
            }
            if self
                .watcher
                .watch(Path::new(root), RecursiveMode::Recursive)
                .is_ok()
            {
                self.watched_root_mtimes
                    .insert(root.clone(), import_watcher_root_mtime(root));
                next_roots.insert(root.clone());
            } else {
                event_errors += 1;
            }
        }

        self.watched_roots = next_roots;
        ImportWatcherSummary {
            active_roots: (self.watched_roots != previous_roots)
                .then_some(self.watched_roots.len()),
            event_errors,
            ..ImportWatcherSummary::default()
        }
    }

    fn drain_events(
        &mut self,
        scopes_by_root: &BTreeMap<String, ImportScanScope>,
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
                        if let Some(root) = import_watcher_root_for_path(scopes_by_root, &path) {
                            self.pending_roots.insert(root.to_string());
                        }
                    }
                }
                Ok(Err(_)) => {
                    summary.event_errors += 1;
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    summary.event_errors += 1;
                    return;
                }
            }
        }
    }

    fn poll_changed_roots(&mut self, roots: &BTreeSet<String>, summary: &mut ImportWatcherSummary) {
        for root in roots {
            if !self.watched_roots.contains(root) {
                continue;
            }
            let previous_mtime = self.watched_root_mtimes.get(root).copied().flatten();
            let current_mtime = import_watcher_root_mtime(root);
            self.watched_root_mtimes.insert(root.clone(), current_mtime);
            if previous_mtime.is_some() && current_mtime != previous_mtime {
                summary.events += 1;
                self.pending_roots.insert(root.clone());
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct ImportWatcherSummary {
    pub(crate) active_roots: Option<usize>,
    pub(crate) events: usize,
    pub(crate) requeued: usize,
    pub(crate) event_errors: usize,
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
    scopes_by_root: &'a BTreeMap<String, ImportScanScope>,
    event_path: &Path,
) -> Option<&'a str> {
    scopes_by_root
        .keys()
        .find(|root| event_path.starts_with(Path::new(root.as_str())))
        .map(String::as_str)
}

fn import_watcher_root_mtime(root: &str) -> Option<u128> {
    fs::metadata(root)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
}
