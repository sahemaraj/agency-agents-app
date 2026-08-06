use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::error::AppError;
use crate::library;
use crate::state::AppState;
use crate::types::{
    AgentApproval, AgentApprovalAction, AgentApprovalState, AgentCollection, AgentFolderAssignment,
    AgentLibraryState, AgentPreferredSource, AgentPublisherTrust, AgentRecent, AgentReference,
    AgentSmartFolder, AgentUpdatePolicy, AgentUpdatePolicyRecord, AgentUsage,
    AgentWorkspaceProfile,
};
use crate::{state::append_mcp_audit, types::McpAuditEntry};

const MAX_NAMED_ITEMS: usize = 128;
const MAX_RECENT: usize = 50;
const MAX_DOCUMENT_BYTES: u64 = 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentLibraryDocument {
    schema_version: u32,
    content_kind: String,
    state: AgentLibraryState,
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::InvalidArgument {
        message: message.into(),
    }
}

fn library_path(app_data_dir: &Path) -> PathBuf {
    super::corpus::state_dir(app_data_dir).join("agent-library.json")
}

fn lock_library(app_data_dir: &Path) -> Result<File, AppError> {
    let directory = super::corpus::state_dir(app_data_dir);
    std::fs::create_dir_all(&directory).map_err(|error| AppError::Io {
        message: format!("create Agent library state directory: {error}"),
    })?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("agent-library.lock"))
        .map_err(|error| AppError::Io {
            message: format!("open Agent library lock: {error}"),
        })?;
    file.lock().map_err(|error| AppError::Io {
        message: format!("lock Agent library: {error}"),
    })?;
    Ok(file)
}

async fn lock_library_async(app_data_dir: PathBuf) -> Result<File, AppError> {
    tokio::task::spawn_blocking(move || lock_library(&app_data_dir))
        .await
        .map_err(|error| AppError::Internal {
            message: format!("Agent library lock task failed: {error}"),
        })?
}

fn validate_name(value: &str) -> Result<(), AppError> {
    library::validate_folder_segment(value)
}

fn validate_reference(reference: &AgentReference) -> Result<(), AppError> {
    library::validate_reference(&reference.source_id, &reference.relative_path)
}

fn validate_approval_action(action: &AgentApprovalAction) -> Result<(), AppError> {
    let validate_project = |project_path: &Option<String>| {
        if project_path
            .as_deref()
            .is_some_and(|path| path.len() > 4096 || !Path::new(path).is_absolute())
        {
            Err(invalid("Agent approval project path is invalid"))
        } else {
            Ok(())
        }
    };
    let validate_plan = |revision: &str| {
        if revision.len() == 64 && revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            Ok(())
        } else {
            Err(invalid("Agent approval plan revision is invalid"))
        }
    };
    let validate_lifecycle = |reference: &AgentReference,
                              tool: &str,
                              project_path: &Option<String>,
                              plan_revision: &str| {
        validate_reference(reference)?;
        if !crate::registry::get(tool).is_some_and(crate::registry::ToolMeta::installable) {
            return Err(invalid("Agent approval tool is not installable"));
        }
        validate_project(project_path)?;
        validate_plan(plan_revision)
    };
    match action {
        AgentApprovalAction::SourceRemove { source_id } => {
            library::validate_reference(source_id, "source.md")?;
            if source_id == super::BUILTIN_AGENT_SOURCE_ID {
                return Err(invalid("the built-in Agent source cannot be removed"));
            }
            Ok(())
        }
        AgentApprovalAction::FolderDelete { path, .. } => library::validate_folder_path(path),
        AgentApprovalAction::CollectionDelete { name }
        | AgentApprovalAction::SmartFolderDelete { name }
        | AgentApprovalAction::ProfileDelete { name } => validate_name(name),
        AgentApprovalAction::UpdatePolicySet { reference, .. } => validate_reference(reference),
        AgentApprovalAction::PublisherTrustSet {
            name,
            public_key,
            trusted,
            revoked,
        } => {
            validate_name(name)?;
            if public_key.trim().is_empty() || public_key.len() > 256 || (*trusted && *revoked) {
                return Err(invalid("Agent publisher trust request is invalid"));
            }
            Ok(())
        }
        AgentApprovalAction::DraftPublish { id, plan_revision } => {
            Uuid::parse_str(id).map_err(|_| invalid("Agent draft id is invalid"))?;
            validate_plan(plan_revision)
        }
        AgentApprovalAction::Install {
            reference,
            tool,
            project_path,
            plan_revision,
            ..
        }
        | AgentApprovalAction::Update {
            reference,
            tool,
            project_path,
            plan_revision,
        }
        | AgentApprovalAction::Uninstall {
            reference,
            tool,
            project_path,
            plan_revision,
        } => validate_lifecycle(reference, tool, project_path, plan_revision),
        AgentApprovalAction::Rollback {
            reference,
            tool,
            project_path,
            snapshot_id,
            plan_revision,
        } => {
            validate_lifecycle(reference, tool, project_path, plan_revision)?;
            Uuid::parse_str(snapshot_id).map_err(|_| invalid("Agent snapshot id is invalid"))?;
            Ok(())
        }
        AgentApprovalAction::BatchCollection {
            collection_name,
            operation,
            tool,
            project_path,
            plan_revision,
        } => {
            validate_name(collection_name)?;
            if !matches!(operation.as_str(), "install" | "update" | "uninstall") {
                return Err(invalid("Agent batch operation is invalid"));
            }
            if !crate::registry::get(tool).is_some_and(crate::registry::ToolMeta::installable) {
                return Err(invalid("Agent approval tool is not installable"));
            }
            validate_project(project_path)?;
            validate_plan(plan_revision)
        }
    }
}

