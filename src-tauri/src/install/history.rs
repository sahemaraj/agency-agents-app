use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;
use crate::types::{AgentInstallIdentity, AgentRosterInstallRecord, AgentVersionSnapshot, Scope};
use crate::util::fs::{atomic_write, read_capped};

const MAX_SNAPSHOT_BYTES: u64 = 4 * 1024 * 1024;
const UNJOURNALED_INTENT_FILE: &str = "unjournaled-intent.json";

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotFile {
    name: String,
    sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotManifest {
    identity_hash: String,
    files: Vec<SnapshotFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    roster_record_hash: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum UnjournaledRosterOperationPhase {
    Prepared,
    FilesystemApplied,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UnjournaledRosterHistoryIntent {
    phase: UnjournaledRosterOperationPhase,
    identity: AgentInstallIdentity,
    snapshot_id: String,
    retired_snapshot_ids: Vec<String>,
    previous_index_hash: Option<String>,
    next_index_hash: String,
    expected_record: AgentRosterInstallRecord,
    previous_record: AgentRosterInstallRecord,
    next_record: Option<AgentRosterInstallRecord>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LegacyUnjournaledRosterHistoryIntent {
    identity: AgentInstallIdentity,
    snapshot_id: String,
    retired_snapshot_ids: Vec<String>,
    previous_index_hash: Option<String>,
    next_index_hash: String,
    expected_record: AgentRosterInstallRecord,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum UnjournaledRosterHistoryDocument {
    Current(Box<UnjournaledRosterHistoryIntent>),
    Legacy(Box<LegacyUnjournaledRosterHistoryIntent>),
}

pub(super) struct RosterHistoryMutation {
    pub(super) snapshot: AgentVersionSnapshot,
    app_data_dir: PathBuf,
    identity: AgentInstallIdentity,
    identity_hash: String,
    directory: PathBuf,
    snapshot_directory: PathBuf,
    staging_directory: PathBuf,
    staged_snapshot_directory: PathBuf,
    index_path: PathBuf,
    previous_index: Option<Vec<u8>>,
    next_index: Vec<u8>,
    directory_existed: bool,
    retired: Vec<PathBuf>,
    snapshot_published: bool,
    published: bool,
}

impl RosterHistoryMutation {
    fn validate_owned_paths(&self) -> Result<(), AppError> {
        let directory = app_owned_directory(
            &self.app_data_dir,
            &["agents", "history", &self.identity_hash],
            false,
        )?;
        let staging_id = self
            .staging_directory
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| AppError::StorageCorrupt {
                message: "Agent roster history staging identity changed".into(),
            })?;
        let staging = app_owned_directory(
            &self.app_data_dir,
            &["state", "roster-history-staging", staging_id],
            false,
        )?;
        if directory != self.directory || staging != self.staging_directory {
            return Err(AppError::StorageCorrupt {
                message: "Agent roster history owned path changed".into(),
            });
        }
        Ok(())
    }

    pub(super) fn retired_snapshot_ids(&self) -> Vec<String> {
        self.retired
            .iter()
            .filter_map(|path| path.file_name()?.to_str().map(str::to_owned))
            .collect()
    }

    pub(super) fn staging_path(&self) -> String {
        self.staging_directory.to_string_lossy().into_owned()
    }

    pub(super) fn previous_index_hash(&self) -> Option<String> {
        self.previous_index
            .as_deref()
            .map(crate::render::sha256_hex)
    }

    pub(super) fn next_index_hash(&self) -> String {
        crate::render::sha256_hex(&self.next_index)
    }

    pub(super) async fn prepare_unjournaled_operation(
        &self,
        previous_record: &AgentRosterInstallRecord,
        next_record: Option<&AgentRosterInstallRecord>,
    ) -> Result<(), AppError> {
        self.validate_owned_paths()?;
        let expected_record =
            self.snapshot
                .roster_record
                .clone()
                .ok_or_else(|| AppError::StorageCorrupt {
                    message: "Unjournaled Agent roster history lost its roster metadata".into(),
                })?;
        let intent = UnjournaledRosterHistoryIntent {
            phase: UnjournaledRosterOperationPhase::Prepared,
            identity: self.identity.clone(),
            snapshot_id: self.snapshot.id.clone(),
            retired_snapshot_ids: self.retired_snapshot_ids(),
            previous_index_hash: self.previous_index_hash(),
            next_index_hash: self.next_index_hash(),
            expected_record,
            previous_record: previous_record.clone(),
            next_record: next_record.cloned(),
        };
        validate_unjournaled_intent(&intent)?;
        write_unjournaled_intent(&self.staging_directory, &intent).await
    }

    pub(super) async fn mark_unjournaled_filesystem_applied(&self) -> Result<(), AppError> {
        self.validate_owned_paths()?;
        let intent_path = self.staging_directory.join(UNJOURNALED_INTENT_FILE);
        let bytes = read_capped(&intent_path, MAX_SNAPSHOT_BYTES).await?;
        let mut intent: UnjournaledRosterHistoryIntent =
            serde_json::from_slice(&bytes).map_err(|error| AppError::StorageCorrupt {
                message: format!("parse unjournaled Agent roster history intent: {error}"),
            })?;
        validate_unjournaled_intent(&intent)?;
        if intent.phase != UnjournaledRosterOperationPhase::Prepared {
            return Err(AppError::StorageCorrupt {
                message: "Unjournaled Agent roster operation phase changed".into(),
            });
        }
        intent.phase = UnjournaledRosterOperationPhase::FilesystemApplied;
        write_unjournaled_intent(&self.staging_directory, &intent).await
    }
}

async fn write_unjournaled_intent(
    staging_directory: &Path,
    intent: &UnjournaledRosterHistoryIntent,
) -> Result<(), AppError> {
    let bytes = serde_json::to_vec_pretty(&intent).map_err(|error| AppError::Internal {
        message: format!("serialize unjournaled Agent roster history intent: {error}"),
    })?;
    let path = staging_directory.join(UNJOURNALED_INTENT_FILE);
    atomic_write(&path, &bytes).await?;
    sync_parent_directory_entry(&path)
}

async fn publish_history_index_after_snapshot_move_with<F>(
    staged_snapshot: &Path,
    live_snapshot: &Path,
    index_path: &Path,
    index_bytes: &[u8],
    mut sync_parent: F,
) -> Result<(), AppError>
where
    F: FnMut(&Path) -> Result<(), AppError>,
{
    sync_parent(staged_snapshot)?;
    sync_parent(live_snapshot)?;
    atomic_write(index_path, index_bytes).await?;
    sync_parent(index_path)
}

impl RosterHistoryMutation {
    pub(super) async fn publish(&mut self) -> Result<(), AppError> {
        self.publish_with_sync(sync_parent_directory_entry).await
    }

    async fn publish_with_sync<F>(&mut self, mut sync_parent: F) -> Result<(), AppError>
    where
        F: FnMut(&Path) -> Result<(), AppError>,
    {
        if self.published {
            return Ok(());
        }
        self.validate_owned_paths()?;
        let current_index = match read_capped(&self.index_path, MAX_SNAPSHOT_BYTES).await {
            Ok(bytes) => Some(bytes),
            Err(AppError::Io { .. }) if !self.index_path.exists() => None,
            Err(error) => return Err(error),
        };
        if current_index != self.previous_index {
            return Err(AppError::StorageCorrupt {
                message: "Agent roster history changed before publication".into(),
            });
        }
        validate_snapshot_directory(
            &self.staged_snapshot_directory,
            &self.identity_hash,
            &self.snapshot,
        )
        .await?;
        let directory = app_owned_directory(
            &self.app_data_dir,
            &["agents", "history", &self.identity_hash],
            true,
        )?;
        if directory != self.directory {
            return Err(AppError::StorageCorrupt {
                message: "Agent history directory identity changed before publication".into(),
            });
        }
        match tokio::fs::rename(&self.staged_snapshot_directory, &self.snapshot_directory).await {
            Ok(()) => {}
            Err(error) => {
                return Err(AppError::Io {
                    message: format!("publish staged Agent roster snapshot: {error}"),
                });
            }
        }
        self.snapshot_published = true;
        let published = publish_history_index_after_snapshot_move_with(
            &self.staged_snapshot_directory,
            &self.snapshot_directory,
            &self.index_path,
            &self.next_index,
            |path| sync_parent(path),
        )
        .await;
        match published {
            Ok(()) => {
                self.published = true;
                Ok(())
            }
            Err(error) => match self
                .restore_publication_with_sync(true, &mut sync_parent)
                .await
            {
                Ok(()) => Err(error),
                Err(rollback) => Err(AppError::Internal {
                    message: format!(
                        "publish Agent roster history failed: {error}; restore prior publication failed: {rollback}"
                    ),
                }),
            },
        }
    }

    async fn restore_publication_with_sync<F>(
        &mut self,
        restore_index: bool,
        sync_parent: &mut F,
    ) -> Result<(), AppError>
    where
        F: FnMut(&Path) -> Result<(), AppError>,
    {
        let mut failures = Vec::new();
        if self.published || restore_index {
            let restored_index = async {
                match &self.previous_index {
                    Some(bytes) => atomic_write(&self.index_path, bytes).await?,
                    None => match tokio::fs::remove_file(&self.index_path).await {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => {
                            return Err(AppError::Io {
                                message: format!("remove Agent roster history index: {error}"),
                            });
                        }
                    },
                }
                sync_parent(&self.index_path)
            }
            .await;
            match restored_index {
                Ok(()) => self.published = false,
                Err(error) => failures.push(error.to_string()),
            }
        }
        if self.snapshot_published {
            let restored_snapshot = async {
                let staged = std::fs::symlink_metadata(&self.staged_snapshot_directory);
                let live = std::fs::symlink_metadata(&self.snapshot_directory);
                match (staged, live) {
                    (Err(staged), Ok(_))
                        if staged.kind() == std::io::ErrorKind::NotFound =>
                    {
                        tokio::fs::rename(
                            &self.snapshot_directory,
                            &self.staged_snapshot_directory,
                        )
                        .await
                        .map_err(|error| AppError::Io {
                            message: format!(
                                "restore staged Agent roster snapshot after publication failure: {error}"
                            ),
                        })?;
                    }
                    (Ok(_), Err(live)) if live.kind() == std::io::ErrorKind::NotFound => {}
                    (Err(staged), Err(live))
                        if staged.kind() == std::io::ErrorKind::NotFound
                            && live.kind() == std::io::ErrorKind::NotFound =>
                    {
                        return Err(AppError::StorageCorrupt {
                            message: "Agent roster publication rollback lost its snapshot".into(),
                        });
                    }
                    (Ok(_), Ok(_)) => {
                        return Err(AppError::StorageCorrupt {
                            message: "Agent roster publication rollback found duplicate snapshots"
                                .into(),
                        });
                    }
                    (Err(error), _) | (_, Err(error)) => {
                        return Err(AppError::Io {
                            message: format!(
                                "inspect Agent roster publication rollback snapshot: {error}"
                            ),
                        });
                    }
                }
                sync_parent(&self.snapshot_directory)?;
                sync_parent(&self.staged_snapshot_directory)
            }
            .await;
            match restored_snapshot {
                Ok(()) => self.snapshot_published = false,
                Err(error) => failures.push(error.to_string()),
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(AppError::Internal {
                message: failures.join("; "),
            })
        }
    }

    pub(super) async fn commit(self) -> Result<(), AppError> {
        if !self.published || !self.snapshot_published {
            return Err(AppError::StorageCorrupt {
                message: "Agent roster history was not published before commit".into(),
            });
        }
        self.validate_owned_paths()?;
        for path in &self.retired {
            if path.parent() != Some(self.directory.as_path()) {
                return Err(AppError::StorageCorrupt {
                    message: "Retired Agent roster snapshot escaped its history directory".into(),
                });
            }
            match tokio::fs::remove_dir_all(&path).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(AppError::Io {
                        message: format!("prune retired Agent roster snapshot: {error}"),
                    });
                }
            }
            sync_parent_directory_entry(path)?;
        }
        cleanup_roster_staging(&self.app_data_dir, &self.staging_path()).await?;
        Ok(())
    }

    pub(super) async fn rollback(mut self) -> Result<(), AppError> {
        self.validate_owned_paths()?;
        let history_was_absent = self.previous_index.is_none();
        self.restore_publication_with_sync(false, &mut sync_parent_directory_entry)
            .await?;
        cleanup_roster_staging(&self.app_data_dir, &self.staging_path()).await?;
        let restored_index = match read_capped(&self.index_path, MAX_SNAPSHOT_BYTES).await {
            Ok(bytes) => Some(bytes),
            Err(AppError::Io { .. }) if !self.index_path.exists() => None,
            Err(error) => return Err(error),
        };
        if restored_index != self.previous_index {
            return Err(AppError::StorageCorrupt {
                message: "Agent roster history rollback did not restore its exact index".into(),
            });
        }
        if history_was_absent && !self.directory_existed {
            let _ = tokio::fs::remove_dir(&self.directory).await;
            let _ = tokio::fs::remove_dir(self.directory.parent().unwrap_or(&self.directory)).await;
        }
        for retired in &self.retired {
            if !retired.is_dir() {
                return Err(AppError::StorageCorrupt {
                    message: "Agent roster history rollback lost a retained snapshot".into(),
                });
            }
        }
        Ok(())
    }
}

pub(super) async fn commit_roster_retention(
    app_data_dir: &Path,
    identity: &AgentInstallIdentity,
    retired_snapshot_ids: &[String],
) -> Result<(), AppError> {
    commit_roster_retention_with_sync(
        app_data_dir,
        identity,
        retired_snapshot_ids,
        sync_parent_directory_entry,
    )
    .await
}

async fn commit_roster_retention_with_sync<F>(
    app_data_dir: &Path,
    identity: &AgentInstallIdentity,
    retired_snapshot_ids: &[String],
    mut sync_parent: F,
) -> Result<(), AppError>
where
    F: FnMut(&Path) -> Result<(), AppError>,
{
    if retired_snapshot_ids.is_empty() {
        return Ok(());
    }
    let identity_hash = identity_hash(identity)?;
    let directory =
        app_owned_directory(app_data_dir, &["agents", "history", &identity_hash], false)?;
    let retained = load_index(&directory.join("index.json"))
        .await?
        .into_iter()
        .map(|snapshot| snapshot.id)
        .collect::<std::collections::BTreeSet<_>>();
    for id in retired_snapshot_ids {
        if id.is_empty()
            || !id.is_ascii()
            || id.len() <= 36
            || uuid::Uuid::parse_str(&id[id.len() - 36..]).is_err()
            || Path::new(id).components().count() != 1
            || retained.contains(id)
        {
            return Err(AppError::StorageCorrupt {
                message: "Agent roster retention recovery metadata is invalid".into(),
            });
        }
        let path = directory.join(id);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata)
                if !metadata.is_dir()
                    || metadata.file_type().is_symlink()
                    || crate::skills::metadata_is_reparse_point(&metadata) =>
            {
                return Err(AppError::StorageCorrupt {
                    message: "Retired Agent roster snapshot is not a regular directory".into(),
                });
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                sync_parent(&path)?;
                continue;
            }
            Err(error) => {
                return Err(AppError::Io {
                    message: format!("inspect retired Agent roster snapshot: {error}"),
                });
            }
        }
        match tokio::fs::remove_dir_all(&path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(AppError::Io {
                    message: format!("prune retired Agent roster snapshot: {error}"),
                });
            }
        }
        sync_parent(&path)?;
    }
    Ok(())
}

