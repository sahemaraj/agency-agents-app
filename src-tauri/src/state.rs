//! Tauri-managed application state.
//!
//! Holds the agency subsystems:
//! the corpus cache, persisted settings (the source of truth for the
//! network/feature gates), the updater mirror, and the resolved
//! app-data directory that the corpus / install / github / updater
//! modules derive their paths from.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::State;
use tokio::sync::{Mutex, RwLock};

use crate::commands::settings::{self, SettingsLoadState};
use crate::commands::updater::UpdaterState;
use crate::error::AppError;
use crate::types::McpAuditEntry;

pub const MCP_AUDIT_MAX_ENTRIES: usize = 500;
pub const MCP_AUDIT_FIELD_CHARS_CAP: usize = 128;
pub const MCP_AUDIT_PROJECT_CHARS_CAP: usize = 4096;
const MCP_AUDIT_MAX_BYTES: usize = 1024 * 1024;
const MCP_AUDIT_PROCESS_LOCK_DEADLINE: Duration = Duration::from_secs(10);
const MCP_AUDIT_OS_LOCK_DEADLINE: Duration = Duration::from_secs(1);
const MCP_AUDIT_LOCK_RETRY: Duration = Duration::from_millis(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PersistenceDocument {
    pub(crate) name: &'static str,
    pub(crate) relative_path: &'static str,
    pub(crate) version: u32,
    pub(crate) max_bytes: u64,
    pub(crate) parser: &'static str,
    pub(crate) validator: &'static str,
}

impl PersistenceDocument {
    const fn json(
        name: &'static str,
        relative_path: &'static str,
        max_bytes: u64,
        validator: &'static str,
    ) -> Self {
        Self {
            name,
            relative_path,
            version: 1,
            max_bytes,
            parser: "json",
            validator,
        }
    }
}

pub(crate) const PERSISTENCE_INVENTORY: &[PersistenceDocument] = &[
    PersistenceDocument::json("settings", "settings.json", 1_048_576, "settings"),
    PersistenceDocument::json("catalog", "state/catalog.json", 65_536, "catalog_source"),
    PersistenceDocument::json(
        "control_center",
        "state/control-center.json",
        4_194_304,
        "control_center",
    ),
    PersistenceDocument::json(
        "skill_sources",
        "state/skill-sources.json",
        1_048_576,
        "skill_sources",
    ),
    PersistenceDocument::json(
        "skill_trust",
        "state/skill-trust.json",
        1_048_576,
        "skill_trust",
    ),
    PersistenceDocument::json(
        "skill_drafts",
        "state/skill-drafts.json",
        16_777_216,
        "skill_drafts",
    ),
    PersistenceDocument::json(
        "skill_library",
        "state/skill-folders.json",
        4_194_304,
        "skill_library",
    ),
    PersistenceDocument::json(
        "skill_installs",
        "state/skill-installs.json",
        16_777_216,
        "skill_installs",
    ),
    PersistenceDocument::json(
        "agent_sources",
        "state/agent-sources.json",
        1_048_576,
        "agent_sources",
    ),
    PersistenceDocument::json(
        "agent_drafts",
        "state/agent-drafts.json",
        8_388_608,
        "agent_drafts",
    ),
    PersistenceDocument::json(
        "agent_library",
        "state/agent-library.json",
        1_048_576,
        "agent_library",
    ),
    PersistenceDocument::json("installs", "state/installs.json", 16_777_216, "installs"),
    PersistenceDocument::json(
        "ollama_deployments",
        "state/ollama-deployments.json",
        4_194_304,
        "ollama_deployments",
    ),
    PersistenceDocument::json("projects", "state/projects.json", 65_536, "projects"),
    PersistenceDocument::json("experts", "state/experts.json", 4_194_304, "experts"),
    PersistenceDocument::json(
        "expert_activation_requests",
        "state/expert-activation-requests.json",
        524_288,
        "expert_activation_requests",
    ),
    PersistenceDocument::json(
        "expert_activations",
        "state/expert-activations.json",
        524_288,
        "expert_activations",
    ),
    PersistenceDocument::json(
        "expert_runs",
        "state/expert-runs.json",
        4_194_304,
        "expert_runs",
    ),
    PersistenceDocument {
        name: "mcp_audit",
        relative_path: "state/mcp-audit.jsonl",
        version: 1,
        max_bytes: 1_048_576,
        parser: "jsonl",
        validator: "mcp_audit",
    },
];
#[cfg(test)]
static MCP_AUDIT_FAILURES: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<PathBuf, usize>>,
> = std::sync::OnceLock::new();
static MCP_AUDIT_PROCESS_LOCKS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<PathBuf, std::sync::Weak<tokio::sync::Mutex<()>>>>,
> = std::sync::OnceLock::new();

#[derive(Clone)]
pub struct AuthorizedMcpProject {
    identity: String,
    root: Arc<File>,
}

impl AuthorizedMcpProject {
    pub fn identity(&self) -> &str {
        &self.identity
    }

    pub(crate) fn root(&self) -> &File {
        &self.root
    }
}

#[derive(Clone)]
pub struct McpProjectAuthorization(pub Option<AuthorizedMcpProject>);

/// Shared application state. Registered via `Builder::manage()`.
pub struct AppState {
    /// Resolved app-data root — the OS-canonical
    /// `~/Library/Application Support/com.zerologic.agency-agents-app/` directory. The
    /// corpus, install ledger, github cache, and settings file all
    /// derive their paths from this; the security gates that check "is
    /// this path inside our app data dir?" anchor on it too.
    pub app_data_dir: PathBuf,

    #[cfg(not(test))]
    pub(crate) storage_lease: std::sync::Mutex<Option<crate::state_db::StorageLease>>,

    #[cfg(not(test))]
    pub(crate) state_database: crate::state_db::StateDatabase,

    /// Phase 1 (corpus) — memoized in-memory corpus (parsed agents +
    /// index). Built lazily on the first `corpus_*` command (seed + parse
    /// + persist index), then served from this cache. `corpus_refresh`
    ///
    /// swaps the inner Arc after re-indexing the freshly-fetched tree.
    /// Mirrors the `categories_cache` lazy-`Option<Arc<_>>` pattern.
    pub corpus_cache: Arc<Mutex<Option<Arc<crate::corpus::Corpus>>>>,

    /// Single-flight mutex for `corpus_refresh`, same contract as
    /// `catalog_refresh_in_flight`.
    pub corpus_refresh_in_flight: Arc<Mutex<()>>,

    /// Serializes read-modify-write updates to `state/skill-sources.json`.
    pub skill_sources_write_lock: Arc<Mutex<()>>,

    /// Serializes read-modify-write updates to `state/skill-installs.json`.
    pub skill_installs_write_lock: Arc<Mutex<()>>,

    /// Serializes read-modify-write updates to `state/skill-folders.json`.
    pub skill_folders_write_lock: Arc<Mutex<()>>,

    /// Persisted user settings (Phase 12d). Three-state container that
    /// distinguishes file-absent (defaults apply) from file-corrupt
    /// (fail closed — every outbound call denied until repaired).
    /// `require_network` consults this on the first line of every
    /// network-touching command.
    pub settings: Arc<RwLock<SettingsLoadState>>,

    /// Phase 15 — in-memory mirror of the latest update check + cached
    /// `Available` payload. The auto-check scheduler updates this on
    /// every wake, and `update_install` validates the caller-supplied
    /// version arg against the cached entry to defend against UI
    /// staleness. See `crate::commands::updater::UpdaterState` for the
    /// shape and the rationale.
    pub updater_state: Arc<RwLock<UpdaterState>>,
}

impl AppState {
    pub(crate) fn state_database_if_present(&self) -> Option<crate::state_db::StateDatabase> {
        #[cfg(not(test))]
        return Some(self.state_database.clone());

        #[cfg(test)]
        crate::state_db::StateDatabase::existing(&self.app_data_dir)
    }

    fn release_storage_lease(&self) -> Result<(), AppError> {
        #[cfg(not(test))]
        {
            self.storage_lease
                .lock()
                .map_err(|_| AppError::Internal {
                    message: "storage lease lock is poisoned".into(),
                })?
                .take();
        }
        Ok(())
    }

    fn reacquire_storage_lease(&self) -> Result<(), AppError> {
        #[cfg(not(test))]
        {
            let lease = crate::state_db::StorageLease::shared(&self.app_data_dir)?;
            *self.storage_lease.lock().map_err(|_| AppError::Internal {
                message: "storage lease lock is poisoned".into(),
            })? = Some(lease);
        }
        Ok(())
    }

    pub(crate) async fn completed_state_database(
        &self,
    ) -> Result<Option<crate::state_db::StateDatabase>, AppError> {
        let Some(database) = self.state_database_if_present() else {
            return Ok(None);
        };
        match database.migration_state().await? {
            crate::types::StorageMigrationState::Complete => {
                crate::corpus::register_control_center_document(&database).await?;
                Ok(Some(database))
            }
            crate::types::StorageMigrationState::Legacy
            | crate::types::StorageMigrationState::InProgress
            | crate::types::StorageMigrationState::Corrupt => Ok(None),
            crate::types::StorageMigrationState::Unsupported => Err(AppError::StorageUnsupported {
                found: crate::state_db::SCHEMA_VERSION.saturating_add(1),
                supported: crate::state_db::SCHEMA_VERSION,
            }),
        }
    }

    /// Build the state at startup. Resolves the app-data directory and
    /// loads persisted settings; the corpus and updater caches start
    /// empty and hydrate lazily on first use.
    pub fn build() -> Result<Self, AppError> {
        let app_data_dir = resolve_app_data_dir()?;
        if !app_data_dir.exists() {
            std::fs::create_dir_all(&app_data_dir).map_err(|e| AppError::Io {
                message: format!(
                    "could not create app data dir {}: {}",
                    app_data_dir.display(),
                    e
                ),
            })?;
        }

        #[cfg(not(test))]
        let storage_lease = crate::state_db::StorageLease::shared(&app_data_dir)?;
        #[cfg(not(test))]
        let state_database = crate::state_db::StateDatabase::open(&app_data_dir)?;
        #[cfg(not(test))]
        if state_database.migration_state_blocking()?
            == crate::types::StorageMigrationState::Complete
        {
            crate::corpus::register_control_center_document_blocking(&state_database)?;
        }

        // Load settings synchronously at startup. The loader handles
        // file-absent (FirstLaunch → defaults), file-corrupt (Corrupt →
        // fail closed in `require_network`), and good parse (Loaded(s)).
        // Tracing warnings for corrupt cases happen inside the loader.
        let settings_state = settings::load_at_startup(&app_data_dir);
        if matches!(settings_state, SettingsLoadState::Corrupt { .. }) {
            tracing::warn!(
                "settings: load failed at startup; require_network will deny outbound calls until user resets"
            );
        }

        Ok(Self {
            app_data_dir,
            #[cfg(not(test))]
            storage_lease: std::sync::Mutex::new(Some(storage_lease)),
            #[cfg(not(test))]
            state_database,
            corpus_cache: Arc::new(Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(Mutex::new(())),
            skill_sources_write_lock: Arc::new(Mutex::new(())),
            skill_installs_write_lock: Arc::new(Mutex::new(())),
            skill_folders_write_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(RwLock::new(settings_state)),
            updater_state: crate::commands::updater::empty_state(),
        })
    }

    /// Consult paranoid mode + settings load state. Returns `Ok(())` if
    /// the outbound call is allowed, or `AppError::ParanoidModeBlocked`
    /// otherwise. **Every outbound command must call this as its first
    /// line** — see the security review §12d "Cross-cutting concerns".
    ///
    /// Three cases:
    /// - `Loaded(s)` with `paranoid_mode == false` → allow.
    /// - `FirstLaunch` → allow (defaults apply, paranoid OFF — preserves
    ///   the v0.1.0 behaviour for users with no settings file yet).
    /// - `Loaded(s)` with `paranoid_mode == true` OR `Corrupt(...)` →
    ///   deny. Corrupt is a deliberate fail-closed: we don't know what
    ///   the user wanted, so we don't make outbound calls until they
    ///   repair the file (or hit Reset to defaults in the UI).
    pub async fn require_network(&self, feature: &'static str) -> Result<(), AppError> {
        let guard = self.settings.read().await;
        match &*guard {
            SettingsLoadState::Loaded(s) if !s.paranoid_mode => Ok(()),
            SettingsLoadState::FirstLaunch => Ok(()),
            SettingsLoadState::Loaded(_) | SettingsLoadState::Corrupt { .. } => {
                Err(AppError::ParanoidModeBlocked {
                    feature: feature.to_string(),
                })
            }
        }
    }

    #[cfg(test)]
    pub async fn authorize_mcp(
        &self,
        action: McpAction,
        project_path: Option<&str>,
    ) -> Result<Option<AuthorizedMcpProject>, AppError> {
        self.authorize_mcp_client("unknown", action, project_path)
            .await
    }

    pub async fn authorize_mcp_client(
        &self,
        client: &str,
        action: McpAction,
        project_path: Option<&str>,
    ) -> Result<Option<AuthorizedMcpProject>, AppError> {
        if action == McpAction::Read && project_path.is_none() {
            return Ok(None);
        }
        self.authorize_mcp_client_with_load(client, action, project_path, || {
            settings::load_async(&self.app_data_dir)
        })
        .await
    }

    async fn authorize_mcp_client_with_load<F, Fut>(
        &self,
        client: &str,
        action: McpAction,
        project_path: Option<&str>,
        load: F,
    ) -> Result<Option<AuthorizedMcpProject>, AppError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = SettingsLoadState>,
    {
        let mut guard = self.settings.write().await;
        *guard = load().await;
        let identity = match &*guard {
            SettingsLoadState::Loaded(policy) => {
                authorize_mcp_for_client(policy, client, action, project_path)
            }
            SettingsLoadState::FirstLaunch => authorize_mcp_for_client(
                &settings::Settings::default(),
                client,
                action,
                project_path,
            ),
            SettingsLoadState::Corrupt { .. } => Err(mcp_denied(action)),
        }?;
        drop(guard);
        identity.map(open_authorized_mcp_project).transpose()
    }
}

fn open_authorized_mcp_project(identity: String) -> Result<AuthorizedMcpProject, AppError> {
    let path = Path::new(&identity);
    let root_path = path
        .ancestors()
        .last()
        .ok_or_else(|| AppError::InvalidArgument {
            message: "MCP project path has no filesystem root".into(),
        })?;
    let mut root =
        cap_primitives::fs::open_ambient_dir(root_path, cap_primitives::ambient_authority())
            .map_err(|error| AppError::Io {
                message: format!(
                    "open MCP project filesystem root {}: {error}",
                    root_path.display()
                ),
            })?;
    let relative = path
        .strip_prefix(root_path)
        .map_err(|_| AppError::InvalidArgument {
            message: "MCP project path is not beneath its filesystem root".into(),
        })?;
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(AppError::InvalidArgument {
                message: "MCP project path must be absolute and normalized".into(),
            });
        };
        root = cap_primitives::fs::open_dir_nofollow(&root, Path::new(name)).map_err(|error| {
            AppError::InvalidArgument {
                message: format!(
                    "MCP project identity cannot be opened without following links: {error}"
                ),
            }
        })?;
    }
    Ok(AuthorizedMcpProject {
        identity,
        root: Arc::new(root),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpAction {
    Read,
    Source,
    Install,
    Destructive,
    AgentSource,
    AgentInstall,
    AgentDestructive,
}

impl McpAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Source => "source",
            Self::Install => "install",
            Self::Destructive => "destructive",
            Self::AgentSource => "agent_source",
            Self::AgentInstall => "agent_install",
            Self::AgentDestructive => "agent_destructive",
        }
    }
}

