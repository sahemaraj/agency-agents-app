use std::path::{Component, Path};

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
pub(crate) const LOCK_FILENAME: &str = "agency.lock.json";

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

fn normalize(mut lock: AgencyLock, allow_user_scope: bool) -> Result<AgencyLock, AppError> {
    if lock.agency_lock != LOCK_VERSION {
        return Err(invalid("unsupported agency lockfile version"));
    }
    for entry in &mut lock.entries {
        let valid_scope = entry.scope == "project" || (allow_user_scope && entry.scope == "user");
        if !matches!(entry.kind.as_str(), "agent" | "skill")
            || !valid_scope
            || !super::valid_sha256(&entry.source_hash)
            || entry.source_relative_path.is_empty()
            || entry.tool.is_empty()
            || matches!(entry.source, PortableSource::Legacy { .. })
        {
            return Err(invalid("agency lockfile entry is invalid"));
        }
        super::validate_portable_source(&entry.source)?;
        crate::library::validate_reference("lock", &entry.source_relative_path)?;
        for artifact in &entry.artifacts {
            crate::library::validate_reference("lock", &artifact.path)?;
            if !super::valid_sha256(&artifact.content_hash) {
                return Err(invalid("agency lockfile artifact hash is invalid"));
            }
        }
        entry.artifacts.sort();
        entry.artifacts.dedup();
        if entry.artifacts.is_empty() {
            return Err(invalid("agency lockfile entry has no artifacts"));
        }
    }
    lock.entries.sort();
    lock.entries.dedup();
    Ok(lock)
}

fn serialize(lock: &AgencyLock) -> Result<Vec<u8>, AppError> {
    let mut bytes =
        serde_json::to_vec_pretty(&normalize(lock.clone(), false)?).map_err(|error| {
            AppError::Internal {
                message: format!("serialize agency lockfile: {error}"),
            }
        })?;
    bytes.push(b'\n');
    Ok(bytes)
}