fn validated_roster_staging_directory(
    app_data_dir: &Path,
    staging_path: &str,
) -> Result<PathBuf, AppError> {
    let path = PathBuf::from(staging_path);
    let expected_parent =
        app_owned_directory(app_data_dir, &["state", "roster-history-staging"], false)?;
    let valid_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| Uuid::parse_str(name).is_ok());
    if path.parent() != Some(expected_parent.as_path()) || !valid_name {
        return Err(AppError::StorageCorrupt {
            message: "Agent roster history staging path escaped app storage".into(),
        });
    }
    Ok(path)
}

pub(super) fn validate_roster_staging_reference(
    app_data_dir: &Path,
    staging_path: &str,
) -> Result<(), AppError> {
    validated_roster_staging_directory(app_data_dir, staging_path).map(|_| ())
}

async fn optional_index_bytes(path: &Path) -> Result<Option<Vec<u8>>, AppError> {
    match read_capped(path, MAX_SNAPSHOT_BYTES).await {
        Ok(bytes) => Ok(Some(bytes)),
        Err(AppError::Io { .. }) if !path.exists() => Ok(None),
        Err(error) => Err(error),
    }
}

fn index_hash(bytes: Option<&[u8]>) -> Option<String> {
    bytes.map(crate::render::sha256_hex)
}