#[cfg(test)]
pub fn authorize_mcp(
    policy: &settings::Settings,
    action: McpAction,
    project_path: Option<&str>,
) -> Result<Option<String>, AppError> {
    authorize_mcp_for_client(policy, "unknown", action, project_path)
}

pub fn authorize_mcp_for_client(
    policy: &settings::Settings,
    client: &str,
    action: McpAction,
    project_path: Option<&str>,
) -> Result<Option<String>, AppError> {
    if action != McpAction::Read && policy.paranoid_mode {
        return Err(AppError::ParanoidModeBlocked {
            feature: format!("mcp_{}", action.as_str()),
        });
    }
    let client_policy = policy.mcp_client_policies.get(client);
    let enabled = match action {
        McpAction::Read => true,
        McpAction::Source => client_policy
            .map(|value| value.source_access)
            .unwrap_or(policy.mcp_source_access),
        McpAction::Install => client_policy
            .map(|value| value.install_access)
            .unwrap_or(policy.mcp_install_access),
        McpAction::Destructive => client_policy
            .map(|value| value.destructive_access)
            .unwrap_or(policy.mcp_destructive_access),
        McpAction::AgentSource => client_policy
            .map(|value| value.agent_source_access)
            .unwrap_or(policy.mcp_agent_source_access),
        McpAction::AgentInstall => client_policy
            .map(|value| value.agent_install_access)
            .unwrap_or(policy.mcp_agent_install_access),
        McpAction::AgentDestructive => client_policy
            .map(|value| value.agent_destructive_access)
            .unwrap_or(policy.mcp_agent_destructive_access),
    };
    if !enabled {
        return Err(mcp_denied(action));
    }
    let Some(project_path) = project_path else {
        return Ok(None);
    };
    let supplied_project = PathBuf::from(project_path);
    let project_path = settings::canonical_mcp_project_path(project_path)?;
    if supplied_project != project_path {
        return Err(AppError::InvalidArgument {
            message: "MCP project path must match its exact canonical identity".into(),
        });
    }
    let allowed = policy.mcp_project_allowlist.iter().find_map(|allowed| {
        let identity = PathBuf::from(allowed);
        settings::canonical_mcp_project_path(allowed)
            .ok()
            .filter(|resolved| resolved == &identity && resolved == &project_path)
    });
    allowed
        .map(|path| Some(path.to_string_lossy().into_owned()))
        .ok_or_else(|| AppError::InvalidArgument {
            message: "MCP project path is not allowlisted".into(),
        })
}

fn mcp_denied(action: McpAction) -> AppError {
    AppError::InvalidArgument {
        message: format!("MCP {} access is disabled", action.as_str()),
    }
}

pub async fn append_mcp_audit(
    app_data_dir: &Path,
    mut entry: McpAuditEntry,
) -> Result<(), AppError> {
    if let Some(database) = crate::state_db::StateDatabase::completed(app_data_dir).await? {
        sanitize_mcp_audit(&mut entry);
        return database
            .mutate_quiet(mcp_audit_spec(), Vec::new(), move |entries| {
                entries.insert(0, entry);
                entries.truncate(MCP_AUDIT_MAX_ENTRIES);
                Ok(())
            })
            .await;
    }
    let app_data_dir = app_data_dir.to_path_buf();
    tokio::task::spawn_blocking(move || append_mcp_audit_blocking(&app_data_dir, entry))
        .await
        .map_err(|error| AppError::Internal {
            message: format!("MCP audit task failed: {error}"),
        })?
}

