use std::path::{Path, PathBuf};

use tauri::State;

use crate::corpus::state_dir;
use crate::error::AppError;
use crate::library;
use crate::state::AppState;
use crate::types::{
    SkillApproval, SkillApprovalAction, SkillApprovalState, SkillCollection, SkillFolderAssignment,
    SkillFolderState, SkillPreferredSource, SkillPublisherTrust, SkillRecent, SkillReference,
    SkillSmartFolder, SkillUpdatePolicy, SkillUpdatePolicyRecord, SkillUsage,
    SkillWorkspaceProfile,
};
use crate::util::fs::atomic_write;

const MAX_FOLDERS: usize = library::MAX_LIBRARY_FOLDERS;
#[cfg(test)]
const MAX_FOLDER_DEPTH: usize = library::MAX_LIBRARY_FOLDER_DEPTH;
#[cfg(test)]
const MAX_FOLDER_SEGMENT_CHARS: usize = library::MAX_LIBRARY_FOLDER_SEGMENT_CHARS;
const MAX_NAMED_ITEMS: usize = 128;
const MAX_RECENT: usize = 50;

fn folders_path(app_data_dir: &Path) -> PathBuf {
    state_dir(app_data_dir).join("skill-folders.json")
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::InvalidArgument {
        message: message.into(),
    }
}

fn validate_segment(value: &str) -> Result<(), AppError> {
    library::validate_folder_segment(value)
}

fn validate_path(value: &str) -> Result<(), AppError> {
    library::validate_folder_path(value)
}

fn validate_state(value: &SkillFolderState) -> Result<(), AppError> {
    if value.folders.len() > MAX_FOLDERS {
        return Err(invalid(format!(
            "at most {MAX_FOLDERS} folders are allowed"
        )));
    }
    for folder in &value.folders {
        validate_path(folder)?;
    }
    if value.folders.iter().enumerate().any(|(index, folder)| {
        value.folders[index + 1..]
            .iter()
            .any(|other| other.eq_ignore_ascii_case(folder))
    }) {
        return Err(invalid("folder paths must be unique"));
    }
    for assignment in &value.assignments {
        if assignment.source_id.trim().is_empty() || assignment.relative_path.trim().is_empty() {
            return Err(invalid("skill folder assignments require a skill identity"));
        }
        validate_path(&assignment.folder_path)?;
        if !value.folders.contains(&assignment.folder_path) {
            return Err(invalid(format!(
                "assigned folder does not exist: {}",
                assignment.folder_path
            )));
        }
    }
    if value.collections.len() > MAX_NAMED_ITEMS
        || value.smart_folders.len() > MAX_NAMED_ITEMS
        || value.profiles.len() > MAX_NAMED_ITEMS
        || value.approvals.len() > MAX_NAMED_ITEMS
        || value.publisher_trust.len() > MAX_NAMED_ITEMS
        || value.preferred_sources.len() > MAX_NAMED_ITEMS
        || value.usage.len() > MAX_FOLDERS
    {
        return Err(invalid(format!(
            "at most {MAX_NAMED_ITEMS} collections, smart folders, and profiles are allowed"
        )));
    }
    for reference in value
        .favorites
        .iter()
        .chain(value.recent.iter().map(|recent| &recent.skill))
        .chain(
            value
                .collections
                .iter()
                .flat_map(|collection| collection.skills.iter()),
        )
        .chain(value.update_policies.iter().map(|record| &record.skill))
        .chain(value.usage.iter().map(|record| &record.skill))
    {
        validate_reference(reference)?;
    }
    validate_named_unique(
        value.collections.iter().map(|item| item.name.as_str()),
        "collection",
    )?;
    validate_named_unique(
        value.smart_folders.iter().map(|item| item.name.as_str()),
        "smart folder",
    )?;
    validate_named_unique(
        value.profiles.iter().map(|item| item.name.as_str()),
        "profile",
    )?;
    for profile in &value.profiles {
        if profile
            .folders
            .iter()
            .any(|folder| !value.folders.contains(folder))
        {
            return Err(invalid(format!(
                "profile references a missing folder: {}",
                profile.name
            )));
        }
        if profile.collections.iter().any(|name| {
            !value
                .collections
                .iter()
                .any(|collection| collection.name == *name)
        }) {
            return Err(invalid(format!(
                "profile references a missing collection: {}",
                profile.name
            )));
        }
    }
    if value
        .update_policies
        .iter()
        .map(|record| &record.skill)
        .collect::<std::collections::HashSet<_>>()
        .len()
        != value.update_policies.len()
    {
        return Err(invalid("each skill may have only one update policy"));
    }
    for approval in &value.approvals {
        if approval.id.trim().is_empty()
            || approval.requested_by.trim().is_empty()
            || approval.requested_by.chars().count() > 64
        {
            return Err(invalid("approval identity fields are invalid"));
        }
        validate_approval_action(&approval.request)?;
    }
    for trust in &value.publisher_trust {
        validate_name(&trust.name)?;
        if trust.public_key.trim().is_empty()
            || trust.public_key.len() > 256
            || (trust.trusted && trust.revoked)
        {
            return Err(invalid("publisher trust record is invalid"));
        }
    }
    for preference in &value.preferred_sources {
        validate_name(&preference.skill_name)?;
        if preference.source_id.trim().is_empty() || preference.source_id.len() > 128 {
            return Err(invalid("preferred source record is invalid"));
        }
    }
    Ok(())
}