fn approval_audit(action: &AgentApprovalAction) -> (&'static str, &'static str, Option<&str>) {
    match action {
        AgentApprovalAction::SourceRemove { .. } => {
            ("agents_remove_source", "agent_destructive", None)
        }
        AgentApprovalAction::FolderDelete { .. } => {
            ("agents_delete_folder", "agent_destructive", None)
        }
        AgentApprovalAction::CollectionDelete { .. } => {
            ("agents_delete_collection", "agent_destructive", None)
        }
        AgentApprovalAction::SmartFolderDelete { .. } => {
            ("agents_delete_smart_folder", "agent_destructive", None)
        }
        AgentApprovalAction::ProfileDelete { .. } => {
            ("agents_delete_profile", "agent_destructive", None)
        }
        AgentApprovalAction::UpdatePolicySet { .. } => {
            ("agents_set_update_policy", "agent_source", None)
        }
        AgentApprovalAction::PublisherTrustSet { .. } => {
            ("agents_request_publisher_trust", "agent_destructive", None)
        }
        AgentApprovalAction::DraftPublish { .. } => {
            ("agents_request_publish_draft", "agent_source", None)
        }
        AgentApprovalAction::Install { project_path, .. } => {
            ("agents_install", "agent_install", project_path.as_deref())
        }
        AgentApprovalAction::Update { project_path, .. } => {
            ("agents_update", "agent_install", project_path.as_deref())
        }
        AgentApprovalAction::Uninstall { project_path, .. } => (
            "agents_uninstall",
            "agent_destructive",
            project_path.as_deref(),
        ),
        AgentApprovalAction::Rollback { project_path, .. } => (
            "agents_request_rollback",
            "agent_destructive",
            project_path.as_deref(),
        ),
        AgentApprovalAction::BatchCollection { project_path, .. } => (
            "agents_request_batch_collection",
            "agent_install",
            project_path.as_deref(),
        ),
    }
}

async fn append_approval_audit(
    state: &AppState,
    approval: &AgentApproval,
    phase: &str,
    success: bool,
) -> Result<(), AppError> {
    let (tool, action, project_path) = approval_audit(&approval.request);
    append_mcp_audit(
        &state.app_data_dir,
        McpAuditEntry {
            id: approval.id.clone(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            client: Some(approval.requested_by.clone()),
            tool: tool.into(),
            action: action.into(),
            phase: phase.into(),
            success,
            project_path: project_path.map(str::to_owned),
        },
    )
    .await
}

fn all_references(state: &AgentLibraryState) -> Vec<&AgentReference> {
    state
        .favorites
        .iter()
        .chain(state.recent.iter().map(|item| &item.agent))
        .chain(state.collections.iter().flat_map(|item| item.agents.iter()))
        .chain(state.update_policies.iter().map(|item| &item.agent))
        .chain(state.usage.iter().map(|item| &item.agent))
        .collect()
}

fn validate_state(state: &AgentLibraryState) -> Result<(), AppError> {
    if state.folders.len() > library::MAX_LIBRARY_FOLDERS
        || state.collections.len() > MAX_NAMED_ITEMS
        || state.smart_folders.len() > MAX_NAMED_ITEMS
        || state.profiles.len() > MAX_NAMED_ITEMS
        || state.publisher_trust.len() > MAX_NAMED_ITEMS
        || state.preferred_sources.len() > MAX_NAMED_ITEMS
        || state.approvals.len() > MAX_NAMED_ITEMS
        || state.recent.len() > MAX_RECENT
    {
        return Err(invalid("Agent library exceeds a collection limit"));
    }
    for folder in &state.folders {
        library::validate_folder_path(folder)?;
    }
    if state.folders.iter().enumerate().any(|(index, folder)| {
        state.folders[index + 1..]
            .iter()
            .any(|other| other.eq_ignore_ascii_case(folder))
    }) {
        return Err(invalid("Agent folder paths must be unique"));
    }
    for assignment in &state.assignments {
        validate_reference(&AgentReference {
            source_id: assignment.source_id.clone(),
            relative_path: assignment.relative_path.clone(),
        })?;
        library::validate_folder_path(&assignment.folder_path)?;
        if !state.folders.contains(&assignment.folder_path) {
            return Err(invalid(format!(
                "assigned Agent folder does not exist: {}",
                assignment.folder_path
            )));
        }
    }
    for reference in all_references(state) {
        validate_reference(reference)?;
    }
    let unique_references = |values: Vec<&AgentReference>, kind: &str| -> Result<(), AppError> {
        if values.iter().copied().collect::<HashSet<_>>().len() != values.len() {
            return Err(invalid(format!("Agent {kind} references must be unique")));
        }
        Ok(())
    };
    unique_references(state.favorites.iter().collect(), "favorite")?;
    unique_references(
        state
            .update_policies
            .iter()
            .map(|item| &item.agent)
            .collect(),
        "update policy",
    )?;
    for names in [
        state
            .collections
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        state
            .smart_folders
            .iter()
            .map(|item| item.name.as_str())
            .collect(),
        state
            .profiles
            .iter()
            .map(|item| item.name.as_str())
            .collect(),
    ] {
        for name in &names {
            validate_name(name)?;
        }
        if names.iter().enumerate().any(|(index, name)| {
            names[index + 1..]
                .iter()
                .any(|other| other.eq_ignore_ascii_case(name))
        }) {
            return Err(invalid(
                "Agent library names must be unique within their kind",
            ));
        }
    }
    for profile in &state.profiles {
        if profile
            .folders
            .iter()
            .any(|folder| !state.folders.contains(folder))
            || profile.collections.iter().any(|collection| {
                !state
                    .collections
                    .iter()
                    .any(|item| item.name == *collection)
            })
        {
            return Err(invalid(format!(
                "Agent profile has a missing reference: {}",
                profile.name
            )));
        }
    }
    for trust in &state.publisher_trust {
        validate_name(&trust.name)?;
        if trust.public_key.trim().is_empty()
            || trust.public_key.len() > 256
            || (trust.trusted && trust.revoked)
        {
            return Err(invalid("Agent publisher trust record is invalid"));
        }
    }
    for preferred in &state.preferred_sources {
        validate_name(&preferred.agent_name)?;
        if preferred.source_id.trim().is_empty() || preferred.source_id.len() > 128 {
            return Err(invalid("Agent preferred source record is invalid"));
        }
    }
    for approval in &state.approvals {
        if Uuid::parse_str(&approval.id).is_err()
            || approval.requested_by.trim().is_empty()
            || approval.requested_by.chars().count() > 64
        {
            return Err(invalid("Agent approval identity is invalid"));
        }
    }
    Ok(())
}

async fn load(app_data_dir: &Path) -> Result<AgentLibraryState, AppError> {
    if let Some(database) = crate::state_db::StateDatabase::completed(app_data_dir).await? {
        return database
            .read(document_spec())
            .await?
            .ok_or_else(|| AppError::StorageCorrupt {
                message: "Agent library is missing after SQLite migration".into(),
            });
    }
    let path = library_path(app_data_dir);
    let state = match tokio::fs::read(&path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| AppError::JsonParse {
            command: "agent_library_list".into(),
            message: error.to_string(),
            raw_excerpt: String::from_utf8_lossy(&bytes[..bytes.len().min(256)]).into_owned(),
        })?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => AgentLibraryState::default(),
        Err(error) => {
            return Err(AppError::Io {
                message: format!("read Agent library: {error}"),
            });
        }
    };
    validate_state(&state)?;
    Ok(state)
}

