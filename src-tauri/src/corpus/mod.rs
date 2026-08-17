//! Corpus subsystem (Phase 1) — the maintained copy of the agency-agents
//! repo that the whole app reads from.
//!
//! ## Source of truth (systemPatterns.md §1)
//!
//! ```text
//! <app_data_dir>/
//! ├── corpus/                 our maintained copy of the agency-agents repo
//! │   └── <category>/<slug>.md
//! └── state/
//!     └── corpus-index.json   slug → CorpusEntry (hashes, category, version)
//! ```
//!
//! - **Seed**: a baseline corpus ships inside the app bundle
//!   (`resources/corpus-baseline/<category>/<slug>.md`). On first run it is
//!   copied to `<app_data_dir>/corpus/` so the app works offline.
//! - **Refresh** ([`corpus_refresh`]): fetch the GitHub tarball
//!   `https://codeload.github.com/msitarzewski/agency-agents/tar.gz/refs/heads/main`,
//!   extract the category dirs over the working copy, and rebuild
//!   `corpus-index.json`. No runtime git dependency.
//!
//! ## Determinism (contracts.md §E)
//!
//! `corpus-index.json` is keyed by a `BTreeMap` so its serialization has a
//! stable key order. The three per-agent hashes are SHA-256 of canonical
//! byte regions of the source `.md` (see [`parse`]). Nothing in the index
//! carries a timestamp; the only timestamp is [`CorpusMeta::fetched_at`],
//! which lives in a separate meta file, not the index.

pub(crate) mod parse;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::github::extract_github_repo;
use crate::types::{
    Agent, CatalogCandidate, CatalogChange, CatalogDetection, CatalogFeedBatch, CatalogFeedState,
    CatalogPendingRefresh, CatalogPendingSourceTransition, CatalogSnapshotItem,
    CatalogSnapshotProvenance, CatalogSource, CatalogStatus, CatalogUpdateCheck, Category,
    ControlCenterDocument, CorpusEntry, CorpusMeta, PlaybookCatalogEntry, PlaybookDocument,
    PlaybookKind, ProjectReadinessBaseline,
};
use crate::util::fs::atomic_write;

// ---------- Constants ----------

/// The division set for the active catalog = the keys of its `divisions.json`
/// (the canonical division truth the agency-agents repo declares, shared with
/// the CLI installer and the linters). Read the active root's file when present
/// (a clone, or the seeded baseline once it carries one); otherwise fall back to
/// the bundled floor (`agency-categories.json`, itself a mirror of the catalog's
/// `divisions.json`).
///
/// Deriving from `divisions.json` rather than parsing `convert.sh`'s `AGENT_DIRS`
/// fixes a class of drift: a top-level dir that ISN'T a declared division — e.g.
/// `strategy/`, which holds NEXUS playbooks/runbooks with no agent frontmatter —
/// is never surfaced as a division OR scanned as one, and a newly-declared
/// division (e.g. `healthcare`) appears the moment the catalog carries it, with
/// no app-side list to keep in sync. This value doubles as the division list AND
/// the set of directories the indexer scans for agents; both are correct because
/// every agent-bearing dir is a declared division and no non-division dir holds
/// agents (enforced upstream by `check-divisions.sh`'s `NON_DIVISION_DIRS`).
fn discover_categories(root: &Path) -> Vec<String> {
    let meta = std::fs::read_to_string(root.join(DIVISIONS_FILENAME))
        .ok()
        .and_then(|raw| serde_json::from_str::<DivisionsFile>(&raw).ok())
        .map(|f| f.divisions)
        .unwrap_or_else(bundled_division_meta);
    let mut cats: Vec<String> = meta.into_keys().collect();
    cats.sort();
    cats
}

/// Extract the `AGENT_DIRS=( … )` bash array body from a shell script's text.
/// Returns the ordered, de-duplicated directory names, or `None` if the array
/// isn't found. Pure string work so it's unit-testable without the filesystem.
fn parse_agent_dirs(script: &str) -> Option<Vec<String>> {
    let start = script.find("AGENT_DIRS=(")?;
    let after = &script[start + "AGENT_DIRS=(".len()..];
    let end = after.find(')')?;
    let body = &after[..end];

    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for raw_line in body.lines() {
        // Strip an inline comment, then split on whitespace.
        let line = raw_line.split('#').next().unwrap_or("");
        for tok in line.split_whitespace() {
            // Defensive: ignore anything that isn't a plausible dir slug.
            if tok.is_empty()
                || !tok
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                continue;
            }
            if seen.insert(tok.to_string()) {
                out.push(tok.to_string());
            }
        }
    }
    Some(out)
}

/// GitHub `codeload` tarball for the live corpus. Streamed, gunzipped,
/// and unpacked on [`corpus_refresh`]. No git binary required.
const CORPUS_TARBALL_URL: &str =
    "https://codeload.github.com/msitarzewski/agency-agents/tar.gz/refs/heads/main";

/// Git remote used to clone/pull a managed catalog when `git` is available.
const CATALOG_GIT_URL: &str = "https://github.com/msitarzewski/agency-agents.git";

/// Dev-root directory names scanned (under `$HOME`) by the "Find Agency Agents"
/// button when looking for an existing clone.
const SCAN_ROOTS: [&str; 7] = [
    "Software",
    "Projects",
    "git",
    "Developer",
    "code",
    "dev",
    "src",
];

/// User-Agent for the refresh fetch. Mirrors the catalog refresh style.
const USER_AGENT: &str = "agency-agents/0.1 (+https://github.com/msitarzewski/agency-agents)";

/// Whole-request timeout for the tarball fetch. The repo is small (a few
/// hundred small markdown files) so 60s is generous.
const REFRESH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Cap on the raw `tar.gz` response (defends against a hostile mirror).
/// The real tarball is well under 5 MiB; 32 MiB is large headroom.
const MAX_TARBALL_BYTES: u64 = 32 * 1024 * 1024;

/// Cap on a single decompressed agent `.md`. Personas run a few KiB;
/// 1 MiB is absurdly generous and still bounds memory.
pub(crate) const MAX_AGENT_BYTES: u64 = 1024 * 1024;

const MAX_PLAYBOOK_BYTES: u64 = 256 * 1024;
const MAX_PLAYBOOK_DOCUMENTS: usize = 256;
const PLAYBOOK_ROOTS: [&str; 2] = ["strategy", "examples"];

/// Version string recorded for the bundled baseline before any refresh
/// has resolved a commit SHA.
const BASELINE_VERSION: &str = "baseline";

// ---------- On-disk meta ----------

/// `corpus-meta.json` — top-level metadata for the working copy. Distinct
/// from the index (which is per-agent) so [`corpus_status`] can answer
/// "what version / how many / fetched when" with one small read.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredMeta {
    version: String,
    commit: Option<String>,
    fetched_at: String,
    count: u32,
}

impl From<StoredMeta> for CorpusMeta {
    fn from(m: StoredMeta) -> Self {
        CorpusMeta {
            version: m.version,
            commit: m.commit,
            fetched_at: m.fetched_at,
            count: m.count,
        }
    }
}

// ---------- In-memory corpus ----------

/// The parsed, in-memory corpus: every agent plus its index row, ordered
/// deterministically by `(category, slug)`. Memoized on `AppState` so the
/// hot read commands (`corpus_list` / `corpus_get` / `corpus_categories`)
/// never touch disk after the first build.
#[derive(Debug, Clone)]
pub struct Corpus {
    /// Agents in stable `(category, slug)` order. `Agent.body` is fully
    /// populated here; list views clone-and-clear it (see
    /// [`Corpus::list`]).
    agents: Vec<Agent>,
    /// Index rows keyed by slug — `BTreeMap` so the serialized
    /// `corpus-index.json` has stable key order.
    index: BTreeMap<String, CorpusEntry>,
    /// Durable-feed identity and hashes, ordered by `(category, relative path)`.
    active_catalog_snapshot: Vec<CatalogSnapshotItem>,
    /// The category directories this corpus was built from, in tooling order
    /// (from [`discover_categories`]). Drives the Discover grid so the tiles
    /// match the active catalog's actual divisions.
    category_order: Vec<String>,
    /// Division presentation metadata (label / icon / color) keyed by slug,
    /// resolved at build time: the catalog root's `divisions.json` overlaid on
    /// the bundled `agency-categories.json` floor (see [`load_division_meta`]).
    /// Carrying it on the corpus means `categories()` never touches disk and a
    /// catalog that ships a new division presents correctly without an app
    /// update.
    division_meta: BTreeMap<String, CategoryMetaRow>,
    meta: CorpusMeta,
}

impl Corpus {
    /// Number of indexed agents.
    pub fn count(&self) -> u32 {
        self.agents.len() as u32
    }

    /// [`CorpusMeta`] for `corpus_status`.
    pub fn meta(&self) -> CorpusMeta {
        self.meta.clone()
    }

    /// List view — agents (optionally filtered to one `category`) with the
    /// `body` omitted to keep the IPC payload small (contracts.md §C).
    pub fn list(&self, category: Option<&str>) -> Vec<Agent> {
        self.agents
            .iter()
            .filter(|a| category.is_none_or(|c| a.category == c))
            .map(|a| Agent {
                body: String::new(),
                ..a.clone()
            })
            .collect()
    }

    /// Full agent (incl. body) by slug, or `None` if unknown.
    pub fn get(&self, slug: &str) -> Option<Agent> {
        let mut matches = self.agents.iter().filter(|agent| agent.slug == slug);
        let agent = matches.next()?;
        matches.next().is_none().then(|| agent.clone())
    }

    /// Resolve a filename emitted by `convert.sh` back to the catalog's
    /// filename-based identity. Most upstream filenames include a division
    /// prefix while transformed installs use `slugify(frontmatter.name)`.
    pub fn get_by_conversion_slug(&self, slug: &str) -> Option<Agent> {
        let mut matches = self
            .agents
            .iter()
            .filter(|agent| crate::render::slugify(&agent.name) == slug);
        let agent = matches.next()?;
        matches.next().is_none().then(|| agent.clone())
    }

    /// Index row (hashes + category) by slug, for the install/reconcile layer.
    pub fn entry(&self, slug: &str) -> Option<CorpusEntry> {
        self.index.get(slug).cloned()
    }

    /// The active corpus version (from meta), used to stamp ledger records.
    pub fn version(&self) -> String {
        self.meta.version.clone()
    }

    fn catalog_snapshot(&self) -> Vec<CatalogSnapshotItem> {
        self.active_catalog_snapshot.clone()
    }

    /// Per-category counts in tooling order (from [`discover_categories`]).
    /// Label + icon + color come from [`Corpus::division_meta`] — the catalog's
    /// `divisions.json` overlaid on the bundled floor. Categories with zero
    /// agents are still returned so the Discover grid renders the full division
    /// set.
    pub fn categories(&self) -> Vec<Category> {
        let mut counts: BTreeMap<&str, u32> = BTreeMap::new();
        for entry in self.index.values() {
            *counts.entry(entry.category.as_str()).or_default() += 1;
        }
        self.category_order
            .iter()
            .map(|slug| {
                let (label, icon, color) = category_meta_from(&self.division_meta, slug);
                Category {
                    slug: slug.clone(),
                    label,
                    icon,
                    color,
                    count: counts.get(slug.as_str()).copied().unwrap_or(0),
                }
            })
            .collect()
    }

    /// Serialize the index to canonical pretty JSON. Stable key order
    /// (BTreeMap) → byte-identical output for an unchanged corpus.
    fn index_json(&self) -> Result<Vec<u8>, AppError> {
        serde_json::to_vec_pretty(&self.index).map_err(|e| AppError::Internal {
            message: format!("serialize corpus-index.json: {e}"),
        })
    }
}

// ---------- Category metadata ----------

/// The bundled `categories.json` shape we read label + icon from. Only
/// the `categories` map is needed here.
#[derive(Debug, Deserialize)]
struct CategoriesFile {
    categories: BTreeMap<String, CategoryMetaRow>,
}

#[derive(Debug, Clone, Deserialize)]
struct CategoryMetaRow {
    label: String,
    icon: String,
    #[serde(default = "default_division_color")]
    color: String,
}

/// The catalog's `divisions.json` shape (PR #592): the canonical, first-class
/// source for division presentation metadata, shared with the CLI installer +
/// linters. Same row shape as the bundled file, under a `divisions` key.
#[derive(Debug, Deserialize)]
struct DivisionsFile {
    divisions: BTreeMap<String, CategoryMetaRow>,
}

/// Neutral fallback color for a division without one in the metadata.
fn default_division_color() -> String {
    "#94A3B8".to_string()
}

const CATEGORIES_JSON: &str = include_str!("../../data/agency-categories.json");
const DIVISIONS_FILENAME: &str = "divisions.json";

/// The bundled `agency-categories.json` parsed into a slug → row map. This is
/// the floor the app always ships — used directly on first run / for an old
/// clone, and as the base that `divisions.json` overlays onto.
fn bundled_division_meta() -> BTreeMap<String, CategoryMetaRow> {
    serde_json::from_str::<CategoriesFile>(CATEGORIES_JSON)
        .map(|f| f.categories)
        .unwrap_or_default()
}

/// The bundled division slugs (offline default) — the keys of the bundled floor,
/// sorted. Used where the active catalog's own `divisions.json` isn't available
/// to enumerate divisions from (e.g. a tarball with no metadata, or detection).
fn bundled_division_slugs() -> Vec<String> {
    let mut v: Vec<String> = bundled_division_meta().into_keys().collect();
    v.sort();
    v
}

/// Resolve division metadata for the active catalog: start from the bundled
/// floor, then overlay the catalog root's `divisions.json` (PR #592 — the
/// canonical source shared with the CLI installer + linters) when present and
/// parseable. First-run (Bundled) users and pre-#592 clones simply have no
/// `divisions.json`, so they keep the bundled metadata — no drift, no failure.
/// Overlaying (rather than replacing) means a `divisions.json` that omits a
/// division still falls back to the bundled row for it.
fn load_division_meta(catalog_root: &Path) -> BTreeMap<String, CategoryMetaRow> {
    let mut meta = bundled_division_meta();
    let path = catalog_root.join(DIVISIONS_FILENAME);
    match std::fs::read_to_string(&path) {
        Ok(raw) => match serde_json::from_str::<DivisionsFile>(&raw) {
            Ok(file) => {
                for (slug, row) in file.divisions {
                    meta.insert(slug, row);
                }
                tracing::debug!("corpus: division metadata sourced from {}", path.display());
            }
            Err(e) => tracing::warn!(
                "corpus: {} present but unparseable ({e}); using bundled division metadata",
                path.display()
            ),
        },
        // Absent is the common, expected case (first run / old clone) — not a warning.
        Err(_) => tracing::debug!(
            "corpus: no {DIVISIONS_FILENAME} at catalog root; using bundled division metadata"
        ),
    }
    meta
}

/// Resolve `(label, icon, color)` for a category slug from a resolved division
/// metadata map. Falls back to a title-cased slug + a neutral `Folder` icon +
/// a neutral color if the slug is somehow absent (keeps Discover rendering
/// rather than dropping a tile).
fn category_meta_from(
    meta: &BTreeMap<String, CategoryMetaRow>,
    slug: &str,
) -> (String, String, String) {
    match meta.get(slug) {
        Some(row) => (row.label.clone(), row.icon.clone(), row.color.clone()),
        None => (
            title_case(slug),
            "Folder".to_string(),
            default_division_color(),
        ),
    }
}

/// `"game-development"` → `"Game Development"`. Deterministic fallback for
/// the unlikely missing-slug case.
fn title_case(slug: &str) -> String {
    slug.split('-')
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------- Path helpers ----------

/// The working corpus directory: `<app_data_dir>/corpus`. ALWAYS derived
/// from `app_data_dir` — never composed from IPC input.
pub(crate) fn corpus_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("corpus")
}

/// The state directory holding `corpus-index.json` + `corpus-meta.json` and
/// (Phase 2) the install ledger `installs.json`.
pub(crate) fn state_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("state")
}

fn index_path(app_data_dir: &Path) -> PathBuf {
    state_dir(app_data_dir).join("corpus-index.json")
}

fn meta_path(app_data_dir: &Path) -> PathBuf {
    state_dir(app_data_dir).join("corpus-meta.json")
}

fn catalog_source_path(app_data_dir: &Path) -> PathBuf {
    state_dir(app_data_dir).join("catalog.json")
}

fn catalog_source_spec() -> crate::state_db::DocumentSpec<CatalogSource> {
    crate::state_db::DocumentSpec::new("catalog", 1, 65_536, |_| Ok(()))
}

pub(crate) fn catalog_source_import_spec() -> crate::state_db::ImportSpec {
    crate::state_db::ImportSpec::document(catalog_source_spec(), CatalogSource::default())
}

pub(crate) const CONTROL_CENTER_MAX_BYTES: u64 = 4 * 1024 * 1024;
pub(crate) const CONTROL_CENTER_MAX_SNAPSHOT_ITEMS: usize = 10_000;
pub(crate) const CONTROL_CENTER_MAX_FEED_BATCHES: usize = 100;
pub(crate) const CONTROL_CENTER_MAX_FEED_ITEMS: usize = 2_000;
const CONTROL_CENTER_MAX_TEXT_CHARS: usize = 256;
const CONTROL_CENTER_MAX_PATH_CHARS: usize = 512;
const CONTROL_CENTER_MAX_PROJECT_BASELINES: usize = 64;
const CONTROL_CENTER_MAX_PROJECT_AGENTS: usize = 256;
const CONTROL_CENTER_MAX_PROJECT_SKILLS: usize = 256;
const CONTROL_CENTER_MAX_PROJECT_REQUIREMENTS: usize = 32;
const CONTROL_CENTER_MAX_PROJECT_TOOLS: usize = 32;
// A subscription has project identity only and the validator enforces one per
// baseline project, so the honest document maximum is the baseline maximum.
const CONTROL_CENTER_MAX_SUBSCRIPTIONS: usize = CONTROL_CENTER_MAX_PROJECT_BASELINES;
pub(crate) const CONTROL_CENTER_MAX_PENDING_RECOMMENDATIONS: usize = 256;
const CONTROL_CENTER_MAX_DISMISSED_RECOMMENDATIONS: usize = 256;
const CATALOG_SOURCE_TRANSITION_UNAVAILABLE: &str =
    "Catalog source change is incomplete. Retry the source selection.";
const CATALOG_SOURCE_TRANSITION_MISMATCH: &str =
    "Active catalog source matches neither side of the pending source change.";

