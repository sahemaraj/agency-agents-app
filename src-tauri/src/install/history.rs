use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;
use crate::types::{AgentInstallIdentity, AgentVersionSnapshot, Scope};
use crate::util::fs::{atomic_write, read_capped};

const MAX_SNAPSHOT_BYTES: u64 = 4 * 1024 * 1024;

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

pub(super) struct RosterHistoryMutation {
    pub(super) snapshot: AgentVersionSnapshot,
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

    pub(super) async fn publish(&mut self) -> Result<(), AppError> {
        if self.published {
            return Ok(());
        }
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
        tokio::fs::create_dir_all(&self.directory)
            .await
            .map_err(|error| AppError::Io {
                message: format!("create Agent history directory: {error}"),
            })?;
        match tokio::fs::rename(&self.staged_snapshot_directory, &self.snapshot_directory).await {
            Ok(()) => {}
            Err(error) => {
                return Err(AppError::Io {
                    message: format!("publish staged Agent roster snapshot: {error}"),
                });
            }
        }
        self.snapshot_published = true;
        if let Err(error) = atomic_write(&self.index_path, &self.next_index).await {
            return match tokio::fs::rename(
                &self.snapshot_directory,
                &self.staged_snapshot_directory,
            )
            .await
            {
                Ok(()) => {
                    self.snapshot_published = false;
                    Err(error)
                }
                Err(rollback) => Err(AppError::Internal {
                    message: format!(
                        "publish Agent roster history index failed: {error}; restore staged snapshot failed: {rollback}"
                    ),
                }),
            };
        }
        self.published = true;
        Ok(())
    }

    pub(super) async fn commit(self) -> Result<(), AppError> {
        if !self.published || !self.snapshot_published {
            return Err(AppError::StorageCorrupt {
                message: "Agent roster history was not published before commit".into(),
            });
        }
        for path in self.retired {
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
        }
        match tokio::fs::remove_dir_all(&self.staging_directory).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(AppError::Io {
                    message: format!("remove Agent roster history staging: {error}"),
                });
            }
        }
        Ok(())
    }

    pub(super) async fn rollback(self) -> Result<(), AppError> {
        let history_was_absent = self.previous_index.is_none();
        if self.published {
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
        }
        if self.snapshot_published {
            match tokio::fs::remove_dir_all(&self.snapshot_directory).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(AppError::Io {
                        message: format!("remove failed Agent roster snapshot: {error}"),
                    });
                }
            }
        }
        let _ = tokio::fs::remove_dir_all(&self.staging_directory).await;
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
    if retired_snapshot_ids.is_empty() {
        return Ok(());
    }
    let directory = app_data_dir
        .join("agents/history")
        .join(identity_hash(identity)?);
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
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
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
    }
    Ok(())
}

fn validated_roster_staging_directory(
    app_data_dir: &Path,
    staging_path: &str,
) -> Result<PathBuf, AppError> {
    let path = PathBuf::from(staging_path);
    let expected_parent = app_data_dir.join("state/roster-history-staging");
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
    let manifest_bytes = read_capped(&directory.join("manifest.json"), MAX_SNAPSHOT_BYTES).await?;
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
        let content = directory.join("content").join(&file.name);
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
    let directory = app_data_dir.join("agents/history").join(&identity_hash);
    let index_path = directory.join("index.json");
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
        return validate_snapshot_directory(&final_snapshot, &identity_hash, &snapshot).await;
    }
    if current_hash.as_deref() != previous_index_hash {
        return Err(AppError::StorageCorrupt {
            message: "Agent roster history changed before recovery publication".into(),
        });
    }
    let staging = validated_roster_staging_directory(app_data_dir, staging_path)?;
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
    let staged_snapshot = staging.join("snapshot");
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
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| AppError::Io {
            message: format!("create Agent roster history directory: {error}"),
        })?;
    if must_move {
        tokio::fs::rename(&staged_snapshot, &final_snapshot)
            .await
            .map_err(|error| AppError::Io {
                message: format!("recover staged Agent roster snapshot: {error}"),
            })?;
    }
    atomic_write(&index_path, &staged_index).await?;
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
    tokio::fs::remove_dir_all(path)
        .await
        .map_err(|error| AppError::Io {
            message: format!("remove Agent roster history staging: {error}"),
        })
}

pub(super) async fn sweep_roster_staging(
    app_data_dir: &Path,
    retained_staging_paths: &[String],
) -> Result<(), AppError> {
    let retained = retained_staging_paths
        .iter()
        .map(|path| validated_roster_staging_directory(app_data_dir, path))
        .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
    let root = app_data_dir.join("state/roster-history-staging");
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
    app_data_dir.join("agents/history").join(hash)
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
    let directory = app_data_dir.join("agents/history").join(&identity_hash);
    let directory_existed = directory.is_dir();
    let index_path = directory.join("index.json");
    let previous_index = match read_capped(&index_path, MAX_SNAPSHOT_BYTES).await {
        Ok(bytes) => Some(bytes),
        Err(AppError::Io { .. }) if !index_path.exists() => None,
        Err(error) => return Err(error),
    };
    let staging_directory = app_data_dir
        .join("state/roster-history-staging")
        .join(Uuid::new_v4().to_string());
    let staged_snapshot_directory = staging_directory.join("snapshot");
    let staged_index_path = staging_directory.join("index.json");
    let content_directory = staged_snapshot_directory.join("content");
    tokio::fs::create_dir_all(&content_directory)
        .await
        .map_err(|error| AppError::Io {
            message: format!("create Agent snapshot staging directory: {error}"),
        })?;
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
        let _ = tokio::fs::remove_dir_all(&staging_directory).await;
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
    let directory = app_data_dir
        .join("agents/history")
        .join(identity_hash(identity)?);
    load_index(&directory.join("index.json")).await
}

pub(super) async fn snapshot_contents(
    app_data_dir: &Path,
    identity: &AgentInstallIdentity,
    snapshot_id: &str,
    destinations: &[PathBuf],
) -> Result<(AgentVersionSnapshot, Vec<Vec<u8>>), AppError> {
    let identity_hash = identity_hash(identity)?;
    let directory = app_data_dir.join("agents/history").join(&identity_hash);
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
