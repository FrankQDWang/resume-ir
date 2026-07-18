use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind};
use std::path::Path;

const SEARCH_PUBLICATION_LOCK_FILE: &str = "search-publication.lock";
#[cfg(windows)]
const FILE_SHARE_READ: u32 = 0x0000_0001;

#[must_use = "dropping the guard releases exclusive search publication ownership"]
pub(super) struct SearchPublicationLock {
    file: File,
}

impl SearchPublicationLock {
    pub(super) fn acquire(data_dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(data_dir)?;
        let path = data_dir.join(SEARCH_PUBLICATION_LOCK_FILE);
        let file = open_lock_file(&path)?;
        file.lock()?;
        validate_open_lock_file(&path, &file)?;
        Ok(Self { file })
    }
}

impl Drop for SearchPublicationLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn open_lock_file(path: &Path) -> io::Result<File> {
    validate_existing_lock_path(path)?;
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(FILE_SHARE_READ);
    }
    let file = options.open(path)?;
    validate_open_lock_file(path, &file)?;
    Ok(file)
}

fn validate_existing_lock_path(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => validate_lock_metadata(&metadata),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn validate_open_lock_file(path: &Path, file: &File) -> io::Result<()> {
    let opened = file.metadata()?;
    validate_lock_metadata(&opened)?;
    let current = fs::symlink_metadata(path)?;
    validate_lock_metadata(&current)?;
    if !same_file_identity(file, path, &opened, &current)? {
        return Err(io::Error::other(
            "search publication lock identity changed during open",
        ));
    }
    Ok(())
}

fn validate_lock_metadata(metadata: &fs::Metadata) -> io::Result<()> {
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(io::Error::other(
            "search publication lock must be a regular non-symlink file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o777 != 0o600 {
            return Err(io::Error::other(
                "search publication lock must be owner-only read-write",
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn same_file_identity(
    _file: &File,
    _path: &Path,
    opened: &fs::Metadata,
    current: &fs::Metadata,
) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    Ok(opened.dev() == current.dev() && opened.ino() == current.ino())
}

#[cfg(windows)]
fn same_file_identity(
    file: &File,
    path: &Path,
    _opened: &fs::Metadata,
    _current: &fs::Metadata,
) -> io::Result<bool> {
    let opened = same_file::Handle::from_file(file.try_clone()?)?;
    let current = same_file::Handle::from_path(path)?;
    let final_metadata = fs::symlink_metadata(path)?;
    validate_lock_metadata(&final_metadata)?;
    Ok(opened == current)
}

#[cfg(not(any(unix, windows)))]
fn same_file_identity(
    _file: &File,
    _path: &Path,
    _opened: &fs::Metadata,
    _current: &fs::Metadata,
) -> io::Result<bool> {
    Ok(false)
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs::{self, OpenOptions};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{SearchPublicationLock, SEARCH_PUBLICATION_LOCK_FILE};

    static NEXT_TEST_DIR_ID: AtomicU64 = AtomicU64::new(0);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = env::temp_dir().join(format!(
                "resume-ir-index-publication-lock-{}-{suffix}-{}",
                std::process::id(),
                NEXT_TEST_DIR_ID.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn different_data_directories_do_not_share_publication_lock() {
        let temp = TestDir::new();
        let first = temp.0.join("first");
        let second = temp.0.join("second");

        let _first_lock = SearchPublicationLock::acquire(&first).unwrap();
        let _second_lock = SearchPublicationLock::acquire(&second).unwrap();
    }

    #[test]
    fn same_data_directory_rejects_a_second_publication_owner() {
        let temp = TestDir::new();
        let data_dir = temp.0.join("shared");
        let _owner = SearchPublicationLock::acquire(&data_dir).unwrap();
        let contender = OpenOptions::new()
            .read(true)
            .write(true)
            .open(data_dir.join(SEARCH_PUBLICATION_LOCK_FILE));

        #[cfg(windows)]
        assert!(contender.is_err());
        #[cfg(not(windows))]
        assert!(matches!(
            contender.unwrap().try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
    }

    #[test]
    fn publication_lock_can_be_reacquired_after_owner_release() {
        let temp = TestDir::new();
        let data_dir = temp.0.join("shared");
        let owner = SearchPublicationLock::acquire(&data_dir).unwrap();

        drop(owner);

        let _next_owner = SearchPublicationLock::acquire(&data_dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn publication_lock_rejects_existing_non_private_file_without_mutating_it() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TestDir::new();
        let data_dir = temp.0.join("shared");
        fs::create_dir_all(&data_dir).unwrap();
        let path = data_dir.join(SEARCH_PUBLICATION_LOCK_FILE);
        fs::write(&path, []).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();

        assert!(SearchPublicationLock::acquire(&data_dir).is_err());

        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o666
        );
    }

    #[cfg(unix)]
    #[test]
    fn publication_lock_rejects_symlink_without_touching_target() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let temp = TestDir::new();
        let data_dir = temp.0.join("shared");
        fs::create_dir_all(&data_dir).unwrap();
        let target = temp.0.join("target");
        fs::write(&target, b"sentinel").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
        symlink(&target, data_dir.join(SEARCH_PUBLICATION_LOCK_FILE)).unwrap();

        assert!(SearchPublicationLock::acquire(&data_dir).is_err());

        assert_eq!(fs::read(&target).unwrap(), b"sentinel");
        assert_eq!(
            fs::metadata(target).unwrap().permissions().mode() & 0o777,
            0o644
        );
    }
}
