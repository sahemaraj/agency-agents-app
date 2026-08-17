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

async fn save_index(path: &Path, snapshots: &[AgentVersionSnapshot]) -> Result<(), AppError> {
    let bytes = serde_json::to_vec_pretty(snapshots).map_err(|error| AppError::Internal {
        message: format!("serialize Agent version history: {error}"),
    })?;
    atomic_write(path, &bytes).await
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
    create_snapshot_from_bytes_protected(
        app_data_dir,
        identity,
        &contents,
        source_hash,
        rendered_hash,
        created_at,
        protected_snapshot_id,
        None,
    )
    .await
}

pub(super) async fn create_snapshot_from_bytes(
    app_data_dir: &Path,
    identity: &AgentInstallIdentity,
    contents: &[Vec<u8>],
    source_hash: &str,
    rendered_hash: &str,
    created_at: &str,
) -> Result<AgentVersionSnapshot, AppError> {
    create_snapshot_from_bytes_protected(
        app_data_dir,
        identity,
        contents,
        source_hash,
        rendered_hash,
        created_at,
        None,
        None,
    )
    .await
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
) -> Result<AgentVersionSnapshot, AppError> {
    if contents.is_empty() {
        return Err(invalid("Agent version snapshot requires at least one file"));
    }
    let identity_hash = identity_hash(identity)?;
    let directory = app_data_dir.join("agents/history").join(&identity_hash);
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| AppError::Io {
            message: format!(
                "create Agent history directory {}: {error}",
                directory.display()
            ),
        })?;
    let id = format!("{}-{}", created_at.replace([':', '/'], "-"), Uuid::new_v4());
    let snapshot_directory = directory.join(&id);
    let content_directory = snapshot_directory.join("content");
    tokio::fs::create_dir_all(&content_directory)
        .await
        .map_err(|error| AppError::Io {
            message: format!("create Agent snapshot directory: {error}"),
        })?;

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
            identity_hash,
            files,
        };
        let manifest_bytes =
            serde_json::to_vec_pretty(&manifest).map_err(|error| AppError::Internal {
                message: format!("serialize Agent snapshot manifest: {error}"),
            })?;
        atomic_write(&snapshot_directory.join("manifest.json"), &manifest_bytes).await?;

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
        let index_path = directory.join("index.json");
        let mut snapshots = load_index(&index_path).await?;
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
        save_index(&index_path, &snapshots).await?;
        for retired_snapshot in retired {
            let path = PathBuf::from(retired_snapshot.content_path);
            if path.parent() == Some(directory.as_path()) {
                let _ = tokio::fs::remove_dir_all(path).await;
            }
        }
        Ok(snapshot)
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_dir_all(&snapshot_directory).await;
    }
    result
}

pub(super) async fn create_roster_snapshot(
    app_data_dir: &Path,
    identity: &AgentInstallIdentity,
    source_paths: &[PathBuf],
    record: &crate::types::AgentRosterInstallRecord,
    created_at: &str,
    protected_snapshot_id: Option<&str>,
) -> Result<AgentVersionSnapshot, AppError> {
    if source_paths.is_empty() {
        return Err(invalid("Agent roster snapshot requires its aggregate file"));
    }
    let mut contents = Vec::with_capacity(source_paths.len());
    for source in source_paths {
        regular_file(source)?;
        contents.push(read_capped(source, MAX_SNAPSHOT_BYTES).await?);
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

    let mut contents = Vec::with_capacity(manifest.files.len());
    for file in &manifest.files {
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
    use crate::types::{AgentInstallIdentity, AgentReference, Scope};

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