fn valid_text(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.chars().count() <= CONTROL_CENTER_MAX_TEXT_CHARS
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_snapshot_item(item: &CatalogSnapshotItem) -> Result<(), AppError> {
    let normalized = crate::library::normalize_relative_path(&item.relative_path)?;
    if !valid_text(&item.category)
        || normalized != item.relative_path
        || item.relative_path.chars().count() > CONTROL_CENTER_MAX_PATH_CHARS
        || item.relative_path.split('/').next() != Some(item.category.as_str())
        || !valid_hash(&item.source_hash)
        || !valid_hash(&item.body_hash)
    {
        return Err(AppError::InvalidArgument {
            message: "control-center catalog item is invalid".into(),
        });
    }
    Ok(())
}

fn item_key(item: &CatalogSnapshotItem) -> (&str, &str) {
    (&item.category, &item.relative_path)
}

fn catalog_snapshot_revision(snapshot: &[CatalogSnapshotItem]) -> String {
    let mut digest = Sha256::new();
    for item in snapshot {
        for value in [
            item.category.as_bytes(),
            item.relative_path.as_bytes(),
            item.source_hash.as_bytes(),
            item.body_hash.as_bytes(),
        ] {
            digest.update((value.len() as u64).to_le_bytes());
            digest.update(value);
        }
    }
    hex::encode(digest.finalize())
}

fn catalog_source_key(source: &CatalogSource) -> String {
    let mut digest = Sha256::new();
    match source {
        CatalogSource::Bundled => digest.update(b"bundled"),
        CatalogSource::Managed { path } => {
            digest.update(b"managed");
            digest.update((path.len() as u64).to_le_bytes());
            digest.update(path.as_bytes());
        }
        CatalogSource::UserClone { path, .. } => {
            digest.update(b"user_clone");
            digest.update((path.len() as u64).to_le_bytes());
            digest.update(path.as_bytes());
        }
    }
    hex::encode(digest.finalize())
}

fn change_key(change: &CatalogChange) -> (u8, &str, &str) {
    match change {
        CatalogChange::Added { item } => (0, &item.category, &item.relative_path),
        CatalogChange::Updated { after, .. } => (1, &after.category, &after.relative_path),
        CatalogChange::Removed { item } => (2, &item.category, &item.relative_path),
        CatalogChange::Renamed { after, .. } => (3, &after.category, &after.relative_path),
    }
}

fn validate_change(change: &CatalogChange) -> Result<(), AppError> {
    match change {
        CatalogChange::Added { item } | CatalogChange::Removed { item } => {
            validate_snapshot_item(item)
        }
        CatalogChange::Updated { before, after } => {
            validate_snapshot_item(before)?;
            validate_snapshot_item(after)?;
            if item_key(before) != item_key(after)
                || (before.source_hash == after.source_hash && before.body_hash == after.body_hash)
            {
                return Err(AppError::InvalidArgument {
                    message: "control-center catalog update is invalid".into(),
                });
            }
            Ok(())
        }
        CatalogChange::Renamed { before, after } => {
            validate_snapshot_item(before)?;
            validate_snapshot_item(after)?;
            if item_key(before) == item_key(after) || before.source_hash != after.source_hash {
                return Err(AppError::InvalidArgument {
                    message: "control-center catalog rename is invalid".into(),
                });
            }
            Ok(())
        }
    }
}

fn valid_timestamp(value: &str) -> bool {
    valid_text(value) && chrono::DateTime::parse_from_rfc3339(value).is_ok()
}

fn valid_project_path(value: &str) -> bool {
    let path = Path::new(value);
    !value.chars().any(char::is_control)
        && value.chars().count() <= CONTROL_CENTER_MAX_PATH_CHARS
        && path.is_absolute()
        && !path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
        && path.components().collect::<PathBuf>().as_os_str() == path.as_os_str()
}

fn validate_project_baseline(baseline: &ProjectReadinessBaseline) -> Result<(), AppError> {
    if !valid_project_path(&baseline.project_path)
        || !valid_text(&baseline.label)
        || baseline.agents.len() > CONTROL_CENTER_MAX_PROJECT_AGENTS
        || baseline.skills.len() > CONTROL_CENTER_MAX_PROJECT_SKILLS
        || baseline.agent_requirements.len() > CONTROL_CENTER_MAX_PROJECT_AGENTS
        || baseline.skill_requirements.len() > CONTROL_CENTER_MAX_PROJECT_SKILLS
        || baseline.instructions.len() > CONTROL_CENTER_MAX_PROJECT_REQUIREMENTS
        || baseline.mcp_servers.len() > CONTROL_CENTER_MAX_PROJECT_REQUIREMENTS
        || baseline.tools.len() > CONTROL_CENTER_MAX_PROJECT_TOOLS
    {
        return Err(AppError::InvalidArgument {
            message: "control-center project baseline exceeds its limits".into(),
        });
    }
    let mut exact_agent_keys = BTreeSet::new();
    for requirement in &baseline.agent_requirements {
        crate::library::validate_reference(
            &requirement.reference.source_id,
            &requirement.reference.relative_path,
        )?;
        if requirement.reference.relative_path.chars().count() > CONTROL_CENTER_MAX_PATH_CHARS
            || !valid_text(&requirement.tool)
            || !exact_agent_keys.insert((
                &requirement.reference.source_id,
                &requirement.reference.relative_path,
                &requirement.tool,
            ))
        {
            return Err(AppError::InvalidArgument {
                message: "control-center project baseline has duplicate Agent target requirements"
                    .into(),
            });
        }
    }
    let mut exact_skill_keys = BTreeSet::new();
    for requirement in &baseline.skill_requirements {
        crate::library::validate_reference(
            &requirement.reference.source_id,
            &requirement.reference.relative_path,
        )?;
        if requirement.reference.relative_path.chars().count() > CONTROL_CENTER_MAX_PATH_CHARS
            || !valid_text(&requirement.runtime)
            || !exact_skill_keys.insert((
                &requirement.reference.source_id,
                &requirement.reference.relative_path,
                &requirement.runtime,
            ))
        {
            return Err(AppError::InvalidArgument {
                message: "control-center project baseline has duplicate Skill runtime requirements"
                    .into(),
            });
        }
    }
    let mut agent_keys = BTreeSet::new();
    for reference in &baseline.agents {
        crate::library::validate_reference(&reference.source_id, &reference.relative_path)?;
        if reference.relative_path.chars().count() > CONTROL_CENTER_MAX_PATH_CHARS
            || !agent_keys.insert((&reference.source_id, &reference.relative_path))
        {
            return Err(AppError::InvalidArgument {
                message: "control-center project baseline has duplicate Agent references".into(),
            });
        }
    }
    let mut skill_keys = BTreeSet::new();
    for reference in &baseline.skills {
        crate::library::validate_reference(&reference.source_id, &reference.relative_path)?;
        if reference.relative_path.chars().count() > CONTROL_CENTER_MAX_PATH_CHARS
            || !skill_keys.insert((&reference.source_id, &reference.relative_path))
        {
            return Err(AppError::InvalidArgument {
                message: "control-center project baseline has duplicate Skill references".into(),
            });
        }
    }
    for requirement in baseline.instructions.iter().chain(&baseline.mcp_servers) {
        if !valid_text(&requirement.id) {
            return Err(AppError::InvalidArgument {
                message: "control-center project requirement is invalid".into(),
            });
        }
    }
    if baseline.instructions.iter().any(|requirement| {
        requirement.known
            && !matches!(
                requirement.id.as_str(),
                "agents" | "claude" | "gemini" | "copilot"
            )
    }) || baseline
        .mcp_servers
        .iter()
        .any(|requirement| requirement.known && requirement.id != "agency-agents")
        || baseline
            .tools
            .iter()
            .any(|tool| !valid_text(tool) || crate::registry::get(tool).is_none())
    {
        return Err(AppError::InvalidArgument {
            message: "control-center project tool is invalid".into(),
        });
    }
    Ok(())
}

pub(crate) fn validate_control_center(document: &ControlCenterDocument) -> Result<(), AppError> {
    if document.project_baselines.len() > CONTROL_CENTER_MAX_PROJECT_BASELINES {
        return Err(AppError::InvalidArgument {
            message: "control-center exceeds its project baseline limit".into(),
        });
    }
    if document.project_subscriptions.len() > CONTROL_CENTER_MAX_SUBSCRIPTIONS {
        return Err(AppError::InvalidArgument {
            message: "control-center exceeds its project subscription limit".into(),
        });
    }
    if document.active_catalog_snapshot.len() > CONTROL_CENTER_MAX_SNAPSHOT_ITEMS
        || document.catalog_feed.len() > CONTROL_CENTER_MAX_FEED_BATCHES
        || document
            .catalog_feed
            .iter()
            .map(|batch| batch.changes.len())
            .sum::<usize>()
            > CONTROL_CENTER_MAX_FEED_ITEMS
    {
        return Err(AppError::InvalidArgument {
            message: "control-center catalog state exceeds its item limits".into(),
        });
    }

    let mut baseline_projects = BTreeSet::new();
    for baseline in &document.project_baselines {
        validate_project_baseline(baseline)?;
        if !baseline_projects.insert(baseline.project_path.as_str()) {
            return Err(AppError::InvalidArgument {
                message: "control-center project baselines are not unique".into(),
            });
        }
    }
    let mut subscription_projects = BTreeSet::new();
    let last_success = document
        .catalog_last_success_at
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok());
    for subscription in &document.project_subscriptions {
        if !valid_project_path(&subscription.project_path)
            || !baseline_projects.contains(subscription.project_path.as_str())
            || !subscription_projects.insert(subscription.project_path.as_str())
            || subscription
                .last_seen_batch
                .as_deref()
                .is_some_and(|value| !valid_timestamp(value))
            || subscription
                .last_seen_batch
                .as_deref()
                .is_some_and(|value| {
                    let cursor = chrono::DateTime::parse_from_rfc3339(value).ok();
                    cursor.is_none() || last_success.is_none() || cursor > last_success
                })
            || subscription.dismissed_recommendation_ids.len()
                > CONTROL_CENTER_MAX_DISMISSED_RECOMMENDATIONS
            || subscription.pending_recommendation_ids.len()
                > CONTROL_CENTER_MAX_PENDING_RECOMMENDATIONS
            || subscription
                .pending_recommendation_ids
                .iter()
                .any(|id| !valid_hash(id))
            || subscription
                .pending_recommendation_ids
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != subscription.pending_recommendation_ids.len()
            || subscription
                .dismissed_recommendation_ids
                .iter()
                .any(|id| !valid_hash(id))
        {
            return Err(AppError::InvalidArgument {
                message: "control-center project subscription is invalid".into(),
            });
        }
    }

    for item in &document.active_catalog_snapshot {
        validate_snapshot_item(item)?;
    }
    if document
        .active_catalog_snapshot
        .windows(2)
        .any(|pair| item_key(&pair[0]) >= item_key(&pair[1]))
    {
        return Err(AppError::InvalidArgument {
            message: "control-center catalog snapshot is not uniquely sorted".into(),
        });
    }

    match &document.active_catalog_provenance {
        Some(provenance)
            if valid_hash(&provenance.source_key)
                && valid_hash(&provenance.revision)
                && provenance.revision
                    == catalog_snapshot_revision(&document.active_catalog_snapshot) => {}
        // Documents written before provenance was introduced remain readable;
        // the next pull establishes a silent same-source baseline before it
        // mutates catalog bytes.
        None => {}
        _ => {
            return Err(AppError::InvalidArgument {
                message: "control-center catalog snapshot provenance is invalid".into(),
            });
        }
    }
    if let Some(pending) = &document.catalog_pending_refresh {
        let Some(provenance) = &document.active_catalog_provenance else {
            return Err(AppError::InvalidArgument {
                message: "control-center pending catalog refresh has no baseline".into(),
            });
        };
        if !valid_hash(&pending.source_key)
            || !valid_hash(&pending.baseline_revision)
            || !matches!(pending.command.as_str(), "corpus_refresh" | "catalog_pull")
            || !valid_timestamp(&pending.started_at)
            || pending.source_key != provenance.source_key
            || pending.baseline_revision != provenance.revision
            || !document.catalog_stale
        {
            return Err(AppError::InvalidArgument {
                message: "control-center pending catalog refresh is invalid".into(),
            });
        }
    }
    if let Some(pending) = &document.catalog_pending_source_transition {
        let Some(provenance) = &document.active_catalog_provenance else {
            return Err(AppError::InvalidArgument {
                message: "control-center pending catalog source change has no baseline".into(),
            });
        };
        if !valid_hash(&pending.from_source_key)
            || !valid_hash(&pending.to_source_key)
            || pending.from_source_key == pending.to_source_key
            || pending.from_source_key != provenance.source_key
            || !valid_timestamp(&pending.started_at)
            || document.catalog_pending_refresh.is_some()
            || !document.catalog_stale
        {
            return Err(AppError::InvalidArgument {
                message: "control-center pending catalog source change is invalid".into(),
            });
        }
    }

    let mut previous_at: Option<chrono::DateTime<chrono::FixedOffset>> = None;
    for batch in &document.catalog_feed {
        if !valid_timestamp(&batch.at) {
            return Err(AppError::InvalidArgument {
                message: "control-center catalog feed timestamp is invalid".into(),
            });
        }
        let at = chrono::DateTime::parse_from_rfc3339(&batch.at).map_err(|_| {
            AppError::InvalidArgument {
                message: "control-center catalog feed timestamp is invalid".into(),
            }
        })?;
        if previous_at.is_some_and(|previous| previous > at) {
            return Err(AppError::InvalidArgument {
                message: "control-center catalog feed is not chronological".into(),
            });
        }
        previous_at = Some(at);
        for change in &batch.changes {
            validate_change(change)?;
        }
        if batch
            .changes
            .windows(2)
            .any(|pair| change_key(&pair[0]) >= change_key(&pair[1]))
        {
            return Err(AppError::InvalidArgument {
                message: "control-center catalog changes are not uniquely sorted".into(),
            });
        }
    }

    if document
        .catalog_last_success_at
        .as_deref()
        .is_some_and(|value| !valid_timestamp(value))
        || document.catalog_feed.last().is_some_and(|batch| {
            document.catalog_last_success_at.as_deref() != Some(batch.at.as_str())
        })
        || document
            .catalog_error
            .as_deref()
            .is_some_and(|value| !valid_text(value))
        || (!document.catalog_stale && document.catalog_error.is_some())
    {
        return Err(AppError::InvalidArgument {
            message: "control-center catalog refresh state is invalid".into(),
        });
    }
    Ok(())
}

pub(crate) fn control_center_spec() -> crate::state_db::DocumentSpec<ControlCenterDocument> {
    crate::state_db::DocumentSpec::new(
        "control_center",
        1,
        CONTROL_CENTER_MAX_BYTES,
        validate_control_center,
    )
}

pub(crate) fn control_center_import_spec() -> crate::state_db::ImportSpec {
    crate::state_db::ImportSpec::document(control_center_spec(), ControlCenterDocument::default())
}

fn control_center_inventory_document() -> crate::state::PersistenceDocument {
    *crate::state::PERSISTENCE_INVENTORY
        .iter()
        .find(|document| document.name == "control_center")
        .expect("control_center persistence inventory entry")
}

pub(crate) async fn register_control_center_document(
    database: &crate::state_db::StateDatabase,
) -> Result<(), AppError> {
    database
        .register_completed_document(
            control_center_inventory_document(),
            control_center_spec(),
            ControlCenterDocument::default(),
        )
        .await
}

#[cfg(not(test))]
pub(crate) fn register_control_center_document_blocking(
    database: &crate::state_db::StateDatabase,
) -> Result<(), AppError> {
    database.register_completed_document_blocking(
        control_center_inventory_document(),
        control_center_spec(),
        ControlCenterDocument::default(),
    )
}

fn diff_catalog_snapshots(
    old: &[CatalogSnapshotItem],
    new: &[CatalogSnapshotItem],
) -> Vec<CatalogChange> {
    let old_by_key = old
        .iter()
        .map(|item| (item_key(item), item))
        .collect::<BTreeMap<_, _>>();
    let new_by_key = new
        .iter()
        .map(|item| (item_key(item), item))
        .collect::<BTreeMap<_, _>>();

    let mut removed = old_by_key
        .iter()
        .filter(|(key, _)| !new_by_key.contains_key(*key))
        .map(|(_, item)| (*item).clone())
        .collect::<Vec<_>>();
    let mut added = new_by_key
        .iter()
        .filter(|(key, _)| !old_by_key.contains_key(*key))
        .map(|(_, item)| (*item).clone())
        .collect::<Vec<_>>();
    let mut changes = old_by_key
        .iter()
        .filter_map(|(key, before)| {
            let after = new_by_key.get(key)?;
            (before.source_hash != after.source_hash || before.body_hash != after.body_hash).then(
                || CatalogChange::Updated {
                    before: (*before).clone(),
                    after: (*after).clone(),
                },
            )
        })
        .collect::<Vec<_>>();

    let rename_candidates = removed
        .iter()
        .enumerate()
        .flat_map(|(removed_index, before)| {
            added
                .iter()
                .enumerate()
                .filter(move |(_, after)| before.source_hash == after.source_hash)
                .map(move |(added_index, _)| (removed_index, added_index))
        })
        .collect::<Vec<_>>();
    if let [(removed_index, added_index)] = rename_candidates.as_slice() {
        let after = added.remove(*added_index);
        let before = removed.remove(*removed_index);
        changes.push(CatalogChange::Renamed { before, after });
    }
    changes.extend(added.into_iter().map(|item| CatalogChange::Added { item }));
    changes.extend(
        removed
            .into_iter()
            .map(|item| CatalogChange::Removed { item }),
    );
    changes.sort_by(|left, right| {
        let left = change_key(left);
        let right = change_key(right);
        left.cmp(&right)
    });
    changes
}

pub(crate) async fn load_control_center(
    database: &crate::state_db::StateDatabase,
) -> Result<ControlCenterDocument, AppError> {
    Ok(database
        .read(control_center_spec())
        .await?
        .unwrap_or_default())
}

async fn persist_catalog_refresh(
    database: &crate::state_db::StateDatabase,
    source_key: String,
    snapshot: Vec<CatalogSnapshotItem>,
    at: String,
) -> Result<(), AppError> {
    database
        .mutate(
            control_center_spec(),
            ControlCenterDocument::default(),
            move |document| {
                let Some(provenance) = document.active_catalog_provenance.as_ref() else {
                    return Err(AppError::InvalidArgument {
                        message: "catalog refresh has no durable baseline".into(),
                    });
                };
                let Some(pending) = document.catalog_pending_refresh.as_ref() else {
                    return Err(AppError::InvalidArgument {
                        message: "catalog refresh was not durably prepared".into(),
                    });
                };
                if provenance.source_key != source_key
                    || pending.source_key != source_key
                    || pending.baseline_revision != provenance.revision
                {
                    return Err(AppError::InvalidArgument {
                        message: "catalog source changed before its refresh was committed".into(),
                    });
                }
                let changes = diff_catalog_snapshots(&document.active_catalog_snapshot, &snapshot);
                if changes.len() > CONTROL_CENTER_MAX_FEED_ITEMS {
                    return Err(AppError::InvalidArgument {
                        message: "catalog refresh has too many changes for the durable feed".into(),
                    });
                }
                document.catalog_feed.push(CatalogFeedBatch {
                    at: at.clone(),
                    changes,
                });
                while document.catalog_feed.len() > CONTROL_CENTER_MAX_FEED_BATCHES
                    || document
                        .catalog_feed
                        .iter()
                        .map(|batch| batch.changes.len())
                        .sum::<usize>()
                        > CONTROL_CENTER_MAX_FEED_ITEMS
                {
                    document.catalog_feed.remove(0);
                }
                document.active_catalog_snapshot = snapshot;
                document.active_catalog_provenance = Some(CatalogSnapshotProvenance {
                    source_key,
                    revision: catalog_snapshot_revision(&document.active_catalog_snapshot),
                });
                document.catalog_last_success_at = Some(at);
                document.catalog_pending_refresh = None;
                document.catalog_stale = false;
                document.catalog_error = None;
                Ok(())
            },
        )
        .await
}

async fn begin_catalog_refresh(
    database: &crate::state_db::StateDatabase,
    source_key: String,
    entrypoint: CatalogRefreshEntrypoint,
    started_at: String,
) -> Result<(), AppError> {
    database
        .mutate(
            control_center_spec(),
            ControlCenterDocument::default(),
            move |document| {
                if document.catalog_pending_source_transition.is_some() {
                    return Err(AppError::InvalidArgument {
                        message: CATALOG_SOURCE_TRANSITION_UNAVAILABLE.into(),
                    });
                }
                let provenance = document.active_catalog_provenance.as_ref().ok_or_else(|| {
                    AppError::InvalidArgument {
                        message: "catalog refresh has no durable baseline".into(),
                    }
                })?;
                if provenance.source_key != source_key {
                    return Err(AppError::InvalidArgument {
                        message: "catalog source changed before refresh preparation".into(),
                    });
                }
                match document.catalog_pending_refresh.as_ref() {
                    Some(pending)
                        if pending.source_key != source_key
                            || pending.baseline_revision != provenance.revision =>
                    {
                        return Err(AppError::InvalidArgument {
                            message: "pending catalog refresh belongs to another baseline".into(),
                        });
                    }
                    Some(_) => {}
                    None => {
                        document.catalog_pending_refresh = Some(CatalogPendingRefresh {
                            source_key,
                            baseline_revision: provenance.revision.clone(),
                            command: entrypoint.feature().into(),
                            started_at,
                        });
                    }
                }
                document.catalog_stale = true;
                document.catalog_error = None;
                Ok(())
            },
        )
        .await
}

#[cfg(test)]
async fn persist_catalog_baseline(
    database: &crate::state_db::StateDatabase,
    source_key: String,
    snapshot: Vec<CatalogSnapshotItem>,
) -> Result<(), AppError> {
    write_catalog_baseline(database, source_key, snapshot, true).await
}

async fn write_catalog_baseline(
    database: &crate::state_db::StateDatabase,
    source_key: String,
    snapshot: Vec<CatalogSnapshotItem>,
    clear_history: bool,
) -> Result<(), AppError> {
    database
        .mutate(
            control_center_spec(),
            ControlCenterDocument::default(),
            move |document| {
                document.active_catalog_snapshot = snapshot;
                document.active_catalog_provenance = Some(CatalogSnapshotProvenance {
                    source_key,
                    revision: catalog_snapshot_revision(&document.active_catalog_snapshot),
                });
                document.catalog_pending_refresh = None;
                if clear_history {
                    document.catalog_feed.clear();
                    document.catalog_last_success_at = None;
                    for subscription in &mut document.project_subscriptions {
                        subscription.last_seen_batch = None;
                        subscription.pending_recommendation_ids.clear();
                        subscription.dismissed_recommendation_ids.clear();
                    }
                }
                document.catalog_stale = false;
                document.catalog_error = None;
                Ok(())
            },
        )
        .await
}

