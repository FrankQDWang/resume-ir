use crate::codec::{
    read_snapshot_with_control, write_snapshot_with_control, KEY_FILE, MANIFEST_FILE, SNAPSHOT_FILE,
};
use crate::model::{validate_documents_with_control, VectorDocument, VectorIndexError};
use crate::model_contract::VectorModelContract;
use crate::private_storage::{
    create_private_directory, random_suffix, same_open_file_identity, sync_directory,
    PinnedPrivateDirectory,
};
use crate::snapshot_model::{VectorSnapshotSummary, VectorSnapshotUpdate};
use crate::snapshot_root::VectorSnapshotReader;
use crate::VectorSnapshotPublishControl;
use core_domain::ActiveSearchProjection;
use fs4::fs_std::FileExt;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;

pub(crate) const SNAPSHOTS_DIR: &str = "snapshots";
pub(crate) const STAGING_DIR: &str = "staging";
pub(crate) const GENERATION_PINS_DIR: &str = "generation-pins";
pub(crate) const READER_LOCK_FILE: &str = "snapshot-readers.lock";
pub(crate) const PUBLICATION_LOCK_FILE: &str = "snapshot-publication.lock";
const MAX_GENERATION_LEN: usize = 128;
#[cfg(windows)]
const FILE_FLAG_WRITE_THROUGH: u32 = 0x8000_0000;

#[derive(Clone)]
pub struct VectorSnapshotStore {
    root: PathBuf,
    root_identity: Arc<PinnedPrivateDirectory>,
    model_contract: VectorModelContract,
}

#[derive(Clone, Copy)]
struct StagedPublication<'a> {
    generation: &'a str,
    staging: &'a PinnedPrivateDirectory,
    published: &'a Path,
    lease: &'a VectorSnapshotPublicationLease,
}

impl VectorSnapshotStore {
    pub fn new(
        root: impl AsRef<Path>,
        model_contract: VectorModelContract,
    ) -> Result<Self, VectorIndexError> {
        model_contract.validate()?;
        match fs::symlink_metadata(root.as_ref()) {
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {
                create_private_directory(root.as_ref())?;
                let parent = root.as_ref().parent().ok_or(VectorIndexError::Storage)?;
                sync_directory(parent)?;
            }
            Err(_) => return Err(VectorIndexError::Storage),
        }
        let root = canonical_index_root(root.as_ref())?;
        let root_identity = Arc::new(PinnedPrivateDirectory::acquire(&root)?);
        Ok(Self {
            root,
            root_identity,
            model_contract,
        })
    }

    /// Builds, validates, and atomically publishes one immutable generation.
    ///
    /// Publication does not make this generation active. The metadata store
    /// must atomically select it after both full-text and vector validation.
    pub fn publish_generation<P, I>(
        &self,
        generation: &str,
        active_projection: P,
        documents: I,
    ) -> Result<VectorSnapshotSummary, VectorIndexError>
    where
        P: IntoIterator<Item = ActiveSearchProjection>,
        I: IntoIterator<Item = VectorDocument>,
    {
        self.publish_generation_with_control(
            generation,
            active_projection,
            documents,
            VectorSnapshotPublishControl::disabled(),
        )
    }

    /// Builds and publishes one generation with cooperative cancellation at
    /// record batches and crash-safe phase boundaries.
    pub fn publish_generation_with_control<P, I>(
        &self,
        generation: &str,
        active_projection: P,
        documents: I,
        control: VectorSnapshotPublishControl<'_>,
    ) -> Result<VectorSnapshotSummary, VectorIndexError>
    where
        P: IntoIterator<Item = ActiveSearchProjection>,
        I: IntoIterator<Item = VectorDocument>,
    {
        self.publish_generation_with_control_and_staging_observer(
            generation,
            active_projection,
            documents,
            control,
            |_| {},
        )
    }

