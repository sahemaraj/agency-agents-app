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
        let loaded = settings::load_async(&self.app_data_dir).await;
        {
            let mut guard = self.settings.write().await;
            *guard = loaded.clone();
        }
        let identity = match &loaded {
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
    let project_path = settings::canonical_mcp_project_path(project_path)?;
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

pub async fn append_mcp_audit(app_data_dir: &Path, entry: McpAuditEntry) -> Result<(), AppError> {
    let app_data_dir = app_data_dir.to_path_buf();
    tokio::task::spawn_blocking(move || append_mcp_audit_blocking(&app_data_dir, entry))
        .await
        .map_err(|error| AppError::Internal {
            message: format!("MCP audit task failed: {error}"),
        })?
}

pub async fn load_mcp_audit(app_data_dir: &Path) -> Result<Vec<McpAuditEntry>, AppError> {
    let app_data_dir = app_data_dir.to_path_buf();
    tokio::task::spawn_blocking(move || load_mcp_audit_blocking(&app_data_dir))
        .await
        .map_err(|error| AppError::Internal {
            message: format!("MCP audit task failed: {error}"),
        })?
}

#[tauri::command]
pub async fn mcp_audit_list(state: State<'_, AppState>) -> Result<Vec<McpAuditEntry>, AppError> {
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
    entry.tool = if entry.tool.starts_with("skills_")
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

// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::settings::Settings;
    use crate::types::McpAuditEntry;

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
