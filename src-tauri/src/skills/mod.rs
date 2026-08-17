use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{File, Metadata, OpenOptions};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;
use uuid::Uuid;

use crate::corpus::state_dir;
use crate::error::AppError;
use crate::github::auth::{KeychainSlot, SystemKeychain};
use crate::github::url::parse_github_url;
use crate::state::{AppState, AuthorizedMcpProject};
use crate::types::{
    InstalledSkill, SkillBatchResult, SkillDestinationPresence, SkillFileContent,
    SkillInstallRecord, SkillInstallState, SkillMutationPlan, SkillPackageFile, SkillPackageResult,
    SkillPlanPackage, SkillSource, SkillSourceKind, SkillSourceResult, SkillTrustFingerprint,
    SkillTrustedExecutable, SkillType, SkillValidationCode, SkillValidationError,
    SkillVersionSnapshot,
};
use crate::util::fs::{atomic_write, read_capped};

pub mod drafts;
pub(crate) mod install;
pub mod mcp;
pub mod organize;

pub const MAX_SKILL_FILES: usize = 512;
pub const MAX_SKILL_FILE_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_SKILL_TOTAL_BYTES: u64 = 32 * 1024 * 1024;
const MAX_SKILL_GROUP_DEPTH: usize = 4;
const MAX_SKILL_TAGS: usize = 12;
const MAX_SKILL_DEPENDENCIES: usize = 32;
const MAX_SKILL_TAXONOMY_SEGMENT_BYTES: usize = 32;
const SKILL_TRUST_KEY_ACCOUNT: &str = "skill-trust-hmac-v1";
const MAX_SKILL_HISTORY_ENTRIES: usize = 10;
const MAX_SKILL_HISTORY_SCAN_ENTRIES: usize = 64;
const MAX_SKILL_HISTORY_MANIFEST_BYTES: u64 = 64 * 1024;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct SkillTrustRecord {
    source_id: String,
    relative_path: String,
    tree_hash: String,
    executables: Vec<SkillTrustedExecutable>,
    granted_at: String,
    signature: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UnsignedSkillTrustRecord<'a> {
    source_id: &'a str,
    relative_path: &'a str,
    tree_hash: &'a str,
    executables: &'a [SkillTrustedExecutable],
    granted_at: &'a str,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SkillInstallOperation {
    previous: Option<SkillInstallRecord>,
    next: SkillInstallRecord,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SkillMoveOperation {
    previous: SkillInstallRecord,
    next: SkillInstallRecord,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SkillUninstallOperation {
    previous: SkillInstallRecord,
    target_hash: String,
    quarantine: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct SkillVersionIdentity {
    source_id: String,
    relative_path: String,
    runtime: String,
    project_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SkillVersionManifest {
    version: u32,
    identity: SkillVersionIdentity,
    content_hash: String,
    created_at: String,
}

#[cfg(test)]
type RefreshFsProbe = Vec<(&'static str, std::thread::ThreadId)>;

#[cfg(test)]
fn refresh_fs_probe() -> &'static std::sync::Mutex<RefreshFsProbe> {
    static PROBE: std::sync::OnceLock<std::sync::Mutex<RefreshFsProbe>> =
        std::sync::OnceLock::new();
    PROBE.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

#[cfg(test)]
fn record_refresh_fs(event: &'static str) {
    refresh_fs_probe()
        .lock()
        .expect("refresh filesystem probe")
        .push((event, std::thread::current().id()));
}

#[cfg(not(test))]
fn record_refresh_fs(_: &'static str) {}

#[cfg(test)]
type UninstallMissingProbe = Box<dyn FnOnce(&Path) + Send>;

#[cfg(test)]
fn uninstall_missing_probes() -> &'static std::sync::Mutex<HashMap<PathBuf, UninstallMissingProbe>>
{
    static PROBES: std::sync::OnceLock<std::sync::Mutex<HashMap<PathBuf, UninstallMissingProbe>>> =
        std::sync::OnceLock::new();
    PROBES.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

#[cfg(test)]
fn set_uninstall_missing_probe(target: PathBuf, probe: UninstallMissingProbe) {
    uninstall_missing_probes()
        .lock()
        .expect("uninstall missing probes")
        .insert(target, probe);
}

#[cfg(test)]
fn after_missing_uninstall_validation(target: &Path) {
    if let Some(probe) = uninstall_missing_probes()
        .lock()
        .expect("uninstall missing probes")
        .remove(target)
    {
        probe(target);
    }
}

#[cfg(not(test))]
fn after_missing_uninstall_validation(_: &Path) {}

#[cfg(test)]
type UninstallBeforeQuarantineProbe = Box<dyn FnOnce(&Path) + Send>;

#[cfg(test)]
fn uninstall_before_quarantine_probes(
) -> &'static std::sync::Mutex<HashMap<PathBuf, UninstallBeforeQuarantineProbe>> {
    static PROBES: std::sync::OnceLock<
        std::sync::Mutex<HashMap<PathBuf, UninstallBeforeQuarantineProbe>>,
    > = std::sync::OnceLock::new();
    PROBES.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

#[cfg(test)]
fn set_uninstall_before_quarantine_probe(target: PathBuf, probe: UninstallBeforeQuarantineProbe) {
    uninstall_before_quarantine_probes()
        .lock()
        .expect("uninstall before quarantine probes")
        .insert(target, probe);
}

#[cfg(test)]
fn before_uninstall_quarantine(target: &Path) {
    if let Some(probe) = uninstall_before_quarantine_probes()
        .lock()
        .expect("uninstall before quarantine probes")
        .remove(target)
    {
        probe(target);
    }
}

#[cfg(not(test))]
fn before_uninstall_quarantine(_: &Path) {}

#[cfg(test)]
fn reset_refresh_fs_probe() {
    refresh_fs_probe()
        .lock()
        .expect("refresh filesystem probe")
        .clear();
}

#[cfg(test)]
fn take_refresh_fs_probe() -> RefreshFsProbe {
    std::mem::take(&mut *refresh_fs_probe().lock().expect("refresh filesystem probe"))
}

pub(crate) fn skill_sources_path(app_data_dir: &Path) -> PathBuf {
    state_dir(app_data_dir).join("skill-sources.json")
}

fn skill_trust_path(app_data_dir: &Path) -> PathBuf {
    state_dir(app_data_dir).join("skill-trust.json")
}

async fn load_skill_trust(app_data_dir: &Path) -> Result<Vec<SkillTrustRecord>, AppError> {
    let path = skill_trust_path(app_data_dir);
    match tokio::fs::read(&path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| AppError::JsonParse {
            command: "skill_trust_list".into(),
            message: error.to_string(),
            raw_excerpt: String::from_utf8_lossy(&bytes[..bytes.len().min(256)]).into_owned(),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(AppError::Io {
            message: format!("read {}: {error}", path.display()),
        }),
    }
}

async fn save_skill_trust(
    app_data_dir: &Path,
    records: &[SkillTrustRecord],
) -> Result<(), AppError> {
    let directory = state_dir(app_data_dir);
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| AppError::Io {
            message: format!("create state dir {}: {error}", directory.display()),
        })?;
    let bytes = serde_json::to_vec_pretty(records).map_err(|error| AppError::Internal {
        message: format!("serialize skill-trust.json: {error}"),
    })?;
    atomic_write(&skill_trust_path(app_data_dir), &bytes).await
}

fn validate_skill_trust(records: &[SkillTrustRecord]) -> Result<(), AppError> {
    let mut identities = HashSet::new();
    if records.iter().any(|record| {
        record.source_id.is_empty()
            || record.relative_path.is_empty()
            || record.tree_hash.len() != 64
            || record.signature.len() != 64
            || !identities.insert((&record.source_id, &record.relative_path))
    }) {
        return Err(AppError::InvalidArgument {
            message: "persisted skill trust records are invalid".into(),
        });
    }
    Ok(())
}

fn skill_trust_spec() -> crate::state_db::DocumentSpec<Vec<SkillTrustRecord>> {
    crate::state_db::DocumentSpec::new("skill_trust", 1, 1_048_576, |records| {
        validate_skill_trust(records)
    })
}

pub(crate) fn skill_trust_import_spec() -> crate::state_db::ImportSpec {
    crate::state_db::ImportSpec::new("skill_trust", "[]", parse_skill_trust_import)
}

fn validate_imported_skill_trust(
    records: &[SkillTrustRecord],
    keychain: &dyn KeychainSlot,
) -> Result<(), AppError> {
    validate_skill_trust(records).map_err(|_| AppError::StorageCorrupt {
        message: "skill trust legacy state is invalid".into(),
    })?;
    if records.is_empty() {
        return Ok(());
    }
    let key = read_trust_key_with(keychain)
        .map_err(|_| AppError::StorageCorrupt {
            message: "skill trust key could not be verified".into(),
        })?
        .ok_or_else(|| AppError::StorageCorrupt {
            message: "skill trust key is missing".into(),
        })?;
    if records
        .iter()
        .any(|record| !verify_trust_record(record, &key))
    {
        return Err(AppError::StorageCorrupt {
            message: "skill trust signature verification failed".into(),
        });
    }
    Ok(())
}

fn parse_skill_trust_import(raw: &[u8]) -> Result<String, AppError> {
    let records = serde_json::from_slice::<Vec<SkillTrustRecord>>(raw).map_err(|_| {
        AppError::StorageCorrupt {
            message: "skill trust legacy state is malformed".into(),
        }
    })?;
    validate_imported_skill_trust(&records, &SystemKeychain)?;
    serde_json::to_string(&records).map_err(|_| AppError::Internal {
        message: "serialize skill trust migration state".into(),
    })
}

async fn load_skill_trust_for_state(state: &AppState) -> Result<Vec<SkillTrustRecord>, AppError> {
    let records = if let Some(database) = state.completed_state_database().await? {
        database
            .read(skill_trust_spec())
            .await?
            .ok_or_else(|| AppError::StorageCorrupt {
                message: "skill trust is missing after SQLite migration".into(),
            })?
    } else {
        load_skill_trust(&state.app_data_dir).await?
    };
    validate_skill_trust(&records)?;
    if records.is_empty() {
        return Ok(records);
    }
    let key = tokio::task::spawn_blocking(|| read_trust_key_with(&SystemKeychain))
        .await
        .map_err(|error| AppError::Internal {
            message: format!("skill trust key task failed: {error}"),
        })??
        .ok_or_else(|| AppError::KeychainUnavailable {
            message: "skill trust key is missing".into(),
        })?;
    if records
        .iter()
        .any(|record| !verify_trust_record(record, &key))
    {
        return Err(AppError::StorageCorrupt {
            message: "skill trust signature verification failed".into(),
        });
    }
    Ok(records)
}

async fn mutate_skill_trust<R>(
    state: &AppState,
    mutation: impl FnOnce(&mut Vec<SkillTrustRecord>) -> Result<R, AppError> + Send + 'static,
) -> Result<R, AppError>
where
    R: Send + 'static,
{
    if let Some(database) = state.completed_state_database().await? {
        return database
            .mutate(skill_trust_spec(), Vec::new(), mutation)
            .await;
    }
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_skill_installs_async(state.app_data_dir.clone()).await?;
    let mut records = load_skill_trust(&state.app_data_dir).await?;
    let result = mutation(&mut records)?;
    validate_skill_trust(&records)?;
    save_skill_trust(&state.app_data_dir, &records).await?;
    Ok(result)
}

fn decode_trust_key(value: &str) -> Result<Vec<u8>, AppError> {
    let key = base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|error| AppError::KeychainUnavailable {
            message: format!("decode {SKILL_TRUST_KEY_ACCOUNT}: {error}"),
        })?;
    if key.len() != 32 {
        return Err(AppError::KeychainUnavailable {
            message: format!("{SKILL_TRUST_KEY_ACCOUNT} has an invalid length"),
        });
    }
    Ok(key)
}

fn read_trust_key_with(keychain: &dyn KeychainSlot) -> Result<Option<Vec<u8>>, AppError> {
    keychain
        .read(SKILL_TRUST_KEY_ACCOUNT)?
        .map(|value| decode_trust_key(&value))
        .transpose()
}

fn load_or_create_trust_key_with(
    keychain: &dyn KeychainSlot,
    has_existing_records: bool,
) -> Result<Vec<u8>, AppError> {
    if let Some(key) = read_trust_key_with(keychain)? {
        return Ok(key);
    }
    if has_existing_records {
        return Err(AppError::KeychainUnavailable {
            message: "skill trust key is missing; revoke existing trust records before granting new trust"
                .into(),
        });
    }
    let mut key = [0_u8; 32];
    getrandom::fill(&mut key).map_err(|error| AppError::KeychainUnavailable {
        message: format!("generate {SKILL_TRUST_KEY_ACCOUNT}: {error}"),
    })?;
    keychain.write(
        SKILL_TRUST_KEY_ACCOUNT,
        &base64::engine::general_purpose::STANDARD.encode(key),
    )?;
    Ok(key.to_vec())
}

fn trust_payload(record: &SkillTrustRecord) -> Result<Vec<u8>, AppError> {
    serde_json::to_vec(&UnsignedSkillTrustRecord {
        source_id: &record.source_id,
        relative_path: &record.relative_path,
        tree_hash: &record.tree_hash,
        executables: &record.executables,
        granted_at: &record.granted_at,
    })
    .map_err(|error| AppError::Internal {
        message: format!("serialize skill trust payload: {error}"),
    })
}

fn sign_trust_record(record: &SkillTrustRecord, key: &[u8]) -> Result<String, AppError> {
    let mut mac = HmacSha256::new_from_slice(key).map_err(|error| AppError::Internal {
        message: format!("initialize skill trust HMAC: {error}"),
    })?;
    mac.update(&trust_payload(record)?);
    Ok(hex::encode(mac.finalize().into_bytes()))
}

fn verify_trust_record(record: &SkillTrustRecord, key: &[u8]) -> bool {
    let Ok(signature) = hex::decode(&record.signature) else {
        return false;
    };
    let Ok(payload) = trust_payload(record) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(key) else {
        return false;
    };
    mac.update(&payload);
    mac.verify_slice(&signature).is_ok()
}

fn lock_skill_sources(app_data_dir: &Path) -> Result<File, AppError> {
    let directory = state_dir(app_data_dir);
    std::fs::create_dir_all(&directory).map_err(|error| AppError::Io {
        message: format!("create state dir {}: {error}", directory.display()),
    })?;
    let path = directory.join("skill-sources.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| AppError::Io {
            message: format!("open skill source lock {}: {error}", path.display()),
        })?;
    file.lock().map_err(|error| AppError::Io {
        message: format!("lock skill source state {}: {error}", path.display()),
    })?;
    Ok(file)
}

fn lock_skill_installs(app_data_dir: &Path) -> Result<File, AppError> {
    let directory = state_dir(app_data_dir);
    std::fs::create_dir_all(&directory).map_err(|error| AppError::Io {
        message: format!("create state dir {}: {error}", directory.display()),
    })?;
    let path = directory.join("skill-installs.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| AppError::Io {
            message: format!("open skill install lock {}: {error}", path.display()),
        })?;
    file.lock().map_err(|error| AppError::Io {
        message: format!("lock skill install state {}: {error}", path.display()),
    })?;
    Ok(file)
}

async fn lock_skill_sources_async(app_data_dir: PathBuf) -> Result<File, AppError> {
    tokio::task::spawn_blocking(move || lock_skill_sources(&app_data_dir))
        .await
        .map_err(|error| AppError::Internal {
            message: format!("skill source lock task failed: {error}"),
        })?
}

async fn lock_skill_installs_async(app_data_dir: PathBuf) -> Result<File, AppError> {
    tokio::task::spawn_blocking(move || lock_skill_installs(&app_data_dir))
        .await
        .map_err(|error| AppError::Internal {
            message: format!("skill install lock task failed: {error}"),
        })?
}

pub(crate) async fn load_skill_sources(app_data_dir: &Path) -> Result<Vec<SkillSource>, AppError> {
    let path = skill_sources_path(app_data_dir);
    match tokio::fs::read(&path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| AppError::JsonParse {
            command: "skill_sources_list".into(),
            message: error.to_string(),
            raw_excerpt: String::from_utf8_lossy(&bytes[..bytes.len().min(256)]).into_owned(),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(AppError::Io {
            message: format!("read {}: {error}", path.display()),
        }),
    }
}

fn validate_skill_sources(sources: &[SkillSource]) -> Result<(), AppError> {
    let mut ids = HashSet::new();
    for source in sources {
        if source.id.is_empty() || source.id.len() > 128 || !ids.insert(source.id.as_str()) {
            return Err(AppError::InvalidArgument {
                message: "skill source ids must be non-empty, unique, and at most 128 bytes".into(),
            });
        }
        match &source.kind {
            SkillSourceKind::Local { root } => {
                if root.len() > 4096 || !Path::new(root).is_absolute() {
                    return Err(AppError::InvalidArgument {
                        message: "local skill source root must be an absolute path".into(),
                    });
                }
            }
            SkillSourceKind::Github {
                repository,
                git_ref,
                subdirectory,
                active_checkout,
            } => {
                if canonical_github_repository(repository)? != *repository
                    || validated_git_ref(git_ref.as_deref())? != *git_ref
                    || validated_subdirectory(subdirectory.as_deref())? != *subdirectory
                    || active_checkout
                        .as_ref()
                        .is_some_and(|path| path.len() > 4096 || !Path::new(path).is_absolute())
                {
                    return Err(AppError::InvalidArgument {
                        message: "persisted GitHub skill source is invalid".into(),
                    });
                }
            }
        }
    }
    Ok(())
}

fn skill_sources_spec() -> crate::state_db::DocumentSpec<Vec<SkillSource>> {
    crate::state_db::DocumentSpec::new("skill_sources", 1, 1_048_576, |sources| {
        validate_skill_sources(sources)
    })
}

pub(crate) fn skill_sources_import_spec() -> crate::state_db::ImportSpec {
    crate::state_db::ImportSpec::document(skill_sources_spec(), Vec::new())
}

pub(crate) async fn load_skill_sources_for_state(
    state: &AppState,
) -> Result<Vec<SkillSource>, AppError> {
    if let Some(database) = state.completed_state_database().await? {
        return database.read(skill_sources_spec()).await?.ok_or_else(|| {
            AppError::StorageCorrupt {
                message: "skill sources are missing after SQLite migration".into(),
            }
        });
    }
    load_skill_sources(&state.app_data_dir).await
}

async fn mutate_skill_sources<R>(
    state: &AppState,
    mutation: impl FnOnce(&mut Vec<SkillSource>) -> Result<R, AppError> + Send + 'static,
) -> Result<R, AppError>
where
    R: Send + 'static,
{
    if let Some(database) = state.completed_state_database().await? {
        return database
            .mutate(skill_sources_spec(), Vec::new(), mutation)
            .await;
    }
    let _guard = state.skill_sources_write_lock.lock().await;
    let _file_guard = lock_skill_sources_async(state.app_data_dir.clone()).await?;
    let mut sources = load_skill_sources(&state.app_data_dir).await?;
    let result = mutation(&mut sources)?;
    validate_skill_sources(&sources)?;
    save_skill_sources(&state.app_data_dir, &sources).await?;
    Ok(result)
}

async fn save_skill_sources(app_data_dir: &Path, sources: &[SkillSource]) -> Result<(), AppError> {
    let directory = state_dir(app_data_dir);
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| AppError::Io {
            message: format!("create state dir {}: {error}", directory.display()),
        })?;
    let bytes = serde_json::to_vec_pretty(sources).map_err(|error| AppError::Internal {
        message: format!("serialize skill-sources.json: {error}"),
    })?;
    atomic_write(&skill_sources_path(app_data_dir), &bytes).await
}

pub(crate) async fn add_local_source(
    state: &AppState,
    root: &Path,
) -> Result<SkillSource, AppError> {
    ensure_local_source(state, root)
        .await
        .map(|(source, _)| source)
}

pub(crate) async fn ensure_local_source(
    state: &AppState,
    root: &Path,
) -> Result<(SkillSource, bool), AppError> {
    if !root.is_absolute() {
        return Err(AppError::InvalidArgument {
            message: "local skill source root must be absolute".into(),
        });
    }
    let metadata = std::fs::symlink_metadata(root).map_err(|_| AppError::InvalidArgument {
        message: format!(
            "local skill source root must be an existing directory: {}",
            root.display()
        ),
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AppError::InvalidArgument {
            message: format!(
                "local skill source root must be a real directory: {}",
                root.display()
            ),
        });
    }
    let canonical_root =
        std::fs::canonicalize(root).map_err(|error| AppError::InvalidArgument {
            message: format!(
                "could not resolve local skill source root {}: {error}",
                root.display()
            ),
        })?;
    let root_string = canonical_root.to_string_lossy().into_owned();

    mutate_skill_sources(state, move |sources| {
        if let Some(existing) = sources.iter().find(
            |source| matches!(&source.kind, SkillSourceKind::Local { root } if root == &root_string),
        ) {
            return Ok((existing.clone(), false));
        }
        let source = SkillSource {
            id: Uuid::new_v4().to_string(),
            kind: SkillSourceKind::Local { root: root_string },
        };
        sources.push(source.clone());
        Ok((source, true))
    })
    .await
}

pub(crate) async fn remove_skill_source(
    state: &AppState,
    source_id: &str,
) -> Result<bool, AppError> {
    let source_id = source_id.to_owned();
    mutate_skill_sources(state, move |sources| {
        let original_len = sources.len();
        sources.retain(|source| source.id != source_id);
        Ok(sources.len() != original_len)
    })
    .await
}

pub(crate) fn canonical_github_repository(repository: &str) -> Result<String, AppError> {
    let trimmed = repository.trim();
    let authority = trimmed
        .split_once("://")
        .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or_default())
        .unwrap_or_default();
    if authority.contains('@') {
        return Err(AppError::InvalidArgument {
            message: "GitHub repository URL must not contain credentials".into(),
        });
    }
    let repo = parse_github_url(trimmed).ok_or_else(|| AppError::InvalidArgument {
        message: "repository must be a valid github.com repository URL".into(),
    })?;
    Ok(format!(
        "https://github.com/{}/{}.git",
        repo.owner, repo.repo
    ))
}

pub(crate) fn validated_git_ref(git_ref: Option<&str>) -> Result<Option<String>, AppError> {
    let Some(value) = git_ref else {
        return Ok(None);
    };
    if value.is_empty()
        || value.starts_with('-')
        || value.ends_with(['.', '/'])
        || value.contains([
            ' ', '\t', '\n', '\r', '\\', '~', '^', ':', '?', '*', '[', '\0',
        ])
        || value.contains("..")
        || value.contains("//")
        || value.contains("@{")
        || value.split('/').any(|part| part.ends_with(".lock"))
    {
        return Err(AppError::InvalidArgument {
            message: "Git ref is empty, option-like, or not a normalized ref name".into(),
        });
    }
    Ok(Some(value.to_string()))
}

pub(crate) fn validated_subdirectory(
    subdirectory: Option<&str>,
) -> Result<Option<String>, AppError> {
    let Some(value) = subdirectory else {
        return Ok(None);
    };
    let path = Path::new(value);
    let parts = path
        .components()
        .map(|component| match component {
            Component::Normal(part) => part.to_str().ok_or_else(|| AppError::InvalidArgument {
                message: "GitHub source subdirectory must be valid UTF-8".into(),
            }),
            _ => Err(AppError::InvalidArgument {
                message: "GitHub source subdirectory must be normalized and relative".into(),
            }),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let normalized = parts.join("/");
    if normalized.is_empty() || normalized != value || value.contains('\\') {
        return Err(AppError::InvalidArgument {
            message: "GitHub source subdirectory must be normalized and relative".into(),
        });
    }
    Ok(Some(normalized))
}

pub(crate) async fn add_github_source(
    state: &AppState,
    repository: &str,
    git_ref: Option<&str>,
    subdirectory: Option<&str>,
) -> Result<SkillSource, AppError> {
    let repository = canonical_github_repository(repository)?;
    let git_ref = validated_git_ref(git_ref)?;
    let subdirectory = validated_subdirectory(subdirectory)?;

    mutate_skill_sources(state, move |sources| {
        if let Some(existing) = sources.iter().find(|source| {
            matches!(
                &source.kind,
                SkillSourceKind::Github {
                    repository: existing_repository,
                    git_ref: existing_ref,
                    subdirectory: existing_subdirectory,
                    ..
                } if existing_repository == &repository
                    && existing_ref == &git_ref
                    && existing_subdirectory == &subdirectory
            )
        }) {
            return Ok(existing.clone());
        }
        let source = SkillSource {
            id: Uuid::new_v4().to_string(),
            kind: SkillSourceKind::Github {
                repository,
                git_ref,
                subdirectory,
                active_checkout: None,
            },
        };
        sources.push(source.clone());
        Ok(source)
    })
    .await
}

async fn refresh_fs<T, F>(event: &'static str, operation: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        record_refresh_fs(event);
        operation()
    })
    .await
    .map_err(|error| AppError::Internal {
        message: format!("skill source refresh filesystem task failed: {error}"),
    })?
}

async fn cleanup_unreferenced(path: PathBuf) {
    let _ = refresh_fs("failed_stage_cleanup", move || {
        record_refresh_fs("recursive_cleanup");
        match std::fs::remove_dir_all(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(AppError::Io {
                message: format!("remove unreferenced checkout {}: {error}", path.display()),
            }),
        }
    })
    .await;
}

pub(crate) async fn refresh_git_source(
    state: &AppState,
    source_id: &str,
) -> Result<SkillSourceResult, AppError> {
    let source = load_skill_sources_for_state(state)
        .await?
        .into_iter()
        .find(|source| source.id == source_id)
        .ok_or_else(|| AppError::InvalidArgument {
            message: format!("unknown skill source id: {source_id}"),
        })?;
    let repository = match source.kind {
        SkillSourceKind::Github { repository, .. } => repository,
        SkillSourceKind::Local { .. } => {
            return Err(AppError::InvalidArgument {
                message: "local skill sources do not use Git refresh".into(),
            });
        }
    };
    refresh_git_source_from(state, source_id, &repository).await
}

async fn refresh_git_source_from(
    state: &AppState,
    source_id: &str,
    clone_source: &str,
) -> Result<SkillSourceResult, AppError> {
    state.require_network("skill_source_refresh").await?;
    let sources = load_skill_sources_for_state(state).await?;
    let source_index = sources
        .iter()
        .position(|source| source.id == source_id)
        .ok_or_else(|| AppError::InvalidArgument {
            message: format!("unknown skill source id: {source_id}"),
        })?;
    let (git_ref, subdirectory) = match &sources[source_index].kind {
        SkillSourceKind::Github {
            git_ref,
            subdirectory,
            ..
        } => (git_ref.clone(), subdirectory.clone()),
        SkillSourceKind::Local { .. } => {
            return Err(AppError::InvalidArgument {
                message: "local skill sources do not use Git refresh".into(),
            });
        }
    };

    let managed_root = state.app_data_dir.join("skills").join("sources");
    let staging = managed_root.join(format!(".staging-{}", Uuid::new_v4()));
    let staging_for_create = staging.clone();
    refresh_fs("staging_create", move || {
        std::fs::create_dir_all(&managed_root).map_err(|error| AppError::Io {
            message: format!(
                "create managed skill source directory {}: {error}",
                managed_root.display()
            ),
        })?;
        std::fs::create_dir(&staging_for_create).map_err(|error| AppError::Io {
            message: format!(
                "create skill source staging directory {}: {error}",
                staging_for_create.display()
            ),
        })
    })
    .await?;

    let staging_arg = staging.to_string_lossy().into_owned();
    if let Err(error) = crate::corpus::run_git(
        &["clone", "--no-checkout", "--", clone_source, &staging_arg],
        None,
    )
    .await
    {
        cleanup_unreferenced(staging).await;
        return Err(error);
    }
    let checkout_ref = git_ref.as_deref().unwrap_or("HEAD");
    if let Err(error) = crate::corpus::run_git(
        &["checkout", "--detach", checkout_ref, "--"],
        Some(&staging),
    )
    .await
    {
        cleanup_unreferenced(staging).await;
        return Err(error);
    }

    let candidate_source = sources[source_index].clone();
    let staging_for_validation = staging.clone();
    let subdirectory_for_validation = subdirectory.clone();
    let candidate = match refresh_fs("canonicalize", move || {
        let checkout_root =
            std::fs::canonicalize(&staging_for_validation).map_err(|error| AppError::Io {
                message: format!(
                    "resolve staged checkout {}: {error}",
                    staging_for_validation.display()
                ),
            })?;
        let selected = subdirectory_for_validation
            .as_deref()
            .map(|subdirectory| checkout_root.join(subdirectory))
            .unwrap_or_else(|| checkout_root.clone());
        let metadata = std::fs::symlink_metadata(&selected).map_err(|error| AppError::Io {
            message: format!(
                "inspect selected skill source {}: {error}",
                selected.display()
            ),
        })?;
        if metadata.file_type().is_symlink()
            || metadata_is_reparse_point(&metadata)
            || !metadata.is_dir()
        {
            return Err(AppError::InvalidArgument {
                message: "GitHub source subdirectory must be a real directory".into(),
            });
        }
        let selected = std::fs::canonicalize(&selected).map_err(|error| AppError::Io {
            message: format!(
                "resolve selected skill source {}: {error}",
                selected.display()
            ),
        })?;
        if !selected.starts_with(&checkout_root) {
            return Err(AppError::InvalidArgument {
                message: "GitHub source subdirectory resolves outside the staged checkout".into(),
            });
        }
        let mut staged_source = candidate_source;
        if let SkillSourceKind::Github {
            active_checkout, ..
        } = &mut staged_source.kind
        {
            *active_checkout = Some(selected.to_string_lossy().into_owned());
        }
        discover_source_blocking(staged_source)
    })
    .await
    {
        Ok(candidate) => candidate,
        Err(error) => {
            cleanup_unreferenced(staging).await;
            return Err(error);
        }
    };

    let generation_id = Uuid::new_v4().to_string();
    let source_directory = state.app_data_dir.join("skills/sources").join(source_id);
    let generation = source_directory.join(&generation_id);
    let staging_for_rename = staging.clone();
    let generation_for_rename = generation.clone();
    if let Err(error) = refresh_fs("activation_rename", move || {
        std::fs::create_dir_all(&source_directory).map_err(|error| AppError::Io {
            message: format!(
                "create managed source directory {}: {error}",
                source_directory.display()
            ),
        })?;
        std::fs::rename(&staging_for_rename, &generation_for_rename).map_err(|error| AppError::Io {
            message: format!(
                "activate staged checkout {} -> {}: {error}",
                staging_for_rename.display(),
                generation_for_rename.display()
            ),
        })
    })
    .await
    {
        cleanup_unreferenced(staging).await;
        return Err(error);
    }

    let active_checkout = subdirectory
        .as_deref()
        .map(|subdirectory| generation.join(subdirectory))
        .unwrap_or_else(|| generation.clone());
    let mut active_source = candidate.source;
    if let SkillSourceKind::Github {
        active_checkout: active,
        ..
    } = &mut active_source.kind
    {
        *active = Some(active_checkout.to_string_lossy().into_owned());
    }
    refresh_fs("state_persist", || Ok(())).await?;
    let source_id = source_id.to_owned();
    let persisted_source = active_source.clone();
    if let Err(error) = mutate_skill_sources(state, move |sources| {
        let source = sources
            .iter_mut()
            .find(|source| source.id == source_id)
            .ok_or_else(|| AppError::InvalidArgument {
                message: format!("unknown skill source id: {source_id}"),
            })?;
        *source = persisted_source;
        Ok(())
    })
    .await
    {
        cleanup_unreferenced(generation).await;
        return Err(error);
    }

    Ok(SkillSourceResult {
        source: active_source,
        packages: candidate.packages,
        errors: candidate.errors,
    })
}

pub(crate) async fn discover_source(source: SkillSource) -> Result<SkillSourceResult, AppError> {
    tokio::task::spawn_blocking(move || discover_source_blocking(source))
        .await
        .map_err(|error| AppError::Internal {
            message: format!("skill source discovery task failed: {error}"),
        })?
}

async fn persisted_trust_key() -> Option<Vec<u8>> {
    tokio::task::spawn_blocking(|| read_trust_key_with(&SystemKeychain))
        .await
        .ok()
        .and_then(Result::ok)
        .flatten()
}

async fn apply_persisted_trust(
    state: &AppState,
    result: &mut SkillSourceResult,
) -> Result<(), AppError> {
    let Ok(records) = load_skill_trust_for_state(state).await else {
        return Ok(());
    };
    if records.is_empty() {
        return Ok(());
    }
    let key = persisted_trust_key().await;
    let source_root = canonical_skill_source_root(&result.source)?;
    apply_skill_trust(&source_root, result, &records, key.as_deref());
    Ok(())
}

pub(crate) async fn inspect_skill_sources(
    state: &AppState,
) -> Result<Vec<SkillSourceResult>, AppError> {
    let sources = load_skill_sources_for_state(state).await?;
    let mut results = Vec::with_capacity(sources.len());
    for source in sources {
        match discover_source(source.clone()).await {
            Ok(mut result) => {
                apply_persisted_trust(state, &mut result).await?;
                results.push(result);
            }
            Err(error) => results.push(SkillSourceResult {
                source,
                packages: Vec::new(),
                errors: vec![SkillValidationError {
                    code: SkillValidationCode::Io,
                    path: ".".into(),
                    message: error.to_string(),
                }],
            }),
        }
    }
    Ok(results)
}

pub(crate) async fn refresh_skill_source(
    state: &AppState,
    source_id: &str,
) -> Result<SkillSourceResult, AppError> {
    let source = load_skill_sources_for_state(state)
        .await?
        .into_iter()
        .find(|source| source.id == source_id)
        .ok_or_else(|| AppError::InvalidArgument {
            message: format!("unknown skill source id: {source_id}"),
        })?;
    let mut result = match source.kind {
        SkillSourceKind::Local { .. } => discover_source(source).await,
        SkillSourceKind::Github { .. } => refresh_git_source(state, source_id).await,
    }?;
    apply_persisted_trust(state, &mut result).await?;
    Ok(result)
}

pub(crate) async fn refresh_all_skill_sources(
    state: &AppState,
) -> Result<Vec<SkillSourceResult>, AppError> {
    let sources = load_skill_sources_for_state(state).await?;
    let mut results = Vec::with_capacity(sources.len());
    for source in sources {
        match refresh_skill_source(state, &source.id).await {
            Ok(result) => results.push(result),
            Err(error) => results.push(SkillSourceResult {
                source,
                packages: Vec::new(),
                errors: vec![SkillValidationError {
                    code: SkillValidationCode::Io,
                    path: ".".into(),
                    message: error.to_string(),
                }],
            }),
        }
    }
    Ok(results)
}

fn destination(
    root: &Path,
    runtime: &str,
    scope: &str,
    project_path: Option<String>,
    name: &str,
) -> SkillDestinationPresence {
    let path = root.join(if runtime == "claudeCode" {
        ".claude/skills"
    } else {
        ".agents/skills"
    });
    let path = path.join(name);
    SkillDestinationPresence {
        runtime: runtime.into(),
        scope: scope.into(),
        project_path,
        present: std::fs::symlink_metadata(&path).is_ok(),
        path: path.to_string_lossy().into_owned(),
    }
}

pub(crate) fn skill_destination_presence(
    home: &Path,
    project_paths: &[String],
    name: &str,
) -> Vec<SkillDestinationPresence> {
    let mut destinations = vec![
        destination(home, "claudeCode", "user", None, name),
        destination(home, "codex", "user", None, name),
    ];
    for project_path in project_paths {
        let root = Path::new(project_path);
        destinations.push(destination(
            root,
            "claudeCode",
            "project",
            Some(project_path.clone()),
            name,
        ));
        destinations.push(destination(
            root,
            "codex",
            "project",
            Some(project_path.clone()),
            name,
        ));
    }
    destinations
}

fn discover_source_blocking(source: SkillSource) -> Result<SkillSourceResult, AppError> {
    let root = match &source.kind {
        SkillSourceKind::Local { root } => PathBuf::from(root),
        SkillSourceKind::Github {
            active_checkout: Some(root),
            ..
        } => PathBuf::from(root),
        SkillSourceKind::Github { .. } => {
            return Err(AppError::InvalidArgument {
                message: "GitHub source has no active checkout".into(),
            });
        }
    };
    let root_metadata =
        std::fs::symlink_metadata(&root).map_err(|error| AppError::InvalidArgument {
            message: format!(
                "could not inspect skill source root {}: {error}",
                root.display()
            ),
        })?;
    if root_metadata.file_type().is_symlink() || metadata_is_reparse_point(&root_metadata) {
        return Ok(SkillSourceResult {
            source,
            packages: Vec::new(),
            errors: vec![unsafe_entry_error(
                ".".into(),
                "The registered source root is a link or reparse point. Register its real directory instead.",
            )],
        });
    }
    let canonical_root =
        std::fs::canonicalize(&root).map_err(|error| AppError::InvalidArgument {
            message: format!(
                "could not resolve skill source root {}: {error}",
                root.display()
            ),
        })?;

    let mut package_roots = Vec::new();
    let mut errors = Vec::new();
    let mut directories = VecDeque::from([canonical_root.clone()]);

    while let Some(directory) = directories.pop_front() {
        let directory_metadata =
            std::fs::symlink_metadata(&directory).map_err(|error| AppError::Io {
                message: format!("inspect {} before descent: {error}", directory.display()),
            })?;
        if directory_metadata.file_type().is_symlink()
            || metadata_is_reparse_point(&directory_metadata)
        {
            errors.push(unsafe_entry_error(
                relative_path(&canonical_root, &directory),
                "Links and reparse points are not allowed in skill sources. Remove the link and refresh.",
            ));
            continue;
        }
        let mut entries = read_directory_sorted(&directory)?;
        for (path, metadata) in entries.drain(..) {
            let relative = relative_path(&canonical_root, &path);
            if metadata.file_type().is_symlink() {
                errors.push(unsafe_entry_error(
                    relative,
                    "Symbolic links are not allowed in skill sources. Remove the link and refresh.",
                ));
                continue;
            }
            if metadata_is_reparse_point(&metadata) {
                errors.push(unsafe_entry_error(
                    relative,
                    "Windows reparse points are not allowed in skill sources. Remove the link and refresh.",
                ));
                continue;
            }
            if metadata.is_dir()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with('.'))
            {
                continue;
            }
            if metadata.is_dir() {
                directories.push_back(path);
            } else if metadata.is_file()
                && path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md")
            {
                package_roots.push(directory.clone());
            } else if !metadata.is_file() {
                errors.push(unsafe_entry_error(
                    relative,
                    "Special filesystem entries are not allowed in skill sources. Remove the entry and refresh.",
                ));
            }
        }
    }

    package_roots.sort();
    package_roots.dedup();
    errors.sort_by(|left, right| left.path.cmp(&right.path));
    let mut packages = Vec::with_capacity(package_roots.len());
    for package_root in package_roots {
        let relative = relative_path(&canonical_root, &package_root);
        let canonical_package = match std::fs::canonicalize(&package_root) {
            Ok(path) if path.starts_with(&canonical_root) => path,
            Ok(_) => {
                errors.push(unsafe_entry_error(
                    relative,
                    "Skill package resolves outside its registered source. Remove the link and refresh.",
                ));
                continue;
            }
            Err(error) => {
                errors.push(SkillValidationError {
                    code: SkillValidationCode::Io,
                    path: relative,
                    message: format!("Could not resolve skill package: {error}"),
                });
                continue;
            }
        };
        packages.push(validate_package(
            &source.id,
            &canonical_root,
            &canonical_package,
        ));
    }
    packages.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    errors.sort_by(|left, right| left.path.cmp(&right.path));

    Ok(SkillSourceResult {
        source,
        packages,
        errors,
    })
}

fn resolved_source_root(source: &SkillSource) -> Result<PathBuf, AppError> {
    match &source.kind {
        SkillSourceKind::Local { root } => Ok(PathBuf::from(root)),
        SkillSourceKind::Github {
            active_checkout: Some(root),
            ..
        } => Ok(PathBuf::from(root)),
        SkillSourceKind::Github { .. } => Err(AppError::InvalidArgument {
            message: "GitHub source has no active checkout".into(),
        }),
    }
}

pub(crate) struct ResolvedSkillPackage {
    root: PathBuf,
    package: SkillPackageResult,
}

impl ResolvedSkillPackage {
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn files(&self) -> &[SkillPackageFile] {
        &self.package.files
    }

    pub(crate) fn name(&self) -> Option<&str> {
        self.package.name.as_deref()
    }
}

pub(crate) async fn resolve_skill_package(
    state: &AppState,
    source_id: &str,
    relative_path: &str,
) -> Result<ResolvedSkillPackage, AppError> {
    let source = load_skill_sources_for_state(state)
        .await?
        .into_iter()
        .find(|source| source.id == source_id)
        .ok_or_else(|| AppError::InvalidArgument {
            message: format!("unknown skill source id: {source_id}"),
        })?;
    let mut result = discover_source(source.clone()).await?;
    apply_persisted_trust(state, &mut result).await?;
    let package = result
        .packages
        .into_iter()
        .find(|package| package.relative_path == relative_path && package.installable)
        .ok_or_else(|| AppError::InvalidArgument {
            message: format!("unknown installable skill package: {relative_path}"),
        })?;
    let source_root = canonical_skill_source_root(&source)?;
    let root =
        std::fs::canonicalize(source_root.join(&package.relative_path)).map_err(|error| {
            AppError::InvalidArgument {
                message: format!(
                    "could not resolve skill package {}: {error}",
                    package.relative_path
                ),
            }
        })?;
    let metadata = std::fs::symlink_metadata(&root).map_err(|error| AppError::Io {
        message: format!("inspect skill package {}: {error}", root.display()),
    })?;
    if !root.starts_with(&source_root)
        || !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata_is_reparse_point(&metadata)
    {
        return Err(AppError::InvalidArgument {
            message: "skill package escaped or is not a real directory".into(),
        });
    }
    Ok(ResolvedSkillPackage { root, package })
}

pub(crate) async fn list_skill_files(
    state: &AppState,
    source_id: &str,
    relative_path: &str,
) -> Result<Vec<SkillPackageFile>, AppError> {
    Ok(resolve_skill_package(state, source_id, relative_path)
        .await?
        .package
        .files)
}

pub(crate) async fn read_skill_file(
    state: &AppState,
    source_id: &str,
    relative_path: &str,
    file_path: &str,
) -> Result<SkillFileContent, AppError> {
    let package = resolve_skill_package(state, source_id, relative_path).await?;
    let file_path = normalized_requested_file_path(file_path)?;
    let file = package
        .package
        .files
        .iter()
        .find(|file| file.relative_path == file_path)
        .ok_or_else(|| AppError::InvalidArgument {
            message: format!("skill file is not in the validated inventory: {file_path}"),
        })?;
    let path = package.root.join(&file.relative_path);
    let metadata = std::fs::symlink_metadata(&path).map_err(|error| AppError::Io {
        message: format!("inspect skill file {}: {error}", path.display()),
    })?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata_is_reparse_point(&metadata)
    {
        return Err(AppError::InvalidArgument {
            message: "skill file is not a real regular file".into(),
        });
    }
    let path = std::fs::canonicalize(&path).map_err(|error| AppError::Io {
        message: format!("resolve skill file {}: {error}", path.display()),
    })?;
    if !path.starts_with(&package.root) {
        return Err(AppError::InvalidArgument {
            message: "skill file escaped its validated package".into(),
        });
    }
    let bytes =
        read_bounded(&path, MAX_SKILL_FILE_BYTES).map_err(|error| AppError::InvalidArgument {
            message: error.message(),
        })?;
    if bytes.len() as u64 != file.size_bytes
        || format!("{:x}", Sha256::digest(&bytes)) != file.sha256
    {
        return Err(AppError::InvalidArgument {
            message: "skill file changed since validation; refresh the source before reading"
                .into(),
        });
    }
    match String::from_utf8(bytes) {
        Ok(text) => Ok(SkillFileContent {
            relative_path: file.relative_path.clone(),
            mime_type: "text/plain".into(),
            text: Some(text),
            base64: None,
        }),
        Err(error) => Ok(SkillFileContent {
            relative_path: file.relative_path.clone(),
            mime_type: "application/octet-stream".into(),
            text: None,
            base64: Some(base64::engine::general_purpose::STANDARD.encode(error.into_bytes())),
        }),
    }
}

fn canonical_skill_source_root(source: &SkillSource) -> Result<PathBuf, AppError> {
    let root = resolved_source_root(source)?;
    let metadata = std::fs::symlink_metadata(&root).map_err(|error| AppError::InvalidArgument {
        message: format!("could not inspect skill source {}: {error}", root.display()),
    })?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata_is_reparse_point(&metadata)
    {
        return Err(AppError::InvalidArgument {
            message: "skill source root must be a real directory".into(),
        });
    }
    std::fs::canonicalize(&root).map_err(|error| AppError::InvalidArgument {
        message: format!("could not resolve skill source {}: {error}", root.display()),
    })
}

fn normalized_requested_file_path(file_path: &str) -> Result<String, AppError> {
    if file_path.is_empty() || file_path.contains('\\') {
        return Err(AppError::InvalidArgument {
            message: "skill file path must be a normalized relative path".into(),
        });
    }
    let parts = Path::new(file_path)
        .components()
        .map(|component| match component {
            Component::Normal(part) => part.to_str().ok_or_else(|| AppError::InvalidArgument {
                message: "skill file path must be valid UTF-8".into(),
            }),
            _ => Err(AppError::InvalidArgument {
                message: "skill file path must be a normalized relative path".into(),
            }),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let normalized = parts.join("/");
    if normalized.is_empty() || normalized != file_path {
        return Err(AppError::InvalidArgument {
            message: "skill file path must be a normalized relative path".into(),
        });
    }
    Ok(normalized)
}

fn canonical_target_base(path: &Path) -> Result<PathBuf, AppError> {
    if !path.is_absolute() {
        return Err(AppError::InvalidArgument {
            message: "skill target root must be absolute".into(),
        });
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|error| AppError::InvalidArgument {
        message: format!(
            "could not inspect skill target root {}: {error}",
            path.display()
        ),
    })?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata_is_reparse_point(&metadata)
    {
        return Err(AppError::InvalidArgument {
            message: format!(
                "skill target root must be a real directory: {}",
                path.display()
            ),
        });
    }
    std::fs::canonicalize(path).map_err(|error| AppError::InvalidArgument {
        message: format!(
            "could not resolve skill target root {}: {error}",
            path.display()
        ),
    })
}

fn reject_linked_destination_ancestors(base: &Path, destination: &Path) -> Result<(), AppError> {
    let parent = destination
        .parent()
        .ok_or_else(|| AppError::InvalidArgument {
            message: "skill destination has no parent".into(),
        })?;
    let relative = parent
        .strip_prefix(base)
        .map_err(|_| AppError::InvalidArgument {
            message: "skill destination escaped its target root".into(),
        })?;
    let mut current = base.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata)
                if metadata.file_type().is_symlink() || metadata_is_reparse_point(&metadata) =>
            {
                return Err(AppError::InvalidArgument {
                    message: format!(
                        "skill destination contains a linked ancestor: {}",
                        current.display()
                    ),
                });
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(AppError::InvalidArgument {
                    message: format!(
                        "skill destination ancestor is not a directory: {}",
                        current.display()
                    ),
                });
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(AppError::Io {
                    message: format!("inspect skill destination {}: {error}", current.display()),
                })
            }
        }
    }
    Ok(())
}

fn installed_view(record: &SkillInstallRecord, state: SkillInstallState) -> InstalledSkill {
    InstalledSkill {
        source_id: record.source_id.clone(),
        relative_path: record.relative_path.clone(),
        name: record.name.clone(),
        runtime: record.runtime.clone(),
        scope: record.scope.clone(),
        project_path: record.project_path.clone(),
        path: record.dest.clone(),
        state,
        tracked: true,
    }
}

fn read_directory_sorted(directory: &Path) -> Result<Vec<(PathBuf, Metadata)>, AppError> {
    let mut entries = std::fs::read_dir(directory)
        .map_err(|error| AppError::Io {
            message: format!("read directory {}: {error}", directory.display()),
        })?
        .map(|entry| {
            let entry = entry.map_err(|error| AppError::Io {
                message: format!("read entry in {}: {error}", directory.display()),
            })?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).map_err(|error| AppError::Io {
                message: format!("inspect {}: {error}", path.display()),
            })?;
            Ok((path, metadata))
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(entries)
}

pub(crate) fn validate_package(
    source_id: &str,
    source_root: &Path,
    package_root: &Path,
) -> SkillPackageResult {
    let relative = match normalized_relative_path(source_root, package_root) {
        Ok(relative) => relative,
        Err(error) => {
            return invalid_package_root(source_id, ".", &error.message);
        }
    };
    let mut result = SkillPackageResult {
        source_id: source_id.into(),
        relative_path: relative,
        name: None,
        description: None,
        skill_type: SkillType::Other,
        group: Vec::new(),
        tags: Vec::new(),
        dependencies: Vec::new(),
        recommended_skills: Vec::new(),
        version: None,
        channel: "stable".into(),
        changelog: None,
        publisher: None,
        publisher_key: None,
        publisher_verified: false,
        validation_results: Vec::new(),
        permissions: Vec::new(),
        quality_score: 0,
        quality_checks: Vec::new(),
        files: Vec::new(),
        trust_fingerprint: None,
        errors: Vec::new(),
        installable: false,
    };

    match std::fs::symlink_metadata(package_root) {
        Ok(metadata)
            if metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && !metadata_is_reparse_point(&metadata) => {}
        Ok(_) => {
            result.errors.push(unsafe_entry_error(
                ".".into(),
                "Skill package roots must be real directories, not links, reparse points, or special entries.",
            ));
            return result;
        }
        Err(error) => {
            result.errors.push(SkillValidationError {
                code: SkillValidationCode::Io,
                path: ".".into(),
                message: format!("Could not inspect skill package root: {error}"),
            });
            return result;
        }
    }

    result.files = inventory_package(package_root, &mut result.errors);

    let mut skill_text = None;
    match read_bounded(&package_root.join("SKILL.md"), MAX_SKILL_FILE_BYTES) {
        Ok(bytes) => {
            skill_text = String::from_utf8(bytes.clone()).ok();
            match parse_skill_metadata(&bytes) {
                Ok(metadata) => {
                    result.name = Some(metadata.name.clone());
                    result.description = Some(metadata.description);
                    result.skill_type = metadata.skill_type;
                    result.group = metadata.group;
                    result.tags = metadata.tags;
                    result.dependencies = metadata.dependencies;
                    result.recommended_skills = metadata.recommended_skills;
                    result.version = metadata.version;
                    result.channel = metadata.channel;
                    result.changelog = metadata.changelog;
                    for required in &metadata.validation {
                        if result
                            .files
                            .iter()
                            .any(|file| file.relative_path == *required)
                        {
                            result
                                .validation_results
                                .push(format!("PASS required file: {required}"));
                        } else {
                            result.errors.push(SkillValidationError {
                                code: SkillValidationCode::InvalidMetadata,
                                path: "SKILL.md".into(),
                                message: format!("Required validation file is missing: {required}"),
                            });
                        }
                    }
                    if let Some(publisher) = metadata.publisher {
                        result.publisher = Some(publisher.name.clone());
                        result.publisher_key = Some(publisher.public_key.clone());
                        result.publisher_verified = verify_publisher(
                            &publisher,
                            result.name.as_deref().unwrap_or_default(),
                            result.version.as_deref().unwrap_or("0.0.0"),
                            &result.channel,
                            skill_text.as_deref().unwrap_or_default(),
                            &result.files,
                        );
                        if !result.publisher_verified {
                            result.errors.push(SkillValidationError {
                                code: SkillValidationCode::InvalidMetadata,
                                path: "SKILL.md".into(),
                                message: "Publisher signature verification failed.".into(),
                            });
                        }
                    }
                    let directory_name = package_root.file_name().and_then(|name| name.to_str());
                    if directory_name != Some(metadata.name.as_str()) {
                        result.errors.push(SkillValidationError {
                            code: SkillValidationCode::InvalidMetadata,
                            path: "SKILL.md".into(),
                            message: format!(
                                "Skill name '{}' must match directory '{}'.",
                                metadata.name,
                                directory_name.unwrap_or_default()
                            ),
                        });
                    }
                }
                Err(message) => result.errors.push(SkillValidationError {
                    code: SkillValidationCode::InvalidMetadata,
                    path: "SKILL.md".into(),
                    message,
                }),
            }
        }
        Err(error) => {
            let code = error.code();
            result.errors.push(SkillValidationError {
                code,
                path: "SKILL.md".into(),
                message: error.message(),
            });
        }
    }

    if result
        .errors
        .iter()
        .any(|error| error.code == SkillValidationCode::TrustRequired)
        && result
            .errors
            .iter()
            .all(|error| error.code == SkillValidationCode::TrustRequired)
    {
        if let Ok((tree_hash, executables)) = trust_fingerprint(package_root, &result) {
            result.trust_fingerprint = Some(SkillTrustFingerprint {
                tree_hash,
                executables,
            });
        }
    }
    sort_validation_errors(&mut result.errors);
    result.installable = result.errors.is_empty();
    analyze_package(&mut result, skill_text.as_deref().unwrap_or_default());
    result
}

fn invalid_package_root(source_id: &str, relative_path: &str, message: &str) -> SkillPackageResult {
    SkillPackageResult {
        source_id: source_id.into(),
        relative_path: relative_path.into(),
        name: None,
        description: None,
        skill_type: SkillType::Other,
        group: Vec::new(),
        tags: Vec::new(),
        dependencies: Vec::new(),
        recommended_skills: Vec::new(),
        version: None,
        channel: "stable".into(),
        changelog: None,
        publisher: None,
        publisher_key: None,
        publisher_verified: false,
        validation_results: Vec::new(),
        permissions: Vec::new(),
        quality_score: 0,
        quality_checks: Vec::new(),
        files: Vec::new(),
        trust_fingerprint: None,
        errors: vec![unsafe_entry_error(
            ".".into(),
            &format!("{message} Keep the package inside its registered source."),
        )],
        installable: false,
    }
}

fn analyze_package(result: &mut SkillPackageResult, skill_text: &str) {
    let lower = skill_text.to_ascii_lowercase();
    if result.files.iter().any(|file| {
        file.relative_path
            .to_ascii_lowercase()
            .starts_with("scripts/")
    }) {
        result.permissions.push("execute-scripts".into());
    }
    if ["https://", "http://", "curl ", "wget ", "fetch("]
        .iter()
        .any(|token| lower.contains(token))
    {
        result.permissions.push("network".into());
    }
    if ["~/", "/users/", "filesystem", "read file", "write file"]
        .iter()
        .any(|token| lower.contains(token))
    {
        result.permissions.push("filesystem".into());
    }
    if ["mcp", "command line", "shell command"]
        .iter()
        .any(|token| lower.contains(token))
    {
        result.permissions.push("external-tools".into());
    }

    let mut score = 0;
    if result.name.is_some() && result.description.is_some() {
        score += 20;
        result.quality_checks.push("Valid required metadata".into());
    }
    if result
        .description
        .as_ref()
        .is_some_and(|description| description.chars().count() >= 80)
    {
        score += 20;
        result.quality_checks.push("Detailed description".into());
    }
    if result.skill_type != SkillType::Other {
        score += 20;
        result.quality_checks.push("Explicit skill type".into());
    }
    if !result.group.is_empty() || !result.tags.is_empty() {
        score += 20;
        result
            .quality_checks
            .push("Discoverability metadata".into());
    }
    if lower.contains("```")
        || lower.contains("example")
        || result
            .files
            .iter()
            .any(|file| file.relative_path.starts_with("references/"))
    {
        score += 20;
        result.quality_checks.push("Examples or references".into());
    }
    result.quality_score = score;
}

fn inventory_package(
    package_root: &Path,
    errors: &mut Vec<SkillValidationError>,
) -> Vec<SkillPackageFile> {
    let mut files = Vec::new();
    let mut file_count = 0_usize;
    let mut total_bytes = 0_u64;
    let mut directories = VecDeque::from([package_root.to_path_buf()]);

    while let Some(directory) = directories.pop_front() {
        let entries = match read_directory_sorted(&directory) {
            Ok(entries) => entries,
            Err(error) => {
                errors.push(SkillValidationError {
                    code: SkillValidationCode::Io,
                    path: normalized_relative_path(package_root, &directory)
                        .unwrap_or_else(|_| ".".into()),
                    message: error.to_string(),
                });
                continue;
            }
        };
        for (path, metadata) in entries {
            let relative = match normalized_relative_path(package_root, &path) {
                Ok(relative) => relative,
                Err(error) => {
                    errors.push(error);
                    continue;
                }
            };
            if metadata.file_type().is_symlink() || metadata_is_reparse_point(&metadata) {
                errors.push(unsafe_entry_error(
                    relative,
                    "Links and reparse points are not allowed in skill packages. Remove the entry and refresh.",
                ));
                continue;
            }
            if metadata.is_dir() {
                if has_executable_suffix(&path) {
                    errors.push(unsafe_entry_error(
                        relative,
                        "Executable and script file types are not allowed in skill packages. Remove the entry and refresh.",
                    ));
                } else if has_prohibited_surface(&relative) {
                    errors.push(unsafe_entry_error(
                        relative,
                        "Hooks, MCP, and plugin surfaces are not allowed in skill packages. Remove the entry and refresh.",
                    ));
                } else {
                    directories.push_back(path);
                }
                continue;
            }
            if !metadata.is_file() {
                errors.push(unsafe_entry_error(
                    relative,
                    "Special filesystem entries are not allowed in skill packages. Remove the entry and refresh.",
                ));
                continue;
            }
            file_count += 1;
            let mut rejected = false;
            let script_candidate = is_scripts_path(&relative);
            if file_count > MAX_SKILL_FILES {
                errors.push(SkillValidationError {
                    code: SkillValidationCode::UnsafeEntry,
                    path: relative.clone(),
                    message: format!(
                        "Skill package exceeds the {MAX_SKILL_FILES}-file limit. Remove files and refresh."
                    ),
                });
                rejected = true;
            }
            if !script_candidate && !is_passive_skill_file(&path) {
                errors.push(unsafe_entry_error(
                    relative.clone(),
                    "Only documentation and passive resources are allowed outside scripts/. Move executable or support code into scripts/ for explicit trust.",
                ));
                rejected = true;
            }
            if has_prohibited_surface(&relative) {
                errors.push(unsafe_entry_error(
                    relative.clone(),
                    "Hooks, MCP, and plugin surfaces are not allowed in skill packages. Remove the entry and refresh.",
                ));
                rejected = true;
            }
            if metadata_is_executable(&metadata) && !script_candidate {
                errors.push(unsafe_entry_error(
                    relative.clone(),
                    "Executable permission bits are allowed only inside scripts/. Remove execute permissions or move the entry.",
                ));
                rejected = true;
            }
            if rejected {
                continue;
            }
            let bytes = match read_bounded(&path, MAX_SKILL_FILE_BYTES) {
                Ok(bytes) => bytes,
                Err(error) => {
                    let code = error.code();
                    errors.push(SkillValidationError {
                        code,
                        path: relative,
                        message: format!(
                            "{} {}",
                            error.message(),
                            if code == SkillValidationCode::UnsafeEntry {
                                "Reduce the file size and refresh."
                            } else {
                                "Fix file access and refresh."
                            }
                        ),
                    });
                    continue;
                }
            };
            let next_total = total_bytes + bytes.len() as u64;
            if next_total > MAX_SKILL_TOTAL_BYTES {
                errors.push(SkillValidationError {
                    code: SkillValidationCode::UnsafeEntry,
                    path: relative,
                    message: format!(
                        "Skill package exceeds the {}-byte total limit. Remove content and refresh.",
                        MAX_SKILL_TOTAL_BYTES
                    ),
                });
                continue;
            }
            total_bytes = next_total;
            if script_candidate
                && !errors
                    .iter()
                    .any(|error| error.code == SkillValidationCode::TrustRequired)
            {
                errors.push(SkillValidationError {
                    code: SkillValidationCode::TrustRequired,
                    path: "scripts".into(),
                    message:
                        "This package contains scripts. Review and trust this exact version before installing."
                            .into(),
                });
            }
            files.push(SkillPackageFile {
                relative_path: relative,
                size_bytes: bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(&bytes)),
            });
        }
    }
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    files
}

enum BoundedReadError {
    Io(String),
    TooLarge(String),
}

impl BoundedReadError {
    fn code(&self) -> SkillValidationCode {
        match self {
            Self::Io(_) => SkillValidationCode::Io,
            Self::TooLarge(_) => SkillValidationCode::UnsafeEntry,
        }
    }

    fn message(self) -> String {
        match self {
            Self::Io(message) | Self::TooLarge(message) => message,
        }
    }
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, BoundedReadError> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|error| {
            BoundedReadError::Io(format!("Could not open {}: {error}", path.display()))
        })?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            BoundedReadError::Io(format!("Could not read {}: {error}", path.display()))
        })?;
    if bytes.len() as u64 > limit {
        return Err(BoundedReadError::TooLarge(format!(
            "{} exceeds the {limit}-byte file limit.",
            path.display()
        )));
    }
    Ok(bytes)
}

