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
const LOCK_FILENAME: &str = "agency.lock.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgencyLock {
    agency_lock: u32,
    entries: Vec<LockEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LockEntry {
    artifacts: Vec<LockArtifact>,
    kind: String,
    scope: String,
    source: PortableSource,
    source_hash: String,
    source_relative_path: String,
    tool: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LockArtifact {
    content_hash: String,
    path: String,
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
    entry: LockEntry,
    status: LockEntryStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LockCheckResult {
    lock: AgencyLock,
    entries: Vec<LockCheckEntry>,
    clean: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LockOperation {
    kind: String,
    source_relative_path: String,
    tool: String,
    action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LockPlan {
    project_path: String,
    check: LockCheckResult,
    operations: Vec<LockOperation>,
    warnings: Vec<String>,
    blockers: Vec<String>,
    revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LockApplyResponse {
    plan: LockPlan,
    applied: bool,
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
    let mut bytes = serde_json::to_vec_pretty(&normalize(lock.clone())?).map_err(|error| {
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
    normalize(lock)
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

    for record in agent_records
        .iter()
        .filter(|record| record.project_path.as_deref() == Some(project_string.as_ref()))
    {
        let Ok(source) = portable_agent_source(record, &agent_sources) else {
            continue;
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
        let Ok(source) = portable_skill_source(record, &skill_sources) else {
            continue;
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
    normalize(AgencyLock {
        agency_lock: LOCK_VERSION,
        entries,
    })
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

fn classify(
    ledger: Option<&LockEntry>,
    disk: LockEntryStatus,
    expected: &LockEntry,
) -> LockEntryStatus {
    let Some(ledger) = ledger else {
        return if disk == LockEntryStatus::Missing {
            LockEntryStatus::Missing
        } else {
            LockEntryStatus::Foreign
        };
    };
    if ledger.source_hash != expected.source_hash || ledger.source != expected.source {
        return LockEntryStatus::Outdated;
    }
    disk
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
            let disk = artifact_state(project, &entry.artifacts);
            let status = classify(ledger, disk, &entry);
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

async fn plan_at(state: &AppState, project: &Path) -> Result<LockPlan, AppError> {
    let check = check_at(state, project).await?;
    let workspace = build_workspace_pack_plan(
        state,
        pack_from_lock(&check.lock),
        Some(project.to_string_lossy().into_owned()),
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
            }),
            LockEntryStatus::Outdated => operations.push(LockOperation {
                kind: checked.entry.kind.clone(),
                source_relative_path: checked.entry.source_relative_path.clone(),
                tool: checked.entry.tool.clone(),
                action: "update".into(),
            }),
            LockEntryStatus::Extra => warnings.push(format!(
                "Project has an unlocked {}: {}",
                checked.entry.kind, checked.entry.source_relative_path
            )),
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
        project_path: project.to_string_lossy().into_owned(),
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
    plan_at(&state, &project).await
}

#[tauri::command]
pub async fn lock_apply(
    app: AppHandle,
    state: State<'_, AppState>,
    project_path: String,
    revision: String,
) -> Result<LockApplyResponse, AppError> {
    let project = canonical_project(&project_path)?;
    let mut plan = plan_at(&state, &project).await?;
    if plan.revision != revision || !plan.blockers.is_empty() {
        return Ok(LockApplyResponse {
            plan,
            applied: false,
        });
    }
    let mut workspace = build_workspace_pack_plan(
        &state,
        pack_from_lock(&plan.check.lock),
        Some(plan.project_path.clone()),
    )
    .await?;
    materialize_workspace_pack_sources(&state, &workspace.source_additions).await?;
    workspace =
        build_workspace_pack_plan(&state, workspace.pack, Some(plan.project_path.clone())).await?;
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
            let record = super::do_install(
                &app,
                &state,
                item.reference.clone(),
                item.tool.clone(),
                Some(plan.project_path.clone()),
                true,
            )
            .await?;
            let expected = plan
                .check
                .lock
                .entries
                .iter()
                .find(|entry| {
                    entry.kind == "agent"
                        && entry.source_relative_path == operation.source_relative_path
                        && entry.tool == operation.tool
                })
                .ok_or_else(|| invalid("lockfile Agent entry disappeared"))?;
            if record.source_hash != expected.source_hash
                || installed_source_revision(&state, &item.reference, &record.source_hash).await?
                    != match &expected.source {
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
            skills::install_skill_with_dependencies(
                &state,
                &item.reference.source_id,
                &item.reference.relative_path,
                &item.runtime,
                Some(&plan.project_path),
            )
            .await?;
        }
    }
    crate::util::fs::atomic_write(&project.join(LOCK_FILENAME), &serialize(&plan.check.lock)?)
        .await?;
    plan = plan_at(&state, &project).await?;
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
    plan_at(state, &canonical_project(project_path)?).await
}

pub(crate) async fn mcp_lock_apply(
    state: &AppState,
    project_path: &str,
    revision: &str,
    authorization: Option<&crate::state::AuthorizedMcpProject>,
) -> Result<LockApplyResponse, AppError> {
    let project = canonical_project(project_path)?;
    let mut plan = plan_at(state, &project).await?;
    if plan.revision != revision || !plan.blockers.is_empty() {
        return Ok(LockApplyResponse {
            plan,
            applied: false,
        });
    }
    if plan
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
            super::mcp_install_agent_clean(
                state,
                item.reference.clone(),
                item.tool.clone(),
                Some(plan.project_path.clone()),
                authorization,
            )
            .await?;
        } else {
            let item = workspace
                .skills
                .iter()
                .find(|item| {
                    item.reference.relative_path == operation.source_relative_path
                        && item.runtime == operation.tool
                })
                .ok_or_else(|| invalid("lockfile Skill operation could not be resolved"))?;
            skills::install_skill_with_dependencies_authorized(
                state,
                &item.reference.source_id,
                &item.reference.relative_path,
                &item.runtime,
                Some(&plan.project_path),
                authorization,
            )
            .await?;
        }
    }
    crate::util::fs::atomic_write(&project.join(LOCK_FILENAME), &serialize(&plan.check.lock)?)
        .await?;
    plan = plan_at(state, &project).await?;
    Ok(LockApplyResponse {
        applied: plan.check.clean,
        plan,
    })
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
    fn lock_status_classification_covers_disk_and_ledger_drift() {
        let expected = entry(&"c".repeat(64));
        assert_eq!(
            classify(Some(&expected), LockEntryStatus::Current, &expected),
            LockEntryStatus::Current
        );
        assert_eq!(
            classify(None, LockEntryStatus::Missing, &expected),
            LockEntryStatus::Missing
        );
        assert_eq!(
            classify(None, LockEntryStatus::Modified, &expected),
            LockEntryStatus::Foreign
        );
        assert_eq!(
            classify(Some(&expected), LockEntryStatus::Modified, &expected),
            LockEntryStatus::Modified
        );
        assert_eq!(
            classify(
                Some(&entry(&"d".repeat(64))),
                LockEntryStatus::Current,
                &expected
            ),
            LockEntryStatus::Outdated
        );
    }
}
