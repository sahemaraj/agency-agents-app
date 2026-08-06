use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::error::AppError;
use crate::library;
use crate::state::AppState;
use crate::types::{
    AgentDraft, AgentDraftInput, AgentDraftState, AgentPackageResult, AgentReference, AgentSource,
    AgentSourceKind, AgentValidationCode, AgentValidationError, SkillPackageResult, SkillReference,
};

const MAX_AGENT_DRAFTS: usize = 256;
const PUBLISHED_AGENT_SOURCE_ID: &str = "published:agents";

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PublishOperation {
    draft_id: String,
    relative_path: String,
    expected_hash: String,
    #[serde(default)]
    approval_id: Option<String>,
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::InvalidArgument {
        message: message.into(),
    }
}

fn index_path(app_data_dir: &Path) -> PathBuf {
    super::corpus::state_dir(app_data_dir).join("agent-drafts.json")
}

fn ensure_owned_root(app_data_dir: &Path, name: &str) -> Result<PathBuf, AppError> {
    let mut current = app_data_dir.to_path_buf();
    for segment in ["agents", name] {
        current.push(segment);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata)
                if metadata.is_dir()
                    && !metadata.file_type().is_symlink()
                    && !crate::skills::metadata_is_reparse_point(&metadata) => {}
            Ok(_) => {
                return Err(invalid(format!(
                    "app-owned Agent {name} path must be a real directory: {}",
                    current.display()
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&current).map_err(|error| AppError::Io {
                    message: format!(
                        "create Agent {name} directory {}: {error}",
                        current.display()
                    ),
                })?;
            }
            Err(error) => {
                return Err(AppError::Io {
                    message: format!(
                        "inspect Agent {name} directory {}: {error}",
                        current.display()
                    ),
                });
            }
        }
    }
    std::fs::canonicalize(&current).map_err(|error| AppError::Io {
        message: format!(
            "resolve Agent {name} directory {}: {error}",
            current.display()
        ),
    })
}

fn lock_drafts(app_data_dir: &Path) -> Result<File, AppError> {
    let directory = super::corpus::state_dir(app_data_dir);
    std::fs::create_dir_all(&directory).map_err(|error| AppError::Io {
        message: format!("create Agent draft state directory: {error}"),
    })?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("agent-drafts.lock"))
        .map_err(|error| AppError::Io {
            message: format!("open Agent draft lock: {error}"),
        })?;
    file.lock().map_err(|error| AppError::Io {
        message: format!("lock Agent drafts: {error}"),
    })?;
    Ok(file)
}

async fn lock_drafts_async(app_data_dir: PathBuf) -> Result<File, AppError> {
    tokio::task::spawn_blocking(move || lock_drafts(&app_data_dir))
        .await
        .map_err(|error| AppError::Internal {
            message: format!("Agent draft lock task failed: {error}"),
        })?
}

async fn load_index(app_data_dir: &Path) -> Result<Vec<AgentDraft>, AppError> {
    if let Some(database) = crate::state_db::StateDatabase::completed(app_data_dir).await? {
        return database
            .read(document_spec())
            .await?
            .ok_or_else(|| AppError::StorageCorrupt {
                message: "Agent drafts are missing after SQLite migration".into(),
            });
    }
    let drafts = match tokio::fs::read(index_path(app_data_dir)).await {
        Ok(bytes) => serde_json::from_slice::<Vec<AgentDraft>>(&bytes).map_err(|error| {
            AppError::JsonParse {
                command: "agent_drafts_list".into(),
                message: error.to_string(),
                raw_excerpt: String::from_utf8_lossy(&bytes[..bytes.len().min(256)]).into_owned(),
            }
        })?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            return Err(AppError::Io {
                message: format!("read Agent draft index: {error}"),
            });
        }
    };
    validate_index(&drafts)?;
    Ok(drafts)
}

fn validate_index(drafts: &[AgentDraft]) -> Result<(), AppError> {
    if drafts.len() > MAX_AGENT_DRAFTS
        || drafts.iter().any(|draft| {
            Uuid::parse_str(&draft.id).is_err()
                || library::normalize_relative_path(&draft.relative_path).is_err()
        })
    {
        return Err(invalid("Agent draft index is invalid or exceeds its limit"));
    }
    Ok(())
}

fn document_spec() -> crate::state_db::DocumentSpec<Vec<AgentDraft>> {
    crate::state_db::DocumentSpec::new("agent_drafts", 1, 8_388_608, |drafts| {
        validate_index(drafts)
    })
}

pub(crate) fn import_spec() -> crate::state_db::ImportSpec {
    crate::state_db::ImportSpec::document(document_spec(), Vec::new())
}

async fn save_index(app_data_dir: &Path, drafts: &[AgentDraft]) -> Result<(), AppError> {
    validate_index(drafts)?;
    if let Some(database) = crate::state_db::StateDatabase::completed(app_data_dir).await? {
        let replacement = drafts.to_vec();
        return database
            .mutate(document_spec(), Vec::new(), move |current| {
                *current = replacement;
                Ok(())
            })
            .await;
    }
    let bytes = serde_json::to_vec_pretty(drafts).map_err(|error| AppError::Internal {
        message: format!("serialize Agent draft index: {error}"),
    })?;
    crate::util::fs::atomic_write(&index_path(app_data_dir), &bytes).await
}

