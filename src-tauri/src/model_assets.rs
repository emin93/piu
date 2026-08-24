use std::{
    collections::HashSet,
    ffi::OsString,
    io::{self, Read as _},
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
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
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
    sync::watch,
};
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
const REMOVAL_STAGING_PREFIX: &str = ".piu-removal-";
const REMOVAL_RECOVERY_FILE: &str = "recovery.json";
const REMOVAL_RECOVERY_MAX_BYTES: u64 = 128 * 1024;
const REMOVAL_RECOVERY_MAX_ENTRIES: usize = 64;

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
    Network(#[from] reqwest::Error),
    #[error("Hugging Face timed out while waiting for {0}")]
    NetworkTimeout(&'static str),
    #[error("download was cancelled")]
    Cancelled,
    #[error("another model resource operation is already running")]
    OperationInProgress,
    #[error("model assets are not owned by this Più manifest and were left untouched")]
    NotOwned,
    #[error(
        "model asset revision differs from the pinned Più revision; remove it manually before downloading"
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
            | Self::OperationInProgress => ModelAssetErrorCode::Storage,
            Self::Manifest(_) => ModelAssetErrorCode::Manifest,
        }
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
    validation: Option<ValidationPlan>,
}

struct ValidationPlan {
    migrate_legacy_marker: bool,
}

/// Capability-anchors every asset operation at the application-owned model root.
/// Parent directories and final components are opened without following symlinks,
/// so a concurrent path replacement cannot redirect reads or writes outside the root.
struct SafeStorage {
    root: PathBuf,
    directory: Dir,
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