    #[cfg(test)]
    fn publish_generation_with_staging_observer<P, I, O>(
        &self,
        generation: &str,
        active_projection: P,
        documents: I,
        staging_observer: O,
    ) -> Result<VectorSnapshotSummary, VectorIndexError>
    where
        P: IntoIterator<Item = ActiveSearchProjection>,
        I: IntoIterator<Item = VectorDocument>,
        O: FnOnce(&Path),
    {
        self.publish_generation_with_control_and_staging_observer(
            generation,
            active_projection,
            documents,
            VectorSnapshotPublishControl::disabled(),
            staging_observer,
        )
    }

    fn publish_generation_with_control_and_staging_observer<P, I, O>(
        &self,
        generation: &str,
        active_projection: P,
        documents: I,
        control: VectorSnapshotPublishControl<'_>,
        staging_observer: O,
    ) -> Result<VectorSnapshotSummary, VectorIndexError>
    where
        P: IntoIterator<Item = ActiveSearchProjection>,
        I: IntoIterator<Item = VectorDocument>,
        O: FnOnce(&Path),
    {
        validate_generation(generation)?;
        control.check()?;
        let active_projection = collect_with_control(active_projection, control)?;
        let documents = collect_with_control(documents, control)?;
        validate_documents_with_control(
            &self.model_contract,
            &active_projection,
            &documents,
            control,
        )?;
        control.check()?;
        self.prepare_layout()?;
        control.check()?;
        let publication_lease =
            VectorSnapshotPublicationLease::acquire(&self.root, &self.root_identity)?;
        publication_lease.validate_root(&self.root, &self.root_identity)?;
        control.check()?;
        let published = self.root.join(SNAPSHOTS_DIR).join(generation);
        reject_symlink_or_existing_generation(&published)?;

        let staging = self
            .root
            .join(STAGING_DIR)
            .join(format!("{generation}.tmp-{}", random_suffix()?));
        create_private_directory(&staging)?;
        let staging = PinnedPrivateDirectory::acquire(&staging)?;
        staging_observer(staging.path());
        control.check()?;
        let publication = StagedPublication {
            generation,
            staging: &staging,
            published: &published,
            lease: &publication_lease,
        };
        let result =
            self.build_validate_and_publish(&active_projection, &documents, publication, control);
        preserve_primary_after_cleanup(
            result,
            || self.cleanup_failed_staging(&staging, &publication_lease),
            |_| {},
        )
    }

    /// Derives and publishes a complete generation from one explicitly opened
    /// base generation plus a version-bound update plan. The reader is consumed
    /// and its generation pin is released before acquiring publication fences.
    pub fn publish_generation_from(
        &self,
        base: VectorSnapshotReader,
        generation: &str,
        update: VectorSnapshotUpdate,
    ) -> Result<VectorSnapshotSummary, VectorIndexError> {
        self.publish_generation_from_with_control(
            base,
            generation,
            update,
            VectorSnapshotPublishControl::disabled(),
        )
    }

    /// Materializes an exact-base update with cooperative cancellation, then
    /// releases the base reader before entering normal controlled publication.
    pub fn publish_generation_from_with_control(
        &self,
        base: VectorSnapshotReader,
        generation: &str,
        update: VectorSnapshotUpdate,
        control: VectorSnapshotPublishControl<'_>,
    ) -> Result<VectorSnapshotSummary, VectorIndexError> {
        if !base.belongs_to(&self.root) {
            return Err(VectorIndexError::LeaseRootMismatch);
        }
        if base.summary().model_contract() != &self.model_contract {
            return Err(VectorIndexError::InvalidModelContract);
        }
        control.check()?;
        let materialized = update.apply_with_control(base.documents(), control);
        drop(base);
        let (active_projection, documents) = materialized?;
        self.publish_generation_with_control(generation, active_projection, documents, control)
    }