fn validate_approval_action(action: &SkillApprovalAction) -> Result<(), AppError> {
    match action {
        SkillApprovalAction::FolderCreate { path }
        | SkillApprovalAction::FolderDelete { path, .. } => validate_path(path),
        SkillApprovalAction::FolderRename { path, new_name } => {
            validate_path(path)?;
            validate_name(new_name)
        }
        SkillApprovalAction::FolderMove { path, new_parent } => {
            validate_path(path)?;
            if let Some(parent) = new_parent {
                validate_path(parent)?;
            }
            Ok(())
        }
        SkillApprovalAction::FolderAssign {
            source_id,
            relative_path,
            folder_path,
        } => {
            validate_reference(&SkillReference {
                source_id: source_id.clone(),
                relative_path: relative_path.clone(),
            })?;
            if let Some(folder) = folder_path {
                validate_path(folder)?;
            }
            Ok(())
        }
        SkillApprovalAction::Install {
            source_id,
            relative_path,
            runtime,
            project_path,
        } => {
            validate_reference(&SkillReference {
                source_id: source_id.clone(),
                relative_path: relative_path.clone(),
            })?;
            if !matches!(runtime.as_str(), "claudeCode" | "codex") {
                return Err(invalid(
                    "approval install runtime must be claudeCode or codex",
                ));
            }
            if project_path
                .as_ref()
                .is_some_and(|path| path.is_empty() || path.len() > 4096)
            {
                return Err(invalid("approval project path is invalid"));
            }
            Ok(())
        }
        SkillApprovalAction::CollectionDelete { name }
        | SkillApprovalAction::SmartFolderDelete { name }
        | SkillApprovalAction::ProfileDelete { name } => validate_name(name),
        SkillApprovalAction::UpdatePolicySet {
            source_id,
            relative_path,
            ..
        } => validate_reference(&SkillReference {
            source_id: source_id.clone(),
            relative_path: relative_path.clone(),
        }),
        SkillApprovalAction::Rollback {
            source_id,
            relative_path,
            runtime,
            snapshot_path,
            ..
        } => {
            validate_reference(&SkillReference {
                source_id: source_id.clone(),
                relative_path: relative_path.clone(),
            })?;
            if !matches!(runtime.as_str(), "claudeCode" | "codex") {
                return Err(invalid("runtime must be claudeCode or codex"));
            }
            if snapshot_path.trim().is_empty() {
                return Err(invalid("snapshot_path is required"));
            }
            Ok(())
        }
        SkillApprovalAction::PublisherTrustSet {
            name,
            public_key,
            trusted,
            revoked,
        } => {
            validate_name(name)?;
            if public_key.trim().is_empty() || public_key.len() > 256 || (*trusted && *revoked) {
                return Err(invalid("publisher trust request is invalid"));
            }
            Ok(())
        }
        SkillApprovalAction::BatchCollection {
            collection_name,
            operation,
            runtime,
            project_path,
        } => {
            validate_name(collection_name)?;
            if !matches!(operation.as_str(), "install" | "update" | "uninstall")
                || !matches!(runtime.as_str(), "claudeCode" | "codex")
                || project_path
                    .as_deref()
                    .is_some_and(|path| !Path::new(path).is_absolute())
            {
                return Err(invalid("batch collection request is invalid"));
            }
            Ok(())
        }
    }
}

fn validate_reference(value: &SkillReference) -> Result<(), AppError> {
    library::validate_reference(&value.source_id, &value.relative_path)
}

fn validate_name(value: &str) -> Result<(), AppError> {
    validate_segment(value)
}

fn validate_named_unique<'a>(
    values: impl Iterator<Item = &'a str>,
    kind: &str,
) -> Result<(), AppError> {
    let values = values.collect::<Vec<_>>();
    for value in &values {
        validate_name(value)?;
    }
    if values.iter().enumerate().any(|(index, value)| {
        values[index + 1..]
            .iter()
            .any(|other| other.eq_ignore_ascii_case(value))
    }) {
        return Err(invalid(format!("{kind} names must be unique")));
    }
    Ok(())
}

async fn load(app_data_dir: &Path) -> Result<SkillFolderState, AppError> {
    let path = folders_path(app_data_dir);
    let state = match tokio::fs::read(&path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| AppError::JsonParse {
            command: "skill_folders_list".into(),
            message: error.to_string(),
            raw_excerpt: String::from_utf8_lossy(&bytes[..bytes.len().min(256)]).into_owned(),
        })?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => SkillFolderState::default(),
        Err(error) => {
            return Err(AppError::Io {
                message: format!("read {}: {error}", path.display()),
            })
        }
    };
    validate_state(&state)?;
    Ok(state)
}

async fn save(app_data_dir: &Path, state: &SkillFolderState) -> Result<(), AppError> {
    validate_state(state)?;
    let directory = state_dir(app_data_dir);
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| AppError::Io {
            message: format!("create state dir {}: {error}", directory.display()),
        })?;
    let bytes = serde_json::to_vec_pretty(state).map_err(|error| AppError::Internal {
        message: format!("serialize skill-folders.json: {error}"),
    })?;
    atomic_write(&folders_path(app_data_dir), &bytes).await
}

fn create(state: &mut SkillFolderState, path: String) -> Result<(), AppError> {
    library::create_folder(&mut state.folders, path)
}