#[derive(Deserialize)]
struct SkillMetadata {
    name: String,
    description: String,
    #[serde(default, rename = "type")]
    skill_type: SkillType,
    #[serde(default)]
    group: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default, rename = "recommended-skills")]
    recommended_skills: Vec<String>,
    version: Option<String>,
    #[serde(default = "default_skill_channel")]
    channel: String,
    changelog: Option<String>,
    publisher: Option<SkillPublisherMetadata>,
    #[serde(default)]
    validation: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct SkillPublisherMetadata {
    name: String,
    public_key: String,
    signature: String,
}

fn default_skill_channel() -> String {
    "stable".into()
}

fn parse_skill_metadata(bytes: &[u8]) -> Result<SkillMetadata, String> {
    let source =
        std::str::from_utf8(bytes).map_err(|error| format!("SKILL.md must be UTF-8: {error}"))?;
    let rest = source
        .strip_prefix("---\n")
        .ok_or_else(|| "SKILL.md must start with YAML frontmatter.".to_string())?;
    let end = rest
        .find("\n---")
        .ok_or_else(|| "SKILL.md frontmatter must end with '---'.".to_string())?;
    let yaml = &rest[..end];
    let metadata: SkillMetadata = serde_yaml::from_str(yaml)
        .map_err(|error| format!("SKILL.md frontmatter is invalid: {error}"))?;
    if !valid_skill_name(&metadata.name) {
        return Err(
            "SKILL.md name must be 1-64 lowercase ASCII letters, digits, or single hyphens.".into(),
        );
    }
    let description_length = metadata.description.trim().chars().count();
    if !(1..=1024).contains(&description_length) {
        return Err("SKILL.md description must contain 1-1024 trimmed characters.".into());
    }
    if metadata.group.len() > MAX_SKILL_GROUP_DEPTH {
        return Err(format!(
            "SKILL.md group must contain at most {MAX_SKILL_GROUP_DEPTH} nested segments."
        ));
    }
    if metadata.tags.len() > MAX_SKILL_TAGS {
        return Err(format!(
            "SKILL.md tags must contain at most {MAX_SKILL_TAGS} entries."
        ));
    }
    if metadata.dependencies.len() > MAX_SKILL_DEPENDENCIES
        || metadata.recommended_skills.len() > MAX_SKILL_DEPENDENCIES
    {
        return Err(format!(
            "SKILL.md dependencies and recommended-skills may contain at most {MAX_SKILL_DEPENDENCIES} entries each."
        ));
    }
    if metadata
        .version
        .as_deref()
        .is_some_and(|version| !valid_semver(version))
    {
        return Err("SKILL.md version must be MAJOR.MINOR.PATCH.".into());
    }
    if !matches!(metadata.channel.as_str(), "stable" | "beta") {
        return Err("SKILL.md channel must be stable or beta.".into());
    }
    if metadata
        .changelog
        .as_deref()
        .is_some_and(|value| value.trim().is_empty() || value.chars().count() > 4096)
    {
        return Err("SKILL.md changelog must contain 1-4096 characters.".into());
    }
    if metadata.validation.len() > 32
        || metadata
            .validation
            .iter()
            .any(|path| normalized_requested_file_path(path).is_err())
    {
        return Err(
            "SKILL.md validation must contain at most 32 normalized package file paths.".into(),
        );
    }
    if metadata
        .group
        .iter()
        .chain(metadata.tags.iter())
        .any(|value| !valid_taxonomy_segment(value))
    {
        return Err(format!(
            "SKILL.md group and tags must use 1-{MAX_SKILL_TAXONOMY_SEGMENT_BYTES} lowercase letters, digits, or single hyphens."
        ));
    }
    let unique_tags = metadata.tags.iter().collect::<HashSet<_>>();
    if unique_tags.len() != metadata.tags.len() {
        return Err("SKILL.md tags must not contain duplicates.".into());
    }
    for dependency in metadata
        .dependencies
        .iter()
        .chain(metadata.recommended_skills.iter())
    {
        if !valid_skill_name(dependency) {
            return Err(
                "SKILL.md dependencies and recommended-skills must use valid skill names.".into(),
            );
        }
    }
    if metadata.dependencies.iter().collect::<HashSet<_>>().len() != metadata.dependencies.len()
        || metadata
            .recommended_skills
            .iter()
            .collect::<HashSet<_>>()
            .len()
            != metadata.recommended_skills.len()
    {
        return Err("SKILL.md dependency lists must not contain duplicates.".into());
    }
    Ok(metadata)
}