async fn save_index_after_filesystem(
    app_data_dir: &Path,
    drafts: &[AgentDraft],
    operation_id: Option<&str>,
) -> Result<(), AppError> {
    let Some(operation_id) = operation_id else {
        return save_index(app_data_dir, drafts).await;
    };
    validate_index(drafts)?;
    let database = crate::state_db::StateDatabase::completed(app_data_dir)
        .await?
        .ok_or_else(|| AppError::StorageCorrupt {
            message: "Agent filesystem operation lost its SQLite database".into(),
        })?;
    let replacement = drafts.to_vec();
    database
        .mutate_after_filesystem(document_spec(), Vec::new(), operation_id, move |current| {
            *current = replacement;
            Ok(())
        })
        .await
}

fn empty_validation(id: &str, input: &AgentDraftInput) -> AgentPackageResult {
    AgentPackageResult {
        reference: AgentReference {
            source_id: format!("draft:{id}"),
            relative_path: input.relative_path.clone(),
        },
        agent: None,
        source_hash: hex::encode(Sha256::digest(input.text.as_bytes())),
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
        diagnostics: vec![AgentValidationError {
            code: AgentValidationCode::InvalidMetadata,
            path: input.relative_path.clone(),
            message: "Agent Markdown requires valid YAML frontmatter".into(),
        }],
        installable: false,
    }
}

fn validate_input(input: &AgentDraftInput) -> Result<(), AppError> {
    library::normalize_relative_path(&input.relative_path)?;
    if Path::new(&input.relative_path)
        .extension()
        .and_then(|value| value.to_str())
        != Some("md")
    {
        return Err(invalid("Agent drafts must use a .md relative path"));
    }
    if input.text.len() as u64 > super::corpus::MAX_AGENT_BYTES {
        return Err(invalid("Agent draft exceeds the 1 MiB limit"));
    }
    Ok(())
}

async fn stage_input(
    root: &Path,
    id: &str,
    input: &AgentDraftInput,
) -> Result<(PathBuf, AgentPackageResult), AppError> {
    validate_input(input)?;
    let staging = root.join(format!(".staging-{id}-{}", Uuid::new_v4()));
    let target = staging.join(&input.relative_path);
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| AppError::Io {
                message: format!("create Agent draft staging directory: {error}"),
            })?;
    }
    if let Err(error) = crate::util::fs::atomic_write(&target, input.text.as_bytes()).await {
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err(error);
    }
    let validation = super::inspect_file(&format!("draft:{id}"), &staging, &target)?
        .unwrap_or_else(|| empty_validation(id, input));
    Ok((staging, validation))
}

pub async fn create(state: &AppState, input: AgentDraftInput) -> Result<AgentDraft, AppError> {
    let _guard = lock_drafts_async(state.app_data_dir.clone()).await?;
    let root = ensure_owned_root(&state.app_data_dir, "drafts")?;
    let mut drafts = load_index(&state.app_data_dir).await?;
    if drafts.len() == MAX_AGENT_DRAFTS {
        return Err(invalid("Agent draft inbox is full"));
    }
    let id = Uuid::new_v4().to_string();
    let (staging, validation) = stage_input(&root, &id, &input).await?;
    let destination = root.join(&id);
    if let Err(error) = tokio::fs::rename(&staging, &destination).await {
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err(AppError::Io {
            message: format!("activate Agent draft: {error}"),
        });
    }
    let draft = AgentDraft {
        id,
        submitted_at: chrono::Utc::now().to_rfc3339(),
        state: AgentDraftState::Pending,
        relative_path: input.relative_path,
        source_hash: validation.source_hash.clone(),
        validation,
        published_source_id: None,
    };
    drafts.push(draft.clone());
    if let Err(error) = save_index(&state.app_data_dir, &drafts).await {
        let _ = tokio::fs::remove_dir_all(&destination).await;
        return Err(error);
    }
    Ok(draft)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "kebab-case")]
struct AgentFromSkillMetadata<'a> {
    name: &'a str,
    description: &'a str,
    version: Option<&'a str>,
    channel: &'a str,
    required_skills: [&'a str; 1],
    groups: &'a [String],
    tags: &'a [String],
    permissions: &'a [String],
}

fn skill_agent_name(skill_text: &str, fallback: &str) -> String {
    let body = skill_text
        .strip_prefix("---\n")
        .and_then(|rest| rest.find("\n---").map(|end| &rest[end + 4..]))
        .unwrap_or(skill_text);
    body.lines()
        .find_map(|line| {
            line.strip_prefix("# ")
                .map(str::trim)
                .filter(|line| !line.is_empty())
        })
        .unwrap_or(fallback)
        .to_string()
}