async fn ensure_catalog_baseline(
    database: &crate::state_db::StateDatabase,
    source_key: String,
    snapshot: Vec<CatalogSnapshotItem>,
) -> Result<(), AppError> {
    let document = load_control_center(database).await?;
    if document.catalog_pending_source_transition.is_some() {
        return Err(AppError::InvalidArgument {
            message: CATALOG_SOURCE_TRANSITION_UNAVAILABLE.into(),
        });
    }
    let revision = catalog_snapshot_revision(&snapshot);
    let (needs_baseline, clear_history) = match document.active_catalog_provenance.as_ref() {
        None => (true, true),
        Some(provenance) if provenance.source_key != source_key => (true, true),
        Some(provenance) => (
            provenance.revision != revision && document.catalog_pending_refresh.is_none(),
            false,
        ),
    };
    if needs_baseline {
        write_catalog_baseline(database, source_key, snapshot, clear_history).await?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CatalogSourceSelection {
    Preserve,
    Transition,
    Restore,
}

async fn prepare_catalog_source_selection(
    database: &crate::state_db::StateDatabase,
    target_source_key: String,
    started_at: String,
) -> Result<CatalogSourceSelection, AppError> {
    let document = load_control_center(database).await?;
    let provenance =
        document
            .active_catalog_provenance
            .as_ref()
            .ok_or_else(|| AppError::InvalidArgument {
                message: "catalog source change has no durable baseline".into(),
            })?;
    if let Some(pending) = document.catalog_pending_source_transition.as_ref() {
        if target_source_key == pending.from_source_key {
            return Ok(CatalogSourceSelection::Restore);
        }
        if target_source_key == pending.to_source_key {
            return Ok(CatalogSourceSelection::Transition);
        }
    } else {
        if target_source_key == provenance.source_key {
            return Ok(CatalogSourceSelection::Preserve);
        }
        if document.catalog_pending_refresh.is_some() {
            return Err(AppError::InvalidArgument {
                message: "retry the pending catalog refresh before changing sources".into(),
            });
        }
    }

    database
        .mutate(
            control_center_spec(),
            ControlCenterDocument::default(),
            move |document| {
                let provenance = document.active_catalog_provenance.as_ref().ok_or_else(|| {
                    AppError::InvalidArgument {
                        message: "catalog source change has no durable baseline".into(),
                    }
                })?;
                if document.catalog_pending_refresh.is_some() {
                    return Err(AppError::InvalidArgument {
                        message: "retry the pending catalog refresh before changing sources".into(),
                    });
                }
                let from_source_key = document
                    .catalog_pending_source_transition
                    .as_ref()
                    .map(|pending| pending.from_source_key.clone())
                    .unwrap_or_else(|| provenance.source_key.clone());
                if from_source_key == target_source_key {
                    return Err(AppError::InvalidArgument {
                        message: "catalog source transition target is invalid".into(),
                    });
                }
                document.catalog_pending_source_transition = Some(CatalogPendingSourceTransition {
                    from_source_key,
                    to_source_key: target_source_key,
                    started_at,
                });
                document.catalog_stale = true;
                document.catalog_error = Some(CATALOG_SOURCE_TRANSITION_UNAVAILABLE.into());
                Ok(())
            },
        )
        .await?;
    Ok(CatalogSourceSelection::Transition)
}

async fn finish_catalog_source_selection(
    database: &crate::state_db::StateDatabase,
    target_source_key: String,
    snapshot: Vec<CatalogSnapshotItem>,
    mode: CatalogSourceSelection,
) -> Result<(), AppError> {
    database
        .mutate(
            control_center_spec(),
            ControlCenterDocument::default(),
            move |document| {
                let provenance = document.active_catalog_provenance.as_ref().ok_or_else(|| {
                    AppError::InvalidArgument {
                        message: "catalog source change has no durable baseline".into(),
                    }
                })?;
                match mode {
                    CatalogSourceSelection::Preserve => {
                        if document.catalog_pending_source_transition.is_some()
                            || provenance.source_key != target_source_key
                        {
                            return Err(AppError::InvalidArgument {
                                message: "catalog source changed during rebuild".into(),
                            });
                        }
                        if document.catalog_pending_refresh.is_none() {
                            document.active_catalog_snapshot = snapshot;
                            document.active_catalog_provenance = Some(CatalogSnapshotProvenance {
                                source_key: target_source_key,
                                revision: catalog_snapshot_revision(
                                    &document.active_catalog_snapshot,
                                ),
                            });
                        }
                    }
                    CatalogSourceSelection::Transition => {
                        let pending = document
                            .catalog_pending_source_transition
                            .as_ref()
                            .ok_or_else(|| AppError::InvalidArgument {
                                message: "catalog source transition is not pending".into(),
                            })?;
                        if pending.to_source_key != target_source_key {
                            return Err(AppError::InvalidArgument {
                                message: "catalog source changed during rebuild".into(),
                            });
                        }
                        document.active_catalog_snapshot = snapshot;
                        document.active_catalog_provenance = Some(CatalogSnapshotProvenance {
                            source_key: target_source_key,
                            revision: catalog_snapshot_revision(&document.active_catalog_snapshot),
                        });
                        document.catalog_feed.clear();
                        document.catalog_last_success_at = None;
                        for subscription in &mut document.project_subscriptions {
                            subscription.last_seen_batch = None;
                            subscription.pending_recommendation_ids.clear();
                            subscription.dismissed_recommendation_ids.clear();
                        }
                        document.catalog_pending_source_transition = None;
                        document.catalog_stale = false;
                        document.catalog_error = None;
                    }
                    CatalogSourceSelection::Restore => {
                        let pending = document
                            .catalog_pending_source_transition
                            .as_ref()
                            .ok_or_else(|| AppError::InvalidArgument {
                                message: "catalog source transition is not pending".into(),
                            })?;
                        if pending.from_source_key != target_source_key {
                            return Err(AppError::InvalidArgument {
                                message: "catalog source changed during rebuild".into(),
                            });
                        }
                        document.active_catalog_snapshot = snapshot;
                        document.active_catalog_provenance = Some(CatalogSnapshotProvenance {
                            source_key: target_source_key,
                            revision: catalog_snapshot_revision(&document.active_catalog_snapshot),
                        });
                        document.catalog_pending_source_transition = None;
                        document.catalog_stale = false;
                        document.catalog_error = None;
                    }
                }
                Ok(())
            },
        )
        .await
}

async fn mark_catalog_feed_stale(
    database: &crate::state_db::StateDatabase,
    error: &str,
) -> Result<(), AppError> {
    let error = error
        .trim()
        .chars()
        .take(CONTROL_CENTER_MAX_TEXT_CHARS)
        .collect::<String>();
    let error = if error.is_empty() {
        "Catalog refresh failed".to_string()
    } else {
        error
    };
    database
        .mutate_quiet(
            control_center_spec(),
            ControlCenterDocument::default(),
            move |document| {
                document.catalog_stale = true;
                document.catalog_error = Some(error);
                Ok(())
            },
        )
        .await
}

async fn catalog_feed_state_for_source(
    database: &crate::state_db::StateDatabase,
    active_source_key: &str,
) -> Result<CatalogFeedState, AppError> {
    let document = load_control_center(database).await?;
    let mismatched = document.catalog_pending_source_transition.is_some()
        || document
            .active_catalog_provenance
            .as_ref()
            .is_some_and(|provenance| provenance.source_key != active_source_key)
        || (document.active_catalog_provenance.is_none() && !document.catalog_feed.is_empty());
    if mismatched {
        return Ok(CatalogFeedState {
            last_success_at: None,
            stale: true,
            error: Some(CATALOG_SOURCE_TRANSITION_UNAVAILABLE.into()),
            batches: Vec::new(),
        });
    }
    Ok(CatalogFeedState {
        last_success_at: document.catalog_last_success_at,
        stale: document.catalog_stale,
        error: document.catalog_error,
        batches: document.catalog_feed,
    })
}

// ---------- Catalog source (where the corpus content lives) ----------

/// Load the persisted [`CatalogSource`], or [`CatalogSource::Bundled`] when no
/// choice has been made yet / the file is unreadable. The catalog SOURCE
/// (content location) is distinct from the STATE dir (index/meta/ledger/backups
/// always live under app data, regardless of source).
pub(crate) async fn load_catalog_source(app_data_dir: &Path) -> CatalogSource {
    if let Ok(Some(database)) = crate::state_db::StateDatabase::completed(app_data_dir).await {
        return database
            .read(catalog_source_spec())
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
    }
    let path = catalog_source_path(app_data_dir);
    match tokio::fs::read(&path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => CatalogSource::default(),
    }
}

/// Strict passive variant for diagnostics. Unlike the normal app loader, a
/// corrupt persisted value is evidence of an unavailable authority, not a
/// reason to silently substitute the bundled default.
pub(crate) async fn load_catalog_source_checked(
    app_data_dir: &Path,
) -> Result<CatalogSource, AppError> {
    if let Some(database) = crate::state_db::StateDatabase::completed(app_data_dir).await? {
        return database.read(catalog_source_spec()).await?.ok_or_else(|| {
            AppError::StorageCorrupt {
                message: "catalog source is missing after SQLite migration".into(),
            }
        });
    }
    let path = catalog_source_path(app_data_dir);
    match tokio::fs::read(&path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| AppError::JsonParse {
            command: "doctor_report".into(),
            message: error.to_string(),
            raw_excerpt: String::new(),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(CatalogSource::default()),
        Err(error) => Err(AppError::Io {
            message: format!("read catalog source: {error}"),
        }),
    }
}

/// Persist the chosen [`CatalogSource`] to `state/catalog.json`.
pub(crate) async fn save_catalog_source(
    app_data_dir: &Path,
    source: &CatalogSource,
) -> Result<(), AppError> {
    if let Some(database) = crate::state_db::StateDatabase::completed(app_data_dir).await? {
        let source = source.clone();
        return database
            .mutate(
                catalog_source_spec(),
                CatalogSource::default(),
                move |current| {
                    *current = source;
                    Ok(())
                },
            )
            .await;
    }
    let sdir = state_dir(app_data_dir);
    tokio::fs::create_dir_all(&sdir)
        .await
        .map_err(|e| AppError::Io {
            message: format!("create state dir {}: {e}", sdir.display()),
        })?;
    let bytes = serde_json::to_vec_pretty(source).map_err(|e| AppError::Internal {
        message: format!("serialize catalog.json: {e}"),
    })?;
    atomic_write(&catalog_source_path(app_data_dir), &bytes).await
}

/// Resolve the active catalog ROOT directory (where `<category>/<slug>.md` and
/// `scripts/convert.sh` live) for a source. `Bundled` lives inside app data;
/// `Managed`/`UserClone` point at a clone elsewhere on disk.
pub(crate) fn catalog_root(app_data_dir: &Path, source: &CatalogSource) -> PathBuf {
    match source {
        CatalogSource::Bundled => corpus_dir(app_data_dir),
        CatalogSource::Managed { path } => PathBuf::from(path),
        CatalogSource::UserClone { path, .. } => PathBuf::from(path),
    }
}

// ---------- Build / load ----------

/// Resolve the active corpus for the current process:
///
/// 1. Seed the working copy from the bundled baseline if `corpus/` is
///    empty (first run).
/// 2. Parse + index everything under `corpus/`.
/// 3. Write `corpus-index.json` + `corpus-meta.json` if they are missing
///    or stale (so reconciliation has the index on disk too).
///
/// `baseline_dir` is the bundled baseline resolved from the Tauri
/// resource dir (`resource_dir()/resources/corpus-baseline`). `Never`
/// panics: a fully empty or unreadable corpus yields an empty [`Corpus`]
/// with `count == 0` so the UI degrades to "no agents" rather than
/// failing to launch.
pub async fn resolve_active(app_data_dir: &Path, baseline_dir: &Path) -> Corpus {
    let source = load_catalog_source(app_data_dir).await;
    let dir = catalog_root(app_data_dir, &source);

    // Only the Bundled source seeds from the baseline (into app data). Managed /
    // UserClone roots are populated by provisioning (detect/clone/pull) — if one
    // is empty here it just hasn't been provisioned yet, so we serve what's
    // there (possibly empty) rather than stamping the baseline over a clone.
    if matches!(source, CatalogSource::Bundled) && is_empty_dir(&dir) {
        let seed_cats = discover_categories(baseline_dir);
        if let Err(e) = seed_from_baseline(baseline_dir, &dir, &seed_cats).await {
            tracing::warn!("corpus: seed from baseline failed: {e}");
        }
    }

    // Categories for indexing come from the ACTIVE root's tooling — after the
    // seed (or in a clone) `scripts/convert.sh` lives alongside the agents, so
    // the division set always reflects the catalog actually present.
    let categories = discover_categories(&dir);

    // Determine the version to stamp the index with: keep whatever a prior
    // refresh recorded, else the baseline marker.
    let version = match load_stored_meta(app_data_dir).await {
        Some(m) => m.version,
        None => BASELINE_VERSION.to_string(),
    };

    let mut corpus = match build_from_dir(&dir, &version, &categories).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("corpus: index build failed ({e}); serving empty corpus");
            empty_corpus(&version, &categories)
        }
    };

    // Prefer the catalog's own divisions.json (PR #592) for division label /
    // icon / color, falling back to the bundled metadata for first-run users
    // and pre-#592 clones that don't carry it yet.
    corpus.division_meta = load_division_meta(&dir);

    // Persist index + meta (best effort — read commands work from the
    // in-memory copy regardless; the on-disk index exists for the
    // reconciliation subsystem built in a later phase).
    if let Err(e) = persist(app_data_dir, &corpus).await {
        tracing::warn!("corpus: persist index/meta failed: {e}");
    }

    corpus
}

/// Recursively collect every `*.md` under `root`, sorted by full path for
/// determinism. Real catalog clones nest agents in subdirectories (e.g.
/// `game-development/godot/<slug>.md`, `game-development/unity/<slug>.md`), so a
/// flat top-level scan would silently miss them.
fn collect_md_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for ent in rd.flatten() {
            let path = ent.path();
            match ent.file_type() {
                Ok(ft) if ft.is_dir() => stack.push(path),
                Ok(_) if path.extension().and_then(|e| e.to_str()) == Some("md") => out.push(path),
                _ => {}
            }
        }
    }
    out.sort();
    out
}

/// Find `<file_name>` anywhere under `dir` (depth-first). Used by `read_source`
/// to resolve a nested agent's canonical file when the flat path doesn't exist.
fn find_md_under(dir: &Path, file_name: &str) -> Option<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    let mut found = None;
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for ent in rd.flatten() {
            let path = ent.path();
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.file_type().is_symlink()
                || crate::skills::metadata_is_reparse_point(&metadata)
            {
                continue;
            }
            if metadata.is_dir() {
                stack.push(path);
            } else if path.file_name().and_then(|n| n.to_str()) == Some(file_name) {
                if found.is_some() {
                    return None;
                }
                found = Some(path);
            }
        }
    }
    found
}

fn normalized_corpus_relative_path(root: &Path, path: &Path) -> Result<String, AppError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| AppError::InvalidArgument {
            message: "catalog Agent path is outside the active corpus".into(),
        })?;
    let mut parts = Vec::new();
    for component in relative.components() {
        let std::path::Component::Normal(part) = component else {
            return Err(AppError::InvalidArgument {
                message: "catalog Agent path is not normalized and relative".into(),
            });
        };
        let part = part.to_str().ok_or_else(|| AppError::InvalidArgument {
            message: "catalog Agent path is not UTF-8".into(),
        })?;
        parts.push(part);
    }
    let normalized = crate::library::normalize_relative_path(&parts.join("/"))?;
    if normalized.chars().count() > CONTROL_CENTER_MAX_PATH_CHARS {
        return Err(AppError::InvalidArgument {
            message: "catalog Agent path exceeds its 512-character limit".into(),
        });
    }
    Ok(normalized)
}

/// Build an in-memory [`Corpus`] by walking `<dir>/<category>/**/<slug>.md`
/// for every known category (recursively — real clones nest agents in
/// subdirs). Files without valid frontmatter (READMEs, workflow docs) are
/// skipped. The category is the top-level dir; the resulting `agents` vec and
/// `index` map are ordered deterministically by `(category, path)`.
async fn build_from_dir(
    dir: &Path,
    version: &str,
    categories: &[String],
) -> Result<Corpus, AppError> {
    let mut rows: Vec<(Agent, CorpusEntry, CatalogSnapshotItem)> = Vec::new();

    for category in categories.iter() {
        let category = category.as_str();
        let cat_dir = dir.join(category);
        if !cat_dir.is_dir() {
            continue; // category dir absent — fine, skip.
        }
        // Recursive, sorted-by-path collection (catches nested agents).
        let files = collect_md_files(&cat_dir);

        for path in files {
            let Some(slug) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let raw = match read_capped(&path, MAX_AGENT_BYTES).await {
                Ok(bytes) => bytes,
                Err(e) => {
                    tracing::warn!("corpus: skip {} ({e})", path.display());
                    continue;
                }
            };
            let source = match String::from_utf8(raw) {
                Ok(s) => s,
                Err(_) => {
                    tracing::warn!("corpus: skip {} (non-utf8)", path.display());
                    continue;
                }
            };
            match parse::parse_agent(slug, category, &source) {
                Ok(Some((agent, entry))) => {
                    let relative_path = normalized_corpus_relative_path(dir, &path)?;
                    let snapshot = CatalogSnapshotItem {
                        category: category.to_string(),
                        relative_path,
                        source_hash: entry.source_hash.clone(),
                        body_hash: entry.body_hash.clone(),
                    };
                    validate_snapshot_item(&snapshot)?;
                    rows.push((agent, entry, snapshot));
                }
                Ok(None) => {} // not an agent (no frontmatter) — skip silently.
                Err(e) => tracing::warn!("corpus: {e}"),
            }
        }
    }

    // `rows` is already in `(category, path)` order because we iterate
    // `categories` in tooling order and `collect_md_files` sorts by path.
    let mut agents = Vec::with_capacity(rows.len());
    let mut index = BTreeMap::new();
    let mut active_catalog_snapshot = Vec::with_capacity(rows.len());
    for (agent, entry, snapshot) in rows {
        index.insert(entry.slug.clone(), entry);
        agents.push(agent);
        active_catalog_snapshot.push(snapshot);
    }

    let count = agents.len() as u32;
    Ok(Corpus {
        agents,
        index,
        active_catalog_snapshot,
        category_order: categories.to_vec(),
        // Bundled floor; resolve_active overlays the catalog's divisions.json.
        division_meta: bundled_division_meta(),
        meta: CorpusMeta {
            version: version.to_string(),
            commit: None,
            // The build itself carries no timestamp; fetched_at reflects
            // when the *content* was last fetched. For a baseline build
            // that is the seed time, captured at persist below if no meta
            // exists yet.
            fetched_at: String::new(),
            count,
        },
    })
}

fn empty_corpus(version: &str, categories: &[String]) -> Corpus {
    Corpus {
        agents: Vec::new(),
        index: BTreeMap::new(),
        active_catalog_snapshot: Vec::new(),
        category_order: categories.to_vec(),
        division_meta: bundled_division_meta(),
        meta: CorpusMeta {
            version: version.to_string(),
            commit: None,
            fetched_at: String::new(),
            count: 0,
        },
    }
}

// ---------- Seeding ----------

/// True if `dir` does not exist or contains no entries.
fn is_empty_dir(dir: &Path) -> bool {
    match std::fs::read_dir(dir) {
        Ok(mut it) => it.next().is_none(),
        Err(_) => true,
    }
}

/// Copy `<baseline>/<category>/*.md` into `<dest>/<category>/` for each
/// `category`, plus the repo tooling (`scripts/convert.sh`) so the seeded
/// working copy can discover its own divisions. Anything else in the baseline
/// is ignored. Idempotent: re-seeding overwrites file-for-file.
async fn seed_from_baseline(
    baseline: &Path,
    dest: &Path,
    categories: &[String],
) -> Result<(), AppError> {
    if !baseline.exists() {
        return Err(AppError::Io {
            message: format!("baseline corpus not found at {}", baseline.display()),
        });
    }
    let mut seeded = 0u32;
    for category in categories.iter() {
        let src_cat = baseline.join(category);
        let mut read = match tokio::fs::read_dir(&src_cat).await {
            Ok(r) => r,
            Err(_) => continue,
        };
        let dst_cat = dest.join(category);
        tokio::fs::create_dir_all(&dst_cat)
            .await
            .map_err(|e| AppError::Io {
                message: format!("create {}: {e}", dst_cat.display()),
            })?;
        while let Ok(Some(ent)) = read.next_entry().await {
            let path = ent.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let Some(fname) = path.file_name() else {
                continue;
            };
            let bytes = read_capped(&path, MAX_AGENT_BYTES).await?;
            atomic_write(&dst_cat.join(fname), &bytes).await?;
            seeded += 1;
        }
    }

    // Carry the tooling forward so the seeded copy is self-describing: the
    // category list is then read from the working tree, not just the baseline.
    let src_script = baseline.join("scripts").join("convert.sh");
    if let Ok(bytes) = read_capped(&src_script, MAX_AGENT_BYTES).await {
        let dst_script = dest.join("scripts").join("convert.sh");
        if let Some(parent) = dst_script.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let _ = atomic_write(&dst_script, &bytes).await;
    }

    tracing::info!("corpus: seeded {seeded} agents from baseline");
    Ok(())
}

// ---------- Persistence ----------

/// Write `corpus-index.json` + `corpus-meta.json` atomically into the
/// state dir. The meta `fetched_at` is preserved from any prior meta;
/// when none exists (fresh baseline seed) it is stamped once with the
/// current UTC time so subsequent launches don't re-stamp it (keeps the
/// index byte-stable across launches).
async fn persist(app_data_dir: &Path, corpus: &Corpus) -> Result<(), AppError> {
    let sdir = state_dir(app_data_dir);
    tokio::fs::create_dir_all(&sdir)
        .await
        .map_err(|e| AppError::Io {
            message: format!("create state dir {}: {e}", sdir.display()),
        })?;

    // Index — deterministic, no timestamp.
    let index_bytes = corpus.index_json()?;
    atomic_write(&index_path(app_data_dir), &index_bytes).await?;

    // Meta — preserve prior fetched_at/commit if present; else stamp now.
    let prior = load_stored_meta(app_data_dir).await;
    let fetched_at = prior
        .as_ref()
        .map(|m| m.fetched_at.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    let commit = prior.as_ref().and_then(|m| m.commit.clone());

    let stored = StoredMeta {
        version: corpus.meta.version.clone(),
        commit,
        fetched_at,
        count: corpus.count(),
    };
    let meta_bytes = serde_json::to_vec_pretty(&stored).map_err(|e| AppError::Internal {
        message: format!("serialize corpus-meta.json: {e}"),
    })?;
    atomic_write(&meta_path(app_data_dir), &meta_bytes).await?;
    Ok(())
}

/// Load `corpus-meta.json` if present + parseable, else `None`.
async fn load_stored_meta(app_data_dir: &Path) -> Option<StoredMeta> {
    let path = meta_path(app_data_dir);
    let bytes = tokio::fs::read(&path).await.ok()?;
    serde_json::from_slice(&bytes).ok()
}

// ---------- Refresh (live tarball) ----------

/// Fetch the GitHub tarball, extract its category dirs over the working
/// copy, re-index, and persist. Returns the fresh [`CorpusMeta`].
///
/// The extraction is done into a temp dir first, then the known category
/// dirs are swapped in, so a partial/failed download never corrupts the
/// live `corpus/`.
async fn refresh(app_data_dir: &Path) -> Result<CorpusMeta, AppError> {
    // A read-only catalog source (Bundled-app-data is fine to refresh; a
    // user clone we lack permission to manage is NOT) must never be written by
    // a tarball refresh. Bundled writes into app data, so it's always allowed.
    let source = load_catalog_source(app_data_dir).await;
    if matches!(&source, CatalogSource::UserClone { manage: false, .. }) {
        return Err(AppError::InvalidArgument {
            message: "catalog source is a read-only user clone; enable manage-with-permission or switch source to refresh".into(),
        });
    }

    let bytes = download_corpus_tarball().await?;
    refresh_from_tarball(app_data_dir, &source, &bytes).await
}

fn staged_catalog_path(live: &Path, label: &str) -> Result<PathBuf, AppError> {
    let parent = live.parent().ok_or_else(|| AppError::InvalidArgument {
        message: "managed catalog root must have a parent directory".into(),
    })?;
    std::fs::create_dir_all(parent).map_err(|error| AppError::Io {
        message: format!("create managed catalog parent: {error}"),
    })?;
    let name = live
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("catalog");
    Ok(parent.join(format!(".{name}.{label}-{}", uuid::Uuid::new_v4())))
}

fn activate_staged_catalog(live: &Path, staged: &Path) -> Result<Option<PathBuf>, AppError> {
    let backup = if live.exists() {
        validate_real_directory(live, "managed catalog root")?;
        let backup = staged_catalog_path(live, "backup")?;
        std::fs::rename(live, &backup).map_err(|error| AppError::Io {
            message: format!("backup managed catalog before refresh: {error}"),
        })?;
        Some(backup)
    } else {
        None
    };
    if let Err(error) = std::fs::rename(staged, live) {
        if let Some(backup) = &backup {
            let _ = std::fs::rename(backup, live);
        }
        return Err(AppError::Io {
            message: format!("activate staged managed catalog: {error}"),
        });
    }
    Ok(backup)
}

fn restore_catalog_snapshot(live: &Path, backup: Option<&Path>) -> Result<(), AppError> {
    if live.exists() {
        std::fs::remove_dir_all(live).map_err(|error| AppError::Io {
            message: format!("remove failed managed catalog refresh: {error}"),
        })?;
    }
    if let Some(backup) = backup {
        std::fs::rename(backup, live).map_err(|error| AppError::Io {
            message: format!("restore managed catalog after refresh failure: {error}"),
        })?;
    }
    Ok(())
}

fn validate_runbooks_manifest(root: &Path) -> Result<(), AppError> {
    let path = root.join("strategy/runbooks.json");
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(AppError::Io {
                message: format!("read staged runbooks manifest: {error}"),
            });
        }
    };
    if bytes.len() as u64 > MAX_PLAYBOOK_BYTES {
        return Err(AppError::InvalidArgument {
            message: "runbooks manifest exceeds the playbook byte limit".into(),
        });
    }
    serde_json::from_slice::<RunbooksFile>(&bytes).map_err(|error| AppError::InvalidArgument {
        message: format!("staged runbooks manifest is invalid: {error}"),
    })?;
    Ok(())
}

async fn refresh_from_tarball(
    app_data_dir: &Path,
    source: &CatalogSource,
    bytes: &[u8],
) -> Result<CorpusMeta, AppError> {
    // Discover the live category set from the tarball's OWN tooling
    // (`scripts/convert.sh`) so a freshly-added upstream division is picked up
    // automatically. Falls back to the canonical default if absent.
    let categories = categories_from_tarball(bytes).unwrap_or_else(bundled_division_slugs);

    // Extract the category dirs (+ the tooling) into the active catalog root.
    // The tarball has a single top-level `agency-agents-main/` prefix we strip.
    let dir = catalog_root(app_data_dir, source);
    let staged = staged_catalog_path(&dir, "staging")?;
    std::fs::create_dir(&staged).map_err(|error| AppError::Io {
        message: format!("create staged managed catalog: {error}"),
    })?;

    // Re-index from the freshly-written working copy. Use a `main`-tagged
    // version marker; codeload does not expose the resolved commit SHA in
    // the tarball, so we record the ref name. A later phase can resolve
    // the exact SHA via the GitHub API if needed.
    let version = format!("github:main@{}", chrono::Utc::now().format("%Y-%m-%d"));
    let staged_result = async {
        let extracted = extract_categories(bytes, &staged, &categories)?;
        if extracted == 0 {
            return Err(AppError::Internal {
                message: "corpus tarball contained no agent files under known categories".into(),
            });
        }
        let mut corpus = build_from_dir(&staged, &version, &categories).await?;
        corpus.division_meta = load_division_meta(&staged);
        playbook_catalog(&staged)?;
        validate_runbooks_manifest(&staged)?;
        Ok::<Corpus, AppError>(corpus)
    }
    .await;
    let mut corpus = match staged_result {
        Ok(corpus) => corpus,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staged);
            return Err(error);
        }
    };
    let fetched_at = chrono::Utc::now().to_rfc3339();
    corpus.meta.fetched_at = fetched_at.clone();

    let backup = match activate_staged_catalog(&dir, &staged) {
        Ok(backup) => backup,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staged);
            return Err(error);
        }
    };

    // Persist a fresh meta (overwrite fetched_at/version this time —
    // unlike the baseline persist which preserves prior fetched_at).
    let sdir = state_dir(app_data_dir);
    let persist_result = async {
        tokio::fs::create_dir_all(&sdir)
            .await
            .map_err(|e| AppError::Io {
                message: format!("create state dir {}: {e}", sdir.display()),
            })?;
        let index_bytes = corpus.index_json()?;
        atomic_write(&index_path(app_data_dir), &index_bytes).await?;
        let stored = StoredMeta {
            version: version.clone(),
            commit: None,
            fetched_at: fetched_at.clone(),
            count: corpus.count(),
        };
        let meta_bytes = serde_json::to_vec_pretty(&stored).map_err(|e| AppError::Internal {
            message: format!("serialize corpus-meta.json: {e}"),
        })?;
        atomic_write(&meta_path(app_data_dir), &meta_bytes).await
    }
    .await;
    if let Err(error) = persist_result {
        if let Err(rollback) = restore_catalog_snapshot(&dir, backup.as_deref()) {
            return Err(AppError::Internal {
                message: format!("catalog refresh failed: {error}; rollback failed: {rollback}"),
            });
        }
        return Err(error);
    }
    if let Some(backup) = backup {
        let _ = std::fs::remove_dir_all(backup);
    }

    Ok(corpus.meta)
}