fn valid_semver(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn verify_publisher(
    publisher: &SkillPublisherMetadata,
    name: &str,
    version: &str,
    channel: &str,
    skill_text: &str,
    files: &[SkillPackageFile],
) -> bool {
    let body = skill_text
        .strip_prefix("---\n")
        .and_then(|rest| rest.find("\n---").map(|end| &rest[end + 4..]))
        .unwrap_or(skill_text);
    let mut signed_parts = vec![
        name.as_bytes(),
        version.as_bytes(),
        channel.as_bytes(),
        body.as_bytes(),
    ];
    for file in files.iter().filter(|file| file.relative_path != "SKILL.md") {
        signed_parts.push(file.relative_path.as_bytes());
        signed_parts.push(file.sha256.as_bytes());
    }
    crate::library::verify_publisher_signature(
        &publisher.name,
        &publisher.public_key,
        &publisher.signature,
        &signed_parts,
    )
}

fn valid_taxonomy_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_SKILL_TAXONOMY_SEGMENT_BYTES
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn has_executable_suffix(path: &Path) -> bool {
    const SUFFIXES: [&str; 16] = [
        ".sh", ".bash", ".zsh", ".fish", ".ps1", ".bat", ".cmd", ".com", ".exe", ".dll", ".dylib",
        ".so", ".app", ".py", ".rb", ".pl",
    ];
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    SUFFIXES.iter().any(|suffix| lower.ends_with(suffix))
}

fn is_passive_skill_file(path: &Path) -> bool {
    const EXTENSIONS: [&str; 26] = [
        "md", "markdown", "txt", "json", "jsonl", "yaml", "yml", "toml", "csv", "tsv", "xml",
        "css", "png", "jpg", "jpeg", "gif", "webp", "avif", "svg", "pdf", "ico", "woff", "woff2",
        "ttf", "otf", "lock",
    ];
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            EXTENSIONS
                .iter()
                .any(|allowed| extension.eq_ignore_ascii_case(allowed))
        })
}

fn has_reserved_surface(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    let normalized = lower.trim_start_matches('.');
    ["scripts", "hooks", "mcp", "plugin", "plugins"]
        .iter()
        .any(|surface| {
            normalized == *surface
                || normalized
                    .strip_prefix(surface)
                    .is_some_and(|suffix| suffix.starts_with('.'))
        })
}

fn is_scripts_path(relative: &str) -> bool {
    relative
        .split('/')
        .next()
        .is_some_and(|component| component.eq_ignore_ascii_case("scripts"))
}

fn has_prohibited_surface(relative: &str) -> bool {
    relative.split('/').enumerate().any(|(index, component)| {
        !(index == 0 && component.eq_ignore_ascii_case("scripts"))
            && has_reserved_surface(Path::new(component))
    })
}

fn trust_fingerprint(
    package_root: &Path,
    package: &SkillPackageResult,
) -> Result<(String, Vec<SkillTrustedExecutable>), AppError> {
    let tree_hash = install::validated_tree_hash(package_root, &package.files)?;
    let mut executables = Vec::new();
    for file in package
        .files
        .iter()
        .filter(|file| is_scripts_path(&file.relative_path))
    {
        let path = package_root.join(normalized_requested_file_path(&file.relative_path)?);
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| AppError::Io {
            message: format!("inspect trusted script {}: {error}", path.display()),
        })?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata_is_reparse_point(&metadata)
        {
            return Err(AppError::InvalidArgument {
                message: format!(
                    "trusted script inventory entry is not a regular file: {}",
                    file.relative_path
                ),
            });
        }
        executables.push(SkillTrustedExecutable {
            relative_path: file.relative_path.clone(),
            sha256: file.sha256.clone(),
            executable: metadata_is_executable(&metadata),
        });
    }
    executables.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok((tree_hash, executables))
}

fn apply_skill_trust(
    source_root: &Path,
    result: &mut SkillSourceResult,
    records: &[SkillTrustRecord],
    key: Option<&[u8]>,
) {
    let Some(key) = key else {
        return;
    };
    for package in &mut result.packages {
        if !package
            .errors
            .iter()
            .any(|error| error.code == SkillValidationCode::TrustRequired)
        {
            continue;
        }
        let Some(record) = records.iter().find(|record| {
            record.source_id == package.source_id
                && record.relative_path == package.relative_path
                && verify_trust_record(record, key)
        }) else {
            continue;
        };
        let Ok((tree_hash, executables)) =
            trust_fingerprint(&source_root.join(&package.relative_path), package)
        else {
            continue;
        };
        if record.tree_hash != tree_hash || record.executables != executables {
            continue;
        }
        package
            .errors
            .retain(|error| error.code != SkillValidationCode::TrustRequired);
        package.installable = package.errors.is_empty();
    }
}

fn normalized_relative_path(root: &Path, path: &Path) -> Result<String, SkillValidationError> {
    let relative = path.strip_prefix(root).map_err(|_| {
        unsafe_entry_error(
            ".".into(),
            "Skill package paths must remain inside the package root. Move the entry inside the package and refresh.",
        )
    })?;
    if relative.as_os_str().is_empty() {
        return Ok(".".into());
    }

    let mut parts = Vec::new();
    for component in relative.components() {
        let std::path::Component::Normal(value) = component else {
            return Err(unsafe_entry_error(
                relative.to_string_lossy().into_owned(),
                "Skill package paths must contain only normal relative components. Rename the entry and refresh.",
            ));
        };
        let Some(value) = value.to_str() else {
            return Err(unsafe_entry_error(
                relative.to_string_lossy().into_owned(),
                "Skill package paths must be valid UTF-8. Rename the entry and refresh.",
            ));
        };
        parts.push(value);
    }
    Ok(parts.join("/"))
}

fn sort_validation_errors(errors: &mut [SkillValidationError]) {
    errors.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| validation_code_rank(left.code).cmp(&validation_code_rank(right.code)))
            .then_with(|| left.message.cmp(&right.message))
    });
}

fn validation_code_rank(code: SkillValidationCode) -> u8 {
    match code {
        SkillValidationCode::InvalidMetadata => 0,
        SkillValidationCode::TrustRequired => 1,
        SkillValidationCode::UnsafeEntry => 2,
        SkillValidationCode::Io => 3,
    }
}

fn unsafe_entry_error(path: String, message: &str) -> SkillValidationError {
    SkillValidationError {
        code: SkillValidationCode::UnsafeEntry,
        path,
        message: message.into(),
    }
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg_attr(not(any(test, windows)), allow(dead_code))]
pub(crate) fn is_windows_reparse_point(attributes: u32) -> bool {
    attributes & 0x400 != 0
}

#[cfg(windows)]
pub(crate) fn metadata_is_reparse_point(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    is_windows_reparse_point(metadata.file_attributes())
}

#[cfg(not(windows))]
pub(crate) fn metadata_is_reparse_point(_: &Metadata) -> bool {
    false
}

#[cfg(unix)]
fn metadata_is_executable(metadata: &Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn metadata_is_executable(_: &Metadata) -> bool {
    false
}

#[tauri::command]
pub async fn skill_sources_list(state: State<'_, AppState>) -> Result<Vec<SkillSource>, AppError> {
    load_skill_sources_for_state(&state).await
}

#[tauri::command]
pub async fn skill_sources_inspect(
    state: State<'_, AppState>,
) -> Result<Vec<SkillSourceResult>, AppError> {
    inspect_skill_sources(&state).await
}

#[tauri::command]
pub async fn skill_trust_grant(
    state: State<'_, AppState>,
    source_id: String,
    relative_path: String,
) -> Result<SkillPackageResult, AppError> {
    let source = load_skill_sources_for_state(&state)
        .await?
        .into_iter()
        .find(|source| source.id == source_id)
        .ok_or_else(|| AppError::InvalidArgument {
            message: format!("unknown skill source id: {source_id}"),
        })?;
    let source_root = canonical_skill_source_root(&source)?;
    let mut result = discover_source(source).await?;
    let package = result
        .packages
        .iter()
        .find(|package| package.relative_path == relative_path)
        .ok_or_else(|| AppError::InvalidArgument {
            message: format!("unknown skill package: {relative_path}"),
        })?;
    if !package
        .errors
        .iter()
        .any(|error| error.code == SkillValidationCode::TrustRequired)
        || package
            .errors
            .iter()
            .any(|error| error.code != SkillValidationCode::TrustRequired)
    {
        return Err(AppError::InvalidArgument {
            message: "only otherwise-valid packages containing scripts can be trusted".into(),
        });
    }
    let (tree_hash, executables) = trust_fingerprint(&source_root.join(&relative_path), package)?;
    let records = load_skill_trust_for_state(&state).await?;
    let has_existing_records = !records.is_empty();
    let key = tokio::task::spawn_blocking(move || {
        load_or_create_trust_key_with(&SystemKeychain, has_existing_records)
    })
    .await
    .map_err(|error| AppError::Internal {
        message: format!("skill trust key task failed: {error}"),
    })??;
    let mut record = SkillTrustRecord {
        source_id: source_id.clone(),
        relative_path: relative_path.clone(),
        tree_hash,
        executables,
        granted_at: chrono::Utc::now().to_rfc3339(),
        signature: String::new(),
    };
    record.signature = sign_trust_record(&record, &key)?;
    let retained_relative_path = relative_path.clone();
    let records = mutate_skill_trust(&state, move |records| {
        records.retain(|existing| {
            existing.source_id != source_id || existing.relative_path != retained_relative_path
        });
        records.push(record);
        records.sort_by(|left, right| {
            left.source_id
                .cmp(&right.source_id)
                .then_with(|| left.relative_path.cmp(&right.relative_path))
        });
        Ok(records.clone())
    })
    .await?;
    apply_skill_trust(&source_root, &mut result, &records, Some(&key));
    result
        .packages
        .into_iter()
        .find(|package| package.relative_path == relative_path)
        .ok_or_else(|| AppError::Internal {
            message: "trusted skill package disappeared during validation".into(),
        })
}

#[tauri::command]
pub async fn skill_trust_revoke(
    state: State<'_, AppState>,
    source_id: String,
    relative_path: String,
) -> Result<bool, AppError> {
    mutate_skill_trust(&state, move |records| {
        let before = records.len();
        records.retain(|record| {
            record.source_id != source_id || record.relative_path != relative_path
        });
        Ok(records.len() != before)
    })
    .await
}

#[tauri::command]
pub async fn skill_package_destinations(
    state: State<'_, AppState>,
    source_id: String,
    relative_path: String,
    project_paths: Vec<String>,
) -> Result<Vec<SkillDestinationPresence>, AppError> {
    let package = resolve_skill_package(&state, &source_id, &relative_path).await?;
    let name = package
        .name()
        .map(str::to_owned)
        .ok_or_else(|| AppError::InvalidArgument {
            message: format!("installable skill package has no name: {relative_path}"),
        })?;
    let home = dirs::home_dir().ok_or_else(|| AppError::Io {
        message: "could not resolve home directory".into(),
    })?;
    Ok(skill_destination_presence(&home, &project_paths, &name))
}

pub(crate) async fn install_skill(
    state: &AppState,
    source_id: &str,
    relative_path: &str,
    runtime: &str,
    project_path: Option<&str>,
) -> Result<InstalledSkill, AppError> {
    install_skill_authorized(state, source_id, relative_path, runtime, project_path, None).await
}

pub(crate) async fn plan_skill_install(
    state: &AppState,
    source_id: &str,
    relative_path: &str,
    runtime: &str,
    project_path: Option<&str>,
) -> Result<SkillMutationPlan, AppError> {
    let sources = inspect_skill_sources(state).await?;
    let packages = sources
        .iter()
        .flat_map(|source| source.packages.iter())
        .filter(|package| package.installable)
        .collect::<Vec<_>>();
    let root = packages
        .iter()
        .copied()
        .find(|package| package.source_id == source_id && package.relative_path == relative_path)
        .ok_or_else(|| AppError::InvalidArgument {
            message: "skill package does not exist or is not installable".into(),
        })?;

    fn visit<'a>(
        package: &'a SkillPackageResult,
        packages: &[&'a SkillPackageResult],
        visiting: &mut HashSet<String>,
        visited: &mut HashSet<String>,
        ordered: &mut Vec<&'a SkillPackageResult>,
        blockers: &mut Vec<String>,
        preferred_sources: &HashMap<String, String>,
    ) {
        let Some(name) = package.name.as_ref() else {
            blockers.push(format!("{} has no package name", package.relative_path));
            return;
        };
        if visited.contains(name) {
            return;
        }
        if !visiting.insert(name.clone()) {
            blockers.push(format!("dependency cycle detected at {name}"));
            return;
        }
        for dependency in &package.dependencies {
            let matches = packages
                .iter()
                .copied()
                .filter(|candidate| candidate.name.as_deref() == Some(dependency))
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [candidate] => visit(
                    candidate,
                    packages,
                    visiting,
                    visited,
                    ordered,
                    blockers,
                    preferred_sources,
                ),
                [] => blockers.push(format!("missing dependency: {dependency}")),
                _ => {
                    let preferred = preferred_sources.get(dependency).and_then(|source_id| {
                        matches
                            .iter()
                            .copied()
                            .find(|candidate| &candidate.source_id == source_id)
                    });
                    if let Some(candidate) = preferred {
                        visit(
                            candidate,
                            packages,
                            visiting,
                            visited,
                            ordered,
                            blockers,
                            preferred_sources,
                        );
                    } else {
                        blockers.push(format!("ambiguous dependency: {dependency}"));
                    }
                }
            }
        }
        visiting.remove(name);
        if visited.insert(name.clone()) {
            ordered.push(package);
        }
    }

    let mut blockers = Vec::new();
    let mut ordered = Vec::new();
    let preferred_sources = organize::list(state)
        .await?
        .preferred_sources
        .into_iter()
        .map(|preference| (preference.skill_name, preference.source_id))
        .collect::<HashMap<_, _>>();
    visit(
        root,
        &packages,
        &mut HashSet::new(),
        &mut HashSet::new(),
        &mut ordered,
        &mut blockers,
        &preferred_sources,
    );

    let home = dirs::home_dir().ok_or_else(|| AppError::Io {
        message: "could not resolve home directory".into(),
    })?;
    let project = project_path
        .map(|path| canonical_project_string(Some(path)))
        .transpose()?
        .flatten()
        .map(PathBuf::from);
    let base = project.as_deref().unwrap_or(&home);
    let records = install::load_ledger_for_state(state).await?;
    let mut planned = Vec::new();
    for package in ordered {
        let name = package.name.clone().unwrap_or_default();
        let destination = install::target_path(base, project.as_deref(), runtime, &name)?;
        if destination.exists()
            && !records.iter().any(|record| {
                record.dest == destination.to_string_lossy()
                    && record.source_id == package.source_id
                    && record.relative_path == package.relative_path
            })
        {
            blockers.push(format!(
                "destination is occupied by unmanaged content: {}",
                destination.display()
            ));
        }
        planned.push(SkillPlanPackage {
            source_id: package.source_id.clone(),
            relative_path: package.relative_path.clone(),
            name,
            dependency: package.source_id != source_id || package.relative_path != relative_path,
            destination: destination.to_string_lossy().into_owned(),
            file_count: package.files.len() as u32,
            permissions: package.permissions.clone(),
        });
    }
    Ok(SkillMutationPlan {
        operation: "install".into(),
        runtime: runtime.into(),
        project_path: project.map(|path| path.to_string_lossy().into_owned()),
        rollback_available: planned
            .iter()
            .any(|item| Path::new(&item.destination).exists()),
        packages: planned,
        warnings: Vec::new(),
        blockers,
    })
}

pub(crate) async fn install_skill_with_dependencies(
    state: &AppState,
    source_id: &str,
    relative_path: &str,
    runtime: &str,
    project_path: Option<&str>,
) -> Result<Vec<InstalledSkill>, AppError> {
    install_skill_with_dependencies_authorized(
        state,
        source_id,
        relative_path,
        runtime,
        project_path,
        None,
    )
    .await
}

pub(crate) async fn install_skill_with_dependencies_authorized(
    state: &AppState,
    source_id: &str,
    relative_path: &str,
    runtime: &str,
    project_path: Option<&str>,
    project_authorization: Option<&AuthorizedMcpProject>,
) -> Result<Vec<InstalledSkill>, AppError> {
    let plan = plan_skill_install(state, source_id, relative_path, runtime, project_path).await?;
    if !plan.blockers.is_empty() {
        return Err(AppError::InvalidArgument {
            message: plan.blockers.join("; "),
        });
    }
    let before = install::load_ledger_for_state(state).await?;
    let mut installed = Vec::new();
    let mut created = Vec::new();
    for package in &plan.packages {
        let already_managed = before.iter().any(|record| {
            record.source_id == package.source_id
                && record.relative_path == package.relative_path
                && record.runtime == runtime
                && record.project_path == plan.project_path
        });
        if package.dependency && already_managed {
            continue;
        }
        match install_skill_authorized(
            state,
            &package.source_id,
            &package.relative_path,
            runtime,
            project_path,
            project_authorization,
        )
        .await
        {
            Ok(result) => {
                if !already_managed {
                    created.push(package.clone());
                }
                installed.push(result);
            }
            Err(error) => {
                for created in created.iter().rev() {
                    let _ = uninstall_skill_authorized(
                        state,
                        &created.source_id,
                        &created.relative_path,
                        runtime,
                        project_path,
                        project_authorization,
                    )
                    .await;
                }
                return Err(error);
            }
        }
    }
    Ok(installed)
}