async fn exact_skill(
    state: &AppState,
    reference: &SkillReference,
) -> Result<SkillPackageResult, AppError> {
    crate::skills::inspect_skill_sources(state)
        .await?
        .into_iter()
        .flat_map(|result| result.packages)
        .find(|package| {
            package.source_id == reference.source_id
                && package.relative_path == reference.relative_path
        })
        .filter(|package| package.installable)
        .ok_or_else(|| invalid("selected Skill is missing or not installable"))
}

pub(crate) async fn input_from_skill(
    state: &AppState,
    reference: &SkillReference,
) -> Result<AgentDraftInput, AppError> {
    let package = exact_skill(state, reference).await?;
    let skill_name = package
        .name
        .as_deref()
        .ok_or_else(|| invalid("selected Skill has no valid name"))?;
    let description = package
        .description
        .as_deref()
        .ok_or_else(|| invalid("selected Skill has no valid description"))?;
    let skill_text = crate::skills::read_skill_file(
        state,
        &reference.source_id,
        &reference.relative_path,
        "SKILL.md",
    )
    .await?
    .text
    .ok_or_else(|| invalid("selected Skill metadata must be UTF-8"))?;
    let name = skill_agent_name(&skill_text, skill_name);
    let metadata = serde_yaml::to_string(&AgentFromSkillMetadata {
        name: &name,
        description,
        version: package.version.as_deref(),
        channel: &package.channel,
        required_skills: [skill_name],
        groups: &package.group,
        tags: &package.tags,
        permissions: &package.permissions,
    })
    .map_err(|error| AppError::Internal {
        message: format!("serialize Agent-from-Skill metadata: {error}"),
    })?;
    let category = package
        .group
        .first()
        .map(String::as_str)
        .unwrap_or("custom");
    let relative_path = format!("{category}/{skill_name}-agent.md");
    let body = format!(
        "# {name} Agent\n\nYou are a specialized agent for: {description}\n\n## Required Skill\n\nUse the `{skill_name}` skill for every task. If it is unavailable, state that limitation before giving consequential guidance.\n\n## Operating Contract\n\n1. Load and follow the required skill before acting.\n2. Preserve its evidence, validation, and safety requirements.\n3. Separate verified guidance from assumptions or supplemental knowledge.\n"
    );
    Ok(AgentDraftInput {
        relative_path,
        text: format!("---\n{}---\n\n{body}", metadata.trim_start_matches("---\n")),
    })
}

pub(crate) async fn create_from_skill(
    state: &AppState,
    reference: SkillReference,
) -> Result<AgentDraft, AppError> {
    create(state, input_from_skill(state, &reference).await?).await
}

pub async fn list(state: &AppState) -> Result<Vec<AgentDraft>, AppError> {
    load_index(&state.app_data_dir).await
}

pub async fn get(state: &AppState, id: &str) -> Result<AgentDraft, AppError> {
    Uuid::parse_str(id).map_err(|_| invalid("Agent draft id is invalid"))?;
    load_index(&state.app_data_dir)
        .await?
        .into_iter()
        .find(|draft| draft.id == id)
        .ok_or_else(|| invalid(format!("Agent draft not found: {id}")))
}

pub async fn edit(
    state: &AppState,
    id: &str,
    input: AgentDraftInput,
) -> Result<AgentDraft, AppError> {
    Uuid::parse_str(id).map_err(|_| invalid("Agent draft id is invalid"))?;
    let _guard = lock_drafts_async(state.app_data_dir.clone()).await?;
    let root = ensure_owned_root(&state.app_data_dir, "drafts")?;
    let mut drafts = load_index(&state.app_data_dir).await?;
    let index = drafts
        .iter()
        .position(|draft| draft.id == id)
        .ok_or_else(|| invalid(format!("Agent draft not found: {id}")))?;
    if drafts[index].state != AgentDraftState::Pending {
        return Err(invalid("only pending Agent drafts can be edited"));
    }
    let (staging, validation) = stage_input(&root, id, &input).await?;
    let destination = root.join(id);
    let backup = root.join(format!(".backup-{id}-{}", Uuid::new_v4()));
    tokio::fs::rename(&destination, &backup)
        .await
        .map_err(|error| AppError::Io {
            message: format!("backup Agent draft before edit: {error}"),
        })?;
    if let Err(error) = tokio::fs::rename(&staging, &destination).await {
        let _ = tokio::fs::rename(&backup, &destination).await;
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err(AppError::Io {
            message: format!("activate Agent draft edit: {error}"),
        });
    }
    drafts[index].relative_path = input.relative_path;
    drafts[index].source_hash = validation.source_hash.clone();
    drafts[index].validation = validation;
    let result = drafts[index].clone();
    if let Err(error) = save_index(&state.app_data_dir, &drafts).await {
        let _ = tokio::fs::remove_dir_all(&destination).await;
        let _ = tokio::fs::rename(&backup, &destination).await;
        return Err(error);
    }
    let _ = tokio::fs::remove_dir_all(&backup).await;
    Ok(result)
}