pub async fn load_mcp_audit(app_data_dir: &Path) -> Result<Vec<McpAuditEntry>, AppError> {
    if let Some(database) = crate::state_db::StateDatabase::completed(app_data_dir).await? {
        return database
            .read(mcp_audit_spec())
            .await?
            .ok_or_else(|| AppError::StorageCorrupt {
                message: "MCP audit is missing after SQLite migration".into(),
            });
    }
    let app_data_dir = app_data_dir.to_path_buf();
    tokio::task::spawn_blocking(move || load_mcp_audit_blocking(&app_data_dir))
        .await
        .map_err(|error| AppError::Internal {
            message: format!("MCP audit task failed: {error}"),
        })?
}

fn validate_mcp_audit(entries: &[McpAuditEntry]) -> Result<(), AppError> {
    if entries.len() > MCP_AUDIT_MAX_ENTRIES
        || entries.iter().any(|entry| {
            let mut sanitized = entry.clone();
            sanitize_mcp_audit(&mut sanitized);
            sanitized != *entry
        })
    {
        return Err(AppError::InvalidArgument {
            message: "MCP audit contains unbounded or unredacted entries".into(),
        });
    }
    Ok(())
}

fn mcp_audit_spec() -> crate::state_db::DocumentSpec<Vec<McpAuditEntry>> {
    crate::state_db::DocumentSpec::new("mcp_audit", 1, MCP_AUDIT_MAX_BYTES as u64, |entries| {
        validate_mcp_audit(entries)
    })
}

fn parse_mcp_audit_import(raw: &[u8]) -> Result<String, AppError> {
    let mut entries = raw
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            serde_json::from_slice::<McpAuditEntry>(line).map_err(|_| AppError::StorageCorrupt {
                message: "MCP audit legacy state is malformed".into(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    entries.reverse();
    entries.truncate(MCP_AUDIT_MAX_ENTRIES);
    validate_mcp_audit(&entries).map_err(|_| AppError::StorageCorrupt {
        message: "MCP audit legacy state is invalid".into(),
    })?;
    serde_json::to_string(&entries).map_err(|_| AppError::Internal {
        message: "serialize MCP audit migration state".into(),
    })
}

fn migration_import_specs() -> Vec<crate::state_db::ImportSpec> {
    vec![
        crate::commands::settings::settings_import_spec(),
        crate::corpus::catalog_source_import_spec(),
        crate::corpus::control_center_import_spec(),
        crate::skills::skill_sources_import_spec(),
        crate::skills::skill_trust_import_spec(),
        crate::skills::drafts::import_spec(),
        crate::skills::organize::import_spec(),
        crate::skills::install::import_spec(),
        crate::agents::agent_sources_import_spec(),
        crate::agents::drafts::import_spec(),
        crate::agents::organize::import_spec(),
        crate::install::installs_import_spec(),
        crate::ollama::import_spec(),
        crate::install::projects_import_spec(),
        crate::experts::experts_import_spec(),
        crate::experts::activation_requests_import_spec(),
        crate::experts::activations_import_spec(),
        crate::expert_runs::import_spec(),
        crate::state_db::ImportSpec::new("mcp_audit", "", parse_mcp_audit_import),
    ]
}

#[tauri::command]
pub async fn mcp_audit_list(state: State<'_, AppState>) -> Result<Vec<McpAuditEntry>, AppError> {
    crate::skills::mcp::reconcile_factory_terminal_audits(&state).await?;
    load_mcp_audit(&state.app_data_dir).await
}

fn mcp_audit_paths(app_data_dir: &Path) -> (PathBuf, PathBuf) {
    let directory = app_data_dir.join("state");
    (
        directory.join("mcp-audit.jsonl"),
        directory.join("mcp-audit.lock"),
    )
}

struct McpAuditLock {
    _process: tokio::sync::OwnedMutexGuard<()>,
    _file: File,
}

fn lock_mcp_audit(app_data_dir: &Path) -> Result<McpAuditLock, AppError> {
    let (path, lock_path) = mcp_audit_paths(app_data_dir);
    let directory = path.parent().expect("audit path has parent");
    std::fs::create_dir_all(directory).map_err(|error| AppError::Io {
        message: format!(
            "create MCP audit directory {}: {error}",
            directory.display()
        ),
    })?;
    let process_lock = {
        let mut locks = MCP_AUDIT_PROCESS_LOCKS
            .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
            .lock()
            .map_err(|_| AppError::Internal {
                message: "MCP audit process lock registry is poisoned".into(),
            })?;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(&lock_path).and_then(std::sync::Weak::upgrade) {
            lock
        } else {
            let lock = Arc::new(tokio::sync::Mutex::new(()));
            locks.insert(lock_path.clone(), Arc::downgrade(&lock));
            lock
        }
    };
    let process_deadline = Instant::now() + MCP_AUDIT_PROCESS_LOCK_DEADLINE;
    let process = loop {
        match Arc::clone(&process_lock).try_lock_owned() {
            Ok(guard) => break guard,
            Err(_) if Instant::now() < process_deadline => {
                std::thread::sleep(MCP_AUDIT_LOCK_RETRY);
            }
            Err(_) => {
                return Err(AppError::Io {
                    message: "MCP audit process lock deadline exceeded".into(),
                })
            }
        }
    };
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| AppError::Io {
            message: format!("open MCP audit lock {}: {error}", lock_path.display()),
        })?;
    let os_deadline = Instant::now() + MCP_AUDIT_OS_LOCK_DEADLINE;
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) if Instant::now() < os_deadline => {
                std::thread::sleep(MCP_AUDIT_LOCK_RETRY);
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(AppError::Io {
                    message: format!(
                        "MCP audit lock deadline exceeded for {}",
                        lock_path.display()
                    ),
                })
            }
            Err(std::fs::TryLockError::Error(error)) => {
                return Err(AppError::Io {
                    message: format!("lock MCP audit {}: {error}", lock_path.display()),
                })
            }
        }
    }
    Ok(McpAuditLock {
        _process: process,
        _file: lock,
    })
}

fn append_mcp_audit_blocking(
    app_data_dir: &Path,
    mut entry: McpAuditEntry,
) -> Result<(), AppError> {
    let _lock = lock_mcp_audit(app_data_dir)?;
    #[cfg(test)]
    if should_inject_mcp_audit_failure(app_data_dir) {
        return Err(AppError::Io {
            message: "injected MCP audit append failure".into(),
        });
    }
    sanitize_mcp_audit(&mut entry);
    let (path, _) = mcp_audit_paths(app_data_dir);
    if path.exists() {
        compact_mcp_audit(&path)?;
    }
    let mut line = serde_json::to_vec(&entry).map_err(|error| AppError::Internal {
        message: format!("serialize MCP audit entry: {error}"),
    })?;
    line.push(b'\n');
    if audit_needs_separator(&path)? {
        line.insert(0, b'\n');
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| AppError::Io {
            message: format!("open MCP audit {}: {error}", path.display()),
        })?;
    file.write_all(&line).map_err(|error| AppError::Io {
        message: format!("append MCP audit {}: {error}", path.display()),
    })?;
    file.sync_data().map_err(|error| AppError::Io {
        message: format!("sync MCP audit {}: {error}", path.display()),
    })?;
    compact_mcp_audit(&path)
}

fn audit_needs_separator(path: &Path) -> Result<bool, AppError> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(AppError::Io {
                message: format!("open MCP audit {}: {error}", path.display()),
            })
        }
    };
    if file
        .metadata()
        .map_err(|error| AppError::Io {
            message: format!("inspect MCP audit {}: {error}", path.display()),
        })?
        .len()
        == 0
    {
        return Ok(false);
    }
    file.seek(SeekFrom::End(-1)).map_err(|error| AppError::Io {
        message: format!("seek MCP audit {}: {error}", path.display()),
    })?;
    let mut last = [0];
    file.read_exact(&mut last).map_err(|error| AppError::Io {
        message: format!("read MCP audit {}: {error}", path.display()),
    })?;
    Ok(last[0] != b'\n')
}

fn load_mcp_audit_blocking(app_data_dir: &Path) -> Result<Vec<McpAuditEntry>, AppError> {
    let _lock = lock_mcp_audit(app_data_dir)?;
    let (path, _) = mcp_audit_paths(app_data_dir);
    let bytes = read_mcp_audit_tail(&path)?;
    let mut entries = bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_slice::<McpAuditEntry>(line).ok())
        .collect::<Vec<_>>();
    entries.reverse();
    entries.truncate(MCP_AUDIT_MAX_ENTRIES);
    Ok(entries)
}

