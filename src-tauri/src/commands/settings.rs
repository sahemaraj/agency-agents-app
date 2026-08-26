//! Settings persistence (Phase 12d).
//!
//! Stores user-configurable preferences in
//! `~/Library/Application Support/com.zerologic.agency-agents-app/settings.json`. Loaded once
//! at app startup into `AppState.settings` and refreshed by every
//! `settings_set` / `settings_reset` call so live readers (e.g. the
//! paranoid-mode gate consulted by `require_network`) see changes
//! immediately without a process restart.
//!
//! ## Security gates (security-review §12d)
//!
//! - **File-absent vs file-corrupt distinction.** [`SettingsLoadState`]
//!   carries three variants: `FirstLaunch` (file missing → defaults
//!   apply, paranoid OFF), `Loaded(Settings)` (good parse → use as-is),
//!   `Corrupt(message)` (file present but unreadable → **fail closed**;
//!   `require_network` denies everything until the user repairs).
//! - **Atomic writes.** Every save goes through [`crate::util::fs::atomic_write`]
//!   — temp + fsync + rename + fsync(parent). No torn writes.
//! - **Bounded path.** Settings always live at
//!   `state.app_data_dir.join("settings.json")`. No IPC argument can
//!   influence the location.
//! - **Size cap.** [`MAX_SETTINGS_BYTES`] (1 MiB) enforced on both read
//!   (via `read_capped`) and write (pre-serialize check + post-serialize
//!   check, defense in depth).
//! - **Schema validation.** `#[serde(default)]` on every field absorbs
//!   forward-compat additions; unknown enum variants fall back to the
//!   default with a stderr warning rather than rejecting the whole file.
//! - **Numeric clamps.** [`Settings::clamp`] re-applies the ranges
//!   declared in the type docs after every load and write so a manual
//!   edit (`settings.json` is plain JSON the user can poke at) can't
//!   smuggle an out-of-range value. MCP project paths are canonicalized
//!   only on explicit save; reload preserves their immutable identity.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::error::AppError;
use crate::state::AppState;
use crate::util::fs::{atomic_write, read_capped};

/// Hard cap on settings.json size. 1 MiB is wildly generous for what is
/// at most a few dozen scalar fields — protects against accidental or
/// hostile bloat (e.g. a future bug that appends to an array forever).
pub const MAX_SETTINGS_BYTES: u64 = 1024 * 1024;

/// On-disk + IPC payload. Every field has `#[serde(default)]` so a
/// future version that adds a field reads cleanly into an older shape
/// (missing fields take their defaults) and an older version reading a
/// newer file ignores fields it doesn't know about.
///
/// **Numeric clamping** is applied by [`Self::clamp`] after every load
/// and before every save. Don't bypass it — the caps are part of the
/// contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Master "block all outbound network" switch. When true,
    /// `require_network` denies every call. Default false (first launch
    /// = current behaviour preserved).
    pub paranoid_mode: bool,

    /// Show the "Catalog is N days old — refresh?" banner when the
    /// active catalog is at least this many days old. Default 14.
    /// Clamped to `[1, 365]` on every load and save.
    pub catalog_stale_banner_days: u32,

    /// Phase 12c — when true, PackageDetail probes `api.github.com` for
    /// repo stats whenever the package's homepage is a GitHub URL.
    /// Default **false** (off) so the v0.1.x posture of "no GitHub
    /// traffic unless the user opts in" is preserved on every fresh
    /// install. The runtime gate is `commands::github::*` which
    /// short-circuits to `Ok(None)` when this is false — before any
    /// outbound call. Paranoid mode overrides this regardless.
    pub github_enabled: bool,

    /// Phase 13 — master AI Features toggle. When false, AI-derived
    /// presentation data is hidden in the UI. Default **true**.
    ///
    /// This is a *rendering* gate — the enrichment payload is bundled
    /// into the binary regardless, so toggling this on/off doesn't
    /// trigger any I/O, network, or LLM calls.
    #[serde(default = "default_ai_features_enabled")]
    pub ai_features_enabled: bool,

    /// Phase 15 — opt-in daily auto-check for in-app updates. Default
    /// **false** so a fresh install never reaches out to the manifest
    /// endpoint without the user clicking either the manual "Check for
    /// updates" button or this toggle. When enabled (and Offline Mode
    /// is off), the scheduler in [`crate::commands::updater`] wakes
    /// every 24 h and runs `update_check_now`. Paranoid mode and a
    /// `Corrupt` settings state both suppress the scheduler — same gate
    /// every other outbound feature consults.
    #[serde(default)]
    pub update_auto_check: bool,

    /// Phase 16 — native alerts for newly actionable local Agent or Skill
    /// drift while the running app is backgrounded. Explicit opt-in only.
    #[serde(default)]
    pub drift_notifications: bool,

    /// Phase 15 — versions the user explicitly dismissed via the
    /// title-bar indicator's `×` button. Bounded at 10 entries with
    /// oldest-evicted-on-push (see [`Settings::push_skipped_version`]).
    /// The skip is per-version: a *newer* release re-triggers the
    /// indicator even if every previous version is in this list.
    #[serde(default)]
    pub skipped_update_versions: Vec<String>,

    /// Per-tool custom install base path (tool id → absolute base directory).
    /// When set for a tool, user-scope installs + detection resolve against
    /// this base instead of the OS home — e.g. pointing Claude Code at a WSL
    /// home (`\\wsl.localhost\Ubuntu\home\me`) from the Windows app. An empty
    /// or absent entry means "use the OS home". Project-scope installs are
    /// unaffected (they resolve against the chosen project root).
    #[serde(default)]
    pub tool_paths: HashMap<String, String>,

    /// Allow MCP clients to add or refresh registered skill sources.
    /// Off by default; reads remain available independently.
    #[serde(default)]
    pub mcp_source_access: bool,

    /// Allow MCP clients to install, update, or enable managed skills.
    /// Off by default.
    #[serde(default)]
    pub mcp_install_access: bool,

    /// Allow MCP clients to disable or uninstall managed skills.
    /// Off by default.
    #[serde(default)]
    pub mcp_destructive_access: bool,

    /// Allow MCP clients to add, refresh, draft, or organize Agents.
    /// Separate from Skills and off by default.
    #[serde(default)]
    pub mcp_agent_source_access: bool,

    /// Allow MCP clients to install, update, or enable managed Agents.
    /// Separate from Skills and off by default.
    #[serde(default)]
    pub mcp_agent_install_access: bool,

    /// Allow MCP clients to request destructive Agent lifecycle changes.
    /// Separate from Skills and off by default.
    #[serde(default)]
    pub mcp_agent_destructive_access: bool,

    /// Exact canonical project roots MCP mutations may target. User-scope
    /// mutations do not consult this list.
    #[serde(default)]
    pub mcp_project_allowlist: Vec<String>,

    /// Optional per-client overrides. Missing clients inherit the global
    /// mutation policy above.
    #[serde(default)]
    pub mcp_client_policies: HashMap<String, McpClientPolicy>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct McpClientPolicy {
    pub source_access: bool,
    pub install_access: bool,
    pub destructive_access: bool,
    pub agent_source_access: bool,
    pub agent_install_access: bool,
    pub agent_destructive_access: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SecurityPosture {
    Strict,
    LocalDevelopment,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SecurityPosturePreset {
    Strict,
    LocalDevelopment,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GeneralSettingsPatch {
    pub paranoid_mode: Option<bool>,
    pub catalog_stale_banner_days: Option<u32>,
    pub github_enabled: Option<bool>,
    pub ai_features_enabled: Option<bool>,
    pub update_auto_check: Option<bool>,
    pub drift_notifications: Option<bool>,
    pub tool_paths: Option<HashMap<String, String>>,
}

impl GeneralSettingsPatch {
    fn apply(self, settings: &mut Settings) {
        macro_rules! apply {
            ($field:ident) => {
                if let Some(value) = self.$field {
                    settings.$field = value;
                }
            };
        }
        apply!(paranoid_mode);
        apply!(catalog_stale_banner_days);
        apply!(github_enabled);
        apply!(ai_features_enabled);
        apply!(update_auto_check);
        apply!(drift_notifications);
        apply!(tool_paths);
    }
}

/// Default factory for [`Settings::ai_features_enabled`] — separated
/// out so `#[serde(default = "…")]` can pick it up for forward-compat
/// on settings.json files written before Phase 13.
fn default_ai_features_enabled() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            paranoid_mode: false,
            catalog_stale_banner_days: 14,
            // Off by default per Phase 12c plan: anonymous GitHub probes
            // are opt-in so first-launch posture stays "zero outbound
            // beyond what the user has already consented to".
            github_enabled: false,
            // On by default per Phase 13 plan: AI-enriched rendering is
            // a value-add the project wants to show off out of the box.
            // Toggling off reverts the UI to plain source/catalog metadata.
            ai_features_enabled: default_ai_features_enabled(),
            // Off by default per Phase 15 plan: the manifest endpoint
            // stays cold until the user explicitly opts in (or hits the
            // manual "Check for updates" button).
            update_auto_check: false,
            drift_notifications: false,
            // Empty by default — populated as the user dismisses
            // individual versions via the title-bar indicator's `×`.
            skipped_update_versions: Vec::new(),
            // Empty by default — user opts a tool into a custom base path
            // (e.g. a WSL home) from the Tools panel.
            tool_paths: HashMap::new(),
            mcp_source_access: false,
            mcp_install_access: false,
            mcp_destructive_access: false,
            mcp_agent_source_access: false,
            mcp_agent_install_access: false,
            mcp_agent_destructive_access: false,
            mcp_project_allowlist: Vec::new(),
            mcp_client_policies: HashMap::new(),
        }
    }
}