fn validate_publication_transition(
    previous: &[AgentVersionSnapshot],
    next: &[AgentVersionSnapshot],
    snapshot_id: &str,
    retired_snapshot_ids: &[String],
    expected_record: &crate::types::AgentRosterInstallRecord,
    expected_content_path: &Path,
) -> Result<AgentVersionSnapshot, AppError> {
    let snapshot = next
        .iter()
        .find(|snapshot| snapshot.id == snapshot_id)
        .cloned()
        .ok_or_else(|| AppError::StorageCorrupt {
            message: "Agent roster staged index lost its preservation snapshot".into(),
        })?;
    if snapshot.roster_record.as_ref() != Some(expected_record)
        || snapshot.rendered_hash != expected_record.rendered_hash
        || Path::new(&snapshot.content_path) != expected_content_path
    {
        return Err(AppError::StorageCorrupt {
            message: "Agent roster staged preservation metadata changed".into(),
        });
    }
    let retired = retired_snapshot_ids
        .iter()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    let removed = previous
        .iter()
        .filter(|candidate| !next.iter().any(|next| next.id == candidate.id))
        .map(|candidate| candidate.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    if removed != retired
        || next.len() + retired.len() != previous.len() + 1
        || previous.iter().any(|candidate| {
            !retired.contains(candidate.id.as_str()) && !next.iter().any(|next| next == candidate)
        })
        || next.iter().any(|candidate| {
            candidate.id != snapshot_id && !previous.iter().any(|previous| previous == candidate)
        })
    {
        return Err(AppError::StorageCorrupt {
            message: "Agent roster staged history transition changed".into(),
        });
    }
    Ok(snapshot)
}

async fn validate_snapshot_directory(
    directory: &Path,
    identity_hash: &str,
    snapshot: &AgentVersionSnapshot,
) -> Result<(), AppError> {
    let metadata = std::fs::symlink_metadata(directory).map_err(|error| AppError::Io {
        message: format!("inspect staged Agent roster snapshot: {error}"),
    })?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || crate::skills::metadata_is_reparse_point(&metadata)
    {
        return Err(AppError::StorageCorrupt {
            message: "Staged Agent roster snapshot is not a regular directory".into(),
        });
    }
    let manifest_path = directory.join("manifest.json");
    regular_file(&manifest_path)?;
    let content_directory = directory.join("content");
    let content_metadata =
        std::fs::symlink_metadata(&content_directory).map_err(|error| AppError::Io {
            message: format!("inspect staged Agent snapshot content directory: {error}"),
        })?;
    if !content_metadata.is_dir()
        || content_metadata.file_type().is_symlink()
        || crate::skills::metadata_is_reparse_point(&content_metadata)
        || std::fs::canonicalize(&content_directory).map_err(|error| AppError::Io {
            message: format!("resolve staged Agent snapshot content directory: {error}"),
        })? != content_directory
    {
        return Err(AppError::StorageCorrupt {
            message: "Staged Agent snapshot content directory contains a link or reparse point"
                .into(),
        });
    }
    let manifest_bytes = read_capped(&manifest_path, MAX_SNAPSHOT_BYTES).await?;
    let manifest: SnapshotManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|error| AppError::StorageCorrupt {
            message: format!("parse staged Agent roster snapshot manifest: {error}"),
        })?;
    let expected_roster_record_hash = snapshot
        .roster_record
        .as_ref()
        .map(roster_record_hash)
        .transpose()?;
    if manifest.identity_hash != identity_hash
        || manifest.roster_record_hash != expected_roster_record_hash
        || manifest.files.is_empty()
        || manifest.files.len() != snapshot.artifact_hashes.len()
    {
        return Err(AppError::StorageCorrupt {
            message: "Staged Agent roster snapshot manifest changed".into(),
        });
    }
    for (index, file) in manifest.files.iter().enumerate() {
        if file.name != format!("{index}.bin") {
            return Err(AppError::StorageCorrupt {
                message: "Staged Agent roster snapshot filename changed".into(),
            });
        }
        let content = content_directory.join(&file.name);
        regular_file(&content)?;
        let bytes = read_capped(&content, MAX_SNAPSHOT_BYTES).await?;
        let hash = crate::render::sha256_hex(&bytes);
        if hash != file.sha256 || snapshot.artifact_hashes[index] != hash {
            return Err(AppError::StorageCorrupt {
                message: "Staged Agent roster snapshot content changed".into(),
            });
        }
        if snapshot.roster_record.is_some()
            && (manifest.files.len() != 1 || hash != snapshot.rendered_hash)
        {
            return Err(AppError::StorageCorrupt {
                message: "Staged Agent roster snapshot rendered hash changed".into(),
            });
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn recover_roster_publication(
    app_data_dir: &Path,
    identity: &AgentInstallIdentity,
    snapshot_id: &str,
    retired_snapshot_ids: &[String],
    staging_path: &str,
    previous_index_hash: Option<&str>,
    next_index_hash: &str,
    expected_record: &crate::types::AgentRosterInstallRecord,
) -> Result<(), AppError> {
    if snapshot_id.is_empty()
        || !snapshot_id.is_ascii()
        || snapshot_id.len() <= 36
        || Uuid::parse_str(&snapshot_id[snapshot_id.len() - 36..]).is_err()
        || Path::new(snapshot_id).components().count() != 1
    {
        return Err(AppError::StorageCorrupt {
            message: "Agent roster preservation snapshot id is invalid".into(),
        });
    }
    let identity_hash = identity_hash(identity)?;
    let directory =
        app_owned_directory(app_data_dir, &["agents", "history", &identity_hash], false)?;
    let index_path = directory.join("index.json");
    let staging = validated_roster_staging_directory(app_data_dir, staging_path)?;
    let staged_snapshot = staging.join("snapshot");
    let current = optional_index_bytes(&index_path).await?;
    let current_hash = index_hash(current.as_deref());
    let final_snapshot = directory.join(snapshot_id);
    if final_snapshot.parent() != Some(directory.as_path()) {
        return Err(AppError::StorageCorrupt {
            message: "Agent roster preservation escaped its history directory".into(),
        });
    }
    if current_hash.as_deref() == Some(next_index_hash) {
        let next: Vec<AgentVersionSnapshot> =
            serde_json::from_slice(current.as_deref().ok_or_else(|| AppError::StorageCorrupt {
                message: "Published Agent roster history index disappeared".into(),
            })?)
            .map_err(|error| AppError::StorageCorrupt {
                message: format!("parse published Agent roster history: {error}"),
            })?;
        let snapshot = next
            .into_iter()
            .find(|snapshot| snapshot.id == snapshot_id)
            .ok_or_else(|| AppError::StorageCorrupt {
                message: "Published Agent roster history lost its snapshot".into(),
            })?;
        if snapshot.roster_record.as_ref() != Some(expected_record)
            || Path::new(&snapshot.content_path) != final_snapshot
        {
            return Err(AppError::StorageCorrupt {
                message: "Published Agent roster preservation changed".into(),
            });
        }
        validate_snapshot_directory(&final_snapshot, &identity_hash, &snapshot).await?;
        sync_parent_directory_entry(&staged_snapshot)?;
        sync_parent_directory_entry(&final_snapshot)?;
        sync_parent_directory_entry(&index_path)?;
        return Ok(());
    }
    if current_hash.as_deref() != previous_index_hash {
        return Err(AppError::StorageCorrupt {
            message: "Agent roster history changed before recovery publication".into(),
        });
    }
    let staged_index = read_capped(&staging.join("index.json"), MAX_SNAPSHOT_BYTES).await?;
    if crate::render::sha256_hex(&staged_index) != next_index_hash {
        return Err(AppError::StorageCorrupt {
            message: "Staged Agent roster history index changed".into(),
        });
    }
    let previous: Vec<AgentVersionSnapshot> = match current.as_deref() {
        Some(bytes) => serde_json::from_slice(bytes).map_err(|error| AppError::StorageCorrupt {
            message: format!("parse prior Agent roster history: {error}"),
        })?,
        None => Vec::new(),
    };
    let next: Vec<AgentVersionSnapshot> =
        serde_json::from_slice(&staged_index).map_err(|error| AppError::StorageCorrupt {
            message: format!("parse staged Agent roster history: {error}"),
        })?;
    let snapshot = validate_publication_transition(
        &previous,
        &next,
        snapshot_id,
        retired_snapshot_ids,
        expected_record,
        &final_snapshot,
    )?;
    let (source, must_move) = match (
        std::fs::symlink_metadata(&staged_snapshot),
        std::fs::symlink_metadata(&final_snapshot),
    ) {
        (Ok(_), Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            (staged_snapshot.as_path(), true)
        }
        (Err(error), Ok(_)) if error.kind() == std::io::ErrorKind::NotFound => {
            (final_snapshot.as_path(), false)
        }
        _ => {
            return Err(AppError::StorageCorrupt {
                message: "Agent roster preservation staging state is ambiguous".into(),
            });
        }
    };
    validate_snapshot_directory(source, &identity_hash, &snapshot).await?;
    let created_directory =
        app_owned_directory(app_data_dir, &["agents", "history", &identity_hash], true)?;
    if created_directory != directory {
        return Err(AppError::StorageCorrupt {
            message: "Agent roster history directory changed during recovery".into(),
        });
    }
    if must_move {
        tokio::fs::rename(&staged_snapshot, &final_snapshot)
            .await
            .map_err(|error| AppError::Io {
                message: format!("recover staged Agent roster snapshot: {error}"),
            })?;
    }
    publish_history_index_after_snapshot_move_with(
        &staged_snapshot,
        &final_snapshot,
        &index_path,
        &staged_index,
        sync_parent_directory_entry,
    )
    .await?;
    let published = read_capped(&index_path, MAX_SNAPSHOT_BYTES).await?;
    if crate::render::sha256_hex(&published) != next_index_hash {
        return Err(AppError::StorageCorrupt {
            message: "Recovered Agent roster history publication failed verification".into(),
        });
    }
    Ok(())
}

pub(super) async fn cleanup_roster_staging(
    app_data_dir: &Path,
    staging_path: &str,
) -> Result<(), AppError> {
    cleanup_roster_staging_with_sync(app_data_dir, staging_path, sync_parent_directory_entry).await
}

async fn cleanup_roster_staging_with_sync<F>(
    app_data_dir: &Path,
    staging_path: &str,
    mut sync_parent: F,
) -> Result<(), AppError>
where
    F: FnMut(&Path) -> Result<(), AppError>,
{
    let path = validated_roster_staging_directory(app_data_dir, staging_path)?;
    match std::fs::symlink_metadata(&path) {
        Ok(metadata)
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || crate::skills::metadata_is_reparse_point(&metadata) =>
        {
            return Err(AppError::StorageCorrupt {
                message: "Agent roster history staging is not a regular directory".into(),
            });
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(AppError::Io {
                message: format!("inspect Agent roster history staging: {error}"),
            });
        }
    }
    let intent = path.join(UNJOURNALED_INTENT_FILE);
    let intent_bytes = match read_capped(&intent, MAX_SNAPSHOT_BYTES).await {
        Ok(bytes) => Some(bytes),
        Err(AppError::Io { .. }) if !intent.exists() => None,
        Err(error) => return Err(error),
    };
    if let Some(intent_bytes) = intent_bytes {
        tokio::fs::remove_file(&intent)
            .await
            .map_err(|error| AppError::Io {
                message: format!("remove Agent roster history intent: {error}"),
            })?;
        if let Err(error) = sync_parent(&intent) {
            let restored = async {
                atomic_write(&intent, &intent_bytes).await?;
                sync_parent(&intent)
            }
            .await;
            return match restored {
                Ok(()) => Err(error),
                Err(restore) => Err(AppError::Internal {
                    message: format!(
                        "sync Agent roster history intent deletion failed: {error}; restore intent failed: {restore}"
                    ),
                }),
            };
        }
    }
    tokio::fs::remove_dir_all(&path)
        .await
        .map_err(|error| AppError::Io {
            message: format!("remove Agent roster history staging: {error}"),
        })?;
    sync_parent(&path)
}

fn validate_unjournaled_identity(
    identity: &AgentInstallIdentity,
    expected_record: &AgentRosterInstallRecord,
) -> Result<(), AppError> {
    let project_path =
        identity
            .project_path
            .as_deref()
            .ok_or_else(|| AppError::StorageCorrupt {
                message: "Unjournaled Agent roster history identity is not project scoped".into(),
            })?;
    let expected_relative_path = format!(
        "projects/{}.md",
        crate::render::sha256_hex(expected_record.project_path.as_bytes())
    );
    if identity.scope != Scope::Project
        || project_path != expected_record.project_path
        || identity.tool != expected_record.tool
        || identity.reference.source_id != format!("roster:{}", expected_record.tool)
        || identity.reference.relative_path != expected_relative_path
    {
        return Err(AppError::StorageCorrupt {
            message: "Unjournaled Agent roster history identity changed".into(),
        });
    }
    Ok(())
}

fn validate_unjournaled_intent(intent: &UnjournaledRosterHistoryIntent) -> Result<(), AppError> {
    validate_unjournaled_identity(&intent.identity, &intent.expected_record)?;
    if intent.expected_record.tool != intent.previous_record.tool
        || intent.expected_record.scope != intent.previous_record.scope
        || intent.expected_record.project_path != intent.previous_record.project_path
        || intent.expected_record.dest != intent.previous_record.dest
        || intent.expected_record.members != intent.previous_record.members
        || intent.expected_record.disabled_path != intent.previous_record.disabled_path
        || intent.expected_record.installed_at != intent.previous_record.installed_at
        || intent.next_record.as_ref().is_some_and(|next| {
            next.tool != intent.previous_record.tool
                || next.project_path != intent.previous_record.project_path
        })
    {
        return Err(AppError::StorageCorrupt {
            message: "Unjournaled Agent roster history identity changed".into(),
        });
    }
    Ok(())
}

fn roster_file_state_matches(
    record: Option<&AgentRosterInstallRecord>,
    active_hash: Option<&str>,
    disabled_hash: Option<&str>,
) -> bool {
    match record {
        Some(record) if record.disabled_path.is_some() => {
            active_hash.is_none() && disabled_hash == Some(record.rendered_hash.as_str())
        }
        Some(record) => {
            active_hash == Some(record.rendered_hash.as_str()) && disabled_hash.is_none()
        }
        None => active_hash.is_none() && disabled_hash.is_none(),
    }
}

async fn recover_unjournaled_roster_publication(
    app_data_dir: &Path,
    staging_path: &str,
    intent: &UnjournaledRosterHistoryIntent,
) -> Result<(), AppError> {
    recover_roster_publication(
        app_data_dir,
        &intent.identity,
        &intent.snapshot_id,
        &intent.retired_snapshot_ids,
        staging_path,
        intent.previous_index_hash.as_deref(),
        &intent.next_index_hash,
        &intent.expected_record,
    )
    .await
}

async fn finalize_recovered_unjournaled_roster(
    app_data_dir: &Path,
    staging_path: &str,
    intent: &UnjournaledRosterHistoryIntent,
) -> Result<(), AppError> {
    commit_roster_retention(app_data_dir, &intent.identity, &intent.retired_snapshot_ids).await?;
    cleanup_roster_staging(app_data_dir, staging_path).await
}

pub(super) async fn recover_unjournaled_roster_operations(
    state: &crate::state::AppState,
) -> Result<(), AppError> {
    let app_data_dir = &state.app_data_dir;
    let root = app_owned_directory(app_data_dir, &["state", "roster-history-staging"], false)?;
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(AppError::Io {
                message: format!("read unjournaled Agent roster history staging: {error}"),
            });
        }
    };
    let mut paths = entries
        .take(1025)
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|error| AppError::Io {
                    message: format!("read unjournaled Agent roster history entry: {error}"),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if paths.len() > 1024 {
        return Err(AppError::StorageCorrupt {
            message: "Agent roster history staging exceeds its entry limit".into(),
        });
    }
    paths.sort();
    for staging in paths {
        let staging_path = staging.to_string_lossy().into_owned();
        validated_roster_staging_directory(app_data_dir, &staging_path)?;
        let metadata = std::fs::symlink_metadata(&staging).map_err(|error| AppError::Io {
            message: format!("inspect unjournaled Agent roster history staging: {error}"),
        })?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || crate::skills::metadata_is_reparse_point(&metadata)
        {
            return Err(AppError::StorageCorrupt {
                message: "Unjournaled Agent roster history staging is not a regular directory"
                    .into(),
            });
        }
        let intent_path = staging.join(UNJOURNALED_INTENT_FILE);
        match std::fs::symlink_metadata(&intent_path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(AppError::Io {
                    message: format!("inspect unjournaled Agent roster history intent: {error}"),
                });
            }
            Ok(metadata)
                if !metadata.is_file()
                    || metadata.file_type().is_symlink()
                    || crate::skills::metadata_is_reparse_point(&metadata) =>
            {
                return Err(AppError::StorageCorrupt {
                    message: "Unjournaled Agent roster history intent is not a regular file".into(),
                });
            }
            Ok(_) => {}
        }
        let bytes = read_capped(&intent_path, MAX_SNAPSHOT_BYTES).await?;
        let document: UnjournaledRosterHistoryDocument =
            serde_json::from_slice(&bytes).map_err(|error| AppError::StorageCorrupt {
                message: format!("parse unjournaled Agent roster history intent: {error}"),
            })?;
        let intent = match document {
            UnjournaledRosterHistoryDocument::Current(intent) => *intent,
            UnjournaledRosterHistoryDocument::Legacy(intent) => {
                let intent = *intent;
                validate_unjournaled_identity(&intent.identity, &intent.expected_record)?;
                recover_roster_publication(
                    app_data_dir,
                    &intent.identity,
                    &intent.snapshot_id,
                    &intent.retired_snapshot_ids,
                    &staging_path,
                    intent.previous_index_hash.as_deref(),
                    &intent.next_index_hash,
                    &intent.expected_record,
                )
                .await?;
                commit_roster_retention(
                    app_data_dir,
                    &intent.identity,
                    &intent.retired_snapshot_ids,
                )
                .await?;
                cleanup_roster_staging(app_data_dir, &staging_path).await?;
                continue;
            }
        };
        validate_unjournaled_intent(&intent)?;
        super::validate_roster_record(&intent.previous_record).map_err(|_| {
            AppError::StorageCorrupt {
                message: "Unjournaled Agent roster previous metadata is invalid".into(),
            }
        })?;
        if let Some(next) = &intent.next_record {
            super::validate_roster_record(next).map_err(|_| AppError::StorageCorrupt {
                message: "Unjournaled Agent roster next metadata is invalid".into(),
            })?;
        }
        let (active, disabled) =
            super::validated_recovery_roster_paths(state, &intent.previous_record).await?;
        if let Some(next) = &intent.next_record {
            let next_paths = super::validated_recovery_roster_paths(state, next).await?;
            if next_paths != (active.clone(), disabled.clone()) {
                return Err(AppError::StorageCorrupt {
                    message: "Unjournaled Agent roster destinations changed".into(),
                });
            }
        }
        let active_hash = super::recovery_file_hash(&active)?;
        let disabled_hash = super::recovery_file_hash(&disabled)?;
        let files_are_previous = roster_file_state_matches(
            Some(&intent.expected_record),
            active_hash.as_deref(),
            disabled_hash.as_deref(),
        );
        let files_are_next = roster_file_state_matches(
            intent.next_record.as_ref(),
            active_hash.as_deref(),
            disabled_hash.as_deref(),
        );
        let mut rosters = super::load_rosters_for_state(state).await?;
        let matching = rosters
            .iter()
            .enumerate()
            .filter(|(_, record)| super::same_roster_install(record, &intent.previous_record))
            .collect::<Vec<_>>();
        if matching.len() > 1 {
            return Err(AppError::StorageCorrupt {
                message: "Unjournaled Agent roster ledger contains duplicate identity rows".into(),
            });
        }
        let ledger_is_previous = matching.first().is_some_and(|(_, record)| {
            super::exact_roster_install(record, &intent.previous_record)
        });
        let ledger_is_next = match (&intent.next_record, matching.first()) {
            (Some(next), Some((_, record))) => super::exact_roster_install(record, next),
            (None, None) => true,
            _ => false,
        };

        match intent.phase {
            UnjournaledRosterOperationPhase::Prepared
                if files_are_previous && ledger_is_previous =>
            {
                cleanup_roster_staging(app_data_dir, &staging_path).await?;
            }
            UnjournaledRosterOperationPhase::Prepared if files_are_next && ledger_is_previous => {
                recover_unjournaled_roster_publication(app_data_dir, &staging_path, &intent)
                    .await?;
                let index = matching[0].0;
                match &intent.next_record {
                    Some(next) => rosters[index] = next.clone(),
                    None => {
                        rosters.remove(index);
                    }
                }
                rosters.sort_by(|left, right| {
                    left.project_path
                        .cmp(&right.project_path)
                        .then_with(|| left.tool.cmp(&right.tool))
                });
                super::save_rosters_for_state(state, &rosters).await?;
                let mut applied = intent.clone();
                applied.phase = UnjournaledRosterOperationPhase::FilesystemApplied;
                write_unjournaled_intent(&staging, &applied).await?;
                finalize_recovered_unjournaled_roster(app_data_dir, &staging_path, &applied)
                    .await?;
            }
            UnjournaledRosterOperationPhase::Prepared if files_are_next && ledger_is_next => {
                recover_unjournaled_roster_publication(app_data_dir, &staging_path, &intent)
                    .await?;
                let mut applied = intent.clone();
                applied.phase = UnjournaledRosterOperationPhase::FilesystemApplied;
                write_unjournaled_intent(&staging, &applied).await?;
                finalize_recovered_unjournaled_roster(app_data_dir, &staging_path, &applied)
                    .await?;
            }
            UnjournaledRosterOperationPhase::FilesystemApplied
                if files_are_next && ledger_is_next =>
            {
                recover_unjournaled_roster_publication(app_data_dir, &staging_path, &intent)
                    .await?;
                finalize_recovered_unjournaled_roster(app_data_dir, &staging_path, &intent).await?;
            }
            _ => {
                return Err(AppError::StorageCorrupt {
                    message: "Unjournaled Agent roster operation found mixed or changed state"
                        .into(),
                });
            }
        }
    }
    Ok(())
}

pub(super) async fn sweep_roster_staging(
    app_data_dir: &Path,
    retained_staging_paths: &[String],
) -> Result<(), AppError> {
    let retained = retained_staging_paths
        .iter()
        .map(|path| validated_roster_staging_directory(app_data_dir, path))
        .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
    let root = app_owned_directory(app_data_dir, &["state", "roster-history-staging"], false)?;
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(AppError::Io {
                message: format!("read Agent roster history staging: {error}"),
            });
        }
    };
    for (index, entry) in entries.enumerate() {
        if index >= 1024 {
            return Err(AppError::StorageCorrupt {
                message: "Agent roster history staging exceeds its entry limit".into(),
            });
        }
        let entry = entry.map_err(|error| AppError::Io {
            message: format!("read Agent roster history staging entry: {error}"),
        })?;
        let path = entry.path();
        let name = entry.file_name();
        let valid_name = name
            .to_str()
            .is_some_and(|name| Uuid::parse_str(name).is_ok());
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| AppError::Io {
            message: format!("inspect Agent roster history staging entry: {error}"),
        })?;
        if !valid_name
            || path.parent() != Some(root.as_path())
            || !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || crate::skills::metadata_is_reparse_point(&metadata)
        {
            return Err(AppError::StorageCorrupt {
                message: "Agent roster history staging contains an invalid entry".into(),
            });
        }
        if !retained.contains(&path) {
            tokio::fs::remove_dir_all(&path)
                .await
                .map_err(|error| AppError::Io {
                    message: format!("remove orphaned Agent roster history staging: {error}"),
                })?;
        }
    }
    Ok(())
}

