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
    AgentSourceKind, AgentValidationCode, AgentValidationError,
};

const MAX_AGENT_DRAFTS: usize = 256;
const PUBLISHED_AGENT_SOURCE_ID: &str = "published:agents";

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
    if drafts.len() > MAX_AGENT_DRAFTS
        || drafts.iter().any(|draft| {
            Uuid::parse_str(&draft.id).is_err()
                || library::normalize_relative_path(&draft.relative_path).is_err()
        })
    {
        return Err(invalid("Agent draft index is invalid or exceeds its limit"));
    }
    Ok(drafts)
}

async fn save_index(app_data_dir: &Path, drafts: &[AgentDraft]) -> Result<(), AppError> {
    if drafts.len() > MAX_AGENT_DRAFTS {
        return Err(invalid(format!(
            "at most {MAX_AGENT_DRAFTS} Agent drafts are allowed"
        )));
    }
    let bytes = serde_json::to_vec_pretty(drafts).map_err(|error| AppError::Internal {
        message: format!("serialize Agent draft index: {error}"),
    })?;
    crate::util::fs::atomic_write(&index_path(app_data_dir), &bytes).await
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
    drafts[index].state = AgentDraftState::Published;
    drafts[index].published_source_id = Some(published_source.id);
    let result = drafts[index].clone();
    if let Err(error) = save_index(&state.app_data_dir, &drafts).await {
        if source_created {
            remove_published_source(&state.app_data_dir).await;
        }
        let _ = std::fs::remove_file(&destination);
        return Err(error);
    }
    let _ = tokio::fs::remove_dir_all(draft_root.join(id)).await;
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