impl Settings {
    /// Inclusive lower bound for `catalog_stale_banner_days`.
    pub const CATALOG_STALE_DAYS_MIN: u32 = 1;
    /// Inclusive upper bound for `catalog_stale_banner_days`.
    pub const CATALOG_STALE_DAYS_MAX: u32 = 365;
    /// Phase 15 — maximum entries kept in [`Self::skipped_update_versions`].
    /// Push beyond this evicts the oldest entry (FIFO) so the list
    /// can't grow without bound across decades of releases.
    pub const SKIPPED_UPDATE_VERSIONS_CAP: usize = 10;
    pub const MCP_PROJECT_ALLOWLIST_CAP: usize = 64;
    pub const MCP_PROJECT_PATH_CHARS_CAP: usize = 4096;

    fn security_posture(&self) -> SecurityPosture {
        let no_mutations = !self.mcp_source_access
            && !self.mcp_install_access
            && !self.mcp_destructive_access
            && !self.mcp_agent_source_access
            && !self.mcp_agent_install_access
            && !self.mcp_agent_destructive_access;
        let all_mutations = self.mcp_source_access
            && self.mcp_install_access
            && self.mcp_destructive_access
            && self.mcp_agent_source_access
            && self.mcp_agent_install_access
            && self.mcp_agent_destructive_access;
        if self.paranoid_mode
            && !self.github_enabled
            && !self.update_auto_check
            && !self.drift_notifications
            && no_mutations
            && self.mcp_client_policies.is_empty()
        {
            SecurityPosture::Strict
        } else if !self.paranoid_mode && all_mutations && self.mcp_client_policies.is_empty() {
            SecurityPosture::LocalDevelopment
        } else {
            SecurityPosture::Custom
        }
    }

    fn apply_security_posture(&mut self, preset: SecurityPosturePreset) {
        let enabled = matches!(preset, SecurityPosturePreset::LocalDevelopment);
        self.paranoid_mode = !enabled;
        if !enabled {
            self.github_enabled = false;
            self.update_auto_check = false;
            self.drift_notifications = false;
        }
        self.mcp_source_access = enabled;
        self.mcp_install_access = enabled;
        self.mcp_destructive_access = enabled;
        self.mcp_agent_source_access = enabled;
        self.mcp_agent_install_access = enabled;
        self.mcp_agent_destructive_access = enabled;
        self.mcp_client_policies.clear();
    }

    /// Apply the numeric clamps declared in the field docs. Idempotent;
    /// safe to call on already-clamped values.
    pub fn clamp(&mut self) {
        self.catalog_stale_banner_days = self
            .catalog_stale_banner_days
            .clamp(Self::CATALOG_STALE_DAYS_MIN, Self::CATALOG_STALE_DAYS_MAX);
        self.mcp_client_policies
            .retain(|client, _| matches!(client.as_str(), "claude" | "codex"));
        // Enforce the cap on every load/save in addition to the push
        // helper so a hand-edited settings.json with 50 skip entries
        // gets pruned on read.
        if self.skipped_update_versions.len() > Self::SKIPPED_UPDATE_VERSIONS_CAP {
            let excess = self.skipped_update_versions.len() - Self::SKIPPED_UPDATE_VERSIONS_CAP;
            self.skipped_update_versions.drain(..excess);
        }
        let mut seen = HashSet::new();
        self.mcp_project_allowlist = self
            .mcp_project_allowlist
            .drain(..)
            .filter(|path| path.chars().count() <= Self::MCP_PROJECT_PATH_CHARS_CAP)
            .filter(|path| seen.insert(path.clone()))
            .take(Self::MCP_PROJECT_ALLOWLIST_CAP)
            .collect();
    }

    fn canonicalize_new_mcp_project_allowlist(&mut self, existing: &[String]) {
        let mut seen = HashSet::new();
        self.mcp_project_allowlist = self
            .mcp_project_allowlist
            .drain(..)
            .filter(|path| path.chars().count() <= Self::MCP_PROJECT_PATH_CHARS_CAP)
            .filter_map(|path| {
                if existing.iter().any(|saved| saved == &path) {
                    Some(path)
                } else {
                    canonical_mcp_project_path(path.trim())
                        .ok()
                        .map(|path| path.to_string_lossy().into_owned())
                }
            })
            .filter(|path| seen.insert(path.clone()))
            .take(Self::MCP_PROJECT_ALLOWLIST_CAP)
            .collect();
    }

    /// Phase 15 — push `version` onto [`Self::skipped_update_versions`]
    /// with FIFO eviction when the cap is reached. Duplicate-safe: if
    /// `version` is already in the list, the entry is moved to the
    /// tail (so a re-skip refreshes its position rather than padding
    /// the cap with duplicates).
    ///
    /// Returns `true` when the list changed, `false` when the version
    /// was already at the tail. Callers persist the settings whenever
    /// this returns `true`.
    #[allow(dead_code)] // used by Phase 15 updater commands
    pub fn push_skipped_version(&mut self, version: String) -> bool {
        // De-duplicate: drop any existing entry for this version so the
        // push always moves it to the tail.
        let already_at_tail = self
            .skipped_update_versions
            .last()
            .is_some_and(|v| v == &version);
        if already_at_tail {
            return false;
        }
        self.skipped_update_versions.retain(|v| v != &version);
        self.skipped_update_versions.push(version);
        while self.skipped_update_versions.len() > Self::SKIPPED_UPDATE_VERSIONS_CAP {
            self.skipped_update_versions.remove(0);
        }
        true
    }
}

pub(crate) fn canonical_mcp_project_path(path: &str) -> Result<PathBuf, AppError> {
    let path = Path::new(path);
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(AppError::InvalidArgument {
            message: "MCP project path must be absolute and normalized".into(),
        });
    }
    path.ancestors()
        .find_map(|ancestor| {
            std::fs::canonicalize(ancestor)
                .ok()
                .map(|base| (ancestor, base))
        })
        .and_then(|(ancestor, base)| {
            path.strip_prefix(ancestor).ok().map(|suffix| {
                if suffix.as_os_str().is_empty() {
                    base
                } else {
                    base.join(suffix)
                }
            })
        })
        .ok_or_else(|| AppError::InvalidArgument {
            message: "MCP project path has no resolvable absolute ancestor".into(),
        })
}

/// Three-state container for the in-memory settings cache.
///
/// The distinction between `FirstLaunch` and `Corrupt` is **load-bearing**
/// (security review §12d): the former applies defaults (paranoid OFF),
/// the latter fails closed (paranoid effectively ON until the user
/// repairs the file or hits the reset button in the Settings UI).
#[derive(Debug, Clone)]
pub enum SettingsLoadState {
    /// `settings.json` did not exist when we tried to read it. New
    /// installs, freshly-reset apps, etc. Defaults apply.
    FirstLaunch,
    /// Successfully parsed. Carries the clamped, validated Settings.
    Loaded(Settings),
    /// File present but unreadable (bad JSON, oversize, read error).
    /// `require_network` denies every call until repaired. The message
    /// is surfaced via `settings_get` so the UI can show a clear "Reset
    /// to defaults" affordance instead of silently rolling back.
    Corrupt { message: String },
}