fn relocate(state: &mut SkillFolderState, path: &str, destination: String) -> Result<(), AppError> {
    let rewrites = library::rewrite_folder_paths(&state.folders, path, destination)?;
    for folder in &mut state.folders {
        if let Some((_, updated)) = rewrites.iter().find(|(current, _)| current == folder) {
            *folder = updated.clone();
        }
    }
    for assignment in &mut state.assignments {
        if let Some((_, updated)) = rewrites
            .iter()
            .find(|(current, _)| current == &assignment.folder_path)
        {
            assignment.folder_path = updated.clone();
        }
    }
    for profile in &mut state.profiles {
        for folder in &mut profile.folders {
            if let Some((_, updated)) = rewrites.iter().find(|(current, _)| current == folder) {
                *folder = updated.clone();
            }
        }
    }
    state.folders.sort();
    Ok(())
}

pub async fn list(state: &AppState) -> Result<SkillFolderState, AppError> {
    load(&state.app_data_dir).await
}

pub async fn create_folder(state: &AppState, path: String) -> Result<SkillFolderState, AppError> {
    let _guard = state.skill_folders_write_lock.lock().await;
    let mut folders = load(&state.app_data_dir).await?;
    create(&mut folders, path)?;
    save(&state.app_data_dir, &folders).await?;
    Ok(folders)
}

pub async fn rename_folder(
    state: &AppState,
    path: String,
    new_name: String,
) -> Result<SkillFolderState, AppError> {
    validate_segment(&new_name)?;
    let parent = path.rsplit_once('/').map(|(parent, _)| parent);
    let destination = parent
        .map(|parent| format!("{parent}/{new_name}"))
        .unwrap_or(new_name);
    let _guard = state.skill_folders_write_lock.lock().await;
    let mut folders = load(&state.app_data_dir).await?;
    relocate(&mut folders, &path, destination)?;
    save(&state.app_data_dir, &folders).await?;
    Ok(folders)
}

pub async fn move_folder(
    state: &AppState,
    path: String,
    new_parent: Option<String>,
) -> Result<SkillFolderState, AppError> {
    if let Some(parent) = &new_parent {
        validate_path(parent)?;
    }
    let name = path
        .rsplit('/')
        .next()
        .ok_or_else(|| invalid("folder path is empty"))?;
    let destination = new_parent
        .as_ref()
        .map(|parent| format!("{parent}/{name}"))
        .unwrap_or_else(|| name.to_owned());
    let _guard = state.skill_folders_write_lock.lock().await;
    let mut folders = load(&state.app_data_dir).await?;
    if let Some(parent) = &new_parent {
        if !folders.folders.contains(parent) {
            return Err(invalid(format!("parent folder does not exist: {parent}")));
        }
    }
    relocate(&mut folders, &path, destination)?;
    save(&state.app_data_dir, &folders).await?;
    Ok(folders)
}

pub async fn delete_folder(
    state: &AppState,
    path: String,
    recursive: bool,
) -> Result<SkillFolderState, AppError> {
    validate_path(&path)?;
    let _guard = state.skill_folders_write_lock.lock().await;
    let mut folders = load(&state.app_data_dir).await?;
    let prefix = format!("{path}/");
    let assigned = folders.assignments.iter().any(|assignment| {
        assignment.folder_path == path || assignment.folder_path.starts_with(&prefix)
    });
    if assigned && !recursive {
        return Err(invalid(
            "folder is not empty; set recursive=true to remove descendants and assignments",
        ));
    }
    let removed = library::deleted_folder_paths(&folders.folders, &path, recursive)?;
    folders.folders.retain(|folder| !removed.contains(folder));
    folders.assignments.retain(|assignment| {
        assignment.folder_path != path && !assignment.folder_path.starts_with(&prefix)
    });
    for profile in &mut folders.profiles {
        profile
            .folders
            .retain(|folder| folder != &path && !folder.starts_with(&prefix));
    }
    save(&state.app_data_dir, &folders).await?;
    Ok(folders)
}

pub async fn assign_folder(
    state: &AppState,
    source_id: String,
    relative_path: String,
    folder_path: Option<String>,
) -> Result<SkillFolderState, AppError> {
    if source_id.trim().is_empty() || relative_path.trim().is_empty() {
        return Err(invalid("source_id and relative_path are required"));
    }
    let _guard = state.skill_folders_write_lock.lock().await;
    let mut folders = load(&state.app_data_dir).await?;
    folders.assignments.retain(|assignment| {
        assignment.source_id != source_id || assignment.relative_path != relative_path
    });
    if let Some(folder_path) = folder_path {
        validate_path(&folder_path)?;
        if !folders.folders.contains(&folder_path) {
            return Err(invalid(format!("folder does not exist: {folder_path}")));
        }
        folders.assignments.push(SkillFolderAssignment {
            source_id,
            relative_path,
            folder_path,
        });
    }
    folders.assignments.sort_by(|left, right| {
        (&left.source_id, &left.relative_path).cmp(&(&right.source_id, &right.relative_path))
    });
    save(&state.app_data_dir, &folders).await?;
    Ok(folders)
}