fn roster_record_hash(record: &crate::types::AgentRosterInstallRecord) -> Result<String, AppError> {
    let bytes = serde_json::to_vec(record).map_err(|error| AppError::Internal {
        message: format!("serialize Agent roster snapshot metadata: {error}"),
    })?;
    Ok(crate::render::sha256_hex(&bytes))
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::InvalidArgument {
        message: message.into(),
    }
}

fn app_owned_directory(
    app_data_dir: &Path,
    components: &[&str],
    create: bool,
) -> Result<PathBuf, AppError> {
    let mut current = std::fs::canonicalize(app_data_dir).map_err(|error| AppError::Io {
        message: format!("resolve Agent app-data directory: {error}"),
    })?;
    for (index, component) in components.iter().enumerate() {
        let candidate = current.join(component);
        if create {
            match std::fs::create_dir(&candidate) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(AppError::Io {
                        message: format!("create app-owned Agent history directory: {error}"),
                    });
                }
            }
        }
        let metadata = match std::fs::symlink_metadata(&candidate) {
            Ok(metadata) => metadata,
            Err(error) if !create && error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(components[index..]
                    .iter()
                    .fold(current, |path, component| path.join(component)));
            }
            Err(error) => {
                return Err(AppError::Io {
                    message: format!("inspect app-owned Agent history directory: {error}"),
                });
            }
        };
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || crate::skills::metadata_is_reparse_point(&metadata)
            || std::fs::canonicalize(&candidate).map_err(|error| AppError::Io {
                message: format!("resolve app-owned Agent history directory: {error}"),
            })? != candidate
        {
            return Err(AppError::StorageCorrupt {
                message: format!(
                    "App-owned Agent history directory contains a link or reparse point: {}",
                    candidate.display()
                ),
            });
        }
        current = candidate;
    }
    Ok(current)
}