impl SettingsLoadState {
    /// Convenience for the gate: returns the effective settings when
    /// they should be honoured, or `None` when the load failed and we
    /// should fall back to "deny outbound" semantics.
    ///
    /// `AppState::require_network` reaches for the variants directly
    /// rather than this helper (to keep the gate's logic visible in one
    /// place), but the helper is the canonical reference for anything
    /// else that needs the same disambiguation — kept available for
    /// future callers (settings UI, diagnostics) and exercised by tests.
    #[allow(dead_code)]
    pub fn effective_settings(&self) -> Option<Settings> {
        match self {
            SettingsLoadState::Loaded(s) => Some(s.clone()),
            SettingsLoadState::FirstLaunch => Some(Settings::default()),
            SettingsLoadState::Corrupt { .. } => None,
        }
    }
}

/// Resolve the canonical settings path inside `app_data_dir`.
///
/// Always `<app_data_dir>/settings.json`. The directory is created if
/// missing — the caller (typically `AppState::build`) has already
/// ensured `app_data_dir` exists, so this is a defense-in-depth mkdir.
pub fn settings_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("settings.json")
}

fn validate_persisted_settings(settings: &Settings) -> Result<(), AppError> {
    let mut clamped = settings.clone();
    clamped.clamp();
    if clamped != *settings {
        return Err(AppError::InvalidArgument {
            message: "persisted settings exceed supported bounds".into(),
        });
    }
    Ok(())
}

fn settings_spec() -> crate::state_db::DocumentSpec<Settings> {
    crate::state_db::DocumentSpec::new(
        "settings",
        1,
        MAX_SETTINGS_BYTES,
        validate_persisted_settings,
    )
}

pub(crate) fn settings_import_spec() -> crate::state_db::ImportSpec {
    crate::state_db::ImportSpec::document(settings_spec(), Settings::default())
}

fn settings_database(
    app_data_dir: &Path,
) -> Result<Option<crate::state_db::StateDatabase>, AppError> {
    if !app_data_dir
        .join("state")
        .join("agency-agents.sqlite3")
        .exists()
    {
        return Ok(None);
    }
    let database = crate::state_db::StateDatabase::open(app_data_dir)?;
    match database.migration_state_blocking()? {
        crate::types::StorageMigrationState::Complete => Ok(Some(database)),
        crate::types::StorageMigrationState::Legacy
        | crate::types::StorageMigrationState::InProgress => Ok(None),
        crate::types::StorageMigrationState::Corrupt => Err(AppError::StorageCorrupt {
            message: "settings database is corrupt".into(),
        }),
        crate::types::StorageMigrationState::Unsupported => Err(AppError::StorageUnsupported {
            found: crate::state_db::SCHEMA_VERSION.saturating_add(1),
            supported: crate::state_db::SCHEMA_VERSION,
        }),
    }
}

async fn settings_database_async(
    app_data_dir: &Path,
) -> Result<Option<crate::state_db::StateDatabase>, AppError> {
    let app_data_dir = app_data_dir.to_path_buf();
    tokio::task::spawn_blocking(move || settings_database(&app_data_dir))
        .await
        .map_err(|error| AppError::Internal {
            message: format!("settings database task failed: {error}"),
        })?
}

/// Synchronous startup loader. Called from `AppState::build()` (which is
/// a non-async function) so we use the blocking `std::fs` API rather
/// than tokio. The trade-off accepted is a single small read on startup
/// in exchange for a much simpler init story.
///
/// Returns the same three-state shape as the async loader so callers
/// stay uniform.
pub fn load_at_startup(app_data_dir: &Path) -> SettingsLoadState {
    match settings_database(app_data_dir) {
        Ok(Some(database)) => {
            return match database.read_blocking(settings_spec()) {
                Ok(Some(settings)) => SettingsLoadState::Loaded(settings),
                Ok(None) => SettingsLoadState::Corrupt {
                    message: "settings are missing after SQLite migration".into(),
                },
                Err(error) => SettingsLoadState::Corrupt {
                    message: error.to_string(),
                },
            };
        }
        Ok(None) => {}
        Err(error) => {
            return SettingsLoadState::Corrupt {
                message: error.to_string(),
            };
        }
    }
    let path = settings_path(app_data_dir);

    let meta = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return SettingsLoadState::FirstLaunch;
        }
        Err(e) => {
            // Stat failed for some non-NotFound reason (permission denied,
            // EIO, etc.). Treat as corrupt — fail closed.
            tracing::warn!("settings: stat failed at {}: {e}", path.display());
            return SettingsLoadState::Corrupt {
                message: format!("stat {}: {e}", path.display()),
            };
        }
    };

    if meta.len() > MAX_SETTINGS_BYTES {
        tracing::warn!(
            "settings: {} is {} bytes, exceeds {}-byte cap; treating as corrupt",
            path.display(),
            meta.len(),
            MAX_SETTINGS_BYTES
        );
        return SettingsLoadState::Corrupt {
            message: format!(
                "settings.json is {} bytes, exceeds {}-byte cap",
                meta.len(),
                MAX_SETTINGS_BYTES
            ),
        };
    }

    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("settings: read failed at {}: {e}", path.display());
            return SettingsLoadState::Corrupt {
                message: format!("read {}: {e}", path.display()),
            };
        }
    };

    match serde_json::from_slice::<Settings>(&bytes) {
        Ok(mut s) => {
            s.clamp();
            SettingsLoadState::Loaded(s)
        }
        Err(e) => {
            tracing::warn!(
                "settings: parse failed at {}: {e}; treating as corrupt",
                path.display()
            );
            SettingsLoadState::Corrupt {
                message: format!("parse {}: {e}", path.display()),
            }
        }
    }
}

/// Async loader, identical semantics to [`load_at_startup`] but
/// non-blocking. Used by tests and any future callers that need to
/// re-read from disk without blocking the runtime.
pub(crate) async fn load_async(app_data_dir: &Path) -> SettingsLoadState {
    match settings_database_async(app_data_dir).await {
        Ok(Some(database)) => {
            return match database.read(settings_spec()).await {
                Ok(Some(settings)) => SettingsLoadState::Loaded(settings),
                Ok(None) => SettingsLoadState::Corrupt {
                    message: "settings are missing after SQLite migration".into(),
                },
                Err(error) => SettingsLoadState::Corrupt {
                    message: error.to_string(),
                },
            };
        }
        Ok(None) => {}
        Err(error) => {
            return SettingsLoadState::Corrupt {
                message: error.to_string(),
            };
        }
    }
    let path = settings_path(app_data_dir);

    let meta = match tokio::fs::metadata(&path).await {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return SettingsLoadState::FirstLaunch;
        }
        Err(e) => {
            tracing::warn!("settings: stat failed at {}: {e}", path.display());
            return SettingsLoadState::Corrupt {
                message: format!("stat {}: {e}", path.display()),
            };
        }
    };

    if meta.len() > MAX_SETTINGS_BYTES {
        return SettingsLoadState::Corrupt {
            message: format!(
                "settings.json is {} bytes, exceeds {}-byte cap",
                meta.len(),
                MAX_SETTINGS_BYTES
            ),
        };
    }

    let bytes = match read_capped(&path, MAX_SETTINGS_BYTES).await {
        Ok(b) => b,
        Err(e) => {
            return SettingsLoadState::Corrupt {
                message: format!("read {}: {e}", path.display()),
            };
        }
    };

    match serde_json::from_slice::<Settings>(&bytes) {
        Ok(mut s) => {
            s.clamp();
            SettingsLoadState::Loaded(s)
        }
        Err(e) => SettingsLoadState::Corrupt {
            message: format!("parse {}: {e}", path.display()),
        },
    }
}

