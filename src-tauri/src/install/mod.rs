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

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use chrono::Utc;
use tauri::{AppHandle, State};
use tokio::io::AsyncWriteExt;

use crate::corpus;
use crate::error::AppError;
use crate::registry;
use crate::render;
use crate::state::{AppState, AuthorizedMcpProject};
use crate::types::{
    AgentApprovalAction, AgentDiff, AgentInstallIdentity, AgentMutationPlan, AgentPlanItem,
    AgentReference, AgentSourceResult, AgentVersionSnapshot, InstallRecord, InstallState,
    InstalledAgent, ProjectInfo, Tool, ToolInfo, ToolVersion, UpdateKind,
};
use crate::util::fs::{atomic_write, read_capped};

mod history;

/// Cap on an installed agent file we read back during reconciliation.
const MAX_INSTALLED_BYTES: u64 = 4 * 1024 * 1024;
const MAX_REGISTERED_PROJECTS: usize = 200;
const MAX_PROJECT_REGISTRY_BYTES: u64 = 64 * 1024;
const MAX_LEDGER_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const MAX_AGENT_HISTORY_ENTRIES: usize = 10;

// ---------- Ledger persistence ----------

fn ledger_path(app: &AppHandle) -> Result<PathBuf, AppError> {
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

pub(crate) async fn load_ledger(
    app: &AppHandle,
    state: &AppState,
) -> Result<Vec<InstallRecord>, AppError> {
    corpus::ensure_corpus(app, state).await?;
    let built_in = crate::agents::inspect_builtin_agent_source(&state.app_data_dir).await?;
    let path = ledger_path(app)?;
    load_migrated_ledger_path(&path, Some(&built_in), &now_iso()).await
}

async fn load_ledger_for_state(state: &AppState) -> Result<Vec<InstallRecord>, AppError> {
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

async fn save_ledger(app: &AppHandle, records: &[InstallRecord]) -> Result<(), AppError> {
    save_ledger_for(&corpus::app_data_dir(app)?, records).await
}

async fn save_ledger_for(app_data_dir: &Path, records: &[InstallRecord]) -> Result<(), AppError> {
    let path = ledger_path_for(app_data_dir);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::Io {
                message: format!("create state dir {}: {e}", parent.display()),
            })?;
    }
    let bytes = serde_json::to_vec_pretty(records).map_err(|e| AppError::Io {
        message: format!("serialize installs.json: {e}"),
    })?;
    atomic_write(&path, &bytes).await
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
    let records = serde_json::from_slice(&original).map_err(|error| AppError::Io {
        message: format!("parse installs.json: {error}"),
    })?;
    let (records, changed) = migrate_install_records(records, built_in)?;
    if !changed {
        return Ok(records);
    }

    let parent = path.parent().ok_or_else(|| AppError::Io {
        message: format!("install ledger has no parent: {}", path.display()),
    })?;
    let backup = parent.join(format!("installs.migration-{}.json.bak", fs_stamp(stamp)));
    atomic_write(&backup, &original).await?;
    let migrated = serde_json::to_vec_pretty(&records).map_err(|error| AppError::Io {
        message: format!("serialize migrated installs.json: {error}"),
    })?;
    atomic_write(path, &migrated).await?;
    Ok(records)
}

