use std::{
    collections::HashSet,
    ffi::OsString,
    fs::File as StdFile,
    io::{self, Read as _},
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions as CapOpenOptions},
};
use futures_util::StreamExt;
use reqwest::{Client, StatusCode, header};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::{fs::File, io::AsyncWriteExt, sync::watch};
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

const EMBEDDED_MANIFEST: &str = include_str!("model-assets-v1.json");
const EMBEDDED_MTP_CONFIG_SOURCE: &[u8] = include_bytes!("model-assets-mtp-config.json");
const MANIFEST_SCHEMA_VERSION: u32 = 1;
const PINNED_REPOSITORY: &str = "orcarouter/Qwen3.8-27B-Uncensored-MLX";
const PINNED_REVISION: &str = "0f88c40e9eff87740295f27654558fcb77e21ae5";

#[derive(Deserialize)]
struct MtpConfig {
    block_size: u8,
}

fn embedded_mtp_config() -> &'static [u8] {
    // Source files conventionally end in a newline; the immutable Hugging Face
    // object does not. Strip only that repository-source terminator so this slice
    // is byte-for-byte the pinned 2,976-byte object whose digest is in the manifest.
    EMBEDDED_MTP_CONFIG_SOURCE
        .strip_suffix(b"\n")
        .expect("embedded MTP config source has one repository newline")
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetManifest {
    pub schema_version: u32,
    pub manifest_id: String,
    pub repository: String,
    pub revision: String,
    pub source_last_modified: String,
    pub mtp_block_size: u8,
    pub drafter_selection_note: String,
    pub files: Vec<ManifestFile>,
}

impl AssetManifest {
    pub fn total_size_bytes(&self) -> u64 {
        self.files.iter().map(|file| file.size_bytes).sum()
    }

    fn validate(&self) -> Result<(), ManifestError> {
        if self.schema_version != MANIFEST_SCHEMA_VERSION {
            return Err(ManifestError::Schema(self.schema_version));
        }
        if self.revision.len() != 40 || !self.revision.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(ManifestError::Revision(self.revision.clone()));
        }
        if self.files.is_empty() {
            return Err(ManifestError::Empty);
        }
        if self.mtp_block_size != 3 {
            return Err(ManifestError::Drafter(self.mtp_block_size));
        }

        if self.repository == PINNED_REPOSITORY && self.revision == PINNED_REVISION {
            let config: MtpConfig = serde_json::from_slice(embedded_mtp_config())?;
            let config_file = self
                .files
                .iter()
                .find(|file| file.source_path == "mtp/config.json")
                .ok_or_else(|| ManifestError::Path("mtp/config.json".into()))?;
            if config.block_size != self.mtp_block_size
                || config_file.size_bytes != embedded_mtp_config().len() as u64
                || config_file.sha256 != sha256_hex(embedded_mtp_config())
            {
                return Err(ManifestError::Drafter(self.mtp_block_size));
            }
        }

        let mut destinations = HashSet::new();
        for file in &self.files {
            if file.size_bytes == 0
                || file.sha256.len() != 64
                || !file.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(ManifestError::Integrity(file.install_path.clone()));
            }
            let install_path = Path::new(&file.install_path);
            if install_path.is_absolute()
                || install_path.components().any(|component| {
                    matches!(
                        component,
                        Component::ParentDir | Component::RootDir | Component::Prefix(_)
                    )
                })
            {
                return Err(ManifestError::Path(file.install_path.clone()));
            }
            let expected_prefix = match file.asset {
                ModelAsset::Target => "4-bit/",
                ModelAsset::Drafter => "mtp/",
            };
            if !file.source_path.starts_with(expected_prefix)
                || !destinations.insert(file.install_path.clone())
            {
                return Err(ManifestError::Path(file.install_path.clone()));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ModelAsset {
    Target,
    Drafter,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestFile {
    pub asset: ModelAsset,
    pub source_path: String,
    pub install_path: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("model asset manifest could not be read: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported model asset manifest schema {0}")]
    Schema(u32),
    #[error("model asset manifest has an invalid revision: {0}")]
    Revision(String),
    #[error("model asset manifest is empty")]
    Empty,
    #[error(
        "model asset manifest has unsupported MTP block size {0}; the pinned config requires block 3"
    )]
    Drafter(u8),
    #[error("model asset manifest has invalid integrity data for {0}")]
    Integrity(String),
    #[error("model asset manifest has an unsafe or duplicate path: {0}")]
    Path(String),
}

pub fn production_manifest() -> Result<AssetManifest, ManifestError> {
    let manifest = serde_json::from_str::<AssetManifest>(EMBEDDED_MANIFEST)?;
    manifest.validate()?;
    Ok(manifest)
}

const OWNERSHIP_FILE: &str = ".piu-model-assets.json";
const PART_SUFFIX: &str = ".part";
const PART_METADATA_SUFFIX: &str = ".metadata.json";
const KEYCHAIN_SERVICE: &str = "ch.emin.piu.huggingface";
const KEYCHAIN_ACCOUNT: &str = "access-token";
const DISK_SAFETY_RESERVE_BYTES: u64 = 1_073_741_824;
const PROGRESS_CHUNK_BYTES: u64 = 16 * 1024 * 1024;
const OWNERSHIP_METADATA_MAX_BYTES: u64 = 64 * 1024;
const PARTIAL_METADATA_MAX_BYTES: u64 = 8 * 1024;
const PRIVATE_WRITE_RECOVERY_MAX_BYTES: u64 = 16 * 1024;
const PRIVATE_WRITE_RECOVERY_SUFFIX: &str = ".piu-work.json";
const REMOVAL_STAGING_PREFIX: &str = ".piu-removal-";
const REMOVAL_RECOVERY_FILE: &str = "recovery.json";
const REMOVAL_RECOVERY_MAX_BYTES: u64 = 128 * 1024;
const REMOVAL_RECOVERY_MAX_ENTRIES: usize = 64;
const NAMESPACE_SCAN_MAX_ENTRIES: usize = 4_096;
static PRIVATE_WRITE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy)]
struct NetworkTimeouts {
    connect: Duration,
    headers: Duration,
    read: Duration,
}

impl Default for NetworkTimeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(15),
            headers: Duration::from_secs(30),
            read: Duration::from_secs(60),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ModelAssetPhase {
    Initializing,
    Missing,
    Downloading,
    Verifying,
    Removing,
    Ready,
    Cancelled,
    AuthenticationRequired,
    Failed,
    RevisionMismatch,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ModelAssetErrorCode {
    Authentication,
    InsufficientSpace,
    Integrity,
    RevisionMismatch,
    Cancellation,
    Network,
    Ownership,
    Storage,
    Manifest,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ModelAssetAction {
    Download,
    Cancel,
    Authorize,
    Remove,
    RetryRecovery,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct ModelAssetStatus {
    pub phase: ModelAssetPhase,
    pub repository: String,
    pub revision: String,
    pub manifest_id: String,
    #[ts(type = "number")]
    pub total_bytes: u64,
    #[ts(type = "number")]
    pub transferred_bytes: u64,
    #[ts(type = "number")]
    pub remaining_bytes: u64,
    #[ts(type = "number")]
    pub current_free_bytes: u64,
    #[ts(type = "number")]
    pub required_free_bytes: u64,
    pub current_asset: Option<ModelAsset>,
    pub current_file: Option<String>,
    #[ts(type = "number | null")]
    pub operation_id: Option<u64>,
    pub authentication_configured: bool,
    pub can_resume: bool,
    pub available_actions: Vec<ModelAssetAction>,
    pub error_code: Option<ModelAssetErrorCode>,
    pub message: Option<String>,
}

impl ModelAssetStatus {
    fn missing(manifest: &AssetManifest, free_bytes: u64, authentication_configured: bool) -> Self {
        let total_bytes = manifest.total_size_bytes();
        Self {
            phase: ModelAssetPhase::Missing,
            repository: manifest.repository.clone(),
            revision: manifest.revision.clone(),
            manifest_id: manifest.manifest_id.clone(),
            total_bytes,
            transferred_bytes: 0,
            remaining_bytes: total_bytes,
            current_free_bytes: free_bytes,
            required_free_bytes: total_bytes.saturating_add(DISK_SAFETY_RESERVE_BYTES),
            current_asset: None,
            current_file: None,
            operation_id: None,
            authentication_configured,
            can_resume: false,
            available_actions: Vec::new(),
            error_code: None,
            message: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum ModelAssetError {
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error("could not access model storage at {path}: {source}")]
    Storage {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not access the Hugging Face credential: {0}")]
    Credential(String),
    #[error("Hugging Face rejected that token. Check it and try again.")]
    Authentication,
    #[error("could not reach Hugging Face: {0}")]
    Network(String),
    #[error("Hugging Face timed out while waiting for {0}")]
    NetworkTimeout(&'static str),
    #[error("download was cancelled")]
    Cancelled,
    #[error("another model resource operation is already running")]
    OperationInProgress,
    #[error("model assets are not owned by this Più manifest and were left untouched")]
    NotOwned,
    #[error(
        "An older Più model revision is installed. Remove it here, then download the pinned revision."
    )]
    RevisionMismatch,
    #[error("download response for {0} did not support safe resumption")]
    InvalidRange(String),
    #[error("download for {path} ended at {actual} bytes; {expected} bytes are pinned")]
    SizeMismatch {
        path: String,
        actual: u64,
        expected: u64,
    },
    #[error("downloaded file {0} did not match its pinned SHA-256; the unsafe partial was removed")]
    Integrity(String),
    #[error("model asset {0} changed while Più was verifying it")]
    ChangedDuringVerification(String),
    #[error("not enough disk space: {available} bytes free, {required} bytes required")]
    InsufficientSpace { available: u64, required: u64 },
    #[error("model resources are unavailable: {0}")]
    Unavailable(String),
    #[error("model assets require recovery before another operation: {0}")]
    RecoveryRequired(String),
}

impl ModelAssetError {
    pub fn code(&self) -> ModelAssetErrorCode {
        match self {
            Self::Authentication => ModelAssetErrorCode::Authentication,
            Self::InsufficientSpace { .. } => ModelAssetErrorCode::InsufficientSpace,
            Self::Integrity(_) | Self::ChangedDuringVerification(_) => {
                ModelAssetErrorCode::Integrity
            }
            Self::RevisionMismatch => ModelAssetErrorCode::RevisionMismatch,
            Self::Cancelled => ModelAssetErrorCode::Cancellation,
            Self::Network(_)
            | Self::NetworkTimeout(_)
            | Self::InvalidRange(_)
            | Self::SizeMismatch { .. } => ModelAssetErrorCode::Network,
            Self::NotOwned => ModelAssetErrorCode::Ownership,
            Self::Storage { .. }
            | Self::Credential(_)
            | Self::Unavailable(_)
            | Self::RecoveryRequired(_)
            | Self::OperationInProgress => ModelAssetErrorCode::Storage,
            Self::Manifest(_) => ModelAssetErrorCode::Manifest,
        }
    }
}

impl From<reqwest::Error> for ModelAssetError {
    fn from(error: reqwest::Error) -> Self {
        let message = if let Some(status) = error.status() {
            format!("Hugging Face returned HTTP {status}")
        } else if error.is_timeout() {
            "the request timed out".into()
        } else if error.is_connect() {
            "the connection failed".into()
        } else if error.is_redirect() {
            "the download redirect could not be followed".into()
        } else {
            "the request failed".into()
        };
        Self::Network(message)
    }
}

trait CredentialStore: Send + Sync {
    fn get(&self) -> Result<Option<String>, ModelAssetError>;
    fn set(&self, token: &str) -> Result<(), ModelAssetError>;
}

struct KeychainCredentialStore;

#[cfg(target_os = "macos")]
impl CredentialStore for KeychainCredentialStore {
    fn get(&self) -> Result<Option<String>, ModelAssetError> {
        match security_framework::passwords::get_generic_password(
            KEYCHAIN_SERVICE,
            KEYCHAIN_ACCOUNT,
        ) {
            Ok(bytes) => String::from_utf8(bytes)
                .map(Some)
                .map_err(|error| ModelAssetError::Credential(error.to_string())),
            Err(error) if error.code() == -25300 => Ok(None),
            Err(error) => Err(ModelAssetError::Credential(error.to_string())),
        }
    }

    fn set(&self, token: &str) -> Result<(), ModelAssetError> {
        security_framework::passwords::set_generic_password(
            KEYCHAIN_SERVICE,
            KEYCHAIN_ACCOUNT,
            token.as_bytes(),
        )
        .map_err(|error| ModelAssetError::Credential(error.to_string()))
    }
}

#[cfg(not(target_os = "macos"))]
impl CredentialStore for KeychainCredentialStore {
    fn get(&self) -> Result<Option<String>, ModelAssetError> {
        Ok(None)
    }

    fn set(&self, _token: &str) -> Result<(), ModelAssetError> {
        Err(ModelAssetError::Credential(
            "Più model credentials require macOS Keychain".into(),
        ))
    }
}

trait DiskSpace: Send + Sync {
    fn available(&self, path: &Path) -> Result<u64, ModelAssetError>;
}

struct SystemDiskSpace;

impl DiskSpace for SystemDiskSpace {
    fn available(&self, path: &Path) -> Result<u64, ModelAssetError> {
        fs4::available_space(path).map_err(|source| ModelAssetError::Storage {
            path: path.to_path_buf(),
            source,
        })
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum OperationKind {
    Download,
    Validation,
    Removal,
}

struct ActiveOperation {
    operation_id: u64,
    cancellation: CancellationToken,
    kind: OperationKind,
    can_cancel: bool,
}

struct TransferProgress {
    completed_base: u64,
    last_reported: u64,
}

impl TransferProgress {
    fn new(sampled_total: u64, current_file_bytes: u64) -> Self {
        Self {
            completed_base: sampled_total.saturating_sub(current_file_bytes),
            last_reported: sampled_total,
        }
    }

    fn observe(&mut self, current_file_bytes: u64, file_size: u64) -> Option<u64> {
        let total = self.completed_base.saturating_add(current_file_bytes);
        if total.saturating_sub(self.last_reported) >= PROGRESS_CHUNK_BYTES
            || current_file_bytes == file_size
        {
            self.last_reported = total;
            Some(total)
        } else {
            None
        }
    }
}

struct InstallInspection {
    status: ModelAssetStatus,
    requires_validation: bool,
}

/// Capability-anchors every asset operation at the application-owned model root.
/// Parent directories and final components are opened without following symlinks,
/// so a concurrent path replacement cannot redirect reads or writes outside the root.
struct SafeStorage {
    root: PathBuf,
    directory: Dir,
}

/// A file that stays under an operation-private name until its opened inode has
/// been flushed and revalidated. Replacing the private pathname cannot redirect
/// writes because all bytes go through the already-open file description.
struct PrivateWrite {
    output: Option<File>,
    sync_handle: StdFile,
    directory: Dir,
    temporary_name: OsString,
    destination_name: OsString,
    temporary_path: PathBuf,
    opened_device: u64,
    opened_inode: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PrivateWriteRecovery {
    schema_version: u32,
    owner: String,
    temporary_path: PathBuf,
    partial: PartialMetadata,
}

trait RemovalPersistence: Send + Sync {
    fn commit_journal(
        &self,
        storage: &SafeStorage,
        path: &Path,
        phase: RemovalRecoveryPhase,
    ) -> io::Result<()>;
    fn stage(&self, storage: &SafeStorage, from: &Path, to: &Path) -> io::Result<()>;
    fn delete(&self, storage: &SafeStorage, path: &Path) -> io::Result<()>;
    fn complete_mutations(&self, storage: &SafeStorage, journal: &Path) -> io::Result<()>;
}

struct DurableRemovalPersistence;

impl RemovalPersistence for DurableRemovalPersistence {
    fn commit_journal(
        &self,
        storage: &SafeStorage,
        path: &Path,
        _phase: RemovalRecoveryPhase,
    ) -> io::Result<()> {
        storage.order_recovery_phase(path)
    }

    fn stage(&self, storage: &SafeStorage, from: &Path, to: &Path) -> io::Result<()> {
        storage.rename_durable(from, to)
    }

    fn delete(&self, storage: &SafeStorage, path: &Path) -> io::Result<()> {
        storage.remove_file_durable(path)
    }

    fn complete_mutations(&self, storage: &SafeStorage, journal: &Path) -> io::Result<()> {
        storage.complete_recovery_mutations(journal)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileIdentity {
    device: u64,
    inode: u64,
    size: u64,
    links: u64,
    changed_at_seconds: i64,
    changed_at_nanoseconds: i64,
}

struct StagedRemoval {
    original: PathBuf,
    staged: PathBuf,
    identity: FileIdentity,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum RemovalRecoveryPhase {
    Staging,
    Deleting,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RemovalRecoveryEntry {
    original: PathBuf,
    staged: PathBuf,
    size_bytes: u64,
    sha256: String,
    identity: Option<FileIdentity>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RemovalRecoveryPlan {
    schema_version: u32,
    owner: String,
    phase: RemovalRecoveryPhase,
    staging_directory: PathBuf,
    entries: Vec<RemovalRecoveryEntry>,
}

impl RemovalRecoveryPlan {
    fn validate(&self, staging_directory: &Path) -> bool {
        self.schema_version == 1
            && self.owner == "ch.emin.piu"
            && self.staging_directory == staging_directory
            && !self.entries.is_empty()
            && self.entries.len() <= REMOVAL_RECOVERY_MAX_ENTRIES
            && self.entries.iter().all(|entry| {
                safe_relative_path(&entry.original)
                    && entry
                        .staged
                        .parent()
                        .is_some_and(|parent| parent == staging_directory)
                    && entry.staged.file_name().is_some_and(|name| {
                        name == "ownership-marker" || name.to_string_lossy().starts_with("asset-")
                    })
                    && entry.size_bytes > 0
                    && entry.sha256.len() == 64
                    && entry.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
                    && match self.phase {
                        RemovalRecoveryPhase::Staging => entry.identity.is_none(),
                        RemovalRecoveryPhase::Deleting => entry.identity.is_some_and(|identity| {
                            identity.links == 1 && identity.size == entry.size_bytes
                        }),
                    }
            })
    }
}

impl SafeStorage {
    fn open(anchor: PathBuf, relative_root: &Path) -> io::Result<Self> {
        let trusted_parent_path = anchor.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "model storage anchor must have a trusted parent",
            )
        })?;
        let anchor_name = anchor
            .file_name()
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "model storage anchor is empty")
            })?
            .to_os_string();
        let trusted_parent = Dir::open_ambient_dir(trusted_parent_path, ambient_authority())?;
        Self::open_beneath(
            trusted_parent,
            Path::new(&anchor_name),
            anchor,
            relative_root,
        )
    }

    fn open_beneath(
        mut directory: Dir,
        relative_anchor: &Path,
        anchor: PathBuf,
        relative_root: &Path,
    ) -> io::Result<Self> {
        for component in relative_anchor
            .components()
            .chain(relative_root.components())
        {
            let Component::Normal(component) = component else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "model storage path must be relative",
                ));
            };
            match directory.open_dir_nofollow(component) {
                Ok(next) => directory = next,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    match directory.create_dir(component) {
                        Ok(()) => {}
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                        Err(error) => return Err(error),
                    }
                    directory = directory.open_dir_nofollow(component)?;
                }
                Err(error) => return Err(error),
            }
        }
        let root = anchor.join(relative_root);
        Ok(Self { root, directory })
    }

    fn parent_and_name(&self, relative: &Path, create: bool) -> io::Result<(Dir, OsString)> {
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "asset path must contain only relative normal components",
            ));
        }
        let name = relative
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "asset path is empty"))?
            .to_os_string();
        let mut directory = self.directory.try_clone()?;
        if let Some(parent) = relative.parent() {
            for component in parent.components() {
                let Component::Normal(component) = component else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "unsafe asset path",
                    ));
                };
                match directory.open_dir_nofollow(component) {
                    Ok(next) => directory = next,
                    Err(error) if create && error.kind() == io::ErrorKind::NotFound => {
                        match directory.create_dir(component) {
                            Ok(()) => {}
                            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                            Err(error) => return Err(error),
                        }
                        directory = directory.open_dir_nofollow(component)?;
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        Ok((directory, name))
    }

    fn open_read(&self, relative: &Path) -> io::Result<StdFile> {
        let (directory, name) = self.parent_and_name(relative, false)?;
        let mut options = CapOpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        directory
            .open_with(name, &options)
            .map(|file| file.into_std())
    }

    fn read_bounded(&self, relative: &Path, maximum_bytes: u64) -> io::Result<Vec<u8>> {
        self.read_bounded_with_identity(relative, maximum_bytes)
            .map(|(bytes, _)| bytes)
    }

    #[cfg(unix)]
    fn read_bounded_with_identity(
        &self,
        relative: &Path,
        maximum_bytes: u64,
    ) -> io::Result<(Vec<u8>, FileIdentity)> {
        use std::os::unix::fs::MetadataExt;

        let (directory, name) = self.parent_and_name(relative, false)?;
        let mut options = CapOpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let mut file = directory.open_with(name, &options)?.into_std();
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() > maximum_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "model asset metadata exceeds its schema size limit",
            ));
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.by_ref()
            .take(maximum_bytes.saturating_add(1))
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > maximum_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "model asset metadata exceeds its schema size limit",
            ));
        }
        Ok((
            bytes,
            FileIdentity {
                device: metadata.dev(),
                inode: metadata.ino(),
                size: metadata.size(),
                links: metadata.nlink(),
                changed_at_seconds: metadata.ctime(),
                changed_at_nanoseconds: metadata.ctime_nsec(),
            },
        ))
    }

    fn create_private_write(&self, destination: &Path) -> io::Result<PrivateWrite> {
        let (directory, destination_name, temporary_name, temporary_path, file) =
            self.create_private_slot(destination)?;
        PrivateWrite::new(
            file,
            directory,
            temporary_name,
            destination_name,
            temporary_path,
        )
    }

    fn create_private_slot(
        &self,
        destination: &Path,
    ) -> io::Result<(Dir, OsString, OsString, PathBuf, StdFile)> {
        let (directory, destination_name) = self.parent_and_name(destination, true)?;
        for _ in 0..64 {
            let identifier = PRIVATE_WRITE_ID.fetch_add(1, Ordering::Relaxed);
            let mut temporary_name = OsString::from(".");
            temporary_name.push(&destination_name);
            temporary_name.push(format!(".piu-write-{}-{identifier}", std::process::id()));
            let mut options = CapOpenOptions::new();
            options
                .read(true)
                .write(true)
                .create_new(true)
                .follow(FollowSymlinks::No);
            match directory.open_with(&temporary_name, &options) {
                Ok(file) => {
                    let temporary_path = destination
                        .parent()
                        .unwrap_or_else(|| Path::new(""))
                        .join(&temporary_name);
                    return Ok((
                        directory,
                        destination_name,
                        temporary_name,
                        temporary_path,
                        file.into_std(),
                    ));
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate an operation-private model asset file",
        ))
    }

    fn sync_parent(&self, relative: &Path) -> io::Result<()> {
        let (directory, _) = self.parent_and_name(relative, false)?;
        directory.into_std_file().sync_all()
    }

    fn open_std_read(&self, relative: &Path) -> io::Result<StdFile> {
        let (directory, name) = self.parent_and_name(relative, false)?;
        let mut options = CapOpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        directory
            .open_with(name, &options)
            .map(|file| file.into_std())
    }

    /// Orders a recovery journal and its directory entry before the destructive
    /// phase it describes. On macOS the barrier prevents APFS/the device from
    /// persisting a later rename or unlink without the earlier journal state.
    fn order_recovery_phase(&self, journal: &Path) -> io::Result<()> {
        let journal_file = self.open_std_read(journal)?;
        journal_file.sync_all()?;
        self.sync_parent(journal)?;
        platform_phase_barrier(&journal_file)
    }

    /// Makes every fsync'd destructive mutation before this point durable before
    /// its recovery journal is removed. The still-open journal is a same-volume
    /// durability anchor.
    fn complete_recovery_mutations(&self, journal: &Path) -> io::Result<()> {
        let journal_file = self.open_std_read(journal)?;
        journal_file.sync_all()?;
        self.sync_parent(journal)?;
        platform_full_sync(&journal_file)
    }

    fn rename_durable(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.rename(from, to)?;
        self.sync_parent(from)?;
        if from.parent() != to.parent() {
            self.sync_parent(to)?;
        }
        Ok(())
    }

    fn remove_file_durable(&self, relative: &Path) -> io::Result<()> {
        self.remove_file(relative)?;
        self.sync_parent(relative)
    }

    fn remove_dir_durable(&self, relative: &Path) -> io::Result<()> {
        self.remove_dir(relative)?;
        self.sync_parent(relative)
    }

    fn metadata(&self, relative: &Path) -> io::Result<Option<cap_std::fs::Metadata>> {
        let (directory, name) = match self.parent_and_name(relative, false) {
            Ok(value) => value,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        match directory.symlink_metadata(name) {
            Ok(metadata) => Ok(Some(metadata)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn remove_file(&self, relative: &Path) -> io::Result<()> {
        let (directory, name) = self.parent_and_name(relative, false)?;
        directory.remove_file(name)
    }

    fn create_dir(&self, relative: &Path) -> io::Result<()> {
        let (directory, name) = self.parent_and_name(relative, true)?;
        directory.create_dir(name)
    }

    fn remove_dir(&self, relative: &Path) -> io::Result<()> {
        let (directory, name) = self.parent_and_name(relative, false)?;
        directory.remove_dir(name)
    }

    fn scan_entry_names(
        &self,
        relative: Option<&Path>,
        mut inspect: impl FnMut(OsString) -> io::Result<()>,
    ) -> io::Result<()> {
        let directory = if let Some(relative) = relative.filter(|path| !path.as_os_str().is_empty())
        {
            let (parent, name) = self.parent_and_name(relative, false)?;
            parent.open_dir_nofollow(name)?
        } else {
            self.directory.try_clone()?
        };
        for (index, entry) in directory.entries()?.enumerate() {
            if index >= NAMESPACE_SCAN_MAX_ENTRIES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "model asset namespace exceeds its scan limit",
                ));
            }
            inspect(entry?.file_name())?;
        }
        Ok(())
    }

    #[cfg(unix)]
    fn identity(&self, relative: &Path) -> io::Result<Option<FileIdentity>> {
        use cap_std::fs::MetadataExt;

        Ok(self.metadata(relative)?.map(|metadata| FileIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.size(),
            links: metadata.nlink(),
            changed_at_seconds: metadata.ctime(),
            changed_at_nanoseconds: metadata.ctime_nsec(),
        }))
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let (from_directory, from_name) = self.parent_and_name(from, false)?;
        let (to_directory, to_name) = self.parent_and_name(to, true)?;
        from_directory.rename(from_name, &to_directory, to_name)
    }

    fn absolute(&self, relative: &Path) -> PathBuf {
        self.root.join(relative)
    }
}