pub async fn import_folders(
    state: &AppState,
    imported: SkillFolderState,
) -> Result<SkillFolderState, AppError> {
    validate_state(&imported)?;
    let _guard = state.skill_folders_write_lock.lock().await;
    let mut folders = load(&state.app_data_dir).await?;
    for path in imported.folders {
        if !folders
            .folders
            .iter()
            .any(|current| current.eq_ignore_ascii_case(&path))
        {
            folders.folders.push(path);
        }
    }
    for assignment in imported.assignments {
        folders.assignments.retain(|current| {
            current.source_id != assignment.source_id
                || current.relative_path != assignment.relative_path
        });
        folders.assignments.push(assignment);
    }
    folders.folders.sort();
    folders.assignments.sort_by(|left, right| {
        (&left.source_id, &left.relative_path).cmp(&(&right.source_id, &right.relative_path))
    });
    save(&state.app_data_dir, &folders).await?;
    Ok(folders)
}

pub async fn set_favorite(
    state: &AppState,
    skill: SkillReference,
    favorite: bool,
) -> Result<SkillFolderState, AppError> {
    validate_reference(&skill)?;
    let _guard = state.skill_folders_write_lock.lock().await;
    let mut library = load(&state.app_data_dir).await?;
    library.favorites.retain(|current| current != &skill);
    if favorite {
        library.favorites.push(skill);
        library.favorites.sort_by(|left, right| {
            (&left.source_id, &left.relative_path).cmp(&(&right.source_id, &right.relative_path))
        });
    }
    save(&state.app_data_dir, &library).await?;
    Ok(library)
}

pub async fn touch_recent(
    state: &AppState,
    skill: SkillReference,
) -> Result<SkillFolderState, AppError> {
    validate_reference(&skill)?;
    let _guard = state.skill_folders_write_lock.lock().await;
    let mut library = load(&state.app_data_dir).await?;
    library.recent.retain(|current| current.skill != skill);
    library.recent.insert(
        0,
        SkillRecent {
            skill,
            viewed_at: chrono::Utc::now().to_rfc3339(),
        },
    );
    library.recent.truncate(MAX_RECENT);
    save(&state.app_data_dir, &library).await?;
    Ok(library)
}

pub async fn save_collection(
    state: &AppState,
    collection: SkillCollection,
) -> Result<SkillFolderState, AppError> {
    validate_name(&collection.name)?;
    for skill in &collection.skills {
        validate_reference(skill)?;
    }
    let _guard = state.skill_folders_write_lock.lock().await;
    let mut library = load(&state.app_data_dir).await?;
    library
        .collections
        .retain(|current| !current.name.eq_ignore_ascii_case(&collection.name));
    library.collections.push(collection);
    library
        .collections
        .sort_by(|left, right| left.name.cmp(&right.name));
    save(&state.app_data_dir, &library).await?;
    Ok(library)
}

pub async fn delete_collection(
    state: &AppState,
    name: String,
) -> Result<SkillFolderState, AppError> {
    validate_name(&name)?;
    let _guard = state.skill_folders_write_lock.lock().await;
    let mut library = load(&state.app_data_dir).await?;
    let before = library.collections.len();
    library.collections.retain(|current| current.name != name);
    if library.collections.len() == before {
        return Err(invalid(format!("collection does not exist: {name}")));
    }
    for profile in &mut library.profiles {
        profile.collections.retain(|current| current != &name);
    }
    save(&state.app_data_dir, &library).await?;
    Ok(library)
}

pub async fn save_smart_folder(
    state: &AppState,
    smart_folder: SkillSmartFolder,
) -> Result<SkillFolderState, AppError> {
    validate_name(&smart_folder.name)?;
    if smart_folder.rule == Default::default() {
        return Err(invalid("a smart folder requires at least one rule"));
    }
    let _guard = state.skill_folders_write_lock.lock().await;
    let mut library = load(&state.app_data_dir).await?;
    library
        .smart_folders
        .retain(|current| !current.name.eq_ignore_ascii_case(&smart_folder.name));
    library.smart_folders.push(smart_folder);
    library
        .smart_folders
        .sort_by(|left, right| left.name.cmp(&right.name));
    save(&state.app_data_dir, &library).await?;
    Ok(library)
}

pub async fn delete_smart_folder(
    state: &AppState,
    name: String,
) -> Result<SkillFolderState, AppError> {
    validate_name(&name)?;
    let _guard = state.skill_folders_write_lock.lock().await;
    let mut library = load(&state.app_data_dir).await?;
    let before = library.smart_folders.len();
    library.smart_folders.retain(|current| current.name != name);
    if library.smart_folders.len() == before {
        return Err(invalid(format!("smart folder does not exist: {name}")));
    }
    save(&state.app_data_dir, &library).await?;
    Ok(library)
}

pub async fn save_profile(
    state: &AppState,
    profile: SkillWorkspaceProfile,
) -> Result<SkillFolderState, AppError> {
    validate_name(&profile.name)?;
    let _guard = state.skill_folders_write_lock.lock().await;
    let mut library = load(&state.app_data_dir).await?;
    library
        .profiles
        .retain(|current| !current.name.eq_ignore_ascii_case(&profile.name));
    library.profiles.push(profile);
    library
        .profiles
        .sort_by(|left, right| left.name.cmp(&right.name));
    save(&state.app_data_dir, &library).await?;
    Ok(library)
}

pub async fn delete_profile(state: &AppState, name: String) -> Result<SkillFolderState, AppError> {
    validate_name(&name)?;
    let _guard = state.skill_folders_write_lock.lock().await;
    let mut library = load(&state.app_data_dir).await?;
    let before = library.profiles.len();
    library.profiles.retain(|current| current.name != name);
    if library.profiles.len() == before {
        return Err(invalid(format!("profile does not exist: {name}")));
    }
    save(&state.app_data_dir, &library).await?;
    Ok(library)
}