/// Fetch the GitHub `codeload` tarball for the corpus (capped, timed out).
/// Shared by [`refresh`] and managed-catalog provisioning (the git-absent path).
async fn download_corpus_tarball() -> Result<Vec<u8>, AppError> {
    let client = reqwest::Client::builder()
        .timeout(REFRESH_TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| AppError::Network {
            url: CORPUS_TARBALL_URL.to_string(),
            message: format!("client build: {e}"),
        })?;
    let resp = client
        .get(CORPUS_TARBALL_URL)
        .send()
        .await
        .map_err(|e| AppError::Network {
            url: CORPUS_TARBALL_URL.to_string(),
            message: e.to_string(),
        })?;
    if !resp.status().is_success() {
        return Err(AppError::HttpStatus {
            url: CORPUS_TARBALL_URL.to_string(),
            status: resp.status().as_u16(),
        });
    }
    let bytes = resp.bytes().await.map_err(|e| AppError::Network {
        url: CORPUS_TARBALL_URL.to_string(),
        message: format!("read body: {e}"),
    })?;
    if bytes.len() as u64 > MAX_TARBALL_BYTES {
        return Err(AppError::Io {
            message: format!(
                "corpus tarball {} bytes exceeds {} cap",
                bytes.len(),
                MAX_TARBALL_BYTES
            ),
        });
    }
    Ok(bytes.to_vec())
}

/// Gunzip the tarball and decode it to raw `tar` bytes, capped against a gzip
/// bomb. Shared by [`extract_categories`] and [`categories_from_tarball`].
fn gunzip_capped(tar_gz: &[u8]) -> Result<Vec<u8>, AppError> {
    use std::io::Read;
    let gz = flate2::read::GzDecoder::new(tar_gz);
    let mut capped = gz.take(MAX_TARBALL_BYTES * 8);
    let mut tar_bytes = Vec::new();
    capped
        .read_to_end(&mut tar_bytes)
        .map_err(|e| AppError::Io {
            message: format!("gunzip corpus tarball: {e}"),
        })?;
    Ok(tar_bytes)
}

/// Read `scripts/convert.sh` out of the tarball and parse its `AGENT_DIRS`
/// array, so a refresh adopts upstream's current division set. `None` if the
/// script isn't present or doesn't parse (caller falls back to the default).
fn categories_from_tarball(tar_gz: &[u8]) -> Option<Vec<String>> {
    use std::io::Read;
    let tar_bytes = gunzip_capped(tar_gz).ok()?;
    let mut archive = tar::Archive::new(std::io::Cursor::new(tar_bytes));
    for entry in archive.entries().ok()? {
        let mut entry = entry.ok()?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path().ok()?;
        let comps: Vec<String> = path
            .components()
            .filter_map(|c| match c {
                std::path::Component::Normal(s) => s.to_str().map(|s| s.to_string()),
                _ => None,
            })
            .collect();
        // top/scripts/convert.sh
        if comps.len() == 3 && comps[1] == "scripts" && comps[2] == "convert.sh" {
            let mut text = String::new();
            entry.read_to_string(&mut text).ok()?;
            return parse_agent_dirs(&text).filter(|v| !v.is_empty());
        }
    }
    None
}

fn archive_path_components(path: &Path) -> Result<Vec<String>, AppError> {
    if path.is_absolute() {
        return Err(AppError::InvalidArgument {
            message: "catalog archive paths must be relative".into(),
        });
    }
    path.components()
        .map(|component| {
            let std::path::Component::Normal(value) = component else {
                return Err(AppError::InvalidArgument {
                    message: "catalog archive paths must contain only normal components".into(),
                });
            };
            value
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| AppError::InvalidArgument {
                    message: "catalog archive paths must be valid UTF-8".into(),
                })
        })
        .collect()
}

fn managed_playbook_target(root: &Path, relative_path: &str) -> Result<PathBuf, AppError> {
    let canonical_root = validate_real_directory(root, "managed catalog root")?;
    let mut directory = root.to_path_buf();
    for component in Path::new(relative_path)
        .parent()
        .into_iter()
        .flat_map(Path::components)
    {
        let std::path::Component::Normal(component) = component else {
            return Err(AppError::InvalidArgument {
                message: "managed playbook paths must be normalized and relative".into(),
            });
        };
        directory.push(component);
        match std::fs::symlink_metadata(&directory) {
            Ok(metadata)
                if metadata.is_dir()
                    && !metadata.file_type().is_symlink()
                    && !crate::skills::metadata_is_reparse_point(&metadata) => {}
            Ok(_) => {
                return Err(AppError::InvalidArgument {
                    message: "managed playbook paths cannot contain links or special entries"
                        .into(),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&directory).map_err(|error| AppError::Io {
                    message: format!("create managed playbook directory: {error}"),
                })?;
            }
            Err(error) => {
                return Err(AppError::Io {
                    message: format!("inspect managed playbook directory: {error}"),
                });
            }
        }
        let canonical = std::fs::canonicalize(&directory).map_err(|error| AppError::Io {
            message: format!("resolve managed playbook directory: {error}"),
        })?;
        if !canonical.starts_with(&canonical_root) {
            return Err(AppError::InvalidArgument {
                message: "managed playbook path resolved outside the catalog".into(),
            });
        }
    }
    let target = root.join(relative_path);
    if let Ok(metadata) = std::fs::symlink_metadata(&target) {
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || crate::skills::metadata_is_reparse_point(&metadata)
        {
            return Err(AppError::InvalidArgument {
                message: "managed playbook targets must be real files".into(),
            });
        }
    }
    Ok(target)
}

/// Gunzip + untar `tar_gz`, writing every `<category>/<slug>.md` whose category
/// is in `categories` into `<dest>/<category>/`, plus `scripts/convert.sh` (so
/// the working copy stays self-describing). The codeload tarball nests
/// everything under a single `agency-agents-main/` top-level dir, which we
/// strip. Returns the count of agent files written.
///
/// Path-traversal safe: we only ever join the *sanitized* `category` +
/// `file_name` onto `dest`; the raw archive path is never used to build a
/// write target.
fn extract_categories(tar_gz: &[u8], dest: &Path, categories: &[String]) -> Result<u32, AppError> {
    use std::io::Read;

    let tar_bytes = gunzip_capped(tar_gz)?;
    let mut archive = tar::Archive::new(std::io::Cursor::new(tar_bytes));
    let entries = archive.entries().map_err(|e| AppError::Io {
        message: format!("read tar entries: {e}"),
    })?;

    let is_category = |c: &str| categories.iter().any(|cat| cat == c);
    let mut written = 0u32;
    let mut playbooks_written = 0usize;
    for entry in entries {
        let mut entry = entry.map_err(|e| AppError::Io {
            message: format!("tar entry: {e}"),
        })?;
        let path = entry.path().map_err(|e| AppError::Io {
            message: format!("tar entry path: {e}"),
        })?;
        // Strip the single top-level `agency-agents-main/` component.
        let comps = archive_path_components(&path)?;
        let relative = (comps.len() >= 3).then(|| comps[1..].join("/"));
        let in_playbook_root = comps
            .get(1)
            .is_some_and(|value| PLAYBOOK_ROOTS.contains(&value.as_str()));
        if !entry.header().entry_type().is_file() {
            if in_playbook_root && !entry.header().entry_type().is_dir() {
                return Err(AppError::InvalidArgument {
                    message: "catalog playbooks cannot contain links or special archive entries"
                        .into(),
                });
            }
            continue;
        }

        // Persist the tooling so subsequent launches re-derive categories.
        if comps.len() == 3 && comps[1] == "scripts" && comps[2] == "convert.sh" {
            let scripts_dir = dest.join("scripts");
            let _ = std::fs::create_dir_all(&scripts_dir);
            let mut buf = Vec::new();
            if entry.read_to_end(&mut buf).is_ok() {
                let _ = std::fs::write(scripts_dir.join("convert.sh"), &buf);
            }
            continue;
        }

        let retain_runbook_manifest = relative.as_deref() == Some("strategy/runbooks.json");
        let retain_playbook = relative
            .as_deref()
            .is_some_and(|value| validate_playbook_relative_path(value).is_ok());
        if retain_runbook_manifest || retain_playbook {
            if retain_playbook {
                if playbooks_written >= MAX_PLAYBOOK_DOCUMENTS {
                    return Err(AppError::InvalidArgument {
                        message: format!(
                            "catalog contains more than {MAX_PLAYBOOK_DOCUMENTS} playbook documents"
                        ),
                    });
                }
                playbooks_written += 1;
            }
            if entry.size() > MAX_PLAYBOOK_BYTES {
                return Err(AppError::InvalidArgument {
                    message: format!(
                        "catalog playbook exceeds the {MAX_PLAYBOOK_BYTES}-byte limit"
                    ),
                });
            }
            let relative = relative.expect("retained paths have a relative path");
            let target = managed_playbook_target(dest, &relative)?;
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).map_err(|e| AppError::Io {
                message: format!("read managed catalog content: {e}"),
            })?;
            if retain_playbook && std::str::from_utf8(&buf).is_err() {
                return Err(AppError::InvalidArgument {
                    message: "catalog playbooks must be valid UTF-8".into(),
                });
            }
            std::fs::write(target, buf).map_err(|e| AppError::Io {
                message: format!("write managed catalog content: {e}"),
            })?;
            continue;
        }

        if comps.len() < 3 {
            continue; // need top/<category>/<file>
        }
        let category = comps[1].as_str();
        let fname = comps.last().unwrap().as_str();
        if !is_category(category) {
            continue;
        }
        if !fname.ends_with(".md") || fname == "README.md" {
            continue;
        }
        // Sanitized target — built only from validated components.
        let cat_dir = dest.join(category);
        std::fs::create_dir_all(&cat_dir).map_err(|e| AppError::Io {
            message: format!("create {}: {e}", cat_dir.display()),
        })?;
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).map_err(|e| AppError::Io {
            message: format!("read tar file {}: {e}", fname),
        })?;
        std::fs::write(cat_dir.join(fname), &buf).map_err(|e| AppError::Io {
            message: format!("write {}: {e}", cat_dir.join(fname).display()),
        })?;
        written += 1;
    }
    Ok(written)
}

// ---------- Small fs helper ----------

/// Read up to `max` bytes; error (not truncate) on oversize. Mirrors
/// `util::fs::read_capped` but accepts a sync `Path` + tokio read so we
/// don't need to thread the catalog's exact helper here.
async fn read_capped(path: &Path, max: u64) -> Result<Vec<u8>, AppError> {
    let bytes = tokio::fs::read(path).await.map_err(|e| AppError::Io {
        message: format!("read {}: {e}", path.display()),
    })?;
    if bytes.len() as u64 > max {
        return Err(AppError::Io {
            message: format!("{} exceeds {} byte cap", path.display(), max),
        });
    }
    Ok(bytes)
}

// =====================================================================
// Catalog detection / provisioning / pull (#1 clone-as-source-of-truth)
// =====================================================================

/// `~/.agency-agents` — the default managed-catalog location (shared with the
/// agency-agents CLI).
fn home_agency_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".agency-agents"))
}

/// Is a `git` binary on PATH? Determines clone/pull vs tarball-snapshot.
async fn git_available() -> bool {
    run_git(&["--version"], None).await.is_ok()
}

/// Is `root` a git checkout (so a pull is `git pull`, not a tarball swap)?
fn has_git_dir(root: &Path) -> bool {
    root.join(".git").exists()
}