fn compact_mcp_audit(path: &Path) -> Result<(), AppError> {
    let bytes = read_mcp_audit_tail(path)?;
    let lines = bytes
        .split_inclusive(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let metadata = std::fs::metadata(path).map_err(|error| AppError::Io {
        message: format!("inspect MCP audit {}: {error}", path.display()),
    })?;
    if metadata.len() <= MCP_AUDIT_MAX_BYTES as u64 && lines.len() <= MCP_AUDIT_MAX_ENTRIES {
        return Ok(());
    }
    let start = lines.len().saturating_sub(MCP_AUDIT_MAX_ENTRIES);
    let retained = lines[start..].concat();
    atomic_replace_mcp_audit(path, &retained)
}

fn read_mcp_audit_tail(path: &Path) -> Result<Vec<u8>, AppError> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(AppError::Io {
                message: format!("open MCP audit {}: {error}", path.display()),
            })
        }
    };
    let length = file
        .metadata()
        .map_err(|error| AppError::Io {
            message: format!("inspect MCP audit {}: {error}", path.display()),
        })?
        .len();
    let start = length.saturating_sub(MCP_AUDIT_MAX_BYTES as u64);
    file.seek(SeekFrom::Start(start))
        .map_err(|error| AppError::Io {
            message: format!("seek MCP audit {}: {error}", path.display()),
        })?;
    let mut bytes = Vec::with_capacity((length - start) as usize);
    file.take(MCP_AUDIT_MAX_BYTES as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| AppError::Io {
            message: format!("read MCP audit {}: {error}", path.display()),
        })?;
    if start > 0 {
        bytes = bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|newline| bytes[(newline + 1)..].to_vec())
            .unwrap_or_default();
    }
    Ok(bytes)
}

fn atomic_replace_mcp_audit(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let temporary = path.with_extension(format!("jsonl.tmp-{}", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| AppError::Io {
            message: format!(
                "create MCP audit temporary {}: {error}",
                temporary.display()
            ),
        })?;
    file.write_all(bytes).map_err(|error| AppError::Io {
        message: format!("write MCP audit temporary {}: {error}", temporary.display()),
    })?;
    file.sync_all().map_err(|error| AppError::Io {
        message: format!("sync MCP audit temporary {}: {error}", temporary.display()),
    })?;
    replace_mcp_audit_file(&temporary, path).map_err(|error| AppError::Io {
        message: format!("replace MCP audit {}: {error}", path.display()),
    })?;
    if let Some(parent) = path.parent() {
        if let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }
    }
    Ok(())
}

fn replace_mcp_audit_file(temporary: &Path, path: &Path) -> std::io::Result<()> {
    replace_mcp_audit_file_with(temporary, path, platform_replace_mcp_audit_file)
}

fn replace_mcp_audit_file_with<R>(
    temporary: &Path,
    path: &Path,
    mut replace: R,
) -> std::io::Result<()>
where
    R: FnMut(&Path, &Path) -> std::io::Result<()>,
{
    replace(temporary, path)
}

#[cfg(not(windows))]
fn platform_replace_mcp_audit_file(temporary: &Path, path: &Path) -> std::io::Result<()> {
    std::fs::rename(temporary, path)
}