fn document_spec() -> crate::state_db::DocumentSpec<AgentLibraryState> {
    crate::state_db::DocumentSpec::new("agent_library", 1, 1_048_576, validate_state)
}

pub(crate) fn import_spec() -> crate::state_db::ImportSpec {
    crate::state_db::ImportSpec::document(document_spec(), AgentLibraryState::default())
}

async fn save(app_data_dir: &Path, state: &AgentLibraryState) -> Result<(), AppError> {
    validate_state(state)?;
    if let Some(database) = crate::state_db::StateDatabase::completed(app_data_dir).await? {
        let replacement = state.clone();
        return database
            .mutate(
                document_spec(),
                AgentLibraryState::default(),
                move |current| {
                    *current = replacement;
                    Ok(())
                },
            )
            .await;
    }
    let bytes = serde_json::to_vec_pretty(state).map_err(|error| AppError::Internal {
        message: format!("serialize Agent library: {error}"),
    })?;
    crate::util::fs::atomic_write(&library_path(app_data_dir), &bytes).await
}

async fn require_known(state: &AppState, reference: &AgentReference) -> Result<(), AppError> {
    validate_reference(reference)?;
    super::resolve_agent_package(&state.app_data_dir, reference).await?;
    Ok(())
}

async fn require_known_state(
    state: &AppState,
    library: &AgentLibraryState,
) -> Result<(), AppError> {
    for reference in all_references(library) {
        require_known(state, reference).await?;
    }
    for assignment in &library.assignments {
        require_known(
            state,
            &AgentReference {
                source_id: assignment.source_id.clone(),
                relative_path: assignment.relative_path.clone(),
            },
        )
        .await?;
    }
    Ok(())
}

fn apply_rewrites(state: &mut AgentLibraryState, rewrites: &[(String, String)]) {
    for folder in &mut state.folders {
        if let Some((_, value)) = rewrites.iter().find(|(from, _)| from == folder) {
            *folder = value.clone();
        }
    }
    for assignment in &mut state.assignments {
        if let Some((_, value)) = rewrites
            .iter()
            .find(|(from, _)| from == &assignment.folder_path)
        {
            assignment.folder_path = value.clone();
        }
    }
    for profile in &mut state.profiles {
        for folder in &mut profile.folders {
            if let Some((_, value)) = rewrites.iter().find(|(from, _)| from == folder) {
                *folder = value.clone();
            }
        }
    }
    state.folders.sort();
}

pub async fn list(state: &AppState) -> Result<AgentLibraryState, AppError> {
    load(&state.app_data_dir).await
}

pub async fn create_folder(state: &AppState, path: String) -> Result<AgentLibraryState, AppError> {
    let _guard = lock_library_async(state.app_data_dir.clone()).await?;
    let mut value = load(&state.app_data_dir).await?;
    library::create_folder(&mut value.folders, path)?;
    save(&state.app_data_dir, &value).await?;
    Ok(value)
}

pub async fn rename_folder(
    state: &AppState,
    path: String,
    new_name: String,
) -> Result<AgentLibraryState, AppError> {
    let _guard = lock_library_async(state.app_data_dir.clone()).await?;
    let mut value = load(&state.app_data_dir).await?;
    let rewrites = library::rename_folder_paths(&value.folders, &path, &new_name)?;
    apply_rewrites(&mut value, &rewrites);
    save(&state.app_data_dir, &value).await?;
    Ok(value)
}

pub async fn move_folder(
    state: &AppState,
    path: String,
    new_parent: Option<String>,
) -> Result<AgentLibraryState, AppError> {
    let _guard = lock_library_async(state.app_data_dir.clone()).await?;
    let mut value = load(&state.app_data_dir).await?;
    let rewrites = library::move_folder_paths(&value.folders, &path, new_parent.as_deref())?;
    apply_rewrites(&mut value, &rewrites);
    save(&state.app_data_dir, &value).await?;
    Ok(value)
}

pub async fn delete_folder(
    state: &AppState,
    path: String,
    recursive: bool,
) -> Result<AgentLibraryState, AppError> {
    let _guard = lock_library_async(state.app_data_dir.clone()).await?;
    let mut value = load(&state.app_data_dir).await?;
    let removed = library::deleted_folder_paths(&value.folders, &path, recursive)?;
    if !recursive
        && value
            .assignments
            .iter()
            .any(|item| item.folder_path == path)
    {
        return Err(invalid(
            "Agent folder has assignments; use recursive deletion",
        ));
    }
    value.folders.retain(|folder| !removed.contains(folder));
    value
        .assignments
        .retain(|item| !removed.contains(&item.folder_path));
    for profile in &mut value.profiles {
        profile.folders.retain(|folder| !removed.contains(folder));
    }
    save(&state.app_data_dir, &value).await?;
    Ok(value)
}

pub async fn assign_folder(
    state: &AppState,
    reference: AgentReference,
    folder_path: Option<String>,
) -> Result<AgentLibraryState, AppError> {
    require_known(state, &reference).await?;
    let _guard = lock_library_async(state.app_data_dir.clone()).await?;
    let mut value = load(&state.app_data_dir).await?;
    value.assignments.retain(|item| {
        item.source_id != reference.source_id || item.relative_path != reference.relative_path
    });
    if let Some(folder_path) = folder_path {
        if !value.folders.contains(&folder_path) {
            return Err(invalid(format!(
                "Agent folder does not exist: {folder_path}"
            )));
        }
        value.assignments.push(AgentFolderAssignment {
            source_id: reference.source_id,
            relative_path: reference.relative_path,
            folder_path,
        });
    }
    save(&state.app_data_dir, &value).await?;
    Ok(value)
}