    fn open_read(&self, relative: &Path) -> io::Result<File> {
        let (directory, name) = self.parent_and_name(relative, false)?;
        let mut options = CapOpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        directory
            .open_with(name, &options)
            .map(|file| File::from_std(file.into_std()))
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

    fn open_write(&self, relative: &Path, append: bool) -> io::Result<File> {
        use std::os::unix::fs::MetadataExt;

        let (directory, name) = self.parent_and_name(relative, true)?;
        let mut options = CapOpenOptions::new();
        options
            .write(true)
            .create(true)
            .append(append)
            .follow(FollowSymlinks::No);
        let file = directory.open_with(name, &options)?.into_std();
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "model asset writes require a private single-link file",
            ));
        }
        if !append {
            file.set_len(0)?;
        }
        Ok(File::from_std(file))
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

    fn entry_names(&self, relative: Option<&Path>) -> io::Result<Vec<OsString>> {
        let directory = if let Some(relative) = relative {
            let (parent, name) = self.parent_and_name(relative, false)?;
            parent.open_dir_nofollow(name)?
        } else {
            self.directory.try_clone()?
        };
        directory
            .entries()?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect()
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

struct ModelAssetManagerInner {
    root: PathBuf,
    storage: Option<SafeStorage>,
    manifest: AssetManifest,
    resolve_base_url: String,
    whoami_url: String,
    client: Client,
    network_timeouts: NetworkTimeouts,
    credentials: Arc<dyn CredentialStore>,
    disk_space: Arc<dyn DiskSpace>,
    status: watch::Sender<ModelAssetStatus>,
    active: Mutex<Option<ActiveOperation>>,
    invalid_finals: Mutex<HashSet<String>>,
    next_operation_id: AtomicU64,
    initialization_error: Option<String>,
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
        Self::new(
            app_data.to_path_buf(),
            PathBuf::from("models/qwen3.8-27b-uncensored-mlx"),
            manifest,
            resolve_base_url,
            "https://huggingface.co/api/whoami-v2".into(),
            Arc::new(KeychainCredentialStore),
            Arc::new(SystemDiskSpace),
        )
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
                storage: None,
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
                invalid_finals: Mutex::new(HashSet::new()),
                next_operation_id: AtomicU64::new(1),
                initialization_error: Some(message),
            }))
        })
    }

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
        manifest.validate()?;
        let root = storage_anchor.join(&storage_relative);
        let storage = SafeStorage::open(storage_anchor, &storage_relative).map_err(|source| {
            ModelAssetError::Storage {
                path: root.clone(),
                source,
            }
        })?;
        Self::recover_staged_removals(&storage)?;
        let free_bytes = disk_space.available(&root)?;
        let has_credentials = credentials.get()?.is_some();
        let inspection = Self::inspect_install(&storage, &manifest, free_bytes, has_credentials)?;
        let (status, _) = watch::channel(inspection.status);
        let manager = Self(Arc::new(ModelAssetManagerInner {
            root,
            storage: Some(storage),
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
            invalid_finals: Mutex::new(HashSet::new()),
            next_operation_id: AtomicU64::new(1),
            initialization_error: None,
        }));
        if let Some(plan) = inspection.validation {
            manager.start_background_validation(plan);
        }
        Ok(manager)
    }

    fn recover_staged_removals(storage: &SafeStorage) -> Result<(), ModelAssetError> {
        let mut staging_directories = storage
            .entry_names(None)
            .map_err(|source| ModelAssetError::Storage {
                path: storage.root.clone(),
                source,
            })?
            .into_iter()
            .filter_map(|name| {
                name.to_str()
                    .filter(|name| name.starts_with(REMOVAL_STAGING_PREFIX))
                    .map(PathBuf::from)
            })
            .collect::<Vec<_>>();
        staging_directories.sort();
        for staging_directory in staging_directories {
            Self::recover_staged_removal(storage, &staging_directory)?;
        }
        Ok(())
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
        if storage
            .entry_names(Some(staging_directory))
            .map_err(|source| ModelAssetError::Storage {
                path: storage.absolute(staging_directory),
                source,
            })?
            .iter()
            .any(|name| !allowed_names.contains(name))
        {
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
                                .rename(&entry.staged, &entry.original)
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
                        .remove_file(&entry.staged)
                        .map_err(|_| ModelAssetError::NotOwned)?;
                }
            }
        }
        Self::remove_recovery_metadata(storage, staging_directory)?;
        storage
            .remove_dir(staging_directory)
            .map_err(|_| ModelAssetError::NotOwned)
    }

    fn remove_abandoned_empty_staging(
        storage: &SafeStorage,
        staging_directory: &Path,
    ) -> Result<(), ModelAssetError> {
        let temporary = staging_directory.join(format!("{REMOVAL_RECOVERY_FILE}.tmp"));
        let entries = storage
            .entry_names(Some(staging_directory))
            .map_err(|_| ModelAssetError::NotOwned)?;
        if entries.is_empty() {
            return storage
                .remove_dir(staging_directory)
                .map_err(|_| ModelAssetError::NotOwned);
        }
        if entries == [OsString::from(format!("{REMOVAL_RECOVERY_FILE}.tmp"))]
            && storage
                .identity(&temporary)
                .map_err(|_| ModelAssetError::NotOwned)?
                .is_some_and(|identity| {
                    identity.links == 1 && identity.size <= REMOVAL_RECOVERY_MAX_BYTES
                })
        {
            storage
                .remove_file(&temporary)
                .map_err(|_| ModelAssetError::NotOwned)?;
            return storage
                .remove_dir(staging_directory)
                .map_err(|_| ModelAssetError::NotOwned);
        }
        Err(ModelAssetError::NotOwned)
    }

    fn remove_recovery_metadata(
        storage: &SafeStorage,
        staging_directory: &Path,
    ) -> Result<(), ModelAssetError> {
        for name in [
            format!("{REMOVAL_RECOVERY_FILE}.tmp"),
            REMOVAL_RECOVERY_FILE.into(),
        ] {
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
                .remove_file(&path)
                .map_err(|_| ModelAssetError::NotOwned)?;
        }
        Ok(())
    }

    fn start_background_validation(&self, plan: ValidationPlan) {
        let operation_id = self.0.next_operation_id.fetch_add(1, Ordering::Relaxed);
        let cancellation = CancellationToken::new();
        *self.0.active.lock().expect("model asset operation lock") = Some(ActiveOperation {
            operation_id,
            cancellation: cancellation.clone(),
            kind: OperationKind::Validation,
        });
        let mut status = self.status();
        status.operation_id = Some(operation_id);
        self.publish(status);
        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            let result = manager
                .validate_existing_install(operation_id, cancellation, plan)
                .await;
            manager.finish_operation(operation_id, result);
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
            let result = manager.download_all(operation_id, cancellation).await;
            manager.finish_operation(operation_id, result);
        });
        Ok(operation_id)
    }

    fn begin_download(&self) -> Result<(u64, Option<CancellationToken>), ModelAssetError> {
        if let Some(error) = &self.0.initialization_error {
            return Err(ModelAssetError::Unavailable(error.clone()));
        }
        let mut active = self.0.active.lock().expect("model asset operation lock");
        if let Some(active) = active.as_ref() {
            return if active.kind == OperationKind::Download {
                Ok((active.operation_id, None))
            } else {
                Err(ModelAssetError::OperationInProgress)
            };
        }
        if self.status().phase == ModelAssetPhase::Ready {
            return Ok((0, None));
        }
        if self.status().phase == ModelAssetPhase::RevisionMismatch {
            return Err(ModelAssetError::RevisionMismatch);
        }
        let operation_id = self.0.next_operation_id.fetch_add(1, Ordering::Relaxed);
        let cancellation = CancellationToken::new();
        *active = Some(ActiveOperation {
            operation_id,
            cancellation: cancellation.clone(),
            kind: OperationKind::Download,
        });
        drop(active);
        Ok((operation_id, Some(cancellation)))
    }

    pub fn cancel_download(&self) -> bool {
        let active = self.0.active.lock().expect("model asset operation lock");
        if let Some(active) = active.as_ref() {
            active.cancellation.cancel();
            true
        } else {
            false
        }
    }

    pub async fn authorize_hugging_face(&self, token: String) -> Result<(), ModelAssetError> {
        if let Some(error) = &self.0.initialization_error {
            return Err(ModelAssetError::Unavailable(error.clone()));
        }
        let token = token.trim();
        if token.is_empty() {
            return Err(ModelAssetError::Authentication);
        }
        let response = tokio::time::timeout(
            self.0.network_timeouts.headers,
            self.0
                .client
                .get(&self.0.whoami_url)
                .bearer_auth(token)
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
        self.0.credentials.set(token)?;
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

    async fn remove_owned_assets_with_hook(
        &self,
        after_verified: impl FnMut(&Path),
    ) -> Result<ModelAssetStatus, ModelAssetError> {
        if let Some(error) = &self.0.initialization_error {
            return Err(ModelAssetError::Unavailable(error.clone()));
        }
        let (operation_id, cancellation) = self.begin_removal()?;
        match self
            .perform_owned_removal(operation_id, &cancellation, after_verified)
            .await
        {
            Ok(status) => Ok(status),
            Err(error) => {
                self.publish_operation_failure(operation_id, &error);
                Err(error)
            }
        }
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

    async fn perform_owned_removal(
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
        if let Err(error) = self
            .write_json_atomic(
                &staging_root.join(REMOVAL_RECOVERY_FILE),
                &recovery_plan,
                cancellation,
            )
            .await
        {
            let _ = Self::remove_abandoned_empty_staging(self.storage(), &staging_root);
            return Err(error);
        }
        let mut staged = Vec::with_capacity(marker.files.len() + 1);
        for (index, file) in marker.files.iter().enumerate() {
            ensure_not_cancelled(cancellation).inspect_err(|_| {
                let _ = self.rollback_staged(&staging_root, &staged);
            })?;
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
                self.rollback_staged(&staging_root, &staged)?;
                return Err(ModelAssetError::NotOwned);
            }
            let staged_path = staging_root.join(format!("asset-{index}"));
            if let Err(source) = self.storage().rename(relative, &staged_path) {
                self.rollback_staged(&staging_root, &staged)?;
                return Err(self.relative_storage_error(relative, source));
            }
            let (hash, identity) = self
                .sha256_relative_with_identity(&staged_path, cancellation)
                .await
                .map_err(|error| match error {
                    ModelAssetError::Cancelled => ModelAssetError::Cancelled,
                    _ => ModelAssetError::NotOwned,
                })
                .or_else(|error| {
                    self.restore_staged_path(relative, &staged_path)?;
                    self.rollback_staged(&staging_root, &staged)?;
                    Err(error)
                })?;
            staged.push(StagedRemoval {
                original: relative.to_path_buf(),
                staged: staged_path,
                identity,
            });
            if hash != file.sha256 || identity.size != file.size_bytes {
                self.rollback_staged(&staging_root, &staged)?;
                return Err(ModelAssetError::NotOwned);
            }
            after_verified(relative);
        }

        ensure_not_cancelled(cancellation).inspect_err(|_| {
            let _ = self.rollback_staged(&staging_root, &staged);
        })?;
        let staged_marker = staging_root.join("ownership-marker");
        if let Err(source) = self.storage().rename(marker_path, &staged_marker) {
            self.rollback_staged(&staging_root, &staged)?;
            return Err(self.relative_storage_error(marker_path, source));
        }
        let (staged_marker_bytes, marker_identity) = self
            .storage()
            .read_bounded_with_identity(&staged_marker, OWNERSHIP_METADATA_MAX_BYTES)
            .map_err(|_| ModelAssetError::NotOwned)
            .or_else(|error| {
                self.restore_staged_path(marker_path, &staged_marker)?;
                self.rollback_staged(&staging_root, &staged)?;
                Err(error)
            })?;
        staged.push(StagedRemoval {
            original: marker_path.to_path_buf(),
            staged: staged_marker,
            identity: marker_identity,
        });
        if staged_marker_bytes != marker_bytes || self.staged_identities_match(&staged).is_err() {
            self.rollback_staged(&staging_root, &staged)?;
            return Err(ModelAssetError::NotOwned);
        }
        ensure_not_cancelled(cancellation).inspect_err(|_| {
            let _ = self.rollback_staged(&staging_root, &staged);
        })?;
        recovery_plan.phase = RemovalRecoveryPhase::Deleting;
        for entry in &mut recovery_plan.entries {
            entry.identity = staged
                .iter()
                .find(|staged| staged.staged == entry.staged)
                .map(|staged| staged.identity);
            if entry.identity.is_none() {
                self.rollback_staged(&staging_root, &staged)?;
                return Err(ModelAssetError::NotOwned);
            }
        }
        if let Err(error) = self
            .write_json_atomic(
                &staging_root.join(REMOVAL_RECOVERY_FILE),
                &recovery_plan,
                cancellation,
            )
            .await
        {
            self.rollback_staged(&staging_root, &staged)?;
            return Err(error);
        }
        for entry in &staged {
            if self
                .storage()
                .identity(&entry.staged)
                .map_err(|_| ModelAssetError::NotOwned)?
                != Some(entry.identity)
            {
                self.rollback_staged(&staging_root, &staged)?;
                return Err(ModelAssetError::NotOwned);
            }
            self.storage()
                .remove_file(&entry.staged)
                .map_err(|source| self.relative_storage_error(&entry.staged, source))?;
        }
        Self::remove_recovery_metadata(self.storage(), &staging_root)?;
        self.storage()
            .remove_dir(&staging_root)
            .map_err(|source| self.relative_storage_error(&staging_root, source))?;
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
                Ok(()) => return Ok(path),
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
                .metadata(&entry.original)
                .map_err(|_| ModelAssetError::NotOwned)?
                .is_some()
            {
                return Err(ModelAssetError::NotOwned);
            }
            self.storage()
                .rename(&entry.staged, &entry.original)
                .map_err(|_| ModelAssetError::NotOwned)?;
        }
        Self::remove_recovery_metadata(self.storage(), staging_root)?;
        self.storage()
            .remove_dir(staging_root)
            .map_err(|_| ModelAssetError::NotOwned)
    }

    fn restore_staged_path(&self, original: &Path, staged: &Path) -> Result<(), ModelAssetError> {
        if self
            .storage()
            .metadata(original)
            .map_err(|_| ModelAssetError::NotOwned)?
            .is_some()
        {
            return Err(ModelAssetError::NotOwned);
        }
        self.storage()
            .rename(staged, original)
            .map_err(|_| ModelAssetError::NotOwned)
    }

    async fn download_all(
        &self,
        operation_id: u64,
        cancellation: CancellationToken,
    ) -> Result<(), ModelAssetError> {
        let transferred = self.transferred_bytes();
        let (free, remaining, required) = self.ensure_space(transferred)?;
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
            if self
                .final_is_valid(file, operation_id, &cancellation)
                .await?
            {
                continue;
            }
            // Hash validation may have removed a same-size corrupt final after the
            // optimistic startup sample. Recompute the gate before allocating its
            // replacement so stale bytes can never hide the actual space requirement.
            let transferred = self.transferred_bytes();
            let (free, remaining, required) = self.ensure_space(transferred)?;
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

    async fn validate_existing_install(
        &self,
        operation_id: u64,
        cancellation: CancellationToken,
        plan: ValidationPlan,
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
            match self
                .sha256_relative(Path::new(&file.install_path), &cancellation)
                .await
            {
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
        if plan.migrate_legacy_marker {
            self.write_ownership_marker(&cancellation).await?;
        }
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
        status.current_free_bytes = self.0.disk_space.available(&self.0.root)?;
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
        let expected_metadata = PartialMetadata::from_manifest(&self.0.manifest, manifest_file);
        let recorded_metadata = self
            .storage()
            .read_bounded(&metadata_path, PARTIAL_METADATA_MAX_BYTES)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<PartialMetadata>(&bytes).ok());
        let partial_metadata = self
            .storage()
            .metadata(&partial)
            .map_err(|source| self.relative_storage_error(&partial, source))?;
        let safe_bound_partial = partial_metadata.as_ref().is_some_and(|metadata| {
            metadata.is_file()
                && !metadata.file_type().is_symlink()
                && recorded_metadata.as_ref() == Some(&expected_metadata)
                && metadata.len() <= manifest_file.size_bytes
        });
        if !safe_bound_partial && (partial_metadata.is_some() || recorded_metadata.is_some()) {
            self.reset_partial(&partial, &metadata_path)?;
        }
        let mut offset = if safe_bound_partial {
            partial_metadata.expect("bound partial metadata").len()
        } else {
            0
        };
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
            if self.sha256_relative(&partial, cancellation).await? == manifest_file.sha256 {
                ensure_not_cancelled(cancellation)?;
                self.storage()
                    .rename(&partial, &destination)
                    .map_err(|source| self.relative_storage_error(&destination, source))?;
                self.remove_if_present(&metadata_path)?;
                self.clear_invalid_final(&manifest_file.install_path);
                return Ok(());
            }
            self.reset_partial(&partial, &metadata_path)?;
            self.write_json_atomic(&metadata_path, &expected_metadata, cancellation)
                .await?;
            offset = 0;
        }
        let token = self.0.credentials.get()?;
        self.publish_current_work(manifest_file, operation_id, ModelAssetPhase::Downloading);
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

        let mut output = self
            .storage()
            .open_write(&partial, offset > 0)
            .map_err(|source| self.relative_storage_error(&partial, source))?;
        let mut stream = response.bytes_stream();
        let mut file_bytes = offset;
        let mut progress = TransferProgress::new(self.transferred_bytes(), offset);
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
        output
            .flush()
            .await
            .map_err(|source| self.relative_storage_error(&partial, source))?;
        drop(output);
        if file_bytes != manifest_file.size_bytes {
            return Err(ModelAssetError::SizeMismatch {
                path: manifest_file.source_path.clone(),
                actual: file_bytes,
                expected: manifest_file.size_bytes,
            });
        }
        let mut status = self.status();
        status.phase = ModelAssetPhase::Verifying;
        status.current_asset = Some(manifest_file.asset);
        status.current_file = Some(manifest_file.install_path.clone());
        self.publish(status);
        if self.sha256_relative(&partial, cancellation).await? != manifest_file.sha256 {
            self.reset_partial(&partial, &metadata_path)?;
            return Err(ModelAssetError::Integrity(
                manifest_file.source_path.clone(),
            ));
        }
        ensure_not_cancelled(cancellation)?;
        self.storage()
            .rename(&partial, &destination)
            .map_err(|source| self.relative_storage_error(&destination, source))?;
        self.remove_if_present(&metadata_path)?;
        self.clear_invalid_final(&manifest_file.install_path);
        Ok(())
    }

    async fn final_is_valid(
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
            match self.sha256_relative(path, cancellation).await {
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

    fn finish_operation(&self, operation_id: u64, result: Result<(), ModelAssetError>) {
        let Err(error) = result else { return };
        self.publish_operation_failure(operation_id, &error);
    }

    fn publish_operation_failure(&self, operation_id: u64, error: &ModelAssetError) {
        let mut active = self.0.active.lock().expect("model asset operation lock");
        if active.as_ref().map(|active| active.operation_id) == Some(operation_id) {
            *active = None;
        }
        drop(active);
        let mut status = self.status();
        status.operation_id = None;
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
        status.message = Some(error.to_string());
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
        if let Ok(free) = self.0.disk_space.available(&self.0.root) {
            status.current_free_bytes = free;
        }
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

    fn publish(&self, status: ModelAssetStatus) {
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
            if marker.repository == manifest.repository && marker.revision != manifest.revision {
                status.phase = ModelAssetPhase::RevisionMismatch;
                status.message = Some(ModelAssetError::RevisionMismatch.to_string());
                status.error_code = Some(ModelAssetErrorCode::RevisionMismatch);
                return Ok(InstallInspection {
                    status,
                    validation: None,
                });
            }
            let marker_is_legacy = marker.schema_version == 0 && marker.matches_payload(manifest);
            let marker_is_current = marker.matches(manifest);
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
            if (marker_is_legacy || marker_is_current) && all_files_have_pinned_size {
                // Size is only a cheap candidate check. The model remains unavailable
                // until cancellation-aware background SHA-256 validation completes.
                status.phase = ModelAssetPhase::Verifying;
                status.transferred_bytes = status.total_bytes;
                status.remaining_bytes = 0;
                status.required_free_bytes = 0;
                status.message = Some("Verifying installed model assets before use.".into());
                return Ok(InstallInspection {
                    status,
                    validation: Some(ValidationPlan {
                        migrate_legacy_marker: marker_is_legacy,
                    }),
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
            validation: None,
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
        let mut temporary = path.as_os_str().to_os_string();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        let bytes = serde_json::to_vec_pretty(value).expect("asset metadata serialization");
        let mut output = self
            .storage()
            .open_write(&temporary, false)
            .map_err(|source| self.relative_storage_error(&temporary, source))?;
        output
            .write_all(&bytes)
            .await
            .map_err(|source| self.relative_storage_error(&temporary, source))?;
        output
            .flush()
            .await
            .map_err(|source| self.relative_storage_error(&temporary, source))?;
        output
            .sync_all()
            .await
            .map_err(|source| self.relative_storage_error(&temporary, source))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;

            if output
                .metadata()
                .await
                .map_err(|source| self.relative_storage_error(&temporary, source))?
                .nlink()
                != 1
            {
                return Err(self.relative_storage_error(
                    &temporary,
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "model asset metadata gained an unsafe hard link",
                    ),
                ));
            }
        }
        drop(output);
        ensure_not_cancelled(cancellation)?;
        self.storage()
            .rename(&temporary, path)
            .map_err(|source| self.relative_storage_error(path, source))
    }

    async fn sha256_relative(
        &self,
        path: &Path,
        cancellation: &CancellationToken,
    ) -> Result<String, ModelAssetError> {
        self.sha256_relative_with_identity(path, cancellation)
            .await
            .map(|(hash, _)| hash)
    }

    #[cfg(unix)]
    async fn sha256_relative_with_identity(
        &self,
        path: &Path,
        cancellation: &CancellationToken,
    ) -> Result<(String, FileIdentity), ModelAssetError> {
        self.sha256_relative_with_identity_and_hook(path, cancellation, || {})
            .await
    }

    #[cfg(unix)]
    async fn sha256_relative_with_identity_and_hook(
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
            .await
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
            let read = tokio::select! {
                _ = cancellation.cancelled() => return Err(ModelAssetError::Cancelled),
                read = file.read(&mut buffer) => read,
            }
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
            .await
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

    fn clear_invalid_final(&self, install_path: &str) {
        self.0
            .invalid_finals
            .lock()
            .expect("invalid model assets lock")
            .remove(install_path);
    }

    fn storage(&self) -> &SafeStorage {
        self.0
            .storage
            .as_ref()
            .expect("available model manager has safe storage")
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
        self.schema_version == 1 && self.matches_payload(manifest)
    }

    fn matches_payload(&self, manifest: &AssetManifest) -> bool {
        self.owner == "ch.emin.piu"
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
        if !matches!(self.schema_version, 0 | 1)
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

fn safe_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn ensure_not_cancelled(cancellation: &CancellationToken) -> Result<(), ModelAssetError> {
    if cancellation.is_cancelled() {
        Err(ModelAssetError::Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        convert::Infallible,
        fs,
        sync::{
            Arc, Mutex,
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
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if fs::metadata(manager.0.root.join(&partial))
                    .is_ok_and(|metadata| metadata.len() >= 4)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("partial progress");
        running.abort();
        assert!(running.await.is_err());
        let stopped_bytes = fs::metadata(manager.0.root.join(&partial))
            .expect("stopped partial")
            .len();
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
        wait_for_phase(&manager, ModelAssetPhase::Downloading).await;
        tokio::time::sleep(Duration::from_millis(55)).await;
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

        let missing = manager
            .remove_owned_assets_with_hook(|relative| {
                fs::write(root.join(relative), replacement).expect("replacement file");
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

        assert!(
            manager
                .write_ownership_marker(&CancellationToken::new())
                .await
                .is_err()
        );
        assert_eq!(
            fs::read(&outside_marker).expect("outside marker"),
            b"outside marker remains"
        );
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
        let replacement = manager
            .sha256_relative_with_identity_and_hook(relative, &CancellationToken::new(), || {
                fs::rename(&visible, &retained).expect("retain opened file");
                fs::write(&visible, &bytes).expect("same-byte replacement");
            })
            .await;
        assert!(matches!(
            replacement,
            Err(ModelAssetError::ChangedDuringVerification(_))
        ));

        fs::remove_file(&visible).expect("replacement cleanup");
        fs::rename(&retained, &visible).expect("restore candidate");
        let mut changed = bytes.clone();
        changed[0] ^= 0xff;
        let mutation = manager
            .sha256_relative_with_identity_and_hook(relative, &CancellationToken::new(), || {
                fs::write(&visible, changed).expect("same-size mutation")
            })
            .await;
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
            .open_write(relative, false)
            .expect("write through retained capability");
        file.write_all(b"capability-bound")
            .await
            .expect("write retained file");
        file.flush().await.expect("flush retained file");

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
        assert!(status.validation.is_none());
    }

    #[tokio::test]
    async fn exact_legacy_ownership_marker_migrates_only_after_background_hashing() {
        let bytes = b"legacy marker fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let root = temporary.path().join("models");
        fs::create_dir_all(root.join("target")).expect("target directory");
        fs::write(root.join("target/model.bin"), &bytes).expect("model bytes");
        let manifest = fixture_manifest(&bytes);
        let mut marker = OwnershipMarker::from_manifest(&manifest);
        marker.schema_version = 0;
        fs::write(
            root.join(OWNERSHIP_FILE),
            serde_json::to_vec(&marker).expect("marker JSON"),
        )
        .expect("legacy marker write");

        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );
        assert_eq!(manager.status().phase, ModelAssetPhase::Verifying);
        wait_for_phase(&manager, ModelAssetPhase::Ready).await;
        let migrated: OwnershipMarker =
            serde_json::from_slice(&fs::read(root.join(OWNERSHIP_FILE)).expect("migrated marker"))
                .expect("migrated JSON");

        assert_eq!(migrated.schema_version, 1);
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

        let started = std::time::Instant::now();
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );

        assert!(started.elapsed() < Duration::from_millis(250));
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

    #[tokio::test(flavor = "current_thread")]
    async fn cancelling_background_hash_never_migrates_marker_or_publishes_ready() {
        let bytes = vec![7; 16 * 1024 * 1024];
        let fixture = Fixture::start(bytes.clone()).await;
        let temporary = TempDir::new().expect("temporary model root");
        let root = temporary.path().join("models");
        fs::create_dir_all(root.join("target")).expect("target directory");
        fs::write(root.join("target/model.bin"), &bytes).expect("model bytes");
        let manifest = fixture_manifest(&bytes);
        let mut marker = OwnershipMarker::from_manifest(&manifest);
        marker.schema_version = 0;
        fs::write(
            root.join(OWNERSHIP_FILE),
            serde_json::to_vec(&marker).expect("legacy marker JSON"),
        )
        .expect("legacy marker write");
        let manager = test_manager(
            &temporary,
            &fixture,
            &bytes,
            Arc::new(MemoryCredentials::default()),
            u64::MAX,
        );

        assert!(manager.cancel_download());
        wait_for_phase(&manager, ModelAssetPhase::Cancelled).await;
        let retained: OwnershipMarker =
            serde_json::from_slice(&fs::read(root.join(OWNERSHIP_FILE)).expect("retained marker"))
                .expect("retained marker JSON");
        assert_eq!(retained.schema_version, 0);
        assert_ne!(manager.status().phase, ModelAssetPhase::Ready);
    }

    #[test]
    fn storage_initialization_failure_is_contained_in_resource_status() {
        let app_data_file = tempfile::NamedTempFile::new().expect("app data file");

        let manager = ModelAssetManager::production_or_unavailable(app_data_file.path());

        assert_eq!(manager.status().phase, ModelAssetPhase::Failed);
        assert!(
            manager
                .status()
                .message
                .expect("failure message")
                .contains("model storage")
        );
        assert!(matches!(
            manager.start_download(),
            Err(ModelAssetError::Unavailable(_))
        ));
    }
}
