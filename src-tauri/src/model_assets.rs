use std::{
    collections::HashSet,
    fs, io,
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use futures_util::StreamExt;
use reqwest::{Client, StatusCode, header};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::{
    fs::{File, OpenOptions},
    io::{AsyncReadExt, AsyncWriteExt},
    sync::watch,
};
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

const EMBEDDED_MANIFEST: &str = include_str!("model-assets-v1.json");
const MANIFEST_SCHEMA_VERSION: u32 = 1;

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
        // Do not replace this with mlx-community/Qwen3.8-27B-MTP-4bit: that
        // independently published drafter declares block size 3. Più requires the
        // model author's same-revision `mtp/` assets, whose config declares block 4.
        if self.mtp_block_size != 4 || !self.drafter_selection_note.contains("block size 3") {
            return Err(ManifestError::Drafter(self.mtp_block_size));
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
    #[error("model asset manifest has unsupported MTP block size {0}; Più requires block 4")]
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
const KEYCHAIN_SERVICE: &str = "ch.emin.piu.huggingface";
const KEYCHAIN_ACCOUNT: &str = "access-token";
const DISK_SAFETY_RESERVE_BYTES: u64 = 1_073_741_824;
const PROGRESS_CHUNK_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ModelAssetPhase {
    Missing,
    Downloading,
    Verifying,
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
    #[error("download was cancelled")]
    Cancelled,
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
            Self::Integrity(_) => ModelAssetErrorCode::Integrity,
            Self::RevisionMismatch => ModelAssetErrorCode::RevisionMismatch,
            Self::Cancelled => ModelAssetErrorCode::Cancellation,
            Self::Network(_) | Self::InvalidRange(_) | Self::SizeMismatch { .. } => {
                ModelAssetErrorCode::Network
            }
            Self::NotOwned => ModelAssetErrorCode::Ownership,
            Self::Storage { .. } | Self::Credential(_) | Self::Unavailable(_) => {
                ModelAssetErrorCode::Storage
            }
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

struct ActiveDownload {
    operation_id: u64,
    cancellation: CancellationToken,
}

struct ModelAssetManagerInner {
    root: PathBuf,
    manifest: AssetManifest,
    resolve_base_url: String,
    whoami_url: String,
    client: Client,
    credentials: Arc<dyn CredentialStore>,
    disk_space: Arc<dyn DiskSpace>,
    status: watch::Sender<ModelAssetStatus>,
    active: Mutex<Option<ActiveDownload>>,
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
            app_data.join("models/qwen3.8-27b-uncensored-mlx"),
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
                resolve_base_url: format!(
                    "https://huggingface.co/{}/resolve/{}",
                    manifest.repository, manifest.revision
                ),
                whoami_url: "https://huggingface.co/api/whoami-v2".into(),
                manifest,
                client: Client::new(),
                credentials: Arc::new(KeychainCredentialStore),
                disk_space: Arc::new(SystemDiskSpace),
                status,
                active: Mutex::new(None),
                next_operation_id: AtomicU64::new(1),
                initialization_error: Some(message),
            }))
        })
    }

    fn new(
        root: PathBuf,
        manifest: AssetManifest,
        resolve_base_url: String,
        whoami_url: String,
        credentials: Arc<dyn CredentialStore>,
        disk_space: Arc<dyn DiskSpace>,
    ) -> Result<Self, ModelAssetError> {
        manifest.validate()?;
        fs::create_dir_all(&root).map_err(|source| ModelAssetError::Storage {
            path: root.clone(),
            source,
        })?;
        let free_bytes = disk_space.available(&root)?;
        let has_credentials = credentials.get()?.is_some();
        let status = Self::inspect_install(&root, &manifest, free_bytes, has_credentials)?;
        let (status, _) = watch::channel(status);
        Ok(Self(Arc::new(ModelAssetManagerInner {
            root,
            manifest,
            resolve_base_url,
            whoami_url,
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::limited(10))
                .build()?,
            credentials,
            disk_space,
            status,
            active: Mutex::new(None),
            next_operation_id: AtomicU64::new(1),
            initialization_error: None,
        })))
    }

    pub fn subscribe(&self) -> watch::Receiver<ModelAssetStatus> {
        self.0.status.subscribe()
    }

    pub fn status(&self) -> ModelAssetStatus {
        self.0.status.borrow().clone()
    }

    pub fn start_download(&self) -> Result<u64, ModelAssetError> {
        if let Some(error) = &self.0.initialization_error {
            return Err(ModelAssetError::Unavailable(error.clone()));
        }
        let mut active = self.0.active.lock().expect("model asset download lock");
        if let Some(active) = active.as_ref() {
            return Ok(active.operation_id);
        }
        if self.status().phase == ModelAssetPhase::Ready {
            return Ok(0);
        }
        if self.status().phase == ModelAssetPhase::RevisionMismatch {
            return Err(ModelAssetError::RevisionMismatch);
        }
        let operation_id = self.0.next_operation_id.fetch_add(1, Ordering::Relaxed);
        let cancellation = CancellationToken::new();
        *active = Some(ActiveDownload {
            operation_id,
            cancellation: cancellation.clone(),
        });
        drop(active);

        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            let result = manager.download_all(operation_id, cancellation).await;
            manager.finish_operation(operation_id, result);
        });
        Ok(operation_id)
    }

    pub fn cancel_download(&self) -> bool {
        let active = self.0.active.lock().expect("model asset download lock");
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
        let response = self
            .0
            .client
            .get(&self.0.whoami_url)
            .bearer_auth(token)
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(ModelAssetError::Authentication);
        }
        self.0.credentials.set(token)?;
        let mut status = self.status();
        status.authentication_configured = true;
        if status.phase == ModelAssetPhase::AuthenticationRequired {
            status.phase = ModelAssetPhase::Missing;
            status.message = Some("Hugging Face access is connected. Resume the download.".into());
        }
        self.publish(status);
        Ok(())
    }

    pub async fn remove_owned_assets(&self) -> Result<ModelAssetStatus, ModelAssetError> {
        if let Some(error) = &self.0.initialization_error {
            return Err(ModelAssetError::Unavailable(error.clone()));
        }
        if self.cancel_download() {
            return Err(ModelAssetError::Cancelled);
        }
        let marker_path = self.0.root.join(OWNERSHIP_FILE);
        let marker: OwnershipMarker = serde_json::from_slice(
            &tokio::fs::read(&marker_path)
                .await
                .map_err(|source| self.storage_error(&marker_path, source))?,
        )
        .map_err(|_| ModelAssetError::NotOwned)?;
        if !marker.authorizes_removal(&self.0.manifest) {
            return Err(ModelAssetError::NotOwned);
        }
        // Validate every extant entry before deleting anything. This makes recovery
        // atomic from an ownership perspective: a tampered or redirected path leaves
        // the complete old installation and its evidence intact for manual recovery.
        for file in &marker.files {
            let relative = Path::new(&file.install_path);
            let path = self.0.root.join(relative);
            let Ok(metadata) = tokio::fs::symlink_metadata(&path).await else {
                continue;
            };
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || metadata.len() != file.size_bytes
                || has_symlink_ancestor(&self.0.root, relative).await?
                || sha256_file(&path).await? != file.sha256
            {
                return Err(ModelAssetError::NotOwned);
            }
        }
        for file in &marker.files {
            let path = self.0.root.join(&file.install_path);
            if tokio::fs::try_exists(&path).await.unwrap_or(false) {
                tokio::fs::remove_file(&path)
                    .await
                    .map_err(|source| self.storage_error(&path, source))?;
            }
        }
        tokio::fs::remove_file(&marker_path)
            .await
            .map_err(|source| self.storage_error(&marker_path, source))?;
        for directory in [self.0.root.join("target"), self.0.root.join("drafter")] {
            let _ = tokio::fs::remove_dir(directory).await;
        }
        let free = self.0.disk_space.available(&self.0.root)?;
        let status =
            ModelAssetStatus::missing(&self.0.manifest, free, self.0.credentials.get()?.is_some());
        self.publish(status.clone());
        Ok(status)
    }

    async fn download_all(
        &self,
        operation_id: u64,
        cancellation: CancellationToken,
    ) -> Result<(), ModelAssetError> {
        let transferred = self.transferred_bytes();
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
            if self.final_is_valid(file).await? {
                continue;
            }
            self.download_file(file, operation_id, &cancellation)
                .await?;
        }
        self.write_ownership_marker().await?;
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
        let destination = self.0.root.join(&manifest_file.install_path);
        let partial = partial_path(&destination);
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|source| self.storage_error(parent, source))?;
        }
        let partial_size = tokio::fs::metadata(&partial)
            .await
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        // A partial beyond the immutable pinned size cannot be a valid prefix. Reset it
        // before constructing the Range request so relaunch cannot enter an append loop.
        let mut offset = if partial_size > manifest_file.size_bytes {
            tokio::fs::write(&partial, &[])
                .await
                .map_err(|source| self.storage_error(&partial, source))?;
            0
        } else {
            partial_size
        };
        if offset == manifest_file.size_bytes {
            let mut status = self.status();
            status.phase = ModelAssetPhase::Verifying;
            status.current_asset = Some(manifest_file.asset);
            status.current_file = Some(manifest_file.install_path.clone());
            self.publish(status);
            if sha256_file(&partial).await? == manifest_file.sha256 {
                tokio::fs::rename(&partial, &destination)
                    .await
                    .map_err(|source| self.storage_error(&destination, source))?;
                return Ok(());
            }
            tokio::fs::write(&partial, &[])
                .await
                .map_err(|source| self.storage_error(&partial, source))?;
            offset = 0;
        }
        let token = self.0.credentials.get()?;
        let url = format!("{}/{}", self.0.resolve_base_url, manifest_file.source_path);
        let mut request = self.0.client.get(url);
        if offset > 0 {
            request = request.header(header::RANGE, format!("bytes={offset}-"));
        }
        if let Some(token) = token.as_ref() {
            request = request.bearer_auth(token);
        }
        let response = request.send().await?;
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

        let mut output = OpenOptions::new()
            .create(true)
            .write(true)
            .append(offset > 0)
            .truncate(offset == 0)
            .open(&partial)
            .await
            .map_err(|source| self.storage_error(&partial, source))?;
        let mut stream = response.bytes_stream();
        let mut file_bytes = offset;
        let mut last_reported = self.status().transferred_bytes;
        while let Some(chunk) = tokio::select! {
            _ = cancellation.cancelled() => return Err(ModelAssetError::Cancelled),
            chunk = stream.next() => chunk,
        } {
            let chunk = chunk?;
            output
                .write_all(&chunk)
                .await
                .map_err(|source| self.storage_error(&partial, source))?;
            file_bytes = file_bytes.saturating_add(chunk.len() as u64);
            let total = self.transferred_bytes();
            if total.saturating_sub(last_reported) >= PROGRESS_CHUNK_BYTES
                || file_bytes == manifest_file.size_bytes
            {
                last_reported = total;
                self.publish_progress(manifest_file, operation_id, total);
            }
        }
        output
            .flush()
            .await
            .map_err(|source| self.storage_error(&partial, source))?;
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
        if sha256_file(&partial).await? != manifest_file.sha256 {
            tokio::fs::remove_file(&partial)
                .await
                .map_err(|source| self.storage_error(&partial, source))?;
            return Err(ModelAssetError::Integrity(
                manifest_file.source_path.clone(),
            ));
        }
        tokio::fs::rename(&partial, &destination)
            .await
            .map_err(|source| self.storage_error(&destination, source))?;
        Ok(())
    }

    async fn final_is_valid(&self, file: &ManifestFile) -> Result<bool, ModelAssetError> {
        let path = self.0.root.join(&file.install_path);
        let Ok(metadata) = tokio::fs::metadata(&path).await else {
            return Ok(false);
        };
        if metadata.len() == file.size_bytes && sha256_file(&path).await? == file.sha256 {
            return Ok(true);
        }
        tokio::fs::remove_file(&path)
            .await
            .map_err(|source| self.storage_error(&path, source))?;
        Ok(false)
    }

    fn finish_operation(&self, operation_id: u64, result: Result<(), ModelAssetError>) {
        let mut active = self.0.active.lock().expect("model asset download lock");
        if active.as_ref().map(|active| active.operation_id) == Some(operation_id) {
            *active = None;
        }
        drop(active);
        let Err(error) = result else { return };
        let mut status = self.status();
        status.operation_id = None;
        status.current_asset = None;
        status.current_file = None;
        status.transferred_bytes = self.transferred_bytes();
        status.remaining_bytes = status.total_bytes.saturating_sub(status.transferred_bytes);
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
        status.can_resume = status.transferred_bytes > 0;
        self.publish(status);
    }

    fn publish(&self, status: ModelAssetStatus) {
        self.0.status.send_replace(status);
    }

    fn transferred_bytes(&self) -> u64 {
        self.0
            .manifest
            .files
            .iter()
            .map(|file| {
                let final_path = self.0.root.join(&file.install_path);
                let partial_path = partial_path(&final_path);
                fs::metadata(&final_path)
                    .map(|metadata| metadata.len().min(file.size_bytes))
                    .or_else(|_| {
                        fs::metadata(partial_path)
                            .map(|metadata| metadata.len().min(file.size_bytes))
                    })
                    .unwrap_or(0)
            })
            .sum()
    }

    fn inspect_install(
        root: &Path,
        manifest: &AssetManifest,
        free_bytes: u64,
        authentication_configured: bool,
    ) -> Result<ModelAssetStatus, ModelAssetError> {
        let mut status = ModelAssetStatus::missing(manifest, free_bytes, authentication_configured);
        let marker_path = root.join(OWNERSHIP_FILE);
        if marker_path.exists() {
            let marker: OwnershipMarker =
                serde_json::from_slice(&fs::read(&marker_path).map_err(|source| {
                    ModelAssetError::Storage {
                        path: marker_path.clone(),
                        source,
                    }
                })?)
                .map_err(|_| ModelAssetError::NotOwned)?;
            if marker.repository == manifest.repository && marker.revision != manifest.revision {
                status.phase = ModelAssetPhase::RevisionMismatch;
                status.message = Some(ModelAssetError::RevisionMismatch.to_string());
                status.error_code = Some(ModelAssetErrorCode::RevisionMismatch);
                return Ok(status);
            }
            if marker.schema_version == 0
                && marker.matches_payload(manifest)
                && manifest.files.iter().all(|file| {
                    fs::metadata(root.join(&file.install_path))
                        .map(|metadata| metadata.len() == file.size_bytes)
                        .unwrap_or(false)
                })
            {
                let upgraded = OwnershipMarker::from_manifest(manifest);
                let temporary = root.join(format!("{OWNERSHIP_FILE}.tmp"));
                fs::write(
                    &temporary,
                    serde_json::to_vec_pretty(&upgraded).expect("marker serialization"),
                )
                .map_err(|source| ModelAssetError::Storage {
                    path: temporary.clone(),
                    source,
                })?;
                fs::rename(&temporary, &marker_path).map_err(|source| {
                    ModelAssetError::Storage {
                        path: marker_path.clone(),
                        source,
                    }
                })?;
                status.phase = ModelAssetPhase::Ready;
                status.transferred_bytes = status.total_bytes;
                status.remaining_bytes = 0;
                status.required_free_bytes = 0;
                return Ok(status);
            }
            if marker.matches(manifest)
                && manifest.files.iter().all(|file| {
                    fs::metadata(root.join(&file.install_path))
                        .map(|metadata| metadata.len() == file.size_bytes)
                        .unwrap_or(false)
                })
            {
                status.phase = ModelAssetPhase::Ready;
                status.transferred_bytes = status.total_bytes;
                status.remaining_bytes = 0;
                status.required_free_bytes = 0;
                return Ok(status);
            }
        }
        status.transferred_bytes = manifest
            .files
            .iter()
            .map(|file| {
                let final_path = root.join(&file.install_path);
                fs::metadata(&final_path)
                    .or_else(|_| fs::metadata(partial_path(&final_path)))
                    .map(|metadata| metadata.len().min(file.size_bytes))
                    .unwrap_or(0)
            })
            .sum();
        status.remaining_bytes = status.total_bytes.saturating_sub(status.transferred_bytes);
        status.required_free_bytes = status
            .remaining_bytes
            .saturating_add(DISK_SAFETY_RESERVE_BYTES);
        status.can_resume = status.transferred_bytes > 0;
        Ok(status)
    }

    async fn write_ownership_marker(&self) -> Result<(), ModelAssetError> {
        let marker = OwnershipMarker::from_manifest(&self.0.manifest);
        let path = self.0.root.join(OWNERSHIP_FILE);
        let temporary = self.0.root.join(format!("{OWNERSHIP_FILE}.tmp"));
        tokio::fs::write(
            &temporary,
            serde_json::to_vec_pretty(&marker).expect("marker serialization"),
        )
        .await
        .map_err(|source| self.storage_error(&temporary, source))?;
        tokio::fs::rename(&temporary, &path)
            .await
            .map_err(|source| self.storage_error(&path, source))
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

async fn sha256_file(path: &Path) -> Result<String, ModelAssetError> {
    let mut file = File::open(path)
        .await
        .map_err(|source| ModelAssetError::Storage {
            path: path.to_path_buf(),
            source,
        })?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|source| ModelAssetError::Storage {
                path: path.to_path_buf(),
                source,
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

async fn has_symlink_ancestor(root: &Path, relative: &Path) -> Result<bool, ModelAssetError> {
    let mut current = root.to_path_buf();
    let Some(parent) = relative.parent() else {
        return Ok(false);
    };
    for component in parent.components() {
        let Component::Normal(component) = component else {
            return Ok(true);
        };
        current.push(component);
        match tokio::fs::symlink_metadata(&current).await {
            Ok(metadata) if metadata.file_type().is_symlink() => return Ok(true),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(source) => {
                return Err(ModelAssetError::Storage {
                    path: current,
                    source,
                });
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use std::{
        convert::Infallible,
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
        if bearer(&headers) == Some("valid-token") {
            (StatusCode::OK, "{}")
        } else {
            state.mode.store(AUTH_REQUIRED, Ordering::Relaxed);
            (StatusCode::UNAUTHORIZED, "invalid token")
        }
    }

    async fn serve_blob(
        State(state): State<Arc<FixtureState>>,
        headers: HeaderMap,
    ) -> Response<Body> {
        state.requests.fetch_add(1, Ordering::Relaxed);
        let mode = state.mode.load(Ordering::Relaxed);
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
            mtp_block_size: 4,
            drafter_selection_note: "The block size 3 alternative is incompatible.".into(),
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
        let manifest = fixture_manifest(bytes);
        ModelAssetManager::new(
            temporary.path().join("models"),
            manifest.clone(),
            format!("{}/resolve/{}", fixture.base_url, manifest.revision),
            format!("{}/whoami", fixture.base_url),
            credentials,
            Arc::new(FixedDisk(AtomicU64::new(free_bytes))),
        )
        .expect("test manager")
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

    #[test]
    fn production_manifest_is_an_exact_complete_revision_pin() {
        let manifest = production_manifest().expect("valid embedded manifest");

        assert_eq!(manifest.repository, "orcarouter/Qwen3.8-27B-Uncensored-MLX");
        assert_eq!(
            manifest.revision,
            "0f88c40e9eff87740295f27654558fcb77e21ae5"
        );
        assert_eq!(manifest.files.len(), 19);
        assert_eq!(manifest.mtp_block_size, 4);
        assert_eq!(manifest.total_size_bytes(), 16_950_451_879);
        assert!(
            manifest
                .files
                .iter()
                .all(|file| file.sha256.len() == 64 && file.size_bytes > 0)
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
        tokio::fs::create_dir_all(destination.parent().expect("target directory"))
            .await
            .expect("create target directory");
        tokio::fs::write(partial_path(&destination), &bytes[..7])
            .await
            .expect("seed interrupted download");

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
        assert_eq!(fixture.state.ranged_requests.load(Ordering::Relaxed), 1);
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
        tokio::fs::create_dir_all(destination.parent().expect("target directory"))
            .await
            .expect("create target directory");
        tokio::fs::write(partial_path(&destination), b"unsafe-oversized-partial")
            .await
            .expect("oversized partial");

        manager.start_download().expect("recover oversized partial");
        wait_for_phase(&manager, ModelAssetPhase::Ready).await;

        assert_eq!(fixture.state.ranged_requests.load(Ordering::Relaxed), 0);
        assert_eq!(
            tokio::fs::read(destination).await.expect("installed bytes"),
            bytes
        );
    }

    #[tokio::test]
    async fn graphical_authentication_keeps_token_in_store_and_resumes_after_expiry() {
        let bytes = b"credential fixture".to_vec();
        let fixture = Fixture::start(bytes.clone()).await;
        fixture.mode(AUTH_REQUIRED);
        let temporary = TempDir::new().expect("temporary model root");
        let credentials = Arc::new(MemoryCredentials::default());
        let manager = test_manager(&temporary, &fixture, &bytes, credentials.clone(), u64::MAX);

        manager
            .start_download()
            .expect("start unauthenticated download");
        wait_for_phase(&manager, ModelAssetPhase::AuthenticationRequired).await;
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

        fixture.mode(DISCONNECT);
        manager.start_download().expect("resume into disconnect");
        let failed = wait_for_phase(&manager, ModelAssetPhase::Failed).await;
        assert!(failed.can_resume);

        fixture.mode(NORMAL);
        manager.start_download().expect("resume after disconnect");
        wait_for_phase(&manager, ModelAssetPhase::Ready).await;
        assert!(fixture.state.ranged_requests.load(Ordering::Relaxed) >= 2);
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

        let status = ModelAssetManager::inspect_install(&root, &manifest, u64::MAX, false)
            .expect("inspect mismatch");

        assert_eq!(status.phase, ModelAssetPhase::RevisionMismatch);
    }

    #[test]
    fn exact_legacy_ownership_marker_migrates_atomically_to_current_schema() {
        let bytes = b"legacy marker fixture";
        let manifest = fixture_manifest(bytes);
        let temporary = TempDir::new().expect("temporary model root");
        let root = temporary.path().join("models");
        fs::create_dir_all(root.join("target")).expect("target directory");
        fs::write(root.join("target/model.bin"), bytes).expect("model bytes");
        let mut marker = OwnershipMarker::from_manifest(&manifest);
        marker.schema_version = 0;
        fs::write(
            root.join(OWNERSHIP_FILE),
            serde_json::to_vec(&marker).expect("marker JSON"),
        )
        .expect("legacy marker write");

        let status = ModelAssetManager::inspect_install(&root, &manifest, u64::MAX, false)
            .expect("migrate exact marker");
        let migrated: OwnershipMarker =
            serde_json::from_slice(&fs::read(root.join(OWNERSHIP_FILE)).expect("migrated marker"))
                .expect("migrated JSON");

        assert_eq!(status.phase, ModelAssetPhase::Ready);
        assert_eq!(migrated.schema_version, 1);
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
