use super::lock_ops::{
    classify_open_error, try_exclusive, unlock, ExclusiveLockAttempt, LockOpenErrorClass,
    LockPlatform,
};
use super::private_lock_options;

#[test]
fn legacy_fs4_owner_is_reported_as_contention() {
    let directory = tempfile::tempdir().unwrap();
    let lock_path = directory.path().join("legacy-owner.lock");
    let holder = private_lock_options().open(&lock_path).unwrap();
    let contender = private_lock_options().open(&lock_path).unwrap();
    fs4::fs_std::FileExt::lock_exclusive(&holder).unwrap();

    assert_eq!(
        try_exclusive(&contender).unwrap(),
        ExclusiveLockAttempt::Contended
    );

    fs4::fs_std::FileExt::unlock(&holder).unwrap();
    assert_eq!(
        try_exclusive(&contender).unwrap(),
        ExclusiveLockAttempt::Acquired
    );
    unlock(&contender).unwrap();
}

#[test]
fn windows_sharing_violation_is_the_only_open_contention_code() {
    assert_eq!(
        classify_open_error(LockPlatform::Windows, Some(32)),
        LockOpenErrorClass::Contended
    );
    assert_eq!(
        classify_open_error(LockPlatform::Windows, Some(5)),
        LockOpenErrorClass::Storage
    );
    assert_eq!(
        classify_open_error(LockPlatform::Other, Some(32)),
        LockOpenErrorClass::Storage
    );
    assert_eq!(
        classify_open_error(LockPlatform::Windows, None),
        LockOpenErrorClass::Storage
    );
}