pub(crate) async fn batch_collection(
    state: &AppState,
    collection_name: &str,
    operation: &str,
    runtime: &str,
    project_path: Option<&str>,
) -> Result<SkillBatchResult, AppError> {
    if !matches!(operation, "install" | "update" | "uninstall") {
        return Err(AppError::InvalidArgument {
            message: "batch operation must be install, update, or uninstall".into(),
        });
    }
    let library = organize::list(state).await?;
    let collection = library
        .collections
        .iter()
        .find(|collection| collection.name == collection_name)
        .ok_or_else(|| AppError::InvalidArgument {
            message: format!("collection does not exist: {collection_name}"),
        })?;
    for skill in &collection.skills {
        resolve_skill_package(state, &skill.source_id, &skill.relative_path).await?;
    }
    let mut completed = Vec::new();
    for skill in &collection.skills {
        let result = match operation {
            "install" => install_skill_with_dependencies(
                state,
                &skill.source_id,
                &skill.relative_path,
                runtime,
                project_path,
            )
            .await
            .map(|_| ()),
            "update" => update_skill(
                state,
                &skill.source_id,
                &skill.relative_path,
                runtime,
                project_path,
            )
            .await
            .map(|_| ()),
            "uninstall" => uninstall_skill(
                state,
                &skill.source_id,
                &skill.relative_path,
                runtime,
                project_path,
            )
            .await
            .map(|_| ()),
            _ => unreachable!(),
        };
        if let Err(error) = result {
            for completed_skill in collection.skills.iter().take(completed.len()).rev() {
                match operation {
                    "install" => {
                        let _ = uninstall_skill(
                            state,
                            &completed_skill.source_id,
                            &completed_skill.relative_path,
                            runtime,
                            project_path,
                        )
                        .await;
                    }
                    "uninstall" => {
                        let _ = install_skill(
                            state,
                            &completed_skill.source_id,
                            &completed_skill.relative_path,
                            runtime,
                            project_path,
                        )
                        .await;
                    }
                    // Each update already keeps a recoverable version. Automatic
                    // rollback needs the exact snapshot selected by the user.
                    "update" => {}
                    _ => unreachable!(),
                }
            }
            return Err(AppError::InvalidArgument {
                message: format!(
                    "batch {operation} failed after {} item(s): {error}",
                    completed.len()
                ),
            });
        }
        completed.push(format!("{}/{}", skill.source_id, skill.relative_path));
    }
    Ok(SkillBatchResult {
        operation: operation.into(),
        completed,
        rolled_back: false,
    })
}

pub(crate) async fn install_skill_authorized(
    state: &AppState,
    source_id: &str,
    relative_path: &str,
    runtime: &str,
    project_path: Option<&str>,
    project_authorization: Option<&AuthorizedMcpProject>,
) -> Result<InstalledSkill, AppError> {
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_skill_installs_async(state.app_data_dir.clone()).await?;
    let package = resolve_skill_package(state, source_id, relative_path).await?;
    let name = package
        .name()
        .map(str::to_owned)
        .ok_or_else(|| AppError::InvalidArgument {
            message: format!("installable skill package has no name: {relative_path}"),
        })?;

    let home = dirs::home_dir().ok_or_else(|| AppError::Io {
        message: "could not resolve home directory".into(),
    })?;
    let project = authorized_project_base(project_path, project_authorization)?;
    let base = match project.as_deref() {
        Some(project) => project.to_path_buf(),
        None => canonical_target_base(&home)?,
    };
    let destination = install::target_path(&base, project.as_deref(), runtime, &name)?;
    if project_authorization.is_none() {
        reject_linked_destination_ancestors(&base, &destination)?;
        if let Ok(metadata) = std::fs::symlink_metadata(&destination) {
            if metadata.file_type().is_symlink() || metadata_is_reparse_point(&metadata) {
                return Err(AppError::InvalidArgument {
                    message: format!("skill destination is a link: {}", destination.display()),
                });
            }
        }
    }

    let mut records = install::load_ledger_for_state(state).await?;
    let old_records = records.clone();
    let project_string = project
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned());
    let existing_index = records.iter().position(|record| {
        record.source_id == source_id
            && record.relative_path == relative_path
            && record.runtime == runtime
            && record.project_path == project_string
    });
    let source_hash = install::validated_tree_hash(package.root(), package.files())?;
    let replace_managed = if let Some(index) = existing_index {
        let record = &records[index];
        let (_, recorded_destination) =
            record_destination_authorized(record, project_authorization)?;
        if recorded_destination != destination {
            return Err(AppError::InvalidArgument {
                message: "tracked skill destination does not match the requested target".into(),
            });
        }
        if record.disabled_path.is_some() {
            return Err(AppError::InvalidArgument {
                message: "disabled skills are managed in Phase 4".into(),
            });
        }
        match mutation_tree_hash(project_authorization, &destination)? {
            Some(disk_hash) if disk_hash != record.installed_hash => {
                return Err(AppError::InvalidArgument {
                    message: "skill destination has local modifications".into(),
                })
            }
            Some(_) if record.source_hash == source_hash => {
                return Ok(installed_view(record, SkillInstallState::Current))
            }
            Some(_) => true,
            None => false,
        }
    } else {
        false
    };
    if replace_managed {
        let previous = &records[existing_index.expect("managed replacement has an install")];
        let project_capability = project_authorization
            .map(|authorization| {
                install::project_directory_capability(
                    authorization.root(),
                    &install::project_target_path(runtime, &previous.name)?,
                )
            })
            .transpose()?;
        create_skill_version_snapshot(state, previous, &destination, project_capability.as_ref())
            .await?;
    }

    let installed_hash = source_hash.clone();
    let record = SkillInstallRecord {
        source_id: source_id.into(),
        relative_path: relative_path.into(),
        name,
        runtime: runtime.into(),
        scope: if project.is_some() { "project" } else { "user" }.into(),
        project_path: project_string,
        dest: destination.to_string_lossy().into_owned(),
        source_hash,
        installed_hash,
        installed_at: chrono::Utc::now().to_rfc3339(),
        disabled_path: None,
    };
    let previous_record = existing_index.map(|index| records[index].clone());
    if let Some(index) = existing_index {
        records[index] = record.clone();
    } else {
        records.push(record.clone());
    }

    let database = state.completed_state_database().await?;
    let operation = if let Some(database) = &database {
        Some(
            database
                .prepare_filesystem_operation(
                    if previous_record.is_some() {
                        "skill_update"
                    } else {
                        "skill_install"
                    },
                    &SkillInstallOperation {
                        previous: previous_record,
                        next: record.clone(),
                    },
                )
                .await?,
        )
    } else {
        install::save_ledger_for_state(state, &records).await?;
        None
    };
    let backups = state.app_data_dir.join("skill-backups");
    let install_result = match project_authorization {
        Some(authorization) => match &operation {
            Some(operation) => install::install_validated_directory_in_project_with_id(
                authorization.root(),
                package.root(),
                package.files(),
                &install::project_target_path(runtime, &record.name)?,
                &backups,
                replace_managed,
                &operation.id,
            ),
            None => install::install_validated_directory_in_project(
                authorization.root(),
                package.root(),
                package.files(),
                &install::project_target_path(runtime, &record.name)?,
                &backups,
                replace_managed,
            ),
        },
        None => match &operation {
            Some(operation) => install::install_validated_directory_with_id(
                package.root(),
                package.files(),
                &destination,
                &backups,
                replace_managed,
                &operation.id,
            ),
            None => install::install_validated_directory(
                package.root(),
                package.files(),
                &destination,
                &backups,
                replace_managed,
            ),
        },
    };
    if let Err(error) = install_result {
        if let (Some(database), Some(operation)) = (&database, &operation) {
            database.abort_filesystem_operation(&operation.id).await?;
            return Err(error);
        }
        return match install::save_ledger_for_state(state, &old_records).await {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error("install skill", error, rollback)),
        };
    }
    if let (Some(database), Some(operation)) = (&database, &operation) {
        install::save_ledger_after_filesystem(state, &records, &operation.id).await?;
        database.commit_filesystem_operation(&operation.id).await?;
    }
    Ok(installed_view(&record, SkillInstallState::Current))
}

fn same_skill_install(left: &SkillInstallRecord, right: &SkillInstallRecord) -> bool {
    left.source_id == right.source_id
        && left.relative_path == right.relative_path
        && left.runtime == right.runtime
        && left.project_path == right.project_path
}

fn remove_recovery_directory(path: &Path, expected_hash: &str) -> Result<(), AppError> {
    if !path.exists() {
        return Ok(());
    }
    if skill_tree_hash(path)?.as_deref() != Some(expected_hash) {
        return Err(AppError::StorageCorrupt {
            message: "Skill install recovery found changed staged content".into(),
        });
    }
    std::fs::remove_dir_all(path).map_err(|error| AppError::Io {
        message: format!("clean Skill install recovery directory: {error}"),
    })
}

async fn apply_recovered_install(
    state: &AppState,
    operation_id: &str,
    payload: &SkillInstallOperation,
) -> Result<(), AppError> {
    let mut records = install::load_ledger_for_state(state).await?;
    match records
        .iter()
        .position(|record| same_skill_install(record, &payload.next))
    {
        Some(index) if records[index] == payload.next => {}
        Some(index)
            if payload
                .previous
                .as_ref()
                .is_some_and(|previous| records[index] == *previous) =>
        {
            records[index] = payload.next.clone();
        }
        Some(_) => {
            return Err(AppError::StorageCorrupt {
                message: "Skill install recovery found changed ledger metadata".into(),
            });
        }
        None if payload.previous.is_none() => records.push(payload.next.clone()),
        None => {
            return Err(AppError::StorageCorrupt {
                message: "Skill install recovery lost its previous ledger metadata".into(),
            });
        }
    }
    install::save_ledger_after_filesystem(state, &records, operation_id).await
}

async fn apply_recovered_move(
    state: &AppState,
    operation_id: &str,
    payload: &SkillMoveOperation,
) -> Result<(), AppError> {
    let mut records = install::load_ledger_for_state(state).await?;
    let index = records
        .iter()
        .position(|record| same_skill_install(record, &payload.next))
        .ok_or_else(|| AppError::StorageCorrupt {
            message: "Skill move recovery lost its ledger metadata".into(),
        })?;
    if records[index] == payload.previous {
        records[index] = payload.next.clone();
    } else if records[index] != payload.next {
        return Err(AppError::StorageCorrupt {
            message: "Skill move recovery found changed ledger metadata".into(),
        });
    }
    install::save_ledger_after_filesystem(state, &records, operation_id).await
}

async fn recover_move_operation(
    state: &AppState,
    database: &crate::state_db::StateDatabase,
    operation: &crate::state_db::FilesystemOperation,
) -> Result<(), AppError> {
    let payload =
        serde_json::from_value::<SkillMoveOperation>(operation.payload.clone()).map_err(|_| {
            AppError::StorageCorrupt {
                message: "Skill move recovery payload is invalid".into(),
            }
        })?;
    if !same_skill_install(&payload.previous, &payload.next)
        || payload.previous.installed_hash != payload.next.installed_hash
    {
        return Err(AppError::StorageCorrupt {
            message: "Skill move recovery identity changed".into(),
        });
    }
    let (base, destination) = record_destination(&payload.next)?;
    let disabled_record = if payload.next.disabled_path.is_some() {
        &payload.next
    } else {
        &payload.previous
    };
    let disabled = disabled_destination(disabled_record, &base, &destination)?;
    let (source, target) = match operation.kind.as_str() {
        "skill_disable"
            if payload.previous.disabled_path.is_none() && payload.next.disabled_path.is_some() =>
        {
            (&destination, &disabled)
        }
        "skill_enable"
            if payload.previous.disabled_path.is_some() && payload.next.disabled_path.is_none() =>
        {
            (&disabled, &destination)
        }
        _ => {
            return Err(AppError::StorageCorrupt {
                message: "Skill move recovery transition is invalid".into(),
            });
        }
    };
    let source_hash = skill_tree_hash(source)?;
    let target_hash = skill_tree_hash(target)?;
    match operation.phase {
        crate::state_db::FilesystemOperationPhase::Prepared => {
            if target_hash.as_deref() == Some(&payload.next.installed_hash) && source_hash.is_none()
            {
                apply_recovered_move(state, &operation.id, &payload).await?;
                database.commit_filesystem_operation(&operation.id).await
            } else if source_hash.as_deref() == Some(&payload.previous.installed_hash)
                && target_hash.is_none()
            {
                database.abort_filesystem_operation(&operation.id).await
            } else {
                Err(AppError::StorageCorrupt {
                    message: "Skill move recovery found changed or duplicate content".into(),
                })
            }
        }
        crate::state_db::FilesystemOperationPhase::FilesystemApplied => {
            let records = install::load_ledger_for_state(state).await?;
            if target_hash.as_deref() != Some(&payload.next.installed_hash)
                || source_hash.is_some()
                || !records.iter().any(|record| record == &payload.next)
            {
                Err(AppError::StorageCorrupt {
                    message: "Skill move recovery found changed committed state".into(),
                })
            } else {
                database.commit_filesystem_operation(&operation.id).await
            }
        }
        crate::state_db::FilesystemOperationPhase::Committed => Ok(()),
    }
}

async fn apply_recovered_uninstall(
    state: &AppState,
    operation_id: &str,
    previous: &SkillInstallRecord,
) -> Result<(), AppError> {
    let mut records = install::load_ledger_for_state(state).await?;
    if let Some(index) = records
        .iter()
        .position(|record| same_skill_install(record, previous))
    {
        if records[index] != *previous {
            return Err(AppError::StorageCorrupt {
                message: "Skill uninstall recovery found changed ledger metadata".into(),
            });
        }
        records.remove(index);
    }
    install::save_ledger_after_filesystem(state, &records, operation_id).await
}

async fn recover_uninstall_operation(
    state: &AppState,
    database: &crate::state_db::StateDatabase,
    operation: &crate::state_db::FilesystemOperation,
) -> Result<(), AppError> {
    let payload = serde_json::from_value::<SkillUninstallOperation>(operation.payload.clone())
        .map_err(|_| AppError::StorageCorrupt {
            message: "Skill uninstall recovery payload is invalid".into(),
        })?;
    let (base, destination) = record_destination(&payload.previous)?;
    let target = if payload.previous.disabled_path.is_some() {
        disabled_destination(&payload.previous, &base, &destination)?
    } else {
        destination
    };
    let quarantine = PathBuf::from(&payload.quarantine);
    if quarantine.parent() != target.parent()
        || !quarantine
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(".agency-uninstall-"))
    {
        return Err(AppError::StorageCorrupt {
            message: "Skill uninstall recovery quarantine is invalid".into(),
        });
    }
    let target_hash = skill_tree_hash(&target)?;
    let quarantine_hash = skill_tree_hash(&quarantine)?;
    match operation.phase {
        crate::state_db::FilesystemOperationPhase::Prepared => {
            if target_hash.as_deref() == Some(&payload.target_hash) && quarantine_hash.is_none() {
                database.abort_filesystem_operation(&operation.id).await
            } else if target_hash.is_none()
                && quarantine_hash.as_deref() == Some(&payload.target_hash)
            {
                mutation_uninstall(
                    None,
                    &quarantine,
                    &state.app_data_dir.join("skill-backups"),
                    payload.target_hash != payload.previous.installed_hash,
                )?;
                apply_recovered_uninstall(state, &operation.id, &payload.previous).await?;
                database.commit_filesystem_operation(&operation.id).await
            } else if target_hash.is_none() && quarantine_hash.is_none() {
                apply_recovered_uninstall(state, &operation.id, &payload.previous).await?;
                database.commit_filesystem_operation(&operation.id).await
            } else {
                Err(AppError::StorageCorrupt {
                    message: "Skill uninstall recovery found changed or duplicate content".into(),
                })
            }
        }
        crate::state_db::FilesystemOperationPhase::FilesystemApplied => {
            let records = install::load_ledger_for_state(state).await?;
            if target_hash.is_some()
                || quarantine_hash.is_some()
                || records
                    .iter()
                    .any(|record| same_skill_install(record, &payload.previous))
            {
                Err(AppError::StorageCorrupt {
                    message: "Skill uninstall recovery found changed committed state".into(),
                })
            } else {
                database.commit_filesystem_operation(&operation.id).await
            }
        }
        crate::state_db::FilesystemOperationPhase::Committed => Ok(()),
    }
}

pub(crate) async fn recover_install_operations(state: &AppState) -> Result<(), AppError> {
    let Some(database) = state.completed_state_database().await? else {
        return Ok(());
    };
    for operation in database.pending_filesystem_operations().await? {
        if !matches!(operation.kind.as_str(), "skill_install" | "skill_update") {
            continue;
        }
        let payload = serde_json::from_value::<SkillInstallOperation>(operation.payload.clone())
            .map_err(|_| AppError::StorageCorrupt {
                message: "Skill install recovery payload is invalid".into(),
            })?;
        if payload
            .previous
            .as_ref()
            .is_some_and(|previous| !same_skill_install(previous, &payload.next))
        {
            return Err(AppError::StorageCorrupt {
                message: "Skill install recovery identity changed".into(),
            });
        }
        let (_, destination) = record_destination(&payload.next)?;
        let parent = destination
            .parent()
            .ok_or_else(|| AppError::StorageCorrupt {
                message: "Skill install recovery destination has no parent".into(),
            })?;
        let stage = parent.join(format!(".agency-skill-{}.stage", operation.id));
        let retired = parent.join(format!(".agency-skill-{}.previous", operation.id));
        let result = match operation.phase {
            crate::state_db::FilesystemOperationPhase::Prepared => {
                let destination_hash = skill_tree_hash(&destination)?;
                if destination_hash.as_deref() == Some(&payload.next.installed_hash) {
                    apply_recovered_install(state, &operation.id, &payload).await?;
                    if let Some(previous) = &payload.previous {
                        remove_recovery_directory(&retired, &previous.installed_hash)?;
                    } else if retired.exists() {
                        return Err(AppError::StorageCorrupt {
                            message: "Skill install recovery found an unexpected retired tree"
                                .into(),
                        });
                    }
                    remove_recovery_directory(&stage, &payload.next.installed_hash)?;
                    database.commit_filesystem_operation(&operation.id).await
                } else if let Some(previous) = &payload.previous {
                    if destination_hash.as_deref() == Some(&previous.installed_hash) {
                        remove_recovery_directory(&stage, &payload.next.installed_hash)?;
                        database.abort_filesystem_operation(&operation.id).await
                    } else if destination_hash.is_none()
                        && skill_tree_hash(&retired)?.as_deref() == Some(&previous.installed_hash)
                    {
                        std::fs::rename(&retired, &destination).map_err(|error| AppError::Io {
                            message: format!("restore retired Skill install: {error}"),
                        })?;
                        remove_recovery_directory(&stage, &payload.next.installed_hash)?;
                        database.abort_filesystem_operation(&operation.id).await
                    } else {
                        Err(AppError::StorageCorrupt {
                            message: "Skill install recovery found changed destination content"
                                .into(),
                        })
                    }
                } else if destination_hash.is_none() && stage.exists() {
                    if skill_tree_hash(&stage)?.as_deref() != Some(&payload.next.installed_hash) {
                        Err(AppError::StorageCorrupt {
                            message: "Skill install recovery found changed staged content".into(),
                        })
                    } else {
                        std::fs::rename(&stage, &destination).map_err(|error| AppError::Io {
                            message: format!("publish recovered Skill install: {error}"),
                        })?;
                        apply_recovered_install(state, &operation.id, &payload).await?;
                        database.commit_filesystem_operation(&operation.id).await
                    }
                } else if destination_hash.is_none() {
                    database.abort_filesystem_operation(&operation.id).await
                } else {
                    Err(AppError::StorageCorrupt {
                        message: "Skill install recovery found changed destination content".into(),
                    })
                }
            }
            crate::state_db::FilesystemOperationPhase::FilesystemApplied => {
                if skill_tree_hash(&destination)?.as_deref() != Some(&payload.next.installed_hash) {
                    Err(AppError::StorageCorrupt {
                        message: "Skill install recovery found changed committed content".into(),
                    })
                } else {
                    let records = install::load_ledger_for_state(state).await?;
                    if !records.iter().any(|record| record == &payload.next) {
                        Err(AppError::StorageCorrupt {
                            message: "Skill install recovery lost committed ledger metadata".into(),
                        })
                    } else {
                        if let Some(previous) = &payload.previous {
                            remove_recovery_directory(&retired, &previous.installed_hash)?;
                        }
                        remove_recovery_directory(&stage, &payload.next.installed_hash)?;
                        database.commit_filesystem_operation(&operation.id).await
                    }
                }
            }
            crate::state_db::FilesystemOperationPhase::Committed => Ok(()),
        };
        if let Err(error) = result {
            database
                .retain_filesystem_operation_error(&operation.id, &error.to_string())
                .await?;
            return Err(error);
        }
    }
    for operation in database.pending_filesystem_operations().await? {
        let result = match operation.kind.as_str() {
            "skill_disable" | "skill_enable" => {
                recover_move_operation(state, &database, &operation).await
            }
            "skill_uninstall" => recover_uninstall_operation(state, &database, &operation).await,
            _ => continue,
        };
        if let Err(error) = result {
            database
                .retain_filesystem_operation_error(&operation.id, &error.to_string())
                .await?;
            return Err(error);
        }
    }
    Ok(())
}

fn skill_record_index(
    records: &[SkillInstallRecord],
    source_id: &str,
    relative_path: &str,
    runtime: &str,
    project_path: &Option<String>,
) -> Result<usize, AppError> {
    records
        .iter()
        .position(|record| {
            record.source_id == source_id
                && record.relative_path == relative_path
                && record.runtime == runtime
                && &record.project_path == project_path
        })
        .ok_or_else(|| AppError::InvalidArgument {
            message: "unknown tracked skill installation".into(),
        })
}

fn authorized_project_base(
    project_path: Option<&str>,
    authorization: Option<&AuthorizedMcpProject>,
) -> Result<Option<PathBuf>, AppError> {
    match authorization {
        Some(authorization) => {
            if project_path != Some(authorization.identity()) {
                return Err(AppError::InvalidArgument {
                    message: "MCP project capability does not match the requested project".into(),
                });
            }
            Ok(Some(PathBuf::from(authorization.identity())))
        }
        None => project_path
            .map(|path| canonical_target_base(Path::new(path)))
            .transpose(),
    }
}

fn canonical_project_string_authorized(
    project_path: Option<&str>,
    authorization: Option<&AuthorizedMcpProject>,
) -> Result<Option<String>, AppError> {
    match authorization {
        Some(authorization) => {
            if project_path != Some(authorization.identity()) {
                return Err(AppError::InvalidArgument {
                    message: "MCP project capability does not match the requested project".into(),
                });
            }
            Ok(Some(authorization.identity().to_owned()))
        }
        None => canonical_project_string(project_path),
    }
}

fn canonical_project_string(project_path: Option<&str>) -> Result<Option<String>, AppError> {
    project_path
        .map(|path| stored_project_base(path).map(|path| path.to_string_lossy().into_owned()))
        .transpose()
}

fn stored_project_base(path: &str) -> Result<PathBuf, AppError> {
    let path = Path::new(path);
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(AppError::InvalidArgument {
            message: "tracked project path must be an absolute normalized path".into(),
        });
    }
    let canonical = path
        .ancestors()
        .find_map(|ancestor| {
            std::fs::canonicalize(ancestor)
                .ok()
                .map(|base| (ancestor, base))
        })
        .map(|(ancestor, base)| {
            path.strip_prefix(ancestor)
                .map(|suffix| base.join(suffix))
                .unwrap_or(base)
        })
        .ok_or_else(|| AppError::InvalidArgument {
            message: "tracked project path has no resolvable absolute ancestor".into(),
        })?;
    match std::fs::symlink_metadata(&canonical) {
        Ok(_) => canonical_target_base(&canonical),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(canonical),
        Err(error) => Err(AppError::Io {
            message: format!(
                "inspect tracked project path {}: {error}",
                canonical.display()
            ),
        }),
    }
}

fn record_destination(record: &SkillInstallRecord) -> Result<(PathBuf, PathBuf), AppError> {
    let project = record
        .project_path
        .as_deref()
        .map(stored_project_base)
        .transpose()?;
    let base = match (&record.scope[..], project.as_deref()) {
        ("project", Some(project)) => project.to_path_buf(),
        ("user", None) => {
            canonical_target_base(&dirs::home_dir().ok_or_else(|| AppError::Io {
                message: "could not resolve home directory".into(),
            })?)?
        }
        _ => {
            return Err(AppError::InvalidArgument {
                message: "tracked skill has inconsistent scope and project path".into(),
            })
        }
    };
    let destination =
        install::target_path(&base, project.as_deref(), &record.runtime, &record.name)?;
    reject_linked_destination_ancestors(&base, &destination)?;
    if record.dest != destination.to_string_lossy() {
        return Err(AppError::InvalidArgument {
            message: "tracked skill destination no longer matches its runtime, scope, and name"
                .into(),
        });
    }
    Ok((base, destination))
}

fn record_destination_authorized(
    record: &SkillInstallRecord,
    authorization: Option<&AuthorizedMcpProject>,
) -> Result<(PathBuf, PathBuf), AppError> {
    let Some(authorization) = authorization else {
        return record_destination(record);
    };
    if record.scope != "project" || record.project_path.as_deref() != Some(authorization.identity())
    {
        return Err(AppError::InvalidArgument {
            message: "tracked skill does not belong to the authorized MCP project".into(),
        });
    }
    let base = PathBuf::from(authorization.identity());
    let relative = install::project_target_path(&record.runtime, &record.name)?;
    let destination = base.join(relative);
    if record.dest != destination.to_string_lossy() {
        return Err(AppError::InvalidArgument {
            message: "tracked skill destination no longer matches its runtime, scope, and name"
                .into(),
        });
    }
    Ok((base, destination))
}

fn authorized_relative_path(
    authorization: &AuthorizedMcpProject,
    path: &Path,
) -> Result<PathBuf, AppError> {
    path.strip_prefix(Path::new(authorization.identity()))
        .map(Path::to_path_buf)
        .map_err(|_| AppError::InvalidArgument {
            message: "tracked skill path is outside the authorized MCP project".into(),
        })
}

fn mutation_tree_hash(
    authorization: Option<&AuthorizedMcpProject>,
    path: &Path,
) -> Result<Option<String>, AppError> {
    match authorization {
        Some(authorization) => install::project_tree_hash(
            authorization.root(),
            &authorized_relative_path(authorization, path)?,
        ),
        None => skill_tree_hash(path),
    }
}

fn mutation_rename(
    authorization: Option<&AuthorizedMcpProject>,
    source: &Path,
    destination: &Path,
) -> Result<(), AppError> {
    match authorization {
        Some(authorization) => install::rename_project_directory(
            authorization.root(),
            &authorized_relative_path(authorization, source)?,
            &authorized_relative_path(authorization, destination)?,
        ),
        None => install::disable_directory(source, destination),
    }
}

fn mutation_uninstall(
    authorization: Option<&AuthorizedMcpProject>,
    destination: &Path,
    backups: &Path,
    modified: bool,
) -> Result<Option<PathBuf>, AppError> {
    match authorization {
        Some(authorization) => install::uninstall_project_directory(
            authorization.root(),
            &authorized_relative_path(authorization, destination)?,
            backups,
            modified,
        ),
        None => install::uninstall_directory(destination, backups, modified),
    }
}