/// Serialize `settings`, enforce the size cap, then atomically persist.
///
/// Order: (1) clamp numerics (no-op if already in range), (2) serialize
/// to bytes, (3) reject if the byte length exceeds the cap, (4)
/// `atomic_write` into place, (5) return the clamped struct so callers
/// can re-broadcast the canonicalized values.
pub(crate) async fn persist(
    app_data_dir: &Path,
    mut settings: Settings,
) -> Result<Settings, AppError> {
    let existing_allowlist = match load_async(app_data_dir).await {
        SettingsLoadState::Loaded(existing) => existing.mcp_project_allowlist,
        SettingsLoadState::FirstLaunch | SettingsLoadState::Corrupt { .. } => Vec::new(),
    };
    settings.canonicalize_new_mcp_project_allowlist(&existing_allowlist);
    settings.clamp();
    let bytes = serde_json::to_vec_pretty(&settings).map_err(|e| AppError::Internal {
        message: format!("serialize settings: {e}"),
    })?;
    if bytes.len() as u64 > MAX_SETTINGS_BYTES {
        return Err(AppError::InvalidArgument {
            message: format!(
                "serialized settings are {} bytes, exceeds {}-byte cap",
                bytes.len(),
                MAX_SETTINGS_BYTES
            ),
        });
    }

    if let Some(database) = settings_database_async(app_data_dir).await? {
        let replacement = settings.clone();
        database
            .mutate(settings_spec(), Settings::default(), move |current| {
                *current = replacement;
                Ok(())
            })
            .await?;
        return Ok(settings);
    }

    // Defense in depth — ensure the parent dir exists. `AppState::build`
    // already mkdir_p'd it, but a fresh checkout of the app on a system
    // that's never run Shikigami could plausibly hit this otherwise.
    if !app_data_dir.exists() {
        tokio::fs::create_dir_all(app_data_dir)
            .await
            .map_err(|e| AppError::Io {
                message: format!("create settings parent {}: {e}", app_data_dir.display()),
            })?;
    }

    let path = settings_path(app_data_dir);
    atomic_write(&path, &bytes).await?;
    Ok(settings)
}

// ---------- Commands ----------

pub(crate) async fn settings_set_inner(
    state: &AppState,
    patch: GeneralSettingsPatch,
) -> Result<Settings, AppError> {
    let _policy_lease =
        crate::state_db::SecurityPolicyLease::exclusive(&state.app_data_dir).await?;
    let mut cache = state.settings.write().await;
    if let SettingsLoadState::Corrupt { message } = &*cache {
        return Err(AppError::Internal {
            message: format!("settings file is unreadable: {message}"),
        });
    }
    let mut latest = match load_async(&state.app_data_dir).await {
        SettingsLoadState::Loaded(latest) => latest,
        SettingsLoadState::FirstLaunch => Settings::default(),
        SettingsLoadState::Corrupt { message } => {
            return Err(AppError::Internal {
                message: format!("settings file is unreadable: {message}"),
            });
        }
    };
    patch.apply(&mut latest);
    let clamped = persist(&state.app_data_dir, latest).await?;
    *cache = SettingsLoadState::Loaded(clamped.clone());
    Ok(clamped)
}

pub(crate) async fn mcp_policy_set_inner(
    state: &AppState,
    source_access: bool,
    install_access: bool,
    destructive_access: bool,
    project_allowlist: Vec<String>,
) -> Result<Settings, AppError> {
    let _policy_lease =
        crate::state_db::SecurityPolicyLease::exclusive(&state.app_data_dir).await?;
    let mut cache = state.settings.write().await;
    if let SettingsLoadState::Corrupt { message } = &*cache {
        return Err(AppError::Internal {
            message: format!("settings file is unreadable: {message}"),
        });
    }
    let mut latest = match load_async(&state.app_data_dir).await {
        SettingsLoadState::Loaded(latest) => latest,
        SettingsLoadState::FirstLaunch => Settings::default(),
        SettingsLoadState::Corrupt { message } => {
            return Err(AppError::Internal {
                message: format!("settings file is unreadable: {message}"),
            });
        }
    };
    latest.mcp_source_access = source_access;
    latest.mcp_install_access = install_access;
    latest.mcp_destructive_access = destructive_access;
    latest.mcp_project_allowlist = project_allowlist;
    let clamped = persist(&state.app_data_dir, latest).await?;
    *cache = SettingsLoadState::Loaded(clamped.clone());
    Ok(clamped)
}

pub(crate) async fn mcp_agent_policy_set_inner(
    state: &AppState,
    source_access: bool,
    install_access: bool,
    destructive_access: bool,
) -> Result<Settings, AppError> {
    let _policy_lease =
        crate::state_db::SecurityPolicyLease::exclusive(&state.app_data_dir).await?;
    let mut cache = state.settings.write().await;
    let mut latest = match load_async(&state.app_data_dir).await {
        SettingsLoadState::Loaded(latest) => latest,
        SettingsLoadState::FirstLaunch => Settings::default(),
        SettingsLoadState::Corrupt { message } => {
            return Err(AppError::Internal {
                message: format!("settings file is unreadable: {message}"),
            })
        }
    };
    latest.mcp_agent_source_access = source_access;
    latest.mcp_agent_install_access = install_access;
    latest.mcp_agent_destructive_access = destructive_access;
    let saved = persist(&state.app_data_dir, latest).await?;
    *cache = SettingsLoadState::Loaded(saved.clone());
    Ok(saved)
}

pub(crate) async fn security_posture_apply_inner(
    state: &AppState,
    preset: SecurityPosturePreset,
) -> Result<Settings, AppError> {
    let _policy_lease =
        crate::state_db::SecurityPolicyLease::exclusive(&state.app_data_dir).await?;
    let mut cache = state.settings.write().await;
    if let SettingsLoadState::Corrupt { message } = &*cache {
        return Err(AppError::Internal {
            message: format!("settings file is unreadable: {message}"),
        });
    }
    let mut latest = match load_async(&state.app_data_dir).await {
        SettingsLoadState::Loaded(latest) => latest,
        SettingsLoadState::FirstLaunch => Settings::default(),
        SettingsLoadState::Corrupt { message } => {
            *cache = SettingsLoadState::Corrupt {
                message: message.clone(),
            };
            return Err(AppError::Internal {
                message: format!("settings file is unreadable: {message}"),
            });
        }
    };
    latest.apply_security_posture(preset);
    let saved = persist(&state.app_data_dir, latest).await?;
    debug_assert_eq!(
        saved.security_posture(),
        match preset {
            SecurityPosturePreset::Strict => SecurityPosture::Strict,
            SecurityPosturePreset::LocalDevelopment => SecurityPosture::LocalDevelopment,
        }
    );
    *cache = SettingsLoadState::Loaded(saved.clone());
    Ok(saved)
}

/// Read the current settings.
///
/// Always returns the *currently-loaded* state — does not re-read from
/// disk on every call (the in-memory cache is authoritative and is
/// refreshed by `settings_set` / `settings_reset`).
///
/// Returns an error when the loaded state is `Corrupt`, so the frontend
/// can surface a "Settings file unreadable — reset to defaults?" prompt
/// without exposing the corrupt JSON contents.
#[tauri::command]
pub async fn settings_get(state: State<'_, AppState>) -> Result<Settings, AppError> {
    let guard = state.settings.read().await;
    match &*guard {
        SettingsLoadState::Loaded(s) => Ok(s.clone()),
        SettingsLoadState::FirstLaunch => Ok(Settings::default()),
        SettingsLoadState::Corrupt { message } => Err(AppError::Internal {
            message: format!("settings file is unreadable: {message}"),
        }),
    }
}

/// Write general settings and update the in-memory cache. MCP security policy
/// is preserved from the latest disk state and can only be changed through
/// `mcp_policy_set`, preventing stale renderer snapshots from resurrecting it.
#[tauri::command]
pub async fn settings_set(
    patch: GeneralSettingsPatch,
    state: State<'_, AppState>,
) -> Result<Settings, AppError> {
    settings_set_inner(&state, patch).await
}