    fn build_validate_and_publish(
        &self,
        active_projection: &[ActiveSearchProjection],
        documents: &[VectorDocument],
        publication: StagedPublication<'_>,
        control: VectorSnapshotPublishControl<'_>,
    ) -> Result<VectorSnapshotSummary, VectorIndexError> {
        control.check()?;
        publication.staging.validate_current()?;
        let expected = write_snapshot_with_control(
            publication.staging.path(),
            &publication.staging.path().join(KEY_FILE),
            publication.generation,
            &self.model_contract,
            active_projection,
            documents,
            control,
        )?;
        control.check()?;
        publication.staging.validate_current()?;
        self.validate_and_publish_staging_with_control(expected, publication, control)
    }

    #[cfg(test)]
    fn validate_and_publish_staging(
        &self,
        generation: &str,
        expected: VectorSnapshotSummary,
        staging: &PinnedPrivateDirectory,
        published: &Path,
        publication_lease: &VectorSnapshotPublicationLease,
    ) -> Result<VectorSnapshotSummary, VectorIndexError> {
        let publication = StagedPublication {
            generation,
            staging,
            published,
            lease: publication_lease,
        };
        self.validate_and_publish_staging_with_control_and_observer(
            expected,
            publication,
            VectorSnapshotPublishControl::disabled(),
            |_| {},
        )
    }

    fn validate_and_publish_staging_with_control(
        &self,
        expected: VectorSnapshotSummary,
        publication: StagedPublication<'_>,
        control: VectorSnapshotPublishControl<'_>,
    ) -> Result<VectorSnapshotSummary, VectorIndexError> {
        self.validate_and_publish_staging_with_control_and_observer(
            expected,
            publication,
            control,
            |_| {},
        )
    }

    #[cfg(test)]
    fn validate_and_publish_staging_with_observer(
        &self,
        generation: &str,
        expected: VectorSnapshotSummary,
        staging: &PinnedPrivateDirectory,
        published: &Path,
        publication_lease: &VectorSnapshotPublicationLease,
        after_rename: impl FnOnce(&Path),
    ) -> Result<VectorSnapshotSummary, VectorIndexError> {
        let publication = StagedPublication {
            generation,
            staging,
            published,
            lease: publication_lease,
        };
        self.validate_and_publish_staging_with_control_and_observer(
            expected,
            publication,
            VectorSnapshotPublishControl::disabled(),
            after_rename,
        )
    }

    fn validate_and_publish_staging_with_control_and_observer(
        &self,
        expected: VectorSnapshotSummary,
        publication: StagedPublication<'_>,
        control: VectorSnapshotPublishControl<'_>,
        after_rename: impl FnOnce(&Path),
    ) -> Result<VectorSnapshotSummary, VectorIndexError> {
        control.check()?;
        publication
            .lease
            .validate_root(&self.root, &self.root_identity)?;
        publication.staging.validate_current()?;
        let (_, _, validated) = read_snapshot_with_control(
            publication.staging.path(),
            &publication.staging.path().join(KEY_FILE),
            publication.generation,
            &self.model_contract,
            control,
        )?;
        publication.staging.validate_current()?;
        if validated != expected {
            return Err(VectorIndexError::CorruptSnapshot);
        }
        control.check()?;

        // Cancellation is intentionally not observed after this point. Pin
        // creation, atomic rename, and directory durability form one commit.
        reject_symlink_or_existing_generation(publication.published)?;
        let pin_path = generation_pin_path(&self.root, publication.generation);
        reject_symlink_or_existing_generation(&pin_path)?;
        drop(open_lock_file(&pin_path, true)?);
        publication
            .lease
            .validate_root(&self.root, &self.root_identity)?;
        publication.staging.validate_current()?;
        if fs::rename(publication.staging.path(), publication.published).is_err() {
            return preserve_primary_after_cleanup(
                Err(VectorIndexError::Storage),
                || {
                    fs::remove_file(&pin_path).map_err(|_| VectorIndexError::Storage)?;
                    sync_directory(&self.root.join(GENERATION_PINS_DIR))
                },
                |_| {},
            );
        }
        after_rename(publication.published);
        publication
            .staging
            .validate_identity_at(publication.published)?;
        sync_directory(&self.root.join(SNAPSHOTS_DIR))?;
        publication
            .lease
            .validate_root(&self.root, &self.root_identity)?;
        publication
            .staging
            .validate_identity_at(publication.published)?;
        Ok(expected)
    }