pub async fn replace_library(
    state: &AppState,
    replacement: SkillFolderState,
) -> Result<SkillFolderState, AppError> {
    validate_state(&replacement)?;
    let _guard = state.skill_folders_write_lock.lock().await;
    save(&state.app_data_dir, &replacement).await?;
    Ok(replacement)
}

pub async fn set_update_policy(
    state: &AppState,
    skill: SkillReference,
    policy: SkillUpdatePolicy,
) -> Result<SkillFolderState, AppError> {
    validate_reference(&skill)?;
    let _guard = state.skill_folders_write_lock.lock().await;
    let mut library = load(&state.app_data_dir).await?;
    library
        .update_policies
        .retain(|record| record.skill != skill);
    library
        .update_policies
        .push(SkillUpdatePolicyRecord { skill, policy });
    library.update_policies.sort_by(|left, right| {
        (&left.skill.source_id, &left.skill.relative_path)
            .cmp(&(&right.skill.source_id, &right.skill.relative_path))
    });
    save(&state.app_data_dir, &library).await?;
    Ok(library)
}

pub async fn set_publisher_trust(
    state: &AppState,
    trust: SkillPublisherTrust,
) -> Result<SkillFolderState, AppError> {
    validate_name(&trust.name)?;
    if trust.public_key.trim().is_empty() || trust.public_key.len() > 256 {
        return Err(invalid(
            "publisher public key must contain 1-256 characters",
        ));
    }
    if trust.trusted && trust.revoked {
        return Err(invalid("a publisher key cannot be trusted and revoked"));
    }
    let _guard = state.skill_folders_write_lock.lock().await;
    let mut library = load(&state.app_data_dir).await?;
    library
        .publisher_trust
        .retain(|current| current.public_key != trust.public_key);
    library.publisher_trust.push(trust);
    save(&state.app_data_dir, &library).await?;
    Ok(library)
}

pub async fn set_preferred_source(
    state: &AppState,
    preference: SkillPreferredSource,
) -> Result<SkillFolderState, AppError> {
    validate_name(&preference.skill_name)?;
    if preference.source_id.trim().is_empty() || preference.source_id.len() > 128 {
        return Err(invalid("source_id must contain 1-128 characters"));
    }
    let _guard = state.skill_folders_write_lock.lock().await;
    let mut library = load(&state.app_data_dir).await?;
    library
        .preferred_sources
        .retain(|current| current.skill_name != preference.skill_name);
    library.preferred_sources.push(preference);
    save(&state.app_data_dir, &library).await?;
    Ok(library)
}

pub async fn record_usage(
    state: &AppState,
    skill: SkillReference,
    event: &str,
) -> Result<SkillFolderState, AppError> {
    validate_reference(&skill)?;
    if !matches!(event, "fetch" | "install" | "reject") {
        return Err(invalid("usage event must be fetch, install, or reject"));
    }
    let _guard = state.skill_folders_write_lock.lock().await;
    let mut library = load(&state.app_data_dir).await?;
    let usage = if let Some(usage) = library.usage.iter_mut().find(|item| item.skill == skill) {
        usage
    } else {
        library.usage.push(SkillUsage {
            skill,
            fetches: 0,
            installs: 0,
            rejections: 0,
            last_used_at: String::new(),
        });
        library.usage.last_mut().expect("usage was inserted")
    };
    match event {
        "fetch" => usage.fetches = usage.fetches.saturating_add(1),
        "install" => usage.installs = usage.installs.saturating_add(1),
        "reject" => usage.rejections = usage.rejections.saturating_add(1),
        _ => unreachable!(),
    }
    usage.last_used_at = chrono::Utc::now().to_rfc3339();
    save(&state.app_data_dir, &library).await?;
    Ok(library)
}

pub async fn export_library(state: &AppState, path: String) -> Result<u32, AppError> {
    let library = load(&state.app_data_dir).await?;
    let bytes = serde_json::to_vec_pretty(&library).map_err(|error| AppError::Internal {
        message: format!("serialize skill library export: {error}"),
    })?;
    atomic_write(Path::new(&path), &bytes).await?;
    Ok(library.folders.len() as u32
        + library.collections.len() as u32
        + library.smart_folders.len() as u32
        + library.profiles.len() as u32)
}

pub async fn import_library(state: &AppState, path: String) -> Result<SkillFolderState, AppError> {
    let bytes = tokio::fs::read(&path).await.map_err(|error| AppError::Io {
        message: format!("read {path}: {error}"),
    })?;
    if bytes.len() > 1024 * 1024 {
        return Err(invalid("skill library import exceeds 1 MiB"));
    }
    let replacement = serde_json::from_slice::<SkillFolderState>(&bytes).map_err(|error| {
        AppError::JsonParse {
            command: "skill_library_import".into(),
            message: error.to_string(),
            raw_excerpt: String::from_utf8_lossy(&bytes[..bytes.len().min(256)]).into_owned(),
        }
    })?;
    replace_library(state, replacement).await
}