fn disabled_destination(
    record: &SkillInstallRecord,
    base: &Path,
    destination: &Path,
) -> Result<PathBuf, AppError> {
    let disabled = record
        .disabled_path
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| AppError::InvalidArgument {
            message: "skill is not disabled".into(),
        })?;
    let expected_parent = destination
        .parent()
        .ok_or_else(|| AppError::InvalidArgument {
            message: "skill destination has no parent".into(),
        })?;
    if disabled.parent() != Some(expected_parent)
        || !disabled
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(".agency-disabled-"))
    {
        return Err(AppError::InvalidArgument {
            message: "tracked disabled skill path is outside its managed destination".into(),
        });
    }
    reject_linked_destination_ancestors(base, &disabled)?;
    Ok(disabled)
}

fn disabled_destination_authorized(
    record: &SkillInstallRecord,
    base: &Path,
    destination: &Path,
    authorization: Option<&AuthorizedMcpProject>,
) -> Result<PathBuf, AppError> {
    let Some(authorization) = authorization else {
        return disabled_destination(record, base, destination);
    };
    if base != Path::new(authorization.identity()) {
        return Err(AppError::InvalidArgument {
            message: "tracked disabled skill is outside the authorized MCP project".into(),
        });
    }
    let disabled = record
        .disabled_path
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| AppError::InvalidArgument {
            message: "skill is not disabled".into(),
        })?;
    let expected_parent = destination
        .parent()
        .ok_or_else(|| AppError::InvalidArgument {
            message: "skill destination has no parent".into(),
        })?;
    if disabled.parent() != Some(expected_parent)
        || !disabled
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(".agency-disabled-"))
    {
        return Err(AppError::InvalidArgument {
            message: "tracked disabled skill path is outside its managed destination".into(),
        });
    }
    authorized_relative_path(authorization, &disabled)?;
    Ok(disabled)
}

fn skill_directory_present(path: &Path) -> Result<bool, AppError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(AppError::Io {
                message: format!("inspect skill directory {}: {error}", path.display()),
            })
        }
    };
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata_is_reparse_point(&metadata)
    {
        return Err(AppError::InvalidArgument {
            message: format!(
                "skill directory must be a real directory: {}",
                path.display()
            ),
        });
    }
    Ok(true)
}

fn skill_tree_hash(path: &Path) -> Result<Option<String>, AppError> {
    if !skill_directory_present(path)? {
        return Ok(None);
    }
    install::tree_hash(path).map(Some)
}

fn rollback_error(action: &str, original: AppError, rollback: AppError) -> AppError {
    AppError::Internal {
        message: format!("{action} failed: {original}; rollback failed: {rollback}"),
    }
}

async fn source_status(state: &AppState, record: &SkillInstallRecord) -> (bool, bool) {
    match resolve_skill_package(state, &record.source_id, &record.relative_path).await {
        Ok(package) => match install::validated_tree_hash(package.root(), package.files()) {
            Ok(hash) => (true, hash == record.source_hash),
            Err(_) => (false, false),
        },
        Err(_) => (false, false),
    }
}

pub(crate) async fn update_skill(
    state: &AppState,
    source_id: &str,
    relative_path: &str,
    runtime: &str,
    project_path: Option<&str>,
) -> Result<InstalledSkill, AppError> {
    update_skill_authorized(state, source_id, relative_path, runtime, project_path, None).await
}

pub(crate) async fn update_skill_authorized(
    state: &AppState,
    source_id: &str,
    relative_path: &str,
    runtime: &str,
    project_path: Option<&str>,
    project_authorization: Option<&AuthorizedMcpProject>,
) -> Result<InstalledSkill, AppError> {
    install_skill_authorized(
        state,
        source_id,
        relative_path,
        runtime,
        project_path,
        project_authorization,
    )
    .await
}

pub(crate) async fn disable_skill(
    state: &AppState,
    source_id: &str,
    relative_path: &str,
    runtime: &str,
    project_path: Option<&str>,
) -> Result<InstalledSkill, AppError> {
    disable_skill_authorized(state, source_id, relative_path, runtime, project_path, None).await
}

pub(crate) async fn disable_skill_authorized(
    state: &AppState,
    source_id: &str,
    relative_path: &str,
    runtime: &str,
    project_path: Option<&str>,
    project_authorization: Option<&AuthorizedMcpProject>,
) -> Result<InstalledSkill, AppError> {
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_skill_installs_async(state.app_data_dir.clone()).await?;
    let project = canonical_project_string_authorized(project_path, project_authorization)?;
    let mut records = install::load_ledger_for_state(state).await?;
    let index = skill_record_index(&records, source_id, relative_path, runtime, &project)?;
    if records[index].disabled_path.is_some() {
        let (base, destination) =
            record_destination_authorized(&records[index], project_authorization)?;
        let disabled = disabled_destination_authorized(
            &records[index],
            &base,
            &destination,
            project_authorization,
        )?;
        let hash = mutation_tree_hash(project_authorization, &disabled)?;
        return Ok(installed_view(
            &records[index],
            install::classify(
                hash.as_deref(),
                &records[index].installed_hash,
                true,
                true,
                true,
            ),
        ));
    }
    let (base, destination) =
        record_destination_authorized(&records[index], project_authorization)?;
    let disk_hash = mutation_tree_hash(project_authorization, &destination)?.ok_or_else(|| {
        AppError::InvalidArgument {
            message: "missing skills cannot be disabled".into(),
        }
    })?;
    if disk_hash != records[index].installed_hash {
        return Err(AppError::InvalidArgument {
            message: "modified skills cannot be disabled".into(),
        });
    }
    let disabled = destination
        .parent()
        .ok_or_else(|| AppError::InvalidArgument {
            message: "skill destination has no parent".into(),
        })?
        .join(format!(
            ".agency-disabled-{}-{}",
            Uuid::new_v4(),
            records[index].name
        ));
    let previous = records[index].clone();
    records[index].disabled_path = Some(disabled.to_string_lossy().into_owned());
    let database = state.completed_state_database().await?;
    let operation = if let Some(database) = &database {
        Some(
            database
                .prepare_filesystem_operation(
                    "skill_disable",
                    &SkillMoveOperation {
                        previous,
                        next: records[index].clone(),
                    },
                )
                .await?,
        )
    } else {
        None
    };
    mutation_rename(project_authorization, &destination, &disabled)?;
    let disabled_hash = match mutation_tree_hash(project_authorization, &disabled)? {
        Some(hash) => hash,
        None => {
            let error = AppError::Internal {
                message: "disabled skill disappeared during transition".into(),
            };
            return match mutation_rename(project_authorization, &disabled, &destination) {
                Ok(()) => Err(error),
                Err(rollback) => Err(rollback_error("disable skill", error, rollback)),
            };
        }
    };
    if disabled_hash != records[index].installed_hash {
        let error = AppError::InvalidArgument {
            message: "disabled skill changed during transition".into(),
        };
        return match mutation_rename(project_authorization, &disabled, &destination) {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error("disable skill", error, rollback)),
        };
    }
    if project_authorization.is_none() {
        reject_linked_destination_ancestors(&base, &disabled)?;
    }
    let save = match &operation {
        Some(operation) => {
            install::save_ledger_after_filesystem(state, &records, &operation.id).await
        }
        None => install::save_ledger_for_state(state, &records).await,
    };
    if let Err(error) = save {
        return match mutation_rename(project_authorization, &disabled, &destination) {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error("disable skill", error, rollback)),
        };
    }
    if let (Some(database), Some(operation)) = (&database, &operation) {
        database.commit_filesystem_operation(&operation.id).await?;
    }
    Ok(installed_view(
        &records[index],
        install::classify(
            Some(&disabled_hash),
            &records[index].installed_hash,
            true,
            true,
            true,
        ),
    ))
}

pub(crate) async fn enable_skill(
    state: &AppState,
    source_id: &str,
    relative_path: &str,
    runtime: &str,
    project_path: Option<&str>,
) -> Result<InstalledSkill, AppError> {
    enable_skill_authorized(state, source_id, relative_path, runtime, project_path, None).await
}

pub(crate) async fn enable_skill_authorized(
    state: &AppState,
    source_id: &str,
    relative_path: &str,
    runtime: &str,
    project_path: Option<&str>,
    project_authorization: Option<&AuthorizedMcpProject>,
) -> Result<InstalledSkill, AppError> {
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_skill_installs_async(state.app_data_dir.clone()).await?;
    let project = canonical_project_string_authorized(project_path, project_authorization)?;
    let mut records = install::load_ledger_for_state(state).await?;
    let index = skill_record_index(&records, source_id, relative_path, runtime, &project)?;
    let (base, destination) =
        record_destination_authorized(&records[index], project_authorization)?;
    let disabled = disabled_destination_authorized(
        &records[index],
        &base,
        &destination,
        project_authorization,
    )?;
    mutation_tree_hash(project_authorization, &disabled)?.ok_or_else(|| {
        AppError::InvalidArgument {
            message: "disabled skill is missing".into(),
        }
    })?;
    let previous = records[index].clone();
    records[index].disabled_path = None;
    let database = state.completed_state_database().await?;
    let operation = if let Some(database) = &database {
        Some(
            database
                .prepare_filesystem_operation(
                    "skill_enable",
                    &SkillMoveOperation {
                        previous,
                        next: records[index].clone(),
                    },
                )
                .await?,
        )
    } else {
        None
    };
    mutation_rename(project_authorization, &disabled, &destination)?;
    let disk_hash = match mutation_tree_hash(project_authorization, &destination)? {
        Some(hash) => hash,
        None => {
            let error = AppError::Internal {
                message: "enabled skill disappeared during transition".into(),
            };
            return match mutation_rename(project_authorization, &destination, &disabled) {
                Ok(()) => Err(error),
                Err(rollback) => Err(rollback_error("enable skill", error, rollback)),
            };
        }
    };
    let save = match &operation {
        Some(operation) => {
            install::save_ledger_after_filesystem(state, &records, &operation.id).await
        }
        None => install::save_ledger_for_state(state, &records).await,
    };
    if let Err(error) = save {
        return match mutation_rename(project_authorization, &destination, &disabled) {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error("enable skill", error, rollback)),
        };
    }
    if let (Some(database), Some(operation)) = (&database, &operation) {
        database.commit_filesystem_operation(&operation.id).await?;
    }
    let (source_available, source_current) = source_status(state, &records[index]).await;
    Ok(installed_view(
        &records[index],
        install::classify(
            Some(&disk_hash),
            &records[index].installed_hash,
            source_available,
            source_current,
            false,
        ),
    ))
}

pub(crate) async fn uninstall_skill(
    state: &AppState,
    source_id: &str,
    relative_path: &str,
    runtime: &str,
    project_path: Option<&str>,
) -> Result<bool, AppError> {
    uninstall_skill_authorized(state, source_id, relative_path, runtime, project_path, None).await
}

pub(crate) async fn uninstall_skill_authorized(
    state: &AppState,
    source_id: &str,
    relative_path: &str,
    runtime: &str,
    project_path: Option<&str>,
    project_authorization: Option<&AuthorizedMcpProject>,
) -> Result<bool, AppError> {
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_skill_installs_async(state.app_data_dir.clone()).await?;
    let project = canonical_project_string_authorized(project_path, project_authorization)?;
    let mut records = install::load_ledger_for_state(state).await?;
    let index = skill_record_index(&records, source_id, relative_path, runtime, &project)?;
    let record = records[index].clone();
    let (base, destination) = record_destination_authorized(&record, project_authorization)?;
    let target = if record.disabled_path.is_some() {
        disabled_destination_authorized(&record, &base, &destination, project_authorization)?
    } else {
        destination
    };
    let mut project_target = project_authorization
        .map(|authorization| {
            install::project_directory_capability(
                authorization.root(),
                &authorized_relative_path(authorization, &target)?,
            )
        })
        .transpose()?;
    let old_records = records.clone();
    records.remove(index);
    before_uninstall_quarantine(&target);
    let target_hash = match project_target.as_ref() {
        Some(capability) => install::project_capability_tree_hash(capability)?,
        None => mutation_tree_hash(None, &target)?,
    };
    if target_hash.is_none() {
        after_missing_uninstall_validation(&target);
        install::save_ledger_for_state(state, &records).await?;
        return Ok(true);
    }
    let expected_target_hash = target_hash.expect("checked present Skill target hash");
    let quarantine = target
        .parent()
        .ok_or_else(|| AppError::InvalidArgument {
            message: "skill target has no parent".into(),
        })?
        .join(format!(
            ".agency-uninstall-{}-{}",
            Uuid::new_v4(),
            record.name
        ));
    let database = state.completed_state_database().await?;
    let operation = if let Some(database) = &database {
        Some(
            database
                .prepare_filesystem_operation(
                    "skill_uninstall",
                    &SkillUninstallOperation {
                        previous: record.clone(),
                        target_hash: expected_target_hash.clone(),
                        quarantine: quarantine.to_string_lossy().into_owned(),
                    },
                )
                .await?,
        )
    } else {
        None
    };
    if let Some(capability) = project_target.as_mut() {
        install::rename_project_capability(
            capability,
            quarantine
                .file_name()
                .expect("generated quarantine has a name")
                .to_os_string(),
        )?;
    } else {
        mutation_rename(None, &target, &quarantine)?;
    }
    let disk_hash = match project_target.as_ref() {
        Some(capability) => install::project_capability_tree_hash(capability),
        None => mutation_tree_hash(None, &quarantine),
    };
    let disk_hash = match disk_hash {
        Ok(Some(hash)) => hash,
        Ok(None) => {
            let error = AppError::Internal {
                message: "quarantined skill disappeared during uninstall".into(),
            };
            let restore = match project_target.as_mut() {
                Some(capability) => install::rename_project_capability(
                    capability,
                    target
                        .file_name()
                        .expect("skill target has a name")
                        .to_os_string(),
                ),
                None => mutation_rename(None, &quarantine, &target),
            };
            return match restore {
                Ok(()) => Err(error),
                Err(rollback) => Err(rollback_error("uninstall skill", error, rollback)),
            };
        }
        Err(error) => {
            let restore = match project_target.as_mut() {
                Some(capability) => install::rename_project_capability(
                    capability,
                    target
                        .file_name()
                        .expect("skill target has a name")
                        .to_os_string(),
                ),
                None => mutation_rename(None, &quarantine, &target),
            };
            return match restore {
                Ok(()) => Err(error),
                Err(rollback) => Err(rollback_error("uninstall skill", error, rollback)),
            };
        }
    };
    if disk_hash != expected_target_hash {
        let error = AppError::StorageCorrupt {
            message: "skill changed while it was being quarantined".into(),
        };
        let restore = match project_target.as_mut() {
            Some(capability) => install::rename_project_capability(
                capability,
                target
                    .file_name()
                    .expect("skill target has a name")
                    .to_os_string(),
            ),
            None => mutation_rename(None, &quarantine, &target),
        };
        return match restore {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error("uninstall skill", error, rollback)),
        };
    }
    let modified = disk_hash != record.installed_hash;
    if operation.is_none() {
        if let Err(error) = install::save_ledger_for_state(state, &records).await {
            let restore = match project_target.as_mut() {
                Some(capability) => install::rename_project_capability(
                    capability,
                    target
                        .file_name()
                        .expect("skill target has a name")
                        .to_os_string(),
                ),
                None => mutation_rename(None, &quarantine, &target),
            };
            return match restore {
                Ok(()) => Err(error),
                Err(rollback) => Err(rollback_error("uninstall skill", error, rollback)),
            };
        }
    }
    let removal = match project_target.as_ref() {
        Some(capability) => install::uninstall_project_capability(
            capability,
            &state.app_data_dir.join("skill-backups"),
            modified,
        ),
        None => mutation_uninstall(
            None,
            &quarantine,
            &state.app_data_dir.join("skill-backups"),
            modified,
        ),
    };
    if let Err(error) = removal {
        #[cfg(windows)]
        if project_target.is_some() {
            return Err(error);
        }
        let ledger = if operation.is_some() {
            Ok(())
        } else {
            install::save_ledger_for_state(state, &old_records).await
        };
        let restore = match project_target.as_mut() {
            Some(capability) => install::rename_project_capability(
                capability,
                target
                    .file_name()
                    .expect("skill target has a name")
                    .to_os_string(),
            ),
            None => mutation_rename(None, &quarantine, &target),
        };
        return match (ledger, restore) {
            (Ok(()), Ok(())) => Err(error),
            (Err(rollback), Ok(())) | (Ok(()), Err(rollback)) => {
                Err(rollback_error("uninstall skill", error, rollback))
            }
            (Err(ledger), Err(restore)) => Err(AppError::Internal {
                message: format!(
                    "uninstall skill failed: {error}; ledger rollback failed: {ledger}; target restore failed: {restore}"
                ),
            }),
        };
    }
    if let (Some(database), Some(operation)) = (&database, &operation) {
        install::save_ledger_after_filesystem(state, &records, &operation.id).await?;
        database.commit_filesystem_operation(&operation.id).await?;
    }
    Ok(true)
}

#[tauri::command]
pub async fn skill_backups_list(state: State<'_, AppState>) -> Result<Vec<String>, AppError> {
    let directory = state.app_data_dir.join("skill-backups");
    let mut backups = match std::fs::read_dir(&directory) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.path().to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            return Err(AppError::Io {
                message: format!("read skill backups {}: {error}", directory.display()),
            })
        }
    };
    backups.sort();
    Ok(backups)
}

fn skill_version_identity(
    source_id: &str,
    relative_path: &str,
    runtime: &str,
    project_path: Option<String>,
) -> Result<SkillVersionIdentity, AppError> {
    crate::library::validate_reference(source_id, relative_path)?;
    let relative_path = crate::library::normalize_relative_path(relative_path)?;
    install::project_target_path(runtime, "history")?;
    Ok(SkillVersionIdentity {
        source_id: source_id.into(),
        relative_path,
        runtime: runtime.into(),
        project_path,
    })
}

fn skill_version_identity_for_record(
    record: &SkillInstallRecord,
) -> Result<SkillVersionIdentity, AppError> {
    let canonical_project = canonical_project_string(record.project_path.as_deref())?;
    if canonical_project != record.project_path {
        return Err(AppError::InvalidArgument {
            message: "tracked Skill project identity is not canonical".into(),
        });
    }
    skill_version_identity(
        &record.source_id,
        &record.relative_path,
        &record.runtime,
        canonical_project,
    )
}

fn skill_version_identity_hash(identity: &SkillVersionIdentity) -> Result<String, AppError> {
    let bytes = serde_json::to_vec(identity).map_err(|error| AppError::Internal {
        message: format!("serialize Skill version identity: {error}"),
    })?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn skill_history_root(state: &AppState) -> PathBuf {
    state.app_data_dir.join("skills/history")
}

fn skill_history_directory(
    state: &AppState,
    identity: &SkillVersionIdentity,
) -> Result<PathBuf, AppError> {
    Ok(skill_history_root(state).join(skill_version_identity_hash(identity)?))
}

fn require_real_directory(path: &Path, label: &str) -> Result<(), AppError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| AppError::Io {
        message: format!("inspect {label} {}: {error}", path.display()),
    })?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata_is_reparse_point(&metadata)
    {
        return Err(AppError::InvalidArgument {
            message: format!("{label} must be a real directory"),
        });
    }
    Ok(())
}

fn require_regular_file(path: &Path, label: &str) -> Result<(), AppError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| AppError::Io {
        message: format!("inspect {label} {}: {error}", path.display()),
    })?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata_is_reparse_point(&metadata)
    {
        return Err(AppError::InvalidArgument {
            message: format!("{label} must be a regular file"),
        });
    }
    Ok(())
}

fn ensure_skill_history_directory(
    state: &AppState,
    identity: &SkillVersionIdentity,
) -> Result<PathBuf, AppError> {
    let root = skill_history_root(state);
    std::fs::create_dir_all(&root).map_err(|error| AppError::Io {
        message: format!("create Skill history directory {}: {error}", root.display()),
    })?;
    require_real_directory(&root, "Skill history root")?;
    let canonical_app =
        std::fs::canonicalize(&state.app_data_dir).map_err(|error| AppError::Io {
            message: format!("resolve app data directory: {error}"),
        })?;
    let canonical_root = std::fs::canonicalize(&root).map_err(|error| AppError::Io {
        message: format!("resolve Skill history root: {error}"),
    })?;
    if !canonical_root.starts_with(&canonical_app) {
        return Err(AppError::InvalidArgument {
            message: "Skill history root escaped app data".into(),
        });
    }
    let directory = skill_history_directory(state, identity)?;
    std::fs::create_dir_all(&directory).map_err(|error| AppError::Io {
        message: format!(
            "create Skill identity history {}: {error}",
            directory.display()
        ),
    })?;
    require_real_directory(&directory, "Skill identity history")?;
    let canonical_directory = std::fs::canonicalize(&directory).map_err(|error| AppError::Io {
        message: format!("resolve Skill identity history: {error}"),
    })?;
    if canonical_directory.parent() != Some(canonical_root.as_path()) {
        return Err(AppError::InvalidArgument {
            message: "Skill identity history escaped its root".into(),
        });
    }
    Ok(directory)
}

fn validated_skill_snapshot_inventory(
    content: &Path,
) -> Result<(Vec<SkillPackageFile>, String), AppError> {
    require_real_directory(content, "Skill snapshot content")?;
    let mut errors = Vec::new();
    let files = inventory_package(content, &mut errors);
    if files.is_empty()
        || errors
            .iter()
            .any(|error| error.code != SkillValidationCode::TrustRequired)
    {
        return Err(AppError::InvalidArgument {
            message: "Skill snapshot contains an unsafe, oversized, or unreadable entry".into(),
        });
    }
    let hash = install::validated_tree_hash(content, &files)?;
    Ok((files, hash))
}

async fn validate_skill_version_snapshot(
    state: &AppState,
    identity: &SkillVersionIdentity,
    snapshot_path: &Path,
) -> Result<(SkillVersionSnapshot, PathBuf, Vec<SkillPackageFile>), AppError> {
    require_real_directory(snapshot_path, "Skill version snapshot")?;
    let directory = skill_history_directory(state, identity)?;
    require_real_directory(&directory, "Skill identity history")?;
    let canonical_directory = std::fs::canonicalize(&directory).map_err(|error| AppError::Io {
        message: format!("resolve Skill identity history: {error}"),
    })?;
    let canonical_snapshot =
        std::fs::canonicalize(snapshot_path).map_err(|error| AppError::Io {
            message: format!("resolve Skill version snapshot: {error}"),
        })?;
    if canonical_snapshot.parent() != Some(canonical_directory.as_path())
        || canonical_snapshot
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| Uuid::parse_str(name).ok())
            .is_none()
    {
        return Err(AppError::InvalidArgument {
            message: "Skill version snapshot does not belong to this exact install".into(),
        });
    }
    let manifest_path = canonical_snapshot.join("manifest.json");
    require_regular_file(&manifest_path, "Skill version manifest")?;
    let manifest_bytes = read_capped(&manifest_path, MAX_SKILL_HISTORY_MANIFEST_BYTES).await?;
    let manifest: SkillVersionManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|error| AppError::InvalidArgument {
            message: format!("Skill version manifest is invalid: {error}"),
        })?;
    if manifest.version != 1
        || manifest.identity != *identity
        || manifest.content_hash.len() != 64
        || !manifest
            .content_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || chrono::DateTime::parse_from_rfc3339(&manifest.created_at).is_err()
    {
        return Err(AppError::InvalidArgument {
            message: "Skill version manifest identity or metadata changed".into(),
        });
    }
    let content = canonical_snapshot.join("content");
    let (files, content_hash) = validated_skill_snapshot_inventory(&content)?;
    if content_hash != manifest.content_hash {
        return Err(AppError::InvalidArgument {
            message: "Skill version snapshot content failed verification".into(),
        });
    }
    Ok((
        SkillVersionSnapshot {
            path: canonical_snapshot.to_string_lossy().into_owned(),
            created_at: manifest.created_at,
            content_hash,
        },
        content,
        files,
    ))
}

async fn exact_skill_version_snapshots(
    state: &AppState,
    identity: &SkillVersionIdentity,
) -> Result<Vec<SkillVersionSnapshot>, AppError> {
    let directory = skill_history_directory(state, identity)?;
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| AppError::Io {
                message: format!("read Skill identity history: {error}"),
            })?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(AppError::Io {
                message: format!("read Skill identity history: {error}"),
            })
        }
    };
    if entries.len() > MAX_SKILL_HISTORY_SCAN_ENTRIES {
        return Err(AppError::InvalidArgument {
            message: "Skill identity history exceeds its bounded entry limit".into(),
        });
    }
    let mut snapshots = Vec::new();
    for entry in entries {
        if let Ok((snapshot, _, _)) =
            validate_skill_version_snapshot(state, identity, &entry.path()).await
        {
            snapshots.push(snapshot);
        }
    }
    snapshots.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.path.cmp(&left.path))
    });
    Ok(snapshots)
}

async fn create_skill_version_snapshot(
    state: &AppState,
    record: &SkillInstallRecord,
    source: &Path,
    project_capability: Option<&install::ProjectDirectoryCapability>,
) -> Result<SkillVersionSnapshot, AppError> {
    let identity = skill_version_identity_for_record(record)?;
    let directory = ensure_skill_history_directory(state, &identity)?;
    let snapshot_directory = directory.join(Uuid::new_v4().to_string());
    std::fs::create_dir(&snapshot_directory).map_err(|error| AppError::Io {
        message: format!("create Skill version snapshot: {error}"),
    })?;
    let content = snapshot_directory.join("content");
    let result = async {
        match project_capability {
            Some(capability) => install::copy_project_capability_snapshot(capability, &content)?,
            None => {
                let (files, _) = validated_skill_snapshot_inventory(source)?;
                install::install_validated_directory(
                    source,
                    &files,
                    &content,
                    &state.app_data_dir.join("skill-backups"),
                    false,
                )?;
            }
        }
        let (_, content_hash) = validated_skill_snapshot_inventory(&content)?;
        if content_hash != record.installed_hash {
            return Err(AppError::InvalidArgument {
                message: "Skill changed before its version snapshot was created".into(),
            });
        }
        let created_at = chrono::Utc::now().to_rfc3339();
        let manifest = SkillVersionManifest {
            version: 1,
            identity: identity.clone(),
            content_hash: content_hash.clone(),
            created_at: created_at.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&manifest).map_err(|error| AppError::Internal {
            message: format!("serialize Skill version manifest: {error}"),
        })?;
        if bytes.len() as u64 > MAX_SKILL_HISTORY_MANIFEST_BYTES {
            return Err(AppError::Internal {
                message: "Skill version manifest exceeds its bounded size".into(),
            });
        }
        atomic_write(&snapshot_directory.join("manifest.json"), &bytes).await?;
        Ok(SkillVersionSnapshot {
            path: snapshot_directory.to_string_lossy().into_owned(),
            created_at,
            content_hash,
        })
    }
    .await;
    let snapshot = match result {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&snapshot_directory);
            return Err(error);
        }
    };
    let snapshots = exact_skill_version_snapshots(state, &identity).await?;
    for retired in snapshots.into_iter().skip(MAX_SKILL_HISTORY_ENTRIES) {
        let retired = PathBuf::from(retired.path);
        if retired.parent() == Some(directory.as_path()) {
            let _ = std::fs::remove_dir_all(retired);
        }
    }
    Ok(snapshot)
}

pub(crate) async fn skill_version_history(
    state: &AppState,
    source_id: &str,
    relative_path: &str,
    runtime: &str,
    project_path: Option<&str>,
) -> Result<Vec<SkillVersionSnapshot>, AppError> {
    let project = canonical_project_string(project_path)?;
    let records = install::load_ledger_for_state(state).await?;
    let record =
        &records[skill_record_index(&records, source_id, relative_path, runtime, &project)?];
    let identity = skill_version_identity_for_record(record)?;
    let mut snapshots = exact_skill_version_snapshots(state, &identity).await?;
    snapshots.truncate(MAX_SKILL_HISTORY_ENTRIES);
    Ok(snapshots)
}