async fn ensure_published_source(
    app_data_dir: &Path,
    root: &Path,
) -> Result<(AgentSource, bool), AppError> {
    let root = std::fs::canonicalize(root).map_err(|error| AppError::Io {
        message: format!("resolve published Agent source {}: {error}", root.display()),
    })?;
    let root = root.to_string_lossy().into_owned();
    let _guard = super::lock_sources_async(app_data_dir.to_path_buf()).await?;
    let mut sources = super::load_registered_sources(app_data_dir).await?;
    if let Some(source) = sources.iter().find(|source| {
        source.id == PUBLISHED_AGENT_SOURCE_ID
            && matches!(&source.kind, AgentSourceKind::Published { root: value } if value == &root)
    }) {
        return Ok((source.clone(), false));
    }
    if sources
        .iter()
        .any(|source| source.id == PUBLISHED_AGENT_SOURCE_ID)
    {
        return Err(invalid("published Agent source registration conflicts"));
    }
    let source = AgentSource {
        id: PUBLISHED_AGENT_SOURCE_ID.into(),
        label: "Published Agents".into(),
        enabled: true,
        kind: AgentSourceKind::Published { root },
    };
    sources.push(source.clone());
    super::save_registered_sources(app_data_dir, &sources).await?;
    Ok((source, true))
}

async fn remove_published_source(app_data_dir: &Path) {
    let _ = super::remove_agent_source(app_data_dir, PUBLISHED_AGENT_SOURCE_ID).await;
}

fn validate_recovery_file(path: &Path, expected_hash: &str) -> Result<(), AppError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| AppError::Io {
        message: format!("inspect Agent recovery file: {error}"),
    })?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || crate::skills::metadata_is_reparse_point(&metadata)
        || metadata.len() > super::corpus::MAX_AGENT_BYTES
    {
        return Err(AppError::StorageCorrupt {
            message: "Agent publish recovery found an unsafe file".into(),
        });
    }
    let bytes = std::fs::read(path).map_err(|error| AppError::Io {
        message: format!("read Agent recovery file: {error}"),
    })?;
    if hex::encode(Sha256::digest(&bytes)) != expected_hash {
        return Err(AppError::StorageCorrupt {
            message: "Agent publish recovery found changed content".into(),
        });
    }
    Ok(())
}

fn validate_recovery_tree(root: &Path) -> Result<(), AppError> {
    let metadata = std::fs::symlink_metadata(root).map_err(|error| AppError::Io {
        message: format!("inspect Agent recovery directory: {error}"),
    })?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || crate::skills::metadata_is_reparse_point(&metadata)
    {
        return Err(AppError::StorageCorrupt {
            message: "Agent publish recovery found an unsafe directory".into(),
        });
    }
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(&directory).map_err(|error| AppError::Io {
            message: format!("scan Agent recovery directory: {error}"),
        })? {
            let entry = entry.map_err(|error| AppError::Io {
                message: format!("scan Agent recovery entry: {error}"),
            })?;
            let metadata =
                std::fs::symlink_metadata(entry.path()).map_err(|error| AppError::Io {
                    message: format!("inspect Agent recovery entry: {error}"),
                })?;
            if metadata.file_type().is_symlink()
                || crate::skills::metadata_is_reparse_point(&metadata)
                || (!metadata.is_dir() && !metadata.is_file())
            {
                return Err(AppError::StorageCorrupt {
                    message: "Agent publish recovery found an unsafe tree entry".into(),
                });
            }
            if metadata.is_dir() {
                directories.push(entry.path());
            }
        }
    }
    Ok(())
}