async fn read(project: &Path) -> Result<AgencyLock, AppError> {
    let bytes =
        crate::util::fs::read_capped(&project.join(LOCK_FILENAME), super::MAX_LEDGER_BYTES).await?;
    let lock = serde_json::from_slice(&bytes).map_err(|error| AppError::JsonParse {
        command: "lock_check".into(),
        message: error.to_string(),
        raw_excerpt: String::from_utf8_lossy(&bytes[..bytes.len().min(256)]).into_owned(),
    })?;
    normalize(lock, false)
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
    normalize(
        AgencyLock {
            agency_lock: LOCK_VERSION,
            entries,
        },
        false,
    )
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

    for record in agent_records
        .iter()
        .filter(|record| record.project_path.as_deref() == Some(project_string.as_ref()))
    {
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

    for record in skill_records
        .iter()
        .filter(|record| record.project_path.as_deref() == Some(project_string.as_ref()))
    {
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
    let lock = current_lock(state, &project).await?;
    crate::util::fs::atomic_write(&project.join(LOCK_FILENAME), &serialize(&lock)?).await
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

fn verify_artifacts<F>(
    project: &Path,
    artifacts: &[LockArtifact],
    read_file: &mut F,
) -> Result<VerifyStatus, AppError>
where
    F: FnMut(&Path) -> std::io::Result<Vec<u8>>,
{
    let mut status = VerifyStatus::Ok;
    for artifact in artifacts {
        let path = project.join(&artifact.path);
        match read_file(&path) {
            Ok(bytes) if render::sha256_hex(&bytes) == artifact.content_hash => {}
            Ok(_) => status = VerifyStatus::Modified,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if status != VerifyStatus::Modified {
                    status = VerifyStatus::Missing;
                }
            }
            Err(error) => {
                return Err(AppError::Io {
                    message: format!("read lockfile artifact {}: {error}", path.display()),
                })
            }
        }
    }
    Ok(status)
}

pub(crate) fn verify_lockfile<F>(
    lockfile_bytes: &[u8],
    project: &Path,
    mut read_file: F,
) -> Result<VerifyResult, AppError>
where
    F: FnMut(&Path) -> std::io::Result<Vec<u8>>,
{
    if lockfile_bytes.len() as u64 > super::MAX_LEDGER_BYTES {
        return Err(invalid("agency lockfile exceeds the size limit"));
    }
    let lock = serde_json::from_slice(lockfile_bytes).map_err(|error| AppError::JsonParse {
        command: "verify".into(),
        message: error.to_string(),
        raw_excerpt: String::from_utf8_lossy(&lockfile_bytes[..lockfile_bytes.len().min(256)])
            .into_owned(),
    })?;
    let lock = normalize(lock, true)?;
    let entries = lock
        .entries
        .into_iter()
        .map(|entry| {
            let (status, reason) = if entry.scope == "user" {
                (VerifyStatus::Skipped, Some("user-scope"))
            } else {
                (
                    verify_artifacts(project, &entry.artifacts, &mut read_file)?,
                    None,
                )
            };
            Ok(VerifyEntry {
                entry,
                status,
                reason,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    let clean = entries
        .iter()
        .all(|entry| matches!(entry.status, VerifyStatus::Ok | VerifyStatus::Skipped));
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

async fn plan_at(
    state: &AppState,
    project: &Path,
    allow_merge: bool,
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
    let mut warnings = Vec::new();
    let mut blockers = workspace
        .blockers
        .into_iter()
        .filter(|blocker| !blocker.contains("state outdated"))
        .collect::<Vec<_>>();
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
    plan_at(&state, &project, false).await
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
    let mut plan = plan_at(state, project, allow_merge).await?;
    if plan.revision != revision || !plan.blockers.is_empty() {
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
    for operation in &plan.operations {
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
                    super::do_install(
                        app,
                        state,
                        item.reference.clone(),
                        item.tool.clone(),
                        Some(plan.project_path.clone()),
                        true,
                    )
                    .await?
                }
                LockApplyPolicy::Mcp(authorization) => {
                    super::mcp_install_agent_clean(
                        state,
                        item.reference.clone(),
                        item.tool.clone(),
                        Some(plan.project_path.clone()),
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
                for (locked, installed_hash) in expected.artifacts.iter_mut().zip(actual_hashes) {
                    locked.content_hash = installed_hash;
                }
            }
            let expected_source_hash = expected.source_hash.clone();
            let expected_source = expected.source.clone();
            if record.source_hash != expected_source_hash
                || installed_source_revision(state, &item.reference, &record.source_hash).await?
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
                    skills::install_skill_with_dependencies_authorized(
                        state,
                        &item.reference.source_id,
                        &item.reference.relative_path,
                        &item.runtime,
                        Some(&plan.project_path),
                        *authorization,
                    )
                    .await?;
                }
                LockApplyPolicy::Desktop(_) | LockApplyPolicy::Cli { .. } => {
                    skills::install_skill_with_dependencies(
                        state,
                        &item.reference.source_id,
                        &item.reference.relative_path,
                        &item.runtime,
                        Some(&plan.project_path),
                    )
                    .await?;
                }
            }
        }
    }
    crate::util::fs::atomic_write(&project.join(LOCK_FILENAME), &serialize(&plan.check.lock)?)
        .await?;
    plan = plan_at(state, project, allow_merge).await?;
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
    plan_at(state, &canonical_project(project_path)?, false).await
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
    plan_at(state, &canonical_project(project_path)?, allow_merge).await
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
    fn stateless_verify_reports_only_filesystem_statuses_and_skips_user_scope() {
        let project = Path::new("/project");
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

        let mut user = entry(&"f".repeat(64));
        user.scope = "user".into();
        user.source_relative_path = "engineering/user.md".into();
        user.artifacts[0].path = ".codex/agents/user.md".into();

        let bytes = serde_json::to_vec(&AgencyLock {
            agency_lock: LOCK_VERSION,
            entries: vec![user, missing, modified, ok],
        })
        .unwrap();
        let result = verify_lockfile(&bytes, project, |path| match path {
            path if path == project.join("agents/ok.md") => Ok(b"ok\n".to_vec()),
            path if path == project.join("agents/modified.md") => Ok(b"changed\n".to_vec()),
            path if path == project.join("agents/missing.md") => {
                Err(std::io::Error::from(std::io::ErrorKind::NotFound))
            }
            path => panic!("user-scope artifact must not be read: {}", path.display()),
        })
        .unwrap();

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
                (
                    "engineering/user.md",
                    "user",
                    VerifyStatus::Skipped,
                    Some("user-scope"),
                ),
            ]
        );
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
}