pub(crate) async fn rollback_skill_authorized(
    state: &AppState,
    source_id: &str,
    relative_path: &str,
    runtime: &str,
    project_path: Option<&str>,
    snapshot_path: &str,
    project_authorization: Option<&AuthorizedMcpProject>,
) -> Result<InstalledSkill, AppError> {
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_skill_installs_async(state.app_data_dir.clone()).await?;
    let project = canonical_project_string_authorized(project_path, project_authorization)?;
    let mut records = install::load_ledger_for_state(state).await?;
    let index = skill_record_index(&records, source_id, relative_path, runtime, &project)?;
    let old_records = records.clone();
    let record = records[index].clone();
    let (_, destination) = record_destination_authorized(&record, project_authorization)?;
    let identity = skill_version_identity_for_record(&record)?;
    let selected_path = PathBuf::from(snapshot_path);
    validate_skill_version_snapshot(state, &identity, &selected_path).await?;
    let project_capability = project_authorization
        .map(|authorization| {
            install::project_directory_capability(
                authorization.root(),
                &install::project_target_path(runtime, &record.name)?,
            )
        })
        .transpose()?;
    create_skill_version_snapshot(state, &record, &destination, project_capability.as_ref())
        .await?;
    let (selected, content, files) =
        validate_skill_version_snapshot(state, &identity, &selected_path).await?;

    let backup_root = state.app_data_dir.join("skill-backups");
    records[index].source_hash = selected.content_hash.clone();
    records[index].installed_hash = selected.content_hash;
    records[index].installed_at = chrono::Utc::now().to_rfc3339();
    install::save_ledger_for_state(state, &records).await?;

    let install_result = match project_authorization {
        Some(authorization) => install::install_validated_directory_in_project(
            authorization.root(),
            &content,
            &files,
            &install::project_target_path(runtime, &record.name)?,
            &backup_root,
            true,
        ),
        None => {
            install::install_validated_directory(&content, &files, &destination, &backup_root, true)
        }
    };
    if let Err(error) = install_result {
        return match install::save_ledger_for_state(state, &old_records).await {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error("rollback skill", error, rollback)),
        };
    }
    Ok(installed_view(&records[index], SkillInstallState::Current))
}