pub(crate) async fn recover_publish_operations(state: &AppState) -> Result<(), AppError> {
    let Some(database) = state.completed_state_database().await? else {
        return Ok(());
    };
    for operation in database.pending_filesystem_operations().await? {
        if operation.kind != "agent_publish" {
            continue;
        }
        let payload = serde_json::from_value::<PublishOperation>(operation.payload.clone())
            .map_err(|_| AppError::StorageCorrupt {
                message: "Agent publish recovery payload is invalid".into(),
            })?;
        Uuid::parse_str(&payload.draft_id).map_err(|_| AppError::StorageCorrupt {
            message: "Agent publish recovery draft id is invalid".into(),
        })?;
        let relative_path =
            library::normalize_relative_path(&payload.relative_path).map_err(|_| {
                AppError::StorageCorrupt {
                    message: "Agent publish recovery path is invalid".into(),
                }
            })?;
        let draft_root = ensure_owned_root(&state.app_data_dir, "drafts")?;
        let published_root = ensure_owned_root(&state.app_data_dir, "published")?;
        let source_root = draft_root.join(&payload.draft_id);
        let source = source_root.join(&relative_path);
        let destination = published_root.join(&relative_path);
        let drafts = load_index(&state.app_data_dir).await?;
        let draft = drafts
            .iter()
            .find(|draft| draft.id == payload.draft_id)
            .ok_or_else(|| AppError::StorageCorrupt {
                message: "Agent publish recovery lost its draft metadata".into(),
            })?;
        let result = match operation.phase {
            crate::state_db::FilesystemOperationPhase::Prepared => {
                if draft.state != AgentDraftState::Pending
                    || draft.source_hash != payload.expected_hash
                {
                    Err(AppError::StorageCorrupt {
                        message: "Agent publish recovery found changed draft metadata".into(),
                    })
                } else {
                    validate_recovery_file(&source, &payload.expected_hash)?;
                    if destination.exists() {
                        validate_recovery_file(&destination, &payload.expected_hash)?;
                        std::fs::remove_file(&destination).map_err(|error| AppError::Io {
                            message: format!("remove recovered Agent duplicate: {error}"),
                        })?;
                    }
                    Ok(())
                }
            }
            crate::state_db::FilesystemOperationPhase::FilesystemApplied => {
                if draft.state != AgentDraftState::Published
                    || draft.source_hash != payload.expected_hash
                {
                    Err(AppError::StorageCorrupt {
                        message: "Agent publish recovery found changed committed metadata".into(),
                    })
                } else {
                    validate_recovery_file(&destination, &payload.expected_hash)?;
                    if source_root.exists() {
                        validate_recovery_tree(&source_root)?;
                        validate_recovery_file(&source, &payload.expected_hash)?;
                        std::fs::remove_dir_all(&source_root).map_err(|error| AppError::Io {
                            message: format!("clean recovered Agent draft: {error}"),
                        })?;
                    }
                    Ok(())
                }
            }
            crate::state_db::FilesystemOperationPhase::Committed => Ok(()),
        };
        if let Err(error) = result {
            database
                .retain_filesystem_operation_error(&operation.id, &error.to_string())
                .await?;
            super::organize::reconcile_draft_publish_approval(
                state,
                payload.approval_id.as_deref(),
                &payload.draft_id,
                &payload.expected_hash,
                false,
                Some(error.to_string()),
            )
            .await?;
            return Err(error);
        }
        let completed =
            operation.phase == crate::state_db::FilesystemOperationPhase::FilesystemApplied;
        super::organize::reconcile_draft_publish_approval(
            state,
            payload.approval_id.as_deref(),
            &payload.draft_id,
            &payload.expected_hash,
            completed,
            None,
        )
        .await?;
        if completed {
            database.commit_filesystem_operation(&operation.id).await?;
        } else {
            database.abort_filesystem_operation(&operation.id).await?;
        }
    }
    Ok(())
}

fn ensure_destination_parent(root: &Path, relative_path: &str) -> Result<(), AppError> {
    let mut current = root.to_path_buf();
    let parent = Path::new(relative_path)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    for component in parent.components() {
        current.push(component.as_os_str());
        match std::fs::symlink_metadata(&current) {
            Ok(metadata)
                if metadata.is_dir()
                    && !metadata.file_type().is_symlink()
                    && !crate::skills::metadata_is_reparse_point(&metadata) => {}
            Ok(_) => return Err(invalid("published Agent path contains an unsafe entry")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&current).map_err(|error| AppError::Io {
                    message: format!("create published Agent directory: {error}"),
                })?;
            }
            Err(error) => {
                return Err(AppError::Io {
                    message: format!("inspect published Agent directory: {error}"),
                });
            }
        }
    }
    Ok(())
}