    fn cleanup_failed_staging(
        &self,
        staging: &PinnedPrivateDirectory,
        publication_lease: &VectorSnapshotPublicationLease,
    ) -> Result<(), VectorIndexError> {
        publication_lease.validate_root(&self.root, &self.root_identity)?;
        match fs::symlink_metadata(staging.path()) {
            Ok(_) => {
                staging.validate_current()?;
                fs::remove_dir_all(staging.path()).map_err(|_| VectorIndexError::Storage)?;
                sync_directory(&self.root.join(STAGING_DIR))
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(_) => Err(VectorIndexError::Storage),
        }
    }

    fn prepare_layout(&self) -> Result<(), VectorIndexError> {
        self.root_identity.validate_current()?;
        fs::create_dir_all(&self.root).map_err(|_| VectorIndexError::Storage)?;
        let mut layout_created = false;
        for directory in [SNAPSHOTS_DIR, STAGING_DIR, GENERATION_PINS_DIR] {
            let path = self.root.join(directory);
            match fs::symlink_metadata(&path) {
                Ok(_) => require_regular_directory(&path)?,
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    create_private_directory(&path)?;
                    layout_created = true;
                }
                Err(_) => return Err(VectorIndexError::Storage),
            }
        }
        drop(open_lock_file(&self.root.join(READER_LOCK_FILE), true)?);
        drop(open_lock_file(
            &self.root.join(PUBLICATION_LOCK_FILE),
            true,
        )?);
        if layout_created {
            sync_directory(&self.root)?;
        }
        self.root_identity.validate_current()?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FailedStagingCleanupClass {
    LayoutChanged,
    StorageUnavailable,
}

impl FailedStagingCleanupClass {
    fn from_error(error: &VectorIndexError) -> Self {
        match error {
            VectorIndexError::LeaseRootMismatch | VectorIndexError::StorageLayoutInvalid => {
                Self::LayoutChanged
            }
            VectorIndexError::Cancelled
            | VectorIndexError::PublicationBusy
            | VectorIndexError::InvalidDimension { .. }
            | VectorIndexError::InvalidVectorValue
            | VectorIndexError::InvalidModelId
            | VectorIndexError::InvalidIdentity
            | VectorIndexError::InvalidGeneration
            | VectorIndexError::InvalidModelContract
            | VectorIndexError::SemanticUnavailable
            | VectorIndexError::PublicationProjectionMismatch
            | VectorIndexError::DuplicateVectorId
            | VectorIndexError::ConflictingDocumentVersion
            | VectorIndexError::GenerationAlreadyExists
            | VectorIndexError::GenerationNotFound
            | VectorIndexError::SchemaMismatch
            | VectorIndexError::CorruptSnapshot
            | VectorIndexError::Storage => Self::StorageUnavailable,
        }
    }
}

fn preserve_primary_after_cleanup<T>(
    result: Result<T, VectorIndexError>,
    cleanup: impl FnOnce() -> Result<(), VectorIndexError>,
    observe_cleanup_failure: impl FnOnce(FailedStagingCleanupClass),
) -> Result<T, VectorIndexError> {
    match result {
        Ok(value) => Ok(value),
        Err(primary) => {
            // A failed cleanup leaves only a randomly named staging directory
            // or an unpaired generation pin inside the controlled layout. GC
            // owns their recovery; cleanup must never rewrite the root cause.
            if let Err(cleanup_error) = cleanup() {
                observe_cleanup_failure(FailedStagingCleanupClass::from_error(&cleanup_error));
            }
            Err(primary)
        }
    }
}

struct VectorSnapshotPublicationLease {
    root: PathBuf,
    root_identity: PinnedPrivateDirectory,
    snapshots_identity: PinnedPrivateDirectory,
    staging_identity: PinnedPrivateDirectory,
    pins_identity: PinnedPrivateDirectory,
    file: File,
}

impl VectorSnapshotPublicationLease {
    fn acquire(
        root: &Path,
        expected_root: &PinnedPrivateDirectory,
    ) -> Result<Self, VectorIndexError> {
        expected_root.validate_current()?;
        let root_identity = PinnedPrivateDirectory::acquire(root)?;
        if !expected_root.same_identity(&root_identity) {
            return Err(VectorIndexError::StorageLayoutInvalid);
        }
        let snapshots_identity = PinnedPrivateDirectory::acquire(&root.join(SNAPSHOTS_DIR))?;
        let staging_identity = PinnedPrivateDirectory::acquire(&root.join(STAGING_DIR))?;
        let pins_identity = PinnedPrivateDirectory::acquire(&root.join(GENERATION_PINS_DIR))?;
        let file = open_lock_file(&root.join(PUBLICATION_LOCK_FILE), false)?;
        match file.try_lock_exclusive() {
            Ok(true) => {}
            Ok(false) => return Err(VectorIndexError::PublicationBusy),
            Err(_) => return Err(VectorIndexError::Storage),
        }
        let lease = Self {
            root: root.to_path_buf(),
            root_identity,
            snapshots_identity,
            staging_identity,
            pins_identity,
            file,
        };
        lease.validate_root(root, expected_root)?;
        Ok(lease)
    }

    fn validate_root(
        &self,
        root: &Path,
        expected_root: &PinnedPrivateDirectory,
    ) -> Result<(), VectorIndexError> {
        if self.root != root {
            return Err(VectorIndexError::LeaseRootMismatch);
        }
        expected_root.validate_current()?;
        self.root_identity.validate_current()?;
        self.snapshots_identity.validate_current()?;
        self.staging_identity.validate_current()?;
        self.pins_identity.validate_current()?;
        if !expected_root.same_identity(&self.root_identity) {
            return Err(VectorIndexError::StorageLayoutInvalid);
        }
        Ok(())
    }
}

impl Drop for VectorSnapshotPublicationLease {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;

impl fmt::Debug for VectorSnapshotStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VectorSnapshotStore")
            .field("root", &"<redacted>")
            .field("model_contract", &self.model_contract)
            .finish()
    }
}

fn collect_with_control<T>(
    values: impl IntoIterator<Item = T>,
    control: VectorSnapshotPublishControl<'_>,
) -> Result<Vec<T>, VectorIndexError> {
    control.check()?;
    let values = values.into_iter();
    let mut collected = Vec::with_capacity(values.size_hint().0);
    for (index, value) in values.enumerate() {
        collected.push(value);
        control.check_after_record(index + 1)?;
    }
    control.check()?;
    Ok(collected)
}

pub(crate) fn validate_generation(generation: &str) -> Result<(), VectorIndexError> {
    if generation.is_empty()
        || generation.len() > MAX_GENERATION_LEN
        || generation == "."
        || generation == ".."
        || generation.starts_with('.')
        || !generation
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        Err(VectorIndexError::InvalidGeneration)
    } else {
        Ok(())
    }
}

