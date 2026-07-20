use super::*;

use std::os::unix::fs::PermissionsExt;
use std::str::FromStr;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use core_domain::{ActiveSearchProjection, DocumentId, ResumeVersionId};

use crate::private_storage::create_private_directory;
use crate::{VectorDocument, VectorModelContract, VectorSnapshotStore};

#[test]
fn exact_retirement_defers_for_a_reader_and_never_collects_unrelated_artifacts() {
    let root = temp_dir("exact-generation-retirement");
    let store = VectorSnapshotStore::new(&root, VectorModelContract::Disabled).unwrap();
    for generation in [
        "generation-target",
        "generation-retained",
        "generation-unrelated",
    ] {
        publish_disabled(&store, generation);
    }
    for path in [
        root.join("staging/generation-target.tmp-0123456789abcdef01234567"),
        root.join("staging/generation-unrelated.tmp-0123456789abcdef01234567"),
    ] {
        create_private_directory(&path).unwrap();
    }
    let retained = BTreeSet::from(["generation-retained".to_string()]);
    let snapshot_root = VectorSnapshotRoot::new(&root).unwrap();
    let lease = snapshot_root.acquire_read_lease().unwrap();
    let reader = snapshot_root
        .open_generation_with_lease("generation-target", &VectorModelContract::Disabled, lease)
        .unwrap();

    assert_eq!(
        try_retire_unpublished_generation(&root, "generation-target", &retained).unwrap(),
        VectorGenerationRetirement::Deferred
    );
    assert!(root.join("snapshots/generation-target").is_dir());
    drop(reader);

    let VectorGenerationRetirement::Retired(summary) =
        try_retire_unpublished_generation(&root, "generation-target", &retained).unwrap()
    else {
        panic!("exact target was not retired");
    };
    assert!(summary.removed_generation());
    assert_eq!(summary.removed_staging(), 1);
    assert!(summary.removed_generation_pin());
    assert!(!root.join("snapshots/generation-target").exists());
    assert!(!root.join("generation-pins/generation-target.lock").exists());
    for path in [
        root.join("snapshots/generation-retained"),
        root.join("snapshots/generation-unrelated"),
        root.join("staging/generation-unrelated.tmp-0123456789abcdef01234567"),
        root.join("generation-pins/generation-retained.lock"),
        root.join("generation-pins/generation-unrelated.lock"),
    ] {
        assert!(
            path.exists(),
            "unrelated exact-generation artifact was removed"
        );
    }
    assert_eq!(
        try_retire_unpublished_generation(&root, "generation-target", &retained).unwrap(),
        VectorGenerationRetirement::Absent
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn exact_retirement_rejects_a_metadata_retained_target() {
    let root = temp_dir("exact-retained-rejected");
    let store = VectorSnapshotStore::new(&root, VectorModelContract::Disabled).unwrap();
    publish_disabled(&store, "generation-retained");
    let retained = BTreeSet::from(["generation-retained".to_string()]);

    assert_eq!(
        try_retire_unpublished_generation(&root, "generation-retained", &retained).unwrap_err(),
        VectorIndexError::StorageLayoutInvalid
    );
    assert!(root.join("snapshots/generation-retained").is_dir());
    assert!(root
        .join("generation-pins/generation-retained.lock")
        .is_file());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn prepared_gc_releases_root_fence_before_commit() {
    let root = temp_dir("prepared-reader-entry");
    let store = VectorSnapshotStore::new(&root, VectorModelContract::Disabled).unwrap();
    for generation in ["generation-old", "generation-retained"] {
        publish_disabled(&store, generation);
    }
    let snapshot_root = VectorSnapshotRoot::new(&root).unwrap();
    let retained = BTreeSet::from(["generation-retained".to_string()]);
    let acquisition = snapshot_root.try_acquire_snapshot_gc().unwrap().unwrap();
    let VectorSnapshotGcPreparation::Prepared(prepared) = snapshot_root
        .prepare_snapshot_gc(acquisition, &retained)
        .unwrap()
    else {
        panic!("GC unexpectedly deferred");
    };

    let reader_root = snapshot_root.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        let lease = reader_root.acquire_read_lease().unwrap();
        let opened = reader_root
            .open_generation_with_lease(
                "generation-retained",
                &VectorModelContract::Disabled,
                lease,
            )
            .unwrap();
        entered_tx.send(opened.generation().to_string()).unwrap();
    });
    assert_eq!(
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        "generation-retained"
    );
    reader.join().unwrap();

    let VectorSnapshotGcCommitReport::Complete(summary) = commit_snapshot_gc(prepared) else {
        panic!("GC unexpectedly failed");
    };
    assert_eq!(summary.removed_generations(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn late_generation_identity_replacement_reports_zero_progress() {
    let root = temp_dir("late-generation-identity-replacement");
    let store = VectorSnapshotStore::new(&root, VectorModelContract::Disabled).unwrap();
    for generation in [
        "generation-a-free",
        "generation-b-retained",
        "generation-z-replaced",
    ] {
        publish_disabled(&store, generation);
    }
    let snapshot_root = VectorSnapshotRoot::new(&root).unwrap();
    let retained = BTreeSet::from(["generation-b-retained".to_string()]);
    let acquisition = snapshot_root.try_acquire_snapshot_gc().unwrap().unwrap();
    let VectorSnapshotGcPreparation::Prepared(prepared) = snapshot_root
        .prepare_snapshot_gc(acquisition, &retained)
        .unwrap()
    else {
        panic!("GC unexpectedly deferred");
    };
    let replaced = root.join("snapshots/generation-z-replaced");
    let displaced = root.join("generation-z-original");
    fs::rename(&replaced, &displaced).unwrap();
    create_private_directory(&replaced).unwrap();
    fs::write(replaced.join("replacement-marker"), b"replacement").unwrap();

    let VectorSnapshotGcCommitReport::PartialFailure(failure) = commit_snapshot_gc(prepared) else {
        panic!("identity replacement unexpectedly committed");
    };
    assert_eq!(
        failure.failure_class(),
        VectorSnapshotGcFailureClass::LayoutChanged
    );
    assert_eq!(
        failure.failure_phase(),
        VectorSnapshotGcFailurePhase::Preflight
    );
    assert_eq!(failure.progress().removed_generations(), 0);
    assert!(root.join("snapshots/generation-a-free").is_dir());
    assert_eq!(
        fs::read(replaced.join("replacement-marker")).unwrap(),
        b"replacement"
    );
    assert!(displaced.is_dir());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn late_staging_identity_replacement_reports_zero_progress() {
    let root = temp_dir("late-staging-identity-replacement");
    let store = VectorSnapshotStore::new(&root, VectorModelContract::Disabled).unwrap();
    for generation in ["generation-a-free", "generation-b-retained"] {
        publish_disabled(&store, generation);
    }
    let replaced = root.join("staging/orphan.tmp-0123456789abcdef01234567");
    create_private_directory(&replaced).unwrap();
    let snapshot_root = VectorSnapshotRoot::new(&root).unwrap();
    let retained = BTreeSet::from(["generation-b-retained".to_string()]);
    let acquisition = snapshot_root.try_acquire_snapshot_gc().unwrap().unwrap();
    let VectorSnapshotGcPreparation::Prepared(prepared) = snapshot_root
        .prepare_snapshot_gc(acquisition, &retained)
        .unwrap()
    else {
        panic!("GC unexpectedly deferred");
    };
    let displaced = root.join("staging-original");
    fs::rename(&replaced, &displaced).unwrap();
    create_private_directory(&replaced).unwrap();
    fs::write(replaced.join("replacement-marker"), b"replacement").unwrap();

    let VectorSnapshotGcCommitReport::PartialFailure(failure) = commit_snapshot_gc(prepared) else {
        panic!("identity replacement unexpectedly committed");
    };
    assert_eq!(
        failure.failure_class(),
        VectorSnapshotGcFailureClass::LayoutChanged
    );
    assert_eq!(
        failure.failure_phase(),
        VectorSnapshotGcFailurePhase::Preflight
    );
    assert_eq!(failure.progress().removed_generations(), 0);
    assert!(root.join("snapshots/generation-a-free").is_dir());
    assert_eq!(
        fs::read(replaced.join("replacement-marker")).unwrap(),
        b"replacement"
    );
    assert!(displaced.is_dir());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn partial_commit_is_reported_and_the_next_attempt_converges() {
    let root = temp_dir("partial-commit-converges");
    let store = VectorSnapshotStore::new(&root, VectorModelContract::Disabled).unwrap();
    for generation in [
        "generation-a-free",
        "generation-b-retained",
        "generation-z-free",
    ] {
        publish_disabled(&store, generation);
    }
    let snapshot_root = VectorSnapshotRoot::new(&root).unwrap();
    let retained = BTreeSet::from(["generation-b-retained".to_string()]);
    let acquisition = snapshot_root.try_acquire_snapshot_gc().unwrap().unwrap();
    let VectorSnapshotGcPreparation::Prepared(prepared) = snapshot_root
        .prepare_snapshot_gc(acquisition, &retained)
        .unwrap()
    else {
        panic!("GC unexpectedly deferred");
    };
    let VectorSnapshotGcCommitReport::PartialFailure(failure) =
        commit_snapshot_gc_with_observer(prepared, |removed| {
            if removed == 1 {
                Err(VectorIndexError::Storage)
            } else {
                Ok(())
            }
        })
    else {
        panic!("fault injection unexpectedly completed");
    };
    assert_eq!(failure.progress().removed_generations(), 1);
    assert_eq!(failure.remaining_generations(), 1);
    assert_eq!(
        failure.failure_class(),
        VectorSnapshotGcFailureClass::StorageUnavailable
    );
    assert_eq!(
        failure.failure_phase(),
        VectorSnapshotGcFailurePhase::GenerationRemoval
    );

    let acquisition = snapshot_root.try_acquire_snapshot_gc().unwrap().unwrap();
    let VectorSnapshotGcPreparation::Prepared(prepared) = snapshot_root
        .prepare_snapshot_gc(acquisition, &retained)
        .unwrap()
    else {
        panic!("retry unexpectedly deferred");
    };
    let VectorSnapshotGcCommitReport::Complete(summary) = commit_snapshot_gc(prepared) else {
        panic!("retry did not converge");
    };
    assert_eq!(summary.removed_generations(), 1);
    for generation in ["generation-a-free", "generation-z-free"] {
        assert!(!root.join("snapshots").join(generation).exists());
        assert!(!root
            .join("generation-pins")
            .join(format!("{generation}.lock"))
            .exists());
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn controlled_commit_interrupts_between_generation_removals() {
    let root = temp_dir("controlled-commit-interrupts");
    let store = VectorSnapshotStore::new(&root, VectorModelContract::Disabled).unwrap();
    for generation in [
        "generation-a-free",
        "generation-b-retained",
        "generation-z-free",
    ] {
        publish_disabled(&store, generation);
    }
    let snapshot_root = VectorSnapshotRoot::new(&root).unwrap();
    let retained = BTreeSet::from(["generation-b-retained".to_string()]);
    let acquisition = snapshot_root.try_acquire_snapshot_gc().unwrap().unwrap();
    let VectorSnapshotGcPreparation::Prepared(prepared) = snapshot_root
        .prepare_snapshot_gc(acquisition, &retained)
        .unwrap()
    else {
        panic!("GC unexpectedly deferred");
    };
    let checks = std::cell::Cell::new(0_usize);
    let cancel_check = || {
        let next = checks.get() + 1;
        checks.set(next);
        next >= 3
    };

    let VectorSnapshotGcCommitReport::Interrupted(progress) =
        commit_snapshot_gc_with_cancel_check(prepared, &cancel_check)
    else {
        panic!("controlled GC did not interrupt");
    };
    assert_eq!(progress.removed_generations(), 1);
    assert_eq!(checks.get(), 3);
    assert_eq!(
        ["generation-a-free", "generation-z-free"]
            .into_iter()
            .filter(|generation| root.join("snapshots").join(generation).exists())
            .count(),
        1
    );
    let _ = fs::remove_dir_all(root);
}

fn publish_disabled(store: &VectorSnapshotStore, generation: &str) {
    store
        .publish_generation(
            generation,
            [projection(generation)],
            std::iter::empty::<VectorDocument>(),
        )
        .unwrap();
}

fn projection(seed: &str) -> ActiveSearchProjection {
    ActiveSearchProjection {
        document_id: DocumentId::from_str(&stable_id("doc_", seed)).unwrap(),
        resume_version_id: ResumeVersionId::from_str(&stable_id("ver_", seed)).unwrap(),
    }
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
    let path = std::env::temp_dir().join(format!("resume-ir-vector-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}