pub async fn set_favorite(
    state: &AppState,
    reference: AgentReference,
    favorite: bool,
) -> Result<AgentLibraryState, AppError> {
    require_known(state, &reference).await?;
    let _guard = lock_library_async(state.app_data_dir.clone()).await?;
    let mut value = load(&state.app_data_dir).await?;
    value.favorites.retain(|item| item != &reference);
    if favorite {
        value.favorites.push(reference);
    }
    save(&state.app_data_dir, &value).await?;
    Ok(value)
}

pub async fn touch_recent(
    state: &AppState,
    reference: AgentReference,
) -> Result<AgentLibraryState, AppError> {
    require_known(state, &reference).await?;
    let _guard = lock_library_async(state.app_data_dir.clone()).await?;
    let mut value = load(&state.app_data_dir).await?;
    value.recent.retain(|item| item.agent != reference);
    value.recent.insert(
        0,
        AgentRecent {
            agent: reference,
            viewed_at: chrono::Utc::now().to_rfc3339(),
        },
    );
    value.recent.truncate(MAX_RECENT);
    save(&state.app_data_dir, &value).await?;
    Ok(value)
}

pub async fn save_collection(
    state: &AppState,
    collection: AgentCollection,
) -> Result<AgentLibraryState, AppError> {
    validate_name(&collection.name)?;
    for reference in &collection.agents {
        require_known(state, reference).await?;
    }
    let _guard = lock_library_async(state.app_data_dir.clone()).await?;
    let mut value = load(&state.app_data_dir).await?;
    value
        .collections
        .retain(|item| item.name != collection.name);
    value.collections.push(collection);
    value
        .collections
        .sort_by(|left, right| left.name.cmp(&right.name));
    save(&state.app_data_dir, &value).await?;
    Ok(value)
}

pub async fn delete_collection(
    state: &AppState,
    name: String,
) -> Result<AgentLibraryState, AppError> {
    let _guard = lock_library_async(state.app_data_dir.clone()).await?;
    let mut value = load(&state.app_data_dir).await?;
    value.collections.retain(|item| item.name != name);
    for profile in &mut value.profiles {
        profile.collections.retain(|item| item != &name);
    }
    save(&state.app_data_dir, &value).await?;
    Ok(value)
}

pub async fn save_smart_folder(
    state: &AppState,
    smart_folder: AgentSmartFolder,
) -> Result<AgentLibraryState, AppError> {
    validate_name(&smart_folder.name)?;
    let _guard = lock_library_async(state.app_data_dir.clone()).await?;
    let mut value = load(&state.app_data_dir).await?;
    value
        .smart_folders
        .retain(|item| item.name != smart_folder.name);
    value.smart_folders.push(smart_folder);
    value
        .smart_folders
        .sort_by(|left, right| left.name.cmp(&right.name));
    save(&state.app_data_dir, &value).await?;
    Ok(value)
}

pub async fn delete_smart_folder(
    state: &AppState,
    name: String,
) -> Result<AgentLibraryState, AppError> {
    let _guard = lock_library_async(state.app_data_dir.clone()).await?;
    let mut value = load(&state.app_data_dir).await?;
    value.smart_folders.retain(|item| item.name != name);
    save(&state.app_data_dir, &value).await?;
    Ok(value)
}

pub async fn save_profile(
    state: &AppState,
    profile: AgentWorkspaceProfile,
) -> Result<AgentLibraryState, AppError> {
    validate_name(&profile.name)?;
    let _guard = lock_library_async(state.app_data_dir.clone()).await?;
    let mut value = load(&state.app_data_dir).await?;
    value.profiles.retain(|item| item.name != profile.name);
    value.profiles.push(profile);
    value
        .profiles
        .sort_by(|left, right| left.name.cmp(&right.name));
    save(&state.app_data_dir, &value).await?;
    Ok(value)
}

pub async fn delete_profile(state: &AppState, name: String) -> Result<AgentLibraryState, AppError> {
    let _guard = lock_library_async(state.app_data_dir.clone()).await?;
    let mut value = load(&state.app_data_dir).await?;
    value.profiles.retain(|item| item.name != name);
    save(&state.app_data_dir, &value).await?;
    Ok(value)
}

pub async fn replace_library(
    state: &AppState,
    value: AgentLibraryState,
) -> Result<AgentLibraryState, AppError> {
    validate_state(&value)?;
    require_known_state(state, &value).await?;
    let _guard = lock_library_async(state.app_data_dir.clone()).await?;
    save(&state.app_data_dir, &value).await?;
    Ok(value)
}

fn checked_document_path(path: &str) -> Result<PathBuf, AppError> {
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(invalid("Agent library document path must be absolute"));
    }
    Ok(path)
}

pub async fn export_library(state: &AppState, path: String) -> Result<u32, AppError> {
    let path = checked_document_path(&path)?;
    let document = AgentLibraryDocument {
        schema_version: 1,
        content_kind: "agents".into(),
        state: load(&state.app_data_dir).await?,
    };
    let bytes = serde_json::to_vec_pretty(&document).map_err(|error| AppError::Internal {
        message: format!("serialize Agent library export: {error}"),
    })?;
    crate::util::fs::atomic_write(&path, &bytes).await?;
    Ok(bytes.len() as u32)
}

pub async fn import_library(state: &AppState, path: String) -> Result<AgentLibraryState, AppError> {
    let path = checked_document_path(&path)?;
    let bytes = crate::util::fs::read_capped(&path, MAX_DOCUMENT_BYTES).await?;
    let document: AgentLibraryDocument =
        serde_json::from_slice(&bytes).map_err(|error| AppError::JsonParse {
            command: "agent_library_import".into(),
            message: error.to_string(),
            raw_excerpt: String::from_utf8_lossy(&bytes[..bytes.len().min(256)]).into_owned(),
        })?;
    if document.schema_version != 1 || document.content_kind != "agents" {
        return Err(invalid("document is not a supported Agent library export"));
    }
    replace_library(state, document.state).await
}

