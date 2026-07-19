use std::fs::File;
use std::io;

const WINDOWS_ERROR_SHARING_VIOLATION: i32 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ExclusiveLockAttempt {
    Acquired,
    Contended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LockPlatform {
    Windows,
    Other,
}

impl LockPlatform {
    const fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Other
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LockOpenErrorClass {
    Contended,
    Storage,
}

pub(super) const fn classify_open_error(
    platform: LockPlatform,
    raw_os_error: Option<i32>,
) -> LockOpenErrorClass {
    match (platform, raw_os_error) {
        (LockPlatform::Windows, Some(WINDOWS_ERROR_SHARING_VIOLATION)) => {
            LockOpenErrorClass::Contended
        }
        (LockPlatform::Windows | LockPlatform::Other, _) => LockOpenErrorClass::Storage,
    }
}

pub(super) fn classify_current_open_error(error: &io::Error) -> LockOpenErrorClass {
    classify_open_error(LockPlatform::current(), error.raw_os_error())
}

pub(super) fn try_exclusive(file: &File) -> io::Result<ExclusiveLockAttempt> {
    fs4::fs_std::FileExt::try_lock_exclusive(file).map(|acquired| {
        if acquired {
            ExclusiveLockAttempt::Acquired
        } else {
            ExclusiveLockAttempt::Contended
        }
    })
}

pub(super) fn lock_exclusive(file: &File) -> io::Result<()> {
    fs4::fs_std::FileExt::lock_exclusive(file)
}

pub(super) fn unlock(file: &File) -> io::Result<()> {
    fs4::fs_std::FileExt::unlock(file)
}