#[tauri::command]
pub async fn mcp_policy_set(
    source_access: bool,
    install_access: bool,
    destructive_access: bool,
    project_allowlist: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Settings, AppError> {
    mcp_policy_set_inner(
        &state,
        source_access,
        install_access,
        destructive_access,
        project_allowlist,
    )
    .await
}

#[tauri::command]
pub async fn mcp_agent_policy_set(
    source_access: bool,
    install_access: bool,
    destructive_access: bool,
    state: State<'_, AppState>,
) -> Result<Settings, AppError> {
    mcp_agent_policy_set_inner(&state, source_access, install_access, destructive_access).await
}

#[tauri::command]
pub async fn security_posture_apply(
    preset: SecurityPosturePreset,
    state: State<'_, AppState>,
) -> Result<Settings, AppError> {
    security_posture_apply_inner(&state, preset).await
}

#[tauri::command]
pub async fn mcp_client_policy_set(
    client: String,
    source_access: bool,
    install_access: bool,
    destructive_access: bool,
    state: State<'_, AppState>,
) -> Result<Settings, AppError> {
    if !matches!(client.as_str(), "claude" | "codex") {
        return Err(AppError::InvalidArgument {
            message: "MCP client must be claude or codex".into(),
        });
    }
    let _policy_lease =
        crate::state_db::SecurityPolicyLease::exclusive(&state.app_data_dir).await?;
    let mut cache = state.settings.write().await;
    let mut latest = match load_async(&state.app_data_dir).await {
        SettingsLoadState::Loaded(latest) => latest,
        SettingsLoadState::FirstLaunch => Settings::default(),
        SettingsLoadState::Corrupt { message } => {
            return Err(AppError::Internal {
                message: format!("settings file is unreadable: {message}"),
            })
        }
    };
    let policy = latest.mcp_client_policies.entry(client).or_default();
    policy.source_access = source_access;
    policy.install_access = install_access;
    policy.destructive_access = destructive_access;
    let saved = persist(&state.app_data_dir, latest).await?;
    *cache = SettingsLoadState::Loaded(saved.clone());
    Ok(saved)
}

#[tauri::command]
pub async fn mcp_agent_client_policy_set(
    client: String,
    source_access: bool,
    install_access: bool,
    destructive_access: bool,
    state: State<'_, AppState>,
) -> Result<Settings, AppError> {
    if !matches!(client.as_str(), "claude" | "codex") {
        return Err(AppError::InvalidArgument {
            message: "MCP client must be claude or codex".into(),
        });
    }
    let _policy_lease =
        crate::state_db::SecurityPolicyLease::exclusive(&state.app_data_dir).await?;
    let mut cache = state.settings.write().await;
    let mut latest = match load_async(&state.app_data_dir).await {
        SettingsLoadState::Loaded(latest) => latest,
        SettingsLoadState::FirstLaunch => Settings::default(),
        SettingsLoadState::Corrupt { message } => {
            return Err(AppError::Internal {
                message: format!("settings file is unreadable: {message}"),
            })
        }
    };
    let policy = latest.mcp_client_policies.entry(client).or_default();
    policy.agent_source_access = source_access;
    policy.agent_install_access = install_access;
    policy.agent_destructive_access = destructive_access;
    let saved = persist(&state.app_data_dir, latest).await?;
    *cache = SettingsLoadState::Loaded(saved.clone());
    Ok(saved)
}

/// Overwrite `settings.json` with the defaults and update the
/// in-memory cache. Used by the UI's "Reset to defaults" button when
/// the file is corrupt or the user just wants to start fresh.
#[tauri::command]
pub async fn settings_reset(state: State<'_, AppState>) -> Result<Settings, AppError> {
    let _policy_lease =
        crate::state_db::SecurityPolicyLease::exclusive(&state.app_data_dir).await?;
    let mut cache = state.settings.write().await;
    let defaults = Settings::default();
    let clamped = persist(&state.app_data_dir, defaults).await?;
    *cache = SettingsLoadState::Loaded(clamped.clone());
    Ok(clamped)
}

/// Return the app's version string from the Tauri package info. Source of
/// truth is `Cargo.toml` (`tauri.conf.json` mirrors it). Avoids reading
/// `package.json` from the renderer.
#[tauri::command]
pub fn app_version<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> String {
    app.package_info().version.to_string()
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app_state(app_data_dir: &Path, settings: SettingsLoadState) -> AppState {
        AppState {
            app_data_dir: app_data_dir.to_path_buf(),
            corpus_cache: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            corpus_refresh_in_flight: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            skill_sources_write_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            skill_installs_write_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            skill_folders_write_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            settings: std::sync::Arc::new(tokio::sync::RwLock::new(settings)),
            updater_state: crate::commands::updater::empty_state(),
        }
    }

    #[test]
    fn security_posture_classification_is_exact() {
        let mut settings = Settings::default();
        settings.apply_security_posture(SecurityPosturePreset::Strict);
        assert_eq!(settings.security_posture(), SecurityPosture::Strict);

        let mut network_opt_ins = settings.clone();
        network_opt_ins.github_enabled = true;
        assert_eq!(network_opt_ins.security_posture(), SecurityPosture::Custom);

        settings.apply_security_posture(SecurityPosturePreset::LocalDevelopment);
        assert_eq!(
            settings.security_posture(),
            SecurityPosture::LocalDevelopment
        );

        settings.mcp_agent_install_access = false;
        assert_eq!(settings.security_posture(), SecurityPosture::Custom);
    }

    #[test]
    fn strict_clears_every_override_and_network_mutation_path() {
        let mut settings = Settings {
            paranoid_mode: false,
            github_enabled: true,
            update_auto_check: true,
            drift_notifications: true,
            catalog_stale_banner_days: 31,
            ai_features_enabled: false,
            tool_paths: HashMap::from([("codex".into(), "/custom".into())]),
            mcp_source_access: true,
            mcp_install_access: true,
            mcp_destructive_access: true,
            mcp_agent_source_access: true,
            mcp_agent_install_access: true,
            mcp_agent_destructive_access: true,
            mcp_project_allowlist: vec!["/retained/project".into()],
            mcp_client_policies: HashMap::from([(
                "claude".into(),
                McpClientPolicy {
                    source_access: true,
                    install_access: true,
                    destructive_access: true,
                    agent_source_access: true,
                    agent_install_access: true,
                    agent_destructive_access: true,
                },
            )]),
            ..Settings::default()
        };

        settings.apply_security_posture(SecurityPosturePreset::Strict);

        assert!(settings.paranoid_mode);
        assert!(!settings.github_enabled);
        assert!(!settings.update_auto_check);
        assert!(!settings.drift_notifications);
        assert!(!settings.mcp_source_access);
        assert!(!settings.mcp_install_access);
        assert!(!settings.mcp_destructive_access);
        assert!(!settings.mcp_agent_source_access);
        assert!(!settings.mcp_agent_install_access);
        assert!(!settings.mcp_agent_destructive_access);
        assert!(settings.mcp_client_policies.is_empty());
        assert_eq!(settings.mcp_project_allowlist, ["/retained/project"]);
        assert_eq!(settings.catalog_stale_banner_days, 31);
        assert!(!settings.ai_features_enabled);
        assert_eq!(settings.tool_paths["codex"], "/custom");
    }

    #[test]
    fn local_development_preserves_network_consent_and_unrelated_settings() {
        let mut settings = Settings {
            paranoid_mode: true,
            github_enabled: true,
            update_auto_check: false,
            drift_notifications: true,
            catalog_stale_banner_days: 45,
            ai_features_enabled: false,
            mcp_project_allowlist: vec!["/retained/project".into()],
            mcp_client_policies: HashMap::from([("codex".into(), McpClientPolicy::default())]),
            ..Settings::default()
        };

        settings.apply_security_posture(SecurityPosturePreset::LocalDevelopment);

        assert!(!settings.paranoid_mode);
        assert!(settings.github_enabled);
        assert!(!settings.update_auto_check);
        assert!(settings.drift_notifications);
        assert!(settings.mcp_source_access);
        assert!(settings.mcp_install_access);
        assert!(settings.mcp_destructive_access);
        assert!(settings.mcp_agent_source_access);
        assert!(settings.mcp_agent_install_access);
        assert!(settings.mcp_agent_destructive_access);
        assert!(settings.mcp_client_policies.is_empty());
        assert_eq!(settings.mcp_project_allowlist, ["/retained/project"]);
        assert_eq!(settings.catalog_stale_banner_days, 45);
        assert!(!settings.ai_features_enabled);
    }

    #[tokio::test]
    async fn mcp_policy_revocations_wait_for_in_flight_mutation_lease() {
        let app = tempfile::tempdir().expect("app data");
        let permissive = persist(
            app.path(),
            Settings {
                mcp_source_access: true,
                mcp_project_allowlist: vec![app.path().to_string_lossy().into_owned()],
                ..Settings::default()
            },
        )
        .await
        .expect("seed permissive policy");
        let state = std::sync::Arc::new(test_app_state(
            app.path(),
            SettingsLoadState::Loaded(permissive),
        ));
        let mutation_lease = crate::state_db::SecurityPolicyLease::exclusive(app.path())
            .await
            .expect("hold mutation policy lease");
        let started = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let mut revoke = tokio::spawn({
            let state = std::sync::Arc::clone(&state);
            let started = std::sync::Arc::clone(&started);
            async move {
                started.wait().await;
                mcp_policy_set_inner(&state, false, false, false, Vec::new()).await
            }
        });
        started.wait().await;

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(500), &mut revoke)
                .await
                .is_err(),
            "policy revoke returned before the in-flight mutation lease was released"
        );

        drop(mutation_lease);
        tokio::time::timeout(std::time::Duration::from_secs(2), revoke)
            .await
            .expect("revoke should resume after mutation lease release")
            .expect("revoke task should not panic")
            .expect("revoke policy");

        let mutation_lease = crate::state_db::SecurityPolicyLease::exclusive(app.path())
            .await
            .expect("hold mutation policy lease for paranoid-mode revoke");
        let started = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let mut paranoid_revoke = tokio::spawn({
            let state = std::sync::Arc::clone(&state);
            let started = std::sync::Arc::clone(&started);
            async move {
                started.wait().await;
                settings_set_inner(
                    &state,
                    GeneralSettingsPatch {
                        paranoid_mode: Some(true),
                        ..GeneralSettingsPatch::default()
                    },
                )
                .await
            }
        });
        started.wait().await;

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(500), &mut paranoid_revoke,)
                .await
                .is_err(),
            "paranoid-mode revoke returned before the in-flight mutation lease was released"
        );

        drop(mutation_lease);
        tokio::time::timeout(std::time::Duration::from_secs(2), paranoid_revoke)
            .await
            .expect("paranoid-mode revoke should resume after mutation lease release")
            .expect("paranoid-mode revoke task should not panic")
            .expect("revoke through paranoid mode");
    }

    #[tokio::test]
    async fn posture_apply_rejects_corrupt_settings_without_changing_cache_or_disk() {
        let app = tempfile::tempdir().expect("app data");
        let corrupt = b"{bad json";
        tokio::fs::write(settings_path(app.path()), corrupt)
            .await
            .expect("write corrupt settings");
        let state = test_app_state(
            app.path(),
            SettingsLoadState::Corrupt {
                message: "parse failure".into(),
            },
        );

        assert!(
            security_posture_apply_inner(&state, SecurityPosturePreset::Strict)
                .await
                .is_err()
        );
        assert_eq!(
            tokio::fs::read(settings_path(app.path())).await.unwrap(),
            corrupt
        );
        assert!(matches!(
            &*state.settings.read().await,
            SettingsLoadState::Corrupt { .. }
        ));
    }

    #[tokio::test]
    async fn posture_apply_marks_a_previously_loaded_cache_corrupt_and_fails_closed() {
        let app = tempfile::tempdir().expect("app data");
        let permissive = Settings {
            github_enabled: true,
            mcp_source_access: true,
            ..Settings::default()
        };
        persist(app.path(), permissive.clone()).await.expect("seed");
        let corrupt = b"{bad json";
        tokio::fs::write(settings_path(app.path()), corrupt)
            .await
            .expect("corrupt persisted settings");
        let state = test_app_state(app.path(), SettingsLoadState::Loaded(permissive));

        assert!(
            security_posture_apply_inner(&state, SecurityPosturePreset::Strict)
                .await
                .is_err()
        );
        assert_eq!(
            tokio::fs::read(settings_path(app.path())).await.unwrap(),
            corrupt
        );
        assert!(matches!(
            &*state.settings.read().await,
            SettingsLoadState::Corrupt { .. }
        ));
        assert!(state.require_network("test").await.is_err());
        assert!(state
            .authorize_mcp_client("claude", crate::state::McpAction::Source, None)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn posture_apply_failure_leaves_disk_and_cache_unchanged() {
        let app = tempfile::tempdir().expect("app data");
        let original = Settings {
            github_enabled: true,
            ..Settings::default()
        };
        persist(app.path(), original.clone()).await.expect("seed");
        tokio::fs::create_dir(settings_path(app.path()).with_extension("json.tmp"))
            .await
            .expect("block atomic temp creation");
        let state = test_app_state(app.path(), SettingsLoadState::Loaded(original.clone()));
        let before = tokio::fs::read(settings_path(app.path())).await.unwrap();

        assert!(
            security_posture_apply_inner(&state, SecurityPosturePreset::Strict)
                .await
                .is_err()
        );

        assert_eq!(
            tokio::fs::read(settings_path(app.path())).await.unwrap(),
            before
        );
        assert!(matches!(
            &*state.settings.read().await,
            SettingsLoadState::Loaded(settings) if settings == &original
        ));
    }

    #[tokio::test]
    async fn posture_apply_serializes_with_other_settings_writes() {
        let app = tempfile::tempdir().expect("app data");
        let original = Settings {
            github_enabled: true,
            update_auto_check: true,
            drift_notifications: true,
            mcp_project_allowlist: vec![app.path().to_string_lossy().into_owned()],
            ..Settings::default()
        };
        let original = persist(app.path(), original).await.expect("seed");
        let state = test_app_state(app.path(), SettingsLoadState::Loaded(original));

        let (preset, unrelated) = tokio::join!(
            security_posture_apply_inner(&state, SecurityPosturePreset::Strict),
            settings_set_inner(
                &state,
                GeneralSettingsPatch {
                    ai_features_enabled: Some(false),
                    ..Default::default()
                },
            ),
        );
        preset.expect("apply preset");
        unrelated.expect("save unrelated setting");

        let SettingsLoadState::Loaded(saved) = load_async(app.path()).await else {
            panic!("settings should remain readable")
        };
        assert_eq!(saved.security_posture(), SecurityPosture::Strict);
        assert!(!saved.ai_features_enabled);
        assert_eq!(
            saved.mcp_project_allowlist,
            [std::fs::canonicalize(app.path())
                .unwrap()
                .to_string_lossy()
                .into_owned()]
        );
        assert!(matches!(
            &*state.settings.read().await,
            SettingsLoadState::Loaded(cached) if cached == &saved
        ));
    }

    /// File-absent → defaults apply (paranoid OFF).
    #[tokio::test]
    async fn missing_file_is_first_launch() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = load_at_startup(tmp.path());
        match state {
            SettingsLoadState::FirstLaunch => {}
            other => panic!("expected FirstLaunch, got {other:?}"),
        }
        // Defaults must have paranoid OFF.
        let effective = state
            .effective_settings()
            .expect("first launch has defaults");
        assert!(!effective.paranoid_mode);
    }

    #[tokio::test]
    async fn completed_sqlite_settings_are_authoritative_and_corruption_fails_closed() {
        let root = tempfile::tempdir().unwrap();
        let database = crate::state_db::StateDatabase::open(root.path()).unwrap();
        let stored = Settings {
            paranoid_mode: true,
            ..Settings::default()
        };
        database
            .mutate(settings_spec(), Settings::default(), move |settings| {
                *settings = stored;
                Ok(())
            })
            .await
            .unwrap();
        std::fs::write(
            settings_path(root.path()),
            serde_json::to_vec(&Settings::default()).unwrap(),
        )
        .unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();

        assert!(matches!(
            load_at_startup(root.path()),
            SettingsLoadState::Loaded(Settings {
                paranoid_mode: true,
                ..
            })
        ));

        database
            .set_migration_state(crate::types::StorageMigrationState::Corrupt)
            .await
            .unwrap();
        assert!(matches!(
            load_at_startup(root.path()),
            SettingsLoadState::Corrupt { .. }
        ));
    }

    /// File-corrupt (bad JSON) → fail closed.
    #[tokio::test]
    async fn corrupt_file_fails_closed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = settings_path(tmp.path());
        tokio::fs::write(&path, b"{not valid json").await.unwrap();

        let state = load_at_startup(tmp.path());
        match &state {
            SettingsLoadState::Corrupt { message } => {
                assert!(message.contains("parse"), "{message}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
        // effective_settings must be None — caller must NOT see "paranoid off".
        assert!(state.effective_settings().is_none());
    }

    /// File-oversize → fail closed.
    #[tokio::test]
    async fn oversize_file_fails_closed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = settings_path(tmp.path());
        // Write 1 MiB + 1 byte.
        let payload = vec![b'a'; (MAX_SETTINGS_BYTES + 1) as usize];
        tokio::fs::write(&path, &payload).await.unwrap();

        let state = load_at_startup(tmp.path());
        match &state {
            SettingsLoadState::Corrupt { message } => {
                assert!(message.contains("exceeds"), "{message}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    /// Round-trip: persist + reload returns the same struct.
    #[tokio::test]
    async fn round_trip_persists_all_fields() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let s = Settings {
            paranoid_mode: true,
            catalog_stale_banner_days: 21,
            github_enabled: true,
            ai_features_enabled: false,
            update_auto_check: true,
            drift_notifications: true,
            skipped_update_versions: vec!["0.3.0".into(), "0.3.1".into()],
            tool_paths: HashMap::from([("claudeCode".to_string(), "/wsl/home/me".to_string())]),
            mcp_source_access: true,
            mcp_install_access: true,
            mcp_destructive_access: true,
            mcp_agent_source_access: true,
            mcp_agent_install_access: true,
            mcp_agent_destructive_access: true,
            mcp_project_allowlist: vec!["/projects/allowed".into()],
            mcp_client_policies: HashMap::new(),
        };
        let written = persist(tmp.path(), s.clone()).await.expect("persist");
        assert_eq!(written, s);

        let reloaded = load_async(tmp.path()).await;
        match reloaded {
            SettingsLoadState::Loaded(loaded) => assert_eq!(loaded, s),
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    /// Phase 12c — `github_enabled` must round-trip with the camelCase
    /// JSON key `githubEnabled`. The field is brand-new and we want a
    /// pinning test that the wire shape matches the frontend type.
    #[tokio::test]
    async fn github_enabled_round_trips_with_camel_case_key() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let s = Settings {
            github_enabled: true,
            ..Settings::default()
        };
        persist(tmp.path(), s.clone()).await.expect("persist");

        // Inspect raw JSON on disk for the camelCase key. We don't want a
        // future serde rename to silently shift the wire shape.
        let raw = tokio::fs::read_to_string(settings_path(tmp.path()))
            .await
            .expect("read raw");
        assert!(
            raw.contains("\"githubEnabled\""),
            "expected camelCase key in raw JSON, got: {raw}"
        );
        assert!(
            !raw.contains("\"github_enabled\""),
            "must not emit snake_case key"
        );

        let reloaded = load_async(tmp.path()).await;
        match reloaded {
            SettingsLoadState::Loaded(loaded) => assert!(loaded.github_enabled),
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    /// Out-of-range numerics get clamped on save.
    #[tokio::test]
    async fn clamps_on_save() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let s = Settings {
            paranoid_mode: false,
            catalog_stale_banner_days: 9999, // way above 365
            github_enabled: false,
            ai_features_enabled: true,
            update_auto_check: false,
            drift_notifications: false,
            skipped_update_versions: Vec::new(),
            tool_paths: HashMap::new(),
            mcp_source_access: false,
            mcp_install_access: false,
            mcp_destructive_access: false,
            mcp_agent_source_access: false,
            mcp_agent_install_access: false,
            mcp_agent_destructive_access: false,
            mcp_project_allowlist: Vec::new(),
            mcp_client_policies: HashMap::new(),
        };
        let written = persist(tmp.path(), s).await.expect("persist");
        assert_eq!(
            written.catalog_stale_banner_days,
            Settings::CATALOG_STALE_DAYS_MAX
        );
    }

    /// Out-of-range numerics get clamped on read too (defense against
    /// hand-edited settings.json).
    #[tokio::test]
    async fn clamps_on_load() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = settings_path(tmp.path());
        // Hand-write a settings file with absurd values.
        let raw = br#"{
            "paranoidMode": false,
            "catalogStaleBannerDays": 99999,
            "caskIconMode": "all",
            "trendingTtlMinutes": 2
        }"#;
        tokio::fs::write(&path, raw).await.unwrap();

        let state = load_at_startup(tmp.path());
        match state {
            SettingsLoadState::Loaded(s) => {
                assert_eq!(
                    s.catalog_stale_banner_days,
                    Settings::CATALOG_STALE_DAYS_MAX
                );
            }
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    /// Missing optional fields take their defaults (forward compat).
    #[tokio::test]
    async fn missing_fields_use_defaults() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = settings_path(tmp.path());
        // Only paranoidMode set — everything else absent.
        let raw = br#"{ "paranoidMode": true }"#;
        tokio::fs::write(&path, raw).await.unwrap();

        let state = load_at_startup(tmp.path());
        match state {
            SettingsLoadState::Loaded(s) => {
                assert!(s.paranoid_mode);
                assert_eq!(s.catalog_stale_banner_days, 14);
                // `github_enabled` was added in 12c — must default to false
                // for forward compat with pre-12c settings files.
                assert!(!s.github_enabled);
                // `ai_features_enabled` was added in Phase 13 — must
                // default to true for forward compat with pre-13 settings
                // files (pre-existing installs see categories + enrichment
                // turned on as soon as they upgrade).
                assert!(s.ai_features_enabled);
                // `update_auto_check` was added in Phase 15 — must default
                // to false for forward compat with pre-15 settings files.
                assert!(!s.update_auto_check);
                // Phase 16 native drift alerts are always explicit opt-in.
                assert!(!s.drift_notifications);
                // `skipped_update_versions` was added in Phase 15 — must
                // default to an empty vec.
                assert!(s.skipped_update_versions.is_empty());
            }
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn legacy_settings_load_and_are_pruned_on_rewrite() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = settings_path(tmp.path());
        tokio::fs::write(
            &path,
            br#"{
                "paranoidMode": true,
                "catalogStaleBannerDays": 30,
                "caskIconMode": "installed-only",
                "trendingTtlMinutes": 120,
                "enhancedTrendingEnabled": true,
                "vulnerabilityScanningEnabled": true,
                "liveEnrichmentEnabled": true
            }"#,
        )
        .await
        .expect("write legacy settings");

        let loaded = match load_async(tmp.path()).await {
            SettingsLoadState::Loaded(settings) => settings,
            other => panic!("expected Loaded, got {other:?}"),
        };
        assert!(loaded.paranoid_mode);
        assert_eq!(loaded.catalog_stale_banner_days, 30);

        persist(tmp.path(), loaded).await.expect("rewrite settings");
        let rewritten = tokio::fs::read_to_string(path)
            .await
            .expect("read rewritten settings");
        for legacy_key in [
            "caskIconMode",
            "trendingTtlMinutes",
            "enhancedTrendingEnabled",
            "vulnerabilityScanningEnabled",
            "liveEnrichmentEnabled",
        ] {
            assert!(
                !rewritten.contains(legacy_key),
                "{legacy_key} was resurrected"
            );
        }
    }

    #[test]
    fn mcp_mutation_permissions_default_off_and_allowlist_is_bounded_and_deduped() {
        let mut settings = Settings::default();
        assert!(!settings.mcp_source_access);
        assert!(!settings.mcp_install_access);
        assert!(!settings.mcp_destructive_access);
        assert!(!settings.mcp_agent_source_access);
        assert!(!settings.mcp_agent_install_access);
        assert!(!settings.mcp_agent_destructive_access);
        assert!(settings.mcp_project_allowlist.is_empty());

        let project = tempfile::tempdir().expect("project");
        let canonical = std::fs::canonicalize(project.path())
            .expect("canonical project")
            .to_string_lossy()
            .into_owned();
        settings.mcp_project_allowlist = (0..80)
            .map(|index| format!("{canonical}/project-{index}"))
            .chain(std::iter::once(format!("{canonical}/project-0")))
            .chain(std::iter::once(String::new()))
            .collect();
        settings.clamp();

        assert_eq!(settings.mcp_project_allowlist.len(), 64);
        assert_eq!(
            settings.mcp_project_allowlist[0],
            format!("{canonical}/project-0")
        );
        assert_eq!(
            settings.mcp_project_allowlist[63],
            format!("{canonical}/project-63")
        );
    }

    #[test]
    fn old_skill_mcp_policy_does_not_enable_agent_mutations() {
        let settings: Settings = serde_json::from_value(serde_json::json!({
            "mcpSourceAccess": true,
            "mcpInstallAccess": true,
            "mcpDestructiveAccess": true,
            "mcpClientPolicies": {
                "claude": {
                    "sourceAccess": true,
                    "installAccess": true,
                    "destructiveAccess": true
                }
            }
        }))
        .expect("old settings remain readable");

        assert!(!settings.mcp_agent_source_access);
        assert!(!settings.mcp_agent_install_access);
        assert!(!settings.mcp_agent_destructive_access);
        let claude = settings.mcp_client_policies.get("claude").expect("policy");
        assert!(!claude.agent_source_access);
        assert!(!claude.agent_install_access);
        assert!(!claude.agent_destructive_access);
    }

    // ---------- Phase 15 — skip-list cap + helpers ----------

    /// Push helper adds entries in order until the cap is reached.
    #[test]
    fn push_skipped_version_appends_until_cap() {
        let mut s = Settings::default();
        for i in 0..Settings::SKIPPED_UPDATE_VERSIONS_CAP {
            let changed = s.push_skipped_version(format!("0.3.{i}"));
            assert!(changed, "first-time push of unique version must change");
        }
        assert_eq!(
            s.skipped_update_versions.len(),
            Settings::SKIPPED_UPDATE_VERSIONS_CAP
        );
    }

    /// Phase 15 §Tests #5 — adding the 11th skip evicts the oldest entry.
    /// This is the canonical bound test.
    #[test]
    fn push_skipped_version_evicts_oldest_on_overflow() {
        let mut s = Settings::default();
        // Fill to cap.
        for i in 0..Settings::SKIPPED_UPDATE_VERSIONS_CAP {
            s.push_skipped_version(format!("v{i}"));
        }
        assert_eq!(s.skipped_update_versions[0], "v0");

        // 11th push: oldest (v0) must be gone, newest (vN) must be at tail.
        let new_version = format!("v{}", Settings::SKIPPED_UPDATE_VERSIONS_CAP);
        s.push_skipped_version(new_version.clone());
        assert_eq!(
            s.skipped_update_versions.len(),
            Settings::SKIPPED_UPDATE_VERSIONS_CAP
        );
        assert!(
            !s.skipped_update_versions.contains(&"v0".to_string()),
            "oldest entry v0 should have been evicted"
        );
        assert_eq!(
            s.skipped_update_versions.last(),
            Some(&new_version),
            "newest entry should be at tail"
        );
    }

    /// Re-pushing an existing version moves it to the tail without
    /// growing the list past the cap.
    #[test]
    fn push_skipped_version_dedupes_and_moves_to_tail() {
        let mut s = Settings::default();
        s.push_skipped_version("a".into());
        s.push_skipped_version("b".into());
        s.push_skipped_version("c".into());

        // Re-push "a" — should move to tail, length unchanged.
        let changed = s.push_skipped_version("a".into());
        assert!(changed);
        assert_eq!(s.skipped_update_versions, vec!["b", "c", "a"]);

        // Pushing the current tail again is a no-op.
        let changed = s.push_skipped_version("a".into());
        assert!(!changed);
        assert_eq!(s.skipped_update_versions, vec!["b", "c", "a"]);
    }

    /// Hand-edited settings.json with a too-long skip list gets pruned
    /// on load via clamp().
    #[test]
    fn clamp_prunes_oversized_skip_list() {
        let mut s = Settings::default();
        for i in 0..(Settings::SKIPPED_UPDATE_VERSIONS_CAP * 3) {
            s.skipped_update_versions.push(format!("v{i}"));
        }
        s.clamp();
        assert_eq!(
            s.skipped_update_versions.len(),
            Settings::SKIPPED_UPDATE_VERSIONS_CAP
        );
        // The most-recent half is retained; the oldest two-thirds are dropped.
        assert!(
            !s.skipped_update_versions.contains(&"v0".to_string()),
            "oldest entries should have been dropped"
        );
    }

    /// Phase 15 — wire shape gate. The new fields must round-trip with
    /// camelCase JSON keys (`updateAutoCheck`, `skippedUpdateVersions`)
    /// so the frontend store can rely on the contract.
    #[tokio::test]
    async fn phase15_fields_round_trip_with_camel_case_keys() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let s = Settings {
            update_auto_check: true,
            skipped_update_versions: vec!["1.0.0".into()],
            ..Settings::default()
        };
        persist(tmp.path(), s.clone()).await.expect("persist");

        let raw = tokio::fs::read_to_string(settings_path(tmp.path()))
            .await
            .expect("read raw");
        assert!(
            raw.contains("\"updateAutoCheck\""),
            "expected camelCase updateAutoCheck key in raw JSON, got: {raw}"
        );
        assert!(
            raw.contains("\"skippedUpdateVersions\""),
            "expected camelCase skippedUpdateVersions key in raw JSON, got: {raw}"
        );
        assert!(
            !raw.contains("\"update_auto_check\""),
            "must not emit snake_case key"
        );

        let reloaded = load_async(tmp.path()).await;
        match reloaded {
            SettingsLoadState::Loaded(loaded) => {
                assert!(loaded.update_auto_check);
                assert_eq!(loaded.skipped_update_versions, vec!["1.0.0"]);
            }
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn phase16_drift_notifications_are_opt_in_and_camel_case() {
        assert!(!Settings::default().drift_notifications);
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut settings = Settings::default();
        GeneralSettingsPatch {
            drift_notifications: Some(true),
            ..GeneralSettingsPatch::default()
        }
        .apply(&mut settings);
        persist(tmp.path(), settings).await.expect("persist");

        let raw = tokio::fs::read_to_string(settings_path(tmp.path()))
            .await
            .expect("read raw");
        assert!(raw.contains("\"driftNotifications\": true"), "{raw}");
        assert!(!raw.contains("drift_notifications"), "{raw}");
    }

    /// Phase 13 — `ai_features_enabled` defaults to true.
    #[test]
    fn ai_features_enabled_defaults_to_true() {
        let s = Settings::default();
        assert!(
            s.ai_features_enabled,
            "AI features ON by default per Phase 13 plan"
        );
    }

    /// Phase 13 — `ai_features_enabled` round-trips on the wire as
    /// camelCase `aiFeaturesEnabled`. Pin the wire shape so a future
    /// serde rename doesn't silently break the frontend store.
    #[tokio::test]
    async fn ai_features_enabled_round_trips_with_camel_case_key() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let s = Settings {
            ai_features_enabled: false,
            ..Settings::default()
        };
        persist(tmp.path(), s.clone()).await.expect("persist");

        let raw = tokio::fs::read_to_string(settings_path(tmp.path()))
            .await
            .expect("read raw");
        assert!(
            raw.contains("\"aiFeaturesEnabled\""),
            "expected camelCase key in raw JSON, got: {raw}"
        );
        assert!(
            !raw.contains("\"ai_features_enabled\""),
            "must not emit snake_case key"
        );

        let reloaded = load_async(tmp.path()).await;
        match reloaded {
            SettingsLoadState::Loaded(loaded) => assert!(!loaded.ai_features_enabled),
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    /// Simulate a crash mid-write: write a `.tmp` file then truncate
    /// it. The final settings.json should remain whatever it was
    /// before (or absent), never the partial tmp contents.
    ///
    /// This exercises the atomic-write contract from `util::fs::atomic_write`:
    /// a crash before the `rename` step leaves the data file unchanged.
    #[tokio::test]
    async fn atomic_write_survives_simulated_crash() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = settings_path(tmp.path());

        // 1. Establish a known-good initial state.
        let original = Settings::default();
        persist(tmp.path(), original.clone()).await.expect("seed");

        // 2. Simulate a crash mid-write by manually creating an
        // oversize / truncated .tmp sibling without renaming it. The
        // existence of `.tmp` must not pollute the final file.
        let mut tmp_name = path.as_os_str().to_owned();
        tmp_name.push(".tmp");
        let tmp_sibling = std::path::PathBuf::from(tmp_name);
        tokio::fs::write(&tmp_sibling, b"\x00 partial garbage")
            .await
            .expect("write partial tmp");

        // 3. Read the final file — must still be the original payload.
        let state = load_at_startup(tmp.path());
        match state {
            SettingsLoadState::Loaded(s) => assert_eq!(s, original),
            other => panic!("expected Loaded with original, got {other:?}"),
        }
    }

    /// `settings_reset` overwrites a corrupt file with defaults.
    #[tokio::test]
    async fn reset_repairs_corrupt_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = settings_path(tmp.path());

        // Plant corrupt content.
        tokio::fs::write(&path, b"{ garbage").await.unwrap();
        let state_before = load_at_startup(tmp.path());
        assert!(matches!(state_before, SettingsLoadState::Corrupt { .. }));

        // Write defaults via persist (what settings_reset uses).
        let written = persist(tmp.path(), Settings::default())
            .await
            .expect("reset");
        assert_eq!(written, Settings::default());

        // Reload — must now be Loaded(defaults).
        let state_after = load_at_startup(tmp.path());
        match state_after {
            SettingsLoadState::Loaded(s) => assert_eq!(s, Settings::default()),
            other => panic!("expected Loaded after reset, got {other:?}"),
        }
    }

    /// effective_settings on FirstLaunch returns defaults (paranoid off).
    #[test]
    fn effective_settings_first_launch_returns_defaults() {
        let state = SettingsLoadState::FirstLaunch;
        let s = state
            .effective_settings()
            .expect("first launch yields defaults");
        assert_eq!(s, Settings::default());
        assert!(!s.paranoid_mode);
    }

    /// effective_settings on Corrupt returns None (fail closed signal).
    #[test]
    fn effective_settings_corrupt_returns_none() {
        let state = SettingsLoadState::Corrupt {
            message: "boom".into(),
        };
        assert!(state.effective_settings().is_none());
    }
}