pub async fn set_update_policy(
    state: &AppState,
    reference: AgentReference,
    policy: AgentUpdatePolicy,
) -> Result<AgentLibraryState, AppError> {
    require_known(state, &reference).await?;
    let _guard = lock_library_async(state.app_data_dir.clone()).await?;
    let mut value = load(&state.app_data_dir).await?;
    value.update_policies.retain(|item| item.agent != reference);
    value.update_policies.push(AgentUpdatePolicyRecord {
        agent: reference,
        policy,
    });
    save(&state.app_data_dir, &value).await?;
    Ok(value)
}

pub async fn set_publisher_trust(
    state: &AppState,
    trust: AgentPublisherTrust,
) -> Result<AgentLibraryState, AppError> {
    let _guard = lock_library_async(state.app_data_dir.clone()).await?;
    let mut value = load(&state.app_data_dir).await?;
    value.publisher_trust.retain(|item| item.name != trust.name);
    value.publisher_trust.push(trust);
    save(&state.app_data_dir, &value).await?;
    Ok(value)
}

pub async fn set_preferred_source(
    state: &AppState,
    preferred: AgentPreferredSource,
) -> Result<AgentLibraryState, AppError> {
    if !super::load_agent_sources(&state.app_data_dir)
        .await?
        .iter()
        .any(|source| source.id == preferred.source_id)
    {
        return Err(invalid("preferred Agent source is unknown"));
    }
    let _guard = lock_library_async(state.app_data_dir.clone()).await?;
    let mut value = load(&state.app_data_dir).await?;
    value
        .preferred_sources
        .retain(|item| item.agent_name != preferred.agent_name);
    value.preferred_sources.push(preferred);
    save(&state.app_data_dir, &value).await?;
    Ok(value)
}

pub async fn record_usage(
    state: &AppState,
    reference: AgentReference,
    event: String,
) -> Result<AgentLibraryState, AppError> {
    require_known(state, &reference).await?;
    let _guard = lock_library_async(state.app_data_dir.clone()).await?;
    let mut value = load(&state.app_data_dir).await?;
    let index = value.usage.iter().position(|item| item.agent == reference);
    let index = if let Some(index) = index {
        index
    } else {
        value.usage.push(AgentUsage {
            agent: reference,
            fetches: 0,
            publishes: 0,
            rejections: 0,
            last_used_at: String::new(),
        });
        value.usage.len() - 1
    };
    let usage = &mut value.usage[index];
    match event.as_str() {
        "fetch" => usage.fetches = usage.fetches.saturating_add(1),
        "publish" => usage.publishes = usage.publishes.saturating_add(1),
        "reject" => usage.rejections = usage.rejections.saturating_add(1),
        _ => {
            return Err(invalid(
                "Agent usage event must be fetch, publish, or reject",
            ))
        }
    }
    usage.last_used_at = chrono::Utc::now().to_rfc3339();
    save(&state.app_data_dir, &value).await?;
    Ok(value)
}

pub async fn submit_approval(
    state: &AppState,
    requested_by: String,
    request: AgentApprovalAction,
) -> Result<AgentApproval, AppError> {
    if requested_by.trim() != requested_by
        || requested_by.is_empty()
        || requested_by.chars().count() > 64
    {
        return Err(invalid("requested_by must contain 1-64 trimmed characters"));
    }
    validate_approval_action(&request)?;
    let _guard = lock_library_async(state.app_data_dir.clone()).await?;
    let mut value = load(&state.app_data_dir).await?;
    if let Some(existing) = value.approvals.iter().find(|approval| {
        approval.state == AgentApprovalState::Pending
            && approval.requested_by == requested_by
            && approval.request == request
    }) {
        return Ok(existing.clone());
    }
    if value.approvals.len() == MAX_NAMED_ITEMS {
        if let Some(index) = value.approvals.iter().position(|approval| {
            matches!(
                approval.state,
                AgentApprovalState::Approved | AgentApprovalState::Rejected
            )
        }) {
            value.approvals.remove(index);
        } else {
            return Err(invalid("Agent approval inbox is full"));
        }
    }
    let approval = AgentApproval {
        id: Uuid::new_v4().to_string(),
        submitted_at: chrono::Utc::now().to_rfc3339(),
        state: AgentApprovalState::Pending,
        requested_by,
        request,
        result: None,
    };
    append_approval_audit(state, &approval, "attempt", false).await?;
    value.approvals.push(approval.clone());
    save(&state.app_data_dir, &value).await?;
    Ok(approval)
}

pub async fn decide_approval(
    state: &AppState,
    id: String,
    approved: bool,
) -> Result<AgentApproval, AppError> {
    if approved {
        return Err(invalid(
            "approval execution requires the desktop approval command",
        ));
    }
    let _guard = lock_library_async(state.app_data_dir.clone()).await?;
    let mut value = load(&state.app_data_dir).await?;
    let approval = value
        .approvals
        .iter_mut()
        .find(|item| item.id == id)
        .ok_or_else(|| invalid(format!("Agent approval not found: {id}")))?;
    if approval.state != AgentApprovalState::Pending {
        return Err(invalid("only pending Agent approvals can be decided"));
    }
    approval.state = AgentApprovalState::Rejected;
    let result = approval.clone();
    save(&state.app_data_dir, &value).await?;
    drop(_guard);
    append_approval_audit(state, &result, "terminal", false).await?;
    Ok(result)
}