async fn prune_migration_backups(app: &AppHandle) -> Result<(), AppError> {
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
fn backups_dir(app: &AppHandle) -> Result<PathBuf, AppError> {
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
        disabled_path: None,
        source_snapshot_hash: source_hash.to_string(),
        capabilities: Vec::new(),
        publisher_key: None,
        publisher_verified: false,
        installed_at: installed_at.to_string(),
        corpus_version: corpus_version.to_string(),
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

pub(crate) async fn do_install(
    app: &AppHandle,
    state: &AppState,
    reference: AgentReference,
    tool: Tool,
    project_path: Option<String>,
    confirmed: bool,
) -> Result<InstallRecord, AppError> {
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
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

async fn do_install_locked(
    app: &AppHandle,
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
    let existing_dest = existing_record
        .as_ref()
        .map(|record| PathBuf::from(&record.dest));
    let targets = install_target_paths(
        &agent,
        &tool,
        &home,
        proot.as_deref(),
        existing_dest.as_deref(),
    )?;
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
    let prior_snapshot = match (existing_record.as_ref(), prior_paths.is_empty()) {
        (Some(record), false) => Some(snapshot_record(state, record, &prior_paths).await?),
        _ => None,
    };
    let write_result = write_agent_files_to(
        &agent,
        &raw,
        &tool,
        &home,
        proot.as_deref(),
        Some(&backups),
        &package.source_hash,
        &package.body_hash,
        &revision,
        &now_iso(),
        existing_dest.as_deref(),
    )
    .await;
    let mut record = match write_result {
        Ok(record) => record,
        Err(error) => {
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
    if let Err(error) = save_ledger(app, &ledger).await {
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

#[derive(Clone)]
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
    let candidates = candidate_dests(&agent, &raw, &tool, &home, proot.as_deref())?;
    let existing = candidates
        .iter()
        .filter(|path| path.exists())
        .cloned()
        .collect::<Vec<_>>();
    if existing.is_empty() {
        return Err(AppError::InvalidArgument {
            message: "no existing Agent destination is available to track".into(),
        });
    }
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
    let (_bytes, rendered_hash) = render::render_with_hash(agent, raw, tool)?;
    let paths = candidate_dests(agent, raw, tool, home, project_root)?;
    let primary = paths.iter().find(|p| p.exists()).unwrap_or(&paths[0]);
    Ok(record_for(
        agent,
        primary,
        tool,
        project_root,
        rendered_hash,
        source_hash,
        body_hash,
        corpus_version,
        installed_at,
    ))
}

/// Possible physical destinations for one logical install. App-authored files
/// historically used the catalog filename slug; upstream `convert.sh` uses
/// `slugify(name)` for transform tools. Recognize both without changing the
/// catalog's stable identity.
fn candidate_dests(
    agent: &crate::types::Agent,
    raw: &str,
    tool: &str,
    home: &Path,
    project_root: Option<&Path>,
) -> Result<Vec<PathBuf>, AppError> {
    let mut paths = render::dests(tool, &agent.slug, home, project_root)?;
    let conversion_slug = render::output_slug(agent, raw, tool);
    if conversion_slug != agent.slug {
        for path in render::dests(tool, &conversion_slug, home, project_root)? {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    Ok(paths)
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

fn move_managed_files(
    sources: &[PathBuf],
    destinations: &[PathBuf],
    expected_hash: &str,
) -> Result<(), AppError> {
    move_managed_files_with(
        sources,
        destinations,
        expected_hash,
        |source, destination| std::fs::rename(source, destination),
    )
}

fn move_managed_files_with<F>(
    sources: &[PathBuf],
    destinations: &[PathBuf],
    expected_hash: &str,
    mut rename: F,
) -> Result<(), AppError>
where
    F: FnMut(&Path, &Path) -> std::io::Result<()>,
{
    if sources.is_empty() || sources.len() != destinations.len() {
        return Err(AppError::InvalidArgument {
            message: "Agent lifecycle move requires matching source and destination files".into(),
        });
    }
    for (source, destination) in sources.iter().zip(destinations) {
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
        if render::sha256_hex(&bytes) != expected_hash {
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
    let (bytes, rendered_hash) = render::render_with_hash(agent, raw, tool)?;
    let paths = install_target_paths(agent, tool, home, project_root, preferred_dest)?;
    for dest in &paths {
        if let Some(bdir) = backup_dir {
            backup_if_differs(dest, bytes.as_bytes(), bdir, installed_at).await?;
        }
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| AppError::Io {
                    message: format!("create {}: {e}", parent.display()),
                })?;
        }
        atomic_write(dest, bytes.as_bytes()).await?;
    }
    Ok(record_for(
        agent,
        &paths[0],
        tool,
        project_root,
        rendered_hash,
        source_hash,
        body_hash,
        corpus_version,
        installed_at,
    ))
}

fn install_target_paths(
    agent: &crate::types::Agent,
    tool: &str,
    home: &Path,
    project_root: Option<&Path>,
    preferred_dest: Option<&Path>,
) -> Result<Vec<PathBuf>, AppError> {
    let mut paths = render::dests(tool, &agent.slug, home, project_root)?;
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

async fn resolve_command_reference(
    app: &AppHandle,
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
        if destinations
            .iter()
            .any(|path| path == Path::new(&record.dest))
        {
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
        let preferred = existing.map(|record| Path::new(&record.dest));
        let paths = match install_target_paths(agent, &tool, &home, project, preferred) {
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
        paths.extend(install_target_paths(
            agent,
            &plan.tool,
            &home,
            project,
            existing.map(|record| Path::new(&record.dest)),
        )?);
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
            app,
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
    rendered_hash: &str,
    authorization: Option<&AuthorizedMcpProject>,
) -> Result<(), AppError> {
    for path in paths.iter().rev() {
        if let Some(authorization) = authorization {
            crate::skills::install::remove_project_file(
                authorization.root(),
                &capability_relative(authorization, path)?,
                rendered_hash,
            )?;
        } else {
            let bytes = read_capped(path, MAX_INSTALLED_BYTES).await?;
            if render::sha256_hex(&bytes) != rendered_hash {
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
    let paths = install_target_paths(&agent, &tool, &home, project, None)?;
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
    let (rendered, rendered_hash) = render::render_with_hash(&agent, &raw, &tool)?;
    let mut created = Vec::with_capacity(paths.len());
    for path in &paths {
        let result = if let Some(authorization) = authorization {
            crate::skills::install::create_project_file(
                authorization.root(),
                &capability_relative(authorization, path)?,
                rendered.as_bytes(),
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
                file.write_all(rendered.as_bytes())
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
            return match rollback_clean_agent_files(&created, &rendered_hash, authorization).await {
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
        rendered_hash.clone(),
        &package.source_hash,
        &package.body_hash,
        &revision,
        &now_iso(),
    );
    record.source_id = reference.source_id.clone();
    record.relative_path = reference.relative_path.clone();
    record.source_snapshot_hash = package.source_hash;
    record.capabilities = package.capabilities;
    record.publisher_key = package.publisher_key;
    record.publisher_verified = package.publisher_verified;
    ledger.push(record.clone());
    if let Err(error) = save_ledger_for(&state.app_data_dir, &ledger).await {
        return match rollback_clean_agent_files(&created, &rendered_hash, authorization).await {
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
        move_managed_files(sources, destinations, &records[index].rendered_hash)?;
    }
    let stored_disabled = records[index].disabled_path.clone();
    records[index].disabled_path = if enable {
        None
    } else {
        Some(disabled[0].to_string_lossy().into_owned())
    };
    if let Err(error) = save_ledger_for(&state.app_data_dir, &records).await {
        records[index].disabled_path = stored_disabled;
        let rollback = if let Some(authorization) = authorization {
            for (source, destination) in sources.iter().zip(destinations).rev() {
                crate::skills::install::rename_project_file(
                    authorization.root(),
                    &capability_relative(authorization, destination)?,
                    &capability_relative(authorization, source)?,
                    &records[index].rendered_hash,
                )?;
            }
            Ok(())
        } else {
            move_managed_files(destinations, sources, &records[index].rendered_hash)
        };
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error("move Agent install", error, rollback)),
        };
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
pub async fn install_agent(
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
    let bytes = read_capped(
        paths.first().ok_or_else(|| AppError::InvalidArgument {
            message: "tracked Agent has no destination".into(),
        })?,
        MAX_INSTALLED_BYTES,
    )
    .await?;
    history::create_snapshot(
        &state.app_data_dir,
        &install_identity(record),
        paths,
        &record.source_snapshot_hash,
        &render::sha256_hex(&bytes),
        &now_iso(),
    )
    .await
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
    app: AppHandle,
    state: State<'_, AppState>,
    source_id: String,
    relative_path: String,
    tool: Tool,
    project_path: Option<String>,
) -> Result<InstallRecord, AppError> {
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
    let mut records = load_ledger(&app, &state).await?;
    let index = install_record_index(
        &records,
        &source_id,
        &relative_path,
        &tool,
        project_path.as_deref(),
    )?;
    if records[index].disabled_path.is_some() {
        return Err(AppError::InvalidArgument {
            message: "Agent install is already disabled".into(),
        });
    }
    let active = resolved_record_paths(&state, &records[index]).await?;
    let disabled = active
        .iter()
        .map(|path| disabled_destination(path))
        .collect::<Result<Vec<_>, _>>()?;
    snapshot_record(&state, &records[index], &active).await?;
    move_managed_files(&active, &disabled, &records[index].rendered_hash)?;
    records[index].disabled_path = Some(disabled[0].to_string_lossy().into_owned());
    if let Err(error) = save_ledger(&app, &records).await {
        return match move_managed_files(&disabled, &active, &records[index].rendered_hash) {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error("disable Agent", error, rollback)),
        };
    }
    Ok(records[index].clone())
}

#[tauri::command]
pub async fn enable_agent(
    app: AppHandle,
    state: State<'_, AppState>,
    source_id: String,
    relative_path: String,
    tool: Tool,
    project_path: Option<String>,
) -> Result<InstallRecord, AppError> {
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
    let mut records = load_ledger(&app, &state).await?;
    let index = install_record_index(
        &records,
        &source_id,
        &relative_path,
        &tool,
        project_path.as_deref(),
    )?;
    let stored_disabled =
        records[index]
            .disabled_path
            .clone()
            .ok_or_else(|| AppError::InvalidArgument {
                message: "Agent install is not disabled".into(),
            })?;
    let active = resolved_record_paths(&state, &records[index]).await?;
    let disabled = active
        .iter()
        .map(|path| disabled_destination(path))
        .collect::<Result<Vec<_>, _>>()?;
    if disabled[0] != Path::new(&stored_disabled) {
        return Err(AppError::InvalidArgument {
            message: "stored Agent disabled path does not match its destination".into(),
        });
    }
    move_managed_files(&disabled, &active, &records[index].rendered_hash)?;
    records[index].disabled_path = None;
    if let Err(error) = save_ledger(&app, &records).await {
        records[index].disabled_path = Some(stored_disabled);
        return match move_managed_files(&active, &disabled, &records[index].rendered_hash) {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error("enable Agent", error, rollback)),
        };
    }
    Ok(records[index].clone())
}

#[tauri::command]
pub async fn agent_version_rollback(
    app: AppHandle,
    state: State<'_, AppState>,
    source_id: String,
    relative_path: String,
    tool: Tool,
    project_path: Option<String>,
    snapshot_id: String,
) -> Result<InstallRecord, AppError> {
    rollback_agent_version(
        &app,
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
    app: &AppHandle,
    state: &AppState,
    reference: AgentReference,
    tool: Tool,
    project_path: Option<String>,
    snapshot_id: String,
) -> Result<InstallRecord, AppError> {
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
    let mut records = load_ledger(app, state).await?;
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
    let prior = snapshot_record(state, &records[index], &paths).await?;
    let selected =
        history::restore_snapshot(&state.app_data_dir, &identity, &snapshot_id, &paths).await?;
    records[index].source_hash = selected.source_hash.clone();
    records[index].source_snapshot_hash = selected.source_hash;
    records[index].rendered_hash = selected.rendered_hash;
    records[index].installed_at = now_iso();
    if let Err(error) = save_ledger(app, &records).await {
        return match history::restore_snapshot(&state.app_data_dir, &identity, &prior.id, &paths)
            .await
        {
            Ok(_) => Err(error),
            Err(rollback) => Err(rollback_error("rollback Agent version", error, rollback)),
        };
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
                    app,
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
    let (proposed, _hash) = render::render_with_hash(&agent, &raw, &tool)?;

    let home = tool_home(&state, &tool).await?;
    let proot = project_path.as_ref().map(PathBuf::from);
    let ledger = load_ledger(&app, &state).await?;
    let ledger_dest = ledger
        .iter()
        .find(|record| {
            record.source_id == reference.source_id
                && record.relative_path == reference.relative_path
                && record.tool == tool
                && record.project_path == project_path
        })
        .map(|r| PathBuf::from(&r.dest));
    let candidates = candidate_dests(&agent, &raw, &tool, &home, proot.as_deref())?;
    let dest = ledger_dest
        .as_ref()
        .or_else(|| candidates.iter().find(|p| p.exists()))
        .unwrap_or(&candidates[0]);
    let on_disk = match read_capped(dest, MAX_INSTALLED_BYTES).await {
        Ok(b) => Some(String::from_utf8_lossy(&b).into_owned()),
        Err(_) => None,
    };
    let differs = on_disk.as_deref() != Some(proposed.as_str());
    Ok(AgentDiff {
        slug: agent.slug,
        tool,
        project_path,
        dest: dest.to_string_lossy().to_string(),
        on_disk,
        proposed,
        differs,
    })
}

/// Uninstall: remove the written file(s) and the ledger row.
#[tauri::command]
pub async fn uninstall_agent(
    app: AppHandle,
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
    do_uninstall(&app, &state, reference, tool, project_path).await
}

async fn do_uninstall(
    app: &AppHandle,
    state: &AppState,
    reference: AgentReference,
    tool: Tool,
    project_path: Option<String>,
) -> Result<(), AppError> {
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
    do_uninstall_locked(app, state, reference, tool, project_path).await
}

async fn do_uninstall_locked(
    app: &AppHandle,
    state: &AppState,
    reference: AgentReference,
    tool: Tool,
    project_path: Option<String>,
) -> Result<(), AppError> {
    let mut ledger = load_ledger(app, state).await?;
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
    for path in &existing {
        if let Err(error) = remove_file_strict(path).await {
            if let Some(snapshot) = &snapshot {
                if let Err(rollback) = history::restore_snapshot(
                    &state.app_data_dir,
                    &install_identity(&record),
                    &snapshot.id,
                    &existing,
                )
                .await
                {
                    return Err(rollback_error("uninstall Agent", error, rollback));
                }
            }
            return Err(error);
        }
    }
    ledger.remove(index);
    if let Err(error) = save_ledger(app, &ledger).await {
        if let Some(snapshot) = &snapshot {
            if let Err(rollback) = history::restore_snapshot(
                &state.app_data_dir,
                &install_identity(&record),
                &snapshot.id,
                &existing,
            )
            .await
            {
                return Err(rollback_error("save Agent uninstall", error, rollback));
            }
        }
        return Err(error);
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
    do_uninstall(app, state, reference, tool, project_path).await
}

/// Forget a project WITHOUT touching the files on disk: drop every ledger row
/// whose `project_path` matches, then save. The agent/skill files this app
/// wrote stay exactly where they are — this only makes the app stop tracking
/// them, so the project leaves the Projects list (the Foreign sweep re-scans
/// only project roots the ledger still references, so dropped rows don't come
/// back). Callers that want the files gone use `uninstall_agent` per row first.
#[tauri::command]
pub async fn project_forget(
    app: AppHandle,
    state: State<'_, AppState>,
    project_path: String,
) -> Result<(), AppError> {
    let mut ledger = load_ledger(&app, &state).await?;
    prune_project_rows(&mut ledger, &project_path);
    save_ledger(&app, &ledger).await?;
    Ok(())
}

/// Drop every ledger row whose `project_path` matches, keeping all others
/// (other projects AND user-global rows). Pure so it's unit-testable without an
/// AppHandle; the command just wraps it with load/save.
fn prune_project_rows(records: &mut Vec<InstallRecord>, project_path: &str) {
    records.retain(|r| r.project_path.as_deref() != Some(project_path));
}

pub(crate) async fn mcp_reconcile_agent_installs(
    state: &AppState,
) -> Result<Vec<InstalledAgent>, AppError> {
    let _guard = state.skill_installs_write_lock.lock().await;
    let _file_guard = lock_agent_installs_async(state.app_data_dir.clone()).await?;
    let ledger = load_ledger_for_state(state).await?;
    let agent_sources = crate::agents::inspect_agent_sources(&state.app_data_dir).await?;
    let mut installed = Vec::with_capacity(ledger.len());
    for record in ledger {
        let destination_hash = match read_capped(Path::new(&record.dest), MAX_INSTALLED_BYTES).await
        {
            Ok(bytes) => Some(render::sha256_hex(&bytes)),
            Err(AppError::Io { .. }) if !Path::new(&record.dest).exists() => None,
            Err(error) => return Err(error),
        };
        let disabled_hash = match record.disabled_path.as_deref() {
            Some(path) if Path::new(path).exists() => Some(render::sha256_hex(
                &read_capped(Path::new(path), MAX_INSTALLED_BYTES).await?,
            )),
            _ => None,
        };
        let package = find_agent_package(&agent_sources, &record);
        let lifecycle = classify_install(ReconcileFacts {
            tracked: true,
            destination_hash: destination_hash.as_deref(),
            disabled_path: record.disabled_path.is_some(),
            disabled_hash: disabled_hash.as_deref(),
            rendered_hash: &record.rendered_hash,
            installed_source_hash: &record.source_snapshot_hash,
            current_source_hash: package.map(|package| package.source_hash.as_str()),
        });
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
pub async fn installs_reconcile(
    app: AppHandle,
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
    let mut ledger = load_ledger(&app, &state).await?;
    let agent_sources = crate::agents::inspect_agent_sources(&state.app_data_dir).await?;
    let mut out = Vec::with_capacity(ledger.len());
    for r in &ledger {
        let dest = PathBuf::from(&r.dest);
        let disk_hash = if dest.exists() {
            read_capped(&dest, MAX_INSTALLED_BYTES)
                .await
                .ok()
                .map(|b| render::sha256_hex(&b))
        } else {
            None
        };
        let disabled_hash = match r.disabled_path.as_deref().map(Path::new) {
            Some(path) if path.exists() => read_capped(path, MAX_INSTALLED_BYTES)
                .await
                .ok()
                .map(|bytes| render::sha256_hex(&bytes)),
            _ => None,
        };
        let package = find_agent_package(&agent_sources, r);
        let current_source_hash = package.map(|package| package.source_hash.as_str());
        let st = classify_install(ReconcileFacts {
            tracked: true,
            destination_hash: disk_hash.as_deref(),
            disabled_path: r.disabled_path.is_some(),
            disabled_hash: disabled_hash.as_deref(),
            rendered_hash: &r.rendered_hash,
            installed_source_hash: &r.source_snapshot_hash,
            current_source_hash,
        });
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
                let disk = read_capped(&byte_path, MAX_INSTALLED_BYTES).await.ok();
                let canonical = matches!(
                    (raw.as_deref(), disk.as_deref()),
                    (Some(rw), Some(db)) if bytes_match_render(&agent, rw, tool, db)
                );
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
        ledger.extend(adopted);
        save_ledger(&app, &ledger).await?;
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

/// Open a path in the OS file manager (Finder / Explorer / xdg-open).
/// Best-effort: returns an error the UI can toast if the path is missing or no
/// opener is available. Used by the Tools panel's "Reveal" affordance.
#[tauri::command]
pub async fn reveal_path(path: String) -> Result<(), AppError> {
    tokio::task::spawn_blocking(move || {
        #[cfg(target_os = "macos")]
        let program = "open";
        #[cfg(target_os = "windows")]
        let program = "explorer";
        #[cfg(all(unix, not(target_os = "macos")))]
        let program = "xdg-open";
        std::process::Command::new(program)
            .arg(&path)
            .status()
            .map(|_| ())
            .map_err(|e| AppError::Io {
                message: format!("could not open {path}: {e}"),
            })
    })
    .await
    .map_err(|e| AppError::Io {
        message: e.to_string(),
    })?
}

/// The `<bin> --version`-style probe command for a tool, or `None` when we don't
/// know one. Best-effort and uneven by nature — GUI tools may not ship a CLI.
fn version_cmd(tool: &str) -> Option<(&'static str, Vec<&'static str>)> {
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

// ---------- Loadouts (Agentfile) ----------

/// Portable manifest of an install set — "set up a new Mac in one click".
/// JSON so it's diffable + shareable; `tool` uses the camelCase wire value.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Agentfile {
    /// Format version.
    agentfile: u32,
    installs: Vec<LoadoutEntry>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
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
) -> Result<u32, AppError> {
    let ledger = load_ledger(&app, &state).await?;
    let installs: Vec<LoadoutEntry> = ledger
        .iter()
        .map(|r| LoadoutEntry {
            slug: r.slug.clone(),
            tool: r.tool.clone(),
            project_path: r.project_path.clone(),
        })
        .collect();
    let n = installs.len() as u32;
    let af = Agentfile {
        agentfile: 1,
        installs,
    };
    let bytes = serde_json::to_vec_pretty(&af).map_err(|e| AppError::Io {
        message: format!("serialize Agentfile: {e}"),
    })?;
    atomic_write(Path::new(&path), &bytes).await?;
    Ok(n)
}

/// Import an Agentfile from `path`, installing every entry. Returns the records
/// that installed successfully (entries that fail — e.g. a project tool whose
/// path no longer exists — are skipped, not fatal).
#[tauri::command]
pub async fn loadout_import(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<Vec<InstallRecord>, AppError> {
    let bytes = read_capped(Path::new(&path), MAX_INSTALLED_BYTES).await?;
    let af: Agentfile = serde_json::from_slice(&bytes).map_err(|e| AppError::Io {
        message: format!("parse Agentfile: {e}"),
    })?;
    let mut out = Vec::with_capacity(af.installs.len());
    corpus::ensure_corpus(&app, &state).await?;
    let sources = crate::agents::inspect_agent_sources(&state.app_data_dir).await?;
    for e in af.installs {
        if let Ok(reference) = resolve_reference_request(&sources, None, None, Some(&e.slug)) {
            if let Ok(record) =
                do_install(&app, &state, reference, e.tool, e.project_path, false).await
            {
                out.push(record);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

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
            disabled_path: None,
            source_snapshot_hash: String::new(),
            capabilities: Vec::new(),
            publisher_key: None,
            publisher_verified: false,
            installed_at: String::new(),
            corpus_version: String::new(),
        }
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
    async fn batch_failure_restores_preexisting_files_and_removes_created_files() {
        let root = tempfile::tempdir().unwrap();
        let existing = root.path().join("existing.md");
        let created = root.path().join("created.md");
        std::fs::write(&existing, b"before").unwrap();
        let snapshot = capture_batch_files(&[existing.clone(), created.clone()])
            .await
            .unwrap();
        std::fs::write(&existing, b"after").unwrap();
        std::fs::write(&created, b"new").unwrap();

        restore_batch_files(&snapshot).await.unwrap();

        assert_eq!(std::fs::read(&existing).unwrap(), b"before");
        assert!(!created.exists());
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
        let agent = sample_agent();
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
            disabled_path: None,
            source_snapshot_hash: "sh".into(),
            capabilities: Vec::new(),
            publisher_key: None,
            publisher_verified: false,
            installed_at: "2026-06-05T00:00:00Z".into(),
            corpus_version: "v".into(),
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
}