#[tauri::command]
pub async fn skill_install_plan(
    state: State<'_, AppState>,
    source_id: String,
    relative_path: String,
    runtime: String,
    project_path: Option<String>,
) -> Result<SkillMutationPlan, AppError> {
    plan_skill_install(
        &state,
        &source_id,
        &relative_path,
        &runtime,
        project_path.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn skill_install_with_dependencies(
    state: State<'_, AppState>,
    source_id: String,
    relative_path: String,
    runtime: String,
    project_path: Option<String>,
) -> Result<Vec<InstalledSkill>, AppError> {
    install_skill_with_dependencies(
        &state,
        &source_id,
        &relative_path,
        &runtime,
        project_path.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn skill_collection_batch(
    state: State<'_, AppState>,
    collection_name: String,
    operation: String,
    runtime: String,
    project_path: Option<String>,
) -> Result<SkillBatchResult, AppError> {
    batch_collection(
        &state,
        &collection_name,
        &operation,
        &runtime,
        project_path.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn skill_version_history_list(
    state: State<'_, AppState>,
    source_id: String,
    relative_path: String,
    runtime: String,
    project_path: Option<String>,
) -> Result<Vec<SkillVersionSnapshot>, AppError> {
    skill_version_history(
        &state,
        &source_id,
        &relative_path,
        &runtime,
        project_path.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn skill_version_rollback(
    state: State<'_, AppState>,
    source_id: String,
    relative_path: String,
    runtime: String,
    project_path: Option<String>,
    snapshot_path: String,
) -> Result<InstalledSkill, AppError> {
    rollback_skill_authorized(
        &state,
        &source_id,
        &relative_path,
        &runtime,
        project_path.as_deref(),
        &snapshot_path,
        None,
    )
    .await
}

pub(crate) async fn reconcile_skill_installs(
    state: &AppState,
    project_paths: &[String],
) -> Result<Vec<InstalledSkill>, AppError> {
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_skill_installs_async(state.app_data_dir.clone()).await?;
    let sources = inspect_skill_sources(state).await?;
    let mut packages = HashMap::new();
    for result in &sources {
        for package in result.packages.iter().filter(|package| package.installable) {
            if let Some(name) = &package.name {
                let root = resolved_source_root(&result.source)?;
                let hash = install::tree_hash(&root.join(&package.relative_path))?;
                packages.insert(
                    (result.source.id.clone(), package.relative_path.clone()),
                    (name.clone(), hash),
                );
            }
        }
    }

    let home = dirs::home_dir().ok_or_else(|| AppError::Io {
        message: "could not resolve home directory".into(),
    })?;
    let home = canonical_target_base(&home)?;
    let projects = project_paths
        .iter()
        .map(|path| canonical_target_base(Path::new(path)))
        .collect::<Result<Vec<_>, _>>()?;
    let records = install::load_ledger_for_state(state).await?;
    let mut output = Vec::new();
    let mut tracked_destinations = HashSet::new();
    for record in &records {
        let (base, destination) = record_destination(record)?;
        tracked_destinations.insert(destination.to_string_lossy().into_owned());
        let source = packages.get(&(record.source_id.clone(), record.relative_path.clone()));
        let disk_hash = if record.disabled_path.is_some() {
            let disabled = disabled_destination(record, &base, &destination)?;
            skill_tree_hash(&disabled)?
        } else if std::fs::symlink_metadata(&record.dest).is_ok() {
            skill_tree_hash(&destination)?
        } else {
            None
        };
        let state = install::classify(
            disk_hash.as_deref(),
            &record.installed_hash,
            source.is_some(),
            source.is_some_and(|(_, hash)| hash == &record.source_hash),
            record.disabled_path.is_some(),
        );
        output.push(installed_view(record, state));
    }

    for result in sources {
        for package in result
            .packages
            .into_iter()
            .filter(|package| package.installable)
        {
            let Some(name) = package.name else { continue };
            for runtime in ["claudeCode", "codex"] {
                let mut targets = vec![(None, home.clone())];
                targets.extend(
                    projects
                        .iter()
                        .cloned()
                        .map(|project| (Some(project.to_string_lossy().into_owned()), project)),
                );
                for (project_path, base) in targets {
                    let project = project_path.is_some().then_some(base.as_path());
                    let destination = install::target_path(&home, project, runtime, &name)?;
                    let dest = destination.to_string_lossy().into_owned();
                    if tracked_destinations.contains(&dest)
                        || std::fs::symlink_metadata(&destination).is_err()
                    {
                        continue;
                    }
                    tracked_destinations.insert(dest.clone());
                    output.push(InstalledSkill {
                        source_id: result.source.id.clone(),
                        relative_path: package.relative_path.clone(),
                        name: name.clone(),
                        runtime: runtime.into(),
                        scope: if project_path.is_some() {
                            "project"
                        } else {
                            "user"
                        }
                        .into(),
                        project_path,
                        path: dest,
                        state: SkillInstallState::Foreign,
                        tracked: false,
                    });
                }
            }
        }
    }
    output.sort_by(|left, right| {
        (&left.name, &left.runtime, &left.project_path).cmp(&(
            &right.name,
            &right.runtime,
            &right.project_path,
        ))
    });
    Ok(output)
}

#[tauri::command]
pub async fn skill_install(
    state: State<'_, AppState>,
    source_id: String,
    relative_path: String,
    runtime: String,
    project_path: Option<String>,
) -> Result<InstalledSkill, AppError> {
    install_skill(
        &state,
        &source_id,
        &relative_path,
        &runtime,
        project_path.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn skill_update(
    state: State<'_, AppState>,
    source_id: String,
    relative_path: String,
    runtime: String,
    project_path: Option<String>,
) -> Result<InstalledSkill, AppError> {
    update_skill(
        &state,
        &source_id,
        &relative_path,
        &runtime,
        project_path.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn skill_disable(
    state: State<'_, AppState>,
    source_id: String,
    relative_path: String,
    runtime: String,
    project_path: Option<String>,
) -> Result<InstalledSkill, AppError> {
    disable_skill(
        &state,
        &source_id,
        &relative_path,
        &runtime,
        project_path.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn skill_enable(
    state: State<'_, AppState>,
    source_id: String,
    relative_path: String,
    runtime: String,
    project_path: Option<String>,
) -> Result<InstalledSkill, AppError> {
    enable_skill(
        &state,
        &source_id,
        &relative_path,
        &runtime,
        project_path.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn skill_uninstall(
    state: State<'_, AppState>,
    source_id: String,
    relative_path: String,
    runtime: String,
    project_path: Option<String>,
) -> Result<bool, AppError> {
    uninstall_skill(
        &state,
        &source_id,
        &relative_path,
        &runtime,
        project_path.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn skill_installs_reconcile(
    state: State<'_, AppState>,
    project_paths: Vec<String>,
) -> Result<Vec<InstalledSkill>, AppError> {
    reconcile_skill_installs(&state, &project_paths).await
}

#[tauri::command]
pub async fn skill_source_add_local(
    state: State<'_, AppState>,
    root: String,
) -> Result<SkillSource, AppError> {
    add_local_source(&state, Path::new(&root)).await
}

#[tauri::command]
pub async fn skill_source_add_github(
    state: State<'_, AppState>,
    repository: String,
    git_ref: Option<String>,
    subdirectory: Option<String>,
) -> Result<SkillSource, AppError> {
    add_github_source(
        &state,
        &repository,
        git_ref.as_deref(),
        subdirectory.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn skill_source_refresh(
    state: State<'_, AppState>,
    source_id: String,
) -> Result<SkillSourceResult, AppError> {
    refresh_skill_source(&state, &source_id).await
}

#[tauri::command]
pub async fn skill_source_remove(
    state: State<'_, AppState>,
    source_id: String,
) -> Result<bool, AppError> {
    remove_skill_source(&state, &source_id).await
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use ed25519_dalek::{Signer, SigningKey};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use std::time::Duration;

    use serde_json::json;
    use sha2::Digest;
    use tempfile::tempdir;

    use crate::types::SkillType;

    use super::{
        add_github_source, add_local_source, apply_skill_trust, discover_source,
        discover_source_blocking, ensure_local_source, inspect_skill_sources,
        is_windows_reparse_point, load_or_create_trust_key_with, load_skill_sources,
        load_skill_sources_for_state, read_skill_file, refresh_git_source_from,
        remove_skill_source, reset_refresh_fs_probe, resolve_skill_package, sign_trust_record,
        skill_destination_presence, skill_sources_path, skill_sources_spec, take_refresh_fs_probe,
        trust_fingerprint, validate_imported_skill_trust, validate_package, verify_publisher,
        SkillPublisherMetadata, SkillTrustRecord, MAX_SKILL_FILES, MAX_SKILL_FILE_BYTES,
    };

    #[test]
    fn publisher_signature_covers_identity_version_channel_and_skill_body() {
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let mut payload = sha2::Sha256::new();
        for part in ["Acme", "reviewer", "1.2.3", "stable", "\nInstructions\n"] {
            payload.update(part.as_bytes());
            if part != "\nInstructions\n" {
                payload.update([0]);
            }
        }
        let publisher = SkillPublisherMetadata {
            name: "Acme".into(),
            public_key: base64::engine::general_purpose::STANDARD
                .encode(signing_key.verifying_key().to_bytes()),
            signature: base64::engine::general_purpose::STANDARD
                .encode(signing_key.sign(&payload.finalize()).to_bytes()),
        };
        let skill = "---\nname: reviewer\n---\nInstructions\n";
        assert!(verify_publisher(
            &publisher,
            "reviewer",
            "1.2.3",
            "stable",
            skill,
            &[]
        ));
        assert!(!verify_publisher(
            &publisher,
            "reviewer",
            "1.2.4",
            "stable",
            skill,
            &[]
        ));
    }

    #[test]
    fn source_state_lock_serializes_independent_writers() {
        let root = tempdir().expect("app data");
        let first = super::lock_skill_sources(root.path()).expect("first lock");
        let path = root.path().to_path_buf();
        let (tx, rx) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let _second = super::lock_skill_sources(&path).expect("second lock");
            tx.send(()).expect("signal acquired");
        });

        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
        drop(first);
        rx.recv_timeout(Duration::from_secs(2))
            .expect("second writer acquires after release");
        waiter.join().expect("waiter");
    }

    #[tokio::test]
    async fn sqlite_sources_preserve_independent_state_updates() {
        let app = tempdir().expect("app data");
        let left_root = tempdir().expect("left source");
        let right_root = tempdir().expect("right source");
        let database = crate::state_db::StateDatabase::open(app.path()).expect("open database");
        database
            .mutate(skill_sources_spec(), Vec::new(), |_| Ok(()))
            .await
            .expect("seed skill sources");
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .expect("complete migration");
        let left = test_state(app.path());
        let right = test_state(app.path());

        let (left_result, right_result) = tokio::join!(
            add_local_source(&left, left_root.path()),
            add_local_source(&right, right_root.path()),
        );
        left_result.expect("add left source");
        right_result.expect("add right source");

        assert_eq!(
            load_skill_sources_for_state(&left)
                .await
                .expect("load shared sources")
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn sqlite_skill_install_commits_a_filesystem_operation() {
        let app = tempdir().expect("app data");
        let source = tempdir().expect("source");
        let project = tempdir().expect("project");
        let database = crate::state_db::StateDatabase::open(app.path()).expect("open database");
        database
            .mutate(skill_sources_spec(), Vec::new(), |_| Ok(()))
            .await
            .expect("seed skill sources");
        let installs = crate::state_db::DocumentSpec::<Vec<crate::types::SkillInstallRecord>>::new(
            "skill_installs",
            1,
            16_777_216,
            |_| Ok(()),
        );
        database
            .mutate(installs, Vec::new(), |_| Ok(()))
            .await
            .expect("seed skill installs");
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .expect("complete migration");
        let state = test_state(app.path());
        write_skill(source.path(), "reviewer", "reviewer", "Reviews changes");
        let registered = add_local_source(&state, source.path())
            .await
            .expect("register source");

        super::install_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(project.path().to_string_lossy().as_ref()),
        )
        .await
        .expect("install skill");
        super::disable_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(project.path().to_string_lossy().as_ref()),
        )
        .await
        .expect("disable skill");
        super::enable_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(project.path().to_string_lossy().as_ref()),
        )
        .await
        .expect("enable skill");
        super::uninstall_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(project.path().to_string_lossy().as_ref()),
        )
        .await
        .expect("uninstall skill");

        let connection =
            rusqlite::Connection::open(app.path().join("state/agency-agents.sqlite3")).unwrap();
        for kind in [
            "skill_install",
            "skill_disable",
            "skill_enable",
            "skill_uninstall",
        ] {
            let count: u32 = connection
                .query_row(
                    "SELECT count(*) FROM filesystem_operations \
                     WHERE kind = ?1 AND phase = 'committed'",
                    [kind],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "missing {kind}");
        }
    }

    #[tokio::test]
    async fn prepared_skill_install_recovery_rolls_forward_exact_content_once() {
        let app = tempdir().expect("app data");
        let source = tempdir().expect("source");
        let project = tempdir().expect("project");
        let database = crate::state_db::StateDatabase::open(app.path()).expect("open database");
        database
            .mutate(skill_sources_spec(), Vec::new(), |_| Ok(()))
            .await
            .unwrap();
        let installs = crate::state_db::DocumentSpec::<Vec<crate::types::SkillInstallRecord>>::new(
            "skill_installs",
            1,
            16_777_216,
            |_| Ok(()),
        );
        database
            .mutate(installs, Vec::new(), |_| Ok(()))
            .await
            .unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let state = test_state(app.path());
        write_skill(source.path(), "reviewer", "reviewer", "Reviews changes");
        let registered = add_local_source(&state, source.path()).await.unwrap();
        let package = resolve_skill_package(&state, &registered.id, "reviewer")
            .await
            .unwrap();
        let hash = super::install::validated_tree_hash(package.root(), package.files()).unwrap();
        let project_root = std::fs::canonicalize(project.path()).unwrap();
        let destination = project_root.join(".agents/skills/reviewer");
        let record = crate::types::SkillInstallRecord {
            source_id: registered.id,
            relative_path: "reviewer".into(),
            name: "reviewer".into(),
            runtime: "codex".into(),
            scope: "project".into(),
            project_path: Some(project_root.to_string_lossy().into_owned()),
            dest: destination.to_string_lossy().into_owned(),
            source_hash: hash.clone(),
            installed_hash: hash,
            installed_at: chrono::Utc::now().to_rfc3339(),
            disabled_path: None,
        };
        let operation = database
            .prepare_filesystem_operation(
                "skill_install",
                &super::SkillInstallOperation {
                    previous: None,
                    next: record.clone(),
                },
            )
            .await
            .unwrap();
        super::install::install_validated_directory_with_id(
            package.root(),
            package.files(),
            &destination,
            &app.path().join("skill-backups"),
            false,
            &operation.id,
        )
        .unwrap();

        super::recover_install_operations(&state).await.unwrap();
        super::recover_install_operations(&state).await.unwrap();

        assert_eq!(
            super::install::load_ledger_for_state(&state).await.unwrap(),
            [record]
        );
        assert!(database
            .pending_filesystem_operations()
            .await
            .unwrap()
            .is_empty());
    }
    use crate::commands::settings::{Settings, SettingsLoadState};
    use crate::error::AppError;
    use crate::github::auth::KeychainSlot;
    use crate::state::AppState;
    use crate::types::{SkillInstallState, SkillSource, SkillSourceKind, SkillValidationCode};

    fn test_state(app_data_dir: &Path) -> AppState {
        let mut state = AppState::build().expect("build app state");
        state.app_data_dir = app_data_dir.to_path_buf();
        state
    }

    fn write_skill(root: &Path, relative_dir: &str, name: &str, description: &str) {
        let package = root.join(relative_dir);
        std::fs::create_dir_all(&package).expect("create package");
        std::fs::write(
            package.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n"),
        )
        .expect("write SKILL.md");
    }

    fn write_skill_md(package: &Path, frontmatter: &str) {
        std::fs::create_dir_all(package).expect("create package");
        std::fs::write(
            package.join("SKILL.md"),
            format!("---\n{frontmatter}---\n\n# Skill\n"),
        )
        .expect("write SKILL.md");
    }

    fn snapshot_skill_file(snapshot: &Path) -> PathBuf {
        let content = snapshot.join("content/SKILL.md");
        if content.exists() {
            content
        } else {
            snapshot.join("SKILL.md")
        }
    }

    #[tokio::test]
    async fn skill_version_history_is_exact_across_same_name_install_identities() {
        let app = tempdir().expect("app data");
        let first_source = tempdir().expect("first source");
        let second_source = tempdir().expect("second source");
        let first_project = tempdir().expect("first project");
        let second_project = tempdir().expect("second project");
        let state = test_state(app.path());
        write_skill(
            first_source.path(),
            "reviewer",
            "reviewer",
            "First source v1",
        );
        write_skill(
            second_source.path(),
            "reviewer",
            "reviewer",
            "Second source v1",
        );
        let first = add_local_source(&state, first_source.path()).await.unwrap();
        let second = add_local_source(&state, second_source.path())
            .await
            .unwrap();
        let first_project_path = first_project.path().to_string_lossy().into_owned();
        let second_project_path = second_project.path().to_string_lossy().into_owned();

        super::install_skill(
            &state,
            &first.id,
            "reviewer",
            "codex",
            Some(&first_project_path),
        )
        .await
        .unwrap();
        super::install_skill(
            &state,
            &first.id,
            "reviewer",
            "claudeCode",
            Some(&first_project_path),
        )
        .await
        .unwrap();
        super::install_skill(
            &state,
            &second.id,
            "reviewer",
            "codex",
            Some(&second_project_path),
        )
        .await
        .unwrap();

        write_skill(
            first_source.path(),
            "reviewer",
            "reviewer",
            "First source v2",
        );
        super::update_skill(
            &state,
            &first.id,
            "reviewer",
            "codex",
            Some(&first_project_path),
        )
        .await
        .unwrap();
        super::update_skill(
            &state,
            &first.id,
            "reviewer",
            "claudeCode",
            Some(&first_project_path),
        )
        .await
        .unwrap();
        write_skill(
            second_source.path(),
            "reviewer",
            "reviewer",
            "Second source v2",
        );
        super::update_skill(
            &state,
            &second.id,
            "reviewer",
            "codex",
            Some(&second_project_path),
        )
        .await
        .unwrap();

        let first_codex = super::skill_version_history(
            &state,
            &first.id,
            "reviewer",
            "codex",
            Some(&first_project_path),
        )
        .await
        .unwrap();
        let first_claude = super::skill_version_history(
            &state,
            &first.id,
            "reviewer",
            "claudeCode",
            Some(&first_project_path),
        )
        .await
        .unwrap();
        let second_codex = super::skill_version_history(
            &state,
            &second.id,
            "reviewer",
            "codex",
            Some(&second_project_path),
        )
        .await
        .unwrap();
        assert_eq!(first_codex.len(), 1);
        assert_eq!(first_claude.len(), 1);
        assert_eq!(second_codex.len(), 1);
        assert_ne!(first_codex[0].path, first_claude[0].path);
        assert_ne!(first_codex[0].path, second_codex[0].path);

        let cross_identity = super::rollback_skill_authorized(
            &state,
            &first.id,
            "reviewer",
            "codex",
            Some(&first_project_path),
            &second_codex[0].path,
            None,
        )
        .await;
        assert!(cross_identity.is_err());

        let restored = super::rollback_skill_authorized(
            &state,
            &first.id,
            "reviewer",
            "codex",
            Some(&first_project_path),
            &first_codex[0].path,
            None,
        )
        .await
        .expect("rollback exact snapshot");
        assert!(
            std::fs::read_to_string(Path::new(&restored.path).join("SKILL.md"))
                .unwrap()
                .contains("First source v1")
        );
    }

    #[tokio::test]
    async fn skill_version_history_rejects_tampered_snapshot_content() {
        let app = tempdir().expect("app data");
        let source = tempdir().expect("source");
        let project = tempdir().expect("project");
        let state = test_state(app.path());
        write_skill(source.path(), "reviewer", "reviewer", "Version one");
        let registered = add_local_source(&state, source.path()).await.unwrap();
        let project_path = project.path().to_string_lossy().into_owned();
        super::install_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .unwrap();
        write_skill(source.path(), "reviewer", "reviewer", "Version two");
        super::update_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .unwrap();
        let snapshot = super::skill_version_history(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .unwrap()
        .pop()
        .expect("version snapshot");
        std::fs::write(snapshot_skill_file(Path::new(&snapshot.path)), b"tampered")
            .expect("tamper snapshot");

        assert!(super::skill_version_history(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .unwrap()
        .is_empty());
        assert!(super::rollback_skill_authorized(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
            &snapshot.path,
            None,
        )
        .await
        .is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn skill_version_history_rejects_linked_snapshot_content() {
        use std::os::unix::fs::symlink;

        let app = tempdir().expect("app data");
        let source = tempdir().expect("source");
        let project = tempdir().expect("project");
        let outside = tempdir().expect("outside");
        let state = test_state(app.path());
        write_skill(source.path(), "reviewer", "reviewer", "Version one");
        let registered = add_local_source(&state, source.path()).await.unwrap();
        let project_path = project.path().to_string_lossy().into_owned();
        super::install_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .unwrap();
        write_skill(source.path(), "reviewer", "reviewer", "Version two");
        super::update_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .unwrap();
        let snapshot = super::skill_version_history(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .unwrap()
        .pop()
        .expect("version snapshot");
        let content = snapshot_skill_file(Path::new(&snapshot.path));
        std::fs::write(outside.path().join("SKILL.md"), b"outside").unwrap();
        std::fs::remove_file(&content).unwrap();
        symlink(outside.path().join("SKILL.md"), &content).unwrap();

        assert!(super::skill_version_history(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .unwrap()
        .is_empty());
        assert!(super::rollback_skill_authorized(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
            &snapshot.path,
            None,
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn legacy_name_only_skill_backups_are_not_version_candidates() {
        let app = tempdir().expect("app data");
        let source = tempdir().expect("source");
        let project = tempdir().expect("project");
        let state = test_state(app.path());
        write_skill(source.path(), "reviewer", "reviewer", "Version one");
        let registered = add_local_source(&state, source.path()).await.unwrap();
        let project_path = project.path().to_string_lossy().into_owned();
        super::install_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .unwrap();
        let legacy = app.path().join("skill-backups/reviewer-legacy");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("SKILL.md"), b"legacy").unwrap();

        assert!(super::skill_version_history(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .unwrap()
        .is_empty());
        assert!(super::rollback_skill_authorized(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
            legacy.to_string_lossy().as_ref(),
            None,
        )
        .await
        .is_err());
    }

    fn validate_fixture(source: &Path, relative_dir: &str) -> crate::types::SkillPackageResult {
        validate_package("source-id", source, &source.join(relative_dir))
    }

    fn error_paths(result: &crate::types::SkillPackageResult) -> Vec<&str> {
        result
            .errors
            .iter()
            .map(|error| error.path.as_str())
            .collect()
    }

    fn git(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn local_git_repo() -> tempfile::TempDir {
        let repo = tempdir().expect("git repo");
        git(repo.path(), &["init", "-q"]);
        git(
            repo.path(),
            &["config", "user.email", "tests@example.invalid"],
        );
        git(repo.path(), &["config", "user.name", "Tests"]);
        write_skill(repo.path(), "skills/example", "example", "Example");
        write_skill(repo.path(), "outside", "outside", "Outside");
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-qm", "initial"]);
        git(repo.path(), &["tag", "v1"]);
        repo
    }

    async fn set_settings(state: &AppState, value: SettingsLoadState) {
        *state.settings.write().await = value;
    }

    #[tokio::test]
    async fn inspection_reads_registered_sources_without_refreshing_them() {
        let app = tempdir().expect("app data");
        let source_root = tempdir().expect("source");
        let state = test_state(app.path());
        write_skill(
            source_root.path(),
            "skills/reviewer",
            "reviewer",
            "Reviews changes",
        );
        add_local_source(&state, source_root.path())
            .await
            .expect("register source");

        let results = inspect_skill_sources(&state)
            .await
            .expect("inspect sources");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].packages.len(), 1);
        assert_eq!(results[0].packages[0].name.as_deref(), Some("reviewer"));
    }

    #[tokio::test]
    async fn package_access_reads_only_the_validated_inventory() {
        let app = tempdir().expect("app data");
        let source = tempdir().expect("source");
        let package = source.path().join("reviewer");
        write_skill(&package, "", "reviewer", "Reviews changes");
        std::fs::create_dir_all(package.join("references")).expect("reference directory");
        std::fs::write(package.join("references/checklist.md"), b"# Checklist\n")
            .expect("reference file");
        std::fs::write(package.join("assets.png"), [0, 159, 146, 150]).expect("binary file");
        let state = test_state(app.path());
        let registered = add_local_source(&state, source.path())
            .await
            .expect("register source");

        let resolved = resolve_skill_package(&state, &registered.id, "reviewer")
            .await
            .expect("resolve validated package");
        assert_eq!(
            resolved
                .files()
                .iter()
                .map(|file| file.relative_path.as_str())
                .collect::<Vec<_>>(),
            ["SKILL.md", "assets.png", "references/checklist.md"]
        );

        let text = read_skill_file(
            &state,
            &registered.id,
            "reviewer",
            "references/checklist.md",
        )
        .await
        .expect("read listed text file");
        assert_eq!(text.mime_type, "text/plain");
        assert_eq!(text.text.as_deref(), Some("# Checklist\n"));
        assert!(text.base64.is_none());

        let binary = read_skill_file(&state, &registered.id, "reviewer", "assets.png")
            .await
            .expect("read listed binary file");
        assert_eq!(binary.mime_type, "application/octet-stream");
        assert!(binary.text.is_none());
        assert_eq!(binary.base64.as_deref(), Some("AJ+Slg=="));

        for path in ["unlisted.md", "../Cargo.toml", "references/../../SKILL.md"] {
            assert!(
                read_skill_file(&state, &registered.id, "reviewer", path)
                    .await
                    .is_err(),
                "unlisted or traversal path {path:?} was accepted"
            );
        }
    }

    #[tokio::test]
    async fn package_access_rejects_invalid_and_oversize_packages() {
        let app = tempdir().expect("app data");
        let source = tempdir().expect("source");
        let invalid = source.path().join("invalid");
        write_skill(&invalid, "", "invalid", "Invalid package");
        std::fs::write(invalid.join("run.sh"), b"#!/bin/sh\n").expect("unsafe file");
        let oversized = source.path().join("oversized");
        write_skill(&oversized, "", "oversized", "Oversized package");
        std::fs::write(
            oversized.join("too-large.bin"),
            vec![0; MAX_SKILL_FILE_BYTES as usize + 1],
        )
        .expect("oversized file");
        let state = test_state(app.path());
        let registered = add_local_source(&state, source.path())
            .await
            .expect("register source");

        for relative_path in ["invalid", "oversized"] {
            assert!(
                resolve_skill_package(&state, &registered.id, relative_path)
                    .await
                    .is_err(),
                "invalid package {relative_path:?} was resolved"
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn package_access_rejects_linked_entries() {
        use std::os::unix::fs::symlink;

        let app = tempdir().expect("app data");
        let source = tempdir().expect("source");
        let package = source.path().join("linked");
        write_skill(&package, "", "linked", "Linked package");
        symlink("SKILL.md", package.join("reference.md")).expect("linked file");
        let state = test_state(app.path());
        let registered = add_local_source(&state, source.path())
            .await
            .expect("register source");

        assert!(resolve_skill_package(&state, &registered.id, "linked")
            .await
            .is_err());
    }

    #[test]
    fn destination_presence_reports_only_existing_exact_skill_directories() {
        let home = tempdir().expect("home");
        let project = tempdir().expect("project");
        std::fs::create_dir_all(home.path().join(".claude/skills/reviewer"))
            .expect("create Claude user skill");
        std::fs::create_dir_all(project.path().join(".agents/skills/reviewer"))
            .expect("create Codex project skill");

        let destinations = skill_destination_presence(
            home.path(),
            &[project.path().to_string_lossy().into_owned()],
            "reviewer",
        );

        assert_eq!(destinations.len(), 4);
        assert!(destinations.iter().any(|destination| {
            destination.runtime == "claudeCode"
                && destination.scope == "user"
                && destination.present
        }));
        assert!(destinations.iter().any(|destination| {
            destination.runtime == "codex" && destination.scope == "project" && destination.present
        }));
        assert_eq!(
            destinations
                .iter()
                .filter(|destination| destination.present)
                .count(),
            2
        );
    }

    async fn register_test_github(
        state: &AppState,
        git_ref: Option<&str>,
        subdirectory: Option<&str>,
    ) -> SkillSource {
        add_github_source(
            state,
            "https://github.com/owner/repo",
            git_ref,
            subdirectory,
        )
        .await
        .expect("register GitHub source")
    }

    #[tokio::test]
    async fn github_source_registration_rejects_untrusted_inputs() {
        let app = tempdir().expect("app data");
        let state = test_state(app.path());

        let source = add_github_source(
            &state,
            "http://github.com/Owner/Repo.git",
            Some("v1"),
            Some("skills/example"),
        )
        .await
        .expect("register canonical source");
        let duplicate = add_github_source(
            &state,
            "https://github.com/Owner/Repo",
            Some("v1"),
            Some("skills/example"),
        )
        .await
        .expect("deduplicate canonical source");
        assert_eq!(duplicate.id, source.id);
        assert!(matches!(
            source.kind,
            SkillSourceKind::Github {
                ref repository,
                git_ref: Some(ref git_ref),
                subdirectory: Some(ref subdirectory),
                active_checkout: None,
            } if repository == "https://github.com/Owner/Repo.git"
                && git_ref == "v1"
                && subdirectory == "skills/example"
        ));
        let before = std::fs::read(skill_sources_path(app.path())).expect("state bytes");

        for (repository, git_ref, subdirectory) in [
            ("https://example.com/owner/repo", None, None),
            ("https://user:secret@github.com/owner/repo", None, None),
            ("https://github.com/owner/repo", Some(""), None),
            (
                "https://github.com/owner/repo",
                Some("--upload-pack=x"),
                None,
            ),
            ("https://github.com/owner/repo", None, Some("/absolute")),
            ("https://github.com/owner/repo", None, Some("../escape")),
            (
                "https://github.com/owner/repo",
                None,
                Some("skills//example"),
            ),
        ] {
            assert!(matches!(
                add_github_source(&state, repository, git_ref, subdirectory).await,
                Err(AppError::InvalidArgument { .. })
            ));
        }
        assert_eq!(
            std::fs::read(skill_sources_path(app.path())).expect("preserved state"),
            before
        );
    }

    #[tokio::test]
    async fn github_refresh_network_policy_matrix() {
        let repo = local_git_repo();

        for settings in [
            SettingsLoadState::FirstLaunch,
            SettingsLoadState::Loaded(Settings::default()),
        ] {
            let app = tempdir().expect("app data");
            let state = test_state(app.path());
            set_settings(&state, settings).await;
            let source = register_test_github(&state, Some("v1"), Some("skills")).await;
            assert!(refresh_git_source_from(
                &state,
                &source.id,
                repo.path().to_string_lossy().as_ref()
            )
            .await
            .is_ok());
        }

        let paranoid = Settings {
            paranoid_mode: true,
            ..Settings::default()
        };
        for settings in [
            SettingsLoadState::Loaded(paranoid),
            SettingsLoadState::Corrupt {
                message: "bad settings".into(),
            },
        ] {
            let app = tempdir().expect("app data");
            let state = test_state(app.path());
            set_settings(&state, settings).await;
            let source = register_test_github(&state, None, None).await;
            let result =
                refresh_git_source_from(&state, &source.id, "/definitely/not/a/git/repo").await;
            assert!(matches!(
                result,
                Err(AppError::ParanoidModeBlocked { feature })
                    if feature == "skill_source_refresh"
            ));
        }
    }

    #[tokio::test]
    async fn github_refresh_transaction_uses_local_repo() {
        let app = tempdir().expect("app data");
        let repo = local_git_repo();
        let state = test_state(app.path());
        set_settings(&state, SettingsLoadState::FirstLaunch).await;
        let source = register_test_github(&state, Some("v1"), Some("skills")).await;

        let result =
            refresh_git_source_from(&state, &source.id, repo.path().to_string_lossy().as_ref())
                .await
                .expect("refresh local repository");

        assert_eq!(result.packages.len(), 1);
        assert_eq!(result.packages[0].relative_path, "example");
        assert!(result.packages[0].installable);
        let active = match &result.source.kind {
            SkillSourceKind::Github {
                active_checkout: Some(path),
                ..
            } => PathBuf::from(path),
            other => panic!("missing active checkout: {other:?}"),
        };
        assert!(active.ends_with("skills"));
        assert!(active.join("example/SKILL.md").is_file());
        assert!(active.ancestors().any(|path| path
            .file_name()
            .is_some_and(|name| name == source.id.as_str())));
        assert_eq!(
            load_skill_sources(app.path()).await.expect("reload")[0],
            result.source
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_refresh_preserves_active_generation() {
        use std::os::unix::fs::symlink;

        let app = tempdir().expect("app data");
        let repo = local_git_repo();
        let state = test_state(app.path());
        set_settings(&state, SettingsLoadState::FirstLaunch).await;
        let source = register_test_github(&state, Some("v1"), Some("skills")).await;
        let first =
            refresh_git_source_from(&state, &source.id, repo.path().to_string_lossy().as_ref())
                .await
                .expect("seed active generation");
        let state_path = skill_sources_path(app.path());
        let before = std::fs::read(&state_path).expect("state bytes");
        let active_before = first.source.kind.clone();

        for clone_source in [
            "/missing/repository",
            repo.path().to_string_lossy().as_ref(),
        ] {
            let result = refresh_git_source_from(&state, &source.id, clone_source).await;
            if clone_source != "/missing/repository" {
                assert!(result.is_ok(), "control refresh must succeed");
                continue;
            }
            assert!(result.is_err());
            assert_eq!(
                load_skill_sources(app.path()).await.expect("preserved")[0].kind,
                active_before
            );
            assert_eq!(std::fs::read(&state_path).expect("state bytes"), before);
        }

        let missing_ref = register_test_github(&state, Some("missing-ref"), None).await;
        let missing_ref_before = std::fs::read(&state_path).expect("before missing ref");
        assert!(refresh_git_source_from(
            &state,
            &missing_ref.id,
            repo.path().to_string_lossy().as_ref()
        )
        .await
        .is_err());
        assert_eq!(
            std::fs::read(&state_path).expect("after missing ref"),
            missing_ref_before
        );

        let escaped_repo = local_git_repo();
        symlink("..", escaped_repo.path().join("escape")).expect("escaping symlink");
        git(escaped_repo.path(), &["add", "escape"]);
        git(
            escaped_repo.path(),
            &["commit", "-qm", "escaping subdirectory"],
        );
        let escaped = register_test_github(&state, None, Some("escape")).await;
        let escaped_before = std::fs::read(&state_path).expect("before escape");
        assert!(refresh_git_source_from(
            &state,
            &escaped.id,
            escaped_repo.path().to_string_lossy().as_ref()
        )
        .await
        .is_err());
        assert_eq!(
            std::fs::read(&state_path).expect("after escape"),
            escaped_before
        );

        let tmp_path = Path::new(&format!("{}.tmp", state_path.display())).to_path_buf();
        std::fs::create_dir(&tmp_path).expect("block atomic temp file");
        let persist_before = std::fs::read(&state_path).expect("before persistence failure");
        assert!(refresh_git_source_from(
            &state,
            &source.id,
            repo.path().to_string_lossy().as_ref()
        )
        .await
        .is_err());
        assert_eq!(
            std::fs::read(&state_path).expect("after persistence failure"),
            persist_before
        );
    }

    #[tokio::test]
    async fn invalid_git_packages_remain_inspectable() {
        let app = tempdir().expect("app data");
        let repo = local_git_repo();
        std::fs::write(repo.path().join("skills/example/run.sh"), b"unsafe")
            .expect("write unsafe surface");
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-qm", "unsafe package"]);
        let state = test_state(app.path());
        set_settings(&state, SettingsLoadState::FirstLaunch).await;
        let source = register_test_github(&state, None, Some("skills")).await;

        let result =
            refresh_git_source_from(&state, &source.id, repo.path().to_string_lossy().as_ref())
                .await
                .expect("refresh invalid package");

        assert_eq!(result.packages.len(), 1);
        assert!(!result.packages[0].installable);
        assert!(result.packages[0]
            .errors
            .iter()
            .any(|error| error.path == "run.sh"));
        assert!(matches!(
            result.source.kind,
            SkillSourceKind::Github {
                active_checkout: Some(_),
                ..
            }
        ));
    }

    #[tokio::test]
    async fn concurrent_git_refresh_preserves_source_records() {
        let app = tempdir().expect("app data");
        let repo = local_git_repo();
        let state = Arc::new(test_state(app.path()));
        set_settings(&state, SettingsLoadState::FirstLaunch).await;
        let first = register_test_github(&state, None, Some("skills")).await;
        let second = add_github_source(
            &state,
            "https://github.com/other/repo",
            None,
            Some("skills"),
        )
        .await
        .expect("register second source");

        let refresh = |source: SkillSource| {
            let state = Arc::clone(&state);
            let clone_source = repo.path().to_string_lossy().into_owned();
            tokio::spawn(
                async move { refresh_git_source_from(&state, &source.id, &clone_source).await },
            )
        };
        let first_result = refresh(first)
            .await
            .expect("first join")
            .expect("first refresh");
        let second_result = refresh(second)
            .await
            .expect("second join")
            .expect("second refresh");
        let persisted = load_skill_sources(app.path()).await.expect("load sources");

        assert_eq!(persisted.len(), 2);
        assert!(persisted
            .iter()
            .any(|source| source == &first_result.source));
        assert!(persisted
            .iter()
            .any(|source| source == &second_result.source));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn github_refresh_filesystem_transaction_runs_in_spawn_blocking() {
        let app = tempdir().expect("app data");
        let repo = local_git_repo();
        let state = test_state(app.path());
        set_settings(&state, SettingsLoadState::FirstLaunch).await;
        let source = register_test_github(&state, None, Some("skills")).await;
        let async_thread = std::thread::current().id();
        reset_refresh_fs_probe();

        refresh_git_source_from(&state, &source.id, repo.path().to_string_lossy().as_ref())
            .await
            .expect("successful refresh");
        refresh_git_source_from(&state, &source.id, repo.path().to_string_lossy().as_ref())
            .await
            .expect("second successful refresh");
        assert!(
            refresh_git_source_from(&state, &source.id, "/missing/repository")
                .await
                .is_err()
        );

        let probe = take_refresh_fs_probe();
        for required in [
            "staging_create",
            "canonicalize",
            "activation_rename",
            "state_persist",
            "failed_stage_cleanup",
            "recursive_cleanup",
        ] {
            assert!(
                probe.iter().any(|(event, _)| *event == required),
                "missing probe {required}: {probe:?}"
            );
        }
        assert!(
            probe.iter().all(|(_, thread)| *thread != async_thread),
            "filesystem transaction touched async thread: {probe:?}"
        );
        assert!(
            !probe.iter().any(|(event, _)| *event == "obsolete_cleanup"),
            "successful obsolete generations must not be cleaned in Phase 1"
        );
        let source_dir = app.path().join("skills/sources").join(&source.id);
        assert_eq!(
            std::fs::read_dir(source_dir)
                .expect("source generations")
                .count(),
            2,
            "both successful immutable generations remain"
        );
    }

    #[test]
    fn github_source_kind_serializes_camel_case_variant_fields() {
        let kind = SkillSourceKind::Github {
            repository: "owner/repo".into(),
            git_ref: Some("v1.0.0".into()),
            subdirectory: Some("skills".into()),
            active_checkout: Some("/tmp/checkout".into()),
        };

        assert_eq!(
            serde_json::to_value(kind).expect("serialize"),
            json!({
                "kind": "github",
                "repository": "owner/repo",
                "gitRef": "v1.0.0",
                "subdirectory": "skills",
                "activeCheckout": "/tmp/checkout"
            })
        );
    }

    #[tokio::test]
    async fn local_source_tracer() {
        let app = tempdir().expect("app data");
        let source = tempdir().expect("source");
        write_skill(source.path(), "nested/example", "example", "Example skill");
        std::fs::write(
            source.path().join("nested/example/reference.md"),
            b"reference\n",
        )
        .expect("write reference");
        std::fs::write(source.path().join("nested/skill.md"), b"not exact").expect("write decoy");
        let state = test_state(app.path());

        let registered = add_local_source(&state, source.path())
            .await
            .expect("register source");
        let registered_again = add_local_source(&state, source.path())
            .await
            .expect("register source once");
        assert_eq!(registered_again.id, registered.id);
        let persisted = load_skill_sources(app.path())
            .await
            .expect("reload sources");
        assert_eq!(persisted, vec![registered.clone()]);

        let result = discover_source(registered).await.expect("refresh source");
        assert!(result.errors.is_empty());
        assert_eq!(result.packages.len(), 1);
        let package = &result.packages[0];
        assert_eq!(package.relative_path, "nested/example");
        assert_eq!(package.name.as_deref(), Some("example"));
        assert_eq!(package.description.as_deref(), Some("Example skill"));
        assert!(package.installable);
        assert!(package.errors.is_empty());
        assert_eq!(
            package
                .files
                .iter()
                .map(|file| file.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec!["SKILL.md", "reference.md"]
        );
        assert!(package.files.iter().all(|file| file.sha256.len() == 64));

        let reloaded = load_skill_sources(app.path())
            .await
            .expect("reload sources again");
        assert_eq!(reloaded[0].id, result.source.id);
        assert!(skill_sources_path(app.path()).exists());
    }

    #[tokio::test]
    async fn root_skill_package_is_discovered() {
        let parent = tempdir().expect("source parent");
        let source = parent.path().join("root-skill");
        write_skill(&source, "", "root-skill", "Root skill");
        let registered = SkillSource {
            id: "source-id".into(),
            kind: SkillSourceKind::Local {
                root: source.to_string_lossy().into_owned(),
            },
        };

        let result = discover_source(registered).await.expect("discover source");

        assert_eq!(result.packages.len(), 1);
        assert_eq!(result.packages[0].relative_path, ".");
        assert!(result.packages[0].installable, "{:?}", result.packages[0]);
    }

    #[tokio::test]
    async fn discovery_skips_hidden_runtime_skill_mirrors() {
        let source = tempdir().expect("source");
        write_skill(source.path(), "visible", "visible", "Visible skill");
        write_skill(
            source.path(),
            ".cursor/skills/visible",
            "visible",
            "Runtime mirror",
        );
        let registered = SkillSource {
            id: "source-id".into(),
            kind: SkillSourceKind::Local {
                root: source.path().to_string_lossy().into_owned(),
            },
        };

        let result = discover_source(registered).await.expect("discover source");

        assert_eq!(result.packages.len(), 1);
        assert_eq!(result.packages[0].relative_path, "visible");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn discovery_rejects_symlinked_ancestor_outside_source() {
        use std::os::unix::fs::symlink;

        let app = tempdir().expect("app data");
        let source = tempdir().expect("source");
        let external = tempdir().expect("external");
        write_skill(
            external.path(),
            "escaped",
            "escaped",
            "Must not be discovered",
        );
        symlink(external.path(), source.path().join("linked")).expect("create symlink");
        let state = test_state(app.path());
        let registered = add_local_source(&state, source.path())
            .await
            .expect("register source");

        let result = discover_source(registered).await.expect("refresh source");

        assert!(result.packages.is_empty());
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].code, SkillValidationCode::UnsafeEntry);
        assert_eq!(result.errors[0].path, "linked");
        assert!(result.errors[0].message.contains("Remove the link"));
    }

    #[test]
    fn windows_reparse_attribute_fails_closed() {
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

        assert!(!is_windows_reparse_point(0));
        assert!(is_windows_reparse_point(FILE_ATTRIBUTE_REPARSE_POINT));
        assert!(is_windows_reparse_point(
            FILE_ATTRIBUTE_REPARSE_POINT | 0x10
        ));
    }

    #[tokio::test]
    async fn invalid_local_roots_preserve_state() {
        let app = tempdir().expect("app data");
        let valid = tempdir().expect("valid source");
        let state = test_state(app.path());
        add_local_source(&state, valid.path())
            .await
            .expect("seed valid source");
        let state_path = skill_sources_path(app.path());
        let before = std::fs::read(&state_path).expect("read initial state");

        let relative = add_local_source(&state, Path::new("relative")).await;
        let missing = add_local_source(&state, &app.path().join("missing")).await;
        let file = app.path().join("file");
        std::fs::write(&file, b"x").expect("write file");
        let not_directory = add_local_source(&state, &file).await;

        for result in [relative, missing, not_directory] {
            assert!(matches!(result, Err(AppError::InvalidArgument { .. })));
        }
        assert_eq!(
            std::fs::read(&state_path).expect("read preserved state"),
            before
        );
    }

    #[tokio::test]
    async fn removing_source_only_unregisters_it() {
        let app = tempdir().expect("app data");
        let source = tempdir().expect("source");
        write_skill(source.path(), "example", "example", "Example");
        let state = test_state(app.path());
        let registered = add_local_source(&state, source.path())
            .await
            .expect("register source");

        assert!(remove_skill_source(&state, &registered.id)
            .await
            .expect("remove source"));
        assert!(load_skill_sources(app.path())
            .await
            .expect("reload sources")
            .is_empty());
        assert!(source.path().join("example/SKILL.md").is_file());
        assert!(!remove_skill_source(&state, &registered.id)
            .await
            .expect("remove missing source"));
    }

    #[tokio::test]
    async fn concurrent_local_registration_preserves_both_sources() {
        let app = tempdir().expect("app data");
        let first = tempdir().expect("first source");
        let second = tempdir().expect("second source");
        let state = Arc::new(test_state(app.path()));

        let first_task = {
            let state = Arc::clone(&state);
            let root = first.path().to_path_buf();
            tokio::spawn(async move { add_local_source(&state, &root).await })
        };
        let second_task = {
            let state = Arc::clone(&state);
            let root = second.path().to_path_buf();
            tokio::spawn(async move { add_local_source(&state, &root).await })
        };

        let first_source = first_task.await.expect("first join").expect("first add");
        let second_source = second_task.await.expect("second join").expect("second add");
        let persisted = load_skill_sources(app.path()).await.expect("load sources");

        assert_eq!(persisted.len(), 2);
        assert!(persisted.iter().any(|source| source.id == first_source.id));
        assert!(persisted.iter().any(|source| source.id == second_source.id));
    }

    #[tokio::test]
    async fn concurrent_same_local_source_has_one_atomic_creator() {
        let app = tempdir().expect("app data");
        let source = tempdir().expect("source");
        let state = Arc::new(test_state(app.path()));

        let tasks = (0..2).map(|_| {
            let state = Arc::clone(&state);
            let root = source.path().to_path_buf();
            tokio::spawn(async move { ensure_local_source(&state, &root).await })
        });
        let mut results = Vec::new();
        for task in tasks {
            results.push(task.await.expect("join").expect("ensure source"));
        }

        assert_eq!(results.iter().filter(|(_, created)| *created).count(), 1);
        assert_eq!(results[0].0.id, results[1].0.id);
        assert_eq!(load_skill_sources(app.path()).await.expect("load").len(), 1);
    }

    #[test]
    fn validation_matrix() {
        let source = tempdir().expect("source");
        for name in ["a".to_string(), "a".repeat(64)] {
            write_skill(source.path(), &name, &name, "d");
            assert!(
                validate_fixture(source.path(), &name).installable,
                "valid name length {}",
                name.len()
            );
        }

        for (directory, name) in [
            ("empty-name", ""),
            ("overlong", &"a".repeat(65)),
            ("uppercase", "Uppercase"),
            ("underscore", "under_score"),
            ("leading-hyphen", "-leading"),
            ("trailing-hyphen", "trailing-"),
            ("double-hyphen", "double--hyphen"),
            ("folder", "different"),
        ] {
            write_skill(source.path(), directory, name, "description");
            let result = validate_fixture(source.path(), directory);
            assert!(!result.installable, "invalid name {name:?} was accepted");
            assert!(
                result
                    .errors
                    .iter()
                    .any(|error| error.code == SkillValidationCode::InvalidMetadata),
                "invalid name {name:?} lacked a metadata error"
            );
        }

        let descriptions = [
            ("missing-description", "name: missing-description\n"),
            (
                "empty-description",
                "name: empty-description\ndescription: ''\n",
            ),
            (
                "non-string-description",
                "name: non-string-description\ndescription:\n  nested: value\n",
            ),
            (
                "overlong-description",
                &format!(
                    "name: overlong-description\ndescription: '{}'\n",
                    "d".repeat(1025)
                ),
            ),
        ];
        for (directory, frontmatter) in descriptions {
            let package = source.path().join(directory);
            write_skill_md(&package, frontmatter);
            let result = validate_fixture(source.path(), directory);
            assert!(
                !result.installable
                    && result
                        .errors
                        .iter()
                        .any(|error| error.code == SkillValidationCode::InvalidMetadata),
                "invalid description for {directory} was accepted"
            );
        }

        let valid_description = source.path().join("description-limit");
        write_skill_md(
            &valid_description,
            &format!(
                "name: description-limit\ndescription: '{}'\n",
                "d".repeat(1024)
            ),
        );
        assert!(
            validate_fixture(source.path(), "description-limit").installable,
            "1024-character description should be valid"
        );

        let malformed = source.path().join("malformed");
        std::fs::create_dir_all(&malformed).expect("create malformed package");
        std::fs::write(
            malformed.join("SKILL.md"),
            b"---\nname: malformed\ndescription: [\n---\n",
        )
        .expect("write malformed frontmatter");
        assert!(
            !validate_fixture(source.path(), "malformed").installable,
            "malformed frontmatter was accepted"
        );

        let taxonomy = source.path().join("taxonomy");
        write_skill_md(
            &taxonomy,
            "name: taxonomy\ndescription: Typed skill\ntype: development\ngroup:\n  - frontend\n  - react\ntags:\n  - typescript\n  - ui\n",
        );
        let taxonomy = validate_fixture(source.path(), "taxonomy");
        assert!(taxonomy.installable, "{:?}", taxonomy.errors);
        assert_eq!(taxonomy.skill_type, SkillType::Development);
        assert_eq!(taxonomy.group, ["frontend", "react"]);
        assert_eq!(taxonomy.tags, ["typescript", "ui"]);

        for (directory, extra) in [
            ("bad-type", "type: unknown\n"),
            ("bad-group", "group: [one, two, three, four, five]\n"),
            ("bad-tag", "tags: [UPPERCASE]\n"),
            ("duplicate-tags", "tags: [ui, ui]\n"),
        ] {
            let package = source.path().join(directory);
            write_skill_md(
                &package,
                &format!("name: {directory}\ndescription: Invalid taxonomy\n{extra}"),
            );
            assert!(
                !validate_fixture(source.path(), directory).installable,
                "invalid taxonomy for {directory} was accepted"
            );
        }

        let inert = source.path().join("inert-files");
        write_skill(&inert, "", "inert-files", "Inert content");
        let fixtures = [
            ("references/guide.md", b"# Guide\n".as_slice()),
            ("assets/image.png", &[0, 1, 2, 3]),
            ("templates/example.txt", b"{{ exact }}\n".as_slice()),
        ];
        for (relative, bytes) in fixtures {
            let path = inert.join(relative);
            std::fs::create_dir_all(path.parent().expect("fixture parent"))
                .expect("create fixture parent");
            std::fs::write(path, bytes).expect("write inert fixture");
        }
        let result = validate_fixture(source.path(), "inert-files");
        assert!(result.installable, "{:?}", result.errors);
        for (relative, bytes) in fixtures {
            let file = result
                .files
                .iter()
                .find(|file| file.relative_path == relative)
                .unwrap_or_else(|| panic!("missing inventory entry {relative}"));
            assert_eq!(file.size_bytes, bytes.len() as u64);
            assert_eq!(file.sha256, format!("{:x}", sha2::Sha256::digest(bytes)));
        }
    }

    #[test]
    fn cross_platform_executable_surfaces_are_rejected() {
        let source = tempdir().expect("source");
        let package = source.path().join("unsafe-surfaces");
        write_skill(&package, "", "unsafe-surfaces", "Unsafe surfaces");

        for relative in [
            "nested/run.sh",
            "nested/run.BASH",
            "nested/run.zsh",
            "nested/run.fish",
            "nested/run.ps1",
            "nested/run.py",
            "nested/payload.js",
            "nested/run.bat",
            "nested/run.cmd",
            "nested/run.com",
            "nested/run.exe",
            "nested/run.dll",
            "nested/run.dylib",
            "nested/run.so",
            "scripts/tool.txt",
            "HOOKS/config.json",
            "mcp.json",
            "plugin.yaml",
        ] {
            let path = package.join(relative);
            std::fs::create_dir_all(path.parent().expect("unsafe parent"))
                .expect("create unsafe parent");
            std::fs::write(path, b"unsafe").expect("write unsafe file");
        }
        std::fs::create_dir_all(package.join("nested/Bundle.APP"))
            .expect("create app bundle directory");

        #[cfg(unix)]
        {
            use std::os::unix::fs::{symlink, PermissionsExt};

            std::fs::write(package.join("executable.txt"), b"executable")
                .expect("write executable");
            let mut permissions = std::fs::metadata(package.join("executable.txt"))
                .expect("executable metadata")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(package.join("executable.txt"), permissions)
                .expect("set executable mode");
            symlink("SKILL.md", package.join("linked.md")).expect("create package symlink");

            use std::os::unix::net::UnixListener;
            UnixListener::bind(package.join("special.sock")).expect("create special entry");
        }

        let result = validate_fixture(source.path(), "unsafe-surfaces");
        assert!(!result.installable);
        let paths = error_paths(&result);
        for expected in [
            "HOOKS",
            "mcp.json",
            "nested/Bundle.APP",
            "nested/payload.js",
            "nested/run.sh",
            "plugin.yaml",
            "scripts",
        ] {
            assert!(
                paths.contains(&expected),
                "missing {expected} in {:?}",
                result.errors
            );
        }
        #[cfg(unix)]
        for expected in ["executable.txt", "linked.md", "special.sock"] {
            assert!(
                paths.contains(&expected),
                "missing {expected} in {:?}",
                result.errors
            );
        }

        let outside = tempdir().expect("outside");
        write_skill(outside.path(), "escaped", "escaped", "Outside source");
        let escaped = validate_package("source-id", source.path(), &outside.path().join("escaped"));
        assert!(
            !escaped.installable
                && escaped
                    .errors
                    .iter()
                    .any(|error| error.code == SkillValidationCode::UnsafeEntry),
            "package outside source root was accepted"
        );

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        assert!(is_windows_reparse_point(FILE_ATTRIBUTE_REPARSE_POINT));
    }

    #[test]
    fn scripts_are_bounded_trust_candidates_but_other_capability_surfaces_remain_unsafe() {
        let source = tempdir().expect("source");
        let package = source.path().join("trusted-scripts");
        write_skill(&package, "", "trusted-scripts", "Script-bearing skill");
        std::fs::create_dir_all(package.join("scripts")).expect("scripts");
        std::fs::write(package.join("scripts/run.py"), b"print('ok')\n").expect("python script");
        std::fs::write(package.join("scripts/run.sh"), b"#!/bin/sh\nexit 0\n")
            .expect("shell script");
        std::fs::create_dir_all(package.join("scripts/mcp")).expect("nested mcp");
        std::fs::write(package.join("scripts/mcp/server.py"), b"print('never')\n")
            .expect("nested mcp server");
        std::fs::create_dir_all(package.join("hooks")).expect("hooks");
        std::fs::write(package.join("hooks/preinstall.txt"), b"never").expect("hook");

        let result = validate_fixture(source.path(), "trusted-scripts");

        assert!(!result.installable);
        assert!(result
            .files
            .iter()
            .any(|file| file.relative_path == "scripts/run.py"));
        assert!(result
            .files
            .iter()
            .any(|file| file.relative_path == "scripts/run.sh"));
        assert!(result.errors.iter().any(|error| {
            error.code == SkillValidationCode::TrustRequired && error.path == "scripts"
        }));
        assert!(result.errors.iter().any(|error| {
            error.code == SkillValidationCode::UnsafeEntry && error.path == "hooks"
        }));
        assert!(result.errors.iter().any(|error| {
            error.code == SkillValidationCode::UnsafeEntry && error.path == "scripts/mcp"
        }));
    }

    #[test]
    fn exact_version_trust_is_signed_and_invalidated_by_script_changes() {
        let source = tempdir().expect("source");
        let package = source.path().join("skill-with-scripts");
        write_skill(&package, "", "skill-with-scripts", "Script-bearing skill");
        std::fs::create_dir_all(package.join("scripts")).expect("scripts");
        std::fs::write(package.join("scripts/run.py"), b"print('v1')\n").expect("script");
        let skill_source = SkillSource {
            id: "source-id".into(),
            kind: SkillSourceKind::Local {
                root: source.path().to_string_lossy().into_owned(),
            },
        };
        let mut result = discover_source_blocking(skill_source).expect("discover");
        let candidate = result.packages[0].clone();
        let (tree_hash, executables) =
            trust_fingerprint(&package, &candidate).expect("fingerprint");
        let key = [7_u8; 32];
        let mut record = SkillTrustRecord {
            source_id: "source-id".into(),
            relative_path: "skill-with-scripts".into(),
            tree_hash,
            executables,
            granted_at: "2026-07-30T00:00:00Z".into(),
            signature: String::new(),
        };
        record.signature = sign_trust_record(&record, &key).expect("sign");

        apply_skill_trust(source.path(), &mut result, &[record.clone()], Some(&key));
        assert!(result.packages[0].installable);

        std::fs::write(package.join("scripts/run.py"), b"print('v2')\n").expect("mutate");
        let mut changed = discover_source_blocking(result.source.clone()).expect("rediscover");
        apply_skill_trust(source.path(), &mut changed, &[record.clone()], Some(&key));
        assert!(!changed.packages[0].installable);
        assert!(changed.packages[0]
            .errors
            .iter()
            .any(|error| error.code == SkillValidationCode::TrustRequired));

        std::fs::write(package.join("scripts/run.py"), b"print('v1')\n").expect("restore");
        let mut forged = record;
        forged.signature.replace_range(..2, "00");
        let mut original = discover_source_blocking(result.source).expect("rediscover original");
        apply_skill_trust(source.path(), &mut original, &[forged], Some(&key));
        assert!(!original.packages[0].installable);
    }

    #[test]
    fn missing_key_never_silently_rekeys_existing_trust_records() {
        #[derive(Default)]
        struct MemoryKeychain(std::sync::Mutex<std::collections::HashMap<String, String>>);
        impl KeychainSlot for MemoryKeychain {
            fn read(&self, account: &str) -> Result<Option<String>, AppError> {
                Ok(self.0.lock().expect("keychain").get(account).cloned())
            }
            fn write(&self, account: &str, value: &str) -> Result<(), AppError> {
                self.0
                    .lock()
                    .expect("keychain")
                    .insert(account.into(), value.into());
                Ok(())
            }
            fn delete(&self, account: &str) -> Result<(), AppError> {
                self.0.lock().expect("keychain").remove(account);
                Ok(())
            }
        }

        let keychain = MemoryKeychain::default();
        let key = load_or_create_trust_key_with(&keychain, false).expect("create key");
        assert_eq!(key.len(), 32);
        assert_eq!(
            load_or_create_trust_key_with(&keychain, true).expect("reuse key"),
            key
        );

        let missing = MemoryKeychain::default();
        assert!(matches!(
            load_or_create_trust_key_with(&missing, true),
            Err(AppError::KeychainUnavailable { .. })
        ));

        let mut record = SkillTrustRecord {
            source_id: "source-id".into(),
            relative_path: "skill".into(),
            tree_hash: "a".repeat(64),
            executables: Vec::new(),
            granted_at: "2026-08-06T00:00:00Z".into(),
            signature: String::new(),
        };
        record.signature = sign_trust_record(&record, &key).expect("sign record");
        assert!(validate_imported_skill_trust(&[record.clone()], &keychain).is_ok());
        assert!(matches!(
            validate_imported_skill_trust(&[record.clone()], &missing),
            Err(AppError::StorageCorrupt { .. })
        ));
        record.signature.replace_range(..2, "00");
        assert!(matches!(
            validate_imported_skill_trust(&[record], &keychain),
            Err(AppError::StorageCorrupt { .. })
        ));
    }

    #[test]
    fn package_bounds_are_inclusive() {
        let source = tempdir().expect("source");

        let count_limit = source.path().join("count-limit");
        write_skill(&count_limit, "", "count-limit", "Count limit");
        for index in 0..(MAX_SKILL_FILES - 1) {
            std::fs::write(count_limit.join(format!("file-{index:03}.txt")), b"x")
                .expect("write counted file");
        }
        let exact_count = validate_fixture(source.path(), "count-limit");
        assert!(
            exact_count.installable,
            "exact file-count limit should be valid"
        );
        assert_eq!(exact_count.files.len(), MAX_SKILL_FILES);
        std::fs::write(count_limit.join("file-512.txt"), b"x").expect("write file 513");
        let over_count = validate_fixture(source.path(), "count-limit");
        assert!(!over_count.installable, "file 513 should be rejected");
        assert_eq!(
            over_count.files.len(),
            MAX_SKILL_FILES,
            "the first 512 files should remain inspectable"
        );
        assert!(error_paths(&over_count).contains(&"file-512.txt"));

        let file_limit = source.path().join("file-limit");
        write_skill(&file_limit, "", "file-limit", "File limit");
        std::fs::write(
            file_limit.join("exact.png"),
            vec![0_u8; MAX_SKILL_FILE_BYTES as usize],
        )
        .expect("write exact-size file");
        assert!(
            validate_fixture(source.path(), "file-limit").installable,
            "exact per-file limit should be valid"
        );
        std::fs::write(
            file_limit.join("too-large.png"),
            vec![0_u8; MAX_SKILL_FILE_BYTES as usize + 1],
        )
        .expect("write oversize file");
        assert!(
            !validate_fixture(source.path(), "file-limit").installable,
            "file beyond per-file limit should be rejected"
        );

        let total_limit = source.path().join("total-limit");
        write_skill(&total_limit, "", "total-limit", "Total limit");
        let skill_size = std::fs::metadata(total_limit.join("SKILL.md"))
            .expect("SKILL.md metadata")
            .len();
        for index in 0..7 {
            std::fs::write(
                total_limit.join(format!("part-{index}.png")),
                vec![0_u8; MAX_SKILL_FILE_BYTES as usize],
            )
            .expect("write total-limit part");
        }
        std::fs::write(
            total_limit.join("part-7.png"),
            vec![0_u8; (MAX_SKILL_FILE_BYTES - skill_size) as usize],
        )
        .expect("write total-limit remainder");
        assert!(
            validate_fixture(source.path(), "total-limit").installable,
            "exact total byte limit should be valid"
        );
        std::fs::write(total_limit.join("zz-extra.png"), b"x").expect("write aggregate overflow");
        assert!(
            !validate_fixture(source.path(), "total-limit").installable,
            "first byte beyond total limit should be rejected"
        );
    }

    #[test]
    fn cap_failures_continue_and_collect_later_errors() {
        let source = tempdir().expect("source");
        let package = source.path().join("continue-after-caps");
        write_skill(
            &package,
            "",
            "continue-after-caps",
            "Continue after cap failures",
        );
        std::fs::write(
            package.join("a-too-large.bin"),
            vec![0_u8; MAX_SKILL_FILE_BYTES as usize + 1],
        )
        .expect("write oversize file");
        std::fs::create_dir_all(package.join("scripts")).expect("create reserved directory");
        std::fs::write(package.join("scripts/ignored.txt"), b"unsafe")
            .expect("write reserved content");
        std::fs::write(package.join("z-last.sh"), b"unsafe").expect("write executable suffix");

        let result = validate_fixture(source.path(), "continue-after-caps");

        assert!(!result.installable);
        let paths = error_paths(&result);
        for expected in ["a-too-large.bin", "scripts", "z-last.sh"] {
            assert!(
                paths.contains(&expected),
                "missing later error {expected} in {:?}",
                result.errors
            );
        }
        assert!(
            result
                .files
                .iter()
                .any(|file| file.relative_path == "SKILL.md"),
            "valid files should remain inspectable after cap failures"
        );
    }

    #[tokio::test]
    async fn invalid_packages_remain_inspectable() {
        let source = tempdir().expect("source");
        write_skill(
            source.path(),
            "inspectable",
            "inspectable",
            "Inspectable invalid package",
        );
        std::fs::write(source.path().join("inspectable/run.sh"), b"unsafe")
            .expect("write executable surface");
        let registered = SkillSource {
            id: "source-id".into(),
            kind: SkillSourceKind::Local {
                root: source.path().to_string_lossy().into_owned(),
            },
        };

        let result = discover_source(registered).await.expect("discover source");

        assert_eq!(result.packages.len(), 1);
        let package = &result.packages[0];
        assert_eq!(package.name.as_deref(), Some("inspectable"));
        assert_eq!(
            package.description.as_deref(),
            Some("Inspectable invalid package")
        );
        assert!(!package.installable);
        assert!(package
            .files
            .iter()
            .any(|file| file.relative_path == "SKILL.md"));
        assert!(
            package.errors.iter().any(|error| {
                error.code == SkillValidationCode::UnsafeEntry && error.path == "run.sh"
            }),
            "{:?}",
            package.errors
        );
    }

    #[tokio::test]
    async fn install_plan_orders_dependencies_before_the_requested_skill() {
        let app = tempdir().expect("app data");
        let source = tempdir().expect("source");
        let project = tempdir().expect("project");
        let state = test_state(app.path());
        write_skill(
            source.path(),
            "foundation",
            "foundation",
            "Shared foundation",
        );
        write_skill_md(
            &source.path().join("frontend"),
            "name: frontend\ndescription: Frontend workflow\ndependencies:\n  - foundation\n",
        );
        let registered = add_local_source(&state, source.path())
            .await
            .expect("register source");

        let plan = super::plan_skill_install(
            &state,
            &registered.id,
            "frontend",
            "codex",
            Some(&project.path().to_string_lossy()),
        )
        .await
        .expect("build install plan");

        assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
        assert_eq!(
            plan.packages
                .iter()
                .map(|package| (package.name.as_str(), package.dependency))
                .collect::<Vec<_>>(),
            vec![("foundation", true), ("frontend", false)]
        );
    }

    #[tokio::test]
    async fn core_lifecycle_uses_the_requested_project_runtime_and_states() {
        let app = tempdir().expect("app data");
        let source = tempdir().expect("source");
        let project = tempdir().expect("project");
        let state = test_state(app.path());
        write_skill(source.path(), "reviewer", "reviewer", "Reviews changes");
        let registered = add_local_source(&state, source.path())
            .await
            .expect("register source");
        let project_path = project.path().to_string_lossy().into_owned();
        let canonical_project = std::fs::canonicalize(project.path())
            .expect("canonical project")
            .to_string_lossy()
            .into_owned();

        let claude = super::install_skill(
            &state,
            &registered.id,
            "reviewer",
            "claudeCode",
            Some(&project_path),
        )
        .await
        .expect("install Claude project skill");
        assert_eq!(claude.scope, "project");
        assert_eq!(claude.runtime, "claudeCode");
        assert_eq!(
            claude.project_path.as_deref(),
            Some(canonical_project.as_str())
        );
        assert_eq!(
            Path::new(&claude.path),
            Path::new(&canonical_project).join(".claude/skills/reviewer")
        );

        let installed = super::install_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .expect("install Codex project skill");
        assert_eq!(installed.scope, "project");
        assert_eq!(installed.runtime, "codex");
        assert_eq!(
            installed.project_path.as_deref(),
            Some(canonical_project.as_str())
        );
        assert_eq!(
            Path::new(&installed.path),
            Path::new(&canonical_project).join(".agents/skills/reviewer")
        );

        let reconciled =
            super::reconcile_skill_installs(&state, std::slice::from_ref(&project_path))
                .await
                .expect("reconcile current skill");
        assert!(reconciled.iter().any(|skill| {
            skill.path == installed.path && skill.state == SkillInstallState::Current
        }));

        let disabled = super::disable_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .expect("disable skill");
        assert_eq!(disabled.state, SkillInstallState::Disabled);
        assert!(!Path::new(&installed.path).exists());

        let enabled = super::enable_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .expect("enable skill");
        assert_eq!(enabled.state, SkillInstallState::Current);
        assert!(Path::new(&installed.path).is_dir());

        std::fs::write(
            source.path().join("reviewer/SKILL.md"),
            "---\nname: reviewer\ndescription: Reviews changes\n---\n# Updated reviewer\n",
        )
        .expect("update source");
        let updated = super::update_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .expect("update skill");
        assert_eq!(updated.state, SkillInstallState::Current);
        assert!(
            std::fs::read_to_string(Path::new(&updated.path).join("SKILL.md"))
                .expect("read updated skill")
                .contains("# Updated reviewer")
        );

        std::fs::write(Path::new(&updated.path).join("LOCAL.md"), b"keep me")
            .expect("modify installed skill");
        assert!(super::disable_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .is_err());
        assert!(super::uninstall_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .expect("uninstall modified skill"));
        assert!(!Path::new(&updated.path).exists());
        assert!(state.app_data_dir.join("skill-backups").is_dir());
    }

    #[tokio::test]
    async fn install_ledger_lock_serializes_independent_app_states() {
        let app = tempdir().expect("app data");
        let source = tempdir().expect("source");
        let project = tempdir().expect("project");
        write_skill(source.path(), "reviewer", "reviewer", "Reviews changes");
        let first_state = Arc::new(test_state(app.path()));
        let second_state = Arc::new(test_state(app.path()));
        let registered = add_local_source(&first_state, source.path())
            .await
            .expect("register source");
        let project_path = project.path().to_string_lossy().into_owned();
        let first = super::lock_skill_installs(app.path()).expect("first process lock");
        let second = Arc::clone(&second_state);
        let source_id = registered.id.clone();
        let mut waiter = tokio::spawn(async move {
            super::install_skill(
                &second,
                &source_id,
                "reviewer",
                "codex",
                Some(&project_path),
            )
            .await
        });

        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut waiter)
                .await
                .is_err()
        );
        drop(first);
        let second_installed = waiter
            .await
            .expect("second app state join")
            .expect("second app state install");
        assert!(Path::new(&second_installed.path).is_dir());
        let installed = super::install_skill(
            &first_state,
            &registered.id,
            "reviewer",
            "claudeCode",
            Some(project.path().to_string_lossy().as_ref()),
        )
        .await
        .expect("first app state install");
        assert!(Path::new(&installed.path).is_dir());
    }

    #[test]
    fn rollback_errors_include_the_original_and_restore_failures() {
        let error = super::rollback_error(
            "disable skill",
            AppError::Io {
                message: "save ledger".into(),
            },
            AppError::Io {
                message: "restore directory".into(),
            },
        );
        assert!(error.to_string().contains("save ledger"));
        assert!(error.to_string().contains("restore directory"));
    }

    #[tokio::test]
    async fn lifecycle_rejects_renamed_and_replaced_disabled_roots() {
        let app = tempdir().expect("app data");
        let source = tempdir().expect("source");
        let project = tempdir().expect("project");
        let state = test_state(app.path());
        write_skill(source.path(), "reviewer", "reviewer", "Reviews changes");
        let registered = add_local_source(&state, source.path())
            .await
            .expect("register source");
        let project_path = project.path().to_string_lossy().into_owned();
        let installed = super::install_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .expect("install skill");

        let mut records = super::install::load_ledger(&state.app_data_dir)
            .await
            .expect("load ledger");
        records[0].name = "renamed".into();
        super::install::save_ledger(&state.app_data_dir, &records)
            .await
            .expect("seed renamed record");
        assert!(super::disable_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .is_err());
        assert!(Path::new(&installed.path).is_dir());

        records[0].name = "reviewer".into();
        super::install::save_ledger(&state.app_data_dir, &records)
            .await
            .expect("restore record");
        let disabled = super::disable_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .expect("disable skill");
        let disabled_path = super::install::load_ledger(&state.app_data_dir)
            .await
            .expect("load disabled ledger")[0]
            .disabled_path
            .clone()
            .expect("disabled path");
        assert_eq!(disabled.state, SkillInstallState::Disabled);

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let outside = tempdir().expect("outside");
            std::fs::remove_dir_all(&disabled_path).expect("remove disabled root");
            symlink(outside.path(), &disabled_path).expect("replace disabled root with link");
            assert!(super::enable_skill(
                &state,
                &registered.id,
                "reviewer",
                "codex",
                Some(&project_path),
            )
            .await
            .is_err());
        }
    }

    #[tokio::test]
    async fn reconciliation_hashes_disabled_roots_before_reporting_disabled() {
        let app = tempdir().expect("app data");
        let source = tempdir().expect("source");
        let project = tempdir().expect("project");
        let state = test_state(app.path());
        write_skill(source.path(), "reviewer", "reviewer", "Reviews changes");
        let registered = add_local_source(&state, source.path())
            .await
            .expect("register source");
        let project_path = project.path().to_string_lossy().into_owned();
        super::install_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .expect("install skill");
        super::disable_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .expect("disable skill");
        let disabled_path = super::install::load_ledger(&state.app_data_dir)
            .await
            .expect("load disabled ledger")[0]
            .disabled_path
            .clone()
            .expect("disabled path");
        std::fs::write(Path::new(&disabled_path).join("LOCAL.md"), b"modified")
            .expect("modify disabled root");

        let reconciled = super::reconcile_skill_installs(&state, &[project_path])
            .await
            .expect("reconcile disabled skill");
        assert_eq!(reconciled[0].state, SkillInstallState::Modified);
    }

    #[tokio::test]
    async fn missing_project_root_reconciles_missing_and_uninstalls_ledger_only() {
        let app = tempdir().expect("app data");
        let source = tempdir().expect("source");
        let project = tempdir().expect("project");
        let state = test_state(app.path());
        write_skill(source.path(), "reviewer", "reviewer", "Reviews changes");
        let registered = add_local_source(&state, source.path())
            .await
            .expect("register source");
        let project_path = project.path().to_string_lossy().into_owned();
        super::install_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .expect("install skill");
        std::fs::rename(project.path(), app.path().join("unmounted-project"))
            .expect("move project root away");

        let reconciled = super::reconcile_skill_installs(&state, &[])
            .await
            .expect("reconcile missing project");
        assert_eq!(reconciled[0].state, SkillInstallState::Missing);
        assert!(super::uninstall_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .expect("remove missing install ledger"));
        assert!(super::install::load_ledger(&state.app_data_dir)
            .await
            .expect("load ledger")
            .is_empty());
    }

    #[tokio::test]
    async fn enable_reports_source_unavailable_after_restoring_the_active_directory() {
        let app = tempdir().expect("app data");
        let source = tempdir().expect("source");
        let project = tempdir().expect("project");
        let state = test_state(app.path());
        write_skill(source.path(), "reviewer", "reviewer", "Reviews changes");
        let registered = add_local_source(&state, source.path())
            .await
            .expect("register source");
        let project_path = project.path().to_string_lossy().into_owned();
        super::install_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .expect("install skill");
        super::disable_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .expect("disable skill");
        std::fs::remove_dir_all(source.path().join("reviewer")).expect("remove source package");

        let enabled = super::enable_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .expect("enable missing-source skill");
        assert_eq!(enabled.state, SkillInstallState::SourceUnavailable);
        assert!(Path::new(&enabled.path).is_dir());
    }

    #[tokio::test]
    async fn uninstall_leaves_content_created_after_missing_validation_untouched() {
        let app = tempdir().expect("app data");
        let source = tempdir().expect("source");
        let project = tempdir().expect("project");
        let state = test_state(app.path());
        write_skill(source.path(), "reviewer", "reviewer", "Reviews changes");
        let registered = add_local_source(&state, source.path())
            .await
            .expect("register source");
        let project_path = project.path().to_string_lossy().into_owned();
        let installed = super::install_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .expect("install skill");
        std::fs::remove_dir_all(&installed.path).expect("remove validated target");
        super::set_uninstall_missing_probe(
            PathBuf::from(&installed.path),
            Box::new(|target| {
                std::fs::create_dir_all(target).expect("recreate target");
                std::fs::write(target.join("LOCAL.md"), b"arrived after validation")
                    .expect("write replacement content");
            }),
        );

        assert!(super::uninstall_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .expect("ledger-only uninstall"));
        assert_eq!(
            std::fs::read(Path::new(&installed.path).join("LOCAL.md"))
                .expect("replacement content remains"),
            b"arrived after validation"
        );
    }

    #[test]
    fn uninstall_missing_probes_are_isolated_by_target() {
        let root = tempdir().expect("probe targets");
        let first = root.path().join("first");
        let second = root.path().join("second");
        let first_called = Arc::new(AtomicBool::new(false));
        let second_called = Arc::new(AtomicBool::new(false));
        let first_called_by_probe = Arc::clone(&first_called);
        let second_called_by_probe = Arc::clone(&second_called);
        let expected_first = first.clone();
        let expected_second = second.clone();
        super::set_uninstall_missing_probe(
            first.clone(),
            Box::new(move |target| {
                assert_eq!(target, expected_first);
                first_called_by_probe.store(true, Ordering::SeqCst);
            }),
        );
        super::set_uninstall_missing_probe(
            second.clone(),
            Box::new(move |target| {
                assert_eq!(target, expected_second);
                second_called_by_probe.store(true, Ordering::SeqCst);
            }),
        );

        let first_thread =
            std::thread::spawn(move || super::after_missing_uninstall_validation(&first));
        let second_thread =
            std::thread::spawn(move || super::after_missing_uninstall_validation(&second));
        first_thread.join().expect("first probe");
        second_thread.join().expect("second probe");

        assert!(first_called.load(Ordering::SeqCst));
        assert!(second_called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn uninstall_backs_up_replacement_that_arrives_before_quarantine() {
        let app = tempdir().expect("app data");
        let source = tempdir().expect("source");
        let project = tempdir().expect("project");
        let state = test_state(app.path());
        write_skill(source.path(), "reviewer", "reviewer", "Reviews changes");
        let registered = add_local_source(&state, source.path())
            .await
            .expect("register source");
        let project_path = project.path().to_string_lossy().into_owned();
        let installed = super::install_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .expect("install skill");
        let target = PathBuf::from(&installed.path);
        let displaced = target.with_file_name(".pre-quarantine-original");
        super::set_uninstall_before_quarantine_probe(
            target.clone(),
            Box::new(move |target| {
                std::fs::rename(target, &displaced).expect("displace original target");
                std::fs::create_dir(target).expect("create replacement target");
                std::fs::write(target.join("LOCAL.md"), b"replacement before quarantine")
                    .expect("write replacement content");
            }),
        );

        assert!(super::uninstall_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .expect("uninstall replacement"));
        let backups = std::fs::read_dir(state.app_data_dir.join("skill-backups"))
            .expect("replacement backup")
            .collect::<Result<Vec<_>, _>>()
            .expect("backup entries");
        assert_eq!(backups.len(), 1);
        assert_eq!(
            std::fs::read(backups[0].path().join("LOCAL.md")).expect("backed-up replacement"),
            b"replacement before quarantine"
        );
    }

    #[tokio::test]
    async fn uninstall_restores_quarantined_target_when_ledger_write_fails() {
        let app = tempdir().expect("app data");
        let source = tempdir().expect("source");
        let project = tempdir().expect("project");
        let state = test_state(app.path());
        write_skill(source.path(), "reviewer", "reviewer", "Reviews changes");
        let registered = add_local_source(&state, source.path())
            .await
            .expect("register source");
        let project_path = project.path().to_string_lossy().into_owned();
        let installed = super::install_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .expect("install skill");
        let before = super::install::load_ledger(&state.app_data_dir)
            .await
            .expect("initial ledger");
        let ledger = super::install::ledger_path(&state.app_data_dir);
        std::fs::create_dir(ledger.with_extension("json.tmp")).expect("block atomic ledger temp");

        assert!(super::uninstall_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&project_path),
        )
        .await
        .is_err());
        assert!(Path::new(&installed.path).is_dir());
        assert_eq!(
            super::install::load_ledger(&state.app_data_dir)
                .await
                .expect("restored ledger"),
            before
        );
    }
}