pub(crate) fn canonical_index_root(root: &Path) -> Result<PathBuf, VectorIndexError> {
    let canonical = fs::canonicalize(root).map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            VectorIndexError::GenerationNotFound
        } else {
            VectorIndexError::Storage
        }
    })?;
    require_regular_directory(&canonical)?;
    Ok(canonical)
}

pub(crate) fn generation_pin_path(root: &Path, generation: &str) -> PathBuf {
    root.join(GENERATION_PINS_DIR)
        .join(format!("{generation}.lock"))
}

pub(crate) fn open_lock_file(path: &Path, create: bool) -> Result<File, VectorIndexError> {
    let existed = match fs::symlink_metadata(path) {
        Ok(metadata) => {
            validate_private_lock_metadata(&metadata)?;
            true
        }
        Err(error) if error.kind() == ErrorKind::NotFound && create => false,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(VectorIndexError::GenerationNotFound);
        }
        Err(_) => return Err(VectorIndexError::Storage),
    };
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .truncate(false)
        .create(create);
    #[cfg(unix)]
    options.mode(0o600);
    #[cfg(windows)]
    options.custom_flags(FILE_FLAG_WRITE_THROUGH);
    let file = options.open(path).map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            VectorIndexError::GenerationNotFound
        } else {
            VectorIndexError::Storage
        }
    })?;
    let opened = file.metadata().map_err(|_| VectorIndexError::Storage)?;
    validate_private_lock_metadata(&opened)?;
    let current = fs::symlink_metadata(path).map_err(|_| VectorIndexError::Storage)?;
    validate_private_lock_metadata(&current)?;
    if !same_open_file_identity(&file, path, &opened, &current)? {
        return Err(VectorIndexError::StorageLayoutInvalid);
    }
    if !existed {
        file.sync_all().map_err(|_| VectorIndexError::Storage)?;
        let parent = path.parent().ok_or(VectorIndexError::Storage)?;
        sync_directory(parent)?;
    }
    Ok(file)
}

