use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicationArtifact {
    FullText,
    Vector,
}

impl PublicationArtifact {
    fn root_name(self) -> &'static str {
        match self {
            Self::FullText => "search-index",
            Self::Vector => "vector-index",
        }
    }
}

/// Test-only owner of one exact artifact publication lock.
///
/// Holding this gate gives cross-process tests a causal barrier after a
/// durable repair attempt starts and before a replacement artifact can be
/// published. Its debug form deliberately carries no filesystem identity.
pub struct PublicationGate {
    file: File,
    held: bool,
}

impl PublicationGate {
    pub fn acquire(data_dir: &Path, artifact: PublicationArtifact) -> io::Result<Self> {
        let root = data_dir.join(artifact.root_name());
        create_owner_only_directory(&root)?;
        let lock_path = root.join("snapshot-publication.lock");
        let mut options = OpenOptions::new();
        options.create_new(true).read(true).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options.open(&lock_path)?;
        #[cfg(unix)]
        fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o600))?;
        validate_owner_only_regular_file(&lock_path)?;
        file.lock()?;
        Ok(Self { file, held: true })
    }

    pub fn is_held(&self) -> bool {
        self.held
    }

    pub fn release(mut self) -> io::Result<()> {
        self.file.unlock()?;
        self.held = false;
        Ok(())
    }
}

impl fmt::Debug for PublicationGate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PublicationGate(<redacted>)")
    }
}

impl Drop for PublicationGate {
    fn drop(&mut self) {
        if self.held {
            let _ = self.file.unlock();
        }
    }
}

fn create_owner_only_directory(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => return Err(io::Error::other("publication gate root is not a directory")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            builder.mode(0o700);
            builder.create(path)?;
        }
        Err(error) => return Err(error),
    }
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    validate_owner_only_directory(path)
}

fn validate_owner_only_directory(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_dir() {
        return Err(io::Error::other("publication gate root is not a directory"));
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o777 != 0o700 {
        return Err(io::Error::other("publication gate root is not owner-only"));
    }
    Ok(())
}

fn validate_owner_only_regular_file(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(io::Error::other(
            "publication gate lock is not a regular file",
        ));
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o777 != 0o600 {
        return Err(io::Error::other("publication gate lock is not owner-only"));
    }
    Ok(())
}