#[cfg(unix)]
fn sync_parent_directory_entry(path: &Path) -> Result<(), AppError> {
    let parent = path.parent().ok_or_else(|| AppError::Io {
        message: format!("Agent history path has no parent: {}", path.display()),
    })?;
    std::fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| AppError::Io {
            message: format!("sync Agent history directory {}: {error}", parent.display()),
        })
}

#[cfg(not(unix))]
fn sync_parent_directory_entry(_path: &Path) -> Result<(), AppError> {
    Ok(())
}

fn identity_hash(identity: &AgentInstallIdentity) -> Result<String, AppError> {
    crate::library::validate_reference(
        &identity.reference.source_id,
        &identity.reference.relative_path,
    )?;
    match (identity.scope, identity.project_path.as_deref()) {
        (Scope::User, None) | (Scope::Project, Some(_)) => {}
        _ => {
            return Err(invalid(
                "Agent install identity scope and project path disagree",
            ))
        }
    }
    let bytes = serde_json::to_vec(identity).map_err(|error| AppError::Internal {
        message: format!("serialize Agent install identity: {error}"),
    })?;
    Ok(crate::render::sha256_hex(&bytes))
}

#[cfg(test)]
pub(super) fn history_directory(app_data_dir: &Path, identity: &AgentInstallIdentity) -> PathBuf {
    let hash = identity_hash(identity).unwrap_or_else(|_| "invalid".into());
    std::fs::canonicalize(app_data_dir)
        .unwrap_or_else(|_| app_data_dir.to_path_buf())
        .join("agents/history")
        .join(hash)
}

async fn load_index(path: &Path) -> Result<Vec<AgentVersionSnapshot>, AppError> {
    match read_capped(path, MAX_SNAPSHOT_BYTES).await {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| AppError::Io {
            message: format!("parse Agent version history {}: {error}", path.display()),
        }),
        Err(AppError::Io { .. }) if !path.exists() => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

fn regular_file(path: &Path) -> Result<(), AppError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| AppError::Io {
        message: format!("inspect Agent snapshot input {}: {error}", path.display()),
    })?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || crate::skills::metadata_is_reparse_point(&metadata)
    {
        return Err(invalid(format!(
            "Agent snapshot input must be a regular file: {}",
            path.display()
        )));
    }
    Ok(())
}

pub(super) async fn create_snapshot(
    app_data_dir: &Path,
    identity: &AgentInstallIdentity,
    source_paths: &[PathBuf],
    source_hash: &str,
    rendered_hash: &str,
    created_at: &str,
) -> Result<AgentVersionSnapshot, AppError> {
    create_snapshot_protected(
        app_data_dir,
        identity,
        source_paths,
        source_hash,
        rendered_hash,
        created_at,
        None,
    )
    .await
}

pub(super) async fn create_snapshot_protected(
    app_data_dir: &Path,
    identity: &AgentInstallIdentity,
    source_paths: &[PathBuf],
    source_hash: &str,
    rendered_hash: &str,
    created_at: &str,
    protected_snapshot_id: Option<&str>,
) -> Result<AgentVersionSnapshot, AppError> {
    if source_paths.is_empty() {
        return Err(invalid("Agent version snapshot requires at least one file"));
    }
    let mut contents = Vec::with_capacity(source_paths.len());
    for source in source_paths {
        regular_file(source)?;
        contents.push(read_capped(source, MAX_SNAPSHOT_BYTES).await?);
    }
    let mut mutation = create_snapshot_from_bytes_protected(
        app_data_dir,
        identity,
        &contents,
        source_hash,
        rendered_hash,
        created_at,
        protected_snapshot_id,
        None,
    )
    .await?;
    let snapshot = mutation.snapshot.clone();
    mutation.publish().await?;
    mutation.commit().await?;
    Ok(snapshot)
}

pub(super) async fn create_snapshot_from_bytes(
    app_data_dir: &Path,
    identity: &AgentInstallIdentity,
    contents: &[Vec<u8>],
    source_hash: &str,
    rendered_hash: &str,
    created_at: &str,
) -> Result<AgentVersionSnapshot, AppError> {
    let mut mutation = create_snapshot_from_bytes_protected(
        app_data_dir,
        identity,
        contents,
        source_hash,
        rendered_hash,
        created_at,
        None,
        None,
    )
    .await?;
    let snapshot = mutation.snapshot.clone();
    mutation.publish().await?;
    mutation.commit().await?;
    Ok(snapshot)
}