pub async fn submit_approval(
    state: &AppState,
    requested_by: String,
    request: SkillApprovalAction,
) -> Result<SkillApproval, AppError> {
    if requested_by.trim().is_empty() || requested_by.chars().count() > 64 {
        return Err(invalid("requested_by must contain 1-64 characters"));
    }
    validate_approval_action(&request)?;
    let _guard = state.skill_folders_write_lock.lock().await;
    let mut library = load(&state.app_data_dir).await?;
    if library.approvals.len() == MAX_NAMED_ITEMS {
        if let Some(index) = library
            .approvals
            .iter()
            .position(|approval| approval.state != SkillApprovalState::Pending)
        {
            library.approvals.remove(index);
        } else {
            return Err(invalid("approval inbox is full"));
        }
    }
    let approval = SkillApproval {
        id: uuid::Uuid::new_v4().to_string(),
        submitted_at: chrono::Utc::now().to_rfc3339(),
        state: SkillApprovalState::Pending,
        requested_by,
        request,
        result: None,
    };
    library.approvals.push(approval.clone());
    save(&state.app_data_dir, &library).await?;
    Ok(approval)
}

pub async fn approve(state: &AppState, id: String) -> Result<SkillApproval, AppError> {
    let request = {
        let _guard = state.skill_folders_write_lock.lock().await;
        let mut library = load(&state.app_data_dir).await?;
        let approval = library
            .approvals
            .iter_mut()
            .find(|approval| approval.id == id)
            .ok_or_else(|| invalid(format!("approval does not exist: {id}")))?;
        if approval.state != SkillApprovalState::Pending {
            return Err(invalid("only pending approvals can be approved"));
        }
        approval.state = SkillApprovalState::Running;
        let request = approval.request.clone();
        save(&state.app_data_dir, &library).await?;
        request
    };

    let operation = match request {
        SkillApprovalAction::FolderCreate { path } => create_folder(state, path).await.map(|_| ()),
        SkillApprovalAction::FolderRename { path, new_name } => {
            rename_folder(state, path, new_name).await.map(|_| ())
        }
        SkillApprovalAction::FolderMove { path, new_parent } => {
            move_folder(state, path, new_parent).await.map(|_| ())
        }
        SkillApprovalAction::FolderDelete { path, recursive } => {
            delete_folder(state, path, recursive).await.map(|_| ())
        }
        SkillApprovalAction::FolderAssign {
            source_id,
            relative_path,
            folder_path,
        } => assign_folder(state, source_id, relative_path, folder_path)
            .await
            .map(|_| ()),
        SkillApprovalAction::Install {
            source_id,
            relative_path,
            runtime,
            project_path,
        } => super::install_skill(
            state,
            &source_id,
            &relative_path,
            &runtime,
            project_path.as_deref(),
        )
        .await
        .map(|_| ()),
        SkillApprovalAction::CollectionDelete { name } => {
            delete_collection(state, name).await.map(|_| ())
        }
        SkillApprovalAction::SmartFolderDelete { name } => {
            delete_smart_folder(state, name).await.map(|_| ())
        }
        SkillApprovalAction::ProfileDelete { name } => {
            delete_profile(state, name).await.map(|_| ())
        }
        SkillApprovalAction::UpdatePolicySet {
            source_id,
            relative_path,
            policy,
        } => set_update_policy(
            state,
            SkillReference {
                source_id,
                relative_path,
            },
            policy,
        )
        .await
        .map(|_| ()),
        SkillApprovalAction::Rollback {
            source_id,
            relative_path,
            runtime,
            project_path,
            snapshot_path,
        } => super::rollback_skill_authorized(
            state,
            &source_id,
            &relative_path,
            &runtime,
            project_path.as_deref(),
            &snapshot_path,
            None,
        )
        .await
        .map(|_| ()),
        SkillApprovalAction::PublisherTrustSet {
            name,
            public_key,
            trusted,
            revoked,
        } => set_publisher_trust(
            state,
            SkillPublisherTrust {
                name,
                public_key,
                trusted,
                revoked,
            },
        )
        .await
        .map(|_| ()),
        SkillApprovalAction::BatchCollection {
            collection_name,
            operation,
            runtime,
            project_path,
        } => super::batch_collection(
            state,
            &collection_name,
            &operation,
            &runtime,
            project_path.as_deref(),
        )
        .await
        .map(|_| ()),
    };

    let _guard = state.skill_folders_write_lock.lock().await;
    let mut library = load(&state.app_data_dir).await?;
    let approval = library
        .approvals
        .iter_mut()
        .find(|approval| approval.id == id)
        .ok_or_else(|| invalid(format!("approval disappeared: {id}")))?;
    match operation {
        Ok(()) => {
            approval.state = SkillApprovalState::Approved;
            approval.result = Some("completed".into());
        }
        Err(error) => {
            approval.state = SkillApprovalState::Pending;
            approval.result = Some(error.to_string());
        }
    }
    let result = approval.clone();
    save(&state.app_data_dir, &library).await?;
    Ok(result)
}

pub async fn reject_approval(state: &AppState, id: String) -> Result<SkillApproval, AppError> {
    let _guard = state.skill_folders_write_lock.lock().await;
    let mut library = load(&state.app_data_dir).await?;
    let approval = library
        .approvals
        .iter_mut()
        .find(|approval| approval.id == id)
        .ok_or_else(|| invalid(format!("approval does not exist: {id}")))?;
    if approval.state != SkillApprovalState::Pending {
        return Err(invalid("only pending approvals can be rejected"));
    }
    approval.state = SkillApprovalState::Rejected;
    approval.result = Some("rejected by desktop user".into());
    let result = approval.clone();
    save(&state.app_data_dir, &library).await?;
    Ok(result)
}