async fn execute_organization_approval(
    state: &AppState,
    action: &AgentApprovalAction,
) -> Result<String, AppError> {
    let value = match action {
        AgentApprovalAction::SourceRemove { source_id } => {
            serde_json::to_value(super::remove_agent_source(&state.app_data_dir, source_id).await?)
        }
        AgentApprovalAction::FolderDelete { path, recursive } => {
            serde_json::to_value(delete_folder(state, path.clone(), *recursive).await?)
        }
        AgentApprovalAction::CollectionDelete { name } => {
            serde_json::to_value(delete_collection(state, name.clone()).await?)
        }
        AgentApprovalAction::SmartFolderDelete { name } => {
            serde_json::to_value(delete_smart_folder(state, name.clone()).await?)
        }
        AgentApprovalAction::ProfileDelete { name } => {
            serde_json::to_value(delete_profile(state, name.clone()).await?)
        }
        AgentApprovalAction::UpdatePolicySet { reference, policy } => {
            serde_json::to_value(set_update_policy(state, reference.clone(), *policy).await?)
        }
        AgentApprovalAction::PublisherTrustSet {
            name,
            public_key,
            trusted,
            revoked,
        } => serde_json::to_value(
            set_publisher_trust(
                state,
                AgentPublisherTrust {
                    name: name.clone(),
                    public_key: public_key.clone(),
                    trusted: *trusted,
                    revoked: *revoked,
                },
            )
            .await?,
        ),
        AgentApprovalAction::DraftPublish { id, plan_revision } => {
            let draft = super::drafts::get(state, id).await?;
            if draft.source_hash != *plan_revision {
                return Err(invalid(
                    "Agent draft changed after publication was requested",
                ));
            }
            serde_json::to_value(super::drafts::publish(state, id).await?)
        }
        _ => {
            return Err(invalid(
                "Agent approval action is not an organization mutation",
            ))
        }
    }
    .map_err(|error| AppError::Internal {
        message: format!("serialize approved Agent organization result: {error}"),
    })?;
    serde_json::to_string_pretty(&value).map_err(|error| AppError::Internal {
        message: format!("serialize approved Agent organization result: {error}"),
    })
}

async fn approve_with_execution(
    app: &AppHandle,
    state: &AppState,
    id: String,
) -> Result<AgentApproval, AppError> {
    let action = {
        let _guard = lock_library_async(state.app_data_dir.clone()).await?;
        let mut value = load(&state.app_data_dir).await?;
        let approval = value
            .approvals
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or_else(|| invalid(format!("Agent approval not found: {id}")))?;
        if approval.state != AgentApprovalState::Pending {
            return Err(invalid("only pending Agent approvals can be approved"));
        }
        approval.state = AgentApprovalState::Running;
        let action = approval.request.clone();
        save(&state.app_data_dir, &value).await?;
        action
    };

    let execution = match action {
        AgentApprovalAction::SourceRemove { .. }
        | AgentApprovalAction::FolderDelete { .. }
        | AgentApprovalAction::CollectionDelete { .. }
        | AgentApprovalAction::SmartFolderDelete { .. }
        | AgentApprovalAction::ProfileDelete { .. }
        | AgentApprovalAction::UpdatePolicySet { .. }
        | AgentApprovalAction::PublisherTrustSet { .. }
        | AgentApprovalAction::DraftPublish { .. } => {
            execute_organization_approval(state, &action).await
        }
        _ => crate::install::execute_agent_lifecycle_approval(app, state, &action).await,
    };

    let _guard = lock_library_async(state.app_data_dir.clone()).await?;
    let mut value = load(&state.app_data_dir).await?;
    let approval = value
        .approvals
        .iter_mut()
        .find(|item| item.id == id)
        .ok_or_else(|| invalid(format!("Agent approval not found: {id}")))?;
    if approval.state != AgentApprovalState::Running {
        return Err(invalid("running Agent approval state changed unexpectedly"));
    }
    match execution {
        Ok(result) => {
            approval.state = AgentApprovalState::Approved;
            approval.result = Some(result);
        }
        Err(error) => {
            approval.state = AgentApprovalState::Rejected;
            approval.result = Some(error.to_string());
        }
    }
    let result = approval.clone();
    save(&state.app_data_dir, &value).await?;
    drop(_guard);
    append_approval_audit(
        state,
        &result,
        "terminal",
        result.state == AgentApprovalState::Approved,
    )
    .await?;
    Ok(result)
}