pub async fn publish(state: &AppState, id: &str) -> Result<AgentDraft, AppError> {
    Uuid::parse_str(id).map_err(|_| invalid("Agent draft id is invalid"))?;
    let _guard = lock_drafts_async(state.app_data_dir.clone()).await?;
    let draft_root = ensure_owned_root(&state.app_data_dir, "drafts")?;
    let published_root = ensure_owned_root(&state.app_data_dir, "published")?;
    let mut drafts = load_index(&state.app_data_dir).await?;
    let index = drafts
        .iter()
        .position(|draft| draft.id == id)
        .ok_or_else(|| invalid(format!("Agent draft not found: {id}")))?;
    if drafts[index].state != AgentDraftState::Pending || !drafts[index].validation.installable {
        return Err(invalid("only valid pending Agent drafts can be published"));
    }
    let relative_path = drafts[index].relative_path.clone();
    let source = draft_root.join(id).join(&relative_path);
    let current = super::inspect_file(&format!("draft:{id}"), &draft_root.join(id), &source)?
        .ok_or_else(|| invalid("Agent draft no longer has valid frontmatter"))?;
    if !current.installable || current.source_hash != drafts[index].source_hash {
        return Err(invalid(
            "Agent draft changed after validation; edit it again",
        ));
    }
    let database = state.completed_state_database().await?;
    let operation = if let Some(database) = &database {
        let approval_id = super::organize::list(state)
            .await?
            .approvals
            .into_iter()
            .find(|approval| {
                approval.state == crate::types::AgentApprovalState::Running
                    && approval.request
                        == (crate::types::AgentApprovalAction::DraftPublish {
                            id: id.to_owned(),
                            plan_revision: current.source_hash.clone(),
                        })
            })
            .map(|approval| approval.id);
        Some(
            database
                .prepare_filesystem_operation(
                    "agent_publish",
                    &PublishOperation {
                        draft_id: id.to_owned(),
                        relative_path: relative_path.clone(),
                        expected_hash: current.source_hash.clone(),
                        approval_id,
                    },
                )
                .await?,
        )
    } else {
        None
    };
    ensure_destination_parent(&published_root, &relative_path)?;
    let destination = published_root.join(&relative_path);
    let bytes = std::fs::read(&source).map_err(|error| AppError::Io {
        message: format!("read Agent draft for publication: {error}"),
    })?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                invalid(format!(
                    "published Agent already exists; choose a new path: {relative_path}"
                ))
            } else {
                AppError::Io {
                    message: format!("create published Agent: {error}"),
                }
            }
        })?;
    use std::io::Write;
    if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
        let _ = std::fs::remove_file(&destination);
        return Err(AppError::Io {
            message: format!("write published Agent: {error}"),
        });
    }
    let (published_source, source_created) =
        match ensure_published_source(&state.app_data_dir, &published_root).await {
            Ok(value) => value,
            Err(error) => {
                let _ = std::fs::remove_file(&destination);
                return Err(error);
            }
        };
    match super::inspect_file(PUBLISHED_AGENT_SOURCE_ID, &published_root, &destination) {
        Ok(Some(validation))
            if validation.installable && validation.source_hash == current.source_hash => {}
        Ok(_) => {
            if source_created {
                remove_published_source(&state.app_data_dir).await;
            }
            let _ = std::fs::remove_file(&destination);
            return Err(invalid("published Agent failed source validation"));
        }
        Err(error) => {
            if source_created {
                remove_published_source(&state.app_data_dir).await;
            }
            let _ = std::fs::remove_file(&destination);
            return Err(error);
        }
    }
    drafts[index].state = AgentDraftState::Published;
    drafts[index].published_source_id = Some(published_source.id);
    let result = drafts[index].clone();
    if let Err(error) = save_index_after_filesystem(
        &state.app_data_dir,
        &drafts,
        operation.as_ref().map(|operation| operation.id.as_str()),
    )
    .await
    {
        if source_created {
            remove_published_source(&state.app_data_dir).await;
        }
        let _ = std::fs::remove_file(&destination);
        return Err(error);
    }
    if let Err(error) = tokio::fs::remove_dir_all(draft_root.join(id)).await {
        if let (Some(database), Some(operation)) = (&database, &operation) {
            database
                .retain_filesystem_operation_error(&operation.id, &error.to_string())
                .await?;
            return Err(AppError::Io {
                message: format!("clean published Agent draft: {error}"),
            });
        }
    }
    if let (Some(database), Some(operation)) = (&database, &operation) {
        database.commit_filesystem_operation(&operation.id).await?;
    }
    Ok(result)
}

pub async fn reject(state: &AppState, id: &str) -> Result<AgentDraft, AppError> {
    Uuid::parse_str(id).map_err(|_| invalid("Agent draft id is invalid"))?;
    let _guard = lock_drafts_async(state.app_data_dir.clone()).await?;
    let root = ensure_owned_root(&state.app_data_dir, "drafts")?;
    let mut drafts = load_index(&state.app_data_dir).await?;
    let index = drafts
        .iter()
        .position(|draft| draft.id == id)
        .ok_or_else(|| invalid(format!("Agent draft not found: {id}")))?;
    if drafts[index].state != AgentDraftState::Pending {
        return Err(invalid("only pending Agent drafts can be rejected"));
    }
    let directory = root.join(id);
    let quarantine = root.join(format!(".rejected-{id}"));
    tokio::fs::rename(&directory, &quarantine)
        .await
        .map_err(|error| AppError::Io {
            message: format!("quarantine rejected Agent draft: {error}"),
        })?;
    drafts[index].state = AgentDraftState::Rejected;
    let result = drafts[index].clone();
    if let Err(error) = save_index(&state.app_data_dir, &drafts).await {
        let _ = tokio::fs::rename(&quarantine, &directory).await;
        return Err(error);
    }
    let _ = tokio::fs::remove_dir_all(&quarantine).await;
    Ok(result)
}

pub async fn duplicate(
    state: &AppState,
    reference: &AgentReference,
) -> Result<AgentDraft, AppError> {
    let text = super::read_agent_text(&state.app_data_dir, reference).await?;
    create(
        state,
        AgentDraftInput {
            relative_path: reference.relative_path.clone(),
            text,
        },
    )
    .await
}

#[tauri::command]
pub async fn agent_drafts_list(state: State<'_, AppState>) -> Result<Vec<AgentDraft>, AppError> {
    list(&state).await
}

#[tauri::command]
pub async fn agent_draft_get(
    state: State<'_, AppState>,
    id: String,
) -> Result<AgentDraft, AppError> {
    get(&state, &id).await
}

#[tauri::command]
pub async fn agent_draft_create(
    state: State<'_, AppState>,
    input: AgentDraftInput,
) -> Result<AgentDraft, AppError> {
    create(&state, input).await
}