#[tauri::command]
pub async fn skill_folders_list(state: State<'_, AppState>) -> Result<SkillFolderState, AppError> {
    list(&state).await
}

#[tauri::command]
pub async fn skill_folder_create(
    state: State<'_, AppState>,
    path: String,
) -> Result<SkillFolderState, AppError> {
    create_folder(&state, path).await
}

#[tauri::command]
pub async fn skill_folder_rename(
    state: State<'_, AppState>,
    path: String,
    new_name: String,
) -> Result<SkillFolderState, AppError> {
    rename_folder(&state, path, new_name).await
}

#[tauri::command]
pub async fn skill_folder_move(
    state: State<'_, AppState>,
    path: String,
    new_parent: Option<String>,
) -> Result<SkillFolderState, AppError> {
    move_folder(&state, path, new_parent).await
}

#[tauri::command]
pub async fn skill_folder_delete(
    state: State<'_, AppState>,
    path: String,
    recursive: bool,
) -> Result<SkillFolderState, AppError> {
    delete_folder(&state, path, recursive).await
}

#[tauri::command]
pub async fn skill_folder_assign(
    state: State<'_, AppState>,
    source_id: String,
    relative_path: String,
    folder_path: Option<String>,
) -> Result<SkillFolderState, AppError> {
    assign_folder(&state, source_id, relative_path, folder_path).await
}

#[tauri::command]
pub async fn skill_folders_import(
    state: State<'_, AppState>,
    imported: SkillFolderState,
) -> Result<SkillFolderState, AppError> {
    import_folders(&state, imported).await
}

#[tauri::command]
pub async fn skill_favorite_set(
    state: State<'_, AppState>,
    skill: SkillReference,
    favorite: bool,
) -> Result<SkillFolderState, AppError> {
    set_favorite(&state, skill, favorite).await
}

#[tauri::command]
pub async fn skill_recent_touch(
    state: State<'_, AppState>,
    skill: SkillReference,
) -> Result<SkillFolderState, AppError> {
    touch_recent(&state, skill).await
}

#[tauri::command]
pub async fn skill_collection_save(
    state: State<'_, AppState>,
    collection: SkillCollection,
) -> Result<SkillFolderState, AppError> {
    save_collection(&state, collection).await
}

#[tauri::command]
pub async fn skill_collection_delete(
    state: State<'_, AppState>,
    name: String,
) -> Result<SkillFolderState, AppError> {
    delete_collection(&state, name).await
}

#[tauri::command]
pub async fn skill_smart_folder_save(
    state: State<'_, AppState>,
    smart_folder: SkillSmartFolder,
) -> Result<SkillFolderState, AppError> {
    save_smart_folder(&state, smart_folder).await
}

#[tauri::command]
pub async fn skill_smart_folder_delete(
    state: State<'_, AppState>,
    name: String,
) -> Result<SkillFolderState, AppError> {
    delete_smart_folder(&state, name).await
}

#[tauri::command]
pub async fn skill_profile_save(
    state: State<'_, AppState>,
    profile: SkillWorkspaceProfile,
) -> Result<SkillFolderState, AppError> {
    save_profile(&state, profile).await
}

#[tauri::command]
pub async fn skill_profile_delete(
    state: State<'_, AppState>,
    name: String,
) -> Result<SkillFolderState, AppError> {
    delete_profile(&state, name).await
}

#[tauri::command]
pub async fn skill_library_replace(
    state: State<'_, AppState>,
    replacement: SkillFolderState,
) -> Result<SkillFolderState, AppError> {
    replace_library(&state, replacement).await
}

#[tauri::command]
pub async fn skill_library_export(
    state: State<'_, AppState>,
    path: String,
) -> Result<u32, AppError> {
    export_library(&state, path).await
}

#[tauri::command]
pub async fn skill_library_import(
    state: State<'_, AppState>,
    path: String,
) -> Result<SkillFolderState, AppError> {
    import_library(&state, path).await
}

#[tauri::command]
pub async fn skill_update_policy_set(
    state: State<'_, AppState>,
    skill: SkillReference,
    policy: SkillUpdatePolicy,
) -> Result<SkillFolderState, AppError> {
    set_update_policy(&state, skill, policy).await
}

#[tauri::command]
pub async fn skill_publisher_trust_set(
    state: State<'_, AppState>,
    trust: SkillPublisherTrust,
) -> Result<SkillFolderState, AppError> {
    set_publisher_trust(&state, trust).await
}

#[tauri::command]
pub async fn skill_preferred_source_set(
    state: State<'_, AppState>,
    preference: SkillPreferredSource,
) -> Result<SkillFolderState, AppError> {
    set_preferred_source(&state, preference).await
}

#[tauri::command]
pub async fn skill_approval_approve(
    state: State<'_, AppState>,
    id: String,
) -> Result<SkillApproval, AppError> {
    approve(&state, id).await
}