#[cfg(target_os = "macos")]
fn macos_fcntl_sync(file: &StdFile, command: libc::c_int) -> io::Result<()> {
    use std::os::fd::AsRawFd;

    // SAFETY: `file` owns a valid descriptor for the duration of the call, and
    // both sync commands ignore the variadic argument.
    let result = unsafe { libc::fcntl(file.as_raw_fd(), command) };
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn platform_phase_barrier(file: &StdFile) -> io::Result<()> {
    match macos_fcntl_sync(file, libc::F_BARRIERFSYNC) {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.raw_os_error(),
                Some(libc::EINVAL) | Some(libc::ENOTSUP) | Some(libc::ENOTTY)
            ) =>
        {
            macos_fcntl_sync(file, libc::F_FULLFSYNC)
        }
        Err(error) => Err(error),
    }
}

#[cfg(not(target_os = "macos"))]
fn platform_phase_barrier(file: &StdFile) -> io::Result<()> {
    file.sync_all()
}

#[cfg(target_os = "macos")]
fn platform_full_sync(file: &StdFile) -> io::Result<()> {
    macos_fcntl_sync(file, libc::F_FULLFSYNC)
}

#[cfg(not(target_os = "macos"))]
fn platform_full_sync(file: &StdFile) -> io::Result<()> {
    file.sync_all()
}

#[cfg(unix)]
fn std_file_identity(file: &StdFile) -> io::Result<FileIdentity> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file.metadata()?;
    Ok(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        size: metadata.size(),
        links: metadata.nlink(),
        changed_at_seconds: metadata.ctime(),
        changed_at_nanoseconds: metadata.ctime_nsec(),
    })
}

impl PrivateWrite {
    fn new(
        file: StdFile,
        directory: Dir,
        temporary_name: OsString,
        destination_name: OsString,
        temporary_path: PathBuf,
    ) -> io::Result<Self> {
        let identity = std_file_identity(&file)?;
        if identity.links != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "operation-private model asset file has multiple links",
            ));
        }
        let output = File::from_std(file.try_clone()?);
        Ok(Self {
            output: Some(output),
            sync_handle: file,
            directory,
            temporary_name,
            destination_name,
            temporary_path,
            opened_device: identity.device,
            opened_inode: identity.inode,
        })
    }

    fn temporary_path(&self) -> &Path {
        &self.temporary_path
    }

    fn adopt_existing(&mut self, expected: FileIdentity) -> io::Result<()> {
        drop(self.output.take());
        self.directory.rename(
            &self.destination_name,
            &self.directory,
            &self.temporary_name,
        )?;
        let mut options = CapOpenOptions::new();
        options
            .read(true)
            .write(true)
            .append(true)
            .follow(FollowSymlinks::No);
        let opened = match self.directory.open_with(&self.temporary_name, &options) {
            Ok(file) => file.into_std(),
            Err(error) => {
                let _ = self.directory.rename(
                    &self.temporary_name,
                    &self.directory,
                    &self.destination_name,
                );
                return Err(error);
            }
        };
        let actual = std_file_identity(&opened)?;
        if actual.device != expected.device
            || actual.inode != expected.inode
            || actual.size != expected.size
            || actual.links != 1
        {
            drop(opened);
            let _ = self.directory.rename(
                &self.temporary_name,
                &self.directory,
                &self.destination_name,
            );
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "model asset partial changed while it became operation-private",
            ));
        }
        self.output = Some(File::from_std(opened.try_clone()?));
        self.sync_handle = opened;
        self.opened_device = actual.device;
        self.opened_inode = actual.inode;
        Ok(())
    }

    async fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.output
            .as_mut()
            .expect("private output is open")
            .write_all(bytes)
            .await
    }

    fn write_all_blocking(&mut self, bytes: &[u8]) -> io::Result<()> {
        drop(self.output.take());
        std::io::Write::write_all(&mut self.sync_handle, bytes)
    }

    async fn publish(mut self) -> io::Result<()> {
        if let Some(mut output) = self.output.take() {
            output.flush().await?;
            output.sync_all().await?;
            drop(output);
        }
        tokio::task::spawn_blocking(move || self.publish_opened_inode())
            .await
            .map_err(|error| {
                io::Error::other(format!("private publication task failed: {error}"))
            })?
    }

    async fn discard(mut self) -> io::Result<()> {
        drop(self.output.take());
        tokio::task::spawn_blocking(move || self.remove_unpublished_private_inode())
            .await
            .map_err(|error| io::Error::other(format!("private cleanup task failed: {error}")))?
    }

    fn discard_blocking(mut self) -> io::Result<()> {
        drop(self.output.take());
        self.remove_unpublished_private_inode()
    }

    fn publish_blocking(mut self) -> io::Result<()> {
        drop(self.output.take());
        self.publish_opened_inode()
    }

    fn publish_opened_inode(&mut self) -> io::Result<()> {
        use cap_std::fs::MetadataExt as CapMetadataExt;
        use std::os::unix::fs::MetadataExt as StdMetadataExt;

        self.sync_handle.sync_all()?;
        let opened = self.sync_handle.metadata()?;
        let visible = self.directory.symlink_metadata(&self.temporary_name)?;
        let same_opened_inode = StdMetadataExt::dev(&opened) == self.opened_device
            && StdMetadataExt::ino(&opened) == self.opened_inode
            && StdMetadataExt::nlink(&opened) == 1;
        let same_visible_inode = visible.is_file()
            && !visible.file_type().is_symlink()
            && CapMetadataExt::dev(&visible) == self.opened_device
            && CapMetadataExt::ino(&visible) == self.opened_inode
            && CapMetadataExt::nlink(&visible) == 1
            && CapMetadataExt::size(&visible) == StdMetadataExt::size(&opened);
        if !same_opened_inode || !same_visible_inode {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "operation-private model asset path changed before publication",
            ));
        }
        self.directory.rename(
            &self.temporary_name,
            &self.directory,
            &self.destination_name,
        )?;
        self.directory.try_clone()?.into_std_file().sync_all()?;
        let published = self.directory.symlink_metadata(&self.destination_name)?;
        if CapMetadataExt::dev(&published) != self.opened_device
            || CapMetadataExt::ino(&published) != self.opened_inode
            || CapMetadataExt::nlink(&published) != 1
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "published model asset does not match its private inode",
            ));
        }
        Ok(())
    }

    fn remove_unpublished_private_inode(&self) -> io::Result<()> {
        use cap_std::fs::MetadataExt;

        let metadata = match self.directory.symlink_metadata(&self.temporary_name) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.dev() != self.opened_device
            || metadata.ino() != self.opened_inode
            || metadata.nlink() != 1
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "operation-private model asset path changed before cleanup",
            ));
        }
        self.directory.remove_file(&self.temporary_name)?;
        self.directory.try_clone()?.into_std_file().sync_all()
    }
}

struct ModelAssetManagerInner {
    root: PathBuf,
    storage: OnceLock<SafeStorage>,
    manifest: AssetManifest,
    resolve_base_url: String,
    whoami_url: String,
    client: Client,
    network_timeouts: NetworkTimeouts,
    credentials: Arc<dyn CredentialStore>,
    disk_space: Arc<dyn DiskSpace>,
    status: watch::Sender<ModelAssetStatus>,
    active: Mutex<Option<ActiveOperation>>,
    recovery_required: AtomicBool,
    recovery_serialization: Mutex<()>,
    invalid_finals: Mutex<HashSet<String>>,
    removal_persistence: Mutex<Arc<dyn RemovalPersistence>>,
    next_operation_id: AtomicU64,
}

/// Owns the complete lifecycle of Più's one pinned local model resource set.
///
/// The production constructor intentionally accepts only the application data directory:
/// callers cannot choose a model, revision, download origin, or storage location.
#[derive(Clone)]
pub struct ModelAssetManager(Arc<ModelAssetManagerInner>);

impl ModelAssetManager {
    pub fn production(app_data: &Path) -> Result<Self, ModelAssetError> {
        let manifest = production_manifest()?;
        let resolve_base_url = format!(
            "https://huggingface.co/{}/resolve/{}",
            manifest.repository, manifest.revision
        );
        let manager = Self::new_uninitialized(
            app_data.to_path_buf(),
            PathBuf::from("models/qwen3.8-27b-uncensored-mlx"),
            manifest,
            resolve_base_url,
            "https://huggingface.co/api/whoami-v2".into(),
            Arc::new(KeychainCredentialStore),
            Arc::new(SystemDiskSpace),
            NetworkTimeouts::default(),
        )?;
        manager.start_deferred_initialization(
            app_data.to_path_buf(),
            PathBuf::from("models/qwen3.8-27b-uncensored-mlx"),
        );
        Ok(manager)
    }

    pub fn production_or_unavailable(app_data: &Path) -> Self {
        Self::production(app_data).unwrap_or_else(|error| {
            let manifest = production_manifest().expect("embedded model manifest is tested");
            let message = error.to_string();
            let root = app_data.join("models/qwen3.8-27b-uncensored-mlx");
            let mut status = ModelAssetStatus::missing(&manifest, 0, false);
            status.phase = ModelAssetPhase::Failed;
            status.message = Some(message.clone());
            status.error_code = Some(error.code());
            let (status, _) = watch::channel(status);
            Self(Arc::new(ModelAssetManagerInner {
                root,
                storage: OnceLock::new(),
                resolve_base_url: format!(
                    "https://huggingface.co/{}/resolve/{}",
                    manifest.repository, manifest.revision
                ),
                whoami_url: "https://huggingface.co/api/whoami-v2".into(),
                manifest,
                client: Client::new(),
                network_timeouts: NetworkTimeouts::default(),
                credentials: Arc::new(KeychainCredentialStore),
                disk_space: Arc::new(SystemDiskSpace),
                status,
                active: Mutex::new(None),
                recovery_required: AtomicBool::new(false),
                recovery_serialization: Mutex::new(()),
                invalid_finals: Mutex::new(HashSet::new()),
                removal_persistence: Mutex::new(Arc::new(DurableRemovalPersistence)),
                next_operation_id: AtomicU64::new(1),
            }))
        })
    }

