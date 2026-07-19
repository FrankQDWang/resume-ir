use std::sync::mpsc;
use std::thread;

use meta_store::{DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, UnixTimestamp};
use tempfile::tempdir;

use super::reconcile_search_artifacts;
use crate::SearchPublicationVectorization;

#[test]
fn maintenance_defers_when_a_foreground_publication_owns_the_session() {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let owner = match DataDirectoryOwnerLease::try_acquire(&data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data dir is owned"),
    };
    let store = owner.open_store().unwrap();
    store.run_migrations().unwrap();
    let holder_store = store.open_sibling().unwrap();
    let (acquired_sender, acquired_receiver) = mpsc::sync_channel(1);
    let (release_sender, release_receiver) = mpsc::sync_channel(1);

    thread::scope(|scope| {
        scope.spawn(move || {
            let _publication_session = holder_store.wait_for_search_publication_session().unwrap();
            acquired_sender.send(()).unwrap();
            release_receiver.recv().unwrap();
        });
        acquired_receiver.recv().unwrap();

        let summary = reconcile_search_artifacts(
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_000),
            &SearchPublicationVectorization::default(),
        )
        .unwrap();

        assert_eq!(summary, Default::default());
        release_sender.send(()).unwrap();
    });
}
