//! Install + reconcile — the cross-tool agent state layer (contracts.md §C,
//! systemPatterns.md §2–5). This is the differentiator: the AI tools have no
//! install registry, so the app IS the database.
//!
//! - **ledger** (`installs.json`): every install action we performed.
//! - **reconcile**: diff ledger ↔ disk ↔ Agent sources into the seven states.
//! - **tools / projects**: detected tools and project-scoped install surfaces.
//!
//! Provenance is by hash-match only — we never mutate agent content. An
//! installed file is "ours/current" when its bytes equal a fresh render of its
//! slug for its tool (the deterministic `render/` layer makes that reproducible).

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::path::{Component, Path, PathBuf};

use chrono::Utc;
use tauri::{AppHandle, Runtime, State};
use tokio::io::AsyncWriteExt;

use crate::corpus;
use crate::error::AppError;
use crate::registry;
use crate::render;
use crate::state::{AppState, AuthorizedMcpProject};
use crate::types::{
    AgentApprovalAction, AgentArtifactDiff, AgentDiff, AgentInstallIdentity, AgentMutationPlan,
    AgentPlanItem, AgentReference, AgentRosterDestinationObservation, AgentRosterInstallRecord,
    AgentRosterMember, AgentRosterMutationPlan, AgentRosterPathObservation, AgentSourceResult,
    AgentVersionSnapshot, BaselineAgentRequirement, BaselineRequirement, BaselineSkillRequirement,
    CatalogChange, CatalogFeedBatch, InstallArtifact, InstallRecord, InstallState, InstalledAgent,
    InstalledAgentRoster, ProjectInfo, ProjectReadinessBaseline, ProjectReadinessOverall,
    ProjectReadinessReport, ProjectRecommendation, ProjectRecommendationTarget,
    ProjectSubscription, ReadinessCategoryKind, ReadinessCategoryReport, ReadinessCategoryState,
    ReadinessRow, ReadinessRowState, RecommendationChangeKind, RecommendationLifecycle,
    RecommendationOperation, SkillReference, Tool, ToolInfo, ToolVersion, UpdateKind,
};
use crate::util::fs::{atomic_write, read_capped};

mod history;

/// Cap on an installed agent file we read back during reconciliation.
const MAX_INSTALLED_BYTES: u64 = 4 * 1024 * 1024;
const MAX_REGISTERED_PROJECTS: usize = 200;
const MAX_PROJECT_REGISTRY_BYTES: u64 = 64 * 1024;
const MAX_LEDGER_BYTES: u64 = 16 * 1024 * 1024;
const MAX_AGENT_BATCH_ROOTS: usize = 64;
const MAX_ROSTER_RECORDS: usize = 128;
const MAX_ROSTER_MEMBERS: usize = 512;
pub(crate) const MAX_AGENT_HISTORY_ENTRIES: usize = 10;
const OPENCLAW_DEPLOYMENT_NOTICE: &str =
    "Workspace files installed; OpenClaw registration and restart remain required.";

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)] // ponytail: keep untagged legacy rows directly deserializable.
enum InstallLedgerEntry {
    Agent(InstallRecord),
    Roster(AgentRosterInstallRecord),
}

fn replace_agent_entries(
    entries: Vec<InstallLedgerEntry>,
    records: Vec<InstallRecord>,
) -> Vec<InstallLedgerEntry> {
    records
        .into_iter()
        .map(InstallLedgerEntry::Agent)
        .chain(
            entries
                .into_iter()
                .filter(|entry| matches!(entry, InstallLedgerEntry::Roster(_))),
        )
        .collect()
}

fn replace_roster_entries(
    entries: Vec<InstallLedgerEntry>,
    rosters: Vec<AgentRosterInstallRecord>,
) -> Vec<InstallLedgerEntry> {
    entries
        .into_iter()
        .filter(|entry| matches!(entry, InstallLedgerEntry::Agent(_)))
        .chain(rosters.into_iter().map(InstallLedgerEntry::Roster))
        .collect()
}

fn agent_entries(entries: Vec<InstallLedgerEntry>) -> Vec<InstallRecord> {
    entries
        .into_iter()
        .filter_map(|entry| match entry {
            InstallLedgerEntry::Agent(record) => Some(record),
            InstallLedgerEntry::Roster(_) => None,
        })
        .collect()
}

fn roster_entries(entries: Vec<InstallLedgerEntry>) -> Vec<AgentRosterInstallRecord> {
    entries
        .into_iter()
        .filter_map(|entry| match entry {
            InstallLedgerEntry::Agent(_) => None,
            InstallLedgerEntry::Roster(record) => Some(record),
        })
        .collect()
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_roster_record(record: &AgentRosterInstallRecord) -> Result<(), AppError> {
    if !matches!(record.tool.as_str(), "aider" | "windsurf")
        || record.scope != crate::types::Scope::Project
        || record.members.len() < 2
        || record.members.len() > MAX_ROSTER_MEMBERS
        || !valid_sha256(&record.rendered_hash)
        || record.installed_at.len() > 64
    {
        return Err(AppError::InvalidArgument {
            message: "Agent roster install record is invalid".into(),
        });
    }
    let mut sorted = record.members.clone();
    sorted.sort();
    sorted.dedup_by(|left, right| left.reference == right.reference);
    if sorted != record.members
        || record.members.iter().any(|member| {
            crate::library::validate_reference(
                &member.reference.source_id,
                &member.reference.relative_path,
            )
            .is_err()
                || member.name.is_empty()
                || member.name.len() > 256
                || !valid_sha256(&member.source_hash)
        })
    {
        return Err(AppError::InvalidArgument {
            message: "Agent roster membership is invalid".into(),
        });
    }
    let project = Path::new(&record.project_path);
    if !project.is_absolute() {
        return Err(AppError::InvalidArgument {
            message: "Agent roster project path must be absolute".into(),
        });
    }
    let expected = render::dests(&record.tool, "roster", Path::new("/"), Some(project))?;
    if expected.len() != 1 || expected[0] != Path::new(&record.dest) {
        return Err(AppError::InvalidArgument {
            message: "Agent roster destination does not match its tool registry".into(),
        });
    }
    let expected_disabled = disabled_destination(Path::new(&record.dest))?;
    if record
        .disabled_path
        .as_deref()
        .is_some_and(|path| Path::new(path) != expected_disabled)
    {
        return Err(AppError::InvalidArgument {
            message: "Agent roster disabled destination is invalid".into(),
        });
    }
    Ok(())
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentInstallOperation {
    previous: Option<InstallRecord>,
    next: InstallRecord,
    targets: Vec<String>,
    rendered: AgentRenderedPayload,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(untagged)]
enum AgentRenderedPayload {
    One(String),
    Many(Vec<String>),
}

impl AgentRenderedPayload {
    fn contents(&self) -> Vec<&str> {
        match self {
            Self::One(content) => vec![content],
            Self::Many(contents) => contents.iter().map(String::as_str).collect(),
        }
    }
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentMoveOperation {
    previous: InstallRecord,
    next: InstallRecord,
    active: Vec<String>,
    disabled: Vec<String>,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentUninstallOperation {
    previous: InstallRecord,
    paths: Vec<String>,
    hashes: Vec<Option<String>>,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentRosterHistoryReference {
    snapshot_id: String,
    rendered_hash: String,
    retired_snapshot_ids: Vec<String>,
    staging_path: String,
    previous_index_hash: Option<String>,
    next_index_hash: String,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(tag = "state", content = "snapshot", rename_all = "camelCase")]
enum AgentRosterPreservation {
    AbsentBeforeMutation,
    Snapshot(AgentRosterHistoryReference),
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentRosterWriteOperation {
    previous: Option<AgentRosterInstallRecord>,
    next: AgentRosterInstallRecord,
    staged_path: String,
    preservation: AgentRosterPreservation,
}

#[derive(Clone, Copy, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
enum UnjournaledRosterWritePhase {
    Prepared,
    FilesystemApplied,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UnjournaledRosterWriteIntent {
    phase: UnjournaledRosterWritePhase,
    previous: Option<AgentRosterInstallRecord>,
    next: AgentRosterInstallRecord,
    staged_path: String,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentRosterMoveOperation {
    previous: AgentRosterInstallRecord,
    next: AgentRosterInstallRecord,
    history: AgentRosterHistoryReference,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentRosterUninstallOperation {
    previous: AgentRosterInstallRecord,
    preservation: AgentRosterPreservation,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentRosterRetentionOperation {
    previous: AgentRosterInstallRecord,
    history: AgentRosterHistoryReference,
}

// ---------- Ledger persistence ----------

fn ledger_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, AppError> {
    let adir = corpus::app_data_dir(app)?;
    Ok(ledger_path_for(&adir))
}

fn ledger_path_for(app_data_dir: &Path) -> PathBuf {
    corpus::state_dir(app_data_dir).join("installs.json")
}

fn lock_agent_installs(app_data_dir: &Path) -> Result<File, AppError> {
    let directory = corpus::state_dir(app_data_dir);
    std::fs::create_dir_all(&directory).map_err(|error| AppError::Io {
        message: format!("create Agent install state directory: {error}"),
    })?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("installs.lock"))
        .map_err(|error| AppError::Io {
            message: format!("open Agent install state lock: {error}"),
        })?;
    file.lock().map_err(|error| AppError::Io {
        message: format!("lock Agent install state: {error}"),
    })?;
    Ok(file)
}

async fn lock_agent_installs_async(app_data_dir: PathBuf) -> Result<File, AppError> {
    tokio::task::spawn_blocking(move || lock_agent_installs(&app_data_dir))
        .await
        .map_err(|error| AppError::Internal {
            message: format!("Agent install lock task failed: {error}"),
        })?
}

pub(crate) async fn load_ledger<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
) -> Result<Vec<InstallRecord>, AppError> {
    corpus::ensure_corpus(app, state).await?;
    if let Some(database) = state.completed_state_database().await? {
        return database
            .read(installs_spec())
            .await?
            .ok_or_else(|| AppError::StorageCorrupt {
                message: "Agent install ledger is missing after SQLite migration".into(),
            })
            .map(agent_entries);
    }
    let built_in = crate::agents::inspect_builtin_agent_source(&state.app_data_dir).await?;
    let path = ledger_path(app)?;
    load_migrated_ledger_path(&path, Some(&built_in), &now_iso()).await
}

async fn load_ledger_for_state(state: &AppState) -> Result<Vec<InstallRecord>, AppError> {
    if let Some(database) = state.completed_state_database().await? {
        return database
            .read(installs_spec())
            .await?
            .ok_or_else(|| AppError::StorageCorrupt {
                message: "Agent install ledger is missing after SQLite migration".into(),
            })
            .map(agent_entries);
    }
    let built_in = crate::agents::inspect_builtin_agent_source(&state.app_data_dir)
        .await
        .ok();
    load_migrated_ledger_path(
        &ledger_path_for(&state.app_data_dir),
        built_in.as_ref(),
        &now_iso(),
    )
    .await
}

async fn load_install_entries_for_state(
    state: &AppState,
) -> Result<Vec<InstallLedgerEntry>, AppError> {
    if let Some(database) = state.completed_state_database().await? {
        return database
            .read(installs_spec())
            .await?
            .ok_or_else(|| AppError::StorageCorrupt {
                message: "Agent install ledger is missing after SQLite migration".into(),
            });
    }
    let path = ledger_path_for(&state.app_data_dir);
    match read_capped(&path, MAX_LEDGER_BYTES).await {
        Ok(bytes) => {
            let entries =
                serde_json::from_slice::<Vec<InstallLedgerEntry>>(&bytes).map_err(|error| {
                    AppError::Io {
                        message: format!("parse installs.json: {error}"),
                    }
                })?;
            validate_install_entries(&entries)?;
            Ok(entries)
        }
        Err(AppError::Io { .. }) if !path.exists() => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

async fn load_rosters_for_state(
    state: &AppState,
) -> Result<Vec<AgentRosterInstallRecord>, AppError> {
    Ok(roster_entries(load_install_entries_for_state(state).await?))
}

/// Passive ledger read for diagnostics. Unlike `load_ledger_for_state`, this
/// never migrates a legacy document or writes a backup.
pub(crate) async fn load_ledger_read_only(
    state: &AppState,
) -> Result<Vec<InstallRecord>, AppError> {
    if let Some(database) = state.completed_state_database().await? {
        return database
            .read(installs_spec())
            .await?
            .ok_or_else(|| AppError::StorageCorrupt {
                message: "Agent install ledger is missing after SQLite migration".into(),
            })
            .map(agent_entries);
    }
    load_ledger_read_only_at(&ledger_path_for(&state.app_data_dir)).await
}

async fn load_ledger_read_only_at(path: &Path) -> Result<Vec<InstallRecord>, AppError> {
    match crate::util::fs::read_capped(path, MAX_LEDGER_BYTES).await {
        Ok(bytes) => {
            let entries: Vec<InstallLedgerEntry> =
                serde_json::from_slice(&bytes).map_err(|error| AppError::JsonParse {
                    command: "doctor_report".into(),
                    message: error.to_string(),
                    raw_excerpt: String::new(),
                })?;
            validate_install_entries(&entries)?;
            Ok(agent_entries(entries))
        }
        Err(AppError::Io { .. }) if !path.exists() => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

async fn save_ledger<R: Runtime>(
    app: &AppHandle<R>,
    records: &[InstallRecord],
) -> Result<(), AppError> {
    save_ledger_for(&corpus::app_data_dir(app)?, records).await
}

async fn save_ledger_for(app_data_dir: &Path, records: &[InstallRecord]) -> Result<(), AppError> {
    validate_install_ledger(records)?;
    if let Some(database) = crate::state_db::StateDatabase::completed(app_data_dir).await? {
        let replacement = records.to_vec();
        return database
            .mutate(installs_spec(), Vec::new(), move |current| {
                *current = replace_agent_entries(std::mem::take(current), replacement);
                Ok(())
            })
            .await;
    }
    let path = ledger_path_for(app_data_dir);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::Io {
                message: format!("create state dir {}: {e}", parent.display()),
            })?;
    }
    let existing = match read_capped(&path, MAX_LEDGER_BYTES).await {
        Ok(bytes) => {
            serde_json::from_slice::<Vec<InstallLedgerEntry>>(&bytes).map_err(|error| {
                AppError::Io {
                    message: format!("parse installs.json before preserving roster rows: {error}"),
                }
            })?
        }
        Err(AppError::Io { .. }) if !path.exists() => Vec::new(),
        Err(error) => return Err(error),
    };
    let entries = replace_agent_entries(existing, records.to_vec());
    validate_install_entries(&entries)?;
    let bytes = serde_json::to_vec_pretty(&entries).map_err(|e| AppError::Io {
        message: format!("serialize installs.json: {e}"),
    })?;
    atomic_write(&path, &bytes).await
}

async fn save_ledger_after_filesystem(
    state: &AppState,
    records: &[InstallRecord],
    operation_id: &str,
) -> Result<(), AppError> {
    validate_install_ledger(records)?;
    let database =
        state
            .completed_state_database()
            .await?
            .ok_or_else(|| AppError::StorageCorrupt {
                message: "Agent install operation lost its SQLite database".into(),
            })?;
    let replacement = records.to_vec();
    database
        .mutate_after_filesystem(installs_spec(), Vec::new(), operation_id, move |current| {
            *current = replace_agent_entries(std::mem::take(current), replacement);
            Ok(())
        })
        .await
}

async fn save_rosters_for_state(
    state: &AppState,
    rosters: &[AgentRosterInstallRecord],
) -> Result<(), AppError> {
    let entries = rosters
        .iter()
        .cloned()
        .map(InstallLedgerEntry::Roster)
        .collect::<Vec<_>>();
    validate_install_entries(&entries)?;
    if let Some(database) = state.completed_state_database().await? {
        let replacement = rosters.to_vec();
        return database
            .mutate(installs_spec(), Vec::new(), move |current| {
                *current = replace_roster_entries(std::mem::take(current), replacement);
                Ok(())
            })
            .await;
    }
    let path = ledger_path_for(&state.app_data_dir);
    let current = load_install_entries_for_state(state).await?;
    let entries = replace_roster_entries(current, rosters.to_vec());
    validate_install_entries(&entries)?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| AppError::Io {
                message: format!("create Agent roster state directory: {error}"),
            })?;
    }
    let bytes = serde_json::to_vec_pretty(&entries).map_err(|error| AppError::Internal {
        message: format!("serialize Agent roster ledger: {error}"),
    })?;
    atomic_write(&path, &bytes).await
}

async fn save_rosters_after_filesystem(
    state: &AppState,
    rosters: &[AgentRosterInstallRecord],
    operation_id: &str,
) -> Result<(), AppError> {
    let database =
        state
            .completed_state_database()
            .await?
            .ok_or_else(|| AppError::StorageCorrupt {
                message: "Agent roster operation lost its SQLite database".into(),
            })?;
    let replacement = rosters.to_vec();
    database
        .mutate_after_filesystem(installs_spec(), Vec::new(), operation_id, move |current| {
            *current = replace_roster_entries(std::mem::take(current), replacement);
            Ok(())
        })
        .await
}

fn validate_install_ledger(records: &[InstallRecord]) -> Result<(), AppError> {
    let (_, changed) = migrate_install_records(records.to_vec(), None)?;
    if changed {
        return Err(AppError::InvalidArgument {
            message: "Agent install ledger requires migration".into(),
        });
    }
    Ok(())
}

fn validate_install_entries(entries: &[InstallLedgerEntry]) -> Result<(), AppError> {
    let agents = entries
        .iter()
        .filter_map(|entry| match entry {
            InstallLedgerEntry::Agent(record) => Some(record.clone()),
            InstallLedgerEntry::Roster(_) => None,
        })
        .collect::<Vec<_>>();
    validate_install_ledger(&agents)?;
    let rosters = entries
        .iter()
        .filter_map(|entry| match entry {
            InstallLedgerEntry::Agent(_) => None,
            InstallLedgerEntry::Roster(record) => Some(record),
        })
        .collect::<Vec<_>>();
    if rosters.len() > MAX_ROSTER_RECORDS {
        return Err(AppError::InvalidArgument {
            message: format!("at most {MAX_ROSTER_RECORDS} Agent roster records are allowed"),
        });
    }
    let mut identities = BTreeSet::new();
    for record in rosters {
        validate_roster_record(record)?;
        if !identities.insert((record.tool.as_str(), record.project_path.as_str())) {
            return Err(AppError::InvalidArgument {
                message: "duplicate Agent roster install identity".into(),
            });
        }
    }
    Ok(())
}

fn installs_spec() -> crate::state_db::DocumentSpec<Vec<InstallLedgerEntry>> {
    crate::state_db::DocumentSpec::new("installs", 1, MAX_LEDGER_BYTES, |entries| {
        validate_install_entries(entries)
    })
}

pub(crate) fn installs_import_spec() -> crate::state_db::ImportSpec {
    crate::state_db::ImportSpec::document(installs_spec(), Vec::new())
}

/// Upgrade pre-source ledgers entirely in memory. Existing source-aware rows
/// must already be valid; legacy rows resolve to the built-in source only when
/// the slug has one exact match.
fn migrate_install_records(
    mut records: Vec<InstallRecord>,
    built_in: Option<&AgentSourceResult>,
) -> Result<(Vec<InstallRecord>, bool), AppError> {
    let mut changed = false;
    for record in &mut records {
        match (record.source_id.is_empty(), record.relative_path.is_empty()) {
            (true, true) => {
                let built_in = built_in.ok_or_else(|| AppError::InvalidArgument {
                    message:
                        "legacy Agent install records require the built-in source for migration"
                            .into(),
                })?;
                let matches = built_in
                    .agents
                    .iter()
                    .filter(|package| {
                        package
                            .agent
                            .as_ref()
                            .is_some_and(|agent| agent.slug == record.slug)
                    })
                    .collect::<Vec<_>>();
                if matches.len() == 1 {
                    record.source_id = crate::agents::BUILTIN_AGENT_SOURCE_ID.into();
                    record.relative_path = matches[0].reference.relative_path.clone();
                } else {
                    record.source_id = "legacy:unresolved".into();
                    record.relative_path = unresolved_relative_path(record);
                }
                changed = true;
            }
            (false, false) => {
                crate::library::validate_reference(&record.source_id, &record.relative_path)?;
            }
            _ => {
                return Err(AppError::InvalidArgument {
                    message: format!(
                        "install record for {} has an incomplete source identity",
                        record.slug
                    ),
                });
            }
        }
        if record.source_snapshot_hash.is_empty() {
            record.source_snapshot_hash = record.source_hash.clone();
            changed = true;
        }
        if !record.artifacts.is_empty() {
            let valid_hash =
                |hash: &str| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit());
            let destinations = record
                .artifacts
                .iter()
                .map(|artifact| artifact.dest.as_str())
                .collect::<BTreeSet<_>>();
            if record.artifacts.len() > 8
                || destinations.len() != record.artifacts.len()
                || record.artifacts[0].dest != record.dest
                || record.artifacts[0].rendered_hash != record.rendered_hash
                || record.artifacts[0].disabled_path != record.disabled_path
                || record
                    .artifacts
                    .iter()
                    .any(|artifact| !valid_hash(&artifact.rendered_hash))
            {
                return Err(AppError::InvalidArgument {
                    message: format!("invalid Agent artifact manifest for {}", record.slug),
                });
            }
        }
    }
    Ok((records, changed))
}

fn unresolved_relative_path(record: &InstallRecord) -> String {
    let leaf = record.dest.rsplit(['/', '\\']).next().unwrap_or_default();
    let filename = leaf.rsplit_once('.').map_or(leaf, |(stem, _)| stem);
    let filename = if filename.is_empty() {
        &record.slug
    } else {
        filename
    };
    let stem = filename
        .chars()
        .map(|value| {
            if value.is_ascii_alphanumeric() || matches!(value, '-' | '_') {
                value.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_owned();
    let key = format!(
        "{}\0{}\0{:?}\0{}\0{}",
        record.slug,
        record.tool,
        record.scope,
        record.project_path.as_deref().unwrap_or_default(),
        record.dest
    );
    let hash = render::sha256_hex(key.as_bytes());
    format!(
        "legacy/{}-{}.md",
        if stem.is_empty() { "agent" } else { &stem },
        &hash[..12]
    )
}

async fn load_migrated_ledger_path(
    path: &Path,
    built_in: Option<&AgentSourceResult>,
    stamp: &str,
) -> Result<Vec<InstallRecord>, AppError> {
    let original = match read_capped(path, MAX_LEDGER_BYTES).await {
        Ok(bytes) => bytes,
        Err(AppError::Io { .. }) if !path.exists() => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let entries: Vec<InstallLedgerEntry> =
        serde_json::from_slice(&original).map_err(|error| AppError::Io {
            message: format!("parse installs.json: {error}"),
        })?;
    let rosters = roster_entries(entries.clone());
    let (records, changed) = migrate_install_records(agent_entries(entries), built_in)?;
    if !changed {
        return Ok(records);
    }

    let parent = path.parent().ok_or_else(|| AppError::Io {
        message: format!("install ledger has no parent: {}", path.display()),
    })?;
    let backup = parent.join(format!("installs.migration-{}.json.bak", fs_stamp(stamp)));
    atomic_write(&backup, &original).await?;
    let migrated = serde_json::to_vec_pretty(&replace_roster_entries(
        records
            .clone()
            .into_iter()
            .map(InstallLedgerEntry::Agent)
            .collect(),
        rosters,
    ))
    .map_err(|error| AppError::Io {
        message: format!("serialize migrated installs.json: {error}"),
    })?;
    atomic_write(path, &migrated).await?;
    Ok(records)
}

async fn prune_migration_backups<R: Runtime>(app: &AppHandle<R>) -> Result<(), AppError> {
    let path = ledger_path(app)?;
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    let mut entries = match tokio::fs::read_dir(parent).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(AppError::Io {
                message: format!("read install migration backups: {error}"),
            });
        }
    };
    let mut backups = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(|error| AppError::Io {
        message: format!("scan install migration backups: {error}"),
    })? {
        if entry.file_name().to_str().is_some_and(|name| {
            name.starts_with("installs.migration-") && name.ends_with(".json.bak")
        }) {
            backups.push(entry.path());
        }
    }
    backups.sort();
    let remove_count = backups.len().saturating_sub(MAX_AGENT_HISTORY_ENTRIES);
    for backup in backups.into_iter().take(remove_count) {
        tokio::fs::remove_file(&backup)
            .await
            .map_err(|error| AppError::Io {
                message: format!(
                    "prune install migration backup {}: {error}",
                    backup.display()
                ),
            })?;
    }
    Ok(())
}

fn home() -> Result<PathBuf, AppError> {
    dirs::home_dir().ok_or_else(|| AppError::Io {
        message: "cannot resolve home directory".into(),
    })
}

/// User-scope base directory for a tool's installs **and** detection: the
/// per-tool custom path the user configured (e.g. a WSL home) if any, else the
/// OS home. Project-scope installs ignore this — they resolve against the
/// chosen project root. Because the ledger stores the resolved `dest`, reconcile
/// stays correct with no per-tool logic of its own.
pub(crate) async fn tool_home(state: &AppState, tool: &str) -> Result<PathBuf, AppError> {
    let os_home = home()?;
    let base = state
        .settings
        .read()
        .await
        .effective_settings()
        .map(|s| resolve_tool_base(&s.tool_paths, tool, &os_home))
        .unwrap_or(os_home);
    Ok(base)
}

/// Pure per-tool base resolution: a configured, non-empty custom path wins;
/// otherwise the OS home. Split out from [`tool_home`] so it's unit-testable
/// without standing up an `AppState`.
fn resolve_tool_base(
    tool_paths: &std::collections::HashMap<String, String>,
    tool: &str,
    os_home: &Path,
) -> PathBuf {
    tool_paths
        .get(tool)
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| os_home.to_path_buf())
}

fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

/// Where overwritten files are preserved before a write replaces them. Lives
/// under app data, NOT inside any tool's agent dir — so the Foreign sweep never
/// mistakes a backup for an installed agent. Every destructive write copies the
/// prior bytes here first, making install/update/restore reversible.
fn backups_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, AppError> {
    Ok(backups_dir_for(&corpus::app_data_dir(app)?))
}

fn backups_dir_for(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("backups")
}

/// Filesystem-safe variant of an RFC3339 timestamp (no colons).
fn fs_stamp(iso: &str) -> String {
    iso.replace([':', '/'], "-")
}

/// Build the ledger record for a render. Shared by the write path
/// (`write_agent_files`) and the no-write Track path so both agree on what a
/// row looks like.
#[allow(clippy::too_many_arguments)]
fn record_for(
    agent: &crate::types::Agent,
    primary_dest: &Path,
    tool: &str,
    project_root: Option<&Path>,
    rendered_hash: String,
    source_hash: &str,
    body_hash: &str,
    corpus_version: &str,
    installed_at: &str,
) -> InstallRecord {
    InstallRecord {
        slug: agent.slug.clone(),
        source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
        relative_path: format!("{}/{}.md", agent.category, agent.slug),
        tool: tool.to_string(),
        scope: render::scope_for(project_root),
        project_path: project_root.map(|p| p.to_string_lossy().to_string()),
        dest: primary_dest.to_string_lossy().to_string(),
        source_hash: source_hash.to_string(),
        body_hash: body_hash.to_string(),
        rendered_hash,
        artifacts: Vec::new(),
        disabled_path: None,
        source_snapshot_hash: source_hash.to_string(),
        capabilities: Vec::new(),
        publisher_key: None,
        publisher_verified: false,
        installed_at: installed_at.to_string(),
        corpus_version: corpus_version.to_string(),
        deployment_notice: (tool == "openclaw").then(|| OPENCLAW_DEPLOYMENT_NOTICE.into()),
    }
}

fn artifact_manifest(
    paths: &[PathBuf],
    rendered: &[render::RenderedArtifact],
) -> Result<Vec<InstallArtifact>, AppError> {
    if paths.is_empty() || paths.len() != rendered.len() || paths.len() > 8 {
        return Err(AppError::InvalidArgument {
            message: "Agent artifact destinations do not match rendered artifacts".into(),
        });
    }
    Ok(paths
        .iter()
        .zip(rendered)
        .map(|(path, artifact)| InstallArtifact {
            dest: path.to_string_lossy().into_owned(),
            rendered_hash: render::sha256_hex(artifact.content.as_bytes()),
            disabled_path: None,
        })
        .collect())
}

fn record_artifacts(record: &InstallRecord) -> Vec<InstallArtifact> {
    if record.artifacts.is_empty() {
        vec![InstallArtifact {
            dest: record.dest.clone(),
            rendered_hash: record.rendered_hash.clone(),
            disabled_path: record.disabled_path.clone(),
        }]
    } else {
        record.artifacts.clone()
    }
}

/// Copy `dest`'s current bytes into `backup_dir` before it's overwritten, but
/// only if it exists AND differs from the incoming bytes (no-op writes leave no
/// litter). Backup name keeps the original filename + a timestamp so it's
/// human-recoverable. Best-effort within a still-fallible signature: a failed
/// backup aborts the write (we never overwrite what we couldn't preserve).
async fn backup_if_differs(
    dest: &Path,
    new_bytes: &[u8],
    backup_dir: &Path,
    stamp: &str,
) -> Result<(), AppError> {
    let existing = match tokio::fs::read(dest).await {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(AppError::Io {
                message: format!("read existing file {} before backup: {e}", dest.display()),
            })
        }
    };
    if existing == new_bytes {
        return Ok(()); // identical → not a destructive write
    }
    tokio::fs::create_dir_all(backup_dir)
        .await
        .map_err(|e| AppError::Io {
            message: format!("create backups dir {}: {e}", backup_dir.display()),
        })?;
    let fname = dest
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "agent".into());
    let backup = backup_dir.join(format!("{fname}.{}.bak", fs_stamp(stamp)));
    atomic_write(&backup, &existing).await
}

// ---------- Install / update (shared core) ----------

pub(crate) async fn do_install<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    reference: AgentReference,
    tool: Tool,
    project_path: Option<String>,
    confirmed: bool,
) -> Result<InstallRecord, AppError> {
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
    recover_agent_operations_locked(state).await?;
    do_install_locked(app, state, reference, tool, project_path, confirmed).await
}

pub(crate) async fn do_install_legacy(
    app: &AppHandle,
    state: &AppState,
    slug: String,
    tool: Tool,
    project_path: Option<String>,
) -> Result<InstallRecord, AppError> {
    let reference = resolve_command_reference(app, state, None, None, Some(&slug)).await?;
    do_install(app, state, reference, tool, project_path, false).await
}

async fn do_install_locked<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    reference: AgentReference,
    tool: Tool,
    project_path: Option<String>,
    confirmed: bool,
) -> Result<InstallRecord, AppError> {
    corpus::ensure_corpus(app, state).await?;
    let package = crate::agents::resolve_agent_package(&state.app_data_dir, &reference).await?;
    if !package.installable {
        return Err(AppError::InvalidArgument {
            message: format!(
                "Agent package is not installable: {}:{}",
                reference.source_id, reference.relative_path
            ),
        });
    }
    let raw = crate::agents::read_agent_text(&state.app_data_dir, &reference).await?;
    if render::sha256_hex(raw.as_bytes()) != package.source_hash {
        return Err(AppError::InvalidArgument {
            message: "Agent source changed after inspection".into(),
        });
    }
    let agent = package
        .agent
        .clone()
        .ok_or_else(|| AppError::InvalidArgument {
            message: "Agent package has no valid metadata".into(),
        })?;
    let revision = package
        .version
        .clone()
        .unwrap_or_else(|| package.source_hash.clone());

    let home = tool_home(state, &tool).await?;
    let proot = project_path.as_ref().map(PathBuf::from);
    let backups = backups_dir(app)?;
    let mut ledger = load_ledger(app, state).await?;
    let existing_record = ledger
        .iter()
        .find(|record| {
            record.source_id == reference.source_id
                && record.relative_path == reference.relative_path
                && record.tool == tool
                && record.project_path == project_path
        })
        .cloned();
    if let Some(existing) = &existing_record {
        let library = crate::agents::organize::list(state).await?;
        let policy = library
            .update_policies
            .iter()
            .find(|entry| entry.agent == reference)
            .map(|entry| entry.policy)
            .unwrap_or(crate::types::AgentUpdatePolicy::Notify);
        let publisher_trusted =
            package.publisher_verified && agent_publisher_is_trusted(&library, &package);
        if !update_policy_allows(
            policy,
            confirmed,
            &existing.capabilities,
            &package.capabilities,
            existing.publisher_key.as_deref(),
            package.publisher_key.as_deref(),
            publisher_trusted,
        ) {
            return Err(AppError::InvalidArgument {
                message: format!(
                    "Agent update requires review or is blocked by {:?} policy",
                    policy
                ),
            });
        }
    }
    let targets = match existing_record.as_ref() {
        Some(record) => resolved_record_paths(state, record).await?,
        None => install_target_paths(&agent, Some(&raw), &tool, &home, proot.as_deref(), None)?,
    };
    ensure_destinations_available(
        &ledger,
        &reference,
        &tool,
        project_path.as_deref(),
        &targets,
        false,
    )?;
    let prior_paths = targets
        .iter()
        .filter(|path| path.exists())
        .cloned()
        .collect::<Vec<_>>();
    let absent_paths = targets
        .iter()
        .filter(|path| !path.exists())
        .cloned()
        .collect::<Vec<_>>();
    let installed_at = now_iso();
    let rendered = render::render_artifacts(&agent, &raw, &tool)?;
    let manifest = artifact_manifest(&targets, &rendered)?;
    let mut planned_record = record_for(
        &agent,
        &targets[0],
        &tool,
        proot.as_deref(),
        manifest[0].rendered_hash.clone(),
        &package.source_hash,
        &package.body_hash,
        &revision,
        &installed_at,
    );
    planned_record.artifacts = manifest;
    planned_record.source_id = reference.source_id.clone();
    planned_record.relative_path = reference.relative_path.clone();
    planned_record.source_snapshot_hash = package.source_hash.clone();
    planned_record.capabilities = package.capabilities.clone();
    planned_record.publisher_key = package.publisher_key.clone();
    planned_record.publisher_verified = package.publisher_verified;
    let database = state.completed_state_database().await?;
    let operation = if let Some(database) = &database {
        Some(
            database
                .prepare_filesystem_operation(
                    if existing_record.is_some() {
                        "agent_update"
                    } else {
                        "agent_install"
                    },
                    &AgentInstallOperation {
                        previous: existing_record.clone(),
                        next: planned_record.clone(),
                        targets: targets
                            .iter()
                            .map(|path| path.to_string_lossy().into_owned())
                            .collect(),
                        rendered: if rendered.len() == 1 {
                            AgentRenderedPayload::One(rendered[0].content.clone())
                        } else {
                            AgentRenderedPayload::Many(
                                rendered.iter().map(|item| item.content.clone()).collect(),
                            )
                        },
                    },
                )
                .await?,
        )
    } else {
        None
    };
    let prior_snapshot = match (existing_record.as_ref(), prior_paths.is_empty()) {
        (Some(record), false) => Some(snapshot_record(state, record, &prior_paths).await?),
        _ => None,
    };
    let write_result = write_agent_files_at(
        &agent,
        &raw,
        &tool,
        proot.as_deref(),
        Some(&backups),
        &package.source_hash,
        &package.body_hash,
        &revision,
        &installed_at,
        &targets,
    )
    .await;
    let mut record = match write_result {
        Ok(record) => record,
        Err(error) => {
            let restore = restore_install_transaction(
                state,
                existing_record.as_ref(),
                prior_snapshot.as_ref(),
                &prior_paths,
                &absent_paths,
            )
            .await;
            return match restore {
                Ok(()) => {
                    if let (Some(database), Some(operation)) = (&database, &operation) {
                        database.abort_filesystem_operation(&operation.id).await?;
                    }
                    Err(error)
                }
                Err(rollback) => Err(rollback_error("install Agent", error, rollback)),
            };
        }
    };
    record.source_id = reference.source_id.clone();
    record.relative_path = reference.relative_path.clone();
    record.source_snapshot_hash = package.source_hash;
    record.capabilities = package.capabilities;
    record.publisher_key = package.publisher_key;
    record.publisher_verified = package.publisher_verified;

    ledger.retain(|existing| {
        !(existing.source_id == reference.source_id
            && existing.relative_path == reference.relative_path
            && existing.tool == tool
            && existing.project_path == project_path)
    });
    ledger.push(record.clone());
    let save = match &operation {
        Some(operation) => save_ledger_after_filesystem(state, &ledger, &operation.id).await,
        None => save_ledger(app, &ledger).await,
    };
    if let Err(error) = save {
        if operation.is_some() {
            return Err(error);
        }
        return match restore_install_transaction(
            state,
            existing_record.as_ref(),
            prior_snapshot.as_ref(),
            &prior_paths,
            &absent_paths,
        )
        .await
        {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error("save Agent install", error, rollback)),
        };
    }
    if let (Some(database), Some(operation)) = (&database, &operation) {
        database.commit_filesystem_operation(&operation.id).await?;
    }
    Ok(record)
}

async fn restore_install_transaction(
    state: &AppState,
    prior_record: Option<&InstallRecord>,
    prior_snapshot: Option<&AgentVersionSnapshot>,
    prior_paths: &[PathBuf],
    absent_paths: &[PathBuf],
) -> Result<(), AppError> {
    for path in absent_paths {
        match tokio::fs::remove_file(path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(AppError::Io {
                    message: format!("remove failed Agent install {}: {error}", path.display()),
                })
            }
        }
    }
    match (prior_record, prior_snapshot) {
        (Some(record), Some(snapshot)) => history::restore_snapshot(
            &state.app_data_dir,
            &install_identity(record),
            &snapshot.id,
            prior_paths,
        )
        .await
        .map(|_| ()),
        (_, None) if prior_paths.is_empty() => Ok(()),
        _ => Err(AppError::Internal {
            message: "Agent install rollback has content without a verified snapshot".into(),
        }),
    }
}

#[derive(Clone, PartialEq, Eq)]
struct BatchFileSnapshot {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
}

async fn capture_batch_files(paths: &[PathBuf]) -> Result<Vec<BatchFileSnapshot>, AppError> {
    let mut unique = paths.to_vec();
    unique.sort();
    unique.dedup();
    let mut snapshots = Vec::with_capacity(unique.len());
    for path in unique {
        let bytes = match std::fs::symlink_metadata(&path) {
            Ok(metadata)
                if metadata.is_file()
                    && !metadata.file_type().is_symlink()
                    && !crate::skills::metadata_is_reparse_point(&metadata) =>
            {
                Some(read_capped(&path, MAX_INSTALLED_BYTES).await?)
            }
            Ok(_) => {
                return Err(AppError::InvalidArgument {
                    message: format!(
                        "Agent batch destination is not a regular file: {}",
                        path.display()
                    ),
                })
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(AppError::Io {
                    message: format!(
                        "inspect Agent batch destination {}: {error}",
                        path.display()
                    ),
                })
            }
        };
        snapshots.push(BatchFileSnapshot { path, bytes });
    }
    Ok(snapshots)
}

async fn restore_batch_files(snapshots: &[BatchFileSnapshot]) -> Result<(), AppError> {
    let mut first_error = None;
    for snapshot in snapshots.iter().rev() {
        let result = match &snapshot.bytes {
            Some(bytes) => {
                if let Some(parent) = snapshot.path.parent() {
                    if let Err(error) = tokio::fs::create_dir_all(parent).await {
                        Err(AppError::Io {
                            message: format!(
                                "restore Agent batch directory {}: {error}",
                                parent.display()
                            ),
                        })
                    } else {
                        atomic_write(&snapshot.path, bytes).await
                    }
                } else {
                    Err(AppError::InvalidArgument {
                        message: "Agent batch destination has no parent".into(),
                    })
                }
            }
            None => match tokio::fs::remove_file(&snapshot.path).await {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(AppError::Io {
                    message: format!(
                        "remove created Agent batch file {}: {error}",
                        snapshot.path.display()
                    ),
                }),
            },
        };
        if result.is_err() && first_error.is_none() {
            first_error = result.err();
        }
    }
    first_error.map_or(Ok(()), Err)
}

async fn verify_batch_files(snapshots: &[BatchFileSnapshot]) -> Result<(), AppError> {
    let current = capture_batch_files(
        &snapshots
            .iter()
            .map(|snapshot| snapshot.path.clone())
            .collect::<Vec<_>>(),
    )
    .await?;
    if current == snapshots {
        Ok(())
    } else {
        Err(AppError::StorageCorrupt {
            message: "Agent roster rollback did not restore exact destination bytes".into(),
        })
    }
}

async fn restore_batch_files_verified(snapshots: &[BatchFileSnapshot]) -> Result<(), AppError> {
    if verify_batch_files(snapshots).await.is_ok() {
        return Ok(());
    }
    restore_batch_files(snapshots).await?;
    verify_batch_files(snapshots).await
}

async fn close_prepared_roster_failure(
    database: Option<&crate::state_db::StateDatabase>,
    operation: Option<&crate::state_db::FilesystemOperation>,
    restoration: &Result<(), AppError>,
) -> Result<(), AppError> {
    let (Some(database), Some(operation)) = (database, operation) else {
        return Ok(());
    };
    match restoration {
        Ok(()) => {
            if let Err(error) = database.abort_filesystem_operation(&operation.id).await {
                let _ = database
                    .retain_filesystem_operation_error(&operation.id, &error.to_string())
                    .await;
                return Err(error);
            }
        }
        Err(error) => {
            database
                .retain_filesystem_operation_error(&operation.id, &error.to_string())
                .await?;
        }
    }
    Ok(())
}

async fn rollback_roster_history(
    mutation: &mut Option<history::RosterHistoryMutation>,
) -> Result<(), AppError> {
    match mutation.take() {
        Some(mutation) => mutation.rollback().await,
        None => Ok(()),
    }
}

async fn rollback_unjournaled_roster_history(
    mutation: &mut Option<history::RosterHistoryMutation>,
    action: &str,
    error: AppError,
) -> AppError {
    match rollback_roster_history(mutation).await {
        Ok(()) => error,
        Err(rollback) => rollback_error(action, error, rollback),
    }
}

async fn commit_roster_history(
    mutation: &mut Option<history::RosterHistoryMutation>,
) -> Result<(), AppError> {
    if let Some(mutation) = mutation.take() {
        mutation.commit().await?;
    }
    Ok(())
}

async fn publish_roster_history(
    mutation: &mut Option<history::RosterHistoryMutation>,
) -> Result<(), AppError> {
    if let Some(mutation) = mutation.as_mut() {
        mutation.publish().await?;
    }
    Ok(())
}

async fn publish_roster_history_if(
    mutation: &mut Option<history::RosterHistoryMutation>,
    should_publish: bool,
) -> Result<(), AppError> {
    if should_publish {
        publish_roster_history(mutation).await?;
    }
    Ok(())
}

async fn publish_unjournaled_roster_history(
    mutation: &mut Option<history::RosterHistoryMutation>,
) -> Result<(), AppError> {
    if let Some(mutation) = mutation.as_mut() {
        mutation.mark_unjournaled_filesystem_applied().await?;
        mutation.publish().await?;
    }
    Ok(())
}

async fn prepare_unjournaled_roster_operation(
    mutation: &mut Option<history::RosterHistoryMutation>,
    should_prepare: bool,
    previous: Option<&AgentRosterInstallRecord>,
    next: Option<&AgentRosterInstallRecord>,
) -> Result<(), AppError> {
    if should_prepare {
        if let (Some(mutation), Some(previous)) = (mutation.as_mut(), previous) {
            mutation
                .prepare_unjournaled_operation(previous, next)
                .await?;
        }
    }
    Ok(())
}

async fn prepare_roster_retention(
    database: Option<&crate::state_db::StateDatabase>,
    previous: &AgentRosterInstallRecord,
    mutation: Option<&history::RosterHistoryMutation>,
) -> Result<Option<crate::state_db::FilesystemOperation>, AppError> {
    let (Some(database), Some(mutation)) = (database, mutation) else {
        return Ok(None);
    };
    database
        .prepare_filesystem_operation(
            "agent_roster_retention",
            &AgentRosterRetentionOperation {
                previous: previous.clone(),
                history: roster_history_reference(mutation),
            },
        )
        .await
        .map(Some)
}

async fn finalize_roster_history(
    database: Option<&crate::state_db::StateDatabase>,
    retention: Option<&crate::state_db::FilesystemOperation>,
    mutation: &mut Option<history::RosterHistoryMutation>,
) -> Result<(), AppError> {
    commit_roster_history(mutation).await?;
    let (Some(database), Some(retention)) = (database, retention) else {
        return Ok(());
    };
    database.mark_filesystem_applied(&retention.id).await?;
    database.commit_filesystem_operation(&retention.id).await
}

async fn abort_roster_retention(
    database: Option<&crate::state_db::StateDatabase>,
    retention: Option<&crate::state_db::FilesystemOperation>,
) -> Result<(), AppError> {
    let (Some(database), Some(retention)) = (database, retention) else {
        return Ok(());
    };
    database.abort_filesystem_operation(&retention.id).await
}

fn roster_history_reference(
    mutation: &history::RosterHistoryMutation,
) -> AgentRosterHistoryReference {
    AgentRosterHistoryReference {
        snapshot_id: mutation.snapshot.id.clone(),
        rendered_hash: mutation.snapshot.rendered_hash.clone(),
        retired_snapshot_ids: mutation.retired_snapshot_ids(),
        staging_path: mutation.staging_path(),
        previous_index_hash: mutation.previous_index_hash(),
        next_index_hash: mutation.next_index_hash(),
    }
}

fn roster_preservation(
    mutation: Option<&history::RosterHistoryMutation>,
) -> AgentRosterPreservation {
    match mutation {
        Some(mutation) => AgentRosterPreservation::Snapshot(roster_history_reference(mutation)),
        None => AgentRosterPreservation::AbsentBeforeMutation,
    }
}

fn roster_preservation_hash(preservation: &AgentRosterPreservation) -> Option<&str> {
    match preservation {
        AgentRosterPreservation::AbsentBeforeMutation => None,
        AgentRosterPreservation::Snapshot(reference) => Some(&reference.rendered_hash),
    }
}

async fn begin_roster_preservation(
    state: &AppState,
    path: &Path,
    previous: &AgentRosterInstallRecord,
    protected_snapshot_id: Option<&str>,
) -> Result<Option<history::RosterHistoryMutation>, AppError> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(AppError::Io {
                message: format!("inspect Agent roster preservation input: {error}"),
            });
        }
        Ok(_) => {}
    }
    let bytes = read_capped(path, MAX_INSTALLED_BYTES).await?;
    let mut preserved = previous.clone();
    preserved.rendered_hash = render::sha256_hex(&bytes);
    let destination = path.to_path_buf();
    history::begin_roster_snapshot(
        &state.app_data_dir,
        &roster_identity(previous),
        std::slice::from_ref(&destination),
        &preserved,
        &now_iso(),
        protected_snapshot_id,
    )
    .await
    .map(Some)
}

async fn validate_recovery_roster_history(
    state: &AppState,
    previous: Option<&AgentRosterInstallRecord>,
    destination: &Path,
    preservation: &AgentRosterPreservation,
) -> Result<(), AppError> {
    let (previous, reference) = match (previous, preservation) {
        (_, AgentRosterPreservation::AbsentBeforeMutation) => return Ok(()),
        (Some(previous), AgentRosterPreservation::Snapshot(reference)) => (previous, reference),
        (None, AgentRosterPreservation::Snapshot(_)) => {
            return Err(AppError::StorageCorrupt {
                message: "Agent roster recovery preservation metadata is inconsistent".into(),
            });
        }
    };
    let prior_hash = reference.rendered_hash.as_str();
    if !valid_sha256(prior_hash) {
        return Err(AppError::StorageCorrupt {
            message: "Agent roster recovery preservation hash is invalid".into(),
        });
    }
    let destination = destination.to_path_buf();
    let mut expected = previous.clone();
    expected.rendered_hash = prior_hash.into();
    history::recover_roster_publication(
        &state.app_data_dir,
        &roster_identity(previous),
        &reference.snapshot_id,
        &reference.retired_snapshot_ids,
        &reference.staging_path,
        reference.previous_index_hash.as_deref(),
        &reference.next_index_hash,
        &expected,
    )
    .await?;
    let (snapshot, contents) = history::snapshot_contents(
        &state.app_data_dir,
        &roster_identity(previous),
        &reference.snapshot_id,
        std::slice::from_ref(&destination),
    )
    .await?;
    if snapshot.rendered_hash != prior_hash
        || snapshot.roster_record.as_ref() != Some(&expected)
        || contents.len() != 1
        || render::sha256_hex(&contents[0]) != prior_hash
    {
        return Err(AppError::StorageCorrupt {
            message: "Agent roster recovery preservation snapshot changed".into(),
        });
    }
    Ok(())
}

async fn commit_recovered_roster_retention(
    state: &AppState,
    previous: &AgentRosterInstallRecord,
    preservation: &AgentRosterPreservation,
) -> Result<(), AppError> {
    let AgentRosterPreservation::Snapshot(reference) = preservation else {
        return Ok(());
    };
    history::commit_roster_retention(
        &state.app_data_dir,
        &roster_identity(previous),
        &reference.retired_snapshot_ids,
    )
    .await?;
    history::cleanup_roster_staging(&state.app_data_dir, &reference.staging_path).await
}

fn verify_roster_preimage(path: &Path, expected_hash: Option<&str>) -> Result<(), AppError> {
    if recovery_file_hash(path)?.as_deref() == expected_hash {
        return Ok(());
    }
    Err(AppError::InvalidArgument {
        message: "Agent roster target changed immediately before mutation".into(),
    })
}

fn combine_roster_restoration(
    files: Result<(), AppError>,
    history: Result<(), AppError>,
) -> Result<(), AppError> {
    match (files, history) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(files), Err(history)) => Err(AppError::Internal {
            message: format!(
                "Agent roster file restoration failed: {files}; history restoration failed: {history}"
            ),
        }),
    }
}

async fn rollback_unjournaled_roster_success(
    state: &AppState,
    prior_files: &[BatchFileSnapshot],
    prior_rosters: &[AgentRosterInstallRecord],
    history_mutation: &mut Option<history::RosterHistoryMutation>,
) -> Result<(), AppError> {
    let files = restore_batch_files_verified(prior_files).await;
    let ledger = save_rosters_for_state(state, prior_rosters).await;
    let history = rollback_roster_history(history_mutation).await;
    match (files, ledger, history) {
        (Ok(()), Ok(()), Ok(())) => Ok(()),
        (files, ledger, history) => Err(AppError::Internal {
            message: format!(
                "Agent roster unjournaled compensation failed: files={files:?}; ledger={ledger:?}; history={history:?}"
            ),
        }),
    }
}

async fn compensate_roster_commit_failure(
    state: &AppState,
    database: &crate::state_db::StateDatabase,
    operation: &crate::state_db::FilesystemOperation,
    prior_files: &[BatchFileSnapshot],
    prior_rosters: &[AgentRosterInstallRecord],
    history: Result<(), AppError>,
    commit_error: AppError,
) -> AppError {
    let files = restore_batch_files_verified(prior_files).await;
    let ledger = save_rosters_for_state(state, prior_rosters).await;
    let ledger = match ledger {
        Ok(()) => match load_rosters_for_state(state).await {
            Ok(current) if current == prior_rosters => Ok(()),
            Ok(_) => Err(AppError::StorageCorrupt {
                message: "Agent roster commit compensation did not restore exact ledger rows"
                    .into(),
            }),
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    };
    let detail = match (&files, &ledger, &history) {
        (Ok(()), Ok(()), Ok(())) => {
            format!("journal commit failed after exact compensation: {commit_error}")
        }
        (files, ledger, history) => format!(
            "journal commit failed: {commit_error}; file compensation: {files:?}; ledger compensation: {ledger:?}; history compensation: {history:?}"
        ),
    };
    let retained = database
        .retain_filesystem_operation_error(&operation.id, &detail)
        .await;
    match (files, ledger, history, retained) {
        (Ok(()), Ok(()), Ok(()), Ok(())) => commit_error,
        (files, ledger, history, retained) => AppError::Internal {
            message: format!(
                "Agent roster journal commit failed: {commit_error}; file compensation: {files:?}; ledger compensation: {ledger:?}; history compensation: {history:?}; retain recovery error: {retained:?}"
            ),
        },
    }
}

/// Track a recognized on-disk agent into the ledger **without writing anything**
/// (contrast `do_install`, which renders + overwrites). We record the canonical
/// render's hash + the current corpus source/body hashes, but leave the user's
/// file exactly as it is. Reconcile then tells the truth: if the on-disk bytes
/// match the canonical render it shows `Current`; if they differ (older catalog
/// version, or hand-edited) it shows `Modified`, and an explicit Update (which
/// backs up first) reconciles it. This is the safe replacement for "Adopt".
async fn do_track(
    app: &AppHandle,
    state: &AppState,
    reference: AgentReference,
    tool: Tool,
    project_path: Option<String>,
) -> Result<InstallRecord, AppError> {
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
    recover_agent_operations_locked(state).await?;
    corpus::ensure_corpus(app, state).await?;
    let package = crate::agents::resolve_agent_package(&state.app_data_dir, &reference).await?;
    if !package.installable {
        return Err(AppError::InvalidArgument {
            message: "Agent package is not installable".into(),
        });
    }
    let raw = crate::agents::read_agent_text(&state.app_data_dir, &reference).await?;
    if render::sha256_hex(raw.as_bytes()) != package.source_hash {
        return Err(AppError::InvalidArgument {
            message: "Agent source changed after inspection".into(),
        });
    }
    let agent = package
        .agent
        .clone()
        .ok_or_else(|| AppError::InvalidArgument {
            message: "Agent package has no valid metadata".into(),
        })?;
    let revision = package
        .version
        .clone()
        .unwrap_or_else(|| package.source_hash.clone());

    let home = tool_home(state, &tool).await?;
    let proot = project_path.as_ref().map(PathBuf::from);
    let existing = candidate_destination_sets(&agent, &raw, &tool, &home, proot.as_deref())?
        .into_iter()
        .find(|paths| paths.iter().all(|path| path.exists()));
    let Some(existing) = existing else {
        return Err(AppError::InvalidArgument {
            message: "no complete existing Agent destination is available to track".into(),
        });
    };
    let mut ledger = load_ledger(app, state).await?;
    ensure_destinations_available(
        &ledger,
        &reference,
        &tool,
        project_path.as_deref(),
        &existing,
        true,
    )?;
    let mut record = track_agent_record(
        &agent,
        &raw,
        &tool,
        &home,
        proot.as_deref(),
        &package.source_hash,
        &package.body_hash,
        &revision,
        &now_iso(),
    )?;
    record.source_id = reference.source_id.clone();
    record.relative_path = reference.relative_path.clone();
    record.source_snapshot_hash = package.source_hash;
    record.capabilities = package.capabilities;
    record.publisher_key = package.publisher_key;
    record.publisher_verified = package.publisher_verified;

    ledger.retain(|current| {
        !(current.source_id == reference.source_id
            && current.relative_path == reference.relative_path
            && current.tool == tool
            && current.project_path == project_path)
    });
    ledger.push(record.clone());
    save_ledger(app, &ledger).await?;
    Ok(record)
}

/// Build a ledger record for Track: compute the canonical render's hash and the
/// destination, but write NOTHING. Pure (Tauri-free) so it's unit-testable
/// against a tempdir — and the test can assert no file appears.
#[allow(clippy::too_many_arguments)]
fn track_agent_record(
    agent: &crate::types::Agent,
    raw: &str,
    tool: &str,
    home: &Path,
    project_root: Option<&Path>,
    source_hash: &str,
    body_hash: &str,
    corpus_version: &str,
    installed_at: &str,
) -> Result<InstallRecord, AppError> {
    let rendered = render::render_artifacts(agent, raw, tool)?;
    let candidate_sets = candidate_destination_sets(agent, raw, tool, home, project_root)?;
    let paths = candidate_sets
        .iter()
        .find(|paths| paths.iter().all(|path| path.exists()))
        .unwrap_or(&candidate_sets[0]);
    let manifest = artifact_manifest(paths, &rendered)?;
    let mut record = record_for(
        agent,
        &paths[0],
        tool,
        project_root,
        manifest[0].rendered_hash.clone(),
        source_hash,
        body_hash,
        corpus_version,
        installed_at,
    );
    record.artifacts = manifest;
    Ok(record)
}

/// Possible physical destinations for one logical install. App-authored files
/// historically used the catalog filename slug; upstream `convert.sh` uses
/// `slugify(name)` for transform tools. Recognize both without changing the
/// catalog's stable identity.
#[cfg(test)]
fn candidate_dests(
    agent: &crate::types::Agent,
    raw: &str,
    tool: &str,
    home: &Path,
    project_root: Option<&Path>,
) -> Result<Vec<PathBuf>, AppError> {
    Ok(
        candidate_destination_sets(agent, raw, tool, home, project_root)?
            .into_iter()
            .flatten()
            .collect(),
    )
}

fn candidate_destination_sets(
    agent: &crate::types::Agent,
    raw: &str,
    tool: &str,
    home: &Path,
    project_root: Option<&Path>,
) -> Result<Vec<Vec<PathBuf>>, AppError> {
    let mut sets = vec![render::dests(tool, &agent.slug, home, project_root)?];
    let conversion_slug = render::output_slug(agent, raw, tool);
    if conversion_slug != agent.slug {
        let converted = render::dests(tool, &conversion_slug, home, project_root)?;
        if !sets.contains(&converted) {
            sets.push(converted);
        }
    }
    Ok(sets)
}

/// True when the tool's per-agent unit is a directory (`{slug}/LEAF`) rather
/// than a bare `{slug}.ext` file — i.e. a dest template has a `/` right after
/// `{slug}`. skill-md tools (Antigravity, Osaurus) are dir-units.
fn tool_is_dir_unit(tool: &str) -> bool {
    let Some(meta) = crate::registry::get(tool) else {
        return false;
    };
    let Some(dest) = meta.dest.as_ref() else {
        return false;
    };
    dest.user.iter().chain(dest.project.iter()).any(|t| {
        t.split_once("{slug}")
            .is_some_and(|(_, after)| after.starts_with('/'))
    })
}

/// Back up divergent files, then remove every existing physical destination.
/// Backup is a separate first pass so a preservation failure cannot occur after
/// an earlier destination has already been deleted.
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
async fn remove_agent_files(
    agent: &crate::types::Agent,
    raw: &str,
    tool: &str,
    home: &Path,
    project_root: Option<&Path>,
    ledger_dest: Option<&Path>,
    backup_dir: &Path,
    stamp: &str,
) -> Result<(), AppError> {
    let (canonical, _) = render::render_with_hash(agent, raw, tool)?;
    let mut paths = candidate_dests(agent, raw, tool, home, project_root)?;
    if let Some(path) = ledger_dest {
        let path = path.to_path_buf();
        if !paths.contains(&path) {
            paths.push(path);
        }
    }

    let existing: Vec<PathBuf> = paths.into_iter().filter(|p| p.exists()).collect();
    for (index, path) in existing.iter().enumerate() {
        let backup_stamp = format!("{stamp}-{index}");
        backup_if_differs(path, canonical.as_bytes(), backup_dir, &backup_stamp).await?;
    }
    // Dir-unit tools (skill-md: Antigravity, Osaurus) install each agent as
    // `<slug>/SKILL.md`. Removing only the leaf file orphans the now-empty
    // `<slug>/` dir, which the reconcile scan re-surfaces as an untracked
    // phantom (#60). Prune the agent dir after deleting the file. `remove_dir`
    // is empty-only, so a dir a user dropped their own files into is left be.
    let dir_unit = tool_is_dir_unit(tool);
    for path in existing {
        remove_file_strict(&path).await?;
        if dir_unit {
            if let Some(agent_dir) = path.parent() {
                let _ = tokio::fs::remove_dir(agent_dir).await;
            }
        }
    }
    Ok(())
}

async fn remove_file_strict(path: &Path) -> Result<(), AppError> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(AppError::Io {
            message: format!("remove agent file {}: {e}", path.display()),
        }),
    }
}

async fn remove_agent_artifacts_with<F>(paths: &[PathBuf], mut remove: F) -> Result<(), AppError>
where
    F: FnMut(&Path) -> std::io::Result<()>,
{
    let prior = capture_batch_files(paths).await?;
    for path in paths {
        if let Err(error) = remove(path) {
            let original = AppError::Io {
                message: format!("remove agent file {}: {error}", path.display()),
            };
            return match restore_batch_files(&prior).await {
                Ok(()) => Err(original),
                Err(rollback) => Err(rollback_error(
                    "uninstall Agent artifacts",
                    original,
                    rollback,
                )),
            };
        }
    }
    Ok(())
}

fn disabled_destination(destination: &Path) -> Result<PathBuf, AppError> {
    let parent = destination
        .parent()
        .ok_or_else(|| AppError::InvalidArgument {
            message: format!("Agent destination has no parent: {}", destination.display()),
        })?;
    let name = destination
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| AppError::InvalidArgument {
            message: format!(
                "Agent destination has no portable filename: {}",
                destination.display()
            ),
        })?;
    Ok(parent.join(format!(".{name}.agency-agents-disabled")))
}

#[cfg(test)]
fn move_managed_files(
    sources: &[PathBuf],
    destinations: &[PathBuf],
    expected_hash: &str,
) -> Result<(), AppError> {
    let hashes = vec![expected_hash.to_owned(); sources.len()];
    move_managed_artifacts(sources, destinations, &hashes)
}

fn move_managed_artifacts(
    sources: &[PathBuf],
    destinations: &[PathBuf],
    expected_hashes: &[String],
) -> Result<(), AppError> {
    move_managed_artifacts_with(
        sources,
        destinations,
        expected_hashes,
        |source, destination| std::fs::rename(source, destination),
    )?;
    for destination in destinations {
        sync_parent_directory_entry(destination)?;
    }
    Ok(())
}

#[cfg(unix)]
fn sync_parent_directory_entry(path: &Path) -> Result<(), AppError> {
    let parent = path.parent().ok_or_else(|| AppError::Io {
        message: format!("Agent filesystem target has no parent: {}", path.display()),
    })?;
    std::fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| AppError::Io {
            message: format!(
                "sync Agent filesystem directory {}: {error}",
                parent.display()
            ),
        })
}

#[cfg(not(unix))]
fn sync_parent_directory_entry(_path: &Path) -> Result<(), AppError> {
    Ok(())
}

#[cfg(test)]
fn move_managed_files_with<F>(
    sources: &[PathBuf],
    destinations: &[PathBuf],
    expected_hash: &str,
    rename: F,
) -> Result<(), AppError>
where
    F: FnMut(&Path, &Path) -> std::io::Result<()>,
{
    let hashes = vec![expected_hash.to_owned(); sources.len()];
    move_managed_artifacts_with(sources, destinations, &hashes, rename)
}

fn move_managed_artifacts_with<F>(
    sources: &[PathBuf],
    destinations: &[PathBuf],
    expected_hashes: &[String],
    mut rename: F,
) -> Result<(), AppError>
where
    F: FnMut(&Path, &Path) -> std::io::Result<()>,
{
    if sources.is_empty()
        || sources.len() != destinations.len()
        || sources.len() != expected_hashes.len()
    {
        return Err(AppError::InvalidArgument {
            message: "Agent lifecycle move requires matching source and destination files".into(),
        });
    }
    for ((source, destination), expected_hash) in
        sources.iter().zip(destinations).zip(expected_hashes)
    {
        if source.parent() != destination.parent() {
            return Err(AppError::InvalidArgument {
                message: "Agent lifecycle files must move within the same parent directory".into(),
            });
        }
        let metadata = std::fs::symlink_metadata(source).map_err(|error| AppError::Io {
            message: format!("inspect managed Agent file {}: {error}", source.display()),
        })?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || crate::skills::metadata_is_reparse_point(&metadata)
        {
            return Err(AppError::InvalidArgument {
                message: format!(
                    "managed Agent path is not a regular file: {}",
                    source.display()
                ),
            });
        }
        let bytes = std::fs::read(source).map_err(|error| AppError::Io {
            message: format!("read managed Agent file {}: {error}", source.display()),
        })?;
        if render::sha256_hex(&bytes) != *expected_hash {
            return Err(AppError::InvalidArgument {
                message: format!("managed Agent file was modified: {}", source.display()),
            });
        }
        if std::fs::symlink_metadata(destination).is_ok() {
            return Err(AppError::InvalidArgument {
                message: format!(
                    "Agent lifecycle destination is occupied: {}",
                    destination.display()
                ),
            });
        }
    }

    for (moved, (source, destination)) in sources.iter().zip(destinations).enumerate() {
        if let Err(error) = rename(source, destination) {
            for index in (0..moved).rev() {
                if let Err(rollback) = rename(&destinations[index], &sources[index]) {
                    return Err(AppError::Internal {
                        message: format!(
                            "move Agent file {} failed: {error}; restore {} failed: {rollback}",
                            source.display(),
                            sources[index].display()
                        ),
                    });
                }
            }
            return Err(AppError::Io {
                message: format!(
                    "move managed Agent file {} -> {}: {error}",
                    source.display(),
                    destination.display()
                ),
            });
        }
    }
    Ok(())
}

fn record_rendered_hashes(record: &InstallRecord, count: usize) -> Vec<String> {
    if record.artifacts.is_empty() {
        vec![record.rendered_hash.clone(); count]
    } else {
        record
            .artifacts
            .iter()
            .map(|artifact| artifact.rendered_hash.clone())
            .collect()
    }
}

fn set_record_disabled_paths(record: &mut InstallRecord, disabled: Option<&[PathBuf]>) {
    record.disabled_path = disabled
        .and_then(|paths| paths.first())
        .map(|path| path.to_string_lossy().into_owned());
    if !record.artifacts.is_empty() {
        for (index, artifact) in record.artifacts.iter_mut().enumerate() {
            artifact.disabled_path = disabled
                .and_then(|paths| paths.get(index))
                .map(|path| path.to_string_lossy().into_owned());
        }
    }
}

fn install_identity(record: &InstallRecord) -> AgentInstallIdentity {
    AgentInstallIdentity {
        reference: AgentReference {
            source_id: record.source_id.clone(),
            relative_path: record.relative_path.clone(),
        },
        tool: record.tool.clone(),
        scope: record.scope,
        project_path: record.project_path.clone(),
    }
}

fn install_record_index(
    records: &[InstallRecord],
    source_id: &str,
    relative_path: &str,
    tool: &str,
    project_path: Option<&str>,
) -> Result<usize, AppError> {
    crate::library::validate_reference(source_id, relative_path)?;
    records
        .iter()
        .position(|record| {
            record.source_id == source_id
                && record.relative_path == relative_path
                && record.tool == tool
                && record.project_path.as_deref() == project_path
        })
        .ok_or_else(|| AppError::InvalidArgument {
            message: format!("Agent install is not tracked: {source_id}:{relative_path}"),
        })
}

async fn resolved_record_paths(
    state: &AppState,
    record: &InstallRecord,
) -> Result<Vec<PathBuf>, AppError> {
    let home = tool_home(state, &record.tool).await?;
    let project = record.project_path.as_deref().map(Path::new);
    if !record.artifacts.is_empty() {
        let recorded = record
            .artifacts
            .iter()
            .map(|artifact| PathBuf::from(&artifact.dest))
            .collect::<Vec<_>>();
        let templates = crate::registry::get(&record.tool)
            .and_then(|meta| meta.dest.as_ref())
            .map(|dest| {
                if project.is_some() {
                    &dest.project
                } else {
                    &dest.user
                }
            })
            .ok_or_else(|| AppError::InvalidArgument {
                message: "tracked Agent tool has no destinations".into(),
            })?;
        let first = recorded.first().ok_or_else(|| AppError::InvalidArgument {
            message: "tracked Agent artifact manifest is empty".into(),
        })?;
        let first_template = templates.first().ok_or_else(|| AppError::InvalidArgument {
            message: "tracked Agent tool has no destinations".into(),
        })?;
        let candidate_roots = if let Some(project) = project {
            vec![project.to_path_buf()]
        } else {
            first.ancestors().map(Path::to_path_buf).collect()
        };
        let matches = candidate_roots
            .into_iter()
            .filter_map(|root| {
                let slug = destination_template_slug(&root, first_template, first)?;
                let expected = templates
                    .iter()
                    .map(|template| root.join(template.replace("{slug}", &slug)))
                    .collect::<Vec<_>>();
                (expected == recorded).then_some((root, slug))
            })
            .collect::<Vec<_>>();
        if matches.len() != 1 || templates.len() != recorded.len() {
            return Err(AppError::InvalidArgument {
                message:
                    "tracked Agent artifact manifest does not match one exact registry topology"
                        .into(),
            });
        }
        let disabled_paths_match = if record.disabled_path.is_some() {
            let expected_disabled = recorded
                .iter()
                .map(|path| disabled_destination(path))
                .collect::<Result<Vec<_>, _>>()?;
            record
                .artifacts
                .iter()
                .zip(expected_disabled)
                .all(|(artifact, expected)| artifact.disabled_path.as_deref() == expected.to_str())
        } else {
            record
                .artifacts
                .iter()
                .all(|artifact| artifact.disabled_path.is_none())
        };
        if record.artifacts.len() > 8
            || record.artifacts[0].dest != record.dest
            || record.artifacts[0].rendered_hash != record.rendered_hash
            || record.artifacts[0].disabled_path != record.disabled_path
            || !disabled_paths_match
        {
            return Err(AppError::InvalidArgument {
                message: "tracked Agent artifact manifest no longer matches its tool registry"
                    .into(),
            });
        }
        return Ok(recorded);
    }
    let mut paths = render::dests(&record.tool, &record.slug, &home, project)?;
    let recorded = PathBuf::from(&record.dest);
    if paths.len() == 1 {
        paths[0] = recorded;
    } else if let Some(index) = paths.iter().position(|path| path == &recorded) {
        paths.swap(0, index);
    } else {
        return Err(AppError::InvalidArgument {
            message: "tracked Agent destination no longer matches its tool registry".into(),
        });
    }
    Ok(paths)
}

fn destination_template_slug(root: &Path, template: &str, target: &Path) -> Option<String> {
    let relative = target.strip_prefix(root).ok()?;
    let relative = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    let (before, after) = template.split_once("{slug}")?;
    let token = relative
        .strip_prefix(before.trim_matches('/'))?
        .strip_prefix('/')?
        .strip_suffix(after.trim_start_matches('/'))?
        .trim_end_matches('/');
    (!token.is_empty()
        && token
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'))
    .then(|| token.to_owned())
}

fn same_agent_install(left: &InstallRecord, right: &InstallRecord) -> bool {
    left.source_id == right.source_id
        && left.relative_path == right.relative_path
        && left.tool == right.tool
        && left.project_path == right.project_path
}

fn exact_agent_install(left: &InstallRecord, right: &InstallRecord) -> bool {
    serde_json::to_value(left).ok() == serde_json::to_value(right).ok()
}

fn ensure_agent_recovery_parent(base: &Path, target: &Path) -> Result<(), AppError> {
    let base = std::fs::canonicalize(base).map_err(|error| AppError::Io {
        message: format!("resolve Agent recovery base: {error}"),
    })?;
    let relative = target
        .strip_prefix(&base)
        .map_err(|_| AppError::StorageCorrupt {
            message: "Agent recovery destination escaped its configured base".into(),
        })?;
    let mut current = base;
    for component in relative
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .components()
    {
        current.push(component.as_os_str());
        match std::fs::symlink_metadata(&current) {
            Ok(metadata)
                if metadata.is_dir()
                    && !metadata.file_type().is_symlink()
                    && !crate::skills::metadata_is_reparse_point(&metadata) => {}
            Ok(_) => {
                return Err(AppError::StorageCorrupt {
                    message: "Agent recovery destination contains an unsafe ancestor".into(),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&current).map_err(|error| AppError::Io {
                    message: format!("create Agent recovery directory: {error}"),
                })?;
            }
            Err(error) => {
                return Err(AppError::Io {
                    message: format!("inspect Agent recovery directory: {error}"),
                });
            }
        }
    }
    Ok(())
}

async fn apply_recovered_agent_install(
    state: &AppState,
    operation_id: &str,
    payload: &AgentInstallOperation,
) -> Result<(), AppError> {
    let mut records = load_ledger_for_state(state).await?;
    match records
        .iter()
        .position(|record| same_agent_install(record, &payload.next))
    {
        Some(index) if exact_agent_install(&records[index], &payload.next) => {}
        Some(index)
            if payload
                .previous
                .as_ref()
                .is_some_and(|previous| exact_agent_install(&records[index], previous)) =>
        {
            records[index] = payload.next.clone();
        }
        Some(_) => {
            return Err(AppError::StorageCorrupt {
                message: "Agent install recovery found changed ledger metadata".into(),
            });
        }
        None if payload.previous.is_none() => records.push(payload.next.clone()),
        None => {
            return Err(AppError::StorageCorrupt {
                message: "Agent install recovery lost its previous ledger metadata".into(),
            });
        }
    }
    save_ledger_after_filesystem(state, &records, operation_id).await
}

async fn recover_agent_install_operation(
    state: &AppState,
    database: &crate::state_db::StateDatabase,
    operation: &crate::state_db::FilesystemOperation,
) -> Result<(), AppError> {
    let payload = serde_json::from_value::<AgentInstallOperation>(operation.payload.clone())
        .map_err(|_| AppError::StorageCorrupt {
            message: "Agent install recovery payload is invalid".into(),
        })?;
    let contents = payload.rendered.contents();
    let artifacts = record_artifacts(&payload.next);
    if contents.len() != artifacts.len()
        || contents.iter().zip(&artifacts).any(|(content, artifact)| {
            content.len() as u64 > MAX_INSTALLED_BYTES
                || render::sha256_hex(content.as_bytes()) != artifact.rendered_hash
        })
        || payload
            .previous
            .as_ref()
            .is_some_and(|previous| !same_agent_install(previous, &payload.next))
    {
        return Err(AppError::StorageCorrupt {
            message: "Agent install recovery payload changed identity or content".into(),
        });
    }
    let resolved = resolved_record_paths(state, &payload.next).await?;
    let stored = payload
        .targets
        .iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    if resolved != stored || resolved.is_empty() {
        return Err(AppError::StorageCorrupt {
            message: "Agent install recovery destinations changed".into(),
        });
    }
    let base = payload
        .next
        .project_path
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or(tool_home(state, &payload.next.tool).await?);
    let previous = payload.previous.as_ref().map(record_artifacts);
    let mut needs_write = Vec::with_capacity(resolved.len());
    for (index, ((target, _content), artifact)) in
        resolved.iter().zip(&contents).zip(&artifacts).enumerate()
    {
        ensure_agent_recovery_parent(&base, target)?;
        match std::fs::symlink_metadata(target) {
            Ok(metadata)
                if metadata.is_file()
                    && !metadata.file_type().is_symlink()
                    && !crate::skills::metadata_is_reparse_point(&metadata) =>
            {
                let bytes = std::fs::read(target).map_err(|error| AppError::Io {
                    message: format!("read Agent recovery destination: {error}"),
                })?;
                let hash = render::sha256_hex(&bytes);
                if hash != artifact.rendered_hash {
                    if previous
                        .as_ref()
                        .and_then(|items| items.get(index))
                        .is_none_or(|previous| hash != previous.rendered_hash)
                    {
                        return Err(AppError::StorageCorrupt {
                            message: "Agent install recovery found changed destination content"
                                .into(),
                        });
                    }
                    needs_write.push(true);
                } else {
                    needs_write.push(false);
                }
            }
            Ok(_) => {
                return Err(AppError::StorageCorrupt {
                    message: "Agent install recovery found an unsafe destination".into(),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                needs_write.push(true);
            }
            Err(error) => {
                return Err(AppError::Io {
                    message: format!("inspect Agent recovery destination: {error}"),
                });
            }
        }
    }
    let prior = capture_batch_files(&resolved).await?;
    for ((target, content), needs_write) in resolved.iter().zip(&contents).zip(needs_write) {
        if !needs_write {
            continue;
        }
        let result = async {
            backup_if_differs(
                target,
                content.as_bytes(),
                &backups_dir_for(&state.app_data_dir),
                &operation.id,
            )
            .await?;
            atomic_write(target, content.as_bytes()).await
        }
        .await;
        if let Err(error) = result {
            return match restore_batch_files(&prior).await {
                Ok(()) => Err(error),
                Err(rollback) => Err(rollback_error(
                    "recover Agent install artifacts",
                    error,
                    rollback,
                )),
            };
        }
    }
    match operation.phase {
        crate::state_db::FilesystemOperationPhase::Prepared => {
            apply_recovered_agent_install(state, &operation.id, &payload).await?;
            database.commit_filesystem_operation(&operation.id).await
        }
        crate::state_db::FilesystemOperationPhase::FilesystemApplied => {
            let records = load_ledger_for_state(state).await?;
            if !records
                .iter()
                .any(|record| exact_agent_install(record, &payload.next))
            {
                Err(AppError::StorageCorrupt {
                    message: "Agent install recovery lost committed ledger metadata".into(),
                })
            } else {
                database.commit_filesystem_operation(&operation.id).await
            }
        }
        crate::state_db::FilesystemOperationPhase::Committed => Ok(()),
    }
}

fn recovery_file_hash(path: &Path) -> Result<Option<String>, AppError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(AppError::Io {
                message: format!("inspect Agent recovery file: {error}"),
            });
        }
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || crate::skills::metadata_is_reparse_point(&metadata)
        || metadata.len() > MAX_INSTALLED_BYTES
    {
        return Err(AppError::StorageCorrupt {
            message: "Agent recovery found an unsafe file".into(),
        });
    }
    let bytes = std::fs::read(path).map_err(|error| AppError::Io {
        message: format!("read Agent recovery file: {error}"),
    })?;
    Ok(Some(render::sha256_hex(&bytes)))
}

async fn apply_recovered_agent_move(
    state: &AppState,
    operation_id: &str,
    payload: &AgentMoveOperation,
) -> Result<(), AppError> {
    let mut records = load_ledger_for_state(state).await?;
    let index = records
        .iter()
        .position(|record| same_agent_install(record, &payload.next))
        .ok_or_else(|| AppError::StorageCorrupt {
            message: "Agent move recovery lost its ledger metadata".into(),
        })?;
    if exact_agent_install(&records[index], &payload.previous) {
        records[index] = payload.next.clone();
    } else if !exact_agent_install(&records[index], &payload.next) {
        return Err(AppError::StorageCorrupt {
            message: "Agent move recovery found changed ledger metadata".into(),
        });
    }
    save_ledger_after_filesystem(state, &records, operation_id).await
}

async fn recover_agent_move_operation(
    state: &AppState,
    database: &crate::state_db::StateDatabase,
    operation: &crate::state_db::FilesystemOperation,
) -> Result<(), AppError> {
    let payload =
        serde_json::from_value::<AgentMoveOperation>(operation.payload.clone()).map_err(|_| {
            AppError::StorageCorrupt {
                message: "Agent move recovery payload is invalid".into(),
            }
        })?;
    if !same_agent_install(&payload.previous, &payload.next)
        || payload.previous.rendered_hash != payload.next.rendered_hash
    {
        return Err(AppError::StorageCorrupt {
            message: "Agent move recovery identity changed".into(),
        });
    }
    let active = resolved_record_paths(state, &payload.next).await?;
    let disabled = active
        .iter()
        .map(|path| disabled_destination(path))
        .collect::<Result<Vec<_>, _>>()?;
    if payload.active.iter().map(PathBuf::from).collect::<Vec<_>>() != active
        || payload
            .disabled
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>()
            != disabled
    {
        return Err(AppError::StorageCorrupt {
            message: "Agent move recovery destinations changed".into(),
        });
    }
    let (sources, targets) = match operation.kind.as_str() {
        "agent_disable"
            if payload.previous.disabled_path.is_none() && payload.next.disabled_path.is_some() =>
        {
            (&active, &disabled)
        }
        "agent_enable"
            if payload.previous.disabled_path.is_some() && payload.next.disabled_path.is_none() =>
        {
            (&disabled, &active)
        }
        _ => {
            return Err(AppError::StorageCorrupt {
                message: "Agent move recovery transition is invalid".into(),
            });
        }
    };
    let mut remaining_sources = Vec::new();
    let mut remaining_targets = Vec::new();
    let expected_hashes = record_rendered_hashes(&payload.next, sources.len());
    let mut remaining_hashes = Vec::new();
    for ((source, target), expected_hash) in sources.iter().zip(targets).zip(&expected_hashes) {
        let source_hash = recovery_file_hash(source)?;
        let target_hash = recovery_file_hash(target)?;
        match (source_hash.as_deref(), target_hash.as_deref()) {
            (Some(hash), None) if hash == expected_hash => {
                remaining_sources.push(source.clone());
                remaining_targets.push(target.clone());
                remaining_hashes.push(expected_hash.clone());
            }
            (None, Some(hash)) if hash == expected_hash => {}
            _ => {
                return Err(AppError::StorageCorrupt {
                    message: "Agent move recovery found changed or duplicate content".into(),
                });
            }
        }
    }
    match operation.phase {
        crate::state_db::FilesystemOperationPhase::Prepared => {
            if !remaining_sources.is_empty() {
                move_managed_artifacts(&remaining_sources, &remaining_targets, &remaining_hashes)?;
            }
            apply_recovered_agent_move(state, &operation.id, &payload).await?;
            database.commit_filesystem_operation(&operation.id).await
        }
        crate::state_db::FilesystemOperationPhase::FilesystemApplied => {
            if !remaining_sources.is_empty() {
                return Err(AppError::StorageCorrupt {
                    message: "Agent move recovery found incomplete committed files".into(),
                });
            }
            let records = load_ledger_for_state(state).await?;
            if !records
                .iter()
                .any(|record| exact_agent_install(record, &payload.next))
            {
                Err(AppError::StorageCorrupt {
                    message: "Agent move recovery lost committed ledger metadata".into(),
                })
            } else {
                database.commit_filesystem_operation(&operation.id).await
            }
        }
        crate::state_db::FilesystemOperationPhase::Committed => Ok(()),
    }
}

async fn apply_recovered_agent_uninstall(
    state: &AppState,
    operation_id: &str,
    previous: &InstallRecord,
) -> Result<(), AppError> {
    let mut records = load_ledger_for_state(state).await?;
    if let Some(index) = records
        .iter()
        .position(|record| same_agent_install(record, previous))
    {
        if !exact_agent_install(&records[index], previous) {
            return Err(AppError::StorageCorrupt {
                message: "Agent uninstall recovery found changed ledger metadata".into(),
            });
        }
        records.remove(index);
    }
    save_ledger_after_filesystem(state, &records, operation_id).await
}

async fn recover_agent_uninstall_operation(
    state: &AppState,
    database: &crate::state_db::StateDatabase,
    operation: &crate::state_db::FilesystemOperation,
) -> Result<(), AppError> {
    let payload = serde_json::from_value::<AgentUninstallOperation>(operation.payload.clone())
        .map_err(|_| AppError::StorageCorrupt {
            message: "Agent uninstall recovery payload is invalid".into(),
        })?;
    let active = resolved_record_paths(state, &payload.previous).await?;
    let paths = if payload.previous.disabled_path.is_some() {
        active
            .iter()
            .map(|path| disabled_destination(path))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        active
    };
    if payload.paths.iter().map(PathBuf::from).collect::<Vec<_>>() != paths
        || paths.len() != payload.hashes.len()
    {
        return Err(AppError::StorageCorrupt {
            message: "Agent uninstall recovery destinations changed".into(),
        });
    }
    match operation.phase {
        crate::state_db::FilesystemOperationPhase::Prepared => {
            let mut existing = Vec::new();
            for (path, expected_hash) in paths.iter().zip(&payload.hashes) {
                match (recovery_file_hash(path)?, expected_hash) {
                    (Some(hash), Some(expected)) if hash == *expected => {
                        existing.push(path.clone())
                    }
                    (None, _) | (_, None) if !path.exists() => {}
                    _ => {
                        return Err(AppError::StorageCorrupt {
                            message: "Agent uninstall recovery found changed content".into(),
                        });
                    }
                }
            }
            remove_agent_artifacts_with(&existing, |path| std::fs::remove_file(path)).await?;
            apply_recovered_agent_uninstall(state, &operation.id, &payload.previous).await?;
            database.commit_filesystem_operation(&operation.id).await
        }
        crate::state_db::FilesystemOperationPhase::FilesystemApplied => {
            let records = load_ledger_for_state(state).await?;
            if paths
                .iter()
                .map(|path| recovery_file_hash(path))
                .collect::<Result<Vec<_>, _>>()?
                .iter()
                .any(Option::is_some)
                || records
                    .iter()
                    .any(|record| same_agent_install(record, &payload.previous))
            {
                Err(AppError::StorageCorrupt {
                    message: "Agent uninstall recovery found changed committed state".into(),
                })
            } else {
                database.commit_filesystem_operation(&operation.id).await
            }
        }
        crate::state_db::FilesystemOperationPhase::Committed => Ok(()),
    }
}

fn same_roster_install(left: &AgentRosterInstallRecord, right: &AgentRosterInstallRecord) -> bool {
    left.tool == right.tool && left.project_path == right.project_path
}

fn exact_roster_install(left: &AgentRosterInstallRecord, right: &AgentRosterInstallRecord) -> bool {
    left == right
}

async fn validated_recovery_roster_paths(
    state: &AppState,
    record: &AgentRosterInstallRecord,
) -> Result<(PathBuf, PathBuf), AppError> {
    validate_roster_record(record)?;
    let project = canonical_registered_instruction_project(state, &record.project_path).await?;
    let expected = render::dests(&record.tool, "roster", Path::new("/"), Some(&project))?;
    let active = expected
        .into_iter()
        .next()
        .ok_or_else(|| AppError::StorageCorrupt {
            message: "Agent roster recovery target registry is empty".into(),
        })?;
    if active != Path::new(&record.dest) {
        return Err(AppError::StorageCorrupt {
            message: "Agent roster recovery destination changed from the tool registry".into(),
        });
    }
    let disabled = disabled_destination(&active)?;
    if record
        .disabled_path
        .as_deref()
        .is_some_and(|path| Path::new(path) != disabled)
    {
        return Err(AppError::StorageCorrupt {
            message: "Agent roster recovery disabled destination changed".into(),
        });
    }
    Ok((active, disabled))
}

fn validate_roster_staging_path(
    app_data_dir: &Path,
    staged_path: &str,
) -> Result<PathBuf, AppError> {
    let path = PathBuf::from(staged_path);
    let expected_parent = app_data_dir.join("state/roster-staging");
    let valid_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".bin"))
        .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok());
    if path.parent() != Some(expected_parent.as_path()) || !valid_name {
        return Err(AppError::StorageCorrupt {
            message: "Agent roster recovery staging path escaped app storage".into(),
        });
    }
    Ok(path)
}

fn unjournaled_roster_write_intent_path(
    app_data_dir: &Path,
    staged_path: &str,
) -> Result<PathBuf, AppError> {
    let staged = validate_roster_staging_path(app_data_dir, staged_path)?;
    let id = staged
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| AppError::StorageCorrupt {
            message: "Unjournaled Agent roster staging identity is invalid".into(),
        })?;
    Ok(staged.with_file_name(format!("{id}.intent.json")))
}

async fn write_unjournaled_roster_write_intent(
    state: &AppState,
    intent: &UnjournaledRosterWriteIntent,
) -> Result<PathBuf, AppError> {
    validate_roster_record(&intent.next)?;
    if intent.next.disabled_path.is_some()
        || intent.previous.as_ref().is_some_and(|previous| {
            validate_roster_record(previous).is_err()
                || !same_roster_install(previous, &intent.next)
        })
    {
        return Err(AppError::StorageCorrupt {
            message: "Unjournaled Agent roster write identity changed".into(),
        });
    }
    let staged = validate_roster_staging_path(&state.app_data_dir, &intent.staged_path)?;
    let staged_hash = recovery_file_hash(&staged)?;
    if staged_hash.as_deref() != Some(intent.next.rendered_hash.as_str()) {
        return Err(AppError::StorageCorrupt {
            message: "Unjournaled Agent roster staged bytes changed".into(),
        });
    }
    let path = unjournaled_roster_write_intent_path(&state.app_data_dir, &intent.staged_path)?;
    let bytes = serde_json::to_vec_pretty(intent).map_err(|error| AppError::Internal {
        message: format!("serialize unjournaled Agent roster write intent: {error}"),
    })?;
    atomic_write(&path, &bytes).await?;
    sync_parent_directory_entry(&path)?;
    Ok(path)
}

async fn prepare_unjournaled_roster_write(
    state: &AppState,
    should_prepare: bool,
    previous: Option<&AgentRosterInstallRecord>,
    next: &AgentRosterInstallRecord,
    staged: &Path,
) -> Result<(), AppError> {
    if !should_prepare {
        return Ok(());
    }
    let (active, disabled) = validated_recovery_roster_paths(state, next).await?;
    if recovery_file_hash(&active)?.is_some() || recovery_file_hash(&disabled)?.is_some() {
        return Err(AppError::StorageCorrupt {
            message: "Unjournaled Agent roster write expected an absent destination".into(),
        });
    }
    write_unjournaled_roster_write_intent(
        state,
        &UnjournaledRosterWriteIntent {
            phase: UnjournaledRosterWritePhase::Prepared,
            previous: previous.cloned(),
            next: next.clone(),
            staged_path: staged.to_string_lossy().into_owned(),
        },
    )
    .await?;
    Ok(())
}

async fn mark_unjournaled_roster_write_applied(
    state: &AppState,
    should_mark: bool,
    staged: &Path,
) -> Result<(), AppError> {
    if !should_mark {
        return Ok(());
    }
    let staged_path = staged.to_string_lossy().into_owned();
    let path = unjournaled_roster_write_intent_path(&state.app_data_dir, &staged_path)?;
    let bytes = read_capped(&path, MAX_LEDGER_BYTES).await?;
    let mut intent: UnjournaledRosterWriteIntent =
        serde_json::from_slice(&bytes).map_err(|error| AppError::StorageCorrupt {
            message: format!("parse unjournaled Agent roster write intent: {error}"),
        })?;
    if intent.phase != UnjournaledRosterWritePhase::Prepared || intent.staged_path != staged_path {
        return Err(AppError::StorageCorrupt {
            message: "Unjournaled Agent roster write phase changed".into(),
        });
    }
    intent.phase = UnjournaledRosterWritePhase::FilesystemApplied;
    write_unjournaled_roster_write_intent(state, &intent).await?;
    Ok(())
}

async fn cleanup_roster_staging_artifacts(state: &AppState, staged: &Path) {
    let _ = cleanup_roster_staging_artifacts_durable(state, staged).await;
}

async fn cleanup_roster_staging_artifacts_durable(
    state: &AppState,
    staged: &Path,
) -> Result<(), AppError> {
    cleanup_roster_staging_artifacts_durable_with_sync(state, staged, sync_parent_directory_entry)
        .await
}

async fn cleanup_roster_staging_artifacts_durable_with_sync<F>(
    state: &AppState,
    staged: &Path,
    mut sync_parent: F,
) -> Result<(), AppError>
where
    F: FnMut(&Path) -> Result<(), AppError>,
{
    let staged_path = staged.to_string_lossy().into_owned();
    let intent = unjournaled_roster_write_intent_path(&state.app_data_dir, &staged_path)?;
    let intent_bytes = match read_capped(&intent, MAX_LEDGER_BYTES).await {
        Ok(bytes) => Some(bytes),
        Err(AppError::Io { .. }) if !intent.exists() => None,
        Err(error) => return Err(error),
    };
    if let Some(intent_bytes) = intent_bytes {
        tokio::fs::remove_file(&intent)
            .await
            .map_err(|error| AppError::Io {
                message: format!("remove Agent roster write intent: {error}"),
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
                        "sync Agent roster write intent deletion failed: {error}; restore intent failed: {restore}"
                    ),
                }),
            };
        }
    }
    match tokio::fs::remove_file(staged).await {
        Ok(()) => sync_parent(staged)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(AppError::Io {
                message: format!("remove Agent roster staging bytes: {error}"),
            });
        }
    }
    Ok(())
}

async fn recover_unjournaled_roster_writes(state: &AppState) -> Result<(), AppError> {
    let root = state.app_data_dir.join("state/roster-staging");
    for directory in [state.app_data_dir.join("state"), root.clone()] {
        let metadata = match std::fs::symlink_metadata(&directory) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(AppError::Io {
                    message: format!("inspect Agent roster write staging root: {error}"),
                });
            }
        };
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || crate::skills::metadata_is_reparse_point(&metadata)
        {
            return Err(AppError::StorageCorrupt {
                message: "Agent roster write staging root is not a regular directory".into(),
            });
        }
    }
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(AppError::Io {
                message: format!("read unjournaled Agent roster write staging: {error}"),
            });
        }
    };
    let entries = entries
        .take(1025)
        .map(|entry| {
            entry.map_err(|error| AppError::Io {
                message: format!("read unjournaled Agent roster write entry: {error}"),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if entries.len() > 1024 {
        return Err(AppError::StorageCorrupt {
            message: "Unjournaled Agent roster write staging exceeds its entry limit".into(),
        });
    }
    let mut intents = entries
        .into_iter()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.ends_with(".intent.json"))
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    intents.sort();
    for path in intents {
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| AppError::Io {
            message: format!("inspect unjournaled Agent roster write intent: {error}"),
        })?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || crate::skills::metadata_is_reparse_point(&metadata)
        {
            return Err(AppError::StorageCorrupt {
                message: "Unjournaled Agent roster write intent is not a regular file".into(),
            });
        }
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .and_then(|value| value.strip_suffix(".intent.json"))
            .filter(|value| uuid::Uuid::parse_str(value).is_ok())
            .ok_or_else(|| AppError::StorageCorrupt {
                message: "Unjournaled Agent roster write intent name is invalid".into(),
            })?;
        let staged = root.join(format!("{name}.bin"));
        let bytes = read_capped(&path, MAX_LEDGER_BYTES).await?;
        let intent: UnjournaledRosterWriteIntent =
            serde_json::from_slice(&bytes).map_err(|error| AppError::StorageCorrupt {
                message: format!("parse unjournaled Agent roster write intent: {error}"),
            })?;
        if intent.staged_path != staged.to_string_lossy() {
            return Err(AppError::StorageCorrupt {
                message: "Unjournaled Agent roster write staging path changed".into(),
            });
        }
        write_unjournaled_roster_write_intent(state, &intent).await?;
        let (active, disabled) = validated_recovery_roster_paths(state, &intent.next).await?;
        let active_hash = recovery_file_hash(&active)?;
        if recovery_file_hash(&disabled)?.is_some() {
            return Err(AppError::StorageCorrupt {
                message: "Unjournaled Agent roster write found an occupied disabled destination"
                    .into(),
            });
        }
        let files_are_previous = active_hash.is_none();
        let files_are_next = active_hash.as_deref() == Some(intent.next.rendered_hash.as_str());
        let mut rosters = load_rosters_for_state(state).await?;
        let matching = rosters
            .iter()
            .enumerate()
            .filter(|(_, record)| same_roster_install(record, &intent.next))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if matching.len() > 1 {
            return Err(AppError::StorageCorrupt {
                message: "Unjournaled Agent roster write found duplicate ledger rows".into(),
            });
        }
        let ledger_is_previous = match (&intent.previous, matching.first()) {
            (Some(previous), Some(index)) => exact_roster_install(&rosters[*index], previous),
            (None, None) => true,
            _ => false,
        };
        let ledger_is_next = matching
            .first()
            .is_some_and(|index| exact_roster_install(&rosters[*index], &intent.next));
        if intent.phase == UnjournaledRosterWritePhase::Prepared
            && files_are_previous
            && ledger_is_previous
        {
            cleanup_roster_staging_artifacts_durable(state, &staged).await?;
            continue;
        }
        if !files_are_next && files_are_previous && ledger_is_next {
            let staged_bytes = read_capped(&staged, MAX_INSTALLED_BYTES).await?;
            publish_roster_bytes(&active, &staged_bytes, intent.previous.is_some()).await?;
            sync_parent_directory_entry(&active)?;
        } else if !files_are_next {
            return Err(AppError::StorageCorrupt {
                message: "Unjournaled Agent roster write found changed destination bytes".into(),
            });
        }
        if ledger_is_previous {
            match matching.first() {
                Some(index) => rosters[*index] = intent.next.clone(),
                None => rosters.push(intent.next.clone()),
            }
            rosters.sort_by(|left, right| {
                left.project_path
                    .cmp(&right.project_path)
                    .then_with(|| left.tool.cmp(&right.tool))
            });
            save_rosters_for_state(state, &rosters).await?;
        } else if !ledger_is_next {
            return Err(AppError::StorageCorrupt {
                message: "Unjournaled Agent roster write found changed ledger metadata".into(),
            });
        }
        let mut applied = intent;
        applied.phase = UnjournaledRosterWritePhase::FilesystemApplied;
        write_unjournaled_roster_write_intent(state, &applied).await?;
        cleanup_roster_staging_artifacts_durable(state, &staged).await?;
    }
    Ok(())
}

async fn apply_recovered_roster_write(
    state: &AppState,
    operation_id: &str,
    payload: &AgentRosterWriteOperation,
) -> Result<(), AppError> {
    let mut rosters = load_rosters_for_state(state).await?;
    match rosters
        .iter()
        .position(|record| same_roster_install(record, &payload.next))
    {
        Some(index) if exact_roster_install(&rosters[index], &payload.next) => {}
        Some(index)
            if payload
                .previous
                .as_ref()
                .is_some_and(|previous| exact_roster_install(&rosters[index], previous)) =>
        {
            rosters[index] = payload.next.clone();
        }
        Some(_) => {
            return Err(AppError::StorageCorrupt {
                message: "Agent roster recovery found changed ledger metadata".into(),
            });
        }
        None if payload.previous.is_none() => rosters.push(payload.next.clone()),
        None => {
            return Err(AppError::StorageCorrupt {
                message: "Agent roster recovery lost its previous ledger metadata".into(),
            });
        }
    }
    rosters.sort_by(|left, right| {
        left.project_path
            .cmp(&right.project_path)
            .then_with(|| left.tool.cmp(&right.tool))
    });
    save_rosters_after_filesystem(state, &rosters, operation_id).await
}

async fn recover_roster_write_operation(
    state: &AppState,
    database: &crate::state_db::StateDatabase,
    operation: &crate::state_db::FilesystemOperation,
) -> Result<(), AppError> {
    let payload = serde_json::from_value::<AgentRosterWriteOperation>(operation.payload.clone())
        .map_err(|_| AppError::StorageCorrupt {
            message: "Agent roster write recovery payload is invalid".into(),
        })?;
    validate_roster_record(&payload.next)?;
    if payload.previous.as_ref().is_some_and(|previous| {
        validate_roster_record(previous).is_err() || !same_roster_install(previous, &payload.next)
    }) {
        return Err(AppError::StorageCorrupt {
            message: "Agent roster write recovery identity changed".into(),
        });
    }
    canonical_registered_instruction_project(state, &payload.next.project_path).await?;
    let destination = PathBuf::from(&payload.next.dest);
    let staged = validate_roster_staging_path(&state.app_data_dir, &payload.staged_path)?;
    validate_recovery_roster_history(
        state,
        payload.previous.as_ref(),
        &destination,
        &payload.preservation,
    )
    .await?;
    let destination_hash = recovery_file_hash(&destination)?;
    match operation.phase {
        crate::state_db::FilesystemOperationPhase::Prepared => {
            if destination_hash.as_deref() != Some(payload.next.rendered_hash.as_str()) {
                let matches_previous = payload.previous.is_some()
                    && destination_hash.as_deref()
                        == roster_preservation_hash(&payload.preservation);
                let safe_missing = payload.previous.is_some()
                    && matches!(
                        payload.preservation,
                        AgentRosterPreservation::AbsentBeforeMutation
                    )
                    && destination_hash.is_none();
                let safe_new = payload.previous.is_none()
                    && matches!(
                        payload.preservation,
                        AgentRosterPreservation::AbsentBeforeMutation
                    )
                    && destination_hash.is_none();
                if !matches_previous && !safe_missing && !safe_new {
                    return Err(AppError::StorageCorrupt {
                        message: "Agent roster recovery found changed destination content".into(),
                    });
                }
                let bytes = read_capped(&staged, MAX_INSTALLED_BYTES).await?;
                if render::sha256_hex(&bytes) != payload.next.rendered_hash {
                    return Err(AppError::StorageCorrupt {
                        message: "Agent roster recovery staging bytes changed".into(),
                    });
                }
                publish_roster_bytes(&destination, &bytes, payload.previous.is_some()).await?;
            }
            sync_parent_directory_entry(&destination)?;
            apply_recovered_roster_write(state, &operation.id, &payload).await?;
            if let Some(previous) = &payload.previous {
                commit_recovered_roster_retention(state, previous, &payload.preservation).await?;
            }
            database.commit_filesystem_operation(&operation.id).await?;
        }
        crate::state_db::FilesystemOperationPhase::FilesystemApplied => {
            if destination_hash.as_deref() != Some(payload.next.rendered_hash.as_str())
                || !load_rosters_for_state(state)
                    .await?
                    .iter()
                    .any(|record| exact_roster_install(record, &payload.next))
            {
                return Err(AppError::StorageCorrupt {
                    message: "Agent roster recovery found changed committed state".into(),
                });
            }
            if let Some(previous) = &payload.previous {
                commit_recovered_roster_retention(state, previous, &payload.preservation).await?;
            }
            database.commit_filesystem_operation(&operation.id).await?;
        }
        crate::state_db::FilesystemOperationPhase::Committed => {}
    }
    let _ = tokio::fs::remove_file(staged).await;
    Ok(())
}

async fn recover_roster_move_operation(
    state: &AppState,
    database: &crate::state_db::StateDatabase,
    operation: &crate::state_db::FilesystemOperation,
) -> Result<(), AppError> {
    let payload = serde_json::from_value::<AgentRosterMoveOperation>(operation.payload.clone())
        .map_err(|_| AppError::StorageCorrupt {
            message: "Agent roster move recovery payload is invalid".into(),
        })?;
    let (previous_active, previous_disabled) =
        validated_recovery_roster_paths(state, &payload.previous).await?;
    let (next_active, next_disabled) =
        validated_recovery_roster_paths(state, &payload.next).await?;
    if !same_roster_install(&payload.previous, &payload.next)
        || payload.previous.members != payload.next.members
        || payload.previous.rendered_hash != payload.next.rendered_hash
    {
        return Err(AppError::StorageCorrupt {
            message: "Agent roster move recovery identity changed".into(),
        });
    }
    if previous_active != next_active || previous_disabled != next_disabled {
        return Err(AppError::StorageCorrupt {
            message: "Agent roster move recovery destinations disagree".into(),
        });
    }
    let active = next_active;
    let disabled = next_disabled;
    if payload.history.rendered_hash != payload.previous.rendered_hash {
        return Err(AppError::StorageCorrupt {
            message: "Agent roster move recovery preservation hash changed".into(),
        });
    }
    let (source, destination) = match operation.kind.as_str() {
        "agent_disable"
            if payload.previous.disabled_path.is_none() && payload.next.disabled_path.is_some() =>
        {
            (&active, &disabled)
        }
        "agent_enable"
            if payload.previous.disabled_path.is_some() && payload.next.disabled_path.is_none() =>
        {
            (&disabled, &active)
        }
        _ => {
            return Err(AppError::StorageCorrupt {
                message: "Agent roster move recovery transition is invalid".into(),
            });
        }
    };
    validate_recovery_roster_history(
        state,
        Some(&payload.previous),
        source,
        &AgentRosterPreservation::Snapshot(payload.history.clone()),
    )
    .await?;
    let source_hash = recovery_file_hash(source)?;
    let destination_hash = recovery_file_hash(destination)?;
    let moved = source_hash.is_none()
        && destination_hash.as_deref() == Some(payload.next.rendered_hash.as_str());
    let pending = source_hash.as_deref() == Some(payload.next.rendered_hash.as_str())
        && destination_hash.is_none();
    if !moved && !pending {
        return Err(AppError::StorageCorrupt {
            message: "Agent roster move recovery found changed or duplicate content".into(),
        });
    }
    if operation.phase == crate::state_db::FilesystemOperationPhase::Prepared {
        if pending {
            move_managed_artifacts(
                std::slice::from_ref(source),
                std::slice::from_ref(destination),
                std::slice::from_ref(&payload.next.rendered_hash),
            )?;
        }
        let mut rosters = load_rosters_for_state(state).await?;
        let index = roster_record_index(
            &rosters,
            &payload.previous.tool,
            &payload.previous.project_path,
        )?;
        if exact_roster_install(&rosters[index], &payload.previous) {
            rosters[index] = payload.next.clone();
        } else if !exact_roster_install(&rosters[index], &payload.next) {
            return Err(AppError::StorageCorrupt {
                message: "Agent roster move recovery found changed ledger metadata".into(),
            });
        }
        save_rosters_after_filesystem(state, &rosters, &operation.id).await?;
        commit_recovered_roster_retention(
            state,
            &payload.previous,
            &AgentRosterPreservation::Snapshot(payload.history.clone()),
        )
        .await?;
        database.commit_filesystem_operation(&operation.id).await
    } else if operation.phase == crate::state_db::FilesystemOperationPhase::FilesystemApplied {
        if pending
            || !load_rosters_for_state(state)
                .await?
                .iter()
                .any(|record| exact_roster_install(record, &payload.next))
        {
            Err(AppError::StorageCorrupt {
                message: "Agent roster move recovery found changed committed state".into(),
            })
        } else {
            commit_recovered_roster_retention(
                state,
                &payload.previous,
                &AgentRosterPreservation::Snapshot(payload.history.clone()),
            )
            .await?;
            database.commit_filesystem_operation(&operation.id).await
        }
    } else {
        Ok(())
    }
}

async fn recover_roster_uninstall_operation(
    state: &AppState,
    database: &crate::state_db::StateDatabase,
    operation: &crate::state_db::FilesystemOperation,
) -> Result<(), AppError> {
    let payload =
        serde_json::from_value::<AgentRosterUninstallOperation>(operation.payload.clone())
            .map_err(|_| AppError::StorageCorrupt {
                message: "Agent roster uninstall recovery payload is invalid".into(),
            })?;
    let (active, disabled) = validated_recovery_roster_paths(state, &payload.previous).await?;
    let path = if payload.previous.disabled_path.is_some() {
        disabled
    } else {
        active
    };
    validate_recovery_roster_history(state, Some(&payload.previous), &path, &payload.preservation)
        .await?;
    let current_hash = recovery_file_hash(&path)?;
    if operation.phase == crate::state_db::FilesystemOperationPhase::Prepared {
        if let Some(hash) = current_hash {
            if Some(hash.as_str()) != roster_preservation_hash(&payload.preservation) {
                return Err(AppError::StorageCorrupt {
                    message: "Agent roster uninstall recovery found changed content".into(),
                });
            }
            tokio::fs::remove_file(&path)
                .await
                .map_err(|error| AppError::Io {
                    message: format!("recover Agent roster uninstall: {error}"),
                })?;
        }
        sync_parent_directory_entry(&path)?;
        let mut rosters = load_rosters_for_state(state).await?;
        if let Some(index) = rosters
            .iter()
            .position(|record| same_roster_install(record, &payload.previous))
        {
            if !exact_roster_install(&rosters[index], &payload.previous) {
                return Err(AppError::StorageCorrupt {
                    message: "Agent roster uninstall recovery found changed ledger metadata".into(),
                });
            }
            rosters.remove(index);
        }
        save_rosters_after_filesystem(state, &rosters, &operation.id).await?;
        commit_recovered_roster_retention(state, &payload.previous, &payload.preservation).await?;
        database.commit_filesystem_operation(&operation.id).await
    } else if operation.phase == crate::state_db::FilesystemOperationPhase::FilesystemApplied {
        if current_hash.is_some()
            || load_rosters_for_state(state)
                .await?
                .iter()
                .any(|record| same_roster_install(record, &payload.previous))
        {
            Err(AppError::StorageCorrupt {
                message: "Agent roster uninstall recovery found changed committed state".into(),
            })
        } else {
            commit_recovered_roster_retention(state, &payload.previous, &payload.preservation)
                .await?;
            database.commit_filesystem_operation(&operation.id).await
        }
    } else {
        Ok(())
    }
}

async fn recover_roster_retention_operation(
    state: &AppState,
    database: &crate::state_db::StateDatabase,
    operation: &crate::state_db::FilesystemOperation,
) -> Result<(), AppError> {
    let payload =
        serde_json::from_value::<AgentRosterRetentionOperation>(operation.payload.clone())
            .map_err(|_| AppError::StorageCorrupt {
                message: "Agent roster retention recovery payload is invalid".into(),
            })?;
    validate_roster_record(&payload.previous)?;
    let mut preserved = payload.previous.clone();
    preserved.rendered_hash = payload.history.rendered_hash.clone();
    history::recover_roster_publication(
        &state.app_data_dir,
        &roster_identity(&payload.previous),
        &payload.history.snapshot_id,
        &payload.history.retired_snapshot_ids,
        &payload.history.staging_path,
        payload.history.previous_index_hash.as_deref(),
        &payload.history.next_index_hash,
        &preserved,
    )
    .await?;
    history::commit_roster_retention(
        &state.app_data_dir,
        &roster_identity(&payload.previous),
        &payload.history.retired_snapshot_ids,
    )
    .await?;
    history::cleanup_roster_staging(&state.app_data_dir, &payload.history.staging_path).await?;
    if operation.phase == crate::state_db::FilesystemOperationPhase::Prepared {
        database.mark_filesystem_applied(&operation.id).await?;
    }
    if operation.phase != crate::state_db::FilesystemOperationPhase::Committed {
        database.commit_filesystem_operation(&operation.id).await?;
    }
    Ok(())
}

fn pending_roster_staging_path(
    operation: &crate::state_db::FilesystemOperation,
) -> Result<Option<String>, AppError> {
    let invalid = || AppError::StorageCorrupt {
        message: "Agent roster recovery payload is invalid during staging cleanup".into(),
    };
    match operation.kind.as_str() {
        "agent_install" | "agent_update"
            if operation
                .payload
                .get("next")
                .and_then(|next| next.get("members"))
                .is_some() =>
        {
            let payload =
                serde_json::from_value::<AgentRosterWriteOperation>(operation.payload.clone())
                    .map_err(|_| invalid())?;
            Ok(match payload.preservation {
                AgentRosterPreservation::AbsentBeforeMutation => None,
                AgentRosterPreservation::Snapshot(history) => Some(history.staging_path),
            })
        }
        "agent_disable" | "agent_enable"
            if operation
                .payload
                .get("next")
                .and_then(|next| next.get("members"))
                .is_some() =>
        {
            let payload =
                serde_json::from_value::<AgentRosterMoveOperation>(operation.payload.clone())
                    .map_err(|_| invalid())?;
            Ok(Some(payload.history.staging_path))
        }
        "agent_uninstall"
            if operation
                .payload
                .get("previous")
                .and_then(|previous| previous.get("members"))
                .is_some() =>
        {
            let payload =
                serde_json::from_value::<AgentRosterUninstallOperation>(operation.payload.clone())
                    .map_err(|_| invalid())?;
            Ok(match payload.preservation {
                AgentRosterPreservation::AbsentBeforeMutation => None,
                AgentRosterPreservation::Snapshot(history) => Some(history.staging_path),
            })
        }
        "agent_roster_retention" => {
            let payload =
                serde_json::from_value::<AgentRosterRetentionOperation>(operation.payload.clone())
                    .map_err(|_| invalid())?;
            Ok(Some(payload.history.staging_path))
        }
        _ => Ok(None),
    }
}

async fn recover_agent_operations_locked(state: &AppState) -> Result<(), AppError> {
    recover_unjournaled_roster_writes(state).await?;
    history::recover_unjournaled_roster_operations(state).await?;
    let Some(database) = state.completed_state_database().await? else {
        history::sweep_roster_staging(&state.app_data_dir, &[]).await?;
        return Ok(());
    };
    let pending = database.pending_filesystem_operations().await?;
    let mut retained_staging_paths = Vec::new();
    for operation in &pending {
        let path = match pending_roster_staging_path(operation) {
            Ok(path) => path,
            Err(error) => {
                database
                    .retain_filesystem_operation_error(&operation.id, &error.to_string())
                    .await?;
                return Err(error);
            }
        };
        if let Some(path) = path {
            if let Err(error) =
                history::validate_roster_staging_reference(&state.app_data_dir, &path)
            {
                database
                    .retain_filesystem_operation_error(&operation.id, &error.to_string())
                    .await?;
                return Err(error);
            }
            retained_staging_paths.push(path);
        }
    }
    history::sweep_roster_staging(&state.app_data_dir, &retained_staging_paths).await?;
    for operation in pending {
        if !matches!(operation.kind.as_str(), "agent_install" | "agent_update") {
            continue;
        }
        let result = if operation
            .payload
            .get("next")
            .and_then(|next| next.get("members"))
            .is_some()
        {
            recover_roster_write_operation(state, &database, &operation).await
        } else {
            recover_agent_install_operation(state, &database, &operation).await
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
            "agent_disable" | "agent_enable" => {
                if operation
                    .payload
                    .get("next")
                    .and_then(|next| next.get("members"))
                    .is_some()
                {
                    recover_roster_move_operation(state, &database, &operation).await
                } else {
                    recover_agent_move_operation(state, &database, &operation).await
                }
            }
            "agent_uninstall" => {
                if operation
                    .payload
                    .get("previous")
                    .and_then(|previous| previous.get("members"))
                    .is_some()
                {
                    recover_roster_uninstall_operation(state, &database, &operation).await
                } else {
                    recover_agent_uninstall_operation(state, &database, &operation).await
                }
            }
            _ => continue,
        };
        if let Err(error) = result {
            database
                .retain_filesystem_operation_error(&operation.id, &error.to_string())
                .await?;
            return Err(error);
        }
    }
    for operation in database.pending_filesystem_operations().await? {
        if operation.kind != "agent_roster_retention" {
            continue;
        }
        if let Err(error) = recover_roster_retention_operation(state, &database, &operation).await {
            database
                .retain_filesystem_operation_error(&operation.id, &error.to_string())
                .await?;
            return Err(error);
        }
    }
    Ok(())
}

pub(crate) async fn recover_agent_operations(state: &AppState) -> Result<(), AppError> {
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
    recover_agent_operations_locked(state).await
}

fn rollback_error(action: &str, original: AppError, rollback: AppError) -> AppError {
    AppError::Internal {
        message: format!("{action} failed: {original}; rollback failed: {rollback}"),
    }
}

/// Render + write the agent file(s) and build the ledger record. Pure of Tauri
/// (`home`/`project_root` passed explicitly) so the full render→write→record
/// path is unit-testable against a tempdir. Returns the record; caller persists
/// it to the ledger.
///
/// When `backup_dir` is `Some`, any existing dest whose bytes differ from the
/// incoming render is copied there before being overwritten — every destructive
/// write is reversible. `None` skips backups (only for callers that have already
/// guaranteed there's nothing to preserve).
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
async fn write_agent_files(
    agent: &crate::types::Agent,
    raw: &str,
    tool: &str,
    home: &Path,
    project_root: Option<&Path>,
    backup_dir: Option<&Path>,
    source_hash: &str,
    body_hash: &str,
    corpus_version: &str,
    installed_at: &str,
) -> Result<InstallRecord, AppError> {
    write_agent_files_to(
        agent,
        raw,
        tool,
        home,
        project_root,
        backup_dir,
        source_hash,
        body_hash,
        corpus_version,
        installed_at,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
async fn write_agent_files_to(
    agent: &crate::types::Agent,
    raw: &str,
    tool: &str,
    home: &Path,
    project_root: Option<&Path>,
    backup_dir: Option<&Path>,
    source_hash: &str,
    body_hash: &str,
    corpus_version: &str,
    installed_at: &str,
    preferred_dest: Option<&Path>,
) -> Result<InstallRecord, AppError> {
    let paths = install_target_paths(agent, Some(raw), tool, home, project_root, preferred_dest)?;
    write_agent_files_at(
        agent,
        raw,
        tool,
        project_root,
        backup_dir,
        source_hash,
        body_hash,
        corpus_version,
        installed_at,
        &paths,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn write_agent_files_at(
    agent: &crate::types::Agent,
    raw: &str,
    tool: &str,
    project_root: Option<&Path>,
    backup_dir: Option<&Path>,
    source_hash: &str,
    body_hash: &str,
    corpus_version: &str,
    installed_at: &str,
    paths: &[PathBuf],
) -> Result<InstallRecord, AppError> {
    let rendered = render::render_artifacts(agent, raw, tool)?;
    let manifest = artifact_manifest(paths, &rendered)?;
    let prior = capture_batch_files(paths).await?;
    for ((dest, artifact), entry) in paths.iter().zip(&rendered).zip(&manifest) {
        if let Some(bdir) = backup_dir {
            backup_if_differs(dest, artifact.content.as_bytes(), bdir, installed_at).await?;
        }
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| AppError::Io {
                    message: format!("create {}: {e}", parent.display()),
                })?;
        }
        let write = async {
            atomic_write(dest, artifact.content.as_bytes()).await?;
            let saved = read_capped(dest, MAX_INSTALLED_BYTES).await?;
            if render::sha256_hex(&saved) != entry.rendered_hash {
                return Err(AppError::Internal {
                    message: format!("verify Agent artifact {} failed", dest.display()),
                });
            }
            Ok(())
        }
        .await;
        if let Err(error) = write {
            return match restore_batch_files(&prior).await {
                Ok(()) => Err(error),
                Err(rollback) => Err(rollback_error("write Agent artifacts", error, rollback)),
            };
        }
    }
    let mut record = record_for(
        agent,
        &paths[0],
        tool,
        project_root,
        manifest[0].rendered_hash.clone(),
        source_hash,
        body_hash,
        corpus_version,
        installed_at,
    );
    record.artifacts = manifest;
    Ok(record)
}

fn install_target_paths(
    agent: &crate::types::Agent,
    raw: Option<&str>,
    tool: &str,
    home: &Path,
    project_root: Option<&Path>,
    preferred_dest: Option<&Path>,
) -> Result<Vec<PathBuf>, AppError> {
    let slug =
        if crate::registry::get(tool).and_then(|meta| meta.slug_from.as_deref()) == Some("name") {
            raw.map(|source| render::output_slug(agent, source, tool))
                .unwrap_or_else(|| render::slugify(&agent.name))
        } else {
            agent.slug.clone()
        };
    let mut paths = render::dests(tool, &slug, home, project_root)?;
    if let Some(preferred) = preferred_dest {
        if paths.len() == 1 {
            paths[0] = preferred.to_path_buf();
        } else if let Some(index) = paths.iter().position(|path| path == preferred) {
            paths.swap(0, index);
        }
    }
    Ok(paths)
}

// ---------- Reconciliation core (pure, testable) ----------

#[cfg(test)]
#[derive(Clone, Copy)]
struct ReconcileFacts<'a> {
    tracked: bool,
    destination_hash: Option<&'a str>,
    disabled_path: bool,
    disabled_hash: Option<&'a str>,
    rendered_hash: &'a str,
    installed_source_hash: &'a str,
    current_source_hash: Option<&'a str>,
}

#[cfg(test)]
fn classify_install(facts: ReconcileFacts<'_>) -> InstallState {
    if !facts.tracked {
        return InstallState::Foreign;
    }
    if facts.disabled_path {
        return if facts.destination_hash.is_none()
            && facts.disabled_hash == Some(facts.rendered_hash)
        {
            InstallState::Disabled
        } else if facts.disabled_hash.is_some() || facts.destination_hash.is_some() {
            InstallState::Modified
        } else if facts.current_source_hash.is_none() {
            InstallState::SourceUnavailable
        } else {
            InstallState::Missing
        };
    }
    if facts.current_source_hash.is_none() {
        return InstallState::SourceUnavailable;
    }
    match facts.destination_hash {
        None => InstallState::Missing,
        Some(hash) if hash != facts.rendered_hash => InstallState::Modified,
        Some(_) if facts.current_source_hash != Some(facts.installed_source_hash) => {
            InstallState::Outdated
        }
        Some(_) => InstallState::Current,
    }
}

fn classify_artifact_install(
    active_hashes: &[Option<&str>],
    disabled_hashes: &[Option<&str>],
    rendered_hashes: &[&str],
    disabled: bool,
    installed_source_hash: &str,
    current_source_hash: Option<&str>,
) -> InstallState {
    if active_hashes.len() != rendered_hashes.len()
        || disabled_hashes.len() != rendered_hashes.len()
        || rendered_hashes.is_empty()
    {
        return InstallState::Modified;
    }
    if disabled {
        if active_hashes.iter().all(Option::is_none)
            && disabled_hashes
                .iter()
                .zip(rendered_hashes)
                .all(|(actual, expected)| actual.as_deref() == Some(*expected))
        {
            return InstallState::Disabled;
        }
        if active_hashes.iter().any(Option::is_some)
            || disabled_hashes.iter().any(|hash| hash.is_some())
        {
            return InstallState::Modified;
        }
        return if current_source_hash.is_none() {
            InstallState::SourceUnavailable
        } else {
            InstallState::Missing
        };
    }
    if current_source_hash.is_none() {
        return InstallState::SourceUnavailable;
    }
    if active_hashes
        .iter()
        .zip(rendered_hashes)
        .any(|(actual, expected)| actual.is_some_and(|hash| hash != *expected))
    {
        return InstallState::Modified;
    }
    if active_hashes.iter().any(Option::is_none) {
        return InstallState::Missing;
    }
    if current_source_hash != Some(installed_source_hash) {
        InstallState::Outdated
    } else {
        InstallState::Current
    }
}

async fn record_reconcile_hashes(
    state: &AppState,
    record: &InstallRecord,
) -> Result<(Vec<String>, Vec<Option<String>>, Vec<Option<String>>), AppError> {
    let active = resolved_record_paths(state, record).await?;
    let manifest = record_artifacts(record);
    let expected = if record.artifacts.is_empty() {
        vec![record.rendered_hash.clone(); active.len()]
    } else {
        manifest
            .iter()
            .map(|artifact| artifact.rendered_hash.clone())
            .collect()
    };
    let mut active_hashes = Vec::with_capacity(active.len());
    let mut disabled_hashes = Vec::with_capacity(active.len());
    for path in &active {
        active_hashes.push(recovery_file_hash(path)?);
        disabled_hashes.push(recovery_file_hash(&disabled_destination(path)?)?);
    }
    Ok((expected, active_hashes, disabled_hashes))
}

fn find_agent_package<'a>(
    sources: &'a [AgentSourceResult],
    record: &InstallRecord,
) -> Option<&'a crate::types::AgentPackageResult> {
    sources
        .iter()
        .find(|result| result.source.id == record.source_id)
        .and_then(|result| {
            result.agents.iter().find(|package| {
                package.reference.source_id == record.source_id
                    && package.reference.relative_path == record.relative_path
            })
        })
}

fn resolve_reference_request(
    sources: &[AgentSourceResult],
    source_id: Option<&str>,
    relative_path: Option<&str>,
    legacy_slug: Option<&str>,
) -> Result<AgentReference, AppError> {
    match (source_id, relative_path, legacy_slug) {
        (Some(source_id), Some(relative_path), None) => {
            crate::library::validate_reference(source_id, relative_path)?;
            Ok(AgentReference {
                source_id: source_id.into(),
                relative_path: relative_path.into(),
            })
        }
        (None, None, Some(slug)) => {
            let matches = sources
                .iter()
                .filter(|source| source.source.id == crate::agents::BUILTIN_AGENT_SOURCE_ID)
                .flat_map(|source| &source.agents)
                .filter(|package| {
                    package.installable
                        && package
                            .agent
                            .as_ref()
                            .is_some_and(|agent| agent.slug == slug)
                })
                .collect::<Vec<_>>();
            if matches.len() != 1 {
                return Err(AppError::InvalidArgument {
                    message: format!(
                        "legacy Agent slug must resolve to one built-in package: {slug}"
                    ),
                });
            }
            Ok(matches[0].reference.clone())
        }
        _ => Err(AppError::InvalidArgument {
            message: "provide sourceId + relativePath, or one legacy slug".into(),
        }),
    }
}

async fn resolve_command_reference<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    source_id: Option<&str>,
    relative_path: Option<&str>,
    legacy_slug: Option<&str>,
) -> Result<AgentReference, AppError> {
    corpus::ensure_corpus(app, state).await?;
    let sources = crate::agents::inspect_agent_sources(&state.app_data_dir).await?;
    resolve_reference_request(&sources, source_id, relative_path, legacy_slug)
}

fn ensure_destinations_available(
    records: &[InstallRecord],
    reference: &AgentReference,
    tool: &str,
    project_path: Option<&str>,
    destinations: &[PathBuf],
    allow_untracked: bool,
) -> Result<(), AppError> {
    let same_identity = |record: &InstallRecord| {
        record.source_id == reference.source_id
            && record.relative_path == reference.relative_path
            && record.tool == tool
            && record.project_path.as_deref() == project_path
    };
    for record in records.iter().filter(|record| {
        record.tool == tool
            && record.project_path.as_deref() == project_path
            && !same_identity(record)
    }) {
        if record_artifacts(record).iter().any(|artifact| {
            destinations
                .iter()
                .any(|path| path == Path::new(&artifact.dest))
        }) {
            return Err(AppError::InvalidArgument {
                message: format!(
                    "Agent destination collision: {}:{} conflicts with {}:{} at {}",
                    record.source_id,
                    record.relative_path,
                    reference.source_id,
                    reference.relative_path,
                    record.dest
                ),
            });
        }
    }
    let owned = records.iter().any(same_identity);
    if !owned && !allow_untracked {
        if let Some(path) = destinations.iter().find(|path| path.exists()) {
            return Err(AppError::InvalidArgument {
                message: format!(
                    "Agent destination contains untracked content: {}",
                    path.display()
                ),
            });
        }
    }
    Ok(())
}

fn update_policy_allows(
    policy: crate::types::AgentUpdatePolicy,
    confirmed: bool,
    installed_capabilities: &[String],
    proposed_capabilities: &[String],
    installed_publisher_key: Option<&str>,
    proposed_publisher_key: Option<&str>,
    proposed_publisher_trusted: bool,
) -> bool {
    if policy == crate::types::AgentUpdatePolicy::Pin {
        return false;
    }
    let broadened = proposed_capabilities
        .iter()
        .any(|capability| !installed_capabilities.contains(capability));
    if broadened && !confirmed {
        return false;
    }
    match policy {
        crate::types::AgentUpdatePolicy::Pin => false,
        crate::types::AgentUpdatePolicy::Notify
        | crate::types::AgentUpdatePolicy::ReviewScripts => confirmed,
        crate::types::AgentUpdatePolicy::AutoTrusted => {
            confirmed
                || (!broadened
                    && proposed_publisher_trusted
                    && installed_publisher_key.is_some()
                    && installed_publisher_key == proposed_publisher_key)
        }
    }
}

fn agent_publisher_is_trusted(
    library: &crate::types::AgentLibraryState,
    package: &crate::types::AgentPackageResult,
) -> bool {
    package
        .publisher
        .as_ref()
        .zip(package.publisher_key.as_ref())
        .is_some_and(|(name, key)| {
            library.publisher_trust.iter().any(|trust| {
                trust.name == *name && trust.public_key == *key && trust.trusted && !trust.revoked
            })
        })
}

fn package_by_reference<'a>(
    sources: &'a [AgentSourceResult],
    reference: &AgentReference,
) -> Option<&'a crate::types::AgentPackageResult> {
    sources
        .iter()
        .find(|source| source.source.id == reference.source_id)
        .and_then(|source| {
            source
                .agents
                .iter()
                .find(|package| package.reference == *reference)
        })
}

async fn build_mutation_plan(
    state: &AppState,
    roots: Vec<AgentReference>,
    tool: Tool,
    project_path: Option<String>,
    operation: &str,
    include_dependencies: bool,
) -> Result<AgentMutationPlan, AppError> {
    if !matches!(operation, "install" | "update" | "uninstall") {
        return Err(AppError::InvalidArgument {
            message: format!("unknown Agent mutation operation: {operation}"),
        });
    }
    let sources = crate::agents::inspect_agent_sources(&state.app_data_dir).await?;
    let library = crate::agents::organize::list(state).await?;
    let mut warnings = Vec::new();
    let mut blockers = Vec::new();
    let ordered = if include_dependencies && operation != "uninstall" {
        let resolution =
            crate::agents::resolve_agent_dependencies(&sources, &roots, &library.preferred_sources);
        warnings.extend(resolution.warnings);
        blockers.extend(resolution.blockers);
        resolution.ordered
    } else {
        let mut ordered = roots.clone();
        ordered.sort();
        ordered.dedup();
        ordered
    };
    let home = tool_home(state, &tool).await?;
    let project = project_path.as_deref().map(Path::new);
    let records = load_ledger_for_state(state).await?;
    let mut agents = Vec::with_capacity(ordered.len());

    for reference in ordered {
        let existing = records.iter().find(|record| {
            record.source_id == reference.source_id
                && record.relative_path == reference.relative_path
                && record.tool == tool
                && record.project_path == project_path
        });
        if operation == "uninstall" {
            if let Some(record) = existing {
                agents.push(AgentPlanItem {
                    reference: reference.clone(),
                    name: record.slug.clone(),
                    source_hash: record.source_snapshot_hash.clone(),
                    dependency: !roots.contains(&reference),
                    destination: record.dest.clone(),
                    rendered_file_count: render::dests(&tool, &record.slug, &home, project)
                        .map(|paths| paths.len() as u32)
                        .unwrap_or(1),
                    capabilities: record.capabilities.clone(),
                });
            } else {
                blockers.push(format!(
                    "Agent is not tracked for uninstall: {}:{}",
                    reference.source_id, reference.relative_path
                ));
            }
            continue;
        }

        let Some(package) = package_by_reference(&sources, &reference) else {
            blockers.push(format!(
                "Agent source is unavailable: {}:{}",
                reference.source_id, reference.relative_path
            ));
            continue;
        };
        let Some(agent) = package.agent.as_ref() else {
            blockers.push(format!(
                "Agent metadata is invalid: {}:{}",
                reference.source_id, reference.relative_path
            ));
            continue;
        };
        let raw = match crate::agents::read_agent_text(&state.app_data_dir, &reference).await {
            Ok(raw) => raw,
            Err(error) => {
                blockers.push(error.to_string());
                continue;
            }
        };
        let paths = match existing {
            Some(record) => resolved_record_paths(state, record).await,
            None => install_target_paths(agent, Some(&raw), &tool, &home, project, None),
        };
        let paths = match paths {
            Ok(paths) => paths,
            Err(error) => {
                blockers.push(error.to_string());
                continue;
            }
        };
        if operation == "install" {
            if let Err(error) = ensure_destinations_available(
                &records,
                &reference,
                &tool,
                project_path.as_deref(),
                &paths,
                false,
            ) {
                blockers.push(error.to_string());
            }
        } else if existing.is_none() {
            blockers.push(format!(
                "Agent is not tracked for update: {}:{}",
                reference.source_id, reference.relative_path
            ));
        }
        if operation == "update" {
            if let Some(existing) = existing {
                let policy = library
                    .update_policies
                    .iter()
                    .find(|entry| entry.agent == reference)
                    .map(|entry| entry.policy)
                    .unwrap_or(crate::types::AgentUpdatePolicy::Notify);
                if policy == crate::types::AgentUpdatePolicy::Pin {
                    blockers.push(format!("Agent update is pinned: {}", agent.name));
                } else if !update_policy_allows(
                    policy,
                    false,
                    &existing.capabilities,
                    &package.capabilities,
                    existing.publisher_key.as_deref(),
                    package.publisher_key.as_deref(),
                    package.publisher_verified && agent_publisher_is_trusted(&library, package),
                ) {
                    warnings.push(format!(
                        "Agent update requires explicit review: {} ({policy:?})",
                        agent.name
                    ));
                }
            }
        }
        agents.push(AgentPlanItem {
            reference: reference.clone(),
            name: agent.name.clone(),
            source_hash: package.source_hash.clone(),
            dependency: !roots.contains(&reference),
            destination: paths[0].to_string_lossy().into_owned(),
            rendered_file_count: paths.len() as u32,
            capabilities: package.capabilities.clone(),
        });
        let _ = raw;
    }
    if tool == "openclaw" {
        warnings.push(
            "OpenClaw installs workspace files only. Register the agent and restart OpenClaw separately; Agency Agents will not run the OpenClaw CLI."
                .into(),
        );
    }
    warnings.sort();
    warnings.dedup();
    blockers.sort();
    blockers.dedup();
    let mut plan = AgentMutationPlan {
        revision: String::new(),
        operation: operation.into(),
        tool,
        scope: render::scope_for(project),
        project_path,
        agents,
        warnings,
        blockers,
        rollback_available: true,
    };
    plan.revision =
        render::sha256_hex(
            &serde_json::to_vec(&plan).map_err(|error| AppError::Internal {
                message: format!("serialize Agent mutation plan: {error}"),
            })?,
        );
    Ok(plan)
}

fn roster_tool(tool: &str) -> Result<(), AppError> {
    if registry::get(tool)
        .is_some_and(|meta| meta.install_kind.as_deref() == Some("roster") && meta.installable())
    {
        Ok(())
    } else {
        Err(AppError::InvalidArgument {
            message: "Agent roster target must be Aider or Windsurf".into(),
        })
    }
}

fn validate_roster_references(
    mut references: Vec<AgentReference>,
) -> Result<Vec<AgentReference>, AppError> {
    if references.len() < 2 || references.len() > MAX_ROSTER_MEMBERS {
        return Err(AppError::InvalidArgument {
            message: format!(
                "Agent roster must contain between 2 and {MAX_ROSTER_MEMBERS} exact references"
            ),
        });
    }
    for reference in &references {
        crate::library::validate_reference(&reference.source_id, &reference.relative_path)?;
    }
    references.sort();
    references.dedup();
    if references.len() < 2 {
        return Err(AppError::InvalidArgument {
            message: "Agent roster must contain at least two distinct exact references".into(),
        });
    }
    Ok(references)
}

async fn current_roster_state(
    state: &AppState,
    record: &AgentRosterInstallRecord,
) -> Result<InstallState, AppError> {
    let active = Path::new(&record.dest);
    let disabled = disabled_destination(active)?;
    let active_hash = recovery_file_hash(active)?;
    let disabled_hash = recovery_file_hash(&disabled)?;
    if record.disabled_path.is_some() {
        return Ok(match (active_hash.as_deref(), disabled_hash.as_deref()) {
            (None, Some(hash)) if hash == record.rendered_hash => InstallState::Disabled,
            (Some(_), _) | (_, Some(_)) => InstallState::Modified,
            _ => InstallState::Missing,
        });
    }
    if active_hash
        .as_deref()
        .is_some_and(|hash| hash != record.rendered_hash)
    {
        return Ok(InstallState::Modified);
    }
    if active_hash.is_none() {
        return Ok(InstallState::Missing);
    }
    let sources = crate::agents::inspect_agent_sources(&state.app_data_dir).await?;
    for member in &record.members {
        let Some(package) = package_by_reference(&sources, &member.reference) else {
            return Ok(InstallState::SourceUnavailable);
        };
        if package.source_hash != member.source_hash {
            return Ok(InstallState::Outdated);
        }
    }
    Ok(InstallState::Current)
}

async fn build_roster_plan(
    state: &AppState,
    references: Vec<AgentReference>,
    tool: Tool,
    project_path: String,
    operation: &str,
) -> Result<AgentRosterMutationPlan, AppError> {
    roster_tool(&tool)?;
    if !matches!(operation, "install" | "update" | "uninstall") {
        return Err(AppError::InvalidArgument {
            message: format!("unknown Agent roster operation: {operation}"),
        });
    }
    let project = canonical_registered_instruction_project(state, &project_path).await?;
    let project_path = project.to_string_lossy().into_owned();
    let destination = render::dests(&tool, "roster", Path::new("/"), Some(&project))?
        .into_iter()
        .next()
        .ok_or_else(|| AppError::InvalidArgument {
            message: "Agent roster target has no destination".into(),
        })?;
    let existing = load_rosters_for_state(state)
        .await?
        .into_iter()
        .find(|record| record.tool == tool && record.project_path == project_path);
    let destination_observation = observe_roster_destination(&destination).await?;
    let mut blockers = Vec::new();
    let state_value = match &existing {
        Some(record) => Some(current_roster_state(state, record).await?),
        None => None,
    };
    if operation == "install" && existing.is_none() && destination.exists() {
        blockers.push(format!(
            "Refusing to overwrite foreign project rules: {}",
            destination.display()
        ));
    }
    if operation != "install" && existing.is_none() {
        blockers.push(format!("Agent roster is not tracked for {operation}"));
    }
    if operation == "update"
        && existing
            .as_ref()
            .is_some_and(|record| record.disabled_path.is_some())
    {
        blockers.push("Enable the Agent roster before updating it".into());
    }

    let references = validate_roster_references(references)?;
    let members = if operation == "uninstall" {
        match &existing {
            Some(record) => {
                let tracked = record
                    .members
                    .iter()
                    .map(|member| member.reference.clone())
                    .collect::<Vec<_>>();
                if references != tracked {
                    blockers.push(
                        "Selected Agent roster membership changed; review the tracked roster"
                            .into(),
                    );
                }
                record.members.clone()
            }
            None => Vec::new(),
        }
    } else {
        let sources = crate::agents::inspect_agent_sources(&state.app_data_dir).await?;
        let mut members = Vec::with_capacity(references.len());
        for reference in references {
            let Some(package) = package_by_reference(&sources, &reference) else {
                blockers.push(format!(
                    "Agent source is unavailable: {}:{}",
                    reference.source_id, reference.relative_path
                ));
                continue;
            };
            let Some(agent) = package.agent.as_ref() else {
                blockers.push(format!(
                    "Agent metadata is invalid: {}:{}",
                    reference.source_id, reference.relative_path
                ));
                continue;
            };
            members.push(AgentRosterMember {
                reference,
                name: agent.name.clone(),
                source_hash: package.source_hash.clone(),
            });
        }
        members
    };
    blockers.sort();
    blockers.dedup();
    let mut plan = AgentRosterMutationPlan {
        revision: String::new(),
        operation: operation.into(),
        tool,
        scope: crate::types::Scope::Project,
        project_path,
        destination: destination.to_string_lossy().into_owned(),
        members,
        state: state_value,
        destination_observation,
        warnings: Vec::new(),
        blockers,
        rollback_available: existing.is_some(),
    };
    plan.revision =
        render::sha256_hex(
            &serde_json::to_vec(&plan).map_err(|error| AppError::Internal {
                message: format!("serialize Agent roster plan: {error}"),
            })?,
        );
    Ok(plan)
}

async fn observe_roster_path(path: &Path) -> Result<AgentRosterPathObservation, AppError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AgentRosterPathObservation {
                kind: "missing".into(),
                hash: None,
            });
        }
        Err(error) => {
            return Err(AppError::Io {
                message: format!(
                    "inspect Agent roster destination {}: {error}",
                    path.display()
                ),
            });
        }
    };
    if metadata.file_type().is_symlink() || crate::skills::metadata_is_reparse_point(&metadata) {
        return Ok(AgentRosterPathObservation {
            kind: "symlink".into(),
            hash: None,
        });
    }
    if metadata.is_file() {
        let bytes = read_capped(path, MAX_INSTALLED_BYTES).await?;
        return Ok(AgentRosterPathObservation {
            kind: "file".into(),
            hash: Some(render::sha256_hex(&bytes)),
        });
    }
    Ok(AgentRosterPathObservation {
        kind: "other".into(),
        hash: None,
    })
}

async fn observe_roster_destination(
    active: &Path,
) -> Result<AgentRosterDestinationObservation, AppError> {
    let disabled = disabled_destination(active)?;
    Ok(AgentRosterDestinationObservation {
        active: observe_roster_path(active).await?,
        disabled: observe_roster_path(&disabled).await?,
    })
}

fn require_roster_revision(plan: &AgentRosterMutationPlan, revision: &str) -> Result<(), AppError> {
    if revision == plan.revision {
        Ok(())
    } else {
        Err(AppError::InvalidArgument {
            message: "Agent roster plan is stale; review the current plan again".into(),
        })
    }
}

async fn render_roster_members(
    state: &AppState,
    members: &[AgentRosterMember],
    tool: &str,
) -> Result<String, AppError> {
    let mut sources = Vec::with_capacity(members.len());
    for member in members {
        let raw = crate::agents::read_agent_text(&state.app_data_dir, &member.reference).await?;
        if render::sha256_hex(raw.as_bytes()) != member.source_hash {
            return Err(AppError::InvalidArgument {
                message: format!(
                    "Agent roster source changed after review: {}:{}",
                    member.reference.source_id, member.reference.relative_path
                ),
            });
        }
        sources.push(raw);
    }
    let rendered = render::render_roster(&sources, tool)?;
    if rendered.len() as u64 > MAX_INSTALLED_BYTES {
        return Err(AppError::InvalidArgument {
            message: format!("Agent roster exceeds the {MAX_INSTALLED_BYTES}-byte install limit"),
        });
    }
    Ok(rendered)
}

fn roster_staging_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir
        .join("state/roster-staging")
        .join(format!("{}.bin", uuid::Uuid::new_v4()))
}

async fn stage_roster_bytes(app_data_dir: &Path, bytes: &[u8]) -> Result<PathBuf, AppError> {
    let path = roster_staging_path(app_data_dir);
    tokio::fs::create_dir_all(path.parent().expect("roster staging path has parent"))
        .await
        .map_err(|error| AppError::Io {
            message: format!("create Agent roster staging directory: {error}"),
        })?;
    sync_parent_directory_entry(path.parent().expect("roster staging path has parent"))?;
    atomic_write(&path, bytes).await?;
    let stored = read_capped(&path, MAX_INSTALLED_BYTES).await?;
    if stored != bytes {
        return Err(AppError::Internal {
            message: "verify Agent roster staging bytes failed".into(),
        });
    }
    Ok(path)
}

async fn publish_roster_bytes(
    destination: &Path,
    bytes: &[u8],
    replace_managed: bool,
) -> Result<(), AppError> {
    if replace_managed {
        return atomic_write(destination, bytes).await;
    }
    let parent = destination
        .parent()
        .ok_or_else(|| AppError::InvalidArgument {
            message: "Agent roster destination has no parent".into(),
        })?;
    let filename = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| AppError::InvalidArgument {
            message: "Agent roster destination filename is invalid".into(),
        })?;
    let staged = parent.join(format!(
        ".{filename}.agency-roster-{}.tmp",
        uuid::Uuid::new_v4()
    ));
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)
        .await
        .map_err(|error| AppError::Io {
            message: format!(
                "stage Agent roster beside project rules {}: {error}",
                staged.display()
            ),
        })?;
    let result = async {
        file.write_all(bytes).await.map_err(|error| AppError::Io {
            message: format!(
                "write Agent roster staging file {}: {error}",
                staged.display()
            ),
        })?;
        file.sync_all().await.map_err(|error| AppError::Io {
            message: format!(
                "sync Agent roster staging file {}: {error}",
                staged.display()
            ),
        })?;
        drop(file);
        tokio::fs::hard_link(&staged, destination)
            .await
            .map_err(|error| AppError::Io {
                message: format!(
                    "publish Agent roster without replacing project rules {}: {error}",
                    destination.display()
                ),
            })
    }
    .await;
    let _ = tokio::fs::remove_file(&staged).await;
    result
}

async fn apply_roster_plan(
    state: &AppState,
    reviewed: &AgentRosterMutationPlan,
    revision: &str,
    confirmed: bool,
) -> Result<AgentRosterInstallRecord, AppError> {
    if !confirmed {
        return Err(AppError::InvalidArgument {
            message: "Agent roster mutation requires explicit confirmation".into(),
        });
    }
    if !matches!(reviewed.operation.as_str(), "install" | "update") {
        return Err(AppError::InvalidArgument {
            message: "Agent roster write plan has the wrong operation".into(),
        });
    }
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
    recover_agent_operations_locked(state).await?;
    let current = build_roster_plan(
        state,
        reviewed
            .members
            .iter()
            .map(|member| member.reference.clone())
            .collect(),
        reviewed.tool.clone(),
        reviewed.project_path.clone(),
        &reviewed.operation,
    )
    .await?;
    require_roster_revision(&current, revision)?;
    if !current.blockers.is_empty() {
        return Err(AppError::InvalidArgument {
            message: format!(
                "Agent roster plan is blocked: {}",
                current.blockers.join("; ")
            ),
        });
    }
    let rendered = render_roster_members(state, &current.members, &current.tool).await?;
    let rendered_hash = render::sha256_hex(rendered.as_bytes());
    let destination = PathBuf::from(&current.destination);
    let mut rosters = load_rosters_for_state(state).await?;
    let prior_rosters = rosters.clone();
    let existing_index = rosters.iter().position(|record| {
        record.tool == current.tool && record.project_path == current.project_path
    });
    let previous = existing_index.map(|index| rosters[index].clone());
    match std::fs::symlink_metadata(&destination) {
        Ok(metadata)
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || crate::skills::metadata_is_reparse_point(&metadata) =>
        {
            return Err(AppError::InvalidArgument {
                message: "Agent roster destination must be a regular file".into(),
            });
        }
        Ok(_) if previous.is_none() => {
            return Err(AppError::InvalidArgument {
                message: format!(
                    "Refusing to overwrite foreign project rules: {}",
                    destination.display()
                ),
            });
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(AppError::Io {
                message: format!("inspect Agent roster destination: {error}"),
            });
        }
    }
    let prior = capture_batch_files(std::slice::from_ref(&destination)).await?;
    let next = AgentRosterInstallRecord {
        tool: current.tool.clone(),
        scope: crate::types::Scope::Project,
        project_path: current.project_path.clone(),
        dest: current.destination.clone(),
        members: current.members.clone(),
        rendered_hash,
        disabled_path: None,
        installed_at: now_iso(),
    };
    validate_roster_record(&next)?;
    let staged = stage_roster_bytes(&state.app_data_dir, rendered.as_bytes()).await?;
    let observation = observe_roster_destination(&destination).await;
    if !matches!(&observation, Ok(value) if *value == current.destination_observation) {
        let _ = tokio::fs::remove_file(&staged).await;
        return match observation {
            Ok(_) => Err(AppError::InvalidArgument {
                message: "Agent roster destination changed after review".into(),
            }),
            Err(error) => Err(error),
        };
    }
    let database = state.completed_state_database().await?;
    let mut history_mutation = match &previous {
        Some(record) => begin_roster_preservation(state, &destination, record, None).await?,
        None => None,
    };
    let prior_hash = prior[0].bytes.as_deref().map(render::sha256_hex);
    let preservation = roster_preservation(history_mutation.as_ref());
    if roster_preservation_hash(&preservation) != prior_hash.as_deref() {
        let error = rollback_unjournaled_roster_history(
            &mut history_mutation,
            "preserve Agent roster",
            AppError::InvalidArgument {
                message: "Agent roster destination changed during preservation".into(),
            },
        )
        .await;
        let _ = tokio::fs::remove_file(&staged).await;
        return Err(error);
    }
    let operation = if let Some(database) = &database {
        match database
            .prepare_filesystem_operation(
                if previous.is_some() {
                    "agent_update"
                } else {
                    "agent_install"
                },
                &AgentRosterWriteOperation {
                    previous: previous.clone(),
                    next: next.clone(),
                    staged_path: staged.to_string_lossy().into_owned(),
                    preservation,
                },
            )
            .await
        {
            Ok(operation) => Some(operation),
            Err(error) => {
                let history = rollback_roster_history(&mut history_mutation).await;
                let _ = tokio::fs::remove_file(&staged).await;
                return match history {
                    Ok(()) => Err(error),
                    Err(rollback) => Err(rollback_error("prepare Agent roster", error, rollback)),
                };
            }
        }
    } else {
        None
    };
    let history_ready = async {
        publish_roster_history_if(&mut history_mutation, database.is_some()).await?;
        prepare_unjournaled_roster_operation(
            &mut history_mutation,
            database.is_none(),
            previous.as_ref(),
            Some(&next),
        )
        .await?;
        prepare_unjournaled_roster_write(
            state,
            database.is_none() && history_mutation.is_none(),
            previous.as_ref(),
            &next,
            &staged,
        )
        .await
    }
    .await;
    if let Err(error) = history_ready {
        let history = rollback_roster_history(&mut history_mutation).await;
        close_prepared_roster_failure(database.as_ref(), operation.as_ref(), &history).await?;
        cleanup_roster_staging_artifacts(state, &staged).await;
        return match history {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error(
                "publish Agent roster history",
                error,
                rollback,
            )),
        };
    }
    let write = async {
        backup_if_differs(
            &destination,
            rendered.as_bytes(),
            &backups_dir_for(&state.app_data_dir),
            &now_iso(),
        )
        .await?;
        publish_roster_bytes(&destination, rendered.as_bytes(), previous.is_some()).await?;
        let installed = read_capped(&destination, MAX_INSTALLED_BYTES).await?;
        if render::sha256_hex(&installed) != next.rendered_hash {
            return Err(AppError::Internal {
                message: "verify Agent roster destination failed".into(),
            });
        }
        sync_parent_directory_entry(&destination)?;
        Ok(())
    }
    .await;
    if let Err(error) = write {
        let files = restore_batch_files_verified(&prior).await;
        let history = rollback_roster_history(&mut history_mutation).await;
        let restored = combine_roster_restoration(files, history);
        close_prepared_roster_failure(database.as_ref(), operation.as_ref(), &restored).await?;
        cleanup_roster_staging_artifacts(state, &staged).await;
        return match restored {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error("write Agent roster", error, rollback)),
        };
    }
    if let Some(index) = existing_index {
        rosters[index] = next.clone();
    } else {
        rosters.push(next.clone());
    }
    rosters.sort_by(|left, right| {
        left.project_path
            .cmp(&right.project_path)
            .then_with(|| left.tool.cmp(&right.tool))
    });
    let saved = match &operation {
        Some(operation) => save_rosters_after_filesystem(state, &rosters, &operation.id).await,
        None => save_rosters_for_state(state, &rosters).await,
    };
    if let Err(error) = saved {
        let files = restore_batch_files_verified(&prior).await;
        let history = rollback_roster_history(&mut history_mutation).await;
        let restored = combine_roster_restoration(files, history);
        close_prepared_roster_failure(database.as_ref(), operation.as_ref(), &restored).await?;
        cleanup_roster_staging_artifacts(state, &staged).await;
        return match restored {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error("save Agent roster", error, rollback)),
        };
    }
    if let Err(error) = if database.is_none() {
        mark_unjournaled_roster_write_applied(state, history_mutation.is_none(), &staged).await?;
        publish_unjournaled_roster_history(&mut history_mutation).await
    } else {
        Ok(())
    } {
        let rollback = rollback_unjournaled_roster_success(
            state,
            &prior,
            &prior_rosters,
            &mut history_mutation,
        )
        .await;
        cleanup_roster_staging_artifacts(state, &staged).await;
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error(
                "publish unjournaled Agent roster history",
                error,
                rollback,
            )),
        };
    }
    let retention = if let Some(previous) = &previous {
        match prepare_roster_retention(database.as_ref(), previous, history_mutation.as_ref()).await
        {
            Ok(retention) => retention,
            Err(error) => {
                let history = rollback_roster_history(&mut history_mutation).await;
                let error = if let (Some(database), Some(operation)) = (&database, &operation) {
                    compensate_roster_commit_failure(
                        state,
                        database,
                        operation,
                        &prior,
                        &prior_rosters,
                        history,
                        error,
                    )
                    .await
                } else {
                    error
                };
                cleanup_roster_staging_artifacts(state, &staged).await;
                return Err(error);
            }
        }
    } else {
        None
    };
    if let (Some(database), Some(operation)) = (&database, &operation) {
        if let Err(error) = database.commit_filesystem_operation(&operation.id).await {
            if let Err(rollback) = abort_roster_retention(Some(database), retention.as_ref()).await
            {
                cleanup_roster_staging_artifacts(state, &staged).await;
                return Err(rollback_error(
                    "abort Agent roster retention",
                    error,
                    rollback,
                ));
            }
            let history = rollback_roster_history(&mut history_mutation).await;
            let error = compensate_roster_commit_failure(
                state,
                database,
                operation,
                &prior,
                &prior_rosters,
                history,
                error,
            )
            .await;
            cleanup_roster_staging_artifacts(state, &staged).await;
            return Err(error);
        }
    }
    let finalized =
        finalize_roster_history(database.as_ref(), retention.as_ref(), &mut history_mutation).await;
    finalized?;
    cleanup_roster_staging_artifacts_durable(state, &staged).await?;
    Ok(next)
}

fn roster_identity(record: &AgentRosterInstallRecord) -> AgentInstallIdentity {
    AgentInstallIdentity {
        reference: AgentReference {
            source_id: format!("roster:{}", record.tool),
            relative_path: format!(
                "projects/{}.md",
                render::sha256_hex(record.project_path.as_bytes())
            ),
        },
        tool: record.tool.clone(),
        scope: crate::types::Scope::Project,
        project_path: Some(record.project_path.clone()),
    }
}

fn roster_record_index(
    rosters: &[AgentRosterInstallRecord],
    tool: &str,
    project_path: &str,
) -> Result<usize, AppError> {
    rosters
        .iter()
        .position(|record| record.tool == tool && record.project_path == project_path)
        .ok_or_else(|| AppError::InvalidArgument {
            message: "Agent roster install is not tracked".into(),
        })
}

async fn roster_history(
    state: &AppState,
    tool: &str,
    project_path: &str,
) -> Result<Vec<AgentVersionSnapshot>, AppError> {
    roster_tool(tool)?;
    let project = canonical_registered_instruction_project(state, project_path).await?;
    let rosters = load_rosters_for_state(state).await?;
    let index = roster_record_index(&rosters, tool, &project.to_string_lossy())?;
    history::list_snapshots(&state.app_data_dir, &roster_identity(&rosters[index])).await
}

async fn move_roster(
    state: &AppState,
    tool: &str,
    project_path: &str,
    enable: bool,
) -> Result<AgentRosterInstallRecord, AppError> {
    roster_tool(tool)?;
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
    recover_agent_operations_locked(state).await?;
    let project = canonical_registered_instruction_project(state, project_path).await?;
    let project_path = project.to_string_lossy().into_owned();
    let mut rosters = load_rosters_for_state(state).await?;
    let prior_rosters = rosters.clone();
    let index = roster_record_index(&rosters, tool, &project_path)?;
    let previous = rosters[index].clone();
    if enable != previous.disabled_path.is_some() {
        return Err(AppError::InvalidArgument {
            message: if enable {
                "Agent roster is not disabled"
            } else {
                "Agent roster is already disabled"
            }
            .into(),
        });
    }
    let active = PathBuf::from(&previous.dest);
    let disabled = disabled_destination(&active)?;
    if previous
        .disabled_path
        .as_deref()
        .is_some_and(|stored| Path::new(stored) != disabled)
    {
        return Err(AppError::StorageCorrupt {
            message: "Agent roster disabled destination changed".into(),
        });
    }
    let (source, destination) = if enable {
        (&disabled, &active)
    } else {
        (&active, &disabled)
    };
    if destination.exists() {
        return Err(AppError::InvalidArgument {
            message: format!(
                "Agent roster lifecycle destination is occupied: {}",
                destination.display()
            ),
        });
    }
    let prior = capture_batch_files(&[active.clone(), disabled.clone()]).await?;
    let mut next = previous.clone();
    next.disabled_path = (!enable).then(|| disabled.to_string_lossy().into_owned());
    let database = state.completed_state_database().await?;
    let mut history_mutation = begin_roster_preservation(state, source, &previous, None)
        .await?
        .ok_or_else(|| AppError::StorageCorrupt {
            message: "Agent roster lifecycle source disappeared before preservation".into(),
        })
        .map(Some)?;
    let history_reference = roster_history_reference(history_mutation.as_ref().unwrap());
    if history_reference.rendered_hash != previous.rendered_hash {
        return Err(rollback_unjournaled_roster_history(
            &mut history_mutation,
            "preserve Agent roster lifecycle",
            AppError::InvalidArgument {
                message: "Agent roster lifecycle source changed during preservation".into(),
            },
        )
        .await);
    }
    let operation = if let Some(database) = &database {
        match database
            .prepare_filesystem_operation(
                if enable {
                    "agent_enable"
                } else {
                    "agent_disable"
                },
                &AgentRosterMoveOperation {
                    previous: previous.clone(),
                    next: next.clone(),
                    history: history_reference,
                },
            )
            .await
        {
            Ok(operation) => Some(operation),
            Err(error) => {
                let history = rollback_roster_history(&mut history_mutation).await;
                return match history {
                    Ok(()) => Err(error),
                    Err(rollback) => Err(rollback_error(
                        "prepare Agent roster lifecycle",
                        error,
                        rollback,
                    )),
                };
            }
        }
    } else {
        None
    };
    let history_ready = async {
        publish_roster_history_if(&mut history_mutation, database.is_some()).await?;
        prepare_unjournaled_roster_operation(
            &mut history_mutation,
            database.is_none(),
            Some(&previous),
            Some(&next),
        )
        .await
    }
    .await;
    if let Err(error) = history_ready {
        let history = rollback_roster_history(&mut history_mutation).await;
        close_prepared_roster_failure(database.as_ref(), operation.as_ref(), &history).await?;
        return match history {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error(
                "publish Agent roster lifecycle history",
                error,
                rollback,
            )),
        };
    }
    if let Err(error) = move_managed_artifacts(
        std::slice::from_ref(source),
        std::slice::from_ref(destination),
        std::slice::from_ref(&previous.rendered_hash),
    ) {
        let files = restore_batch_files_verified(&prior).await;
        let history = rollback_roster_history(&mut history_mutation).await;
        let restored = combine_roster_restoration(files, history);
        close_prepared_roster_failure(database.as_ref(), operation.as_ref(), &restored).await?;
        return match restored {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error("move Agent roster", error, rollback)),
        };
    }
    rosters[index] = next.clone();
    let saved = match &operation {
        Some(operation) => save_rosters_after_filesystem(state, &rosters, &operation.id).await,
        None => save_rosters_for_state(state, &rosters).await,
    };
    if let Err(error) = saved {
        let files = restore_batch_files_verified(&prior).await;
        let history = rollback_roster_history(&mut history_mutation).await;
        let rollback = combine_roster_restoration(files, history);
        close_prepared_roster_failure(database.as_ref(), operation.as_ref(), &rollback).await?;
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error("move Agent roster", error, rollback)),
        };
    }
    if let Err(error) = if database.is_none() {
        publish_unjournaled_roster_history(&mut history_mutation).await
    } else {
        Ok(())
    } {
        let rollback = rollback_unjournaled_roster_success(
            state,
            &prior,
            &prior_rosters,
            &mut history_mutation,
        )
        .await;
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error(
                "publish unjournaled Agent roster lifecycle history",
                error,
                rollback,
            )),
        };
    }
    let retention =
        match prepare_roster_retention(database.as_ref(), &previous, history_mutation.as_ref())
            .await
        {
            Ok(retention) => retention,
            Err(error) => {
                let history = rollback_roster_history(&mut history_mutation).await;
                let error = if let (Some(database), Some(operation)) = (&database, &operation) {
                    compensate_roster_commit_failure(
                        state,
                        database,
                        operation,
                        &prior,
                        &prior_rosters,
                        history,
                        error,
                    )
                    .await
                } else {
                    error
                };
                return Err(error);
            }
        };
    if let (Some(database), Some(operation)) = (&database, &operation) {
        if let Err(error) = database.commit_filesystem_operation(&operation.id).await {
            if let Err(rollback) = abort_roster_retention(Some(database), retention.as_ref()).await
            {
                return Err(rollback_error(
                    "abort Agent roster retention",
                    error,
                    rollback,
                ));
            }
            let history = rollback_roster_history(&mut history_mutation).await;
            let error = compensate_roster_commit_failure(
                state,
                database,
                operation,
                &prior,
                &prior_rosters,
                history,
                error,
            )
            .await;
            return Err(error);
        }
    }
    finalize_roster_history(database.as_ref(), retention.as_ref(), &mut history_mutation).await?;
    Ok(next)
}

async fn uninstall_roster(
    state: &AppState,
    reviewed: &AgentRosterMutationPlan,
    revision: &str,
) -> Result<AgentRosterInstallRecord, AppError> {
    if reviewed.operation != "uninstall" {
        return Err(AppError::InvalidArgument {
            message: "Agent roster uninstall plan has the wrong operation".into(),
        });
    }
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
    recover_agent_operations_locked(state).await?;
    let current = build_roster_plan(
        state,
        reviewed
            .members
            .iter()
            .map(|member| member.reference.clone())
            .collect(),
        reviewed.tool.clone(),
        reviewed.project_path.clone(),
        "uninstall",
    )
    .await?;
    require_roster_revision(&current, revision)?;
    if !current.blockers.is_empty() {
        return Err(AppError::InvalidArgument {
            message: format!(
                "Agent roster plan is blocked: {}",
                current.blockers.join("; ")
            ),
        });
    }
    let mut rosters = load_rosters_for_state(state).await?;
    let prior_rosters = rosters.clone();
    let index = roster_record_index(&rosters, &current.tool, &current.project_path)?;
    let previous = rosters[index].clone();
    let active = PathBuf::from(&previous.dest);
    let path = previous
        .disabled_path
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or(active);
    let prior = capture_batch_files(std::slice::from_ref(&path)).await?;
    let current_hash = prior[0].bytes.as_deref().map(render::sha256_hex);
    let observation = observe_roster_destination(Path::new(&current.destination)).await;
    if !matches!(&observation, Ok(value) if *value == current.destination_observation) {
        return match observation {
            Ok(_) => Err(AppError::InvalidArgument {
                message: "Agent roster destination changed after review".into(),
            }),
            Err(error) => Err(error),
        };
    }
    if prior[0].bytes.is_some() {
        backup_if_differs(
            &path,
            &[],
            &backups_dir_for(&state.app_data_dir),
            &now_iso(),
        )
        .await?;
    }
    let database = state.completed_state_database().await?;
    let mut history_mutation = begin_roster_preservation(state, &path, &previous, None).await?;
    let preservation = roster_preservation(history_mutation.as_ref());
    if roster_preservation_hash(&preservation) != current_hash.as_deref() {
        return Err(rollback_unjournaled_roster_history(
            &mut history_mutation,
            "preserve Agent roster uninstall",
            AppError::InvalidArgument {
                message: "Agent roster uninstall target changed during preservation".into(),
            },
        )
        .await);
    }
    let operation = if let Some(database) = &database {
        match database
            .prepare_filesystem_operation(
                "agent_uninstall",
                &AgentRosterUninstallOperation {
                    previous: previous.clone(),
                    preservation,
                },
            )
            .await
        {
            Ok(operation) => Some(operation),
            Err(error) => {
                let history = rollback_roster_history(&mut history_mutation).await;
                return match history {
                    Ok(()) => Err(error),
                    Err(rollback) => Err(rollback_error(
                        "prepare Agent roster uninstall",
                        error,
                        rollback,
                    )),
                };
            }
        }
    } else {
        None
    };
    let history_ready = async {
        publish_roster_history_if(&mut history_mutation, database.is_some()).await?;
        prepare_unjournaled_roster_operation(
            &mut history_mutation,
            database.is_none(),
            Some(&previous),
            None,
        )
        .await
    }
    .await;
    if let Err(error) = history_ready {
        let history = rollback_roster_history(&mut history_mutation).await;
        close_prepared_roster_failure(database.as_ref(), operation.as_ref(), &history).await?;
        return match history {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error(
                "publish Agent roster uninstall history",
                error,
                rollback,
            )),
        };
    }
    if let Err(error) = verify_roster_preimage(&path, current_hash.as_deref()) {
        let history = rollback_roster_history(&mut history_mutation).await;
        close_prepared_roster_failure(database.as_ref(), operation.as_ref(), &history).await?;
        return match history {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error(
                "revalidate Agent roster uninstall",
                error,
                rollback,
            )),
        };
    }
    if current_hash.is_some() {
        if let Err(error) = tokio::fs::remove_file(&path)
            .await
            .map_err(|error| AppError::Io {
                message: format!("remove Agent roster {}: {error}", path.display()),
            })
        {
            let files = restore_batch_files_verified(&prior).await;
            let history = rollback_roster_history(&mut history_mutation).await;
            let restored = combine_roster_restoration(files, history);
            close_prepared_roster_failure(database.as_ref(), operation.as_ref(), &restored).await?;
            return match restored {
                Ok(()) => Err(error),
                Err(rollback) => Err(rollback_error("uninstall Agent roster", error, rollback)),
            };
        }
        if let Err(error) = sync_parent_directory_entry(&path) {
            let files = restore_batch_files_verified(&prior).await;
            let history = rollback_roster_history(&mut history_mutation).await;
            let restored = combine_roster_restoration(files, history);
            close_prepared_roster_failure(database.as_ref(), operation.as_ref(), &restored).await?;
            return match restored {
                Ok(()) => Err(error),
                Err(rollback) => Err(rollback_error(
                    "sync Agent roster uninstall",
                    error,
                    rollback,
                )),
            };
        }
    }
    rosters.remove(index);
    let saved = match &operation {
        Some(operation) => save_rosters_after_filesystem(state, &rosters, &operation.id).await,
        None => save_rosters_for_state(state, &rosters).await,
    };
    if let Err(error) = saved {
        let files = restore_batch_files_verified(&prior).await;
        let history = rollback_roster_history(&mut history_mutation).await;
        let rollback = combine_roster_restoration(files, history);
        close_prepared_roster_failure(database.as_ref(), operation.as_ref(), &rollback).await?;
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error("uninstall Agent roster", error, rollback)),
        };
    }
    if let Err(error) = if database.is_none() {
        publish_unjournaled_roster_history(&mut history_mutation).await
    } else {
        Ok(())
    } {
        let rollback = rollback_unjournaled_roster_success(
            state,
            &prior,
            &prior_rosters,
            &mut history_mutation,
        )
        .await;
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error(
                "publish unjournaled Agent roster uninstall history",
                error,
                rollback,
            )),
        };
    }
    let retention =
        match prepare_roster_retention(database.as_ref(), &previous, history_mutation.as_ref())
            .await
        {
            Ok(retention) => retention,
            Err(error) => {
                let history = rollback_roster_history(&mut history_mutation).await;
                let error = if let (Some(database), Some(operation)) = (&database, &operation) {
                    compensate_roster_commit_failure(
                        state,
                        database,
                        operation,
                        &prior,
                        &prior_rosters,
                        history,
                        error,
                    )
                    .await
                } else {
                    error
                };
                return Err(error);
            }
        };
    if let (Some(database), Some(operation)) = (&database, &operation) {
        if let Err(error) = database.commit_filesystem_operation(&operation.id).await {
            if let Err(rollback) = abort_roster_retention(Some(database), retention.as_ref()).await
            {
                return Err(rollback_error(
                    "abort Agent roster retention",
                    error,
                    rollback,
                ));
            }
            let history = rollback_roster_history(&mut history_mutation).await;
            let error = compensate_roster_commit_failure(
                state,
                database,
                operation,
                &prior,
                &prior_rosters,
                history,
                error,
            )
            .await;
            return Err(error);
        }
    }
    finalize_roster_history(database.as_ref(), retention.as_ref(), &mut history_mutation).await?;
    Ok(previous)
}

async fn rollback_roster(
    state: &AppState,
    tool: &str,
    project_path: &str,
    snapshot_id: &str,
) -> Result<AgentRosterInstallRecord, AppError> {
    roster_tool(tool)?;
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
    recover_agent_operations_locked(state).await?;
    let project = canonical_registered_instruction_project(state, project_path).await?;
    let project_path = project.to_string_lossy().into_owned();
    let mut rosters = load_rosters_for_state(state).await?;
    let prior_rosters = rosters.clone();
    let index = roster_record_index(&rosters, tool, &project_path)?;
    let previous = rosters[index].clone();
    if previous.disabled_path.is_some() {
        return Err(AppError::InvalidArgument {
            message: "Enable the Agent roster before rollback".into(),
        });
    }
    let identity = roster_identity(&previous);
    let destination = PathBuf::from(&previous.dest);
    let (snapshot, contents) = history::snapshot_contents(
        &state.app_data_dir,
        &identity,
        snapshot_id,
        std::slice::from_ref(&destination),
    )
    .await?;
    let mut next = snapshot
        .roster_record
        .clone()
        .ok_or_else(|| AppError::InvalidArgument {
            message: "Selected history entry is not an Agent roster snapshot".into(),
        })?;
    if next.tool != tool || next.project_path != project_path || next.dest != previous.dest {
        return Err(AppError::InvalidArgument {
            message: "Agent roster snapshot identity changed".into(),
        });
    }
    // Rollback publishes into the required active destination. Historical
    // lifecycle state must not point the restored ledger at a disabled path.
    next.disabled_path = None;
    validate_roster_record(&next)?;
    let rendered = contents
        .into_iter()
        .next()
        .ok_or_else(|| AppError::StorageCorrupt {
            message: "Agent roster snapshot has no aggregate file".into(),
        })?;
    if render::sha256_hex(&rendered) != next.rendered_hash {
        return Err(AppError::StorageCorrupt {
            message: "Agent roster snapshot hash changed".into(),
        });
    }
    next.installed_at = now_iso();
    let prior = capture_batch_files(std::slice::from_ref(&destination)).await?;
    let staged = stage_roster_bytes(&state.app_data_dir, &rendered).await?;
    let database = state.completed_state_database().await?;
    let mut history_mutation =
        begin_roster_preservation(state, &destination, &previous, Some(snapshot_id)).await?;
    let prior_hash = prior[0].bytes.as_deref().map(render::sha256_hex);
    let preservation = roster_preservation(history_mutation.as_ref());
    if roster_preservation_hash(&preservation) != prior_hash.as_deref() {
        let error = rollback_unjournaled_roster_history(
            &mut history_mutation,
            "preserve Agent roster rollback",
            AppError::InvalidArgument {
                message: "Agent roster rollback target changed during preservation".into(),
            },
        )
        .await;
        let _ = tokio::fs::remove_file(&staged).await;
        return Err(error);
    }
    let operation = if let Some(database) = &database {
        match database
            .prepare_filesystem_operation(
                "agent_update",
                &AgentRosterWriteOperation {
                    previous: Some(previous.clone()),
                    next: next.clone(),
                    staged_path: staged.to_string_lossy().into_owned(),
                    preservation,
                },
            )
            .await
        {
            Ok(operation) => Some(operation),
            Err(error) => {
                let history = rollback_roster_history(&mut history_mutation).await;
                let _ = tokio::fs::remove_file(&staged).await;
                return match history {
                    Ok(()) => Err(error),
                    Err(rollback) => Err(rollback_error(
                        "prepare Agent roster rollback",
                        error,
                        rollback,
                    )),
                };
            }
        }
    } else {
        None
    };
    let history_ready = async {
        publish_roster_history_if(&mut history_mutation, database.is_some()).await?;
        prepare_unjournaled_roster_operation(
            &mut history_mutation,
            database.is_none(),
            Some(&previous),
            Some(&next),
        )
        .await?;
        prepare_unjournaled_roster_write(
            state,
            database.is_none() && history_mutation.is_none(),
            Some(&previous),
            &next,
            &staged,
        )
        .await
    }
    .await;
    if let Err(error) = history_ready {
        let history = rollback_roster_history(&mut history_mutation).await;
        close_prepared_roster_failure(database.as_ref(), operation.as_ref(), &history).await?;
        cleanup_roster_staging_artifacts(state, &staged).await;
        return match history {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error(
                "publish Agent roster rollback history",
                error,
                rollback,
            )),
        };
    }
    if let Err(error) = verify_roster_preimage(&destination, prior_hash.as_deref()) {
        let history = rollback_roster_history(&mut history_mutation).await;
        close_prepared_roster_failure(database.as_ref(), operation.as_ref(), &history).await?;
        cleanup_roster_staging_artifacts(state, &staged).await;
        return match history {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error(
                "revalidate Agent roster rollback",
                error,
                rollback,
            )),
        };
    }
    if let Err(error) = publish_roster_bytes(&destination, &rendered, true).await {
        let files = restore_batch_files_verified(&prior).await;
        let history = rollback_roster_history(&mut history_mutation).await;
        let rollback = combine_roster_restoration(files, history);
        close_prepared_roster_failure(database.as_ref(), operation.as_ref(), &rollback).await?;
        cleanup_roster_staging_artifacts(state, &staged).await;
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error("rollback Agent roster", error, rollback)),
        };
    }
    if let Err(error) = sync_parent_directory_entry(&destination) {
        let files = restore_batch_files_verified(&prior).await;
        let history = rollback_roster_history(&mut history_mutation).await;
        let rollback = combine_roster_restoration(files, history);
        close_prepared_roster_failure(database.as_ref(), operation.as_ref(), &rollback).await?;
        cleanup_roster_staging_artifacts(state, &staged).await;
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error(
                "sync Agent roster rollback",
                error,
                rollback,
            )),
        };
    }
    rosters[index] = next.clone();
    let saved = match &operation {
        Some(operation) => save_rosters_after_filesystem(state, &rosters, &operation.id).await,
        None => save_rosters_for_state(state, &rosters).await,
    };
    if let Err(error) = saved {
        let files = restore_batch_files_verified(&prior).await;
        let history = rollback_roster_history(&mut history_mutation).await;
        let rollback = combine_roster_restoration(files, history);
        close_prepared_roster_failure(database.as_ref(), operation.as_ref(), &rollback).await?;
        cleanup_roster_staging_artifacts(state, &staged).await;
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error(
                "save Agent roster rollback",
                error,
                rollback,
            )),
        };
    }
    if let Err(error) = if database.is_none() {
        mark_unjournaled_roster_write_applied(state, history_mutation.is_none(), &staged).await?;
        publish_unjournaled_roster_history(&mut history_mutation).await
    } else {
        Ok(())
    } {
        let rollback = rollback_unjournaled_roster_success(
            state,
            &prior,
            &prior_rosters,
            &mut history_mutation,
        )
        .await;
        cleanup_roster_staging_artifacts(state, &staged).await;
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error(
                "publish unjournaled Agent roster rollback history",
                error,
                rollback,
            )),
        };
    }
    let retention =
        match prepare_roster_retention(database.as_ref(), &previous, history_mutation.as_ref())
            .await
        {
            Ok(retention) => retention,
            Err(error) => {
                let history = rollback_roster_history(&mut history_mutation).await;
                let error = if let (Some(database), Some(operation)) = (&database, &operation) {
                    compensate_roster_commit_failure(
                        state,
                        database,
                        operation,
                        &prior,
                        &prior_rosters,
                        history,
                        error,
                    )
                    .await
                } else {
                    error
                };
                cleanup_roster_staging_artifacts(state, &staged).await;
                return Err(error);
            }
        };
    if let (Some(database), Some(operation)) = (&database, &operation) {
        if let Err(error) = database.commit_filesystem_operation(&operation.id).await {
            if let Err(rollback) = abort_roster_retention(Some(database), retention.as_ref()).await
            {
                cleanup_roster_staging_artifacts(state, &staged).await;
                return Err(rollback_error(
                    "abort Agent roster retention",
                    error,
                    rollback,
                ));
            }
            let history = rollback_roster_history(&mut history_mutation).await;
            let error = compensate_roster_commit_failure(
                state,
                database,
                operation,
                &prior,
                &prior_rosters,
                history,
                error,
            )
            .await;
            cleanup_roster_staging_artifacts(state, &staged).await;
            return Err(error);
        }
    }
    let finalized =
        finalize_roster_history(database.as_ref(), retention.as_ref(), &mut history_mutation).await;
    finalized?;
    cleanup_roster_staging_artifacts_durable(state, &staged).await?;
    Ok(next)
}

async fn execute_install_plan(
    app: &AppHandle,
    state: &AppState,
    plan: &AgentMutationPlan,
    confirmed: bool,
) -> Result<Vec<InstallRecord>, AppError> {
    if !plan.blockers.is_empty() {
        return Err(AppError::InvalidArgument {
            message: format!(
                "Agent mutation plan is blocked: {}",
                plan.blockers.join("; ")
            ),
        });
    }
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
    recover_agent_operations_locked(state).await?;
    let original_ledger = load_ledger(app, state).await?;
    let sources = crate::agents::inspect_agent_sources(&state.app_data_dir).await?;
    let home = tool_home(state, &plan.tool).await?;
    let project = plan.project_path.as_deref().map(Path::new);
    let mut paths = Vec::new();
    for item in &plan.agents {
        let package = package_by_reference(&sources, &item.reference).ok_or_else(|| {
            AppError::InvalidArgument {
                message: format!(
                    "Agent source became unavailable: {}:{}",
                    item.reference.source_id, item.reference.relative_path
                ),
            }
        })?;
        let agent = package
            .agent
            .as_ref()
            .ok_or_else(|| AppError::InvalidArgument {
                message: "Agent package metadata became invalid".into(),
            })?;
        let existing = original_ledger.iter().find(|record| {
            record.source_id == item.reference.source_id
                && record.relative_path == item.reference.relative_path
                && record.tool == plan.tool
                && record.project_path == plan.project_path
        });
        paths.extend(match existing {
            Some(record) => resolved_record_paths(state, record).await?,
            None => install_target_paths(agent, None, &plan.tool, &home, project, None)?,
        });
    }
    let files = capture_batch_files(&paths).await?;
    let mut installed = Vec::with_capacity(plan.agents.len());
    for item in &plan.agents {
        match do_install_locked(
            app,
            state,
            item.reference.clone(),
            plan.tool.clone(),
            plan.project_path.clone(),
            confirmed,
        )
        .await
        {
            Ok(record) => installed.push(record),
            Err(error) => {
                let files_result = restore_batch_files(&files).await;
                let ledger_result = save_ledger(app, &original_ledger).await;
                return match (files_result, ledger_result) {
                    (Ok(()), Ok(())) => Err(error),
                    (Err(rollback), Ok(())) | (Ok(()), Err(rollback)) => {
                        Err(rollback_error("Agent batch", error, rollback))
                    }
                    (Err(files), Err(ledger)) => Err(AppError::Internal {
                        message: format!(
                            "Agent batch failed: {error}; file rollback failed: {files}; ledger rollback failed: {ledger}"
                        ),
                    }),
                };
            }
        }
    }
    Ok(installed)
}

async fn execute_uninstall_plan(
    app: &AppHandle,
    state: &AppState,
    plan: &AgentMutationPlan,
) -> Result<Vec<InstallRecord>, AppError> {
    if !plan.blockers.is_empty() {
        return Err(AppError::InvalidArgument {
            message: format!(
                "Agent mutation plan is blocked: {}",
                plan.blockers.join("; ")
            ),
        });
    }
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
    recover_agent_operations_locked(state).await?;
    let original_ledger = load_ledger(app, state).await?;
    let mut paths = Vec::new();
    let mut removed = Vec::with_capacity(plan.agents.len());
    for item in &plan.agents {
        let index = install_record_index(
            &original_ledger,
            &item.reference.source_id,
            &item.reference.relative_path,
            &plan.tool,
            plan.project_path.as_deref(),
        )?;
        let record = original_ledger[index].clone();
        let active = resolved_record_paths(state, &record).await?;
        if record.disabled_path.is_some() {
            paths.extend(
                active
                    .iter()
                    .map(|path| disabled_destination(path))
                    .collect::<Result<Vec<_>, _>>()?,
            );
        } else {
            paths.extend(active);
        }
        removed.push(record);
    }
    let files = capture_batch_files(&paths).await?;
    for item in &plan.agents {
        if let Err(error) = do_uninstall_locked(
            state,
            item.reference.clone(),
            plan.tool.clone(),
            plan.project_path.clone(),
        )
        .await
        {
            let files_result = restore_batch_files(&files).await;
            let ledger_result = save_ledger(app, &original_ledger).await;
            return match (files_result, ledger_result) {
                (Ok(()), Ok(())) => Err(error),
                (Err(rollback), Ok(())) | (Ok(()), Err(rollback)) => {
                    Err(rollback_error("Agent uninstall batch", error, rollback))
                }
                (Err(files), Err(ledger)) => Err(AppError::Internal {
                    message: format!(
                        "Agent uninstall batch failed: {error}; file rollback failed: {files}; ledger rollback failed: {ledger}"
                    ),
                }),
            };
        }
    }
    Ok(removed)
}

async fn collection_references(
    state: &AppState,
    name: &str,
) -> Result<Vec<AgentReference>, AppError> {
    crate::agents::organize::list(state)
        .await?
        .collections
        .into_iter()
        .find(|collection| collection.name == name)
        .map(|collection| collection.agents)
        .ok_or_else(|| AppError::InvalidArgument {
            message: format!("unknown Agent collection: {name}"),
        })
}

fn authorized_agent_project<'a>(
    project_path: Option<&str>,
    authorization: Option<&'a AuthorizedMcpProject>,
) -> Result<Option<&'a AuthorizedMcpProject>, AppError> {
    match (project_path, authorization) {
        (None, None) => Ok(None),
        (Some(path), Some(authorization)) if path == authorization.identity() => {
            Ok(Some(authorization))
        }
        (Some(_), Some(_)) => Err(AppError::InvalidArgument {
            message: "MCP project capability does not match the requested project".into(),
        }),
        (Some(_), None) => Err(AppError::InvalidArgument {
            message: "MCP project mutation requires an authorized project capability".into(),
        }),
        (None, Some(_)) => Err(AppError::InvalidArgument {
            message: "unexpected MCP project capability for a user-scope mutation".into(),
        }),
    }
}

fn capability_relative(
    authorization: &AuthorizedMcpProject,
    path: &Path,
) -> Result<PathBuf, AppError> {
    let relative =
        path.strip_prefix(authorization.identity())
            .map_err(|_| AppError::InvalidArgument {
                message: "Agent destination escaped the authorized MCP project".into(),
            })?;
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(AppError::InvalidArgument {
            message: "Agent destination is not a normalized project-relative path".into(),
        });
    }
    Ok(relative.to_path_buf())
}

pub(crate) async fn mcp_agent_plan(
    state: &AppState,
    reference: AgentReference,
    tool: Tool,
    project_path: Option<String>,
    operation: &str,
    include_dependencies: bool,
    authorization: Option<&AuthorizedMcpProject>,
) -> Result<AgentMutationPlan, AppError> {
    authorized_agent_project(project_path.as_deref(), authorization)?;
    build_mutation_plan(
        state,
        vec![reference],
        tool,
        project_path,
        operation,
        include_dependencies,
    )
    .await
}

pub(crate) async fn mcp_agent_is_tracked(
    state: &AppState,
    reference: &AgentReference,
    tool: &str,
    project_path: Option<&str>,
) -> Result<bool, AppError> {
    Ok(load_ledger_for_state(state).await?.iter().any(|record| {
        record.source_id == reference.source_id
            && record.relative_path == reference.relative_path
            && record.tool == tool
            && record.project_path.as_deref() == project_path
    }))
}

pub(crate) async fn mcp_collection_plan(
    state: &AppState,
    name: &str,
    tool: Tool,
    project_path: Option<String>,
    operation: &str,
    authorization: Option<&AuthorizedMcpProject>,
) -> Result<AgentMutationPlan, AppError> {
    authorized_agent_project(project_path.as_deref(), authorization)?;
    collection_plan(state, name, tool, project_path, operation).await
}

async fn rollback_clean_agent_files(
    paths: &[PathBuf],
    rendered_hashes: &[String],
    authorization: Option<&AuthorizedMcpProject>,
) -> Result<(), AppError> {
    if paths.len() != rendered_hashes.len() {
        return Err(AppError::InvalidArgument {
            message: "Agent rollback paths and artifact hashes disagree".into(),
        });
    }
    for (path, rendered_hash) in paths.iter().zip(rendered_hashes).rev() {
        if let Some(authorization) = authorization {
            crate::skills::install::remove_project_file(
                authorization.root(),
                &capability_relative(authorization, path)?,
                rendered_hash,
            )?;
        } else {
            let bytes = read_capped(path, MAX_INSTALLED_BYTES).await?;
            if render::sha256_hex(&bytes) != *rendered_hash {
                return Err(AppError::InvalidArgument {
                    message: format!(
                        "new Agent install changed before rollback: {}",
                        path.display()
                    ),
                });
            }
            remove_file_strict(path).await?;
        }
    }
    Ok(())
}

pub(crate) async fn mcp_install_agent_clean(
    state: &AppState,
    reference: AgentReference,
    tool: Tool,
    project_path: Option<String>,
    authorization: Option<&AuthorizedMcpProject>,
) -> Result<InstallRecord, AppError> {
    let authorization = authorized_agent_project(project_path.as_deref(), authorization)?;
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
    recover_agent_operations_locked(state).await?;
    let package = crate::agents::resolve_agent_package(&state.app_data_dir, &reference).await?;
    if !package.installable {
        return Err(AppError::InvalidArgument {
            message: "Agent package is not installable".into(),
        });
    }
    let raw = crate::agents::read_agent_text(&state.app_data_dir, &reference).await?;
    if render::sha256_hex(raw.as_bytes()) != package.source_hash {
        return Err(AppError::InvalidArgument {
            message: "Agent source changed after inspection".into(),
        });
    }
    let agent = package.agent.ok_or_else(|| AppError::InvalidArgument {
        message: "Agent package has no valid metadata".into(),
    })?;
    let home = tool_home(state, &tool).await?;
    let project = project_path.as_deref().map(Path::new);
    let paths = install_target_paths(&agent, Some(&raw), &tool, &home, project, None)?;
    let mut ledger = load_ledger_for_state(state).await?;
    if ledger.iter().any(|record| {
        record.source_id == reference.source_id
            && record.relative_path == reference.relative_path
            && record.tool == tool
            && record.project_path == project_path
    }) {
        return Err(AppError::InvalidArgument {
            message: "managed Agent replacement requires desktop approval".into(),
        });
    }
    ensure_destinations_available(
        &ledger,
        &reference,
        &tool,
        project_path.as_deref(),
        &paths,
        false,
    )?;
    let rendered = render::render_artifacts(&agent, &raw, &tool)?;
    let manifest = artifact_manifest(&paths, &rendered)?;
    let hashes = manifest
        .iter()
        .map(|artifact| artifact.rendered_hash.clone())
        .collect::<Vec<_>>();
    let mut created = Vec::with_capacity(paths.len());
    for (path, artifact) in paths.iter().zip(&rendered) {
        let result = if let Some(authorization) = authorization {
            crate::skills::install::create_project_file(
                authorization.root(),
                &capability_relative(authorization, path)?,
                artifact.content.as_bytes(),
            )
        } else {
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|error| AppError::Io {
                        message: format!("create Agent destination {}: {error}", parent.display()),
                    })?;
            }
            let mut file = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .await
                .map_err(|error| AppError::Io {
                    message: format!("create Agent destination {}: {error}", path.display()),
                })?;
            let result = async {
                file.write_all(artifact.content.as_bytes())
                    .await
                    .map_err(|error| AppError::Io {
                        message: format!("write Agent destination {}: {error}", path.display()),
                    })?;
                file.sync_all().await.map_err(|error| AppError::Io {
                    message: format!("sync Agent destination {}: {error}", path.display()),
                })
            }
            .await;
            if result.is_err() {
                let _ = tokio::fs::remove_file(path).await;
            }
            result
        };
        if let Err(error) = result {
            return match rollback_clean_agent_files(
                &created,
                &hashes[..created.len()],
                authorization,
            )
            .await
            {
                Ok(()) => Err(error),
                Err(rollback) => Err(rollback_error("install Agent", error, rollback)),
            };
        }
        created.push(path.clone());
    }
    let revision = package
        .version
        .clone()
        .unwrap_or_else(|| package.source_hash.clone());
    let mut record = record_for(
        &agent,
        &paths[0],
        &tool,
        project,
        manifest[0].rendered_hash.clone(),
        &package.source_hash,
        &package.body_hash,
        &revision,
        &now_iso(),
    );
    record.artifacts = manifest;
    record.source_id = reference.source_id.clone();
    record.relative_path = reference.relative_path.clone();
    record.source_snapshot_hash = package.source_hash;
    record.capabilities = package.capabilities;
    record.publisher_key = package.publisher_key;
    record.publisher_verified = package.publisher_verified;
    ledger.push(record.clone());
    if let Err(error) = save_ledger_for(&state.app_data_dir, &ledger).await {
        return match rollback_clean_agent_files(&created, &hashes, authorization).await {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error("save Agent install", error, rollback)),
        };
    }
    Ok(record)
}

pub(crate) async fn mcp_move_agent_install(
    state: &AppState,
    reference: AgentReference,
    tool: Tool,
    project_path: Option<String>,
    enable: bool,
    authorization: Option<&AuthorizedMcpProject>,
) -> Result<InstallRecord, AppError> {
    let authorization = authorized_agent_project(project_path.as_deref(), authorization)?;
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
    recover_agent_operations_locked(state).await?;
    let mut records = load_ledger_for_state(state).await?;
    let index = install_record_index(
        &records,
        &reference.source_id,
        &reference.relative_path,
        &tool,
        project_path.as_deref(),
    )?;
    if records[index].disabled_path.is_some() != enable {
        return Err(AppError::InvalidArgument {
            message: if enable {
                "Agent install is not disabled".into()
            } else {
                "Agent install is already disabled".into()
            },
        });
    }
    let active = resolved_record_paths(state, &records[index]).await?;
    let disabled = active
        .iter()
        .map(|path| disabled_destination(path))
        .collect::<Result<Vec<_>, _>>()?;
    let (sources, destinations) = if enable {
        (&disabled, &active)
    } else {
        (&active, &disabled)
    };
    if let Some(stored) = records[index].disabled_path.as_deref() {
        if disabled[0] != Path::new(stored) {
            return Err(AppError::InvalidArgument {
                message: "stored Agent disabled path does not match its destination".into(),
            });
        }
    }
    let stored_disabled = records[index].disabled_path.clone();
    let previous = records[index].clone();
    let hashes = record_rendered_hashes(&records[index], active.len());
    set_record_disabled_paths(
        &mut records[index],
        if enable { None } else { Some(&disabled) },
    );
    let database = state.completed_state_database().await?;
    let operation = if let Some(database) = &database {
        Some(
            database
                .prepare_filesystem_operation(
                    if enable {
                        "agent_enable"
                    } else {
                        "agent_disable"
                    },
                    &AgentMoveOperation {
                        previous,
                        next: records[index].clone(),
                        active: active
                            .iter()
                            .map(|path| path.to_string_lossy().into_owned())
                            .collect(),
                        disabled: disabled
                            .iter()
                            .map(|path| path.to_string_lossy().into_owned())
                            .collect(),
                    },
                )
                .await?,
        )
    } else {
        None
    };
    if let Some(authorization) = authorization {
        let contents = sources
            .iter()
            .map(|path| {
                crate::skills::install::read_project_file(
                    authorization.root(),
                    &capability_relative(authorization, path)?,
                    MAX_INSTALLED_BYTES,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        history::create_snapshot_from_bytes(
            &state.app_data_dir,
            &install_identity(&records[index]),
            &contents,
            &records[index].source_snapshot_hash,
            &records[index].rendered_hash,
            &now_iso(),
        )
        .await?;
        for (moved, (source, destination)) in sources.iter().zip(destinations).enumerate() {
            let result = crate::skills::install::rename_project_file(
                authorization.root(),
                &capability_relative(authorization, source)?,
                &capability_relative(authorization, destination)?,
                &records[index].rendered_hash,
            );
            if let Err(error) = result {
                for rollback in (0..moved).rev() {
                    crate::skills::install::rename_project_file(
                        authorization.root(),
                        &capability_relative(authorization, &destinations[rollback])?,
                        &capability_relative(authorization, &sources[rollback])?,
                        &records[index].rendered_hash,
                    )?;
                }
                return Err(error);
            }
        }
    } else {
        snapshot_record(state, &records[index], sources).await?;
        move_managed_artifacts(sources, destinations, &hashes)?;
    }
    let save = match &operation {
        Some(operation) => save_ledger_after_filesystem(state, &records, &operation.id).await,
        None => save_ledger_for(&state.app_data_dir, &records).await,
    };
    if let Err(error) = save {
        set_record_disabled_paths(
            &mut records[index],
            stored_disabled.as_ref().map(|_| disabled.as_slice()),
        );
        let rollback = if let Some(authorization) = authorization {
            let mut result = Ok(());
            for (source, destination) in sources.iter().zip(destinations).rev() {
                if let Err(error) = crate::skills::install::rename_project_file(
                    authorization.root(),
                    &capability_relative(authorization, destination)?,
                    &capability_relative(authorization, source)?,
                    &records[index].rendered_hash,
                ) {
                    result = Err(error);
                    break;
                }
            }
            result
        } else {
            move_managed_artifacts(destinations, sources, &hashes)
        };
        if let (Some(database), Some(operation)) = (&database, &operation) {
            return match rollback {
                Ok(()) => {
                    database.abort_filesystem_operation(&operation.id).await?;
                    Err(error)
                }
                Err(rollback) => {
                    database
                        .retain_filesystem_operation_error(&operation.id, &rollback.to_string())
                        .await?;
                    Err(rollback_error("move Agent install", error, rollback))
                }
            };
        }
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error("move Agent install", error, rollback)),
        };
    }
    if let (Some(database), Some(operation)) = (&database, &operation) {
        database.commit_filesystem_operation(&operation.id).await?;
    }
    Ok(records[index].clone())
}

pub(crate) async fn mcp_agent_version_history(
    state: &AppState,
    reference: AgentReference,
    tool: Tool,
    project_path: Option<String>,
    authorization: Option<&AuthorizedMcpProject>,
) -> Result<Vec<AgentVersionSnapshot>, AppError> {
    authorized_agent_project(project_path.as_deref(), authorization)?;
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
    let records = load_ledger_for_state(state).await?;
    let index = install_record_index(
        &records,
        &reference.source_id,
        &reference.relative_path,
        &tool,
        project_path.as_deref(),
    )?;
    history::list_snapshots(&state.app_data_dir, &install_identity(&records[index])).await
}

pub(crate) async fn mcp_rollback_revision(
    state: &AppState,
    reference: &AgentReference,
    tool: &str,
    project_path: Option<&str>,
    snapshot_id: &str,
) -> Result<String, AppError> {
    let records = load_ledger_for_state(state).await?;
    let index = install_record_index(
        &records,
        &reference.source_id,
        &reference.relative_path,
        tool,
        project_path,
    )?;
    let snapshot = history::list_snapshots(&state.app_data_dir, &install_identity(&records[index]))
        .await?
        .into_iter()
        .find(|snapshot| snapshot.id == snapshot_id)
        .ok_or_else(|| AppError::InvalidArgument {
            message: "Agent version snapshot does not belong to this install".into(),
        })?;
    let bytes = serde_json::to_vec(&(
        "rollback",
        &reference,
        tool,
        project_path,
        &records[index].dest,
        &records[index].rendered_hash,
        &records[index].source_snapshot_hash,
        snapshot,
    ))
    .map_err(|error| AppError::Internal {
        message: format!("serialize Agent rollback plan: {error}"),
    })?;
    Ok(render::sha256_hex(&bytes))
}

/// Classify one ledger row given what's on disk now and the current corpus
/// source hash for that slug. `disk` is `None` when the file is gone, else the
/// SHA-256 of its current bytes. See systemPatterns.md §4.
#[cfg(test)]
fn classify(
    disk: Option<&str>,
    rendered_hash: &str,
    record_source: &str,
    corpus_source: Option<&str>,
) -> InstallState {
    classify_install(ReconcileFacts {
        tracked: true,
        destination_hash: disk,
        disabled_path: false,
        disabled_hash: None,
        rendered_hash,
        installed_source_hash: record_source,
        current_source_hash: corpus_source,
    })
}

/// True if `file_bytes` are byte-identical to the canonical render of `agent`
/// for `tool`. Pure (no I/O) so it's unit-testable. When they match, the file
/// on disk IS this agent verbatim — there's nothing to "adopt"; reconcile can
/// treat it as `Current` even if we didn't install it.
#[cfg(test)]
fn bytes_match_render(
    agent: &crate::types::Agent,
    raw: &str,
    tool: &str,
    file_bytes: &[u8],
) -> bool {
    match render::render_with_hash(agent, raw, tool) {
        Ok((_, expected)) => render::sha256_hex(file_bytes) == expected,
        Err(_) => false,
    }
}

async fn destinations_match_render(
    agent: &crate::types::Agent,
    raw: &str,
    tool: &str,
    output_slug: &str,
    home: &Path,
    project_root: Option<&Path>,
) -> bool {
    let Ok(rendered) = render::render_artifacts(agent, raw, tool) else {
        return false;
    };
    let Ok(paths) = render::dests(tool, output_slug, home, project_root) else {
        return false;
    };
    if paths.len() != rendered.len() {
        return false;
    }
    for (path, expected) in paths.iter().zip(rendered) {
        let Ok(bytes) = read_capped(path, MAX_INSTALLED_BYTES).await else {
            return false;
        };
        if bytes != expected.content.as_bytes() {
            return false;
        }
    }
    true
}

// ---------- Tool detection ----------

fn detect(tool: &str, home: &Path) -> (bool, Option<String>) {
    // Registry-driven: detected if ANY of the tool's `detect.dirs` exists under
    // `home`; the agents dir comes from `detect.agentsDir`. Recognized-only tools
    // (no `detect` block) → (false, None).
    let Some(det) = registry::get(tool).and_then(|m| m.detect.as_ref()) else {
        return (false, None);
    };
    let detected = det.dirs.iter().any(|d| home.join(d).exists());
    let agents_dir = det
        .agents_dir
        .as_ref()
        .map(|sub| home.join(sub).to_string_lossy().to_string());
    (detected, agents_dir)
}

/// The tools Phase 2 can install to — the wired (installable) registry ids, in
/// registry order. Sourced from the embedded JSON so adding a tool is adding a
/// file, not editing this list.
fn supported() -> Vec<&'static str> {
    registry::wired().map(|m| m.id.as_str()).collect()
}

// ---------- Tauri commands ----------

/// Install (or re-install) `slug` into `tool`. For project-scoped tools pass
/// the project root in `project_path`. Returns the ledger record.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // ponytail: flat Tauri args preserve the existing command ABI.
pub async fn install_agent<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    source_id: Option<String>,
    relative_path: Option<String>,
    slug: Option<String>,
    tool: Tool,
    project_path: Option<String>,
    confirmed: Option<bool>,
) -> Result<InstallRecord, AppError> {
    let reference = resolve_command_reference(
        &app,
        &state,
        source_id.as_deref(),
        relative_path.as_deref(),
        slug.as_deref(),
    )
    .await?;
    do_install(
        &app,
        &state,
        reference,
        tool,
        project_path,
        confirmed.unwrap_or(false),
    )
    .await
}

/// Update an install to the current corpus version (re-render + write). The
/// prior file is backed up first (see `do_install`), so an Update applied to a
/// Modified file preserves the user's edits in `backups/` before restoring the
/// canonical render. Separate command from install for intent + UX.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // ponytail: flat Tauri args preserve the existing command ABI.
pub async fn update_agent(
    app: AppHandle,
    state: State<'_, AppState>,
    source_id: Option<String>,
    relative_path: Option<String>,
    slug: Option<String>,
    tool: Tool,
    project_path: Option<String>,
    confirmed: Option<bool>,
) -> Result<InstallRecord, AppError> {
    let reference = resolve_command_reference(
        &app,
        &state,
        source_id.as_deref(),
        relative_path.as_deref(),
        slug.as_deref(),
    )
    .await?;
    do_install(
        &app,
        &state,
        reference,
        tool,
        project_path,
        confirmed.unwrap_or(false),
    )
    .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // ponytail: flat Tauri args preserve the existing command ABI.
pub async fn agent_install_plan(
    app: AppHandle,
    state: State<'_, AppState>,
    source_id: Option<String>,
    relative_path: Option<String>,
    slug: Option<String>,
    tool: Tool,
    project_path: Option<String>,
    include_dependencies: Option<bool>,
) -> Result<AgentMutationPlan, AppError> {
    corpus::ensure_corpus(&app, &state).await?;
    let reference = resolve_command_reference(
        &app,
        &state,
        source_id.as_deref(),
        relative_path.as_deref(),
        slug.as_deref(),
    )
    .await?;
    build_mutation_plan(
        &state,
        vec![reference],
        tool,
        project_path,
        "install",
        include_dependencies.unwrap_or(true),
    )
    .await
}

#[tauri::command]
pub async fn agent_update_plan(
    app: AppHandle,
    state: State<'_, AppState>,
    source_id: Option<String>,
    relative_path: Option<String>,
    slug: Option<String>,
    tool: Tool,
    project_path: Option<String>,
) -> Result<AgentMutationPlan, AppError> {
    corpus::ensure_corpus(&app, &state).await?;
    let reference = resolve_command_reference(
        &app,
        &state,
        source_id.as_deref(),
        relative_path.as_deref(),
        slug.as_deref(),
    )
    .await?;
    build_mutation_plan(&state, vec![reference], tool, project_path, "update", false).await
}

#[tauri::command]
pub async fn agent_uninstall_plan(
    app: AppHandle,
    state: State<'_, AppState>,
    source_id: Option<String>,
    relative_path: Option<String>,
    slug: Option<String>,
    tool: Tool,
    project_path: Option<String>,
) -> Result<AgentMutationPlan, AppError> {
    corpus::ensure_corpus(&app, &state).await?;
    let reference = resolve_command_reference(
        &app,
        &state,
        source_id.as_deref(),
        relative_path.as_deref(),
        slug.as_deref(),
    )
    .await?;
    build_mutation_plan(
        &state,
        vec![reference],
        tool,
        project_path,
        "uninstall",
        false,
    )
    .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // ponytail: flat Tauri args preserve the existing command ABI.
pub async fn agent_install_with_dependencies(
    app: AppHandle,
    state: State<'_, AppState>,
    source_id: Option<String>,
    relative_path: Option<String>,
    slug: Option<String>,
    tool: Tool,
    project_path: Option<String>,
    confirmed: Option<bool>,
) -> Result<Vec<InstallRecord>, AppError> {
    corpus::ensure_corpus(&app, &state).await?;
    let reference = resolve_command_reference(
        &app,
        &state,
        source_id.as_deref(),
        relative_path.as_deref(),
        slug.as_deref(),
    )
    .await?;
    let plan =
        build_mutation_plan(&state, vec![reference], tool, project_path, "install", true).await?;
    execute_install_plan(&app, &state, &plan, confirmed.unwrap_or(false)).await
}

async fn collection_plan(
    state: &AppState,
    name: &str,
    tool: Tool,
    project_path: Option<String>,
    operation: &str,
) -> Result<AgentMutationPlan, AppError> {
    let roots = collection_references(state, name).await?;
    build_mutation_plan(
        state,
        roots,
        tool,
        project_path,
        operation,
        operation != "uninstall",
    )
    .await
}

fn validate_batch_references(
    mut references: Vec<AgentReference>,
) -> Result<Vec<AgentReference>, AppError> {
    if references.is_empty() || references.len() > MAX_AGENT_BATCH_ROOTS {
        return Err(AppError::InvalidArgument {
            message: format!(
                "Agent batch must contain between 1 and {MAX_AGENT_BATCH_ROOTS} exact references"
            ),
        });
    }
    for reference in &references {
        crate::library::validate_reference(&reference.source_id, &reference.relative_path)?;
    }
    references.sort();
    references.dedup();
    Ok(references)
}

#[tauri::command]
pub async fn agent_batch_install_plan(
    app: AppHandle,
    state: State<'_, AppState>,
    references: Vec<AgentReference>,
    tool: Tool,
    project_path: Option<String>,
) -> Result<AgentMutationPlan, AppError> {
    corpus::ensure_corpus(&app, &state).await?;
    build_mutation_plan(
        &state,
        validate_batch_references(references)?,
        tool,
        project_path,
        "install",
        true,
    )
    .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // ponytail: flat Tauri args preserve the command ABI.
pub async fn agent_batch_apply(
    app: AppHandle,
    state: State<'_, AppState>,
    references: Vec<AgentReference>,
    tool: Tool,
    project_path: Option<String>,
    plan_revision: String,
    confirmed: Option<bool>,
) -> Result<Vec<InstallRecord>, AppError> {
    corpus::ensure_corpus(&app, &state).await?;
    if confirmed != Some(true) {
        return Err(AppError::InvalidArgument {
            message: "Agent batch requires explicit confirmation".into(),
        });
    }
    let plan = build_mutation_plan(
        &state,
        validate_batch_references(references)?,
        tool,
        project_path,
        "install",
        true,
    )
    .await?;
    require_plan_revision(&plan, &plan_revision)?;
    execute_install_plan(&app, &state, &plan, true).await
}

#[tauri::command]
pub async fn agent_rosters_reconcile(
    state: State<'_, AppState>,
) -> Result<Vec<InstalledAgentRoster>, AppError> {
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
    recover_agent_operations_locked(&state).await?;
    let records = load_rosters_for_state(&state).await?;
    let mut installed = Vec::with_capacity(records.len());
    for record in records {
        let lifecycle = current_roster_state(&state, &record).await?;
        installed.push(InstalledAgentRoster {
            record,
            state: lifecycle,
        });
    }
    Ok(installed)
}

#[tauri::command]
pub async fn agent_roster_plan<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    references: Vec<AgentReference>,
    tool: Tool,
    project_path: String,
    operation: String,
) -> Result<AgentRosterMutationPlan, AppError> {
    corpus::ensure_corpus(&app, &state).await?;
    build_roster_plan(&state, references, tool, project_path, &operation).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // ponytail: flat Tauri args preserve the command ABI.
pub async fn agent_roster_apply<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    references: Vec<AgentReference>,
    tool: Tool,
    project_path: String,
    operation: String,
    plan_revision: String,
    confirmed: Option<bool>,
) -> Result<AgentRosterInstallRecord, AppError> {
    corpus::ensure_corpus(&app, &state).await?;
    if confirmed != Some(true) {
        return Err(AppError::InvalidArgument {
            message: "Agent roster mutation requires explicit confirmation".into(),
        });
    }
    let plan = build_roster_plan(&state, references, tool, project_path, &operation).await?;
    if operation == "uninstall" {
        uninstall_roster(&state, &plan, &plan_revision).await
    } else {
        apply_roster_plan(&state, &plan, &plan_revision, true).await
    }
}

fn require_roster_confirmation(confirmed: Option<bool>) -> Result<(), AppError> {
    if confirmed == Some(true) {
        Ok(())
    } else {
        Err(AppError::InvalidArgument {
            message: "Agent roster lifecycle mutation requires explicit confirmation".into(),
        })
    }
}

#[tauri::command]
pub async fn agent_roster_disable(
    state: State<'_, AppState>,
    tool: Tool,
    project_path: String,
    confirmed: Option<bool>,
) -> Result<AgentRosterInstallRecord, AppError> {
    require_roster_confirmation(confirmed)?;
    move_roster(&state, &tool, &project_path, false).await
}

#[tauri::command]
pub async fn agent_roster_enable(
    state: State<'_, AppState>,
    tool: Tool,
    project_path: String,
    confirmed: Option<bool>,
) -> Result<AgentRosterInstallRecord, AppError> {
    require_roster_confirmation(confirmed)?;
    move_roster(&state, &tool, &project_path, true).await
}

#[tauri::command]
pub async fn agent_roster_version_history(
    state: State<'_, AppState>,
    tool: Tool,
    project_path: String,
) -> Result<Vec<AgentVersionSnapshot>, AppError> {
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
    recover_agent_operations_locked(&state).await?;
    roster_history(&state, &tool, &project_path).await
}

#[tauri::command]
pub async fn agent_roster_version_rollback(
    state: State<'_, AppState>,
    tool: Tool,
    project_path: String,
    snapshot_id: String,
    confirmed: Option<bool>,
) -> Result<AgentRosterInstallRecord, AppError> {
    require_roster_confirmation(confirmed)?;
    rollback_roster(&state, &tool, &project_path, &snapshot_id).await
}

#[tauri::command]
pub async fn agent_collection_install_plan(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    tool: Tool,
    project_path: Option<String>,
) -> Result<AgentMutationPlan, AppError> {
    corpus::ensure_corpus(&app, &state).await?;
    collection_plan(&state, &name, tool, project_path, "install").await
}

#[tauri::command]
pub async fn agent_collection_update_plan(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    tool: Tool,
    project_path: Option<String>,
) -> Result<AgentMutationPlan, AppError> {
    corpus::ensure_corpus(&app, &state).await?;
    collection_plan(&state, &name, tool, project_path, "update").await
}

#[tauri::command]
pub async fn agent_collection_uninstall_plan(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    tool: Tool,
    project_path: Option<String>,
) -> Result<AgentMutationPlan, AppError> {
    corpus::ensure_corpus(&app, &state).await?;
    collection_plan(&state, &name, tool, project_path, "uninstall").await
}

#[tauri::command]
pub async fn agent_collection_apply(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    tool: Tool,
    project_path: Option<String>,
    operation: String,
    confirmed: Option<bool>,
) -> Result<Vec<InstallRecord>, AppError> {
    corpus::ensure_corpus(&app, &state).await?;
    if !matches!(operation.as_str(), "install" | "update" | "uninstall") {
        return Err(AppError::InvalidArgument {
            message: format!("unknown Agent collection operation: {operation}"),
        });
    }
    let plan = collection_plan(&state, &name, tool, project_path, &operation).await?;
    if operation == "uninstall" {
        execute_uninstall_plan(&app, &state, &plan).await
    } else {
        execute_install_plan(&app, &state, &plan, confirmed.unwrap_or(false)).await
    }
}

async fn snapshot_record(
    state: &AppState,
    record: &InstallRecord,
    paths: &[PathBuf],
) -> Result<AgentVersionSnapshot, AppError> {
    snapshot_record_protected(state, record, paths, None).await
}

async fn snapshot_record_protected(
    state: &AppState,
    record: &InstallRecord,
    paths: &[PathBuf],
    protected_snapshot_id: Option<&str>,
) -> Result<AgentVersionSnapshot, AppError> {
    let bytes = read_capped(
        paths.first().ok_or_else(|| AppError::InvalidArgument {
            message: "tracked Agent has no destination".into(),
        })?,
        MAX_INSTALLED_BYTES,
    )
    .await?;
    let identity = install_identity(record);
    let rendered_hash = render::sha256_hex(&bytes);
    let created_at = now_iso();
    if let Some(protected_snapshot_id) = protected_snapshot_id {
        history::create_snapshot_protected(
            &state.app_data_dir,
            &identity,
            paths,
            &record.source_snapshot_hash,
            &rendered_hash,
            &created_at,
            Some(protected_snapshot_id),
        )
        .await
    } else {
        history::create_snapshot(
            &state.app_data_dir,
            &identity,
            paths,
            &record.source_snapshot_hash,
            &rendered_hash,
            &created_at,
        )
        .await
    }
}

#[tauri::command]
pub async fn agent_version_history(
    app: AppHandle,
    state: State<'_, AppState>,
    source_id: String,
    relative_path: String,
    tool: Tool,
    project_path: Option<String>,
) -> Result<Vec<AgentVersionSnapshot>, AppError> {
    // ponytail: one process lock serializes both install ledgers; split only if measured contention warrants it.
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
    let records = load_ledger(&app, &state).await?;
    let index = install_record_index(
        &records,
        &source_id,
        &relative_path,
        &tool,
        project_path.as_deref(),
    )?;
    history::list_snapshots(&state.app_data_dir, &install_identity(&records[index])).await
}

#[tauri::command]
pub async fn disable_agent(
    _app: AppHandle,
    state: State<'_, AppState>,
    source_id: String,
    relative_path: String,
    tool: Tool,
    project_path: Option<String>,
) -> Result<InstallRecord, AppError> {
    mcp_move_agent_install(
        &state,
        AgentReference {
            source_id,
            relative_path,
        },
        tool,
        project_path,
        false,
        None,
    )
    .await
}

#[tauri::command]
pub async fn enable_agent(
    _app: AppHandle,
    state: State<'_, AppState>,
    source_id: String,
    relative_path: String,
    tool: Tool,
    project_path: Option<String>,
) -> Result<InstallRecord, AppError> {
    mcp_move_agent_install(
        &state,
        AgentReference {
            source_id,
            relative_path,
        },
        tool,
        project_path,
        true,
        None,
    )
    .await
}

#[tauri::command]
pub async fn agent_version_rollback(
    _app: AppHandle,
    state: State<'_, AppState>,
    source_id: String,
    relative_path: String,
    tool: Tool,
    project_path: Option<String>,
    snapshot_id: String,
) -> Result<InstallRecord, AppError> {
    rollback_agent_version(
        &state,
        AgentReference {
            source_id,
            relative_path,
        },
        tool,
        project_path,
        snapshot_id,
    )
    .await
}

async fn rollback_agent_version(
    state: &AppState,
    reference: AgentReference,
    tool: Tool,
    project_path: Option<String>,
    snapshot_id: String,
) -> Result<InstallRecord, AppError> {
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
    recover_agent_operations_locked(state).await?;
    let mut records = load_ledger_for_state(state).await?;
    let index = install_record_index(
        &records,
        &reference.source_id,
        &reference.relative_path,
        &tool,
        project_path.as_deref(),
    )?;
    if records[index].disabled_path.is_some() {
        return Err(AppError::InvalidArgument {
            message: "enable the Agent before rolling back a version".into(),
        });
    }
    let identity = install_identity(&records[index]);
    let paths = resolved_record_paths(state, &records[index]).await?;
    let (selected_meta, selected_contents) =
        history::snapshot_contents(&state.app_data_dir, &identity, &snapshot_id, &paths).await?;
    if !records[index].artifacts.is_empty()
        && selected_meta.artifact_hashes.len() != records[index].artifacts.len()
    {
        return Err(AppError::InvalidArgument {
            message: "Agent version snapshot artifact count changed".into(),
        });
    }
    let prior =
        snapshot_record_protected(state, &records[index], &paths, Some(&selected_meta.id)).await?;
    let previous = records[index].clone();
    let mut next = previous.clone();
    next.source_hash = selected_meta.source_hash.clone();
    next.source_snapshot_hash = selected_meta.source_hash.clone();
    next.rendered_hash = selected_meta.rendered_hash.clone();
    if !next.artifacts.is_empty() {
        for (artifact, hash) in next
            .artifacts
            .iter_mut()
            .zip(selected_meta.artifact_hashes.iter())
        {
            artifact.rendered_hash.clone_from(hash);
        }
        next.rendered_hash = next.artifacts[0].rendered_hash.clone();
    }
    next.installed_at = now_iso();
    let rendered = selected_contents
        .into_iter()
        .map(|bytes| {
            String::from_utf8(bytes).map_err(|_| AppError::InvalidArgument {
                message: "Agent version snapshot content is not UTF-8".into(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let database = state.completed_state_database().await?;
    let operation = if let Some(database) = &database {
        Some(
            database
                .prepare_filesystem_operation(
                    "agent_update",
                    &AgentInstallOperation {
                        previous: Some(previous),
                        next: next.clone(),
                        targets: paths
                            .iter()
                            .map(|path| path.to_string_lossy().into_owned())
                            .collect(),
                        rendered: if rendered.len() == 1 {
                            AgentRenderedPayload::One(rendered[0].clone())
                        } else {
                            AgentRenderedPayload::Many(rendered)
                        },
                    },
                )
                .await?,
        )
    } else {
        None
    };
    if let Err(error) =
        history::restore_snapshot(&state.app_data_dir, &identity, &snapshot_id, &paths).await
    {
        if let (Some(database), Some(operation)) = (&database, &operation) {
            database.abort_filesystem_operation(&operation.id).await?;
        }
        return Err(error);
    }
    records[index] = next;
    let save = match &operation {
        Some(operation) => save_ledger_after_filesystem(state, &records, &operation.id).await,
        None => save_ledger_for(&state.app_data_dir, &records).await,
    };
    if let Err(error) = save {
        let restore =
            history::restore_snapshot(&state.app_data_dir, &identity, &prior.id, &paths).await;
        if let (Some(database), Some(operation)) = (&database, &operation) {
            return match restore {
                Ok(_) => {
                    database.abort_filesystem_operation(&operation.id).await?;
                    Err(error)
                }
                Err(rollback) => {
                    database
                        .retain_filesystem_operation_error(&operation.id, &rollback.to_string())
                        .await?;
                    Err(rollback_error("rollback Agent version", error, rollback))
                }
            };
        }
        return match restore {
            Ok(_) => Err(error),
            Err(rollback) => Err(rollback_error("rollback Agent version", error, rollback)),
        };
    }
    if let (Some(database), Some(operation)) = (&database, &operation) {
        database.commit_filesystem_operation(&operation.id).await?;
    }
    Ok(records[index].clone())
}

fn require_plan_revision(plan: &AgentMutationPlan, expected: &str) -> Result<(), AppError> {
    if plan.revision != expected {
        return Err(AppError::InvalidArgument {
            message: "Agent approval plan is stale; submit a new approval request".into(),
        });
    }
    if !plan.blockers.is_empty() {
        return Err(AppError::InvalidArgument {
            message: format!(
                "Agent approval plan is blocked: {}",
                plan.blockers.join("; ")
            ),
        });
    }
    Ok(())
}

pub(crate) async fn execute_agent_lifecycle_approval(
    app: &AppHandle,
    state: &AppState,
    action: &AgentApprovalAction,
) -> Result<String, AppError> {
    corpus::ensure_corpus(app, state).await?;
    match action {
        AgentApprovalAction::Install {
            reference,
            tool,
            project_path,
            include_dependencies,
            plan_revision,
        } => {
            let plan = build_mutation_plan(
                state,
                vec![reference.clone()],
                tool.clone(),
                project_path.clone(),
                "install",
                *include_dependencies,
            )
            .await?;
            require_plan_revision(&plan, plan_revision)?;
            serde_json::to_string_pretty(&execute_install_plan(app, state, &plan, true).await?)
                .map_err(|error| AppError::Internal {
                    message: format!("serialize approved Agent install: {error}"),
                })
        }
        AgentApprovalAction::Update {
            reference,
            tool,
            project_path,
            plan_revision,
        } => {
            let plan = build_mutation_plan(
                state,
                vec![reference.clone()],
                tool.clone(),
                project_path.clone(),
                "update",
                false,
            )
            .await?;
            require_plan_revision(&plan, plan_revision)?;
            serde_json::to_string_pretty(&execute_install_plan(app, state, &plan, true).await?)
                .map_err(|error| AppError::Internal {
                    message: format!("serialize approved Agent update: {error}"),
                })
        }
        AgentApprovalAction::Uninstall {
            reference,
            tool,
            project_path,
            plan_revision,
        } => {
            let plan = build_mutation_plan(
                state,
                vec![reference.clone()],
                tool.clone(),
                project_path.clone(),
                "uninstall",
                false,
            )
            .await?;
            require_plan_revision(&plan, plan_revision)?;
            serde_json::to_string_pretty(&execute_uninstall_plan(app, state, &plan).await?).map_err(
                |error| AppError::Internal {
                    message: format!("serialize approved Agent uninstall: {error}"),
                },
            )
        }
        AgentApprovalAction::Rollback {
            reference,
            tool,
            project_path,
            snapshot_id,
            plan_revision,
        } => {
            let current =
                mcp_rollback_revision(state, reference, tool, project_path.as_deref(), snapshot_id)
                    .await?;
            if current != *plan_revision {
                return Err(AppError::InvalidArgument {
                    message: "Agent rollback approval is stale; submit a new request".into(),
                });
            }
            serde_json::to_string_pretty(
                &rollback_agent_version(
                    state,
                    reference.clone(),
                    tool.clone(),
                    project_path.clone(),
                    snapshot_id.clone(),
                )
                .await?,
            )
            .map_err(|error| AppError::Internal {
                message: format!("serialize approved Agent rollback: {error}"),
            })
        }
        AgentApprovalAction::BatchCollection {
            collection_name,
            operation,
            tool,
            project_path,
            plan_revision,
        } => {
            let plan = collection_plan(
                state,
                collection_name,
                tool.clone(),
                project_path.clone(),
                operation,
            )
            .await?;
            require_plan_revision(&plan, plan_revision)?;
            let records = if operation == "uninstall" {
                execute_uninstall_plan(app, state, &plan).await?
            } else {
                execute_install_plan(app, state, &plan, true).await?
            };
            serde_json::to_string_pretty(&records).map_err(|error| AppError::Internal {
                message: format!("serialize approved Agent batch: {error}"),
            })
        }
        _ => Err(AppError::InvalidArgument {
            message: "Agent approval action is not a lifecycle mutation".into(),
        }),
    }
}

/// Track a recognized Foreign install into the ledger **non-destructively** —
/// we record provenance but never write to the user's file. This is the safe
/// replacement for the old "Adopt" (which overwrote the on-disk file). After
/// tracking, reconcile shows `Current` if the file already matches the canonical
/// render, or `Modified` if it differs (then an explicit Update reconciles it).
#[tauri::command]
pub async fn track_agent(
    app: AppHandle,
    state: State<'_, AppState>,
    source_id: Option<String>,
    relative_path: Option<String>,
    slug: Option<String>,
    tool: Tool,
    project_path: Option<String>,
) -> Result<InstallRecord, AppError> {
    let reference = resolve_command_reference(
        &app,
        &state,
        source_id.as_deref(),
        relative_path.as_deref(),
        slug.as_deref(),
    )
    .await?;
    do_track(&app, &state, reference, tool, project_path).await
}

/// Diff what's on disk against the canonical render the app would write — powers
/// "review before Update" without touching any file.
#[tauri::command]
pub async fn agent_diff(
    app: AppHandle,
    state: State<'_, AppState>,
    source_id: Option<String>,
    relative_path: Option<String>,
    slug: Option<String>,
    tool: Tool,
    project_path: Option<String>,
) -> Result<AgentDiff, AppError> {
    let reference = resolve_command_reference(
        &app,
        &state,
        source_id.as_deref(),
        relative_path.as_deref(),
        slug.as_deref(),
    )
    .await?;
    let package = crate::agents::resolve_agent_package(&state.app_data_dir, &reference).await?;
    let agent = package.agent.ok_or_else(|| AppError::InvalidArgument {
        message: "Agent package has no valid metadata".into(),
    })?;
    let raw = crate::agents::read_agent_text(&state.app_data_dir, &reference).await?;
    let rendered = render::render_artifacts(&agent, &raw, &tool)?;

    let home = tool_home(&state, &tool).await?;
    let proot = project_path.as_ref().map(PathBuf::from);
    let ledger = load_ledger(&app, &state).await?;
    let ledger_record = ledger.iter().find(|record| {
        record.source_id == reference.source_id
            && record.relative_path == reference.relative_path
            && record.tool == tool
            && record.project_path == project_path
    });
    let paths = match ledger_record {
        Some(record) => resolved_record_paths(&state, record).await?,
        None => install_target_paths(&agent, Some(&raw), &tool, &home, proot.as_deref(), None)?,
    };
    if paths.len() != rendered.len() {
        return Err(AppError::InvalidArgument {
            message: "Agent diff destinations do not match rendered artifacts".into(),
        });
    }
    let mut artifacts = Vec::with_capacity(paths.len());
    for (dest, rendered) in paths.iter().zip(rendered) {
        let on_disk = match read_capped(dest, MAX_INSTALLED_BYTES).await {
            Ok(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
            Err(_) => None,
        };
        let differs = on_disk.as_deref() != Some(rendered.content.as_str());
        artifacts.push(AgentArtifactDiff {
            dest: dest.to_string_lossy().into_owned(),
            on_disk,
            proposed: rendered.content,
            differs,
        });
    }
    let primary = artifacts.first().ok_or_else(|| AppError::InvalidArgument {
        message: "Agent diff has no rendered artifacts".into(),
    })?;
    Ok(AgentDiff {
        slug: agent.slug,
        tool,
        project_path,
        dest: primary.dest.clone(),
        on_disk: primary.on_disk.clone(),
        proposed: primary.proposed.clone(),
        differs: artifacts.iter().any(|artifact| artifact.differs),
        artifacts,
    })
}

/// Uninstall: remove the written file(s) and the ledger row.
#[tauri::command]
pub async fn uninstall_agent<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    source_id: Option<String>,
    relative_path: Option<String>,
    slug: Option<String>,
    tool: Tool,
    project_path: Option<String>,
) -> Result<(), AppError> {
    let reference = resolve_command_reference(
        &app,
        &state,
        source_id.as_deref(),
        relative_path.as_deref(),
        slug.as_deref(),
    )
    .await?;
    do_uninstall(&state, reference, tool, project_path).await
}

async fn do_uninstall(
    state: &AppState,
    reference: AgentReference,
    tool: Tool,
    project_path: Option<String>,
) -> Result<(), AppError> {
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
    recover_agent_operations_locked(state).await?;
    do_uninstall_locked(state, reference, tool, project_path).await
}

async fn do_uninstall_locked(
    state: &AppState,
    reference: AgentReference,
    tool: Tool,
    project_path: Option<String>,
) -> Result<(), AppError> {
    let mut ledger = load_ledger_for_state(state).await?;
    let index = install_record_index(
        &ledger,
        &reference.source_id,
        &reference.relative_path,
        &tool,
        project_path.as_deref(),
    )?;
    let record = ledger[index].clone();
    let active = resolved_record_paths(state, &record).await?;
    let paths = if record.disabled_path.is_some() {
        active
            .iter()
            .map(|path| disabled_destination(path))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        active
    };
    let existing = paths
        .iter()
        .filter(|path| path.exists())
        .cloned()
        .collect::<Vec<_>>();
    let snapshot = if existing.is_empty() {
        None
    } else {
        Some(snapshot_record(state, &record, &existing).await?)
    };
    let hashes = paths
        .iter()
        .map(|path| recovery_file_hash(path))
        .collect::<Result<Vec<_>, _>>()?;
    let database = state.completed_state_database().await?;
    let operation = if let Some(database) = &database {
        Some(
            database
                .prepare_filesystem_operation(
                    "agent_uninstall",
                    &AgentUninstallOperation {
                        previous: record.clone(),
                        paths: paths
                            .iter()
                            .map(|path| path.to_string_lossy().into_owned())
                            .collect(),
                        hashes,
                    },
                )
                .await?,
        )
    } else {
        None
    };
    if let Err(error) =
        remove_agent_artifacts_with(&existing, |path| std::fs::remove_file(path)).await
    {
        if let (Some(database), Some(operation)) = (&database, &operation) {
            database.abort_filesystem_operation(&operation.id).await?;
        }
        return Err(error);
    }
    ledger.remove(index);
    let save = match &operation {
        Some(operation) => save_ledger_after_filesystem(state, &ledger, &operation.id).await,
        None => save_ledger_for(&state.app_data_dir, &ledger).await,
    };
    if let Err(error) = save {
        let restore = match &snapshot {
            Some(snapshot) => history::restore_snapshot(
                &state.app_data_dir,
                &install_identity(&record),
                &snapshot.id,
                &existing,
            )
            .await
            .map(|_| ()),
            None => Ok(()),
        };
        if let (Some(database), Some(operation)) = (&database, &operation) {
            return match restore {
                Ok(()) => {
                    database.abort_filesystem_operation(&operation.id).await?;
                    Err(error)
                }
                Err(rollback) => {
                    database
                        .retain_filesystem_operation_error(&operation.id, &rollback.to_string())
                        .await?;
                    Err(rollback_error("save Agent uninstall", error, rollback))
                }
            };
        }
        if let Err(rollback) = restore {
            return Err(rollback_error("save Agent uninstall", error, rollback));
        }
        return Err(error);
    }
    if let (Some(database), Some(operation)) = (&database, &operation) {
        database.commit_filesystem_operation(&operation.id).await?;
    }
    if tool_is_dir_unit(&record.tool) {
        for path in existing {
            if let Some(parent) = path.parent() {
                let _ = tokio::fs::remove_dir(parent).await;
            }
        }
    }
    Ok(())
}

pub(crate) async fn do_uninstall_legacy(
    app: &AppHandle,
    state: &AppState,
    slug: String,
    tool: Tool,
    project_path: Option<String>,
) -> Result<(), AppError> {
    let reference = resolve_command_reference(app, state, None, None, Some(&slug)).await?;
    do_uninstall(state, reference, tool, project_path).await
}

/// Forget a project WITHOUT touching the files on disk: drop every ledger row
/// whose `project_path` matches, then save. The agent/skill files this app
/// wrote stay exactly where they are — this only makes the app stop tracking
/// them, so the project leaves the Projects list (the Foreign sweep re-scans
/// only project roots the ledger still references, so dropped rows don't come
/// back). Callers that want the files gone use `uninstall_agent` per row first.
#[tauri::command]
pub async fn project_forget(
    _app: AppHandle,
    state: State<'_, AppState>,
    project_path: String,
) -> Result<(), AppError> {
    project_forget_for_state(&state, &project_path).await
}

/// Drop every ledger row whose `project_path` matches, keeping all others
/// (other projects AND user-global rows). Pure so it's unit-testable without an
/// AppHandle; the command serializes recovery and persistence around it.
fn prune_project_rows(records: &mut Vec<InstallRecord>, project_path: &str) {
    records.retain(|r| r.project_path.as_deref() != Some(project_path));
}

async fn project_forget_for_state(state: &AppState, project_path: &str) -> Result<(), AppError> {
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
    recover_agent_operations_locked(state).await?;
    let mut ledger = load_ledger_for_state(state).await?;
    prune_project_rows(&mut ledger, project_path);
    save_ledger_for(&state.app_data_dir, &ledger).await
}

async fn commit_agent_adoptions(
    state: &AppState,
    adopted: Vec<InstallRecord>,
) -> Result<(), AppError> {
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
    recover_agent_operations_locked(state).await?;
    let mut ledger = load_ledger_for_state(state).await?;
    let mut changed = false;

    for record in adopted {
        if ledger
            .iter()
            .any(|current| same_agent_install(current, &record))
        {
            continue;
        }

        let paths = resolved_record_paths(state, &record).await?;
        let (expected, active, disabled) = record_reconcile_hashes(state, &record).await?;
        let expected_active = expected.into_iter().map(Some).collect::<Vec<_>>();
        if active != expected_active || disabled.iter().any(Option::is_some) {
            return Err(AppError::InvalidArgument {
                message: format!(
                    "Agent files changed during Agent adoption: {}:{}",
                    record.source_id, record.relative_path
                ),
            });
        }

        let reference = AgentReference {
            source_id: record.source_id.clone(),
            relative_path: record.relative_path.clone(),
        };
        ensure_destinations_available(
            &ledger,
            &reference,
            &record.tool,
            record.project_path.as_deref(),
            &paths,
            true,
        )?;
        ledger.push(record);
        changed = true;
    }

    if changed {
        save_ledger_for(&state.app_data_dir, &ledger).await?;
    }
    Ok(())
}

pub(crate) async fn mcp_reconcile_agent_installs(
    state: &AppState,
) -> Result<Vec<InstalledAgent>, AppError> {
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
    reconcile_agent_installs_locked(state).await
}

async fn reconcile_agent_installs_locked(
    state: &AppState,
) -> Result<Vec<InstalledAgent>, AppError> {
    let ledger = load_ledger_for_state(state).await?;
    let agent_sources = crate::agents::inspect_agent_sources(&state.app_data_dir).await?;
    let mut installed = Vec::with_capacity(ledger.len());
    for record in ledger {
        let package = find_agent_package(&agent_sources, &record);
        let (expected, active, disabled) = record_reconcile_hashes(state, &record).await?;
        let lifecycle = classify_artifact_install(
            &active.iter().map(Option::as_deref).collect::<Vec<_>>(),
            &disabled.iter().map(Option::as_deref).collect::<Vec<_>>(),
            &expected.iter().map(String::as_str).collect::<Vec<_>>(),
            record.disabled_path.is_some(),
            &record.source_snapshot_hash,
            package.map(|package| package.source_hash.as_str()),
        );
        let update_kind = (lifecycle == InstallState::Outdated).then(|| {
            if package.map(|value| value.body_hash.as_str()) == Some(record.body_hash.as_str()) {
                UpdateKind::Cosmetic
            } else {
                UpdateKind::Substantive
            }
        });
        installed.push(InstalledAgent {
            slug: record.slug.clone(),
            name: package
                .and_then(|value| value.agent.as_ref())
                .map(|agent| agent.name.clone())
                .unwrap_or_else(|| record.slug.clone()),
            source_id: record.source_id,
            relative_path: record.relative_path,
            tool: record.tool,
            scope: record.scope,
            project_path: record.project_path,
            dest: record.dest,
            state: lifecycle,
            update_kind,
            tracked: true,
        });
    }
    Ok(installed)
}

/// The reconciled Library view — every ledger row resolved against disk +
/// Agent sources into one of the seven states.
#[tauri::command]
pub async fn installs_reconcile<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    mut project_roots: Vec<String>,
) -> Result<Vec<InstalledAgent>, AppError> {
    project_roots.extend(
        registered_projects(&state.app_data_dir)
            .await?
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned()),
    );
    let corpus = corpus::ensure_corpus(&app, &state).await?;
    let ledger = load_ledger(&app, &state).await?;
    let agent_sources = crate::agents::inspect_agent_sources(&state.app_data_dir).await?;
    let mut out = Vec::with_capacity(ledger.len());
    for r in &ledger {
        let package = find_agent_package(&agent_sources, r);
        let current_source_hash = package.map(|package| package.source_hash.as_str());
        let (expected, active, disabled) = record_reconcile_hashes(&state, r).await?;
        let st = classify_artifact_install(
            &active.iter().map(Option::as_deref).collect::<Vec<_>>(),
            &disabled.iter().map(Option::as_deref).collect::<Vec<_>>(),
            &expected.iter().map(String::as_str).collect::<Vec<_>>(),
            r.disabled_path.is_some(),
            &r.source_snapshot_hash,
            current_source_hash,
        );
        // Cosmetic vs substantive: only meaningful when Outdated. Body unchanged
        // upstream → the update is metadata-only.
        let update_kind = if st == InstallState::Outdated {
            Some(
                if package.map(|value| value.body_hash.as_str()) == Some(r.body_hash.as_str()) {
                    UpdateKind::Cosmetic
                } else {
                    UpdateKind::Substantive
                },
            )
        } else {
            None
        };
        let name = package
            .and_then(|value| value.agent.as_ref())
            .map(|agent| agent.name.clone())
            .unwrap_or_else(|| r.slug.clone());
        out.push(InstalledAgent {
            slug: r.slug.clone(),
            name,
            source_id: r.source_id.clone(),
            relative_path: r.relative_path.clone(),
            tool: r.tool.clone(),
            scope: r.scope,
            project_path: r.project_path.clone(),
            dest: r.dest.clone(),
            state: st,
            update_kind,
            tracked: true,
        });
    }

    // Foreign sweep: files on disk we did NOT install but recognize as corpus
    // agents (slug matches a known agent). A file that is BYTE-IDENTICAL to the
    // canonical render IS that agent, verbatim — installed outside the app (e.g.
    // the CLI install.sh), but in sync. We surface it as `Current` (nothing to
    // decide). Only a recognized-but-DIFFERENT file (older version, or
    // hand-edited) stays `Foreign` and asks for a look. Scans each supported
    // tool's dir(s) — user dirs + every project dir in the ledger.
    let ledger_keys: std::collections::HashSet<(String, Tool, Option<String>)> = ledger
        .iter()
        .map(|r| (r.slug.clone(), r.tool.clone(), r.project_path.clone()))
        .collect();
    // Every project root we know about: ledger dirs UNION the caller's registered
    // project roots. The latter is why a just-added folder (or one whose rows were
    // dropped by "Remove from app only") re-surfaces its on-disk agents instead of
    // staying invisible until something new is installed into it.
    let project_dirs: Vec<PathBuf> = ledger
        .iter()
        .filter_map(|r| r.project_path.clone())
        .chain(project_roots)
        .collect::<std::collections::BTreeSet<String>>()
        .into_iter()
        .map(PathBuf::from)
        .collect();
    // Byte-perfect foreign matches are adopted into the ledger (see below) —
    // collect the new rows here and persist them once after the sweep.
    let mut adopted: Vec<InstallRecord> = Vec::new();
    let mut adopted_seen: std::collections::HashSet<(String, Tool, Option<String>)> =
        std::collections::HashSet::new();
    for tool in supported() {
        // Resolve each tool against its own base (honors a per-tool custom
        // path, e.g. a WSL home), so the sweep looks where the tool lives.
        let home = tool_home(&state, tool).await?;
        // Some tools namespace the output slug (e.g. Osaurus dirs are
        // `agency-<slug>`); strip it before recognizing the agent.
        let prefix = crate::registry::get(tool)
            .and_then(|m| m.slug_prefix.as_deref())
            .unwrap_or("");
        // Dual-scope tools are scanned in BOTH places: the user-global dir (key
        // None) AND every project root the ledger knows about (key Some(path)).
        // Each entry: (scope-key, agents-root, suffix-after-`{slug}`).
        let mut scan_roots: Vec<(Option<String>, PathBuf, String)> = Vec::new();
        if render::supports_user(tool) {
            scan_roots.extend(
                agent_units(tool, &home, None)
                    .into_iter()
                    .map(|(d, s)| (None, d, s)),
            );
        }
        if render::supports_project(tool) {
            scan_roots.extend(project_dirs.iter().flat_map(|p| {
                let key = Some(p.to_string_lossy().to_string());
                agent_units(tool, &home, Some(p))
                    .into_iter()
                    .map(move |(d, s)| (key.clone(), d, s))
            }));
        }
        for (proj, agents_root, suffix) in scan_roots {
            let mut rd = match tokio::fs::read_dir(&agents_root).await {
                Ok(r) => r,
                Err(_) => continue,
            };
            // A leading `/` in the suffix means the per-agent unit is a DIRECTORY
            // (e.g. `{slug}/SKILL.md`); otherwise it's a file (`{slug}.md`).
            let dir_unit = suffix.starts_with('/');
            while let Ok(Some(ent)) = rd.next_entry().await {
                let name = ent.file_name();
                let Some(name) = name.to_str() else { continue };
                // Recover the `{slug}` token + the file that holds the canonical
                // bytes. Dir unit: the entry IS the slug dir, bytes at <dir>/<leaf>.
                // File unit: the entry is `<slug><suffix>`.
                let (token, byte_path) = if dir_unit {
                    (
                        name.to_string(),
                        agents_root.join(name).join(suffix.trim_start_matches('/')),
                    )
                } else if name.ends_with(suffix.as_str()) && name.len() > suffix.len() {
                    (
                        name[..name.len() - suffix.len()].to_string(),
                        agents_root.join(name),
                    )
                } else {
                    continue; // not a unit for this template (stray file/dir)
                };
                let cand = token.strip_prefix(prefix).unwrap_or(&token);
                let Some(agent) = corpus
                    .get(cand)
                    .or_else(|| corpus.get_by_conversion_slug(cand))
                else {
                    continue; // unrecognized → not ours to claim
                };
                let slug = agent.slug.clone();
                if ledger_keys.contains(&(slug.clone(), tool.to_string(), proj.clone())) {
                    continue; // already in the ledger
                }
                // Read the on-disk bytes + canonical source once. A byte-perfect
                // match is unambiguously our render, so ADOPT it into the ledger
                // (tracked) — the app then manages it like any install, whether
                // the CLI or the app wrote it. Only agency-catalog agents ever get
                // here (recognized above), so we never claim unrelated files. A
                // recognized-but-DIVERGENT file stays Foreign + untracked.
                let raw = corpus::read_source(&app, &agent.category, &slug).await.ok();
                let canonical = match raw.as_deref() {
                    Some(raw) => {
                        destinations_match_render(
                            &agent,
                            raw,
                            tool,
                            cand,
                            &home,
                            proj.as_deref().map(Path::new),
                        )
                        .await
                    }
                    None => false,
                };
                let mut tracked = false;
                let state = if canonical {
                    if let (Some(rw), Some(entry)) = (raw.as_deref(), corpus.entry(&slug)) {
                        let key = (slug.clone(), tool.to_string(), proj.clone());
                        if !adopted_seen.contains(&key) {
                            if let Ok(rec) = track_agent_record(
                                &agent,
                                rw,
                                tool,
                                &home,
                                proj.as_deref().map(std::path::Path::new),
                                &entry.source_hash,
                                &entry.body_hash,
                                &corpus.version(),
                                &now_iso(),
                            ) {
                                adopted.push(rec);
                                adopted_seen.insert(key);
                            }
                        }
                        tracked = true;
                    }
                    InstallState::Current
                } else {
                    InstallState::Foreign
                };
                out.push(InstalledAgent {
                    slug,
                    name: agent.name.clone(),
                    source_id: if tracked {
                        crate::agents::BUILTIN_AGENT_SOURCE_ID.into()
                    } else {
                        String::new()
                    },
                    relative_path: if tracked {
                        format!("{}/{}.md", agent.category, agent.slug)
                    } else {
                        String::new()
                    },
                    tool: tool.to_string(),
                    scope: render::scope_for(proj.as_deref().map(std::path::Path::new)),
                    project_path: proj.clone(),
                    dest: byte_path.to_string_lossy().to_string(),
                    state,
                    update_kind: None,
                    tracked,
                });
            }
        }
    }

    // Persist the byte-perfect adoptions in one write. Idempotent: next reconcile
    // finds them in the ledger (skipped by the sweep), so steady state is no write.
    if !adopted.is_empty() {
        commit_agent_adoptions(&state, adopted).await?;
    }

    // Collapse to one row per LOGICAL install (slug, tool, project). Copilot
    // dual-writes to ~/.github and ~/.copilot, so the Foreign sweep finds the
    // same agent twice; other tools could too. One logical install = one row
    // (its Track/Update/Remove already cover every physical dest).
    let mut seen = std::collections::HashSet::new();
    out.retain(|a| seen.insert((a.slug.clone(), a.tool.clone(), a.project_path.clone())));

    prune_migration_backups(&app).await?;

    Ok(out)
}

/// For the Foreign sweep: each scannable agents-root for a tool, paired with the
/// path suffix that follows `{slug}` in the dest template. The suffix tells the
/// sweep whether a per-agent UNIT is a file (e.g. `.md`) or a directory (e.g.
/// `/SKILL.md`), and where the canonical bytes live inside a dir unit.
///
/// Splitting the TEMPLATE (not a `{slug}`-substituted path) is what makes
/// dir-structured tools work: Osaurus's `.osaurus/skills/{slug}/SKILL.md` scans
/// `.osaurus/skills` instead of a bogus `.osaurus/skills/_probe`.
fn agent_units(tool: &str, home: &Path, project_root: Option<&Path>) -> Vec<(PathBuf, String)> {
    let Some(meta) = crate::registry::get(tool) else {
        return Vec::new();
    };
    let Some(dest) = meta.dest.as_ref() else {
        return Vec::new();
    };
    let (templates, root): (&[String], &Path) = match project_root {
        Some(p) => (&dest.project, p),
        None => (&dest.user, home),
    };
    let templates = if matches!(tool, "kimi" | "openclaw") {
        &templates[..templates.len().min(1)]
    } else {
        templates
    };
    templates
        .iter()
        .filter_map(|t| {
            t.split_once("{slug}")
                .map(|(before, after)| (root.join(before), after.to_string()))
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// All install records that match a given agent (for the persona detail panel).
#[tauri::command]
pub async fn installs_for_agent(
    app: AppHandle,
    state: State<'_, AppState>,
    slug: String,
) -> Result<Vec<InstallRecord>, AppError> {
    let ledger = load_ledger(&app, &state).await?;
    Ok(ledger.into_iter().filter(|r| r.slug == slug).collect())
}

/// Detected AI tools + their deployment surface and installed counts.
#[tauri::command]
pub async fn tools_list(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<ToolInfo>, AppError> {
    let ledger = load_ledger(&app, &state).await?;
    let os_home = home()?;
    let supported = supported();
    let mut out = Vec::with_capacity(supported.len());
    for tool in supported {
        let installed_count = ledger.iter().filter(|r| r.tool == tool).count() as u32;
        // Resolve against the per-tool base so detection + user_dest reflect a
        // custom path (e.g. a WSL home). custom_path exposes the override to the
        // UI (None when it equals the OS home).
        let home = tool_home(&state, tool).await?;
        let custom_path = (home != os_home).then(|| home.to_string_lossy().to_string());
        let (detected, user_dest) = detect(tool, &home);
        out.push(ToolInfo {
            tool: tool.to_string(),
            label: render::label(tool),
            detected,
            // Primary/display scope: dual-scope tools read "user" (global-first);
            // Cursor is the project-only exception. Per-install scope is derived
            // from the chosen project root, not this field.
            scope: if render::supports_user(tool) {
                crate::types::Scope::User
            } else {
                crate::types::Scope::Project
            },
            user_dest,
            installed_count,
            custom_path,
        });
    }
    Ok(out)
}

pub(crate) async fn tool_detected(state: &AppState, tool: &str) -> Result<bool, AppError> {
    let base = tool_home(state, tool).await?;
    Ok(detect(tool, &base).0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
enum RevealPlatform {
    MacOs,
    Windows,
    Linux,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RevealOpenerSpec {
    program: OsString,
    args: Vec<OsString>,
}

fn current_reveal_platform() -> RevealPlatform {
    #[cfg(target_os = "macos")]
    return RevealPlatform::MacOs;
    #[cfg(target_os = "windows")]
    return RevealPlatform::Windows;
    #[cfg(all(unix, not(target_os = "macos")))]
    return RevealPlatform::Linux;
}

fn reveal_opener_spec(
    target: &Path,
    target_is_dir: bool,
    platform: RevealPlatform,
) -> RevealOpenerSpec {
    match platform {
        RevealPlatform::MacOs => RevealOpenerSpec {
            program: OsString::from("/usr/bin/open"),
            args: vec![OsString::from("-R"), target.as_os_str().to_owned()],
        },
        RevealPlatform::Windows => {
            let args = if target_is_dir {
                vec![target.as_os_str().to_owned()]
            } else {
                vec![OsString::from("/select,"), target.as_os_str().to_owned()]
            };
            RevealOpenerSpec {
                program: OsString::from("explorer"),
                args,
            }
        }
        RevealPlatform::Linux => RevealOpenerSpec {
            program: OsString::from("xdg-open"),
            args: vec![if target_is_dir {
                target.as_os_str().to_owned()
            } else {
                target.parent().unwrap_or(target).as_os_str().to_owned()
            }],
        },
    }
}

fn validate_reveal_target(path: &str, roots: &[PathBuf]) -> Result<PathBuf, AppError> {
    let supplied = Path::new(path);
    let normalized = supplied.components().collect::<PathBuf>();
    if path.is_empty()
        || path.contains("://")
        || path.to_ascii_lowercase().starts_with("file:")
        || !supplied.is_absolute()
        || supplied
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
        || normalized.as_os_str() != supplied.as_os_str()
    {
        return Err(AppError::InvalidArgument {
            message: "reveal path must be an absolute normalized filesystem path".into(),
        });
    }
    std::fs::symlink_metadata(supplied).map_err(|error| AppError::InvalidArgument {
        message: format!("reveal path must exist: {error}"),
    })?;
    let canonical = std::fs::canonicalize(supplied).map_err(|error| AppError::InvalidArgument {
        message: format!("could not canonicalize reveal path: {error}"),
    })?;
    if roots.iter().any(|root| canonical.starts_with(root)) {
        Ok(canonical)
    } else {
        Err(AppError::InvalidArgument {
            message: "reveal path is outside supported Agency Agents locations".into(),
        })
    }
}

async fn reveal_allowed_roots(state: &AppState) -> Result<Vec<PathBuf>, AppError> {
    reveal_allowed_roots_for_home(state, &home()?).await
}

async fn reveal_allowed_roots_for_home(
    state: &AppState,
    home: &Path,
) -> Result<Vec<PathBuf>, AppError> {
    let mut candidates = vec![state.app_data_dir.clone()];
    let settings = state.settings.read().await.effective_settings();
    for tool in registry::wired().filter(|tool| tool.supports_user()) {
        let base = settings
            .as_ref()
            .map(|settings| resolve_tool_base(&settings.tool_paths, &tool.id, home))
            .unwrap_or_else(|| home.to_path_buf());
        if let Some(destinations) = tool.dest.as_ref() {
            candidates.extend(destinations.user.iter().filter_map(|template| {
                template
                    .split_once("{slug}")
                    .map(|(prefix, _)| base.join(prefix))
            }));
        }
    }
    for runtime in ["claudeCode", "codex"] {
        candidates.push(
            crate::skills::install::target_path(home, None, runtime, "probe")?
                .parent()
                .expect("skill target always has a parent")
                .to_path_buf(),
        );
    }

    let mut projects = registered_projects(&state.app_data_dir).await?;
    projects.extend(
        load_ledger_for_state(state)
            .await?
            .into_iter()
            .filter_map(|record| record.project_path.map(PathBuf::from)),
    );
    projects.extend(
        crate::skills::install::load_ledger_for_state(state)
            .await?
            .into_iter()
            .filter_map(|record| record.project_path.map(PathBuf::from)),
    );
    for project in projects {
        candidates.push(project.clone());
        for runtime in ["claudeCode", "codex"] {
            candidates.push(
                crate::skills::install::target_path(home, Some(&project), runtime, "probe")?
                    .parent()
                    .expect("skill target always has a parent")
                    .to_path_buf(),
            );
        }
    }

    Ok(candidates
        .into_iter()
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect())
}

fn reveal_path_from_roots_with_executor<F>(
    path: String,
    roots: Vec<PathBuf>,
    platform: RevealPlatform,
    executor: F,
) -> Result<(), AppError>
where
    F: FnOnce(RevealOpenerSpec) -> std::io::Result<bool>,
{
    let target = validate_reveal_target(&path, &roots)?;
    let spec = reveal_opener_spec(&target, target.is_dir(), platform);
    let success = executor(spec).map_err(|error| AppError::Io {
        message: format!("could not reveal {path}: {error}"),
    })?;
    if success {
        Ok(())
    } else {
        Err(AppError::Io {
            message: format!("file manager could not reveal {path}"),
        })
    }
}

pub(crate) async fn reveal_path_for_state(state: &AppState, path: String) -> Result<(), AppError> {
    let roots = reveal_allowed_roots(state).await?;
    tokio::task::spawn_blocking(move || {
        reveal_path_from_roots_with_executor(path, roots, current_reveal_platform(), |spec| {
            std::process::Command::new(&spec.program)
                .args(&spec.args)
                .status()
                .map(|status| status.success())
        })
    })
    .await
    .map_err(|error| AppError::Io {
        message: format!("file manager task failed: {error}"),
    })?
}

/// Reveal an app-owned or supported installation path in the OS file manager.
#[tauri::command]
pub async fn reveal_path(state: State<'_, AppState>, path: String) -> Result<(), AppError> {
    reveal_path_for_state(&state, path).await
}

/// The `<bin> --version`-style probe command for a tool, or `None` when we don't
/// know one. Best-effort and uneven by nature — GUI tools may not ship a CLI.
fn version_cmd(tool: &str) -> Option<(&'static str, Vec<&'static str>)> {
    if tool == "openclaw" {
        return None;
    }
    // The registry is cached for the process lifetime (`OnceLock`), so its
    // `&str`s are effectively `'static` — fine to hand to the version probe.
    let v = registry::get(tool)?.version.as_ref()?;
    Some((v.bin.as_str(), v.args.iter().map(String::as_str).collect()))
}

/// First non-empty trimmed line of version output, capped to a sane length.
fn first_version_line(s: &str) -> Option<String> {
    s.lines().map(str::trim).find(|l| !l.is_empty()).map(|l| {
        let capped: String = l.chars().take(48).collect();
        capped
    })
}

async fn probe_version(tool: &str) -> Option<String> {
    let (bin, args) = version_cmd(tool)?;
    let fut = tokio::process::Command::new(bin).args(args).output();
    match tokio::time::timeout(std::time::Duration::from_secs(3), fut).await {
        Ok(Ok(o)) if o.status.success() => first_version_line(&String::from_utf8_lossy(&o.stdout))
            .or_else(|| first_version_line(&String::from_utf8_lossy(&o.stderr))),
        _ => None,
    }
}

/// Best-effort version probe across all supported tools, run concurrently with a
/// per-tool timeout. A tool whose binary isn't on PATH (or that has no known
/// version command) comes back as `version: None` — the UI just omits it.
#[tauri::command]
pub async fn tool_versions() -> Result<Vec<ToolVersion>, AppError> {
    let supported = supported();
    let mut handles = Vec::with_capacity(supported.len());
    for tool in supported {
        handles.push(tokio::spawn(async move {
            ToolVersion {
                tool: tool.to_string(),
                version: probe_version(tool).await,
            }
        }));
    }
    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        if let Ok(v) = h.await {
            out.push(v);
        }
    }
    Ok(out)
}

fn project_registry_path(app_data_dir: &Path) -> PathBuf {
    corpus::state_dir(app_data_dir).join("projects.json")
}

fn lock_project_registry(app_data_dir: &Path) -> Result<std::fs::File, AppError> {
    let directory = corpus::state_dir(app_data_dir);
    std::fs::create_dir_all(&directory).map_err(|error| AppError::Io {
        message: format!("create project registry directory: {error}"),
    })?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("projects.lock"))
        .map_err(|error| AppError::Io {
            message: format!("open project registry lock: {error}"),
        })?;
    file.lock().map_err(|error| AppError::Io {
        message: format!("lock project registry: {error}"),
    })?;
    Ok(file)
}

pub(crate) async fn registered_projects(app_data_dir: &Path) -> Result<Vec<PathBuf>, AppError> {
    if let Some(database) = crate::state_db::StateDatabase::completed(app_data_dir).await? {
        return database
            .read(projects_spec())
            .await?
            .ok_or_else(|| AppError::StorageCorrupt {
                message: "project registry is missing after SQLite migration".into(),
            });
    }
    let path = project_registry_path(app_data_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = read_capped(&path, MAX_PROJECT_REGISTRY_BYTES).await?;
    let paths: Vec<String> = serde_json::from_slice(&raw).map_err(|error| AppError::Io {
        message: format!("parse projects.json: {error}"),
    })?;
    if paths.len() > MAX_REGISTERED_PROJECTS {
        return Err(AppError::InvalidArgument {
            message: "project registry exceeds limit".into(),
        });
    }
    Ok(paths.into_iter().map(PathBuf::from).collect())
}

async fn save_registered_projects(
    app_data_dir: &Path,
    projects: &[PathBuf],
) -> Result<(), AppError> {
    validate_projects(projects)?;
    if let Some(database) = crate::state_db::StateDatabase::completed(app_data_dir).await? {
        let replacement = projects.to_vec();
        return database
            .mutate(projects_spec(), Vec::new(), move |current| {
                *current = replacement;
                Ok(())
            })
            .await;
    }
    let bytes = serde_json::to_vec_pretty(
        &projects
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
    )
    .map_err(|error| AppError::Internal {
        message: format!("serialize projects.json: {error}"),
    })?;
    atomic_write(&project_registry_path(app_data_dir), &bytes).await
}

fn validate_projects(projects: &[PathBuf]) -> Result<(), AppError> {
    let mut unique = std::collections::HashSet::new();
    if projects.len() > MAX_REGISTERED_PROJECTS
        || projects.iter().any(|path| {
            !path.is_absolute()
                || path.components().any(|component| {
                    matches!(
                        component,
                        std::path::Component::CurDir | std::path::Component::ParentDir
                    )
                })
                || !unique.insert(path)
        })
    {
        return Err(AppError::InvalidArgument {
            message: "project registry is invalid or exceeds its limit".into(),
        });
    }
    Ok(())
}

fn projects_spec() -> crate::state_db::DocumentSpec<Vec<PathBuf>> {
    crate::state_db::DocumentSpec::new("projects", 1, MAX_PROJECT_REGISTRY_BYTES, |projects| {
        validate_projects(projects)
    })
}

pub(crate) fn projects_import_spec() -> crate::state_db::ImportSpec {
    crate::state_db::ImportSpec::document(projects_spec(), Vec::new())
}

async fn register_project(app_data_dir: &Path, path: &str) -> Result<PathBuf, AppError> {
    let supplied = Path::new(path);
    if !supplied.is_absolute()
        || supplied.components().any(|part| {
            matches!(
                part,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        return Err(AppError::InvalidArgument {
            message: "project path must be absolute and normalized".into(),
        });
    }
    let metadata = std::fs::symlink_metadata(supplied).map_err(|error| AppError::Io {
        message: format!("inspect project path: {error}"),
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AppError::InvalidArgument {
            message: "project path must be a real directory, not a link".into(),
        });
    }
    let canonical = std::fs::canonicalize(supplied).map_err(|error| AppError::Io {
        message: format!("canonicalize project path: {error}"),
    })?;
    let _lock = lock_project_registry(app_data_dir)?;
    let mut projects = registered_projects(app_data_dir).await?;
    if !projects.contains(&canonical) {
        if projects.len() >= MAX_REGISTERED_PROJECTS {
            return Err(AppError::InvalidArgument {
                message: "project registry is full".into(),
            });
        }
        projects.push(canonical.clone());
        projects.sort();
        save_registered_projects(app_data_dir, &projects).await?;
    }
    Ok(canonical)
}

async fn unregister_project(app_data_dir: &Path, path: &str) -> Result<bool, AppError> {
    let canonical = PathBuf::from(path);
    let _lock = lock_project_registry(app_data_dir)?;
    let mut projects = registered_projects(app_data_dir).await?;
    let before = projects.len();
    projects.retain(|project| project != &canonical);
    if projects.len() == before {
        return Ok(false);
    }
    save_registered_projects(app_data_dir, &projects).await?;
    if let Some(database) = crate::state_db::StateDatabase::completed(app_data_dir).await? {
        let removed = canonical.to_string_lossy().into_owned();
        database
            .mutate(
                corpus::control_center_spec(),
                crate::types::ControlCenterDocument::default(),
                move |document| {
                    document
                        .project_baselines
                        .retain(|baseline| baseline.project_path != removed);
                    document
                        .project_subscriptions
                        .retain(|subscription| subscription.project_path != removed);
                    Ok(())
                },
            )
            .await?;
    }
    Ok(true)
}

pub(crate) async fn project_is_registered(
    app_data_dir: &Path,
    path: &Path,
) -> Result<bool, AppError> {
    Ok(registered_projects(app_data_dir)
        .await?
        .iter()
        .any(|project| project == path))
}

#[tauri::command]
pub async fn project_register(
    state: State<'_, AppState>,
    path: String,
) -> Result<String, AppError> {
    Ok(register_project(&state.app_data_dir, &path)
        .await?
        .to_string_lossy()
        .into_owned())
}

#[tauri::command]
pub async fn project_unregister(
    state: State<'_, AppState>,
    path: String,
) -> Result<bool, AppError> {
    unregister_project(&state.app_data_dir, &path).await
}

/// Registered project roots union project-scoped agent ledger entries.
#[tauri::command]
pub async fn projects_list(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<ProjectInfo>, AppError> {
    let ledger = load_ledger(&app, &state).await?;
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for path in registered_projects(&state.app_data_dir).await? {
        counts
            .entry(path.to_string_lossy().into_owned())
            .or_default();
    }
    for r in &ledger {
        if let Some(p) = &r.project_path {
            *counts.entry(p.clone()).or_default() += 1;
        }
    }
    Ok(counts
        .into_iter()
        .map(|(path, installed_count)| {
            let label = Path::new(&path)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| path.clone());
            ProjectInfo {
                path,
                label,
                installed_count,
            }
        })
        .collect())
}

// ---------- Project readiness and opt-in catalog subscriptions ----------

struct ReadinessEvidence {
    agents: Result<BTreeMap<BaselineAgentRequirement, ReadinessRowState>, String>,
    skills: Result<BTreeMap<BaselineSkillRequirement, ReadinessRowState>, String>,
    instructions: Result<BTreeMap<String, ReadinessRowState>, String>,
    mcp_servers: Result<BTreeMap<String, ReadinessRowState>, String>,
    tools: Result<BTreeMap<Tool, ReadinessRowState>, String>,
}

const LEGACY_UNREVIEWED_TARGET: &str = "legacyUnreviewed";

fn exact_agent_requirements(baseline: &ProjectReadinessBaseline) -> Vec<BaselineAgentRequirement> {
    if !baseline.agent_requirements.is_empty() {
        return baseline.agent_requirements.clone();
    }
    let tool = baseline
        .tools
        .as_slice()
        .first()
        .filter(|_| baseline.tools.len() == 1)
        .cloned()
        .unwrap_or_else(|| LEGACY_UNREVIEWED_TARGET.into());
    baseline
        .agents
        .iter()
        .cloned()
        .map(|reference| BaselineAgentRequirement {
            reference,
            tool: tool.clone(),
        })
        .collect()
}

fn exact_skill_requirements(baseline: &ProjectReadinessBaseline) -> Vec<BaselineSkillRequirement> {
    if !baseline.skill_requirements.is_empty() {
        return baseline.skill_requirements.clone();
    }
    baseline
        .skills
        .iter()
        .cloned()
        .map(|reference| BaselineSkillRequirement {
            reference,
            runtime: LEGACY_UNREVIEWED_TARGET.into(),
        })
        .collect()
}

impl ReadinessEvidence {
    #[cfg(test)]
    fn ready() -> Self {
        Self {
            agents: Ok(BTreeMap::new()),
            skills: Ok(BTreeMap::new()),
            instructions: Ok(BTreeMap::new()),
            mcp_servers: Ok(BTreeMap::new()),
            tools: Ok(BTreeMap::new()),
        }
    }
}

fn readiness_category_state(rows: &[ReadinessRow]) -> ReadinessCategoryState {
    if rows.is_empty() {
        ReadinessCategoryState::NotRequired
    } else if rows
        .iter()
        .any(|row| row.state == ReadinessRowState::Unavailable)
    {
        ReadinessCategoryState::Unavailable
    } else if rows
        .iter()
        .any(|row| row.state == ReadinessRowState::NeedsAttention)
    {
        ReadinessCategoryState::NeedsAttention
    } else if rows
        .iter()
        .any(|row| row.state == ReadinessRowState::Unverifiable)
    {
        ReadinessCategoryState::Unverifiable
    } else {
        ReadinessCategoryState::Ready
    }
}

fn readiness_rows<T: Ord>(
    required: impl IntoIterator<Item = (T, String, String)>,
    evidence: &Result<BTreeMap<T, ReadinessRowState>, String>,
) -> Vec<ReadinessRow> {
    required
        .into_iter()
        .map(|(key, id, label)| {
            let state = match evidence {
                Ok(states) => states
                    .get(&key)
                    .copied()
                    .unwrap_or(ReadinessRowState::NeedsAttention),
                Err(_) => ReadinessRowState::Unavailable,
            };
            let detail = match (state, evidence) {
                (ReadinessRowState::Ready, _) => "Current evidence matches the baseline",
                (ReadinessRowState::NeedsAttention, _) => {
                    "Required current evidence is missing or drifted"
                }
                (ReadinessRowState::Unavailable, Err(_)) => {
                    "Required inspection failed; retry for current evidence"
                }
                (ReadinessRowState::Unavailable, _) => "Required evidence is unavailable",
                (ReadinessRowState::Unverifiable, _) => {
                    "Requirement is not mapped to a known bounded identifier"
                }
            };
            ReadinessRow {
                id,
                label,
                state,
                evidence: crate::commands::doctor::sanitize_field(
                    detail,
                    dirs::home_dir().as_deref(),
                ),
            }
        })
        .collect()
}

fn requirement_rows(
    requirements: &[BaselineRequirement],
    evidence: &Result<BTreeMap<String, ReadinessRowState>, String>,
) -> Vec<ReadinessRow> {
    requirements
        .iter()
        .map(|requirement| {
            let state = if !requirement.known {
                ReadinessRowState::Unverifiable
            } else {
                match evidence {
                    Ok(states) => states
                        .get(&requirement.id)
                        .copied()
                        .unwrap_or(ReadinessRowState::NeedsAttention),
                    Err(_) => ReadinessRowState::Unavailable,
                }
            };
            ReadinessRow {
                id: requirement.id.clone(),
                label: requirement.id.clone(),
                state,
                evidence: match state {
                    ReadinessRowState::Ready => "Known requirement is present".into(),
                    ReadinessRowState::NeedsAttention => "Known requirement is not present".into(),
                    ReadinessRowState::Unavailable => {
                        "Required inspection failed; retry for current evidence".into()
                    }
                    ReadinessRowState::Unverifiable => {
                        "Opaque requirement; map it to a known bounded identifier to verify it"
                            .into()
                    }
                },
            }
        })
        .collect()
}

fn build_readiness_report(
    project_path: &str,
    baseline: Option<&ProjectReadinessBaseline>,
    evidence: ReadinessEvidence,
) -> ProjectReadinessReport {
    let Some(baseline) = baseline else {
        return ProjectReadinessReport {
            project_path: project_path.into(),
            overall: ProjectReadinessOverall::NotConfigured,
            baseline: None,
            subscribed: false,
            categories: [
                ReadinessCategoryKind::AgentRoster,
                ReadinessCategoryKind::Skills,
                ReadinessCategoryKind::Instructions,
                ReadinessCategoryKind::Mcp,
                ReadinessCategoryKind::Tools,
            ]
            .into_iter()
            .map(|category| ReadinessCategoryReport {
                category,
                state: ReadinessCategoryState::NotRequired,
                rows: Vec::new(),
            })
            .collect(),
        };
    };
    let agent_rows = readiness_rows(
        exact_agent_requirements(baseline)
            .into_iter()
            .map(|requirement| {
                let id = format!(
                    "{}:{}:{}",
                    requirement.reference.source_id,
                    requirement.reference.relative_path,
                    requirement.tool
                );
                (requirement, id.clone(), id)
            }),
        &evidence.agents,
    );
    let skill_rows = readiness_rows(
        exact_skill_requirements(baseline)
            .into_iter()
            .map(|requirement| {
                let id = format!(
                    "{}:{}:{}",
                    requirement.reference.source_id,
                    requirement.reference.relative_path,
                    requirement.runtime
                );
                (requirement, id.clone(), id)
            }),
        &evidence.skills,
    );
    let instruction_rows = requirement_rows(&baseline.instructions, &evidence.instructions);
    let mcp_rows = requirement_rows(&baseline.mcp_servers, &evidence.mcp_servers);
    let tool_rows = readiness_rows(
        baseline
            .tools
            .iter()
            .map(|tool| (tool.clone(), tool.clone(), render::label(tool))),
        &evidence.tools,
    );
    let categories = vec![
        (ReadinessCategoryKind::AgentRoster, agent_rows),
        (ReadinessCategoryKind::Skills, skill_rows),
        (ReadinessCategoryKind::Instructions, instruction_rows),
        (ReadinessCategoryKind::Mcp, mcp_rows),
        (ReadinessCategoryKind::Tools, tool_rows),
    ]
    .into_iter()
    .map(|(category, rows)| ReadinessCategoryReport {
        category,
        state: readiness_category_state(&rows),
        rows,
    })
    .collect::<Vec<_>>();
    let overall = if categories
        .iter()
        .any(|category| category.state == ReadinessCategoryState::Unavailable)
    {
        ProjectReadinessOverall::Unavailable
    } else if categories.iter().any(|category| {
        matches!(
            category.state,
            ReadinessCategoryState::NeedsAttention | ReadinessCategoryState::Unverifiable
        )
    }) {
        ProjectReadinessOverall::NeedsAttention
    } else {
        ProjectReadinessOverall::Ready
    };
    ProjectReadinessReport {
        project_path: project_path.into(),
        overall,
        baseline: Some(baseline.clone()),
        subscribed: false,
        categories,
    }
}

async fn control_center_database(
    state: &AppState,
) -> Result<crate::state_db::StateDatabase, AppError> {
    state
        .completed_state_database()
        .await?
        .ok_or_else(|| AppError::StorageCorrupt {
            message: "Storage migration must complete before project readiness changes".into(),
        })
}

async fn exact_registered_project(
    state: &AppState,
    project_path: &str,
) -> Result<String, AppError> {
    Ok(
        canonical_registered_instruction_project(state, project_path)
            .await?
            .to_string_lossy()
            .into_owned(),
    )
}

async fn persist_project_baseline(
    state: &AppState,
    baseline: ProjectReadinessBaseline,
    subscribe: bool,
) -> Result<ProjectReadinessBaseline, AppError> {
    let returned = baseline.clone();
    control_center_database(state)
        .await?
        .mutate(
            corpus::control_center_spec(),
            crate::types::ControlCenterDocument::default(),
            move |document| apply_project_baseline_and_subscription(document, baseline, subscribe),
        )
        .await?;
    Ok(returned)
}

async fn persist_registered_project_baseline(
    state: &AppState,
    supplied_project_path: &str,
    mut baseline: ProjectReadinessBaseline,
    subscribe: bool,
) -> Result<ProjectReadinessBaseline, AppError> {
    let _project_lock = lock_project_registry(&state.app_data_dir)?;
    baseline.project_path = exact_registered_project(state, supplied_project_path).await?;
    persist_project_baseline(state, baseline, subscribe).await
}

fn apply_project_baseline_and_subscription(
    document: &mut crate::types::ControlCenterDocument,
    baseline: ProjectReadinessBaseline,
    subscribe: bool,
) -> Result<(), AppError> {
    let mut next = document.clone();
    let project_path = baseline.project_path.clone();
    next.project_baselines
        .retain(|existing| existing.project_path != project_path);
    next.project_baselines.push(baseline);
    next.project_baselines
        .sort_by(|left, right| left.project_path.cmp(&right.project_path));
    if subscribe {
        set_project_subscription(&mut next, project_path, true)?;
    }
    corpus::validate_control_center(&next)?;
    *document = next;
    Ok(())
}

#[cfg(test)]
fn resolve_team_references(
    mut slugs: Vec<String>,
    sources: &[AgentSourceResult],
) -> Result<Vec<AgentReference>, AppError> {
    slugs.sort();
    slugs.dedup();
    if slugs.len() > 256 || slugs.iter().any(|slug| slug.is_empty()) {
        return Err(AppError::InvalidArgument {
            message: "Team baseline exceeds its Agent limit".into(),
        });
    }
    if sources.iter().any(|source| !source.errors.is_empty()) {
        return Err(AppError::InvalidArgument {
            message: "Agent source inspection is incomplete; retry before saving the Team baseline"
                .into(),
        });
    }
    let mut agents = Vec::with_capacity(slugs.len());
    for slug in slugs {
        let matches = sources
            .iter()
            .flat_map(|source| &source.agents)
            .filter(|package| {
                package.installable
                    && package
                        .agent
                        .as_ref()
                        .is_some_and(|agent| agent.slug == slug)
            })
            .map(|package| package.reference.clone())
            .collect::<Vec<_>>();
        let [reference] = matches.as_slice() else {
            return Err(AppError::InvalidArgument {
                message: format!(
                    "Team Agent '{slug}' is unavailable or ambiguous across current sources"
                ),
            });
        };
        agents.push(reference.clone());
    }
    agents.sort();
    Ok(agents)
}

fn baseline_from_workspace_pack(
    project_path: String,
    pack: WorkspacePack,
) -> ProjectReadinessBaseline {
    let mut agent_requirements = pack
        .agents
        .iter()
        .map(|item| BaselineAgentRequirement {
            reference: item.reference.clone(),
            tool: item.tool.clone(),
        })
        .collect::<Vec<_>>();
    let mut skill_requirements = pack
        .skills
        .iter()
        .map(|item| BaselineSkillRequirement {
            reference: item.reference.clone(),
            runtime: item.runtime.clone(),
        })
        .collect::<Vec<_>>();
    let mut agents = pack
        .agents
        .iter()
        .map(|item| item.reference.clone())
        .collect::<Vec<_>>();
    let mut skills = pack
        .skills
        .iter()
        .map(|item| item.reference.clone())
        .collect::<Vec<_>>();
    let mut tools = pack
        .agents
        .iter()
        .map(|item| item.tool.clone())
        .chain(pack.skills.iter().map(|item| item.runtime.clone()))
        .collect::<Vec<_>>();
    agents.sort();
    agents.dedup();
    skills.sort();
    skills.dedup();
    tools.sort();
    tools.dedup();
    agent_requirements.sort();
    agent_requirements.dedup();
    skill_requirements.sort();
    skill_requirements.dedup();
    ProjectReadinessBaseline {
        project_path,
        label: pack.name,
        agent_requirements,
        skill_requirements,
        agents,
        skills,
        instructions: pack
            .instructions
            .into_iter()
            .map(|id| BaselineRequirement { id, known: false })
            .collect(),
        mcp_servers: pack
            .mcp_servers
            .into_iter()
            .map(|id| BaselineRequirement { id, known: false })
            .collect(),
        tools,
    }
}

fn validate_team_requirements(
    mut requirements: Vec<BaselineAgentRequirement>,
    sources: &[AgentSourceResult],
    installed: &[InstalledAgent],
    project_path: &str,
) -> Result<Vec<BaselineAgentRequirement>, AppError> {
    requirements.sort();
    requirements.dedup();
    if requirements.is_empty() || requirements.len() > 256 {
        return Err(AppError::InvalidArgument {
            message: "Team baseline requires at least one reviewed Agent target".into(),
        });
    }
    if sources.iter().any(|source| !source.errors.is_empty()) {
        return Err(AppError::InvalidArgument {
            message: "Agent source inspection is incomplete; retry before saving the Team baseline"
                .into(),
        });
    }
    for requirement in &requirements {
        if !render::supports_project(&requirement.tool)
            || sources
                .iter()
                .flat_map(|source| &source.agents)
                .filter(|package| package.installable && package.reference == requirement.reference)
                .count()
                != 1
            || !installed.iter().any(|row| {
                row.tracked
                    && row.project_path.as_deref() == Some(project_path)
                    && row.source_id == requirement.reference.source_id
                    && row.relative_path == requirement.reference.relative_path
                    && row.tool == requirement.tool
                    && !matches!(
                        row.state,
                        InstallState::Missing
                            | InstallState::Foreign
                            | InstallState::SourceUnavailable
                    )
            })
        {
            return Err(AppError::InvalidArgument {
                message: "Team baseline contains an unavailable or ambiguous Agent target".into(),
            });
        }
    }
    Ok(requirements)
}

#[tauri::command]
pub async fn project_baseline_save_team(
    state: State<'_, AppState>,
    project_path: String,
    label: String,
    requirements: Vec<BaselineAgentRequirement>,
    subscribe: bool,
) -> Result<ProjectReadinessBaseline, AppError> {
    let reviewed_project_path = exact_registered_project(&state, &project_path).await?;
    let sources = crate::agents::inspect_agent_sources(&state.app_data_dir).await?;
    let installed = mcp_reconcile_agent_installs(&state).await?;
    let agent_requirements =
        validate_team_requirements(requirements, &sources, &installed, &reviewed_project_path)?;
    let mut agents = agent_requirements
        .iter()
        .map(|requirement| requirement.reference.clone())
        .collect::<Vec<_>>();
    let mut tools = agent_requirements
        .iter()
        .map(|requirement| requirement.tool.clone())
        .collect::<Vec<_>>();
    agents.sort();
    agents.dedup();
    tools.sort();
    tools.dedup();
    persist_registered_project_baseline(
        &state,
        &project_path,
        ProjectReadinessBaseline {
            project_path: String::new(),
            label,
            agent_requirements,
            skill_requirements: Vec::new(),
            agents,
            skills: Vec::new(),
            instructions: Vec::new(),
            mcp_servers: Vec::new(),
            tools,
        },
        subscribe,
    )
    .await
}

#[tauri::command]
pub async fn project_baseline_import_pack(
    state: State<'_, AppState>,
    project_path: String,
    pack: WorkspacePack,
) -> Result<ProjectReadinessBaseline, AppError> {
    let pack = normalize_workspace_pack(pack)?;
    let baseline = baseline_from_workspace_pack(String::new(), pack);
    persist_registered_project_baseline(&state, &project_path, baseline, false).await
}

fn status_from_agent_install(state: InstallState) -> ReadinessRowState {
    match state {
        InstallState::Current => ReadinessRowState::Ready,
        InstallState::SourceUnavailable => ReadinessRowState::Unavailable,
        _ => ReadinessRowState::NeedsAttention,
    }
}

fn status_from_skill_install(state: crate::types::SkillInstallState) -> ReadinessRowState {
    match state {
        crate::types::SkillInstallState::Current => ReadinessRowState::Ready,
        crate::types::SkillInstallState::SourceUnavailable => ReadinessRowState::Unavailable,
        _ => ReadinessRowState::NeedsAttention,
    }
}

#[tauri::command]
pub async fn project_readiness_get(
    state: State<'_, AppState>,
    project_path: String,
) -> Result<ProjectReadinessReport, AppError> {
    let project_path = exact_registered_project(&state, &project_path).await?;
    let database = control_center_database(&state).await?;
    let document = corpus::load_control_center(&database).await?;
    let baseline = document
        .project_baselines
        .iter()
        .find(|baseline| baseline.project_path == project_path)
        .cloned();
    let Some(baseline) = baseline else {
        return Ok(build_readiness_report(
            &project_path,
            None,
            ReadinessEvidence {
                agents: Ok(BTreeMap::new()),
                skills: Ok(BTreeMap::new()),
                instructions: Ok(BTreeMap::new()),
                mcp_servers: Ok(BTreeMap::new()),
                tools: Ok(BTreeMap::new()),
            },
        ));
    };

    let agent_evidence = match mcp_reconcile_agent_installs(&state).await {
        Ok(installed) => {
            let sources = crate::agents::inspect_agent_sources(&state.app_data_dir).await;
            sources.map(|sources| {
                let available = sources
                    .iter()
                    .flat_map(|source| &source.agents)
                    .filter(|package| package.installable)
                    .map(|package| package.reference.clone())
                    .collect::<BTreeSet<_>>();
                exact_agent_requirements(&baseline)
                    .into_iter()
                    .map(|requirement| {
                        let reference = &requirement.reference;
                        let state = installed
                            .iter()
                            .filter(|row| {
                                row.project_path.as_deref() == Some(project_path.as_str())
                                    && row.source_id == reference.source_id
                                    && row.relative_path == reference.relative_path
                                    && row.tool == requirement.tool
                            })
                            .map(|row| status_from_agent_install(row.state))
                            .min_by_key(|state| match state {
                                ReadinessRowState::Ready => 0,
                                ReadinessRowState::NeedsAttention => 1,
                                ReadinessRowState::Unavailable => 2,
                                ReadinessRowState::Unverifiable => 3,
                            })
                            .unwrap_or(if available.contains(reference) {
                                ReadinessRowState::NeedsAttention
                            } else {
                                ReadinessRowState::Unavailable
                            });
                        (requirement, state)
                    })
                    .collect()
            })
        }
        Err(error) => Err(error),
    }
    .map_err(|error| error.to_string());

    let registered = registered_projects(&state.app_data_dir)
        .await?
        .into_iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let skill_evidence = match crate::skills::reconcile_skill_installs(&state, &registered).await {
        Ok(installed) => crate::skills::inspect_skill_sources(&state)
            .await
            .map(|sources| {
                let available = sources
                    .iter()
                    .flat_map(|source| &source.packages)
                    .filter(|package| package.installable)
                    .map(|package| SkillReference {
                        source_id: package.source_id.clone(),
                        relative_path: package.relative_path.clone(),
                    })
                    .collect::<BTreeSet<_>>();
                exact_skill_requirements(&baseline)
                    .into_iter()
                    .map(|requirement| {
                        let reference = &requirement.reference;
                        let status = installed
                            .iter()
                            .filter(|row| {
                                row.project_path.as_deref() == Some(project_path.as_str())
                                    && row.source_id == reference.source_id
                                    && row.relative_path == reference.relative_path
                                    && row.runtime == requirement.runtime
                            })
                            .map(|row| status_from_skill_install(row.state))
                            .min_by_key(|state| match state {
                                ReadinessRowState::Ready => 0,
                                ReadinessRowState::NeedsAttention => 1,
                                ReadinessRowState::Unavailable => 2,
                                ReadinessRowState::Unverifiable => 3,
                            })
                            .unwrap_or(if available.contains(reference) {
                                ReadinessRowState::NeedsAttention
                            } else {
                                ReadinessRowState::Unavailable
                            });
                        (requirement, status)
                    })
                    .collect()
            }),
        Err(error) => Err(error),
    }
    .map_err(|error| error.to_string());

    let instruction_evidence = inspect_project_instruction_targets(&state, &project_path)
        .await
        .map(|targets| {
            targets
                .into_iter()
                .map(|target| {
                    let state = if target.blockers.is_empty() && target.exists {
                        ReadinessRowState::Ready
                    } else if target.blockers.is_empty() {
                        ReadinessRowState::NeedsAttention
                    } else {
                        ReadinessRowState::Unavailable
                    };
                    (target.id, state)
                })
                .collect()
        })
        .map_err(|error| error.to_string());
    let mcp_evidence = crate::commands::mcp_clients::mcp_inventory_for_state(&state)
        .await
        .and_then(|inventory| {
            if !inventory.issues.is_empty() {
                return Err(AppError::Io {
                    message: "MCP inventory inspection is incomplete".into(),
                });
            }
            Ok(inventory
                .servers
                .into_iter()
                .filter(|server| {
                    server.project_path.as_deref() == Some(project_path.as_str())
                        || server.project_path.is_none()
                })
                .map(|server| {
                    let state = if server.enabled
                        && server.validation == crate::types::McpInventoryValidation::Valid
                    {
                        ReadinessRowState::Ready
                    } else {
                        ReadinessRowState::NeedsAttention
                    };
                    (server.name, state)
                })
                .collect())
        })
        .map_err(|error| error.to_string());
    let mut tool_evidence = BTreeMap::new();
    for tool in &baseline.tools {
        match tool_detected(&state, tool).await {
            Ok(true) => {
                tool_evidence.insert(tool.clone(), ReadinessRowState::Ready);
            }
            Ok(false) => {
                tool_evidence.insert(tool.clone(), ReadinessRowState::NeedsAttention);
            }
            Err(_) => {
                tool_evidence.insert(tool.clone(), ReadinessRowState::Unavailable);
            }
        }
    }
    let mut report = build_readiness_report(
        &project_path,
        Some(&baseline),
        ReadinessEvidence {
            agents: agent_evidence,
            skills: skill_evidence,
            instructions: instruction_evidence,
            mcp_servers: mcp_evidence,
            tools: Ok(tool_evidence),
        },
    );
    report.subscribed = document
        .project_subscriptions
        .iter()
        .any(|subscription| subscription.project_path == project_path);
    Ok(report)
}

fn catalog_item_reference(item: &crate::types::CatalogSnapshotItem) -> AgentReference {
    AgentReference {
        source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
        relative_path: item.relative_path.clone(),
    }
}

fn catalog_change_for_reference(
    current: &AgentReference,
    change: &CatalogChange,
) -> Option<(AgentReference, String)> {
    match change {
        CatalogChange::Added { item } if catalog_item_reference(item) == *current => Some((
            current.clone(),
            "Required Agent was added in a successful catalog refresh".into(),
        )),
        CatalogChange::Updated { before, after } if catalog_item_reference(before) == *current => {
            Some((
                catalog_item_reference(after),
                "Required Agent was updated in a successful catalog refresh".into(),
            ))
        }
        CatalogChange::Removed { item } if catalog_item_reference(item) == *current => Some((
            current.clone(),
            "Required Agent was removed in a successful catalog refresh".into(),
        )),
        CatalogChange::Renamed { before, after } if catalog_item_reference(before) == *current => {
            let next = catalog_item_reference(after);
            Some((
                next.clone(),
                format!(
                    "Required Agent was renamed: {} → {}",
                    current.relative_path, next.relative_path
                ),
            ))
        }
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecommendationInstallState {
    Current,
    Outdated,
    Absent,
    Missing,
    Unsafe,
}

fn recommendation_install_state(
    installed: Option<&[InstalledAgent]>,
    project_path: &str,
    reference: &AgentReference,
    tool: &str,
) -> RecommendationInstallState {
    let Some(installed) = installed else {
        return RecommendationInstallState::Unsafe;
    };
    let matches = installed
        .iter()
        .filter(|row| {
            row.project_path.as_deref() == Some(project_path)
                && row.source_id == reference.source_id
                && row.relative_path == reference.relative_path
                && row.tool == tool
        })
        .collect::<Vec<_>>();
    let row = match matches.as_slice() {
        [] => return RecommendationInstallState::Absent,
        [row] => row,
        _ => return RecommendationInstallState::Unsafe,
    };
    if !row.tracked {
        return RecommendationInstallState::Unsafe;
    }
    match row.state {
        InstallState::Current => RecommendationInstallState::Current,
        InstallState::Outdated => RecommendationInstallState::Outdated,
        InstallState::Missing => RecommendationInstallState::Missing,
        InstallState::Modified
        | InstallState::Foreign
        | InstallState::Disabled
        | InstallState::SourceUnavailable => RecommendationInstallState::Unsafe,
    }
}

fn derive_project_recommendations_with_installs(
    baseline: &ProjectReadinessBaseline,
    subscription: &ProjectSubscription,
    batches: &[CatalogFeedBatch],
    available: &BTreeMap<AgentReference, usize>,
    installed: Option<&[InstalledAgent]>,
) -> Vec<ProjectRecommendation> {
    let mut recommendations = Vec::new();
    let required = baseline
        .agents
        .iter()
        .cloned()
        .chain(
            baseline
                .agent_requirements
                .iter()
                .map(|requirement| requirement.reference.clone()),
        )
        .collect::<BTreeSet<_>>();
    for baseline_reference in required {
        let mut current_reference = baseline_reference.clone();
        let mut relevant = Vec::new();
        for batch in batches {
            for change in &batch.changes {
                let Some((action_reference, summary)) =
                    catalog_change_for_reference(&current_reference, change)
                else {
                    continue;
                };
                relevant.push((batch, change, action_reference.clone(), summary));
                if matches!(change, CatalogChange::Renamed { .. }) {
                    current_reference = action_reference;
                }
            }
        }
        let latest_relevant = relevant.len().checked_sub(1);
        for (index, (batch, change, action_reference, summary)) in relevant.into_iter().enumerate()
        {
            let change_kind = match change {
                CatalogChange::Added { .. } => RecommendationChangeKind::Added,
                CatalogChange::Updated { .. } => RecommendationChangeKind::Updated,
                CatalogChange::Removed { .. } => RecommendationChangeKind::Removed,
                CatalogChange::Renamed { .. } => RecommendationChangeKind::Renamed,
            };
            let requirements = exact_agent_requirements(baseline)
                .into_iter()
                .filter(|requirement| requirement.reference == baseline_reference)
                .filter(|requirement| requirement.tool != LEGACY_UNREVIEWED_TARGET)
                .collect::<Vec<_>>();
            let mut unsafe_action_target = false;
            let mut all_targets_current = !requirements.is_empty();
            let targets = requirements
                .iter()
                .filter_map(|requirement| {
                    let target_operation = match change_kind {
                        RecommendationChangeKind::Updated => match recommendation_install_state(
                            installed,
                            &baseline.project_path,
                            &action_reference,
                            &requirement.tool,
                        ) {
                            RecommendationInstallState::Current => return None,
                            RecommendationInstallState::Outdated => {
                                all_targets_current = false;
                                RecommendationOperation::Update
                            }
                            RecommendationInstallState::Absent
                            | RecommendationInstallState::Missing
                            | RecommendationInstallState::Unsafe => {
                                all_targets_current = false;
                                unsafe_action_target = true;
                                return None;
                            }
                        },
                        RecommendationChangeKind::Added | RecommendationChangeKind::Renamed => {
                            match recommendation_install_state(
                                installed,
                                &baseline.project_path,
                                &action_reference,
                                &requirement.tool,
                            ) {
                                RecommendationInstallState::Current => return None,
                                RecommendationInstallState::Absent
                                | RecommendationInstallState::Missing => {
                                    all_targets_current = false;
                                    RecommendationOperation::Install
                                }
                                RecommendationInstallState::Outdated => {
                                    all_targets_current = false;
                                    RecommendationOperation::Update
                                }
                                RecommendationInstallState::Unsafe => {
                                    all_targets_current = false;
                                    unsafe_action_target = true;
                                    return None;
                                }
                            }
                        }
                        RecommendationChangeKind::Removed => {
                            all_targets_current = false;
                            RecommendationOperation::Informational
                        }
                    };
                    Some(ProjectRecommendationTarget {
                        reference: action_reference.clone(),
                        tool: requirement.tool.clone(),
                        project_path: baseline.project_path.clone(),
                        operation: target_operation,
                    })
                })
                .collect::<Vec<_>>();
            if matches!(
                change_kind,
                RecommendationChangeKind::Updated | RecommendationChangeKind::Added
            ) && all_targets_current
            {
                continue;
            }
            let finalize_only =
                change_kind == RecommendationChangeKind::Renamed && all_targets_current;
            let id = render::sha256_hex(
                &serde_json::to_vec(&(
                    baseline.project_path.as_str(),
                    &baseline_reference,
                    batch.at.as_str(),
                    change,
                ))
                .expect("catalog recommendation identity is serializable"),
            );
            if subscription.dismissed_recommendation_ids.contains(&id) {
                continue;
            }
            let lifecycle = if Some(index) != latest_relevant {
                RecommendationLifecycle::Superseded
            } else if matches!(change_kind, RecommendationChangeKind::Removed)
                || (targets.is_empty() && !finalize_only)
                || unsafe_action_target
                || available.get(&action_reference) != Some(&1)
            {
                RecommendationLifecycle::Blocked
            } else if subscription.pending_recommendation_ids.contains(&id) {
                RecommendationLifecycle::Pending
            } else if subscription
                .last_seen_batch
                .as_deref()
                .is_none_or(|cursor| {
                    chrono::DateTime::parse_from_rfc3339(&batch.at).ok()
                        > chrono::DateTime::parse_from_rfc3339(cursor).ok()
                })
            {
                RecommendationLifecycle::New
            } else {
                RecommendationLifecycle::Pending
            };
            recommendations.push(ProjectRecommendation {
                id,
                project_path: baseline.project_path.clone(),
                batch_at: batch.at.clone(),
                lifecycle,
                summary,
                change_kind,
                baseline_reference: baseline_reference.clone(),
                agent_references: vec![action_reference],
                targets,
                finalize_only,
            });
        }
    }
    recommendations.sort_by(|left, right| {
        chrono::DateTime::parse_from_rfc3339(&left.batch_at)
            .ok()
            .cmp(&chrono::DateTime::parse_from_rfc3339(&right.batch_at).ok())
            .then_with(|| left.baseline_reference.cmp(&right.baseline_reference))
            .then_with(|| left.id.cmp(&right.id))
    });
    recommendations
}

fn current_project_recommendation_candidate_ids(
    baseline: &ProjectReadinessBaseline,
    subscription: &ProjectSubscription,
    batches: &[CatalogFeedBatch],
    available: &BTreeMap<AgentReference, usize>,
    installed: Option<&[InstalledAgent]>,
) -> BTreeSet<String> {
    let mut candidate_projection = subscription.clone();
    candidate_projection.dismissed_recommendation_ids.clear();
    derive_project_recommendations_with_installs(
        baseline,
        &candidate_projection,
        batches,
        available,
        installed,
    )
    .into_iter()
    .filter(|recommendation| recommendation.lifecycle != RecommendationLifecycle::Superseded)
    .map(|recommendation| recommendation.id)
    .collect()
}

#[cfg(test)]
fn derive_project_recommendations(
    baseline: &ProjectReadinessBaseline,
    subscription: &ProjectSubscription,
    batches: &[CatalogFeedBatch],
    available: &BTreeMap<AgentReference, usize>,
) -> Vec<ProjectRecommendation> {
    derive_project_recommendations_with_installs(baseline, subscription, batches, available, None)
}

fn set_project_subscription(
    document: &mut crate::types::ControlCenterDocument,
    project_path: String,
    enabled: bool,
) -> Result<(), AppError> {
    if enabled
        && !document
            .project_baselines
            .iter()
            .any(|baseline| baseline.project_path == project_path)
    {
        return Err(AppError::InvalidArgument {
            message: "Save a project baseline before subscribing".into(),
        });
    }
    document
        .project_subscriptions
        .retain(|subscription| subscription.project_path != project_path);
    if enabled {
        document.project_subscriptions.push(ProjectSubscription {
            project_path,
            last_seen_batch: None,
            pending_recommendation_ids: Vec::new(),
            dismissed_recommendation_ids: Vec::new(),
        });
        document
            .project_subscriptions
            .sort_by(|left, right| left.project_path.cmp(&right.project_path));
    }
    Ok(())
}

#[tauri::command]
pub async fn project_subscription_set(
    state: State<'_, AppState>,
    project_path: String,
    enabled: bool,
) -> Result<bool, AppError> {
    let _project_lock = lock_project_registry(&state.app_data_dir)?;
    let project_path = exact_registered_project(&state, &project_path).await?;
    control_center_database(&state)
        .await?
        .mutate(
            corpus::control_center_spec(),
            crate::types::ControlCenterDocument::default(),
            move |document| set_project_subscription(document, project_path, enabled),
        )
        .await?;
    Ok(enabled)
}

async fn current_recommendations(
    state: &AppState,
    project_path: &str,
    include_seen_latest: bool,
) -> Result<
    (
        ProjectSubscription,
        Vec<ProjectRecommendation>,
        BTreeSet<String>,
    ),
    AppError,
> {
    let database = control_center_database(state).await?;
    let document = corpus::load_control_center(&database).await?;
    let baseline = document
        .project_baselines
        .iter()
        .find(|baseline| baseline.project_path == project_path)
        .ok_or_else(|| AppError::InvalidArgument {
            message: "Project has no readiness baseline".into(),
        })?;
    let subscription = document
        .project_subscriptions
        .iter()
        .find(|subscription| subscription.project_path == project_path)
        .cloned()
        .ok_or_else(|| AppError::InvalidArgument {
            message: "Project is not subscribed".into(),
        })?;
    let available = available_agent_reference_counts(state).await;
    let mut projection = subscription.clone();
    if include_seen_latest {
        projection.last_seen_batch = None;
    }
    let installed = mcp_reconcile_agent_installs(state).await.ok();
    let recommendations = derive_project_recommendations_with_installs(
        baseline,
        &projection,
        &document.catalog_feed,
        &available,
        installed.as_deref(),
    );
    let candidate_ids = current_project_recommendation_candidate_ids(
        baseline,
        &projection,
        &document.catalog_feed,
        &available,
        installed.as_deref(),
    );
    Ok((subscription, recommendations, candidate_ids))
}

async fn available_agent_reference_counts(state: &AppState) -> BTreeMap<AgentReference, usize> {
    crate::agents::inspect_agent_sources(&state.app_data_dir)
        .await
        .map(|sources| {
            let mut counts = BTreeMap::new();
            for reference in sources
                .into_iter()
                .flat_map(|source| source.agents)
                .filter(|package| package.installable)
                .map(|package| package.reference)
            {
                *counts.entry(reference).or_insert(0) += 1;
            }
            counts
        })
        .unwrap_or_default()
}

#[tauri::command]
pub async fn project_recommendations_list(
    state: State<'_, AppState>,
    project_path: String,
) -> Result<Vec<ProjectRecommendation>, AppError> {
    let _project_lock = lock_project_registry(&state.app_data_dir)?;
    let project_path = exact_registered_project(&state, &project_path).await?;
    list_project_recommendations_for_state(&state, &project_path).await
}

async fn list_project_recommendations_for_state(
    state: &AppState,
    project_path: &str,
) -> Result<Vec<ProjectRecommendation>, AppError> {
    let (_, recommendations, candidate_ids) =
        current_recommendations(state, project_path, false).await?;
    let inferred_pending_ids = recommendations
        .iter()
        .filter(|recommendation| recommendation.lifecycle == RecommendationLifecycle::Pending)
        .map(|recommendation| recommendation.id.clone())
        .collect::<BTreeSet<_>>();
    let retained_project = project_path.to_string();
    control_center_database(state)
        .await?
        .mutate(
            corpus::control_center_spec(),
            crate::types::ControlCenterDocument::default(),
            move |document| {
                prune_project_pending_recommendations(
                    document,
                    &retained_project,
                    &candidate_ids,
                    &inferred_pending_ids,
                )
            },
        )
        .await?;
    Ok(recommendations)
}

fn prune_project_pending_recommendations(
    document: &mut crate::types::ControlCenterDocument,
    project_path: &str,
    candidate_ids: &BTreeSet<String>,
    inferred_pending_ids: &BTreeSet<String>,
) -> Result<(), AppError> {
    let subscription = document
        .project_subscriptions
        .iter_mut()
        .find(|subscription| subscription.project_path == project_path)
        .ok_or_else(|| AppError::InvalidArgument {
            message: "Project is not subscribed".into(),
        })?;
    subscription
        .pending_recommendation_ids
        .retain(|id| candidate_ids.contains(id));
    subscription
        .dismissed_recommendation_ids
        .retain(|id| candidate_ids.contains(id));
    for id in inferred_pending_ids {
        if !subscription.pending_recommendation_ids.contains(id) {
            subscription.pending_recommendation_ids.push(id.clone());
        }
    }
    subscription.pending_recommendation_ids.sort();
    if subscription.pending_recommendation_ids.len()
        > corpus::CONTROL_CENTER_MAX_PENDING_RECOMMENDATIONS
    {
        return Err(AppError::InvalidArgument {
            message: "Pending recommendations exceed the subscription limit".into(),
        });
    }
    Ok(())
}

fn advance_project_recommendation_cursor(
    document: &mut crate::types::ControlCenterDocument,
    project_path: &str,
    expected_cursor: Option<&str>,
    surfaced_cursor: &str,
    mut pending_recommendation_ids: Vec<String>,
) -> Result<(), AppError> {
    let surfaced_at = chrono::DateTime::parse_from_rfc3339(surfaced_cursor).map_err(|_| {
        AppError::InvalidArgument {
            message: "Recommendation cursor is invalid".into(),
        }
    })?;
    if document
        .catalog_last_success_at
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .is_none_or(|last_success| surfaced_at > last_success)
    {
        return Err(AppError::InvalidArgument {
            message: "Recommendation cursor is newer than the durable catalog feed".into(),
        });
    }
    let subscription = document
        .project_subscriptions
        .iter_mut()
        .find(|subscription| subscription.project_path == project_path)
        .ok_or_else(|| AppError::InvalidArgument {
            message: "Project is not subscribed".into(),
        })?;
    if subscription.last_seen_batch.as_deref() != expected_cursor {
        return Err(AppError::InvalidArgument {
            message: "Recommendation acknowledgement is stale".into(),
        });
    }
    if subscription
        .last_seen_batch
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .is_some_and(|cursor| surfaced_at <= cursor)
    {
        return Err(AppError::InvalidArgument {
            message: "Recommendation cursor must advance".into(),
        });
    }
    let pending_count = pending_recommendation_ids.len();
    pending_recommendation_ids.sort();
    pending_recommendation_ids.dedup();
    if pending_recommendation_ids.len() != pending_count
        || pending_recommendation_ids.len() > corpus::CONTROL_CENTER_MAX_PENDING_RECOMMENDATIONS
        || pending_recommendation_ids
            .iter()
            .any(|id| id.len() != 64 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err(AppError::InvalidArgument {
            message: "Pending recommendation receipt is invalid".into(),
        });
    }
    subscription.last_seen_batch = Some(surfaced_cursor.into());
    subscription.pending_recommendation_ids = pending_recommendation_ids;
    Ok(())
}

#[tauri::command]
pub async fn project_recommendations_acknowledge(
    state: State<'_, AppState>,
    project_path: String,
    batch_at: String,
    recommendation_ids: Vec<String>,
) -> Result<bool, AppError> {
    let _project_lock = lock_project_registry(&state.app_data_dir)?;
    let project_path = exact_registered_project(&state, &project_path).await?;
    if recommendation_ids.is_empty()
        || recommendation_ids.len() > corpus::CONTROL_CENTER_MAX_PENDING_RECOMMENDATIONS
        || recommendation_ids
            .iter()
            .any(|id| id.len() != 64 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err(AppError::InvalidArgument {
            message: "Surfaced recommendation receipt is invalid".into(),
        });
    }
    let (subscription, recommendations, candidate_ids) =
        current_recommendations(&state, &project_path, false).await?;
    let mut expected_ids = recommendations
        .iter()
        .filter(|recommendation| recommendation.lifecycle == RecommendationLifecycle::New)
        .map(|recommendation| recommendation.id.clone())
        .collect::<Vec<_>>();
    let surfaced_cursor = recommendations
        .iter()
        .filter(|recommendation| recommendation.lifecycle == RecommendationLifecycle::New)
        .max_by_key(|recommendation| {
            chrono::DateTime::parse_from_rfc3339(&recommendation.batch_at).ok()
        })
        .map(|recommendation| recommendation.batch_at.as_str());
    let mut supplied_ids = recommendation_ids;
    expected_ids.sort();
    supplied_ids.sort();
    let supplied_count = supplied_ids.len();
    supplied_ids.dedup();
    if supplied_ids.len() != supplied_count
        || supplied_ids != expected_ids
        || surfaced_cursor != Some(batch_at.as_str())
    {
        return Err(AppError::InvalidArgument {
            message: "Recommendation receipt does not match the current surfaced set".into(),
        });
    }
    let expected_cursor = subscription.last_seen_batch;
    let mut pending_ids = subscription
        .pending_recommendation_ids
        .into_iter()
        .filter(|id| candidate_ids.contains(id))
        .collect::<Vec<_>>();
    pending_ids.extend(supplied_ids);
    pending_ids.sort();
    pending_ids.dedup();
    if pending_ids.len() > corpus::CONTROL_CENTER_MAX_PENDING_RECOMMENDATIONS {
        return Err(AppError::InvalidArgument {
            message: "Pending recommendations exceed the subscription limit".into(),
        });
    }
    control_center_database(&state)
        .await?
        .mutate(
            corpus::control_center_spec(),
            crate::types::ControlCenterDocument::default(),
            move |document| {
                advance_project_recommendation_cursor(
                    document,
                    &project_path,
                    expected_cursor.as_deref(),
                    &batch_at,
                    pending_ids,
                )
            },
        )
        .await?;
    Ok(true)
}

#[tauri::command]
pub async fn project_recommendation_dismiss(
    state: State<'_, AppState>,
    project_path: String,
    recommendation_id: String,
) -> Result<(), AppError> {
    let _project_lock = lock_project_registry(&state.app_data_dir)?;
    let project_path = exact_registered_project(&state, &project_path).await?;
    let (_, recommendations, candidate_ids) =
        current_recommendations(&state, &project_path, true).await?;
    if !recommendations
        .iter()
        .any(|recommendation| recommendation.id == recommendation_id)
    {
        return Err(AppError::InvalidArgument {
            message: "Recommendation is not current for this project subscription".into(),
        });
    }
    control_center_database(&state)
        .await?
        .mutate(
            corpus::control_center_spec(),
            crate::types::ControlCenterDocument::default(),
            move |document| {
                dismiss_project_recommendation(
                    document,
                    &project_path,
                    recommendation_id,
                    &candidate_ids,
                )
            },
        )
        .await
}

fn dismiss_project_recommendation(
    document: &mut crate::types::ControlCenterDocument,
    project_path: &str,
    recommendation_id: String,
    candidate_ids: &BTreeSet<String>,
) -> Result<(), AppError> {
    let subscription = document
        .project_subscriptions
        .iter_mut()
        .find(|subscription| subscription.project_path == project_path)
        .ok_or_else(|| AppError::InvalidArgument {
            message: "Project is not subscribed".into(),
        })?;
    subscription
        .pending_recommendation_ids
        .retain(|id| id != &recommendation_id && candidate_ids.contains(id));
    subscription
        .dismissed_recommendation_ids
        .retain(|id| candidate_ids.contains(id));
    if !subscription
        .dismissed_recommendation_ids
        .contains(&recommendation_id)
    {
        subscription
            .dismissed_recommendation_ids
            .push(recommendation_id);
        subscription.dismissed_recommendation_ids.sort();
    }
    Ok(())
}

#[tauri::command]
pub async fn project_recommendation_open(
    state: State<'_, AppState>,
    project_path: String,
    recommendation_id: String,
) -> Result<ProjectRecommendation, AppError> {
    let _project_lock = lock_project_registry(&state.app_data_dir)?;
    let project_path = exact_registered_project(&state, &project_path).await?;
    let (_, recommendations, _) = current_recommendations(&state, &project_path, true).await?;
    let recommendation = recommendations
        .into_iter()
        .find(|recommendation| recommendation.id == recommendation_id)
        .ok_or_else(|| AppError::InvalidArgument {
            message: "Recommendation is no longer available".into(),
        })?;
    if !matches!(
        recommendation.lifecycle,
        RecommendationLifecycle::New | RecommendationLifecycle::Pending
    ) {
        return Err(AppError::InvalidArgument {
            message: if recommendation.lifecycle == RecommendationLifecycle::Blocked {
                "Recommendation exact references no longer resolve"
            } else {
                "Only an actionable recommendation can enter deployment review"
            }
            .into(),
        });
    }
    if recommendation.finalize_only {
        return Err(AppError::InvalidArgument {
            message: "Rename recommendation is ready to finish without another install".into(),
        });
    }
    if recommendation.targets.iter().any(|target| {
        target.project_path != project_path
            || target.operation == RecommendationOperation::Informational
    }) {
        return Err(AppError::InvalidArgument {
            message: "Recommendation has no safe deployment operation".into(),
        });
    }
    let installed = mcp_reconcile_agent_installs(&state).await?;
    validate_project_recommendation_target_states(&recommendation, &installed)?;
    for target in recommendation
        .targets
        .iter()
        .filter(|target| target.operation == RecommendationOperation::Install)
    {
        let plan = build_mutation_plan(
            &state,
            vec![target.reference.clone()],
            target.tool.clone(),
            Some(target.project_path.clone()),
            "install",
            true,
        )
        .await?;
        if !plan.blockers.is_empty() {
            return Err(AppError::InvalidArgument {
                message: format!(
                    "Install recommendation no longer has a safe destination: {}",
                    plan.blockers.join("; ")
                ),
            });
        }
    }
    Ok(recommendation)
}

fn validate_project_recommendation_target_states(
    recommendation: &ProjectRecommendation,
    installed: &[InstalledAgent],
) -> Result<(), AppError> {
    for target in &recommendation.targets {
        let state = recommendation_install_state(
            Some(installed),
            &target.project_path,
            &target.reference,
            &target.tool,
        );
        let valid = match target.operation {
            RecommendationOperation::Update => state == RecommendationInstallState::Outdated,
            RecommendationOperation::Install => matches!(
                state,
                RecommendationInstallState::Absent | RecommendationInstallState::Missing
            ),
            RecommendationOperation::Informational => false,
        };
        if !valid {
            return Err(AppError::InvalidArgument {
                message: format!(
                    "Recommendation target is no longer safe for its {:?} operation",
                    target.operation
                ),
            });
        }
    }
    Ok(())
}

fn finalize_rename_recommendation_in_document(
    document: &mut crate::types::ControlCenterDocument,
    project_path: &str,
    recommendation_id: &str,
    available: &BTreeMap<AgentReference, usize>,
    installed: &[InstalledAgent],
) -> Result<ProjectReadinessBaseline, AppError> {
    let mut next = document.clone();
    let baseline_index = next
        .project_baselines
        .iter()
        .position(|baseline| baseline.project_path == project_path)
        .ok_or_else(|| AppError::InvalidArgument {
            message: "Project has no readiness baseline".into(),
        })?;
    let baseline_snapshot = next.project_baselines[baseline_index].clone();
    let subscription = next
        .project_subscriptions
        .iter()
        .find(|subscription| subscription.project_path == project_path)
        .cloned()
        .ok_or_else(|| AppError::InvalidArgument {
            message: "Project is not subscribed".into(),
        })?;
    let recommendation = derive_project_recommendations_with_installs(
        &baseline_snapshot,
        &subscription,
        &next.catalog_feed,
        available,
        Some(installed),
    )
    .into_iter()
    .find(|recommendation| recommendation.id == recommendation_id)
    .ok_or_else(|| AppError::InvalidArgument {
        message: "Rename recommendation is stale".into(),
    })?;
    if !matches!(
        recommendation.lifecycle,
        RecommendationLifecycle::New | RecommendationLifecycle::Pending
    ) || recommendation.change_kind != RecommendationChangeKind::Renamed
        || recommendation.agent_references.len() != 1
        || !recommendation.finalize_only
        || !recommendation.targets.is_empty()
    {
        return Err(AppError::InvalidArgument {
            message: "Recommendation is not a current actionable rename".into(),
        });
    }
    let new_reference = recommendation.agent_references[0].clone();
    let mut effective_requirements = exact_agent_requirements(&baseline_snapshot);
    let matching_requirements = effective_requirements
        .iter()
        .filter(|requirement| requirement.reference == recommendation.baseline_reference)
        .collect::<Vec<_>>();
    let required_tools = matching_requirements
        .iter()
        .map(|requirement| requirement.tool.clone())
        .collect::<BTreeSet<_>>();
    if required_tools.is_empty()
        || matching_requirements.len() != required_tools.len()
        || required_tools.iter().any(|tool| {
            recommendation_install_state(Some(installed), project_path, &new_reference, tool)
                != RecommendationInstallState::Current
        })
    {
        return Err(AppError::InvalidArgument {
            message: "Every exact renamed Agent target must be current before finalizing".into(),
        });
    }

    let baseline = &mut next.project_baselines[baseline_index];
    for requirement in &mut effective_requirements {
        if requirement.reference == recommendation.baseline_reference
            && required_tools.contains(&requirement.tool)
        {
            requirement.reference = new_reference.clone();
        }
    }
    baseline.agent_requirements = effective_requirements;
    for reference in &mut baseline.agents {
        if *reference == recommendation.baseline_reference {
            *reference = new_reference.clone();
        }
    }
    let mut seen_agents = BTreeSet::new();
    baseline
        .agents
        .retain(|reference| seen_agents.insert(reference.clone()));
    let returned = baseline.clone();
    next.project_subscriptions
        .iter_mut()
        .find(|subscription| subscription.project_path == project_path)
        .expect("validated project subscription exists")
        .pending_recommendation_ids
        .retain(|id| id != recommendation_id);
    corpus::validate_control_center(&next)?;
    *document = next;
    Ok(returned)
}

#[tauri::command]
pub async fn project_recommendation_finalize(
    state: State<'_, AppState>,
    project_path: String,
    recommendation_id: String,
) -> Result<ProjectReadinessBaseline, AppError> {
    if recommendation_id.len() != 64
        || !recommendation_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(AppError::InvalidArgument {
            message: "Recommendation id is invalid".into(),
        });
    }
    let _project_lock = lock_project_registry(&state.app_data_dir)?;
    let project_path = exact_registered_project(&state, &project_path).await?;
    let _install_guard = state.skill_installs_write_lock.lock().await;
    let _install_file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
    let available = available_agent_reference_counts(&state).await;
    let installed = reconcile_agent_installs_locked(&state).await?;
    let finalized_project = project_path.clone();
    control_center_database(&state)
        .await?
        .mutate(
            corpus::control_center_spec(),
            crate::types::ControlCenterDocument::default(),
            move |document| {
                finalize_rename_recommendation_in_document(
                    document,
                    &finalized_project,
                    &recommendation_id,
                    &available,
                    &installed,
                )
            },
        )
        .await
}

// ---------- Project instruction snippets ----------

const MAX_PROJECT_INSTRUCTION_BYTES: u64 = MAX_INSTALLED_BYTES;
const MAX_PROJECT_INSTRUCTION_SNIPPETS: usize = 32;
const MAX_PROJECT_INSTRUCTION_ID_BYTES: usize = 64;
const MAX_PROJECT_INSTRUCTION_CONTENT_BYTES: usize = 64 * 1024;
const PROJECT_INSTRUCTION_MARKER_PREFIX: &str = "<!-- agency-agents:instruction:v1:";

#[derive(Clone, Copy)]
struct ProjectInstructionTargetDef {
    id: &'static str,
    label: &'static str,
    relative_path: &'static str,
}

const PROJECT_INSTRUCTION_TARGETS: [ProjectInstructionTargetDef; 4] = [
    ProjectInstructionTargetDef {
        id: "agents",
        label: "AGENTS.md",
        relative_path: "AGENTS.md",
    },
    ProjectInstructionTargetDef {
        id: "claude",
        label: "CLAUDE.md",
        relative_path: "CLAUDE.md",
    },
    ProjectInstructionTargetDef {
        id: "gemini",
        label: "GEMINI.md",
        relative_path: "GEMINI.md",
    },
    ProjectInstructionTargetDef {
        id: "copilot",
        label: "GitHub Copilot",
        relative_path: ".github/copilot-instructions.md",
    },
];

fn project_instruction_targets() -> &'static [ProjectInstructionTargetDef] {
    &PROJECT_INSTRUCTION_TARGETS
}

fn project_instruction_target(id: &str) -> Result<ProjectInstructionTargetDef, AppError> {
    project_instruction_targets()
        .iter()
        .copied()
        .find(|target| target.id == id)
        .ok_or_else(|| AppError::InvalidArgument {
            message: "project instruction target is unsupported".into(),
        })
}

#[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProjectInstructionOperation {
    Upsert,
    Remove,
}

#[derive(Debug, Clone)]
struct ParsedProjectInstructionSnippet {
    id: String,
    content: String,
    span_start: usize,
    marker_start: usize,
    span_end: usize,
}

#[derive(Debug, Clone)]
struct ProjectInstructionComposition {
    proposed: String,
    adoption: bool,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInstructionSnippet {
    id: String,
    content: String,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInstructionTarget {
    id: String,
    label: String,
    relative_path: String,
    destination: String,
    state: String,
    exists: bool,
    current: String,
    snippets: Vec<ProjectInstructionSnippet>,
    blockers: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInstructionPlan {
    project_path: String,
    target: String,
    label: String,
    relative_path: String,
    destination: String,
    operation: ProjectInstructionOperation,
    snippet_id: String,
    current: String,
    proposed: String,
    exists: bool,
    adoption: bool,
    backup_required: bool,
    no_op: bool,
    warnings: Vec<String>,
    blockers: Vec<String>,
    revision: String,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInstructionApplyResult {
    destination: String,
    outcome: String,
    backup_path: Option<String>,
    message: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInstructionApplyResponse {
    plan: ProjectInstructionPlan,
    result: Option<ProjectInstructionApplyResult>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProjectInstructionApplyOperation {
    project_path: String,
    target: String,
    destination: String,
    before_hash: Option<String>,
    after_hash: String,
    backup_path: Option<String>,
}

fn invalid_project_instruction(message: impl Into<String>) -> AppError {
    AppError::InvalidArgument {
        message: message.into(),
    }
}

fn validate_project_instruction_id(id: &str) -> Result<(), AppError> {
    if id.is_empty()
        || id.len() > MAX_PROJECT_INSTRUCTION_ID_BYTES
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || !id.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        || !id.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
    {
        return Err(invalid_project_instruction(
            "instruction snippet id must be a lowercase portable slug",
        ));
    }
    Ok(())
}

fn validate_project_instruction_content(content: &str) -> Result<(), AppError> {
    if content.is_empty()
        || content.len() > MAX_PROJECT_INSTRUCTION_CONTENT_BYTES
        || content
            .chars()
            .any(|character| character.is_control() && character != '\n' && character != '\t')
        || content.contains("agency-agents:instruction")
    {
        return Err(invalid_project_instruction(
            "instruction snippet content is invalid or exceeds its limit",
        ));
    }
    let lower = content.to_ascii_lowercase();
    if contains_workspace_pack_credential(content)
        || ["sk-", "ghp_", "github_pat_", "xoxb-", "akia"]
            .iter()
            .any(|marker| lower.contains(marker))
    {
        return Err(invalid_project_instruction(
            "instruction snippet must not contain credentials",
        ));
    }
    Ok(())
}

fn parse_project_instruction_marker(marker: &str) -> Result<(&str, &str), AppError> {
    let value = marker
        .strip_prefix(PROJECT_INSTRUCTION_MARKER_PREFIX)
        .and_then(|value| value.strip_suffix(" -->"))
        .ok_or_else(|| invalid_project_instruction("instruction ownership marker is malformed"))?;
    let (id, kind) = value
        .rsplit_once(':')
        .ok_or_else(|| invalid_project_instruction("instruction ownership marker is malformed"))?;
    validate_project_instruction_id(id)?;
    if !matches!(kind, "begin" | "end") {
        return Err(invalid_project_instruction(
            "instruction ownership marker is malformed",
        ));
    }
    Ok((id, kind))
}

fn parse_project_instruction_snippets(
    current: &str,
) -> Result<Vec<ParsedProjectInstructionSnippet>, AppError> {
    if current.matches("agency-agents:instruction").count()
        != current.matches(PROJECT_INSTRUCTION_MARKER_PREFIX).count()
    {
        return Err(invalid_project_instruction(
            "instruction ownership marker is malformed",
        ));
    }
    let mut snippets = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut open: Option<(String, usize, usize)> = None;
    let mut cursor = 0;
    while let Some(offset) = current[cursor..].find(PROJECT_INSTRUCTION_MARKER_PREFIX) {
        let marker_start = cursor + offset;
        let marker_end = current[marker_start..]
            .find("-->")
            .map(|end| marker_start + end + 3)
            .ok_or_else(|| {
                invalid_project_instruction("instruction ownership marker is malformed")
            })?;
        let (id, kind) = parse_project_instruction_marker(&current[marker_start..marker_end])?;
        match kind {
            "begin" => {
                if open.is_some() || !seen.insert(id.to_owned()) {
                    return Err(invalid_project_instruction(
                        "instruction ownership markers are nested or duplicated",
                    ));
                }
                if current.as_bytes().get(marker_end) != Some(&b'\n') {
                    return Err(invalid_project_instruction(
                        "instruction ownership marker is malformed",
                    ));
                }
                open = Some((id.to_owned(), marker_start, marker_end + 1));
            }
            "end" => {
                let (open_id, begin_start, content_start) = open.take().ok_or_else(|| {
                    invalid_project_instruction("instruction ownership marker is malformed")
                })?;
                if open_id != id
                    || marker_start == 0
                    || current.as_bytes().get(marker_start - 1) != Some(&b'\n')
                {
                    return Err(invalid_project_instruction(
                        "instruction ownership marker is malformed",
                    ));
                }
                let content_end = marker_start - 1;
                let content = &current[content_start..content_end];
                validate_project_instruction_content(content)?;
                let span_start = if begin_start >= 2
                    && &current.as_bytes()[begin_start - 2..begin_start] == b"\n\n"
                {
                    begin_start - 2
                } else {
                    begin_start
                };
                snippets.push(ParsedProjectInstructionSnippet {
                    id: open_id,
                    content: content.to_owned(),
                    span_start,
                    marker_start: begin_start,
                    span_end: marker_end,
                });
            }
            _ => unreachable!(),
        }
        cursor = marker_end;
    }
    if open.is_some() || current[cursor..].contains("agency-agents:instruction") {
        return Err(invalid_project_instruction(
            "instruction ownership marker is malformed",
        ));
    }
    if snippets.len() > MAX_PROJECT_INSTRUCTION_SNIPPETS {
        return Err(invalid_project_instruction(
            "instruction file exceeds its managed snippet limit",
        ));
    }
    Ok(snippets)
}

fn project_instruction_block(id: &str, content: &str) -> String {
    format!(
        "{PROJECT_INSTRUCTION_MARKER_PREFIX}{id}:begin -->\n{content}\n{PROJECT_INSTRUCTION_MARKER_PREFIX}{id}:end -->"
    )
}

fn compose_project_instruction(
    current: &str,
    operation: ProjectInstructionOperation,
    snippet_id: &str,
    content: &str,
) -> Result<ProjectInstructionComposition, AppError> {
    validate_project_instruction_id(snippet_id)?;
    let snippets = parse_project_instruction_snippets(current)?;
    let existing = snippets.iter().find(|snippet| snippet.id == snippet_id);
    let adoption = operation == ProjectInstructionOperation::Upsert
        && existing.is_none()
        && snippets.is_empty()
        && !current.is_empty();
    let proposed = match operation {
        ProjectInstructionOperation::Upsert => {
            validate_project_instruction_content(content)?;
            let block = project_instruction_block(snippet_id, content);
            if let Some(existing) = existing {
                let owned_separator = &current[existing.span_start..existing.marker_start];
                format!(
                    "{}{}{}{}",
                    &current[..existing.span_start],
                    owned_separator,
                    block,
                    &current[existing.span_end..]
                )
            } else {
                if snippets.len() >= MAX_PROJECT_INSTRUCTION_SNIPPETS {
                    return Err(invalid_project_instruction(
                        "instruction file reached its managed snippet limit",
                    ));
                }
                format!(
                    "{current}{}{block}",
                    if current.is_empty() { "" } else { "\n\n" }
                )
            }
        }
        ProjectInstructionOperation::Remove => existing.map_or_else(
            || current.to_owned(),
            |existing| {
                format!(
                    "{}{}",
                    &current[..existing.span_start],
                    &current[existing.span_end..]
                )
            },
        ),
    };
    if proposed.len() as u64 > MAX_PROJECT_INSTRUCTION_BYTES {
        return Err(invalid_project_instruction(
            "proposed instruction file exceeds its byte limit",
        ));
    }
    parse_project_instruction_snippets(&proposed)?;
    Ok(ProjectInstructionComposition { proposed, adoption })
}

async fn read_project_instruction_target(
    project: &Path,
    target: ProjectInstructionTargetDef,
) -> Result<Option<String>, AppError> {
    let relative = Path::new(target.relative_path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(invalid_project_instruction(
            "instruction target path is invalid",
        ));
    }
    let mut candidate = project.to_path_buf();
    let components = relative.components().collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        candidate.push(component.as_os_str());
        let metadata = match std::fs::symlink_metadata(&candidate) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(AppError::Io {
                    message: format!("inspect instruction target: {error}"),
                })
            }
        };
        if metadata.file_type().is_symlink()
            || (index + 1 < components.len() && !metadata.is_dir())
            || (index + 1 == components.len() && !metadata.is_file())
        {
            return Err(invalid_project_instruction(
                "instruction target must be a regular file beneath real directories",
            ));
        }
    }
    let bytes = read_capped(&candidate, MAX_PROJECT_INSTRUCTION_BYTES).await?;
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| invalid_project_instruction("instruction target must be valid UTF-8"))
}

async fn canonical_registered_instruction_project(
    state: &AppState,
    project_path: &str,
) -> Result<PathBuf, AppError> {
    if project_path.is_empty()
        || project_path.len() > 4096
        || project_path.chars().any(char::is_control)
    {
        return Err(invalid_project_instruction("project path is invalid"));
    }
    let supplied = PathBuf::from(project_path);
    if !supplied.is_absolute()
        || supplied
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(invalid_project_instruction(
            "project path must be absolute and normalized",
        ));
    }
    let metadata = std::fs::symlink_metadata(&supplied).map_err(|error| AppError::Io {
        message: format!("inspect instruction project: {error}"),
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(invalid_project_instruction(
            "instruction project must be a real directory",
        ));
    }
    let canonical = std::fs::canonicalize(&supplied).map_err(|error| AppError::Io {
        message: format!("canonicalize instruction project: {error}"),
    })?;
    if supplied != canonical
        || !registered_projects(&state.app_data_dir)
            .await?
            .contains(&canonical)
    {
        return Err(invalid_project_instruction(
            "instruction project must be the exact registered canonical path",
        ));
    }
    Ok(canonical)
}

fn project_instruction_target_view(
    project: &Path,
    target: ProjectInstructionTargetDef,
    current: Result<Option<String>, AppError>,
) -> ProjectInstructionTarget {
    let destination = project.join(target.relative_path);
    match current {
        Err(error) => ProjectInstructionTarget {
            id: target.id.into(),
            label: target.label.into(),
            relative_path: target.relative_path.into(),
            destination: destination.to_string_lossy().into_owned(),
            state: "blocked".into(),
            exists: destination.exists(),
            current: String::new(),
            snippets: Vec::new(),
            blockers: vec![error.to_string()],
        },
        Ok(None) => ProjectInstructionTarget {
            id: target.id.into(),
            label: target.label.into(),
            relative_path: target.relative_path.into(),
            destination: destination.to_string_lossy().into_owned(),
            state: "absent".into(),
            exists: false,
            current: String::new(),
            snippets: Vec::new(),
            blockers: Vec::new(),
        },
        Ok(Some(current)) => match parse_project_instruction_snippets(&current) {
            Err(error) => ProjectInstructionTarget {
                id: target.id.into(),
                label: target.label.into(),
                relative_path: target.relative_path.into(),
                destination: destination.to_string_lossy().into_owned(),
                state: "blocked".into(),
                exists: true,
                current,
                snippets: Vec::new(),
                blockers: vec![error.to_string()],
            },
            Ok(snippets) => ProjectInstructionTarget {
                id: target.id.into(),
                label: target.label.into(),
                relative_path: target.relative_path.into(),
                destination: destination.to_string_lossy().into_owned(),
                state: if snippets.is_empty() {
                    "existingUnmanaged"
                } else {
                    "managed"
                }
                .into(),
                exists: true,
                current,
                snippets: snippets
                    .into_iter()
                    .map(|snippet| ProjectInstructionSnippet {
                        id: snippet.id,
                        content: snippet.content,
                    })
                    .collect(),
                blockers: Vec::new(),
            },
        },
    }
}

async fn inspect_project_instruction_targets(
    state: &AppState,
    project_path: &str,
) -> Result<Vec<ProjectInstructionTarget>, AppError> {
    let project = canonical_registered_instruction_project(state, project_path).await?;
    let mut inspected = Vec::with_capacity(PROJECT_INSTRUCTION_TARGETS.len());
    for target in project_instruction_targets() {
        inspected.push(project_instruction_target_view(
            &project,
            *target,
            read_project_instruction_target(&project, *target).await,
        ));
    }
    Ok(inspected)
}

fn finalize_project_instruction_plan(plan: &mut ProjectInstructionPlan) -> Result<(), AppError> {
    plan.warnings.sort();
    plan.warnings.dedup();
    plan.blockers.sort();
    plan.blockers.dedup();
    plan.revision.clear();
    plan.revision =
        render::sha256_hex(
            &serde_json::to_vec(plan).map_err(|error| AppError::Internal {
                message: format!("serialize project instruction plan: {error}"),
            })?,
        );
    Ok(())
}

async fn build_project_instruction_plan(
    state: &AppState,
    project_path: &str,
    target_id: &str,
    operation: ProjectInstructionOperation,
    snippet_id: &str,
    content: &str,
) -> Result<ProjectInstructionPlan, AppError> {
    let project = canonical_registered_instruction_project(state, project_path).await?;
    let target = project_instruction_target(target_id)?;
    let destination = project.join(target.relative_path);
    let current = read_project_instruction_target(&project, target).await;
    let (exists, current, mut blockers) = match current {
        Ok(Some(current)) => (true, current, Vec::new()),
        Ok(None) => (false, String::new(), Vec::new()),
        Err(error) => (destination.exists(), String::new(), vec![error.to_string()]),
    };
    let composition = if blockers.is_empty() {
        match compose_project_instruction(&current, operation, snippet_id, content) {
            Ok(composition) => Some(composition),
            Err(error) => {
                blockers.push(error.to_string());
                None
            }
        }
    } else {
        None
    };
    let proposed = composition.as_ref().map_or_else(
        || current.clone(),
        |composition| composition.proposed.clone(),
    );
    let adoption = composition
        .as_ref()
        .is_some_and(|composition| composition.adoption);
    let no_op = current == proposed;
    if no_op && blockers.is_empty() {
        blockers.push("instruction plan has no changes to apply".into());
    }
    let mut warnings = Vec::new();
    if adoption {
        warnings.push("Existing user-authored content will be adopted without being owned".into());
    }
    if exists && !no_op {
        warnings.push("Exact current bytes will be backed up before the change".into());
    }
    if state.completed_state_database().await?.is_none() {
        blockers.push("Storage migration must complete before instruction changes".into());
    }
    let mut plan = ProjectInstructionPlan {
        project_path: project.to_string_lossy().into_owned(),
        target: target.id.into(),
        label: target.label.into(),
        relative_path: target.relative_path.into(),
        destination: destination.to_string_lossy().into_owned(),
        operation,
        snippet_id: snippet_id.into(),
        current,
        proposed,
        exists,
        adoption,
        backup_required: exists && !no_op,
        no_op,
        warnings,
        blockers,
        revision: String::new(),
    };
    finalize_project_instruction_plan(&mut plan)?;
    Ok(plan)
}

fn project_instruction_missing_directories(
    project: &Path,
    destination: &Path,
) -> Result<Vec<PathBuf>, AppError> {
    let parent = destination.parent().ok_or_else(|| {
        invalid_project_instruction("instruction destination has no parent directory")
    })?;
    let mut missing = Vec::new();
    let relative = parent
        .strip_prefix(project)
        .map_err(|_| invalid_project_instruction("instruction destination escapes the project"))?;
    let mut candidate = project.to_path_buf();
    for component in relative.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(invalid_project_instruction(
                "instruction destination is invalid",
            ));
        }
        candidate.push(component.as_os_str());
        match std::fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(invalid_project_instruction(
                    "instruction destination parent is unsafe",
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(candidate.clone())
            }
            Err(error) => {
                return Err(AppError::Io {
                    message: format!("inspect instruction destination parent: {error}"),
                })
            }
        }
    }
    Ok(missing)
}

async fn rollback_project_instruction_operation(
    app_data_dir: &Path,
    payload: &ProjectInstructionApplyOperation,
) -> Result<(), AppError> {
    let target = project_instruction_target(&payload.target)?;
    let project = PathBuf::from(&payload.project_path);
    let destination = PathBuf::from(&payload.destination);
    let project_is_exact = std::fs::symlink_metadata(&project)
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        && std::fs::canonicalize(&project).is_ok_and(|canonical| canonical == project);
    if !project_is_exact || project.join(target.relative_path) != destination {
        return Err(AppError::StorageCorrupt {
            message: "project instruction recovery path is invalid".into(),
        });
    }
    let current = read_project_instruction_target(&project, target)
        .await?
        .map(String::into_bytes);
    let current_hash = current.as_deref().map(render::sha256_hex);
    match &payload.before_hash {
        Some(before_hash) if current_hash.as_ref() == Some(before_hash) => {}
        Some(before_hash) if current_hash.as_deref() == Some(payload.after_hash.as_str()) => {
            let backup =
                payload
                    .backup_path
                    .as_deref()
                    .ok_or_else(|| AppError::StorageCorrupt {
                        message: "project instruction recovery backup is missing".into(),
                    })?;
            let backup = Path::new(backup);
            let backup_root = backups_dir_for(app_data_dir);
            let backup_is_safe = backup.parent() == Some(backup_root.as_path())
                && std::fs::symlink_metadata(&backup_root).is_ok_and(|metadata| {
                    metadata.is_dir()
                        && !metadata.file_type().is_symlink()
                        && !crate::skills::metadata_is_reparse_point(&metadata)
                })
                && std::fs::symlink_metadata(backup).is_ok_and(|metadata| {
                    metadata.is_file()
                        && !metadata.file_type().is_symlink()
                        && !crate::skills::metadata_is_reparse_point(&metadata)
                });
            if !backup_is_safe {
                return Err(AppError::StorageCorrupt {
                    message: "project instruction recovery backup path is unsafe".into(),
                });
            }
            let bytes = read_capped(backup, MAX_PROJECT_INSTRUCTION_BYTES).await?;
            if render::sha256_hex(&bytes) != *before_hash {
                return Err(AppError::StorageCorrupt {
                    message: "project instruction recovery backup hash is invalid".into(),
                });
            }
            atomic_write(&destination, &bytes).await?;
        }
        None if current.is_none() => {}
        None if current_hash.as_deref() == Some(payload.after_hash.as_str()) => {
            tokio::fs::remove_file(&destination)
                .await
                .map_err(|error| AppError::Io {
                    message: format!("remove recovered instruction target: {error}"),
                })?;
        }
        _ => {
            return Err(AppError::StorageCorrupt {
                message: "project instruction recovery found unexpected destination bytes".into(),
            })
        }
    }
    Ok(())
}

pub(crate) async fn recover_project_instruction_operations(
    state: &AppState,
) -> Result<(), AppError> {
    let Some(database) = state.completed_state_database().await? else {
        return Ok(());
    };
    for operation in database
        .pending_filesystem_operations()
        .await?
        .into_iter()
        .filter(|operation| operation.kind == "project_instruction_apply")
    {
        let recovery = async {
            let payload: ProjectInstructionApplyOperation =
                serde_json::from_value(operation.payload.clone()).map_err(|error| {
                    AppError::StorageCorrupt {
                        message: format!("parse project instruction recovery operation: {error}"),
                    }
                })?;
            rollback_project_instruction_operation(&state.app_data_dir, &payload).await?;
            match operation.phase {
                crate::state_db::FilesystemOperationPhase::Prepared => {
                    database.abort_filesystem_operation(&operation.id).await
                }
                crate::state_db::FilesystemOperationPhase::FilesystemApplied => {
                    database.commit_filesystem_operation(&operation.id).await
                }
                crate::state_db::FilesystemOperationPhase::Committed => Ok(()),
            }
        }
        .await;
        if let Err(error) = recovery {
            database
                .retain_filesystem_operation_error(&operation.id, &error.to_string())
                .await?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn project_instructions_inspect(
    state: State<'_, AppState>,
    project_path: String,
) -> Result<Vec<ProjectInstructionTarget>, AppError> {
    inspect_project_instruction_targets(&state, &project_path).await
}

#[tauri::command]
pub async fn project_instruction_plan(
    state: State<'_, AppState>,
    project_path: String,
    target: String,
    operation: ProjectInstructionOperation,
    snippet_id: String,
    content: String,
) -> Result<ProjectInstructionPlan, AppError> {
    build_project_instruction_plan(
        &state,
        &project_path,
        &target,
        operation,
        &snippet_id,
        &content,
    )
    .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn project_instruction_apply(
    state: State<'_, AppState>,
    project_path: String,
    target: String,
    operation: ProjectInstructionOperation,
    snippet_id: String,
    content: String,
    revision: String,
    confirmed: bool,
) -> Result<ProjectInstructionApplyResponse, AppError> {
    apply_project_instruction(
        &state,
        project_path,
        target,
        operation,
        snippet_id,
        content,
        revision,
        confirmed,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn apply_project_instruction(
    state: &AppState,
    project_path: String,
    target: String,
    operation: ProjectInstructionOperation,
    snippet_id: String,
    content: String,
    revision: String,
    confirmed: bool,
) -> Result<ProjectInstructionApplyResponse, AppError> {
    if !confirmed {
        return Err(invalid_project_instruction(
            "project instruction apply requires explicit confirmation",
        ));
    }
    // ponytail: one existing registry lock serializes instruction writes; split
    // per project only if contention becomes measurable.
    let _project_lock = lock_project_registry(&state.app_data_dir)?;
    let plan = build_project_instruction_plan(
        state,
        &project_path,
        &target,
        operation,
        &snippet_id,
        &content,
    )
    .await?;
    if plan.revision != revision || !plan.blockers.is_empty() || plan.no_op {
        return Ok(ProjectInstructionApplyResponse { plan, result: None });
    }
    let database =
        state
            .completed_state_database()
            .await?
            .ok_or_else(|| AppError::StorageCorrupt {
                message: "storage migration is incomplete".into(),
            })?;
    let destination = PathBuf::from(&plan.destination);
    let backup_path = plan.backup_required.then(|| {
        backups_dir_for(&state.app_data_dir).join(format!(
            "project-instruction-{}-{}-{}.bak",
            plan.target,
            plan.snippet_id,
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ))
    });
    let created_directories =
        project_instruction_missing_directories(Path::new(&plan.project_path), &destination)?;
    let payload = ProjectInstructionApplyOperation {
        project_path: plan.project_path.clone(),
        target: plan.target.clone(),
        destination: plan.destination.clone(),
        before_hash: plan
            .exists
            .then(|| render::sha256_hex(plan.current.as_bytes())),
        after_hash: render::sha256_hex(plan.proposed.as_bytes()),
        backup_path: backup_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
    };
    let journal = database
        .prepare_filesystem_operation("project_instruction_apply", &payload)
        .await?;
    let mut filesystem_applied = false;
    let attempt = async {
        let target_def = project_instruction_target(&plan.target)?;
        let current_before_backup =
            read_project_instruction_target(Path::new(&plan.project_path), target_def).await?;
        if current_before_backup.as_deref() != plan.exists.then_some(plan.current.as_str()) {
            return Err(invalid_project_instruction(
                "instruction target changed during apply",
            ));
        }
        if let Some(backup) = &backup_path {
            tokio::fs::create_dir_all(backups_dir_for(&state.app_data_dir))
                .await
                .map_err(|error| AppError::Io {
                    message: format!("create instruction backup directory: {error}"),
                })?;
            atomic_write(
                backup,
                current_before_backup
                    .as_deref()
                    .unwrap_or_default()
                    .as_bytes(),
            )
            .await?;
            let verified = read_capped(backup, MAX_PROJECT_INSTRUCTION_BYTES).await?;
            if verified != plan.current.as_bytes() {
                return Err(AppError::Internal {
                    message: "instruction backup verification failed".into(),
                });
            }
        }
        for directory in &created_directories {
            tokio::fs::create_dir(directory)
                .await
                .map_err(|error| AppError::Io {
                    message: format!("create instruction target directory: {error}"),
                })?;
        }
        let current =
            read_project_instruction_target(Path::new(&plan.project_path), target_def).await?;
        if current.as_deref() != plan.exists.then_some(plan.current.as_str()) {
            return Err(invalid_project_instruction(
                "instruction target changed during apply",
            ));
        }
        atomic_write(&destination, plan.proposed.as_bytes()).await?;
        database.mark_filesystem_applied(&journal.id).await?;
        filesystem_applied = true;
        database.commit_filesystem_operation(&journal.id).await
    }
    .await;
    match attempt {
        Ok(()) => Ok(ProjectInstructionApplyResponse {
            result: Some(ProjectInstructionApplyResult {
                destination: plan.destination.clone(),
                outcome: "succeeded".into(),
                backup_path: payload.backup_path.clone(),
                message: None,
            }),
            plan,
        }),
        Err(error) => {
            let rollback =
                rollback_project_instruction_operation(&state.app_data_dir, &payload).await;
            let journal_close = match &rollback {
                Ok(()) if filesystem_applied => {
                    database.commit_filesystem_operation(&journal.id).await
                }
                Ok(()) => database.abort_filesystem_operation(&journal.id).await,
                Err(rollback_error) => {
                    database
                        .retain_filesystem_operation_error(
                            &journal.id,
                            &format!("{error}; rollback: {rollback_error}"),
                        )
                        .await
                }
            };
            let journal_close_error = journal_close.err().map(|error| error.to_string());
            if let Some(close_error) = &journal_close_error {
                let _ = database
                    .retain_filesystem_operation_error(
                        &journal.id,
                        &format!("{error}; journal close: {close_error}"),
                    )
                    .await;
            }
            Ok(ProjectInstructionApplyResponse {
                result: Some(ProjectInstructionApplyResult {
                    destination: plan.destination.clone(),
                    outcome: if rollback.is_ok() && journal_close_error.is_none() {
                        "rolledBack"
                    } else {
                        "rollbackFailed"
                    }
                    .into(),
                    backup_path: payload.backup_path,
                    message: Some(match (rollback, journal_close_error) {
                        (Ok(()), None) => error.to_string(),
                        (Ok(()), Some(close_error)) => {
                            format!("{error}; journal close: {close_error}")
                        }
                        (Err(rollback_error), _) => {
                            format!("{error}; rollback: {rollback_error}")
                        }
                    }),
                }),
                plan,
            })
        }
    }
}

// ---------- Loadouts (Agentfile) ----------

const WORKSPACE_PACK_VERSION: u32 = 1;
const MAX_WORKSPACE_PACK_ITEMS: usize = 256;
const MAX_WORKSPACE_PACK_NAME_BYTES: usize = 160;
const MAX_WORKSPACE_PACK_REQUIREMENT_BYTES: usize = 512;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspacePack {
    workspace_pack: u32,
    name: String,
    scope: WorkspacePackScope,
    #[serde(default)]
    agents: Vec<WorkspacePackAgent>,
    #[serde(default)]
    skills: Vec<WorkspacePackSkill>,
    #[serde(default)]
    runbook: Option<String>,
    #[serde(default)]
    instructions: Vec<String>,
    #[serde(default)]
    mcp_servers: Vec<String>,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WorkspacePackScope {
    User,
    Project,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkspacePackAgent {
    reference: AgentReference,
    tool: Tool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkspacePackSkill {
    reference: SkillReference,
    runtime: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePackAgentPlan {
    reference: AgentReference,
    name: String,
    tool: Tool,
    destinations: Vec<String>,
    dependency: bool,
    state: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePackSkillPlan {
    reference: SkillReference,
    name: String,
    runtime: String,
    destinations: Vec<String>,
    dependency: bool,
    state: String,
    permissions: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePackPlan {
    pack: WorkspacePack,
    project_path: Option<String>,
    agents: Vec<WorkspacePackAgentPlan>,
    skills: Vec<WorkspacePackSkillPlan>,
    warnings: Vec<String>,
    blockers: Vec<String>,
    rollback_scope: Vec<String>,
    revision: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePackApplyItem {
    kind: String,
    source_id: String,
    relative_path: String,
    target: String,
    destination: String,
    outcome: String,
    message: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePackApplyResult {
    revision: String,
    outcome: String,
    items: Vec<WorkspacePackApplyItem>,
    rolled_back: bool,
    rollback_errors: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePackApplyResponse {
    plan: WorkspacePackPlan,
    result: Option<WorkspacePackApplyResult>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum WorkspacePackCreated {
    Agent {
        reference: AgentReference,
        tool: Tool,
        project_path: Option<String>,
    },
    Skill {
        reference: SkillReference,
        runtime: String,
        project_path: Option<String>,
    },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct WorkspacePackApplyOperation {
    revision: String,
    expected_created: Vec<WorkspacePackCreated>,
}

#[derive(Debug)]
enum WorkspacePackInput {
    Pack(WorkspacePack),
    Legacy(Agentfile),
}

fn invalid_workspace_pack(message: impl Into<String>) -> AppError {
    AppError::InvalidArgument {
        message: message.into(),
    }
}

fn validate_workspace_pack_text(value: &str, label: &str, max: usize) -> Result<(), AppError> {
    if value.trim() != value
        || value.is_empty()
        || value.len() > max
        || value.chars().any(char::is_control)
    {
        return Err(invalid_workspace_pack(format!(
            "Workspace Pack {label} is invalid"
        )));
    }
    Ok(())
}

fn contains_workspace_pack_credential(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    [
        "token=",
        "api_key=",
        "api-key=",
        "apikey=",
        "authorization:",
        "password=",
        "-----begin private key",
    ]
    .iter()
    .any(|marker| value.contains(marker))
}

fn validate_workspace_pack_runtime(value: &str, label: &str) -> Result<(), AppError> {
    validate_workspace_pack_text(value, label, 128)
}

fn normalize_workspace_pack(mut pack: WorkspacePack) -> Result<WorkspacePack, AppError> {
    if pack.workspace_pack != WORKSPACE_PACK_VERSION {
        return Err(invalid_workspace_pack("Unsupported Workspace Pack version"));
    }
    validate_workspace_pack_text(&pack.name, "name", MAX_WORKSPACE_PACK_NAME_BYTES)?;
    if contains_workspace_pack_credential(&pack.name) {
        return Err(invalid_workspace_pack(
            "Workspace Pack must not contain credentials",
        ));
    }
    for (label, count) in [
        ("agents", pack.agents.len()),
        ("skills", pack.skills.len()),
        ("instructions", pack.instructions.len()),
        ("MCP requirements", pack.mcp_servers.len()),
    ] {
        if count > MAX_WORKSPACE_PACK_ITEMS {
            return Err(invalid_workspace_pack(format!(
                "Workspace Pack has too many {label}"
            )));
        }
    }
    for agent in &pack.agents {
        crate::library::validate_reference(
            &agent.reference.source_id,
            &agent.reference.relative_path,
        )?;
        validate_workspace_pack_runtime(&agent.tool, "agent tool")?;
    }
    for skill in &pack.skills {
        crate::library::validate_reference(
            &skill.reference.source_id,
            &skill.reference.relative_path,
        )?;
        validate_workspace_pack_runtime(&skill.runtime, "skill runtime")?;
    }
    if let Some(runbook) = &pack.runbook {
        validate_workspace_pack_text(runbook, "runbook", MAX_WORKSPACE_PACK_NAME_BYTES)?;
        if contains_workspace_pack_credential(runbook) {
            return Err(invalid_workspace_pack(
                "Workspace Pack must not contain credentials",
            ));
        }
    }
    for value in pack.instructions.iter().chain(&pack.mcp_servers) {
        validate_workspace_pack_text(value, "requirement", MAX_WORKSPACE_PACK_REQUIREMENT_BYTES)?;
        if contains_workspace_pack_credential(value) {
            return Err(invalid_workspace_pack(
                "Workspace Pack requirements must not contain credentials",
            ));
        }
    }

    pack.agents
        .sort_by(|left, right| (&left.reference, &left.tool).cmp(&(&right.reference, &right.tool)));
    pack.agents.dedup();
    pack.skills.sort_by(|left, right| {
        (
            &left.reference.source_id,
            &left.reference.relative_path,
            &left.runtime,
        )
            .cmp(&(
                &right.reference.source_id,
                &right.reference.relative_path,
                &right.runtime,
            ))
    });
    pack.skills.dedup();
    pack.instructions.sort();
    pack.instructions.dedup();
    pack.mcp_servers.sort();
    pack.mcp_servers.dedup();
    Ok(pack)
}

fn serialize_workspace_pack(pack: &WorkspacePack) -> Result<Vec<u8>, AppError> {
    let mut bytes =
        serde_json::to_vec_pretty(&normalize_workspace_pack(pack.clone())?).map_err(|error| {
            AppError::Io {
                message: format!("serialize Workspace Pack: {error}"),
            }
        })?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn parse_workspace_pack_input(bytes: &[u8]) -> Result<WorkspacePackInput, AppError> {
    if bytes.len() as u64 > MAX_INSTALLED_BYTES {
        return Err(invalid_workspace_pack("Workspace Pack is too large"));
    }
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| invalid_workspace_pack(format!("parse Workspace Pack: {error}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| invalid_workspace_pack("Workspace Pack must be a JSON object"))?;
    match (
        object
            .get("workspacePack")
            .and_then(serde_json::Value::as_u64),
        object.get("agentfile").and_then(serde_json::Value::as_u64),
    ) {
        (Some(version), None) if version == u64::from(WORKSPACE_PACK_VERSION) => {
            validate_workspace_pack_json_shape(object)?;
            let pack = serde_json::from_value(value).map_err(|error| {
                invalid_workspace_pack(format!("parse Workspace Pack: {error}"))
            })?;
            Ok(WorkspacePackInput::Pack(normalize_workspace_pack(pack)?))
        }
        (None, Some(1)) => {
            let legacy: Agentfile = serde_json::from_value(value)
                .map_err(|error| invalid_workspace_pack(format!("parse Agentfile: {error}")))?;
            if legacy.installs.len() > MAX_WORKSPACE_PACK_ITEMS {
                return Err(invalid_workspace_pack("Agentfile has too many installs"));
            }
            Ok(WorkspacePackInput::Legacy(legacy))
        }
        _ => Err(invalid_workspace_pack(
            "Unsupported or ambiguous Workspace Pack format",
        )),
    }
}

fn validate_workspace_pack_json_shape(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), AppError> {
    let allowed = [
        "workspacePack",
        "name",
        "scope",
        "agents",
        "skills",
        "runbook",
        "instructions",
        "mcpServers",
    ];
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(invalid_workspace_pack(
            "Workspace Pack contains an unknown field",
        ));
    }
    for (label, entries, target) in [
        ("agent", object.get("agents"), "tool"),
        ("skill", object.get("skills"), "runtime"),
    ] {
        let Some(entries) = entries.and_then(serde_json::Value::as_array) else {
            continue;
        };
        for entry in entries {
            let entry = entry.as_object().ok_or_else(|| {
                invalid_workspace_pack(format!("Workspace Pack {label} entry is invalid"))
            })?;
            if entry
                .keys()
                .any(|key| !matches!(key.as_str(), "reference") && key != target)
            {
                return Err(invalid_workspace_pack(format!(
                    "Workspace Pack {label} contains an unknown field"
                )));
            }
            let reference = entry
                .get("reference")
                .and_then(serde_json::Value::as_object)
                .ok_or_else(|| {
                    invalid_workspace_pack(format!("Workspace Pack {label} reference is invalid"))
                })?;
            if reference
                .keys()
                .any(|key| !matches!(key.as_str(), "sourceId" | "relativePath"))
            {
                return Err(invalid_workspace_pack(format!(
                    "Workspace Pack {label} reference contains an unknown field"
                )));
            }
        }
    }
    Ok(())
}

fn workspace_pack_from_ledgers(
    name: String,
    scope: WorkspacePackScope,
    project_path: Option<&str>,
    agent_records: &[InstallRecord],
    skill_records: &[crate::types::SkillInstallRecord],
) -> Result<WorkspacePack, AppError> {
    let selected = |candidate: Option<&str>| match scope {
        WorkspacePackScope::User => candidate.is_none(),
        WorkspacePackScope::Project => candidate == project_path,
    };
    normalize_workspace_pack(WorkspacePack {
        workspace_pack: WORKSPACE_PACK_VERSION,
        name,
        scope,
        agents: agent_records
            .iter()
            .filter(|record| selected(record.project_path.as_deref()))
            .map(|record| WorkspacePackAgent {
                reference: AgentReference {
                    source_id: record.source_id.clone(),
                    relative_path: record.relative_path.clone(),
                },
                tool: record.tool.clone(),
            })
            .collect(),
        skills: skill_records
            .iter()
            .filter(|record| selected(record.project_path.as_deref()))
            .map(|record| WorkspacePackSkill {
                reference: SkillReference {
                    source_id: record.source_id.clone(),
                    relative_path: record.relative_path.clone(),
                },
                runtime: record.runtime.clone(),
            })
            .collect(),
        runbook: None,
        instructions: Vec::new(),
        mcp_servers: Vec::new(),
    })
}

fn convert_legacy_agentfile(
    legacy: Agentfile,
    sources: &[AgentSourceResult],
) -> Result<WorkspacePack, AppError> {
    for entry in &legacy.installs {
        if let Some(path) = &entry.project_path {
            if path.trim() != path
                || path.is_empty()
                || path.len() > 4096
                || path.chars().any(char::is_control)
            {
                return Err(invalid_workspace_pack(
                    "Legacy Agentfile project path is invalid",
                ));
            }
        }
    }
    let project_paths = legacy
        .installs
        .iter()
        .filter_map(|entry| entry.project_path.as_deref())
        .collect::<std::collections::BTreeSet<_>>();
    let scope = if project_paths.is_empty() {
        WorkspacePackScope::User
    } else if project_paths.len() == 1
        && legacy
            .installs
            .iter()
            .all(|entry| entry.project_path.is_some())
    {
        WorkspacePackScope::Project
    } else {
        return Err(invalid_workspace_pack(
            "Legacy Agentfile must contain exactly one logical scope",
        ));
    };
    let agents = legacy
        .installs
        .into_iter()
        .map(|entry| {
            validate_workspace_pack_text(&entry.slug, "legacy slug", 160)?;
            validate_workspace_pack_runtime(&entry.tool, "legacy tool")?;
            Ok(WorkspacePackAgent {
                reference: resolve_legacy_workspace_pack_reference(sources, &entry.slug)?,
                tool: entry.tool,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    normalize_workspace_pack(WorkspacePack {
        workspace_pack: WORKSPACE_PACK_VERSION,
        name: "Imported Agentfile".into(),
        scope,
        agents,
        skills: Vec::new(),
        runbook: None,
        instructions: Vec::new(),
        mcp_servers: Vec::new(),
    })
}

fn resolve_legacy_workspace_pack_reference(
    sources: &[AgentSourceResult],
    slug: &str,
) -> Result<AgentReference, AppError> {
    let matches = sources
        .iter()
        .flat_map(|source| &source.agents)
        .filter(|package| {
            package.installable
                && package
                    .agent
                    .as_ref()
                    .is_some_and(|agent| agent.slug == slug)
        })
        .collect::<Vec<_>>();
    if matches.len() == 1 {
        Ok(matches[0].reference.clone())
    } else {
        Err(invalid_workspace_pack(format!(
            "Legacy Agent slug must resolve to one exact installable package: {slug}"
        )))
    }
}

fn finalize_workspace_pack_plan(plan: &mut WorkspacePackPlan) -> Result<(), AppError> {
    plan.agents.sort_by(|left, right| {
        (
            &left.reference,
            &left.tool,
            left.dependency,
            &left.destinations,
        )
            .cmp(&(
                &right.reference,
                &right.tool,
                right.dependency,
                &right.destinations,
            ))
    });
    plan.agents.dedup_by(|right, left| {
        right.reference == left.reference
            && right.tool == left.tool
            && right.destinations == left.destinations
    });
    plan.skills.sort_by(|left, right| {
        (
            &left.reference.source_id,
            &left.reference.relative_path,
            &left.runtime,
            left.dependency,
            &left.destinations,
        )
            .cmp(&(
                &right.reference.source_id,
                &right.reference.relative_path,
                &right.runtime,
                right.dependency,
                &right.destinations,
            ))
    });
    plan.skills.dedup_by(|right, left| {
        right.reference == left.reference
            && right.runtime == left.runtime
            && right.destinations == left.destinations
    });
    for values in [
        &mut plan.warnings,
        &mut plan.blockers,
        &mut plan.rollback_scope,
    ] {
        values.sort();
        values.dedup();
    }
    plan.revision.clear();
    plan.revision =
        render::sha256_hex(
            &serde_json::to_vec(plan).map_err(|error| AppError::Internal {
                message: format!("serialize Workspace Pack plan: {error}"),
            })?,
        );
    Ok(())
}

fn require_workspace_pack_revision(
    plan: &WorkspacePackPlan,
    expected: &str,
) -> Result<(), AppError> {
    if plan.revision == expected {
        Ok(())
    } else {
        Err(invalid_workspace_pack(
            "Workspace Pack plan changed; review the refreshed plan",
        ))
    }
}

fn initial_workspace_pack_results(plan: &WorkspacePackPlan) -> Vec<WorkspacePackApplyItem> {
    let mut items = Vec::new();
    for agent in &plan.agents {
        for destination in &agent.destinations {
            items.push(WorkspacePackApplyItem {
                kind: "agent".into(),
                source_id: agent.reference.source_id.clone(),
                relative_path: agent.reference.relative_path.clone(),
                target: agent.tool.clone(),
                destination: destination.clone(),
                outcome: if agent.state == "current" {
                    "current"
                } else {
                    "pending"
                }
                .into(),
                message: None,
            });
        }
    }
    for skill in &plan.skills {
        for destination in &skill.destinations {
            items.push(WorkspacePackApplyItem {
                kind: "skill".into(),
                source_id: skill.reference.source_id.clone(),
                relative_path: skill.reference.relative_path.clone(),
                target: skill.runtime.clone(),
                destination: destination.clone(),
                outcome: if skill.state == "current" {
                    "current"
                } else {
                    "pending"
                }
                .into(),
                message: None,
            });
        }
    }
    items
}

async fn bind_workspace_pack_project(
    state: &AppState,
    scope: WorkspacePackScope,
    project_path: Option<String>,
) -> Result<Option<String>, AppError> {
    match (scope, project_path) {
        (WorkspacePackScope::User, None) => Ok(None),
        (WorkspacePackScope::User, Some(_)) => Err(invalid_workspace_pack(
            "User Workspace Pack must not include a project binding",
        )),
        (WorkspacePackScope::Project, None) => Ok(None),
        (WorkspacePackScope::Project, Some(path)) => {
            let canonical = std::fs::canonicalize(&path).map_err(|error| AppError::Io {
                message: format!("canonicalize Workspace Pack project: {error}"),
            })?;
            if !registered_projects(&state.app_data_dir)
                .await?
                .contains(&canonical)
            {
                return Err(invalid_workspace_pack(
                    "Workspace Pack project binding must be registered",
                ));
            }
            Ok(Some(canonical.to_string_lossy().into_owned()))
        }
    }
}

fn agent_install_state_name(state: InstallState) -> &'static str {
    match state {
        InstallState::Current => "current",
        InstallState::Outdated => "outdated",
        InstallState::Modified => "modified",
        InstallState::Missing => "missingTracked",
        InstallState::Foreign => "foreign",
        InstallState::Disabled => "disabled",
        InstallState::SourceUnavailable => "sourceUnavailable",
    }
}

fn skill_install_state_name(state: crate::types::SkillInstallState) -> &'static str {
    match state {
        crate::types::SkillInstallState::Current => "current",
        crate::types::SkillInstallState::Outdated => "outdated",
        crate::types::SkillInstallState::Modified => "modified",
        crate::types::SkillInstallState::Missing => "missingTracked",
        crate::types::SkillInstallState::Foreign => "foreign",
        crate::types::SkillInstallState::Disabled => "disabled",
        crate::types::SkillInstallState::SourceUnavailable => "sourceUnavailable",
    }
}

async fn build_workspace_pack_plan(
    state: &AppState,
    pack: WorkspacePack,
    project_path: Option<String>,
) -> Result<WorkspacePackPlan, AppError> {
    let project_path = bind_workspace_pack_project(state, pack.scope, project_path).await?;
    if pack.scope == WorkspacePackScope::Project && project_path.is_none() {
        let mut plan = WorkspacePackPlan {
            agents: pack
                .agents
                .iter()
                .map(|item| WorkspacePackAgentPlan {
                    reference: item.reference.clone(),
                    name: item.reference.relative_path.clone(),
                    tool: item.tool.clone(),
                    destinations: Vec::new(),
                    dependency: false,
                    state: "blocked".into(),
                })
                .collect(),
            skills: pack
                .skills
                .iter()
                .map(|item| WorkspacePackSkillPlan {
                    reference: item.reference.clone(),
                    name: item.reference.relative_path.clone(),
                    runtime: item.runtime.clone(),
                    destinations: Vec::new(),
                    dependency: false,
                    state: "blocked".into(),
                    permissions: Vec::new(),
                })
                .collect(),
            pack,
            project_path: None,
            warnings: Vec::new(),
            blockers: vec!["Project Workspace Pack requires an explicit project binding".into()],
            rollback_scope: Vec::new(),
            revision: String::new(),
        };
        finalize_workspace_pack_plan(&mut plan)?;
        return Ok(plan);
    }
    let agent_sources = crate::agents::inspect_agent_sources(&state.app_data_dir).await?;
    let installed_agents = mcp_reconcile_agent_installs(state).await?;
    let registered = registered_projects(&state.app_data_dir)
        .await?
        .into_iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let installed_skills = crate::skills::reconcile_skill_installs(state, &registered).await?;
    let mut plan = WorkspacePackPlan {
        pack,
        project_path,
        agents: Vec::new(),
        skills: Vec::new(),
        warnings: Vec::new(),
        blockers: Vec::new(),
        rollback_scope: Vec::new(),
        revision: String::new(),
    };

    for requested in &plan.pack.agents {
        let mutation = match build_mutation_plan(
            state,
            vec![requested.reference.clone()],
            requested.tool.clone(),
            plan.project_path.clone(),
            "install",
            true,
        )
        .await
        {
            Ok(mutation) => mutation,
            Err(error) => {
                plan.blockers.push(error.to_string());
                plan.agents.push(WorkspacePackAgentPlan {
                    reference: requested.reference.clone(),
                    name: requested.reference.relative_path.clone(),
                    tool: requested.tool.clone(),
                    destinations: Vec::new(),
                    dependency: false,
                    state: "blocked".into(),
                });
                continue;
            }
        };
        plan.warnings.extend(mutation.warnings);
        plan.blockers.extend(mutation.blockers);
        let has_requested = mutation
            .agents
            .iter()
            .any(|item| !item.dependency && item.reference == requested.reference);
        let home = tool_home(state, &requested.tool).await?;
        for item in mutation.agents {
            let existing = installed_agents.iter().find(|installed| {
                installed.source_id == item.reference.source_id
                    && installed.relative_path == item.reference.relative_path
                    && installed.tool == requested.tool
                    && installed.project_path == plan.project_path
            });
            let state_name = existing
                .map(|installed| agent_install_state_name(installed.state))
                .unwrap_or("missing");
            if existing.is_some_and(|installed| installed.state != InstallState::Current) {
                plan.blockers.push(format!(
                    "Agent is not safe to apply in state {state_name}: {}:{}",
                    item.reference.source_id, item.reference.relative_path
                ));
            }
            let destinations = package_by_reference(&agent_sources, &item.reference)
                .and_then(|package| package.agent.as_ref())
                .map(|agent| {
                    install_target_paths(
                        agent,
                        None,
                        &requested.tool,
                        &home,
                        plan.project_path.as_deref().map(Path::new),
                        existing.map(|installed| Path::new(&installed.dest)),
                    )
                    .map(|paths| {
                        paths
                            .into_iter()
                            .map(|path| path.to_string_lossy().into_owned())
                            .collect::<Vec<_>>()
                    })
                })
                .transpose()?
                .unwrap_or_else(|| vec![item.destination.clone()]);
            if state_name == "missing" {
                plan.rollback_scope.extend(destinations.clone());
            }
            plan.agents.push(WorkspacePackAgentPlan {
                reference: item.reference,
                name: item.name,
                tool: requested.tool.clone(),
                destinations,
                dependency: item.dependency,
                state: state_name.into(),
            });
        }
        if !has_requested {
            plan.agents.push(WorkspacePackAgentPlan {
                reference: requested.reference.clone(),
                name: requested.reference.relative_path.clone(),
                tool: requested.tool.clone(),
                destinations: Vec::new(),
                dependency: false,
                state: "blocked".into(),
            });
        }
    }

    for requested in &plan.pack.skills {
        let mutation = match crate::skills::plan_skill_install(
            state,
            &requested.reference.source_id,
            &requested.reference.relative_path,
            &requested.runtime,
            plan.project_path.as_deref(),
        )
        .await
        {
            Ok(mutation) => mutation,
            Err(error) => {
                plan.blockers.push(error.to_string());
                plan.skills.push(WorkspacePackSkillPlan {
                    reference: requested.reference.clone(),
                    name: requested.reference.relative_path.clone(),
                    runtime: requested.runtime.clone(),
                    destinations: Vec::new(),
                    dependency: false,
                    state: "blocked".into(),
                    permissions: Vec::new(),
                });
                continue;
            }
        };
        plan.warnings.extend(mutation.warnings);
        plan.blockers.extend(mutation.blockers);
        for item in mutation.packages {
            let existing = installed_skills.iter().find(|installed| {
                installed.source_id == item.source_id
                    && installed.relative_path == item.relative_path
                    && installed.runtime == requested.runtime
                    && installed.project_path == plan.project_path
            });
            let state_name = existing
                .map(|installed| skill_install_state_name(installed.state))
                .unwrap_or("missing");
            if existing.is_some_and(|installed| {
                installed.state != crate::types::SkillInstallState::Current
            }) {
                plan.blockers.push(format!(
                    "Skill is not safe to apply in state {state_name}: {}:{}",
                    item.source_id, item.relative_path
                ));
            }
            if state_name == "missing" {
                plan.rollback_scope.push(item.destination.clone());
            }
            plan.skills.push(WorkspacePackSkillPlan {
                reference: SkillReference {
                    source_id: item.source_id,
                    relative_path: item.relative_path,
                },
                name: item.name,
                runtime: requested.runtime.clone(),
                destinations: vec![item.destination],
                dependency: item.dependency,
                state: state_name.into(),
                permissions: item.permissions,
            });
        }
    }
    if plan.pack.runbook.is_some() {
        plan.warnings
            .push("Runbook context is declarative and will not be executed automatically".into());
    }
    if !plan.pack.instructions.is_empty() {
        plan.warnings.push(
            "Instruction requirements are declarative and will not be applied automatically".into(),
        );
    }
    if !plan.pack.mcp_servers.is_empty() {
        plan.warnings.push(
            "MCP requirements are declarative and will not be configured automatically".into(),
        );
    }
    finalize_workspace_pack_plan(&mut plan)?;
    Ok(plan)
}

/// Portable manifest of an install set — "set up a new Mac in one click".
/// JSON so it's diffable + shareable; `tool` uses the camelCase wire value.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Agentfile {
    /// Format version.
    agentfile: u32,
    installs: Vec<LoadoutEntry>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LoadoutEntry {
    slug: String,
    tool: Tool,
    #[serde(default)]
    project_path: Option<String>,
}

/// Export the current ledger as an Agentfile written to `path`. Returns count.
#[tauri::command]
pub async fn loadout_export(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    name: String,
    scope: WorkspacePackScope,
    project_path: Option<String>,
) -> Result<u32, AppError> {
    let project_path = match scope {
        WorkspacePackScope::User if project_path.is_none() => None,
        WorkspacePackScope::Project => {
            let requested = project_path.ok_or_else(|| {
                invalid_workspace_pack("Project Workspace Pack requires a project")
            })?;
            let canonical = std::fs::canonicalize(&requested).map_err(|error| AppError::Io {
                message: format!("canonicalize Workspace Pack project: {error}"),
            })?;
            if !registered_projects(&state.app_data_dir)
                .await?
                .contains(&canonical)
            {
                return Err(invalid_workspace_pack(
                    "Workspace Pack project must be registered",
                ));
            }
            Some(canonical.to_string_lossy().into_owned())
        }
        WorkspacePackScope::User => {
            return Err(invalid_workspace_pack(
                "User Workspace Pack must not include a project",
            ))
        }
    };
    corpus::ensure_corpus(&app, &state).await?;
    let installed_agents = mcp_reconcile_agent_installs(&state).await?;
    let registered = registered_projects(&state.app_data_dir)
        .await?
        .into_iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let installed_skills = crate::skills::reconcile_skill_installs(&state, &registered).await?;
    let selected = |candidate: Option<&str>| match scope {
        WorkspacePackScope::User => candidate.is_none(),
        WorkspacePackScope::Project => candidate == project_path.as_deref(),
    };
    if installed_agents
        .iter()
        .any(|item| selected(item.project_path.as_deref()) && item.state != InstallState::Current)
        || installed_skills.iter().any(|item| {
            item.tracked
                && selected(item.project_path.as_deref())
                && item.state != crate::types::SkillInstallState::Current
        })
    {
        return Err(invalid_workspace_pack(
            "Workspace Pack export requires current managed Agent and Skill state",
        ));
    }
    let agent_records = load_ledger(&app, &state).await?;
    let skill_records = crate::skills::install::load_ledger_for_state(&state).await?;
    if agent_records
        .iter()
        .filter(|record| selected(record.project_path.as_deref()))
        .any(|record| !registry::get(&record.tool).is_some_and(registry::ToolMeta::installable))
        || skill_records
            .iter()
            .filter(|record| selected(record.project_path.as_deref()))
            .any(|record| {
                !registry::get(&record.runtime).is_some_and(registry::ToolMeta::installable)
            })
    {
        return Err(invalid_workspace_pack(
            "Workspace Pack export contains an unsupported target",
        ));
    }
    let pack = workspace_pack_from_ledgers(
        name,
        scope,
        project_path.as_deref(),
        &agent_records,
        &skill_records,
    )?;
    let n = (pack.agents.len() + pack.skills.len()) as u32;
    let bytes = serialize_workspace_pack(&pack)?;
    atomic_write(Path::new(&path), &bytes).await?;
    Ok(n)
}

/// Inspect a Workspace Pack or legacy Agentfile without mutating destinations.
#[tauri::command]
pub async fn loadout_import(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    project_path: Option<String>,
) -> Result<WorkspacePackPlan, AppError> {
    let bytes = read_capped(Path::new(&path), MAX_INSTALLED_BYTES).await?;
    corpus::ensure_corpus(&app, &state).await?;
    let pack = match parse_workspace_pack_input(&bytes)? {
        WorkspacePackInput::Pack(pack) => pack,
        WorkspacePackInput::Legacy(legacy) => convert_legacy_agentfile(
            legacy,
            &crate::agents::inspect_agent_sources(&state.app_data_dir).await?,
        )?,
    };
    build_workspace_pack_plan(&state, pack, project_path).await
}

async fn read_workspace_pack_file(
    app: &AppHandle,
    state: &AppState,
    path: &Path,
) -> Result<WorkspacePack, AppError> {
    let bytes = read_capped(path, MAX_INSTALLED_BYTES).await?;
    corpus::ensure_corpus(app, state).await?;
    match parse_workspace_pack_input(&bytes)? {
        WorkspacePackInput::Pack(pack) => Ok(pack),
        WorkspacePackInput::Legacy(legacy) => convert_legacy_agentfile(
            legacy,
            &crate::agents::inspect_agent_sources(&state.app_data_dir).await?,
        ),
    }
}

fn expected_workspace_pack_creations(plan: &WorkspacePackPlan) -> Vec<WorkspacePackCreated> {
    let mut created = plan
        .agents
        .iter()
        .filter(|item| item.state == "missing")
        .map(|item| WorkspacePackCreated::Agent {
            reference: item.reference.clone(),
            tool: item.tool.clone(),
            project_path: plan.project_path.clone(),
        })
        .chain(
            plan.skills
                .iter()
                .filter(|item| item.state == "missing")
                .map(|item| WorkspacePackCreated::Skill {
                    reference: item.reference.clone(),
                    runtime: item.runtime.clone(),
                    project_path: plan.project_path.clone(),
                }),
        )
        .collect::<Vec<_>>();
    created.dedup();
    created
}

async fn rollback_workspace_pack_created(
    _app: &AppHandle,
    state: &AppState,
    created: &[WorkspacePackCreated],
) -> Vec<String> {
    let mut errors = Vec::new();
    for item in created.iter().rev() {
        let result = match item {
            WorkspacePackCreated::Agent {
                reference,
                tool,
                project_path,
            } => do_uninstall(state, reference.clone(), tool.clone(), project_path.clone()).await,
            WorkspacePackCreated::Skill {
                reference,
                runtime,
                project_path,
            } => crate::skills::uninstall_skill(
                state,
                &reference.source_id,
                &reference.relative_path,
                runtime,
                project_path.as_deref(),
            )
            .await
            .map(|_| ()),
        };
        if let Err(error) = result {
            errors.push(error.to_string());
        }
    }
    errors
}

pub(crate) async fn recover_workspace_pack_operations(
    app: &AppHandle,
    state: &AppState,
) -> Result<(), AppError> {
    let Some(database) = state.completed_state_database().await? else {
        return Ok(());
    };
    for operation in database
        .pending_filesystem_operations()
        .await?
        .into_iter()
        .filter(|operation| operation.kind == "workspace_pack_apply")
    {
        let recovery = async {
            let payload: WorkspacePackApplyOperation =
                serde_json::from_value(operation.payload.clone()).map_err(|error| {
                    AppError::StorageCorrupt {
                        message: format!("parse Workspace Pack recovery operation: {error}"),
                    }
                })?;
            let agent_records = load_ledger_for_state(state).await?;
            let skill_records = crate::skills::install::load_ledger_for_state(state).await?;
            let present = payload
                .expected_created
                .into_iter()
                .filter(|item| match item {
                    WorkspacePackCreated::Agent {
                        reference,
                        tool,
                        project_path,
                    } => agent_records.iter().any(|record| {
                        record.source_id == reference.source_id
                            && record.relative_path == reference.relative_path
                            && record.tool == *tool
                            && record.project_path == *project_path
                    }),
                    WorkspacePackCreated::Skill {
                        reference,
                        runtime,
                        project_path,
                    } => skill_records.iter().any(|record| {
                        record.source_id == reference.source_id
                            && record.relative_path == reference.relative_path
                            && record.runtime == *runtime
                            && record.project_path == *project_path
                    }),
                })
                .collect::<Vec<_>>();
            let errors = rollback_workspace_pack_created(app, state, &present).await;
            if errors.is_empty() {
                database.abort_filesystem_operation(&operation.id).await
            } else {
                Err(AppError::StorageCorrupt {
                    message: format!(
                        "Workspace Pack recovery rollback failed: {}",
                        errors.join("; ")
                    ),
                })
            }
        }
        .await;
        if let Err(error) = recovery {
            database
                .retain_filesystem_operation_error(&operation.id, &error.to_string())
                .await?;
        }
    }
    Ok(())
}

fn update_workspace_pack_item_outcome(
    items: &mut [WorkspacePackApplyItem],
    created: &WorkspacePackCreated,
    outcome: &str,
) {
    for item in items {
        let matches = match created {
            WorkspacePackCreated::Agent {
                reference, tool, ..
            } => {
                item.kind == "agent"
                    && item.source_id == reference.source_id
                    && item.relative_path == reference.relative_path
                    && item.target == *tool
            }
            WorkspacePackCreated::Skill {
                reference, runtime, ..
            } => {
                item.kind == "skill"
                    && item.source_id == reference.source_id
                    && item.relative_path == reference.relative_path
                    && item.target == *runtime
            }
        };
        if matches {
            item.outcome = outcome.into();
        }
    }
}

/// Apply only an unchanged, unblocked Workspace Pack plan.
#[tauri::command]
pub async fn loadout_apply(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    project_path: Option<String>,
    revision: String,
) -> Result<WorkspacePackApplyResponse, AppError> {
    let pack = read_workspace_pack_file(&app, &state, Path::new(&path)).await?;
    let plan = build_workspace_pack_plan(&state, pack, project_path).await?;
    if require_workspace_pack_revision(&plan, &revision).is_err() || !plan.blockers.is_empty() {
        return Ok(WorkspacePackApplyResponse { plan, result: None });
    }

    let expected_created = expected_workspace_pack_creations(&plan);
    let database = state.completed_state_database().await?;
    let operation = if let Some(database) = &database {
        Some(
            database
                .prepare_filesystem_operation(
                    "workspace_pack_apply",
                    &WorkspacePackApplyOperation {
                        revision: plan.revision.clone(),
                        expected_created,
                    },
                )
                .await?,
        )
    } else {
        None
    };
    let mut items = initial_workspace_pack_results(&plan);
    let mut created = Vec::<WorkspacePackCreated>::new();
    let mut failure = None;

    for requested in &plan.pack.agents {
        let root_missing = plan.agents.iter().any(|item| {
            !item.dependency
                && item.reference == requested.reference
                && item.tool == requested.tool
                && item.state == "missing"
        });
        if !root_missing {
            continue;
        }
        let mut mutation = build_mutation_plan(
            &state,
            vec![requested.reference.clone()],
            requested.tool.clone(),
            plan.project_path.clone(),
            "install",
            true,
        )
        .await?;
        mutation.agents.retain(|candidate| {
            plan.agents.iter().any(|item| {
                item.reference == candidate.reference
                    && item.tool == requested.tool
                    && item.state == "missing"
            }) && !created.iter().any(|created| {
                matches!(created, WorkspacePackCreated::Agent { reference, tool, .. }
                    if reference == &candidate.reference && tool == &requested.tool)
            })
        });
        if mutation.agents.is_empty() {
            continue;
        }
        match execute_install_plan(&app, &state, &mutation, true).await {
            Ok(records) => {
                for record in records {
                    let created_item = WorkspacePackCreated::Agent {
                        reference: AgentReference {
                            source_id: record.source_id,
                            relative_path: record.relative_path,
                        },
                        tool: record.tool,
                        project_path: record.project_path,
                    };
                    update_workspace_pack_item_outcome(&mut items, &created_item, "installed");
                    created.push(created_item);
                }
            }
            Err(error) => {
                failure = Some(error);
                break;
            }
        }
    }

    if failure.is_none() {
        for requested in &plan.pack.skills {
            let root_missing = plan.skills.iter().any(|item| {
                !item.dependency
                    && item.reference == requested.reference
                    && item.runtime == requested.runtime
                    && item.state == "missing"
            });
            let already_created = created.iter().any(|created| {
                matches!(created, WorkspacePackCreated::Skill { reference, runtime, .. }
                    if reference == &requested.reference && runtime == &requested.runtime)
            });
            if !root_missing || already_created {
                continue;
            }
            match crate::skills::install_skill_with_dependencies(
                &state,
                &requested.reference.source_id,
                &requested.reference.relative_path,
                &requested.runtime,
                plan.project_path.as_deref(),
            )
            .await
            {
                Ok(installed) => {
                    for record in installed {
                        let created_item = WorkspacePackCreated::Skill {
                            reference: SkillReference {
                                source_id: record.source_id,
                                relative_path: record.relative_path,
                            },
                            runtime: record.runtime,
                            project_path: record.project_path,
                        };
                        update_workspace_pack_item_outcome(&mut items, &created_item, "installed");
                        if !created.contains(&created_item) {
                            created.push(created_item);
                        }
                    }
                }
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
    }

    if failure.is_none() {
        if let Err(error) = mcp_reconcile_agent_installs(&state).await {
            failure = Some(error);
        } else {
            let projects = registered_projects(&state.app_data_dir)
                .await?
                .into_iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            if let Err(error) = crate::skills::reconcile_skill_installs(&state, &projects).await {
                failure = Some(error);
            }
        }
    }

    if let Some(error) = failure {
        let rollback_errors = rollback_workspace_pack_created(&app, &state, &created).await;
        let rollback_outcome = if rollback_errors.is_empty() {
            "rolledBack"
        } else {
            "rollbackFailed"
        };
        for created_item in &created {
            update_workspace_pack_item_outcome(&mut items, created_item, rollback_outcome);
        }
        if let Some(item) = items.iter_mut().find(|item| item.outcome == "pending") {
            item.outcome = "failed".into();
            item.message = Some(error.to_string());
        }
        for item in items.iter_mut().filter(|item| item.outcome == "pending") {
            item.outcome = "skipped".into();
        }
        if let (Some(database), Some(operation)) = (&database, &operation) {
            if rollback_errors.is_empty() {
                database.abort_filesystem_operation(&operation.id).await?;
            } else {
                database
                    .retain_filesystem_operation_error(
                        &operation.id,
                        &format!("{error}; rollback: {}", rollback_errors.join("; ")),
                    )
                    .await?;
            }
        }
        let projects = registered_projects(&state.app_data_dir)
            .await?
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let _ = mcp_reconcile_agent_installs(&state).await;
        let _ = crate::skills::reconcile_skill_installs(&state, &projects).await;
        return Ok(WorkspacePackApplyResponse {
            result: Some(WorkspacePackApplyResult {
                revision: plan.revision.clone(),
                outcome: rollback_outcome.into(),
                items,
                rolled_back: rollback_errors.is_empty(),
                rollback_errors,
            }),
            plan,
        });
    }

    if let (Some(database), Some(operation)) = (&database, &operation) {
        database.commit_filesystem_operation(&operation.id).await?;
    }
    Ok(WorkspacePackApplyResponse {
        result: Some(WorkspacePackApplyResult {
            revision: plan.revision.clone(),
            outcome: "succeeded".into(),
            items,
            rolled_back: false,
            rollback_errors: Vec::new(),
        }),
        plan,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::CatalogSnapshotItem;

    #[tokio::test]
    async fn cleanup_sync_failure_restores_write_intent_and_preserves_payload() {
        let app_data = tempfile::tempdir().unwrap();
        let root = app_data.path().join("state/roster-staging");
        std::fs::create_dir_all(&root).unwrap();
        let id = uuid::Uuid::new_v4();
        let staged = root.join(format!("{id}.bin"));
        let intent = root.join(format!("{id}.intent.json"));
        std::fs::write(&staged, b"roster").unwrap();
        std::fs::write(&intent, b"intent").unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app_data.path().to_path_buf();
        let mut sync_attempts = 0;

        let result = cleanup_roster_staging_artifacts_durable_with_sync(&state, &staged, |_| {
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
        assert_eq!(std::fs::read(&staged).unwrap(), b"roster");
        assert_eq!(sync_attempts, 2);
    }

    fn roster_record(member_count: usize) -> AgentRosterInstallRecord {
        AgentRosterInstallRecord {
            tool: "aider".into(),
            scope: crate::types::Scope::Project,
            project_path: "/project".into(),
            dest: "/project/CONVENTIONS.md".into(),
            members: (0..member_count)
                .map(|index| AgentRosterMember {
                    reference: AgentReference {
                        source_id: "builtin:agency-agents".into(),
                        relative_path: format!("engineering/agent-{index}.md"),
                    },
                    name: format!("Agent {index}"),
                    source_hash: "a".repeat(64),
                })
                .collect(),
            rendered_hash: "b".repeat(64),
            disabled_path: None,
            installed_at: "2026-08-17T00:00:00Z".into(),
        }
    }

    async fn completed_sqlite_roster_install() -> (
        tempfile::TempDir,
        tempfile::TempDir,
        tempfile::TempDir,
        crate::state_db::StateDatabase,
        AppState,
        Vec<AgentReference>,
        AgentRosterInstallRecord,
        Vec<u8>,
    ) {
        let app = tempfile::tempdir().unwrap();
        let source_root = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        for path in ["agent.md", "reviewer.md"] {
            std::fs::write(
                source_root.path().join(path),
                format!("---\nname: {path}\ndescription: Test.\n---\nOriginal {path}.\n"),
            )
            .unwrap();
        }
        let source = crate::agents::add_local_source(app.path(), source_root.path())
            .await
            .unwrap();
        let project_path = std::fs::canonicalize(project.path()).unwrap();
        let database = crate::state_db::StateDatabase::open(app.path()).unwrap();
        database
            .mutate(installs_spec(), Vec::new(), |_| Ok(()))
            .await
            .unwrap();
        database
            .mutate(projects_spec(), Vec::new(), {
                let project_path = project_path.clone();
                move |projects| {
                    projects.push(project_path);
                    Ok(())
                }
            })
            .await
            .unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let connection =
            rusqlite::Connection::open(app.path().join("state/agency-agents.sqlite3")).unwrap();
        connection
            .execute(
                "INSERT INTO state_documents \
                 (name, document_version, revision, payload, updated_at) VALUES \
                 ('agent_sources', 1, 1, ?1, ?2)",
                rusqlite::params![
                    serde_json::to_string(&vec![source.clone()]).unwrap(),
                    chrono::Utc::now().to_rfc3339()
                ],
            )
            .unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app.path().to_path_buf();
        let references = ["agent.md", "reviewer.md"]
            .into_iter()
            .map(|relative_path| AgentReference {
                source_id: source.id.clone(),
                relative_path: relative_path.into(),
            })
            .collect::<Vec<_>>();
        let plan = build_roster_plan(
            &state,
            references.clone(),
            "aider".into(),
            project_path.to_string_lossy().into_owned(),
            "install",
        )
        .await
        .unwrap();
        let record = apply_roster_plan(&state, &plan, &plan.revision, true)
            .await
            .unwrap();
        let bytes = std::fs::read(&record.dest).unwrap();
        (
            app,
            source_root,
            project,
            database,
            state,
            references,
            record,
            bytes,
        )
    }

    async fn assert_exact_roster_state(
        state: &AppState,
        database: &crate::state_db::StateDatabase,
        record: &AgentRosterInstallRecord,
        bytes: &[u8],
        pending: usize,
    ) {
        assert_eq!(std::fs::read(&record.dest).unwrap(), bytes);
        assert_eq!(
            load_rosters_for_state(state).await.unwrap(),
            vec![record.clone()]
        );
        assert_eq!(
            database
                .pending_filesystem_operations()
                .await
                .unwrap()
                .len(),
            pending
        );
    }

    fn history_tree_bytes(app_data_dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        fn collect(root: &Path, directory: &Path, files: &mut Vec<(PathBuf, Vec<u8>)>) {
            let entries = match std::fs::read_dir(directory) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
                Err(error) => panic!("read history tree {}: {error}", directory.display()),
            };
            for entry in entries {
                let entry = entry.unwrap();
                let path = entry.path();
                let metadata = entry.metadata().unwrap();
                if metadata.is_dir() {
                    collect(root, &path, files);
                } else if metadata.is_file() {
                    files.push((
                        path.strip_prefix(root).unwrap().to_path_buf(),
                        std::fs::read(path).unwrap(),
                    ));
                }
            }
        }

        let root = app_data_dir.join("agents/history");
        let mut files = Vec::new();
        collect(&root, &root, &mut files);
        files.sort_by(|left, right| left.0.cmp(&right.0));
        files
    }

    fn command_test_app(app_data_dir: &Path) -> tauri::App<tauri::test::MockRuntime> {
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app_data_dir.to_path_buf();
        let mut context: tauri::Context<tauri::test::MockRuntime> =
            tauri::test::mock_context(tauri::test::noop_assets());
        context.config_mut().identifier = app_data_dir.to_string_lossy().into_owned();
        tauri::test::mock_builder()
            .manage(state)
            .build(context)
            .unwrap()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn roster_production_commands_cover_all_boundaries_and_uninstall_route() {
        use tauri::Manager as _;

        let app_data = tempfile::tempdir().unwrap();
        let source_root = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(app_data.path().join("corpus")).unwrap();
        for path in ["agent.md", "reviewer.md"] {
            std::fs::write(
                source_root.path().join(path),
                format!("---\nname: {path}\ndescription: Test.\n---\nOriginal {path}.\n"),
            )
            .unwrap();
        }
        let source = crate::agents::add_local_source(app_data.path(), source_root.path())
            .await
            .unwrap();
        let references = ["agent.md", "reviewer.md"]
            .into_iter()
            .map(|relative_path| AgentReference {
                source_id: source.id.clone(),
                relative_path: relative_path.into(),
            })
            .collect::<Vec<_>>();
        let project_path = std::fs::canonicalize(project.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let app = command_test_app(app_data.path());
        let handle = app.handle().clone();
        assert_eq!(corpus::app_data_dir(&handle).unwrap(), app_data.path());

        assert!(agent_roster_plan(
            handle.clone(),
            app.state(),
            references.clone(),
            "aider".into(),
            project_path.clone(),
            "install".into(),
        )
        .await
        .is_err());
        project_register(app.state(), project_path.clone())
            .await
            .unwrap();
        let plan = agent_roster_plan(
            handle.clone(),
            app.state(),
            references.clone(),
            "aider".into(),
            project_path.clone(),
            "install".into(),
        )
        .await
        .unwrap();
        assert!(!Path::new(&plan.destination).exists());
        assert!(agent_roster_apply(
            handle.clone(),
            app.state(),
            references.clone(),
            "aider".into(),
            project_path.clone(),
            "install".into(),
            plan.revision.clone(),
            None,
        )
        .await
        .is_err());
        assert!(agent_roster_apply(
            handle.clone(),
            app.state(),
            references.clone(),
            "aider".into(),
            project_path.clone(),
            "install".into(),
            "stale-revision".into(),
            Some(true),
        )
        .await
        .is_err());
        assert!(!Path::new(&plan.destination).exists());
        project_unregister(app.state(), project_path.clone())
            .await
            .unwrap();
        assert!(agent_roster_apply(
            handle.clone(),
            app.state(),
            references.clone(),
            "aider".into(),
            project_path.clone(),
            "install".into(),
            plan.revision.clone(),
            Some(true),
        )
        .await
        .is_err());
        assert!(!Path::new(&plan.destination).exists());
        project_register(app.state(), project_path.clone())
            .await
            .unwrap();
        let plan = agent_roster_plan(
            handle.clone(),
            app.state(),
            references.clone(),
            "aider".into(),
            project_path.clone(),
            "install".into(),
        )
        .await
        .unwrap();
        let installed = agent_roster_apply(
            handle.clone(),
            app.state(),
            references.clone(),
            "aider".into(),
            project_path.clone(),
            "install".into(),
            plan.revision,
            Some(true),
        )
        .await
        .unwrap();
        let reconciled = agent_rosters_reconcile(app.state()).await.unwrap();
        assert_eq!(reconciled.len(), 1);
        assert_eq!(reconciled[0].state, InstallState::Current);

        assert!(
            agent_roster_disable(app.state(), "aider".into(), project_path.clone(), None,)
                .await
                .is_err()
        );
        agent_roster_disable(
            app.state(),
            "aider".into(),
            project_path.clone(),
            Some(true),
        )
        .await
        .unwrap();
        agent_roster_enable(
            app.state(),
            "aider".into(),
            project_path.clone(),
            Some(true),
        )
        .await
        .unwrap();
        let history =
            agent_roster_version_history(app.state(), "aider".into(), project_path.clone())
                .await
                .unwrap();
        let foreign_edit = b"user-authored roster edit";
        std::fs::write(&installed.dest, foreign_edit).unwrap();
        agent_roster_version_rollback(
            app.state(),
            "aider".into(),
            project_path.clone(),
            history[0].id.clone(),
            Some(true),
        )
        .await
        .unwrap();
        let preserved =
            agent_roster_version_history(app.state(), "aider".into(), project_path.clone())
                .await
                .unwrap()
                .into_iter()
                .find(|snapshot| snapshot.rendered_hash == render::sha256_hex(foreign_edit))
                .expect("modified roster bytes must remain rollback-able");
        let (_, preserved_contents) = history::snapshot_contents(
            app_data.path(),
            &roster_identity(&installed),
            &preserved.id,
            std::slice::from_ref(&PathBuf::from(&installed.dest)),
        )
        .await
        .unwrap();
        assert_eq!(preserved_contents, vec![foreign_edit.to_vec()]);

        let uninstall = agent_roster_plan(
            handle.clone(),
            app.state(),
            references.clone(),
            "aider".into(),
            project_path.clone(),
            "uninstall".into(),
        )
        .await
        .unwrap();
        agent_roster_apply(
            handle.clone(),
            app.state(),
            references,
            "aider".into(),
            project_path.clone(),
            "uninstall".into(),
            uninstall.revision,
            Some(true),
        )
        .await
        .unwrap();
        assert!(!Path::new(&installed.dest).exists());
        assert!(agent_rosters_reconcile(app.state())
            .await
            .unwrap()
            .is_empty());

        let reference = AgentReference {
            source_id: source.id.clone(),
            relative_path: "agent.md".into(),
        };
        let package = crate::agents::resolve_agent_package(app_data.path(), &reference)
            .await
            .unwrap();
        let raw = crate::agents::read_agent_text(app_data.path(), &reference)
            .await
            .unwrap();
        let antigravity = install_agent(
            handle.clone(),
            app.state(),
            Some(reference.source_id.clone()),
            Some(reference.relative_path.clone()),
            None,
            "antigravity".into(),
            Some(project_path.clone()),
            Some(true),
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&antigravity.dest).unwrap(),
            render::render(package.agent.as_ref().unwrap(), &raw, "antigravity").unwrap()
        );
        let reconciled =
            installs_reconcile(handle.clone(), app.state(), vec![project_path.clone()])
                .await
                .unwrap();
        assert_eq!(reconciled.len(), 1);
        assert_eq!(reconciled[0].state, InstallState::Current);
        uninstall_agent(
            handle,
            app.state(),
            Some(reference.source_id),
            Some(reference.relative_path),
            None,
            "antigravity".into(),
            Some(project_path),
        )
        .await
        .unwrap();
        let leaf = Path::new(&antigravity.dest);
        assert!(!leaf.exists());
        assert!(!leaf.parent().unwrap().exists());
        assert!(
            installs_reconcile(app.handle().clone(), app.state(), Vec::new())
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn roster_ledger_is_distinct_bounded_and_exactly_ordered() {
        let first = roster_record(2);
        let mut reversed = first.clone();
        reversed.members.reverse();
        assert!(validate_roster_record(&first).is_ok());
        assert!(validate_roster_record(&reversed).is_err());
        assert!(validate_roster_record(&roster_record(1)).is_err());
        assert!(validate_roster_record(&roster_record(MAX_ROSTER_MEMBERS + 1)).is_err());
        assert!(matches!(
            InstallLedgerEntry::Roster(first),
            InstallLedgerEntry::Roster(_)
        ));
    }

    #[test]
    fn replacing_agent_rows_preserves_distinct_roster_rows() {
        let roster = roster_record(2);
        let old = row("old", "codex", None);
        let replacement = row("new", "codex", None);
        let entries = vec![
            InstallLedgerEntry::Agent(old),
            InstallLedgerEntry::Roster(roster.clone()),
        ];

        let replaced = replace_agent_entries(entries, vec![replacement.clone()]);

        assert_eq!(replaced.len(), 2);
        assert!(
            matches!(&replaced[0], InstallLedgerEntry::Agent(record) if record.slug == replacement.slug)
        );
        assert!(matches!(&replaced[1], InstallLedgerEntry::Roster(record) if *record == roster));
    }

    #[tokio::test]
    async fn roster_plan_binds_exact_sorted_members_destination_and_foreign_refusal() {
        let app_data = tempfile::tempdir().unwrap();
        let source_root = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        std::fs::write(
            source_root.path().join("zeta.md"),
            "---\nname: Zeta\ndescription: Last.\n---\nZeta body.\n",
        )
        .unwrap();
        std::fs::write(
            source_root.path().join("alpha.md"),
            "---\nname: Alpha\ndescription: First.\n---\nAlpha body.\n",
        )
        .unwrap();
        let source = crate::agents::add_local_source(app_data.path(), source_root.path())
            .await
            .unwrap();
        let canonical_project = std::fs::canonicalize(project.path()).unwrap();
        save_registered_projects(app_data.path(), std::slice::from_ref(&canonical_project))
            .await
            .unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app_data.path().to_path_buf();
        let references = vec![
            AgentReference {
                source_id: source.id.clone(),
                relative_path: "zeta.md".into(),
            },
            AgentReference {
                source_id: source.id,
                relative_path: "alpha.md".into(),
            },
        ];

        let plan = build_roster_plan(
            &state,
            references,
            "aider".into(),
            canonical_project.to_string_lossy().into_owned(),
            "install",
        )
        .await
        .unwrap();

        assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
        assert_eq!(plan.members[0].reference.relative_path, "alpha.md");
        assert_eq!(plan.members[1].reference.relative_path, "zeta.md");
        assert_eq!(
            plan.destination,
            canonical_project.join("CONVENTIONS.md").to_string_lossy()
        );
        assert_eq!(plan.revision.len(), 64);
        assert!(
            !Path::new(&plan.destination).exists(),
            "planning must be read-only"
        );

        std::fs::write(&plan.destination, b"foreign project rules\n").unwrap();
        let blocked = build_roster_plan(
            &state,
            plan.members
                .iter()
                .map(|member| member.reference.clone())
                .collect(),
            "aider".into(),
            canonical_project.to_string_lossy().into_owned(),
            "install",
        )
        .await
        .unwrap();
        assert!(blocked
            .blockers
            .iter()
            .any(|blocker| blocker.contains("foreign")));
        assert_eq!(
            std::fs::read(&blocked.destination).unwrap(),
            b"foreign project rules\n"
        );

        let disabled = disabled_destination(Path::new(&plan.destination)).unwrap();
        let managed = b"managed disabled roster\n";
        std::fs::write(&disabled, managed).unwrap();
        let disabled_record = AgentRosterInstallRecord {
            tool: "aider".into(),
            scope: crate::types::Scope::Project,
            project_path: canonical_project.to_string_lossy().into_owned(),
            dest: plan.destination.clone(),
            members: plan.members.clone(),
            rendered_hash: render::sha256_hex(managed),
            disabled_path: Some(disabled.to_string_lossy().into_owned()),
            installed_at: "2026-08-17T00:00:00Z".into(),
        };
        save_rosters_for_state(&state, std::slice::from_ref(&disabled_record))
            .await
            .unwrap();
        let disabled_drift = build_roster_plan(
            &state,
            plan.members
                .iter()
                .map(|member| member.reference.clone())
                .collect(),
            "aider".into(),
            canonical_project.to_string_lossy().into_owned(),
            "update",
        )
        .await
        .unwrap();
        assert!(disabled_drift
            .blockers
            .iter()
            .any(|blocker| blocker.contains("Enable")));
        assert_eq!(
            std::fs::read(&disabled_drift.destination).unwrap(),
            b"foreign project rules\n"
        );
    }

    #[tokio::test]
    async fn roster_apply_writes_one_exact_aggregate_and_persists_distinct_truth() {
        let app_data = tempfile::tempdir().unwrap();
        let source_root = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        for (path, name, body) in [
            ("b.md", "Beta", "Beta body."),
            ("a.md", "Alpha", "Alpha body."),
        ] {
            std::fs::write(
                source_root.path().join(path),
                format!("---\nname: {name}\ndescription: {name} description.\n---\n{body}\n"),
            )
            .unwrap();
        }
        let source = crate::agents::add_local_source(app_data.path(), source_root.path())
            .await
            .unwrap();
        let project = std::fs::canonicalize(project.path()).unwrap();
        save_registered_projects(app_data.path(), std::slice::from_ref(&project))
            .await
            .unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app_data.path().to_path_buf();
        let references = ["b.md", "a.md"]
            .into_iter()
            .map(|relative_path| AgentReference {
                source_id: source.id.clone(),
                relative_path: relative_path.into(),
            })
            .collect();
        let plan = build_roster_plan(
            &state,
            references,
            "windsurf".into(),
            project.to_string_lossy().into_owned(),
            "install",
        )
        .await
        .unwrap();

        let installed = apply_roster_plan(&state, &plan, &plan.revision, true)
            .await
            .unwrap();

        let expected = render::render_roster(
            &[
                std::fs::read_to_string(source_root.path().join("a.md")).unwrap(),
                std::fs::read_to_string(source_root.path().join("b.md")).unwrap(),
            ],
            "windsurf",
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&plan.destination).unwrap(),
            expected
        );
        assert_eq!(installed.members, plan.members);
        assert_eq!(load_ledger_for_state(&state).await.unwrap().len(), 0);
        assert_eq!(
            load_rosters_for_state(&state).await.unwrap(),
            vec![installed]
        );
    }

    #[tokio::test]
    async fn roster_update_rejects_destination_bytes_changed_after_review() {
        let app_data = tempfile::tempdir().unwrap();
        let source_root = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        for path in ["agent.md", "reviewer.md"] {
            std::fs::write(
                source_root.path().join(path),
                format!("---\nname: {path}\ndescription: Test.\n---\nOriginal source.\n"),
            )
            .unwrap();
        }
        let source = crate::agents::add_local_source(app_data.path(), source_root.path())
            .await
            .unwrap();
        let project = std::fs::canonicalize(project.path()).unwrap();
        save_registered_projects(app_data.path(), std::slice::from_ref(&project))
            .await
            .unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app_data.path().to_path_buf();
        let references = ["agent.md", "reviewer.md"]
            .into_iter()
            .map(|relative_path| AgentReference {
                source_id: source.id.clone(),
                relative_path: relative_path.into(),
            })
            .collect::<Vec<_>>();
        let install = build_roster_plan(
            &state,
            references.clone(),
            "aider".into(),
            project.to_string_lossy().into_owned(),
            "install",
        )
        .await
        .unwrap();
        apply_roster_plan(&state, &install, &install.revision, true)
            .await
            .unwrap();

        std::fs::write(
            source_root.path().join("agent.md"),
            "---\nname: Agent\ndescription: Test.\n---\nUpdated source.\n",
        )
        .unwrap();
        std::fs::write(&install.destination, b"first foreign modification").unwrap();
        let reviewed = build_roster_plan(
            &state,
            references,
            "aider".into(),
            project.to_string_lossy().into_owned(),
            "update",
        )
        .await
        .unwrap();
        assert_eq!(reviewed.state, Some(InstallState::Modified));

        std::fs::write(&install.destination, b"second foreign modification").unwrap();
        let result = apply_roster_plan(&state, &reviewed, &reviewed.revision, true).await;

        assert!(
            result.is_err(),
            "changed destination must invalidate review"
        );
        assert_eq!(
            std::fs::read(&install.destination).unwrap(),
            b"second foreign modification"
        );
    }

    #[tokio::test]
    async fn roster_uninstall_rejects_destination_bytes_changed_after_review() {
        let app_data = tempfile::tempdir().unwrap();
        let source_root = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        for path in ["agent.md", "reviewer.md"] {
            std::fs::write(
                source_root.path().join(path),
                format!("---\nname: {path}\ndescription: Test.\n---\nSource.\n"),
            )
            .unwrap();
        }
        let source = crate::agents::add_local_source(app_data.path(), source_root.path())
            .await
            .unwrap();
        let project = std::fs::canonicalize(project.path()).unwrap();
        save_registered_projects(app_data.path(), std::slice::from_ref(&project))
            .await
            .unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app_data.path().to_path_buf();
        let references = ["agent.md", "reviewer.md"]
            .into_iter()
            .map(|relative_path| AgentReference {
                source_id: source.id.clone(),
                relative_path: relative_path.into(),
            })
            .collect::<Vec<_>>();
        let install = build_roster_plan(
            &state,
            references.clone(),
            "aider".into(),
            project.to_string_lossy().into_owned(),
            "install",
        )
        .await
        .unwrap();
        apply_roster_plan(&state, &install, &install.revision, true)
            .await
            .unwrap();
        std::fs::write(&install.destination, b"first foreign modification").unwrap();
        let reviewed = build_roster_plan(
            &state,
            references,
            "aider".into(),
            project.to_string_lossy().into_owned(),
            "uninstall",
        )
        .await
        .unwrap();
        assert_eq!(reviewed.state, Some(InstallState::Modified));

        std::fs::write(&install.destination, b"second foreign modification").unwrap();
        let result = uninstall_roster(&state, &reviewed, &reviewed.revision).await;

        assert!(
            result.is_err(),
            "changed destination must invalidate review"
        );
        assert_eq!(
            std::fs::read(&install.destination).unwrap(),
            b"second foreign modification"
        );
        assert_eq!(load_rosters_for_state(&state).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn roster_update_reconcile_disable_enable_uninstall_and_rollback_are_exact() {
        let app_data = tempfile::tempdir().unwrap();
        let source_root = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        for path in ["a.md", "b.md"] {
            std::fs::write(
                source_root.path().join(path),
                format!("---\nname: {path}\ndescription: test.\n---\nold {path}\n"),
            )
            .unwrap();
        }
        let source = crate::agents::add_local_source(app_data.path(), source_root.path())
            .await
            .unwrap();
        let project = std::fs::canonicalize(project.path()).unwrap();
        save_registered_projects(app_data.path(), std::slice::from_ref(&project))
            .await
            .unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app_data.path().to_path_buf();
        let refs = ["a.md", "b.md"]
            .into_iter()
            .map(|relative_path| AgentReference {
                source_id: source.id.clone(),
                relative_path: relative_path.into(),
            })
            .collect::<Vec<_>>();
        let install = build_roster_plan(
            &state,
            refs.clone(),
            "aider".into(),
            project.to_string_lossy().into_owned(),
            "install",
        )
        .await
        .unwrap();
        let first = apply_roster_plan(&state, &install, &install.revision, true)
            .await
            .unwrap();
        let old_bytes = std::fs::read(&first.dest).unwrap();

        std::fs::write(
            source_root.path().join("a.md"),
            "---\nname: a.md\ndescription: test.\n---\nnew a.md\n",
        )
        .unwrap();
        assert_eq!(
            current_roster_state(&state, &first).await.unwrap(),
            InstallState::Outdated
        );
        let update = build_roster_plan(
            &state,
            refs.clone(),
            "aider".into(),
            project.to_string_lossy().into_owned(),
            "update",
        )
        .await
        .unwrap();
        let second = apply_roster_plan(&state, &update, &update.revision, true)
            .await
            .unwrap();
        assert_ne!(std::fs::read(&second.dest).unwrap(), old_bytes);
        let history = roster_history(&state, "aider", project.to_str().unwrap())
            .await
            .unwrap();
        assert_eq!(history.len(), 1);

        let disabled = move_roster(&state, "aider", project.to_str().unwrap(), false)
            .await
            .unwrap();
        assert_eq!(
            current_roster_state(&state, &disabled).await.unwrap(),
            InstallState::Disabled
        );
        let enabled = move_roster(&state, "aider", project.to_str().unwrap(), true)
            .await
            .unwrap();
        assert_eq!(
            current_roster_state(&state, &enabled).await.unwrap(),
            InstallState::Current
        );

        let rolled_back =
            rollback_roster(&state, "aider", project.to_str().unwrap(), &history[0].id)
                .await
                .unwrap();
        assert_eq!(std::fs::read(&rolled_back.dest).unwrap(), old_bytes);
        assert_eq!(rolled_back.rendered_hash, first.rendered_hash);

        let mut mismatched = refs.clone();
        mismatched[0].relative_path = "different.md".into();
        let blocked_uninstall = build_roster_plan(
            &state,
            mismatched,
            "aider".into(),
            project.to_string_lossy().into_owned(),
            "uninstall",
        )
        .await
        .unwrap();
        assert!(blocked_uninstall
            .blockers
            .iter()
            .any(|blocker| blocker.contains("membership changed")));
        std::fs::remove_dir_all(source_root.path()).unwrap();
        let uninstall = build_roster_plan(
            &state,
            refs,
            "aider".into(),
            project.to_string_lossy().into_owned(),
            "uninstall",
        )
        .await
        .unwrap();
        assert!(uninstall.blockers.is_empty(), "{:?}", uninstall.blockers);
        uninstall_roster(&state, &uninstall, &uninstall.revision)
            .await
            .unwrap();
        assert!(!Path::new(&first.dest).exists());
        assert!(load_rosters_for_state(&state).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn roster_publish_never_clobbers_foreign_or_leaves_partial_update() {
        let project = tempfile::tempdir().unwrap();
        let destination = project.path().join("CONVENTIONS.md");
        std::fs::write(&destination, b"foreign").unwrap();
        assert!(publish_roster_bytes(&destination, b"managed", false)
            .await
            .is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"foreign");

        std::fs::write(&destination, b"prior-managed").unwrap();
        std::fs::create_dir(destination.with_extension("md.tmp")).unwrap();
        assert!(publish_roster_bytes(&destination, b"next-managed", true)
            .await
            .is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"prior-managed");
    }

    #[test]
    fn roster_recovery_rejects_legacy_payload_without_explicit_preservation_state() {
        let previous = roster_record(2);
        let legacy_write = serde_json::json!({
            "previous": previous,
            "next": roster_record(2),
            "stagedPath": "/legacy/staged.bin",
            "priorHash": render::sha256_hex(b"legacy"),
            "history": null
        });
        assert!(serde_json::from_value::<AgentRosterWriteOperation>(legacy_write).is_err());

        let legacy_uninstall = serde_json::json!({
            "previous": roster_record(2),
            "currentHash": render::sha256_hex(b"legacy"),
            "history": null
        });
        assert!(serde_json::from_value::<AgentRosterUninstallOperation>(legacy_uninstall).is_err());
    }

    #[test]
    fn roster_preimage_revalidation_preserves_late_external_edit() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("CONVENTIONS.md");
        std::fs::write(&destination, b"preserved managed bytes").unwrap();
        let expected = render::sha256_hex(b"preserved managed bytes");

        std::fs::write(&destination, b"late external edit").unwrap();

        assert!(verify_roster_preimage(&destination, Some(&expected)).is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"late external edit");
    }

    #[tokio::test]
    async fn recovery_without_database_sweeps_orphaned_roster_history_staging() {
        let app_data = tempfile::tempdir().unwrap();
        let orphan = app_data
            .path()
            .join("state/roster-history-staging")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(orphan.join("snapshot/content")).unwrap();
        std::fs::write(orphan.join("snapshot/content/0.bin"), b"orphan").unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app_data.path().to_path_buf();
        assert!(state.completed_state_database().await.unwrap().is_none());

        recover_agent_operations(&state).await.unwrap();

        assert!(!orphan.exists());
    }

    #[derive(Clone, Copy)]
    enum UnjournaledRosterCrashPoint {
        AfterPrepare,
        AfterFilesystem,
        AfterLedger,
        MixedState,
        TamperedIntent,
    }

    async fn assert_unjournaled_roster_operation_recovers_exactly(
        crash_point: UnjournaledRosterCrashPoint,
    ) {
        let app_data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let project = std::fs::canonicalize(project.path()).unwrap();
        let destination = project.join("CONVENTIONS.md");
        let previous_bytes = b"old roster";
        let next_bytes = b"new roster";
        std::fs::write(&destination, previous_bytes).unwrap();
        let mut previous = roster_record(2);
        previous.project_path = project.to_string_lossy().into_owned();
        previous.dest = destination.to_string_lossy().into_owned();
        previous.rendered_hash = render::sha256_hex(previous_bytes);
        let mut next = previous.clone();
        next.rendered_hash = render::sha256_hex(next_bytes);
        next.members[0].source_hash = "c".repeat(64);
        next.installed_at = "2026-08-17T01:00:00Z".into();
        let identity = roster_identity(&previous);
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app_data.path().to_path_buf();
        std::fs::create_dir_all(app_data.path().join("state")).unwrap();
        save_registered_projects(app_data.path(), std::slice::from_ref(&project))
            .await
            .unwrap();
        save_rosters_for_state(&state, std::slice::from_ref(&previous))
            .await
            .unwrap();
        assert!(state.completed_state_database().await.unwrap().is_none());
        for index in 0..MAX_AGENT_HISTORY_ENTRIES {
            history::create_roster_snapshot(
                app_data.path(),
                &identity,
                std::slice::from_ref(&destination),
                &previous,
                &format!("2026-08-17T00:00:{index:02}Z"),
                None,
            )
            .await
            .unwrap();
        }
        let history_before = history_tree_bytes(app_data.path());
        let mutation = history::begin_roster_snapshot(
            app_data.path(),
            &identity,
            std::slice::from_ref(&destination),
            &previous,
            "2026-08-17T02:00:00Z",
            None,
        )
        .await
        .unwrap();
        let staging = PathBuf::from(mutation.staging_path());
        mutation
            .prepare_unjournaled_operation(&previous, Some(&next))
            .await
            .unwrap();
        if matches!(
            crash_point,
            UnjournaledRosterCrashPoint::AfterFilesystem | UnjournaledRosterCrashPoint::AfterLedger
        ) {
            atomic_write(&destination, next_bytes).await.unwrap();
        }
        if matches!(crash_point, UnjournaledRosterCrashPoint::AfterLedger) {
            save_rosters_for_state(&state, std::slice::from_ref(&next))
                .await
                .unwrap();
        }
        if matches!(crash_point, UnjournaledRosterCrashPoint::MixedState) {
            atomic_write(&destination, b"foreign roster edit")
                .await
                .unwrap();
        }
        if matches!(crash_point, UnjournaledRosterCrashPoint::TamperedIntent) {
            let intent_path = staging.join("unjournaled-intent.json");
            let mut intent: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&intent_path).unwrap()).unwrap();
            intent["phase"] = serde_json::Value::String("committed".into());
            atomic_write(&intent_path, &serde_json::to_vec_pretty(&intent).unwrap())
                .await
                .unwrap();
        }
        drop(mutation);

        if matches!(
            crash_point,
            UnjournaledRosterCrashPoint::MixedState | UnjournaledRosterCrashPoint::TamperedIntent
        ) {
            assert!(recover_agent_operations(&state).await.is_err());
            assert!(recover_agent_operations(&state).await.is_err());
            let expected = if matches!(crash_point, UnjournaledRosterCrashPoint::MixedState) {
                b"foreign roster edit".as_slice()
            } else {
                previous_bytes.as_slice()
            };
            assert_eq!(std::fs::read(&destination).unwrap(), expected);
            assert_eq!(
                load_rosters_for_state(&state).await.unwrap(),
                vec![previous.clone()]
            );
            assert!(staging.exists());
            assert_eq!(history_tree_bytes(app_data.path()), history_before);
            return;
        }
        recover_agent_operations(&state).await.unwrap();
        recover_agent_operations(&state).await.unwrap();

        assert!(!staging.exists());
        let aborted = matches!(crash_point, UnjournaledRosterCrashPoint::AfterPrepare);
        let expected_record = if aborted { &previous } else { &next };
        let expected_bytes: &[u8] = if aborted { previous_bytes } else { next_bytes };
        assert_eq!(std::fs::read(&destination).unwrap(), expected_bytes);
        assert_eq!(
            load_rosters_for_state(&state).await.unwrap(),
            vec![expected_record.clone()]
        );
        let snapshots = history::list_snapshots(app_data.path(), &identity)
            .await
            .unwrap();
        assert_eq!(snapshots.len(), MAX_AGENT_HISTORY_ENTRIES);
        if aborted {
            assert_eq!(history_tree_bytes(app_data.path()), history_before);
        } else {
            assert_eq!(snapshots[0].roster_record.as_ref(), Some(&previous));
        }
        let indexed = snapshots
            .iter()
            .map(|snapshot| PathBuf::from(&snapshot.content_path))
            .collect::<std::collections::BTreeSet<_>>();
        let directory = history::history_directory(app_data.path(), &identity);
        let physical = std::fs::read_dir(directory)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .map(|entry| entry.path())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(physical, indexed);
    }

    #[tokio::test]
    async fn unjournaled_roster_crash_after_prepare_aborts_without_history() {
        assert_unjournaled_roster_operation_recovers_exactly(
            UnjournaledRosterCrashPoint::AfterPrepare,
        )
        .await;
    }

    #[tokio::test]
    async fn unjournaled_roster_crash_after_filesystem_finishes_ledger_and_history() {
        assert_unjournaled_roster_operation_recovers_exactly(
            UnjournaledRosterCrashPoint::AfterFilesystem,
        )
        .await;
    }

    #[tokio::test]
    async fn unjournaled_roster_crash_after_ledger_finishes_history_once() {
        assert_unjournaled_roster_operation_recovers_exactly(
            UnjournaledRosterCrashPoint::AfterLedger,
        )
        .await;
    }

    #[tokio::test]
    async fn unjournaled_roster_recovery_fails_closed_on_mixed_state() {
        assert_unjournaled_roster_operation_recovers_exactly(
            UnjournaledRosterCrashPoint::MixedState,
        )
        .await;
    }

    #[tokio::test]
    async fn unjournaled_roster_recovery_fails_closed_on_tampered_phase() {
        assert_unjournaled_roster_operation_recovers_exactly(
            UnjournaledRosterCrashPoint::TamperedIntent,
        )
        .await;
    }

    async fn assert_unjournaled_missing_roster_write_recovers(
        previous_exists: bool,
        crash_point: UnjournaledRosterCrashPoint,
    ) {
        let app_data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let project = std::fs::canonicalize(project.path()).unwrap();
        let destination = project.join("CONVENTIONS.md");
        let mut previous = roster_record(2);
        previous.project_path = project.to_string_lossy().into_owned();
        previous.dest = destination.to_string_lossy().into_owned();
        previous.rendered_hash = render::sha256_hex(b"missing old roster");
        let mut next = previous.clone();
        next.rendered_hash = render::sha256_hex(b"new roster");
        next.members[0].source_hash = "c".repeat(64);
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app_data.path().to_path_buf();
        std::fs::create_dir_all(app_data.path().join("state")).unwrap();
        save_registered_projects(app_data.path(), std::slice::from_ref(&project))
            .await
            .unwrap();
        if previous_exists {
            save_rosters_for_state(&state, std::slice::from_ref(&previous))
                .await
                .unwrap();
        }
        let staged = stage_roster_bytes(app_data.path(), b"new roster")
            .await
            .unwrap();
        prepare_unjournaled_roster_write(
            &state,
            true,
            previous_exists.then_some(&previous),
            &next,
            &staged,
        )
        .await
        .unwrap();
        let intent =
            unjournaled_roster_write_intent_path(app_data.path(), &staged.to_string_lossy())
                .unwrap();
        if matches!(
            crash_point,
            UnjournaledRosterCrashPoint::AfterFilesystem | UnjournaledRosterCrashPoint::AfterLedger
        ) {
            publish_roster_bytes(&destination, b"new roster", previous_exists)
                .await
                .unwrap();
            sync_parent_directory_entry(&destination).unwrap();
        }
        if matches!(crash_point, UnjournaledRosterCrashPoint::AfterLedger) {
            save_rosters_for_state(&state, std::slice::from_ref(&next))
                .await
                .unwrap();
        }

        recover_agent_operations(&state).await.unwrap();
        recover_agent_operations(&state).await.unwrap();

        assert!(!intent.exists());
        assert!(!staged.exists());
        if matches!(crash_point, UnjournaledRosterCrashPoint::AfterPrepare) {
            assert!(!destination.exists());
            let expected = if previous_exists {
                vec![previous]
            } else {
                Vec::new()
            };
            assert_eq!(load_rosters_for_state(&state).await.unwrap(), expected);
        } else {
            assert_eq!(std::fs::read(destination).unwrap(), b"new roster");
            assert_eq!(load_rosters_for_state(&state).await.unwrap(), vec![next]);
        }
    }

    #[tokio::test]
    async fn unjournaled_new_roster_crash_after_prepare_aborts_cleanly() {
        assert_unjournaled_missing_roster_write_recovers(
            false,
            UnjournaledRosterCrashPoint::AfterPrepare,
        )
        .await;
    }

    #[tokio::test]
    async fn unjournaled_new_roster_crash_after_filesystem_finishes_ledger() {
        assert_unjournaled_missing_roster_write_recovers(
            false,
            UnjournaledRosterCrashPoint::AfterFilesystem,
        )
        .await;
    }

    #[tokio::test]
    async fn unjournaled_missing_update_crash_after_ledger_finishes_once() {
        assert_unjournaled_missing_roster_write_recovers(
            true,
            UnjournaledRosterCrashPoint::AfterLedger,
        )
        .await;
    }

    #[tokio::test]
    async fn prepared_roster_write_recovery_rolls_forward_bytes_and_ledger_once() {
        let app_data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let project = std::fs::canonicalize(project.path()).unwrap();
        let destination = project.join("CONVENTIONS.md");
        std::fs::write(&destination, b"old roster").unwrap();
        let mut previous = roster_record(2);
        previous.project_path = project.to_string_lossy().into_owned();
        previous.dest = destination.to_string_lossy().into_owned();
        previous.rendered_hash = render::sha256_hex(b"old roster");
        let mut next = previous.clone();
        next.rendered_hash = render::sha256_hex(b"new roster");
        next.members[0].source_hash = "c".repeat(64);
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app_data.path().to_path_buf();
        let database = crate::state_db::StateDatabase::open(app_data.path()).unwrap();
        database
            .mutate(installs_spec(), Vec::new(), {
                let previous = previous.clone();
                move |entries| {
                    entries.push(InstallLedgerEntry::Roster(previous));
                    Ok(())
                }
            })
            .await
            .unwrap();
        database
            .mutate(projects_spec(), Vec::new(), {
                let project = project.clone();
                move |projects| {
                    projects.push(project);
                    Ok(())
                }
            })
            .await
            .unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        for index in 0..MAX_AGENT_HISTORY_ENTRIES {
            history::create_roster_snapshot(
                app_data.path(),
                &roster_identity(&previous),
                std::slice::from_ref(&destination),
                &previous,
                &format!("2026-08-17T00:00:{index:02}Z"),
                None,
            )
            .await
            .unwrap();
        }
        let history_directory =
            history::history_directory(app_data.path(), &roster_identity(&previous));
        let index_before_unjournaled_preservation =
            std::fs::read(history_directory.join("index.json")).unwrap();
        let physical_before_unjournaled_preservation = std::fs::read_dir(&history_directory)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .count();
        let staged = stage_roster_bytes(app_data.path(), b"new roster")
            .await
            .unwrap();
        let preservation = begin_roster_preservation(&state, &destination, &previous, None)
            .await
            .unwrap()
            .unwrap();
        let orphaned_staging = PathBuf::from(preservation.staging_path());
        drop(preservation);
        assert_eq!(
            std::fs::read(history_directory.join("index.json")).unwrap(),
            index_before_unjournaled_preservation
        );
        assert_eq!(
            std::fs::read_dir(&history_directory)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                .count(),
            physical_before_unjournaled_preservation
        );
        assert!(database
            .pending_filesystem_operations()
            .await
            .unwrap()
            .is_empty());
        recover_agent_operations(&state).await.unwrap();
        assert!(!orphaned_staging.exists());
        assert_eq!(
            history::list_snapshots(app_data.path(), &roster_identity(&previous))
                .await
                .unwrap()
                .len(),
            MAX_AGENT_HISTORY_ENTRIES
        );
        let preservation = begin_roster_preservation(&state, &destination, &previous, None)
            .await
            .unwrap()
            .unwrap();
        let history = roster_history_reference(&preservation);
        let preservation_id = history.snapshot_id.clone();
        let staged_snapshot = PathBuf::from(&history.staging_path).join("snapshot");
        drop(preservation);
        database
            .prepare_filesystem_operation(
                "agent_update",
                &AgentRosterWriteOperation {
                    previous: Some(previous.clone()),
                    next: next.clone(),
                    staged_path: staged.to_string_lossy().into_owned(),
                    preservation: AgentRosterPreservation::Snapshot(history),
                },
            )
            .await
            .unwrap();

        let hidden_snapshot = staged_snapshot.with_extension("crash-test-hidden");
        std::fs::rename(&staged_snapshot, &hidden_snapshot).unwrap();
        assert!(recover_agent_operations(&state).await.is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"old roster");
        assert_eq!(
            load_rosters_for_state(&state).await.unwrap(),
            vec![previous]
        );
        assert_eq!(
            database
                .pending_filesystem_operations()
                .await
                .unwrap()
                .len(),
            1
        );
        std::fs::rename(hidden_snapshot, staged_snapshot).unwrap();

        recover_agent_operations(&state).await.unwrap();
        recover_agent_operations(&state).await.unwrap();

        assert_eq!(std::fs::read(&destination).unwrap(), b"new roster");
        assert_eq!(
            load_rosters_for_state(&state).await.unwrap(),
            vec![next.clone()]
        );
        assert!(database
            .pending_filesystem_operations()
            .await
            .unwrap()
            .is_empty());
        assert!(!staged.exists());
        let (_, contents) = history::snapshot_contents(
            app_data.path(),
            &roster_identity(&next),
            &preservation_id,
            std::slice::from_ref(&destination),
        )
        .await
        .unwrap();
        assert_eq!(contents, vec![b"old roster".to_vec()]);
        assert_eq!(
            history::list_snapshots(app_data.path(), &roster_identity(&next))
                .await
                .unwrap()
                .len(),
            MAX_AGENT_HISTORY_ENTRIES
        );
        let physical_snapshots = std::fs::read_dir(history::history_directory(
            app_data.path(),
            &roster_identity(&next),
        ))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .count();
        assert_eq!(physical_snapshots, MAX_AGENT_HISTORY_ENTRIES);
    }

    #[tokio::test]
    async fn prepared_roster_retention_recovers_after_main_journal_commit() {
        let app_data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let project = std::fs::canonicalize(project.path()).unwrap();
        let destination = project.join("CONVENTIONS.md");
        std::fs::write(&destination, b"managed roster").unwrap();
        let mut previous = roster_record(2);
        previous.project_path = project.to_string_lossy().into_owned();
        previous.dest = destination.to_string_lossy().into_owned();
        previous.rendered_hash = render::sha256_hex(b"managed roster");
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app_data.path().to_path_buf();
        let database = crate::state_db::StateDatabase::open(app_data.path()).unwrap();
        database
            .mutate(projects_spec(), Vec::new(), {
                let project = project.clone();
                move |projects| {
                    projects.push(project);
                    Ok(())
                }
            })
            .await
            .unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        for index in 0..MAX_AGENT_HISTORY_ENTRIES {
            history::create_roster_snapshot(
                app_data.path(),
                &roster_identity(&previous),
                std::slice::from_ref(&destination),
                &previous,
                &format!("2026-08-17T01:00:{index:02}Z"),
                None,
            )
            .await
            .unwrap();
        }
        let mut mutation = begin_roster_preservation(&state, &destination, &previous, None)
            .await
            .unwrap()
            .unwrap();
        let history = roster_history_reference(&mutation);
        assert_eq!(history.retired_snapshot_ids.len(), 1);
        mutation.publish().await.unwrap();
        database
            .prepare_filesystem_operation(
                "agent_roster_retention",
                &AgentRosterRetentionOperation {
                    previous: previous.clone(),
                    history,
                },
            )
            .await
            .unwrap();
        drop(mutation);

        database
            .mutate(projects_spec(), Vec::new(), |projects| {
                projects.clear();
                Ok(())
            })
            .await
            .unwrap();

        let directory = history::history_directory(app_data.path(), &roster_identity(&previous));
        let physical_before = std::fs::read_dir(&directory)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .count();
        assert_eq!(physical_before, MAX_AGENT_HISTORY_ENTRIES + 1);

        recover_agent_operations(&state).await.unwrap();
        recover_agent_operations(&state).await.unwrap();

        let physical_after = std::fs::read_dir(directory)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .count();
        assert_eq!(physical_after, MAX_AGENT_HISTORY_ENTRIES);
        assert!(database
            .pending_filesystem_operations()
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn prepared_roster_move_and_uninstall_recovery_are_idempotent() {
        let app_data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let project = std::fs::canonicalize(project.path()).unwrap();
        let active = project.join("CONVENTIONS.md");
        let disabled = disabled_destination(&active).unwrap();
        std::fs::write(&active, b"managed roster").unwrap();
        let mut previous = roster_record(2);
        previous.project_path = project.to_string_lossy().into_owned();
        previous.dest = active.to_string_lossy().into_owned();
        previous.rendered_hash = render::sha256_hex(b"managed roster");
        let mut disabled_record = previous.clone();
        disabled_record.disabled_path = Some(disabled.to_string_lossy().into_owned());
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app_data.path().to_path_buf();
        let database = crate::state_db::StateDatabase::open(app_data.path()).unwrap();
        database
            .mutate(installs_spec(), Vec::new(), {
                let previous = previous.clone();
                move |entries| {
                    entries.push(InstallLedgerEntry::Roster(previous));
                    Ok(())
                }
            })
            .await
            .unwrap();
        database
            .mutate(projects_spec(), Vec::new(), {
                let project = project.clone();
                move |projects| {
                    projects.push(project);
                    Ok(())
                }
            })
            .await
            .unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let preservation = begin_roster_preservation(&state, &active, &previous, None)
            .await
            .unwrap()
            .unwrap();
        let history = roster_history_reference(&preservation);
        drop(preservation);
        database
            .prepare_filesystem_operation(
                "agent_disable",
                &AgentRosterMoveOperation {
                    previous: previous.clone(),
                    next: disabled_record.clone(),
                    history,
                },
            )
            .await
            .unwrap();

        recover_agent_operations(&state).await.unwrap();
        recover_agent_operations(&state).await.unwrap();
        assert!(!active.exists());
        assert_eq!(std::fs::read(&disabled).unwrap(), b"managed roster");
        assert_eq!(
            load_rosters_for_state(&state).await.unwrap(),
            vec![disabled_record.clone()]
        );

        let preservation = begin_roster_preservation(&state, &disabled, &disabled_record, None)
            .await
            .unwrap()
            .unwrap();
        let history = roster_history_reference(&preservation);
        drop(preservation);
        database
            .prepare_filesystem_operation(
                "agent_uninstall",
                &AgentRosterUninstallOperation {
                    previous: disabled_record,
                    preservation: AgentRosterPreservation::Snapshot(history),
                },
            )
            .await
            .unwrap();
        recover_agent_operations(&state).await.unwrap();
        recover_agent_operations(&state).await.unwrap();
        assert!(!active.exists());
        assert!(!disabled.exists());
        assert!(load_rosters_for_state(&state).await.unwrap().is_empty());
        assert!(database
            .pending_filesystem_operations()
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn roster_recovery_rejects_unregistered_move_and_uninstall_paths_without_mutation() {
        for kind in ["agent_disable", "agent_uninstall"] {
            let app_data = tempfile::tempdir().unwrap();
            let unregistered = tempfile::tempdir().unwrap();
            let project = std::fs::canonicalize(unregistered.path()).unwrap();
            let active = project.join("CONVENTIONS.md");
            let disabled = disabled_destination(&active).unwrap();
            std::fs::write(&active, b"unregistered roster").unwrap();
            let mut previous = roster_record(2);
            previous.project_path = project.to_string_lossy().into_owned();
            previous.dest = active.to_string_lossy().into_owned();
            previous.rendered_hash = render::sha256_hex(b"unregistered roster");
            let mut state = AppState::build().unwrap();
            state.app_data_dir = app_data.path().to_path_buf();
            let database = crate::state_db::StateDatabase::open(app_data.path()).unwrap();
            database
                .mutate(installs_spec(), Vec::new(), {
                    let previous = previous.clone();
                    move |entries| {
                        entries.push(InstallLedgerEntry::Roster(previous));
                        Ok(())
                    }
                })
                .await
                .unwrap();
            database
                .mutate(projects_spec(), Vec::new(), |_| Ok(()))
                .await
                .unwrap();
            database
                .set_migration_state(crate::types::StorageMigrationState::Complete)
                .await
                .unwrap();
            if kind == "agent_disable" {
                let mut next = previous.clone();
                next.disabled_path = Some(disabled.to_string_lossy().into_owned());
                database
                    .prepare_filesystem_operation(
                        kind,
                        &AgentRosterMoveOperation {
                            previous: previous.clone(),
                            next,
                            history: AgentRosterHistoryReference {
                                snapshot_id: "unregistered".into(),
                                rendered_hash: previous.rendered_hash.clone(),
                                retired_snapshot_ids: Vec::new(),
                                staging_path: "/unregistered".into(),
                                previous_index_hash: None,
                                next_index_hash: "0".repeat(64),
                            },
                        },
                    )
                    .await
                    .unwrap();
            } else {
                database
                    .prepare_filesystem_operation(
                        kind,
                        &AgentRosterUninstallOperation {
                            previous: previous.clone(),
                            preservation: AgentRosterPreservation::Snapshot(
                                AgentRosterHistoryReference {
                                    snapshot_id: "unregistered".into(),
                                    rendered_hash: previous.rendered_hash.clone(),
                                    retired_snapshot_ids: Vec::new(),
                                    staging_path: "/unregistered".into(),
                                    previous_index_hash: None,
                                    next_index_hash: "0".repeat(64),
                                },
                            ),
                        },
                    )
                    .await
                    .unwrap();
            }

            assert!(recover_agent_operations(&state).await.is_err());
            assert_eq!(std::fs::read(&active).unwrap(), b"unregistered roster");
            assert!(!disabled.exists());
            assert_eq!(
                load_rosters_for_state(&state).await.unwrap(),
                vec![previous]
            );
            let pending = database.pending_filesystem_operations().await.unwrap();
            assert_eq!(pending.len(), 1);
            assert!(pending[0].recovery_error.is_some());
        }
    }

    #[tokio::test]
    async fn roster_sqlite_ledger_failure_restores_file_row_and_journal() {
        let app_data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let project = std::fs::canonicalize(project.path()).unwrap();
        let active = project.join("CONVENTIONS.md");
        let disabled = disabled_destination(&active).unwrap();
        std::fs::write(&active, b"managed roster").unwrap();
        let mut previous = roster_record(2);
        previous.project_path = project.to_string_lossy().into_owned();
        previous.dest = active.to_string_lossy().into_owned();
        previous.rendered_hash = render::sha256_hex(b"managed roster");
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app_data.path().to_path_buf();
        let database = crate::state_db::StateDatabase::open(app_data.path()).unwrap();
        database
            .mutate(installs_spec(), Vec::new(), {
                let previous = previous.clone();
                move |entries| {
                    entries.push(InstallLedgerEntry::Roster(previous));
                    Ok(())
                }
            })
            .await
            .unwrap();
        database
            .mutate(projects_spec(), Vec::new(), {
                let project = project.clone();
                move |projects| {
                    projects.push(project);
                    Ok(())
                }
            })
            .await
            .unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let connection = inject_agent_ledger_failure(app_data.path());
        let history_before = history_tree_bytes(app_data.path());

        assert!(
            move_roster(&state, "aider", project.to_str().unwrap(), false)
                .await
                .is_err()
        );

        drop(connection);
        assert_eq!(std::fs::read(&active).unwrap(), b"managed roster");
        assert!(!disabled.exists());
        assert_eq!(
            load_rosters_for_state(&state).await.unwrap(),
            vec![previous]
        );
        assert!(database
            .pending_filesystem_operations()
            .await
            .unwrap()
            .is_empty());
        assert_eq!(history_tree_bytes(app_data.path()), history_before);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn roster_filesystem_failures_abort_prepared_journals_without_changing_truth() {
        use std::os::unix::fs::PermissionsExt;

        // Update: the reviewed replacement cannot create its sibling temp file.
        let (app, source_root, project, database, state, references, record, bytes) =
            completed_sqlite_roster_install().await;
        let history_before = history_tree_bytes(app.path());
        std::fs::write(
            source_root.path().join("agent.md"),
            "---\nname: agent.md\ndescription: Test.\n---\nUpdated.\n",
        )
        .unwrap();
        let plan = build_roster_plan(
            &state,
            references,
            "aider".into(),
            std::fs::canonicalize(project.path())
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            "update",
        )
        .await
        .unwrap();
        std::fs::set_permissions(project.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        let result = apply_roster_plan(&state, &plan, &plan.revision, true).await;
        std::fs::set_permissions(project.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
        assert_exact_roster_state(&state, &database, &record, &bytes, 0).await;
        assert_eq!(history_tree_bytes(app.path()), history_before);

        // Move: rename requires write permission on the project directory.
        std::fs::set_permissions(project.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        let result = move_roster(&state, "aider", &record.project_path, false).await;
        std::fs::set_permissions(project.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
        assert_exact_roster_state(&state, &database, &record, &bytes, 0).await;
        assert_eq!(history_tree_bytes(app.path()), history_before);

        // Uninstall: reads remain available but directory mutation is denied.
        let plan = build_roster_plan(
            &state,
            record
                .members
                .iter()
                .map(|member| member.reference.clone())
                .collect(),
            "aider".into(),
            record.project_path.clone(),
            "uninstall",
        )
        .await
        .unwrap();
        std::fs::set_permissions(project.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        let result = uninstall_roster(&state, &plan, &plan.revision).await;
        std::fs::set_permissions(project.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
        assert_exact_roster_state(&state, &database, &record, &bytes, 0).await;
        assert_eq!(history_tree_bytes(app.path()), history_before);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn roster_install_filesystem_and_ledger_failures_abort_cleanly() {
        use std::os::unix::fs::PermissionsExt;

        let (app, _source, _project, database, state, references, record, bytes) =
            completed_sqlite_roster_install().await;
        let history_before = history_tree_bytes(app.path());
        let filesystem_project = tempfile::tempdir().unwrap();
        let ledger_project = tempfile::tempdir().unwrap();
        let filesystem_project_path = std::fs::canonicalize(filesystem_project.path()).unwrap();
        let ledger_project_path = std::fs::canonicalize(ledger_project.path()).unwrap();
        database
            .mutate(projects_spec(), Vec::new(), {
                let additions = vec![filesystem_project_path.clone(), ledger_project_path.clone()];
                move |projects| {
                    projects.extend(additions);
                    projects.sort();
                    projects.dedup();
                    Ok(())
                }
            })
            .await
            .unwrap();

        let plan = build_roster_plan(
            &state,
            references.clone(),
            "aider".into(),
            filesystem_project_path.to_string_lossy().into_owned(),
            "install",
        )
        .await
        .unwrap();
        std::fs::set_permissions(
            filesystem_project.path(),
            std::fs::Permissions::from_mode(0o500),
        )
        .unwrap();
        let result = apply_roster_plan(&state, &plan, &plan.revision, true).await;
        std::fs::set_permissions(
            filesystem_project.path(),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        assert!(result.is_err());
        assert!(!Path::new(&plan.destination).exists());
        assert_exact_roster_state(&state, &database, &record, &bytes, 0).await;
        assert_eq!(history_tree_bytes(app.path()), history_before);

        let plan = build_roster_plan(
            &state,
            references,
            "aider".into(),
            ledger_project_path.to_string_lossy().into_owned(),
            "install",
        )
        .await
        .unwrap();
        let connection = inject_agent_ledger_failure(app.path());
        let result = apply_roster_plan(&state, &plan, &plan.revision, true).await;
        connection
            .execute_batch("DROP TRIGGER fail_agent_lifecycle_ledger;")
            .unwrap();
        assert!(result.is_err());
        assert!(!Path::new(&plan.destination).exists());
        assert_exact_roster_state(&state, &database, &record, &bytes, 0).await;
        assert_eq!(history_tree_bytes(app.path()), history_before);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn roster_rollback_filesystem_and_ledger_failures_restore_exact_truth() {
        use std::os::unix::fs::PermissionsExt;

        let (app, source_root, project, database, state, references, _record, _bytes) =
            completed_sqlite_roster_install().await;
        std::fs::write(
            source_root.path().join("agent.md"),
            "---\nname: agent.md\ndescription: Test.\n---\nUpdated.\n",
        )
        .unwrap();
        let update = build_roster_plan(
            &state,
            references,
            "aider".into(),
            std::fs::canonicalize(project.path())
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            "update",
        )
        .await
        .unwrap();
        let updated = apply_roster_plan(&state, &update, &update.revision, true)
            .await
            .unwrap();
        let updated_bytes = std::fs::read(&updated.dest).unwrap();
        let snapshot = roster_history(&state, "aider", &updated.project_path)
            .await
            .unwrap()
            .remove(0);
        let history_before = history_tree_bytes(app.path());

        std::fs::set_permissions(project.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        let result = rollback_roster(&state, "aider", &updated.project_path, &snapshot.id).await;
        std::fs::set_permissions(project.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
        assert_exact_roster_state(&state, &database, &updated, &updated_bytes, 0).await;
        assert_eq!(history_tree_bytes(app.path()), history_before);

        let connection = inject_agent_ledger_failure(app.path());
        let result = rollback_roster(&state, "aider", &updated.project_path, &snapshot.id).await;
        connection
            .execute_batch("DROP TRIGGER fail_agent_lifecycle_ledger;")
            .unwrap();
        assert!(result.is_err());
        assert_exact_roster_state(&state, &database, &updated, &updated_bytes, 0).await;
        assert_eq!(history_tree_bytes(app.path()), history_before);
    }

    #[tokio::test]
    async fn roster_update_and_uninstall_ledger_failures_restore_exact_truth() {
        let (app, source_root, _project, database, state, references, record, bytes) =
            completed_sqlite_roster_install().await;
        let history_before = history_tree_bytes(app.path());
        std::fs::write(
            source_root.path().join("agent.md"),
            "---\nname: agent.md\ndescription: Test.\n---\nUpdated.\n",
        )
        .unwrap();
        let update = build_roster_plan(
            &state,
            references.clone(),
            "aider".into(),
            record.project_path.clone(),
            "update",
        )
        .await
        .unwrap();
        let connection = inject_agent_ledger_failure(app.path());
        let result = apply_roster_plan(&state, &update, &update.revision, true).await;
        connection
            .execute_batch("DROP TRIGGER fail_agent_lifecycle_ledger;")
            .unwrap();
        assert!(result.is_err());
        assert_exact_roster_state(&state, &database, &record, &bytes, 0).await;
        assert_eq!(history_tree_bytes(app.path()), history_before);

        let uninstall = build_roster_plan(
            &state,
            references,
            "aider".into(),
            record.project_path.clone(),
            "uninstall",
        )
        .await
        .unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_agent_lifecycle_ledger BEFORE UPDATE ON state_documents \
                 WHEN NEW.name = 'installs' BEGIN SELECT RAISE(FAIL, 'injected Agent ledger failure'); END;",
            )
            .unwrap();
        let result = uninstall_roster(&state, &uninstall, &uninstall.revision).await;
        connection
            .execute_batch("DROP TRIGGER fail_agent_lifecycle_ledger;")
            .unwrap();
        assert!(result.is_err());
        assert_exact_roster_state(&state, &database, &record, &bytes, 0).await;
        assert_eq!(history_tree_bytes(app.path()), history_before);
    }

    #[tokio::test]
    async fn roster_commit_failure_compensates_and_recovery_never_silently_completes() {
        let (app, _source, _project, database, state, _references, record, bytes) =
            completed_sqlite_roster_install().await;
        let history_before = history_tree_bytes(app.path());
        let connection =
            rusqlite::Connection::open(app.path().join("state/agency-agents.sqlite3")).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_roster_journal_commit \
                 BEFORE UPDATE ON filesystem_operations \
                 WHEN OLD.phase = 'filesystem_applied' AND NEW.phase = 'committed' \
                 BEGIN SELECT RAISE(FAIL, 'injected roster journal commit failure'); END;",
            )
            .unwrap();

        let result = move_roster(&state, "aider", &record.project_path, false).await;
        assert!(result.is_err());
        assert_exact_roster_state(&state, &database, &record, &bytes, 1).await;
        assert_eq!(history_tree_bytes(app.path()), history_before);
        let pending = database.pending_filesystem_operations().await.unwrap();
        assert_eq!(
            pending[0].phase,
            crate::state_db::FilesystemOperationPhase::FilesystemApplied
        );
        assert!(pending[0].recovery_error.is_some());
        connection
            .execute_batch("DROP TRIGGER fail_roster_journal_commit;")
            .unwrap();

        assert!(recover_agent_operations(&state).await.is_err());
        assert_exact_roster_state(&state, &database, &record, &bytes, 1).await;
        assert!(recover_agent_operations(&state).await.is_err());
        assert_exact_roster_state(&state, &database, &record, &bytes, 1).await;
    }

    #[tokio::test]
    async fn passive_ledger_read_rejects_rows_that_require_migration_without_rewrite() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("installs.json");
        std::fs::write(&path, br#"[{"slug":"legacy","sourceId":"","relativePath":"","tool":"claudeCode","scope":"user","projectPath":null,"dest":"/tmp/legacy.md","sourceHash":"a","bodyHash":"","renderedHash":"b","disabledPath":null,"sourceSnapshotHash":"","capabilities":[],"publisherKey":null,"publisherVerified":false,"installedAt":"now","corpusVersion":"old"}]"#).unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(load_ledger_read_only_at(&path).await.is_err());
        assert_eq!(std::fs::read(path).unwrap(), before);
    }

    #[tokio::test]
    async fn reveal_app_data_target_reaches_opener_spec_without_launching_gui() {
        let app = tempfile::tempdir().unwrap();
        let target = app.path().join("state/installs.json");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"[]").unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app.path().to_path_buf();
        let roots = reveal_allowed_roots(&state).await.unwrap();
        let mut recorded = None;

        reveal_path_from_roots_with_executor(
            target.to_string_lossy().into_owned(),
            roots,
            RevealPlatform::MacOs,
            |spec| {
                recorded = Some(spec);
                Ok(true)
            },
        )
        .unwrap();

        let spec = recorded.expect("validated target must reach the recording executor");
        assert_eq!(spec.program, std::ffi::OsString::from("/usr/bin/open"));
        assert_eq!(
            spec.args,
            vec![
                std::ffi::OsString::from("-R"),
                std::fs::canonicalize(target).unwrap().into_os_string(),
            ]
        );
    }

    #[tokio::test]
    async fn reveal_unrelated_target_is_rejected_before_executor() {
        let app = tempfile::tempdir().unwrap();
        let unrelated = tempfile::tempdir().unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app.path().to_path_buf();
        let roots = reveal_allowed_roots(&state).await.unwrap();
        let mut calls = 0;

        let result = reveal_path_from_roots_with_executor(
            unrelated.path().to_string_lossy().into_owned(),
            roots,
            RevealPlatform::MacOs,
            |_| {
                calls += 1;
                Ok(true)
            },
        );

        assert!(matches!(result, Err(AppError::InvalidArgument { .. })));
        assert_eq!(calls, 0, "rejected input must not reach the executor");
    }

    #[tokio::test]
    async fn reveal_roots_come_from_backend_install_state() {
        use crate::commands::settings::{Settings, SettingsLoadState};

        let app = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let custom_tool_home = tempfile::tempdir().unwrap();
        let registered_project = tempfile::tempdir().unwrap();
        let agent_project = tempfile::tempdir().unwrap();
        let skill_project = tempfile::tempdir().unwrap();
        for path in [
            custom_tool_home.path().join(".claude/agents"),
            home.path().join(".claude/skills"),
            home.path().join(".agents/skills"),
            registered_project.path().join(".claude/skills"),
            registered_project.path().join(".agents/skills"),
            agent_project.path().join(".claude/skills"),
            skill_project.path().join(".agents/skills"),
        ] {
            std::fs::create_dir_all(path).unwrap();
        }
        std::fs::create_dir_all(app.path().join("state")).unwrap();
        std::fs::write(
            app.path().join("state/projects.json"),
            serde_json::to_vec(&vec![registered_project.path()]).unwrap(),
        )
        .unwrap();
        std::fs::write(
            app.path().join("state/installs.json"),
            serde_json::to_vec(&vec![row(
                "agent",
                "claudeCode",
                Some(agent_project.path().to_str().unwrap()),
            )])
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            app.path().join("state/skill-installs.json"),
            serde_json::to_vec(&vec![crate::types::SkillInstallRecord {
                source_id: "builtin:skills".into(),
                relative_path: "reviewer/SKILL.md".into(),
                name: "reviewer".into(),
                runtime: "codex".into(),
                scope: "project".into(),
                project_path: Some(skill_project.path().to_string_lossy().into_owned()),
                dest: skill_project
                    .path()
                    .join(".agents/skills/reviewer")
                    .to_string_lossy()
                    .into_owned(),
                source_hash: "a".repeat(64),
                installed_hash: "b".repeat(64),
                installed_at: "2026-08-12T00:00:00Z".into(),
                disabled_path: None,
            }])
            .unwrap(),
        )
        .unwrap();

        let mut state = AppState::build().unwrap();
        state.app_data_dir = app.path().to_path_buf();
        let mut settings = Settings::default();
        settings.tool_paths.insert(
            "claudeCode".into(),
            custom_tool_home.path().to_string_lossy().into_owned(),
        );
        *state.settings.write().await = SettingsLoadState::Loaded(settings);

        let roots = reveal_allowed_roots_for_home(&state, home.path())
            .await
            .unwrap();
        for expected in [
            app.path().to_path_buf(),
            custom_tool_home.path().join(".claude/agents"),
            home.path().join(".claude/skills"),
            home.path().join(".agents/skills"),
            registered_project.path().to_path_buf(),
            registered_project.path().join(".claude/skills"),
            registered_project.path().join(".agents/skills"),
            agent_project.path().to_path_buf(),
            skill_project.path().to_path_buf(),
        ] {
            let expected = std::fs::canonicalize(expected).unwrap();
            assert!(
                roots.contains(&expected),
                "missing root {}",
                expected.display()
            );
        }
    }

    #[test]
    fn reveal_rejects_forbidden_paths_before_executor() {
        let allowed = tempfile::tempdir().unwrap();
        let unrelated = tempfile::tempdir().unwrap();
        let target = allowed.path().join("target.txt");
        std::fs::write(&target, b"target").unwrap();
        let sibling = allowed.path().parent().unwrap().join(format!(
            "{}-other",
            allowed.path().file_name().unwrap().to_string_lossy()
        ));
        std::fs::create_dir_all(&sibling).unwrap();
        let roots = vec![std::fs::canonicalize(allowed.path()).unwrap()];
        let missing = allowed.path().join("missing");
        let dot = allowed.path().join("./target.txt");
        let parent = allowed.path().join("child/../target.txt");
        let cases = vec![
            String::new(),
            "https://example.com".into(),
            "file:///tmp/example".into(),
            "relative/path".into(),
            missing.to_string_lossy().into_owned(),
            unrelated.path().to_string_lossy().into_owned(),
            sibling.to_string_lossy().into_owned(),
            dot.to_string_lossy().into_owned(),
            parent.to_string_lossy().into_owned(),
        ];
        for path in cases {
            let mut calls = 0;
            let result = reveal_path_from_roots_with_executor(
                path.clone(),
                roots.clone(),
                RevealPlatform::MacOs,
                |_| {
                    calls += 1;
                    Ok(true)
                },
            );
            assert!(
                matches!(result, Err(AppError::InvalidArgument { .. })),
                "unexpected authorization for {path:?}"
            );
            assert_eq!(calls, 0, "rejected {path:?} reached executor");
        }
        let _ = std::fs::remove_dir(sibling);
    }

    #[cfg(unix)]
    #[test]
    fn reveal_allows_contained_symlink_and_rejects_escape() {
        use std::os::unix::fs::symlink;

        let allowed = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let inside_target = allowed.path().join("inside.txt");
        let outside_target = outside.path().join("outside.txt");
        std::fs::write(&inside_target, b"inside").unwrap();
        std::fs::write(&outside_target, b"outside").unwrap();
        let inside_link = allowed.path().join("inside-link");
        let escape_link = allowed.path().join("escape-link");
        symlink(&inside_target, &inside_link).unwrap();
        symlink(&outside_target, &escape_link).unwrap();
        let roots = vec![std::fs::canonicalize(allowed.path()).unwrap()];
        let mut calls = 0;

        reveal_path_from_roots_with_executor(
            inside_link.to_string_lossy().into_owned(),
            roots.clone(),
            RevealPlatform::MacOs,
            |_| {
                calls += 1;
                Ok(true)
            },
        )
        .unwrap();
        assert_eq!(calls, 1);

        let result = reveal_path_from_roots_with_executor(
            escape_link.to_string_lossy().into_owned(),
            roots,
            RevealPlatform::MacOs,
            |_| {
                calls += 1;
                Ok(true)
            },
        );
        assert!(matches!(result, Err(AppError::InvalidArgument { .. })));
        assert_eq!(calls, 1, "symlink escape reached executor");
    }

    #[test]
    fn reveal_opener_specs_are_pure_and_platform_specific() {
        let directory = Path::new("/allowed/directory");
        let file = directory.join("file.txt");
        assert_eq!(
            reveal_opener_spec(&file, false, RevealPlatform::MacOs),
            RevealOpenerSpec {
                program: OsString::from("/usr/bin/open"),
                args: vec![OsString::from("-R"), file.as_os_str().to_owned()],
            }
        );
        assert_eq!(
            reveal_opener_spec(&file, false, RevealPlatform::Windows),
            RevealOpenerSpec {
                program: OsString::from("explorer"),
                args: vec![OsString::from("/select,"), file.as_os_str().to_owned()],
            }
        );
        assert_eq!(
            reveal_opener_spec(&file, false, RevealPlatform::Linux),
            RevealOpenerSpec {
                program: OsString::from("xdg-open"),
                args: vec![directory.as_os_str().to_owned()],
            }
        );
        assert_eq!(
            reveal_opener_spec(directory, true, RevealPlatform::Linux).args,
            vec![directory.as_os_str().to_owned()]
        );
    }

    #[test]
    fn reveal_nonzero_status_is_io_error() {
        let allowed = tempfile::tempdir().unwrap();
        let roots = vec![std::fs::canonicalize(allowed.path()).unwrap()];
        let result = reveal_path_from_roots_with_executor(
            allowed.path().to_string_lossy().into_owned(),
            roots,
            RevealPlatform::MacOs,
            |_| Ok(false),
        );
        assert!(matches!(result, Err(AppError::Io { .. })));
    }

    #[test]
    fn agentfile_roundtrips() {
        let af = Agentfile {
            agentfile: 1,
            installs: vec![
                LoadoutEntry {
                    slug: "a".into(),
                    tool: "claudeCode".to_string(),
                    project_path: None,
                },
                LoadoutEntry {
                    slug: "b".into(),
                    tool: "cursor".to_string(),
                    project_path: Some("/proj".into()),
                },
            ],
        };
        let bytes = serde_json::to_vec(&af).unwrap();
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("\"claudeCode\"") && s.contains("\"projectPath\":\"/proj\""));
        let back: Agentfile = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back.installs.len(), 2);
        assert_eq!(back.installs[1].tool, "cursor");
    }

    fn workspace_pack_fixture() -> WorkspacePack {
        WorkspacePack {
            workspace_pack: 1,
            name: "Review workspace".into(),
            scope: WorkspacePackScope::Project,
            agents: vec![
                WorkspacePackAgent {
                    reference: AgentReference {
                        source_id: "source-b".into(),
                        relative_path: "nested/writer.md".into(),
                    },
                    tool: "claudeCode".into(),
                },
                WorkspacePackAgent {
                    reference: AgentReference {
                        source_id: "source-a".into(),
                        relative_path: "engineering/reviewer.md".into(),
                    },
                    tool: "codex".into(),
                },
            ],
            skills: vec![WorkspacePackSkill {
                reference: crate::types::SkillReference {
                    source_id: "skills".into(),
                    relative_path: "audit".into(),
                },
                runtime: "codex".into(),
            }],
            runbook: Some("startup-mvp".into()),
            instructions: vec!["AGENTS.md conventions".into()],
            mcp_servers: vec!["memory".into()],
        }
    }

    fn recommendation_install(
        reference: &AgentReference,
        tool: &str,
        state: InstallState,
    ) -> InstalledAgent {
        InstalledAgent {
            slug: reference
                .relative_path
                .trim_end_matches(".md")
                .rsplit('/')
                .next()
                .unwrap_or("agent")
                .into(),
            name: reference.relative_path.clone(),
            source_id: reference.source_id.clone(),
            relative_path: reference.relative_path.clone(),
            tool: tool.into(),
            scope: crate::types::Scope::Project,
            project_path: Some("/registered/project".into()),
            dest: format!("/registered/project/.agents/{tool}.md"),
            state,
            update_kind: None,
            tracked: true,
        }
    }

    fn recommendation_baseline(
        reference: &AgentReference,
        tools: &[&str],
    ) -> ProjectReadinessBaseline {
        ProjectReadinessBaseline {
            project_path: "/registered/project".into(),
            label: "Reviewers".into(),
            agent_requirements: tools
                .iter()
                .map(|tool| BaselineAgentRequirement {
                    reference: reference.clone(),
                    tool: (*tool).into(),
                })
                .collect(),
            skill_requirements: Vec::new(),
            agents: vec![reference.clone()],
            skills: Vec::new(),
            instructions: Vec::new(),
            mcp_servers: Vec::new(),
            tools: tools.iter().map(|tool| (*tool).into()).collect(),
        }
    }

    fn recommendation_subscription() -> ProjectSubscription {
        ProjectSubscription {
            project_path: "/registered/project".into(),
            last_seen_batch: None,
            pending_recommendation_ids: Vec::new(),
            dismissed_recommendation_ids: Vec::new(),
        }
    }

    fn updated_batch(reference: &AgentReference) -> Vec<CatalogFeedBatch> {
        vec![CatalogFeedBatch {
            at: "2026-08-17T01:00:00Z".into(),
            changes: vec![CatalogChange::Updated {
                before: CatalogSnapshotItem {
                    category: "engineering".into(),
                    relative_path: reference.relative_path.clone(),
                    source_hash: "a".repeat(64),
                    body_hash: "b".repeat(64),
                },
                after: CatalogSnapshotItem {
                    category: "engineering".into(),
                    relative_path: reference.relative_path.clone(),
                    source_hash: "c".repeat(64),
                    body_hash: "d".repeat(64),
                },
            }],
        }]
    }

    fn renamed_batch(old: &AgentReference, new: &AgentReference) -> Vec<CatalogFeedBatch> {
        vec![CatalogFeedBatch {
            at: "2026-08-17T01:00:00Z".into(),
            changes: vec![CatalogChange::Renamed {
                before: CatalogSnapshotItem {
                    category: "engineering".into(),
                    relative_path: old.relative_path.clone(),
                    source_hash: "a".repeat(64),
                    body_hash: "b".repeat(64),
                },
                after: CatalogSnapshotItem {
                    category: "engineering".into(),
                    relative_path: new.relative_path.clone(),
                    source_hash: "a".repeat(64),
                    body_hash: "b".repeat(64),
                },
            }],
        }]
    }

    fn rename_document(
        old: &AgentReference,
        new: &AgentReference,
        tools: &[&str],
    ) -> (crate::types::ControlCenterDocument, String) {
        let baseline = recommendation_baseline(old, tools);
        let subscription = recommendation_subscription();
        let catalog_feed = renamed_batch(old, new);
        let recommendation_id = derive_project_recommendations_with_installs(
            &baseline,
            &subscription,
            &catalog_feed,
            &BTreeMap::from([(new.clone(), 1)]),
            Some(&[]),
        )[0]
        .id
        .clone();
        (
            crate::types::ControlCenterDocument {
                project_baselines: vec![baseline],
                project_subscriptions: vec![subscription],
                catalog_last_success_at: Some("2026-08-17T01:00:00Z".into()),
                catalog_feed,
                ..Default::default()
            },
            recommendation_id,
        )
    }

    #[test]
    fn project_readiness_uses_locked_precedence_and_empty_categories() {
        let agent_requirement = BaselineAgentRequirement {
            reference: AgentReference {
                source_id: "source-a".into(),
                relative_path: "reviewer.md".into(),
            },
            tool: "codex".into(),
        };
        let baseline = ProjectReadinessBaseline {
            project_path: "/registered/project".into(),
            label: "Team baseline".into(),
            agent_requirements: vec![agent_requirement.clone()],
            skill_requirements: Vec::new(),
            agents: vec![agent_requirement.reference.clone()],
            skills: Vec::new(),
            instructions: vec![BaselineRequirement {
                id: "Follow the repository conventions".into(),
                known: false,
            }],
            mcp_servers: Vec::new(),
            tools: vec![agent_requirement.tool.clone()],
        };
        let report = build_readiness_report(
            "/registered/project",
            Some(&baseline),
            ReadinessEvidence {
                agents: Ok(BTreeMap::from([(
                    baseline.agent_requirements[0].clone(),
                    ReadinessRowState::Ready,
                )])),
                ..ReadinessEvidence::ready()
            },
        );
        assert_eq!(report.overall, ProjectReadinessOverall::NeedsAttention);
        assert_eq!(report.categories[0].state, ReadinessCategoryState::Ready);
        assert_eq!(
            report.categories[1].state,
            ReadinessCategoryState::NotRequired
        );
        assert_eq!(
            report.categories[2].rows[0].state,
            ReadinessRowState::Unverifiable
        );

        let unavailable = build_readiness_report(
            "/registered/project",
            Some(&baseline),
            ReadinessEvidence {
                agents: Err("Agent inspection failed".into()),
                ..ReadinessEvidence::ready()
            },
        );
        assert_eq!(unavailable.overall, ProjectReadinessOverall::Unavailable);
        assert_eq!(
            unavailable.categories[0].rows[0].state,
            ReadinessRowState::Unavailable
        );

        let unconfigured =
            build_readiness_report("/registered/project", None, ReadinessEvidence::ready());
        assert_eq!(unconfigured.overall, ProjectReadinessOverall::NotConfigured);
        assert!(unconfigured
            .categories
            .iter()
            .all(|category| category.state == ReadinessCategoryState::NotRequired));

        let needs_attention = build_readiness_report(
            "/registered/project",
            Some(&baseline),
            ReadinessEvidence::ready(),
        );
        assert_eq!(
            needs_attention.categories[0].rows[0].state,
            ReadinessRowState::NeedsAttention
        );

        let ready_baseline = ProjectReadinessBaseline {
            instructions: Vec::new(),
            ..baseline.clone()
        };
        let ready = build_readiness_report(
            "/registered/project",
            Some(&ready_baseline),
            ReadinessEvidence {
                agents: Ok(BTreeMap::from([(
                    ready_baseline.agent_requirements[0].clone(),
                    ReadinessRowState::Ready,
                )])),
                tools: Ok(BTreeMap::from([("codex".into(), ReadinessRowState::Ready)])),
                ..ReadinessEvidence::ready()
            },
        );
        assert_eq!(ready.overall, ProjectReadinessOverall::Ready);

        let independently_unavailable = build_readiness_report(
            "/registered/project",
            Some(&ready_baseline),
            ReadinessEvidence {
                skills: Err("Skill inspection failed".into()),
                agents: Ok(BTreeMap::from([(
                    ready_baseline.agent_requirements[0].clone(),
                    ReadinessRowState::Ready,
                )])),
                tools: Ok(BTreeMap::from([("codex".into(), ReadinessRowState::Ready)])),
                ..ReadinessEvidence::ready()
            },
        );
        assert_eq!(
            independently_unavailable.overall,
            ProjectReadinessOverall::Ready
        );

        let required_skill = ProjectReadinessBaseline {
            skill_requirements: vec![BaselineSkillRequirement {
                reference: SkillReference {
                    source_id: "skills".into(),
                    relative_path: "audit".into(),
                },
                runtime: "codex".into(),
            }],
            skills: vec![SkillReference {
                source_id: "skills".into(),
                relative_path: "audit".into(),
            }],
            ..ready_baseline
        };
        let independently_unavailable = build_readiness_report(
            "/registered/project",
            Some(&required_skill),
            ReadinessEvidence {
                skills: Err("Skill inspection failed".into()),
                agents: Ok(BTreeMap::from([(
                    required_skill.agent_requirements[0].clone(),
                    ReadinessRowState::Ready,
                )])),
                tools: Ok(BTreeMap::from([("codex".into(), ReadinessRowState::Ready)])),
                ..ReadinessEvidence::ready()
            },
        );
        assert_eq!(
            independently_unavailable.overall,
            ProjectReadinessOverall::Unavailable
        );
    }

    #[test]
    fn team_baseline_requires_one_exact_current_source_reference_per_slug() {
        let exact = built_in_result(&[("reviewer", "review/reviewer.md")]);
        let resolved =
            resolve_team_references(vec!["reviewer".into()], std::slice::from_ref(&exact)).unwrap();
        assert_eq!(resolved[0].relative_path, "review/reviewer.md");
        assert!(
            resolve_team_references(vec!["reviewer".into()], &[exact.clone(), exact],).is_err()
        );
        assert!(resolve_team_references(vec!["missing".into()], &[]).is_err());
    }

    #[test]
    fn workspace_pack_baseline_carries_exact_refs_and_keeps_requirements_opaque() {
        let pack = normalize_workspace_pack(workspace_pack_fixture()).unwrap();
        let baseline = baseline_from_workspace_pack("/registered/project".into(), pack);
        assert_eq!(baseline.agents[0].source_id, "source-a");
        assert_eq!(baseline.skills[0].source_id, "skills");
        assert_eq!(baseline.agent_requirements.len(), 2);
        assert!(baseline.agent_requirements.iter().any(|requirement| {
            requirement.reference.source_id == "source-a" && requirement.tool == "codex"
        }));
        assert_eq!(baseline.skill_requirements[0].runtime, "codex");
        assert!(baseline.instructions.iter().all(|item| !item.known));
        assert!(baseline.mcp_servers.iter().all(|item| !item.known));
        let report = build_readiness_report(
            "/registered/project",
            Some(&baseline),
            ReadinessEvidence::ready(),
        );
        assert!(report.categories[2]
            .rows
            .iter()
            .all(|row| row.state == ReadinessRowState::Unverifiable));
        assert!(report.categories[3]
            .rows
            .iter()
            .all(|row| row.state == ReadinessRowState::Unverifiable));
    }

    #[test]
    fn skill_only_workspace_pack_requires_its_runtime_tool() {
        let mut pack = workspace_pack_fixture();
        pack.agents.clear();
        pack.skills[0].runtime = "claudeCode".into();
        let baseline = baseline_from_workspace_pack(
            "/registered/project".into(),
            normalize_workspace_pack(pack).unwrap(),
        );

        assert_eq!(baseline.tools, vec!["claudeCode"]);
        let missing_tool = build_readiness_report(
            "/registered/project",
            Some(&baseline),
            ReadinessEvidence::ready(),
        );
        assert_eq!(
            missing_tool.categories[4].rows[0].state,
            ReadinessRowState::NeedsAttention
        );

        let detected_tool = build_readiness_report(
            "/registered/project",
            Some(&baseline),
            ReadinessEvidence {
                tools: Ok(BTreeMap::from([(
                    "claudeCode".into(),
                    ReadinessRowState::Ready,
                )])),
                ..ReadinessEvidence::ready()
            },
        );
        assert_eq!(
            detected_tool.categories[4].rows[0].state,
            ReadinessRowState::Ready
        );
    }

    #[test]
    fn legacy_baseline_defaults_exact_tuples_and_migrates_only_one_unambiguous_tool() {
        let baseline: ProjectReadinessBaseline = serde_json::from_value(serde_json::json!({
            "projectPath": "/registered/project",
            "label": "Legacy",
            "agents": [{"sourceId": "source-a", "relativePath": "reviewer.md"}],
            "skills": [{"sourceId": "skills", "relativePath": "audit"}],
            "instructions": [],
            "mcpServers": [],
            "tools": ["codex"]
        }))
        .unwrap();
        assert!(baseline.agent_requirements.is_empty());
        assert!(baseline.skill_requirements.is_empty());
        assert_eq!(exact_agent_requirements(&baseline)[0].tool, "codex");
        assert_eq!(
            exact_skill_requirements(&baseline)[0].runtime,
            LEGACY_UNREVIEWED_TARGET
        );

        let ambiguous: ProjectReadinessBaseline = serde_json::from_value(serde_json::json!({
            "projectPath": "/registered/project",
            "label": "Legacy",
            "agents": [{"sourceId": "source-a", "relativePath": "reviewer.md"}],
            "tools": ["codex", "claudeCode"]
        }))
        .unwrap();
        assert_eq!(
            exact_agent_requirements(&ambiguous)[0].tool,
            LEGACY_UNREVIEWED_TARGET
        );
    }

    #[test]
    fn readiness_requires_the_exact_agent_tool_and_skill_runtime_tuple() {
        let agent = BaselineAgentRequirement {
            reference: AgentReference {
                source_id: "source-a".into(),
                relative_path: "reviewer.md".into(),
            },
            tool: "codex".into(),
        };
        let skill = BaselineSkillRequirement {
            reference: SkillReference {
                source_id: "skills".into(),
                relative_path: "audit".into(),
            },
            runtime: "codex".into(),
        };
        let baseline = ProjectReadinessBaseline {
            project_path: "/registered/project".into(),
            label: "Exact targets".into(),
            agent_requirements: vec![agent.clone()],
            skill_requirements: vec![skill.clone()],
            agents: vec![agent.reference.clone()],
            skills: vec![skill.reference.clone()],
            instructions: Vec::new(),
            mcp_servers: Vec::new(),
            tools: vec![agent.tool.clone()],
        };
        let report = build_readiness_report(
            &baseline.project_path,
            Some(&baseline),
            ReadinessEvidence {
                agents: Ok(BTreeMap::from([(
                    BaselineAgentRequirement {
                        tool: "claudeCode".into(),
                        ..agent
                    },
                    ReadinessRowState::Ready,
                )])),
                skills: Ok(BTreeMap::from([(
                    BaselineSkillRequirement {
                        runtime: "cursor".into(),
                        ..skill
                    },
                    ReadinessRowState::Ready,
                )])),
                ..ReadinessEvidence::ready()
            },
        );
        assert_eq!(
            report.categories[0].state,
            ReadinessCategoryState::NeedsAttention
        );
        assert_eq!(
            report.categories[1].state,
            ReadinessCategoryState::NeedsAttention
        );
    }

    #[test]
    fn atomic_baseline_subscription_failure_leaves_the_document_unchanged() {
        let mut document = crate::types::ControlCenterDocument::default();
        for index in 0..64 {
            let project_path = format!("/registered/project-{index}");
            document.project_baselines.push(ProjectReadinessBaseline {
                project_path: project_path.clone(),
                label: format!("Project {index}"),
                agent_requirements: Vec::new(),
                skill_requirements: Vec::new(),
                agents: Vec::new(),
                skills: Vec::new(),
                instructions: Vec::new(),
                mcp_servers: Vec::new(),
                tools: Vec::new(),
            });
            document.project_subscriptions.push(ProjectSubscription {
                project_path,
                last_seen_batch: None,
                pending_recommendation_ids: Vec::new(),
                dismissed_recommendation_ids: Vec::new(),
            });
        }
        let before = document.clone();
        let extra = ProjectReadinessBaseline {
            project_path: "/registered/extra".into(),
            label: "Extra".into(),
            agent_requirements: Vec::new(),
            skill_requirements: Vec::new(),
            agents: Vec::new(),
            skills: Vec::new(),
            instructions: Vec::new(),
            mcp_servers: Vec::new(),
            tools: Vec::new(),
        };
        assert!(apply_project_baseline_and_subscription(&mut document, extra, true).is_err());
        assert_eq!(document, before);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unregister_cleanup_holds_the_project_lock_against_a_racing_baseline_save() {
        let app_data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let state = std::sync::Arc::new(
            project_instruction_test_state(app_data.path(), project.path()).await,
        );
        let project_path = std::fs::canonicalize(project.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let database = control_center_database(&state).await.unwrap();
        let initial_path = project_path.clone();
        database
            .mutate(
                corpus::control_center_spec(),
                crate::types::ControlCenterDocument::default(),
                move |document| {
                    apply_project_baseline_and_subscription(
                        document,
                        ProjectReadinessBaseline {
                            project_path: initial_path,
                            label: "Initial".into(),
                            agent_requirements: Vec::new(),
                            skill_requirements: Vec::new(),
                            agents: Vec::new(),
                            skills: Vec::new(),
                            instructions: Vec::new(),
                            mcp_servers: Vec::new(),
                            tools: Vec::new(),
                        },
                        true,
                    )
                },
            )
            .await
            .unwrap();

        let unregister_lock = lock_project_registry(&state.app_data_dir).unwrap();
        save_registered_projects(&state.app_data_dir, &[])
            .await
            .unwrap();
        let racing_state = state.clone();
        let racing_path = project_path.clone();
        let save = tokio::spawn(async move {
            persist_registered_project_baseline(
                &racing_state,
                &racing_path,
                ProjectReadinessBaseline {
                    project_path: String::new(),
                    label: "Racing save".into(),
                    agent_requirements: Vec::new(),
                    skill_requirements: Vec::new(),
                    agents: Vec::new(),
                    skills: Vec::new(),
                    instructions: Vec::new(),
                    mcp_servers: Vec::new(),
                    tools: Vec::new(),
                },
                true,
            )
            .await
        });
        let removed_path = project_path.clone();
        database
            .mutate(
                corpus::control_center_spec(),
                crate::types::ControlCenterDocument::default(),
                move |document| {
                    document
                        .project_baselines
                        .retain(|baseline| baseline.project_path != removed_path);
                    document
                        .project_subscriptions
                        .retain(|subscription| subscription.project_path != removed_path);
                    Ok(())
                },
            )
            .await
            .unwrap();
        drop(unregister_lock);

        assert!(save.await.unwrap().is_err());
        let document = corpus::load_control_center(&database).await.unwrap();
        assert!(document.project_baselines.is_empty());
        assert!(document.project_subscriptions.is_empty());
    }

    #[test]
    fn dismissal_prunes_ids_no_longer_represented_before_adding_the_current_id() {
        let mut document = crate::types::ControlCenterDocument {
            project_subscriptions: vec![ProjectSubscription {
                project_path: "/registered/project".into(),
                last_seen_batch: None,
                pending_recommendation_ids: vec!["current".into()],
                dismissed_recommendation_ids: (0..256)
                    .map(|index| format!("stale-{index}"))
                    .collect(),
            }],
            ..crate::types::ControlCenterDocument::default()
        };
        let represented = BTreeSet::from(["current".to_string()]);
        dismiss_project_recommendation(
            &mut document,
            "/registered/project",
            "current".into(),
            &represented,
        )
        .unwrap();
        assert_eq!(
            document.project_subscriptions[0].dismissed_recommendation_ids,
            vec!["current"]
        );
        assert!(document.project_subscriptions[0]
            .pending_recommendation_ids
            .is_empty());
    }

    #[test]
    fn pending_prune_retains_only_latest_represented_recommendations() {
        let mut document = crate::types::ControlCenterDocument {
            project_subscriptions: vec![ProjectSubscription {
                project_path: "/registered/project".into(),
                last_seen_batch: Some("2026-08-17T02:00:00Z".into()),
                pending_recommendation_ids: vec!["a".repeat(64), "b".repeat(64), "c".repeat(64)],
                dismissed_recommendation_ids: Vec::new(),
            }],
            ..Default::default()
        };

        prune_project_pending_recommendations(
            &mut document,
            "/registered/project",
            &BTreeSet::from(["b".repeat(64)]),
            &BTreeSet::from(["b".repeat(64)]),
        )
        .unwrap();

        assert_eq!(
            document.project_subscriptions[0].pending_recommendation_ids,
            vec!["b".repeat(64)]
        );
    }

    #[test]
    fn sequential_dismissals_retain_current_receipts_and_prune_only_obsolete_ids() {
        let first = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/first.md".into(),
        };
        let second = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/second.md".into(),
        };
        let baseline = ProjectReadinessBaseline {
            project_path: "/registered/project".into(),
            label: "Reviewers".into(),
            agent_requirements: vec![
                BaselineAgentRequirement {
                    reference: first.clone(),
                    tool: "codex".into(),
                },
                BaselineAgentRequirement {
                    reference: second.clone(),
                    tool: "codex".into(),
                },
            ],
            skill_requirements: Vec::new(),
            agents: vec![first.clone(), second.clone()],
            skills: Vec::new(),
            instructions: Vec::new(),
            mcp_servers: Vec::new(),
            tools: vec!["codex".into()],
        };
        let batch =
            |reference: &AgentReference, at: &str, before: char, after: char| CatalogFeedBatch {
                at: at.into(),
                changes: vec![CatalogChange::Updated {
                    before: CatalogSnapshotItem {
                        category: "engineering".into(),
                        relative_path: reference.relative_path.clone(),
                        source_hash: before.to_string().repeat(64),
                        body_hash: before.to_string().repeat(64),
                    },
                    after: CatalogSnapshotItem {
                        category: "engineering".into(),
                        relative_path: reference.relative_path.clone(),
                        source_hash: after.to_string().repeat(64),
                        body_hash: after.to_string().repeat(64),
                    },
                }],
            };
        let batches = vec![
            batch(&first, "2026-08-17T01:00:00Z", 'a', 'b'),
            batch(&second, "2026-08-17T02:00:00Z", 'c', 'd'),
        ];
        let available = BTreeMap::from([(first.clone(), 1), (second.clone(), 1)]);
        let installed = vec![
            recommendation_install(&first, "codex", InstallState::Outdated),
            recommendation_install(&second, "codex", InstallState::Outdated),
        ];
        let subscription = recommendation_subscription();
        let visible = derive_project_recommendations_with_installs(
            &baseline,
            &subscription,
            &batches,
            &available,
            Some(&installed),
        );
        let first_id = visible
            .iter()
            .find(|recommendation| recommendation.baseline_reference == first)
            .unwrap()
            .id
            .clone();
        let second_id = visible
            .iter()
            .find(|recommendation| recommendation.baseline_reference == second)
            .unwrap()
            .id
            .clone();
        let mut document = crate::types::ControlCenterDocument {
            project_baselines: vec![baseline.clone()],
            project_subscriptions: vec![subscription],
            catalog_feed: batches.clone(),
            catalog_last_success_at: Some("2026-08-17T02:00:00Z".into()),
            ..Default::default()
        };

        let candidates = current_project_recommendation_candidate_ids(
            &baseline,
            &document.project_subscriptions[0],
            &batches,
            &available,
            Some(&installed),
        );
        dismiss_project_recommendation(
            &mut document,
            "/registered/project",
            first_id.clone(),
            &candidates,
        )
        .unwrap();
        let candidates = current_project_recommendation_candidate_ids(
            &baseline,
            &document.project_subscriptions[0],
            &batches,
            &available,
            Some(&installed),
        );
        dismiss_project_recommendation(
            &mut document,
            "/registered/project",
            second_id.clone(),
            &candidates,
        )
        .unwrap();

        assert!(derive_project_recommendations_with_installs(
            &baseline,
            &document.project_subscriptions[0],
            &batches,
            &available,
            Some(&installed),
        )
        .is_empty());
        let mut retained = vec![first_id.clone(), second_id.clone()];
        retained.sort();
        assert_eq!(
            document.project_subscriptions[0].dismissed_recommendation_ids,
            retained
        );
        assert!(corpus::validate_control_center(&document).is_ok());

        let second_only = recommendation_baseline(&second, &["codex"]);
        document.project_baselines[0] = second_only.clone();
        let candidates = current_project_recommendation_candidate_ids(
            &second_only,
            &document.project_subscriptions[0],
            &batches,
            &available,
            Some(&installed),
        );
        prune_project_pending_recommendations(
            &mut document,
            "/registered/project",
            &candidates,
            &BTreeSet::new(),
        )
        .unwrap();
        assert_eq!(
            document.project_subscriptions[0].dismissed_recommendation_ids,
            vec![second_id]
        );
        assert!(corpus::validate_control_center(&document).is_ok());

        document.catalog_feed.clear();
        document.catalog_last_success_at = None;
        let candidates = current_project_recommendation_candidate_ids(
            &second_only,
            &document.project_subscriptions[0],
            &[],
            &available,
            Some(&installed),
        );
        prune_project_pending_recommendations(
            &mut document,
            "/registered/project",
            &candidates,
            &BTreeSet::new(),
        )
        .unwrap();
        assert!(document.project_subscriptions[0]
            .dismissed_recommendation_ids
            .is_empty());
        assert!(corpus::validate_control_center(&document).is_ok());
    }

    #[test]
    fn project_subscription_is_explicit_opt_in_and_requires_a_baseline() {
        let mut document = crate::types::ControlCenterDocument::default();
        assert!(
            set_project_subscription(&mut document, "/registered/project".into(), true,).is_err()
        );
        document.project_baselines.push(ProjectReadinessBaseline {
            project_path: "/registered/project".into(),
            label: "Review".into(),
            agent_requirements: Vec::new(),
            skill_requirements: Vec::new(),
            agents: Vec::new(),
            skills: Vec::new(),
            instructions: Vec::new(),
            mcp_servers: Vec::new(),
            tools: Vec::new(),
        });
        set_project_subscription(&mut document, "/registered/project".into(), true).unwrap();
        assert_eq!(document.project_subscriptions.len(), 1);
        assert!(document.project_subscriptions[0].last_seen_batch.is_none());
        set_project_subscription(&mut document, "/registered/project".into(), false).unwrap();
        assert!(document.project_subscriptions.is_empty());
    }

    #[tokio::test]
    async fn project_baseline_and_opt_in_round_trip_in_versioned_control_center_sqlite() {
        let app = tempfile::tempdir().unwrap();
        let database = crate::state_db::StateDatabase::open(app.path()).unwrap();
        let baseline = ProjectReadinessBaseline {
            project_path: "/registered/project".into(),
            label: "Review".into(),
            agent_requirements: Vec::new(),
            skill_requirements: Vec::new(),
            agents: vec![AgentReference {
                source_id: "source-a".into(),
                relative_path: "reviewer.md".into(),
            }],
            skills: Vec::new(),
            instructions: Vec::new(),
            mcp_servers: Vec::new(),
            tools: Vec::new(),
        };
        database
            .mutate(
                corpus::control_center_spec(),
                crate::types::ControlCenterDocument::default(),
                move |document| {
                    document.project_baselines.push(baseline);
                    set_project_subscription(document, "/registered/project".into(), true)
                },
            )
            .await
            .unwrap();
        drop(database);

        let reopened = crate::state_db::StateDatabase::open(app.path()).unwrap();
        let document = corpus::load_control_center(&reopened).await.unwrap();
        assert_eq!(
            document.project_baselines[0].agents[0].source_id,
            "source-a"
        );
        assert_eq!(
            document.project_subscriptions[0].project_path,
            "/registered/project"
        );
    }

    #[tokio::test]
    async fn project_recommendation_listing_is_cursor_and_destination_write_free() {
        let app_data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let marker = project.path().join("user-owned.txt");
        std::fs::write(&marker, "unchanged").unwrap();
        let state = project_instruction_test_state(app_data.path(), project.path()).await;
        let project_path = std::fs::canonicalize(project.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let reference = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/reviewer.md".into(),
        };
        let database = control_center_database(&state).await.unwrap();
        let persisted_path = project_path.clone();
        database
            .mutate(
                corpus::control_center_spec(),
                crate::types::ControlCenterDocument::default(),
                move |document| {
                    document.catalog_last_success_at = Some("2026-08-17T01:00:00Z".into());
                    document.catalog_feed = vec![CatalogFeedBatch {
                        at: "2026-08-17T01:00:00Z".into(),
                        changes: vec![CatalogChange::Updated {
                            before: CatalogSnapshotItem {
                                category: "engineering".into(),
                                relative_path: reference.relative_path.clone(),
                                source_hash: "a".repeat(64),
                                body_hash: "b".repeat(64),
                            },
                            after: CatalogSnapshotItem {
                                category: "engineering".into(),
                                relative_path: reference.relative_path.clone(),
                                source_hash: "c".repeat(64),
                                body_hash: "d".repeat(64),
                            },
                        }],
                    }];
                    document.project_baselines = vec![ProjectReadinessBaseline {
                        project_path: persisted_path.clone(),
                        label: "Reviewers".into(),
                        agent_requirements: vec![BaselineAgentRequirement {
                            reference: reference.clone(),
                            tool: "codex".into(),
                        }],
                        skill_requirements: Vec::new(),
                        agents: vec![reference],
                        skills: Vec::new(),
                        instructions: Vec::new(),
                        mcp_servers: Vec::new(),
                        tools: vec!["codex".into()],
                    }];
                    document.project_subscriptions = vec![ProjectSubscription {
                        project_path: persisted_path,
                        last_seen_batch: None,
                        pending_recommendation_ids: Vec::new(),
                        dismissed_recommendation_ids: Vec::new(),
                    }];
                    Ok(())
                },
            )
            .await
            .unwrap();
        let before = corpus::load_control_center(&database).await.unwrap();

        let recommendations = list_project_recommendations_for_state(&state, &project_path)
            .await
            .unwrap();

        assert_eq!(recommendations.len(), 1);
        assert_eq!(
            recommendations[0].change_kind,
            RecommendationChangeKind::Updated
        );
        assert_eq!(
            recommendations[0].lifecycle,
            RecommendationLifecycle::Blocked
        );
        assert!(recommendations[0].targets.is_empty());
        assert_eq!(
            corpus::load_control_center(&database).await.unwrap(),
            before
        );
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "unchanged");
        assert_eq!(std::fs::read_dir(project.path()).unwrap().count(), 1);
    }

    #[test]
    fn subscription_recommendations_obey_cursor_dismissal_supersession_and_blocking() {
        let reference = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/reviewer.md".into(),
        };
        let baseline = ProjectReadinessBaseline {
            project_path: "/registered/project".into(),
            label: "Reviewers".into(),
            agent_requirements: vec![BaselineAgentRequirement {
                reference: reference.clone(),
                tool: "codex".into(),
            }],
            skill_requirements: Vec::new(),
            agents: vec![reference.clone()],
            skills: Vec::new(),
            instructions: Vec::new(),
            mcp_servers: Vec::new(),
            tools: vec!["codex".into()],
        };
        let batches = vec![
            CatalogFeedBatch {
                at: "2026-08-17T01:00:00Z".into(),
                changes: vec![CatalogChange::Updated {
                    before: CatalogSnapshotItem {
                        category: "engineering".into(),
                        relative_path: "engineering/reviewer.md".into(),
                        source_hash: "a".repeat(64),
                        body_hash: "b".repeat(64),
                    },
                    after: CatalogSnapshotItem {
                        category: "engineering".into(),
                        relative_path: "engineering/reviewer.md".into(),
                        source_hash: "c".repeat(64),
                        body_hash: "d".repeat(64),
                    },
                }],
            },
            CatalogFeedBatch {
                at: "2026-08-17T02:00:00Z".into(),
                changes: vec![CatalogChange::Removed {
                    item: CatalogSnapshotItem {
                        category: "engineering".into(),
                        relative_path: "engineering/reviewer.md".into(),
                        source_hash: "c".repeat(64),
                        body_hash: "d".repeat(64),
                    },
                }],
            },
        ];
        let mut subscription = ProjectSubscription {
            project_path: baseline.project_path.clone(),
            last_seen_batch: None,
            pending_recommendation_ids: Vec::new(),
            dismissed_recommendation_ids: Vec::new(),
        };
        let recommendations = derive_project_recommendations(
            &baseline,
            &subscription,
            &batches,
            &BTreeMap::from([(reference.clone(), 1)]),
        );
        assert_eq!(recommendations.len(), 2);
        assert_eq!(
            recommendations[0].lifecycle,
            RecommendationLifecycle::Superseded
        );
        assert_eq!(
            recommendations[1].lifecycle,
            RecommendationLifecycle::Blocked
        );
        assert_eq!(
            recommendations[1].change_kind,
            RecommendationChangeKind::Removed
        );
        assert_eq!(
            recommendations[1].targets[0].operation,
            RecommendationOperation::Informational
        );
        assert_eq!(
            derive_project_recommendations(&baseline, &subscription, &batches, &BTreeMap::new(),)
                [1]
            .lifecycle,
            RecommendationLifecycle::Blocked
        );
        subscription
            .dismissed_recommendation_ids
            .push(recommendations[1].id.clone());
        assert!(derive_project_recommendations(
            &baseline,
            &subscription,
            &batches,
            &BTreeMap::new(),
        )
        .iter()
        .all(|recommendation| recommendation.id != recommendations[1].id));
        subscription.last_seen_batch = Some("2026-08-17T02:00:00Z".into());
        assert!(derive_project_recommendations(
            &baseline,
            &subscription,
            &batches,
            &BTreeMap::from([(reference, 1)]),
        )
        .iter()
        .all(|recommendation| recommendation.lifecycle != RecommendationLifecycle::New));
    }

    #[test]
    fn updated_recommendation_targets_only_the_remaining_outdated_tool() {
        let reference = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/reviewer.md".into(),
        };
        let baseline = recommendation_baseline(&reference, &["claudeCode", "codex"]);
        let installed = vec![
            recommendation_install(&reference, "claudeCode", InstallState::Current),
            recommendation_install(&reference, "codex", InstallState::Outdated),
        ];
        let recommendations = derive_project_recommendations_with_installs(
            &baseline,
            &recommendation_subscription(),
            &updated_batch(&reference),
            &BTreeMap::from([(reference.clone(), 1)]),
            Some(&installed),
        );

        assert_eq!(recommendations.len(), 1);
        assert_eq!(recommendations[0].lifecycle, RecommendationLifecycle::New);
        assert_eq!(recommendations[0].targets.len(), 1);
        assert_eq!(recommendations[0].targets[0].tool, "codex");
        assert_eq!(
            recommendations[0].targets[0].operation,
            RecommendationOperation::Update
        );
    }

    #[test]
    fn acknowledged_recommendation_remains_pending_after_its_cursor_advances() {
        let reference = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/reviewer.md".into(),
        };
        let baseline = recommendation_baseline(&reference, &["codex"]);
        let batches = updated_batch(&reference);
        let available = BTreeMap::from([(reference.clone(), 1)]);
        let installed = vec![recommendation_install(
            &reference,
            "codex",
            InstallState::Outdated,
        )];
        let mut subscription = recommendation_subscription();
        let id = derive_project_recommendations_with_installs(
            &baseline,
            &subscription,
            &batches,
            &available,
            Some(&installed),
        )[0]
        .id
        .clone();
        subscription.last_seen_batch = Some(batches[0].at.clone());
        subscription.pending_recommendation_ids.push(id.clone());

        let pending = derive_project_recommendations_with_installs(
            &baseline,
            &subscription,
            &batches,
            &available,
            Some(&installed),
        );
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, id);
        assert_eq!(pending[0].lifecycle, RecommendationLifecycle::Pending);
    }

    #[test]
    fn legacy_seen_actionable_recommendation_is_recovered_as_pending() {
        let reference = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/reviewer.md".into(),
        };
        let baseline = recommendation_baseline(&reference, &["codex"]);
        let batches = updated_batch(&reference);
        let mut subscription = recommendation_subscription();
        subscription.last_seen_batch = Some(batches[0].at.clone());
        let installed = vec![recommendation_install(
            &reference,
            "codex",
            InstallState::Outdated,
        )];

        let pending = derive_project_recommendations_with_installs(
            &baseline,
            &subscription,
            &batches,
            &BTreeMap::from([(reference, 1)]),
            Some(&installed),
        );

        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].lifecycle, RecommendationLifecycle::Pending);
    }

    #[test]
    fn updated_recommendation_with_all_targets_current_has_no_action() {
        let reference = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/reviewer.md".into(),
        };
        let baseline = recommendation_baseline(&reference, &["claudeCode", "codex"]);
        let installed = vec![
            recommendation_install(&reference, "claudeCode", InstallState::Current),
            recommendation_install(&reference, "codex", InstallState::Current),
        ];

        assert!(derive_project_recommendations_with_installs(
            &baseline,
            &recommendation_subscription(),
            &updated_batch(&reference),
            &BTreeMap::from([(reference.clone(), 1)]),
            Some(&installed),
        )
        .is_empty());
    }

    #[test]
    fn pending_added_recommendation_clears_when_its_exact_target_is_current() {
        let reference = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/reviewer.md".into(),
        };
        let baseline = recommendation_baseline(&reference, &["codex"]);
        let batches = vec![CatalogFeedBatch {
            at: "2026-08-17T01:00:00Z".into(),
            changes: vec![CatalogChange::Added {
                item: CatalogSnapshotItem {
                    category: "engineering".into(),
                    relative_path: reference.relative_path.clone(),
                    source_hash: "a".repeat(64),
                    body_hash: "b".repeat(64),
                },
            }],
        }];
        let available = BTreeMap::from([(reference.clone(), 1)]);
        let mut subscription = recommendation_subscription();
        let id = derive_project_recommendations_with_installs(
            &baseline,
            &subscription,
            &batches,
            &available,
            Some(&[]),
        )[0]
        .id
        .clone();
        subscription.last_seen_batch = Some(batches[0].at.clone());
        subscription.pending_recommendation_ids.push(id);
        let installed = vec![recommendation_install(
            &reference,
            "codex",
            InstallState::Current,
        )];

        assert!(derive_project_recommendations_with_installs(
            &baseline,
            &subscription,
            &batches,
            &available,
            Some(&installed),
        )
        .is_empty());
    }

    #[test]
    fn updated_recommendation_blocks_current_plus_modified_without_partial_action() {
        let reference = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/reviewer.md".into(),
        };
        let baseline = recommendation_baseline(&reference, &["claudeCode", "codex"]);
        let installed = vec![
            recommendation_install(&reference, "claudeCode", InstallState::Current),
            recommendation_install(&reference, "codex", InstallState::Modified),
        ];
        let recommendations = derive_project_recommendations_with_installs(
            &baseline,
            &recommendation_subscription(),
            &updated_batch(&reference),
            &BTreeMap::from([(reference.clone(), 1)]),
            Some(&installed),
        );

        assert_eq!(recommendations.len(), 1);
        assert_eq!(
            recommendations[0].lifecycle,
            RecommendationLifecycle::Blocked
        );
        assert!(recommendations[0].targets.is_empty());
    }

    #[test]
    fn updated_recommendation_revalidation_rejects_a_race_to_current() {
        let reference = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/reviewer.md".into(),
        };
        let baseline = recommendation_baseline(&reference, &["codex"]);
        let before = vec![recommendation_install(
            &reference,
            "codex",
            InstallState::Outdated,
        )];
        let opened = derive_project_recommendations_with_installs(
            &baseline,
            &recommendation_subscription(),
            &updated_batch(&reference),
            &BTreeMap::from([(reference.clone(), 1)]),
            Some(&before),
        );
        let recommendation_id = opened[0].id.clone();
        let after = vec![recommendation_install(
            &reference,
            "codex",
            InstallState::Current,
        )];

        assert!(!derive_project_recommendations_with_installs(
            &baseline,
            &recommendation_subscription(),
            &updated_batch(&reference),
            &BTreeMap::from([(reference, 1)]),
            Some(&after),
        )
        .iter()
        .any(|recommendation| recommendation.id == recommendation_id));
    }

    #[test]
    fn rename_recommendation_tracks_old_identity_and_hands_off_exact_new_reference() {
        let old = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/old-reviewer.md".into(),
        };
        let new = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/new-reviewer.md".into(),
        };
        let baseline = ProjectReadinessBaseline {
            project_path: "/registered/project".into(),
            label: "Reviewers".into(),
            agent_requirements: vec![BaselineAgentRequirement {
                reference: old.clone(),
                tool: "codex".into(),
            }],
            skill_requirements: Vec::new(),
            agents: vec![old.clone()],
            skills: Vec::new(),
            instructions: Vec::new(),
            mcp_servers: Vec::new(),
            tools: vec!["codex".into()],
        };
        let batches = vec![CatalogFeedBatch {
            at: "2026-08-17T01:00:00Z".into(),
            changes: vec![CatalogChange::Renamed {
                before: CatalogSnapshotItem {
                    category: "engineering".into(),
                    relative_path: old.relative_path.clone(),
                    source_hash: "a".repeat(64),
                    body_hash: "b".repeat(64),
                },
                after: CatalogSnapshotItem {
                    category: "engineering".into(),
                    relative_path: new.relative_path.clone(),
                    source_hash: "a".repeat(64),
                    body_hash: "b".repeat(64),
                },
            }],
        }];
        let subscription = ProjectSubscription {
            project_path: baseline.project_path.clone(),
            last_seen_batch: None,
            pending_recommendation_ids: Vec::new(),
            dismissed_recommendation_ids: Vec::new(),
        };

        let ready = derive_project_recommendations_with_installs(
            &baseline,
            &subscription,
            &batches,
            &BTreeMap::from([(new.clone(), 1)]),
            Some(&[]),
        );
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].baseline_reference, old);
        assert_eq!(ready[0].agent_references, vec![new.clone()]);
        assert_eq!(ready[0].change_kind, RecommendationChangeKind::Renamed);
        assert_eq!(ready[0].targets[0].reference, new.clone());
        assert_eq!(ready[0].targets[0].tool, "codex");
        assert_eq!(
            ready[0].targets[0].operation,
            RecommendationOperation::Install
        );
        assert!(ready[0].summary.contains("old-reviewer.md"));
        assert!(ready[0].summary.contains("new-reviewer.md"));
        assert_eq!(ready[0].lifecycle, RecommendationLifecycle::New);

        let ambiguous = derive_project_recommendations_with_installs(
            &baseline,
            &subscription,
            &batches,
            &BTreeMap::from([(new, 2)]),
            Some(&[]),
        );
        assert_eq!(ambiguous[0].lifecycle, RecommendationLifecycle::Blocked);
    }

    #[test]
    fn pending_rename_rederives_remaining_targets_then_becomes_finalize_only() {
        let old = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/old-reviewer.md".into(),
        };
        let new = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/new-reviewer.md".into(),
        };
        let baseline = recommendation_baseline(&old, &["claudeCode", "codex"]);
        let batches = renamed_batch(&old, &new);
        let available = BTreeMap::from([(new.clone(), 1)]);
        let mut subscription = recommendation_subscription();
        let id = derive_project_recommendations_with_installs(
            &baseline,
            &subscription,
            &batches,
            &available,
            Some(&[]),
        )[0]
        .id
        .clone();
        subscription.last_seen_batch = Some(batches[0].at.clone());
        subscription.pending_recommendation_ids.push(id.clone());

        let partial = vec![recommendation_install(
            &new,
            "claudeCode",
            InstallState::Current,
        )];
        let remaining = derive_project_recommendations_with_installs(
            &baseline,
            &subscription,
            &batches,
            &available,
            Some(&partial),
        );
        assert_eq!(remaining[0].lifecycle, RecommendationLifecycle::Pending);
        assert_eq!(remaining[0].targets.len(), 1);
        assert_eq!(remaining[0].targets[0].tool, "codex");
        assert!(!remaining[0].finalize_only);

        let complete = vec![
            recommendation_install(&new, "claudeCode", InstallState::Current),
            recommendation_install(&new, "codex", InstallState::Current),
        ];
        let finalize = derive_project_recommendations_with_installs(
            &baseline,
            &subscription,
            &batches,
            &available,
            Some(&complete),
        );
        assert_eq!(finalize.len(), 1);
        assert_eq!(finalize[0].id, id);
        assert_eq!(finalize[0].lifecycle, RecommendationLifecycle::Pending);
        assert!(finalize[0].targets.is_empty());
        assert!(finalize[0].finalize_only);
    }

    #[test]
    fn mixed_rename_open_validates_each_target_by_its_own_operation() {
        let old = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/old-reviewer.md".into(),
        };
        let new = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/new-reviewer.md".into(),
        };
        let baseline = recommendation_baseline(&old, &["claudeCode", "codex"]);
        let installed = vec![
            recommendation_install(&new, "claudeCode", InstallState::Outdated),
            recommendation_install(&new, "codex", InstallState::Missing),
        ];

        let recommendations = derive_project_recommendations_with_installs(
            &baseline,
            &recommendation_subscription(),
            &renamed_batch(&old, &new),
            &BTreeMap::from([(new, 1)]),
            Some(&installed),
        );

        assert_eq!(recommendations.len(), 1);
        assert_eq!(recommendations[0].lifecycle, RecommendationLifecycle::New);
        assert_eq!(
            recommendations[0]
                .targets
                .iter()
                .map(|target| (target.tool.as_str(), target.operation))
                .collect::<Vec<_>>(),
            vec![
                ("claudeCode", RecommendationOperation::Update),
                ("codex", RecommendationOperation::Install),
            ]
        );
        assert!(
            validate_project_recommendation_target_states(&recommendations[0], &installed,).is_ok()
        );
    }

    #[test]
    fn install_recommendation_open_rejects_unsafe_exact_target_states() {
        let old = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/old-reviewer.md".into(),
        };
        let new = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/new-reviewer.md".into(),
        };
        let baseline = recommendation_baseline(&old, &["codex"]);
        let recommendation = derive_project_recommendations_with_installs(
            &baseline,
            &recommendation_subscription(),
            &renamed_batch(&old, &new),
            &BTreeMap::from([(new.clone(), 1)]),
            Some(&[]),
        )
        .remove(0);
        assert_eq!(
            recommendation.targets[0].operation,
            RecommendationOperation::Install
        );
        assert!(validate_project_recommendation_target_states(&recommendation, &[]).is_ok());
        assert!(validate_project_recommendation_target_states(
            &recommendation,
            &[recommendation_install(&new, "codex", InstallState::Missing)],
        )
        .is_ok());

        let mut untracked_missing = recommendation_install(&new, "codex", InstallState::Missing);
        untracked_missing.tracked = false;
        assert!(validate_project_recommendation_target_states(
            &recommendation,
            &[untracked_missing]
        )
        .is_err());

        for state in [
            InstallState::Current,
            InstallState::Outdated,
            InstallState::Modified,
            InstallState::Foreign,
            InstallState::Disabled,
            InstallState::SourceUnavailable,
        ] {
            let mut row = recommendation_install(&new, "codex", state);
            if state == InstallState::Foreign {
                row.tracked = false;
            }
            assert!(
                validate_project_recommendation_target_states(&recommendation, &[row]).is_err(),
                "Install target unexpectedly accepted {state:?}",
            );
        }
    }

    #[test]
    fn rename_finalize_replaces_every_matching_requirement_after_all_targets_are_current() {
        let old = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/old-reviewer.md".into(),
        };
        let new = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/new-reviewer.md".into(),
        };
        let (mut document, recommendation_id) =
            rename_document(&old, &new, &["claudeCode", "codex"]);
        document.project_subscriptions[0]
            .pending_recommendation_ids
            .push(recommendation_id.clone());
        let installed = vec![
            recommendation_install(&new, "claudeCode", InstallState::Current),
            recommendation_install(&new, "codex", InstallState::Current),
        ];

        let baseline = finalize_rename_recommendation_in_document(
            &mut document,
            "/registered/project",
            &recommendation_id,
            &BTreeMap::from([(new.clone(), 1)]),
            &installed,
        )
        .unwrap();

        assert_eq!(
            baseline
                .agent_requirements
                .iter()
                .map(|requirement| (&requirement.reference, requirement.tool.as_str()))
                .collect::<Vec<_>>(),
            vec![(&new, "claudeCode"), (&new, "codex")]
        );
        assert_eq!(baseline.agents, vec![new]);
        assert_eq!(baseline.tools, vec!["claudeCode", "codex"]);
        assert!(document.project_subscriptions[0]
            .pending_recommendation_ids
            .is_empty());
    }

    #[test]
    fn rename_finalize_rejects_partial_installs_without_mutating_the_baseline() {
        let old = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/old-reviewer.md".into(),
        };
        let new = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/new-reviewer.md".into(),
        };
        let (mut document, recommendation_id) =
            rename_document(&old, &new, &["claudeCode", "codex"]);
        let before = document.clone();
        let installed = vec![
            recommendation_install(&new, "claudeCode", InstallState::Current),
            recommendation_install(&new, "codex", InstallState::Missing),
        ];

        assert!(finalize_rename_recommendation_in_document(
            &mut document,
            "/registered/project",
            &recommendation_id,
            &BTreeMap::from([(new, 1)]),
            &installed,
        )
        .is_err());
        assert_eq!(document, before);
    }

    #[test]
    fn rename_finalize_rejects_a_stale_recommendation_id_without_mutation() {
        let old = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/old-reviewer.md".into(),
        };
        let new = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/new-reviewer.md".into(),
        };
        let (mut document, recommendation_id) = rename_document(&old, &new, &["codex"]);
        document.project_baselines[0].agent_requirements[0].reference = new.clone();
        document.project_baselines[0].agents[0] = new.clone();
        let before = document.clone();
        let installed = vec![recommendation_install(&new, "codex", InstallState::Current)];

        assert!(finalize_rename_recommendation_in_document(
            &mut document,
            "/registered/project",
            &recommendation_id,
            &BTreeMap::from([(new, 1)]),
            &installed,
        )
        .is_err());
        assert_eq!(document, before);
    }

    #[test]
    fn rename_finalize_preserves_a_concurrent_unrelated_baseline_change() {
        let old = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/old-reviewer.md".into(),
        };
        let new = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/new-reviewer.md".into(),
        };
        let other = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/writer.md".into(),
        };
        let (mut document, recommendation_id) = rename_document(&old, &new, &["codex"]);
        document.project_baselines[0]
            .agent_requirements
            .push(BaselineAgentRequirement {
                reference: other.clone(),
                tool: "claudeCode".into(),
            });
        document.project_baselines[0].agents.push(other.clone());
        document.project_baselines[0]
            .tools
            .push("claudeCode".into());
        let installed = vec![recommendation_install(&new, "codex", InstallState::Current)];

        let baseline = finalize_rename_recommendation_in_document(
            &mut document,
            "/registered/project",
            &recommendation_id,
            &BTreeMap::from([(new.clone(), 1), (other.clone(), 1)]),
            &installed,
        )
        .unwrap();

        assert_eq!(baseline.agent_requirements[0].reference, new);
        assert_eq!(baseline.agent_requirements[1].reference, other.clone());
        assert_eq!(baseline.agents[1], other);
        assert_eq!(baseline.tools, vec!["codex", "claudeCode"]);
    }

    #[test]
    fn only_a_later_change_for_the_same_logical_baseline_supersedes() {
        let first = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/first.md".into(),
        };
        let unrelated = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/unrelated.md".into(),
        };
        let baseline = ProjectReadinessBaseline {
            project_path: "/registered/project".into(),
            label: "Reviewers".into(),
            agent_requirements: vec![BaselineAgentRequirement {
                reference: first.clone(),
                tool: "codex".into(),
            }],
            skill_requirements: Vec::new(),
            agents: vec![first.clone()],
            skills: Vec::new(),
            instructions: Vec::new(),
            mcp_servers: Vec::new(),
            tools: vec!["codex".into()],
        };
        let change = |reference: &AgentReference, at: &str, hash: char| CatalogFeedBatch {
            at: at.into(),
            changes: vec![CatalogChange::Updated {
                before: CatalogSnapshotItem {
                    category: "engineering".into(),
                    relative_path: reference.relative_path.clone(),
                    source_hash: format!("{:064x}", hash as u32),
                    body_hash: "b".repeat(64),
                },
                after: CatalogSnapshotItem {
                    category: "engineering".into(),
                    relative_path: reference.relative_path.clone(),
                    source_hash: format!("{:064x}", hash as u32 + 1),
                    body_hash: "c".repeat(64),
                },
            }],
        };
        let subscription = ProjectSubscription {
            project_path: baseline.project_path.clone(),
            last_seen_batch: None,
            pending_recommendation_ids: Vec::new(),
            dismissed_recommendation_ids: Vec::new(),
        };
        let available = BTreeMap::from([(first.clone(), 1)]);
        let installed = vec![recommendation_install(
            &first,
            "codex",
            InstallState::Outdated,
        )];

        let unrelated_later = derive_project_recommendations_with_installs(
            &baseline,
            &subscription,
            &[
                change(&first, "2026-08-17T01:00:00Z", 'a'),
                change(&unrelated, "2026-08-17T02:00:00Z", 'c'),
            ],
            &available,
            Some(&installed),
        );
        assert_eq!(unrelated_later.len(), 1);
        assert_eq!(unrelated_later[0].lifecycle, RecommendationLifecycle::New);

        let same_later = derive_project_recommendations_with_installs(
            &baseline,
            &subscription,
            &[
                change(&first, "2026-08-17T01:00:00Z", 'a'),
                change(&first, "2026-08-17T02:00:00Z", 'c'),
            ],
            &available,
            Some(&installed),
        );
        assert_eq!(same_later.len(), 2);
        assert_eq!(same_later[0].lifecycle, RecommendationLifecycle::Superseded);
        assert_eq!(same_later[1].lifecycle, RecommendationLifecycle::New);

        let latest_id = same_later[1].id.clone();
        let mut document = crate::types::ControlCenterDocument {
            project_subscriptions: vec![ProjectSubscription {
                project_path: baseline.project_path.clone(),
                last_seen_batch: Some("2026-08-17T02:00:00Z".into()),
                pending_recommendation_ids: same_later
                    .iter()
                    .map(|recommendation| recommendation.id.clone())
                    .collect(),
                dismissed_recommendation_ids: Vec::new(),
            }],
            ..Default::default()
        };
        let latest = BTreeSet::from([latest_id.clone()]);
        prune_project_pending_recommendations(
            &mut document,
            &baseline.project_path,
            &latest,
            &latest,
        )
        .unwrap();
        assert_eq!(
            document.project_subscriptions[0].pending_recommendation_ids,
            vec![latest_id]
        );
    }

    #[test]
    fn recommendation_acknowledgement_is_ordered_and_rejects_stale_cursor() {
        let mut document = crate::types::ControlCenterDocument {
            catalog_last_success_at: Some("2026-08-17T03:00:00Z".into()),
            project_subscriptions: vec![ProjectSubscription {
                project_path: "/registered/project".into(),
                last_seen_batch: Some("2026-08-17T01:00:00Z".into()),
                pending_recommendation_ids: vec!["a".repeat(64)],
                dismissed_recommendation_ids: Vec::new(),
            }],
            ..Default::default()
        };
        advance_project_recommendation_cursor(
            &mut document,
            "/registered/project",
            Some("2026-08-17T01:00:00Z"),
            "2026-08-17T02:00:00Z",
            vec!["a".repeat(64), "b".repeat(64)],
        )
        .unwrap();
        assert_eq!(
            document.project_subscriptions[0].last_seen_batch.as_deref(),
            Some("2026-08-17T02:00:00Z")
        );
        assert_eq!(
            document.project_subscriptions[0].pending_recommendation_ids,
            vec!["a".repeat(64), "b".repeat(64)]
        );
        assert!(advance_project_recommendation_cursor(
            &mut document,
            "/registered/project",
            Some("2026-08-17T01:00:00Z"),
            "2026-08-17T03:00:00Z",
            vec!["c".repeat(64)],
        )
        .is_err());
        assert!(advance_project_recommendation_cursor(
            &mut document,
            "/registered/project",
            Some("2026-08-17T02:00:00Z"),
            "2026-08-17T01:00:00Z",
            vec!["c".repeat(64)],
        )
        .is_err());
    }

    #[test]
    fn workspace_pack_v1_is_deterministic_bounded_and_path_private() {
        let mut pack = workspace_pack_fixture();
        pack.agents.reverse();
        pack.agents.push(pack.agents[0].clone());
        let normalized = normalize_workspace_pack(pack).unwrap();
        let bytes = serialize_workspace_pack(&normalized).unwrap();
        assert_eq!(bytes, serialize_workspace_pack(&normalized).unwrap());
        assert_eq!(normalized.agents.len(), 2);
        assert_eq!(normalized.agents[0].reference.source_id, "source-a");
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("\"workspacePack\": 1"));
        assert!(!text.contains("/Users/") && !text.contains("projectPath"));

        let mut oversize = workspace_pack_fixture();
        oversize.name = "x".repeat(161);
        assert!(normalize_workspace_pack(oversize).is_err());
        let mut too_many = workspace_pack_fixture();
        too_many.instructions = (0..257)
            .map(|index| format!("requirement-{index}"))
            .collect();
        assert!(normalize_workspace_pack(too_many).is_err());
        let mut credential = workspace_pack_fixture();
        credential.mcp_servers = vec!["token=secret123".into()];
        assert!(normalize_workspace_pack(credential).is_err());
    }

    #[test]
    fn workspace_pack_reader_rejects_versions_and_preserves_legacy_for_planning() {
        let pack = serialize_workspace_pack(&workspace_pack_fixture()).unwrap();
        assert!(matches!(
            parse_workspace_pack_input(&pack).unwrap(),
            WorkspacePackInput::Pack(_)
        ));
        let legacy = br#"{"agentfile":1,"installs":[{"slug":"reviewer","tool":"claudeCode","projectPath":null}]}"#;
        let WorkspacePackInput::Legacy(legacy) = parse_workspace_pack_input(legacy).unwrap() else {
            panic!("legacy Agentfile must enter review conversion")
        };
        assert_eq!(legacy.installs.len(), 1);
        assert!(parse_workspace_pack_input(
            br#"{"workspacePack":2,"name":"future","scope":"user","agents":[],"skills":[]}"#
        )
        .is_err());
        assert!(parse_workspace_pack_input(br#"{"agentfile":2,"installs":[]}"#).is_err());
        assert!(parse_workspace_pack_input(br#"{"workspacePack":1,"name":"hidden path","scope":"user","agents":[{"reference":{"sourceId":"source-a","relativePath":"reviewer.md","projectPath":"/Users/private"},"tool":"codex"}],"skills":[]}"#).is_err());
    }

    #[test]
    fn workspace_pack_export_selects_one_scope_and_strips_project_paths() {
        let project = "/Users/private/workspace";
        let agents = vec![
            row("global", "codex", None),
            row("project", "claudeCode", Some(project)),
        ];
        let skills = vec![crate::types::SkillInstallRecord {
            source_id: "skills".into(),
            relative_path: "audit".into(),
            name: "audit".into(),
            runtime: "codex".into(),
            scope: "project".into(),
            project_path: Some(project.into()),
            dest: format!("{project}/.agents/skills/audit"),
            source_hash: "a".repeat(64),
            installed_hash: "b".repeat(64),
            installed_at: "now".into(),
            disabled_path: None,
        }];
        let pack = workspace_pack_from_ledgers(
            "Project".into(),
            WorkspacePackScope::Project,
            Some(project),
            &agents,
            &skills,
        )
        .unwrap();
        let text = String::from_utf8(serialize_workspace_pack(&pack).unwrap()).unwrap();
        assert_eq!(pack.agents.len(), 1);
        assert_eq!(pack.skills.len(), 1);
        assert!(text.contains("project.md") && text.contains("\"audit\""));
        assert!(!text.contains(project) && !text.contains("global.md"));
    }

    #[test]
    fn legacy_agentfile_conversion_requires_unambiguous_valid_entries() {
        let sources = vec![built_in_result(&[("reviewer", "review/reviewer.md")])];
        let WorkspacePackInput::Legacy(legacy) = parse_workspace_pack_input(
            br#"{"agentfile":1,"installs":[{"slug":"reviewer","tool":"codex","projectPath":null}]}"#,
        )
        .unwrap()
        else {
            panic!("expected legacy input")
        };
        let pack = convert_legacy_agentfile(legacy, &sources).unwrap();
        assert_eq!(pack.agents[0].reference.relative_path, "review/reviewer.md");
        assert_eq!(pack.scope, WorkspacePackScope::User);

        let ambiguous = vec![built_in_result(&[
            ("reviewer", "one/reviewer.md"),
            ("reviewer", "two/reviewer.md"),
        ])];
        let legacy = Agentfile {
            agentfile: 1,
            installs: vec![LoadoutEntry {
                slug: "reviewer".into(),
                tool: "codex".into(),
                project_path: None,
            }],
        };
        assert!(convert_legacy_agentfile(legacy, &ambiguous).is_err());
    }

    #[test]
    fn workspace_pack_plan_revision_covers_complete_sorted_review() {
        let mut plan = WorkspacePackPlan {
            pack: workspace_pack_fixture(),
            project_path: Some("/bound/project".into()),
            agents: vec![WorkspacePackAgentPlan {
                reference: AgentReference {
                    source_id: "source-a".into(),
                    relative_path: "reviewer.md".into(),
                },
                name: "Reviewer".into(),
                tool: "codex".into(),
                destinations: vec!["/bound/project/.codex/agents/reviewer.md".into()],
                dependency: false,
                state: "missing".into(),
            }],
            skills: vec![WorkspacePackSkillPlan {
                reference: SkillReference {
                    source_id: "skills".into(),
                    relative_path: "audit".into(),
                },
                name: "Audit".into(),
                runtime: "codex".into(),
                destinations: vec!["/bound/project/.agents/skills/audit".into()],
                dependency: false,
                state: "current".into(),
                permissions: vec!["Read".into()],
            }],
            warnings: vec!["z".into(), "a".into(), "a".into()],
            blockers: Vec::new(),
            rollback_scope: vec!["/bound/project/.codex/agents/reviewer.md".into()],
            revision: String::new(),
        };
        finalize_workspace_pack_plan(&mut plan).unwrap();
        let revision = plan.revision.clone();
        assert_eq!(plan.warnings, vec!["a", "z"]);
        finalize_workspace_pack_plan(&mut plan).unwrap();
        assert_eq!(plan.revision, revision);
        plan.blockers.push("changed truth".into());
        finalize_workspace_pack_plan(&mut plan).unwrap();
        assert_ne!(plan.revision, revision);
    }

    #[test]
    fn workspace_pack_apply_requires_exact_revision_and_retains_noops() {
        let mut plan = WorkspacePackPlan {
            pack: workspace_pack_fixture(),
            project_path: Some("/bound/project".into()),
            agents: vec![WorkspacePackAgentPlan {
                reference: AgentReference {
                    source_id: "source-a".into(),
                    relative_path: "reviewer.md".into(),
                },
                name: "Reviewer".into(),
                tool: "codex".into(),
                destinations: vec!["/bound/project/reviewer.md".into()],
                dependency: false,
                state: "current".into(),
            }],
            skills: Vec::new(),
            warnings: Vec::new(),
            blockers: Vec::new(),
            rollback_scope: Vec::new(),
            revision: String::new(),
        };
        finalize_workspace_pack_plan(&mut plan).unwrap();
        assert!(require_workspace_pack_revision(&plan, &plan.revision).is_ok());
        assert!(require_workspace_pack_revision(&plan, "stale").is_err());
        let items = initial_workspace_pack_results(&plan);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].outcome, "current");
        assert_eq!(items[0].destination, "/bound/project/reviewer.md");
    }

    #[tokio::test]
    async fn workspace_pack_parent_operation_is_durable_and_idempotently_abortable() {
        let app = tempfile::tempdir().unwrap();
        let database = crate::state_db::StateDatabase::open(app.path()).unwrap();
        let operation = database
            .prepare_filesystem_operation(
                "workspace_pack_apply",
                &WorkspacePackApplyOperation {
                    revision: "a".repeat(64),
                    expected_created: Vec::new(),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            database
                .pending_filesystem_operations()
                .await
                .unwrap()
                .len(),
            1
        );
        database
            .abort_filesystem_operation(&operation.id)
            .await
            .unwrap();
        database
            .abort_filesystem_operation(&operation.id)
            .await
            .unwrap();
        assert!(database
            .pending_filesystem_operations()
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn workspace_pack_plan_expands_dependencies_and_writes_nothing() {
        use crate::commands::settings::{Settings, SettingsLoadState};

        let app_data = tempfile::tempdir().unwrap();
        let source_root = tempfile::tempdir().unwrap();
        let tool_home = tempfile::tempdir().unwrap();
        std::fs::write(
            source_root.path().join("base.md"),
            "---\nname: Base\ndescription: Foundation.\n---\nBase.\n",
        )
        .unwrap();
        std::fs::write(
            source_root.path().join("lead.md"),
            "---\nname: Lead\ndescription: Leads.\nrequired-agents: [base.md]\n---\nLead.\n",
        )
        .unwrap();
        let source = crate::agents::add_local_source(app_data.path(), source_root.path())
            .await
            .unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app_data.path().to_path_buf();
        let mut settings = Settings::default();
        settings.tool_paths.insert(
            "claudeCode".into(),
            tool_home.path().to_string_lossy().into_owned(),
        );
        *state.settings.write().await = SettingsLoadState::Loaded(settings);
        let pack = WorkspacePack {
            workspace_pack: 1,
            name: "Local review".into(),
            scope: WorkspacePackScope::User,
            agents: vec![WorkspacePackAgent {
                reference: AgentReference {
                    source_id: source.id,
                    relative_path: "lead.md".into(),
                },
                tool: "claudeCode".into(),
            }],
            skills: Vec::new(),
            runbook: Some("review-flow".into()),
            instructions: vec!["Follow AGENTS.md".into()],
            mcp_servers: vec!["memory".into()],
        };

        let plan = build_workspace_pack_plan(&state, pack, None).await.unwrap();

        assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
        assert_eq!(plan.agents.len(), 2);
        assert!(plan.agents.iter().any(|item| item.dependency));
        assert!(plan.agents.iter().all(|item| item.state == "missing"));
        assert_eq!(plan.rollback_scope.len(), 2);
        assert_eq!(plan.revision.len(), 64);
        assert!(plan
            .warnings
            .iter()
            .any(|warning| warning.contains("Instruction")));
        assert!(plan.warnings.iter().any(|warning| warning.contains("MCP")));
        assert!(plan
            .agents
            .iter()
            .flat_map(|item| &item.destinations)
            .all(|destination| !Path::new(destination).exists()));
        assert!(!ledger_path_for(app_data.path()).exists());
        assert!(!crate::skills::install::ledger_path(app_data.path()).exists());
    }

    /// A minimal ledger row for the prune test.
    fn row(slug: &str, tool: &str, project: Option<&str>) -> InstallRecord {
        InstallRecord {
            slug: slug.into(),
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: format!("engineering/{slug}.md"),
            tool: tool.to_string(),
            scope: render::scope_for(project.map(Path::new)),
            project_path: project.map(String::from),
            dest: format!("/dest/{slug}"),
            source_hash: String::new(),
            body_hash: String::new(),
            rendered_hash: String::new(),
            artifacts: Vec::new(),
            disabled_path: None,
            source_snapshot_hash: String::new(),
            capabilities: Vec::new(),
            publisher_key: None,
            publisher_verified: false,
            installed_at: String::new(),
            corpus_version: String::new(),
            deployment_notice: None,
        }
    }

    fn recovery_record(project: &Path, rendered_hash: String) -> InstallRecord {
        let destination = project.join(".codex/agents/frontend-developer.toml");
        InstallRecord {
            slug: "frontend-developer".into(),
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/frontend-developer.md".into(),
            tool: "codex".into(),
            scope: crate::types::Scope::Project,
            project_path: Some(project.to_string_lossy().into_owned()),
            dest: destination.to_string_lossy().into_owned(),
            source_hash: "a".repeat(64),
            body_hash: "b".repeat(64),
            rendered_hash,
            artifacts: Vec::new(),
            disabled_path: None,
            source_snapshot_hash: "a".repeat(64),
            capabilities: Vec::new(),
            publisher_key: None,
            publisher_verified: false,
            installed_at: "2026-08-06T00:00:00Z".into(),
            corpus_version: "1".into(),
            deployment_notice: None,
        }
    }

    async fn completed_sqlite_kimi_install(
        disabled: bool,
    ) -> (
        tempfile::TempDir,
        tempfile::TempDir,
        crate::state_db::StateDatabase,
        AppState,
        InstallRecord,
        Vec<PathBuf>,
        Vec<String>,
    ) {
        use crate::commands::settings::{Settings, SettingsLoadState};

        let app = tempfile::tempdir().unwrap();
        let tool_home = tempfile::tempdir().unwrap();
        let canonical_home = std::fs::canonicalize(tool_home.path()).unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app.path().to_path_buf();
        let mut settings = Settings::default();
        settings
            .tool_paths
            .insert("kimi".into(), canonical_home.to_string_lossy().into_owned());
        *state.settings.write().await = SettingsLoadState::Loaded(settings);
        let database = crate::state_db::StateDatabase::open(app.path()).unwrap();
        database
            .mutate(installs_spec(), Vec::new(), |_| Ok(()))
            .await
            .unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let paths = render::dests("kimi", "frontend-developer", &canonical_home, None).unwrap();
        let contents = vec!["managed agent yaml".to_owned(), "managed system".to_owned()];
        for (path, content) in paths.iter().zip(&contents) {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        let mut record =
            recovery_record(&canonical_home, render::sha256_hex(contents[0].as_bytes()));
        record.tool = "kimi".into();
        record.scope = crate::types::Scope::User;
        record.project_path = None;
        record.dest = paths[0].to_string_lossy().into_owned();
        record.artifacts = paths
            .iter()
            .zip(&contents)
            .map(|(path, content)| InstallArtifact {
                dest: path.to_string_lossy().into_owned(),
                rendered_hash: render::sha256_hex(content.as_bytes()),
                disabled_path: None,
            })
            .collect();
        if disabled {
            let disabled_paths = paths
                .iter()
                .map(|path| disabled_destination(path).unwrap())
                .collect::<Vec<_>>();
            for (active, disabled) in paths.iter().zip(&disabled_paths) {
                std::fs::rename(active, disabled).unwrap();
            }
            set_record_disabled_paths(&mut record, Some(&disabled_paths));
        }
        database
            .mutate(installs_spec(), Vec::new(), {
                let record = record.clone();
                move |records| {
                    records.push(InstallLedgerEntry::Agent(record));
                    Ok(())
                }
            })
            .await
            .unwrap();
        (app, tool_home, database, state, record, paths, contents)
    }

    async fn filesystem_applied_project_disable() -> (
        tempfile::TempDir,
        tempfile::TempDir,
        crate::state_db::StateDatabase,
        AppState,
        InstallRecord,
        InstallRecord,
        PathBuf,
        PathBuf,
    ) {
        let app = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let project_root = std::fs::canonicalize(project.path()).unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app.path().to_path_buf();
        let database = crate::state_db::StateDatabase::open(app.path()).unwrap();
        database
            .mutate(installs_spec(), Vec::new(), |_| Ok(()))
            .await
            .unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let active = project_root.join(".codex/agents/frontend-developer.toml");
        let disabled = disabled_destination(&active).unwrap();
        std::fs::create_dir_all(active.parent().unwrap()).unwrap();
        std::fs::write(&active, b"managed project agent").unwrap();
        let previous = recovery_record(&project_root, render::sha256_hex(b"managed project agent"));
        let mut next = previous.clone();
        set_record_disabled_paths(&mut next, Some(std::slice::from_ref(&disabled)));
        database
            .mutate(installs_spec(), Vec::new(), {
                let previous = previous.clone();
                move |records| {
                    records.push(InstallLedgerEntry::Agent(previous));
                    Ok(())
                }
            })
            .await
            .unwrap();
        let operation = database
            .prepare_filesystem_operation(
                "agent_disable",
                &AgentMoveOperation {
                    previous: previous.clone(),
                    next: next.clone(),
                    active: vec![active.to_string_lossy().into_owned()],
                    disabled: vec![disabled.to_string_lossy().into_owned()],
                },
            )
            .await
            .unwrap();
        std::fs::rename(&active, &disabled).unwrap();
        save_ledger_after_filesystem(&state, std::slice::from_ref(&next), &operation.id)
            .await
            .unwrap();
        (
            app, project, database, state, previous, next, active, disabled,
        )
    }

    fn inject_agent_ledger_failure(app_data_dir: &Path) -> rusqlite::Connection {
        let connection =
            rusqlite::Connection::open(app_data_dir.join("state/agency-agents.sqlite3")).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_agent_lifecycle_ledger \
                 BEFORE UPDATE ON state_documents \
                 WHEN NEW.name = 'installs' \
                 BEGIN SELECT RAISE(FAIL, 'injected Agent ledger failure'); END;",
            )
            .unwrap();
        connection
    }

    fn install_reference(record: &InstallRecord) -> AgentReference {
        AgentReference {
            source_id: record.source_id.clone(),
            relative_path: record.relative_path.clone(),
        }
    }

    async fn assert_exact_kimi_lifecycle_state(
        state: &AppState,
        database: &crate::state_db::StateDatabase,
        record: &InstallRecord,
        active: &[PathBuf],
        contents: &[String],
        disabled: bool,
    ) {
        let records = load_ledger_for_state(state).await.unwrap();
        assert_eq!(records.len(), 1);
        assert!(exact_agent_install(&records[0], record));
        for (path, content) in active.iter().zip(contents) {
            let expected = if disabled {
                disabled_destination(path).unwrap()
            } else {
                path.clone()
            };
            let absent = if disabled {
                path.clone()
            } else {
                disabled_destination(path).unwrap()
            };
            assert_eq!(std::fs::read(&expected).unwrap(), content.as_bytes());
            assert!(!absent.exists());
        }
        assert!(database
            .pending_filesystem_operations()
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn multi_artifact_disable_ledger_failure_restores_every_file_row_and_journal() {
        let (app, _home, database, state, record, paths, contents) =
            completed_sqlite_kimi_install(false).await;
        let connection = inject_agent_ledger_failure(app.path());

        let result = mcp_move_agent_install(
            &state,
            install_reference(&record),
            record.tool.clone(),
            None,
            false,
            None,
        )
        .await;
        connection
            .execute_batch("DROP TRIGGER fail_agent_lifecycle_ledger;")
            .unwrap();

        assert!(result.is_err());
        assert_exact_kimi_lifecycle_state(&state, &database, &record, &paths, &contents, false)
            .await;
    }

    #[tokio::test]
    async fn multi_artifact_enable_ledger_failure_restores_every_file_row_and_journal() {
        let (app, _home, database, state, record, paths, contents) =
            completed_sqlite_kimi_install(true).await;
        let connection = inject_agent_ledger_failure(app.path());

        let result = mcp_move_agent_install(
            &state,
            install_reference(&record),
            record.tool.clone(),
            None,
            true,
            None,
        )
        .await;
        connection
            .execute_batch("DROP TRIGGER fail_agent_lifecycle_ledger;")
            .unwrap();

        assert!(result.is_err());
        assert_exact_kimi_lifecycle_state(&state, &database, &record, &paths, &contents, true)
            .await;
    }

    #[tokio::test]
    async fn multi_artifact_uninstall_ledger_failure_restores_every_file_row_and_journal() {
        let (app, _home, database, state, record, paths, contents) =
            completed_sqlite_kimi_install(false).await;
        let connection = inject_agent_ledger_failure(app.path());

        let result = do_uninstall(
            &state,
            install_reference(&record),
            record.tool.clone(),
            None,
        )
        .await;
        connection
            .execute_batch("DROP TRIGGER fail_agent_lifecycle_ledger;")
            .unwrap();

        assert!(result.is_err());
        assert_exact_kimi_lifecycle_state(&state, &database, &record, &paths, &contents, false)
            .await;
    }

    #[tokio::test]
    async fn multi_artifact_rollback_ledger_failure_restores_every_file_row_and_journal() {
        let (app, _home, database, state, record, paths, contents) =
            completed_sqlite_kimi_install(false).await;
        let selected_contents = vec![b"old agent yaml".to_vec(), b"old system".to_vec()];
        let snapshot = history::create_snapshot_from_bytes(
            app.path(),
            &install_identity(&record),
            &selected_contents,
            &"d".repeat(64),
            &render::sha256_hex(&selected_contents[0]),
            "2020-01-01T00:00:00Z",
        )
        .await
        .unwrap();
        let connection = inject_agent_ledger_failure(app.path());

        let result = rollback_agent_version(
            &state,
            install_reference(&record),
            record.tool.clone(),
            None,
            snapshot.id,
        )
        .await;
        connection
            .execute_batch("DROP TRIGGER fail_agent_lifecycle_ledger;")
            .unwrap();

        assert!(result.is_err());
        assert_exact_kimi_lifecycle_state(&state, &database, &record, &paths, &contents, false)
            .await;
    }

    #[tokio::test]
    async fn configured_tool_base_change_preserves_recorded_manifest_across_lifecycle() {
        use crate::commands::settings::{Settings, SettingsLoadState};

        let (_app, _old_home, database, state, record, paths, _contents) =
            completed_sqlite_kimi_install(false).await;
        let new_home = tempfile::tempdir().unwrap();
        let mut settings = Settings::default();
        settings.tool_paths.insert(
            "kimi".into(),
            std::fs::canonicalize(new_home.path())
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        );
        *state.settings.write().await = SettingsLoadState::Loaded(settings);
        let new_paths = render::dests("kimi", "frontend-developer", new_home.path(), None).unwrap();

        assert_eq!(resolved_record_paths(&state, &record).await.unwrap(), paths);
        let mut tampered = record.clone();
        tampered.artifacts[1].dest = new_paths[1].to_string_lossy().into_owned();
        assert!(resolved_record_paths(&state, &tampered).await.is_err());
        let (expected, active, disabled) = record_reconcile_hashes(&state, &record).await.unwrap();
        assert_eq!(
            active,
            expected.iter().cloned().map(Some).collect::<Vec<_>>()
        );
        assert!(disabled.iter().all(Option::is_none));
        assert_eq!(
            classify_artifact_install(
                &active.iter().map(Option::as_deref).collect::<Vec<_>>(),
                &disabled.iter().map(Option::as_deref).collect::<Vec<_>>(),
                &expected.iter().map(String::as_str).collect::<Vec<_>>(),
                false,
                &record.source_snapshot_hash,
                Some(&record.source_snapshot_hash),
            ),
            InstallState::Current
        );

        let disabled = mcp_move_agent_install(
            &state,
            install_reference(&record),
            record.tool.clone(),
            None,
            false,
            None,
        )
        .await
        .unwrap();
        assert!(disabled.disabled_path.is_some());
        let enabled = mcp_move_agent_install(
            &state,
            install_reference(&record),
            record.tool.clone(),
            None,
            true,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            enabled
                .artifacts
                .iter()
                .map(|artifact| PathBuf::from(&artifact.dest))
                .collect::<Vec<_>>(),
            paths
        );

        let rollback_contents = vec![b"old agent yaml".to_vec(), b"old system".to_vec()];
        let snapshot = history::create_snapshot_from_bytes(
            &state.app_data_dir,
            &install_identity(&enabled),
            &rollback_contents,
            &"d".repeat(64),
            &render::sha256_hex(&rollback_contents[0]),
            "2020-01-01T00:00:00Z",
        )
        .await
        .unwrap();
        rollback_agent_version(
            &state,
            install_reference(&record),
            record.tool.clone(),
            None,
            snapshot.id,
        )
        .await
        .unwrap();
        for (path, expected) in paths.iter().zip(&rollback_contents) {
            assert_eq!(std::fs::read(path).unwrap(), *expected);
        }
        assert!(new_paths.iter().all(|path| !path.exists()));

        do_uninstall(
            &state,
            install_reference(&record),
            record.tool.clone(),
            None,
        )
        .await
        .unwrap();
        assert!(paths.iter().all(|path| !path.exists()));
        assert!(new_paths.iter().all(|path| !path.exists()));
        assert!(load_ledger_for_state(&state).await.unwrap().is_empty());
        assert!(database
            .pending_filesystem_operations()
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn post_ledger_commit_failure_is_finished_before_same_identity_mutates_again() {
        let (app, _home, database, state, record, paths, contents) =
            completed_sqlite_kimi_install(false).await;
        let connection =
            rusqlite::Connection::open(app.path().join("state/agency-agents.sqlite3")).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_agent_journal_commit \
                 BEFORE UPDATE ON filesystem_operations \
                 WHEN OLD.phase = 'filesystem_applied' AND NEW.phase = 'committed' \
                 BEGIN SELECT RAISE(FAIL, 'injected Agent journal commit failure'); END;",
            )
            .unwrap();

        let first = mcp_move_agent_install(
            &state,
            install_reference(&record),
            record.tool.clone(),
            None,
            false,
            None,
        )
        .await;
        assert!(first.is_err());
        assert_eq!(
            database.pending_filesystem_operations().await.unwrap()[0].phase,
            crate::state_db::FilesystemOperationPhase::FilesystemApplied
        );
        connection
            .execute_batch("DROP TRIGGER fail_agent_journal_commit;")
            .unwrap();

        mcp_move_agent_install(
            &state,
            install_reference(&record),
            record.tool.clone(),
            None,
            true,
            None,
        )
        .await
        .unwrap();
        recover_agent_operations(&state).await.unwrap();
        recover_agent_operations(&state).await.unwrap();

        assert_exact_kimi_lifecycle_state(&state, &database, &record, &paths, &contents, false)
            .await;
    }

    #[tokio::test]
    async fn project_forget_finishes_pending_agent_operation_before_pruning() {
        let (_app, project, database, state, _previous, _next, active, disabled) =
            filesystem_applied_project_disable().await;
        let project_path = std::fs::canonicalize(project.path()).unwrap();

        project_forget_for_state(&state, &project_path.to_string_lossy())
            .await
            .unwrap();
        recover_agent_operations(&state).await.unwrap();

        assert!(load_ledger_for_state(&state).await.unwrap().is_empty());
        assert!(!active.exists());
        assert_eq!(std::fs::read(disabled).unwrap(), b"managed project agent");
        assert!(database
            .pending_filesystem_operations()
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn adoption_commit_recovers_pending_operation_and_preserves_concurrent_ledger_change() {
        let (_app, project, database, state, _previous, next, _active, _disabled) =
            filesystem_applied_project_disable().await;
        let project_root = std::fs::canonicalize(project.path()).unwrap();
        let mut adopted = recovery_record(&project_root, render::sha256_hex(b"adopted"));
        adopted.slug = "adopted".into();
        adopted.relative_path = "engineering/adopted.md".into();
        adopted.dest = project_root
            .join(".codex/agents/adopted.toml")
            .to_string_lossy()
            .into_owned();
        std::fs::write(&adopted.dest, b"adopted").unwrap();
        let mut concurrent = recovery_record(&project_root, render::sha256_hex(b"concurrent"));
        concurrent.slug = "concurrent".into();
        concurrent.relative_path = "engineering/concurrent.md".into();
        concurrent.dest = project_root
            .join(".codex/agents/concurrent.toml")
            .to_string_lossy()
            .into_owned();
        std::fs::write(&concurrent.dest, b"concurrent").unwrap();
        database
            .mutate(installs_spec(), Vec::new(), {
                let concurrent = concurrent.clone();
                move |records| {
                    records.push(InstallLedgerEntry::Agent(concurrent));
                    Ok(())
                }
            })
            .await
            .unwrap();

        commit_agent_adoptions(&state, vec![adopted.clone()])
            .await
            .unwrap();
        recover_agent_operations(&state).await.unwrap();

        let records = load_ledger_for_state(&state).await.unwrap();
        assert_eq!(records.len(), 3);
        assert!(records
            .iter()
            .any(|record| exact_agent_install(record, &next)));
        assert!(records
            .iter()
            .any(|record| exact_agent_install(record, &concurrent)));
        assert!(records
            .iter()
            .any(|record| exact_agent_install(record, &adopted)));
        assert!(database
            .pending_filesystem_operations()
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn adoption_commit_rejects_candidate_changed_after_scan_without_tracking_it() {
        let app = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let project_root = std::fs::canonicalize(project.path()).unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app.path().to_path_buf();
        let database = crate::state_db::StateDatabase::open(app.path()).unwrap();
        database
            .mutate(installs_spec(), Vec::new(), |_| Ok(()))
            .await
            .unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let mut adopted = recovery_record(&project_root, render::sha256_hex(b"scanned"));
        adopted.slug = "stale-adoption".into();
        adopted.relative_path = "engineering/stale-adoption.md".into();
        adopted.dest = project_root
            .join(".codex/agents/stale-adoption.toml")
            .to_string_lossy()
            .into_owned();
        std::fs::create_dir_all(Path::new(&adopted.dest).parent().unwrap()).unwrap();
        std::fs::write(&adopted.dest, b"scanned").unwrap();

        // The passive scan already captured `adopted`; the candidate changes
        // before the serialized commit acquires the lifecycle locks.
        std::fs::write(&adopted.dest, b"changed after scan").unwrap();
        let error = commit_agent_adoptions(&state, vec![adopted])
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            AppError::InvalidArgument { message }
                if message.contains("changed during Agent adoption")
        ));
        assert!(load_ledger_for_state(&state).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn prepared_agent_install_recovery_rolls_forward_exact_content_once() {
        let app = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app.path().to_path_buf();
        let database = crate::state_db::StateDatabase::open(app.path()).unwrap();
        database
            .mutate(installs_spec(), Vec::new(), |_| Ok(()))
            .await
            .unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let rendered = "[agent]\nname = \"Frontend Developer\"\n".to_owned();
        let hash = render::sha256_hex(rendered.as_bytes());
        let project_root = std::fs::canonicalize(project.path()).unwrap();
        let destination = project_root.join(".codex/agents/frontend-developer.toml");
        let next = recovery_record(&project_root, hash);
        let operation = database
            .prepare_filesystem_operation(
                "agent_install",
                &AgentInstallOperation {
                    previous: None,
                    next: next.clone(),
                    targets: vec![destination.to_string_lossy().into_owned()],
                    rendered: AgentRenderedPayload::One(rendered.clone()),
                },
            )
            .await
            .unwrap();
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::write(&destination, rendered).unwrap();

        recover_agent_operations(&state).await.unwrap();
        recover_agent_operations(&state).await.unwrap();

        let records = load_ledger_for_state(&state).await.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].source_id, next.source_id);
        assert_eq!(records[0].relative_path, next.relative_path);
        assert_eq!(records[0].rendered_hash, next.rendered_hash);
        assert!(database
            .pending_filesystem_operations()
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            operation.phase,
            crate::state_db::FilesystemOperationPhase::Prepared
        );
    }

    #[tokio::test]
    async fn prepared_multi_artifact_rollback_recovers_every_file_and_ledger_once() {
        use crate::commands::settings::{Settings, SettingsLoadState};

        let app = tempfile::tempdir().unwrap();
        let tool_home = tempfile::tempdir().unwrap();
        let tool_home = std::fs::canonicalize(tool_home.path()).unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app.path().to_path_buf();
        let mut settings = Settings::default();
        settings
            .tool_paths
            .insert("kimi".into(), tool_home.to_string_lossy().into_owned());
        *state.settings.write().await = SettingsLoadState::Loaded(settings);
        let database = crate::state_db::StateDatabase::open(app.path()).unwrap();
        database
            .mutate(installs_spec(), Vec::new(), |_| Ok(()))
            .await
            .unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();

        let paths = render::dests("kimi", "frontend-developer", &tool_home, None).unwrap();
        let old = ["old agent yaml", "old system instructions"];
        let next_content = ["new agent yaml", "new system instructions"];
        for (path, content) in paths.iter().zip(old) {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        let mut previous = recovery_record(&tool_home, render::sha256_hex(old[0].as_bytes()));
        previous.tool = "kimi".into();
        previous.scope = crate::types::Scope::User;
        previous.project_path = None;
        previous.dest = paths[0].to_string_lossy().into_owned();
        previous.artifacts = paths
            .iter()
            .zip(old)
            .map(|(path, content)| InstallArtifact {
                dest: path.to_string_lossy().into_owned(),
                rendered_hash: render::sha256_hex(content.as_bytes()),
                disabled_path: None,
            })
            .collect();
        let mut next = previous.clone();
        next.source_hash = "d".repeat(64);
        next.source_snapshot_hash = next.source_hash.clone();
        next.rendered_hash = render::sha256_hex(next_content[0].as_bytes());
        for (artifact, content) in next.artifacts.iter_mut().zip(next_content) {
            artifact.rendered_hash = render::sha256_hex(content.as_bytes());
        }
        database
            .mutate(installs_spec(), Vec::new(), {
                let previous = previous.clone();
                move |records| {
                    records.push(InstallLedgerEntry::Agent(previous));
                    Ok(())
                }
            })
            .await
            .unwrap();
        database
            .prepare_filesystem_operation(
                "agent_update",
                &AgentInstallOperation {
                    previous: Some(previous),
                    next: next.clone(),
                    targets: paths
                        .iter()
                        .map(|path| path.to_string_lossy().into_owned())
                        .collect(),
                    rendered: AgentRenderedPayload::Many(
                        next_content.iter().map(ToString::to_string).collect(),
                    ),
                },
            )
            .await
            .unwrap();
        std::fs::write(&paths[0], next_content[0]).unwrap();

        recover_agent_operations(&state).await.unwrap();
        recover_agent_operations(&state).await.unwrap();

        assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), next_content[0]);
        assert_eq!(std::fs::read_to_string(&paths[1]).unwrap(), next_content[1]);
        let records = load_ledger_for_state(&state).await.unwrap();
        assert_eq!(records.len(), 1);
        assert!(exact_agent_install(&records[0], &next));
        assert!(database
            .pending_filesystem_operations()
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn multi_artifact_recovery_is_idempotent_across_prepared_and_applied_phases() {
        let (_app, _home, database, state, active_record, active, contents) =
            completed_sqlite_kimi_install(false).await;
        let disabled = active
            .iter()
            .map(|path| disabled_destination(path).unwrap())
            .collect::<Vec<_>>();
        let mut disabled_record = active_record.clone();
        set_record_disabled_paths(&mut disabled_record, Some(&disabled));
        database
            .prepare_filesystem_operation(
                "agent_disable",
                &AgentMoveOperation {
                    previous: active_record.clone(),
                    next: disabled_record.clone(),
                    active: active
                        .iter()
                        .map(|path| path.to_string_lossy().into_owned())
                        .collect(),
                    disabled: disabled
                        .iter()
                        .map(|path| path.to_string_lossy().into_owned())
                        .collect(),
                },
            )
            .await
            .unwrap();
        std::fs::rename(&active[0], &disabled[0]).unwrap();

        recover_agent_operations(&state).await.unwrap();
        recover_agent_operations(&state).await.unwrap();
        assert_exact_kimi_lifecycle_state(
            &state,
            &database,
            &disabled_record,
            &active,
            &contents,
            true,
        )
        .await;

        let operation = database
            .prepare_filesystem_operation(
                "agent_enable",
                &AgentMoveOperation {
                    previous: disabled_record,
                    next: active_record.clone(),
                    active: active
                        .iter()
                        .map(|path| path.to_string_lossy().into_owned())
                        .collect(),
                    disabled: disabled
                        .iter()
                        .map(|path| path.to_string_lossy().into_owned())
                        .collect(),
                },
            )
            .await
            .unwrap();
        let hashes = record_rendered_hashes(&active_record, active.len());
        move_managed_artifacts(&disabled, &active, &hashes).unwrap();
        save_ledger_after_filesystem(&state, std::slice::from_ref(&active_record), &operation.id)
            .await
            .unwrap();

        recover_agent_operations(&state).await.unwrap();
        recover_agent_operations(&state).await.unwrap();
        assert_exact_kimi_lifecycle_state(
            &state,
            &database,
            &active_record,
            &active,
            &contents,
            false,
        )
        .await;

        let operation_hashes = active
            .iter()
            .map(|path| recovery_file_hash(path).unwrap())
            .collect::<Vec<_>>();
        database
            .prepare_filesystem_operation(
                "agent_uninstall",
                &AgentUninstallOperation {
                    previous: active_record,
                    paths: active
                        .iter()
                        .map(|path| path.to_string_lossy().into_owned())
                        .collect(),
                    hashes: operation_hashes,
                },
            )
            .await
            .unwrap();
        std::fs::remove_file(&active[0]).unwrap();

        recover_agent_operations(&state).await.unwrap();
        recover_agent_operations(&state).await.unwrap();
        assert!(active.iter().all(|path| !path.exists()));
        assert!(load_ledger_for_state(&state).await.unwrap().is_empty());
        assert!(database
            .pending_filesystem_operations()
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn rollback_oldest_multi_artifact_snapshot_at_history_cap() {
        use crate::commands::settings::{Settings, SettingsLoadState};

        let app = tempfile::tempdir().unwrap();
        let tool_home = tempfile::tempdir().unwrap();
        let tool_home = std::fs::canonicalize(tool_home.path()).unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app.path().to_path_buf();
        let mut settings = Settings::default();
        settings
            .tool_paths
            .insert("kimi".into(), tool_home.to_string_lossy().into_owned());
        *state.settings.write().await = SettingsLoadState::Loaded(settings);
        let database = crate::state_db::StateDatabase::open(app.path()).unwrap();
        database
            .mutate(installs_spec(), Vec::new(), |_| Ok(()))
            .await
            .unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();

        let reference = AgentReference {
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/frontend-developer.md".into(),
        };
        let paths = render::dests("kimi", "frontend-developer", &tool_home, None).unwrap();
        let active = ["current agent yaml", "current system instructions"];
        for (path, content) in paths.iter().zip(active) {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        let mut record = recovery_record(&tool_home, render::sha256_hex(active[0].as_bytes()));
        record.tool = "kimi".into();
        record.scope = crate::types::Scope::User;
        record.project_path = None;
        record.dest = paths[0].to_string_lossy().into_owned();
        record.artifacts = paths
            .iter()
            .zip(active)
            .map(|(path, content)| InstallArtifact {
                dest: path.to_string_lossy().into_owned(),
                rendered_hash: render::sha256_hex(content.as_bytes()),
                disabled_path: None,
            })
            .collect();
        let identity = install_identity(&record);
        let selected_contents = vec![b"oldest agent yaml".to_vec(), b"oldest system".to_vec()];
        let mut selected = None;
        for index in 0..MAX_AGENT_HISTORY_ENTRIES {
            let contents = if index == 0 {
                selected_contents.clone()
            } else {
                vec![
                    format!("agent version {index}").into_bytes(),
                    format!("system version {index}").into_bytes(),
                ]
            };
            let snapshot = history::create_snapshot_from_bytes(
                app.path(),
                &identity,
                &contents,
                &format!("{:064x}", index + 1),
                &render::sha256_hex(&contents[0]),
                &format!("2020-01-{:02}T00:00:00Z", index + 1),
            )
            .await
            .unwrap();
            if index == 0 {
                selected = Some(snapshot);
            }
        }
        database
            .mutate(installs_spec(), Vec::new(), {
                let record = record.clone();
                move |records| {
                    records.push(InstallLedgerEntry::Agent(record));
                    Ok(())
                }
            })
            .await
            .unwrap();
        let selected = selected.unwrap();

        let restored =
            rollback_agent_version(&state, reference, "kimi".into(), None, selected.id.clone())
                .await
                .expect(
                    "oldest retained multi-artifact snapshot must survive safety snapshot pruning",
                );

        assert_eq!(std::fs::read(&paths[0]).unwrap(), selected_contents[0]);
        assert_eq!(std::fs::read(&paths[1]).unwrap(), selected_contents[1]);
        assert_eq!(
            restored.artifacts[0].rendered_hash,
            render::sha256_hex(&selected_contents[0])
        );
        assert_eq!(
            restored.artifacts[1].rendered_hash,
            render::sha256_hex(&selected_contents[1])
        );
        assert!(database
            .pending_filesystem_operations()
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn prepared_agent_move_and_uninstall_recover_once() {
        let app = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let project = std::fs::canonicalize(project.path()).unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app.path().to_path_buf();
        let database = crate::state_db::StateDatabase::open(app.path()).unwrap();
        let rendered = "[agent]\nname = \"Frontend Developer\"\n";
        let hash = render::sha256_hex(rendered.as_bytes());
        let previous = recovery_record(&project, hash.clone());
        database
            .mutate(installs_spec(), Vec::new(), {
                let previous = previous.clone();
                move |records| {
                    records.push(InstallLedgerEntry::Agent(previous));
                    Ok(())
                }
            })
            .await
            .unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let active = PathBuf::from(&previous.dest);
        let disabled = disabled_destination(&active).unwrap();
        std::fs::create_dir_all(active.parent().unwrap()).unwrap();
        std::fs::write(&active, rendered).unwrap();
        let mut disabled_record = previous.clone();
        disabled_record.disabled_path = Some(disabled.to_string_lossy().into_owned());
        database
            .prepare_filesystem_operation(
                "agent_disable",
                &AgentMoveOperation {
                    previous: previous.clone(),
                    next: disabled_record.clone(),
                    active: vec![active.to_string_lossy().into_owned()],
                    disabled: vec![disabled.to_string_lossy().into_owned()],
                },
            )
            .await
            .unwrap();

        recover_agent_operations(&state).await.unwrap();
        recover_agent_operations(&state).await.unwrap();
        assert!(!active.exists());
        assert_eq!(std::fs::read(&disabled).unwrap(), rendered.as_bytes());
        assert_eq!(
            load_ledger_for_state(&state).await.unwrap()[0].disabled_path,
            disabled_record.disabled_path
        );

        database
            .prepare_filesystem_operation(
                "agent_uninstall",
                &AgentUninstallOperation {
                    previous: disabled_record,
                    paths: vec![disabled.to_string_lossy().into_owned()],
                    hashes: vec![Some(hash)],
                },
            )
            .await
            .unwrap();
        recover_agent_operations(&state).await.unwrap();
        recover_agent_operations(&state).await.unwrap();
        assert!(!disabled.exists());
        assert!(load_ledger_for_state(&state).await.unwrap().is_empty());
        assert!(database
            .pending_filesystem_operations()
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn prepared_agent_uninstall_retains_changed_content() {
        let app = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let project = std::fs::canonicalize(project.path()).unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app.path().to_path_buf();
        let database = crate::state_db::StateDatabase::open(app.path()).unwrap();
        let expected = "expected";
        let record = recovery_record(&project, render::sha256_hex(expected.as_bytes()));
        database
            .mutate(installs_spec(), Vec::new(), {
                let record = record.clone();
                move |records| {
                    records.push(InstallLedgerEntry::Agent(record));
                    Ok(())
                }
            })
            .await
            .unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let destination = PathBuf::from(&record.dest);
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::write(&destination, "changed").unwrap();
        let operation = database
            .prepare_filesystem_operation(
                "agent_uninstall",
                &AgentUninstallOperation {
                    previous: record,
                    paths: vec![destination.to_string_lossy().into_owned()],
                    hashes: vec![Some(render::sha256_hex(expected.as_bytes()))],
                },
            )
            .await
            .unwrap();

        assert!(recover_agent_operations(&state).await.is_err());
        assert_eq!(std::fs::read_to_string(&destination).unwrap(), "changed");
        assert!(database
            .pending_filesystem_operations()
            .await
            .unwrap()
            .iter()
            .any(|pending| pending.id == operation.id && pending.recovery_error.is_some()));
    }

    fn built_in_result(paths: &[(&str, &str)]) -> crate::types::AgentSourceResult {
        crate::types::AgentSourceResult {
            source: crate::types::AgentSource {
                id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
                label: "Agency Agents".into(),
                enabled: true,
                kind: crate::types::AgentSourceKind::BuiltIn,
            },
            agents: paths
                .iter()
                .map(|(slug, relative_path)| crate::types::AgentPackageResult {
                    reference: crate::types::AgentReference {
                        source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
                        relative_path: (*relative_path).into(),
                    },
                    agent: Some(crate::types::Agent {
                        slug: (*slug).into(),
                        name: (*slug).into(),
                        description: String::new(),
                        category: relative_path.split('/').next().unwrap_or_default().into(),
                        emoji: None,
                        color: None,
                        vibe: None,
                        body: String::new(),
                    }),
                    source_hash: format!("source-{slug}"),
                    frontmatter_hash: String::new(),
                    body_hash: String::new(),
                    version: None,
                    channel: None,
                    changelog: None,
                    publisher: None,
                    publisher_key: None,
                    publisher_verified: false,
                    required_agents: Vec::new(),
                    required_skills: Vec::new(),
                    recommended_agents: Vec::new(),
                    groups: Vec::new(),
                    tags: Vec::new(),
                    capabilities: Vec::new(),
                    permissions: Vec::new(),
                    quality_score: 0,
                    quality_checks: Vec::new(),
                    diagnostics: Vec::new(),
                    installable: true,
                })
                .collect(),
            errors: Vec::new(),
            revision: "builtin-revision".into(),
        }
    }

    fn legacy_row(slug: &str, dest: &str) -> InstallRecord {
        serde_json::from_value(serde_json::json!({
            "slug": slug,
            "tool": "codex",
            "scope": "user",
            "projectPath": null,
            "dest": dest,
            "sourceHash": "old-source",
            "bodyHash": "old-body",
            "renderedHash": "old-render",
            "installedAt": "2026-06-05T00:00:00Z",
            "corpusVersion": "old-version"
        }))
        .unwrap()
    }

    #[test]
    fn migration_resolves_only_unique_builtin_slugs() {
        let built_in = built_in_result(&[
            ("unique", "engineering/unique.md"),
            ("duplicate", "one/duplicate.md"),
            ("duplicate", "two/duplicate.md"),
        ]);
        let records = vec![
            legacy_row("unique", "/dest/unique.toml"),
            legacy_row("duplicate", "/dest/duplicate.toml"),
            legacy_row("unknown", "/dest/unknown.toml"),
        ];

        let (migrated, changed) = migrate_install_records(records, Some(&built_in)).unwrap();

        assert!(changed);
        assert_eq!(
            migrated[0].source_id,
            crate::agents::BUILTIN_AGENT_SOURCE_ID
        );
        assert_eq!(migrated[0].relative_path, "engineering/unique.md");
        assert_eq!(migrated[0].source_snapshot_hash, "old-source");
        for row in &migrated[1..] {
            assert_eq!(row.source_id, "legacy:unresolved");
            assert!(row.relative_path.starts_with("legacy/"));
            assert!(row.relative_path.ends_with(".md"));
        }
        assert_ne!(migrated[1].relative_path, migrated[2].relative_path);
    }

    #[test]
    fn migration_normalizes_windows_and_unix_destination_filenames() {
        let windows = unresolved_relative_path(&legacy_row(
            "reviewer",
            r"C:\Users\Example\.codex\agents\Reviewer.MD",
        ));
        let unix = unresolved_relative_path(&legacy_row(
            "reviewer",
            "/Users/Example/.codex/agents/Reviewer.MD",
        ));
        assert!(windows.starts_with("legacy/reviewer-"));
        assert!(unix.starts_with("legacy/reviewer-"));
        assert_ne!(
            windows, unix,
            "distinct portable paths remain collision-free"
        );
    }

    #[test]
    fn migration_preserves_valid_rows_and_rejects_malformed_identity() {
        let built_in = built_in_result(&[("unique", "engineering/unique.md")]);
        let migrated: InstallRecord = serde_json::from_value(serde_json::json!({
            "slug": "external",
            "tool": "codex",
            "scope": "user",
            "projectPath": null,
            "dest": "/dest/external.toml",
            "sourceHash": "source",
            "bodyHash": "body",
            "renderedHash": "render",
            "installedAt": "2026-08-04T00:00:00Z",
            "corpusVersion": "revision",
            "sourceId": "local:external",
            "relativePath": "nested/external.md",
            "disabledPath": null,
            "sourceSnapshotHash": "snapshot"
        }))
        .unwrap();
        let original = serde_json::to_value(&migrated).unwrap();
        let (rows, changed) = migrate_install_records(vec![migrated], None).unwrap();
        assert!(!changed);
        assert_eq!(serde_json::to_value(&rows[0]).unwrap(), original);
        assert!(
            migrate_install_records(vec![legacy_row("legacy", "/dest/legacy.toml")], None).is_err()
        );

        let malformed: InstallRecord = serde_json::from_value(serde_json::json!({
            "slug": "bad",
            "tool": "codex",
            "scope": "user",
            "projectPath": null,
            "dest": "/dest/bad.toml",
            "sourceHash": "source",
            "bodyHash": "body",
            "renderedHash": "render",
            "installedAt": "2026-08-04T00:00:00Z",
            "corpusVersion": "revision",
            "sourceId": "local:external",
            "relativePath": "../bad.md"
        }))
        .unwrap();
        assert!(migrate_install_records(vec![malformed], Some(&built_in)).is_err());
    }

    #[test]
    fn migration_reads_removed_as_missing() {
        let state: InstallState = serde_json::from_str("\"removed\"").unwrap();
        assert_eq!(state, InstallState::Missing);
    }

    #[tokio::test]
    async fn migration_persists_once_and_failed_replace_preserves_original() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("installs.json");
        let built_in = built_in_result(&[("unique", "engineering/unique.md")]);
        let legacy =
            serde_json::to_vec_pretty(&vec![legacy_row("unique", "/dest/unique.toml")]).unwrap();
        std::fs::write(&path, &legacy).unwrap();

        let first = load_migrated_ledger_path(&path, Some(&built_in), "2026-08-04T01:02:03Z")
            .await
            .unwrap();
        assert_eq!(first[0].relative_path, "engineering/unique.md");
        let migrated_bytes = std::fs::read(&path).unwrap();
        assert_ne!(migrated_bytes, legacy);
        let backups = std::fs::read_dir(temp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains("migration"))
            .collect::<Vec<_>>();
        assert_eq!(backups.len(), 1);
        assert_eq!(std::fs::read(backups[0].path()).unwrap(), legacy);

        let second = load_migrated_ledger_path(&path, Some(&built_in), "2026-08-04T02:02:03Z")
            .await
            .unwrap();
        assert_eq!(serde_json::to_vec_pretty(&second).unwrap(), migrated_bytes);
        assert_eq!(
            std::fs::read_dir(temp.path())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().contains("migration"))
                .count(),
            1,
            "an already migrated ledger creates no second backup"
        );

        std::fs::write(&path, &legacy).unwrap();
        std::fs::create_dir(path.with_extension("json.tmp")).unwrap();
        assert!(
            load_migrated_ledger_path(&path, Some(&built_in), "2026-08-04T03:02:03Z",)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), legacy);
    }

    #[tokio::test]
    #[ignore = "set AGENCY_AGENT_LEDGER_REHEARSAL to a copied pre-feature installs.json"]
    async fn migration_rehearsal_copy_preserves_real_destinations() {
        let path = PathBuf::from(std::env::var("AGENCY_AGENT_LEDGER_REHEARSAL").unwrap());
        let records: Vec<InstallRecord> =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let before = records
            .iter()
            .map(|record| {
                let bytes = std::fs::read(&record.dest).unwrap();
                (record.dest.clone(), render::sha256_hex(&bytes))
            })
            .collect::<Vec<_>>();
        let built_in =
            built_in_result(&[("agents-orchestrator", "specialized/agents-orchestrator.md")]);

        let migrated = load_migrated_ledger_path(&path, Some(&built_in), "2026-08-04T10:30:00Z")
            .await
            .unwrap();

        assert!(migrated.iter().all(|record| {
            record.source_id == crate::agents::BUILTIN_AGENT_SOURCE_ID
                && record.relative_path == "specialized/agents-orchestrator.md"
        }));
        for (destination, hash) in before {
            assert_eq!(
                render::sha256_hex(&std::fs::read(destination).unwrap()),
                hash
            );
        }
    }

    #[test]
    fn prune_project_rows_drops_only_that_project() {
        let mut ledger = vec![
            row("a", "claudeCode", Some("/p1")),
            row("b", "cursor", Some("/p1")),
            row("c", "claudeCode", Some("/p2")),
            row("d", "claudeCode", None), // user-global
        ];
        prune_project_rows(&mut ledger, "/p1");
        // Both /p1 rows gone; the other project + the global row survive.
        assert_eq!(ledger.len(), 2);
        assert!(ledger
            .iter()
            .all(|r| r.project_path.as_deref() != Some("/p1")));
        assert!(ledger
            .iter()
            .any(|r| r.slug == "c" && r.project_path.as_deref() == Some("/p2")));
        assert!(ledger
            .iter()
            .any(|r| r.slug == "d" && r.project_path.is_none()));

        // Forgetting an unknown project changes nothing.
        prune_project_rows(&mut ledger, "/nope");
        assert_eq!(ledger.len(), 2);
    }

    #[test]
    fn classify_states() {
        // file gone
        assert_eq!(classify(None, "r", "s1", Some("s1")), InstallState::Missing);
        // bytes differ from what we wrote → user-edited
        assert_eq!(
            classify(Some("x"), "r", "s1", Some("s1")),
            InstallState::Modified
        );
        // matches our render, corpus unchanged → current
        assert_eq!(
            classify(Some("r"), "r", "s1", Some("s1")),
            InstallState::Current
        );
        // matches our render, corpus advanced → outdated
        assert_eq!(
            classify(Some("r"), "r", "s1", Some("s2")),
            InstallState::Outdated
        );
        // source gone while managed → source unavailable
        assert_eq!(
            classify(Some("r"), "r", "s1", None),
            InstallState::SourceUnavailable
        );
    }

    #[test]
    fn reconcile_classifies_all_seven_states_in_precedence_order() {
        let cases = [
            (
                ReconcileFacts {
                    tracked: true,
                    destination_hash: None,
                    disabled_path: true,
                    disabled_hash: Some("render"),
                    rendered_hash: "render",
                    installed_source_hash: "source-1",
                    current_source_hash: Some("source-1"),
                },
                InstallState::Disabled,
            ),
            (
                ReconcileFacts {
                    tracked: true,
                    destination_hash: Some("render"),
                    disabled_path: false,
                    disabled_hash: None,
                    rendered_hash: "render",
                    installed_source_hash: "source-1",
                    current_source_hash: None,
                },
                InstallState::SourceUnavailable,
            ),
            (
                ReconcileFacts {
                    tracked: true,
                    destination_hash: None,
                    disabled_path: false,
                    disabled_hash: None,
                    rendered_hash: "render",
                    installed_source_hash: "source-1",
                    current_source_hash: Some("source-1"),
                },
                InstallState::Missing,
            ),
            (
                ReconcileFacts {
                    tracked: true,
                    destination_hash: Some("edited"),
                    disabled_path: false,
                    disabled_hash: None,
                    rendered_hash: "render",
                    installed_source_hash: "source-1",
                    current_source_hash: Some("source-1"),
                },
                InstallState::Modified,
            ),
            (
                ReconcileFacts {
                    tracked: true,
                    destination_hash: Some("render"),
                    disabled_path: false,
                    disabled_hash: None,
                    rendered_hash: "render",
                    installed_source_hash: "source-1",
                    current_source_hash: Some("source-2"),
                },
                InstallState::Outdated,
            ),
            (
                ReconcileFacts {
                    tracked: true,
                    destination_hash: Some("render"),
                    disabled_path: false,
                    disabled_hash: None,
                    rendered_hash: "render",
                    installed_source_hash: "source-1",
                    current_source_hash: Some("source-1"),
                },
                InstallState::Current,
            ),
            (
                ReconcileFacts {
                    tracked: false,
                    destination_hash: Some("anything"),
                    disabled_path: false,
                    disabled_hash: None,
                    rendered_hash: "render",
                    installed_source_hash: "source-1",
                    current_source_hash: Some("source-1"),
                },
                InstallState::Foreign,
            ),
        ];
        for (facts, expected) in cases {
            assert_eq!(classify_install(facts), expected);
        }

        assert_eq!(
            classify_install(ReconcileFacts {
                tracked: true,
                destination_hash: Some("edited"),
                disabled_path: false,
                disabled_hash: None,
                rendered_hash: "render",
                installed_source_hash: "source-1",
                current_source_hash: None,
            }),
            InstallState::SourceUnavailable,
            "source-unavailable takes precedence over modified"
        );
        for (destination_hash, disabled_hash) in [
            (Some("occupied"), Some("render")),
            (None, Some("wrong-content")),
        ] {
            assert_eq!(
                classify_install(ReconcileFacts {
                    tracked: true,
                    destination_hash,
                    disabled_path: true,
                    disabled_hash,
                    rendered_hash: "render",
                    installed_source_hash: "source-1",
                    current_source_hash: Some("source-1"),
                }),
                InstallState::Modified
            );
        }
    }

    #[test]
    fn reconcile_resolves_same_slug_agents_by_exact_source_reference() {
        let built_in = built_in_result(&[("duplicate", "one/duplicate.md")]);
        let mut external = built_in_result(&[("duplicate", "two/duplicate.md")]);
        external.source.id = "local:external".into();
        external.source.kind = crate::types::AgentSourceKind::Local {
            root: "/external".into(),
        };
        external.agents[0].reference.source_id = "local:external".into();
        external.agents[0].source_hash = "external-hash".into();
        let sources = vec![built_in, external];

        let mut record = row("duplicate", "codex", None);
        record.source_id = "local:external".into();
        record.relative_path = "two/duplicate.md".into();
        assert_eq!(
            find_agent_package(&sources, &record).map(|package| package.source_hash.as_str()),
            Some("external-hash")
        );
    }

    #[test]
    fn install_request_requires_exact_reference_or_unique_builtin_slug() {
        let built_in = built_in_result(&[
            ("duplicate", "one/duplicate.md"),
            ("duplicate", "two/duplicate.md"),
            ("unique", "engineering/unique.md"),
        ]);
        assert!(resolve_reference_request(
            std::slice::from_ref(&built_in),
            None,
            None,
            Some("duplicate"),
        )
        .is_err());
        assert_eq!(
            resolve_reference_request(std::slice::from_ref(&built_in), None, None, Some("unique"),)
                .unwrap()
                .relative_path,
            "engineering/unique.md"
        );
        assert_eq!(
            resolve_reference_request(
                &[built_in],
                Some("local:external"),
                Some("nested/duplicate.md"),
                None,
            )
            .unwrap(),
            AgentReference {
                source_id: "local:external".into(),
                relative_path: "nested/duplicate.md".into(),
            }
        );
    }

    #[test]
    fn install_destination_collision_names_both_source_identities() {
        let mut existing = row("duplicate", "codex", None);
        existing.source_id = "local:first".into();
        existing.relative_path = "one/duplicate.md".into();
        existing.dest = "/dest/duplicate.toml".into();
        let requested = AgentReference {
            source_id: "local:second".into(),
            relative_path: "two/duplicate.md".into(),
        };
        let error = ensure_destinations_available(
            &[existing],
            &requested,
            "codex",
            None,
            &[PathBuf::from("/dest/duplicate.toml")],
            false,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("local:first:one/duplicate.md"));
        assert!(error.contains("local:second:two/duplicate.md"));
    }

    #[test]
    fn install_update_requires_confirmation_for_broadened_capabilities() {
        assert!(!update_policy_allows(
            crate::types::AgentUpdatePolicy::AutoTrusted,
            false,
            &["read".into()],
            &["read".into(), "network".into()],
            Some("publisher-key"),
            Some("publisher-key"),
            true,
        ));
        assert!(update_policy_allows(
            crate::types::AgentUpdatePolicy::AutoTrusted,
            false,
            &["read".into()],
            &["read".into()],
            Some("publisher-key"),
            Some("publisher-key"),
            true,
        ));
        assert!(!update_policy_allows(
            crate::types::AgentUpdatePolicy::AutoTrusted,
            false,
            &[],
            &[],
            Some("publisher-key"),
            Some("publisher-key"),
            false,
        ));
        assert!(!update_policy_allows(
            crate::types::AgentUpdatePolicy::Pin,
            true,
            &[],
            &[],
            None,
            None,
            false,
        ));
        assert!(update_policy_allows(
            crate::types::AgentUpdatePolicy::Notify,
            true,
            &[],
            &[],
            None,
            None,
            false,
        ));
    }

    #[test]
    fn auto_trust_requires_the_exact_active_publisher_identity() {
        let mut package = built_in_result(&[("reviewer", "reviewer.md")])
            .agents
            .remove(0);
        package.publisher = Some("Acme".into());
        package.publisher_key = Some("key-1".into());
        let mut library = crate::types::AgentLibraryState::default();
        library
            .publisher_trust
            .push(crate::types::AgentPublisherTrust {
                name: "Acme".into(),
                public_key: "key-1".into(),
                trusted: true,
                revoked: false,
            });
        assert!(agent_publisher_is_trusted(&library, &package));

        package.publisher_key = Some("key-2".into());
        assert!(!agent_publisher_is_trusted(&library, &package));
        package.publisher_key = Some("key-1".into());
        library.publisher_trust[0].revoked = true;
        assert!(!agent_publisher_is_trusted(&library, &package));
    }

    #[test]
    fn approval_plan_rejects_stale_revisions_and_blockers() {
        let mut plan = AgentMutationPlan {
            revision: "current".into(),
            operation: "update".into(),
            tool: "codex".into(),
            scope: crate::types::Scope::User,
            project_path: None,
            agents: Vec::new(),
            warnings: Vec::new(),
            blockers: Vec::new(),
            rollback_available: false,
        };
        assert!(require_plan_revision(&plan, "stale").is_err());
        assert!(require_plan_revision(&plan, "current").is_ok());

        plan.blockers.push("capabilities broadened".into());
        assert!(require_plan_revision(&plan, "current").is_err());
    }

    #[tokio::test]
    async fn transient_batch_failure_recovery_restores_files_and_prior_ledger() {
        let root = tempfile::tempdir().unwrap();
        let existing = root.path().join("existing.md");
        let created = root.path().join("created.md");
        std::fs::write(&existing, b"before").unwrap();
        let snapshot = capture_batch_files(&[existing.clone(), created.clone()])
            .await
            .unwrap();
        let mut original = row("existing", "claudeCode", None);
        original.source_hash = "a".repeat(64);
        original.source_snapshot_hash = original.source_hash.clone();
        original.body_hash = "b".repeat(64);
        original.rendered_hash = "c".repeat(64);
        let original_ledger = vec![original];
        save_ledger_for(root.path(), &original_ledger)
            .await
            .unwrap();
        std::fs::write(&existing, b"after").unwrap();
        std::fs::write(&created, b"new").unwrap();
        save_ledger_for(root.path(), &[]).await.unwrap();

        restore_batch_files(&snapshot).await.unwrap();
        save_ledger_for(root.path(), &original_ledger)
            .await
            .unwrap();

        assert_eq!(std::fs::read(&existing).unwrap(), b"before");
        assert!(!created.exists());
        let restored: Vec<InstallRecord> =
            serde_json::from_slice(&std::fs::read(ledger_path_for(root.path())).unwrap()).unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].slug, original_ledger[0].slug);
        assert_eq!(restored[0].dest, original_ledger[0].dest);
    }

    #[tokio::test]
    async fn transient_batch_plan_expands_dependencies_without_writing() {
        use crate::commands::settings::{Settings, SettingsLoadState};

        let app_data = tempfile::tempdir().unwrap();
        let source_root = tempfile::tempdir().unwrap();
        let tool_home = tempfile::tempdir().unwrap();
        std::fs::write(
            source_root.path().join("base.md"),
            "---\nname: Base\ndescription: Foundation.\n---\nBase instructions.\n",
        )
        .unwrap();
        std::fs::write(
            source_root.path().join("lead.md"),
            "---\nname: Lead\ndescription: Leads.\nrequired-agents: [base.md]\nrecommended-agents: [optional.md]\n---\nLead instructions.\n",
        )
        .unwrap();
        std::fs::write(
            source_root.path().join("optional.md"),
            "---\nname: Optional\ndescription: Optional support.\n---\nOptional instructions.\n",
        )
        .unwrap();
        let source = crate::agents::add_local_source(app_data.path(), source_root.path())
            .await
            .unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app_data.path().to_path_buf();
        let mut settings = Settings::default();
        settings.tool_paths.insert(
            "claudeCode".into(),
            tool_home.path().to_string_lossy().into_owned(),
        );
        *state.settings.write().await = SettingsLoadState::Loaded(settings);

        let plan = build_mutation_plan(
            &state,
            vec![AgentReference {
                source_id: source.id,
                relative_path: "lead.md".into(),
            }],
            "claudeCode".into(),
            None,
            "install",
            true,
        )
        .await
        .unwrap();

        assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
        assert!(plan
            .warnings
            .iter()
            .any(|warning| warning.contains("Optional")));
        assert_eq!(plan.agents.len(), 2);
        assert!(plan.agents[0].dependency);
        assert_eq!(plan.agents[0].reference.relative_path, "base.md");
        assert!(!plan.agents[1].dependency);
        assert_eq!(plan.agents[1].reference.relative_path, "lead.md");
        assert!(plan
            .agents
            .iter()
            .all(|item| !Path::new(&item.destination).exists()));
        assert!(!ledger_path_for(app_data.path()).exists());

        let blocked = build_mutation_plan(
            &state,
            vec![AgentReference {
                source_id: "local:missing".into(),
                relative_path: "missing.md".into(),
            }],
            "claudeCode".into(),
            None,
            "install",
            true,
        )
        .await
        .unwrap();
        assert!(!blocked.blockers.is_empty());
        assert!(!ledger_path_for(app_data.path()).exists());
    }

    #[test]
    fn resolve_tool_base_prefers_nonempty_override() {
        use std::collections::HashMap;
        let os = Path::new("/Users/me");
        let mut tp: HashMap<String, String> = HashMap::new();
        // No entry → OS home.
        assert_eq!(
            resolve_tool_base(&tp, "claudeCode", os),
            PathBuf::from("/Users/me")
        );
        // Empty entry is treated as unset → OS home.
        tp.insert("claudeCode".into(), String::new());
        assert_eq!(
            resolve_tool_base(&tp, "claudeCode", os),
            PathBuf::from("/Users/me")
        );
        // Non-empty override wins, and ONLY for that tool.
        tp.insert("claudeCode".into(), "/wsl/home/me".into());
        assert_eq!(
            resolve_tool_base(&tp, "claudeCode", os),
            PathBuf::from("/wsl/home/me")
        );
        assert_eq!(
            resolve_tool_base(&tp, "codex", os),
            PathBuf::from("/Users/me")
        );
    }

    #[test]
    fn agent_units_handles_file_and_dir_tools() {
        let home = std::path::Path::new("/home/u");
        // File-per-agent (Claude): root = ~/.claude/agents, suffix = ".md".
        let claude = agent_units("claudeCode", home, None);
        assert!(
            claude
                .iter()
                .any(|(d, s)| d.ends_with(".claude/agents") && s == ".md"),
            "claude: {claude:?}"
        );
        // Dir-per-agent (Osaurus): the bug was scanning `.osaurus/skills/_probe`.
        // It must scan `.osaurus/skills` with a `/SKILL.md` leaf.
        let osa = agent_units("osaurus", home, None);
        assert_eq!(osa.len(), 1, "osaurus: {osa:?}");
        assert!(
            osa[0].0.ends_with(".osaurus/skills"),
            "osaurus dir: {:?}",
            osa[0].0
        );
        assert_eq!(osa[0].1, "/SKILL.md");
    }

    fn sample_agent() -> crate::types::Agent {
        crate::types::Agent {
            slug: "frontend-developer".into(),
            name: "Frontend Developer".into(),
            description: "Builds UIs.".into(),
            category: "engineering".into(),
            emoji: None,
            color: Some("blue".into()),
            vibe: None,
            body: "You are a frontend dev.\n".into(),
        }
    }

    /// Full render → write-to-disk → reconcile loop against a tempdir "home".
    #[tokio::test]
    async fn install_writes_then_reconciles_through_states() {
        let home = tempfile::tempdir().unwrap();
        let mut agent = sample_agent();
        agent.slug = "source-filename-differs".into();
        let raw = "---\nname: Frontend Developer\n---\nORIGINAL\n";

        // Codex (user-scoped, TOML transform).
        let rec = write_agent_files(
            &agent,
            raw,
            "codex",
            home.path(),
            None,
            None,
            "src-1",
            "body-1",
            "v1",
            "2026-06-05T00:00:00Z",
        )
        .await
        .unwrap();

        let path = home
            .path()
            .join(".codex")
            .join("agents")
            .join("frontend-developer.toml");
        assert!(path.exists(), "install wrote the file");
        let on_disk = std::fs::read(&path).unwrap();
        let disk_hash = render::sha256_hex(&on_disk);
        assert_eq!(
            disk_hash, rec.rendered_hash,
            "on-disk bytes match recorded render"
        );

        // Reconcile classifications off the real bytes:
        assert_eq!(
            classify(
                Some(&disk_hash),
                &rec.rendered_hash,
                &rec.source_hash,
                Some("src-1")
            ),
            InstallState::Current
        );
        assert_eq!(
            classify(
                Some(&disk_hash),
                &rec.rendered_hash,
                &rec.source_hash,
                Some("src-2")
            ),
            InstallState::Outdated
        );
        assert_eq!(
            classify(
                Some("useredited"),
                &rec.rendered_hash,
                &rec.source_hash,
                Some("src-1")
            ),
            InstallState::Modified
        );
        // delete → Missing
        std::fs::remove_file(&path).unwrap();
        let gone = if path.exists() {
            Some(disk_hash.as_str())
        } else {
            None
        };
        assert_eq!(
            classify(gone, &rec.rendered_hash, &rec.source_hash, Some("src-1")),
            InstallState::Missing
        );
    }

    #[tokio::test]
    async fn claude_code_writes_raw_verbatim() {
        let home = tempfile::tempdir().unwrap();
        let raw = "---\nname: Frontend Developer\ncolor: blue\n---\nVERBATIM BODY\n";
        write_agent_files(
            &sample_agent(),
            raw,
            "claudeCode",
            home.path(),
            None,
            None,
            "s",
            "b",
            "v",
            "t",
        )
        .await
        .unwrap();
        let got = std::fs::read_to_string(home.path().join(".claude/agents/frontend-developer.md"))
            .unwrap();
        assert_eq!(got, raw, "identity tool ships the source unchanged");
    }

    #[tokio::test]
    async fn project_tool_writes_into_project_root() {
        let home = tempfile::tempdir().unwrap();
        let proj = tempfile::tempdir().unwrap();
        let rec = write_agent_files(
            &sample_agent(),
            "raw",
            "cursor",
            home.path(),
            Some(proj.path()),
            None,
            "s",
            "b",
            "v",
            "t",
        )
        .await
        .unwrap();
        assert!(proj
            .path()
            .join(".cursor/rules/frontend-developer.mdc")
            .exists());
        assert_eq!(
            rec.project_path.as_deref(),
            Some(proj.path().to_string_lossy().as_ref())
        );
        assert_eq!(rec.scope, crate::types::Scope::Project);
    }

    /// A file byte-identical to the canonical render is recognized as in-sync
    /// (so the Foreign sweep can call it Current); any difference is not.
    #[test]
    fn canonical_render_is_recognized_byte_for_byte() {
        let agent = sample_agent();
        let raw = "---\nname: Frontend Developer\ncolor: blue\n---\nBODY\n";
        // The exact canonical render matches…
        let (rendered, _h) = render::render_with_hash(&agent, raw, "codex").unwrap();
        assert!(bytes_match_render(
            &agent,
            raw,
            "codex",
            rendered.as_bytes()
        ));
        // …a hand-edited / different file does not.
        assert!(!bytes_match_render(
            &agent,
            raw,
            "codex",
            b"different bytes"
        ));
        // Identity tool (claude-code ships the source verbatim) also matches.
        let (raw_render, _h2) = render::render_with_hash(&agent, raw, "claudeCode").unwrap();
        assert!(bytes_match_render(
            &agent,
            raw,
            "claudeCode",
            raw_render.as_bytes()
        ));
    }

    #[tokio::test]
    async fn multi_artifact_foreign_match_requires_every_exact_file() {
        let home = tempfile::tempdir().unwrap();
        let agent = sample_agent();
        let raw = "---\nname: Frontend Developer\ndescription: Builds UIs.\n---\nBODY\n";
        let paths = render::dests("kimi", "frontend-developer", home.path(), None).unwrap();
        let rendered = render::render_artifacts(&agent, raw, "kimi").unwrap();
        for (path, artifact) in paths.iter().zip(&rendered) {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, artifact.content.as_bytes()).unwrap();
        }

        assert!(destinations_match_render(
            &agent,
            raw,
            "kimi",
            "frontend-developer",
            home.path(),
            None,
        )
        .await);
        let tracked = track_agent_record(
            &agent,
            raw,
            "kimi",
            home.path(),
            None,
            "src-1",
            "body-1",
            "v1",
            "2026-08-17T01:00:00Z",
        )
        .unwrap();
        assert_eq!(tracked.artifacts.len(), 2);
        assert_eq!(
            tracked
                .artifacts
                .iter()
                .map(|artifact| PathBuf::from(&artifact.dest))
                .collect::<Vec<_>>(),
            paths
        );

        std::fs::write(&paths[1], b"changed secondary").unwrap();
        assert!(
            !destinations_match_render(
                &agent,
                raw,
                "kimi",
                "frontend-developer",
                home.path(),
                None,
            )
            .await
        );
    }

    /// Track records provenance but must NOT create or touch any file.
    #[tokio::test]
    async fn track_writes_no_file() {
        let home = tempfile::tempdir().unwrap();
        let agent = sample_agent();
        let raw = "---\nname: Frontend Developer\n---\nBODY\n";

        let rec = track_agent_record(
            &agent,
            raw,
            "codex",
            home.path(),
            None,
            "src-1",
            "body-1",
            "v1",
            "2026-06-06T00:00:00Z",
        )
        .unwrap();

        let path = home
            .path()
            .join(".codex/agents")
            .join("frontend-developer.toml");
        assert!(!path.exists(), "Track must not write the agent file");
        assert_eq!(
            rec.dest,
            path.to_string_lossy(),
            "record points at the canonical dest"
        );

        // The recorded rendered_hash equals a real render — so if the user's file
        // happens to match it, reconcile yields Current; otherwise Modified.
        let (_b, render_hash) = render::render_with_hash(&agent, raw, "codex").unwrap();
        assert_eq!(rec.rendered_hash, render_hash);
        assert_eq!(
            classify(
                Some(&render_hash),
                &rec.rendered_hash,
                &rec.source_hash,
                Some("src-1")
            ),
            InstallState::Current,
            "a tracked file that matches the canonical render reconciles as Current"
        );
        assert_eq!(
            classify(
                Some("hand-edited"),
                &rec.rendered_hash,
                &rec.source_hash,
                Some("src-1")
            ),
            InstallState::Modified,
            "a tracked file that differs reconciles as Modified (never silently clobbered)"
        );
    }

    #[tokio::test]
    async fn tracked_conversion_slug_update_reuses_existing_destination() {
        let home = tempfile::tempdir().unwrap();
        let backups = tempfile::tempdir().unwrap();
        let mut agent = sample_agent();
        agent.slug = "engineering-frontend-developer".into();
        let raw = "---\nname: Frontend Developer\ndescription: Builds UIs.\n---\nBODY\n";
        let conversion_dest = home
            .path()
            .join(".codex/agents")
            .join("frontend-developer.toml");
        std::fs::create_dir_all(conversion_dest.parent().unwrap()).unwrap();
        std::fs::write(&conversion_dest, b"OLDER CLI OUTPUT").unwrap();

        let tracked = track_agent_record(
            &agent,
            raw,
            "codex",
            home.path(),
            None,
            "src-1",
            "body-1",
            "v1",
            "2026-06-12T00:00:00Z",
        )
        .unwrap();
        assert_eq!(tracked.dest, conversion_dest.to_string_lossy());

        write_agent_files_to(
            &agent,
            raw,
            "codex",
            home.path(),
            None,
            Some(backups.path()),
            "src-2",
            "body-2",
            "v2",
            "2026-06-12T01:00:00Z",
            Some(&conversion_dest),
        )
        .await
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(&conversion_dest).unwrap(),
            render::render(&agent, raw, "codex").unwrap()
        );
        assert!(
            !home
                .path()
                .join(".codex/agents/engineering-frontend-developer.toml")
                .exists(),
            "update must not create a duplicate source-slug file"
        );
    }

    #[tokio::test]
    async fn multi_artifact_update_keeps_recorded_base_when_tool_base_changes() {
        let old_home = tempfile::tempdir().unwrap();
        let new_home = tempfile::tempdir().unwrap();
        let backups = tempfile::tempdir().unwrap();
        let agent = sample_agent();
        let raw = "---\nname: Frontend Developer\ndescription: Builds UIs.\n---\nUPDATED\n";
        let old_paths = render::dests("kimi", "frontend-developer", old_home.path(), None).unwrap();
        for path in &old_paths {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"older managed bytes").unwrap();
        }
        let new_paths = render::dests("kimi", "frontend-developer", new_home.path(), None).unwrap();

        let updated = write_agent_files_at(
            &agent,
            raw,
            "kimi",
            None,
            Some(backups.path()),
            "src-2",
            "body-2",
            "v2",
            "2026-08-17T02:00:00Z",
            &old_paths,
        )
        .await
        .unwrap();

        assert_eq!(
            updated
                .artifacts
                .iter()
                .map(|artifact| PathBuf::from(&artifact.dest))
                .collect::<Vec<_>>(),
            old_paths
        );
        let rendered = render::render_artifacts(&agent, raw, "kimi").unwrap();
        for (path, artifact) in old_paths.iter().zip(rendered) {
            assert_eq!(std::fs::read(path).unwrap(), artifact.content.as_bytes());
        }
        assert!(new_paths.iter().all(|path| !path.exists()));
    }

    #[tokio::test]
    async fn kimi_write_is_atomic_across_both_artifacts() {
        let home = tempfile::tempdir().unwrap();
        let backups = tempfile::tempdir().unwrap();
        let agent = sample_agent();
        let raw = "---\nname: Frontend Developer\ndescription: Builds UIs.\n---\nBODY\n";
        let destinations = render::dests("kimi", "frontend-developer", home.path(), None).unwrap();
        std::fs::create_dir_all(destinations[0].parent().unwrap()).unwrap();
        std::fs::write(&destinations[0], b"previous agent yaml").unwrap();
        std::fs::create_dir(destinations[1].with_extension("md.tmp")).unwrap();

        let result = write_agent_files(
            &agent,
            raw,
            "kimi",
            home.path(),
            None,
            Some(backups.path()),
            "src-2",
            "body-2",
            "v2",
            "2026-08-17T01:02:03Z",
        )
        .await;

        assert!(result.is_err());
        assert_eq!(
            std::fs::read(&destinations[0]).unwrap(),
            b"previous agent yaml"
        );
        assert!(!destinations[1].exists());
    }

    #[tokio::test]
    async fn kimi_write_persists_exact_per_artifact_manifest() {
        let home = tempfile::tempdir().unwrap();
        let agent = sample_agent();
        let raw = "---\nname: Frontend Developer\ndescription: Builds UIs.\n---\nBODY\n";
        let record = write_agent_files(
            &agent,
            raw,
            "kimi",
            home.path(),
            None,
            None,
            "src-1",
            "body-1",
            "v1",
            "2026-08-17T01:02:03Z",
        )
        .await
        .unwrap();
        let rendered = render::render_artifacts(&agent, raw, "kimi").unwrap();
        assert_eq!(record.artifacts.len(), 2);
        for (manifest, expected) in record.artifacts.iter().zip(rendered) {
            assert_eq!(
                std::fs::read(&manifest.dest).unwrap(),
                expected.content.as_bytes()
            );
            assert_eq!(
                manifest.rendered_hash,
                render::sha256_hex(expected.content.as_bytes())
            );
            assert_eq!(manifest.disabled_path, None);
        }
        assert_eq!(record.dest, record.artifacts[0].dest);
        assert_eq!(record.rendered_hash, record.artifacts[0].rendered_hash);
    }

    #[test]
    fn multi_artifact_destinations_use_upstream_name_slug() {
        let home = Path::new("/home/user");
        let mut agent = sample_agent();
        agent.slug = "source-filename-differs".into();
        let raw = "---\nname: Frontend Developer\ndescription: Builds UIs.\n---\nBODY\n";
        assert_eq!(
            install_target_paths(&agent, Some(raw), "kimi", home, None, None).unwrap(),
            vec![
                home.join(".config/kimi/agents/frontend-developer/agent.yaml"),
                home.join(".config/kimi/agents/frontend-developer/system.md"),
            ]
        );
        assert_eq!(
            install_target_paths(&agent, Some(raw), "openclaw", home, None, None).unwrap(),
            vec![
                home.join(".openclaw/agency-agents/frontend-developer/SOUL.md"),
                home.join(".openclaw/agency-agents/frontend-developer/AGENTS.md"),
                home.join(".openclaw/agency-agents/frontend-developer/IDENTITY.md"),
            ]
        );
    }

    #[test]
    fn openclaw_success_serializes_truthful_external_activation_notice() {
        let agent = sample_agent();
        let record = record_for(
            &agent,
            Path::new("/home/user/.openclaw/agency-agents/frontend-developer/SOUL.md"),
            "openclaw",
            None,
            "a".repeat(64),
            &"b".repeat(64),
            &"c".repeat(64),
            "v1",
            "2026-08-17T00:00:00Z",
        );

        let value = serde_json::to_value(record).unwrap();
        assert_eq!(
            value["deploymentNotice"],
            serde_json::Value::String(OPENCLAW_DEPLOYMENT_NOTICE.into())
        );
        assert!(OPENCLAW_DEPLOYMENT_NOTICE.contains("files installed"));
        assert!(OPENCLAW_DEPLOYMENT_NOTICE.contains("registration and restart remain required"));
    }

    #[test]
    fn openclaw_never_has_an_executable_version_probe() {
        assert!(version_cmd("openclaw").is_none());
    }

    #[test]
    fn lifecycle_disable_and_enable_move_exact_managed_files() {
        let root = tempfile::tempdir().unwrap();
        let active = [
            root.path().join("one/agent.md"),
            root.path().join("two/agent.md"),
        ];
        for path in &active {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"managed").unwrap();
        }
        let disabled = active
            .iter()
            .map(|path| disabled_destination(path).unwrap())
            .collect::<Vec<_>>();
        let hash = render::sha256_hex(b"managed");

        move_managed_files(&active, &disabled, &hash).unwrap();
        assert!(active.iter().all(|path| !path.exists()));
        assert!(disabled.iter().all(|path| path.exists()));

        move_managed_files(&disabled, &active, &hash).unwrap();
        assert!(disabled.iter().all(|path| !path.exists()));
        assert!(active
            .iter()
            .all(|path| std::fs::read(path).unwrap() == b"managed"));
    }

    #[test]
    fn multi_artifact_reconcile_uses_required_precedence() {
        let expected = ["one", "two", "three"];
        assert_eq!(
            classify_artifact_install(
                &[Some("one"), Some("changed"), None],
                &[None, None, None],
                &expected,
                false,
                "source-1",
                Some("source-1"),
            ),
            InstallState::Modified,
            "Modified outranks a simultaneously missing artifact"
        );
        assert_eq!(
            classify_artifact_install(
                &[Some("one"), None, Some("three")],
                &[None, None, None],
                &expected,
                false,
                "source-1",
                Some("source-2"),
            ),
            InstallState::Missing,
            "Missing outranks an available source update"
        );
        assert_eq!(
            classify_artifact_install(
                &[Some("one"), Some("two"), Some("three")],
                &[None, None, None],
                &expected,
                false,
                "source-1",
                Some("source-2"),
            ),
            InstallState::Outdated
        );
        assert_eq!(
            classify_artifact_install(
                &[None, None, None],
                &[Some("one"), Some("two"), Some("three")],
                &expected,
                true,
                "source-1",
                Some("source-2"),
            ),
            InstallState::Disabled
        );
        assert_eq!(
            classify_artifact_install(
                &[Some("one"), Some("two"), Some("three")],
                &[None, None, None],
                &expected,
                false,
                "source-1",
                None,
            ),
            InstallState::SourceUnavailable
        );
    }

    #[test]
    fn lifecycle_enable_refuses_occupied_target_without_mutation() {
        let root = tempfile::tempdir().unwrap();
        let active = root.path().join("agent.md");
        let disabled = disabled_destination(&active).unwrap();
        std::fs::write(&active, b"foreign").unwrap();
        std::fs::write(&disabled, b"managed").unwrap();

        assert!(move_managed_files(
            std::slice::from_ref(&disabled),
            std::slice::from_ref(&active),
            &render::sha256_hex(b"managed"),
        )
        .is_err());
        assert_eq!(std::fs::read(&active).unwrap(), b"foreign");
        assert_eq!(std::fs::read(&disabled).unwrap(), b"managed");
    }

    #[test]
    fn lifecycle_mid_move_failure_restores_every_prior_file() {
        let root = tempfile::tempdir().unwrap();
        let active = [root.path().join("one.md"), root.path().join("two.md")];
        for path in &active {
            std::fs::write(path, b"managed").unwrap();
        }
        let disabled = active
            .iter()
            .map(|path| disabled_destination(path).unwrap())
            .collect::<Vec<_>>();
        let calls = std::cell::Cell::new(0usize);
        let result = move_managed_files_with(
            &active,
            &disabled,
            &render::sha256_hex(b"managed"),
            |source, destination| {
                let call = calls.get();
                calls.set(call + 1);
                if call == 1 {
                    Err(std::io::Error::other("injected rename failure"))
                } else {
                    std::fs::rename(source, destination)
                }
            },
        );
        assert!(result.is_err());
        assert!(active
            .iter()
            .all(|path| std::fs::read(path).unwrap() == b"managed"));
        assert!(disabled.iter().all(|path| !path.exists()));
    }

    #[test]
    fn multi_artifact_move_checks_each_hash_and_restores_on_late_failure() {
        let root = tempfile::tempdir().unwrap();
        let active = [
            root.path().join("agent.yaml"),
            root.path().join("system.md"),
        ];
        let bytes = [b"yaml".as_slice(), b"prompt".as_slice()];
        for (path, content) in active.iter().zip(bytes) {
            std::fs::write(path, content).unwrap();
        }
        let disabled = active
            .iter()
            .map(|path| disabled_destination(path).unwrap())
            .collect::<Vec<_>>();
        let hashes = bytes
            .iter()
            .map(|content| render::sha256_hex(content))
            .collect::<Vec<_>>();
        let calls = std::cell::Cell::new(0usize);
        let result =
            move_managed_artifacts_with(&active, &disabled, &hashes, |source, destination| {
                let call = calls.get();
                calls.set(call + 1);
                if call == 1 {
                    Err(std::io::Error::other("injected second artifact failure"))
                } else {
                    std::fs::rename(source, destination)
                }
            });
        assert!(result.is_err());
        for (path, content) in active.iter().zip(bytes) {
            assert_eq!(std::fs::read(path).unwrap(), content);
        }
        assert!(disabled.iter().all(|path| !path.exists()));
    }

    #[tokio::test]
    async fn multi_artifact_uninstall_restores_every_file_on_late_failure() {
        let root = tempfile::tempdir().unwrap();
        let paths = [root.path().join("SOUL.md"), root.path().join("AGENTS.md")];
        for (path, content) in paths.iter().zip([b"soul".as_slice(), b"agents".as_slice()]) {
            std::fs::write(path, content).unwrap();
        }
        let calls = std::cell::Cell::new(0usize);
        let result = remove_agent_artifacts_with(&paths, |path| {
            let call = calls.get();
            calls.set(call + 1);
            if call == 1 {
                Err(std::io::Error::other("injected second artifact failure"))
            } else {
                std::fs::remove_file(path)
            }
        })
        .await;
        assert!(result.is_err());
        assert_eq!(std::fs::read(&paths[0]).unwrap(), b"soul");
        assert_eq!(std::fs::read(&paths[1]).unwrap(), b"agents");
    }

    #[tokio::test]
    async fn deleting_or_modifying_a_secondary_artifact_changes_reconciled_state() {
        use crate::commands::settings::{Settings, SettingsLoadState};

        let home = tempfile::tempdir().unwrap();
        let state = AppState::build().unwrap();
        let mut settings = Settings::default();
        settings
            .tool_paths
            .insert("kimi".into(), home.path().to_string_lossy().into_owned());
        *state.settings.write().await = SettingsLoadState::Loaded(settings);
        let mut agent = sample_agent();
        agent.slug = "source-filename-differs".into();
        let raw = "---\nname: Frontend Developer\ndescription: Builds UIs.\n---\nBODY\n";
        let record = write_agent_files(
            &agent,
            raw,
            "kimi",
            home.path(),
            None,
            None,
            "source-1",
            "body-1",
            "v1",
            "2026-08-17T01:02:03Z",
        )
        .await
        .unwrap();

        std::fs::remove_file(&record.artifacts[1].dest).unwrap();
        let (expected, active, disabled) = record_reconcile_hashes(&state, &record).await.unwrap();
        assert_eq!(
            classify_artifact_install(
                &active.iter().map(Option::as_deref).collect::<Vec<_>>(),
                &disabled.iter().map(Option::as_deref).collect::<Vec<_>>(),
                &expected.iter().map(String::as_str).collect::<Vec<_>>(),
                false,
                "source-1",
                Some("source-1"),
            ),
            InstallState::Missing
        );

        std::fs::write(&record.artifacts[1].dest, b"changed").unwrap();
        let (expected, active, disabled) = record_reconcile_hashes(&state, &record).await.unwrap();
        assert_eq!(
            classify_artifact_install(
                &active.iter().map(Option::as_deref).collect::<Vec<_>>(),
                &disabled.iter().map(Option::as_deref).collect::<Vec<_>>(),
                &expected.iter().map(String::as_str).collect::<Vec<_>>(),
                false,
                "source-1",
                Some("source-1"),
            ),
            InstallState::Modified
        );
    }

    /// A write that overwrites an existing, DIFFERENT file must preserve the old
    /// bytes in the backups dir first; an identical (no-op) write must not.
    #[tokio::test]
    async fn write_backs_up_existing_differing_file() {
        let home = tempfile::tempdir().unwrap();
        let backups = tempfile::tempdir().unwrap();
        let agent = sample_agent();
        let dest = home.path().join(".codex/agents/frontend-developer.toml");

        // Simulate a user-edited file already on disk at the dest.
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, b"USER EDITED CONTENT").unwrap();

        // Update over it (with backups enabled).
        write_agent_files(
            &agent,
            "---\nname: Frontend Developer\n---\nNEW\n",
            "codex",
            home.path(),
            None,
            Some(backups.path()),
            "src-2",
            "body-2",
            "v2",
            "2026-06-06T01:02:03Z",
        )
        .await
        .unwrap();

        // The old bytes were preserved before the overwrite.
        let saved: Vec<_> = std::fs::read_dir(backups.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| std::fs::read(e.path()).unwrap())
            .collect();
        assert_eq!(saved.len(), 1, "exactly one backup created");
        assert_eq!(
            saved[0], b"USER EDITED CONTENT",
            "backup holds the pre-overwrite bytes"
        );

        // A second, byte-identical write makes no new backup (not destructive).
        let before = std::fs::read(&dest).unwrap();
        write_agent_files(
            &agent,
            "---\nname: Frontend Developer\n---\nNEW\n",
            "codex",
            home.path(),
            None,
            Some(backups.path()),
            "src-2",
            "body-2",
            "v2",
            "2026-06-06T02:02:03Z",
        )
        .await
        .unwrap();
        let after = std::fs::read(&dest).unwrap();
        assert_eq!(before, after, "identical render leaves the file unchanged");
        let count = std::fs::read_dir(backups.path()).unwrap().count();
        assert_eq!(count, 1, "no-op write adds no backup");
    }

    #[tokio::test]
    async fn uninstall_canonical_file_needs_no_backup() {
        let home = tempfile::tempdir().unwrap();
        let backups = tempfile::tempdir().unwrap();
        let agent = sample_agent();
        let raw = "---\nname: Frontend Developer\ndescription: Builds UIs.\n---\nBODY\n";
        let dest = home.path().join(".codex/agents/frontend-developer.toml");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, render::render(&agent, raw, "codex").unwrap()).unwrap();

        remove_agent_files(
            &agent,
            raw,
            "codex",
            home.path(),
            None,
            None,
            backups.path(),
            "2026-06-12T00:00:00Z",
        )
        .await
        .unwrap();

        assert!(!dest.exists());
        assert_eq!(std::fs::read_dir(backups.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn uninstall_modified_file_backs_up_before_delete() {
        let home = tempfile::tempdir().unwrap();
        let backups = tempfile::tempdir().unwrap();
        let agent = sample_agent();
        let raw = "---\nname: Frontend Developer\ndescription: Builds UIs.\n---\nBODY\n";
        let dest = home.path().join(".codex/agents/frontend-developer.toml");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, b"USER MODIFIED").unwrap();

        remove_agent_files(
            &agent,
            raw,
            "codex",
            home.path(),
            None,
            None,
            backups.path(),
            "2026-06-12T00:00:00Z",
        )
        .await
        .unwrap();

        assert!(!dest.exists());
        let saved: Vec<_> = std::fs::read_dir(backups.path())
            .unwrap()
            .map(|entry| std::fs::read(entry.unwrap().path()).unwrap())
            .collect();
        assert_eq!(saved, vec![b"USER MODIFIED".to_vec()]);
    }

    #[tokio::test]
    async fn uninstall_missing_file_is_successful() {
        let home = tempfile::tempdir().unwrap();
        let backups = tempfile::tempdir().unwrap();
        remove_agent_files(
            &sample_agent(),
            "---\nname: Frontend Developer\n---\nBODY\n",
            "codex",
            home.path(),
            None,
            None,
            backups.path(),
            "2026-06-12T00:00:00Z",
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn uninstall_copilot_removes_both_destinations() {
        let home = tempfile::tempdir().unwrap();
        let backups = tempfile::tempdir().unwrap();
        let agent = sample_agent();
        let raw = "---\nname: Frontend Developer\n---\nBODY\n";
        for dest in render::dests("copilot", &agent.slug, home.path(), None).unwrap() {
            std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
            std::fs::write(dest, raw).unwrap();
        }

        remove_agent_files(
            &agent,
            raw,
            "copilot",
            home.path(),
            None,
            None,
            backups.path(),
            "2026-06-12T00:00:00Z",
        )
        .await
        .unwrap();

        for dest in render::dests("copilot", &agent.slug, home.path(), None).unwrap() {
            assert!(!dest.exists());
        }
    }

    #[tokio::test]
    async fn uninstall_removes_orphaned_skill_dir() {
        // #60: skill-md tools install each agent as `<slug>/SKILL.md`. Uninstall
        // must remove the whole `<slug>/` dir — a leftover empty dir is otherwise
        // re-surfaced by the reconcile scan as an untracked phantom.
        let home = tempfile::tempdir().unwrap();
        let backups = tempfile::tempdir().unwrap();
        let agent = sample_agent();
        let raw = "---\nname: Frontend Developer\ndescription: Builds UIs.\n---\nBODY\n";

        for tool in ["antigravity", "osaurus"] {
            let dests = candidate_dests(&agent, raw, tool, home.path(), None).unwrap();
            for dest in &dests {
                std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
                std::fs::write(dest, render::render(&agent, raw, tool).unwrap()).unwrap();
            }

            remove_agent_files(
                &agent,
                raw,
                tool,
                home.path(),
                None,
                None,
                backups.path(),
                "2026-06-12T00:00:00Z",
            )
            .await
            .unwrap();

            for dest in &dests {
                assert!(!dest.exists(), "{tool}: SKILL.md still present at {dest:?}");
                let slug_dir = dest.parent().unwrap();
                assert!(
                    !slug_dir.exists(),
                    "{tool}: orphaned skill dir left at {slug_dir:?}"
                );
            }
        }
    }

    #[tokio::test]
    async fn uninstall_backup_failure_preserves_original() {
        let home = tempfile::tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let backup_path = scratch.path().join("not-a-directory");
        std::fs::write(&backup_path, b"occupied").unwrap();
        let agent = sample_agent();
        let raw = "---\nname: Frontend Developer\n---\nBODY\n";
        let dest = home.path().join(".codex/agents/frontend-developer.toml");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, b"USER MODIFIED").unwrap();

        assert!(remove_agent_files(
            &agent,
            raw,
            "codex",
            home.path(),
            None,
            None,
            &backup_path,
            "2026-06-12T00:00:00Z",
        )
        .await
        .is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), b"USER MODIFIED");
    }

    #[tokio::test]
    async fn uninstall_removal_failure_is_reported() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("directory");
        std::fs::create_dir(&directory).unwrap();
        assert!(remove_file_strict(&directory).await.is_err());
        assert!(directory.exists());
    }

    #[test]
    fn ledger_json_roundtrips() {
        let recs = vec![InstallRecord {
            slug: "a".into(),
            source_id: crate::agents::BUILTIN_AGENT_SOURCE_ID.into(),
            relative_path: "engineering/a.md".into(),
            tool: "cursor".to_string(),
            scope: crate::types::Scope::Project,
            project_path: Some("/p".into()),
            dest: "/p/.cursor/rules/a.mdc".into(),
            source_hash: "sh".into(),
            body_hash: "bh".into(),
            rendered_hash: "rh".into(),
            artifacts: Vec::new(),
            disabled_path: None,
            source_snapshot_hash: "sh".into(),
            capabilities: Vec::new(),
            publisher_key: None,
            publisher_verified: false,
            installed_at: "2026-06-05T00:00:00Z".into(),
            corpus_version: "v".into(),
            deployment_notice: None,
        }];
        let bytes = serde_json::to_vec(&recs).unwrap();
        // tool serializes camelCase per the wire contract.
        assert!(String::from_utf8_lossy(&bytes).contains("\"cursor\""));
        let back: Vec<InstallRecord> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].tool, "cursor");
    }

    #[tokio::test]
    async fn project_registry_canonicalizes_deduplicates_and_unregisters() {
        let app = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let canonical = std::fs::canonicalize(project.path()).unwrap();

        assert_eq!(
            register_project(app.path(), project.path().to_str().unwrap())
                .await
                .unwrap(),
            canonical
        );
        register_project(app.path(), project.path().to_str().unwrap())
            .await
            .unwrap();
        assert_eq!(
            registered_projects(app.path()).await.unwrap(),
            vec![canonical.clone()]
        );
        assert!(unregister_project(app.path(), canonical.to_str().unwrap())
            .await
            .unwrap());
        assert!(registered_projects(app.path()).await.unwrap().is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn project_registry_rejects_symlink_paths() {
        use std::os::unix::fs::symlink;

        let app = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let parent = tempfile::tempdir().unwrap();
        let linked = parent.path().join("project-link");
        symlink(project.path(), &linked).unwrap();

        assert!(register_project(app.path(), linked.to_str().unwrap())
            .await
            .is_err());
    }

    #[test]
    fn project_instruction_targets_are_closed_and_deterministic() {
        let targets = project_instruction_targets();
        assert_eq!(
            targets
                .iter()
                .map(|target| (target.id, target.relative_path))
                .collect::<Vec<_>>(),
            vec![
                ("agents", "AGENTS.md"),
                ("claude", "CLAUDE.md"),
                ("gemini", "GEMINI.md"),
                ("copilot", ".github/copilot-instructions.md"),
            ]
        );
        assert!(project_instruction_target("../AGENTS.md").is_err());
    }

    #[test]
    fn project_instruction_snippets_preserve_unowned_bytes_exactly() {
        let original = "# Existing\n\nKeep these bytes.\n";
        let created = compose_project_instruction(
            original,
            ProjectInstructionOperation::Upsert,
            "review-rules",
            "Review every diff.",
        )
        .unwrap();
        assert!(created.adoption);
        assert!(created.proposed.starts_with(original));
        assert_eq!(
            parse_project_instruction_snippets(&created.proposed)
                .unwrap()
                .into_iter()
                .map(|snippet| snippet.id)
                .collect::<Vec<_>>(),
            vec!["review-rules"]
        );

        let replaced = compose_project_instruction(
            &created.proposed,
            ProjectInstructionOperation::Upsert,
            "review-rules",
            "Review the complete diff.",
        )
        .unwrap();
        assert!(!replaced.adoption);
        assert_eq!(replaced.proposed.matches("review-rules:begin").count(), 1);

        let removed = compose_project_instruction(
            &replaced.proposed,
            ProjectInstructionOperation::Remove,
            "review-rules",
            "",
        )
        .unwrap();
        assert_eq!(removed.proposed, original);
        assert!(parse_project_instruction_snippets(&removed.proposed)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn project_instruction_parser_rejects_malformed_duplicate_and_unsafe_content() {
        assert!(parse_project_instruction_snippets(
            "<!-- agency-agents:instruction:v1:a:begin -->\nmissing end"
        )
        .is_err());
        let duplicate = "<!-- agency-agents:instruction:v1:a:begin -->\none\n<!-- agency-agents:instruction:v1:a:end -->\n\n<!-- agency-agents:instruction:v1:a:begin -->\ntwo\n<!-- agency-agents:instruction:v1:a:end -->";
        assert!(parse_project_instruction_snippets(duplicate).is_err());
        assert!(compose_project_instruction(
            "",
            ProjectInstructionOperation::Upsert,
            "bad marker",
            "safe",
        )
        .is_err());
        assert!(compose_project_instruction(
            "",
            ProjectInstructionOperation::Upsert,
            "safe-id",
            "api_key=sk-1234567890abcdef",
        )
        .is_err());
        assert!(compose_project_instruction(
            "",
            ProjectInstructionOperation::Upsert,
            "safe-id",
            "Use ghp_1234567890abcdef",
        )
        .is_err());
        assert!(compose_project_instruction(
            "",
            ProjectInstructionOperation::Upsert,
            "safe-id",
            "unsafe\u{0}control",
        )
        .is_err());

        let mut bounded = String::new();
        for index in 0..MAX_PROJECT_INSTRUCTION_SNIPPETS {
            bounded = compose_project_instruction(
                &bounded,
                ProjectInstructionOperation::Upsert,
                &format!("rule-{index}"),
                "Review.",
            )
            .unwrap()
            .proposed;
        }
        assert!(compose_project_instruction(
            &bounded,
            ProjectInstructionOperation::Upsert,
            "one-too-many",
            "Review.",
        )
        .is_err());
        assert!(validate_project_instruction_content(
            &"x".repeat(MAX_PROJECT_INSTRUCTION_CONTENT_BYTES)
        )
        .is_ok());
        assert!(validate_project_instruction_content(
            &"x".repeat(MAX_PROJECT_INSTRUCTION_CONTENT_BYTES + 1)
        )
        .is_err());
    }

    async fn project_instruction_test_state(app_data: &Path, project: &Path) -> AppState {
        let canonical = std::fs::canonicalize(project).unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app_data.to_path_buf();
        let database = crate::state_db::StateDatabase::open(app_data).unwrap();
        database
            .mutate(projects_spec(), Vec::new(), move |projects| {
                projects.push(canonical);
                Ok(())
            })
            .await
            .unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        state
    }

    #[tokio::test]
    async fn project_instruction_plan_is_deterministic_complete_and_write_free() {
        let app_data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let destination = project.path().join("AGENTS.md");
        let original = "# Existing project rules\n";
        std::fs::write(&destination, original).unwrap();
        let state = project_instruction_test_state(app_data.path(), project.path()).await;
        let project_path = std::fs::canonicalize(project.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();

        let first = build_project_instruction_plan(
            &state,
            &project_path,
            "agents",
            ProjectInstructionOperation::Upsert,
            "review-rules",
            "Review every diff.",
        )
        .await
        .unwrap();
        let second = build_project_instruction_plan(
            &state,
            &project_path,
            "agents",
            ProjectInstructionOperation::Upsert,
            "review-rules",
            "Review every diff.",
        )
        .await
        .unwrap();

        assert_eq!(first, second);
        assert!(first.blockers.is_empty());
        assert!(first.adoption);
        assert!(first.backup_required);
        assert_eq!(first.current, original);
        assert!(first.proposed.contains("review-rules:begin"));
        assert_eq!(std::fs::read_to_string(&destination).unwrap(), original);
        assert!(!backups_dir_for(app_data.path()).exists());
        assert!(state
            .completed_state_database()
            .await
            .unwrap()
            .unwrap()
            .pending_filesystem_operations()
            .await
            .unwrap()
            .is_empty());

        let no_op = build_project_instruction_plan(
            &state,
            &project_path,
            "agents",
            ProjectInstructionOperation::Remove,
            "missing-rule",
            "",
        )
        .await
        .unwrap();
        assert!(no_op.no_op);
        assert_eq!(no_op.current, no_op.proposed);
        assert_eq!(
            no_op.blockers,
            vec!["instruction plan has no changes to apply"]
        );
    }

    #[tokio::test]
    async fn project_instruction_inspection_classifies_all_targets_without_writes() {
        let app_data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let unmanaged = "# Existing project rules\n";
        let managed = project_instruction_block("team-rules", "Review every diff.");
        let malformed = "<!-- agency-agents:instruction:v1:broken:begin -->\nmissing end";
        std::fs::write(project.path().join("AGENTS.md"), unmanaged).unwrap();
        std::fs::write(project.path().join("CLAUDE.md"), &managed).unwrap();
        std::fs::write(project.path().join("GEMINI.md"), malformed).unwrap();
        let state = project_instruction_test_state(app_data.path(), project.path()).await;
        let project_path = std::fs::canonicalize(project.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();

        let inspected = inspect_project_instruction_targets(&state, &project_path)
            .await
            .unwrap();

        assert_eq!(
            inspected
                .iter()
                .map(|target| (target.id.as_str(), target.state.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("agents", "existingUnmanaged"),
                ("claude", "managed"),
                ("gemini", "blocked"),
                ("copilot", "absent"),
            ]
        );
        assert_eq!(
            std::fs::read_to_string(project.path().join("AGENTS.md")).unwrap(),
            unmanaged
        );
        assert_eq!(
            std::fs::read_to_string(project.path().join("CLAUDE.md")).unwrap(),
            managed
        );
        assert_eq!(
            std::fs::read_to_string(project.path().join("GEMINI.md")).unwrap(),
            malformed
        );
        assert!(!project.path().join(".github").exists());
        assert!(state
            .completed_state_database()
            .await
            .unwrap()
            .unwrap()
            .pending_filesystem_operations()
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn project_instruction_inspection_blocks_invalid_utf8_oversize_and_unregistered_roots() {
        let app_data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let unregistered = tempfile::tempdir().unwrap();
        let state = project_instruction_test_state(app_data.path(), project.path()).await;
        let project_path = std::fs::canonicalize(project.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let target = project_instruction_target("agents").unwrap();
        let destination = project.path().join("AGENTS.md");

        std::fs::write(&destination, [0xff]).unwrap();
        assert!(read_project_instruction_target(project.path(), target)
            .await
            .is_err());
        let file = std::fs::File::create(&destination).unwrap();
        file.set_len(MAX_PROJECT_INSTRUCTION_BYTES + 1).unwrap();
        assert!(read_project_instruction_target(project.path(), target)
            .await
            .is_err());
        std::fs::remove_file(&destination).unwrap();
        std::fs::create_dir(&destination).unwrap();
        assert!(read_project_instruction_target(project.path(), target)
            .await
            .is_err());
        assert!(build_project_instruction_plan(
            &state,
            unregistered.path().to_str().unwrap(),
            "agents",
            ProjectInstructionOperation::Upsert,
            "review-rules",
            "Review every diff.",
        )
        .await
        .is_err());
        assert_eq!(
            canonical_registered_instruction_project(&state, &project_path)
                .await
                .unwrap()
                .to_string_lossy(),
            project_path
        );
    }

    #[tokio::test]
    async fn project_instruction_apply_creates_replaces_and_removes_one_owned_snippet() {
        let app_data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let state = project_instruction_test_state(app_data.path(), project.path()).await;
        let project_path = std::fs::canonicalize(project.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let destination = project.path().join("AGENTS.md");

        for (operation, content) in [
            (ProjectInstructionOperation::Upsert, "Review every diff."),
            (
                ProjectInstructionOperation::Upsert,
                "Review the complete diff.",
            ),
            (ProjectInstructionOperation::Remove, ""),
        ] {
            let plan = build_project_instruction_plan(
                &state,
                &project_path,
                "agents",
                operation,
                "review-rules",
                content,
            )
            .await
            .unwrap();
            let applied = apply_project_instruction(
                &state,
                project_path.clone(),
                "agents".into(),
                operation,
                "review-rules".into(),
                content.into(),
                plan.revision,
                true,
            )
            .await
            .unwrap();
            assert_eq!(applied.result.unwrap().outcome, "succeeded");
            assert_eq!(
                std::fs::read_to_string(&destination).unwrap(),
                applied.plan.proposed
            );
        }
        assert_eq!(std::fs::read_to_string(destination).unwrap(), "");
    }

    #[tokio::test]
    async fn project_instruction_apply_rejects_drift_then_backs_up_exact_bytes() {
        let app_data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let destination = project.path().join("CLAUDE.md");
        std::fs::write(&destination, b"original\n").unwrap();
        let state = project_instruction_test_state(app_data.path(), project.path()).await;
        let project_path = std::fs::canonicalize(project.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let plan = build_project_instruction_plan(
            &state,
            &project_path,
            "claude",
            ProjectInstructionOperation::Upsert,
            "team-rules",
            "Use the reviewed team.",
        )
        .await
        .unwrap();
        std::fs::write(&destination, b"external drift\n").unwrap();

        let stale = apply_project_instruction(
            &state,
            project_path.clone(),
            "claude".into(),
            ProjectInstructionOperation::Upsert,
            "team-rules".into(),
            "Use the reviewed team.".into(),
            plan.revision,
            true,
        )
        .await
        .unwrap();
        assert!(stale.result.is_none());
        assert_eq!(std::fs::read(&destination).unwrap(), b"external drift\n");
        assert!(!backups_dir_for(app_data.path()).exists());

        let fresh_revision = stale.plan.revision;
        let applied = apply_project_instruction(
            &state,
            project_path,
            "claude".into(),
            ProjectInstructionOperation::Upsert,
            "team-rules".into(),
            "Use the reviewed team.".into(),
            fresh_revision,
            true,
        )
        .await
        .unwrap();
        let result = applied.result.unwrap();
        assert_eq!(result.outcome, "succeeded");
        let backup = PathBuf::from(result.backup_path.unwrap());
        assert_eq!(std::fs::read(backup).unwrap(), b"external drift\n");
        assert_eq!(
            std::fs::read_to_string(destination).unwrap(),
            applied.plan.proposed
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn project_instruction_write_failure_rolls_back_without_losing_original() {
        use std::os::unix::fs::PermissionsExt;

        let app_data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let destination = project.path().join("AGENTS.md");
        let original = b"original\n";
        std::fs::write(&destination, original).unwrap();
        let state = project_instruction_test_state(app_data.path(), project.path()).await;
        let project_path = std::fs::canonicalize(project.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let plan = build_project_instruction_plan(
            &state,
            &project_path,
            "agents",
            ProjectInstructionOperation::Upsert,
            "review-rules",
            "Review every diff.",
        )
        .await
        .unwrap();
        std::fs::set_permissions(project.path(), std::fs::Permissions::from_mode(0o500)).unwrap();

        let applied = apply_project_instruction(
            &state,
            project_path,
            "agents".into(),
            ProjectInstructionOperation::Upsert,
            "review-rules".into(),
            "Review every diff.".into(),
            plan.revision,
            true,
        )
        .await
        .unwrap();
        std::fs::set_permissions(project.path(), std::fs::Permissions::from_mode(0o700)).unwrap();

        assert_eq!(applied.result.unwrap().outcome, "rolledBack");
        assert_eq!(std::fs::read(destination).unwrap(), original);
        assert!(state
            .completed_state_database()
            .await
            .unwrap()
            .unwrap()
            .pending_filesystem_operations()
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn project_instruction_parent_recovery_is_idempotent_and_exact() {
        let app_data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let state = project_instruction_test_state(app_data.path(), project.path()).await;
        let database = state.completed_state_database().await.unwrap().unwrap();
        let project_path = std::fs::canonicalize(project.path()).unwrap();
        let directory = project_path.join(".github");
        let destination = directory.join("copilot-instructions.md");
        let proposed = project_instruction_block("guardrails", "Review before applying.");
        let payload = ProjectInstructionApplyOperation {
            project_path: project_path.to_string_lossy().into_owned(),
            target: "copilot".into(),
            destination: destination.to_string_lossy().into_owned(),
            before_hash: None,
            after_hash: render::sha256_hex(proposed.as_bytes()),
            backup_path: None,
        };
        let operation = database
            .prepare_filesystem_operation("project_instruction_apply", &payload)
            .await
            .unwrap();
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(&destination, proposed).unwrap();
        database
            .mark_filesystem_applied(&operation.id)
            .await
            .unwrap();

        recover_project_instruction_operations(&state)
            .await
            .unwrap();
        recover_project_instruction_operations(&state)
            .await
            .unwrap();

        assert!(!destination.exists());
        assert!(database
            .pending_filesystem_operations()
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn project_instruction_recovery_aborts_prepared_and_retains_unexpected_bytes() {
        let app_data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let state = project_instruction_test_state(app_data.path(), project.path()).await;
        let database = state.completed_state_database().await.unwrap().unwrap();
        let project_path = std::fs::canonicalize(project.path()).unwrap();
        let destination = project_path.join("AGENTS.md");
        let proposed = project_instruction_block("guardrails", "Review before applying.");
        let payload = ProjectInstructionApplyOperation {
            project_path: project_path.to_string_lossy().into_owned(),
            target: "agents".into(),
            destination: destination.to_string_lossy().into_owned(),
            before_hash: None,
            after_hash: render::sha256_hex(proposed.as_bytes()),
            backup_path: None,
        };
        database
            .prepare_filesystem_operation("project_instruction_apply", &payload)
            .await
            .unwrap();

        recover_project_instruction_operations(&state)
            .await
            .unwrap();
        assert!(!destination.exists());
        assert!(database
            .pending_filesystem_operations()
            .await
            .unwrap()
            .is_empty());

        database
            .prepare_filesystem_operation("project_instruction_apply", &payload)
            .await
            .unwrap();
        std::fs::write(&destination, b"external bytes\n").unwrap();
        recover_project_instruction_operations(&state)
            .await
            .unwrap();

        let pending = database.pending_filesystem_operations().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert!(pending[0].recovery_error.is_some());
        assert_eq!(std::fs::read(destination).unwrap(), b"external bytes\n");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn project_instruction_inspection_rejects_linked_target_components() {
        use std::os::unix::fs::symlink;

        let app_data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), project.path().join(".github")).unwrap();
        let target = project_instruction_target("copilot").unwrap();
        assert!(read_project_instruction_target(project.path(), target)
            .await
            .is_err());
        assert!(!outside.path().join("copilot-instructions.md").exists());

        let state = project_instruction_test_state(app_data.path(), project.path()).await;
        let database = state.completed_state_database().await.unwrap().unwrap();
        let project_path = std::fs::canonicalize(project.path()).unwrap();
        let destination = project_path.join(".github/copilot-instructions.md");
        let proposed = project_instruction_block("guardrails", "Review before applying.");
        let payload = ProjectInstructionApplyOperation {
            project_path: project_path.to_string_lossy().into_owned(),
            target: "copilot".into(),
            destination: destination.to_string_lossy().into_owned(),
            before_hash: None,
            after_hash: render::sha256_hex(proposed.as_bytes()),
            backup_path: None,
        };
        let operation = database
            .prepare_filesystem_operation("project_instruction_apply", &payload)
            .await
            .unwrap();
        std::fs::write(outside.path().join("copilot-instructions.md"), &proposed).unwrap();
        database
            .mark_filesystem_applied(&operation.id)
            .await
            .unwrap();

        recover_project_instruction_operations(&state)
            .await
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(outside.path().join("copilot-instructions.md")).unwrap(),
            proposed
        );
        let pending = database.pending_filesystem_operations().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert!(pending[0].recovery_error.is_some());
    }
}
