use super::*;
use crate::codec::{write_snapshot, KEY_FILE, MANIFEST_FILE};
use crate::private_storage::create_private_directory;
use crate::{
    commit_snapshot_gc, VectorDocumentIdentity, VectorIndexError, VectorModelContract,
    VectorSnapshotGcCommitReport, VectorSnapshotGcPreparation, VectorSnapshotRoot,
    VectorSnapshotUpdate,
};
use core_domain::{ActiveSearchProjection, DocumentId, ResumeVersionId};
use std::cell::Cell;
#[cfg(unix)]
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::str::FromStr;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[test]
fn corrupted_staging_never_becomes_a_published_generation() {
    let root = temp_dir("corrupt-staging");
    let contract = VectorModelContract::enabled("model", 4).unwrap();
    let store = VectorSnapshotStore::new(&root, contract.clone()).unwrap();
    let stable = document("stable", "stable", "stable");
    store
        .publish_generation("generation-stable", [projection(&stable)], [stable])
        .unwrap();
    store.prepare_layout().unwrap();
    let publication_lease =
        VectorSnapshotPublicationLease::acquire(&store.root, &store.root_identity).unwrap();

    let staging = root.join("staging/generation-next.injected");
    create_private_directory(&staging).unwrap();
    let staging = PinnedPrivateDirectory::acquire(&staging).unwrap();
    let next = document("next", "next", "next");
    let expected = write_snapshot(
        staging.path(),
        &root.join(KEY_FILE),
        "generation-next",
        &contract,
        &[projection(&next)],
        &[next],
    )
    .unwrap();
    fs::write(
        staging.path().join(MANIFEST_FILE),
        b"{\"schema_version\":\"vector.snapshot.v2\"}\n",
    )
    .unwrap();

    assert_eq!(
        store
            .validate_and_publish_staging(
                "generation-next",
                expected,
                &staging,
                &root.join("snapshots/generation-next"),
                &publication_lease,
            )
            .unwrap_err(),
        VectorIndexError::CorruptSnapshot
    );
    assert!(!root.join("snapshots/generation-next").exists());
    let snapshot_root = VectorSnapshotRoot::new(&root).unwrap();
    let lease = snapshot_root.acquire_read_lease().unwrap();
    assert!(snapshot_root
        .open_generation_with_lease("generation-stable", &contract, lease)
        .is_ok());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn failed_staging_cleanup_never_overwrites_the_primary_error() {
    let cleanup_class = Cell::new(None);
    let result: Result<(), VectorIndexError> = preserve_primary_after_cleanup(
        Err(VectorIndexError::CorruptSnapshot),
        || Err(VectorIndexError::StorageLayoutInvalid),
        |class| cleanup_class.set(Some(class)),
    );

    assert_eq!(result.unwrap_err(), VectorIndexError::CorruptSnapshot);
    assert_eq!(
        cleanup_class.get(),
        Some(FailedStagingCleanupClass::LayoutChanged)
    );
}

#[cfg(unix)]
#[test]
fn staging_identity_replacement_preserves_primary_and_does_not_delete_replacement() {
    let root = temp_dir("staging-identity-replacement");
    let contract = VectorModelContract::enabled("model", 4).unwrap();
    let store = VectorSnapshotStore::new(&root, contract).unwrap();
    let next = document("next", "next", "next");
    let replacement = RefCell::new(None);

    let error = store
        .publish_generation_with_staging_observer(
            "generation-next",
            [projection(&next)],
            [next],
            |staging| {
                let displaced = staging.with_extension("displaced");
                fs::rename(staging, &displaced).unwrap();
                create_private_directory(staging).unwrap();
                fs::write(staging.join("replacement-marker"), b"replacement").unwrap();
                replacement.replace(Some(staging.to_path_buf()));
            },
        )
        .unwrap_err();

    assert_eq!(error, VectorIndexError::StorageLayoutInvalid);
    let replacement = replacement.into_inner().unwrap();
    assert_eq!(
        fs::read(replacement.join("replacement-marker")).unwrap(),
        b"replacement"
    );
    assert!(!root.join("snapshots/generation-next").exists());
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn published_generation_must_keep_the_original_staging_identity() {
    let root = temp_dir("published-generation-identity");
    let contract = VectorModelContract::enabled("model", 4).unwrap();
    let store = VectorSnapshotStore::new(&root, contract.clone()).unwrap();
    store.prepare_layout().unwrap();
    let publication_lease =
        VectorSnapshotPublicationLease::acquire(&store.root, &store.root_identity).unwrap();
    let staging_path = root.join("staging/generation-next.injected");
    create_private_directory(&staging_path).unwrap();
    let staging = PinnedPrivateDirectory::acquire(&staging_path).unwrap();
    let next = document("next", "next", "next");
    let expected = write_snapshot(
        staging.path(),
        &root.join(KEY_FILE),
        "generation-next",
        &contract,
        &[projection(&next)],
        &[next],
    )
    .unwrap();
    let published = root.join("snapshots/generation-next");
    let displaced = root.join("generation-next-original");

    let error = store
        .validate_and_publish_staging_with_observer(
            "generation-next",
            expected,
            &staging,
            &published,
            &publication_lease,
            |published| {
                fs::rename(published, &displaced).unwrap();
                create_private_directory(published).unwrap();
                fs::write(published.join("replacement-marker"), b"replacement").unwrap();
            },
        )
        .unwrap_err();

    assert_eq!(error, VectorIndexError::StorageLayoutInvalid);
    assert_eq!(
        fs::read(published.join("replacement-marker")).unwrap(),
        b"replacement"
    );
    assert!(displaced.join(MANIFEST_FILE).is_file());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn publication_lease_protects_live_staging_from_gc_until_exact_open() {
    let root = temp_dir("live-staging-publication-lease");
    let contract = VectorModelContract::enabled("model", 4).unwrap();
    let store = VectorSnapshotStore::new(&root, contract.clone()).unwrap();
    let next = document("next", "next", "next");
    let active_projection = [projection(&next)];
    let (staging_ready_tx, staging_ready_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let publisher = store.clone();
    let publish = thread::spawn(move || {
        publisher.publish_generation_with_staging_observer(
            "generation-next",
            active_projection,
            [next],
            |staging| {
                staging_ready_tx.send(staging.to_path_buf()).unwrap();
                resume_rx.recv().unwrap();
            },
        )
    });

    let live_staging = staging_ready_rx.recv().unwrap();
    assert!(live_staging.is_dir());
    let snapshot_root = VectorSnapshotRoot::new(&root).unwrap();
    assert!(snapshot_root.try_acquire_snapshot_gc().unwrap().is_none());
    assert!(live_staging.is_dir());

    resume_tx.send(()).unwrap();
    let summary = publish.join().unwrap().unwrap();
    assert_eq!(summary.generation(), "generation-next");
    assert!(!live_staging.exists());
    let lease = snapshot_root.acquire_read_lease().unwrap();
    assert!(snapshot_root
        .open_generation_with_lease("generation-next", &contract, lease)
        .is_ok());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn publish_from_releases_base_pin_before_waiting_for_publication() {
    let root = temp_dir("publish-from-lock-order");
    let contract = VectorModelContract::enabled("model", 4).unwrap();
    let store = VectorSnapshotStore::new(&root, contract.clone()).unwrap();
    let base_document = document("base", "base", "base");
    store
        .publish_generation(
            "generation-base",
            [projection(&base_document)],
            [base_document.clone()],
        )
        .unwrap();
    let snapshot_root = VectorSnapshotRoot::new(&root).unwrap();
    let read_lease = snapshot_root.acquire_read_lease().unwrap();
    let base_reader = snapshot_root
        .open_generation_with_lease("generation-base", &contract, read_lease)
        .unwrap();
    let gc_acquisition = snapshot_root.try_acquire_snapshot_gc().unwrap().unwrap();
    let update = VectorSnapshotUpdate::new(
        vec![projection(&base_document)],
        Vec::new(),
        BTreeSet::new(),
    )
    .unwrap();
    let publisher_store = store.clone();
    let (publisher_started_tx, publisher_started_rx) = mpsc::channel();
    let publisher = thread::spawn(move || {
        publisher_started_tx.send(()).unwrap();
        publisher_store.publish_generation_from(base_reader, "generation-next", update)
    });
    publisher_started_rx.recv().unwrap();

    let generation_pin =
        open_lock_file(&generation_pin_path(&store.root, "generation-base"), false).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let base_pin_released = loop {
        match generation_pin.try_lock_exclusive() {
            Ok(true) => break true,
            Ok(false) if Instant::now() < deadline => thread::yield_now(),
            Ok(false) => break false,
            Err(error) => panic!("generation pin probe failed: {error}"),
        }
    };
    if !base_pin_released {
        drop(gc_acquisition);
        let _ = publisher.join();
        panic!("publisher waited for publication while retaining the base generation pin");
    }
    generation_pin.unlock().unwrap();

    let VectorSnapshotGcPreparation::Prepared(prepared) = snapshot_root
        .prepare_snapshot_gc(gc_acquisition, &BTreeSet::new())
        .unwrap()
    else {
        panic!("GC unexpectedly deferred");
    };
    let VectorSnapshotGcCommitReport::Complete(removed) = commit_snapshot_gc(prepared) else {
        panic!("GC unexpectedly failed");
    };
    assert_eq!(removed.removed_generations(), 1);
    let published = publisher.join().unwrap().unwrap();
    assert_eq!(published.generation(), "generation-next");
    assert!(!root.join("snapshots/generation-base").exists());
    let lease = snapshot_root.acquire_read_lease().unwrap();
    assert!(snapshot_root
        .open_generation_with_lease("generation-next", &contract, lease)
        .is_ok());
    let _ = fs::remove_dir_all(root);
}

fn projection(document: &VectorDocument) -> ActiveSearchProjection {
    ActiveSearchProjection {
        document_id: DocumentId::from_str(document.document_id()).unwrap(),
        resume_version_id: ResumeVersionId::from_str(document.resume_version_id()).unwrap(),
    }
}

fn document(vector: &str, document: &str, version: &str) -> VectorDocument {
    let identity = VectorDocumentIdentity::new(
        stable_id("vec_", vector),
        stable_id("doc_", document),
        stable_id("ver_", version),
        "model",
    )
    .unwrap();
    VectorDocument::new(identity, vec![1.0, 0.0, 0.0, 0.0]).unwrap()
}

fn stable_id(prefix: &str, part: &str) -> String {
    let mut first = 0xcbf2_9ce4_8422_2325_u64;
    let mut second = 0x6c62_272e_07bb_0142_u64;
    for byte in part.bytes() {
        first = (first ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        second = (second ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{prefix}{first:016x}{second:016x}")
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-vector-unit-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    #[cfg(unix)]
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}