pub(crate) async fn reconcile_draft_publish_approval(
    state: &AppState,
    approval_id: Option<&str>,
    draft_id: &str,
    plan_revision: &str,
    completed: bool,
    error: Option<String>,
) -> Result<(), AppError> {
    let Some(approval_id) = approval_id else {
        return Ok(());
    };
    let _guard = lock_library_async(state.app_data_dir.clone()).await?;
    let mut library = load(&state.app_data_dir).await?;
    let approval = library
        .approvals
        .iter_mut()
        .find(|approval| approval.id == approval_id)
        .ok_or_else(|| invalid("recovered Agent approval no longer exists"))?;
    if approval.request
        != (AgentApprovalAction::DraftPublish {
            id: draft_id.to_owned(),
            plan_revision: plan_revision.to_owned(),
        })
    {
        return Err(AppError::StorageCorrupt {
            message: "Recovered Agent approval revision does not match its operation".into(),
        });
    }
    if approval.state == AgentApprovalState::Approved && completed {
        return Ok(());
    }
    if !matches!(
        approval.state,
        AgentApprovalState::Running | AgentApprovalState::Pending
    ) {
        return Err(AppError::StorageCorrupt {
            message: "Recovered Agent approval is in an incompatible state".into(),
        });
    }
    if completed {
        approval.state = AgentApprovalState::Approved;
        approval.result = Some("completed".into());
    } else {
        approval.state = AgentApprovalState::Pending;
        approval.result = Some(error.unwrap_or_else(|| "publication recovery rolled back".into()));
    }
    let result = approval.clone();
    save(&state.app_data_dir, &library).await?;
    drop(_guard);
    if completed {
        append_approval_audit(state, &result, "terminal", true).await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn agent_library_list(state: State<'_, AppState>) -> Result<AgentLibraryState, AppError> {
    list(&state).await
}

macro_rules! state_command {
    ($name:ident($($arg:ident: $ty:ty),*) => $target:ident) => {
        #[tauri::command]
        pub async fn $name(state: State<'_, AppState>, $($arg: $ty),*) -> Result<AgentLibraryState, AppError> {
            $target(&state, $($arg),*).await
        }
    };
}

state_command!(agent_folder_create(path: String) => create_folder);
state_command!(agent_folder_rename(path: String, new_name: String) => rename_folder);
state_command!(agent_folder_move(path: String, new_parent: Option<String>) => move_folder);
state_command!(agent_folder_delete(path: String, recursive: bool) => delete_folder);
state_command!(agent_folder_assign(reference: AgentReference, folder_path: Option<String>) => assign_folder);
state_command!(agent_favorite_set(reference: AgentReference, favorite: bool) => set_favorite);
state_command!(agent_recent_touch(reference: AgentReference) => touch_recent);
state_command!(agent_collection_save(collection: AgentCollection) => save_collection);
state_command!(agent_collection_delete(name: String) => delete_collection);
state_command!(agent_smart_folder_save(smart_folder: AgentSmartFolder) => save_smart_folder);
state_command!(agent_smart_folder_delete(name: String) => delete_smart_folder);
state_command!(agent_profile_save(profile: AgentWorkspaceProfile) => save_profile);
state_command!(agent_profile_delete(name: String) => delete_profile);
state_command!(agent_library_replace(value: AgentLibraryState) => replace_library);
state_command!(agent_update_policy_set(reference: AgentReference, policy: AgentUpdatePolicy) => set_update_policy);
state_command!(agent_publisher_trust_set(trust: AgentPublisherTrust) => set_publisher_trust);
state_command!(agent_preferred_source_set(preferred: AgentPreferredSource) => set_preferred_source);
state_command!(agent_usage_record(reference: AgentReference, event: String) => record_usage);

#[tauri::command]
pub async fn agent_library_export(
    state: State<'_, AppState>,
    path: String,
) -> Result<u32, AppError> {
    export_library(&state, path).await
}

#[tauri::command]
pub async fn agent_library_import(
    state: State<'_, AppState>,
    path: String,
) -> Result<AgentLibraryState, AppError> {
    import_library(&state, path).await
}

#[tauri::command]
pub async fn agent_approval_approve(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<AgentApproval, AppError> {
    approve_with_execution(&app, &state, id).await
}

#[tauri::command]
pub async fn agent_approval_reject(
    state: State<'_, AppState>,
    id: String,
) -> Result<AgentApproval, AppError> {
    decide_approval(&state, id, false).await
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::types::{
        AgentCollection, AgentReference, AgentSmartFolder, AgentSmartFolderRule,
        AgentWorkspaceProfile,
    };

    fn state(root: &Path) -> AppState {
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.to_path_buf();
        state
    }

    #[tokio::test]
    async fn sqlite_library_preserves_independent_state_updates() {
        let root = tempfile::tempdir().unwrap();
        let database = crate::state_db::StateDatabase::open(root.path()).unwrap();
        database
            .mutate(document_spec(), AgentLibraryState::default(), |_| Ok(()))
            .await
            .unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let left = state(root.path());
        let right = state(root.path());

        let (left_result, right_result) = tokio::join!(
            create_folder(&left, "Engineering".into()),
            create_folder(&right, "Operations".into()),
        );
        left_result.unwrap();
        right_result.unwrap();

        assert_eq!(
            list(&left).await.unwrap().folders,
            ["Engineering", "Operations"]
        );
    }

    #[tokio::test]
    async fn nested_folder_rewrites_update_every_reference_without_moving_agents() {
        let root = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        std::fs::write(
            source.path().join("reviewer.md"),
            "---\nname: Reviewer\ndescription: Reviews code.\n---\nReview carefully.\n",
        )
        .unwrap();
        let registered = super::super::add_local_source(root.path(), source.path())
            .await
            .unwrap();
        let reference = AgentReference {
            source_id: registered.id,
            relative_path: "reviewer.md".into(),
        };
        let app = state(root.path());

        create_folder(&app, "Work".into()).await.unwrap();
        create_folder(&app, "Work/Review".into()).await.unwrap();
        assign_folder(&app, reference.clone(), Some("Work/Review".into()))
            .await
            .unwrap();
        save_profile(
            &app,
            AgentWorkspaceProfile {
                name: "Daily".into(),
                folders: vec!["Work/Review".into()],
                collections: vec![],
            },
        )
        .await
        .unwrap();
        set_favorite(&app, reference.clone(), true).await.unwrap();
        touch_recent(&app, reference.clone()).await.unwrap();
        save_collection(
            &app,
            AgentCollection {
                name: "Reviewers".into(),
                agents: vec![reference],
            },
        )
        .await
        .unwrap();
        save_smart_folder(
            &app,
            AgentSmartFolder {
                name: "Installable".into(),
                rule: AgentSmartFolderRule {
                    installable: Some(true),
                    ..Default::default()
                },
            },
        )
        .await
        .unwrap();

        let state = rename_folder(&app, "Work".into(), "Projects".into())
            .await
            .unwrap();
        assert_eq!(state.folders, ["Projects", "Projects/Review"]);
        assert_eq!(state.assignments[0].folder_path, "Projects/Review");
        assert_eq!(state.profiles[0].folders, ["Projects/Review"]);
        assert!(source.path().join("reviewer.md").is_file());

        let state = delete_folder(&app, "Projects".into(), true).await.unwrap();
        assert!(state.folders.is_empty());
        assert!(state.assignments.is_empty());
        assert!(state.profiles[0].folders.is_empty());
    }

    #[tokio::test]
    async fn export_is_agent_typed_and_skills_documents_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let app = state(root.path());
        create_folder(&app, "Work".into()).await.unwrap();
        let export = root.path().join("agents.json");
        export_library(&app, export.to_string_lossy().into_owned())
            .await
            .unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&export).unwrap()).unwrap();
        assert_eq!(value["contentKind"], "agents");

        let skills = root.path().join("skills.json");
        std::fs::write(
            &skills,
            r#"{"schemaVersion":1,"contentKind":"skills","state":{}}"#,
        )
        .unwrap();
        assert!(import_library(&app, skills.to_string_lossy().into_owned())
            .await
            .is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn secondary_library_mutations_validate_identity_and_serialize_writes() {
        let root = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        std::fs::write(
            source.path().join("reviewer.md"),
            "---\nname: Reviewer\ndescription: Reviews code.\n---\nReview carefully.\n",
        )
        .unwrap();
        let registered = super::super::add_local_source(root.path(), source.path())
            .await
            .unwrap();
        let reference = AgentReference {
            source_id: registered.id.clone(),
            relative_path: "reviewer.md".into(),
        };
        let app = state(root.path());

        let (left, right) = tokio::join!(
            create_folder(&app, "Alpha".into()),
            create_folder(&app, "Beta".into()),
        );
        assert!(left.is_ok() && right.is_ok());
        create_folder(&app, "Archive".into()).await.unwrap();
        let value = move_folder(&app, "Alpha".into(), Some("Archive".into()))
            .await
            .unwrap();
        assert!(value.folders.contains(&"Archive/Alpha".into()));

        set_update_policy(&app, reference.clone(), AgentUpdatePolicy::Pin)
            .await
            .unwrap();
        set_publisher_trust(
            &app,
            AgentPublisherTrust {
                name: "Acme".into(),
                public_key: "public-key".into(),
                trusted: true,
                revoked: false,
            },
        )
        .await
        .unwrap();
        set_preferred_source(
            &app,
            AgentPreferredSource {
                agent_name: "Reviewer".into(),
                source_id: registered.id,
            },
        )
        .await
        .unwrap();
        let value = record_usage(&app, reference, "fetch".into()).await.unwrap();
        assert_eq!(value.update_policies.len(), 1);
        assert_eq!(value.publisher_trust.len(), 1);
        assert_eq!(value.preferred_sources.len(), 1);
        assert_eq!(value.usage[0].fetches, 1);

        let mut stale = value;
        stale.favorites.push(AgentReference {
            source_id: "missing".into(),
            relative_path: "missing.md".into(),
        });
        assert!(replace_library(&app, stale).await.is_err());
    }

    #[tokio::test]
    async fn approval_submissions_are_typed_bounded_and_idempotent() {
        let root = tempfile::tempdir().unwrap();
        let app = state(root.path());
        let action = AgentApprovalAction::FolderDelete {
            path: "Work".into(),
            recursive: true,
        };

        let first = submit_approval(&app, "claude".into(), action.clone())
            .await
            .unwrap();
        let duplicate = submit_approval(&app, "claude".into(), action)
            .await
            .unwrap();
        assert_eq!(first.id, duplicate.id);
        assert_eq!(list(&app).await.unwrap().approvals.len(), 1);
        let rejected = decide_approval(&app, first.id.clone(), false)
            .await
            .unwrap();
        assert_eq!(rejected.state, AgentApprovalState::Rejected);
        let audit = crate::state::load_mcp_audit(root.path()).await.unwrap();
        let approval_audit = audit
            .iter()
            .filter(|entry| entry.id == first.id)
            .collect::<Vec<_>>();
        assert_eq!(approval_audit.len(), 2);
        assert!(approval_audit
            .iter()
            .any(|entry| entry.phase == "attempt" && !entry.success));
        assert!(approval_audit
            .iter()
            .any(|entry| entry.phase == "terminal" && !entry.success));
        assert!(submit_approval(
            &app,
            "claude".into(),
            AgentApprovalAction::SourceRemove {
                source_id: super::super::BUILTIN_AGENT_SOURCE_ID.into(),
            },
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn draft_publish_approval_rejects_a_stale_revision() {
        let root = tempfile::tempdir().unwrap();
        let app = state(root.path());
        let draft = super::super::drafts::create(
            &app,
            crate::types::AgentDraftInput {
                relative_path: "reviewer.md".into(),
                text: "---\nname: Reviewer\ndescription: Reviews code.\n---\nReview carefully.\n"
                    .into(),
            },
        )
        .await
        .unwrap();
        let action = AgentApprovalAction::DraftPublish {
            id: draft.id.clone(),
            plan_revision: draft.source_hash,
        };
        submit_approval(&app, "codex".into(), action.clone())
            .await
            .unwrap();
        super::super::drafts::edit(
            &app,
            &draft.id,
            crate::types::AgentDraftInput {
                relative_path: "reviewer.md".into(),
                text:
                    "---\nname: Reviewer\ndescription: Reviews code.\n---\nReview changed code.\n"
                        .into(),
            },
        )
        .await
        .unwrap();

        assert!(execute_organization_approval(&app, &action).await.is_err());
        assert_eq!(
            super::super::drafts::get(&app, &draft.id)
                .await
                .unwrap()
                .state,
            crate::types::AgentDraftState::Pending
        );
    }

    #[tokio::test]
    async fn draft_publish_approval_executes_the_current_revision() {
        let root = tempfile::tempdir().unwrap();
        let app = state(root.path());
        let draft = super::super::drafts::create(
            &app,
            crate::types::AgentDraftInput {
                relative_path: "reviewer.md".into(),
                text: "---\nname: Reviewer\ndescription: Reviews code.\n---\nReview carefully.\n"
                    .into(),
            },
        )
        .await
        .unwrap();

        execute_organization_approval(
            &app,
            &AgentApprovalAction::DraftPublish {
                id: draft.id.clone(),
                plan_revision: draft.source_hash,
            },
        )
        .await
        .unwrap();

        assert_eq!(
            super::super::drafts::get(&app, &draft.id)
                .await
                .unwrap()
                .state,
            crate::types::AgentDraftState::Published
        );
    }

    #[tokio::test]
    async fn recovery_reconciles_only_the_bound_running_draft_revision() {
        let root = tempfile::tempdir().unwrap();
        let app = state(root.path());
        let revision = "a".repeat(64);
        let approval = submit_approval(
            &app,
            "codex".into(),
            AgentApprovalAction::DraftPublish {
                id: Uuid::new_v4().to_string(),
                plan_revision: revision.clone(),
            },
        )
        .await
        .unwrap();
        let approval_id = approval.id.clone();
        let draft_id = match &approval.request {
            AgentApprovalAction::DraftPublish { id, .. } => id.clone(),
            _ => unreachable!(),
        };
        let mut library = load(root.path()).await.unwrap();
        library.approvals[0].state = AgentApprovalState::Running;
        save(root.path(), &library).await.unwrap();

        reconcile_draft_publish_approval(
            &app,
            Some(&approval_id),
            &draft_id,
            &revision,
            true,
            None,
        )
        .await
        .unwrap();

        let reconciled = list(&app)
            .await
            .unwrap()
            .approvals
            .into_iter()
            .find(|item| item.id == approval_id)
            .unwrap();
        assert_eq!(reconciled.state, AgentApprovalState::Approved);
        assert_eq!(reconciled.result.as_deref(), Some("completed"));
    }

    #[tokio::test]
    async fn corrupt_library_fails_without_rewriting_it() {
        let root = tempfile::tempdir().unwrap();
        let path = library_path(root.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{not-json").unwrap();

        assert!(load(root.path()).await.is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"{not-json");
    }
}