#[tauri::command]
pub async fn skill_approval_reject(
    state: State<'_, AppState>,
    id: String,
) -> Result<SkillApproval, AppError> {
    reject_approval(&state, id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use tokio::sync::{Mutex, RwLock};

    use crate::commands::settings::SettingsLoadState;

    fn state() -> SkillFolderState {
        SkillFolderState {
            folders: vec!["Engineering".into(), "Engineering/Frontend".into()],
            assignments: vec![SkillFolderAssignment {
                source_id: "source".into(),
                relative_path: "skill".into(),
                folder_path: "Engineering/Frontend".into(),
            }],
            ..Default::default()
        }
    }

    fn app_state(root: &Path) -> AppState {
        AppState {
            app_data_dir: root.to_path_buf(),
            corpus_cache: Arc::new(Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(Mutex::new(())),
            skill_sources_write_lock: Arc::new(Mutex::new(())),
            skill_installs_write_lock: Arc::new(Mutex::new(())),
            skill_folders_write_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(RwLock::new(SettingsLoadState::FirstLaunch)),
            updater_state: crate::commands::updater::empty_state(),
        }
    }

    #[test]
    fn rename_updates_descendants_and_assignments_once() {
        let mut folders = state();
        relocate(&mut folders, "Engineering", "Development".into()).unwrap();
        assert_eq!(folders.folders, ["Development", "Development/Frontend"]);
        assert_eq!(folders.assignments[0].folder_path, "Development/Frontend");
    }

    #[test]
    fn move_rejects_descendant_destination_and_collisions() {
        let mut folders = state();
        assert!(relocate(
            &mut folders,
            "Engineering",
            "Engineering/Frontend/Engineering".into()
        )
        .is_err());
        assert!(relocate(&mut folders, "Engineering/Frontend", "Engineering".into()).is_err());
    }

    #[test]
    fn create_requires_existing_parent_and_valid_name() {
        let mut folders = SkillFolderState::default();
        assert!(create(&mut folders, "Missing/Child".into()).is_err());
        create(&mut folders, "Parent".into()).unwrap();
        create(&mut folders, "Parent/Child".into()).unwrap();
        assert!(create(&mut folders, "Parent/Child".into()).is_err());
        assert!(create(&mut folders, " Parent".into()).is_err());
    }

    #[test]
    fn persisted_state_rejects_orphan_assignments() {
        let mut folders = state();
        folders.assignments[0].folder_path = "Missing".into();
        assert!(validate_state(&folders).is_err());
    }

    #[test]
    fn folder_boundaries_and_case_collisions_are_enforced() {
        let deepest = (0..MAX_FOLDER_DEPTH)
            .map(|index| format!("f{index}"))
            .collect::<Vec<_>>()
            .join("/");
        assert!(validate_path(&deepest).is_ok());
        assert!(validate_path(&format!("{deepest}/too-deep")).is_err());
        assert!(validate_segment(&"x".repeat(MAX_FOLDER_SEGMENT_CHARS)).is_ok());
        assert!(validate_segment(&"x".repeat(MAX_FOLDER_SEGMENT_CHARS + 1)).is_err());

        let mut folders = SkillFolderState {
            folders: vec!["Engineering".into(), "engineering".into()],
            ..Default::default()
        };
        assert!(validate_state(&folders).is_err());

        folders.folders = (0..MAX_FOLDERS).map(|index| format!("f{index}")).collect();
        assert!(validate_state(&folders).is_ok());
        folders.folders.push("one-too-many".into());
        assert!(validate_state(&folders).is_err());
    }

    #[test]
    fn batch_approval_rejects_unknown_operations() {
        assert!(
            validate_approval_action(&SkillApprovalAction::BatchCollection {
                collection_name: "Review set".into(),
                operation: "erase".into(),
                runtime: "codex".into(),
                project_path: None,
            })
            .is_err()
        );
    }

    #[tokio::test]
    async fn mutations_persist_and_non_recursive_delete_is_safe() {
        let root = tempfile::tempdir().unwrap();
        let app = app_state(root.path());
        create_folder(&app, "Engineering".into()).await.unwrap();
        create_folder(&app, "Engineering/Frontend".into())
            .await
            .unwrap();
        assign_folder(
            &app,
            "source".into(),
            "skill".into(),
            Some("Engineering/Frontend".into()),
        )
        .await
        .unwrap();

        assert!(delete_folder(&app, "Engineering".into(), false)
            .await
            .is_err());
        delete_folder(&app, "Engineering".into(), true)
            .await
            .unwrap();

        assert_eq!(list(&app).await.unwrap(), SkillFolderState::default());
    }

    #[tokio::test]
    async fn trust_preferences_and_usage_are_validated_and_persisted() {
        let root = tempfile::tempdir().unwrap();
        let app = app_state(root.path());
        set_publisher_trust(
            &app,
            SkillPublisherTrust {
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
            SkillPreferredSource {
                skill_name: "reviewer".into(),
                source_id: "source".into(),
            },
        )
        .await
        .unwrap();
        record_usage(
            &app,
            SkillReference {
                source_id: "source".into(),
                relative_path: "reviewer".into(),
            },
            "fetch",
        )
        .await
        .unwrap();

        let saved = list(&app).await.unwrap();
        assert_eq!(saved.publisher_trust.len(), 1);
        assert_eq!(saved.preferred_sources.len(), 1);
        assert_eq!(saved.usage[0].fetches, 1);
        assert!(set_publisher_trust(
            &app,
            SkillPublisherTrust {
                name: "Acme".into(),
                public_key: "public-key".into(),
                trusted: true,
                revoked: true,
            },
        )
        .await
        .is_err());
    }
}