#[cfg(windows)]
fn platform_replace_mcp_audit_file(temporary: &Path, path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    #[link(name = "Kernel32")]
    unsafe extern "system" {
        #[link_name = "MoveFileExW"]
        fn move_file_ex_w(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }

    let temporary = temporary
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let path = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let replaced = unsafe {
        move_file_ex_w(
            temporary.as_ptr(),
            path.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn sanitize_mcp_audit(entry: &mut McpAuditEntry) {
    entry.id = capped_audit_field(&entry.id, 64);
    entry.timestamp = capped_audit_field(&entry.timestamp, MCP_AUDIT_FIELD_CHARS_CAP);
    let known_factory_tool = matches!(
        entry.tool.as_str(),
        "factory_runs_claim_phase"
            | "factory_runs_complete_phase"
            | "factory_runs_discover_work"
            | "factory_runs_get_claim_contract"
            | "factory_runs_submit_artifact"
            | "factory_runs_submit_blocker"
            | "factory_runs_submit_evidence"
    );
    entry.tool = if (entry.tool.starts_with("skills_") || known_factory_tool)
        && entry
            .tool
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        capped_audit_field(&entry.tool, MCP_AUDIT_FIELD_CHARS_CAP)
    } else {
        "[redacted]".into()
    };
    entry.action = match entry.action.as_str() {
        "read" | "source" | "install" | "destructive" => entry.action.clone(),
        _ => "unknown".into(),
    };
    entry.phase = match entry.phase.as_str() {
        "attempt" | "terminal" => entry.phase.clone(),
        _ => "terminal".into(),
    };
    entry.project_path = entry.project_path.as_deref().map(|path| {
        if contains_secret_marker(path) {
            "[redacted]".into()
        } else {
            capped_audit_field(path, MCP_AUDIT_PROJECT_CHARS_CAP)
        }
    });
}

#[cfg(test)]
pub(crate) fn inject_mcp_audit_failure_after(app_data_dir: &Path, successful_appends: usize) {
    MCP_AUDIT_FAILURES
        .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
        .lock()
        .expect("MCP audit failure injection lock")
        .insert(app_data_dir.to_path_buf(), successful_appends);
}

#[cfg(test)]
fn should_inject_mcp_audit_failure(app_data_dir: &Path) -> bool {
    let mut failures = MCP_AUDIT_FAILURES
        .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
        .lock()
        .expect("MCP audit failure injection lock");
    match failures.get_mut(app_data_dir) {
        Some(remaining) if *remaining == 0 => {
            failures.remove(app_data_dir);
            true
        }
        Some(remaining) => {
            *remaining -= 1;
            false
        }
        None => false,
    }
}

fn capped_audit_field(value: &str, cap: usize) -> String {
    value.chars().take(cap).collect()
}

fn contains_secret_marker(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "authorization",
        "bearer",
        "password",
        "api_key",
        "apikey",
        "token=",
        "token:",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

/// Resolve the canonical app-data root:
/// `~/Library/Application Support/com.zerologic.agency-agents-app/`. The corpus, install
/// ledger, github cache, and settings file all derive their paths from
/// this; the security gates that check "is this path inside our app data
/// dir?" anchor on it too.
fn resolve_app_data_dir() -> Result<PathBuf, AppError> {
    let mut base = dirs::data_dir().ok_or_else(|| AppError::Internal {
        message: "could not resolve OS data dir".into(),
    })?;
    base.push("com.zerologic.agency-agents-app");
    Ok(base)
}

/// Tauri setup hook — instantiates and manages `AppState`.
pub fn initialize<R: tauri::Runtime>(
    app: &mut tauri::App<R>,
) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::Manager;
    let state = AppState::build()?;
    app.manage(state);
    Ok(())
}

pub(crate) async fn recover_filesystem_operations(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<(), AppError> {
    {
        let _catalog = state.corpus_refresh_in_flight.lock().await;
        crate::corpus::recover_catalog_activation_at_startup(&state.app_data_dir)?;
    }
    if state.completed_state_database().await?.is_none() {
        return Ok(());
    }
    let mut failures = Vec::new();
    for result in [
        crate::skills::drafts::recover_publish_operations(state).await,
        crate::agents::drafts::recover_publish_operations(state).await,
        crate::skills::recover_install_operations(state).await,
        crate::install::recover_agent_operations(state).await,
        crate::install::recover_workspace_pack_operations(app, state).await,
        crate::install::recover_project_instruction_operations(state).await,
        crate::experts::recover_activation_operations(app, state).await,
    ] {
        if let Err(error) = result {
            failures.push(error.to_string());
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(AppError::StorageCorrupt {
            message: format!(
                "filesystem recovery requires attention: {}",
                failures.join("; ")
            ),
        })
    }
}

pub(crate) async fn migration_status(
    state: &AppState,
) -> Result<crate::types::StorageMigrationStatus, AppError> {
    let Some(database) = state.state_database_if_present() else {
        return Ok(crate::types::StorageMigrationStatus {
            state: crate::types::StorageMigrationState::Legacy,
            stage: Some("checkingData".into()),
            detail: Some("Ready for the one-time data update.".into()),
            legacy_conflicts: Vec::new(),
        });
    };
    let migration_state = database.migration_state().await?;
    let (stage, detail, legacy_conflicts) = match migration_state {
        crate::types::StorageMigrationState::Legacy => (
            Some("checkingData".into()),
            Some("Ready for the one-time data update.".into()),
            Vec::new(),
        ),
        crate::types::StorageMigrationState::InProgress => (
            Some("movingRecords".into()),
            Some("The previous update was interrupted and can be retried safely.".into()),
            Vec::new(),
        ),
        crate::types::StorageMigrationState::Complete => {
            crate::corpus::register_control_center_document(&database).await?;
            let pending = database.pending_filesystem_operations().await?;
            let recovery_errors = pending
                .iter()
                .filter_map(|operation| operation.recovery_error.as_deref())
                .collect::<Vec<_>>();
            let conflicts = database
                .legacy_conflicts(&state.app_data_dir, PERSISTENCE_INVENTORY)
                .await?;
            if pending.is_empty() {
                (
                    Some("complete".into()),
                    Some(
                        "Data update complete. Reopen connected Claude and Codex sessions.".into(),
                    ),
                    conflicts,
                )
            } else {
                (
                    Some("recovery".into()),
                    Some(if recovery_errors.is_empty() {
                        "Finishing an interrupted package operation.".into()
                    } else {
                        recovery_errors.join("; ")
                    }),
                    conflicts,
                )
            }
        }
        crate::types::StorageMigrationState::Corrupt => (
            Some("failed".into()),
            Some("Nothing was lost. Fix the reported data issue, then retry.".into()),
            Vec::new(),
        ),
        crate::types::StorageMigrationState::Unsupported => (
            Some("unsupported".into()),
            Some("This data was created by a newer Agency Agents version.".into()),
            Vec::new(),
        ),
    };
    Ok(crate::types::StorageMigrationStatus {
        state: migration_state,
        stage,
        detail,
        legacy_conflicts,
    })
}

#[tauri::command]
pub async fn storage_migration_status(
    state: State<'_, AppState>,
) -> Result<crate::types::StorageMigrationStatus, AppError> {
    migration_status(&state).await
}

async fn run_storage_migration(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<crate::types::StorageMigrationStatus, AppError> {
    let database = crate::state_db::StateDatabase::open(&state.app_data_dir)?;
    state.release_storage_lease()?;
    let result = database
        .import_legacy(
            &state.app_data_dir,
            PERSISTENCE_INVENTORY,
            &migration_import_specs(),
        )
        .await;
    let lease = state.reacquire_storage_lease();
    let outcome = match (result, lease) {
        (Ok(outcome), Ok(())) => outcome,
        (Err(error), Ok(())) | (Ok(_), Err(error)) => return Err(error),
        (Err(import_error), Err(lease_error)) => {
            return Err(AppError::Internal {
                message: format!(
                    "storage migration failed ({import_error}); storage lease recovery also failed ({lease_error})"
                ),
            });
        }
    };
    recover_filesystem_operations(app, state).await?;
    let mut status = migration_status(state).await?;
    status.detail = Some(format!(
        "Data update complete. Verified backup: {}. Reopen connected Claude and Codex sessions.",
        outcome.backup_dir.display()
    ));
    Ok(status)
}

#[tauri::command]
pub async fn storage_migration_start(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<crate::types::StorageMigrationStatus, AppError> {
    run_storage_migration(&app, &state).await
}

#[tauri::command]
pub async fn storage_migration_retry(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<crate::types::StorageMigrationStatus, AppError> {
    run_storage_migration(&app, &state).await
}

#[tauri::command]
pub async fn storage_visible_revision(state: State<'_, AppState>) -> Result<u64, AppError> {
    let database = crate::state_db::StateDatabase::open(&state.app_data_dir)?;
    if database.migration_state().await? == crate::types::StorageMigrationState::Complete {
        database.visible_revision().await
    } else {
        Ok(0)
    }
}

#[tauri::command]
pub async fn storage_backup(state: State<'_, AppState>) -> Result<String, AppError> {
    let database =
        state
            .completed_state_database()
            .await?
            .ok_or_else(|| AppError::InvalidArgument {
                message: "SQLite migration must complete before creating a live backup".into(),
            })?;
    let destination = state.app_data_dir.join("state/backups").join(format!(
        "agency-agents-{}.sqlite3",
        chrono::Utc::now().format("%Y%m%dT%H%M%S%.fZ")
    ));
    database.backup_to(&destination).await?;
    Ok(destination.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn storage_open_data_directory(state: State<'_, AppState>) -> Result<(), AppError> {
    crate::install::reveal_path_for_state(&state, state.app_data_dir.to_string_lossy().into_owned())
        .await
}

#[tauri::command]
pub async fn storage_legacy_conflicts_dismiss(state: State<'_, AppState>) -> Result<(), AppError> {
    let database =
        state
            .completed_state_database()
            .await?
            .ok_or_else(|| AppError::InvalidArgument {
                message: "SQLite migration must complete before dismissing legacy conflicts".into(),
            })?;
    database
        .dismiss_legacy_conflicts(&state.app_data_dir, PERSISTENCE_INVENTORY)
        .await
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::settings::Settings;
    use crate::types::McpAuditEntry;

    #[tokio::test]
    async fn every_persistence_document_has_a_working_import_validator() {
        let root = tempfile::tempdir().unwrap();
        let database = crate::state_db::StateDatabase::open(root.path()).unwrap();

        database
            .import_legacy(
                root.path(),
                PERSISTENCE_INVENTORY,
                &migration_import_specs(),
            )
            .await
            .unwrap();

        assert_eq!(
            database.migration_state().await.unwrap(),
            crate::types::StorageMigrationState::Complete
        );
        let connection =
            rusqlite::Connection::open(root.path().join("state/agency-agents.sqlite3")).unwrap();
        let count: i64 = connection
            .query_row("SELECT count(*) FROM state_documents", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, PERSISTENCE_INVENTORY.len() as i64);
    }

    #[tokio::test]
    async fn completed_installation_backfills_control_center_before_status_checks() {
        let root = tempfile::tempdir().unwrap();
        let database = crate::state_db::StateDatabase::open(root.path()).unwrap();
        let mut previous_inventory = PERSISTENCE_INVENTORY.to_vec();
        let removed = previous_inventory.remove(2);
        assert_eq!(removed.name, "control_center");
        let mut previous_specs = migration_import_specs();
        previous_specs.remove(2);
        database
            .import_legacy(root.path(), &previous_inventory, &previous_specs)
            .await
            .unwrap();
        let state = AppState {
            app_data_dir: root.path().to_path_buf(),
            corpus_cache: Arc::new(Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(Mutex::new(())),
            skill_sources_write_lock: Arc::new(Mutex::new(())),
            skill_installs_write_lock: Arc::new(Mutex::new(())),
            skill_folders_write_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(RwLock::new(SettingsLoadState::FirstLaunch)),
            updater_state: crate::commands::updater::empty_state(),
        };

        let status = migration_status(&state).await.unwrap();

        assert_eq!(status.state, crate::types::StorageMigrationState::Complete);
        assert!(status.legacy_conflicts.is_empty());
        let connection =
            rusqlite::Connection::open(root.path().join("state/agency-agents.sqlite3")).unwrap();
        for table in ["state_documents", "legacy_imports"] {
            let query = format!("SELECT count(*) FROM {table} WHERE name = 'control_center'");
            assert_eq!(
                connection
                    .query_row(&query, [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                1
            );
        }
    }

    #[tokio::test]
    #[ignore = "manual rehearsal against an explicitly copied app-data directory"]
    async fn copied_real_state_migrates_with_semantic_equality() {
        let root = PathBuf::from(
            std::env::var("AGENCY_AGENTS_REHEARSAL_DIR")
                .expect("set AGENCY_AGENTS_REHEARSAL_DIR to a copied app-data directory"),
        );
        assert!(root.join(".sqlite-rehearsal-copy").is_file());
        let database = crate::state_db::StateDatabase::open(&root).unwrap();
        database
            .import_legacy(&root, PERSISTENCE_INVENTORY, &migration_import_specs())
            .await
            .unwrap();

        let connection =
            rusqlite::Connection::open(root.join("state/agency-agents.sqlite3")).unwrap();
        for document in PERSISTENCE_INVENTORY {
            let payload: String = connection
                .query_row(
                    "SELECT payload FROM state_documents WHERE name = ?1",
                    [document.name],
                    |row| row.get(0),
                )
                .unwrap();
            let source = root.join(document.relative_path);
            if !source.is_file() {
                continue;
            }
            let expected = if document.parser == "jsonl" {
                serde_json::to_value(load_mcp_audit_blocking(&root).unwrap()).unwrap()
            } else {
                serde_json::from_slice(&std::fs::read(source).unwrap()).unwrap()
            };
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&payload).unwrap(),
                expected
            );
        }

        let restored = root.join("rehearsal-restore/state/agency-agents.sqlite3");
        database.backup_to(&restored).await.unwrap();
        let restored = rusqlite::Connection::open(restored).unwrap();
        let integrity: String = restored
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        let documents: i64 = restored
            .query_row("SELECT count(*) FROM state_documents", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
        assert_eq!(documents, PERSISTENCE_INVENTORY.len() as i64);
    }

    /// Build a minimal AppState whose only meaningful field is `settings`.
    /// All other fields use whatever `AppState::build` resolves — for the
    /// gate-only tests below the app-data path lookup, catalog load, etc., are
    /// irrelevant. Settings slot is overwritten *after* construction so we
    /// don't depend on whatever happens to be on disk for the test user.
    async fn build_state_with(slot: SettingsLoadState) -> AppState {
        let state = AppState::build().expect("AppState::build");
        {
            let mut guard = state.settings.write().await;
            *guard = slot;
        }
        state
    }

    #[test]
    fn persistence_inventory_covers_every_live_legacy_document() {
        let actual = PERSISTENCE_INVENTORY
            .iter()
            .map(|document| {
                format!(
                    "{}|{}|{}|{}|{}|{}",
                    document.name,
                    document.relative_path,
                    document.version,
                    document.max_bytes,
                    document.parser,
                    document.validator
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(
            actual,
            "settings|settings.json|1|1048576|json|settings\n\
catalog|state/catalog.json|1|65536|json|catalog_source\n\
control_center|state/control-center.json|1|4194304|json|control_center\n\
skill_sources|state/skill-sources.json|1|1048576|json|skill_sources\n\
skill_trust|state/skill-trust.json|1|1048576|json|skill_trust\n\
skill_drafts|state/skill-drafts.json|1|16777216|json|skill_drafts\n\
skill_library|state/skill-folders.json|1|4194304|json|skill_library\n\
skill_installs|state/skill-installs.json|1|16777216|json|skill_installs\n\
agent_sources|state/agent-sources.json|1|1048576|json|agent_sources\n\
agent_drafts|state/agent-drafts.json|1|8388608|json|agent_drafts\n\
agent_library|state/agent-library.json|1|1048576|json|agent_library\n\
installs|state/installs.json|1|16777216|json|installs\n\
ollama_deployments|state/ollama-deployments.json|1|4194304|json|ollama_deployments\n\
projects|state/projects.json|1|65536|json|projects\n\
experts|state/experts.json|1|4194304|json|experts\n\
expert_activation_requests|state/expert-activation-requests.json|1|524288|json|expert_activation_requests\n\
expert_activations|state/expert-activations.json|1|524288|json|expert_activations\n\
expert_runs|state/expert-runs.json|1|4194304|json|expert_runs\n\
mcp_audit|state/mcp-audit.jsonl|1|1048576|jsonl|mcp_audit"
        );

        let mut names = PERSISTENCE_INVENTORY
            .iter()
            .map(|document| document.name)
            .collect::<Vec<_>>();
        names.sort_unstable();
        names.dedup();
        assert_eq!(
            names.len(),
            PERSISTENCE_INVENTORY.len(),
            "document names must be unique"
        );

        let mut paths = PERSISTENCE_INVENTORY
            .iter()
            .map(|document| document.relative_path)
            .collect::<Vec<_>>();
        paths.sort_unstable();
        paths.dedup();
        assert_eq!(
            paths.len(),
            PERSISTENCE_INVENTORY.len(),
            "legacy paths must be unique"
        );
    }

    #[tokio::test]
    async fn require_network_allows_first_launch() {
        let state = build_state_with(SettingsLoadState::FirstLaunch).await;
        assert!(state.require_network("trending_fetch").await.is_ok());
    }

    #[tokio::test]
    async fn require_network_allows_loaded_with_paranoid_off() {
        let s = Settings {
            paranoid_mode: false,
            ..Settings::default()
        };
        let state = build_state_with(SettingsLoadState::Loaded(s)).await;
        assert!(state.require_network("catalog_refresh").await.is_ok());
    }

    #[tokio::test]
    async fn require_network_blocks_when_paranoid_on() {
        let s = Settings {
            paranoid_mode: true,
            ..Settings::default()
        };
        let state = build_state_with(SettingsLoadState::Loaded(s)).await;
        let r = state.require_network("trending_fetch").await;
        match r {
            Err(AppError::ParanoidModeBlocked { feature }) => {
                assert_eq!(feature, "trending_fetch");
            }
            other => panic!("expected ParanoidModeBlocked, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn require_network_blocks_when_corrupt() {
        // Fail-closed: corrupt settings file → deny even though paranoid
        // would default false. This is the load-bearing security gate from
        // the §12d review.
        let state = build_state_with(SettingsLoadState::Corrupt {
            message: "bad json".into(),
        })
        .await;
        let r = state.require_network("cask_icon_from_homepage").await;
        match r {
            Err(AppError::ParanoidModeBlocked { feature }) => {
                assert_eq!(feature, "cask_icon_from_homepage");
            }
            other => panic!("expected ParanoidModeBlocked from corrupt, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn require_network_feature_string_round_trips() {
        // The static-str argument must be carried verbatim into the error
        // so the frontend can route the toast to the right setting.
        let state = build_state_with(SettingsLoadState::Corrupt {
            message: "x".into(),
        })
        .await;
        for feat in [
            "trending_fetch",
            "cask_icon_from_homepage",
            "catalog_refresh",
        ] {
            let r = state.require_network(feat).await;
            match r {
                Err(AppError::ParanoidModeBlocked { feature }) => {
                    assert_eq!(feature, feat);
                }
                other => panic!("expected block for {feat}, got {other:?}"),
            }
        }
    }

    #[test]
    fn mcp_read_is_available_but_every_mutation_class_is_denied_by_default() {
        let policy = Settings::default();

        assert!(authorize_mcp(&policy, McpAction::Read, None).is_ok());
        for action in [
            McpAction::Source,
            McpAction::Install,
            McpAction::Destructive,
            McpAction::AgentSource,
            McpAction::AgentInstall,
            McpAction::AgentDestructive,
        ] {
            assert!(
                authorize_mcp(&policy, action, None).is_err(),
                "{action:?} unexpectedly allowed"
            );
        }
    }

    #[test]
    fn skill_mcp_grants_never_imply_agent_mcp_grants() {
        let policy = Settings {
            mcp_source_access: true,
            mcp_install_access: true,
            mcp_destructive_access: true,
            ..Settings::default()
        };

        for action in [
            McpAction::AgentSource,
            McpAction::AgentInstall,
            McpAction::AgentDestructive,
        ] {
            assert!(authorize_mcp(&policy, action, None).is_err());
        }
    }

    #[test]
    fn mcp_client_policy_overrides_global_policy_without_affecting_other_clients() {
        let mut policy = Settings {
            mcp_source_access: true,
            ..Settings::default()
        };
        policy.mcp_client_policies.insert(
            "claude".into(),
            crate::commands::settings::McpClientPolicy {
                source_access: false,
                install_access: true,
                destructive_access: false,
                ..Default::default()
            },
        );

        assert!(authorize_mcp_for_client(&policy, "claude", McpAction::Source, None).is_err());
        assert!(authorize_mcp_for_client(&policy, "claude", McpAction::Install, None).is_ok());
        assert!(authorize_mcp_for_client(&policy, "codex", McpAction::Source, None).is_ok());

        policy
            .mcp_client_policies
            .get_mut("claude")
            .expect("claude policy")
            .agent_source_access = true;
        assert!(authorize_mcp_for_client(&policy, "claude", McpAction::AgentSource, None).is_ok());
        assert!(authorize_mcp_for_client(&policy, "codex", McpAction::AgentSource, None).is_err());
        assert!(authorize_mcp_for_client(&policy, "claude", McpAction::Source, None).is_err());
    }

    #[test]
    fn mcp_project_mutations_require_an_exact_allowlisted_canonical_path() {
        let allowed = tempfile::tempdir().expect("allowed project");
        let denied = tempfile::tempdir().expect("denied project");
        let allowed = std::fs::canonicalize(allowed.path())
            .expect("canonical allowed project")
            .to_string_lossy()
            .into_owned();
        let denied = std::fs::canonicalize(denied.path())
            .expect("canonical denied project")
            .to_string_lossy()
            .into_owned();
        let policy = Settings {
            mcp_install_access: true,
            mcp_agent_install_access: true,
            mcp_project_allowlist: vec![allowed.clone()],
            ..Settings::default()
        };

        assert_eq!(
            authorize_mcp(&policy, McpAction::Install, Some(&allowed))
                .expect("allowed canonical project"),
            Some(allowed.clone())
        );
        assert!(authorize_mcp(&policy, McpAction::Install, Some(&denied)).is_err());
        assert_eq!(
            authorize_mcp(&policy, McpAction::AgentInstall, Some(&allowed))
                .expect("Agent mutation uses the shared exact allowlist"),
            Some(allowed.clone())
        );
        assert!(authorize_mcp(&policy, McpAction::AgentInstall, Some(&denied)).is_err());
        assert!(authorize_mcp(
            &policy,
            McpAction::Install,
            Some(&format!("{allowed}/nested"))
        )
        .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn mcp_project_authorization_rejects_a_symlink_alias_of_an_allowlisted_project() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("root");
        let project = root.path().join("project");
        let alias = root.path().join("alias");
        std::fs::create_dir(&project).expect("project");
        symlink(&project, &alias).expect("alias");
        let canonical = std::fs::canonicalize(&project)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let policy = Settings {
            mcp_install_access: true,
            mcp_project_allowlist: vec![canonical],
            ..Settings::default()
        };

        assert!(authorize_mcp(&policy, McpAction::Install, Some(alias.to_str().unwrap())).is_err());
    }

    #[tokio::test]
    async fn mcp_reads_with_project_paths_require_the_exact_canonical_allowlist_identity() {
        let app = tempfile::tempdir().expect("app data");
        let allowed = tempfile::tempdir().expect("allowed project");
        let denied = tempfile::tempdir().expect("denied project");
        let allowed = std::fs::canonicalize(allowed.path())
            .expect("canonical allowed project")
            .to_string_lossy()
            .into_owned();
        let denied = std::fs::canonicalize(denied.path())
            .expect("canonical denied project")
            .to_string_lossy()
            .into_owned();
        settings::persist(
            app.path(),
            Settings {
                mcp_project_allowlist: vec![allowed.clone()],
                ..Settings::default()
            },
        )
        .await
        .expect("persist allowlist");
        let state = AppState {
            app_data_dir: app.path().to_path_buf(),
            corpus_cache: Arc::new(Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(Mutex::new(())),
            skill_sources_write_lock: Arc::new(Mutex::new(())),
            skill_installs_write_lock: Arc::new(Mutex::new(())),
            skill_folders_write_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(RwLock::new(SettingsLoadState::FirstLaunch)),
            updater_state: crate::commands::updater::empty_state(),
        };

        assert!(state
            .authorize_mcp_client("claude", McpAction::Read, None)
            .await
            .is_ok());
        assert_eq!(
            state
                .authorize_mcp_client("claude", McpAction::Read, Some(&allowed))
                .await
                .expect("allowlisted read")
                .map(|project| project.identity().to_owned()),
            Some(allowed)
        );
        assert!(state
            .authorize_mcp_client("claude", McpAction::Read, Some(&denied))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn mcp_mutations_reload_settings_and_fail_closed_on_revocation_paranoid_or_corruption() {
        let app = tempfile::tempdir().expect("app data");
        let state = AppState {
            app_data_dir: app.path().to_path_buf(),
            corpus_cache: Arc::new(Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(Mutex::new(())),
            skill_sources_write_lock: Arc::new(Mutex::new(())),
            skill_installs_write_lock: Arc::new(Mutex::new(())),
            skill_folders_write_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(RwLock::new(SettingsLoadState::FirstLaunch)),
            updater_state: crate::commands::updater::empty_state(),
        };
        settings::persist(
            app.path(),
            Settings {
                mcp_source_access: true,
                mcp_agent_source_access: true,
                ..Settings::default()
            },
        )
        .await
        .expect("persist enabled policy");
        assert!(state.authorize_mcp(McpAction::Source, None).await.is_ok());
        assert!(state
            .authorize_mcp(McpAction::AgentSource, None)
            .await
            .is_ok());

        settings::persist(app.path(), Settings::default())
            .await
            .expect("persist revoked policy");
        assert!(state.authorize_mcp(McpAction::Source, None).await.is_err());
        assert!(state
            .authorize_mcp(McpAction::AgentSource, None)
            .await
            .is_err());

        settings::persist(
            app.path(),
            Settings {
                paranoid_mode: true,
                mcp_source_access: true,
                mcp_agent_source_access: true,
                ..Settings::default()
            },
        )
        .await
        .expect("persist paranoid policy");
        assert!(state.authorize_mcp(McpAction::Source, None).await.is_err());
        assert!(state
            .authorize_mcp(McpAction::AgentSource, None)
            .await
            .is_err());

        std::fs::write(settings::settings_path(app.path()), b"{invalid").expect("corrupt settings");
        assert!(state.authorize_mcp(McpAction::Source, None).await.is_err());
        assert!(state
            .authorize_mcp(McpAction::AgentSource, None)
            .await
            .is_err());
        assert!(matches!(
            &*state.settings.read().await,
            SettingsLoadState::Corrupt { .. }
        ));
        assert!(state.authorize_mcp(McpAction::Read, None).await.is_ok());
    }

    #[tokio::test]
    async fn completed_strict_cannot_be_followed_by_stale_mcp_authority() {
        let app = tempfile::tempdir().expect("app data");
        let permissive = Settings {
            mcp_source_access: true,
            ..Settings::default()
        };
        settings::persist(app.path(), permissive.clone())
            .await
            .expect("seed permissive policy");
        let state = Arc::new(AppState {
            app_data_dir: app.path().to_path_buf(),
            corpus_cache: Arc::new(Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(Mutex::new(())),
            skill_sources_write_lock: Arc::new(Mutex::new(())),
            skill_installs_write_lock: Arc::new(Mutex::new(())),
            skill_folders_write_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(RwLock::new(SettingsLoadState::Loaded(permissive.clone()))),
            updater_state: crate::commands::updater::empty_state(),
        });
        let load_started = Arc::new(tokio::sync::Notify::new());
        let release_load = Arc::new(tokio::sync::Notify::new());
        let authorize_state = Arc::clone(&state);
        let started = Arc::clone(&load_started);
        let release = Arc::clone(&release_load);
        let authorize = tokio::spawn(async move {
            authorize_state
                .authorize_mcp_client_with_load(
                    "claude",
                    McpAction::Source,
                    None,
                    move || async move {
                        started.notify_one();
                        release.notified().await;
                        SettingsLoadState::Loaded(permissive)
                    },
                )
                .await
        });

        load_started.notified().await;
        assert!(
            state.settings.try_write().is_err(),
            "authorization must hold the settings write lock before loading policy"
        );
        let strict_state = Arc::clone(&state);
        let strict = tokio::spawn(async move {
            crate::commands::settings::security_posture_apply_inner(
                &strict_state,
                crate::commands::settings::SecurityPosturePreset::Strict,
            )
            .await
        });
        release_load.notify_one();
        authorize.await.unwrap().expect("pre-Strict authorization");
        strict.await.unwrap().expect("apply Strict");

        assert!(state.require_network("race_regression").await.is_err());
        assert!(state
            .authorize_mcp_client("claude", McpAction::Source, None)
            .await
            .is_err());
        assert!(matches!(
            &*state.settings.read().await,
            SettingsLoadState::Loaded(settings)
                if settings.paranoid_mode && !settings.mcp_source_access
        ));
    }

    #[cfg(unix)]
    #[test]
    fn mcp_allowlist_rejects_an_ancestor_symlink_retarget() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("root");
        let identity_parent = root.path().join("identity");
        let original_parent = root.path().join("identity-original");
        let attacker_parent = root.path().join("attacker");
        std::fs::create_dir_all(identity_parent.join("project")).expect("identity project");
        std::fs::create_dir_all(attacker_parent.join("project")).expect("attacker project");
        let identity = std::fs::canonicalize(identity_parent.join("project"))
            .expect("canonical identity")
            .to_string_lossy()
            .into_owned();
        let identity_parent = PathBuf::from(&identity)
            .parent()
            .expect("identity parent")
            .to_path_buf();
        let policy = Settings {
            mcp_install_access: true,
            mcp_project_allowlist: vec![identity.clone()],
            ..Settings::default()
        };
        assert_eq!(
            authorize_mcp(&policy, McpAction::Install, Some(&identity)).expect("original identity"),
            Some(identity.clone())
        );

        std::fs::rename(&identity_parent, &original_parent).expect("move original identity");
        symlink(&attacker_parent, &identity_parent).expect("retarget identity ancestor");

        assert!(
            authorize_mcp(&policy, McpAction::Install, Some(&identity)).is_err(),
            "a persisted canonical identity must not follow a retargeted ancestor"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn mcp_allowlist_reload_preserves_identity_and_rejects_ancestor_symlink_retarget() {
        use std::os::unix::fs::symlink;

        let app = tempfile::tempdir().expect("app data");
        let root = tempfile::tempdir().expect("root");
        let identity_parent = root.path().join("identity");
        let original_parent = root.path().join("identity-original");
        let attacker_parent = root.path().join("attacker");
        std::fs::create_dir_all(identity_parent.join("project")).expect("identity project");
        std::fs::create_dir_all(attacker_parent.join("project")).expect("attacker project");
        let identity = std::fs::canonicalize(identity_parent.join("project"))
            .expect("canonical identity")
            .to_string_lossy()
            .into_owned();
        settings::persist(
            app.path(),
            Settings {
                mcp_install_access: true,
                mcp_project_allowlist: vec![identity.clone()],
                ..Settings::default()
            },
        )
        .await
        .expect("persist canonical identity");
        let state = AppState {
            app_data_dir: app.path().to_path_buf(),
            corpus_cache: Arc::new(Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(Mutex::new(())),
            skill_sources_write_lock: Arc::new(Mutex::new(())),
            skill_installs_write_lock: Arc::new(Mutex::new(())),
            skill_folders_write_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(RwLock::new(SettingsLoadState::FirstLaunch)),
            updater_state: crate::commands::updater::empty_state(),
        };
        assert_eq!(
            state
                .authorize_mcp(McpAction::Install, Some(&identity))
                .await
                .expect("original identity")
                .map(|project| project.identity().to_owned()),
            Some(identity.clone())
        );

        let identity_parent = PathBuf::from(&identity)
            .parent()
            .expect("identity parent")
            .to_path_buf();
        std::fs::rename(&identity_parent, &original_parent).expect("move original identity");
        symlink(&attacker_parent, &identity_parent).expect("retarget identity ancestor");

        assert!(
            state
                .authorize_mcp(McpAction::Install, Some(&identity))
                .await
                .is_err(),
            "disk reload must not replace the persisted identity with its new symlink target"
        );
        let loaded = state.settings.read().await;
        let SettingsLoadState::Loaded(settings) = &*loaded else {
            panic!("expected loaded settings");
        };
        assert_eq!(settings.mcp_project_allowlist, vec![identity]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unrelated_updater_save_preserves_retargeted_allowlist_identity_and_denial() {
        use std::os::unix::fs::symlink;

        let app = tempfile::tempdir().expect("app data");
        let root = tempfile::tempdir().expect("root");
        let identity_parent = root.path().join("identity");
        let original_parent = root.path().join("identity-original");
        let attacker_parent = root.path().join("attacker");
        std::fs::create_dir_all(identity_parent.join("project")).expect("identity project");
        std::fs::create_dir_all(attacker_parent.join("project")).expect("attacker project");
        let identity = std::fs::canonicalize(identity_parent.join("project"))
            .expect("canonical identity")
            .to_string_lossy()
            .into_owned();
        let initial = settings::persist(
            app.path(),
            Settings {
                mcp_install_access: true,
                mcp_project_allowlist: vec![identity.clone()],
                ..Settings::default()
            },
        )
        .await
        .expect("persist canonical identity");
        let state = AppState {
            app_data_dir: app.path().to_path_buf(),
            corpus_cache: Arc::new(Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(Mutex::new(())),
            skill_sources_write_lock: Arc::new(Mutex::new(())),
            skill_installs_write_lock: Arc::new(Mutex::new(())),
            skill_folders_write_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(RwLock::new(SettingsLoadState::Loaded(initial))),
            updater_state: crate::commands::updater::empty_state(),
        };

        let identity_parent = PathBuf::from(&identity)
            .parent()
            .expect("identity parent")
            .to_path_buf();
        std::fs::rename(&identity_parent, &original_parent).expect("move original identity");
        symlink(&attacker_parent, &identity_parent).expect("retarget identity ancestor");

        crate::commands::updater::run_skip(&state, "9.9.9")
            .await
            .expect("persist unrelated updater setting");
        let SettingsLoadState::Loaded(saved) = settings::load_async(app.path()).await else {
            panic!("expected loaded settings");
        };
        assert_eq!(
            saved.mcp_project_allowlist[0].as_bytes(),
            identity.as_bytes(),
            "an unrelated save must preserve an existing allowlist entry byte-for-byte"
        );
        assert!(
            state
                .authorize_mcp(McpAction::Install, Some(&identity))
                .await
                .is_err(),
            "an unrelated save must not re-authorize a retargeted identity"
        );
    }

    #[tokio::test]
    async fn serialized_settings_patches_do_not_resurrect_or_drop_mcp_policy() {
        let app = tempfile::tempdir().expect("app data");
        let state = AppState {
            app_data_dir: app.path().to_path_buf(),
            corpus_cache: Arc::new(Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(Mutex::new(())),
            skill_sources_write_lock: Arc::new(Mutex::new(())),
            skill_installs_write_lock: Arc::new(Mutex::new(())),
            skill_folders_write_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(RwLock::new(SettingsLoadState::FirstLaunch)),
            updater_state: crate::commands::updater::empty_state(),
        };
        settings::persist(
            app.path(),
            Settings {
                mcp_source_access: true,
                ..Settings::default()
            },
        )
        .await
        .expect("seed policy");

        let stale_unrelated = crate::commands::settings::GeneralSettingsPatch {
            github_enabled: Some(true),
            ..Default::default()
        };
        crate::commands::settings::settings_set_inner(&state, stale_unrelated)
            .await
            .expect("save general settings");
        let saved = crate::commands::settings::load_async(app.path()).await;
        let SettingsLoadState::Loaded(saved) = saved else {
            panic!("settings should remain readable")
        };
        assert!(saved.github_enabled);
        assert!(
            saved.mcp_source_access,
            "a stale full snapshot must not revoke or resurrect MCP policy"
        );

        let (policy, skip, paranoid, unrelated) = tokio::join!(
            crate::commands::settings::mcp_policy_set_inner(&state, false, true, false, Vec::new(),),
            crate::commands::updater::run_skip(&state, "9.9.10"),
            crate::commands::settings::settings_set_inner(
                &state,
                crate::commands::settings::GeneralSettingsPatch {
                    paranoid_mode: Some(true),
                    ..Default::default()
                },
            ),
            crate::commands::settings::settings_set_inner(
                &state,
                crate::commands::settings::GeneralSettingsPatch {
                    ai_features_enabled: Some(false),
                    ..Default::default()
                },
            ),
        );
        policy.expect("set policy");
        skip.expect("patch updater setting");
        paranoid.expect("enable paranoid mode");
        unrelated.expect("patch unrelated general setting");
        let saved = crate::commands::settings::load_async(app.path()).await;
        let SettingsLoadState::Loaded(saved) = saved else {
            panic!("settings should remain readable")
        };
        assert!(!saved.mcp_source_access);
        assert!(saved.mcp_install_access);
        assert_eq!(saved.skipped_update_versions, vec!["9.9.10"]);
        assert!(saved.paranoid_mode);
        assert!(!saved.ai_features_enabled);
    }

    #[tokio::test]
    async fn sqlite_audit_is_bounded_redacted_and_revision_neutral() {
        let app = tempfile::tempdir().expect("app data");
        let database = crate::state_db::StateDatabase::open(app.path()).unwrap();
        database
            .mutate_quiet(mcp_audit_spec(), Vec::new(), |_| Ok(()))
            .await
            .unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let revision = database.visible_revision().await.unwrap();

        append_mcp_audit(
            app.path(),
            McpAuditEntry {
                id: "audit".into(),
                timestamp: "2026-08-06T00:00:00Z".into(),
                client: None,
                tool: "not trusted".into(),
                action: "invalid".into(),
                phase: "invalid".into(),
                success: false,
                project_path: Some("token=secret".into()),
            },
        )
        .await
        .unwrap();

        assert_eq!(database.visible_revision().await.unwrap(), revision);
        let entries = load_mcp_audit(app.path()).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].tool, "[redacted]");
        assert_eq!(entries[0].project_path.as_deref(), Some("[redacted]"));
    }

    #[test]
    fn mcp_audit_preserves_only_known_factory_tool_names() {
        let entry = |tool: &str| McpAuditEntry {
            id: "factory-audit".into(),
            timestamp: "2026-08-19T00:00:00Z".into(),
            client: Some("codex".into()),
            tool: tool.into(),
            action: "source".into(),
            phase: "terminal".into(),
            success: true,
            project_path: None,
        };
        let mut known = entry("factory_runs_claim_phase");
        sanitize_mcp_audit(&mut known);
        assert_eq!(known.tool, "factory_runs_claim_phase");

        let mut invented = entry("factory_runs_approve_plan");
        sanitize_mcp_audit(&mut invented);
        assert_eq!(invented.tool, "[redacted]");
    }

    #[tokio::test]
    async fn mcp_audit_lock_wait_has_a_bounded_deadline() {
        let app = tempfile::tempdir().expect("app data");
        let (_, lock_path) = mcp_audit_paths(app.path());
        std::fs::create_dir_all(lock_path.parent().expect("lock parent")).expect("state dir");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .expect("open audit lock");
        lock.lock().expect("hold audit lock");

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            append_mcp_audit(
                app.path(),
                McpAuditEntry {
                    id: "bounded-lock".into(),
                    timestamp: "2026-07-30T00:00:00Z".into(),
                    client: None,
                    tool: "skills_search".into(),
                    action: "read".into(),
                    phase: "terminal".into(),
                    success: false,
                    project_path: None,
                },
            ),
        )
        .await
        .expect("audit lock acquisition must have a deadline");
        assert!(
            result.is_err(),
            "contended audit append unexpectedly succeeded"
        );
    }

    #[test]
    fn audit_replacement_delegates_without_deleting_the_existing_journal() {
        use std::cell::RefCell;

        let calls = RefCell::new(Vec::new());
        replace_mcp_audit_file_with(
            Path::new("audit.tmp"),
            Path::new("audit.jsonl"),
            |from, to| {
                calls
                    .borrow_mut()
                    .push(format!("replace:{}:{}", from.display(), to.display()));
                Ok(())
            },
        )
        .expect("platform replacement seam");

        assert_eq!(&*calls.borrow(), &["replace:audit.tmp:audit.jsonl"]);
    }

    #[tokio::test]
    async fn mcp_audit_is_concurrent_bounded_jsonl_and_redacts_untrusted_fields() {
        let app = tempfile::tempdir().expect("app data");
        let mut tasks = Vec::new();
        for index in 0..(MCP_AUDIT_MAX_ENTRIES + 8) {
            let app_data_dir = app.path().to_path_buf();
            tasks.push(tokio::spawn(async move {
                append_mcp_audit(
                    &app_data_dir,
                    McpAuditEntry {
                        id: format!("entry-{index}"),
                        timestamp: "2026-07-30T00:00:00Z".into(),
                        client: None,
                        tool: if index == 0 {
                            "skills_search\nAuthorization: Bearer top-secret".into()
                        } else {
                            "skills_search".into()
                        },
                        action: "read".into(),
                        phase: "terminal".into(),
                        success: index % 2 == 0,
                        project_path: (index == 0).then(|| "/tmp/project?token=top-secret".into()),
                    },
                )
                .await
            }));
        }
        for task in tasks {
            task.await.expect("audit task").expect("append audit");
        }

        let entries = load_mcp_audit(app.path()).await.expect("list audit");
        assert_eq!(entries.len(), MCP_AUDIT_MAX_ENTRIES);
        let raw = tokio::fs::read_to_string(app.path().join("state/mcp-audit.jsonl"))
            .await
            .expect("audit jsonl");
        assert_eq!(raw.lines().count(), MCP_AUDIT_MAX_ENTRIES);
        assert!(raw
            .lines()
            .all(|line| serde_json::from_str::<serde_json::Value>(line).is_ok()));
        assert!(!raw.contains("top-secret"), "{raw}");
        assert!(!raw.to_ascii_lowercase().contains("bearer"), "{raw}");
        assert!(entries.iter().all(|entry| {
            matches!(entry.phase.as_str(), "attempt" | "terminal")
                && entry.tool.chars().count() <= MCP_AUDIT_FIELD_CHARS_CAP
                && entry
                    .project_path
                    .as_ref()
                    .is_none_or(|path| path.chars().count() <= MCP_AUDIT_PROJECT_CHARS_CAP)
        }));
    }

    #[tokio::test]
    async fn mcp_audit_recovers_from_incomplete_or_oversize_prior_content() {
        for prior in [
            b"{\"partial\":true".to_vec(),
            vec![b'x'; MCP_AUDIT_MAX_BYTES + 1],
        ] {
            let app = tempfile::tempdir().expect("app data");
            let path = app.path().join("state/mcp-audit.jsonl");
            std::fs::create_dir_all(path.parent().expect("audit parent"))
                .expect("create audit parent");
            std::fs::write(&path, prior).expect("seed corrupt audit");

            append_mcp_audit(
                app.path(),
                McpAuditEntry {
                    id: "fresh-entry".into(),
                    timestamp: "2026-07-30T00:00:00Z".into(),
                    client: None,
                    tool: "skills_search".into(),
                    action: "read".into(),
                    phase: "terminal".into(),
                    success: true,
                    project_path: None,
                },
            )
            .await
            .expect("append after corrupt audit");

            let entries = load_mcp_audit(app.path()).await.expect("load audit");
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].id, "fresh-entry");
            assert!(
                std::fs::metadata(path).expect("audit metadata").len()
                    <= MCP_AUDIT_MAX_BYTES as u64
            );
        }
    }
}