/// Run `git` with `args` (optionally in `cwd`) off the async runtime. Errors
/// carry git's stderr so failures are diagnosable.
pub(crate) async fn run_git(args: &[&str], cwd: Option<&Path>) -> Result<String, AppError> {
    let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let cwd = cwd.map(|p| p.to_path_buf());
    let out = tokio::task::spawn_blocking(move || {
        let mut c = std::process::Command::new("git");
        if let Some(d) = &cwd {
            c.current_dir(d);
        }
        c.args(&owned).output()
    })
    .await
    .map_err(|e| AppError::Internal {
        message: format!("join git task: {e}"),
    })?
    .map_err(|e| AppError::Io {
        message: format!("spawn git: {e}"),
    })?;

    if !out.status.success() {
        return Err(AppError::Io {
            message: format!(
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Quick agent count for a candidate badge: top-level `.md` files across the
/// root's discovered categories. Cheap + synchronous (cold path, small repo).
fn quick_agent_count(root: &Path) -> u32 {
    let mut n = 0u32;
    for cat in discover_categories(root) {
        if let Ok(rd) = std::fs::read_dir(root.join(&cat)) {
            n += rd
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md"))
                .filter(|e| e.file_name().to_string_lossy() != "README.md")
                .count() as u32;
        }
    }
    n
}

/// Build a [`CatalogCandidate`] for `path` if it looks like a catalog.
fn candidate_for(path: &Path, kind: &str) -> Option<CatalogCandidate> {
    if !looks_like_catalog(path) {
        return None;
    }
    Some(CatalogCandidate {
        path: path.to_string_lossy().to_string(),
        kind: kind.to_string(),
        has_git: has_git_dir(path),
        agent_count: quick_agent_count(path),
    })
}

/// Detect candidate catalogs. Always checks `~/.agency-agents`; when `scan` is
/// true also walks common dev roots for an `agency-agents` checkout (the "Find
/// Agency Agents" button). Pure of app state — safe to call anytime.
async fn detect_catalogs(scan: bool) -> CatalogDetection {
    let git_available = git_available().await;
    let mut candidates: Vec<CatalogCandidate> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let push = |c: Option<CatalogCandidate>,
                list: &mut Vec<CatalogCandidate>,
                seen: &mut std::collections::HashSet<String>| {
        if let Some(c) = c {
            if seen.insert(c.path.clone()) {
                list.push(c);
            }
        }
    };

    if let Some(managed) = home_agency_dir() {
        push(
            candidate_for(&managed, "managed"),
            &mut candidates,
            &mut seen,
        );
    }

    if scan {
        if let Some(home) = dirs::home_dir() {
            for root in SCAN_ROOTS {
                // Look for `<home>/<root>/agency-agents` and a direct
                // `<home>/<root>` that is itself a catalog.
                let base = home.join(root);
                push(
                    candidate_for(&base.join("agency-agents"), "userClone"),
                    &mut candidates,
                    &mut seen,
                );
                // One level of children named with "agency" (cheap heuristic).
                if let Ok(rd) = std::fs::read_dir(&base) {
                    for ent in rd.filter_map(|e| e.ok()) {
                        let p = ent.path();
                        if p.is_dir()
                            && p.file_name()
                                .map(|n| n.to_string_lossy().contains("agency"))
                                .unwrap_or(false)
                        {
                            push(candidate_for(&p, "userClone"), &mut candidates, &mut seen);
                        }
                    }
                }
            }
        }
    }

    CatalogDetection {
        git_available,
        scanned: scan,
        candidates,
    }
}

/// Ensure `~/.agency-agents` holds a catalog, cloning (git) or unpacking the
/// snapshot (no git) as needed. Returns the managed root path. Idempotent: if
/// it already looks like a catalog, this is a no-op (use pull to update).
async fn provision_managed() -> Result<PathBuf, AppError> {
    let path = home_agency_dir().ok_or_else(|| AppError::Io {
        message: "cannot resolve home directory".into(),
    })?;
    if looks_like_catalog(&path) {
        return Ok(path); // already provisioned
    }

    let empty = is_empty_dir(&path);
    if git_available().await && !path.exists() {
        // git clone into a fresh dir (clone requires absent/empty target).
        // Full clone (not shallow) so commit history is available for accurate
        // behind/ahead counts and diff stats in the Catalog status panel.
        run_git(&["clone", CATALOG_GIT_URL, &path.to_string_lossy()], None).await?;
    } else if git_available().await && empty {
        // Full clone (not shallow) so commit history is available for accurate
        // behind/ahead counts and diff stats in the Catalog status panel.
        run_git(&["clone", CATALOG_GIT_URL, &path.to_string_lossy()], None).await?;
    } else {
        // No git (or a non-empty target): drop the snapshot tarball in place.
        tokio::fs::create_dir_all(&path)
            .await
            .map_err(|e| AppError::Io {
                message: format!("create {}: {e}", path.display()),
            })?;
        let bytes = download_corpus_tarball().await?;
        let categories = categories_from_tarball(&bytes).unwrap_or_else(bundled_division_slugs);
        let written = extract_categories(&bytes, &path, &categories)?;
        if written == 0 {
            return Err(AppError::Internal {
                message: "provision: snapshot tarball contained no agent files".into(),
            });
        }
    }
    Ok(path)
}

/// Pull the active catalog root up to date. Git checkout → `git pull --ff-only`;
/// otherwise a tarball refresh into the root. Read-only sources are rejected by
/// the caller; Bundled refreshes its app-data copy.
async fn pull_active(app_data_dir: &Path, source: &CatalogSource) -> Result<(), AppError> {
    if matches!(source, CatalogSource::UserClone { manage: false, .. }) {
        return Err(AppError::InvalidArgument {
            message: "catalog source is read-only (manage-with-permission is off)".into(),
        });
    }
    let root = catalog_root(app_data_dir, source);
    if has_git_dir(&root) && git_available().await {
        run_git(&["-C", &root.to_string_lossy(), "pull", "--ff-only"], None).await?;
        Ok(())
    } else {
        // Tarball refresh writes into the active root (refresh() resolves it).
        refresh(app_data_dir).await.map(|_| ())
    }
}

// =====================================================================
// Tauri commands (contracts.md §C — corpus surface)
// =====================================================================

use crate::state::AppState;
use tauri::{AppHandle, Manager, State};

/// Resolve the bundled baseline dir from the Tauri resource dir. In dev
/// the resources live under the crate; in a bundled app they're inside
/// the `.app`. Tauri's `resource_dir()` resolves both.
fn baseline_dir(app: &AppHandle) -> Result<PathBuf, AppError> {
    let res = app.path().resource_dir().map_err(|e| AppError::Internal {
        message: format!("resolve resource_dir: {e}"),
    })?;
    Ok(res.join("resources").join("corpus-baseline"))
}

/// Resolve the per-app data dir via Tauri's path resolver (honors the
/// bundle id `com.zerologic.agency-agents-app`).
pub(crate) fn app_data_dir(app: &AppHandle) -> Result<PathBuf, AppError> {
    app.path().app_data_dir().map_err(|e| AppError::Internal {
        message: format!("resolve app_data_dir: {e}"),
    })
}

pub(crate) async fn active_catalog_root(app: &AppHandle) -> Result<PathBuf, AppError> {
    let adir = app_data_dir(app)?;
    Ok(active_catalog_root_at(&adir).await)
}

pub(crate) async fn active_catalog_root_at(adir: &Path) -> PathBuf {
    let source = load_catalog_source(adir).await;
    catalog_root(adir, &source)
}

/// Read the raw, byte-exact `.md` source of a seeded agent from the working
/// corpus copy (`<app_data>/corpus/<category>/<slug>.md`). Identity-tool
/// installs (claude-code, copilot) ship this verbatim, and provenance
/// reconciliation re-renders against it. Path is derived from app data +
/// the agent's own category/slug — never from IPC input.
pub(crate) async fn read_source(
    app: &AppHandle,
    category: &str,
    slug: &str,
) -> Result<String, AppError> {
    let adir = app_data_dir(app)?;
    let source = load_catalog_source(&adir).await;
    let cat_dir = catalog_root(&adir, &source).join(category);
    let fname = format!("{slug}.md");
    // Flat path first (the common case); fall back to a recursive search for
    // nested agents (e.g. game-development/godot/<slug>.md in a real clone).
    let flat = cat_dir.join(&fname);
    let path = if flat.exists() {
        flat
    } else {
        find_md_under(&cat_dir, &fname).unwrap_or(flat)
    };
    let bytes = read_capped(&path, MAX_AGENT_BYTES).await?;
    String::from_utf8(bytes).map_err(|e| AppError::Io {
        message: format!("agent source {slug}.md not UTF-8: {e}"),
    })
}

/// Ensure the in-memory corpus is built + memoized on `AppState`, then
/// return the shared `Arc`. First call seeds (if needed), parses, and
/// persists the index; subsequent calls are a cheap cache read.
pub(crate) async fn ensure_corpus(
    app: &AppHandle,
    state: &AppState,
) -> Result<Arc<Corpus>, AppError> {
    // Hold the cache lock across the ENTIRE init — check, seed, parse, store.
    // The frontend fires corpus_list + corpus_categories (+ corpus_status)
    // concurrently on mount; a released-lock double-check would let each run
    // `seed_from_baseline` at once, racing on the same `<file>.tmp` paths
    // (rename → ENOENT). Serializing the first load is correct and cheap:
    // it happens once, and every later call is a fast locked cache read.
    let mut cached = state.corpus_cache.lock().await;
    if let Some(c) = cached.as_ref() {
        return Ok(Arc::clone(c));
    }
    let adir = app_data_dir(app)?;
    let bdir = baseline_dir(app)?;
    let corpus = Arc::new(resolve_active(&adir, &bdir).await);
    *cached = Some(Arc::clone(&corpus));
    Ok(corpus)
}

/// `corpus_status()` — version / commit / fetched-at / count for the
/// active corpus.
#[tauri::command]
pub async fn corpus_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CorpusMeta, AppError> {
    let corpus = ensure_corpus(&app, &state).await?;
    Ok(corpus.meta())
}

/// `corpus_refresh()` — fetch the live tarball, re-index, swap the
/// memoized corpus, and return the fresh meta.
#[tauri::command]
pub async fn corpus_refresh(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CorpusMeta, AppError> {
    refresh_catalog_authority(&app, &state, CatalogRefreshEntrypoint::CorpusRefresh).await
}

/// `catalog_source_get()` — the persisted [`CatalogSource`] (default Bundled).
#[tauri::command]
pub async fn catalog_source_get(app: AppHandle) -> Result<CatalogSource, AppError> {
    let adir = app_data_dir(&app)?;
    Ok(load_catalog_source(&adir).await)
}

/// `catalog_configured()` — whether the user has made an explicit catalog-source
/// choice yet (i.e. `state/catalog.json` exists). Drives the first-run prompt:
/// `false` ⇒ show the catalog-source picker before anything else.
#[tauri::command]
pub async fn catalog_configured(app: AppHandle) -> Result<bool, AppError> {
    let adir = app_data_dir(&app)?;
    Ok(catalog_source_path(&adir).exists())
}

/// `catalog_source_set(source)` — switch where the catalog is read from, then
/// rebuild + swap the in-memory corpus so every view reflects the new source.
/// Validates that a `Managed`/`UserClone` path exists and looks like a catalog
/// (has at least one known category dir or `scripts/convert.sh`).
#[tauri::command]
pub async fn catalog_source_set(
    app: AppHandle,
    state: State<'_, AppState>,
    source: CatalogSource,
) -> Result<CorpusMeta, AppError> {
    // Validate non-bundled roots before committing to them.
    if let CatalogSource::Managed { path } | CatalogSource::UserClone { path, .. } = &source {
        let root = PathBuf::from(path);
        if !root.is_dir() {
            return Err(AppError::InvalidArgument {
                message: format!("catalog path is not a directory: {path}"),
            });
        }
        if !looks_like_catalog(&root) {
            return Err(AppError::InvalidArgument {
                message: format!(
                    "{path} doesn't look like an agency-agents catalog (no scripts/convert.sh or category dirs)"
                ),
            });
        }
    }

    let _flight =
        state
            .corpus_refresh_in_flight
            .try_lock()
            .map_err(|_| AppError::InvalidArgument {
                message: "catalog change already in progress".into(),
            })?;

    let database =
        state
            .completed_state_database()
            .await?
            .ok_or_else(|| AppError::InvalidArgument {
                message: "SQLite migration must complete before changing catalog source".into(),
            })?;
    select_catalog_source(&app, &state, &database, &source).await
}

/// Durably prepare a source selection before changing the active-source
/// document, then rebuild and atomically align feed provenance. A failed or
/// interrupted rebuild leaves the old history internal but unavailable.
async fn select_catalog_source(
    app: &AppHandle,
    state: &AppState,
    database: &crate::state_db::StateDatabase,
    source: &CatalogSource,
) -> Result<CorpusMeta, AppError> {
    let adir = app_data_dir(app)?;
    let current = load_catalog_source(&adir).await;
    let document = load_control_center(database).await?;
    if document.active_catalog_provenance.is_none()
        && document.catalog_pending_source_transition.is_none()
    {
        let current_corpus = read_source_checked(&adir, &current).await?;
        ensure_catalog_baseline(
            database,
            catalog_source_key(&current),
            current_corpus.catalog_snapshot(),
        )
        .await?;
    }
    let target_source_key = catalog_source_key(source);
    let mode = prepare_catalog_source_selection(
        database,
        target_source_key.clone(),
        chrono::Utc::now().to_rfc3339(),
    )
    .await?;
    if current != *source {
        save_catalog_source(&adir, source).await?;
    }
    let fresh = Arc::new(build_source_checked(&adir, source).await?);
    finish_catalog_source_selection(database, target_source_key, fresh.catalog_snapshot(), mode)
        .await?;
    let meta = fresh.meta();
    {
        let mut cached = state.corpus_cache.lock().await;
        *cached = Some(fresh);
    }
    Ok(meta)
}

async fn build_source_checked(
    app_data_dir: &Path,
    source: &CatalogSource,
) -> Result<Corpus, AppError> {
    let corpus = read_source_checked(app_data_dir, source).await?;
    persist(app_data_dir, &corpus).await?;
    Ok(corpus)
}

async fn read_source_checked(
    app_data_dir: &Path,
    source: &CatalogSource,
) -> Result<Corpus, AppError> {
    let dir = catalog_root(app_data_dir, source);
    let categories = discover_categories(&dir);
    let version = load_stored_meta(app_data_dir)
        .await
        .map(|meta| meta.version)
        .unwrap_or_else(|| BASELINE_VERSION.to_string());
    let mut corpus = build_from_dir(&dir, &version, &categories).await?;
    corpus.division_meta = load_division_meta(&dir);
    Ok(corpus)
}

/// `catalog_detect(scan)` — discover candidate catalogs (always checks
/// `~/.agency-agents`; `scan=true` also walks common dev roots).
#[tauri::command]
pub async fn catalog_detect(scan: bool) -> Result<CatalogDetection, AppError> {
    Ok(detect_catalogs(scan).await)
}

/// `catalog_provision_managed()` — clone/snapshot into `~/.agency-agents`, set
/// it as the managed source, and rebuild. The "set one up for me" path.
#[tauri::command]
pub async fn catalog_provision_managed(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CorpusMeta, AppError> {
    state.require_network("catalog_provision_managed").await?;
    let _flight =
        state
            .corpus_refresh_in_flight
            .try_lock()
            .map_err(|_| AppError::InvalidArgument {
                message: "catalog change already in progress".into(),
            })?;
    let database =
        state
            .completed_state_database()
            .await?
            .ok_or_else(|| AppError::InvalidArgument {
                message: "SQLite migration must complete before provisioning a catalog".into(),
            })?;
    let path = provision_managed().await?;
    let source = CatalogSource::Managed {
        path: path.to_string_lossy().to_string(),
    };
    select_catalog_source(&app, &state, &database, &source).await
}

/// `catalog_pull()` — update the active catalog root (git pull or tarball
/// refresh), then rebuild. Rejected for a read-only user clone.
#[tauri::command]
pub async fn catalog_pull(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CorpusMeta, AppError> {
    refresh_catalog_authority(&app, &state, CatalogRefreshEntrypoint::CatalogPull).await
}

async fn catalog_source_transition_recover_state(state: &AppState) -> Result<bool, AppError> {
    let _flight =
        state
            .corpus_refresh_in_flight
            .try_lock()
            .map_err(|_| AppError::InvalidArgument {
                message: "catalog change already in progress".into(),
            })?;
    let database =
        state
            .completed_state_database()
            .await?
            .ok_or_else(|| AppError::InvalidArgument {
                message: "SQLite migration must complete before recovering catalog source".into(),
            })?;
    if load_control_center(&database)
        .await?
        .catalog_pending_source_transition
        .is_none()
    {
        return Ok(false);
    }
    let source = load_catalog_source(&state.app_data_dir).await;
    let (corpus, _) =
        recover_catalog_source_for_refresh(&state.app_data_dir, &database, &source).await?;
    *state.corpus_cache.lock().await = Some(Arc::new(corpus));
    Ok(true)
}

/// Resolve a pending source transition using local bytes only. `false` is an
/// explicit no-op outcome; callers may then choose whether network refresh is
/// permitted for the active source.
#[tauri::command]
pub async fn catalog_source_transition_recover(
    state: State<'_, AppState>,
) -> Result<bool, AppError> {
    catalog_source_transition_recover_state(&state).await
}

#[derive(Clone, Copy)]
enum CatalogRefreshEntrypoint {
    CorpusRefresh,
    CatalogPull,
}

impl CatalogRefreshEntrypoint {
    fn feature(self) -> &'static str {
        match self {
            Self::CorpusRefresh => "corpus_refresh",
            Self::CatalogPull => "catalog_pull",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CatalogRefreshRecovery {
    Continue,
    Complete,
}

async fn recover_catalog_source_for_refresh(
    app_data_dir: &Path,
    database: &crate::state_db::StateDatabase,
    source: &CatalogSource,
) -> Result<(Corpus, CatalogRefreshRecovery), AppError> {
    let source_key = catalog_source_key(source);
    let document = load_control_center(database).await?;
    let mode = match document.catalog_pending_source_transition.as_ref() {
        None => None,
        Some(pending) if pending.from_source_key == source_key => {
            Some(CatalogSourceSelection::Restore)
        }
        Some(pending) if pending.to_source_key == source_key => {
            Some(CatalogSourceSelection::Transition)
        }
        Some(_) => {
            return Err(AppError::InvalidArgument {
                message: CATALOG_SOURCE_TRANSITION_MISMATCH.into(),
            });
        }
    };
    let corpus = if mode == Some(CatalogSourceSelection::Transition) {
        build_source_checked(app_data_dir, source).await?
    } else {
        read_source_checked(app_data_dir, source).await?
    };
    if let Some(mode) = mode {
        finish_catalog_source_selection(database, source_key, corpus.catalog_snapshot(), mode)
            .await?;
    }
    let recovery = if mode == Some(CatalogSourceSelection::Transition) {
        CatalogRefreshRecovery::Complete
    } else {
        CatalogRefreshRecovery::Continue
    };
    Ok((corpus, recovery))
}

async fn refresh_catalog_authority(
    app: &AppHandle,
    state: &AppState,
    entrypoint: CatalogRefreshEntrypoint,
) -> Result<CorpusMeta, AppError> {
    state.require_network(entrypoint.feature()).await?;
    let _flight =
        state
            .corpus_refresh_in_flight
            .try_lock()
            .map_err(|_| AppError::InvalidArgument {
                message: "catalog refresh already in progress".into(),
            })?;
    let adir = app_data_dir(app)?;
    let database =
        state
            .completed_state_database()
            .await?
            .ok_or_else(|| AppError::InvalidArgument {
                message: "SQLite migration must complete before refreshing the catalog".into(),
            })?;
    // Establish a durable same-source baseline before any catalog bytes
    // change. An existing pending marker retains its baseline so a crashed
    // post-pull commit replays the diff on the next successful refresh.
    let source = load_catalog_source(&adir).await;
    let (before, recovery) =
        match recover_catalog_source_for_refresh(&adir, &database, &source).await {
            Ok(result) => result,
            Err(error) => {
                let _ = mark_catalog_feed_stale(&database, &error.to_string()).await;
                return Err(error);
            }
        };
    if recovery == CatalogRefreshRecovery::Complete {
        let meta = before.meta();
        *state.corpus_cache.lock().await = Some(Arc::new(before));
        return Ok(meta);
    }
    if let Err(error) = ensure_catalog_baseline(
        &database,
        catalog_source_key(&source),
        before.catalog_snapshot(),
    )
    .await
    {
        let _ = mark_catalog_feed_stale(&database, &error.to_string()).await;
        return Err(error);
    }
    if let Err(error) = begin_catalog_refresh(
        &database,
        catalog_source_key(&source),
        entrypoint,
        chrono::Utc::now().to_rfc3339(),
    )
    .await
    {
        let _ = mark_catalog_feed_stale(&database, &error.to_string()).await;
        return Err(error);
    }

    if let Err(error) = pull_active(&adir, &source).await {
        let _ = mark_catalog_feed_stale(&database, &error.to_string()).await;
        return Err(error);
    }
    let fresh = match build_source_checked(&adir, &source).await {
        Ok(corpus) => corpus,
        Err(error) => {
            let _ = mark_catalog_feed_stale(&database, &error.to_string()).await;
            return Err(error);
        }
    };
    let meta = fresh.meta();
    if let Err(error) = persist_catalog_refresh(
        &database,
        catalog_source_key(&source),
        fresh.catalog_snapshot(),
        chrono::Utc::now().to_rfc3339(),
    )
    .await
    {
        let _ = mark_catalog_feed_stale(&database, &error.to_string()).await;
        return Err(error);
    }
    *state.corpus_cache.lock().await = Some(Arc::new(fresh));
    Ok(meta)
}

/// Local-only bounded catalog feed projection. The active snapshot remains in
/// SQLite and never crosses IPC.
#[tauri::command]
pub async fn catalog_feed_list(state: State<'_, AppState>) -> Result<CatalogFeedState, AppError> {
    let database =
        state
            .completed_state_database()
            .await?
            .ok_or_else(|| AppError::InvalidArgument {
                message: "SQLite migration must complete before reading catalog changes".into(),
            })?;
    let source = load_catalog_source(&state.app_data_dir).await;
    catalog_feed_state_for_source(&database, &catalog_source_key(&source)).await
}

/// `catalog_status()` — provenance + freshness of the active catalog (source,
/// git commit/branch/dirty, remote repo, version, agent count). Local-only (no
/// network); the git fields are empty for a bundled/snapshot source.
#[tauri::command]
pub async fn catalog_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CatalogStatus, AppError> {
    let adir = app_data_dir(&app)?;
    let source = load_catalog_source(&adir).await;
    let corpus = ensure_corpus(&app, &state).await?;
    let meta = corpus.meta();
    let root = catalog_root(&adir, &source);

    let is_git = has_git_dir(&root) && git_available().await;
    let mut branch = None;
    let mut commit = None;
    let mut last_commit_subject = None;
    let mut last_commit_date = None;
    let mut dirty_count = 0u32;
    let mut remote_url = None;
    let mut repo_slug = None;
    if is_git {
        let rs = root.to_string_lossy().to_string();
        branch = run_git(&["-C", &rs, "rev-parse", "--abbrev-ref", "HEAD"], None)
            .await
            .ok()
            .map(|s| s.trim().to_string());
        commit = run_git(&["-C", &rs, "rev-parse", "--short", "HEAD"], None)
            .await
            .ok()
            .map(|s| s.trim().to_string());
        if let Ok(log) = run_git(&["-C", &rs, "log", "-1", "--format=%s%x1f%cI"], None).await {
            let mut it = log.trim().splitn(2, '\u{1f}');
            last_commit_subject = it.next().map(|s| s.to_string()).filter(|s| !s.is_empty());
            last_commit_date = it
                .next()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
        }
        if let Ok(porcelain) = run_git(&["-C", &rs, "status", "--porcelain"], None).await {
            dirty_count = porcelain.lines().filter(|l| !l.trim().is_empty()).count() as u32;
        }
        remote_url = run_git(&["-C", &rs, "remote", "get-url", "origin"], None)
            .await
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        repo_slug = remote_url
            .as_deref()
            .and_then(extract_github_repo)
            .map(|r| format!("{}/{}", r.owner, r.repo));
    }

    let root_out = match source {
        CatalogSource::Bundled => None,
        _ => Some(root.to_string_lossy().to_string()),
    };

    Ok(CatalogStatus {
        source,
        root: root_out,
        is_git,
        branch,
        commit,
        last_commit_subject,
        last_commit_date,
        dirty_count,
        remote_url,
        repo_slug,
        version: meta.version,
        fetched_at: meta.fetched_at,
        agent_count: corpus.count(),
    })
}

/// `catalog_check_updates()` — fetch the active git catalog and report how far
/// behind/ahead upstream it is, plus a `git diff --stat` preview (the "stats on
/// diffs"). For a non-git source, returns `is_git=false` (the UI offers a plain
/// snapshot refresh instead). Network: runs `git fetch`.
#[tauri::command]
pub async fn catalog_check_updates(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CatalogUpdateCheck, AppError> {
    state.require_network("catalog_check_updates").await?;
    let adir = app_data_dir(&app)?;
    let source = load_catalog_source(&adir).await;
    let root = catalog_root(&adir, &source);

    if !(has_git_dir(&root) && git_available().await) {
        return Ok(CatalogUpdateCheck {
            is_git: false,
            behind: 0,
            ahead: 0,
            changed_files: 0,
            diffstat: String::new(),
            up_to_date: false,
        });
    }

    let rs = root.to_string_lossy().to_string();
    run_git(&["-C", &rs, "fetch", "--quiet"], None).await?;

    // "<ahead>\t<behind>" relative to the upstream tracking branch.
    let (mut ahead, mut behind) = (0u32, 0u32);
    if let Ok(counts) = run_git(
        &[
            "-C",
            &rs,
            "rev-list",
            "--left-right",
            "--count",
            "HEAD...@{u}",
        ],
        None,
    )
    .await
    {
        let mut it = counts.split_whitespace();
        ahead = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        behind = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    }

    let (mut diffstat, mut changed_files) = (String::new(), 0u32);
    if behind > 0 {
        diffstat = run_git(&["-C", &rs, "diff", "--stat", "HEAD..@{u}"], None)
            .await
            .unwrap_or_default();
        if let Ok(names) = run_git(&["-C", &rs, "diff", "--name-only", "HEAD..@{u}"], None).await {
            changed_files = names.lines().filter(|l| !l.trim().is_empty()).count() as u32;
        }
    }

    Ok(CatalogUpdateCheck {
        is_git: true,
        behind,
        ahead,
        changed_files,
        diffstat,
        up_to_date: behind == 0,
    })
}

// ---------- Runbooks (NEXUS scenario rosters) ----------

/// The `strategy/runbooks.json` manifest (catalog PR #664): machine-readable
/// NEXUS runbook rosters referenced BY SLUG (the corpus id / agent `.md` filename
/// stem), so the app resolves each to a catalog agent and can deploy the set.
#[derive(Debug, Clone, Deserialize, Serialize)]
struct RunbooksFile {
    #[serde(default)]
    runbooks: Vec<Runbook>,
}

/// One NEXUS scenario runbook: a titled, mode-sized roster grouped into teams
/// (with activation timing), plus a pointer to its prose doc.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Runbook {
    pub slug: String,
    pub title: String,
    pub mode: String,
    pub duration: String,
    pub summary: String,
    pub doc: String,
    pub roster: Vec<RunbookGroup>,
}

/// A named sub-team within a runbook (e.g. "Core Team"), its activation timing,
/// and its member agents BY SLUG.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RunbookGroup {
    pub group: String,
    pub activation: String,
    pub agents: Vec<String>,
}

fn validate_playbook_relative_path(value: &str) -> Result<String, AppError> {
    let normalized = crate::library::normalize_relative_path(value)?;
    let mut parts = normalized.split('/');
    let root = parts.next().unwrap_or_default();
    if !PLAYBOOK_ROOTS.contains(&root)
        || parts.next().is_none()
        || Path::new(&normalized)
            .extension()
            .and_then(|value| value.to_str())
            != Some("md")
    {
        return Err(AppError::InvalidArgument {
            message: "playbooks must be source-relative Markdown under strategy/ or examples/"
                .into(),
        });
    }
    Ok(normalized)
}

fn playbook_kind(relative_path: &str) -> PlaybookKind {
    if relative_path.starts_with("strategy/") {
        PlaybookKind::Strategy
    } else {
        PlaybookKind::Example
    }
}

fn playbook_title(relative_path: &str, content: &str) -> String {
    content
        .lines()
        .find_map(|line| {
            line.strip_prefix("# ").map(str::trim).filter(|title| {
                !title.is_empty() && title.chars().all(|character| !character.is_control())
            })
        })
        .map(|title| title.chars().take(256).collect())
        .or_else(|| {
            Path::new(relative_path)
                .file_stem()
                .and_then(|value| value.to_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| relative_path.to_owned())
}

fn validate_real_directory(path: &Path, label: &str) -> Result<PathBuf, AppError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| AppError::Io {
        message: format!("inspect {label}: {error}"),
    })?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || crate::skills::metadata_is_reparse_point(&metadata)
    {
        return Err(AppError::InvalidArgument {
            message: format!("{label} must be a real directory, not a link or reparse point"),
        });
    }
    std::fs::canonicalize(path).map_err(|error| AppError::Io {
        message: format!("resolve {label}: {error}"),
    })
}

fn read_playbook(root: &Path, relative_path: &str) -> Result<PlaybookDocument, AppError> {
    let relative_path = validate_playbook_relative_path(relative_path)?;
    let canonical_root = validate_real_directory(root, "catalog root")?;
    let mut candidate = root.to_path_buf();
    for component in relative_path.split('/') {
        candidate.push(component);
        let metadata = std::fs::symlink_metadata(&candidate).map_err(|error| AppError::Io {
            message: format!("inspect playbook {relative_path}: {error}"),
        })?;
        if metadata.file_type().is_symlink() || crate::skills::metadata_is_reparse_point(&metadata)
        {
            return Err(AppError::InvalidArgument {
                message: "playbook paths cannot contain links or reparse points".into(),
            });
        }
    }
    let metadata = std::fs::symlink_metadata(&candidate).map_err(|error| AppError::Io {
        message: format!("inspect playbook {relative_path}: {error}"),
    })?;
    if !metadata.is_file() || metadata.len() > MAX_PLAYBOOK_BYTES {
        return Err(AppError::InvalidArgument {
            message: format!(
                "playbook must be a regular file no larger than {MAX_PLAYBOOK_BYTES} bytes"
            ),
        });
    }
    let canonical = std::fs::canonicalize(&candidate).map_err(|error| AppError::Io {
        message: format!("resolve playbook {relative_path}: {error}"),
    })?;
    if !canonical.starts_with(&canonical_root) {
        return Err(AppError::InvalidArgument {
            message: "playbook resolved outside the active catalog".into(),
        });
    }
    let bytes = std::fs::read(&canonical).map_err(|error| AppError::Io {
        message: format!("read playbook {relative_path}: {error}"),
    })?;
    if bytes.len() as u64 > MAX_PLAYBOOK_BYTES {
        return Err(AppError::InvalidArgument {
            message: format!("playbook exceeds the {MAX_PLAYBOOK_BYTES}-byte limit"),
        });
    }
    let content = String::from_utf8(bytes).map_err(|_| AppError::InvalidArgument {
        message: "playbooks must be valid UTF-8".into(),
    })?;
    Ok(PlaybookDocument {
        title: playbook_title(&relative_path, &content),
        kind: playbook_kind(&relative_path),
        size_bytes: content.len() as u64,
        relative_path,
        content,
    })
}

fn playbook_catalog(root: &Path) -> Result<Vec<PlaybookCatalogEntry>, AppError> {
    validate_real_directory(root, "catalog root")?;
    let mut documents = Vec::new();
    for allowed_root in PLAYBOOK_ROOTS {
        let directory = root.join(allowed_root);
        let metadata = match std::fs::symlink_metadata(&directory) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(AppError::Io {
                    message: format!("inspect {allowed_root} playbooks: {error}"),
                });
            }
        };
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || crate::skills::metadata_is_reparse_point(&metadata)
        {
            return Err(AppError::InvalidArgument {
                message: format!("{allowed_root}/ must be a real directory"),
            });
        }
        let mut directories = VecDeque::from([directory]);
        while let Some(directory) = directories.pop_front() {
            let mut entries = std::fs::read_dir(&directory)
                .map_err(|error| AppError::Io {
                    message: format!("read {allowed_root} playbooks: {error}"),
                })?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| AppError::Io {
                    message: format!("read {allowed_root} playbook entry: {error}"),
                })?;
            entries.sort_by_key(std::fs::DirEntry::file_name);
            for entry in entries {
                let path = entry.path();
                let metadata = std::fs::symlink_metadata(&path).map_err(|error| AppError::Io {
                    message: format!("inspect {allowed_root} playbook entry: {error}"),
                })?;
                if metadata.file_type().is_symlink()
                    || crate::skills::metadata_is_reparse_point(&metadata)
                    || (!metadata.is_dir() && !metadata.is_file())
                {
                    return Err(AppError::InvalidArgument {
                        message: format!(
                            "{allowed_root}/ contains a link, reparse point, or special entry"
                        ),
                    });
                }
                let relative_path = normalized_corpus_relative_path(root, &path)?;
                if metadata.is_dir() {
                    directories.push_back(path);
                    continue;
                }
                if path.extension().and_then(|value| value.to_str()) != Some("md") {
                    continue;
                }
                if documents.len() >= MAX_PLAYBOOK_DOCUMENTS {
                    return Err(AppError::InvalidArgument {
                        message: format!(
                            "catalog contains more than {MAX_PLAYBOOK_DOCUMENTS} playbook documents"
                        ),
                    });
                }
                let document = read_playbook(root, &relative_path)?;
                documents.push(PlaybookCatalogEntry {
                    relative_path: document.relative_path,
                    title: document.title,
                    kind: document.kind,
                    size_bytes: document.size_bytes,
                });
            }
        }
    }
    documents.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(documents)
}

/// `runbooks_list()` — the NEXUS runbook manifest from the active catalog's
/// `strategy/runbooks.json`. Empty when the catalog is the bundled snapshot or an
/// unsynced/pre-#664 clone (no `strategy/` on disk) — the UI treats empty as
/// "sync to unlock", not an error. Local-only (no network).
#[tauri::command]
pub async fn runbooks_list(app: AppHandle) -> Result<Vec<Runbook>, AppError> {
    let adir = app_data_dir(&app)?;
    let source = load_catalog_source(&adir).await;
    let root = catalog_root(&adir, &source);
    let path = root.join("strategy").join("runbooks.json");
    let raw = match tokio::fs::read_to_string(&path).await {
        Ok(r) => r,
        Err(_) => return Ok(Vec::new()), // no strategy/ (bundled / unsynced) → empty
    };
    let file: RunbooksFile = serde_json::from_str(&raw).map_err(|e| AppError::Io {
        message: format!("parse strategy/runbooks.json: {e}"),
    })?;
    Ok(file.runbooks)
}

#[tauri::command]
pub async fn playbooks_list(app: AppHandle) -> Result<Vec<PlaybookCatalogEntry>, AppError> {
    let adir = app_data_dir(&app)?;
    let source = load_catalog_source_checked(&adir).await?;
    let root = catalog_root(&adir, &source);
    tauri::async_runtime::spawn_blocking(move || playbook_catalog(&root))
        .await
        .map_err(|error| AppError::Internal {
            message: format!("playbook catalog task failed: {error}"),
        })?
}

#[tauri::command]
pub async fn playbook_read(
    app: AppHandle,
    relative_path: String,
) -> Result<PlaybookDocument, AppError> {
    let adir = app_data_dir(&app)?;
    let source = load_catalog_source_checked(&adir).await?;
    let root = catalog_root(&adir, &source);
    tauri::async_runtime::spawn_blocking(move || read_playbook(&root, &relative_path))
        .await
        .map_err(|error| AppError::Internal {
            message: format!("playbook read task failed: {error}"),
        })?
}

/// Heuristic: does `root` hold an agency-agents catalog? True if it has the
/// repo tooling or at least one of the canonical category dirs with agents.
fn looks_like_catalog(root: &Path) -> bool {
    if root.join("scripts").join("convert.sh").exists() {
        return true;
    }
    bundled_division_meta()
        .keys()
        .any(|c| root.join(c).is_dir())
}

/// `corpus_list(category?)` — list view (bodies omitted).
#[tauri::command]
pub async fn corpus_list(
    app: AppHandle,
    state: State<'_, AppState>,
    category: Option<String>,
) -> Result<Vec<Agent>, AppError> {
    let corpus = ensure_corpus(&app, &state).await?;
    Ok(corpus.list(category.as_deref()))
}

/// `corpus_get(slug)` — full agent incl. body.
#[tauri::command]
pub async fn corpus_get(
    app: AppHandle,
    state: State<'_, AppState>,
    slug: String,
) -> Result<Agent, AppError> {
    let corpus = ensure_corpus(&app, &state).await?;
    corpus.get(&slug).ok_or(AppError::InvalidArgument {
        message: format!("unknown agent slug: {slug}"),
    })
}

/// `corpus_categories()` — the Discover grid (one tile per division declared
/// by the active catalog's tooling) with per-category counts.
#[tauri::command]
pub async fn corpus_categories(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<Category>, AppError> {
    let corpus = ensure_corpus(&app, &state).await?;
    Ok(corpus.categories())
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::settings::{Settings, SettingsLoadState};
    use crate::types::{
        AgentReference, BaselineRequirement, CatalogChange, CatalogFeedBatch,
        CatalogPendingRefresh, CatalogSnapshotItem, ControlCenterDocument,
        ProjectReadinessBaseline, ProjectSubscription,
    };

    fn test_app_state(app_data_dir: &Path, paranoid_mode: bool) -> AppState {
        AppState {
            app_data_dir: app_data_dir.to_path_buf(),
            corpus_cache: Arc::new(tokio::sync::Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(tokio::sync::Mutex::new(())),
            skill_sources_write_lock: Arc::new(tokio::sync::Mutex::new(())),
            skill_installs_write_lock: Arc::new(tokio::sync::Mutex::new(())),
            skill_folders_write_lock: Arc::new(tokio::sync::Mutex::new(())),
            settings: Arc::new(tokio::sync::RwLock::new(SettingsLoadState::Loaded(
                Settings {
                    paranoid_mode,
                    ..Settings::default()
                },
            ))),
            updater_state: crate::commands::updater::empty_state(),
        }
    }

    fn write_agent(dir: &Path, category: &str, slug: &str, name: &str, body: &str) {
        let cat = dir.join(category);
        std::fs::create_dir_all(&cat).unwrap();
        let content = format!("---\nname: {name}\ndescription: d\n---\n{body}\n");
        std::fs::write(cat.join(format!("{slug}.md")), content).unwrap();
    }

    fn test_catalog_tar(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut archive = tar::Builder::new(gz);
        for (path, bytes) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            archive
                .append_data(&mut header, path, *bytes)
                .expect("append test tar entry");
        }
        let gz = archive.into_inner().expect("finish tar");
        gz.finish().expect("finish gzip")
    }

    fn snapshot_item(category: &str, path: &str, hash: char) -> CatalogSnapshotItem {
        CatalogSnapshotItem {
            category: category.into(),
            relative_path: path.into(),
            source_hash: format!("{:064x}", hash as u32),
            body_hash: format!("{:064x}", hash as u32 + 1),
        }
    }

    async fn commit_catalog_refresh(
        database: &crate::state_db::StateDatabase,
        source_key: String,
        snapshot: Vec<CatalogSnapshotItem>,
        at: &str,
    ) -> Result<(), AppError> {
        begin_catalog_refresh(
            database,
            source_key.clone(),
            CatalogRefreshEntrypoint::CatalogPull,
            at.into(),
        )
        .await?;
        persist_catalog_refresh(database, source_key, snapshot, at.into()).await
    }

    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn control_center_validator_enforces_exact_document_and_field_caps() {
        assert!(validate_control_center(&ControlCenterDocument::default()).is_ok());

        let mut oversized_snapshot = ControlCenterDocument::default();
        oversized_snapshot.active_catalog_snapshot = (0..=CONTROL_CENTER_MAX_SNAPSHOT_ITEMS)
            .map(|index| {
                snapshot_item("engineering", &format!("engineering/agent-{index}.md"), 'a')
            })
            .collect();
        assert!(validate_control_center(&oversized_snapshot).is_err());

        let mut oversized_feed = ControlCenterDocument::default();
        oversized_feed.catalog_feed = vec![CatalogFeedBatch {
            at: "2026-08-17T00:00:00Z".into(),
            changes: (0..=CONTROL_CENTER_MAX_FEED_ITEMS)
                .map(|index| CatalogChange::Added {
                    item: snapshot_item(
                        "engineering",
                        &format!("engineering/agent-{index}.md"),
                        'a',
                    ),
                })
                .collect(),
        }];
        assert!(validate_control_center(&oversized_feed).is_err());

        let mut too_many_batches = ControlCenterDocument::default();
        too_many_batches.catalog_feed = (0..=CONTROL_CENTER_MAX_FEED_BATCHES)
            .map(|_| CatalogFeedBatch {
                at: "2026-08-17T00:00:00Z".into(),
                changes: Vec::new(),
            })
            .collect();
        assert!(validate_control_center(&too_many_batches).is_err());

        for invalid in [
            snapshot_item(&"x".repeat(257), "x/agent.md", 'a'),
            snapshot_item(
                "engineering",
                &format!("engineering/{}.md", "x".repeat(512)),
                'a',
            ),
            snapshot_item("engineering", "engineering/../agent.md", 'a'),
            CatalogSnapshotItem {
                source_hash: "z".repeat(64),
                ..snapshot_item("engineering", "engineering/agent.md", 'a')
            },
        ] {
            let document = ControlCenterDocument {
                active_catalog_snapshot: vec![invalid],
                ..ControlCenterDocument::default()
            };
            assert!(validate_control_center(&document).is_err());
        }
    }

    #[test]
    fn control_center_validator_enforces_project_readiness_caps_and_identity_shape() {
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
            instructions: vec![BaselineRequirement {
                id: "opaque requirement".into(),
                known: false,
            }],
            mcp_servers: Vec::new(),
            tools: vec!["codex".into()],
        };
        let valid = ControlCenterDocument {
            catalog_last_success_at: Some("2026-08-17T00:00:00Z".into()),
            project_baselines: vec![baseline.clone()],
            project_subscriptions: vec![ProjectSubscription {
                project_path: baseline.project_path.clone(),
                last_seen_batch: Some("2026-08-17T00:00:00Z".into()),
                pending_recommendation_ids: Vec::new(),
                dismissed_recommendation_ids: vec!["a".repeat(64)],
            }],
            ..ControlCenterDocument::default()
        };
        assert!(validate_control_center(&valid).is_ok());

        let mut arbitrary_identity = valid.clone();
        arbitrary_identity.project_baselines[0].project_path = "project nickname".into();
        assert!(validate_control_center(&arbitrary_identity).is_err());

        let mut future_cursor = valid.clone();
        future_cursor.project_subscriptions[0].last_seen_batch =
            Some("2026-08-17T01:00:00Z".into());
        assert!(validate_control_center(&future_cursor).is_err());

        let mut too_many_agents = valid.clone();
        too_many_agents.project_baselines[0].agents = (0..=CONTROL_CENTER_MAX_PROJECT_AGENTS)
            .map(|index| AgentReference {
                source_id: "source-a".into(),
                relative_path: format!("agent-{index}.md"),
            })
            .collect();
        assert!(validate_control_center(&too_many_agents).is_err());

        let mut too_many_skills = valid.clone();
        too_many_skills.project_baselines[0].skills = (0..=CONTROL_CENTER_MAX_PROJECT_SKILLS)
            .map(|index| crate::types::SkillReference {
                source_id: "skills".into(),
                relative_path: format!("skill-{index}"),
            })
            .collect();
        assert!(validate_control_center(&too_many_skills).is_err());

        let mut too_many_requirements = valid.clone();
        too_many_requirements.project_baselines[0].instructions = (0
            ..=CONTROL_CENTER_MAX_PROJECT_REQUIREMENTS)
            .map(|index| BaselineRequirement {
                id: format!("requirement-{index}"),
                known: false,
            })
            .collect();
        assert!(validate_control_center(&too_many_requirements).is_err());

        let mut too_many_mcp = valid.clone();
        too_many_mcp.project_baselines[0].mcp_servers = (0
            ..=CONTROL_CENTER_MAX_PROJECT_REQUIREMENTS)
            .map(|index| BaselineRequirement {
                id: format!("opaque-mcp-{index}"),
                known: false,
            })
            .collect();
        assert!(validate_control_center(&too_many_mcp).is_err());

        let mut too_many_tools = valid.clone();
        too_many_tools.project_baselines[0].tools =
            vec!["codex".into(); CONTROL_CENTER_MAX_PROJECT_TOOLS + 1];
        assert!(validate_control_center(&too_many_tools).is_err());

        let mut too_many_dismissals = valid.clone();
        too_many_dismissals.project_subscriptions[0].dismissed_recommendation_ids = (0
            ..=CONTROL_CENTER_MAX_DISMISSED_RECOMMENDATIONS)
            .map(|index| format!("{index:064x}"))
            .collect();
        assert!(validate_control_center(&too_many_dismissals).is_err());

        for invalid in [
            {
                let mut document = valid.clone();
                document.project_baselines[0].label = "x".repeat(CONTROL_CENTER_MAX_TEXT_CHARS + 1);
                document
            },
            {
                let mut document = valid.clone();
                document.project_baselines[0].project_path =
                    format!("/{}", "x".repeat(CONTROL_CENTER_MAX_PATH_CHARS));
                document
            },
            {
                let mut document = valid.clone();
                document.project_baselines[0].agents[0].relative_path =
                    format!("{}.md", "x".repeat(CONTROL_CENTER_MAX_PATH_CHARS));
                document
            },
            {
                let mut document = valid.clone();
                document.project_baselines[0].skills = vec![crate::types::SkillReference {
                    source_id: "skills".into(),
                    relative_path: "x".repeat(CONTROL_CENTER_MAX_PATH_CHARS + 1),
                }];
                document
            },
            {
                let mut document = valid.clone();
                document.project_baselines[0].instructions[0].id =
                    "x".repeat(CONTROL_CENTER_MAX_TEXT_CHARS + 1);
                document
            },
        ] {
            assert!(validate_control_center(&invalid).is_err());
        }
    }

    #[test]
    fn legacy_project_subscription_defaults_to_no_pending_recommendations() {
        let subscription: ProjectSubscription = serde_json::from_value(serde_json::json!({
            "projectPath": "/registered/project",
            "lastSeenBatch": "2026-08-17T00:00:00Z",
            "dismissedRecommendationIds": []
        }))
        .unwrap();

        assert!(subscription.pending_recommendation_ids.is_empty());
    }

    #[test]
    fn pending_recommendations_are_bounded_within_the_control_center_budget() {
        let pending = (0..CONTROL_CENTER_MAX_PENDING_RECOMMENDATIONS)
            .map(|index| format!("{index:064x}"))
            .collect::<Vec<_>>();
        let mut document = ControlCenterDocument::default();
        for index in 0..CONTROL_CENTER_MAX_SUBSCRIPTIONS {
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
                pending_recommendation_ids: pending.clone(),
                dismissed_recommendation_ids: Vec::new(),
            });
        }

        assert!(validate_control_center(&document).is_ok());
        assert!(serde_json::to_vec(&document).unwrap().len() < CONTROL_CENTER_MAX_BYTES as usize);

        document.project_subscriptions[0]
            .pending_recommendation_ids
            .push("f".repeat(64));
        assert!(validate_control_center(&document).is_err());

        document.project_subscriptions[0]
            .pending_recommendation_ids
            .pop();
        document.project_subscriptions[0].pending_recommendation_ids[1] =
            document.project_subscriptions[0].pending_recommendation_ids[0].clone();
        assert!(validate_control_center(&document).is_err());
    }

    fn empty_project_baseline(index: usize) -> ProjectReadinessBaseline {
        ProjectReadinessBaseline {
            project_path: format!("/registered/project-{index}"),
            label: format!("Project {index}"),
            agent_requirements: Vec::new(),
            skill_requirements: Vec::new(),
            agents: Vec::new(),
            skills: Vec::new(),
            instructions: Vec::new(),
            mcp_servers: Vec::new(),
            tools: Vec::new(),
        }
    }

    #[test]
    fn project_baseline_cap_fixture_fails_only_the_exact_65th_baseline_limit() {
        let mut document = ControlCenterDocument {
            project_baselines: (0..=CONTROL_CENTER_MAX_PROJECT_BASELINES)
                .map(empty_project_baseline)
                .collect(),
            ..ControlCenterDocument::default()
        };
        assert!(matches!(
            validate_control_center(&document),
            Err(AppError::InvalidArgument { message })
                if message == "control-center exceeds its project baseline limit"
        ));

        document.project_baselines.pop();
        assert_eq!(
            document.project_baselines.len(),
            CONTROL_CENTER_MAX_PROJECT_BASELINES
        );
        assert!(validate_control_center(&document).is_ok());
    }

    #[test]
    fn project_subscription_capacity_is_the_64_unique_baseline_capacity() {
        assert_eq!(
            CONTROL_CENTER_MAX_SUBSCRIPTIONS,
            CONTROL_CENTER_MAX_PROJECT_BASELINES
        );
        let baselines = (0..CONTROL_CENTER_MAX_PROJECT_BASELINES)
            .map(empty_project_baseline)
            .collect::<Vec<_>>();
        let subscriptions = baselines
            .iter()
            .map(|baseline| ProjectSubscription {
                project_path: baseline.project_path.clone(),
                last_seen_batch: None,
                pending_recommendation_ids: Vec::new(),
                dismissed_recommendation_ids: Vec::new(),
            })
            .collect::<Vec<_>>();
        let mut document = ControlCenterDocument {
            project_baselines: baselines,
            project_subscriptions: subscriptions,
            ..ControlCenterDocument::default()
        };
        assert!(validate_control_center(&document).is_ok());

        // There is no 65th unique subscription identity in the existing DTO:
        // project_path is the identity and every subscription must reference
        // one of the 64 unique baselines. The duplicate below therefore tests
        // the explicit early bound; removing it restores a fully valid maximum.
        document.project_subscriptions.push(ProjectSubscription {
            project_path: document.project_baselines[0].project_path.clone(),
            last_seen_batch: None,
            pending_recommendation_ids: Vec::new(),
            dismissed_recommendation_ids: Vec::new(),
        });
        assert!(matches!(
            validate_control_center(&document),
            Err(AppError::InvalidArgument { message })
                if message == "control-center exceeds its project subscription limit"
        ));

        document.project_subscriptions.pop();
        assert!(validate_control_center(&document).is_ok());
    }

    #[test]
    fn catalog_diff_is_deterministic_and_classifies_add_update_remove() {
        let old = vec![
            snapshot_item("design", "design/removed.md", 'r'),
            snapshot_item("engineering", "engineering/updated.md", 'a'),
            snapshot_item("engineering", "engineering/stable.md", 's'),
        ];
        let new = vec![
            snapshot_item("engineering", "engineering/added.md", 'n'),
            snapshot_item("engineering", "engineering/stable.md", 's'),
            snapshot_item("engineering", "engineering/updated.md", 'b'),
        ];

        let changes = diff_catalog_snapshots(&old, &new);
        assert_eq!(changes.len(), 3);
        assert!(matches!(changes[0], CatalogChange::Added { .. }));
        assert!(matches!(changes[1], CatalogChange::Updated { .. }));
        assert!(matches!(changes[2], CatalogChange::Removed { .. }));
        assert_eq!(changes, diff_catalog_snapshots(&old, &new));
    }

    #[test]
    fn catalog_diff_infers_only_one_unambiguous_same_content_rename() {
        let old = vec![snapshot_item("engineering", "engineering/old.md", 'a')];
        let new = vec![snapshot_item("engineering", "engineering/new.md", 'a')];
        assert!(matches!(
            diff_catalog_snapshots(&old, &new).as_slice(),
            [CatalogChange::Renamed { before, after }]
                if before.relative_path == "engineering/old.md"
                    && after.relative_path == "engineering/new.md"
        ));

        let ambiguous_old = vec![
            snapshot_item("engineering", "engineering/one.md", 'a'),
            snapshot_item("engineering", "engineering/two.md", 'a'),
        ];
        let ambiguous = diff_catalog_snapshots(&ambiguous_old, &new);
        assert_eq!(ambiguous.len(), 3);
        assert!(!ambiguous
            .iter()
            .any(|change| matches!(change, CatalogChange::Renamed { .. })));
    }

    #[tokio::test]
    async fn failed_feed_commit_keeps_old_snapshot_and_replays_the_diff() {
        let root = tempfile::tempdir().unwrap();
        let database = crate::state_db::StateDatabase::open(root.path()).unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let source_key = "a".repeat(64);
        let old = vec![snapshot_item("engineering", "engineering/old.md", 'a')];
        persist_catalog_baseline(&database, source_key.clone(), old.clone())
            .await
            .unwrap();
        commit_catalog_refresh(
            &database,
            source_key.clone(),
            old.clone(),
            "2026-08-17T00:00:00Z",
        )
        .await
        .unwrap();

        let invalid = vec![snapshot_item(&"x".repeat(257), "x/invalid.md", 'b')];
        begin_catalog_refresh(
            &database,
            source_key.clone(),
            CatalogRefreshEntrypoint::CatalogPull,
            "2026-08-17T00:01:00Z".into(),
        )
        .await
        .unwrap();
        assert!(persist_catalog_refresh(
            &database,
            source_key.clone(),
            invalid,
            "2026-08-17T00:01:00Z".into(),
        )
        .await
        .is_err());
        let retained = load_control_center(&database).await.unwrap();
        assert_eq!(retained.active_catalog_snapshot, old);
        assert_eq!(retained.catalog_feed.len(), 1);

        let new = vec![snapshot_item("engineering", "engineering/new.md", 'a')];
        mark_catalog_feed_stale(&database, "injected post-pull commit failure")
            .await
            .unwrap();
        ensure_catalog_baseline(&database, source_key.clone(), new.clone())
            .await
            .unwrap();
        assert_eq!(
            load_control_center(&database)
                .await
                .unwrap()
                .active_catalog_snapshot,
            old,
            "a stale same-source revision must retain the replay baseline"
        );
        persist_catalog_refresh(&database, source_key, new, "2026-08-17T00:02:00Z".into())
            .await
            .unwrap();
        let replayed = load_control_center(&database).await.unwrap();
        assert_eq!(replayed.catalog_feed.len(), 2);
        assert!(matches!(
            replayed.catalog_feed[1].changes.as_slice(),
            [CatalogChange::Renamed { .. }]
        ));
    }

    #[tokio::test]
    async fn process_crash_after_catalog_mutation_replays_from_the_retained_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let catalog = root.path().join("catalog");
        write_agent(&catalog, "engineering", "old", "Agent", "same body");
        let database = crate::state_db::StateDatabase::open(root.path()).unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let source_key = "a".repeat(64);
        let old = build_from_dir(&catalog, "test", &discover_categories(&catalog))
            .await
            .unwrap()
            .catalog_snapshot();
        persist_catalog_baseline(&database, source_key.clone(), old.clone())
            .await
            .unwrap();

        begin_catalog_refresh(
            &database,
            source_key.clone(),
            CatalogRefreshEntrypoint::CatalogPull,
            "2026-08-17T00:00:00Z".into(),
        )
        .await
        .unwrap();
        std::fs::rename(
            catalog.join("engineering/old.md"),
            catalog.join("engineering/new.md"),
        )
        .unwrap();
        let mutated = build_from_dir(&catalog, "test", &discover_categories(&catalog))
            .await
            .unwrap()
            .catalog_snapshot();
        // Simulate process death before the feed transaction by reopening
        // durable state after the real source-tree rename.
        let reopened = crate::state_db::StateDatabase::existing(root.path()).unwrap();
        ensure_catalog_baseline(&reopened, source_key.clone(), mutated.clone())
            .await
            .unwrap();
        let pending = load_control_center(&reopened).await.unwrap();
        assert_eq!(pending.active_catalog_snapshot, old);
        assert!(pending.catalog_pending_refresh.is_some());

        persist_catalog_refresh(
            &reopened,
            source_key,
            mutated,
            "2026-08-17T00:01:00Z".into(),
        )
        .await
        .unwrap();
        let recovered = load_control_center(&reopened).await.unwrap();
        assert!(recovered.catalog_pending_refresh.is_none());
        assert!(matches!(
            recovered.catalog_feed[0].changes.as_slice(),
            [CatalogChange::Renamed { .. }]
        ));
    }

    #[tokio::test]
    async fn pending_refresh_marker_is_bounded_and_bound_to_the_retained_baseline() {
        let root = tempfile::tempdir().unwrap();
        let database = crate::state_db::StateDatabase::open(root.path()).unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let source_key = "a".repeat(64);
        persist_catalog_baseline(&database, source_key.clone(), Vec::new())
            .await
            .unwrap();

        let result = database
            .mutate(
                control_center_spec(),
                ControlCenterDocument::default(),
                move |document| {
                    document.catalog_pending_refresh = Some(CatalogPendingRefresh {
                        source_key,
                        baseline_revision: "b".repeat(64),
                        command: "catalog_pull".into(),
                        started_at: "x".repeat(257),
                    });
                    Ok(())
                },
            )
            .await;

        assert!(matches!(result, Err(AppError::InvalidArgument { .. })));
    }

    #[tokio::test]
    async fn pending_source_transition_marker_is_bounded_and_bound_to_the_active_source() {
        let root = tempfile::tempdir().unwrap();
        let database = crate::state_db::StateDatabase::open(root.path()).unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let source_key = "a".repeat(64);
        persist_catalog_baseline(&database, source_key.clone(), Vec::new())
            .await
            .unwrap();

        let result = database
            .mutate(
                control_center_spec(),
                ControlCenterDocument::default(),
                move |document| {
                    document.catalog_pending_source_transition =
                        Some(CatalogPendingSourceTransition {
                            from_source_key: source_key,
                            to_source_key: "b".repeat(64),
                            started_at: "x".repeat(257),
                        });
                    document.catalog_stale = true;
                    Ok(())
                },
            )
            .await;

        assert!(matches!(result, Err(AppError::InvalidArgument { .. })));

        let result = database
            .mutate(
                control_center_spec(),
                ControlCenterDocument::default(),
                move |document| {
                    document.catalog_pending_source_transition =
                        Some(CatalogPendingSourceTransition {
                            from_source_key: "c".repeat(64),
                            to_source_key: "b".repeat(64),
                            started_at: "2026-08-17T00:00:00Z".into(),
                        });
                    document.catalog_stale = true;
                    Ok(())
                },
            )
            .await;

        assert!(matches!(result, Err(AppError::InvalidArgument { .. })));
    }

    #[tokio::test]
    async fn every_registered_catalog_refresh_entrypoint_is_durably_attributed() {
        for (entrypoint, expected) in [
            (CatalogRefreshEntrypoint::CorpusRefresh, "corpus_refresh"),
            (CatalogRefreshEntrypoint::CatalogPull, "catalog_pull"),
        ] {
            let root = tempfile::tempdir().unwrap();
            let database = crate::state_db::StateDatabase::open(root.path()).unwrap();
            database
                .set_migration_state(crate::types::StorageMigrationState::Complete)
                .await
                .unwrap();
            let source_key = "a".repeat(64);
            persist_catalog_baseline(&database, source_key.clone(), Vec::new())
                .await
                .unwrap();

            begin_catalog_refresh(
                &database,
                source_key,
                entrypoint,
                "2026-08-17T00:00:00Z".into(),
            )
            .await
            .unwrap();

            assert_eq!(
                load_control_center(&database)
                    .await
                    .unwrap()
                    .catalog_pending_refresh
                    .unwrap()
                    .command,
                expected
            );
        }
    }

    #[tokio::test]
    async fn first_pull_baselines_existing_catalog_without_an_all_added_batch() {
        let root = tempfile::tempdir().unwrap();
        let database = crate::state_db::StateDatabase::open(root.path()).unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let existing = vec![snapshot_item("engineering", "engineering/existing.md", 'a')];
        let refreshed = vec![
            snapshot_item("engineering", "engineering/existing.md", 'a'),
            snapshot_item("engineering", "engineering/new.md", 'b'),
        ];

        ensure_catalog_baseline(&database, "a".repeat(64), existing)
            .await
            .unwrap();
        commit_catalog_refresh(&database, "a".repeat(64), refreshed, "2026-08-17T00:00:00Z")
            .await
            .unwrap();

        let document = load_control_center(&database).await.unwrap();
        assert!(matches!(
            document.catalog_feed[0].changes.as_slice(),
            [CatalogChange::Added { item }] if item.relative_path == "engineering/new.md"
        ));
    }

    #[tokio::test]
    async fn source_switch_rebaselines_without_emitting_cross_source_changes() {
        let root = tempfile::tempdir().unwrap();
        let database = crate::state_db::StateDatabase::open(root.path()).unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let first = vec![snapshot_item("engineering", "engineering/first.md", 'a')];
        let second = vec![snapshot_item("design", "design/second.md", 'b')];

        ensure_catalog_baseline(&database, "a".repeat(64), first)
            .await
            .unwrap();
        commit_catalog_refresh(
            &database,
            "a".repeat(64),
            vec![snapshot_item("engineering", "engineering/changed.md", 'c')],
            "2026-08-17T00:00:00Z",
        )
        .await
        .unwrap();
        persist_catalog_baseline(&database, "b".repeat(64), second.clone())
            .await
            .unwrap();
        let switched = load_control_center(&database).await.unwrap();
        assert!(switched.catalog_feed.is_empty());
        assert_eq!(switched.catalog_last_success_at, None);
        commit_catalog_refresh(&database, "b".repeat(64), second, "2026-08-17T00:01:00Z")
            .await
            .unwrap();

        let document = load_control_center(&database).await.unwrap();
        assert_eq!(document.catalog_feed.len(), 1);
        assert!(document.catalog_feed[0].changes.is_empty());
    }

    #[tokio::test]
    async fn selecting_the_active_source_preserves_feed_history_and_last_success() {
        let root = tempfile::tempdir().unwrap();
        let database = crate::state_db::StateDatabase::open(root.path()).unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let source_key = "a".repeat(64);
        let snapshot = vec![snapshot_item("engineering", "engineering/agent.md", 'a')];
        persist_catalog_baseline(&database, source_key.clone(), snapshot.clone())
            .await
            .unwrap();
        commit_catalog_refresh(
            &database,
            source_key.clone(),
            snapshot.clone(),
            "2026-08-17T00:00:00Z",
        )
        .await
        .unwrap();
        let before = load_control_center(&database).await.unwrap();

        let mode = prepare_catalog_source_selection(
            &database,
            source_key.clone(),
            "2026-08-17T00:01:00Z".into(),
        )
        .await
        .unwrap();
        assert_eq!(mode, CatalogSourceSelection::Preserve);
        finish_catalog_source_selection(&database, source_key, snapshot, mode)
            .await
            .unwrap();

        let after = load_control_center(&database).await.unwrap();
        assert_eq!(after.catalog_feed, before.catalog_feed);
        assert_eq!(
            after.catalog_last_success_at,
            before.catalog_last_success_at
        );
    }

    #[tokio::test]
    async fn failed_source_rebuild_never_projects_old_source_history() {
        let root = tempfile::tempdir().unwrap();
        let database = crate::state_db::StateDatabase::open(root.path()).unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let source_a = "a".repeat(64);
        let source_b = "b".repeat(64);
        let old = vec![snapshot_item("engineering", "engineering/old.md", 'a')];
        persist_catalog_baseline(&database, source_a.clone(), old.clone())
            .await
            .unwrap();
        commit_catalog_refresh(&database, source_a, old, "2026-08-17T00:00:00Z")
            .await
            .unwrap();

        assert_eq!(
            prepare_catalog_source_selection(
                &database,
                source_b.clone(),
                "2026-08-17T00:01:00Z".into(),
            )
            .await
            .unwrap(),
            CatalogSourceSelection::Transition
        );
        // Source commit succeeds; rebuilding the selected source then fails.
        // The pending marker remains because finish is never called.
        let projected = catalog_feed_state_for_source(&database, &source_b)
            .await
            .unwrap();
        assert!(projected.stale);
        assert!(projected.error.is_some());
        assert_eq!(projected.last_success_at, None);
        assert!(projected.batches.is_empty());
        let retained = load_control_center(&database).await.unwrap();
        assert_eq!(
            retained.catalog_feed.len(),
            1,
            "old history remains durable for a deterministic retry"
        );
        assert_eq!(
            retained.catalog_last_success_at.as_deref(),
            Some("2026-08-17T00:00:00Z")
        );
    }

    #[tokio::test]
    async fn feed_projection_never_returns_history_for_mismatched_source_provenance() {
        let root = tempfile::tempdir().unwrap();
        let database = crate::state_db::StateDatabase::open(root.path()).unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let source_a = "a".repeat(64);
        let source_b = "b".repeat(64);
        let old = vec![snapshot_item("engineering", "engineering/old.md", 'a')];
        persist_catalog_baseline(&database, source_a.clone(), old.clone())
            .await
            .unwrap();
        commit_catalog_refresh(&database, source_a, old, "2026-08-17T00:00:00Z")
            .await
            .unwrap();

        let projected = catalog_feed_state_for_source(&database, &source_b)
            .await
            .unwrap();
        assert!(projected.stale);
        assert_eq!(
            projected.error.as_deref(),
            Some(CATALOG_SOURCE_TRANSITION_UNAVAILABLE)
        );
        assert_eq!(projected.last_success_at, None);
        assert!(projected.batches.is_empty());
    }

    #[tokio::test]
    async fn source_commit_crash_reopen_hides_mismatched_history() {
        let root = tempfile::tempdir().unwrap();
        let database = crate::state_db::StateDatabase::open(root.path()).unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let source_a = CatalogSource::Bundled;
        let source_b = CatalogSource::Managed {
            path: "/catalog-b".into(),
        };
        let source_a_key = catalog_source_key(&source_a);
        let source_b_key = catalog_source_key(&source_b);
        let old = vec![snapshot_item("engineering", "engineering/old.md", 'a')];
        persist_catalog_baseline(&database, source_a_key.clone(), old.clone())
            .await
            .unwrap();
        commit_catalog_refresh(&database, source_a_key, old, "2026-08-17T00:00:00Z")
            .await
            .unwrap();
        prepare_catalog_source_selection(
            &database,
            source_b_key.clone(),
            "2026-08-17T00:01:00Z".into(),
        )
        .await
        .unwrap();
        save_catalog_source(root.path(), &source_b).await.unwrap();

        let reopened = crate::state_db::StateDatabase::existing(root.path()).unwrap();
        let active = load_catalog_source(root.path()).await;
        assert_eq!(active, source_b);
        let projected = catalog_feed_state_for_source(&reopened, &catalog_source_key(&active))
            .await
            .unwrap();
        assert!(projected.stale);
        assert!(projected.batches.is_empty());
    }

    #[tokio::test]
    async fn source_transition_retry_clears_old_history_exactly_once() {
        let root = tempfile::tempdir().unwrap();
        let database = crate::state_db::StateDatabase::open(root.path()).unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let source_a = "a".repeat(64);
        let source_b = "b".repeat(64);
        let old = vec![snapshot_item("engineering", "engineering/old.md", 'a')];
        let new = vec![snapshot_item("design", "design/new.md", 'b')];
        persist_catalog_baseline(&database, source_a.clone(), old.clone())
            .await
            .unwrap();
        commit_catalog_refresh(&database, source_a, old, "2026-08-17T00:00:00Z")
            .await
            .unwrap();
        prepare_catalog_source_selection(
            &database,
            source_b.clone(),
            "2026-08-17T00:01:00Z".into(),
        )
        .await
        .unwrap();

        let retry_mode = prepare_catalog_source_selection(
            &database,
            source_b.clone(),
            "2026-08-17T00:02:00Z".into(),
        )
        .await
        .unwrap();
        assert_eq!(retry_mode, CatalogSourceSelection::Transition);
        finish_catalog_source_selection(&database, source_b.clone(), new.clone(), retry_mode)
            .await
            .unwrap();
        let completed = load_control_center(&database).await.unwrap();
        assert!(completed.catalog_feed.is_empty());
        assert!(completed.catalog_last_success_at.is_none());
        assert!(completed.catalog_pending_source_transition.is_none());

        commit_catalog_refresh(
            &database,
            source_b.clone(),
            new.clone(),
            "2026-08-17T00:03:00Z",
        )
        .await
        .unwrap();
        let same_mode = prepare_catalog_source_selection(
            &database,
            source_b.clone(),
            "2026-08-17T00:04:00Z".into(),
        )
        .await
        .unwrap();
        assert_eq!(same_mode, CatalogSourceSelection::Preserve);
        finish_catalog_source_selection(&database, source_b, new, same_mode)
            .await
            .unwrap();
        assert_eq!(
            load_control_center(&database)
                .await
                .unwrap()
                .catalog_feed
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn local_retry_recovers_old_side_even_when_network_is_blocked() {
        let root = tempfile::tempdir().unwrap();
        let database = crate::state_db::StateDatabase::open(root.path()).unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let catalog_a = root.path().join("catalog-a");
        let catalog_b = root.path().join("catalog-b");
        write_agent(&catalog_a, "engineering", "old", "Old", "old body");
        write_agent(&catalog_b, "design", "new", "New", "new body");
        let source_a = CatalogSource::Managed {
            path: catalog_a.to_string_lossy().into_owned(),
        };
        let source_b = CatalogSource::Managed {
            path: catalog_b.to_string_lossy().into_owned(),
        };
        save_catalog_source(root.path(), &source_a).await.unwrap();
        let old_snapshot = read_source_checked(root.path(), &source_a)
            .await
            .unwrap()
            .catalog_snapshot();
        persist_catalog_baseline(
            &database,
            catalog_source_key(&source_a),
            old_snapshot.clone(),
        )
        .await
        .unwrap();
        commit_catalog_refresh(
            &database,
            catalog_source_key(&source_a),
            old_snapshot,
            "2026-08-17T00:00:00Z",
        )
        .await
        .unwrap();
        let before = load_control_center(&database).await.unwrap();
        prepare_catalog_source_selection(
            &database,
            catalog_source_key(&source_b),
            "2026-08-17T00:01:00Z".into(),
        )
        .await
        .unwrap();

        let state = test_app_state(root.path(), true);
        assert!(state.require_network("catalog_pull").await.is_err());
        let recovered = catalog_source_transition_recover_state(&state)
            .await
            .unwrap();

        assert!(recovered);
        let restored = load_control_center(&database).await.unwrap();
        assert!(restored.catalog_pending_source_transition.is_none());
        assert_eq!(
            restored.active_catalog_provenance,
            before.active_catalog_provenance
        );
        assert_eq!(restored.catalog_feed, before.catalog_feed);
        assert_eq!(
            restored.catalog_last_success_at,
            before.catalog_last_success_at
        );
    }

    #[tokio::test]
    async fn local_retry_rebuilds_target_side_even_when_network_is_blocked() {
        let root = tempfile::tempdir().unwrap();
        let database = crate::state_db::StateDatabase::open(root.path()).unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let catalog_a = root.path().join("catalog-a");
        let catalog_b = root.path().join("catalog-b");
        write_agent(&catalog_a, "engineering", "old", "Old", "old body");
        write_agent(&catalog_b, "design", "new", "New", "new body");
        let source_a = CatalogSource::Managed {
            path: catalog_a.to_string_lossy().into_owned(),
        };
        let source_b = CatalogSource::Managed {
            path: catalog_b.to_string_lossy().into_owned(),
        };
        save_catalog_source(root.path(), &source_a).await.unwrap();
        let old_snapshot = read_source_checked(root.path(), &source_a)
            .await
            .unwrap()
            .catalog_snapshot();
        persist_catalog_baseline(
            &database,
            catalog_source_key(&source_a),
            old_snapshot.clone(),
        )
        .await
        .unwrap();
        commit_catalog_refresh(
            &database,
            catalog_source_key(&source_a),
            old_snapshot,
            "2026-08-17T00:00:00Z",
        )
        .await
        .unwrap();
        prepare_catalog_source_selection(
            &database,
            catalog_source_key(&source_b),
            "2026-08-17T00:01:00Z".into(),
        )
        .await
        .unwrap();
        save_catalog_source(root.path(), &source_b).await.unwrap();

        let state = test_app_state(root.path(), true);
        assert!(state.require_network("catalog_pull").await.is_err());
        let recovered = catalog_source_transition_recover_state(&state)
            .await
            .unwrap();

        assert!(recovered);
        assert!(state
            .corpus_cache
            .lock()
            .await
            .as_ref()
            .is_some_and(|corpus| corpus.get("new").is_some()));
        assert!(
            index_path(root.path()).is_file(),
            "target rebuild persisted its index"
        );
        let completed = load_control_center(&database).await.unwrap();
        assert!(completed.catalog_pending_source_transition.is_none());
        assert_eq!(
            completed
                .active_catalog_provenance
                .as_ref()
                .map(|provenance| provenance.source_key.as_str()),
            Some(catalog_source_key(&source_b).as_str())
        );
        assert!(completed.catalog_feed.is_empty());
        assert!(completed.catalog_last_success_at.is_none());
    }

    #[tokio::test]
    async fn retry_keeps_history_hidden_when_active_source_matches_neither_transition_side() {
        let root = tempfile::tempdir().unwrap();
        let database = crate::state_db::StateDatabase::open(root.path()).unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let source_a = "a".repeat(64);
        let source_b = "b".repeat(64);
        persist_catalog_baseline(&database, source_a, Vec::new())
            .await
            .unwrap();
        prepare_catalog_source_selection(&database, source_b, "2026-08-17T00:01:00Z".into())
            .await
            .unwrap();
        let catalog_c = root.path().join("catalog-c");
        write_agent(&catalog_c, "engineering", "other", "Other", "other body");
        let source_c = CatalogSource::Managed {
            path: catalog_c.to_string_lossy().into_owned(),
        };

        save_catalog_source(root.path(), &source_c).await.unwrap();
        let state = test_app_state(root.path(), true);
        let error = catalog_source_transition_recover_state(&state)
            .await
            .unwrap_err();

        let AppError::InvalidArgument { message } = error else {
            panic!("expected a bounded invalid-argument recovery error");
        };
        assert!(message.contains("matches neither"));
        assert!(message.chars().count() <= CONTROL_CENTER_MAX_TEXT_CHARS);
        let retained = load_control_center(&database).await.unwrap();
        assert!(retained.catalog_pending_source_transition.is_some());
        let projected = catalog_feed_state_for_source(&database, &catalog_source_key(&source_c))
            .await
            .unwrap();
        assert!(projected.stale);
        assert!(projected.batches.is_empty());
    }

    #[tokio::test]
    async fn local_retry_without_a_pending_transition_is_an_explicit_no_op() {
        let root = tempfile::tempdir().unwrap();
        let database = crate::state_db::StateDatabase::open(root.path()).unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let state = test_app_state(root.path(), true);

        let recovered = catalog_source_transition_recover_state(&state)
            .await
            .unwrap();

        assert!(!recovered);
        assert!(state.corpus_cache.lock().await.is_none());
    }

    #[tokio::test]
    async fn same_source_refresh_diffs_against_its_previous_successful_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let database = crate::state_db::StateDatabase::open(root.path()).unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let first = vec![snapshot_item("engineering", "engineering/agent.md", 'a')];
        let second = vec![snapshot_item("engineering", "engineering/agent.md", 'b')];

        ensure_catalog_baseline(&database, "a".repeat(64), first)
            .await
            .unwrap();
        commit_catalog_refresh(&database, "a".repeat(64), second, "2026-08-17T00:00:00Z")
            .await
            .unwrap();

        let document = load_control_center(&database).await.unwrap();
        assert!(matches!(
            document.catalog_feed[0].changes.as_slice(),
            [CatalogChange::Updated { .. }]
        ));
    }

    #[tokio::test]
    async fn stale_refresh_retains_last_success_and_feed() {
        let root = tempfile::tempdir().unwrap();
        let database = crate::state_db::StateDatabase::open(root.path()).unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let source_key = "a".repeat(64);
        persist_catalog_baseline(
            &database,
            source_key.clone(),
            vec![snapshot_item("engineering", "engineering/agent.md", 'a')],
        )
        .await
        .unwrap();
        commit_catalog_refresh(
            &database,
            source_key.clone(),
            vec![snapshot_item("engineering", "engineering/agent.md", 'a')],
            "2026-08-17T00:00:00Z",
        )
        .await
        .unwrap();

        mark_catalog_feed_stale(&database, "pull failed")
            .await
            .unwrap();

        let state = catalog_feed_state_for_source(&database, &source_key)
            .await
            .unwrap();
        assert_eq!(
            state.last_success_at.as_deref(),
            Some("2026-08-17T00:00:00Z")
        );
        assert_eq!(state.batches.len(), 1);
        assert!(state.stale);
        assert_eq!(state.error.as_deref(), Some("pull failed"));
    }

    #[tokio::test]
    async fn build_indexes_agents_in_stable_order() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        // Write out of order across two categories.
        write_agent(dir, "engineering", "zeta", "Zeta", "z");
        write_agent(dir, "engineering", "alpha", "Alpha", "a");
        write_agent(dir, "design", "mid", "Mid", "m");

        let corpus = build_from_dir(dir, "test", &discover_categories(dir))
            .await
            .unwrap();
        assert_eq!(corpus.count(), 3);
        // design < engineering, and within engineering alpha < zeta.
        let order: Vec<&str> = corpus.agents.iter().map(|a| a.slug.as_str()).collect();
        assert_eq!(order, vec!["mid", "alpha", "zeta"]);
    }

    #[tokio::test]
    async fn build_indexes_nested_agents() {
        // Real clones nest agents in subdirs (game-development/godot/<slug>.md).
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        write_agent(dir, "engineering", "flat-one", "Flat One", "x");
        let nested = dir.join("game-development").join("godot");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(
            nested.join("godot-shader-developer.md"),
            "---\nname: Godot Shader Developer\ndescription: d\n---\nbody\n",
        )
        .unwrap();

        let corpus = build_from_dir(dir, "v", &discover_categories(dir))
            .await
            .unwrap();
        let nested_agent = corpus.get("godot-shader-developer");
        assert!(nested_agent.is_some(), "nested agent must be indexed");
        assert_eq!(
            nested_agent.unwrap().category,
            "game-development",
            "category is the top-level dir, not the subdir"
        );
        assert!(corpus.active_catalog_snapshot.iter().any(|item| {
            item.category == "game-development"
                && item.relative_path == "game-development/godot/godot-shader-developer.md"
        }));
        assert!(corpus.get("flat-one").is_some(), "flat agent still indexed");
    }

    #[tokio::test]
    async fn nested_agents_with_the_same_slug_do_not_collapse_the_catalog() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        for group in ["godot", "unity"] {
            let nested = dir.join("game-development").join(group);
            std::fs::create_dir_all(&nested).unwrap();
            std::fs::write(
                nested.join("shader-developer.md"),
                format!("---\nname: {group} Shader Developer\ndescription: d\n---\nbody\n"),
            )
            .unwrap();
        }

        let corpus = build_from_dir(dir, "v", &discover_categories(dir))
            .await
            .unwrap();

        assert_eq!(corpus.agents.len(), 2);
        assert_eq!(
            corpus.count(),
            2,
            "distinct nested files are distinct agents"
        );
        assert!(
            corpus.get("shader-developer").is_none(),
            "legacy slug lookup must not choose silently between identities"
        );
    }

    #[test]
    fn legacy_recursive_source_lookup_refuses_ambiguous_filenames() {
        let tmp = tempfile::tempdir().unwrap();
        for group in ["godot", "unity"] {
            let nested = tmp.path().join(group);
            std::fs::create_dir(&nested).unwrap();
            std::fs::write(nested.join("reviewer.md"), "agent").unwrap();
        }
        assert!(find_md_under(tmp.path(), "reviewer.md").is_none());
    }

    #[tokio::test]
    async fn index_json_is_byte_stable_across_builds() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        write_agent(dir, "engineering", "alpha", "Alpha", "a");
        write_agent(dir, "design", "mid", "Mid", "m");

        let cats = discover_categories(dir);
        let a = build_from_dir(dir, "v", &cats)
            .await
            .unwrap()
            .index_json()
            .unwrap();
        let b = build_from_dir(dir, "v", &cats)
            .await
            .unwrap()
            .index_json()
            .unwrap();
        assert_eq!(a, b, "corpus-index.json must be deterministic");
    }

    #[tokio::test]
    async fn list_omits_body_get_includes_it() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        write_agent(dir, "engineering", "alpha", "Alpha", "the persona body");
        let corpus = build_from_dir(dir, "v", &discover_categories(dir))
            .await
            .unwrap();

        let listed = corpus.list(None);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].body, "", "list view must omit body");

        let full = corpus.get("alpha").unwrap();
        assert!(
            full.body.contains("the persona body"),
            "get must include body"
        );
    }

    #[tokio::test]
    async fn list_filters_by_category() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        write_agent(dir, "engineering", "alpha", "Alpha", "a");
        write_agent(dir, "design", "mid", "Mid", "m");
        let corpus = build_from_dir(dir, "v", &discover_categories(dir))
            .await
            .unwrap();

        let eng = corpus.list(Some("engineering"));
        assert_eq!(eng.len(), 1);
        assert_eq!(eng[0].slug, "alpha");
    }

    #[tokio::test]
    async fn categories_returns_all_divisions_with_counts() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        write_agent(dir, "engineering", "alpha", "Alpha", "a");
        write_agent(dir, "engineering", "beta", "Beta", "b");
        // No divisions.json in this tempdir → discover falls back to the bundled floor.
        let corpus = build_from_dir(dir, "v", &discover_categories(dir))
            .await
            .unwrap();

        let cats = corpus.categories();
        assert_eq!(cats.len(), 17, "all declared divisions always returned");
        let eng = cats.iter().find(|c| c.slug == "engineering").unwrap();
        assert_eq!(eng.count, 2);
        assert_eq!(eng.label, "Engineering");
        assert_eq!(eng.icon, "Code");
        // Empty category still present with count 0.
        let fin = cats.iter().find(|c| c.slug == "finance").unwrap();
        assert_eq!(fin.count, 0);
        // `healthcare` is a declared division (empty here, count 0). `strategy`
        // is NOT (it holds playbooks/runbooks, not agents) and `integrations` is
        // NOT (it's convert.sh output) — neither may appear as a division.
        let hc = cats.iter().find(|c| c.slug == "healthcare").unwrap();
        assert_eq!(hc.count, 0);
        assert!(
            !cats.iter().any(|c| c.slug == "strategy"),
            "strategy is not a division"
        );
        assert!(
            !cats.iter().any(|c| c.slug == "integrations"),
            "integrations is not a division"
        );
    }

    #[tokio::test]
    async fn non_agent_files_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        write_agent(dir, "engineering", "real", "Real", "x");
        // A README with no frontmatter.
        let cat = dir.join("engineering");
        std::fs::write(cat.join("README.md"), "# Examples\nnope\n").unwrap();
        // A workflow doc with no frontmatter.
        std::fs::write(cat.join("workflow.md"), "# Workflow\nnope\n").unwrap();

        let corpus = build_from_dir(dir, "v", &discover_categories(dir))
            .await
            .unwrap();
        assert_eq!(corpus.count(), 1);
        assert_eq!(corpus.active_catalog_snapshot.len(), 1);
        assert!(corpus.get("real").is_some());
        assert!(corpus.get("workflow").is_none());
    }

    #[tokio::test]
    async fn seed_then_build_round_trips() {
        let baseline = tempfile::tempdir().unwrap();
        write_agent(baseline.path(), "engineering", "alpha", "Alpha", "a");
        write_agent(baseline.path(), "design", "mid", "Mid", "m");

        let app_data = tempfile::tempdir().unwrap();
        let corpus = resolve_active(app_data.path(), baseline.path()).await;
        assert_eq!(corpus.count(), 2);
        // Working copy + index were written.
        assert!(corpus_dir(app_data.path())
            .join("engineering/alpha.md")
            .exists());
        assert!(index_path(app_data.path()).exists());
        assert!(meta_path(app_data.path()).exists());
    }

    #[test]
    fn title_case_handles_hyphens() {
        assert_eq!(title_case("game-development"), "Game Development");
        assert_eq!(title_case("engineering"), "Engineering");
    }

    #[test]
    fn category_meta_resolves_from_bundled_json() {
        let bundled = bundled_division_meta();
        let (label, icon, color) = category_meta_from(&bundled, "engineering");
        assert_eq!(label, "Engineering");
        assert_eq!(icon, "Code");
        assert_eq!(color, "#3B82F6");
    }

    #[test]
    fn category_meta_falls_back_for_unknown_slug() {
        let bundled = bundled_division_meta();
        let (label, icon, color) = category_meta_from(&bundled, "made-up-division");
        assert_eq!(label, "Made Up Division");
        assert_eq!(icon, "Folder");
        assert_eq!(color, default_division_color());
    }

    #[test]
    fn load_division_meta_missing_file_uses_bundled() {
        // First-run / pre-#592 clone: no divisions.json at the root → bundled.
        let root = tempfile::tempdir().unwrap();
        let meta = load_division_meta(root.path());
        assert_eq!(meta.get("engineering").unwrap().color, "#3B82F6");
    }

    #[test]
    fn load_division_meta_overlays_catalog_divisions_json() {
        // A catalog divisions.json overrides a known division AND introduces a
        // brand-new one the bundled floor has never heard of (the whole point:
        // a new catalog division presents correctly without an app update).
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join(DIVISIONS_FILENAME),
            r##"{ "divisions": {
                "engineering": { "label": "Engineering", "icon": "Cpu", "color": "#000000" },
                "robotics":    { "label": "Robotics",    "icon": "Bot", "color": "#FF00FF" }
            } }"##,
        )
        .unwrap();
        let meta = load_division_meta(root.path());
        // Overridden from the catalog.
        let eng = meta.get("engineering").unwrap();
        assert_eq!((eng.icon.as_str(), eng.color.as_str()), ("Cpu", "#000000"));
        // Net-new division, present only in the catalog.
        assert_eq!(meta.get("robotics").unwrap().color, "#FF00FF");
        // A bundled division the catalog file omitted is retained (overlay, not replace).
        assert_eq!(meta.get("marketing").unwrap().label, "Marketing");
    }

    #[test]
    fn load_division_meta_malformed_file_uses_bundled() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(DIVISIONS_FILENAME), "{ not valid json ").unwrap();
        let meta = load_division_meta(root.path());
        assert_eq!(meta.get("engineering").unwrap().color, "#3B82F6");
    }

    /// Parse the REAL bundled baseline corpus (not a synthetic tempdir) so a
    /// malformed real agent (bad frontmatter fence, missing `name`) fails CI
    /// rather than shipping. `cargo test` runs with cwd = crate root, so the
    /// relative resource path resolves. Divisions come from the bundled floor
    /// (`agency-categories.json`, a mirror of the catalog's `divisions.json`), so
    /// `strategy/` (playbooks/runbooks) is NOT a division and `integrations/`
    /// (convert.sh output) is NOT either. Counts are pinned to the agency-agents
    /// snapshot — bump them on a corpus refresh.
    #[tokio::test]
    async fn real_bundled_baseline_parses_completely() {
        let dir = Path::new("resources/corpus-baseline");
        if !dir.exists() {
            // Resources not present in this build context — skip rather than fail.
            return;
        }
        // Divisions come from the bundled floor (no divisions.json in the baseline).
        let categories = discover_categories(dir);
        assert!(
            !categories.iter().any(|c| c == "strategy"),
            "strategy is not a division"
        );
        assert!(
            !categories.iter().any(|c| c == "integrations"),
            "integrations is convert.sh output, not a division"
        );

        let corpus = build_from_dir(dir, "baseline-test", &categories)
            .await
            .unwrap();

        // 209 = 210 prior minus the lone `integrations/` artifact
        // (backend-architect-with-memory), which is convert.sh output, not a
        // catalog persona.
        assert_eq!(
            corpus.count(),
            209,
            "all bundled agent personas indexed (integrations excluded)"
        );

        // Every agent parsed real frontmatter: non-empty name + slug, real category.
        for a in &corpus.agents {
            assert!(!a.name.trim().is_empty(), "agent {} has empty name", a.slug);
            assert!(!a.slug.trim().is_empty(), "agent has empty slug");
            assert!(
                categories.contains(&a.category),
                "agent {} has unknown category {}",
                a.slug,
                a.category
            );
        }

        // Spot-check categories that nest agents in subdirs upstream — these are
        // the ones a flat seeding would silently undercount.
        let cats = corpus.categories();
        assert_eq!(cats.len(), 17, "17 declared divisions");
        let count_of = |slug: &str| {
            cats.iter()
                .find(|c| c.slug == slug)
                .map(|c| c.count)
                .unwrap_or(0)
        };
        assert_eq!(count_of("engineering"), 30);
        assert_eq!(count_of("specialized"), 46);
        // game-development nests agents in unity/, godot/, unreal-engine/ etc.
        // upstream; a flat seeding would silently undercount these.
        assert_eq!(
            count_of("game-development"),
            20,
            "nested game-dev agents included"
        );
        // strategy is NOT a division (playbooks/runbooks, no agent frontmatter),
        // so it never appears as one — regardless of what's on disk.
        assert!(
            !cats.iter().any(|c| c.slug == "strategy"),
            "strategy is not a division"
        );
        // healthcare IS a declared division; the bundled baseline predates its
        // agents, so it's present but empty (count 0) until a sync brings them in.
        assert_eq!(
            count_of("healthcare"),
            0,
            "healthcare present but empty in the stale baseline"
        );
    }

    #[test]
    fn parse_agent_dirs_reads_the_bash_array() {
        let script = r#"
# preamble
ALL_TOOLS=(claude-code copilot)
AGENT_DIRS=(
  academic design engineering   # inline comment ignored
  finance strategy
)
echo done
"#;
        let cats = parse_agent_dirs(script).unwrap();
        assert_eq!(
            cats,
            vec!["academic", "design", "engineering", "finance", "strategy"]
        );
        assert!(!cats.contains(&"integrations".to_string()));
    }

    #[test]
    fn parse_agent_dirs_none_when_absent() {
        assert!(parse_agent_dirs("nothing here").is_none());
    }

    #[tokio::test]
    async fn conversion_slug_resolves_filename_prefixed_agent() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir.join("engineering")).unwrap();
        std::fs::write(
            dir.join("engineering/engineering-frontend-developer.md"),
            "---\nname: Frontend Developer\ndescription: Builds UIs.\n---\nBody\n",
        )
        .unwrap();
        let corpus = build_from_dir(dir, "v", &["engineering".into()])
            .await
            .unwrap();

        let agent = corpus
            .get_by_conversion_slug("frontend-developer")
            .expect("convert.sh filename resolves");
        assert_eq!(agent.slug, "engineering-frontend-developer");
    }

    #[tokio::test]
    async fn catalog_source_persists_and_defaults_bundled() {
        let app_data = tempfile::tempdir().unwrap();
        // No file yet → default Bundled.
        assert_eq!(
            load_catalog_source(app_data.path()).await,
            CatalogSource::Bundled
        );

        let src = CatalogSource::Managed {
            path: "/Users/x/.agency-agents".into(),
        };
        save_catalog_source(app_data.path(), &src).await.unwrap();
        assert_eq!(load_catalog_source(app_data.path()).await, src);

        // catalog.json is valid camelCase-tagged JSON.
        let bytes = std::fs::read(catalog_source_path(app_data.path())).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            text.contains("\"kind\": \"managed\""),
            "tagged on kind: {text}"
        );
    }

    #[tokio::test]
    async fn checked_catalog_source_rejects_corrupt_local_state() {
        let app_data = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(state_dir(app_data.path())).unwrap();
        std::fs::write(catalog_source_path(app_data.path()), b"{not json").unwrap();
        assert!(load_catalog_source_checked(app_data.path()).await.is_err());
    }

    #[test]
    fn catalog_root_resolves_per_source() {
        let app_data = Path::new("/app/data");
        assert_eq!(
            catalog_root(app_data, &CatalogSource::Bundled),
            corpus_dir(app_data)
        );
        assert_eq!(
            catalog_root(
                app_data,
                &CatalogSource::Managed {
                    path: "/home/x/.agency-agents".into()
                }
            ),
            PathBuf::from("/home/x/.agency-agents")
        );
        assert_eq!(
            catalog_root(
                app_data,
                &CatalogSource::UserClone {
                    path: "/src/aa".into(),
                    manage: true
                }
            ),
            PathBuf::from("/src/aa")
        );
    }

    #[test]
    fn looks_like_catalog_detects_tooling_or_categories() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(
            !looks_like_catalog(tmp.path()),
            "empty dir is not a catalog"
        );
        // A category dir is enough.
        std::fs::create_dir_all(tmp.path().join("engineering")).unwrap();
        assert!(looks_like_catalog(tmp.path()));
        // …or the tooling.
        let tmp2 = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp2.path().join("scripts")).unwrap();
        std::fs::write(
            tmp2.path().join("scripts/convert.sh"),
            "AGENT_DIRS=(engineering)\n",
        )
        .unwrap();
        assert!(looks_like_catalog(tmp2.path()));
    }

    #[test]
    fn quick_count_and_candidate_from_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // Not a catalog yet.
        assert!(candidate_for(root, "userClone").is_none());

        write_agent(root, "engineering", "a", "A", "x");
        write_agent(root, "engineering", "b", "B", "y");
        write_agent(root, "design", "c", "C", "z");
        std::fs::write(root.join("engineering/README.md"), "# readme").unwrap();

        assert_eq!(quick_agent_count(root), 3, "README excluded; 3 real agents");
        let cand = candidate_for(root, "userClone").unwrap();
        assert_eq!(cand.kind, "userClone");
        assert_eq!(cand.agent_count, 3);
        assert!(!cand.has_git, "no .git in this tempdir");
    }

    #[test]
    fn discover_categories_falls_back_to_bundled_floor_without_divisions_json() {
        let tmp = tempfile::tempdir().unwrap();
        let cats = discover_categories(tmp.path());
        // No divisions.json → the bundled floor (agency-categories.json) keys.
        assert_eq!(cats, bundled_division_slugs());
        assert!(cats.contains(&"healthcare".to_string()) && cats.contains(&"gis".to_string()));
        assert!(
            !cats.contains(&"strategy".to_string()),
            "no phantom strategy division"
        );
    }

    #[test]
    fn discover_categories_reads_divisions_json() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(DIVISIONS_FILENAME),
            r##"{"divisions":{"healthcare":{"label":"Healthcare","icon":"Stethoscope","color":"#0D9488"},"engineering":{"label":"Engineering","icon":"Code","color":"#3B82F6"}}}"##,
        )
        .unwrap();
        // The active catalog's divisions.json is authoritative — its keys, sorted.
        let cats = discover_categories(tmp.path());
        assert_eq!(
            cats,
            vec!["engineering".to_string(), "healthcare".to_string()]
        );
        assert!(!cats.contains(&"strategy".to_string()));
    }

    #[test]
    fn runbooks_manifest_parses_and_defaults_empty() {
        let raw = r#"{"runbooks":[{"slug":"startup-mvp","title":"Startup MVP Build","mode":"NEXUS-Sprint","duration":"4-6 weeks","summary":"Idea to live.","doc":"strategy/runbooks/scenario-startup-mvp.md","roster":[{"group":"Core Team","activation":"always","agents":["agents-orchestrator","engineering-frontend-developer"]}]}]}"#;
        let file: RunbooksFile = serde_json::from_str(raw).unwrap();
        assert_eq!(file.runbooks.len(), 1);
        let rb = &file.runbooks[0];
        assert_eq!(rb.slug, "startup-mvp");
        assert_eq!(rb.mode, "NEXUS-Sprint");
        assert_eq!(rb.roster[0].agents.len(), 2);
        assert!(rb.roster[0]
            .agents
            .contains(&"engineering-frontend-developer".to_string()));
        // An absent `runbooks` key (bundled / no strategy/) parses to empty, not an error.
        let empty: RunbooksFile = serde_json::from_str("{}").unwrap();
        assert!(empty.runbooks.is_empty());
    }

    #[test]
    fn managed_tar_retains_only_supported_playbook_content() {
        let tar = test_catalog_tar(&[
            (
                "agency-agents-main/engineering/reviewer.md",
                b"---\nname: Reviewer\n---\nBody\n",
            ),
            (
                "agency-agents-main/strategy/runbooks.json",
                br#"{"runbooks":[]}"#,
            ),
            (
                "agency-agents-main/strategy/QUICKSTART.md",
                b"# Quickstart\n",
            ),
            (
                "agency-agents-main/strategy/runbooks/scenario.md",
                b"# Scenario\n",
            ),
            ("agency-agents-main/examples/README.md", b"# Examples\n"),
            ("agency-agents-main/examples/workflow.md", b"# Workflow\n"),
            ("agency-agents-main/examples/execute.sh", b"echo unsafe\n"),
            ("agency-agents-main/docs/private.md", b"# Private\n"),
        ]);
        let dest = tempfile::tempdir().unwrap();

        assert_eq!(
            extract_categories(&tar, dest.path(), &["engineering".into()]).unwrap(),
            1
        );
        for retained in [
            "strategy/runbooks.json",
            "strategy/QUICKSTART.md",
            "strategy/runbooks/scenario.md",
            "examples/README.md",
            "examples/workflow.md",
        ] {
            assert!(dest.path().join(retained).is_file(), "retained {retained}");
        }
        assert!(!dest.path().join("examples/execute.sh").exists());
        assert!(!dest.path().join("docs/private.md").exists());
    }

    #[test]
    fn managed_tar_rejects_linked_playbook_entries() {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut archive = tar::Builder::new(gz);
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_link_name("../../private.md").unwrap();
        header.set_cksum();
        archive
            .append_data(
                &mut header,
                "agency-agents-main/strategy/private.md",
                std::io::empty(),
            )
            .unwrap();
        let tar = archive.into_inner().unwrap().finish().unwrap();
        let dest = tempfile::tempdir().unwrap();
        assert!(extract_categories(&tar, dest.path(), &["engineering".into()]).is_err());
        assert!(!dest.path().join("strategy/private.md").exists());
    }

    #[cfg(unix)]
    #[test]
    fn managed_tar_rejects_linked_destination_paths() {
        use std::os::unix::fs::symlink;

        let tar = test_catalog_tar(&[("agency-agents-main/strategy/plan.md", b"# Plan\n")]);
        let dest = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), dest.path().join("strategy")).unwrap();

        assert!(extract_categories(&tar, dest.path(), &["engineering".into()]).is_err());
        assert!(!outside.path().join("plan.md").exists());
    }

    #[tokio::test]
    async fn managed_refresh_replaces_the_complete_snapshot_and_removes_deleted_docs() {
        let app_data = tempfile::tempdir().unwrap();
        let live = corpus_dir(app_data.path());
        std::fs::create_dir_all(live.join("strategy")).unwrap();
        std::fs::write(live.join("strategy/deleted.md"), "# Deleted\n").unwrap();
        let tar = test_catalog_tar(&[
            (
                "agency-agents-main/engineering/reviewer.md",
                b"---\nname: Reviewer\n---\nBody\n",
            ),
            ("agency-agents-main/strategy/current.md", b"# Current\n"),
        ]);

        refresh_from_tarball(app_data.path(), &CatalogSource::Bundled, &tar)
            .await
            .unwrap();

        assert!(!live.join("strategy/deleted.md").exists());
        assert_eq!(
            std::fs::read_to_string(live.join("strategy/current.md")).unwrap(),
            "# Current\n"
        );
    }

    #[tokio::test]
    async fn failed_managed_refresh_leaves_the_previous_snapshot_byte_identical() {
        let app_data = tempfile::tempdir().unwrap();
        let live = corpus_dir(app_data.path());
        std::fs::create_dir_all(live.join("strategy")).unwrap();
        std::fs::write(live.join("strategy/current.md"), "# Original\n").unwrap();
        let oversized = vec![b'x'; MAX_PLAYBOOK_BYTES as usize + 1];
        let tar = test_catalog_tar(&[
            (
                "agency-agents-main/engineering/reviewer.md",
                b"---\nname: Reviewer\n---\nBody changed\n",
            ),
            ("agency-agents-main/strategy/new.md", b"# New\n"),
            ("agency-agents-main/examples/oversized.md", &oversized),
        ]);

        assert!(
            refresh_from_tarball(app_data.path(), &CatalogSource::Bundled, &tar)
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(live.join("strategy/current.md")).unwrap(),
            "# Original\n"
        );
        assert!(!live.join("strategy/new.md").exists());
        assert!(!live.join("engineering/reviewer.md").exists());
    }

    #[test]
    fn playbook_paths_are_fixed_root_relative_markdown_only() {
        for valid in [
            "strategy/QUICKSTART.md",
            "strategy/runbooks/scenario.md",
            "examples/workflow.md",
        ] {
            assert_eq!(validate_playbook_relative_path(valid).unwrap(), valid);
        }
        for invalid in [
            "strategy",
            "strategy/runbooks.json",
            "strategy/../secrets.md",
            "examples/../../secrets.md",
            "/strategy/secret.md",
            "docs/secret.md",
            "examples\\secret.md",
            "examples/secret.MD",
        ] {
            assert!(
                validate_playbook_relative_path(invalid).is_err(),
                "rejected {invalid}"
            );
        }
        assert!(archive_path_components(Path::new("../strategy/secret.md")).is_err());
        assert!(archive_path_components(Path::new("/strategy/secret.md")).is_err());
        assert!(crate::skills::is_windows_reparse_point(0x400));
    }

    #[test]
    fn playbook_catalog_is_bounded_utf8_and_deterministically_sorted() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("strategy/runbooks")).unwrap();
        std::fs::create_dir_all(root.path().join("examples")).unwrap();
        std::fs::write(root.path().join("strategy/zeta.md"), "# Zeta\nBody").unwrap();
        std::fs::write(
            root.path().join("strategy/runbooks/alpha.md"),
            "# Alpha\nBody",
        )
        .unwrap();
        std::fs::write(root.path().join("examples/beta.md"), "No heading\nBody").unwrap();
        std::fs::write(root.path().join("examples/ignored.txt"), "ignored").unwrap();

        let catalog = playbook_catalog(root.path()).unwrap();
        assert_eq!(
            catalog
                .iter()
                .map(|item| item.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec![
                "examples/beta.md",
                "strategy/runbooks/alpha.md",
                "strategy/zeta.md"
            ]
        );
        assert_eq!(catalog[0].title, "beta");
        assert_eq!(catalog[1].title, "Alpha");

        std::fs::write(root.path().join("examples/invalid.md"), [0xff, 0xfe]).unwrap();
        assert!(
            playbook_catalog(root.path()).is_err(),
            "invalid UTF-8 fails closed"
        );
        std::fs::remove_file(root.path().join("examples/invalid.md")).unwrap();

        std::fs::write(
            root.path().join("examples/oversized.md"),
            vec![b'x'; MAX_PLAYBOOK_BYTES as usize + 1],
        )
        .unwrap();
        assert!(
            playbook_catalog(root.path()).is_err(),
            "oversized docs fail closed"
        );
    }

    #[test]
    fn playbook_catalog_enforces_document_count_cap() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("examples")).unwrap();
        for index in 0..=MAX_PLAYBOOK_DOCUMENTS {
            std::fs::write(
                root.path().join(format!("examples/{index:04}.md")),
                format!("# {index}\n"),
            )
            .unwrap();
        }
        assert!(playbook_catalog(root.path()).is_err());
    }

    #[test]
    fn playbook_read_revalidates_exact_file_and_returns_source_provenance() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("strategy")).unwrap();
        std::fs::write(
            root.path().join("strategy/plan.md"),
            "# Plan\n<script>never run</script>",
        )
        .unwrap();

        let document = read_playbook(root.path(), "strategy/plan.md").unwrap();
        assert_eq!(document.relative_path, "strategy/plan.md");
        assert_eq!(document.title, "Plan");
        assert_eq!(document.content, "# Plan\n<script>never run</script>");
        assert!(read_playbook(root.path(), "../secret.md").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn playbook_catalog_and_read_reject_linked_roots_directories_and_files() {
        use std::os::unix::fs::symlink;

        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.md"), "# Secret").unwrap();

        let linked_root_parent = tempfile::tempdir().unwrap();
        symlink(outside.path(), linked_root_parent.path().join("catalog")).unwrap();
        assert!(playbook_catalog(&linked_root_parent.path().join("catalog")).is_err());

        let root = tempfile::tempdir().unwrap();
        symlink(outside.path(), root.path().join("strategy")).unwrap();
        assert!(playbook_catalog(root.path()).is_err());
        assert!(read_playbook(root.path(), "strategy/secret.md").is_err());

        std::fs::remove_file(root.path().join("strategy")).unwrap();
        std::fs::create_dir_all(root.path().join("strategy")).unwrap();
        symlink(
            outside.path().join("secret.md"),
            root.path().join("strategy/secret.md"),
        )
        .unwrap();
        assert!(playbook_catalog(root.path()).is_err());
        assert!(read_playbook(root.path(), "strategy/secret.md").is_err());
    }
}