pub(crate) fn validate_private_lock_metadata(
    metadata: &fs::Metadata,
) -> Result<(), VectorIndexError> {
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(VectorIndexError::StorageLayoutInvalid);
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o777 != 0o600 {
        return Err(VectorIndexError::StorageLayoutInvalid);
    }
    Ok(())
}

fn reject_symlink_or_existing_generation(path: &Path) -> Result<(), VectorIndexError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(VectorIndexError::StorageLayoutInvalid)
        }
        Ok(_) => Err(VectorIndexError::GenerationAlreadyExists),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(_) => Err(VectorIndexError::Storage),
    }
}

pub(crate) fn require_regular_snapshot_directory(path: &Path) -> Result<(), VectorIndexError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            validate_private_directory_permissions(&metadata)?;
            for file in [SNAPSHOT_FILE, MANIFEST_FILE] {
                let metadata = fs::symlink_metadata(path.join(file)).map_err(|error| {
                    if error.kind() == ErrorKind::NotFound {
                        VectorIndexError::CorruptSnapshot
                    } else {
                        VectorIndexError::Storage
                    }
                })?;
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err(VectorIndexError::StorageLayoutInvalid);
                }
                validate_private_file_permissions(&metadata)?;
            }
            Ok(())
        }
        Ok(_) => Err(VectorIndexError::StorageLayoutInvalid),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            Err(VectorIndexError::GenerationNotFound)
        }
        Err(_) => Err(VectorIndexError::Storage),
    }
}

pub(crate) fn require_regular_directory(path: &Path) -> Result<(), VectorIndexError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| VectorIndexError::Storage)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        validate_private_directory_permissions(&metadata)
    } else {
        Err(VectorIndexError::StorageLayoutInvalid)
    }
}

fn validate_private_file_permissions(_metadata: &fs::Metadata) -> Result<(), VectorIndexError> {
    #[cfg(unix)]
    if _metadata.permissions().mode() & 0o777 != 0o600 {
        return Err(VectorIndexError::StorageLayoutInvalid);
    }
    Ok(())
}

fn validate_private_directory_permissions(
    _metadata: &fs::Metadata,
) -> Result<(), VectorIndexError> {
    #[cfg(unix)]
    if _metadata.permissions().mode() & 0o777 != 0o700 {
        return Err(VectorIndexError::StorageLayoutInvalid);
    }
    Ok(())
}