#[tauri::command]
pub async fn agent_from_skill_preview(
    state: State<'_, AppState>,
    reference: SkillReference,
) -> Result<AgentDraftInput, AppError> {
    input_from_skill(&state, &reference).await
}

#[tauri::command]
pub async fn agent_draft_edit(
    state: State<'_, AppState>,
    id: String,
    input: AgentDraftInput,
) -> Result<AgentDraft, AppError> {
    edit(&state, &id, input).await
}

#[tauri::command]
pub async fn agent_draft_publish(
    state: State<'_, AppState>,
    id: String,
) -> Result<AgentDraft, AppError> {
    publish(&state, &id).await
}

#[tauri::command]
pub async fn agent_draft_reject(
    state: State<'_, AppState>,
    id: String,
) -> Result<AgentDraft, AppError> {
    reject(&state, &id).await
}

#[tauri::command]
pub async fn agent_draft_duplicate(
    app: AppHandle,
    state: State<'_, AppState>,
    reference: AgentReference,
) -> Result<AgentDraft, AppError> {
    if reference.source_id == super::BUILTIN_AGENT_SOURCE_ID {
        super::corpus::ensure_corpus(&app, &state).await?;
    }
    duplicate(&state, &reference).await
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::state::AppState;

    fn state(root: &Path) -> AppState {
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.to_path_buf();
        state
    }

    fn valid_input(path: &str) -> AgentDraftInput {
        AgentDraftInput {
            relative_path: path.into(),
            text: "---\nname: Reviewer\ndescription: Reviews code.\n---\nReview carefully.\n"
                .into(),
        }
    }

    async fn enable_sqlite(root: &Path) {
        let database = crate::state_db::StateDatabase::open(root).unwrap();
        database
            .mutate(document_spec(), Vec::new(), |_| Ok(()))
            .await
            .unwrap();
        database
            .mutate(super::super::agent_sources_spec(), Vec::new(), |_| Ok(()))
            .await
            .unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        super::super::organize::replace_library(
            &state(root),
            crate::types::AgentLibraryState::default(),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn sqlite_publish_commits_a_filesystem_operation() {
        let root = tempfile::tempdir().unwrap();
        enable_sqlite(root.path()).await;
        let app = state(root.path());
        let draft = create(&app, valid_input("engineering/reviewer.md"))
            .await
            .unwrap();

        publish(&app, &draft.id).await.unwrap();

        let connection =
            rusqlite::Connection::open(root.path().join("state/agency-agents.sqlite3")).unwrap();
        let count: u32 = connection
            .query_row(
                "SELECT count(*) FROM filesystem_operations \
                 WHERE kind = 'agent_publish' AND phase = 'committed'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn prepared_publish_recovery_removes_only_the_verified_duplicate() {
        let root = tempfile::tempdir().unwrap();
        enable_sqlite(root.path()).await;
        let app = state(root.path());
        let draft = create(&app, valid_input("engineering/reviewer.md"))
            .await
            .unwrap();
        let database = crate::state_db::StateDatabase::completed(root.path())
            .await
            .unwrap()
            .unwrap();
        database
            .prepare_filesystem_operation(
                "agent_publish",
                &PublishOperation {
                    draft_id: draft.id.clone(),
                    relative_path: draft.relative_path.clone(),
                    expected_hash: draft.source_hash.clone(),
                    approval_id: None,
                },
            )
            .await
            .unwrap();
        let destination = root
            .path()
            .join("agents/published")
            .join(&draft.relative_path);
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::copy(
            root.path()
                .join("agents/drafts")
                .join(&draft.id)
                .join(&draft.relative_path),
            &destination,
        )
        .unwrap();

        recover_publish_operations(&app).await.unwrap();
        recover_publish_operations(&app).await.unwrap();

        assert!(!destination.exists());
        assert!(root
            .path()
            .join("agents/drafts")
            .join(&draft.id)
            .join(&draft.relative_path)
            .is_file());
        assert!(database
            .pending_filesystem_operations()
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn create_from_skill_builds_valid_draft_with_structured_dependency() {
        let root = tempfile::tempdir().unwrap();
        let skill_source = tempfile::tempdir().unwrap();
        let package = skill_source
            .path()
            .join("project-controls/primavera-p6-eppm");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(
            package.join("SKILL.md"),
            "---\nname: primavera-p6-eppm\ndescription: Evidence-backed Primavera P6 guidance.\ntype: other\ngroup: [project-controls, primavera]\ntags: [primavera-p6, scheduling]\nversion: 1.0.0\nchannel: stable\n---\n# Primavera P6 EPPM v25\n\nUse Oracle documentation.\n",
        )
        .unwrap();
        let app = state(root.path());
        let source = crate::skills::add_local_source(&app, skill_source.path())
            .await
            .unwrap();

        let draft = create_from_skill(
            &app,
            crate::types::SkillReference {
                source_id: source.id,
                relative_path: "project-controls/primavera-p6-eppm".into(),
            },
        )
        .await
        .unwrap();

        assert_eq!(
            draft.relative_path,
            "project-controls/primavera-p6-eppm-agent.md"
        );
        assert!(draft.validation.installable);
        assert_eq!(draft.validation.required_skills, ["primavera-p6-eppm"]);
        assert_eq!(draft.validation.groups, ["project-controls", "primavera"]);
        assert_eq!(draft.validation.tags, ["primavera-p6", "scheduling"]);
        assert_eq!(
            draft.validation.agent.as_ref().unwrap().name,
            "Primavera P6 EPPM v25"
        );

        let published = publish(&app, &draft.id).await.unwrap();
        let package = super::super::resolve_agent_package(
            root.path(),
            &AgentReference {
                source_id: published.published_source_id.unwrap(),
                relative_path: published.relative_path,
            },
        )
        .await
        .unwrap();
        assert_eq!(package.required_skills, ["primavera-p6-eppm"]);
    }

    #[tokio::test]
    async fn create_from_skill_rejects_a_nonexistent_exact_reference() {
        let root = tempfile::tempdir().unwrap();
        let app = state(root.path());

        let error = create_from_skill(
            &app,
            SkillReference {
                source_id: "missing:skill-source".into(),
                relative_path: "project-controls/primavera-p6-eppm".into(),
            },
        )
        .await
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("selected Skill is missing or not installable"));
    }

    #[tokio::test]
    async fn invalid_drafts_survive_but_only_valid_pending_drafts_publish() {
        let root = tempfile::tempdir().unwrap();
        let app = state(root.path());
        let invalid = create(
            &app,
            AgentDraftInput {
                relative_path: "reviewer.md".into(),
                text: "unfinished".into(),
            },
        )
        .await
        .unwrap();
        assert!(!invalid.validation.installable);
        assert_eq!(list(&app).await.unwrap().len(), 1);
        assert!(publish(&app, &invalid.id).await.is_err());

        let valid = create(&app, valid_input("engineering/reviewer.md"))
            .await
            .unwrap();
        let published = publish(&app, &valid.id).await.unwrap();
        assert_eq!(published.state, AgentDraftState::Published);
        assert!(root
            .path()
            .join("agents/published/engineering/reviewer.md")
            .is_file());

        let conflicting = create(&app, valid_input("engineering/reviewer.md"))
            .await
            .unwrap();
        assert!(publish(&app, &conflicting.id).await.is_err());
        assert_eq!(
            get(&app, &conflicting.id).await.unwrap().state,
            AgentDraftState::Pending
        );
    }

    #[tokio::test]
    async fn edit_duplicate_and_reject_never_mutate_source_files() {
        let root = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        std::fs::write(
            source.path().join("reviewer.md"),
            valid_input("reviewer.md").text,
        )
        .unwrap();
        let registered = super::super::add_local_source(root.path(), source.path())
            .await
            .unwrap();
        let app = state(root.path());
        let draft = duplicate(
            &app,
            &crate::types::AgentReference {
                source_id: registered.id,
                relative_path: "reviewer.md".into(),
            },
        )
        .await
        .unwrap();
        let edited = edit(
            &app,
            &draft.id,
            AgentDraftInput {
                relative_path: "reviewer-copy.md".into(),
                text: valid_input("reviewer.md").text.replace("Reviewer", "Copy"),
            },
        )
        .await
        .unwrap();
        assert_eq!(edited.relative_path, "reviewer-copy.md");
        assert_eq!(
            std::fs::read_to_string(source.path().join("reviewer.md")).unwrap(),
            valid_input("reviewer.md").text
        );
        assert_eq!(
            reject(&app, &edited.id).await.unwrap().state,
            AgentDraftState::Rejected
        );
    }

    #[tokio::test]
    async fn draft_bounds_and_publication_rollback_preserve_the_pending_draft() {
        let root = tempfile::tempdir().unwrap();
        let app = state(root.path());
        assert!(create(
            &app,
            AgentDraftInput {
                relative_path: "too-large.md".into(),
                text: "x".repeat(super::super::corpus::MAX_AGENT_BYTES as usize + 1),
            },
        )
        .await
        .is_err());
        assert!(create(
            &app,
            AgentDraftInput {
                relative_path: "../escape.md".into(),
                text: "x".into(),
            },
        )
        .await
        .is_err());

        let draft = create(&app, valid_input("reviewer.md")).await.unwrap();
        let source_state = super::super::sources_path(root.path());
        std::fs::create_dir(source_state).unwrap();
        assert!(publish(&app, &draft.id).await.is_err());
        assert_eq!(
            get(&app, &draft.id).await.unwrap().state,
            AgentDraftState::Pending
        );
        assert!(!root.path().join("agents/published/reviewer.md").exists());
        assert!(root.path().join("agents/drafts").join(draft.id).is_dir());
    }

    #[tokio::test]
    async fn corrupt_draft_index_fails_without_rewriting_it() {
        let root = tempfile::tempdir().unwrap();
        let path = index_path(root.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{not-json").unwrap();

        assert!(load_index(root.path()).await.is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"{not-json");
    }
}
