//! Shared DTOs serialized across the Tauri IPC boundary.
//!
//! Every struct uses `#[serde(rename_all = "camelCase")]` so the
//! TypeScript side matches `src/lib/types.ts` exactly.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum StorageMigrationState {
    Legacy,
    InProgress,
    Complete,
    Corrupt,
    Unsupported,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMigrationStatus {
    pub state: StorageMigrationState,
    pub stage: Option<String>,
    pub detail: Option<String>,
    pub legacy_conflicts: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
pub enum McpClient {
    Claude,
    Codex,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum McpClientState {
    Missing,
    Exact,
    Conflict,
    Unavailable,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpClientStatus {
    pub client: McpClient,
    pub installed: bool,
    pub state: McpClientState,
    pub command: String,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum McpServerScope {
    User,
    Project,
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum McpInventoryValidation {
    Valid,
    Warning,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum McpToolDiscovery {
    Known,
    Declared,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpInventoryServer {
    pub client: McpClient,
    pub name: String,
    pub scope: McpServerScope,
    pub project_path: Option<String>,
    pub transport: String,
    pub endpoint: String,
    pub enabled: bool,
    pub environment_keys: Vec<String>,
    pub tool_names: Vec<String>,
    pub tool_discovery: McpToolDiscovery,
    pub validation: McpInventoryValidation,
    pub warnings: Vec<String>,
    pub blockers: Vec<String>,
    pub trusted_template: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTrustedTemplate {
    pub id: String,
    pub name: String,
    pub clients: Vec<McpClient>,
    pub tool_names: Vec<String>,
    pub automatic_configuration: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpInventoryReport {
    pub servers: Vec<McpInventoryServer>,
    pub trusted_templates: Vec<McpTrustedTemplate>,
    pub issues: Vec<String>,
}

// =========================================================
// Agent Skills — trusted source subsystem
// =========================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SkillSourceKind {
    Local {
        root: String,
    },
    Github {
        repository: String,
        git_ref: Option<String>,
        #[serde(default)]
        resolved_commit: Option<String>,
        subdirectory: Option<String>,
        active_checkout: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillSource {
    pub id: String,
    pub kind: SkillSourceKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillPackageFile {
    pub relative_path: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillTrustedExecutable {
    pub relative_path: String,
    pub sha256: String,
    pub executable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillTrustFingerprint {
    pub tree_hash: String,
    pub executables: Vec<SkillTrustedExecutable>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum SkillType {
    Design,
    Development,
    Testing,
    Devops,
    Security,
    Data,
    Ai,
    Productivity,
    #[default]
    Other,
}

impl SkillType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Design => "design",
            Self::Development => "development",
            Self::Testing => "testing",
            Self::Devops => "devops",
            Self::Security => "security",
            Self::Data => "data",
            Self::Ai => "ai",
            Self::Productivity => "productivity",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillFileContent {
    pub relative_path: String,
    pub mime_type: String,
    pub text: Option<String>,
    pub base64: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SkillValidationCode {
    InvalidMetadata,
    TrustRequired,
    UnsafeEntry,
    Io,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillValidationError {
    pub code: SkillValidationCode,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillPackageResult {
    pub source_id: String,
    pub relative_path: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub skill_type: SkillType,
    pub group: Vec<String>,
    pub tags: Vec<String>,
    pub dependencies: Vec<String>,
    pub recommended_skills: Vec<String>,
    pub version: Option<String>,
    pub channel: String,
    pub changelog: Option<String>,
    pub publisher: Option<String>,
    pub publisher_key: Option<String>,
    pub publisher_verified: bool,
    pub validation_results: Vec<String>,
    pub permissions: Vec<String>,
    pub quality_score: u8,
    pub quality_checks: Vec<String>,
    pub files: Vec<SkillPackageFile>,
    pub trust_fingerprint: Option<SkillTrustFingerprint>,
    pub errors: Vec<SkillValidationError>,
    pub installable: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SkillDraftState {
    Pending,
    Published,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillDraftFile {
    pub relative_path: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillDraft {
    pub id: String,
    pub submitted_at: String,
    pub state: SkillDraftState,
    pub tree_hash: String,
    pub files: Vec<SkillDraftFile>,
    pub validation: SkillPackageResult,
    pub published_source_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillSourceResult {
    pub source: SkillSource,
    pub packages: Vec<SkillPackageResult>,
    pub errors: Vec<SkillValidationError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillFolderAssignment {
    pub source_id: String,
    pub relative_path: String,
    pub folder_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct SkillReference {
    pub source_id: String,
    pub relative_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillRecent {
    pub skill: SkillReference,
    pub viewed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillCollection {
    pub name: String,
    #[serde(default)]
    pub skills: Vec<SkillReference>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillSmartFolderRule {
    pub query: Option<String>,
    pub skill_type: Option<SkillType>,
    pub tag: Option<String>,
    pub source_id: Option<String>,
    pub installable: Option<bool>,
    pub favorite: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillSmartFolder {
    pub name: String,
    pub rule: SkillSmartFolderRule,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillWorkspaceProfile {
    pub name: String,
    #[serde(default)]
    pub folders: Vec<String>,
    #[serde(default)]
    pub collections: Vec<String>,
    pub runtime: Option<String>,
    pub project_path: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SkillUpdatePolicy {
    Notify,
    AutoTrusted,
    Pin,
    ReviewScripts,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillUpdatePolicyRecord {
    pub skill: SkillReference,
    pub policy: SkillUpdatePolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillPublisherTrust {
    pub name: String,
    pub public_key: String,
    pub trusted: bool,
    pub revoked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillPreferredSource {
    pub skill_name: String,
    pub source_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillUsage {
    pub skill: SkillReference,
    pub fetches: u64,
    pub installs: u64,
    pub rejections: u64,
    pub last_used_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "action",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SkillApprovalAction {
    FolderCreate {
        path: String,
    },
    FolderRename {
        path: String,
        new_name: String,
    },
    FolderMove {
        path: String,
        new_parent: Option<String>,
    },
    FolderDelete {
        path: String,
        recursive: bool,
    },
    FolderAssign {
        source_id: String,
        relative_path: String,
        folder_path: Option<String>,
    },
    Install {
        source_id: String,
        relative_path: String,
        runtime: String,
        project_path: Option<String>,
    },
    CollectionDelete {
        name: String,
    },
    SmartFolderDelete {
        name: String,
    },
    ProfileDelete {
        name: String,
    },
    UpdatePolicySet {
        source_id: String,
        relative_path: String,
        policy: SkillUpdatePolicy,
    },
    Rollback {
        source_id: String,
        relative_path: String,
        runtime: String,
        project_path: Option<String>,
        snapshot_path: String,
    },
    PublisherTrustSet {
        name: String,
        public_key: String,
        trusted: bool,
        revoked: bool,
    },
    DraftPublish {
        id: String,
        plan_revision: String,
    },
    BatchCollection {
        collection_name: String,
        operation: String,
        runtime: String,
        project_path: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SkillApprovalState {
    Pending,
    Running,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillApproval {
    pub id: String,
    pub submitted_at: String,
    pub state: SkillApprovalState,
    pub requested_by: String,
    pub request: SkillApprovalAction,
    pub result: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillFolderState {
    #[serde(default)]
    pub folders: Vec<String>,
    #[serde(default)]
    pub assignments: Vec<SkillFolderAssignment>,
    #[serde(default)]
    pub favorites: Vec<SkillReference>,
    #[serde(default)]
    pub recent: Vec<SkillRecent>,
    #[serde(default)]
    pub collections: Vec<SkillCollection>,
    #[serde(default)]
    pub smart_folders: Vec<SkillSmartFolder>,
    #[serde(default)]
    pub profiles: Vec<SkillWorkspaceProfile>,
    #[serde(default)]
    pub update_policies: Vec<SkillUpdatePolicyRecord>,
    #[serde(default)]
    pub publisher_trust: Vec<SkillPublisherTrust>,
    #[serde(default)]
    pub preferred_sources: Vec<SkillPreferredSource>,
    #[serde(default)]
    pub usage: Vec<SkillUsage>,
    #[serde(default)]
    pub approvals: Vec<SkillApproval>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillDestinationPresence {
    pub runtime: String,
    pub scope: String,
    pub project_path: Option<String>,
    pub path: String,
    pub present: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillVersionSnapshot {
    pub path: String,
    pub created_at: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillPlanPackage {
    pub source_id: String,
    pub relative_path: String,
    pub name: String,
    pub dependency: bool,
    pub destination: String,
    pub file_count: u32,
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillMutationPlan {
    pub operation: String,
    pub runtime: String,
    pub project_path: Option<String>,
    pub packages: Vec<SkillPlanPackage>,
    pub warnings: Vec<String>,
    pub blockers: Vec<String>,
    pub rollback_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillBatchResult {
    pub operation: String,
    pub completed: Vec<String>,
    pub rolled_back: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallRecord {
    pub source_id: String,
    pub relative_path: String,
    pub name: String,
    pub runtime: String,
    pub scope: String,
    pub project_path: Option<String>,
    pub dest: String,
    pub source_hash: String,
    pub installed_hash: String,
    pub installed_at: String,
    #[serde(default)]
    pub disabled_path: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum SkillInstallState {
    Current,
    Outdated,
    Modified,
    Missing,
    Foreign,
    Disabled,
    SourceUnavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledSkill {
    pub source_id: String,
    pub relative_path: String,
    pub name: String,
    pub runtime: String,
    pub scope: String,
    pub project_path: Option<String>,
    pub path: String,
    pub state: SkillInstallState,
    pub tracked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpAuditEntry {
    pub id: String,
    pub timestamp: String,
    #[serde(default)]
    pub client: Option<String>,
    pub tool: String,
    pub action: String,
    #[serde(default = "default_mcp_audit_phase")]
    pub phase: String,
    pub success: bool,
    pub project_path: Option<String>,
}

fn default_mcp_audit_phase() -> String {
    "terminal".into()
}

// =========================================================
// Shikigami — corpus subsystem (contracts.md §A)
// =========================================================
//
// Wire format mirrors `src/lib/types.ts`.

// ---------- Tools & scope ----------

/// An AI coding tool we can deploy an agent into, identified by its camelCase
/// string id (e.g. `"claudeCode"`, `"geminiCli"`). The id IS the wire value the
/// TS `Tool` union depends on; the authoritative tool set lives in the embedded
/// JSON registry (`crate::registry`) — adding a tool is adding a JSON file, not
/// a Rust variant. Kept as a type alias so every struct field carrying a tool
/// (`InstalledAgent`, `ToolInfo`, `LoadoutEntry`, …) stays wire-compatible.
pub type Tool = String;

/// Deployment scope. User-global tools write to fixed `~/…` dests;
/// project-scoped tools install into a tracked `project_path`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum Scope {
    User,
    Project,
}

// ---------- Catalog source (where the corpus lives) ----------

/// Where the active agent catalog lives on disk. The whole app reads/writes the
/// resolved root, so this is the one knob that says "be a respectful frontend
/// over the user's clone" vs "manage our own copy." Persisted to
/// `state/catalog.json`. Serialized tagged on `kind` so the TS side is a clean
/// discriminated union.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[derive(Default)]
pub enum CatalogSource {
    /// App-managed copy seeded from the bundled baseline (`<app_data>/corpus`).
    /// The always-works default; never touches anything outside app data.
    #[default]
    Bundled,
    /// A clone the app provisioned and owns (default `~/.agency-agents`). The
    /// app may pull/refresh it; it's shared with the CLI.
    Managed { path: String },
    /// The user's own pre-existing clone. `manage` records whether the user
    /// granted permission to pull it (manage-with-permission); when false we
    /// only ever read from it.
    UserClone { path: String, manage: bool },
}

/// A catalog directory discovered on disk (for the first-run / Settings picker).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogCandidate {
    /// Absolute path to the candidate catalog root.
    pub path: String,
    /// `"managed"` for `~/.agency-agents`, else `"userClone"`.
    pub kind: String,
    /// Whether it's a git checkout (has `.git`) — drives pull strategy.
    pub has_git: bool,
    /// Quick agent count (top-level `.md` across discovered categories).
    pub agent_count: u32,
}

/// Result of `catalog_detect` — what the app found, plus whether `git` is on
/// PATH (so the UI can explain clone vs snapshot provisioning).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogDetection {
    pub git_available: bool,
    /// True when a filesystem scan of common dev roots was performed (the
    /// "Find Agency Agents" button), vs the cheap `~/.agency-agents`-only check.
    pub scanned: bool,
    pub candidates: Vec<CatalogCandidate>,
}

/// Live status of the active catalog — source, git provenance, and freshness.
/// Powers the Settings → Catalog panel ("manage the repo": which commit, how
/// far behind, what GitHub repo). All git fields are `None`/0 for a non-git
/// (bundled snapshot) source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogStatus {
    pub source: CatalogSource,
    /// Catalog root path (None for the bundled, app-data-internal source).
    pub root: Option<String>,
    pub is_git: bool,
    pub branch: Option<String>,
    /// Short commit SHA of HEAD.
    pub commit: Option<String>,
    pub last_commit_subject: Option<String>,
    pub last_commit_date: Option<String>,
    /// Count of uncommitted working-tree changes.
    pub dirty_count: u32,
    /// `origin` remote URL, if a git checkout.
    pub remote_url: Option<String>,
    /// `owner/repo` parsed from the remote (for GitHub repo stats), if it's a
    /// github.com remote.
    pub repo_slug: Option<String>,
    pub version: String,
    pub fetched_at: String,
    pub agent_count: u32,
}

/// Result of checking the active catalog for upstream updates — the "stats on
/// diffs" view. Git sources fetch + compare against the upstream branch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogUpdateCheck {
    pub is_git: bool,
    /// Commits the upstream branch has that we don't (how far behind).
    pub behind: u32,
    /// Commits we have that upstream doesn't (local work).
    pub ahead: u32,
    /// Files that would change on pull.
    pub changed_files: u32,
    /// Human-readable `git diff --stat` of HEAD..upstream.
    pub diffstat: String,
    /// True when already at the upstream tip (git) — no-op pull.
    pub up_to_date: bool,
}

/// One Agent in the durable snapshot of the active built-in catalog.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSnapshotItem {
    pub category: String,
    pub relative_path: String,
    pub source_hash: String,
    pub body_hash: String,
}

/// A deterministic change between two successful active-catalog snapshots.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CatalogChange {
    Added {
        item: CatalogSnapshotItem,
    },
    Updated {
        before: CatalogSnapshotItem,
        after: CatalogSnapshotItem,
    },
    Removed {
        item: CatalogSnapshotItem,
    },
    Renamed {
        before: CatalogSnapshotItem,
        after: CatalogSnapshotItem,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogFeedBatch {
    pub at: String,
    pub changes: Vec<CatalogChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatalogSnapshotProvenance {
    pub(crate) source_key: String,
    pub(crate) revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatalogPendingRefresh {
    pub(crate) source_key: String,
    pub(crate) baseline_revision: String,
    pub(crate) command: String,
    pub(crate) started_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatalogPendingSourceTransition {
    pub(crate) from_source_key: String,
    pub(crate) to_source_key: String,
    pub(crate) started_at: String,
}

/// The single forward-compatible control-center document. Later capabilities
/// extend this document; Task 1 stores only active-catalog feed truth.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct ControlCenterDocument {
    pub(crate) active_catalog_snapshot: Vec<CatalogSnapshotItem>,
    pub(crate) active_catalog_provenance: Option<CatalogSnapshotProvenance>,
    pub(crate) catalog_pending_refresh: Option<CatalogPendingRefresh>,
    pub(crate) catalog_pending_source_transition: Option<CatalogPendingSourceTransition>,
    pub(crate) catalog_feed: Vec<CatalogFeedBatch>,
    pub(crate) catalog_last_success_at: Option<String>,
    pub(crate) catalog_stale: bool,
    pub(crate) catalog_error: Option<String>,
    pub(crate) project_baselines: Vec<ProjectReadinessBaseline>,
    pub(crate) project_subscriptions: Vec<ProjectSubscription>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BaselineRequirement {
    pub id: String,
    pub known: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct BaselineAgentRequirement {
    pub reference: AgentReference,
    pub tool: Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct BaselineSkillRequirement {
    pub reference: SkillReference,
    pub runtime: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectReadinessBaseline {
    pub project_path: String,
    pub label: String,
    #[serde(default)]
    pub agent_requirements: Vec<BaselineAgentRequirement>,
    #[serde(default)]
    pub skill_requirements: Vec<BaselineSkillRequirement>,
    /// Legacy reference-only projection retained for persisted v1 documents.
    #[serde(default)]
    pub agents: Vec<AgentReference>,
    /// Legacy reference-only projection retained for persisted v1 documents.
    #[serde(default)]
    pub skills: Vec<SkillReference>,
    #[serde(default)]
    pub instructions: Vec<BaselineRequirement>,
    #[serde(default)]
    pub mcp_servers: Vec<BaselineRequirement>,
    /// Legacy target-only projection retained for persisted v1 documents.
    #[serde(default)]
    pub tools: Vec<Tool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSubscription {
    pub project_path: String,
    pub last_seen_batch: Option<String>,
    #[serde(default)]
    pub pending_recommendation_ids: Vec<String>,
    pub dismissed_recommendation_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ReadinessRowState {
    Ready,
    NeedsAttention,
    Unavailable,
    Unverifiable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ReadinessCategoryState {
    NotRequired,
    Ready,
    NeedsAttention,
    Unavailable,
    Unverifiable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProjectReadinessOverall {
    NotConfigured,
    Ready,
    NeedsAttention,
    Unavailable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ReadinessCategoryKind {
    AgentRoster,
    Skills,
    Instructions,
    Mcp,
    Tools,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessRow {
    pub id: String,
    pub label: String,
    pub state: ReadinessRowState,
    pub evidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessCategoryReport {
    pub category: ReadinessCategoryKind,
    pub state: ReadinessCategoryState,
    pub rows: Vec<ReadinessRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectReadinessReport {
    pub project_path: String,
    pub overall: ProjectReadinessOverall,
    pub baseline: Option<ProjectReadinessBaseline>,
    pub subscribed: bool,
    pub categories: Vec<ReadinessCategoryReport>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecommendationLifecycle {
    New,
    Pending,
    Superseded,
    Dismissed,
    Blocked,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecommendationChangeKind {
    Added,
    Updated,
    Removed,
    Renamed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecommendationOperation {
    Install,
    Update,
    Informational,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRecommendationTarget {
    pub reference: AgentReference,
    pub tool: Tool,
    pub project_path: String,
    pub operation: RecommendationOperation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRecommendation {
    pub id: String,
    pub project_path: String,
    pub batch_at: String,
    pub lifecycle: RecommendationLifecycle,
    pub summary: String,
    pub change_kind: RecommendationChangeKind,
    pub baseline_reference: AgentReference,
    pub agent_references: Vec<AgentReference>,
    pub targets: Vec<ProjectRecommendationTarget>,
    pub finalize_only: bool,
}

/// Bounded UI projection; the potentially 10,000-item snapshot never crosses
/// the Tauri boundary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogFeedState {
    pub last_success_at: Option<String>,
    pub stale: bool,
    pub error: Option<String>,
    pub batches: Vec<CatalogFeedBatch>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PlaybookKind {
    Strategy,
    Example,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlaybookCatalogEntry {
    pub relative_path: String,
    pub title: String,
    pub kind: PlaybookKind,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlaybookDocument {
    pub relative_path: String,
    pub title: String,
    pub kind: PlaybookKind,
    pub size_bytes: u64,
    pub content: String,
}

// ---------- Agent (parsed from the corpus) ----------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct AgentReference {
    pub source_id: String,
    pub relative_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AgentSourceKind {
    BuiltIn,
    Local {
        root: String,
    },
    Github {
        repository: String,
        git_ref: Option<String>,
        #[serde(default)]
        resolved_commit: Option<String>,
        subdirectory: Option<String>,
        active_checkout: Option<String>,
    },
    Published {
        root: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSource {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub kind: AgentSourceKind,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AgentValidationCode {
    InvalidMetadata,
    InvalidPath,
    DuplicateIdentity,
    UnsafeEntry,
    Oversize,
    Io,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentValidationError {
    pub code: AgentValidationCode,
    pub path: String,
    pub message: String,
}

/// An agent as parsed from a single corpus `.md` file. `body` is the
/// markdown persona and is omitted/empty in list views (`corpus_list`)
/// to keep payloads small; `corpus_get` returns it populated.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    /// Filename without `.md`, e.g. `"frontend-developer"`.
    pub slug: String,
    /// Frontmatter `name`.
    pub name: String,
    /// Frontmatter `description`.
    pub description: String,
    /// Parent directory, e.g. `"engineering"`.
    pub category: String,
    /// Frontmatter `emoji`.
    pub emoji: Option<String>,
    /// Frontmatter `color` (named or hex).
    pub color: Option<String>,
    /// Frontmatter `vibe`.
    pub vibe: Option<String>,
    /// Markdown body (persona) — lazy/optional in list views.
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPackageResult {
    pub reference: AgentReference,
    pub agent: Option<Agent>,
    pub source_hash: String,
    pub frontmatter_hash: String,
    pub body_hash: String,
    pub version: Option<String>,
    pub channel: Option<String>,
    pub changelog: Option<String>,
    pub publisher: Option<String>,
    pub publisher_key: Option<String>,
    pub publisher_verified: bool,
    pub required_agents: Vec<String>,
    #[serde(default)]
    pub required_skills: Vec<String>,
    pub recommended_agents: Vec<String>,
    pub groups: Vec<String>,
    pub tags: Vec<String>,
    pub capabilities: Vec<String>,
    pub permissions: Vec<String>,
    pub quality_score: u8,
    pub quality_checks: Vec<String>,
    pub diagnostics: Vec<AgentValidationError>,
    pub installable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRecommendation {
    pub package: AgentPackageResult,
    pub score: u32,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillRecommendation {
    pub package: SkillPackageResult,
    pub score: u32,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TaskRecommendation {
    Agent {
        package: AgentPackageResult,
        score: u32,
        reasons: Vec<String>,
    },
    Skill {
        package: SkillPackageResult,
        score: u32,
        reasons: Vec<String>,
    },
}

impl TaskRecommendation {
    pub fn score(&self) -> u32 {
        match self {
            Self::Agent { score, .. } | Self::Skill { score, .. } => *score,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSourceResult {
    pub source: AgentSource,
    pub agents: Vec<AgentPackageResult>,
    pub errors: Vec<AgentValidationError>,
    pub revision: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AgentDraftState {
    Pending,
    Published,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDraft {
    pub id: String,
    pub submitted_at: String,
    pub state: AgentDraftState,
    pub relative_path: String,
    pub source_hash: String,
    pub validation: AgentPackageResult,
    pub published_source_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentDraftInput {
    pub relative_path: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentFolderAssignment {
    pub source_id: String,
    pub relative_path: String,
    pub folder_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentRecent {
    pub agent: AgentReference,
    pub viewed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCollection {
    pub name: String,
    #[serde(default)]
    pub agents: Vec<AgentReference>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSmartFolderRule {
    pub query: Option<String>,
    pub division: Option<String>,
    pub source_id: Option<String>,
    pub capability: Option<String>,
    pub lifecycle_state: Option<String>,
    pub installable: Option<bool>,
    pub favorite: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSmartFolder {
    pub name: String,
    pub rule: AgentSmartFolderRule,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentWorkspaceProfile {
    pub name: String,
    #[serde(default)]
    pub folders: Vec<String>,
    #[serde(default)]
    pub collections: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AgentUpdatePolicy {
    Notify,
    AutoTrusted,
    Pin,
    ReviewScripts,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentUpdatePolicyRecord {
    pub agent: AgentReference,
    pub policy: AgentUpdatePolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentPublisherTrust {
    pub name: String,
    pub public_key: String,
    pub trusted: bool,
    pub revoked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentPreferredSource {
    pub agent_name: String,
    pub source_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentUsage {
    pub agent: AgentReference,
    pub fetches: u64,
    pub publishes: u64,
    pub rejections: u64,
    pub last_used_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "action",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AgentApprovalAction {
    SourceRemove {
        source_id: String,
    },
    FolderDelete {
        path: String,
        recursive: bool,
    },
    CollectionDelete {
        name: String,
    },
    SmartFolderDelete {
        name: String,
    },
    ProfileDelete {
        name: String,
    },
    UpdatePolicySet {
        reference: AgentReference,
        policy: AgentUpdatePolicy,
    },
    PublisherTrustSet {
        name: String,
        public_key: String,
        trusted: bool,
        revoked: bool,
    },
    DraftPublish {
        id: String,
        plan_revision: String,
    },
    Install {
        reference: AgentReference,
        tool: Tool,
        project_path: Option<String>,
        include_dependencies: bool,
        plan_revision: String,
    },
    Update {
        reference: AgentReference,
        tool: Tool,
        project_path: Option<String>,
        plan_revision: String,
    },
    Uninstall {
        reference: AgentReference,
        tool: Tool,
        project_path: Option<String>,
        plan_revision: String,
    },
    Rollback {
        reference: AgentReference,
        tool: Tool,
        project_path: Option<String>,
        snapshot_id: String,
        plan_revision: String,
    },
    BatchCollection {
        collection_name: String,
        operation: String,
        tool: Tool,
        project_path: Option<String>,
        plan_revision: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AgentApprovalState {
    Pending,
    Running,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentApproval {
    pub id: String,
    pub submitted_at: String,
    pub state: AgentApprovalState,
    pub requested_by: String,
    pub request: AgentApprovalAction,
    pub result: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentLibraryState {
    #[serde(default)]
    pub folders: Vec<String>,
    #[serde(default)]
    pub assignments: Vec<AgentFolderAssignment>,
    #[serde(default)]
    pub favorites: Vec<AgentReference>,
    #[serde(default)]
    pub recent: Vec<AgentRecent>,
    #[serde(default)]
    pub collections: Vec<AgentCollection>,
    #[serde(default)]
    pub smart_folders: Vec<AgentSmartFolder>,
    #[serde(default)]
    pub profiles: Vec<AgentWorkspaceProfile>,
    #[serde(default)]
    pub update_policies: Vec<AgentUpdatePolicyRecord>,
    #[serde(default)]
    pub publisher_trust: Vec<AgentPublisherTrust>,
    #[serde(default)]
    pub preferred_sources: Vec<AgentPreferredSource>,
    #[serde(default)]
    pub usage: Vec<AgentUsage>,
    #[serde(default)]
    pub approvals: Vec<AgentApproval>,
}

// ---------- Corpus index ----------

/// One row of `corpus-index.json`. The three split hashes let update
/// classification distinguish cosmetic (frontmatter-only) from
/// substantive (body) changes. Hash = SHA-256 lowercase hex of UTF-8
/// bytes (contracts.md §E).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusEntry {
    pub slug: String,
    pub name: String,
    pub category: String,
    pub emoji: Option<String>,
    pub color: Option<String>,
    pub vibe: Option<String>,
    pub description: String,
    /// SHA-256 of the full canonical `.md`.
    pub source_hash: String,
    /// SHA-256 of the frontmatter block.
    pub frontmatter_hash: String,
    /// SHA-256 of the body.
    pub body_hash: String,
}

/// Top-level metadata for the maintained corpus copy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusMeta {
    pub version: String,
    pub commit: Option<String>,
    pub fetched_at: String,
    pub count: u32,
}

// ---------- Install ledger ----------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstallArtifact {
    pub dest: String,
    pub rendered_hash: String,
    #[serde(default)]
    pub disabled_path: Option<String>,
}

/// One row of `installs.json` — the ledger of local install actions.
/// `source_hash` records the corpus version installed from;
/// `rendered_hash` is the SHA-256 of the exact bytes written after
/// per-tool conversion, used by reconciliation to classify state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallRecord {
    pub slug: String,
    /// Stable package source. Empty only while reading a pre-migration ledger.
    #[serde(default)]
    pub source_id: String,
    /// Portable source-relative Agent path. Empty only for a pre-migration row.
    #[serde(default)]
    pub relative_path: String,
    pub tool: Tool,
    pub scope: Scope,
    pub project_path: Option<String>,
    /// Absolute path written.
    pub dest: String,
    pub source_hash: String,
    /// SHA-256 of the agent body at install time. Lets reconciliation label an
    /// available update cosmetic (body unchanged) vs substantive. `#[serde(default)]`
    /// so ledgers written before this field still parse (older rows get "").
    #[serde(default)]
    pub body_hash: String,
    pub rendered_hash: String,
    /// Exact ordered files for multi-artifact tools. Empty means the legacy
    /// single-file tuple (`dest`, `rendered_hash`, `disabled_path`).
    #[serde(default)]
    pub artifacts: Vec<InstallArtifact>,
    /// Same-parent hidden destination while the install is disabled.
    #[serde(default)]
    pub disabled_path: Option<String>,
    /// Exact source bytes selected for the installed version.
    #[serde(default)]
    pub source_snapshot_hash: String,
    /// Verified canonical render used as the common ancestor for drift merges.
    /// Legacy rows remain `None` until the next clean install or update.
    #[serde(default)]
    pub base_snapshot_id: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub publisher_key: Option<String>,
    #[serde(default)]
    pub publisher_verified: bool,
    pub installed_at: String,
    pub corpus_version: String,
    #[serde(default)]
    pub source_revision: String,
    /// Truthful post-install follow-up for tools whose files need external activation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment_notice: Option<String>,
}

/// One exact source member of a project-scoped aggregate roster.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct AgentRosterMember {
    pub reference: AgentReference,
    pub name: String,
    pub source_hash: String,
}

/// Lifecycle truth for Aider/Windsurf's one-file project roster. This is a
/// distinct install-ledger entry, never an Agent install row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentRosterInstallRecord {
    pub tool: Tool,
    pub scope: Scope,
    pub project_path: String,
    pub dest: String,
    pub members: Vec<AgentRosterMember>,
    pub rendered_hash: String,
    #[serde(default)]
    pub disabled_path: Option<String>,
    pub installed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledAgentRoster {
    pub record: AgentRosterInstallRecord,
    pub state: InstallState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentRosterPathObservation {
    pub kind: String,
    pub hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentRosterDestinationObservation {
    pub active: AgentRosterPathObservation,
    pub disabled: AgentRosterPathObservation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentRosterMutationPlan {
    pub revision: String,
    pub operation: String,
    pub tool: Tool,
    pub scope: Scope,
    pub project_path: String,
    pub destination: String,
    pub members: Vec<AgentRosterMember>,
    pub state: Option<InstallState>,
    pub destination_observation: AgentRosterDestinationObservation,
    pub warnings: Vec<String>,
    pub blockers: Vec<String>,
    pub rollback_available: bool,
}

// ---------- Reconciliation ----------

/// The seven reconciliation states (like a package manager's installed /
/// outdated states). See systemPatterns.md §4 for the disk ↔ ledger ↔ corpus
/// test that classifies each on-disk agent file.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum InstallState {
    Current,
    Outdated,
    Modified,
    #[serde(alias = "removed")]
    Missing,
    Foreign,
    Disabled,
    SourceUnavailable,
}

/// Whether an available update is cosmetic (frontmatter/metadata only,
/// `body_hash` unchanged) or substantive (prompt body changed).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum UpdateKind {
    Cosmetic,
    Substantive,
}

/// Reconciled view-model for the Library — one on-disk agent file
/// resolved against the ledger and corpus-index. `update_kind` is
/// `Some(..)` only when `state == Outdated`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledAgent {
    pub slug: String,
    pub name: String,
    pub source_id: String,
    pub relative_path: String,
    pub tool: Tool,
    pub scope: Scope,
    pub project_path: Option<String>,
    pub dest: String,
    pub state: InstallState,
    pub update_kind: Option<UpdateKind>,
    /// True when THIS app installed it (it's in the ledger); false when the
    /// Foreign sweep found it on disk (e.g. a prior `install.sh` run). Lets the
    /// UI distinguish "tracked by the app" from "present from other tools"
    /// instead of claiming every recognized file as "installed by you".
    pub tracked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct AgentInstallIdentity {
    pub reference: AgentReference,
    pub tool: Tool,
    pub scope: Scope,
    pub project_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentVersionSnapshot {
    pub id: String,
    pub created_at: String,
    pub source_hash: String,
    pub rendered_hash: String,
    #[serde(default)]
    pub artifact_hashes: Vec<String>,
    pub content_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roster_record: Option<AgentRosterInstallRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentPlanItem {
    pub reference: AgentReference,
    pub name: String,
    pub source_hash: String,
    pub dependency: bool,
    pub destination: String,
    pub rendered_file_count: u32,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AgentMergeOutcome {
    Clean {
        preview_hash: String,
    },
    Conflicts {
        count: u32,
        hunk_summaries: Vec<String>,
    },
    Unavailable {
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentMergePreview {
    pub preview: String,
    pub preview_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentMutationPlan {
    pub revision: String,
    pub operation: String,
    pub tool: Tool,
    pub scope: Scope,
    pub project_path: Option<String>,
    pub agents: Vec<AgentPlanItem>,
    pub warnings: Vec<String>,
    pub blockers: Vec<String>,
    pub rollback_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_outcome: Option<AgentMergeOutcome>,
}

/// Result of `agent_diff` — what's on disk now vs the canonical render the app
/// would write. Powers "review before Update": the UI can show the user exactly
/// what an Update/Restore would change before any file is touched.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentArtifactDiff {
    pub dest: String,
    pub on_disk: Option<String>,
    pub proposed: String,
    pub differs: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDiff {
    pub slug: String,
    pub tool: Tool,
    pub project_path: Option<String>,
    pub dest: String,
    /// Current on-disk contents (None if the file is missing).
    pub on_disk: Option<String>,
    /// The canonical render the app would write.
    pub proposed: String,
    /// Whether the two differ (false ⇒ Update is a no-op).
    pub differs: bool,
    /// Every physical file in this logical install. The legacy top-level fields
    /// mirror the first artifact for existing clients.
    pub artifacts: Vec<AgentArtifactDiff>,
}

// ---------- Tools / categories / projects ----------

/// View-model for the Tools section — a detected AI tool plus its
/// deployment surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    pub tool: Tool,
    pub label: String,
    pub detected: bool,
    pub scope: Scope,
    pub user_dest: Option<String>,
    pub installed_count: u32,
    /// Per-tool custom install base path the user configured (else `None` =
    /// OS home). Detection + `user_dest` already reflect this base.
    pub custom_path: Option<String>,
}

/// Best-effort detected version string for a tool, from probing `<bin>
/// --version`. `version` is `None` when the binary isn't on PATH, the probe
/// timed out, or the tool has no known version command.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolVersion {
    pub tool: Tool,
    pub version: Option<String>,
}

/// One category for the Discover grid. `slug` is the corpus parent dir
/// (e.g. `"engineering"`); `icon` is a PascalCase Lucide icon name the
/// frontend resolves via its static icon map.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    pub slug: String,
    pub label: String,
    pub icon: String,
    /// Brand color (hex) for the division, from the catalog metadata.
    pub color: String,
    pub count: u32,
}

/// A registered project directory for project-scoped installs. The app
/// keeps a Projects list so Library/Tools can show per-project
/// deployment; one agent in five projects = five tracked rows.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    /// Absolute project root path.
    pub path: String,
    /// Display label (defaults to the directory name).
    pub label: String,
    /// Count of agents installed into this project across all
    /// project-scoped tools.
    pub installed_count: u32,
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- Shikigami: Tool id ----------
    //
    // `Tool` is now a `String` camelCase id sourced from the embedded JSON
    // registry (which self-tests its id set in `crate::registry`). A tool id is
    // a plain string, so the former enum-variant serde tests are meaningless;
    // the wire-value coverage that still matters — that a `tool` field on a DTO
    // serializes as the exact camelCase string — lives in
    // `installed_agent_serializes_camel_case_fields` below.

    #[test]
    fn scope_and_states_serialize_camel_case() {
        assert_eq!(serde_json::to_string(&Scope::User).unwrap(), "\"user\"");
        assert_eq!(
            serde_json::to_string(&Scope::Project).unwrap(),
            "\"project\""
        );
        assert_eq!(
            serde_json::to_string(&InstallState::Foreign).unwrap(),
            "\"foreign\""
        );
        assert_eq!(
            serde_json::to_string(&UpdateKind::Substantive).unwrap(),
            "\"substantive\""
        );
    }

    #[test]
    fn agent_merge_outcomes_serialize_for_tauri_plans() {
        assert_eq!(
            serde_json::to_value(AgentMergeOutcome::Clean {
                preview_hash: "a".repeat(64),
            })
            .unwrap(),
            serde_json::json!({"status": "clean", "previewHash": "a".repeat(64)})
        );
        assert_eq!(
            serde_json::to_value(AgentMergeOutcome::Conflicts {
                count: 1,
                hunk_summaries: vec!["Conflict 1".into()],
            })
            .unwrap(),
            serde_json::json!({
                "status": "conflicts",
                "count": 1,
                "hunkSummaries": ["Conflict 1"]
            })
        );
    }

    #[test]
    fn installed_agent_serializes_camel_case_fields() {
        let a = InstalledAgent {
            slug: "frontend-developer".into(),
            name: "Frontend Developer".into(),
            source_id: "builtin:agency-agents".into(),
            relative_path: "engineering/frontend-developer.md".into(),
            tool: "claudeCode".to_string(),
            scope: Scope::User,
            project_path: None,
            dest: "/Users/x/.claude/agents/frontend-developer.md".into(),
            state: InstallState::Outdated,
            update_kind: Some(UpdateKind::Cosmetic),
            tracked: true,
        };
        let v = serde_json::to_value(&a).unwrap();
        for k in [
            "slug",
            "name",
            "sourceId",
            "relativePath",
            "tool",
            "scope",
            "projectPath",
            "dest",
            "state",
            "updateKind",
        ] {
            assert!(
                v.get(k).is_some(),
                "InstalledAgent must have wire field {:?}",
                k
            );
        }
        for snake in ["project_path", "update_kind"] {
            assert!(
                v.get(snake).is_none(),
                "snake key {:?} must not leak",
                snake
            );
        }
        assert_eq!(v["tool"], "claudeCode");
        assert_eq!(v["state"], "outdated");
        assert_eq!(v["updateKind"], "cosmetic");
    }

    #[test]
    fn corpus_entry_serializes_split_hashes_camel_case() {
        let e = CorpusEntry {
            slug: "code-reviewer".into(),
            name: "Code Reviewer".into(),
            category: "engineering".into(),
            emoji: Some("🔍".into()),
            color: None,
            vibe: None,
            description: "Reviews code.".into(),
            source_hash: "a".repeat(64),
            frontmatter_hash: "b".repeat(64),
            body_hash: "c".repeat(64),
        };
        let v = serde_json::to_value(&e).unwrap();
        for k in ["sourceHash", "frontmatterHash", "bodyHash"] {
            assert!(
                v.get(k).is_some(),
                "CorpusEntry must have wire field {:?}",
                k
            );
        }
        for snake in ["source_hash", "frontmatter_hash", "body_hash"] {
            assert!(
                v.get(snake).is_none(),
                "snake key {:?} must not leak",
                snake
            );
        }
    }

    #[test]
    fn agent_source_identity_serializes_without_slug_coupling() {
        let reference = AgentReference {
            source_id: "source-a".into(),
            relative_path: "engineering/ui.md".into(),
        };
        assert_eq!(
            serde_json::to_value(&reference).unwrap(),
            serde_json::json!({
                "sourceId": "source-a",
                "relativePath": "engineering/ui.md"
            })
        );

        let source = AgentSource {
            id: "source-a".into(),
            label: "Agents".into(),
            enabled: true,
            kind: AgentSourceKind::Github {
                repository: "https://github.com/example/agents.git".into(),
                git_ref: Some("main".into()),
                resolved_commit: Some("a".repeat(40)),
                subdirectory: Some("catalog".into()),
                active_checkout: None,
            },
        };
        let value = serde_json::to_value(&source).unwrap();
        assert_eq!(value["kind"]["kind"], "github");
        assert_eq!(value["kind"]["gitRef"], "main");
        assert_eq!(
            value["kind"]["resolvedCommit"],
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(value["kind"]["subdirectory"], "catalog");
    }

    #[test]
    fn catalog_feed_dto_serializes_bounded_typed_changes_in_camel_case() {
        let before = CatalogSnapshotItem {
            category: "engineering".into(),
            relative_path: "engineering/old.md".into(),
            source_hash: "a".repeat(64),
            body_hash: "b".repeat(64),
        };
        let after = CatalogSnapshotItem {
            relative_path: "engineering/new.md".into(),
            ..before.clone()
        };
        let state = CatalogFeedState {
            last_success_at: Some("2026-08-17T00:00:00Z".into()),
            stale: false,
            error: None,
            batches: vec![CatalogFeedBatch {
                at: "2026-08-17T00:00:00Z".into(),
                changes: vec![CatalogChange::Renamed { before, after }],
            }],
        };

        assert_eq!(
            serde_json::to_value(state).unwrap(),
            serde_json::json!({
                "lastSuccessAt": "2026-08-17T00:00:00Z",
                "stale": false,
                "error": null,
                "batches": [{
                    "at": "2026-08-17T00:00:00Z",
                    "changes": [{
                        "kind": "renamed",
                        "before": {
                            "category": "engineering",
                            "relativePath": "engineering/old.md",
                            "sourceHash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                            "bodyHash": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                        },
                        "after": {
                            "category": "engineering",
                            "relativePath": "engineering/new.md",
                            "sourceHash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                            "bodyHash": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                        }
                    }]
                }]
            })
        );
    }

    #[test]
    fn agent_draft_wire_shape_keeps_validation_and_publication_state() {
        let input = AgentDraftInput {
            relative_path: "engineering/reviewer.md".into(),
            text: "---\nname: Reviewer\ndescription: Reviews code.\n---\n".into(),
        };
        let value = serde_json::to_value(&input).unwrap();
        assert_eq!(value["relativePath"], "engineering/reviewer.md");
        assert!(value.get("text").is_some());
        assert_eq!(
            serde_json::to_value(AgentDraftState::Published).unwrap(),
            "published"
        );
    }
}
