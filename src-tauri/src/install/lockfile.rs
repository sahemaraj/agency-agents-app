use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use super::{
    build_workspace_pack_plan, installed_source_revision, materialize_workspace_pack_sources,
    portable_agent_source, portable_skill_source, PortableSource, WorkspacePack,
    WorkspacePackAgent, WorkspacePackScope, WorkspacePackSkill,
};
use crate::error::AppError;
use crate::state::AppState;
use crate::types::{AgentReference, SkillReference};
use crate::{render, skills};

const LOCK_VERSION: u32 = 1;
pub(crate) const LOCK_FILENAME: &str = "shikigami.lock.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgencyLock {
    pub(crate) agency_lock: u32,
    pub(crate) entries: Vec<LockEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LockEntry {
    pub(crate) artifacts: Vec<LockArtifact>,
    pub(crate) kind: String,
    pub(crate) scope: String,
    pub(crate) source: PortableSource,
    pub(crate) source_hash: String,
    pub(crate) source_relative_path: String,
    pub(crate) tool: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LockArtifact {
    pub(crate) content_hash: String,
    pub(crate) path: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LockEntryStatus {
    Current,
    Missing,
    Modified,
    Outdated,
    Foreign,
    Extra,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LockCheckEntry {
    pub(crate) entry: LockEntry,
    pub(crate) status: LockEntryStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LockCheckResult {
    pub(crate) lock: AgencyLock,
    pub(crate) entries: Vec<LockCheckEntry>,
    pub(crate) clean: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LockOperation {
    pub(crate) kind: String,
    pub(crate) source_relative_path: String,
    pub(crate) tool: String,
    pub(crate) action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) merge_preview_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LockPlan {
    pub(crate) project_path: String,
    pub(crate) check: LockCheckResult,
    pub(crate) operations: Vec<LockOperation>,
    pub(crate) warnings: Vec<String>,
    pub(crate) blockers: Vec<String>,
    pub(crate) revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LockApplyResponse {
    pub(crate) plan: LockPlan,
    pub(crate) applied: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum VerifyStatus {
    Ok,
    Missing,
    Modified,
    Skipped,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VerifyEntry {
    pub(crate) entry: LockEntry,
    pub(crate) status: VerifyStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VerifyResult {
    pub(crate) entries: Vec<VerifyEntry>,
    pub(crate) clean: bool,
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::InvalidArgument {
        message: message.into(),
    }
}

fn canonical_project(project_path: &str) -> Result<std::path::PathBuf, AppError> {
    let project = std::fs::canonicalize(project_path).map_err(|error| AppError::Io {
        message: format!("canonicalize lockfile project: {error}"),
    })?;
    if !project.is_dir() {
        return Err(invalid("lockfile project must be a directory"));
    }
    Ok(project)
}

fn relative_project_path(project: &Path, path: &Path) -> Result<String, AppError> {
    let relative = path
        .strip_prefix(project)
        .map_err(|_| invalid("lockfile artifact is outside the project"))?;
    let parts = relative
        .components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .ok_or_else(|| invalid("lockfile artifact path must be UTF-8")),
            _ => Err(invalid("lockfile artifact path must be normalized")),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if parts.is_empty() {
        return Err(invalid("lockfile artifact path must not be empty"));
    }
    Ok(parts.join("/"))
}

fn normalize(mut lock: AgencyLock) -> Result<AgencyLock, AppError> {
    if lock.agency_lock != LOCK_VERSION {
        return Err(invalid("unsupported agency lockfile version"));
    }
    let mut identities = BTreeSet::new();
    let mut artifact_paths = BTreeSet::new();
    for entry in &mut lock.entries {
        if !matches!(entry.kind.as_str(), "agent" | "skill")
            || entry.scope != "project"
            || !super::valid_sha256(&entry.source_hash)
            || entry.source_relative_path.is_empty()
            || entry.tool.is_empty()
            || matches!(entry.source, PortableSource::Legacy { .. })
        {
            return Err(invalid("agency lockfile entry is invalid"));
        }
        if !identities.insert((
            entry.kind.clone(),
            entry.scope.clone(),
            entry.source_relative_path.clone(),
            entry.tool.clone(),
        )) {
            return Err(invalid(
                "agency lockfile contains a duplicate entry identity",
            ));
        }
        super::validate_portable_source(&entry.source)?;
        crate::library::validate_reference("lock", &entry.source_relative_path)?;
        for artifact in &entry.artifacts {
            crate::library::validate_reference("lock", &artifact.path)?;
            if !super::valid_sha256(&artifact.content_hash) {
                return Err(invalid("agency lockfile artifact hash is invalid"));
            }
            if !artifact_paths.insert(artifact.path.clone()) {
                return Err(invalid(
                    "agency lockfile contains a duplicate artifact path",
                ));
            }
        }
        entry.artifacts.sort();
        if entry.artifacts.is_empty() {
            return Err(invalid("agency lockfile entry has no artifacts"));
        }
    }
    lock.entries.sort();
    Ok(lock)
}

fn serialize(lock: &AgencyLock) -> Result<Vec<u8>, AppError> {
    let mut bytes = serde_json::to_vec_pretty(&normalize(lock.clone())?).map_err(|error| {
        AppError::Internal {
            message: format!("serialize agency lockfile: {error}"),
        }
    })?;
    bytes.push(b'\n');
    Ok(bytes)
}

async fn read(project: &Path) -> Result<AgencyLock, AppError> {
    let bytes = read_lockfile_bytes(project).await?;
    let lock = serde_json::from_slice(&bytes).map_err(|error| AppError::JsonParse {
        command: "lock_check".into(),
        message: error.to_string(),
        raw_excerpt: String::from_utf8_lossy(&bytes[..bytes.len().min(256)]).into_owned(),
    })?;
    normalize(lock)
}

async fn read_lockfile_bytes(project: &Path) -> Result<Vec<u8>, AppError> {
    crate::util::fs::read_capped(&project.join(LOCK_FILENAME), super::MAX_LEDGER_BYTES).await
}

async fn require_lockfile_unchanged(project: &Path, expected: &[u8]) -> Result<(), AppError> {
    if read_lockfile_bytes(project).await? == expected {
        Ok(())
    } else {
        Err(invalid(
            "shikigami.lock.json changed on disk during apply; applied changes were not allowed to overwrite it",
        ))
    }
}

async fn partial_apply_error(
    state: &AppState,
    project: &Path,
    original_lockfile_bytes: &[u8],
    applied: usize,
    error: AppError,
) -> AppError {
    let refresh = async {
        let derived = current_lock(state, project).await?;
        let bytes = serialize(&derived)?;
        require_lockfile_unchanged(project, original_lockfile_bytes).await?;
        crate::util::fs::atomic_write(&project.join(LOCK_FILENAME), &bytes).await
    }
    .await;
    AppError::Internal {
        message: match refresh {
            Ok(()) => format!(
                "lock apply failed after {applied} completed operation(s); shikigami.lock.json was refreshed to the derived post-state: {error}"
            ),
            Err(refresh_error) => format!(
                "lock apply failed after {applied} completed operation(s); filesystem or ledger state may include the failed operation, and shikigami.lock.json could not be refreshed ({refresh_error}): {error}"
            ),
        },
    }
}

fn finish_current_lock(
    entries: Vec<LockEntry>,
    mut unresolved: Vec<String>,
) -> Result<AgencyLock, AppError> {
    if !unresolved.is_empty() {
        unresolved.sort();
        unresolved.dedup();
        return Err(invalid(format!(
            "lockfile sources could not be resolved: {}",
            unresolved.join("; ")
        )));
    }
    normalize(AgencyLock {
        agency_lock: LOCK_VERSION,
        entries,
    })
}

fn lock_project_lockfile(project: &Path) -> Result<File, AppError> {
    let path = project.join(".agency-lockfile.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| AppError::Io {
            message: format!("open project lockfile lock {}: {error}", path.display()),
        })?;
    file.lock().map_err(|error| AppError::Io {
        message: format!("lock project lockfile {}: {error}", path.display()),
    })?;
    Ok(file)
}

async fn lock_project_lockfile_async(project: PathBuf) -> Result<File, AppError> {
    tokio::task::spawn_blocking(move || lock_project_lockfile(&project))
        .await
        .map_err(|error| AppError::Internal {
            message: format!("project lockfile lock task failed: {error}"),
        })?
}

async fn current_lock(state: &AppState, project: &Path) -> Result<AgencyLock, AppError> {
    let project_string = project.to_string_lossy();
    let agent_records = super::load_ledger_for_lock(state).await?;
    let skill_records = skills::install::load_ledger_for_state(state).await?;
    let agent_sources = if agent_records.is_empty() {
        Vec::new()
    } else {
        crate::agents::load_agent_sources(&state.app_data_dir).await?
    };
    let skill_sources = if skill_records.is_empty() {
        Vec::new()
    } else {
        skills::load_skill_sources_for_state(state).await?
    };
    let mut entries = Vec::new();
    let mut unresolved = Vec::new();

    for record in agent_records.iter().filter(|record| {
        record.project_path.as_deref() == Some(project_string.as_ref())
            && record.disabled_path.is_none()
    }) {
        let source = match portable_agent_source(record, &agent_sources) {
            Ok(source) => source,
            Err(error) => {
                unresolved.push(format!(
                    "agent {} for {} ({error})",
                    record.relative_path, record.tool
                ));
                continue;
            }
        };
        let artifacts = if record.artifacts.is_empty() {
            vec![LockArtifact {
                path: relative_project_path(project, Path::new(&record.dest))?,
                content_hash: record.rendered_hash.clone(),
            }]
        } else {
            record
                .artifacts
                .iter()
                .map(|artifact| {
                    Ok(LockArtifact {
                        path: relative_project_path(project, Path::new(&artifact.dest))?,
                        content_hash: artifact.rendered_hash.clone(),
                    })
                })
                .collect::<Result<Vec<_>, AppError>>()?
        };
        entries.push(LockEntry {
            kind: "agent".into(),
            source,
            source_relative_path: record.relative_path.clone(),
            source_hash: record.source_hash.clone(),
            tool: record.tool.clone(),
            scope: "project".into(),
            artifacts,
        });
    }

    for record in skill_records.iter().filter(|record| {
        record.project_path.as_deref() == Some(project_string.as_ref())
            && record.disabled_path.is_none()
    }) {
        let source = match portable_skill_source(record, &skill_sources) {
            Ok(source) => source,
            Err(error) => {
                unresolved.push(format!(
                    "skill {} for {} ({error})",
                    record.relative_path, record.runtime
                ));
                continue;
            }
        };
        let package =
            skills::resolve_skill_package(state, &record.source_id, &record.relative_path).await?;
        let destination = Path::new(&record.dest);
        let artifacts = package
            .files()
            .iter()
            .map(|file| {
                Ok(LockArtifact {
                    path: relative_project_path(project, &destination.join(&file.relative_path))?,
                    content_hash: file.sha256.clone(),
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        entries.push(LockEntry {
            kind: "skill".into(),
            source,
            source_relative_path: record.relative_path.clone(),
            source_hash: record.source_hash.clone(),
            tool: record.runtime.clone(),
            scope: "project".into(),
            artifacts,
        });
    }
    finish_current_lock(entries, unresolved)
}

pub(crate) async fn sync_project_lock(
    state: &AppState,
    project_path: &str,
) -> Result<(), AppError> {
    let project = canonical_project(project_path)?;
    let _guard = lock_project_lockfile_async(project.clone()).await?;
    let lock = current_lock(state, &project).await?;
    crate::util::fs::atomic_write(&project.join(LOCK_FILENAME), &serialize(&lock)?).await
}

pub(crate) async fn sync_project_lock_best_effort(state: &AppState, project_path: &str) {
    if let Err(error) = sync_project_lock(state, project_path).await {
        tracing::warn!(
            "project {project_path}: lockfile left unchanged, could not re-derive it: {error}"
        );
    }
}

fn artifact_state(project: &Path, artifacts: &[LockArtifact]) -> LockEntryStatus {
    let mut missing = false;
    for artifact in artifacts {
        let path = project.join(&artifact.path);
        match std::fs::read(&path) {
            Ok(bytes) if render::sha256_hex(&bytes) == artifact.content_hash => {}
            Ok(_) => return LockEntryStatus::Modified,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => missing = true,
            Err(_) => return LockEntryStatus::Modified,
        }
    }
    if missing {
        LockEntryStatus::Missing
    } else {
        LockEntryStatus::Current
    }
}

fn verify_artifacts(
    project: &Path,
    project_root: &File,
    artifacts: &[LockArtifact],
) -> Result<VerifyStatus, AppError> {
    let mut status = VerifyStatus::Ok;
    for artifact in artifacts {
        let path = project.join(&artifact.path);
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if status != VerifyStatus::Modified {
                    status = VerifyStatus::Missing;
                }
                continue;
            }
            Err(error) => {
                return Err(AppError::Io {
                    message: format!("inspect lockfile artifact {}: {error}", path.display()),
                })
            }
        };
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || crate::skills::metadata_is_reparse_point(&metadata)
        {
            return Err(invalid(format!(
                "lockfile artifact is not a regular file: {}",
                path.display()
            )));
        }
        match crate::skills::install::read_project_file(
            project_root,
            Path::new(&artifact.path),
            super::MAX_LEDGER_BYTES,
        ) {
            Ok(bytes) if render::sha256_hex(&bytes) == artifact.content_hash => {}
            Ok(_) => status = VerifyStatus::Modified,
            Err(error) => {
                return Err(AppError::Io {
                    message: format!("read lockfile artifact {}: {error}", path.display()),
                })
            }
        }
    }
    Ok(status)
}

pub(crate) fn verify_lockfile(
    lockfile_bytes: &[u8],
    project: &Path,
) -> Result<VerifyResult, AppError> {
    if lockfile_bytes.len() as u64 > super::MAX_LEDGER_BYTES {
        return Err(invalid("agency lockfile exceeds the size limit"));
    }
    let lock = serde_json::from_slice(lockfile_bytes).map_err(|error| AppError::JsonParse {
        command: "verify".into(),
        message: error.to_string(),
        raw_excerpt: String::from_utf8_lossy(&lockfile_bytes[..lockfile_bytes.len().min(256)])
            .into_owned(),
    })?;
    let lock = normalize(lock)?;
    let project_root =
        cap_primitives::fs::open_ambient_dir(project, cap_primitives::ambient_authority())
            .map_err(|error| AppError::Io {
                message: format!("open lockfile project {}: {error}", project.display()),
            })?;
    let entries = lock
        .entries
        .into_iter()
        .map(|entry| {
            let status = verify_artifacts(project, &project_root, &entry.artifacts)?;
            Ok(VerifyEntry {
                entry,
                status,
                reason: None,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    let clean = entries.iter().all(|entry| entry.status == VerifyStatus::Ok);
    Ok(VerifyResult { entries, clean })
}

fn classify(
    ledger: Option<&LockEntry>,
    expected_disk: LockEntryStatus,
    managed_disk: Option<LockEntryStatus>,
    expected: &LockEntry,
) -> LockEntryStatus {
    let Some(ledger) = ledger else {
        return if expected_disk == LockEntryStatus::Missing {
            LockEntryStatus::Missing
        } else {
            LockEntryStatus::Foreign
        };
    };
    if ledger.source_hash != expected.source_hash || ledger.source != expected.source {
        return match managed_disk {
            Some(LockEntryStatus::Current) => LockEntryStatus::Outdated,
            Some(LockEntryStatus::Missing) => LockEntryStatus::Missing,
            _ => LockEntryStatus::Modified,
        };
    }
    expected_disk
}

fn same_identity(left: &LockEntry, right: &LockEntry) -> bool {
    left.kind == right.kind
        && left.source_relative_path == right.source_relative_path
        && left.tool == right.tool
        && left.scope == right.scope
}

async fn check_at(state: &AppState, project: &Path) -> Result<LockCheckResult, AppError> {
    let lock = read(project).await?;
    let current = current_lock(state, project).await?;
    let mut entries = lock
        .entries
        .iter()
        .cloned()
        .map(|entry| {
            let ledger = current
                .entries
                .iter()
                .find(|candidate| same_identity(candidate, &entry));
            let expected_disk = artifact_state(project, &entry.artifacts);
            let managed_disk = ledger.map(|entry| artifact_state(project, &entry.artifacts));
            let status = classify(ledger, expected_disk, managed_disk, &entry);
            LockCheckEntry { entry, status }
        })
        .collect::<Vec<_>>();
    entries.extend(
        current
            .entries
            .into_iter()
            .filter(|entry| {
                !lock
                    .entries
                    .iter()
                    .any(|locked| same_identity(locked, entry))
            })
            .map(|entry| LockCheckEntry {
                entry,
                status: LockEntryStatus::Extra,
            }),
    );
    entries.sort_by(|left, right| left.entry.cmp(&right.entry));
    let clean = entries
        .iter()
        .all(|entry| entry.status == LockEntryStatus::Current);
    Ok(LockCheckResult {
        lock,
        entries,
        clean,
    })
}

fn pack_from_lock(lock: &AgencyLock) -> WorkspacePack {
    WorkspacePack {
        workspace_pack: super::WORKSPACE_PACK_VERSION,
        name: LOCK_FILENAME.into(),
        scope: WorkspacePackScope::Project,
        agents: lock
            .entries
            .iter()
            .filter(|entry| entry.kind == "agent")
            .map(|entry| WorkspacePackAgent {
                source: entry.source.clone(),
                reference: AgentReference {
                    source_id: "pending:lock".into(),
                    relative_path: entry.source_relative_path.clone(),
                },
                tool: entry.tool.clone(),
            })
            .collect(),
        skills: lock
            .entries
            .iter()
            .filter(|entry| entry.kind == "skill")
            .map(|entry| WorkspacePackSkill {
                source: entry.source.clone(),
                reference: SkillReference {
                    source_id: "pending:lock".into(),
                    relative_path: entry.source_relative_path.clone(),
                },
                runtime: entry.tool.clone(),
            })
            .collect(),
        runbook: None,
        instructions: Vec::new(),
        mcp_servers: Vec::new(),
    }
}

fn merged_lock_operation(
    entry: &LockEntry,
    outcome: crate::types::AgentMergeOutcome,
) -> Result<LockOperation, String> {
    match outcome {
        crate::types::AgentMergeOutcome::Clean { preview_hash } => Ok(LockOperation {
            kind: entry.kind.clone(),
            source_relative_path: entry.source_relative_path.clone(),
            tool: entry.tool.clone(),
            action: "update".into(),
            merge_preview_hash: Some(preview_hash),
        }),
        crate::types::AgentMergeOutcome::Conflicts { count, .. } => Err(format!(
            "Lock entry has {count} merge conflict(s): {}",
            entry.source_relative_path
        )),
        crate::types::AgentMergeOutcome::Unavailable { reason } => Err(format!(
            "Lock entry merge is unavailable ({reason}): {}",
            entry.source_relative_path
        )),
    }
}

fn classify_workspace_warnings(
    warnings: Vec<String>,
    headless: bool,
    blockers: &mut Vec<String>,
) -> Vec<String> {
    warnings
        .into_iter()
        .filter(|warning| {
            if headless && warning.starts_with("Agent update requires explicit review:") {
                blockers.push(warning.clone());
                false
            } else {
                true
            }
        })
        .collect()
}

async fn plan_at(
    state: &AppState,
    project: &Path,
    allow_merge: bool,
    headless: bool,
) -> Result<LockPlan, AppError> {
    let check = check_at(state, project).await?;
    let project_string = project.to_string_lossy().into_owned();
    let workspace = build_workspace_pack_plan(
        state,
        pack_from_lock(&check.lock),
        Some(project_string.clone()),
    )
    .await?;
    let mut operations = Vec::new();
    let mut blockers = workspace
        .blockers
        .into_iter()
        .filter(|blocker| !blocker.contains("state outdated"))
        .collect::<Vec<_>>();
    let mut warnings = classify_workspace_warnings(workspace.warnings, headless, &mut blockers);
    for checked in &check.entries {
        match checked.status {
            LockEntryStatus::Current => {}
            LockEntryStatus::Missing => operations.push(LockOperation {
                kind: checked.entry.kind.clone(),
                source_relative_path: checked.entry.source_relative_path.clone(),
                tool: checked.entry.tool.clone(),
                action: "install".into(),
                merge_preview_hash: None,
            }),
            LockEntryStatus::Outdated => operations.push(LockOperation {
                kind: checked.entry.kind.clone(),
                source_relative_path: checked.entry.source_relative_path.clone(),
                tool: checked.entry.tool.clone(),
                action: "update".into(),
                merge_preview_hash: None,
            }),
            LockEntryStatus::Extra => warnings.push(format!(
                "Project has an unlocked {}: {}",
                checked.entry.kind, checked.entry.source_relative_path
            )),
            LockEntryStatus::Modified if allow_merge && checked.entry.kind == "agent" => {
                let item = workspace.agents.iter().find(|item| {
                    item.reference.relative_path == checked.entry.source_relative_path
                        && item.tool == checked.entry.tool
                });
                let modified_blocker_suffix = item.map(|item| {
                    format!(
                        "{}:{}",
                        item.reference.source_id, item.reference.relative_path
                    )
                });
                let computed = match item {
                    Some(item) => {
                        super::merge_for_reference(
                            state,
                            &item.reference,
                            &item.tool,
                            Some(&project_string),
                        )
                        .await
                    }
                    None => Err(invalid("lockfile Agent merge source could not be resolved")),
                };
                match computed
                    .map(|computed| computed.outcome)
                    .map(|outcome| merged_lock_operation(&checked.entry, outcome))
                {
                    Ok(Ok(operation)) => {
                        if let Some(suffix) = modified_blocker_suffix.as_deref() {
                            blockers.retain(|blocker| {
                                !(blocker.contains("Agent is not safe to apply in state modified")
                                    && blocker.ends_with(suffix))
                            });
                        }
                        operations.push(operation);
                    }
                    Ok(Err(blocker)) => blockers.push(blocker),
                    Err(error) => blockers.push(format!(
                        "Lock entry merge is unavailable ({error}): {}",
                        checked.entry.source_relative_path
                    )),
                }
            }
            LockEntryStatus::Modified | LockEntryStatus::Foreign => blockers.push(format!(
                "Lock entry is not safe to apply in state {:?}: {}",
                checked.status, checked.entry.source_relative_path
            )),
        }
    }
    operations.sort_by(|left, right| {
        (&left.kind, &left.source_relative_path, &left.tool).cmp(&(
            &right.kind,
            &right.source_relative_path,
            &right.tool,
        ))
    });
    warnings.sort();
    warnings.dedup();
    blockers.sort();
    blockers.dedup();
    let mut plan = LockPlan {
        project_path: project_string,
        check,
        operations,
        warnings,
        blockers,
        revision: String::new(),
    };
    plan.revision =
        render::sha256_hex(
            &serde_json::to_vec(&plan).map_err(|error| AppError::Internal {
                message: format!("serialize lock plan: {error}"),
            })?,
        );
    Ok(plan)
}

#[tauri::command]
pub async fn lock_check(
    state: State<'_, AppState>,
    project_path: String,
) -> Result<LockCheckResult, AppError> {
    let project = canonical_project(&project_path)?;
    check_at(&state, &project).await
}

#[tauri::command]
pub async fn lock_plan(
    state: State<'_, AppState>,
    project_path: String,
) -> Result<LockPlan, AppError> {
    let project = canonical_project(&project_path)?;
    plan_at(&state, &project, false, false).await
}

#[tauri::command]
pub async fn lock_apply(
    app: AppHandle,
    state: State<'_, AppState>,
    project_path: String,
    revision: String,
) -> Result<LockApplyResponse, AppError> {
    let project = canonical_project(&project_path)?;
    apply_at(&state, &project, &revision, LockApplyPolicy::Desktop(&app)).await
}

enum LockApplyPolicy<'a> {
    Desktop(&'a AppHandle),
    Mcp(Option<&'a crate::state::AuthorizedMcpProject>),
    Cli { merge: bool },
}

async fn apply_at(
    state: &AppState,
    project: &Path,
    revision: &str,
    policy: LockApplyPolicy<'_>,
) -> Result<LockApplyResponse, AppError> {
    let allow_merge = matches!(&policy, LockApplyPolicy::Cli { merge: true });
    let headless = matches!(&policy, LockApplyPolicy::Cli { .. });
    let _project_guard = lock_project_lockfile_async(project.to_path_buf()).await?;
    let original_lockfile_bytes = read_lockfile_bytes(project).await?;
    let mut plan = plan_at(state, project, allow_merge, headless).await?;
    if plan.revision != revision {
        plan.blockers
            .push("Lock plan changed; review the refreshed plan before applying".into());
        return Ok(LockApplyResponse {
            plan,
            applied: false,
        });
    }
    if !plan.blockers.is_empty() {
        return Ok(LockApplyResponse {
            plan,
            applied: false,
        });
    }
    if matches!(policy, LockApplyPolicy::Mcp(_))
        && plan
            .operations
            .iter()
            .any(|operation| operation.kind == "agent" && operation.action == "update")
    {
        plan.blockers
            .push("MCP lock_apply requires desktop approval for Agent updates".into());
        return Ok(LockApplyResponse {
            plan,
            applied: false,
        });
    }
    let mut workspace = build_workspace_pack_plan(
        state,
        pack_from_lock(&plan.check.lock),
        Some(plan.project_path.clone()),
    )
    .await?;
    materialize_workspace_pack_sources(state, &workspace.source_additions).await?;
    workspace =
        build_workspace_pack_plan(state, workspace.pack, Some(plan.project_path.clone())).await?;
    if !workspace.source_additions.is_empty() {
        return Err(invalid("lockfile sources could not be materialized"));
    }
    for (applied, operation) in plan.operations.clone().into_iter().enumerate() {
        let operation_result: Result<(), AppError> = async {
            if operation.kind == "agent" {
                let item = workspace
                    .agents
                    .iter()
                    .find(|item| {
                        item.reference.relative_path == operation.source_relative_path
                            && item.tool == operation.tool
                    })
                    .ok_or_else(|| invalid("lockfile Agent operation could not be resolved"))?;
                let record = match &policy {
                    LockApplyPolicy::Desktop(app) => {
                        super::do_install_lockfile_desktop(
                            app,
                            state,
                            item.reference.clone(),
                            item.tool.clone(),
                            plan.project_path.clone(),
                        )
                        .await?
                    }
                    LockApplyPolicy::Mcp(authorization) => {
                        super::mcp_install_agent_clean_for_lockfile(
                            state,
                            item.reference.clone(),
                            item.tool.clone(),
                            plan.project_path.clone(),
                            *authorization,
                        )
                        .await?
                    }
                    LockApplyPolicy::Cli { .. } => match operation.merge_preview_hash.as_deref() {
                        Some(preview_hash) => {
                            super::do_install_headless_merge(
                                state,
                                item.reference.clone(),
                                item.tool.clone(),
                                plan.project_path.clone(),
                                preview_hash,
                            )
                            .await?
                        }
                        None => {
                            super::do_install_headless_lock(
                                state,
                                item.reference.clone(),
                                item.tool.clone(),
                                plan.project_path.clone(),
                            )
                            .await?
                        }
                    },
                };
                let expected = plan
                    .check
                    .lock
                    .entries
                    .iter_mut()
                    .find(|entry| {
                        entry.kind == "agent"
                            && entry.source_relative_path == operation.source_relative_path
                            && entry.tool == operation.tool
                    })
                    .ok_or_else(|| invalid("lockfile Agent entry disappeared"))?;
                if operation.merge_preview_hash.is_some() {
                    let actual_hashes = if record.artifacts.is_empty() {
                        vec![record.rendered_hash.clone()]
                    } else {
                        record
                            .artifacts
                            .iter()
                            .map(|artifact| artifact.rendered_hash.clone())
                            .collect()
                    };
                    if expected.artifacts.len() != actual_hashes.len() {
                        return Err(invalid("merged Agent artifact count changed"));
                    }
                    for (locked, installed_hash) in expected.artifacts.iter_mut().zip(actual_hashes)
                    {
                        locked.content_hash = installed_hash;
                    }
                }
                let expected_source_hash = expected.source_hash.clone();
                let expected_source = expected.source.clone();
                if record.source_hash != expected_source_hash
                    || installed_source_revision(state, &item.reference, &record.source_hash)
                        .await?
                        != match &expected_source {
                            PortableSource::Builtin { source_revision } => source_revision.clone(),
                            PortableSource::Github {
                                resolved_commit: Some(commit),
                                ..
                            } => commit.clone(),
                            _ => record.source_hash.clone(),
                        }
                {
                    return Err(invalid(
                        "installed Agent does not match the lockfile source",
                    ));
                }
            } else {
                let item = workspace
                    .skills
                    .iter()
                    .find(|item| {
                        item.reference.relative_path == operation.source_relative_path
                            && item.runtime == operation.tool
                    })
                    .ok_or_else(|| invalid("lockfile Skill operation could not be resolved"))?;
                match &policy {
                    LockApplyPolicy::Mcp(authorization) => {
                        skills::install_skill_with_dependencies_for_lockfile(
                            state,
                            &item.reference.source_id,
                            &item.reference.relative_path,
                            &item.runtime,
                            &plan.project_path,
                            *authorization,
                        )
                        .await?;
                    }
                    LockApplyPolicy::Desktop(_) | LockApplyPolicy::Cli { .. } => {
                        skills::install_skill_with_dependencies_for_lockfile(
                            state,
                            &item.reference.source_id,
                            &item.reference.relative_path,
                            &item.runtime,
                            &plan.project_path,
                            None,
                        )
                        .await?;
                    }
                }
            }
            Ok(())
        }
        .await;
        if let Err(error) = operation_result {
            return Err(partial_apply_error(
                state,
                project,
                &original_lockfile_bytes,
                applied,
                error,
            )
            .await);
        }
    }
    require_lockfile_unchanged(project, &original_lockfile_bytes).await?;
    crate::util::fs::atomic_write(&project.join(LOCK_FILENAME), &serialize(&plan.check.lock)?)
        .await?;
    plan = plan_at(state, project, allow_merge, headless).await?;
    Ok(LockApplyResponse {
        applied: plan.check.clean,
        plan,
    })
}

pub(crate) async fn mcp_lock_check(
    state: &AppState,
    project_path: &str,
) -> Result<LockCheckResult, AppError> {
    check_at(state, &canonical_project(project_path)?).await
}

pub(crate) async fn mcp_lock_plan(
    state: &AppState,
    project_path: &str,
) -> Result<LockPlan, AppError> {
    plan_at(state, &canonical_project(project_path)?, false, false).await
}

pub(crate) async fn mcp_lock_apply(
    state: &AppState,
    project_path: &str,
    revision: &str,
    authorization: Option<&crate::state::AuthorizedMcpProject>,
) -> Result<LockApplyResponse, AppError> {
    let project = canonical_project(project_path)?;
    apply_at(
        state,
        &project,
        revision,
        LockApplyPolicy::Mcp(authorization),
    )
    .await
}

pub(crate) async fn cli_lock_check(
    state: &AppState,
    project_path: &str,
) -> Result<LockCheckResult, AppError> {
    check_at(state, &canonical_project(project_path)?).await
}

pub(crate) async fn cli_lock_plan(
    state: &AppState,
    project_path: &str,
    allow_merge: bool,
) -> Result<LockPlan, AppError> {
    plan_at(state, &canonical_project(project_path)?, allow_merge, true).await
}

pub(crate) async fn cli_lock_apply(
    state: &AppState,
    project_path: &str,
    revision: &str,
    merge: bool,
) -> Result<LockApplyResponse, AppError> {
    let project = canonical_project(project_path)?;
    apply_at(state, &project, revision, LockApplyPolicy::Cli { merge }).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(hash: &str) -> LockEntry {
        LockEntry {
            kind: "agent".into(),
            source: PortableSource::Builtin {
                source_revision: "a".repeat(64),
            },
            source_relative_path: "engineering/reviewer.md".into(),
            source_hash: hash.into(),
            tool: "codex".into(),
            scope: "project".into(),
            artifacts: vec![LockArtifact {
                path: ".codex/agents/reviewer.md".into(),
                content_hash: "b".repeat(64),
            }],
        }
    }

    #[test]
    fn lockfile_serialization_is_byte_identical() {
        let lock = AgencyLock {
            agency_lock: 1,
            entries: vec![entry(&"c".repeat(64))],
        };
        let first = serialize(&lock).unwrap();
        assert_eq!(first, serialize(&lock).unwrap());
        let text = String::from_utf8(first).unwrap();
        let keys = [
            "\"artifacts\"",
            "\"kind\"",
            "\"scope\"",
            "\"source\"",
            "\"sourceHash\"",
            "\"sourceRelativePath\"",
            "\"tool\"",
        ];
        assert!(keys
            .windows(2)
            .all(|pair| text.find(pair[0]).unwrap() < text.find(pair[1]).unwrap()));
    }

    #[test]
    fn unresolved_source_cannot_produce_a_truncated_lockfile() {
        let error = finish_current_lock(
            vec![entry(&"c".repeat(64))],
            vec!["agent engineering/missing.md for codex".into()],
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("agent engineering/missing.md for codex"));
    }

    #[test]
    fn stateless_verify_reports_only_project_filesystem_statuses() {
        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir(project.path().join("agents")).unwrap();
        std::fs::write(project.path().join("agents/ok.md"), b"ok\n").unwrap();
        std::fs::write(project.path().join("agents/modified.md"), b"changed\n").unwrap();
        let mut ok = entry(&"c".repeat(64));
        ok.source_relative_path = "engineering/ok.md".into();
        ok.artifacts[0].path = "agents/ok.md".into();
        ok.artifacts[0].content_hash = render::sha256_hex(b"ok\n");

        let mut modified = entry(&"d".repeat(64));
        modified.source_relative_path = "engineering/modified.md".into();
        modified.artifacts[0].path = "agents/modified.md".into();

        let mut missing = entry(&"e".repeat(64));
        missing.source_relative_path = "engineering/missing.md".into();
        missing.artifacts[0].path = "agents/missing.md".into();

        let bytes = serde_json::to_vec(&AgencyLock {
            agency_lock: LOCK_VERSION,
            entries: vec![missing, modified, ok],
        })
        .unwrap();
        let result = verify_lockfile(&bytes, project.path()).unwrap();

        assert!(!result.clean);
        let mut statuses = result
            .entries
            .iter()
            .map(|entry| {
                (
                    entry.entry.source_relative_path.as_str(),
                    entry.entry.scope.as_str(),
                    entry.status,
                    entry.reason,
                )
            })
            .collect::<Vec<_>>();
        statuses.sort_by_key(|entry| entry.0);
        assert_eq!(
            statuses,
            vec![
                (
                    "engineering/missing.md",
                    "project",
                    VerifyStatus::Missing,
                    None
                ),
                (
                    "engineering/modified.md",
                    "project",
                    VerifyStatus::Modified,
                    None,
                ),
                ("engineering/ok.md", "project", VerifyStatus::Ok, None),
            ]
        );
    }

    #[test]
    fn stateless_verify_rejects_user_scoped_entries() {
        let project = tempfile::tempdir().unwrap();
        let mut user = entry(&"c".repeat(64));
        user.scope = "user".into();
        let bytes = serde_json::to_vec(&AgencyLock {
            agency_lock: LOCK_VERSION,
            entries: vec![user],
        })
        .unwrap();

        assert!(verify_lockfile(&bytes, project.path())
            .unwrap_err()
            .to_string()
            .contains("entry is invalid"));
    }

    #[test]
    fn lockfile_rejects_duplicate_identities_and_artifact_paths() {
        let first = entry(&"c".repeat(64));
        let mut duplicate_identity = first.clone();
        duplicate_identity.source_hash = "d".repeat(64);
        assert!(normalize(AgencyLock {
            agency_lock: LOCK_VERSION,
            entries: vec![first.clone(), duplicate_identity],
        })
        .unwrap_err()
        .to_string()
        .contains("duplicate entry identity"));

        let mut duplicate_artifact = entry(&"d".repeat(64));
        duplicate_artifact.source_relative_path = "engineering/other.md".into();
        assert!(normalize(AgencyLock {
            agency_lock: LOCK_VERSION,
            entries: vec![first, duplicate_artifact],
        })
        .unwrap_err()
        .to_string()
        .contains("duplicate artifact path"));
    }

    #[cfg(unix)]
    #[test]
    fn stateless_verify_rejects_linked_artifact_ancestors() {
        use std::os::unix::fs::symlink;

        let project = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("artifact.md"), b"outside").unwrap();
        symlink(outside.path(), project.path().join("agents")).unwrap();
        let mut linked = entry(&"c".repeat(64));
        linked.artifacts[0].path = "agents/artifact.md".into();
        linked.artifacts[0].content_hash = render::sha256_hex(b"outside");
        let bytes = serde_json::to_vec(&AgencyLock {
            agency_lock: LOCK_VERSION,
            entries: vec![linked],
        })
        .unwrap();

        assert!(verify_lockfile(&bytes, project.path())
            .unwrap_err()
            .to_string()
            .contains("read lockfile artifact"));
    }

    #[tokio::test]
    async fn apply_lock_is_exclusive_and_stale_bytes_are_rejected() {
        let project = tempfile::tempdir().unwrap();
        let lockfile = project.path().join(LOCK_FILENAME);
        std::fs::write(&lockfile, b"planned").unwrap();
        let first = lock_project_lockfile_async(project.path().to_path_buf())
            .await
            .unwrap();
        let mut second = tokio::spawn(lock_project_lockfile_async(project.path().to_path_buf()));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut second)
                .await
                .is_err()
        );
        drop(first);
        tokio::time::timeout(std::time::Duration::from_secs(2), second)
            .await
            .unwrap()
            .unwrap()
            .unwrap();

        let planned = std::fs::read(&lockfile).unwrap();
        std::fs::write(&lockfile, b"edited").unwrap();
        assert!(require_lockfile_unchanged(project.path(), &planned)
            .await
            .unwrap_err()
            .to_string()
            .contains("changed on disk"));
        assert_eq!(std::fs::read(lockfile).unwrap(), b"edited");
    }

    #[test]
    fn lock_status_classification_covers_disk_and_ledger_drift() {
        let expected = entry(&"c".repeat(64));
        assert_eq!(
            classify(
                Some(&expected),
                LockEntryStatus::Current,
                Some(LockEntryStatus::Current),
                &expected,
            ),
            LockEntryStatus::Current
        );
        assert_eq!(
            classify(None, LockEntryStatus::Missing, None, &expected),
            LockEntryStatus::Missing
        );
        assert_eq!(
            classify(None, LockEntryStatus::Modified, None, &expected),
            LockEntryStatus::Foreign
        );
        assert_eq!(
            classify(
                Some(&expected),
                LockEntryStatus::Modified,
                Some(LockEntryStatus::Modified),
                &expected,
            ),
            LockEntryStatus::Modified
        );
        assert_eq!(
            classify(
                Some(&entry(&"d".repeat(64))),
                LockEntryStatus::Modified,
                Some(LockEntryStatus::Current),
                &expected,
            ),
            LockEntryStatus::Outdated
        );
        assert_eq!(
            classify(
                Some(&entry(&"d".repeat(64))),
                LockEntryStatus::Modified,
                Some(LockEntryStatus::Modified),
                &expected,
            ),
            LockEntryStatus::Modified
        );
    }

    #[test]
    fn cli_merge_plans_clean_updates_and_blocks_conflicts() {
        let entry = entry(&"c".repeat(64));
        let clean = merged_lock_operation(
            &entry,
            crate::types::AgentMergeOutcome::Clean {
                preview_hash: "d".repeat(64),
            },
        )
        .unwrap();
        assert_eq!(clean.action, "update");
        assert_eq!(clean.merge_preview_hash, Some("d".repeat(64)));

        let conflict = merged_lock_operation(
            &entry,
            crate::types::AgentMergeOutcome::Conflicts {
                count: 1,
                hunk_summaries: vec!["Conflict 1: merged lines 2-8".into()],
            },
        )
        .unwrap_err();
        assert!(conflict.contains("1 merge conflict"));
    }

    #[test]
    fn headless_lock_plans_promote_update_review_to_a_blocker() {
        let review = "Agent update requires explicit review: Reviewer (Notify)".to_string();
        let mut blockers = Vec::new();
        assert!(classify_workspace_warnings(vec![review.clone()], true, &mut blockers).is_empty());
        assert_eq!(blockers, vec![review.clone()]);

        let mut desktop_blockers = Vec::new();
        assert_eq!(
            classify_workspace_warnings(vec![review.clone()], false, &mut desktop_blockers),
            vec![review]
        );
        assert!(desktop_blockers.is_empty());
    }
}