#[allow(clippy::too_many_arguments)] // ponytail: one internal primitive keeps Agent and roster history atomic.
async fn create_snapshot_from_bytes_protected(
    app_data_dir: &Path,
    identity: &AgentInstallIdentity,
    contents: &[Vec<u8>],
    source_hash: &str,
    rendered_hash: &str,
    created_at: &str,
    protected_snapshot_id: Option<&str>,
    roster_record: Option<&crate::types::AgentRosterInstallRecord>,
) -> Result<RosterHistoryMutation, AppError> {
    if contents.is_empty() {
        return Err(invalid("Agent version snapshot requires at least one file"));
    }
    let identity_hash = identity_hash(identity)?;
    let canonical_app_data = app_owned_directory(app_data_dir, &[], false)?;
    app_owned_directory(app_data_dir, &["agents", "history"], true)?;
    let directory =
        app_owned_directory(app_data_dir, &["agents", "history", &identity_hash], false)?;
    let directory_existed = directory.is_dir();
    let index_path = directory.join("index.json");
    let previous_index = match read_capped(&index_path, MAX_SNAPSHOT_BYTES).await {
        Ok(bytes) => Some(bytes),
        Err(AppError::Io { .. }) if !index_path.exists() => None,
        Err(error) => return Err(error),
    };
    let staging_id = Uuid::new_v4().to_string();
    let staging_directory = app_owned_directory(
        app_data_dir,
        &["state", "roster-history-staging", &staging_id],
        true,
    )?;
    sync_parent_directory_entry(&staging_directory)?;
    let staged_snapshot_directory = app_owned_directory(
        app_data_dir,
        &["state", "roster-history-staging", &staging_id, "snapshot"],
        true,
    )?;
    let staged_index_path = staging_directory.join("index.json");
    let content_directory = app_owned_directory(
        app_data_dir,
        &[
            "state",
            "roster-history-staging",
            &staging_id,
            "snapshot",
            "content",
        ],
        true,
    )?;
    let id = format!("{}-{}", created_at.replace([':', '/'], "-"), Uuid::new_v4());
    let snapshot_directory = directory.join(&id);

    let result = async {
        let mut files = Vec::with_capacity(contents.len());
        for (index, bytes) in contents.iter().enumerate() {
            let sha256 = crate::render::sha256_hex(bytes);
            let name = format!("{index}.bin");
            let destination = content_directory.join(&name);
            atomic_write(&destination, bytes).await?;
            let copied = read_capped(&destination, MAX_SNAPSHOT_BYTES).await?;
            if crate::render::sha256_hex(&copied) != sha256 {
                return Err(AppError::Internal {
                    message: format!("verify Agent snapshot {} failed", destination.display()),
                });
            }
            files.push(SnapshotFile { name, sha256 });
        }
        let manifest = SnapshotManifest {
            identity_hash: identity_hash.clone(),
            files,
            roster_record_hash: roster_record.map(roster_record_hash).transpose()?,
        };
        let manifest_bytes =
            serde_json::to_vec_pretty(&manifest).map_err(|error| AppError::Internal {
                message: format!("serialize Agent snapshot manifest: {error}"),
            })?;
        atomic_write(
            &staged_snapshot_directory.join("manifest.json"),
            &manifest_bytes,
        )
        .await?;

        let snapshot = AgentVersionSnapshot {
            id,
            created_at: created_at.into(),
            source_hash: source_hash.into(),
            rendered_hash: rendered_hash.into(),
            artifact_hashes: contents
                .iter()
                .map(|bytes| crate::render::sha256_hex(bytes))
                .collect(),
            content_path: snapshot_directory.to_string_lossy().into_owned(),
            roster_record: roster_record.cloned(),
        };
        let mut snapshots: Vec<AgentVersionSnapshot> = match &previous_index {
            Some(bytes) => serde_json::from_slice(bytes).map_err(|error| AppError::Io {
                message: format!(
                    "parse Agent version history {}: {error}",
                    index_path.display()
                ),
            })?,
            None => Vec::new(),
        };
        if protected_snapshot_id
            .is_some_and(|protected| !snapshots.iter().any(|snapshot| snapshot.id == protected))
        {
            return Err(invalid(
                "Protected Agent version snapshot failed exact revalidation",
            ));
        }
        snapshots.push(snapshot.clone());
        snapshots.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        let excess = snapshots
            .len()
            .saturating_sub(super::MAX_AGENT_HISTORY_ENTRIES);
        let retired = snapshots
            .iter()
            .rev()
            .filter(|candidate| candidate.id != snapshot.id)
            .filter(|candidate| Some(candidate.id.as_str()) != protected_snapshot_id)
            .take(excess)
            .cloned()
            .collect::<Vec<_>>();
        if retired.len() != excess {
            return Err(invalid(
                "Agent version history cannot be pruned without removing a protected snapshot",
            ));
        }
        let retired_ids = retired
            .iter()
            .map(|snapshot| snapshot.id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        snapshots.retain(|snapshot| !retired_ids.contains(snapshot.id.as_str()));
        let next_index =
            serde_json::to_vec_pretty(&snapshots).map_err(|error| AppError::Internal {
                message: format!("serialize Agent version history: {error}"),
            })?;
        atomic_write(&staged_index_path, &next_index).await?;
        let retired = retired
            .into_iter()
            .map(|snapshot| PathBuf::from(snapshot.content_path))
            .collect();
        Ok(RosterHistoryMutation {
            snapshot,
            app_data_dir: canonical_app_data,
            identity: identity.clone(),
            identity_hash,
            directory,
            snapshot_directory: snapshot_directory.clone(),
            staging_directory: staging_directory.clone(),
            staged_snapshot_directory: staged_snapshot_directory.clone(),
            index_path,
            previous_index,
            next_index,
            directory_existed,
            retired,
            snapshot_published: false,
            published: false,
        })
    }
    .await;
    if result.is_err() {
        let _ = cleanup_roster_staging(app_data_dir, &staging_directory.to_string_lossy()).await;
    }
    result
}

#[cfg(test)]
pub(super) async fn create_roster_snapshot(
    app_data_dir: &Path,
    identity: &AgentInstallIdentity,
    source_paths: &[PathBuf],
    record: &crate::types::AgentRosterInstallRecord,
    created_at: &str,
    protected_snapshot_id: Option<&str>,
) -> Result<AgentVersionSnapshot, AppError> {
    let mut mutation = begin_roster_snapshot(
        app_data_dir,
        identity,
        source_paths,
        record,
        created_at,
        protected_snapshot_id,
    )
    .await?;
    let snapshot = mutation.snapshot.clone();
    mutation.publish().await?;
    mutation.commit().await?;
    Ok(snapshot)
}

pub(super) async fn begin_roster_snapshot(
    app_data_dir: &Path,
    identity: &AgentInstallIdentity,
    source_paths: &[PathBuf],
    record: &crate::types::AgentRosterInstallRecord,
    created_at: &str,
    protected_snapshot_id: Option<&str>,
) -> Result<RosterHistoryMutation, AppError> {
    if source_paths.is_empty() {
        return Err(invalid("Agent roster snapshot requires its aggregate file"));
    }
    let mut contents = Vec::with_capacity(source_paths.len());
    for source in source_paths {
        regular_file(source)?;
        contents.push(read_capped(source, MAX_SNAPSHOT_BYTES).await?);
    }
    if contents.len() != 1 || crate::render::sha256_hex(&contents[0]) != record.rendered_hash {
        return Err(invalid(
            "Agent roster snapshot content does not match its roster metadata",
        ));
    }
    let source_hash =
        crate::render::sha256_hex(&serde_json::to_vec(&record.members).map_err(|error| {
            AppError::Internal {
                message: format!("serialize Agent roster membership: {error}"),
            }
        })?);
    create_snapshot_from_bytes_protected(
        app_data_dir,
        identity,
        &contents,
        &source_hash,
        &record.rendered_hash,
        created_at,
        protected_snapshot_id,
        Some(record),
    )
    .await
}

pub(super) async fn list_snapshots(
    app_data_dir: &Path,
    identity: &AgentInstallIdentity,
) -> Result<Vec<AgentVersionSnapshot>, AppError> {
    let identity_hash = identity_hash(identity)?;
    let directory =
        app_owned_directory(app_data_dir, &["agents", "history", &identity_hash], false)?;
    load_index(&directory.join("index.json")).await
}

pub(super) async fn snapshot_contents(
    app_data_dir: &Path,
    identity: &AgentInstallIdentity,
    snapshot_id: &str,
    destinations: &[PathBuf],
) -> Result<(AgentVersionSnapshot, Vec<Vec<u8>>), AppError> {
    let identity_hash = identity_hash(identity)?;
    let directory =
        app_owned_directory(app_data_dir, &["agents", "history", &identity_hash], false)?;
    let snapshot = list_snapshots(app_data_dir, identity)
        .await?
        .into_iter()
        .find(|snapshot| snapshot.id == snapshot_id)
        .ok_or_else(|| invalid("Agent version snapshot does not belong to this install"))?;
    let snapshot_directory =
        std::fs::canonicalize(&snapshot.content_path).map_err(|error| AppError::Io {
            message: format!("resolve Agent version snapshot: {error}"),
        })?;
    let canonical_directory = std::fs::canonicalize(&directory).map_err(|error| AppError::Io {
        message: format!("resolve Agent history directory: {error}"),
    })?;
    if snapshot_directory.parent() != Some(canonical_directory.as_path()) {
        return Err(invalid(
            "Agent version snapshot escaped its install history",
        ));
    }
    let manifest_bytes = read_capped(
        &snapshot_directory.join("manifest.json"),
        MAX_SNAPSHOT_BYTES,
    )
    .await?;
    let manifest: SnapshotManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|error| AppError::Io {
            message: format!("parse Agent snapshot manifest: {error}"),
        })?;
    if manifest.identity_hash != identity_hash || manifest.files.len() != destinations.len() {
        return Err(invalid(
            "Agent version snapshot identity or file count changed",
        ));
    }
    match (&snapshot.roster_record, &manifest.roster_record_hash) {
        (Some(record), Some(expected)) if roster_record_hash(record)? == *expected => {}
        (None, None) => {}
        (Some(_), None) => {
            return Err(invalid(
                "Legacy Agent roster snapshot metadata is not cryptographically bound",
            ));
        }
        _ => {
            return Err(invalid(
                "Agent roster snapshot metadata failed verification",
            ));
        }
    }

    let mut contents = Vec::with_capacity(manifest.files.len());
    for (index, file) in manifest.files.iter().enumerate() {
        if file.name != format!("{index}.bin") {
            return Err(invalid("Agent version snapshot filename changed"));
        }
        let path = snapshot_directory.join("content").join(&file.name);
        regular_file(&path)?;
        let bytes = read_capped(&path, MAX_SNAPSHOT_BYTES).await?;
        if crate::render::sha256_hex(&bytes) != file.sha256 {
            return Err(invalid(
                "Agent version snapshot content failed verification",
            ));
        }
        contents.push(bytes);
    }

    Ok((snapshot, contents))
}