    #[cfg(test)]
    fn new(
        storage_anchor: PathBuf,
        storage_relative: PathBuf,
        manifest: AssetManifest,
        resolve_base_url: String,
        whoami_url: String,
        credentials: Arc<dyn CredentialStore>,
        disk_space: Arc<dyn DiskSpace>,
    ) -> Result<Self, ModelAssetError> {
        Self::new_with_timeouts(
            storage_anchor,
            storage_relative,
            manifest,
            resolve_base_url,
            whoami_url,
            credentials,
            disk_space,
            NetworkTimeouts::default(),
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    fn new_with_timeouts(
        storage_anchor: PathBuf,
        storage_relative: PathBuf,
        manifest: AssetManifest,
        resolve_base_url: String,
        whoami_url: String,
        credentials: Arc<dyn CredentialStore>,
        disk_space: Arc<dyn DiskSpace>,
        network_timeouts: NetworkTimeouts,
    ) -> Result<Self, ModelAssetError> {
        let manager = Self::new_uninitialized(
            storage_anchor.clone(),
            storage_relative.clone(),
            manifest,
            resolve_base_url,
            whoami_url,
            credentials,
            disk_space,
            network_timeouts,
        )?;
        let inspection = manager.initialize_storage(storage_anchor, storage_relative)?;
        if inspection.requires_validation {
            manager.start_background_validation();
        }
        Ok(manager)
    }

    #[allow(clippy::too_many_arguments)]
    fn new_uninitialized(
        storage_anchor: PathBuf,
        storage_relative: PathBuf,
        manifest: AssetManifest,
        resolve_base_url: String,
        whoami_url: String,
        credentials: Arc<dyn CredentialStore>,
        disk_space: Arc<dyn DiskSpace>,
        network_timeouts: NetworkTimeouts,
    ) -> Result<Self, ModelAssetError> {
        manifest.validate()?;
        let root = storage_anchor.join(storage_relative);
        let mut initial_status = ModelAssetStatus::missing(&manifest, 0, false);
        initial_status.phase = ModelAssetPhase::Initializing;
        initial_status.message = Some("Checking local model resources.".into());
        let (status, _) = watch::channel(initial_status);
        Ok(Self(Arc::new(ModelAssetManagerInner {
            root,
            storage: OnceLock::new(),
            manifest,
            resolve_base_url,
            whoami_url,
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::limited(10))
                .connect_timeout(network_timeouts.connect)
                .read_timeout(network_timeouts.read)
                .build()?,
            network_timeouts,
            credentials,
            disk_space,
            status,
            active: Mutex::new(None),
            recovery_required: AtomicBool::new(false),
            recovery_serialization: Mutex::new(()),
            invalid_finals: Mutex::new(HashSet::new()),
            removal_persistence: Mutex::new(Arc::new(DurableRemovalPersistence)),
            next_operation_id: AtomicU64::new(1),
        })))
    }

    fn initialize_storage(
        &self,
        storage_anchor: PathBuf,
        storage_relative: PathBuf,
    ) -> Result<InstallInspection, ModelAssetError> {
        let storage = SafeStorage::open(storage_anchor, &storage_relative).map_err(|source| {
            ModelAssetError::Storage {
                path: self.0.root.clone(),
                source,
            }
        })?;
        Self::recover_staged_removals(&storage)?;
        Self::recover_private_writes(&storage, &self.0.manifest)?;
        let free_bytes = self.0.disk_space.available(&self.0.root)?;
        let has_credentials = self.0.credentials.get()?.is_some();
        let inspection =
            Self::inspect_install(&storage, &self.0.manifest, free_bytes, has_credentials)?;
        self.0
            .storage
            .set(storage)
            .map_err(|_| ModelAssetError::Unavailable("model storage initialized twice".into()))?;
        self.publish(inspection.status.clone());
        Ok(inspection)
    }

    fn start_deferred_initialization(&self, storage_anchor: PathBuf, storage_relative: PathBuf) {
        self.start_deferred_initialization_with_hook(storage_anchor, storage_relative, || {});
    }

    fn start_deferred_initialization_with_hook(
        &self,
        storage_anchor: PathBuf,
        storage_relative: PathBuf,
        before_recovery: impl FnOnce() + Send + 'static,
    ) {
        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            let worker = manager.clone();
            let result = tokio::task::spawn_blocking(move || {
                before_recovery();
                worker.initialize_storage(storage_anchor, storage_relative)
            })
            .await
            .map_err(|_| {
                ModelAssetError::Unavailable("model resource initialization stopped".into())
            })
            .and_then(|result| result);
            match result {
                Ok(inspection) if inspection.requires_validation => {
                    manager.start_background_validation();
                }
                Ok(_) => {}
                Err(error) => manager.publish_initialization_failure(&error),
            }
        });
    }

    fn publish_initialization_failure(&self, error: &ModelAssetError) {
        let mut status = self.status();
        status.phase = ModelAssetPhase::Failed;
        status.message = Some(
            "Più couldn't prepare its managed model storage. Quit and reopen Più. If the problem continues, reset Più's pre-release application data."
                .into(),
        );
        status.error_code = Some(error.code());
        tracing::error!(%error, "model resource initialization failed");
        self.publish(status);
    }

    fn recover_staged_removals(storage: &SafeStorage) -> Result<(), ModelAssetError> {
        let mut staging_directories = Vec::new();
        storage
            .scan_entry_names(None, |name| {
                if name
                    .to_str()
                    .is_some_and(|name| name.starts_with(REMOVAL_STAGING_PREFIX))
                {
                    if staging_directories.len() >= REMOVAL_RECOVERY_MAX_ENTRIES {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "too many model removal recovery directories",
                        ));
                    }
                    staging_directories.push(PathBuf::from(name));
                }
                Ok(())
            })
            .map_err(|source| ModelAssetError::Storage {
                path: storage.root.clone(),
                source,
            })?;
        staging_directories.sort();
        for staging_directory in staging_directories {
            Self::recover_staged_removal(storage, &staging_directory)?;
        }
        Ok(())
    }

    fn recover_private_writes(
        storage: &SafeStorage,
        manifest: &AssetManifest,
    ) -> Result<(), ModelAssetError> {
        Self::remove_crash_left_publications(
            storage,
            Path::new(OWNERSHIP_FILE),
            OWNERSHIP_METADATA_MAX_BYTES,
        )?;
        for file in &manifest.files {
            let partial = partial_path(Path::new(&file.install_path));
            Self::remove_crash_left_publications(
                storage,
                &partial_metadata_path(&partial),
                PARTIAL_METADATA_MAX_BYTES,
            )?;
            Self::recover_private_write(storage, manifest, file)?;
        }
        Ok(())
    }

    fn remove_crash_left_publications(
        storage: &SafeStorage,
        destination: &Path,
        maximum_bytes: u64,
    ) -> Result<(), ModelAssetError> {
        let parent = destination.parent().unwrap_or_else(|| Path::new(""));
        let destination_name = destination.file_name().ok_or(ModelAssetError::NotOwned)?;
        let mut candidates = Vec::new();
        match storage.scan_entry_names(Some(parent), |name| {
            if is_private_write_name_for(&name, destination_name) {
                if candidates.len() >= REMOVAL_RECOVERY_MAX_ENTRIES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "too many private model metadata publications",
                    ));
                }
                candidates.push(name);
            }
            Ok(())
        }) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(source) => {
                return Err(ModelAssetError::Storage {
                    path: storage.absolute(parent),
                    source,
                });
            }
        }
        for name in candidates {
            let path = parent.join(name);
            let identity = storage
                .identity(&path)
                .map_err(|_| ModelAssetError::NotOwned)?
                .ok_or(ModelAssetError::NotOwned)?;
            if identity.links != 1 || identity.size > maximum_bytes {
                return Err(ModelAssetError::NotOwned);
            }
            storage
                .remove_file_durable(&path)
                .map_err(|_| ModelAssetError::NotOwned)?;
        }
        Ok(())
    }

    fn recover_private_write(
        storage: &SafeStorage,
        manifest: &AssetManifest,
        file: &ManifestFile,
    ) -> Result<(), ModelAssetError> {
        let partial = partial_path(Path::new(&file.install_path));
        let recovery = private_write_recovery_path(&partial);
        let parent = partial.parent().unwrap_or_else(|| Path::new(""));
        let mut candidates = Vec::new();
        let mut recovery_candidates = Vec::new();
        let partial_name = partial.file_name().expect("manifest partial file name");
        let recovery_name = recovery.file_name().expect("recovery file name");
        let scan_result = storage.scan_entry_names(Some(parent), |name| {
            let selected = if is_private_write_name_for(&name, partial_name) {
                Some(&mut candidates)
            } else if is_private_write_name_for(&name, recovery_name) {
                Some(&mut recovery_candidates)
            } else {
                None
            };
            if let Some(selected) = selected {
                if selected.len() >= REMOVAL_RECOVERY_MAX_ENTRIES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "too many private model download files",
                    ));
                }
                selected.push(name);
            }
            Ok(())
        });
        match scan_result {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(source) => {
                return Err(ModelAssetError::Storage {
                    path: storage.absolute(parent),
                    source,
                });
            }
        }
        candidates.sort();
        for name in recovery_candidates {
            let path = parent.join(name);
            let identity = storage
                .identity(&path)
                .map_err(|_| ModelAssetError::NotOwned)?
                .ok_or(ModelAssetError::NotOwned)?;
            if identity.links != 1 || identity.size > PRIVATE_WRITE_RECOVERY_MAX_BYTES {
                return Err(ModelAssetError::NotOwned);
            }
            storage
                .remove_file_durable(&path)
                .map_err(|_| ModelAssetError::NotOwned)?;
        }
        let recovery_identity = storage
            .identity(&recovery)
            .map_err(|_| ModelAssetError::NotOwned)?;
        let Some(recovery_identity) = recovery_identity else {
            for candidate in candidates {
                let path = parent.join(candidate);
                let identity = storage
                    .identity(&path)
                    .map_err(|_| ModelAssetError::NotOwned)?
                    .ok_or(ModelAssetError::NotOwned)?;
                if identity.links != 1 || identity.size != 0 {
                    return Err(ModelAssetError::NotOwned);
                }
                storage
                    .remove_file_durable(&path)
                    .map_err(|_| ModelAssetError::NotOwned)?;
            }
            return Ok(());
        };
        if recovery_identity.links != 1 || recovery_identity.size > PRIVATE_WRITE_RECOVERY_MAX_BYTES
        {
            return Err(ModelAssetError::NotOwned);
        }
        let bytes = storage
            .read_bounded(&recovery, PRIVATE_WRITE_RECOVERY_MAX_BYTES)
            .map_err(|_| ModelAssetError::NotOwned)?;
        let plan: PrivateWriteRecovery =
            serde_json::from_slice(&bytes).map_err(|_| ModelAssetError::NotOwned)?;
        let expected = PartialMetadata::from_manifest(manifest, file);
        if plan.schema_version != 1
            || plan.owner != "ch.emin.piu"
            || plan.partial != expected
            || !safe_relative_path(&plan.temporary_path)
            || plan.temporary_path.parent() != Some(parent)
            || plan
                .temporary_path
                .file_name()
                .is_none_or(|name| !is_private_write_name_for(name, partial_name))
        {
            return Err(ModelAssetError::NotOwned);
        }
        let planned_name = plan
            .temporary_path
            .file_name()
            .expect("validated private write name");
        for candidate in &candidates {
            if candidate != planned_name {
                let path = parent.join(candidate);
                let identity = storage
                    .identity(&path)
                    .map_err(|_| ModelAssetError::NotOwned)?
                    .ok_or(ModelAssetError::NotOwned)?;
                if identity.links != 1 || identity.size != 0 {
                    return Err(ModelAssetError::NotOwned);
                }
                storage
                    .remove_file_durable(&path)
                    .map_err(|_| ModelAssetError::NotOwned)?;
            }
        }
        let planned_path = &plan.temporary_path;
        if let Some(candidate_identity) = storage
            .identity(planned_path)
            .map_err(|_| ModelAssetError::NotOwned)?
        {
            if candidate_identity.links != 1 || candidate_identity.size > file.size_bytes {
                return Err(ModelAssetError::NotOwned);
            }
            let existing_size = Self::bound_partial_size(storage, manifest, file);
            if storage
                .metadata(&partial)
                .map_err(|_| ModelAssetError::NotOwned)?
                .is_some()
                && existing_size.is_none()
            {
                return Err(ModelAssetError::NotOwned);
            }
            if candidate_identity.size == 0
                || existing_size.is_some_and(|size| size >= candidate_identity.size)
            {
                storage
                    .remove_file_durable(planned_path)
                    .map_err(|_| ModelAssetError::NotOwned)?;
            } else {
                storage
                    .rename_durable(planned_path, &partial)
                    .map_err(|_| ModelAssetError::NotOwned)?;
            }
        }
        storage
            .remove_file_durable(&recovery)
            .map_err(|_| ModelAssetError::NotOwned)
    }

    fn recover_staged_removal(
        storage: &SafeStorage,
        staging_directory: &Path,
    ) -> Result<(), ModelAssetError> {
        let directory_metadata = storage
            .metadata(staging_directory)
            .map_err(|source| ModelAssetError::Storage {
                path: storage.absolute(staging_directory),
                source,
            })?
            .ok_or(ModelAssetError::NotOwned)?;
        if !directory_metadata.is_dir() || directory_metadata.file_type().is_symlink() {
            return Err(ModelAssetError::NotOwned);
        }
        let recovery_path = staging_directory.join(REMOVAL_RECOVERY_FILE);
        let recovery_metadata =
            storage
                .metadata(&recovery_path)
                .map_err(|source| ModelAssetError::Storage {
                    path: storage.absolute(&recovery_path),
                    source,
                })?;
        if recovery_metadata.is_none() {
            return Self::remove_abandoned_empty_staging(storage, staging_directory);
        }
        let (bytes, plan_identity) = storage
            .read_bounded_with_identity(&recovery_path, REMOVAL_RECOVERY_MAX_BYTES)
            .map_err(|source| ModelAssetError::Storage {
                path: storage.absolute(&recovery_path),
                source,
            })?;
        if plan_identity.links != 1 {
            return Err(ModelAssetError::NotOwned);
        }
        let plan: RemovalRecoveryPlan =
            serde_json::from_slice(&bytes).map_err(|_| ModelAssetError::NotOwned)?;
        if !plan.validate(staging_directory) {
            return Err(ModelAssetError::NotOwned);
        }
        let allowed_names = plan
            .entries
            .iter()
            .filter_map(|entry| entry.staged.file_name().map(|name| name.to_os_string()))
            .chain([
                OsString::from(REMOVAL_RECOVERY_FILE),
                OsString::from(format!("{REMOVAL_RECOVERY_FILE}.tmp")),
            ])
            .collect::<HashSet<_>>();
        let mut unexpected_name = false;
        storage
            .scan_entry_names(Some(staging_directory), |name| {
                unexpected_name |=
                    !allowed_names.contains(&name) && !is_recovery_private_write_name(&name);
                Ok(())
            })
            .map_err(|source| ModelAssetError::Storage {
                path: storage.absolute(staging_directory),
                source,
            })?;
        if unexpected_name {
            return Err(ModelAssetError::NotOwned);
        }

        match plan.phase {
            RemovalRecoveryPhase::Staging => {
                for entry in plan.entries.iter().rev() {
                    let original_exists = storage
                        .metadata(&entry.original)
                        .map_err(|_| ModelAssetError::NotOwned)?
                        .is_some();
                    let staged_identity = storage
                        .identity(&entry.staged)
                        .map_err(|_| ModelAssetError::NotOwned)?;
                    match (original_exists, staged_identity) {
                        (false, Some(identity))
                            if identity.links == 1 && identity.size == entry.size_bytes =>
                        {
                            storage
                                .rename_durable(&entry.staged, &entry.original)
                                .map_err(|_| ModelAssetError::NotOwned)?;
                        }
                        (true, None) => {}
                        _ => return Err(ModelAssetError::NotOwned),
                    }
                }
            }
            RemovalRecoveryPhase::Deleting => {
                for entry in &plan.entries {
                    let Some(identity) = storage
                        .identity(&entry.staged)
                        .map_err(|_| ModelAssetError::NotOwned)?
                    else {
                        continue;
                    };
                    if Some(identity) != entry.identity {
                        return Err(ModelAssetError::NotOwned);
                    }
                    storage
                        .remove_file_durable(&entry.staged)
                        .map_err(|_| ModelAssetError::NotOwned)?;
                }
            }
        }
        storage
            .complete_recovery_mutations(&recovery_path)
            .map_err(|_| ModelAssetError::NotOwned)?;
        Self::remove_recovery_metadata(storage, staging_directory)?;
        storage
            .remove_dir_durable(staging_directory)
            .map_err(|_| ModelAssetError::NotOwned)
    }

    fn remove_abandoned_empty_staging(
        storage: &SafeStorage,
        staging_directory: &Path,
    ) -> Result<(), ModelAssetError> {
        let mut entries = Vec::with_capacity(2);
        storage
            .scan_entry_names(Some(staging_directory), |name| {
                if entries.len() < 2 {
                    entries.push(name);
                }
                Ok(())
            })
            .map_err(|_| ModelAssetError::NotOwned)?;
        if entries.is_empty() {
            return storage
                .remove_dir_durable(staging_directory)
                .map_err(|_| ModelAssetError::NotOwned);
        }
        if entries.len() == 1
            && (entries[0] == OsString::from(format!("{REMOVAL_RECOVERY_FILE}.tmp"))
                || is_recovery_private_write_name(&entries[0]))
        {
            let temporary = staging_directory.join(&entries[0]);
            if storage
                .identity(&temporary)
                .map_err(|_| ModelAssetError::NotOwned)?
                .is_some_and(|identity| {
                    identity.links == 1 && identity.size <= REMOVAL_RECOVERY_MAX_BYTES
                })
            {
                storage
                    .remove_file_durable(&temporary)
                    .map_err(|_| ModelAssetError::NotOwned)?;
                return storage
                    .remove_dir_durable(staging_directory)
                    .map_err(|_| ModelAssetError::NotOwned);
            }
        }
        Err(ModelAssetError::NotOwned)
    }

    fn remove_recovery_metadata(
        storage: &SafeStorage,
        staging_directory: &Path,
    ) -> Result<(), ModelAssetError> {
        let mut names = Vec::new();
        storage
            .scan_entry_names(Some(staging_directory), |name| {
                if name == REMOVAL_RECOVERY_FILE
                    || name == OsString::from(format!("{REMOVAL_RECOVERY_FILE}.tmp"))
                    || is_recovery_private_write_name(&name)
                {
                    if names.len() >= REMOVAL_RECOVERY_MAX_ENTRIES {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "too many private removal recovery files",
                        ));
                    }
                    names.push(name);
                }
                Ok(())
            })
            .map_err(|_| ModelAssetError::NotOwned)?;
        for name in names {
            let path = staging_directory.join(name);
            let Some(identity) = storage
                .identity(&path)
                .map_err(|_| ModelAssetError::NotOwned)?
            else {
                continue;
            };
            if identity.links != 1 || identity.size > REMOVAL_RECOVERY_MAX_BYTES {
                return Err(ModelAssetError::NotOwned);
            }
            storage
                .remove_file_durable(&path)
                .map_err(|_| ModelAssetError::NotOwned)?;
        }
        Ok(())
    }

    fn start_background_validation(&self) {
        let operation_id = self.0.next_operation_id.fetch_add(1, Ordering::Relaxed);
        let cancellation = CancellationToken::new();
        *self.0.active.lock().expect("model asset operation lock") = Some(ActiveOperation {
            operation_id,
            cancellation: cancellation.clone(),
            kind: OperationKind::Validation,
            can_cancel: true,
        });
        let mut status = self.status();
        status.operation_id = Some(operation_id);
        self.publish(status);
        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            let worker = manager.clone();
            let result = blocking_phase(move || {
                worker.validate_existing_install(operation_id, cancellation)
            })
            .await
            .and_then(|result| result);
            if let Err(error) = result {
                let failure = manager.clone();
                let _ =
                    blocking_phase(move || failure.publish_operation_failure(operation_id, &error))
                        .await;
            }
        });
    }

    pub fn subscribe(&self) -> watch::Receiver<ModelAssetStatus> {
        self.0.status.subscribe()
    }

    pub fn status(&self) -> ModelAssetStatus {
        self.0.status.borrow().clone()
    }

    pub fn start_download(&self) -> Result<u64, ModelAssetError> {
        let (operation_id, cancellation) = self.begin_download()?;
        let Some(cancellation) = cancellation else {
            return Ok(operation_id);
        };
        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            let recovery = manager.clone();
            let result = blocking_phase(move || recovery.reconcile_download_recovery(operation_id))
                .await
                .and_then(|result| result);
            let result = match result {
                Ok(()) => manager.download_all(operation_id, cancellation).await,
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                let failure = manager.clone();
                let _ =
                    blocking_phase(move || failure.publish_operation_failure(operation_id, &error))
                        .await;
            }
        });
        Ok(operation_id)
    }

    fn begin_download(&self) -> Result<(u64, Option<CancellationToken>), ModelAssetError> {
        self.ensure_initialized()?;
        let mut active = self.0.active.lock().expect("model asset operation lock");
        if let Some(active) = active.as_ref() {
            return if active.kind == OperationKind::Download {
                Ok((active.operation_id, None))
            } else {
                Err(ModelAssetError::OperationInProgress)
            };
        }
        let status = self.status();
        if status.phase == ModelAssetPhase::Ready {
            return Ok((0, None));
        }
        if status.phase == ModelAssetPhase::RevisionMismatch {
            return Err(ModelAssetError::RevisionMismatch);
        }
        if status.error_code == Some(ModelAssetErrorCode::Ownership) {
            return Err(ModelAssetError::NotOwned);
        }
        let operation_id = self.0.next_operation_id.fetch_add(1, Ordering::Relaxed);
        let cancellation = CancellationToken::new();
        *active = Some(ActiveOperation {
            operation_id,
            cancellation: cancellation.clone(),
            kind: OperationKind::Download,
            can_cancel: true,
        });
        drop(active);
        Ok((operation_id, Some(cancellation)))
    }

    pub fn cancel_download(&self) -> bool {
        let active = self.0.active.lock().expect("model asset operation lock");
        if let Some(active) = active.as_ref().filter(|active| active.can_cancel) {
            active.cancellation.cancel();
            true
        } else {
            false
        }
    }

    pub async fn authorize_hugging_face(&self, token: String) -> Result<(), ModelAssetError> {
        self.ensure_initialized()?;
        let token = token.trim().to_owned();
        if token.is_empty() {
            return Err(ModelAssetError::Authentication);
        }
        let response = tokio::time::timeout(
            self.0.network_timeouts.headers,
            self.0
                .client
                .get(&self.0.whoami_url)
                .bearer_auth(&token)
                .send(),
        )
        .await
        .map_err(|_| ModelAssetError::NetworkTimeout("authorization response headers"))??;
        if matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ) {
            return Err(ModelAssetError::Authentication);
        }
        response.error_for_status()?;
        let credentials = self.0.credentials.clone();
        blocking_phase(move || credentials.set(&token)).await??;
        let mut status = self.status();
        status.authentication_configured = true;
        status.error_code = None;
        if status.phase == ModelAssetPhase::AuthenticationRequired {
            status.phase = ModelAssetPhase::Missing;
            status.message = Some("Hugging Face access is connected. Resume the download.".into());
        }
        self.publish(status);
        Ok(())
    }

    pub async fn remove_owned_assets(&self) -> Result<ModelAssetStatus, ModelAssetError> {
        self.remove_owned_assets_with_hook(|_| {}).await
    }

    pub async fn retry_recovery(&self) -> Result<ModelAssetStatus, ModelAssetError> {
        self.ensure_initialized()?;
        let manager = self.clone();
        blocking_phase(move || {
            manager.reconcile_recovery_required()?;
            Ok(manager.status())
        })
        .await?
    }

    async fn remove_owned_assets_with_hook(
        &self,
        after_verified: impl FnMut(&Path) + Send + 'static,
    ) -> Result<ModelAssetStatus, ModelAssetError> {
        self.ensure_initialized()?;
        let recovery_manager = self.clone();
        blocking_phase(move || recovery_manager.reconcile_recovery_required()).await??;
        let (operation_id, cancellation) = self.begin_removal()?;
        let manager = self.clone();
        blocking_phase(move || {
            let result = manager.perform_owned_removal(operation_id, &cancellation, after_verified);
            if let Err(error) = &result {
                manager.publish_operation_failure(operation_id, error);
            }
            result
        })
        .await?
    }

    fn begin_removal(&self) -> Result<(u64, CancellationToken), ModelAssetError> {
        let mut active = self.0.active.lock().expect("model asset operation lock");
        if active.is_some() {
            return Err(ModelAssetError::OperationInProgress);
        }
        let operation_id = self.0.next_operation_id.fetch_add(1, Ordering::Relaxed);
        let cancellation = CancellationToken::new();
        *active = Some(ActiveOperation {
            operation_id,
            cancellation: cancellation.clone(),
            kind: OperationKind::Removal,
            can_cancel: true,
        });
        drop(active);
        let mut status = self.status();
        status.phase = ModelAssetPhase::Removing;
        status.operation_id = Some(operation_id);
        status.current_asset = None;
        status.current_file = None;
        status.error_code = None;
        status.message = Some("Verifying and removing Più-owned model assets.".into());
        self.publish(status);
        Ok((operation_id, cancellation))
    }

    fn commit_removal(
        &self,
        operation_id: u64,
        cancellation: &CancellationToken,
    ) -> Result<(), ModelAssetError> {
        let mut active = self.0.active.lock().expect("model asset operation lock");
        let Some(operation) = active.as_mut().filter(|operation| {
            operation.operation_id == operation_id && operation.kind == OperationKind::Removal
        }) else {
            return Err(ModelAssetError::Cancelled);
        };
        ensure_not_cancelled(cancellation)?;
        operation.can_cancel = false;
        drop(active);

        let mut status = self.status();
        status.message = Some("Finalizing removal. Più will finish this safely.".into());
        self.publish(status);
        Ok(())
    }

    fn reconcile_recovery_required(&self) -> Result<(), ModelAssetError> {
        self.serialize_recovery(|| self.reconcile_recovery_required_inner())
    }

    fn reconcile_recovery_required_inner(&self) -> Result<(), ModelAssetError> {
        if !self.0.recovery_required.load(Ordering::Acquire) {
            return Ok(());
        }
        if let Err(error) = Self::recover_staged_removals(self.storage())
            .and_then(|()| Self::recover_private_writes(self.storage(), &self.0.manifest))
        {
            return Err(ModelAssetError::RecoveryRequired(error.to_string()));
        }
        let free = self.0.disk_space.available(&self.0.root)?;
        let has_credentials = self.0.credentials.get()?.is_some();
        let inspection =
            Self::inspect_install(self.storage(), &self.0.manifest, free, has_credentials)?;
        self.0.recovery_required.store(false, Ordering::Release);
        *self.0.active.lock().expect("model asset operation lock") = None;
        self.publish(inspection.status);
        if inspection.requires_validation {
            self.start_background_validation();
        }
        Ok(())
    }

    fn reconcile_download_recovery(&self, operation_id: u64) -> Result<(), ModelAssetError> {
        self.serialize_recovery(|| self.reconcile_download_recovery_inner(operation_id))
    }

    fn reconcile_download_recovery_inner(&self, operation_id: u64) -> Result<(), ModelAssetError> {
        if !self.0.recovery_required.load(Ordering::Acquire) {
            return Ok(());
        }
        if let Err(error) = Self::recover_staged_removals(self.storage())
            .and_then(|()| Self::recover_private_writes(self.storage(), &self.0.manifest))
        {
            return Err(ModelAssetError::RecoveryRequired(error.to_string()));
        }
        let free = self.0.disk_space.available(&self.0.root)?;
        let has_credentials = self.0.credentials.get()?.is_some();
        let inspection =
            Self::inspect_install(self.storage(), &self.0.manifest, free, has_credentials)?;
        let active_matches = self
            .0
            .active
            .lock()
            .expect("model asset operation lock")
            .as_ref()
            .is_some_and(|active| active.operation_id == operation_id);
        if !active_matches {
            return Err(ModelAssetError::Cancelled);
        }
        self.0.recovery_required.store(false, Ordering::Release);
        let mut status = inspection.status;
        status.phase = ModelAssetPhase::Downloading;
        status.operation_id = Some(operation_id);
        status.message = Some("Recovered model resources. Resuming the download.".into());
        self.publish(status);
        Ok(())
    }

    /// Recovery callers enter through `blocking_phase`, so waiting here never
    /// occupies an asynchronous executor thread.
    fn serialize_recovery<T>(
        &self,
        operation: impl FnOnce() -> Result<T, ModelAssetError>,
    ) -> Result<T, ModelAssetError> {
        let _guard = self
            .0
            .recovery_serialization
            .lock()
            .expect("model asset recovery lock");
        operation()
    }

    fn require_recovery(
        &self,
        operation: &ModelAssetError,
        recovery: &ModelAssetError,
    ) -> ModelAssetError {
        self.0.recovery_required.store(true, Ordering::Release);
        ModelAssetError::RecoveryRequired(format!(
            "{operation}; rollback or reconciliation failed: {recovery}"
        ))
    }

    fn rollback_or_require_recovery(
        &self,
        staging_root: &Path,
        staged: &[StagedRemoval],
        operation: ModelAssetError,
    ) -> ModelAssetError {
        match self.rollback_staged(staging_root, staged) {
            Ok(()) => operation,
            Err(recovery) => self.require_recovery(&operation, &recovery),
        }
    }

    fn rollback_after_possible_stage(
        &self,
        staging_root: &Path,
        staged: &mut Vec<StagedRemoval>,
        original: &Path,
        staged_path: &Path,
        expected_size: u64,
        operation: ModelAssetError,
    ) -> ModelAssetError {
        let original_exists = self.storage().metadata(original).ok().flatten().is_some();
        let staged_identity = self.storage().identity(staged_path).ok().flatten();
        match (original_exists, staged_identity) {
            (true, None) => {}
            (false, Some(identity)) if identity.links == 1 && identity.size == expected_size => {
                if !staged.iter().any(|entry| entry.staged == staged_path) {
                    staged.push(StagedRemoval {
                        original: original.to_path_buf(),
                        staged: staged_path.to_path_buf(),
                        identity,
                    });
                }
            }
            _ => {
                self.0.recovery_required.store(true, Ordering::Release);
                return ModelAssetError::RecoveryRequired(format!(
                    "{operation}; the staged namespace could not be reconciled"
                ));
            }
        }
        self.rollback_or_require_recovery(staging_root, staged, operation)
    }

    fn mark_recovery_required(&self, operation: ModelAssetError) -> ModelAssetError {
        self.0.recovery_required.store(true, Ordering::Release);
        ModelAssetError::RecoveryRequired(operation.to_string())
    }

    fn perform_owned_removal(
        &self,
        operation_id: u64,
        cancellation: &CancellationToken,
        mut after_verified: impl FnMut(&Path),
    ) -> Result<ModelAssetStatus, ModelAssetError> {
        ensure_not_cancelled(cancellation)?;
        let marker_path = Path::new(OWNERSHIP_FILE);
        let (marker_bytes, marker_start_identity) = self
            .storage()
            .read_bounded_with_identity(marker_path, OWNERSHIP_METADATA_MAX_BYTES)
            .map_err(|source| self.relative_storage_error(marker_path, source))?;
        if marker_start_identity.links != 1 {
            return Err(ModelAssetError::NotOwned);
        }
        let marker: OwnershipMarker =
            serde_json::from_slice(&marker_bytes).map_err(|_| ModelAssetError::NotOwned)?;
        if !marker.authorizes_removal(&self.0.manifest) {
            return Err(ModelAssetError::NotOwned);
        }
        let mut recovery_entries = Vec::with_capacity(marker.files.len() + 1);
        for (index, file) in marker.files.iter().enumerate() {
            let relative = PathBuf::from(&file.install_path);
            let Some(metadata) = self
                .storage()
                .metadata(&relative)
                .map_err(|_| ModelAssetError::NotOwned)?
            else {
                continue;
            };
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || metadata.len() != file.size_bytes
                || self
                    .storage()
                    .identity(&relative)
                    .map_err(|_| ModelAssetError::NotOwned)?
                    .is_none_or(|identity| identity.links != 1)
            {
                return Err(ModelAssetError::NotOwned);
            }
            recovery_entries.push(RemovalRecoveryEntry {
                original: relative,
                staged: PathBuf::from(format!("asset-{index}")),
                size_bytes: file.size_bytes,
                sha256: file.sha256.clone(),
                identity: None,
            });
        }
        recovery_entries.push(RemovalRecoveryEntry {
            original: marker_path.to_path_buf(),
            staged: PathBuf::from("ownership-marker"),
            size_bytes: marker_bytes.len() as u64,
            sha256: sha256_hex(&marker_bytes),
            identity: None,
        });
        let staging_root = self.create_removal_staging()?;
        for entry in &mut recovery_entries {
            entry.staged = staging_root.join(&entry.staged);
        }
        let mut recovery_plan = RemovalRecoveryPlan {
            schema_version: 1,
            owner: "ch.emin.piu".into(),
            phase: RemovalRecoveryPhase::Staging,
            staging_directory: staging_root.clone(),
            entries: recovery_entries,
        };
        if let Err(error) = self.write_json_atomic_blocking(
            &staging_root.join(REMOVAL_RECOVERY_FILE),
            &recovery_plan,
            cancellation,
        ) {
            return match Self::remove_abandoned_empty_staging(self.storage(), &staging_root) {
                Ok(()) => Err(error),
                Err(recovery) => Err(self.require_recovery(&error, &recovery)),
            };
        }
        if let Err(source) = self.removal_persistence().commit_journal(
            self.storage(),
            &staging_root.join(REMOVAL_RECOVERY_FILE),
            RemovalRecoveryPhase::Staging,
        ) {
            let error =
                self.relative_storage_error(&staging_root.join(REMOVAL_RECOVERY_FILE), source);
            return Err(self.rollback_or_require_recovery(&staging_root, &[], error));
        }
        let mut staged = Vec::with_capacity(marker.files.len() + 1);
        for (index, file) in marker.files.iter().enumerate() {
            if let Err(error) = ensure_not_cancelled(cancellation) {
                return Err(self.rollback_or_require_recovery(&staging_root, &staged, error));
            }
            let relative = Path::new(&file.install_path);
            let Some(metadata) = self
                .storage()
                .metadata(relative)
                .map_err(|_| ModelAssetError::NotOwned)?
            else {
                continue;
            };
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || metadata.len() != file.size_bytes
            {
                return Err(self.rollback_or_require_recovery(
                    &staging_root,
                    &staged,
                    ModelAssetError::NotOwned,
                ));
            }
            let staged_path = staging_root.join(format!("asset-{index}"));
            if let Err(source) =
                self.removal_persistence()
                    .stage(self.storage(), relative, &staged_path)
            {
                let error = self.relative_storage_error(relative, source);
                return Err(self.rollback_after_possible_stage(
                    &staging_root,
                    &mut staged,
                    relative,
                    &staged_path,
                    file.size_bytes,
                    error,
                ));
            }
            let hash_result = self
                .sha256_relative_with_identity(&staged_path, cancellation)
                .map_err(|error| match error {
                    ModelAssetError::Cancelled => ModelAssetError::Cancelled,
                    _ => ModelAssetError::NotOwned,
                });
            let (hash, identity) = match hash_result {
                Ok(value) => value,
                Err(error) => {
                    return Err(self.rollback_after_possible_stage(
                        &staging_root,
                        &mut staged,
                        relative,
                        &staged_path,
                        file.size_bytes,
                        error,
                    ));
                }
            };
            staged.push(StagedRemoval {
                original: relative.to_path_buf(),
                staged: staged_path,
                identity,
            });
            if hash != file.sha256 || identity.size != file.size_bytes {
                return Err(self.rollback_or_require_recovery(
                    &staging_root,
                    &staged,
                    ModelAssetError::NotOwned,
                ));
            }
            after_verified(relative);
        }

        if let Err(error) = self.commit_removal(operation_id, cancellation) {
            return Err(self.rollback_or_require_recovery(&staging_root, &staged, error));
        }
        let staged_marker = staging_root.join("ownership-marker");
        if let Err(source) =
            self.removal_persistence()
                .stage(self.storage(), marker_path, &staged_marker)
        {
            let error = self.relative_storage_error(marker_path, source);
            return Err(self.rollback_after_possible_stage(
                &staging_root,
                &mut staged,
                marker_path,
                &staged_marker,
                marker_bytes.len() as u64,
                error,
            ));
        }
        let marker_result = self
            .storage()
            .read_bounded_with_identity(&staged_marker, OWNERSHIP_METADATA_MAX_BYTES)
            .map_err(|_| ModelAssetError::NotOwned);
        let (staged_marker_bytes, marker_identity) = match marker_result {
            Ok(value) => value,
            Err(error) => {
                return Err(self.rollback_after_possible_stage(
                    &staging_root,
                    &mut staged,
                    marker_path,
                    &staged_marker,
                    marker_bytes.len() as u64,
                    error,
                ));
            }
        };
        staged.push(StagedRemoval {
            original: marker_path.to_path_buf(),
            staged: staged_marker,
            identity: marker_identity,
        });
        if staged_marker_bytes != marker_bytes || self.staged_identities_match(&staged).is_err() {
            return Err(self.rollback_or_require_recovery(
                &staging_root,
                &staged,
                ModelAssetError::NotOwned,
            ));
        }
        if let Err(error) = ensure_not_cancelled(cancellation) {
            return Err(self.rollback_or_require_recovery(&staging_root, &staged, error));
        }
        recovery_plan.phase = RemovalRecoveryPhase::Deleting;
        for entry in &mut recovery_plan.entries {
            entry.identity = staged
                .iter()
                .find(|staged| staged.staged == entry.staged)
                .map(|staged| staged.identity);
            if entry.identity.is_none() {
                return Err(self.rollback_or_require_recovery(
                    &staging_root,
                    &staged,
                    ModelAssetError::NotOwned,
                ));
            }
        }
        if let Err(error) = self.write_json_atomic_blocking(
            &staging_root.join(REMOVAL_RECOVERY_FILE),
            &recovery_plan,
            cancellation,
        ) {
            return Err(self.rollback_or_require_recovery(&staging_root, &staged, error));
        }
        if let Err(source) = self.removal_persistence().commit_journal(
            self.storage(),
            &staging_root.join(REMOVAL_RECOVERY_FILE),
            RemovalRecoveryPhase::Deleting,
        ) {
            let error =
                self.relative_storage_error(&staging_root.join(REMOVAL_RECOVERY_FILE), source);
            return Err(self.rollback_or_require_recovery(&staging_root, &staged, error));
        }
        for entry in &staged {
            let identity = match self.storage().identity(&entry.staged) {
                Ok(identity) => identity,
                Err(_) => return Err(self.mark_recovery_required(ModelAssetError::NotOwned)),
            };
            if identity != Some(entry.identity) {
                return Err(self.mark_recovery_required(ModelAssetError::NotOwned));
            }
            if let Err(source) = self
                .removal_persistence()
                .delete(self.storage(), &entry.staged)
            {
                return Err(
                    self.mark_recovery_required(self.relative_storage_error(&entry.staged, source))
                );
            }
        }
        if let Err(source) = self
            .removal_persistence()
            .complete_mutations(self.storage(), &staging_root.join(REMOVAL_RECOVERY_FILE))
        {
            return Err(self.mark_recovery_required(
                self.relative_storage_error(&staging_root.join(REMOVAL_RECOVERY_FILE), source),
            ));
        }
        if let Err(error) = Self::remove_recovery_metadata(self.storage(), &staging_root) {
            return Err(self.mark_recovery_required(error));
        }
        if let Err(source) = self.storage().remove_dir_durable(&staging_root) {
            return Err(
                self.mark_recovery_required(self.relative_storage_error(&staging_root, source))
            );
        }
        self.0
            .invalid_finals
            .lock()
            .expect("invalid model assets lock")
            .clear();
        let mut active = self.0.active.lock().expect("model asset operation lock");
        if active.as_ref().map(|active| active.operation_id) != Some(operation_id) {
            return Err(ModelAssetError::Cancelled);
        }
        *active = None;
        drop(active);
        let free = self.0.disk_space.available(&self.0.root)?;
        let status =
            ModelAssetStatus::missing(&self.0.manifest, free, self.0.credentials.get()?.is_some());
        self.publish(status.clone());
        Ok(status)
    }

    fn create_removal_staging(&self) -> Result<PathBuf, ModelAssetError> {
        let operation_id = self.0.next_operation_id.fetch_add(1, Ordering::Relaxed);
        for attempt in 0..64 {
            let path = PathBuf::from(format!(
                ".piu-removal-{}-{operation_id}-{attempt}",
                std::process::id()
            ));
            match self.storage().create_dir(&path) {
                Ok(()) => {
                    self.storage()
                        .sync_parent(&path)
                        .map_err(|source| self.relative_storage_error(&path, source))?;
                    return Ok(path);
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(source) => return Err(self.relative_storage_error(&path, source)),
            }
        }
        Err(ModelAssetError::NotOwned)
    }

    fn staged_identities_match(&self, staged: &[StagedRemoval]) -> Result<(), ModelAssetError> {
        for entry in staged {
            if self
                .storage()
                .identity(&entry.staged)
                .map_err(|_| ModelAssetError::NotOwned)?
                != Some(entry.identity)
            {
                return Err(ModelAssetError::NotOwned);
            }
        }
        Ok(())
    }

    fn rollback_staged(
        &self,
        staging_root: &Path,
        staged: &[StagedRemoval],
    ) -> Result<(), ModelAssetError> {
        for entry in staged.iter().rev() {
            if self
                .storage()
                .identity(&entry.staged)
                .map_err(|_| ModelAssetError::NotOwned)?
                != Some(entry.identity)
            {
                return Err(ModelAssetError::NotOwned);
            }
            if self
                .storage()
                .metadata(&entry.original)
                .map_err(|_| ModelAssetError::NotOwned)?
                .is_some()
            {
                return Err(ModelAssetError::NotOwned);
            }
            self.storage()
                .rename_durable(&entry.staged, &entry.original)
                .map_err(|_| ModelAssetError::NotOwned)?;
        }
        self.storage()
            .complete_recovery_mutations(&staging_root.join(REMOVAL_RECOVERY_FILE))
            .map_err(|_| ModelAssetError::NotOwned)?;
        Self::remove_recovery_metadata(self.storage(), staging_root)?;
        self.storage()
            .remove_dir_durable(staging_root)
            .map_err(|_| ModelAssetError::NotOwned)
    }

    async fn download_all(
        &self,
        operation_id: u64,
        cancellation: CancellationToken,
    ) -> Result<(), ModelAssetError> {
        let sampler = self.clone();
        let (transferred, free, remaining, required) = blocking_phase(move || {
            let transferred = sampler.transferred_bytes();
            sampler
                .ensure_space(transferred)
                .map(|(free, remaining, required)| (transferred, free, remaining, required))
        })
        .await??;
        let mut status = self.status();
        status.phase = ModelAssetPhase::Downloading;
        status.operation_id = Some(operation_id);
        status.transferred_bytes = transferred;
        status.remaining_bytes = remaining;
        status.current_free_bytes = free;
        status.required_free_bytes = required;
        status.can_resume = transferred > 0;
        status.message = None;
        status.error_code = None;
        self.publish(status);

        for file in &self.0.manifest.files {
            if cancellation.is_cancelled() {
                return Err(ModelAssetError::Cancelled);
            }
            let validator = self.clone();
            let file_for_validation = file.clone();
            let validation_cancellation = cancellation.clone();
            if blocking_phase(move || {
                validator.final_is_valid(
                    &file_for_validation,
                    operation_id,
                    &validation_cancellation,
                )
            })
            .await??
            {
                continue;
            }
            // Hash validation may have removed a same-size corrupt final after the
            // optimistic startup sample. Recompute the gate before allocating its
            // replacement so stale bytes can never hide the actual space requirement.
            let sampler = self.clone();
            let (free, remaining, required) = blocking_phase(move || {
                let transferred = sampler.transferred_bytes();
                sampler.ensure_space(transferred)
            })
            .await??;
            let mut status = self.status();
            status.current_free_bytes = free;
            status.remaining_bytes = remaining;
            status.required_free_bytes = required;
            self.publish(status);
            self.download_file(file, operation_id, &cancellation)
                .await?;
        }
        ensure_not_cancelled(&cancellation)?;
        self.write_ownership_marker(&cancellation).await?;
        self.publish_ready(operation_id, &cancellation)?;
        Ok(())
    }

    fn ensure_space(&self, transferred: u64) -> Result<(u64, u64, u64), ModelAssetError> {
        let remaining = self
            .0
            .manifest
            .total_size_bytes()
            .saturating_sub(transferred);
        let free = self.0.disk_space.available(&self.0.root)?;
        let required = remaining.saturating_add(DISK_SAFETY_RESERVE_BYTES);
        if free < required {
            return Err(ModelAssetError::InsufficientSpace {
                available: free,
                required,
            });
        }
        Ok((free, remaining, required))
    }

    fn validate_existing_install(
        &self,
        operation_id: u64,
        cancellation: CancellationToken,
    ) -> Result<(), ModelAssetError> {
        for file in &self.0.manifest.files {
            ensure_not_cancelled(&cancellation)?;
            let mut status = self.status();
            status.phase = ModelAssetPhase::Verifying;
            status.operation_id = Some(operation_id);
            status.current_asset = Some(file.asset);
            status.current_file = Some(file.install_path.clone());
            status.current_free_bytes = self.0.disk_space.available(&self.0.root)?;
            self.publish(status);
            match self.sha256_relative(Path::new(&file.install_path), &cancellation) {
                Ok(hash) if hash == file.sha256 => {}
                Err(ModelAssetError::Cancelled) => return Err(ModelAssetError::Cancelled),
                result => {
                    self.0
                        .invalid_finals
                        .lock()
                        .expect("invalid model assets lock")
                        .insert(file.install_path.clone());
                    return match result {
                        Err(error) => Err(error),
                        Ok(_) => Err(ModelAssetError::Integrity(file.source_path.clone())),
                    };
                }
            }
        }
        ensure_not_cancelled(&cancellation)?;
        self.publish_ready(operation_id, &cancellation)
    }

    fn publish_ready(
        &self,
        operation_id: u64,
        cancellation: &CancellationToken,
    ) -> Result<(), ModelAssetError> {
        let mut active = self.0.active.lock().expect("model asset download lock");
        if cancellation.is_cancelled()
            || active.as_ref().map(|active| active.operation_id) != Some(operation_id)
        {
            return Err(ModelAssetError::Cancelled);
        }
        *active = None;
        drop(active);
        let mut status = self.status();
        status.phase = ModelAssetPhase::Ready;
        status.transferred_bytes = status.total_bytes;
        status.remaining_bytes = 0;
        status.required_free_bytes = 0;
        status.current_asset = None;
        status.current_file = None;
        status.operation_id = None;
        status.can_resume = false;
        status.message = Some("The local model and MTP drafter are ready.".into());
        status.error_code = None;
        self.publish(status);
        Ok(())
    }

    async fn download_file(
        &self,
        manifest_file: &ManifestFile,
        operation_id: u64,
        cancellation: &CancellationToken,
    ) -> Result<(), ModelAssetError> {
        let destination = PathBuf::from(&manifest_file.install_path);
        let partial = partial_path(&destination);
        let metadata_path = partial_metadata_path(&partial);
        let preparation = self.clone();
        let prepared_file = manifest_file.clone();
        let prepared_partial = partial.clone();
        let prepared_metadata_path = metadata_path.clone();
        let (expected_metadata, mut offset) = blocking_phase(move || {
            Self::recover_private_write(
                preparation.storage(),
                &preparation.0.manifest,
                &prepared_file,
            )?;
            let expected_metadata =
                PartialMetadata::from_manifest(&preparation.0.manifest, &prepared_file);
            let recorded_metadata = preparation
                .storage()
                .read_bounded(&prepared_metadata_path, PARTIAL_METADATA_MAX_BYTES)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<PartialMetadata>(&bytes).ok());
            let partial_metadata = preparation
                .storage()
                .metadata(&prepared_partial)
                .map_err(|source| preparation.relative_storage_error(&prepared_partial, source))?;
            let safe_bound_partial = partial_metadata.as_ref().is_some_and(|metadata| {
                metadata.is_file()
                    && !metadata.file_type().is_symlink()
                    && recorded_metadata.as_ref() == Some(&expected_metadata)
                    && metadata.len() <= prepared_file.size_bytes
            });
            if !safe_bound_partial && (partial_metadata.is_some() || recorded_metadata.is_some()) {
                preparation.reset_partial(&prepared_partial, &prepared_metadata_path)?;
            }
            let offset = if safe_bound_partial {
                partial_metadata.expect("bound partial metadata").len()
            } else {
                0
            };
            Ok::<_, ModelAssetError>((expected_metadata, offset))
        })
        .await??;
        if offset == 0 {
            self.write_json_atomic(&metadata_path, &expected_metadata, cancellation)
                .await?;
        }
        if offset == manifest_file.size_bytes {
            let mut status = self.status();
            status.phase = ModelAssetPhase::Verifying;
            status.current_asset = Some(manifest_file.asset);
            status.current_file = Some(manifest_file.install_path.clone());
            self.publish(status);
            let validator = self.clone();
            let validation_partial = partial.clone();
            let validation_cancellation = cancellation.clone();
            let hash = blocking_phase(move || {
                validator.sha256_relative(&validation_partial, &validation_cancellation)
            })
            .await??;
            if hash == manifest_file.sha256 {
                ensure_not_cancelled(cancellation)?;
                let finalizer = self.clone();
                let final_partial = partial.clone();
                let final_destination = destination.clone();
                let final_metadata_path = metadata_path.clone();
                let install_path = manifest_file.install_path.clone();
                blocking_phase(move || {
                    finalizer
                        .storage()
                        .rename(&final_partial, &final_destination)
                        .map_err(|source| {
                            finalizer.relative_storage_error(&final_destination, source)
                        })?;
                    finalizer.remove_if_present(&final_metadata_path)?;
                    finalizer.clear_invalid_final(&install_path);
                    Ok::<_, ModelAssetError>(())
                })
                .await??;
                return Ok(());
            }
            let resetter = self.clone();
            let reset_partial = partial.clone();
            let reset_metadata = metadata_path.clone();
            blocking_phase(move || resetter.reset_partial(&reset_partial, &reset_metadata))
                .await??;
            self.write_json_atomic(&metadata_path, &expected_metadata, cancellation)
                .await?;
            offset = 0;
        }
        let credentials = self.0.credentials.clone();
        let token = blocking_phase(move || credentials.get()).await??;
        let publisher = self.clone();
        let published_file = manifest_file.clone();
        blocking_phase(move || {
            publisher.publish_current_work(
                &published_file,
                operation_id,
                ModelAssetPhase::Downloading,
            )
        })
        .await?;
        let url = format!("{}/{}", self.0.resolve_base_url, manifest_file.source_path);
        let mut request = self.0.client.get(url);
        if offset > 0 {
            request = request.header(header::RANGE, format!("bytes={offset}-"));
        }
        if let Some(token) = token.as_ref() {
            request = request.bearer_auth(token);
        }
        let response = tokio::select! {
            _ = cancellation.cancelled() => return Err(ModelAssetError::Cancelled),
            result = tokio::time::timeout(self.0.network_timeouts.headers, request.send()) => {
                result
                    .map_err(|_| ModelAssetError::NetworkTimeout("response headers"))??
            }
        };
        if matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ) {
            return Err(ModelAssetError::Authentication);
        }
        if offset > 0 && response.status() == StatusCode::OK {
            offset = 0;
        } else if offset > 0 && response.status() != StatusCode::PARTIAL_CONTENT {
            return Err(ModelAssetError::InvalidRange(
                manifest_file.source_path.clone(),
            ));
        } else if offset == 0 && !response.status().is_success() {
            return Err(response
                .error_for_status()
                .expect_err("non-success response")
                .into());
        }
        if offset > 0 {
            let expected = format!("bytes {offset}-");
            let content_range = response
                .headers()
                .get(header::CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default();
            if !content_range.starts_with(&expected) {
                return Err(ModelAssetError::InvalidRange(
                    manifest_file.source_path.clone(),
                ));
            }
        }

        let output_manager = self.clone();
        let output_partial = partial.clone();
        let mut output = blocking_phase(move || {
            output_manager
                .storage()
                .create_private_write(&output_partial)
                .map_err(|source| output_manager.relative_storage_error(&output_partial, source))
        })
        .await??;
        let private_recovery_path = private_write_recovery_path(&partial);
        let private_recovery = PrivateWriteRecovery {
            schema_version: 1,
            owner: "ch.emin.piu".into(),
            temporary_path: output.temporary_path().to_path_buf(),
            partial: PartialMetadata::from_manifest(&self.0.manifest, manifest_file),
        };
        if let Err(error) = self
            .write_json_atomic(&private_recovery_path, &private_recovery, cancellation)
            .await
        {
            output
                .discard()
                .await
                .map_err(|source| self.relative_storage_error(&partial, source))?;
            return Err(error);
        }
        if offset > 0 {
            let adopter = self.clone();
            let adopt_partial = partial.clone();
            let adopt_recovery = private_recovery_path.clone();
            output = blocking_phase(move || {
                let expected_identity = adopter
                    .storage()
                    .identity(&adopt_partial)
                    .map_err(|source| adopter.relative_storage_error(&adopt_partial, source))?
                    .filter(|identity| identity.links == 1 && identity.size == offset)
                    .ok_or_else(|| {
                        adopter.relative_storage_error(
                            &adopt_partial,
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                "model asset partial changed before private transfer",
                            ),
                        )
                    })?;
                if let Err(source) = output.adopt_existing(expected_identity) {
                    output.discard_blocking().map_err(|cleanup| {
                        adopter.relative_storage_error(&adopt_partial, cleanup)
                    })?;
                    adopter.remove_if_present_durable(&adopt_recovery)?;
                    return Err(adopter.relative_storage_error(&adopt_partial, source));
                }
                Ok::<_, ModelAssetError>(output)
            })
            .await??;
        }
        let mut stream = response.bytes_stream();
        let mut file_bytes = offset;
        let sampler = self.clone();
        let sampled_total = blocking_phase(move || sampler.transferred_bytes()).await?;
        let mut progress = TransferProgress::new(sampled_total, offset);
        let transfer_result = async {
            loop {
                let chunk = tokio::select! {
                    _ = cancellation.cancelled() => return Err(ModelAssetError::Cancelled),
                    result = tokio::time::timeout(self.0.network_timeouts.read, stream.next()) => {
                        result.map_err(|_| ModelAssetError::NetworkTimeout("response data"))?
                    }
                };
                let Some(chunk) = chunk else { break };
                let chunk = chunk?;
                let chunk_bytes = chunk.len() as u64;
                let actual = file_bytes.saturating_add(chunk_bytes);
                if chunk_bytes > manifest_file.size_bytes.saturating_sub(file_bytes) {
                    return Err(ModelAssetError::SizeMismatch {
                        path: manifest_file.source_path.clone(),
                        actual,
                        expected: manifest_file.size_bytes,
                    });
                }
                output
                    .write_all(&chunk)
                    .await
                    .map_err(|source| self.relative_storage_error(&partial, source))?;
                file_bytes = actual;
                if let Some(total) = progress.observe(file_bytes, manifest_file.size_bytes) {
                    self.publish_progress(manifest_file, operation_id, total);
                }
            }
            if file_bytes != manifest_file.size_bytes {
                return Err(ModelAssetError::SizeMismatch {
                    path: manifest_file.source_path.clone(),
                    actual: file_bytes,
                    expected: manifest_file.size_bytes,
                });
            }
            Ok(())
        }
        .await;
        if let Err(error) = transfer_result {
            output
                .publish()
                .await
                .map_err(|source| self.relative_storage_error(&partial, source))?;
            let cleaner = self.clone();
            let recovery = private_recovery_path.clone();
            blocking_phase(move || cleaner.remove_if_present_durable(&recovery)).await??;
            return Err(error);
        }
        output
            .publish()
            .await
            .map_err(|source| self.relative_storage_error(&partial, source))?;
        let cleaner = self.clone();
        let recovery = private_recovery_path.clone();
        blocking_phase(move || cleaner.remove_if_present_durable(&recovery)).await??;
        let mut status = self.status();
        status.phase = ModelAssetPhase::Verifying;
        status.current_asset = Some(manifest_file.asset);
        status.current_file = Some(manifest_file.install_path.clone());
        self.publish(status);
        let verifier = self.clone();
        let verified_partial = partial.clone();
        let verification_cancellation = cancellation.clone();
        let hash = blocking_phase(move || {
            verifier.sha256_relative(&verified_partial, &verification_cancellation)
        })
        .await??;
        if hash != manifest_file.sha256 {
            let resetter = self.clone();
            let reset_partial = partial.clone();
            let reset_metadata = metadata_path.clone();
            blocking_phase(move || resetter.reset_partial(&reset_partial, &reset_metadata))
                .await??;
            return Err(ModelAssetError::Integrity(
                manifest_file.source_path.clone(),
            ));
        }
        ensure_not_cancelled(cancellation)?;
        let finalizer = self.clone();
        let final_partial = partial.clone();
        let final_destination = destination.clone();
        let final_metadata = metadata_path.clone();
        let install_path = manifest_file.install_path.clone();
        blocking_phase(move || {
            finalizer
                .storage()
                .rename(&final_partial, &final_destination)
                .map_err(|source| finalizer.relative_storage_error(&final_destination, source))?;
            finalizer.remove_if_present(&final_metadata)?;
            finalizer.clear_invalid_final(&install_path);
            Ok::<_, ModelAssetError>(())
        })
        .await??;
        Ok(())
    }

    fn final_is_valid(
        &self,
        file: &ManifestFile,
        operation_id: u64,
        cancellation: &CancellationToken,
    ) -> Result<bool, ModelAssetError> {
        let path = Path::new(&file.install_path);
        let invalid = self
            .0
            .invalid_finals
            .lock()
            .expect("invalid model assets lock")
            .contains(&file.install_path);
        let Some(metadata) = self
            .storage()
            .metadata(path)
            .map_err(|source| self.relative_storage_error(path, source))?
        else {
            self.clear_invalid_final(&file.install_path);
            return Ok(false);
        };
        if !invalid
            && metadata.is_file()
            && !metadata.file_type().is_symlink()
            && metadata.len() == file.size_bytes
        {
            self.publish_current_work(file, operation_id, ModelAssetPhase::Verifying);
            match self.sha256_relative(path, cancellation) {
                Ok(hash) if hash == file.sha256 => {
                    self.remove_if_present(&partial_metadata_path(&partial_path(path)))?;
                    return Ok(true);
                }
                Err(ModelAssetError::ChangedDuringVerification(changed)) => {
                    self.0
                        .invalid_finals
                        .lock()
                        .expect("invalid model assets lock")
                        .insert(file.install_path.clone());
                    return Err(ModelAssetError::ChangedDuringVerification(changed));
                }
                Err(error) => return Err(error),
                Ok(_) => {}
            }
        }
        self.storage()
            .remove_file(path)
            .map_err(|source| self.relative_storage_error(path, source))?;
        self.clear_invalid_final(&file.install_path);
        Ok(false)
    }

    #[cfg(test)]
    fn finish_operation(&self, operation_id: u64, result: Result<(), ModelAssetError>) {
        let Err(error) = result else { return };
        self.publish_operation_failure(operation_id, &error);
    }

    fn publish_operation_failure(&self, operation_id: u64, error: &ModelAssetError) {
        let recovery_required = self.0.recovery_required.load(Ordering::Acquire);
        let mut active = self.0.active.lock().expect("model asset operation lock");
        if !recovery_required
            && active.as_ref().map(|active| active.operation_id) == Some(operation_id)
        {
            *active = None;
        }
        drop(active);
        let mut status = self.status();
        if !recovery_required {
            status.operation_id = None;
        }
        status.current_asset = None;
        status.current_file = None;
        status.transferred_bytes = self.transferred_bytes();
        status.remaining_bytes = status.total_bytes.saturating_sub(status.transferred_bytes);
        status.required_free_bytes = status
            .remaining_bytes
            .saturating_add(DISK_SAFETY_RESERVE_BYTES);
        if let ModelAssetError::InsufficientSpace {
            available,
            required,
        } = &error
        {
            status.current_free_bytes = *available;
            status.required_free_bytes = *required;
        } else if let Ok(free) = self.0.disk_space.available(&self.0.root) {
            status.current_free_bytes = free;
        }
        status.can_resume = status.transferred_bytes > 0;
        status.message = Some(if recovery_required {
            format!(
                "Più couldn't finish recovering model storage. Resolve the reported storage problem, then retry recovery. {error}"
            )
        } else {
            error.to_string()
        });
        status.error_code = Some(error.code());
        status.phase = match error {
            ModelAssetError::Cancelled => ModelAssetPhase::Cancelled,
            ModelAssetError::Authentication => ModelAssetPhase::AuthenticationRequired,
            ModelAssetError::RevisionMismatch => ModelAssetPhase::RevisionMismatch,
            _ => ModelAssetPhase::Failed,
        };
        self.publish(status);
    }

    fn publish_progress(&self, file: &ManifestFile, operation_id: u64, transferred: u64) {
        let mut status = self.status();
        status.phase = ModelAssetPhase::Downloading;
        status.operation_id = Some(operation_id);
        status.current_asset = Some(file.asset);
        status.current_file = Some(file.install_path.clone());
        status.transferred_bytes = transferred.min(status.total_bytes);
        status.remaining_bytes = status.total_bytes.saturating_sub(status.transferred_bytes);
        status.required_free_bytes = status
            .remaining_bytes
            .saturating_add(DISK_SAFETY_RESERVE_BYTES);
        status.can_resume = status.transferred_bytes > 0;
        self.publish(status);
    }

    fn publish_current_work(&self, file: &ManifestFile, operation_id: u64, phase: ModelAssetPhase) {
        let transferred = self.transferred_bytes();
        let mut status = self.status();
        status.phase = phase;
        status.operation_id = Some(operation_id);
        status.current_asset = Some(file.asset);
        status.current_file = Some(file.install_path.clone());
        status.transferred_bytes = transferred;
        status.remaining_bytes = status.total_bytes.saturating_sub(transferred);
        status.required_free_bytes = status
            .remaining_bytes
            .saturating_add(DISK_SAFETY_RESERVE_BYTES);
        status.can_resume = transferred > 0;
        if let Ok(free) = self.0.disk_space.available(&self.0.root) {
            status.current_free_bytes = free;
        }
        self.publish(status);
    }

    fn available_actions(&self, status: &ModelAssetStatus) -> Vec<ModelAssetAction> {
        if self.0.storage.get().is_none() {
            return Vec::new();
        }
        if self.0.recovery_required.load(Ordering::Acquire) {
            return vec![ModelAssetAction::RetryRecovery];
        }
        if let Some(active) = self
            .0
            .active
            .lock()
            .expect("model asset operation lock")
            .as_ref()
        {
            return active
                .can_cancel
                .then_some(ModelAssetAction::Cancel)
                .into_iter()
                .collect();
        }
        match status.phase {
            ModelAssetPhase::Missing | ModelAssetPhase::Cancelled => {
                vec![ModelAssetAction::Download]
            }
            ModelAssetPhase::AuthenticationRequired => vec![ModelAssetAction::Authorize],
            ModelAssetPhase::Ready | ModelAssetPhase::RevisionMismatch => {
                vec![ModelAssetAction::Remove]
            }
            ModelAssetPhase::Failed
                if status.error_code != Some(ModelAssetErrorCode::Ownership) =>
            {
                vec![ModelAssetAction::Download]
            }
            ModelAssetPhase::Initializing
            | ModelAssetPhase::Downloading
            | ModelAssetPhase::Verifying
            | ModelAssetPhase::Removing
            | ModelAssetPhase::Failed => Vec::new(),
        }
    }

    fn publish(&self, mut status: ModelAssetStatus) {
        status.available_actions = self.available_actions(&status);
        self.0.status.send_replace(status);
    }

    fn transferred_bytes(&self) -> u64 {
        let storage = self.storage();
        let invalid_finals = self
            .0
            .invalid_finals
            .lock()
            .expect("invalid model assets lock");
        self.0
            .manifest
            .files
            .iter()
            .map(|file| {
                if invalid_finals.contains(&file.install_path) {
                    return 0;
                }
                storage
                    .metadata(Path::new(&file.install_path))
                    .ok()
                    .flatten()
                    .filter(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
                    .map(|metadata| metadata.len().min(file.size_bytes))
                    .unwrap_or_else(|| {
                        Self::bound_partial_size(storage, &self.0.manifest, file).unwrap_or(0)
                    })
            })
            .sum()
    }

    fn inspect_install(
        storage: &SafeStorage,
        manifest: &AssetManifest,
        free_bytes: u64,
        authentication_configured: bool,
    ) -> Result<InstallInspection, ModelAssetError> {
        let mut status = ModelAssetStatus::missing(manifest, free_bytes, authentication_configured);
        let marker_relative = Path::new(OWNERSHIP_FILE);
        if storage
            .metadata(marker_relative)
            .map_err(|source| ModelAssetError::Storage {
                path: storage.absolute(marker_relative),
                source,
            })?
            .is_some()
        {
            let marker: OwnershipMarker = serde_json::from_slice(
                &storage
                    .read_bounded(marker_relative, OWNERSHIP_METADATA_MAX_BYTES)
                    .map_err(|source| ModelAssetError::Storage {
                        path: storage.absolute(marker_relative),
                        source,
                    })?,
            )
            .map_err(|_| ModelAssetError::NotOwned)?;
            let marker_is_current = marker.matches(manifest);
            let marker_is_removable_old_revision =
                marker.revision != manifest.revision && marker.authorizes_removal(manifest);
            if marker_is_removable_old_revision {
                status.phase = ModelAssetPhase::RevisionMismatch;
                status.message = Some(ModelAssetError::RevisionMismatch.to_string());
                status.error_code = Some(ModelAssetErrorCode::RevisionMismatch);
                return Ok(InstallInspection {
                    status,
                    requires_validation: false,
                });
            }
            if !marker_is_current {
                status.phase = ModelAssetPhase::Failed;
                status.error_code = Some(ModelAssetErrorCode::Ownership);
                status.message = Some(
                    "Più found unsupported pre-release model ownership metadata and left every file untouched. Reset Più's pre-release application data, then download the pinned model again."
                        .into(),
                );
                return Ok(InstallInspection {
                    status,
                    requires_validation: false,
                });
            }
            let all_files_have_pinned_size = manifest.files.iter().all(|file| {
                storage
                    .metadata(Path::new(&file.install_path))
                    .ok()
                    .flatten()
                    .map(|metadata| {
                        metadata.is_file()
                            && !metadata.file_type().is_symlink()
                            && metadata.len() == file.size_bytes
                    })
                    .unwrap_or(false)
            });
            if marker_is_current && all_files_have_pinned_size {
                // Size is only a cheap candidate check. The model remains unavailable
                // until cancellation-aware background SHA-256 validation completes.
                status.phase = ModelAssetPhase::Verifying;
                status.transferred_bytes = status.total_bytes;
                status.remaining_bytes = 0;
                status.required_free_bytes = 0;
                status.message = Some("Verifying installed model assets before use.".into());
                return Ok(InstallInspection {
                    status,
                    requires_validation: true,
                });
            }
        }
        status.transferred_bytes = manifest
            .files
            .iter()
            .map(|file| {
                let final_relative = Path::new(&file.install_path);
                let final_size = storage
                    .metadata(final_relative)
                    .ok()
                    .flatten()
                    .filter(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
                    .map(|metadata| metadata.len().min(file.size_bytes));
                final_size.unwrap_or_else(|| {
                    Self::bound_partial_size(storage, manifest, file).unwrap_or(0)
                })
            })
            .sum();
        status.remaining_bytes = status.total_bytes.saturating_sub(status.transferred_bytes);
        status.required_free_bytes = status
            .remaining_bytes
            .saturating_add(DISK_SAFETY_RESERVE_BYTES);
        status.can_resume = status.transferred_bytes > 0;
        Ok(InstallInspection {
            status,
            requires_validation: false,
        })
    }

    fn bound_partial_size(
        storage: &SafeStorage,
        manifest: &AssetManifest,
        file: &ManifestFile,
    ) -> Option<u64> {
        let partial = partial_path(Path::new(&file.install_path));
        let metadata_relative = partial_metadata_path(&partial);
        let partial_metadata = storage.metadata(&partial).ok().flatten()?;
        if !partial_metadata.is_file()
            || partial_metadata.file_type().is_symlink()
            || partial_metadata.len() > file.size_bytes
            || storage
                .identity(&partial)
                .ok()
                .flatten()
                .is_none_or(|identity| identity.links != 1)
        {
            return None;
        }
        let recorded: PartialMetadata = serde_json::from_slice(
            &storage
                .read_bounded(&metadata_relative, PARTIAL_METADATA_MAX_BYTES)
                .ok()?,
        )
        .ok()?;
        (recorded == PartialMetadata::from_manifest(manifest, file))
            .then_some(partial_metadata.len())
    }

    async fn write_ownership_marker(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<(), ModelAssetError> {
        let marker = OwnershipMarker::from_manifest(&self.0.manifest);
        self.write_json_atomic(Path::new(OWNERSHIP_FILE), &marker, cancellation)
            .await
    }

    async fn write_json_atomic<T: Serialize>(
        &self,
        path: &Path,
        value: &T,
        cancellation: &CancellationToken,
    ) -> Result<(), ModelAssetError> {
        ensure_not_cancelled(cancellation)?;
        let bytes = serde_json::to_vec_pretty(value).expect("asset metadata serialization");
        let creator = self.clone();
        let output_path = path.to_path_buf();
        let mut output = blocking_phase(move || {
            creator
                .storage()
                .create_private_write(&output_path)
                .map_err(|source| creator.relative_storage_error(&output_path, source))
        })
        .await??;
        if let Err(source) = output.write_all(&bytes).await {
            output
                .discard()
                .await
                .map_err(|cleanup| self.relative_storage_error(path, cleanup))?;
            return Err(self.relative_storage_error(path, source));
        }
        if let Err(error) = ensure_not_cancelled(cancellation) {
            output
                .discard()
                .await
                .map_err(|source| self.relative_storage_error(path, source))?;
            return Err(error);
        }
        output
            .publish()
            .await
            .map_err(|source| self.relative_storage_error(path, source))
    }

    fn write_json_atomic_blocking<T: Serialize>(
        &self,
        path: &Path,
        value: &T,
        cancellation: &CancellationToken,
    ) -> Result<(), ModelAssetError> {
        ensure_not_cancelled(cancellation)?;
        let bytes = serde_json::to_vec_pretty(value).expect("asset metadata serialization");
        let mut output = self
            .storage()
            .create_private_write(path)
            .map_err(|source| self.relative_storage_error(path, source))?;
        if let Err(source) = output.write_all_blocking(&bytes) {
            output
                .discard_blocking()
                .map_err(|cleanup| self.relative_storage_error(path, cleanup))?;
            return Err(self.relative_storage_error(path, source));
        }
        if let Err(error) = ensure_not_cancelled(cancellation) {
            output
                .discard_blocking()
                .map_err(|source| self.relative_storage_error(path, source))?;
            return Err(error);
        }
        output
            .publish_blocking()
            .map_err(|source| self.relative_storage_error(path, source))
    }

    fn sha256_relative(
        &self,
        path: &Path,
        cancellation: &CancellationToken,
    ) -> Result<String, ModelAssetError> {
        self.sha256_relative_with_identity(path, cancellation)
            .map(|(hash, _)| hash)
    }

    #[cfg(unix)]
    fn sha256_relative_with_identity(
        &self,
        path: &Path,
        cancellation: &CancellationToken,
    ) -> Result<(String, FileIdentity), ModelAssetError> {
        self.sha256_relative_with_identity_and_hook(path, cancellation, || {})
    }

    #[cfg(unix)]
    fn sha256_relative_with_identity_and_hook(
        &self,
        path: &Path,
        cancellation: &CancellationToken,
        after_read: impl FnOnce() + Send,
    ) -> Result<(String, FileIdentity), ModelAssetError> {
        use std::os::unix::fs::MetadataExt;

        let mut file = self
            .storage()
            .open_read(path)
            .map_err(|source| self.relative_storage_error(path, source))?;
        let metadata = file
            .metadata()
            .map_err(|source| self.relative_storage_error(path, source))?;
        if !metadata.is_file() {
            return Err(self.relative_storage_error(
                path,
                io::Error::new(io::ErrorKind::InvalidData, "model asset is not a file"),
            ));
        }
        let identity = FileIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.size(),
            links: metadata.nlink(),
            changed_at_seconds: metadata.ctime(),
            changed_at_nanoseconds: metadata.ctime_nsec(),
        };
        if identity.links != 1 {
            return Err(ModelAssetError::ChangedDuringVerification(
                path.display().to_string(),
            ));
        }
        let mut hasher = Sha256::new();
        let mut buffer = vec![0_u8; 1024 * 1024];
        loop {
            ensure_not_cancelled(cancellation)?;
            let read = file
                .read(&mut buffer)
                .map_err(|source| self.relative_storage_error(path, source))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        ensure_not_cancelled(cancellation)?;
        after_read();
        let metadata_after = file
            .metadata()
            .map_err(|source| self.relative_storage_error(path, source))?;
        let opened_after = FileIdentity {
            device: metadata_after.dev(),
            inode: metadata_after.ino(),
            size: metadata_after.size(),
            links: metadata_after.nlink(),
            changed_at_seconds: metadata_after.ctime(),
            changed_at_nanoseconds: metadata_after.ctime_nsec(),
        };
        let visible_after = self
            .storage()
            .identity(path)
            .map_err(|source| self.relative_storage_error(path, source))?;
        if opened_after != identity || visible_after != Some(identity) {
            return Err(ModelAssetError::ChangedDuringVerification(
                path.display().to_string(),
            ));
        }
        ensure_not_cancelled(cancellation)?;
        Ok((
            hasher
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            identity,
        ))
    }

    fn reset_partial(&self, partial: &Path, metadata: &Path) -> Result<(), ModelAssetError> {
        self.remove_if_present(partial)?;
        self.remove_if_present(metadata)
    }

    fn remove_if_present(&self, path: &Path) -> Result<(), ModelAssetError> {
        if self
            .storage()
            .metadata(path)
            .map_err(|source| self.relative_storage_error(path, source))?
            .is_some()
        {
            self.storage()
                .remove_file(path)
                .map_err(|source| self.relative_storage_error(path, source))?;
        }
        Ok(())
    }

    fn remove_if_present_durable(&self, path: &Path) -> Result<(), ModelAssetError> {
        if self
            .storage()
            .metadata(path)
            .map_err(|source| self.relative_storage_error(path, source))?
            .is_some()
        {
            self.storage()
                .remove_file_durable(path)
                .map_err(|source| self.relative_storage_error(path, source))?;
        }
        Ok(())
    }

    fn clear_invalid_final(&self, install_path: &str) {
        self.0
            .invalid_finals
            .lock()
            .expect("invalid model assets lock")
            .remove(install_path);
    }

    fn removal_persistence(&self) -> Arc<dyn RemovalPersistence> {
        self.0
            .removal_persistence
            .lock()
            .expect("model asset removal persistence")
            .clone()
    }

    #[cfg(test)]
    fn set_removal_persistence(&self, persistence: Arc<dyn RemovalPersistence>) {
        *self
            .0
            .removal_persistence
            .lock()
            .expect("model asset removal persistence") = persistence;
    }

    fn storage(&self) -> &SafeStorage {
        self.0
            .storage
            .get()
            .expect("available model manager has safe storage")
    }

    fn ensure_initialized(&self) -> Result<(), ModelAssetError> {
        if self.0.storage.get().is_some() {
            return Ok(());
        }
        let status = self.status();
        Err(ModelAssetError::Unavailable(status.message.unwrap_or_else(
            || {
                if status.phase == ModelAssetPhase::Initializing {
                    "model resources are still initializing".into()
                } else {
                    "model resources could not be initialized".into()
                }
            },
        )))
    }

    fn relative_storage_error(&self, path: &Path, source: io::Error) -> ModelAssetError {
        self.storage_error(&self.0.root.join(path), source)
    }

    fn storage_error(&self, path: &Path, source: io::Error) -> ModelAssetError {
        ModelAssetError::Storage {
            path: path.to_path_buf(),
            source,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct OwnershipMarker {
    schema_version: u32,
    owner: String,
    manifest_id: String,
    repository: String,
    revision: String,
    files: Vec<OwnedFile>,
}

impl OwnershipMarker {
    fn from_manifest(manifest: &AssetManifest) -> Self {
        Self {
            schema_version: 1,
            owner: "ch.emin.piu".into(),
            manifest_id: manifest.manifest_id.clone(),
            repository: manifest.repository.clone(),
            revision: manifest.revision.clone(),
            files: manifest.files.iter().map(OwnedFile::from).collect(),
        }
    }

    fn matches(&self, manifest: &AssetManifest) -> bool {
        self.schema_version == 1
            && self.owner == "ch.emin.piu"
            && self.manifest_id == manifest.manifest_id
            && self.repository == manifest.repository
            && self.revision == manifest.revision
            && self.files.len() == manifest.files.len()
            && self
                .files
                .iter()
                .zip(&manifest.files)
                .all(|(owned, expected)| {
                    owned.install_path == expected.install_path
                        && owned.size_bytes == expected.size_bytes
                        && owned.sha256 == expected.sha256
                })
    }

    fn authorizes_removal(&self, manifest: &AssetManifest) -> bool {
        if self.schema_version != 1
            || self.owner != "ch.emin.piu"
            || self.repository != manifest.repository
            || self.revision.len() != 40
            || !self.revision.bytes().all(|byte| byte.is_ascii_hexdigit())
            || self.files.is_empty()
        {
            return false;
        }
        let mut paths = HashSet::new();
        self.files.iter().all(|file| {
            let path = Path::new(&file.install_path);
            let mut components = path.components();
            let safe_root = matches!(
                components.next(),
                Some(Component::Normal(root)) if root == "target" || root == "drafter"
            );
            safe_root
                && components.next().is_some()
                && !path.is_absolute()
                && !path.components().any(|component| {
                    matches!(
                        component,
                        Component::ParentDir
                            | Component::RootDir
                            | Component::Prefix(_)
                            | Component::CurDir
                    )
                })
                && file.size_bytes > 0
                && file.sha256.len() == 64
                && file.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
                && paths.insert(file.install_path.clone())
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct OwnedFile {
    install_path: String,
    size_bytes: u64,
    sha256: String,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct PartialMetadata {
    schema_version: u32,
    manifest_id: String,
    repository: String,
    revision: String,
    source_path: String,
    install_path: String,
    size_bytes: u64,
    sha256: String,
}

impl PartialMetadata {
    fn from_manifest(manifest: &AssetManifest, file: &ManifestFile) -> Self {
        Self {
            schema_version: 1,
            manifest_id: manifest.manifest_id.clone(),
            repository: manifest.repository.clone(),
            revision: manifest.revision.clone(),
            source_path: file.source_path.clone(),
            install_path: file.install_path.clone(),
            size_bytes: file.size_bytes,
            sha256: file.sha256.clone(),
        }
    }
}

impl From<&ManifestFile> for OwnedFile {
    fn from(file: &ManifestFile) -> Self {
        Self {
            install_path: file.install_path.clone(),
            size_bytes: file.size_bytes,
            sha256: file.sha256.clone(),
        }
    }
}

fn partial_path(destination: &Path) -> PathBuf {
    let mut path = destination.as_os_str().to_os_string();
    path.push(PART_SUFFIX);
    PathBuf::from(path)
}

fn partial_metadata_path(partial: &Path) -> PathBuf {
    let mut path = partial.as_os_str().to_os_string();
    path.push(PART_METADATA_SUFFIX);
    PathBuf::from(path)
}

fn private_write_recovery_path(partial: &Path) -> PathBuf {
    let mut path = partial.as_os_str().to_os_string();
    path.push(PRIVATE_WRITE_RECOVERY_SUFFIX);
    PathBuf::from(path)
}

fn is_private_write_name_for(name: &std::ffi::OsStr, destination: &std::ffi::OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let Some(destination) = destination.to_str() else {
        return false;
    };
    let prefix = format!(".{destination}.piu-write-");
    let Some(suffix) = name.strip_prefix(&prefix) else {
        return false;
    };
    suffix.split_once('-').is_some_and(|(process, identifier)| {
        !process.is_empty()
            && process.bytes().all(|byte| byte.is_ascii_digit())
            && !identifier.is_empty()
            && identifier.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn safe_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn is_recovery_private_write_name(name: &std::ffi::OsStr) -> bool {
    let Some(suffix) = name
        .to_str()
        .and_then(|name| name.strip_prefix(".recovery.json.piu-write-"))
    else {
        return false;
    };
    suffix.split_once('-').is_some_and(|(process, identifier)| {
        !process.is_empty()
            && process.bytes().all(|byte| byte.is_ascii_digit())
            && !identifier.is_empty()
            && identifier.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn ensure_not_cancelled(cancellation: &CancellationToken) -> Result<(), ModelAssetError> {
    if cancellation.is_cancelled() {
        Err(ModelAssetError::Cancelled)
    } else {
        Ok(())
    }
}

async fn blocking_phase<T: Send + 'static>(
    operation: impl FnOnce() -> T + Send + 'static,
) -> Result<T, ModelAssetError> {
    match tokio::task::spawn_blocking(operation).await {
        Ok(result) => Ok(result),
        Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
        Err(_) => Err(ModelAssetError::Unavailable(
            "the model resource worker stopped unexpectedly".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        convert::Infallible,
        fs,
        sync::{
            Arc, Condvar, Mutex,
            atomic::{AtomicU8, AtomicU64, Ordering},
        },
        time::Duration,
    };

    use axum::{
        Router,
        body::{Body, Bytes},
        extract::{Path as AxumPath, State},
        http::{HeaderMap, Response, StatusCode},
        response::{IntoResponse, Redirect},
        routing::get,
    };
    use futures_util::stream;
    use tempfile::TempDir;
    use tokio::net::TcpListener;

    use crate::model_asset_boundary::ModelAssetCommandError;

    use super::*;

    const NORMAL: u8 = 0;
    const AUTH_REQUIRED: u8 = 1;
    const CORRUPT: u8 = 2;
    const SLOW: u8 = 3;
    const DISCONNECT: u8 = 4;
    const OVERFLOW: u8 = 5;
    const DELAY_HEADERS: u8 = 6;

    #[derive(Default)]
    struct MemoryCredentials(Mutex<Option<String>>);

    #[derive(Clone, Copy, Eq, PartialEq)]
    enum CrashMutation {
        Stage,
        Delete,
    }

    struct CrashAfterDurableMutation {
        mutation: CrashMutation,
        fired: AtomicBool,
    }

    impl CrashAfterDurableMutation {
        fn new(mutation: CrashMutation) -> Self {
            Self {
                mutation,
                fired: AtomicBool::new(false),
            }
        }
    }

    impl RemovalPersistence for CrashAfterDurableMutation {
        fn commit_journal(
            &self,
            storage: &SafeStorage,
            path: &Path,
            phase: RemovalRecoveryPhase,
        ) -> io::Result<()> {
            DurableRemovalPersistence.commit_journal(storage, path, phase)
        }

        fn stage(&self, storage: &SafeStorage, from: &Path, to: &Path) -> io::Result<()> {
            DurableRemovalPersistence.stage(storage, from, to)?;
            if self.mutation == CrashMutation::Stage && !self.fired.swap(true, Ordering::AcqRel) {
                panic!("simulated process loss after durable stage");
            }
            Ok(())
        }

        fn delete(&self, storage: &SafeStorage, path: &Path) -> io::Result<()> {
            DurableRemovalPersistence.delete(storage, path)?;
            if self.mutation == CrashMutation::Delete && !self.fired.swap(true, Ordering::AcqRel) {
                panic!("simulated process loss after durable delete");
            }
            Ok(())
        }

        fn complete_mutations(&self, storage: &SafeStorage, journal: &Path) -> io::Result<()> {
            DurableRemovalPersistence.complete_mutations(storage, journal)
        }
    }

    struct FailingPhaseBarrier {
        phase: RemovalRecoveryPhase,
        staged: AtomicU64,
        deleted: AtomicU64,
    }

    impl FailingPhaseBarrier {
        fn new(phase: RemovalRecoveryPhase) -> Self {
            Self {
                phase,
                staged: AtomicU64::new(0),
                deleted: AtomicU64::new(0),
            }
        }
    }

    impl RemovalPersistence for FailingPhaseBarrier {
        fn commit_journal(
            &self,
            storage: &SafeStorage,
            path: &Path,
            phase: RemovalRecoveryPhase,
        ) -> io::Result<()> {
            if phase == self.phase {
                return Err(io::Error::other("injected recovery phase barrier failure"));
            }
            DurableRemovalPersistence.commit_journal(storage, path, phase)
        }

        fn stage(&self, storage: &SafeStorage, from: &Path, to: &Path) -> io::Result<()> {
            self.staged.fetch_add(1, Ordering::AcqRel);
            DurableRemovalPersistence.stage(storage, from, to)
        }

        fn delete(&self, storage: &SafeStorage, path: &Path) -> io::Result<()> {
            self.deleted.fetch_add(1, Ordering::AcqRel);
            DurableRemovalPersistence.delete(storage, path)
        }

        fn complete_mutations(&self, storage: &SafeStorage, journal: &Path) -> io::Result<()> {
            DurableRemovalPersistence.complete_mutations(storage, journal)
        }
    }

    struct FailingCompletionSync;

    impl RemovalPersistence for FailingCompletionSync {
        fn commit_journal(
            &self,
            storage: &SafeStorage,
            path: &Path,
            phase: RemovalRecoveryPhase,
        ) -> io::Result<()> {
            DurableRemovalPersistence.commit_journal(storage, path, phase)
        }

        fn stage(&self, storage: &SafeStorage, from: &Path, to: &Path) -> io::Result<()> {
            DurableRemovalPersistence.stage(storage, from, to)
        }

        fn delete(&self, storage: &SafeStorage, path: &Path) -> io::Result<()> {
            DurableRemovalPersistence.delete(storage, path)
        }

        fn complete_mutations(&self, _storage: &SafeStorage, _journal: &Path) -> io::Result<()> {
            Err(io::Error::other("injected full-sync failure"))
        }
    }

    struct BlockingDeleteCommit {
        reached: AtomicBool,
        release: (Mutex<bool>, Condvar),
    }

    impl BlockingDeleteCommit {
        fn new() -> Self {
            Self {
                reached: AtomicBool::new(false),
                release: (Mutex::new(false), Condvar::new()),
            }
        }

        fn release(&self) {
            let (released, ready) = &self.release;
            *released.lock().expect("delete commit release") = true;
            ready.notify_all();
        }
    }

    impl RemovalPersistence for BlockingDeleteCommit {
        fn commit_journal(
            &self,
            storage: &SafeStorage,
            path: &Path,
            phase: RemovalRecoveryPhase,
        ) -> io::Result<()> {
            if phase == RemovalRecoveryPhase::Deleting {
                self.reached.store(true, Ordering::Release);
                let (released, ready) = &self.release;
                let mut released = released.lock().expect("delete commit release");
                while !*released {
                    released = ready.wait(released).expect("delete commit wait");
                }
            }
            DurableRemovalPersistence.commit_journal(storage, path, phase)
        }

        fn stage(&self, storage: &SafeStorage, from: &Path, to: &Path) -> io::Result<()> {
            DurableRemovalPersistence.stage(storage, from, to)
        }

        fn delete(&self, storage: &SafeStorage, path: &Path) -> io::Result<()> {
            DurableRemovalPersistence.delete(storage, path)
        }

        fn complete_mutations(&self, storage: &SafeStorage, journal: &Path) -> io::Result<()> {
            DurableRemovalPersistence.complete_mutations(storage, journal)
        }
    }

    impl CredentialStore for MemoryCredentials {
        fn get(&self) -> Result<Option<String>, ModelAssetError> {
            Ok(self.0.lock().expect("credentials").clone())
        }

        fn set(&self, token: &str) -> Result<(), ModelAssetError> {
            *self.0.lock().expect("credentials") = Some(token.into());
            Ok(())
        }
    }

    struct FixedDisk(AtomicU64);

    impl DiskSpace for FixedDisk {
        fn available(&self, _path: &std::path::Path) -> Result<u64, ModelAssetError> {
            Ok(self.0.load(Ordering::Relaxed))
        }
    }

    #[derive(Default)]
    struct BlockingFixture {
        entered: AtomicBool,
        release: (Mutex<bool>, Condvar),
    }

    impl BlockingFixture {
        fn wait(&self) {
            self.entered.store(true, Ordering::Release);
            let (released, ready) = &self.release;
            let mut released = released.lock().expect("blocking fixture release");
            while !*released {
                released = ready.wait(released).expect("blocking fixture wait");
            }
        }

        fn release(&self) {
            let (released, ready) = &self.release;
            *released.lock().expect("blocking fixture release") = true;
            ready.notify_all();
        }
    }

    struct SlowCredentials(Arc<BlockingFixture>);

    impl CredentialStore for SlowCredentials {
        fn get(&self) -> Result<Option<String>, ModelAssetError> {
            self.0.wait();
            Ok(None)
        }

        fn set(&self, _token: &str) -> Result<(), ModelAssetError> {
            Ok(())
        }
    }

    struct SlowDisk(Arc<BlockingFixture>);

    impl DiskSpace for SlowDisk {
        fn available(&self, _path: &Path) -> Result<u64, ModelAssetError> {
            self.0.wait();
            Ok(u64::MAX)
        }
    }

    #[derive(Default)]
    struct ContendedRecoveryDisk {
        gate: BlockingFixture,
        armed: AtomicBool,
        fail_armed_call: AtomicBool,
        calls: AtomicU64,
        in_flight: AtomicU64,
        max_in_flight: AtomicU64,
    }

    impl ContendedRecoveryDisk {
        fn arm(&self) {
            self.arm_with_failure(false);
        }

        fn arm_failure(&self) {
            self.arm_with_failure(true);
        }

        fn arm_with_failure(&self, fail: bool) {
            self.calls.store(0, Ordering::Release);
            self.max_in_flight.store(0, Ordering::Release);
            self.fail_armed_call.store(fail, Ordering::Release);
            self.armed.store(true, Ordering::Release);
        }

        fn release(&self) {
            self.gate.release();
        }
    }

    impl DiskSpace for ContendedRecoveryDisk {
        fn available(&self, _path: &Path) -> Result<u64, ModelAssetError> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            let in_flight = self.in_flight.fetch_add(1, Ordering::AcqRel) + 1;
            self.max_in_flight.fetch_max(in_flight, Ordering::AcqRel);
            let armed = self.armed.swap(false, Ordering::AcqRel);
            if armed {
                self.gate.wait();
            }
            self.in_flight.fetch_sub(1, Ordering::AcqRel);
            if armed && self.fail_armed_call.swap(false, Ordering::AcqRel) {
                return Err(ModelAssetError::Unavailable(
                    "injected first recovery capacity failure".into(),
                ));
            }
            Ok(u64::MAX)
        }
    }

    struct FixtureState {
        bytes: Vec<u8>,
        mode: AtomicU8,
        requests: AtomicU64,
        ranged_requests: AtomicU64,
    }

    struct Fixture {
        base_url: String,
        state: Arc<FixtureState>,
    }

    impl Fixture {
        async fn start(bytes: Vec<u8>) -> Self {
            let state = Arc::new(FixtureState {
                bytes,
                mode: AtomicU8::new(NORMAL),
                requests: AtomicU64::new(0),
                ranged_requests: AtomicU64::new(0),
            });
            let app = Router::new()
                .route("/resolve/{revision}/{*path}", get(resolve_redirect))
                .route("/blob/{*path}", get(serve_blob))
                .route("/whoami", get(whoami))
                .with_state(state.clone());
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("fixture bind");
            let address = listener.local_addr().expect("fixture address");
            tokio::spawn(async move { axum::serve(listener, app).await.expect("fixture serve") });
            Self {
                base_url: format!("http://{address}"),
                state,
            }
        }

        fn mode(&self, mode: u8) {
            self.state.mode.store(mode, Ordering::Relaxed);
        }
    }

    async fn resolve_redirect(AxumPath((_revision, path)): AxumPath<(String, String)>) -> Redirect {
        Redirect::temporary(&format!("/blob/{path}"))
    }

    async fn whoami(
        State(state): State<Arc<FixtureState>>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        match bearer(&headers) {
            Some("valid-token") => (StatusCode::OK, "{}"),
            Some("rate-limited") => (StatusCode::TOO_MANY_REQUESTS, "try later"),
            Some("service-down") => (StatusCode::SERVICE_UNAVAILABLE, "try later"),
            _ => {
                state.mode.store(AUTH_REQUIRED, Ordering::Relaxed);
                (StatusCode::UNAUTHORIZED, "invalid token")
            }
        }
    }

    async fn serve_blob(
        State(state): State<Arc<FixtureState>>,
        headers: HeaderMap,
    ) -> Response<Body> {
        state.requests.fetch_add(1, Ordering::Relaxed);
        let mode = state.mode.load(Ordering::Relaxed);
        if mode == DELAY_HEADERS {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        if mode == AUTH_REQUIRED && bearer(&headers) != Some("valid-token") {
            return Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Body::empty())
                .expect("auth response");
        }
        let offset = headers
            .get(header::RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("bytes="))
            .and_then(|value| value.strip_suffix('-'))
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        if offset > 0 {
            state.ranged_requests.fetch_add(1, Ordering::Relaxed);
        }
        let mut bytes = state.bytes[offset..].to_vec();
        if mode == CORRUPT && !bytes.is_empty() {
            bytes[0] ^= 0xff;
        } else if mode == OVERFLOW {
            bytes.extend_from_slice(b"server-overflow");
        }
        let body = match mode {
            SLOW => {
                let chunks = bytes
                    .chunks(2)
                    .map(Bytes::copy_from_slice)
                    .collect::<Vec<_>>();
                Body::from_stream(stream::unfold(
                    chunks.into_iter(),
                    |mut chunks| async move {
                        let chunk = chunks.next()?;
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        Some((Ok::<_, Infallible>(chunk), chunks))
                    },
                ))
            }
            DISCONNECT => Body::from_stream(stream::iter([
                Ok::<_, io::Error>(Bytes::copy_from_slice(&bytes[..bytes.len().min(3)])),
                Err(io::Error::new(
                    io::ErrorKind::ConnectionReset,
                    "fixture disconnect",
                )),
            ])),
            _ => Body::from(bytes),
        };
        let mut response = Response::builder().status(if offset > 0 {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        });
        if offset > 0 {
            response = response.header(
                header::CONTENT_RANGE,
                format!(
                    "bytes {offset}-{}/{}",
                    state.bytes.len() - 1,
                    state.bytes.len()
                ),
            );
        }
        response.body(body).expect("blob response")
    }

    fn bearer(headers: &HeaderMap) -> Option<&str> {
        headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
    }

    fn fixture_manifest(bytes: &[u8]) -> AssetManifest {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let sha256 = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        AssetManifest {
            schema_version: 1,
            manifest_id: "fixture-v1".into(),
            repository: "fixture/model".into(),
            revision: "0123456789abcdef0123456789abcdef01234567".into(),
            source_last_modified: "2026-08-24T00:00:00Z".into(),
            mtp_block_size: 3,
            drafter_selection_note: "The pinned fixture uses native block size 3.".into(),
            files: vec![ManifestFile {
                asset: ModelAsset::Target,
                source_path: "4-bit/model.bin".into(),
                install_path: "target/model.bin".into(),
                size_bytes: bytes.len() as u64,
                sha256,
            }],
        }
    }

    fn test_manager(
        temporary: &TempDir,
        fixture: &Fixture,
        bytes: &[u8],
        credentials: Arc<MemoryCredentials>,
        free_bytes: u64,
    ) -> ModelAssetManager {
        test_manager_with_timeouts(
            temporary,
            fixture,
            bytes,
            credentials,
            free_bytes,
            NetworkTimeouts::default(),
        )
    }

    fn test_manager_with_timeouts(
        temporary: &TempDir,
        fixture: &Fixture,
        bytes: &[u8],
        credentials: Arc<MemoryCredentials>,
        free_bytes: u64,
        timeouts: NetworkTimeouts,
    ) -> ModelAssetManager {
        let manifest = fixture_manifest(bytes);
        ModelAssetManager::new_with_timeouts(
            temporary.path().to_path_buf(),
            PathBuf::from("models"),
            manifest.clone(),
            format!("{}/resolve/{}", fixture.base_url, manifest.revision),
            format!("{}/whoami", fixture.base_url),
            credentials,
            Arc::new(FixedDisk(AtomicU64::new(free_bytes))),
            timeouts,
        )
        .expect("test manager")
    }

    async fn seed_bound_partial(manager: &ModelAssetManager, bytes: &[u8]) {
        let destination = Path::new("target/model.bin");
        let partial = partial_path(destination);
        let metadata = partial_metadata_path(&partial);
        let expected = PartialMetadata::from_manifest(
            &manager.0.manifest,
            manager.0.manifest.files.first().expect("fixture file"),
        );
        tokio::fs::create_dir_all(manager.0.root.join("target"))
            .await
            .expect("target directory");
        tokio::fs::write(manager.0.root.join(&partial), bytes)
            .await
            .expect("partial bytes");
        tokio::fs::write(
            manager.0.root.join(metadata),
            serde_json::to_vec(&expected).expect("partial metadata JSON"),
        )
        .await
        .expect("partial metadata");
    }

    async fn wait_for_phase(
        manager: &ModelAssetManager,
        phase: ModelAssetPhase,
    ) -> ModelAssetStatus {
        let mut status = manager.subscribe();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let current = status.borrow().clone();
                if current.phase == phase {
                    return current;
                }
                status.changed().await.expect("status publisher");
            }
        })
        .await
        .expect("phase timeout")
    }

    async fn wait_for_current_file(
        manager: &ModelAssetManager,
        phase: ModelAssetPhase,
    ) -> ModelAssetStatus {
        let mut status = manager.subscribe();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let current = status.borrow().clone();
                if current.phase == phase
                    && current.current_file.as_deref() == Some("target/model.bin")
                {
                    return current;
                }
                status.changed().await.expect("status publisher");
            }
        })
        .await
        .expect("current file timeout")
    }

    async fn wait_for_private_partial_bytes(manager: &ModelAssetManager) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let has_bytes = fs::read_dir(manager.0.root.join("target"))
                    .expect("target directory")
                    .filter_map(Result::ok)
                    .any(|entry| {
                        is_private_write_name_for(
                            &entry.file_name(),
                            std::ffi::OsStr::new("model.bin.part"),
                        ) && entry.metadata().is_ok_and(|metadata| metadata.len() > 0)
                    });
                if has_bytes {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("private partial progress");
    }

    #[test]
    fn production_manifest_is_an_exact_complete_revision_pin() {
        let manifest = production_manifest().expect("valid embedded manifest");

        assert_eq!(manifest.repository, PINNED_REPOSITORY);
        assert_eq!(manifest.revision, PINNED_REVISION);
        assert_eq!(manifest.files.len(), 19);
        let config: MtpConfig =
            serde_json::from_slice(embedded_mtp_config()).expect("pinned MTP config JSON");
        let config_file = manifest
            .files
            .iter()
            .find(|file| file.source_path == "mtp/config.json")
            .expect("pinned MTP config entry");
        assert_eq!(config.block_size, 3);
        assert_eq!(manifest.mtp_block_size, config.block_size);
        assert_eq!(config_file.size_bytes, 2_976);
        assert_eq!(
            config_file.sha256,
            "1d0fae1de88b663ed0daabc8884f8a5dd076011164e4500dda4ebc947079af05"
        );
        assert_eq!(config_file.sha256, sha256_hex(embedded_mtp_config()));
        assert_eq!(manifest.total_size_bytes(), 16_950_451_879);
        assert!(
            manifest
                .files
                .iter()
                .all(|file| file.sha256.len() == 64 && file.size_bytes > 0)
        );
    }

    #[test]
    fn download_progress_is_incremental_and_thresholded_after_one_base_sample() {
        let existing_assets = 9_000_000_000;
        let resumed_file_bytes = 4_000_000;
        let file_size = PROGRESS_CHUNK_BYTES + resumed_file_bytes + 1;
        let mut progress =
            TransferProgress::new(existing_assets + resumed_file_bytes, resumed_file_bytes);

        assert_eq!(
            progress.observe(resumed_file_bytes + PROGRESS_CHUNK_BYTES - 1, file_size),
            None
        );
        assert_eq!(
            progress.observe(resumed_file_bytes + PROGRESS_CHUNK_BYTES, file_size),
            Some(existing_assets + resumed_file_bytes + PROGRESS_CHUNK_BYTES)
        );
        assert_eq!(
            progress.observe(file_size, file_size),
            Some(existing_assets + file_size)
        );
    }

    #[tokio::test]
    async fn range_resume_redirect_relaunch_and_request_coalescing_reach_ready() {
        let bytes = b"pinned model fixture bytes".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let credentials = Arc::new(MemoryCredentials::default());
        let manager = test_manager(&temporary, &fixture, &bytes, credentials.clone(), u64::MAX);
        let destination = manager.0.root.join("target/model.bin");
        seed_bound_partial(&manager, &bytes[..7]).await;

        let relaunched = test_manager(&temporary, &fixture, &bytes, credentials, u64::MAX);
        assert!(relaunched.status().can_resume);
        let first = relaunched.start_download().expect("start download");
        let second = relaunched.start_download().expect("coalesced download");
        assert_eq!(first, second);
        let ready = wait_for_phase(&relaunched, ModelAssetPhase::Ready).await;

        assert_eq!(ready.transferred_bytes, bytes.len() as u64);
        assert_eq!(
            tokio::fs::read(destination).await.expect("installed bytes"),
            bytes
        );
        assert!(
            !manager
                .0
                .root
                .join(partial_metadata_path(&partial_path(Path::new(
                    "target/model.bin"
                ))))
                .exists()
        );
        assert_eq!(fixture.state.ranged_requests.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn relaunch_after_abruptly_stopped_active_download_resumes_the_exact_partial() {
        let bytes = vec![19; 256];
        let fixture = Fixture::start(bytes.clone()).await;
        fixture.mode(SLOW);
        let temporary = TempDir::new().expect("temporary model root");
        let credentials = Arc::new(MemoryCredentials::default());
        let manager = test_manager(&temporary, &fixture, &bytes, credentials.clone(), u64::MAX);
        let (operation_id, cancellation) = manager.begin_download().expect("begin active download");
        let cancellation = cancellation.expect("new download token");
        let running_manager = manager.clone();
        let running = tauri::async_runtime::spawn(async move {
            let result = running_manager
                .download_all(operation_id, cancellation)
                .await;
            running_manager.finish_operation(operation_id, result);
        });
        wait_for_current_file(&manager, ModelAssetPhase::Downloading).await;
        let partial = partial_path(Path::new("target/model.bin"));
        wait_for_private_partial_bytes(&manager).await;
        running.abort();
        assert!(running.await.is_err());
        assert!(!manager.0.root.join(&partial).exists());
        let stopped_bytes = fs::read_dir(manager.0.root.join("target"))
            .expect("target directory")
            .filter_map(Result::ok)
            .find_map(|entry| {
                is_private_write_name_for(
                    &entry.file_name(),
                    std::ffi::OsStr::new("model.bin.part"),
                )
                .then(|| entry.metadata().expect("private partial metadata").len())
            })
            .expect("crash-left private partial");
        assert!(stopped_bytes > 0 && stopped_bytes < bytes.len() as u64);
        drop(manager);

        fixture.mode(NORMAL);
        let relaunched = test_manager(&temporary, &fixture, &bytes, credentials, u64::MAX);
        assert!(relaunched.status().can_resume);
        assert_eq!(relaunched.status().transferred_bytes, stopped_bytes);
        relaunched.start_download().expect("resume after relaunch");
        wait_for_phase(&relaunched, ModelAssetPhase::Ready).await;
        assert!(fixture.state.ranged_requests.load(Ordering::Relaxed) >= 1);
        assert_eq!(
            fs::read(relaunched.0.root.join("target/model.bin")).expect("installed bytes"),
            bytes
        );
    }

    #[tokio::test]
    async fn startup_recovers_a_private_download_inode_left_by_process_crash() {
        let bytes = b"process crash recovery fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let credentials = Arc::new(MemoryCredentials::default());
        let manager = test_manager(&temporary, &fixture, &bytes, credentials.clone(), u64::MAX);
        let file = manager.0.manifest.files.first().expect("fixture file");
        let partial = partial_path(Path::new(&file.install_path));
        let partial_metadata = partial_metadata_path(&partial);
        let expected = PartialMetadata::from_manifest(&manager.0.manifest, file);
        manager
            .write_json_atomic(&partial_metadata, &expected, &CancellationToken::new())
            .await
            .expect("partial provenance");
        let mut private = manager
            .storage()
            .create_private_write(&partial)
            .expect("private download inode");
        let recovery = private_write_recovery_path(&partial);
        manager
            .write_json_atomic(
                &recovery,
                &PrivateWriteRecovery {
                    schema_version: 1,
                    owner: "ch.emin.piu".into(),
                    temporary_path: private.temporary_path().to_path_buf(),
                    partial: expected,
                },
                &CancellationToken::new(),
            )
            .await
            .expect("private write recovery record");
        private
            .write_all(&bytes[..9])
            .await
            .expect("crash-time partial bytes");
        private
            .output
            .as_mut()
            .expect("private output is open")
            .flush()
            .await
            .expect("crash-time partial bytes reached the file");
        let private_path = private.temporary_path().to_path_buf();
        std::mem::forget(private);
        drop(manager);

        let relaunched = test_manager(&temporary, &fixture, &bytes, credentials, u64::MAX);

        assert!(relaunched.status().can_resume);
        assert_eq!(relaunched.status().transferred_bytes, 9);
        assert_eq!(
            fs::read(relaunched.0.root.join(&partial)).expect("recovered partial"),
            &bytes[..9]
        );
        assert!(!relaunched.0.root.join(private_path).exists());
        assert!(!relaunched.0.root.join(recovery).exists());
    }

    #[tokio::test]
    async fn dropping_a_private_write_never_publishes_or_removes_its_inode() {
        let temporary = TempDir::new().expect("private write root");
        let storage = SafeStorage::open(temporary.path().to_path_buf(), Path::new("models"))
            .expect("safe storage");
        let destination = Path::new("metadata.json");
        let mut private = storage
            .create_private_write(destination)
            .expect("private metadata write");
        private
            .write_all(b"not yet published")
            .await
            .expect("private metadata bytes");
        private
            .output
            .as_mut()
            .expect("private output is open")
            .flush()
            .await
            .expect("private metadata bytes reached the file");
        let private_path = private.temporary_path().to_path_buf();

        drop(private);

        assert!(!storage.absolute(destination).exists());
        assert_eq!(
            fs::read(storage.absolute(&private_path)).expect("retained private inode"),
            b"not yet published"
        );
        storage
            .remove_file_durable(&private_path)
            .expect("test cleanup");
    }

    #[tokio::test]
    async fn startup_removes_crash_left_marker_and_partial_metadata_publications() {
        let bytes = b"private metadata recovery fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let credentials = Arc::new(MemoryCredentials::default());
        let manager = test_manager(&temporary, &fixture, &bytes, credentials.clone(), u64::MAX);
        let file = manager.0.manifest.files.first().expect("fixture file");
        let marker = OwnershipMarker::from_manifest(&manager.0.manifest);
        let partial = partial_path(Path::new(&file.install_path));
        let partial_metadata = partial_metadata_path(&partial);
        let metadata = PartialMetadata::from_manifest(&manager.0.manifest, file);

        let mut private_marker = manager
            .storage()
            .create_private_write(Path::new(OWNERSHIP_FILE))
            .expect("private marker");
        private_marker
            .write_all(&serde_json::to_vec(&marker).expect("marker JSON"))
            .await
            .expect("private marker bytes");
        let private_marker_path = private_marker.temporary_path().to_path_buf();
        drop(private_marker);
        let mut private_metadata = manager
            .storage()
            .create_private_write(&partial_metadata)
            .expect("private partial metadata");
        private_metadata
            .write_all(&serde_json::to_vec(&metadata).expect("metadata JSON"))
            .await
            .expect("private metadata bytes");
        let private_metadata_path = private_metadata.temporary_path().to_path_buf();
        drop(private_metadata);
        drop(manager);

        let relaunched = test_manager(&temporary, &fixture, &bytes, credentials, u64::MAX);

        assert_eq!(relaunched.status().phase, ModelAssetPhase::Missing);
        assert!(!relaunched.0.root.join(private_marker_path).exists());
        assert!(!relaunched.0.root.join(private_metadata_path).exists());
        assert!(!relaunched.0.root.join(OWNERSHIP_FILE).exists());
        assert!(!relaunched.0.root.join(partial_metadata).exists());
    }

    #[tokio::test]
    async fn startup_stops_scanning_at_the_bounded_namespace_limit() {
        let bytes = b"bounded namespace fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let root = temporary.path().join("models");
        fs::create_dir_all(&root).expect("model root");
        for index in 0..=NAMESPACE_SCAN_MAX_ENTRIES {
            fs::write(root.join(format!("unknown-{index}")), b"untouched")
                .expect("adversarial namespace entry");
        }
        let manifest = fixture_manifest(&bytes);

        let result = ModelAssetManager::new(
            temporary.path().to_path_buf(),
            PathBuf::from("models"),
            manifest,
            format!("{}/resolve/revision", fixture.base_url),
            format!("{}/whoami", fixture.base_url),
            Arc::new(MemoryCredentials::default()),
            Arc::new(FixedDisk(AtomicU64::new(u64::MAX))),
        );

        assert!(matches!(result, Err(ModelAssetError::Storage { .. })));
        assert_eq!(
            fs::read(root.join(format!("unknown-{NAMESPACE_SCAN_MAX_ENTRIES}")))
                .expect("last adversarial entry"),
            b"untouched"
        );
    }

    #[tokio::test]
    async fn current_asset_and_file_publish_when_transfer_and_recovered_hash_begin() {
        let bytes = vec![42; 128];
        let fixture = Fixture::start(bytes.clone()).await;
        fixture.mode(SLOW);
        let temporary = TempDir::new().expect("transfer root");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        manager.start_download().expect("start slow transfer");
        let transfer = wait_for_current_file(&manager, ModelAssetPhase::Downloading).await;
        assert_eq!(transfer.current_asset, Some(ModelAsset::Target));
        assert_eq!(transfer.transferred_bytes, 0);
        assert!(fixture.state.requests.load(Ordering::Relaxed) <= 1);
        assert!(manager.cancel_download());
        wait_for_phase(&manager, ModelAssetPhase::Cancelled).await;

        let bytes = vec![7; 32 * 1024 * 1024];
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("recovered final root");
        let root = temporary.path().join("models/target");
        fs::create_dir_all(&root).expect("target directory");
        fs::write(root.join("model.bin"), &bytes).expect("recovered final");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        manager.start_download().expect("validate recovered final");
        let verifying = wait_for_current_file(&manager, ModelAssetPhase::Verifying).await;
        assert_eq!(verifying.current_asset, Some(ModelAsset::Target));
        assert_eq!(verifying.transferred_bytes, bytes.len() as u64);
        wait_for_phase(&manager, ModelAssetPhase::Ready).await;
        assert_eq!(fixture.state.requests.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn oversized_partial_is_reset_instead_of_entering_a_resume_loop() {
        let bytes = b"pinned-size".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        let destination = manager.0.root.join("target/model.bin");
        seed_bound_partial(&manager, b"unsafe-oversized-partial").await;

        let relaunched = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            DISK_SAFETY_RESERVE_BYTES,
        );
        assert!(!relaunched.status().can_resume);

        relaunched
            .start_download()
            .expect("start guarded oversized recovery");
        let failed = wait_for_phase(&relaunched, ModelAssetPhase::Failed).await;

        assert_eq!(
            failed.error_code,
            Some(ModelAssetErrorCode::InsufficientSpace)
        );
        assert_eq!(fixture.state.requests.load(Ordering::Relaxed), 0);
        assert_eq!(fixture.state.ranged_requests.load(Ordering::Relaxed), 0);
        assert!(!destination.exists());
    }

    #[tokio::test]
    async fn unbound_or_old_revision_partial_is_never_used_for_range_resume() {
        let bytes = b"new immutable revision bytes".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;

        for metadata_case in ["missing", "old-revision"] {
            let temporary = TempDir::new().expect("temporary model root");
            let manager = test_manager(
                &temporary,
                &fixture,
                &bytes,
                Arc::new(MemoryCredentials::default()),
                u64::MAX,
            );
            seed_bound_partial(&manager, &bytes[..8]).await;
            let partial = partial_path(Path::new("target/model.bin"));
            let metadata_path = partial_metadata_path(&partial);
            if metadata_case == "missing" {
                tokio::fs::remove_file(manager.0.root.join(&metadata_path))
                    .await
                    .expect("remove companion metadata");
            } else {
                let mut old = PartialMetadata::from_manifest(
                    &manager.0.manifest,
                    manager.0.manifest.files.first().expect("fixture file"),
                );
                old.revision = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into();
                tokio::fs::write(
                    manager.0.root.join(&metadata_path),
                    serde_json::to_vec(&old).expect("old partial metadata JSON"),
                )
                .await
                .expect("old partial metadata");
            }

            let relaunched = test_manager(
                &temporary,
                &fixture,
                &bytes,
                Arc::new(MemoryCredentials::default()),
                u64::MAX,
            );
            assert!(!relaunched.status().can_resume);
            relaunched.start_download().expect("restart clean download");
            wait_for_phase(&relaunched, ModelAssetPhase::Ready).await;
            assert!(!manager.0.root.join(&metadata_path).exists());
        }

        assert_eq!(fixture.state.ranged_requests.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn oversized_partial_metadata_is_bounded_and_never_resumed() {
        let bytes = b"bounded sidecar fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        let partial = partial_path(Path::new("target/model.bin"));
        let metadata = partial_metadata_path(&partial);
        tokio::fs::create_dir_all(manager.0.root.join("target"))
            .await
            .expect("target directory");
        tokio::fs::write(manager.0.root.join(&partial), &bytes[..7])
            .await
            .expect("partial bytes");
        tokio::fs::write(
            manager.0.root.join(&metadata),
            vec![b'x'; PARTIAL_METADATA_MAX_BYTES as usize + 1],
        )
        .await
        .expect("oversized sidecar");

        let relaunched = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        assert!(!relaunched.status().can_resume);
        relaunched.start_download().expect("restart clean download");
        wait_for_phase(&relaunched, ModelAssetPhase::Ready).await;

        assert_eq!(fixture.state.ranged_requests.load(Ordering::Relaxed), 0);
        assert!(!manager.0.root.join(metadata).exists());
    }

    #[tokio::test]
    async fn graphical_authentication_keeps_token_in_store_and_resumes_after_expiry() {
        let bytes = b"credential fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        fixture.mode(AUTH_REQUIRED);
        let temporary = TempDir::new().expect("temporary model root");
        let credentials = Arc::new(MemoryCredentials(Mutex::new(Some("expired-token".into()))));
        let manager = test_manager(&temporary, &fixture, &bytes, credentials.clone(), u64::MAX);

        manager
            .start_download()
            .expect("start unauthenticated download");
        wait_for_phase(&manager, ModelAssetPhase::AuthenticationRequired).await;
        assert_eq!(
            credentials
                .get()
                .expect("expired credential read")
                .as_deref(),
            Some("expired-token")
        );
        manager
            .authorize_hugging_face("valid-token".into())
            .await
            .expect("authorize token");
        manager
            .start_download()
            .expect("resume authenticated download");
        wait_for_phase(&manager, ModelAssetPhase::Ready).await;

        assert_eq!(
            credentials.get().expect("credential read").as_deref(),
            Some("valid-token")
        );
        assert!(manager.status().authentication_configured);
        assert_eq!(manager.status().error_code, None);
    }

    #[tokio::test]
    async fn authorization_distinguishes_credentials_from_service_failures() {
        let bytes = b"authorization fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );

        assert!(matches!(
            manager.authorize_hugging_face("bad-token".into()).await,
            Err(ModelAssetError::Authentication)
        ));
        assert!(matches!(
            manager.authorize_hugging_face("rate-limited".into()).await,
            Err(ModelAssetError::Network(_))
        ));
        assert!(matches!(
            manager.authorize_hugging_face("service-down".into()).await,
            Err(ModelAssetError::Network(_))
        ));

        let mut stale = manager.status();
        stale.phase = ModelAssetPhase::AuthenticationRequired;
        stale.error_code = Some(ModelAssetErrorCode::Authentication);
        manager.publish(stale);
        manager
            .authorize_hugging_face("valid-token".into())
            .await
            .expect("valid credentials");
        assert_eq!(manager.status().error_code, None);
    }

    #[tokio::test]
    async fn signed_download_urls_are_reduced_to_safe_errors_at_capture_time() {
        const SENTINEL: &str = "SIGNED_QUERY_SENTINEL_DO_NOT_LOG";
        let bytes = b"safe network error fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        let signed_url = format!(
            "{}/whoami?X-Amz-Credential={SENTINEL}&X-Amz-Signature={SENTINEL}",
            fixture.base_url
        );
        let source = Client::new()
            .get(&signed_url)
            .bearer_auth("service-down")
            .send()
            .await
            .expect("fixture response")
            .error_for_status()
            .expect_err("fixture status error");
        let error = ModelAssetError::from(source);
        let display = error.to_string();
        let debug = format!("{error:?}");
        manager.publish_operation_failure(99, &error);
        let status = manager.status();
        let ipc = ModelAssetCommandError::from(error);
        let exposed = [
            display,
            debug,
            status.message.clone().unwrap_or_default(),
            format!("{status:?}"),
            ipc.message.clone(),
            format!("{ipc:?}"),
        ];

        for text in exposed {
            assert!(!text.contains(SENTINEL), "secret escaped in {text}");
            assert!(!text.contains(&fixture.base_url), "host escaped in {text}");
            assert!(!text.contains("X-Amz"), "query name escaped in {text}");
            assert!(!text.contains("http://"), "URL escaped in {text}");
        }
        assert_eq!(ipc.code, ModelAssetErrorCode::Network);
        assert_eq!(
            ipc.message,
            "could not reach Hugging Face: Hugging Face returned HTTP 503 Service Unavailable"
        );
    }

    #[tokio::test]
    async fn cancellation_and_disconnect_preserve_safe_partial_for_resume() {
        let bytes = vec![42; 128];
        let fixture = Fixture::start(bytes.clone()).await;
        fixture.mode(SLOW);
        let temporary = TempDir::new().expect("temporary model root");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );

        manager.start_download().expect("start slow download");
        wait_for_current_file(&manager, ModelAssetPhase::Downloading).await;
        wait_for_private_partial_bytes(&manager).await;
        assert!(manager.cancel_download());
        let cancelled = wait_for_phase(&manager, ModelAssetPhase::Cancelled).await;
        assert!(cancelled.can_resume);
        let partial = partial_path(Path::new("target/model.bin"));
        assert!(
            manager
                .0
                .root
                .join(partial_metadata_path(&partial))
                .exists()
        );

        fixture.mode(DISCONNECT);
        manager.start_download().expect("resume into disconnect");
        let failed = wait_for_phase(&manager, ModelAssetPhase::Failed).await;
        assert!(failed.can_resume);

        fixture.mode(NORMAL);
        manager.start_download().expect("resume after disconnect");
        wait_for_phase(&manager, ModelAssetPhase::Ready).await;
        assert!(fixture.state.ranged_requests.load(Ordering::Relaxed) >= 2);
        assert!(
            !manager
                .0
                .root
                .join(partial_metadata_path(&partial))
                .exists()
        );
    }

    #[tokio::test]
    async fn response_body_overflow_is_rejected_before_extra_bytes_are_written() {
        let bytes = b"bounded response fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        fixture.mode(OVERFLOW);
        let temporary = TempDir::new().expect("temporary model root");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );

        manager.start_download().expect("start bounded download");
        let failed = wait_for_phase(&manager, ModelAssetPhase::Failed).await;
        let partial = partial_path(Path::new("target/model.bin"));
        let written = fs::metadata(manager.0.root.join(partial))
            .map(|metadata| metadata.len())
            .unwrap_or(0);

        assert_eq!(failed.error_code, Some(ModelAssetErrorCode::Network));
        assert!(written <= bytes.len() as u64);
        assert_ne!(failed.phase, ModelAssetPhase::Ready);
    }

    #[tokio::test]
    async fn cancellation_interrupts_the_response_header_wait() {
        let bytes = b"cancel header fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        fixture.mode(DELAY_HEADERS);
        let temporary = TempDir::new().expect("temporary model root");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );

        manager.start_download().expect("start delayed request");
        wait_for_phase(&manager, ModelAssetPhase::Downloading).await;
        let cancelled_at = std::time::Instant::now();
        assert!(manager.cancel_download());
        wait_for_phase(&manager, ModelAssetPhase::Cancelled).await;

        assert!(cancelled_at.elapsed() < Duration::from_millis(500));
    }

    #[tokio::test]
    async fn response_header_and_body_read_waits_have_explicit_timeouts() {
        let bytes = b"timeout fixture bytes".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let timeouts = NetworkTimeouts {
            connect: Duration::from_secs(1),
            headers: Duration::from_millis(25),
            read: Duration::from_millis(5),
        };

        fixture.mode(DELAY_HEADERS);
        let temporary = TempDir::new().expect("header timeout root");
        let manager = test_manager_with_timeouts(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
            timeouts,
        );
        manager.start_download().expect("start header timeout");
        assert_eq!(
            wait_for_phase(&manager, ModelAssetPhase::Failed)
                .await
                .error_code,
            Some(ModelAssetErrorCode::Network)
        );

        fixture.mode(SLOW);
        let temporary = TempDir::new().expect("read timeout root");
        let manager = test_manager_with_timeouts(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
            timeouts,
        );
        manager.start_download().expect("start read timeout");
        assert_eq!(
            wait_for_phase(&manager, ModelAssetPhase::Failed)
                .await
                .error_code,
            Some(ModelAssetErrorCode::Network)
        );
    }

    #[tokio::test]
    async fn checksum_mismatch_and_insufficient_space_are_actionable() {
        let bytes = b"integrity fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            0,
        );
        manager.start_download().expect("start disk check");
        let disk_failure = wait_for_phase(&manager, ModelAssetPhase::Failed).await;
        assert_eq!(disk_failure.current_free_bytes, 0);
        assert_eq!(
            disk_failure.required_free_bytes,
            bytes.len() as u64 + DISK_SAFETY_RESERVE_BYTES
        );
        assert!(
            disk_failure
                .message
                .expect("disk message")
                .contains("not enough disk space")
        );

        let temporary = TempDir::new().expect("second temporary model root");
        fixture.mode(CORRUPT);
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        manager.start_download().expect("start corrupt download");
        let integrity_failure = wait_for_phase(&manager, ModelAssetPhase::Failed).await;
        assert!(
            integrity_failure
                .message
                .expect("integrity message")
                .contains("SHA-256")
        );
        assert!(!partial_path(&manager.0.root.join("target/model.bin")).exists());
        assert!(
            !manager
                .0
                .root
                .join(partial_metadata_path(&partial_path(Path::new(
                    "target/model.bin"
                ))))
                .exists()
        );
    }

    #[tokio::test]
    async fn removal_deletes_only_exact_owned_files_and_preserves_unknown_files() {
        let bytes = b"ownership fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        manager.start_download().expect("start owned download");
        wait_for_phase(&manager, ModelAssetPhase::Ready).await;
        let unknown = manager.0.root.join("keep-me.txt");
        tokio::fs::write(&unknown, b"not owned")
            .await
            .expect("unknown file");

        let missing = manager
            .remove_owned_assets()
            .await
            .expect("remove owned assets");

        assert_eq!(missing.phase, ModelAssetPhase::Missing);
        assert!(unknown.exists());
        assert!(!manager.0.root.join("target/model.bin").exists());
    }

    #[tokio::test]
    async fn old_piu_revision_can_remove_only_its_verified_listed_files() {
        let bytes = b"old owned fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        let owned = manager.0.root.join("target/model.bin");
        tokio::fs::create_dir_all(owned.parent().expect("target directory"))
            .await
            .expect("target directory");
        tokio::fs::write(&owned, &bytes)
            .await
            .expect("old owned file");
        let unknown = manager.0.root.join("target/notes.txt");
        tokio::fs::write(&unknown, b"user file")
            .await
            .expect("unknown file");
        let mut marker = OwnershipMarker::from_manifest(&manager.0.manifest);
        marker.manifest_id = "old-piu-manifest".into();
        marker.revision = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into();
        tokio::fs::write(
            manager.0.root.join(OWNERSHIP_FILE),
            serde_json::to_vec(&marker).expect("old marker JSON"),
        )
        .await
        .expect("old marker");
        let relaunched = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        assert_eq!(relaunched.status().phase, ModelAssetPhase::RevisionMismatch);

        let missing = relaunched
            .remove_owned_assets()
            .await
            .expect("remove verified old revision");

        assert_eq!(missing.phase, ModelAssetPhase::Missing);
        assert!(!owned.exists());
        assert!(unknown.exists());
    }

    #[tokio::test]
    async fn replacement_between_verification_and_removal_is_never_deleted() {
        let bytes = b"owned race fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        manager.start_download().expect("install owned file");
        wait_for_phase(&manager, ModelAssetPhase::Ready).await;
        let root = manager.0.root.clone();
        let replacement = b"unknown replacement";
        let replacement_root = root.clone();

        let missing = manager
            .remove_owned_assets_with_hook(move |relative| {
                fs::write(replacement_root.join(relative), replacement).expect("replacement file");
            })
            .await
            .expect("remove staged verified file");

        assert_eq!(missing.phase, ModelAssetPhase::Missing);
        assert_eq!(
            fs::read(root.join("target/model.bin")).expect("preserved replacement"),
            replacement
        );
        assert!(!root.join(OWNERSHIP_FILE).exists());
        assert!(fs::read_dir(&root).expect("model root").all(|entry| {
            !entry
                .expect("model root entry")
                .file_name()
                .to_string_lossy()
                .starts_with(".piu-removal-")
        }));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn removal_is_one_cancellable_serialized_operation_with_coherent_status() {
        let bytes = vec![13; 64 * 1024 * 1024];
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        manager.start_download().expect("install owned file");
        wait_for_phase(&manager, ModelAssetPhase::Ready).await;

        let first_manager = manager.clone();
        let first = tokio::spawn(async move { first_manager.remove_owned_assets().await });
        let removing = wait_for_phase(&manager, ModelAssetPhase::Removing).await;
        assert!(removing.operation_id.is_some());
        assert!(matches!(
            manager.remove_owned_assets().await,
            Err(ModelAssetError::OperationInProgress)
        ));
        assert!(manager.cancel_download());
        assert!(matches!(
            first.await.expect("first removal task"),
            Err(ModelAssetError::Cancelled)
        ));
        assert_eq!(
            wait_for_phase(&manager, ModelAssetPhase::Cancelled)
                .await
                .error_code,
            Some(ModelAssetErrorCode::Cancellation)
        );
        assert!(manager.0.root.join("target/model.bin").exists());
        assert!(manager.0.root.join(OWNERSHIP_FILE).exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancellation_accepted_before_removal_commit_prevents_every_delete() {
        let bytes = b"pre-commit cancellation fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        manager.start_download().expect("install owned file");
        wait_for_phase(&manager, ModelAssetPhase::Ready).await;
        let cancellation_manager = manager.clone();

        let result = manager
            .remove_owned_assets_with_hook(move |_| {
                assert!(cancellation_manager.cancel_download());
            })
            .await;

        assert!(matches!(result, Err(ModelAssetError::Cancelled)));
        assert!(manager.0.root.join("target/model.bin").exists());
        assert!(manager.0.root.join(OWNERSHIP_FILE).exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancellation_after_removal_commit_is_rejected_and_deletion_finishes() {
        let bytes = b"post-commit cancellation fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        manager.start_download().expect("install owned file");
        wait_for_phase(&manager, ModelAssetPhase::Ready).await;
        let commit = Arc::new(BlockingDeleteCommit::new());
        manager.set_removal_persistence(commit.clone());
        let removal_manager = manager.clone();
        let removal = tokio::spawn(async move { removal_manager.remove_owned_assets().await });

        while !commit.reached.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
        assert!(!manager.cancel_download());
        assert!(
            !manager
                .status()
                .available_actions
                .contains(&ModelAssetAction::Cancel)
        );
        commit.release();

        assert_eq!(
            removal
                .await
                .expect("removal task")
                .expect("committed removal")
                .phase,
            ModelAssetPhase::Missing
        );
        assert!(!manager.0.root.join("target/model.bin").exists());
        assert!(!manager.0.root.join(OWNERSHIP_FILE).exists());
    }

    #[tokio::test]
    async fn failed_cancellation_rollback_stays_recovery_required_until_reconciled() {
        let bytes = b"rollback failure fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        manager.start_download().expect("install owned file");
        wait_for_phase(&manager, ModelAssetPhase::Ready).await;
        let root = manager.0.root.clone();
        let cancellation_manager = manager.clone();

        let result = manager
            .remove_owned_assets_with_hook(move |relative| {
                fs::write(root.join(relative), b"rollback obstruction")
                    .expect("create rollback obstruction");
                assert!(cancellation_manager.cancel_download());
            })
            .await;

        assert!(matches!(result, Err(ModelAssetError::RecoveryRequired(_))));
        let failed = manager.status();
        assert_eq!(failed.phase, ModelAssetPhase::Failed);
        assert_eq!(failed.error_code, Some(ModelAssetErrorCode::Storage));
        assert!(failed.operation_id.is_some());
        assert_eq!(
            failed.available_actions,
            vec![ModelAssetAction::RetryRecovery]
        );
        assert!(manager.0.recovery_required.load(Ordering::Acquire));
        assert!(
            manager
                .0
                .active
                .lock()
                .expect("active recovery operation")
                .is_some()
        );
        assert!(matches!(
            manager.retry_recovery().await,
            Err(ModelAssetError::RecoveryRequired(_))
        ));
        assert_eq!(
            manager.status().available_actions,
            vec![ModelAssetAction::RetryRecovery]
        );
        assert!(manager.0.recovery_required.load(Ordering::Acquire));

        fs::remove_file(manager.0.root.join("target/model.bin"))
            .expect("remove rollback obstruction");
        manager
            .retry_recovery()
            .await
            .expect("reconcile durable staging journal");
        wait_for_phase(&manager, ModelAssetPhase::Ready).await;
        assert!(!manager.0.recovery_required.load(Ordering::Acquire));
        assert_eq!(
            manager.status().available_actions,
            vec![ModelAssetAction::Remove]
        );
        assert_eq!(
            fs::read(manager.0.root.join("target/model.bin")).expect("restored owned asset"),
            bytes
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn relaunch_recovers_interrupted_removal_before_and_after_commit() {
        use serde_json::json;

        let bytes = b"removal restart fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;

        let temporary = TempDir::new().expect("rollback root");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        manager.start_download().expect("install rollback fixture");
        wait_for_phase(&manager, ModelAssetPhase::Ready).await;
        let marker_bytes = fs::read(manager.0.root.join(OWNERSHIP_FILE)).expect("marker bytes");
        let staging_name = ".piu-removal-restart-rollback";
        let staging = PathBuf::from(staging_name);
        manager
            .storage()
            .create_dir(&staging)
            .expect("staging directory");
        let staged_asset = staging.join("asset-0");
        let staged_marker = staging.join("ownership-marker");
        let plan = json!({
            "schemaVersion": 1,
            "owner": "ch.emin.piu",
            "phase": "staging",
            "stagingDirectory": staging_name,
            "entries": [
                {
                    "original": "target/model.bin",
                    "staged": staged_asset.to_string_lossy(),
                    "sizeBytes": bytes.len(),
                    "sha256": sha256_hex(&bytes),
                    "identity": null
                },
                {
                    "original": OWNERSHIP_FILE,
                    "staged": staged_marker.to_string_lossy(),
                    "sizeBytes": marker_bytes.len(),
                    "sha256": sha256_hex(&marker_bytes),
                    "identity": null
                }
            ]
        });
        manager
            .write_json_atomic(
                &staging.join("recovery.json"),
                &plan,
                &CancellationToken::new(),
            )
            .await
            .expect("recovery plan");
        manager
            .storage()
            .rename(Path::new("target/model.bin"), &staged_asset)
            .expect("stage asset before crash");
        drop(manager);

        let relaunched = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        wait_for_phase(&relaunched, ModelAssetPhase::Ready).await;
        assert_eq!(
            fs::read(relaunched.0.root.join("target/model.bin")).expect("restored asset"),
            bytes
        );
        assert!(!relaunched.0.root.join(staging_name).exists());

        let temporary = TempDir::new().expect("commit root");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        manager.start_download().expect("install commit fixture");
        wait_for_phase(&manager, ModelAssetPhase::Ready).await;
        let marker_bytes = fs::read(manager.0.root.join(OWNERSHIP_FILE)).expect("marker bytes");
        let staging_name = ".piu-removal-restart-commit";
        let staging = PathBuf::from(staging_name);
        manager
            .storage()
            .create_dir(&staging)
            .expect("staging directory");
        let staged_asset = staging.join("asset-0");
        let staged_marker = staging.join("ownership-marker");
        manager
            .storage()
            .rename(Path::new("target/model.bin"), &staged_asset)
            .expect("stage asset");
        manager
            .storage()
            .rename(Path::new(OWNERSHIP_FILE), &staged_marker)
            .expect("stage marker");
        let asset_identity = manager
            .storage()
            .identity(&staged_asset)
            .expect("asset identity")
            .expect("staged asset");
        let marker_identity = manager
            .storage()
            .identity(&staged_marker)
            .expect("marker identity")
            .expect("staged marker");
        let identity_json = |identity: FileIdentity| {
            json!({
                "device": identity.device,
                "inode": identity.inode,
                "size": identity.size,
                "links": identity.links,
                "changedAtSeconds": identity.changed_at_seconds,
                "changedAtNanoseconds": identity.changed_at_nanoseconds
            })
        };
        let plan = json!({
            "schemaVersion": 1,
            "owner": "ch.emin.piu",
            "phase": "deleting",
            "stagingDirectory": staging_name,
            "entries": [
                {
                    "original": "target/model.bin",
                    "staged": staged_asset.to_string_lossy(),
                    "sizeBytes": bytes.len(),
                    "sha256": sha256_hex(&bytes),
                    "identity": identity_json(asset_identity)
                },
                {
                    "original": OWNERSHIP_FILE,
                    "staged": staged_marker.to_string_lossy(),
                    "sizeBytes": marker_bytes.len(),
                    "sha256": sha256_hex(&marker_bytes),
                    "identity": identity_json(marker_identity)
                }
            ]
        });
        manager
            .write_json_atomic(
                &staging.join("recovery.json"),
                &plan,
                &CancellationToken::new(),
            )
            .await
            .expect("committed recovery plan");
        manager
            .storage()
            .remove_file(&staged_asset)
            .expect("simulate one completed delete");
        drop(manager);

        let relaunched = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        assert_eq!(relaunched.status().phase, ModelAssetPhase::Missing);
        assert!(!relaunched.0.root.join(staging_name).exists());
        assert!(!relaunched.0.root.join(OWNERSHIP_FILE).exists());
    }

    #[tokio::test]
    async fn relaunch_cleans_a_crash_left_private_removal_journal_before_asset_mutation() {
        let bytes = b"private removal journal fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        let staging = Path::new(".piu-removal-crashed-journal");
        manager
            .storage()
            .create_dir(staging)
            .expect("removal staging");
        let mut journal = manager
            .storage()
            .create_private_write(&staging.join(REMOVAL_RECOVERY_FILE))
            .expect("private recovery journal");
        journal
            .write_all(b"{\"schemaVersion\":")
            .await
            .expect("incomplete recovery journal");
        std::mem::forget(journal);
        drop(manager);

        let relaunched = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );

        assert_eq!(relaunched.status().phase, ModelAssetPhase::Missing);
        assert!(!relaunched.0.root.join(staging).exists());
    }

    #[tokio::test]
    async fn relaunch_fails_closed_on_unjournaled_staging_and_preserves_every_byte() {
        let bytes = b"fail closed fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let staging = temporary
            .path()
            .join("models/.piu-removal-unknown-operation");
        fs::create_dir_all(&staging).expect("unknown staging directory");
        let unknown = staging.join("unknown.bin");
        fs::write(&unknown, b"must remain untouched").expect("unknown staged bytes");
        let manifest = fixture_manifest(&bytes);

        let result = ModelAssetManager::new(
            temporary.path().to_path_buf(),
            PathBuf::from("models"),
            manifest.clone(),
            format!("{}/resolve/{}", fixture.base_url, manifest.revision),
            format!("{}/whoami", fixture.base_url),
            Arc::new(MemoryCredentials::default()),
            Arc::new(FixedDisk(AtomicU64::new(u64::MAX))),
        );

        assert!(matches!(result, Err(ModelAssetError::NotOwned)));
        assert_eq!(
            fs::read(&unknown).expect("preserved bytes"),
            b"must remain untouched"
        );
    }

    #[tokio::test]
    async fn adversarial_old_marker_paths_and_tampered_files_are_never_removed() {
        let bytes = b"safe fixture bytes".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        let outside = temporary.path().join("outside.txt");
        tokio::fs::write(&outside, b"safe fixture bytes")
            .await
            .expect("outside file");
        let mut marker = OwnershipMarker::from_manifest(&manager.0.manifest);
        marker.revision = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into();
        marker.files[0].install_path = "../outside.txt".into();
        tokio::fs::write(
            manager.0.root.join(OWNERSHIP_FILE),
            serde_json::to_vec(&marker).expect("adversarial marker JSON"),
        )
        .await
        .expect("adversarial marker");

        assert!(matches!(
            manager.remove_owned_assets().await,
            Err(ModelAssetError::NotOwned)
        ));
        assert!(outside.exists());

        let owned = manager.0.root.join("target/model.bin");
        tokio::fs::create_dir_all(owned.parent().expect("target directory"))
            .await
            .expect("target directory");
        tokio::fs::write(&owned, b"tampered fixture")
            .await
            .expect("tampered file");
        marker.files[0].install_path = "target/model.bin".into();
        tokio::fs::write(
            manager.0.root.join(OWNERSHIP_FILE),
            serde_json::to_vec(&marker).expect("old marker JSON"),
        )
        .await
        .expect("old marker");

        assert!(matches!(
            manager.remove_owned_assets().await,
            Err(ModelAssetError::NotOwned)
        ));
        assert!(owned.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn old_marker_cannot_follow_a_symlinked_owned_directory() {
        use std::os::unix::fs::symlink;

        let bytes = b"symlink fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        let outside = temporary.path().join("outside");
        tokio::fs::create_dir(&outside)
            .await
            .expect("outside directory");
        let outside_file = outside.join("model.bin");
        tokio::fs::write(&outside_file, &bytes)
            .await
            .expect("outside file");
        symlink(&outside, manager.0.root.join("target")).expect("target symlink");
        let mut marker = OwnershipMarker::from_manifest(&manager.0.manifest);
        marker.revision = "cccccccccccccccccccccccccccccccccccccccc".into();
        tokio::fs::write(
            manager.0.root.join(OWNERSHIP_FILE),
            serde_json::to_vec(&marker).expect("old marker JSON"),
        )
        .await
        .expect("old marker");

        assert!(matches!(
            manager.remove_owned_assets().await,
            Err(ModelAssetError::NotOwned)
        ));
        assert!(outside_file.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn download_never_follows_symlinked_asset_ancestor() {
        use std::os::unix::fs::symlink;

        let bytes = b"download symlink fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        let outside = temporary.path().join("outside");
        tokio::fs::create_dir(&outside)
            .await
            .expect("outside directory");
        let outside_file = outside.join("model.bin.part");
        tokio::fs::write(&outside_file, b"outside remains unchanged")
            .await
            .expect("outside sentinel");
        symlink(&outside, manager.0.root.join("target")).expect("target symlink");

        manager.start_download().expect("start guarded download");
        let failed = wait_for_phase(&manager, ModelAssetPhase::Failed).await;

        assert_eq!(failed.error_code, Some(ModelAssetErrorCode::Storage));
        assert_eq!(
            tokio::fs::read(&outside_file)
                .await
                .expect("outside sentinel"),
            b"outside remains unchanged"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn download_replaces_part_symlink_without_touching_its_target() {
        use std::os::unix::fs::symlink;

        let bytes = b"partial symlink fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        tokio::fs::create_dir(manager.0.root.join("target"))
            .await
            .expect("target directory");
        let outside = temporary.path().join("outside.txt");
        tokio::fs::write(&outside, b"outside remains unchanged")
            .await
            .expect("outside sentinel");
        let partial = partial_path(Path::new("target/model.bin"));
        symlink(&outside, manager.0.root.join(&partial)).expect("partial symlink");
        let metadata = PartialMetadata::from_manifest(
            &manager.0.manifest,
            manager.0.manifest.files.first().expect("fixture file"),
        );
        tokio::fs::write(
            manager.0.root.join(partial_metadata_path(&partial)),
            serde_json::to_vec(&metadata).expect("partial metadata JSON"),
        )
        .await
        .expect("partial metadata");

        manager.start_download().expect("start guarded download");
        wait_for_phase(&manager, ModelAssetPhase::Ready).await;

        assert_eq!(
            tokio::fs::read(&outside).await.expect("outside sentinel"),
            b"outside remains unchanged"
        );
        assert_eq!(
            tokio::fs::read(manager.0.root.join("target/model.bin"))
                .await
                .expect("installed model"),
            bytes
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn writable_asset_paths_never_modify_multiply_linked_files() {
        let bytes = b"hard-link download fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        let outside_partial = temporary.path().join("outside-partial");
        fs::write(&outside_partial, &bytes[..5]).expect("outside partial");
        fs::create_dir_all(manager.0.root.join("target")).expect("target directory");
        let partial = partial_path(Path::new("target/model.bin"));
        fs::hard_link(&outside_partial, manager.0.root.join(&partial)).expect("partial hard link");
        let metadata = PartialMetadata::from_manifest(
            &manager.0.manifest,
            manager.0.manifest.files.first().expect("fixture file"),
        );
        fs::write(
            manager.0.root.join(partial_metadata_path(&partial)),
            serde_json::to_vec(&metadata).expect("partial metadata JSON"),
        )
        .expect("partial metadata");

        manager.start_download().expect("start guarded resume");
        let failed = wait_for_phase(&manager, ModelAssetPhase::Failed).await;

        assert_eq!(failed.error_code, Some(ModelAssetErrorCode::Storage));
        assert_eq!(
            fs::read(&outside_partial).expect("outside partial"),
            &bytes[..5]
        );

        let outside_marker = temporary.path().join("outside-marker");
        fs::write(&outside_marker, b"outside marker remains").expect("outside marker");
        fs::hard_link(
            &outside_marker,
            manager.0.root.join(format!("{OWNERSHIP_FILE}.tmp")),
        )
        .expect("marker temp hard link");

        manager
            .write_ownership_marker(&CancellationToken::new())
            .await
            .expect("private marker publication");
        assert_eq!(
            fs::read(&outside_marker).expect("outside marker"),
            b"outside marker remains"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn relinking_a_private_write_path_after_open_never_modifies_the_outside_file() {
        let temporary = TempDir::new().expect("temporary storage parent");
        let storage = SafeStorage::open(temporary.path().to_path_buf(), Path::new("models"))
            .expect("safe storage");
        let destination = Path::new("target/model.bin.part");
        let outside = temporary.path().join("outside");
        fs::write(&outside, b"outside remains unchanged").expect("outside sentinel");
        let mut private = storage
            .create_private_write(destination)
            .expect("private write");
        let retained = storage.absolute(Path::new("retained-private-write"));
        fs::rename(storage.absolute(private.temporary_path()), &retained)
            .expect("retain private inode");
        fs::hard_link(&outside, storage.absolute(private.temporary_path()))
            .expect("replace private path with outside hard link");

        private
            .write_all(b"new model bytes")
            .await
            .expect("write opened private inode");
        assert!(private.publish().await.is_err());

        assert_eq!(
            fs::read(&outside).expect("outside sentinel"),
            b"outside remains unchanged"
        );
        assert!(!storage.absolute(destination).exists());
        assert_eq!(
            fs::read(retained).expect("retained private inode"),
            b"new model bytes"
        );
    }

    #[tokio::test]
    async fn durable_phase_boundaries_recover_semantically_after_abrupt_process_loss() {
        let bytes = b"durable removal fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;

        for (mutation, expected_phase) in [
            (CrashMutation::Stage, ModelAssetPhase::Ready),
            (CrashMutation::Delete, ModelAssetPhase::Missing),
        ] {
            let temporary = TempDir::new().expect("temporary model root");
            let credentials = Arc::new(MemoryCredentials::default());
            let manager = test_manager(&temporary, &fixture, &bytes, credentials.clone(), u64::MAX);
            manager.start_download().expect("install fixture");
            wait_for_phase(&manager, ModelAssetPhase::Ready).await;
            manager.set_removal_persistence(Arc::new(CrashAfterDurableMutation::new(mutation)));

            let crashing_manager = manager.clone();
            let crashed =
                tokio::spawn(async move { crashing_manager.remove_owned_assets().await }).await;
            assert!(
                crashed.is_err(),
                "the injected process loss must abort the operation"
            );
            drop(manager);

            let relaunched = test_manager(&temporary, &fixture, &bytes, credentials, u64::MAX);
            if expected_phase == ModelAssetPhase::Ready {
                wait_for_phase(&relaunched, expected_phase).await;
                assert_eq!(
                    fs::read(relaunched.0.root.join("target/model.bin"))
                        .expect("recovered staged asset"),
                    bytes
                );
            } else {
                assert_eq!(relaunched.status().phase, expected_phase);
                assert!(!relaunched.0.root.join("target/model.bin").exists());
                assert!(!relaunched.0.root.join(OWNERSHIP_FILE).exists());
            }
            assert!(
                fs::read_dir(&relaunched.0.root)
                    .expect("model root")
                    .all(|entry| !entry
                        .expect("model root entry")
                        .file_name()
                        .to_string_lossy()
                        .starts_with(REMOVAL_STAGING_PREFIX))
            );
        }
    }

    #[tokio::test]
    async fn failed_phase_barriers_prevent_the_destructive_phase_they_guard() {
        let bytes = b"phase barrier failure fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;

        for phase in [
            RemovalRecoveryPhase::Staging,
            RemovalRecoveryPhase::Deleting,
        ] {
            let temporary = TempDir::new().expect("temporary model root");
            let manager = test_manager(
                &temporary,
                &fixture,
                &bytes,
                Arc::new(MemoryCredentials::default()),
                u64::MAX,
            );
            manager.start_download().expect("install fixture");
            wait_for_phase(&manager, ModelAssetPhase::Ready).await;
            let persistence = Arc::new(FailingPhaseBarrier::new(phase));
            manager.set_removal_persistence(persistence.clone());

            assert!(matches!(
                manager.remove_owned_assets().await,
                Err(ModelAssetError::Storage { .. })
            ));

            if phase == RemovalRecoveryPhase::Staging {
                assert_eq!(persistence.staged.load(Ordering::Acquire), 0);
            } else {
                assert!(persistence.staged.load(Ordering::Acquire) > 0);
            }
            assert_eq!(persistence.deleted.load(Ordering::Acquire), 0);
            assert_eq!(
                fs::read(manager.0.root.join("target/model.bin")).expect("restored asset"),
                bytes
            );
            assert!(manager.0.root.join(OWNERSHIP_FILE).exists());
            assert!(
                fs::read_dir(&manager.0.root)
                    .expect("model root")
                    .all(|entry| !entry
                        .expect("model root entry")
                        .file_name()
                        .to_string_lossy()
                        .starts_with(REMOVAL_STAGING_PREFIX))
            );
        }
    }

    #[tokio::test]
    async fn failed_completion_full_sync_keeps_the_deleting_journal_for_recovery() {
        let bytes = b"completion sync failure fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        manager.start_download().expect("install fixture");
        wait_for_phase(&manager, ModelAssetPhase::Ready).await;
        manager.set_removal_persistence(Arc::new(FailingCompletionSync));

        assert!(matches!(
            manager.remove_owned_assets().await,
            Err(ModelAssetError::RecoveryRequired(_))
        ));
        let staging = fs::read_dir(&manager.0.root)
            .expect("model root")
            .filter_map(Result::ok)
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(REMOVAL_STAGING_PREFIX)
            })
            .expect("retained removal recovery directory");
        assert!(staging.path().join(REMOVAL_RECOVERY_FILE).exists());
        assert!(!manager.0.root.join("target/model.bin").exists());
        assert!(!manager.0.root.join(OWNERSHIP_FILE).exists());
        assert!(manager.0.recovery_required.load(Ordering::Acquire));

        manager
            .retry_recovery()
            .await
            .expect("full-sync and clear the deleting journal");

        assert_eq!(manager.status().phase, ModelAssetPhase::Missing);
        assert!(!staging.path().exists());
        assert!(!manager.0.recovery_required.load(Ordering::Acquire));
    }

    async fn contended_recovery_manager(
        bytes: Vec<u8>,
    ) -> (TempDir, Arc<ContendedRecoveryDisk>, ModelAssetManager) {
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let manifest = fixture_manifest(&bytes);
        let disk = Arc::new(ContendedRecoveryDisk::default());
        let manager = ModelAssetManager::new(
            temporary.path().to_path_buf(),
            PathBuf::from("models"),
            manifest.clone(),
            format!("{}/resolve/{}", fixture.base_url, manifest.revision),
            format!("{}/whoami", fixture.base_url),
            Arc::new(MemoryCredentials::default()),
            disk.clone(),
        )
        .expect("test manager");
        manager.start_download().expect("install fixture");
        wait_for_phase(&manager, ModelAssetPhase::Ready).await;
        manager.set_removal_persistence(Arc::new(FailingCompletionSync));
        assert!(matches!(
            manager.remove_owned_assets().await,
            Err(ModelAssetError::RecoveryRequired(_))
        ));
        assert_eq!(
            manager.status().available_actions,
            vec![ModelAssetAction::RetryRecovery]
        );
        (temporary, disk, manager)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_recovery_retries_serialize_and_coalesce_to_one_safe_outcome() {
        let (_temporary, disk, manager) =
            contended_recovery_manager(b"serialized recovery fixture".to_vec()).await;

        disk.arm();
        let first_manager = manager.clone();
        let first = tokio::spawn(async move { first_manager.retry_recovery().await });
        wait_for_blocking_fixture(&disk.gate).await;
        assert_eq!(
            manager.status().available_actions,
            vec![ModelAssetAction::RetryRecovery]
        );

        let second_manager = manager.clone();
        let mut second = tokio::spawn(async move { second_manager.retry_recovery().await });
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut second)
                .await
                .is_err(),
            "the second retry must wait for the active journal mutation"
        );
        assert_async_executor_yields().await;
        assert_eq!(disk.max_in_flight.load(Ordering::Acquire), 1);
        assert_eq!(disk.calls.load(Ordering::Acquire), 1);
        assert_eq!(
            manager.status().available_actions,
            vec![ModelAssetAction::RetryRecovery]
        );

        disk.release();
        let first_status = first
            .await
            .expect("first retry task")
            .expect("first recovery result");
        let second_status = second
            .await
            .expect("second retry task")
            .expect("coalesced recovery result");
        assert_eq!(first_status, second_status);
        assert_eq!(second_status.phase, ModelAssetPhase::Missing);
        assert_eq!(
            second_status.available_actions,
            vec![ModelAssetAction::Download]
        );
        assert_eq!(
            manager
                .retry_recovery()
                .await
                .expect("already recovered retry result"),
            second_status
        );
        assert_eq!(disk.max_in_flight.load(Ordering::Acquire), 1);
        assert_eq!(disk.calls.load(Ordering::Acquire), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn queued_retry_success_cannot_be_overwritten_by_the_preceding_failure() {
        let (_temporary, disk, manager) =
            contended_recovery_manager(b"ordered recovery fixture".to_vec()).await;
        disk.arm_failure();

        let first_manager = manager.clone();
        let first = tokio::spawn(async move { first_manager.retry_recovery().await });
        wait_for_blocking_fixture(&disk.gate).await;
        let second_manager = manager.clone();
        let mut second = tokio::spawn(async move { second_manager.retry_recovery().await });
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut second)
                .await
                .is_err(),
            "the successful retry must queue behind the failing journal mutation"
        );
        assert_eq!(
            manager.status().available_actions,
            vec![ModelAssetAction::RetryRecovery]
        );

        disk.release();
        assert!(matches!(
            first.await.expect("failing retry task"),
            Err(ModelAssetError::Unavailable(message))
                if message == "injected first recovery capacity failure"
        ));
        let recovered = second
            .await
            .expect("successful retry task")
            .expect("queued recovery result");
        assert_eq!(recovered.phase, ModelAssetPhase::Missing);
        assert_eq!(
            recovered.available_actions,
            vec![ModelAssetAction::Download]
        );
        tokio::task::yield_now().await;
        assert_eq!(manager.status(), recovered);
        assert!(!manager.0.recovery_required.load(Ordering::Acquire));
        assert_eq!(disk.max_in_flight.load(Ordering::Acquire), 1);
        assert_eq!(disk.calls.load(Ordering::Acquire), 2);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn hash_gate_rejects_visible_replacement_and_in_place_mutation_after_reading() {
        let bytes = b"stable hash fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        fs::create_dir_all(manager.0.root.join("target")).expect("target directory");
        let relative = Path::new("target/model.bin");
        let visible = manager.0.root.join(relative);
        fs::write(&visible, &bytes).expect("hash candidate");
        let retained = manager.0.root.join("target/retained.bin");
        let replacement = manager.sha256_relative_with_identity_and_hook(
            relative,
            &CancellationToken::new(),
            || {
                fs::rename(&visible, &retained).expect("retain opened file");
                fs::write(&visible, &bytes).expect("same-byte replacement");
            },
        );
        assert!(matches!(
            replacement,
            Err(ModelAssetError::ChangedDuringVerification(_))
        ));

        fs::remove_file(&visible).expect("replacement cleanup");
        fs::rename(&retained, &visible).expect("restore candidate");
        let mut changed = bytes.clone();
        changed[0] ^= 0xff;
        let mutation = manager.sha256_relative_with_identity_and_hook(
            relative,
            &CancellationToken::new(),
            || fs::write(&visible, changed).expect("same-size mutation"),
        );
        assert!(matches!(
            mutation,
            Err(ModelAssetError::ChangedDuringVerification(_))
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn storage_root_components_cannot_redirect_to_a_symlink() {
        use std::os::unix::fs::symlink;

        let bytes = b"root symlink fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let outside = temporary.path().join("outside");
        tokio::fs::create_dir(&outside)
            .await
            .expect("outside directory");
        symlink(&outside, temporary.path().join("models")).expect("model root symlink");
        let manifest = fixture_manifest(&bytes);

        let result = ModelAssetManager::new(
            temporary.path().to_path_buf(),
            PathBuf::from("models"),
            manifest.clone(),
            format!("{}/resolve/{}", fixture.base_url, manifest.revision),
            format!("{}/whoami", fixture.base_url),
            Arc::new(MemoryCredentials::default()),
            Arc::new(FixedDisk(AtomicU64::new(u64::MAX))),
        );

        assert!(matches!(result, Err(ModelAssetError::Storage { .. })));
        assert!(
            tokio::fs::read_dir(&outside)
                .await
                .expect("outside directory")
                .next_entry()
                .await
                .expect("outside directory entry")
                .is_none()
        );
    }

    #[cfg(unix)]
    #[test]
    fn intermediate_app_data_components_cannot_redirect_to_a_symlink() {
        use std::os::unix::fs::symlink;

        let temporary = TempDir::new().expect("temporary storage parent");
        let trusted = temporary.path().join("trusted");
        let outside = temporary.path().join("outside");
        fs::create_dir(&trusted).expect("trusted directory");
        fs::create_dir(&outside).expect("outside directory");
        symlink(&outside, trusted.join("intermediate")).expect("intermediate symlink");
        let trusted_directory =
            Dir::open_ambient_dir(&trusted, ambient_authority()).expect("trusted capability");

        let result = SafeStorage::open_beneath(
            trusted_directory,
            Path::new("intermediate/app-data"),
            trusted.join("intermediate/app-data"),
            Path::new("models"),
        );

        assert!(result.is_err());
        assert!(
            fs::read_dir(&outside)
                .expect("outside directory")
                .next()
                .is_none()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn swapping_visible_app_data_path_cannot_redirect_bound_storage() {
        use std::os::unix::fs::symlink;

        let temporary = TempDir::new().expect("temporary storage parent");
        let trusted = temporary.path().join("trusted");
        let app_data = trusted.join("app-data");
        let retained = trusted.join("retained-app-data");
        let outside = temporary.path().join("outside");
        fs::create_dir(&trusted).expect("trusted directory");
        fs::create_dir(&app_data).expect("app data directory");
        fs::create_dir(&outside).expect("outside directory");
        let trusted_directory =
            Dir::open_ambient_dir(&trusted, ambient_authority()).expect("trusted capability");
        let storage = SafeStorage::open_beneath(
            trusted_directory,
            Path::new("app-data"),
            app_data.clone(),
            Path::new("models"),
        )
        .expect("bound storage");

        fs::rename(&app_data, &retained).expect("retain original app data");
        symlink(&outside, &app_data).expect("replace visible app data path");
        let relative = Path::new("target/probe.bin");
        let mut file = storage
            .create_private_write(relative)
            .expect("write through retained capability");
        file.write_all(b"capability-bound")
            .await
            .expect("write retained file");
        file.publish().await.expect("publish retained file");

        assert_eq!(
            fs::read(retained.join("models/target/probe.bin")).expect("retained bytes"),
            b"capability-bound"
        );
        assert!(
            fs::read_dir(&outside)
                .expect("outside directory")
                .next()
                .is_none()
        );
    }

    #[test]
    fn mismatched_owned_revision_is_never_treated_as_ready() {
        let bytes = b"old revision fixture";
        let manifest = fixture_manifest(bytes);
        let temporary = TempDir::new().expect("temporary model root");
        let root = temporary.path().join("models");
        fs::create_dir_all(root.join("target")).expect("target directory");
        fs::write(root.join("target/model.bin"), bytes).expect("model bytes");
        let mut marker = OwnershipMarker::from_manifest(&manifest);
        marker.revision = "ffffffffffffffffffffffffffffffffffffffff".into();
        fs::write(
            root.join(OWNERSHIP_FILE),
            serde_json::to_vec(&marker).expect("marker JSON"),
        )
        .expect("marker write");

        let storage = SafeStorage::open(temporary.path().to_path_buf(), Path::new("models"))
            .expect("safe storage");
        let status = ModelAssetManager::inspect_install(&storage, &manifest, u64::MAX, false)
            .expect("inspect mismatch");

        assert_eq!(status.status.phase, ModelAssetPhase::RevisionMismatch);
        assert!(!status.requires_validation);
    }

    #[tokio::test]
    async fn unsupported_ownership_metadata_never_adopts_existing_files() {
        let bytes = b"unsupported ownership fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;

        for metadata_case in [
            "schema-zero-current",
            "schema-zero-old",
            "foreign-owner-current",
            "foreign-owner-old",
            "unsupported-current-manifest",
        ] {
            let temporary = TempDir::new().expect("temporary model root");
            let manifest = fixture_manifest(&bytes);
            let root = temporary.path().join("models");
            fs::create_dir_all(root.join("target")).expect("target directory");
            fs::write(root.join("target/model.bin"), &bytes).expect("existing model bytes");
            let mut marker = OwnershipMarker::from_manifest(&manifest);
            match metadata_case {
                "schema-zero-current" => marker.schema_version = 0,
                "schema-zero-old" => {
                    marker.schema_version = 0;
                    marker.revision = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into();
                }
                "foreign-owner-current" => marker.owner = "com.example.foreign".into(),
                "foreign-owner-old" => {
                    marker.owner = "com.example.foreign".into();
                    marker.revision = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into();
                }
                "unsupported-current-manifest" => {
                    marker.manifest_id = "unsupported-manifest".into()
                }
                _ => unreachable!("fixed ownership metadata case"),
            }
            let marker_bytes = serde_json::to_vec(&marker).expect("unsupported marker JSON");
            fs::write(root.join(OWNERSHIP_FILE), &marker_bytes).expect("unsupported marker");

            let manager = ModelAssetManager::new(
                temporary.path().to_path_buf(),
                PathBuf::from("models"),
                manifest.clone(),
                format!("{}/resolve/{}", fixture.base_url, manifest.revision),
                format!("{}/whoami", fixture.base_url),
                Arc::new(MemoryCredentials::default()),
                Arc::new(FixedDisk(AtomicU64::new(u64::MAX))),
            )
            .expect("unsupported ownership is an actionable resource state");

            assert_eq!(manager.status().phase, ModelAssetPhase::Failed);
            assert_eq!(
                manager.status().error_code,
                Some(ModelAssetErrorCode::Ownership)
            );
            assert!(
                manager
                    .status()
                    .message
                    .expect("ownership recovery message")
                    .contains("Reset Più's pre-release application data")
            );
            assert!(matches!(
                manager.start_download(),
                Err(ModelAssetError::NotOwned)
            ));
            tokio::task::yield_now().await;
            assert_eq!(
                fs::read(root.join(OWNERSHIP_FILE)).expect("preserved unsupported marker"),
                marker_bytes
            );
            assert_eq!(
                fs::read(root.join("target/model.bin")).expect("preserved existing model"),
                bytes
            );
        }

        assert_eq!(fixture.state.requests.load(Ordering::Relaxed), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn same_size_tampering_is_verified_off_the_startup_path() {
        let bytes = vec![42; 32 * 1024 * 1024];
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let root = temporary.path().join("models");
        fs::create_dir_all(root.join("target")).expect("target directory");
        let mut tampered = bytes.clone();
        tampered[0] ^= 0xff;
        fs::write(root.join("target/model.bin"), tampered).expect("tampered model bytes");
        let manifest = fixture_manifest(&bytes);
        fs::write(
            root.join(OWNERSHIP_FILE),
            serde_json::to_vec(&OwnershipMarker::from_manifest(&manifest)).expect("marker JSON"),
        )
        .expect("marker write");

        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );

        assert_eq!(manager.status().phase, ModelAssetPhase::Verifying);
        let failed = wait_for_phase(&manager, ModelAssetPhase::Failed).await;
        assert_eq!(failed.error_code, Some(ModelAssetErrorCode::Integrity));
        assert_eq!(failed.transferred_bytes, 0);
        assert_eq!(failed.remaining_bytes, bytes.len() as u64);
        assert_eq!(
            failed.required_free_bytes,
            bytes.len() as u64 + DISK_SAFETY_RESERVE_BYTES
        );
        assert!(!failed.can_resume);
    }

    #[tokio::test]
    async fn oversized_ownership_marker_fails_startup_inspection_without_unbounded_reading() {
        let bytes = b"bounded marker fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let root = temporary.path().join("models");
        fs::create_dir(&root).expect("model root");
        fs::write(
            root.join(OWNERSHIP_FILE),
            vec![b'x'; OWNERSHIP_METADATA_MAX_BYTES as usize + 1],
        )
        .expect("oversized marker");
        let manifest = fixture_manifest(&bytes);

        let started = std::time::Instant::now();
        let result = ModelAssetManager::new(
            temporary.path().to_path_buf(),
            PathBuf::from("models"),
            manifest.clone(),
            format!("{}/resolve/{}", fixture.base_url, manifest.revision),
            format!("{}/whoami", fixture.base_url),
            Arc::new(MemoryCredentials::default()),
            Arc::new(FixedDisk(AtomicU64::new(u64::MAX))),
        );

        assert!(started.elapsed() < Duration::from_millis(250));
        assert!(matches!(result, Err(ModelAssetError::Storage { .. })));
    }

    async fn wait_for_blocking_fixture(fixture: &BlockingFixture) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !fixture.entered.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("blocking fixture entered");
    }

    async fn assert_async_executor_yields() {
        let (sent, received) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            sent.send(()).expect("executor probe receiver");
        });
        tokio::time::timeout(Duration::from_millis(100), received)
            .await
            .expect("initialization must not occupy the async executor")
            .expect("executor probe");
    }

    fn uninitialized_test_manager(
        temporary: &TempDir,
        credentials: Arc<dyn CredentialStore>,
        disk_space: Arc<dyn DiskSpace>,
    ) -> ModelAssetManager {
        let manifest = fixture_manifest(b"deferred initialization fixture");
        ModelAssetManager::new_uninitialized(
            temporary.path().to_path_buf(),
            PathBuf::from("models"),
            manifest,
            "http://127.0.0.1/resolve/revision".into(),
            "http://127.0.0.1/whoami".into(),
            credentials,
            disk_space,
            NetworkTimeouts::default(),
        )
        .expect("uninitialized manager")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn slow_storage_recovery_is_deferred_past_the_startup_budget_and_yields() {
        let temporary = TempDir::new().expect("temporary model root");
        let gate = Arc::new(BlockingFixture::default());
        let started = std::time::Instant::now();
        let manager = uninitialized_test_manager(
            &temporary,
            Arc::new(MemoryCredentials::default()),
            Arc::new(FixedDisk(AtomicU64::new(u64::MAX))),
        );
        assert!(started.elapsed() < Duration::from_millis(50));
        assert_eq!(manager.status().phase, ModelAssetPhase::Initializing);
        let hook = gate.clone();
        manager.start_deferred_initialization_with_hook(
            temporary.path().to_path_buf(),
            PathBuf::from("models"),
            move || hook.wait(),
        );

        wait_for_blocking_fixture(&gate).await;
        assert_async_executor_yields().await;
        assert_eq!(manager.status().phase, ModelAssetPhase::Initializing);
        gate.release();
        assert_eq!(
            wait_for_phase(&manager, ModelAssetPhase::Missing)
                .await
                .phase,
            ModelAssetPhase::Missing
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn slow_keychain_lookup_runs_off_the_async_executor() {
        let temporary = TempDir::new().expect("temporary model root");
        let gate = Arc::new(BlockingFixture::default());
        let manager = uninitialized_test_manager(
            &temporary,
            Arc::new(SlowCredentials(gate.clone())),
            Arc::new(FixedDisk(AtomicU64::new(u64::MAX))),
        );
        manager
            .start_deferred_initialization(temporary.path().to_path_buf(), PathBuf::from("models"));

        wait_for_blocking_fixture(&gate).await;
        assert_async_executor_yields().await;
        gate.release();
        wait_for_phase(&manager, ModelAssetPhase::Missing).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn slow_storage_capacity_check_runs_off_the_async_executor() {
        let temporary = TempDir::new().expect("temporary model root");
        let gate = Arc::new(BlockingFixture::default());
        let manager = uninitialized_test_manager(
            &temporary,
            Arc::new(MemoryCredentials::default()),
            Arc::new(SlowDisk(gate.clone())),
        );
        manager
            .start_deferred_initialization(temporary.path().to_path_buf(), PathBuf::from("models"));

        wait_for_blocking_fixture(&gate).await;
        assert_async_executor_yields().await;
        gate.release();
        wait_for_phase(&manager, ModelAssetPhase::Missing).await;
    }

    #[tokio::test]
    async fn storage_initialization_failure_is_contained_in_resource_status() {
        let app_data_file = tempfile::NamedTempFile::new().expect("app data file");

        let manager = ModelAssetManager::production_or_unavailable(app_data_file.path());

        assert_eq!(manager.status().phase, ModelAssetPhase::Initializing);
        let failed = wait_for_phase(&manager, ModelAssetPhase::Failed).await;
        assert!(
            failed
                .message
                .expect("failure message")
                .contains("Quit and reopen Più")
        );
        assert!(failed.available_actions.is_empty());
        assert!(matches!(
            manager.start_download(),
            Err(ModelAssetError::Unavailable(_))
        ));
    }
}