pub(super) async fn restore_snapshot(
    app_data_dir: &Path,
    identity: &AgentInstallIdentity,
    snapshot_id: &str,
    destinations: &[PathBuf],
) -> Result<AgentVersionSnapshot, AppError> {
    let (snapshot, contents) =
        snapshot_contents(app_data_dir, identity, snapshot_id, destinations).await?;

    let mut prior = Vec::with_capacity(destinations.len());
    for destination in destinations {
        match tokio::fs::read(destination).await {
            Ok(bytes) => prior.push(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => prior.push(None),
            Err(error) => {
                return Err(AppError::Io {
                    message: format!(
                        "read rollback destination {}: {error}",
                        destination.display()
                    ),
                })
            }
        }
    }
    for (index, (destination, bytes)) in destinations.iter().zip(&contents).enumerate() {
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| AppError::Io {
                    message: format!("create rollback destination {}: {error}", parent.display()),
                })?;
        }
        if let Err(error) = atomic_write(destination, bytes).await {
            for restore_index in (0..index).rev() {
                match &prior[restore_index] {
                    Some(previous) => {
                        let _ = atomic_write(&destinations[restore_index], previous).await;
                    }
                    None => {
                        let _ = tokio::fs::remove_file(&destinations[restore_index]).await;
                    }
                }
            }
            return Err(error);
        }
    }
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::types::{
        AgentInstallIdentity, AgentReference, AgentRosterInstallRecord, AgentRosterMember, Scope,
    };

    #[tokio::test]
    async fn cleanup_sync_failure_restores_history_intent_and_preserves_payload() {
        let app_data = tempfile::tempdir().unwrap();
        let app_data_path = std::fs::canonicalize(app_data.path()).unwrap();
        let staging = app_data_path
            .join("state/roster-history-staging")
            .join(Uuid::new_v4().to_string());
        let payload = staging.join("snapshot/content/0.bin");
        let intent = staging.join(UNJOURNALED_INTENT_FILE);
        std::fs::create_dir_all(payload.parent().unwrap()).unwrap();
        std::fs::write(&payload, b"snapshot").unwrap();
        std::fs::write(&intent, b"intent").unwrap();
        let mut sync_attempts = 0;

        let result =
            cleanup_roster_staging_with_sync(&app_data_path, &staging.to_string_lossy(), |_| {
                sync_attempts += 1;
                if sync_attempts == 1 {
                    Err(AppError::Io {
                        message: "injected marker parent sync failure".into(),
                    })
                } else {
                    Ok(())
                }
            })
            .await;

        assert!(result.is_err());
        assert_eq!(std::fs::read(&intent).unwrap(), b"intent");
        assert_eq!(std::fs::read(&payload).unwrap(), b"snapshot");
        assert_eq!(sync_attempts, 2);
    }

    #[tokio::test]
    async fn history_index_publish_syncs_move_parents_before_index_parent() {
        let root = tempfile::tempdir().unwrap();
        let staging = root.path().join("staging");
        let history = root.path().join("history");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::create_dir_all(&history).unwrap();
        let staged_snapshot = staging.join("snapshot");
        let live_snapshot = history.join("snapshot-id");
        let index = history.join("index.json");
        let mut events = Vec::new();

        publish_history_index_after_snapshot_move_with(
            &staged_snapshot,
            &live_snapshot,
            &index,
            b"[]",
            |path| {
                events.push((path.to_path_buf(), index.exists()));
                Ok(())
            },
        )
        .await
        .unwrap();

        assert_eq!(
            events,
            vec![
                (staged_snapshot, false),
                (live_snapshot, false),
                (index.clone(), true),
            ]
        );
        assert_eq!(std::fs::read(index).unwrap(), b"[]");
    }

    #[tokio::test]
    async fn retention_retry_syncs_history_parent_when_snapshot_is_already_absent() {
        let app_data = tempfile::tempdir().unwrap();
        let app_data_path = std::fs::canonicalize(app_data.path()).unwrap();
        let identity = identity("builtin:agency-agents");
        let identity_hash = identity_hash(&identity).unwrap();
        let history =
            app_owned_directory(&app_data_path, &["agents", "history", &identity_hash], true)
                .unwrap();
        std::fs::write(history.join("index.json"), b"[]").unwrap();
        let retired = format!("retired-{}", Uuid::new_v4());
        let retired_path = history.join(&retired);
        let mut synced = Vec::new();

        commit_roster_retention_with_sync(&app_data_path, &identity, &[retired], |path| {
            synced.push(path.to_path_buf());
            Ok(())
        })
        .await
        .unwrap();

        assert_eq!(synced, vec![retired_path]);
    }

    #[tokio::test]
    async fn direct_publish_sync_failures_restore_exact_history_and_staging() {
        for fail_at in 1..=3 {
            let app_data = tempfile::tempdir().unwrap();
            let app_data_path = std::fs::canonicalize(app_data.path()).unwrap();
            let identity = identity("builtin:agency-agents");
            let mut mutation = create_snapshot_from_bytes_protected(
                &app_data_path,
                &identity,
                &[b"agent".to_vec()],
                &crate::render::sha256_hex(b"source"),
                &crate::render::sha256_hex(b"agent"),
                "2026-08-18T00:00:00Z",
                None,
                None,
            )
            .await
            .unwrap();
            let staging = mutation.staging_directory.clone();
            let staged_snapshot = mutation.staged_snapshot_directory.clone();
            let live_snapshot = mutation.snapshot_directory.clone();
            let index = mutation.index_path.clone();
            let mut sync_attempts = 0;

            let result = mutation
                .publish_with_sync(|_| {
                    sync_attempts += 1;
                    if sync_attempts == fail_at {
                        Err(AppError::Io {
                            message: format!("injected publish sync failure {fail_at}"),
                        })
                    } else {
                        Ok(())
                    }
                })
                .await;

            assert!(result.is_err(), "sync failure {fail_at} must propagate");
            assert!(!index.exists(), "sync failure {fail_at} changed the index");
            assert!(
                !live_snapshot.exists(),
                "sync failure {fail_at} left an unindexed live snapshot"
            );
            assert!(
                staged_snapshot.is_dir(),
                "sync failure {fail_at} lost the recoverable staged snapshot"
            );
            mutation.rollback().await.unwrap();
            assert!(!staging.exists());
        }
    }

    fn identity(source: &str) -> AgentInstallIdentity {
        AgentInstallIdentity {
            reference: AgentReference {
                source_id: source.into(),
                relative_path: "engineering/reviewer.md".into(),
            },
            tool: "copilot".into(),
            scope: Scope::User,
            project_path: None,
        }
    }

    fn roster_identity(project: &Path) -> (AgentInstallIdentity, AgentRosterInstallRecord) {
        let project_path = project.to_string_lossy().into_owned();
        let record = AgentRosterInstallRecord {
            tool: "aider".into(),
            scope: Scope::Project,
            project_path: project_path.clone(),
            dest: project
                .join("CONVENTIONS.md")
                .to_string_lossy()
                .into_owned(),
            members: ["agent.md", "reviewer.md"]
                .into_iter()
                .map(|relative_path| AgentRosterMember {
                    reference: AgentReference {
                        source_id: "builtin:agency-agents".into(),
                        relative_path: relative_path.into(),
                    },
                    name: relative_path.into(),
                    source_hash: "a".repeat(64),
                })
                .collect(),
            rendered_hash: crate::render::sha256_hex(b"roster bytes"),
            disabled_path: None,
            installed_at: "2026-08-17T00:00:00Z".into(),
        };
        let identity = AgentInstallIdentity {
            reference: AgentReference {
                source_id: "roster:aider".into(),
                relative_path: format!(
                    "projects/{}.md",
                    crate::render::sha256_hex(project_path.as_bytes())
                ),
            },
            tool: "aider".into(),
            scope: Scope::Project,
            project_path: Some(project_path),
        };
        (identity, record)
    }

    #[tokio::test]
    async fn roster_snapshot_rejects_index_metadata_changed_after_manifest_commit() {
        let app_data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let project = std::fs::canonicalize(project.path()).unwrap();
        let destination = project.join("CONVENTIONS.md");
        std::fs::write(&destination, b"roster bytes").unwrap();
        let (identity, record) = roster_identity(&project);
        let snapshot = create_roster_snapshot(
            app_data.path(),
            &identity,
            std::slice::from_ref(&destination),
            &record,
            "2026-08-17T00:00:00Z",
            None,
        )
        .await
        .unwrap();
        let index = history_directory(app_data.path(), &identity).join("index.json");
        let mut entries = list_snapshots(app_data.path(), &identity).await.unwrap();
        entries[0].roster_record.as_mut().unwrap().members[0].name = "Tampered".into();
        std::fs::write(&index, serde_json::to_vec_pretty(&entries).unwrap()).unwrap();

        assert!(snapshot_contents(
            app_data.path(),
            &identity,
            &snapshot.id,
            std::slice::from_ref(&destination),
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn roster_publication_rejects_tampered_staging_without_changing_live_history() {
        let app_data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let project = std::fs::canonicalize(project.path()).unwrap();
        let destination = project.join("CONVENTIONS.md");
        std::fs::write(&destination, b"roster bytes").unwrap();
        let (identity, record) = roster_identity(&project);
        create_roster_snapshot(
            app_data.path(),
            &identity,
            std::slice::from_ref(&destination),
            &record,
            "2026-08-17T00:00:00Z",
            None,
        )
        .await
        .unwrap();
        let directory = history_directory(app_data.path(), &identity);
        let index_before = std::fs::read(directory.join("index.json")).unwrap();
        let snapshot_directories_before = std::fs::read_dir(&directory)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .count();

        let mut mutation = begin_roster_snapshot(
            app_data.path(),
            &identity,
            std::slice::from_ref(&destination),
            &record,
            "2026-08-17T00:01:00Z",
            None,
        )
        .await
        .unwrap();
        let staging = mutation.staging_directory.clone();
        std::fs::write(
            mutation.staged_snapshot_directory.join("content/0.bin"),
            b"tampered staging bytes",
        )
        .unwrap();

        assert!(mutation.publish().await.is_err());
        mutation.rollback().await.unwrap();

        assert!(!staging.exists());
        assert_eq!(
            std::fs::read(directory.join("index.json")).unwrap(),
            index_before
        );
        assert_eq!(
            std::fs::read_dir(&directory)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                .count(),
            snapshot_directories_before
        );
    }

    #[tokio::test]
    async fn roster_snapshot_rejects_manifest_filename_escape() {
        let app_data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let project = std::fs::canonicalize(project.path()).unwrap();
        let destination = project.join("CONVENTIONS.md");
        std::fs::write(&destination, b"roster bytes").unwrap();
        let (identity, record) = roster_identity(&project);
        let snapshot = create_roster_snapshot(
            app_data.path(),
            &identity,
            std::slice::from_ref(&destination),
            &record,
            "2026-08-17T00:00:00Z",
            None,
        )
        .await
        .unwrap();
        let manifest_path = PathBuf::from(&snapshot.content_path).join("manifest.json");
        let mut manifest: SnapshotManifest =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        manifest.files[0].name = destination.to_string_lossy().into_owned();
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();

        assert!(snapshot_contents(
            app_data.path(),
            &identity,
            &snapshot.id,
            std::slice::from_ref(&destination),
        )
        .await
        .is_err());
        assert_eq!(std::fs::read(destination).unwrap(), b"roster bytes");
    }

    #[tokio::test]
    async fn roster_publication_rejects_snapshot_id_escape_before_path_use() {
        let app_data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let project = std::fs::canonicalize(project.path()).unwrap();
        let (identity, record) = roster_identity(&project);
        assert!(recover_roster_publication(
            app_data.path(),
            &identity,
            "../escape",
            &[],
            "/invalid",
            None,
            &"0".repeat(64),
            &record,
        )
        .await
        .is_err());
        assert!(!app_data.path().join("agents/history/escape").exists());
    }

    #[tokio::test]
    async fn roster_history_rollback_at_retention_cap_restores_exact_bytes() {
        fn tree(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
            fn collect(root: &Path, path: &Path, files: &mut Vec<(PathBuf, Vec<u8>)>) {
                for entry in std::fs::read_dir(path).unwrap() {
                    let entry = entry.unwrap();
                    let path = entry.path();
                    if entry.metadata().unwrap().is_dir() {
                        collect(root, &path, files);
                    } else {
                        files.push((
                            path.strip_prefix(root).unwrap().to_path_buf(),
                            std::fs::read(path).unwrap(),
                        ));
                    }
                }
            }
            let mut files = Vec::new();
            collect(root, root, &mut files);
            files.sort_by(|left, right| left.0.cmp(&right.0));
            files
        }

        let app_data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let project = std::fs::canonicalize(project.path()).unwrap();
        let destination = project.join("CONVENTIONS.md");
        std::fs::write(&destination, b"roster bytes").unwrap();
        let (identity, record) = roster_identity(&project);
        for index in 0..super::super::MAX_AGENT_HISTORY_ENTRIES {
            create_roster_snapshot(
                app_data.path(),
                &identity,
                std::slice::from_ref(&destination),
                &record,
                &format!("2026-08-17T00:00:{index:02}Z"),
                None,
            )
            .await
            .unwrap();
        }
        let directory = history_directory(app_data.path(), &identity);
        let before = tree(&directory);

        begin_roster_snapshot(
            app_data.path(),
            &identity,
            std::slice::from_ref(&destination),
            &record,
            "2026-08-17T01:00:00Z",
            None,
        )
        .await
        .unwrap()
        .rollback()
        .await
        .unwrap();

        assert_eq!(tree(&directory), before);
    }

    async fn assert_unjournaled_crash_recovery_is_exact(
        publish_index_before_crash: bool,
        legacy_intent: bool,
    ) {
        let app_data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let project = std::fs::canonicalize(project.path()).unwrap();
        let destination = project.join("CONVENTIONS.md");
        std::fs::write(&destination, b"roster bytes").unwrap();
        let (identity, record) = roster_identity(&project);
        for index in 0..super::super::MAX_AGENT_HISTORY_ENTRIES {
            create_roster_snapshot(
                app_data.path(),
                &identity,
                std::slice::from_ref(&destination),
                &record,
                &format!("2026-08-17T00:00:{index:02}Z"),
                None,
            )
            .await
            .unwrap();
        }
        let mutation = begin_roster_snapshot(
            app_data.path(),
            &identity,
            std::slice::from_ref(&destination),
            &record,
            "2026-08-17T01:00:00Z",
            None,
        )
        .await
        .unwrap();
        assert_eq!(mutation.retired_snapshot_ids().len(), 1);
        mutation
            .prepare_unjournaled_operation(&record, Some(&record))
            .await
            .unwrap();
        let staging = mutation.staging_directory.clone();
        if legacy_intent {
            let path = staging.join(UNJOURNALED_INTENT_FILE);
            let mut document: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            for field in ["phase", "previousRecord", "nextRecord"] {
                document.as_object_mut().unwrap().remove(field);
            }
            atomic_write(&path, &serde_json::to_vec_pretty(&document).unwrap())
                .await
                .unwrap();
        } else {
            mutation
                .mark_unjournaled_filesystem_applied()
                .await
                .unwrap();
        }
        std::fs::rename(
            &mutation.staged_snapshot_directory,
            &mutation.snapshot_directory,
        )
        .unwrap();
        if publish_index_before_crash {
            atomic_write(&mutation.index_path, &mutation.next_index)
                .await
                .unwrap();
        }
        drop(mutation);

        let mut state = crate::state::AppState::build().unwrap();
        state.app_data_dir = app_data.path().to_path_buf();
        super::super::save_registered_projects(app_data.path(), std::slice::from_ref(&project))
            .await
            .unwrap();
        super::super::save_rosters_for_state(&state, std::slice::from_ref(&record))
            .await
            .unwrap();
        super::super::recover_agent_operations(&state)
            .await
            .unwrap();
        super::super::recover_agent_operations(&state)
            .await
            .unwrap();

        assert!(!staging.exists());
        let snapshots = list_snapshots(app_data.path(), &identity).await.unwrap();
        assert_eq!(snapshots.len(), super::super::MAX_AGENT_HISTORY_ENTRIES);
        let indexed = snapshots
            .iter()
            .map(|snapshot| PathBuf::from(&snapshot.content_path))
            .collect::<std::collections::BTreeSet<_>>();
        let directory = history_directory(app_data.path(), &identity);
        let physical = std::fs::read_dir(&directory)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .map(|entry| entry.path())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(physical, indexed);
    }

    #[tokio::test]
    async fn unjournaled_crash_after_snapshot_rename_recovers_index_and_retention_twice() {
        assert_unjournaled_crash_recovery_is_exact(false, false).await;
    }

    #[tokio::test]
    async fn unjournaled_crash_after_index_publish_recovers_retention_twice() {
        assert_unjournaled_crash_recovery_is_exact(true, false).await;
    }

    #[tokio::test]
    async fn legacy_unjournaled_intent_recovers_after_upgrade() {
        assert_unjournaled_crash_recovery_is_exact(false, true).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn roster_history_rejects_linked_owned_roots_without_external_deletion() {
        use std::os::unix::fs::symlink;

        let app_data = tempfile::tempdir().unwrap();
        let external_staging = tempfile::tempdir().unwrap();
        let external_entry = external_staging.path().join(Uuid::new_v4().to_string());
        std::fs::create_dir(&external_entry).unwrap();
        std::fs::create_dir(app_data.path().join("state")).unwrap();
        symlink(
            external_staging.path(),
            app_data.path().join("state/roster-history-staging"),
        )
        .unwrap();

        assert!(sweep_roster_staging(app_data.path(), &[]).await.is_err());
        assert!(external_entry.exists());

        let external_history = tempfile::tempdir().unwrap();
        std::fs::create_dir(app_data.path().join("agents")).unwrap();
        symlink(
            external_history.path(),
            app_data.path().join("agents/history"),
        )
        .unwrap();
        let identity = identity("builtin:agency-agents");
        let retired_id = format!("2026-08-17T00-00-00Z-{}", Uuid::new_v4());
        let external_retired = external_history
            .path()
            .join(identity_hash(&identity).unwrap())
            .join(&retired_id);
        std::fs::create_dir_all(&external_retired).unwrap();

        assert!(
            commit_roster_retention(app_data.path(), &identity, &[retired_id])
                .await
                .is_err()
        );
        assert!(external_retired.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn roster_publication_rejects_linked_staged_content() {
        use std::os::unix::fs::symlink;

        let app_data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let project = std::fs::canonicalize(project.path()).unwrap();
        let destination = project.join("CONVENTIONS.md");
        std::fs::write(&destination, b"roster bytes").unwrap();
        let (identity, record) = roster_identity(&project);
        let mut mutation = begin_roster_snapshot(
            app_data.path(),
            &identity,
            std::slice::from_ref(&destination),
            &record,
            "2026-08-17T01:00:00Z",
            None,
        )
        .await
        .unwrap();
        let external_content = tempfile::tempdir().unwrap();
        std::fs::write(external_content.path().join("0.bin"), b"roster bytes").unwrap();
        let content = mutation.staged_snapshot_directory.join("content");
        std::fs::remove_dir_all(&content).unwrap();
        symlink(external_content.path(), &content).unwrap();

        assert!(mutation.publish().await.is_err());
        mutation.rollback().await.unwrap();
        assert_eq!(
            std::fs::read(external_content.path().join("0.bin")).unwrap(),
            b"roster bytes"
        );
    }

    #[tokio::test]
    async fn snapshot_history_is_identity_scoped_and_hash_verified() {
        let app_data = tempfile::tempdir().unwrap();
        let destinations = tempfile::tempdir().unwrap();
        let first = destinations.path().join("github/reviewer.md");
        let second = destinations.path().join("copilot/reviewer.md");
        for (path, bytes) in [
            (&first, b"first".as_slice()),
            (&second, b"second".as_slice()),
        ] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }

        let snapshot = create_snapshot(
            app_data.path(),
            &identity("builtin:agency-agents"),
            &[first.clone(), second.clone()],
            "source-1",
            "render-1",
            "2026-08-04T01:00:00Z",
        )
        .await
        .unwrap();
        assert_eq!(
            list_snapshots(app_data.path(), &identity("builtin:agency-agents"))
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(
            list_snapshots(app_data.path(), &identity("local:other"))
                .await
                .unwrap()
                .is_empty(),
            "history must not cross Agent source identities"
        );

        std::fs::write(&first, b"changed-active").unwrap();
        std::fs::write(&second, b"changed-active").unwrap();
        let content = PathBuf::from(&snapshot.content_path).join("content/0.bin");
        std::fs::write(content, b"tampered-snapshot").unwrap();
        assert!(restore_snapshot(
            app_data.path(),
            &identity("builtin:agency-agents"),
            &snapshot.id,
            &[first.clone(), second.clone()],
        )
        .await
        .is_err());
        assert_eq!(std::fs::read(&first).unwrap(), b"changed-active");
        assert_eq!(std::fs::read(&second).unwrap(), b"changed-active");
    }

    #[tokio::test]
    async fn late_multi_file_restore_failure_restores_every_prior_destination() {
        let app_data = tempfile::tempdir().unwrap();
        let destinations = tempfile::tempdir().unwrap();
        let first = destinations.path().join("agent/agent.yaml");
        let second = destinations.path().join("agent/system.md");
        std::fs::create_dir_all(first.parent().unwrap()).unwrap();
        std::fs::write(&first, b"snapshot-one").unwrap();
        std::fs::write(&second, b"snapshot-two").unwrap();
        let snapshot = create_snapshot(
            app_data.path(),
            &identity("builtin:agency-agents"),
            &[first.clone(), second.clone()],
            "source-1",
            "render-1",
            "2026-08-17T01:00:00Z",
        )
        .await
        .unwrap();

        std::fs::write(&first, b"prior-one").unwrap();
        std::fs::write(&second, b"prior-two").unwrap();
        std::fs::create_dir(second.with_extension("md.tmp")).unwrap();

        assert!(restore_snapshot(
            app_data.path(),
            &identity("builtin:agency-agents"),
            &snapshot.id,
            &[first.clone(), second.clone()],
        )
        .await
        .is_err());
        assert_eq!(std::fs::read(&first).unwrap(), b"prior-one");
        assert_eq!(std::fs::read(&second).unwrap(), b"prior-two");
    }

    #[tokio::test]
    async fn snapshot_index_failure_preserves_existing_history() {
        let app_data = tempfile::tempdir().unwrap();
        let destination = app_data.path().join("active.md");
        std::fs::write(&destination, b"version-1").unwrap();
        let identity = identity("builtin:agency-agents");
        create_snapshot(
            app_data.path(),
            &identity,
            std::slice::from_ref(&destination),
            "source-1",
            "render-1",
            "2026-08-04T01:00:00Z",
        )
        .await
        .unwrap();
        let index = history_directory(app_data.path(), &identity).join("index.json");
        std::fs::create_dir(index.with_extension("json.tmp")).unwrap();
        std::fs::write(&destination, b"version-2").unwrap();

        assert!(create_snapshot(
            app_data.path(),
            &identity,
            std::slice::from_ref(&destination),
            "source-2",
            "render-2",
            "2026-08-04T02:00:00Z",
        )
        .await
        .is_err());
        let history = list_snapshots(app_data.path(), &identity).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].source_hash, "source-1");
    }

    #[tokio::test]
    async fn snapshot_history_retains_only_the_bounded_latest_versions() {
        let app_data = tempfile::tempdir().unwrap();
        let destination = app_data.path().join("active.md");
        let identity = identity("builtin:agency-agents");
        for index in 0..=super::super::MAX_AGENT_HISTORY_ENTRIES {
            std::fs::write(&destination, format!("version-{index}")).unwrap();
            create_snapshot(
                app_data.path(),
                &identity,
                std::slice::from_ref(&destination),
                &format!("source-{index}"),
                &format!("render-{index}"),
                &format!("2026-08-04T{index:02}:00:00Z"),
            )
            .await
            .unwrap();
        }
        let history = list_snapshots(app_data.path(), &identity).await.unwrap();
        assert_eq!(history.len(), super::super::MAX_AGENT_HISTORY_ENTRIES);
        assert_eq!(history[0].source_hash, "source-10");
        assert_eq!(history.last().unwrap().source_hash, "source-1");
    }

    #[tokio::test]
    async fn corrupt_history_index_fails_without_rewriting_it() {
        let app_data = tempfile::tempdir().unwrap();
        let identity = identity("builtin:agency-agents");
        let path = history_directory(app_data.path(), &identity).join("index.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{not-json").unwrap();

        assert!(list_snapshots(app_data.path(), &identity).await.is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"{not-json");
    }
}
