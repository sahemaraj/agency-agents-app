use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::path::{Component, Path, PathBuf};

use base64::Engine as _;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::corpus::state_dir;
use crate::error::AppError;
use crate::state::AppState;
use crate::types::{SkillDraft, SkillDraftFile, SkillDraftState, SkillSource};
use crate::util::fs::atomic_write;

const MAX_DRAFTS: usize = 500;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftInputFile {
    pub relative_path: String,
    pub text: Option<String>,
    pub base64: Option<String>,
}

#[derive(serde::Serialize)]
struct CreatorMetadata {
    name: String,
    description: String,
    #[serde(rename = "type")]
    skill_type: crate::types::SkillType,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    group: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
}

#[cfg(test)]
fn drafts_root(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("skills").join("drafts")
}

#[cfg(test)]
fn published_root(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("skills").join("published")
}

fn ensure_owned_root(app_data_dir: &Path, leaf: &str) -> Result<PathBuf, AppError> {
    let app = std::fs::canonicalize(app_data_dir).map_err(io("resolve app data directory"))?;
    let mut current = app.clone();
    for component in ["skills", leaf] {
        let candidate = current.join(component);
        match std::fs::create_dir(&candidate) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(io("create app-owned skill directory")(error)),
        }
        let metadata = std::fs::symlink_metadata(&candidate)
            .map_err(io("inspect app-owned skill directory"))?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || super::metadata_is_reparse_point(&metadata)
            || std::fs::canonicalize(&candidate).map_err(io("resolve app-owned skill directory"))?
                != candidate
        {
            return Err(AppError::InvalidArgument {
                message: format!(
                    "app-owned skill directory must not contain links or reparse points: {}",
                    candidate.display()
                ),
            });
        }
        current = candidate;
    }
    Ok(current)
}

fn index_path(app_data_dir: &Path) -> PathBuf {
    state_dir(app_data_dir).join("skill-drafts.json")
}

fn lock_drafts(app_data_dir: &Path) -> Result<File, AppError> {
    let directory = state_dir(app_data_dir);
    std::fs::create_dir_all(&directory).map_err(io("create draft state directory"))?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(directory.join("skill-drafts.lock"))
        .map_err(io("open draft lock"))?;
    file.lock().map_err(io("lock draft state"))?;
    Ok(file)
}

fn io(context: &'static str) -> impl Fn(std::io::Error) -> AppError {
    move |error| AppError::Io {
        message: format!("{context}: {error}"),
    }
}

async fn load_index(app_data_dir: &Path) -> Result<Vec<SkillDraft>, AppError> {
    match tokio::fs::read(index_path(app_data_dir)).await {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| AppError::JsonParse {
            command: "skill_drafts_list".into(),
            message: error.to_string(),
            raw_excerpt: String::from_utf8_lossy(&bytes[..bytes.len().min(256)]).into_owned(),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(io("read draft index")(error)),
    }
}

async fn save_index(app_data_dir: &Path, drafts: &[SkillDraft]) -> Result<(), AppError> {
    #[cfg(test)]
    {
        let mut schedules = index_save_schedule()
            .lock()
            .expect("draft index failure probe");
        let should_fail = schedules
            .get_mut(app_data_dir)
            .and_then(std::collections::VecDeque::pop_front)
            .unwrap_or(false);
        if schedules
            .get(app_data_dir)
            .is_some_and(|queue| queue.is_empty())
        {
            schedules.remove(app_data_dir);
        }
        if should_fail {
            return Err(AppError::Io {
                message: "injected draft index save failure".into(),
            });
        }
    }
    let bytes = serde_json::to_vec_pretty(drafts).map_err(|error| AppError::Internal {
        message: format!("serialize skill draft index: {error}"),
    })?;
    atomic_write(&index_path(app_data_dir), &bytes).await
}

#[cfg(test)]
fn index_save_schedule(
) -> &'static std::sync::Mutex<std::collections::HashMap<PathBuf, std::collections::VecDeque<bool>>>
{
    static FAIL: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<PathBuf, std::collections::VecDeque<bool>>>,
    > = std::sync::OnceLock::new();
    FAIL.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

#[cfg(test)]
fn schedule_index_saves(app_data_dir: &Path, schedule: impl IntoIterator<Item = bool>) {
    index_save_schedule()
        .lock()
        .expect("draft index failure probe")
        .insert(app_data_dir.to_path_buf(), schedule.into_iter().collect());
}

#[cfg(test)]
fn cleanup_failures() -> &'static std::sync::Mutex<HashSet<PathBuf>> {
    static FAIL: std::sync::OnceLock<std::sync::Mutex<HashSet<PathBuf>>> =
        std::sync::OnceLock::new();
    FAIL.get_or_init(|| std::sync::Mutex::new(HashSet::new()))
}

fn normalized_path(value: &str) -> Result<PathBuf, AppError> {
    if value.is_empty() || value.contains('\\') || value.contains('\0') {
        return Err(AppError::InvalidArgument {
            message: format!("invalid draft file path: {value}"),
        });
    }
    let path = Path::new(value);
    let normalized = path
        .components()
        .filter_map(|part| match part {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        || normalized != value
    {
        return Err(AppError::InvalidArgument {
            message: format!("draft file path must be normalized and relative: {value}"),
        });
    }
    Ok(path.to_path_buf())
}

fn decode_file(file: &DraftInputFile) -> Result<Vec<u8>, AppError> {
    match (&file.text, &file.base64) {
        (Some(text), None) => {
            if text.len() as u64 > super::MAX_SKILL_FILE_BYTES {
                return Err(AppError::InvalidArgument {
                    message: format!("draft file exceeds size limit: {}", file.relative_path),
                });
            }
            Ok(text.as_bytes().to_vec())
        }
        (None, Some(encoded)) => {
            let max_encoded = (super::MAX_SKILL_FILE_BYTES as usize).div_ceil(3) * 4;
            if encoded.len() > max_encoded {
                return Err(AppError::InvalidArgument {
                    message: format!("draft file exceeds size limit: {}", file.relative_path),
                });
            }
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|_| AppError::InvalidArgument {
                    message: format!("invalid base64 for draft file {}", file.relative_path),
                })
        }
        _ => Err(AppError::InvalidArgument {
            message: format!(
                "draft file {} must provide exactly one of text or base64",
                file.relative_path
            ),
        }),
    }
}

fn portable_path_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

fn validate_unique_paths(paths: &[PathBuf]) -> Result<(), AppError> {
    let mut keys = HashSet::with_capacity(paths.len());
    for path in paths {
        let key = portable_path_key(path);
        if !keys.insert(key.clone()) {
            return Err(AppError::InvalidArgument {
                message: format!("duplicate draft path: {key}"),
            });
        }
    }
    for key in &keys {
        for (index, byte) in key.bytes().enumerate() {
            if byte == b'/' && keys.contains(&key[..index]) {
                return Err(AppError::InvalidArgument {
                    message: format!(
                        "draft file conflicts with directory: {} and {key}",
                        &key[..index]
                    ),
                });
            }
        }
    }
    Ok(())
}

fn tree_hash(files: &[SkillDraftFile]) -> String {
    let mut digest = Sha256::new();
    for file in files {
        digest.update(file.relative_path.as_bytes());
        digest.update([0]);
        digest.update(file.size_bytes.to_le_bytes());
        digest.update(file.sha256.as_bytes());
        digest.update([0]);
    }
    hex::encode(digest.finalize())
}

fn validate_claim(root: &Path, claim: &Path) -> Result<(), AppError> {
    let metadata = std::fs::symlink_metadata(claim).map_err(io("inspect claimed draft"))?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || super::metadata_is_reparse_point(&metadata)
    {
        return Err(AppError::InvalidArgument {
            message: "claimed draft must be a real directory".into(),
        });
    }
    let canonical = std::fs::canonicalize(claim).map_err(io("resolve claimed draft"))?;
    if canonical != claim || canonical.parent() != Some(root) {
        return Err(AppError::InvalidArgument {
            message: "claimed draft escaped the app-owned draft root".into(),
        });
    }
    Ok(())
}

struct EvictionGuard {
    original: PathBuf,
    quarantine: Option<PathBuf>,
    committed: bool,
}

impl EvictionGuard {
    fn quarantine(root: &Path, id: &str) -> Result<Self, AppError> {
        let original = root.join(id);
        let quarantine = match std::fs::symlink_metadata(&original) {
            Ok(_) => {
                let quarantine = root.join(format!(".evicted-{id}-{}", Uuid::new_v4()));
                std::fs::rename(&original, &quarantine)
                    .map_err(io("quarantine evicted draft directory"))?;
                Some(quarantine)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(io("inspect evicted draft directory")(error)),
        };
        Ok(Self {
            original,
            quarantine,
            committed: false,
        })
    }

    fn cleanup(&mut self) -> Result<(), AppError> {
        #[cfg(test)]
        if cleanup_failures()
            .lock()
            .expect("eviction cleanup failure probe")
            .remove(
                self.original
                    .parent()
                    .expect("evicted draft original always has a parent"),
            )
        {
            return Err(AppError::Io {
                message: "injected eviction cleanup failure".into(),
            });
        }
        if let Some(quarantine) = &self.quarantine {
            let metadata =
                std::fs::symlink_metadata(quarantine).map_err(io("inspect evicted quarantine"))?;
            if metadata.file_type().is_symlink() {
                std::fs::remove_file(quarantine).map_err(io("remove evicted draft link"))?;
            } else if super::metadata_is_reparse_point(&metadata) {
                std::fs::remove_dir(quarantine)
                    .map_err(io("remove evicted draft reparse point"))?;
            } else {
                std::fs::remove_dir_all(quarantine)
                    .map_err(io("remove evicted draft directory"))?;
            }
        }
        self.committed = true;
        Ok(())
    }

    fn retain_quarantine(&mut self) {
        self.committed = true;
    }
}

impl Drop for EvictionGuard {
    fn drop(&mut self) {
        if !self.committed {
            if let Some(quarantine) = &self.quarantine {
                let _ = std::fs::rename(quarantine, &self.original);
            }
        }
    }
}

pub async fn submit(state: &AppState, files: Vec<DraftInputFile>) -> Result<SkillDraft, AppError> {
    if files.is_empty() || files.len() > super::MAX_SKILL_FILES {
        return Err(AppError::InvalidArgument {
            message: format!("draft must contain 1-{} files", super::MAX_SKILL_FILES),
        });
    }
    let mut normalized = Vec::with_capacity(files.len());
    for file in files {
        normalized.push((normalized_path(&file.relative_path)?, file));
    }
    validate_unique_paths(
        &normalized
            .iter()
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>(),
    )?;
    let mut decoded = Vec::with_capacity(normalized.len());
    let mut total = 0_u64;
    for (relative, file) in normalized {
        let bytes = decode_file(&file)?;
        let size = bytes.len() as u64;
        if size > super::MAX_SKILL_FILE_BYTES {
            return Err(AppError::InvalidArgument {
                message: format!("draft file exceeds size limit: {}", relative.display()),
            });
        }
        total = total
            .checked_add(size)
            .ok_or_else(|| AppError::InvalidArgument {
                message: "draft total size overflow".into(),
            })?;
        if total > super::MAX_SKILL_TOTAL_BYTES {
            return Err(AppError::InvalidArgument {
                message: "draft exceeds total size limit".into(),
            });
        }
        decoded.push((relative, bytes));
    }

    let _lock = lock_drafts(&state.app_data_dir)?;
    let root = ensure_owned_root(&state.app_data_dir, "drafts")?;
    let mut drafts = load_index(&state.app_data_dir).await?;
    let mut eviction = None;
    let mut evicted_record = None;
    if drafts.len() >= MAX_DRAFTS {
        if let Some(index) = drafts
            .iter()
            .position(|draft| draft.state != SkillDraftState::Pending)
        {
            let evicted = drafts.remove(index);
            eviction = Some(EvictionGuard::quarantine(&root, &evicted.id)?);
            evicted_record = Some((index, evicted));
        } else {
            return Err(AppError::InvalidArgument {
                message: "draft inbox is full".into(),
            });
        }
    }
    let id = Uuid::new_v4().to_string();
    let staging = root.join(format!(".staging-{id}"));
    tokio::fs::create_dir_all(&staging)
        .await
        .map_err(io("create draft staging directory"))?;
    for (relative, bytes) in &decoded {
        let target = staging.join(relative);
        if let Some(parent) = target.parent() {
            if let Err(error) = tokio::fs::create_dir_all(parent).await {
                let _ = tokio::fs::remove_dir_all(&staging).await;
                return Err(io("create draft file directory")(error));
            }
        }
        if let Err(error) = atomic_write(&target, bytes).await {
            let _ = tokio::fs::remove_dir_all(&staging).await;
            return Err(error);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(error) =
                tokio::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).await
            {
                let _ = tokio::fs::remove_dir_all(&staging).await;
                return Err(io("set draft file permissions")(error));
            }
        }
    }

    let metadata = super::validate_package("draft", &staging, &staging);
    let name = metadata.name.clone().unwrap_or_else(|| id.clone());
    let package_staging = root.join(format!(".package-{id}")).join(&name);
    if let Some(parent) = package_staging.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(io("create draft package staging directory"))?;
    }
    if let Err(error) = tokio::fs::rename(&staging, &package_staging).await {
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err(io("prepare draft package")(error));
    }
    let final_root = root.join(&id);
    if let Err(error) = tokio::fs::rename(
        package_staging.parent().expect("package staging parent"),
        &final_root,
    )
    .await
    {
        let _ = tokio::fs::remove_dir_all(root.join(format!(".package-{id}"))).await;
        return Err(io("publish draft staging atomically")(error));
    }

    let validation = super::validate_package("draft", &final_root, &final_root.join(&name));
    let draft_files = validation
        .files
        .iter()
        .map(|file| SkillDraftFile {
            relative_path: file.relative_path.clone(),
            size_bytes: file.size_bytes,
            sha256: file.sha256.clone(),
        })
        .collect::<Vec<_>>();
    let draft = SkillDraft {
        id,
        submitted_at: chrono::Utc::now().to_rfc3339(),
        state: SkillDraftState::Pending,
        tree_hash: tree_hash(&draft_files),
        files: draft_files,
        validation,
        published_source_id: None,
    };
    drafts.push(draft.clone());
    if let Err(error) = save_index(&state.app_data_dir, &drafts).await {
        let _ = tokio::fs::remove_dir_all(&final_root).await;
        return Err(error);
    }
    if let Some(guard) = &mut eviction {
        if let Err(error) = guard.cleanup() {
            drafts.pop();
            if let Some((index, evicted)) = evicted_record {
                drafts.insert(index, evicted);
            }
            let rollback = save_index(&state.app_data_dir, &drafts).await;
            if let Err(rollback) = rollback {
                guard.retain_quarantine();
                return Err(AppError::Io {
                    message: format!(
                        "clean evicted draft failed: {error}; index rollback failed: {rollback}; retained committed replacement and eviction quarantine"
                    ),
                });
            }
            if let Err(remove) = tokio::fs::remove_dir_all(&final_root).await {
                return Err(AppError::Io {
                    message: format!(
                        "clean evicted draft failed: {error}; replacement index rolled back but replacement directory cleanup failed: {remove}"
                    ),
                });
            }
            return Err(error);
        }
    }
    Ok(draft)
}

pub async fn create(
    state: &AppState,
    name: String,
    description: String,
    skill_type: crate::types::SkillType,
    group: Vec<String>,
    tags: Vec<String>,
    body: String,
) -> Result<SkillDraft, AppError> {
    let metadata = serde_yaml::to_string(&CreatorMetadata {
        name,
        description,
        skill_type,
        group,
        tags,
    })
    .map_err(|error| AppError::Internal {
        message: format!("serialize skill creator metadata: {error}"),
    })?;
    let skill_md = format!("---\n{}---\n\n{}\n", metadata.trim_start_matches("---\n"), body.trim());
    submit(
        state,
        vec![DraftInputFile {
            relative_path: "SKILL.md".into(),
            text: Some(skill_md),
            base64: None,
        }],
    )
    .await
}

pub async fn edit(
    state: &AppState,
    source_id: String,
    relative_path: String,
    skill_md: String,
) -> Result<SkillDraft, AppError> {
    let package = super::resolve_skill_package(state, &source_id, &relative_path).await?;
    let mut files = Vec::with_capacity(package.files().len());
    for file in package.files() {
        let content = super::read_skill_file(
            state,
            &source_id,
            &relative_path,
            &file.relative_path,
        )
        .await?;
        files.push(DraftInputFile {
            relative_path: file.relative_path.clone(),
            text: if file.relative_path == "SKILL.md" {
                Some(skill_md.clone())
            } else {
                content.text
            },
            base64: content.base64,
        });
    }
    submit(state, files).await
}

pub async fn list(state: &AppState) -> Result<Vec<SkillDraft>, AppError> {
    let _lock = lock_drafts(&state.app_data_dir)?;
    load_index(&state.app_data_dir).await
}

pub async fn get(state: &AppState, id: &str) -> Result<SkillDraft, AppError> {
    validate_id(id)?;
    list(state)
        .await?
        .into_iter()
        .find(|draft| draft.id == id)
        .ok_or_else(|| AppError::InvalidArgument {
            message: format!("skill draft not found: {id}"),
        })
}

fn validate_id(id: &str) -> Result<(), AppError> {
    if Uuid::parse_str(id).is_err() {
        return Err(AppError::InvalidArgument {
            message: "invalid skill draft id".into(),
        });
    }
    Ok(())
}

async fn ensure_published_source(
    state: &AppState,
    root: &Path,
) -> Result<(SkillSource, bool), AppError> {
    super::ensure_local_source(state, root).await
}

pub async fn publish(state: &AppState, id: &str) -> Result<SkillDraft, AppError> {
    validate_id(id)?;
    let _lock = lock_drafts(&state.app_data_dir)?;
    let root = ensure_owned_root(&state.app_data_dir, "drafts")?;
    let mut drafts = load_index(&state.app_data_dir).await?;
    let index = drafts
        .iter()
        .position(|draft| draft.id == id)
        .ok_or_else(|| AppError::InvalidArgument {
            message: format!("skill draft not found: {id}"),
        })?;
    let draft = &drafts[index];
    if draft.state != SkillDraftState::Pending || !draft.validation.installable {
        return Err(AppError::InvalidArgument {
            message: "only valid pending drafts can be published".into(),
        });
    }
    let persisted_name = draft
        .validation
        .name
        .as_deref()
        .ok_or_else(|| AppError::InvalidArgument {
            message: "draft has no valid skill name".into(),
        })?
        .to_owned();
    let expected_hash = draft.tree_hash.clone();
    let draft_root = root.join(id);
    let claim = root.join(format!(".publishing-{id}-{}", Uuid::new_v4()));
    tokio::fs::rename(&draft_root, &claim)
        .await
        .map_err(io("claim skill draft for publication"))?;
    if let Err(error) = validate_claim(&root, &claim) {
        let _ = tokio::fs::rename(&claim, &draft_root).await;
        return Err(error);
    }

    let current_validation = super::validate_package("draft", &claim, &claim.join(&persisted_name));
    let current_files = current_validation
        .files
        .iter()
        .map(|file| SkillDraftFile {
            relative_path: file.relative_path.clone(),
            size_bytes: file.size_bytes,
            sha256: file.sha256.clone(),
        })
        .collect::<Vec<_>>();
    if !current_validation.installable || tree_hash(&current_files) != expected_hash {
        let _ = tokio::fs::rename(&claim, &draft_root).await;
        return Err(AppError::InvalidArgument {
            message: "draft changed after validation; submit it again".into(),
        });
    }
    let name = current_validation
        .name
        .as_deref()
        .expect("validated skill name");
    let source = claim.join(name);
    let published = match ensure_owned_root(&state.app_data_dir, "published") {
        Ok(root) => root,
        Err(error) => {
            let _ = tokio::fs::rename(&claim, &draft_root).await;
            return Err(error);
        }
    };
    let destination = published.join(name);
    if destination.exists() {
        let _ = tokio::fs::rename(&claim, &draft_root).await;
        return Err(AppError::InvalidArgument {
            message: format!("published skill already exists: {name}"),
        });
    }
    if let Err(error) = tokio::fs::rename(&source, &destination).await {
        let _ = tokio::fs::rename(&claim, &draft_root).await;
        return Err(io("publish skill draft atomically")(error));
    }
    let (published_source, source_created) = match ensure_published_source(state, &published).await
    {
        Ok(source) => source,
        Err(error) => {
            let _ = tokio::fs::rename(&destination, &source).await;
            let _ = tokio::fs::rename(&claim, &draft_root).await;
            return Err(error);
        }
    };
    let draft = drafts
        .get_mut(index)
        .expect("draft index remains stable while locked");
    draft.state = SkillDraftState::Published;
    draft.published_source_id = Some(published_source.id.clone());
    let result = draft.clone();
    if let Err(error) = save_index(&state.app_data_dir, &drafts).await {
        let mut rollback_errors = Vec::new();
        if source_created {
            match super::remove_skill_source(state, &published_source.id).await {
                Ok(true) => {}
                Ok(false) => rollback_errors.push("new source registration was not found".into()),
                Err(error) => rollback_errors.push(format!("remove source registration: {error}")),
            }
        }
        if let Err(error) = tokio::fs::rename(&destination, &source).await {
            rollback_errors.push(format!("restore claimed package: {error}"));
        }
        if let Err(error) = tokio::fs::rename(&claim, &draft_root).await {
            rollback_errors.push(format!("restore draft directory: {error}"));
        }
        if !rollback_errors.is_empty() {
            return Err(AppError::Io {
                message: format!(
                    "save draft index failed: {error}; rollback failed: {}",
                    rollback_errors.join("; ")
                ),
            });
        }
        return Err(error);
    }
    let _ = tokio::fs::remove_dir(&claim).await;
    Ok(result)
}

pub async fn reject(state: &AppState, id: &str) -> Result<SkillDraft, AppError> {
    validate_id(id)?;
    let _lock = lock_drafts(&state.app_data_dir)?;
    let root = ensure_owned_root(&state.app_data_dir, "drafts")?;
    let mut drafts = load_index(&state.app_data_dir).await?;
    let draft = drafts
        .iter_mut()
        .find(|draft| draft.id == id)
        .ok_or_else(|| AppError::InvalidArgument {
            message: format!("skill draft not found: {id}"),
        })?;
    if draft.state != SkillDraftState::Pending {
        return Err(AppError::InvalidArgument {
            message: "only pending drafts can be rejected".into(),
        });
    }
    let directory = root.join(id);
    let quarantine = root.join(format!(".rejected-{id}"));
    tokio::fs::rename(&directory, &quarantine)
        .await
        .map_err(io("quarantine rejected draft"))?;
    draft.state = SkillDraftState::Rejected;
    let result = draft.clone();
    if let Err(error) = save_index(&state.app_data_dir, &drafts).await {
        let _ = tokio::fs::rename(&quarantine, &directory).await;
        return Err(error);
    }
    if let Err(error) = tokio::fs::remove_dir_all(&quarantine).await {
        if let Some(draft) = drafts.iter_mut().find(|draft| draft.id == id) {
            draft.state = SkillDraftState::Pending;
        }
        let _ = tokio::fs::rename(&quarantine, &directory).await;
        let _ = save_index(&state.app_data_dir, &drafts).await;
        return Err(io("remove rejected draft")(error));
    }
    Ok(result)
}

#[tauri::command]
pub async fn skill_drafts_list(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<SkillDraft>, AppError> {
    list(&state).await
}

#[tauri::command]
pub async fn skill_draft_publish(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<SkillDraft, AppError> {
    publish(&state, &id).await
}

#[tauri::command]
pub async fn skill_draft_reject(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<SkillDraft, AppError> {
    reject(&state, &id).await
}

#[tauri::command]
pub async fn skill_draft_create(
    state: tauri::State<'_, AppState>,
    name: String,
    description: String,
    skill_type: crate::types::SkillType,
    group: Vec<String>,
    tags: Vec<String>,
    body: String,
) -> Result<SkillDraft, AppError> {
    create(
        &state,
        name,
        description,
        skill_type,
        group,
        tags,
        body,
    )
    .await
}

#[tauri::command]
pub async fn skill_draft_edit(
    state: tauri::State<'_, AppState>,
    source_id: String,
    relative_path: String,
    skill_md: String,
) -> Result<SkillDraft, AppError> {
    edit(&state, source_id, relative_path, skill_md).await
}

#[tauri::command]
pub async fn skill_text_read(
    state: tauri::State<'_, AppState>,
    source_id: String,
    relative_path: String,
) -> Result<String, AppError> {
    super::read_skill_file(&state, &source_id, &relative_path, "SKILL.md")
        .await?
        .text
        .ok_or_else(|| AppError::InvalidArgument {
            message: "SKILL.md must be UTF-8".into(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::settings::SettingsLoadState;
    use std::sync::Arc;
    use tokio::sync::{Mutex, RwLock};

    fn state(root: &Path) -> AppState {
        AppState {
            app_data_dir: root.to_path_buf(),
            corpus_cache: Arc::new(Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(Mutex::new(())),
            skill_sources_write_lock: Arc::new(Mutex::new(())),
            skill_installs_write_lock: Arc::new(Mutex::new(())),
            skill_folders_write_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(RwLock::new(SettingsLoadState::FirstLaunch)),
            updater_state: Arc::new(RwLock::new(Default::default())),
        }
    }

    fn valid_files() -> Vec<DraftInputFile> {
        vec![DraftInputFile {
            relative_path: "SKILL.md".into(),
            text: Some("---\nname: reviewer\ndescription: Reviews code\n---\n".into()),
            base64: None,
        }]
    }

    #[tokio::test]
    async fn stages_validates_publishes_and_rejects_drafts() {
        let root = tempfile::tempdir().unwrap();
        let app = state(root.path());
        let draft = submit(&app, valid_files()).await.unwrap();
        assert!(draft.validation.installable);
        assert!(!drafts_root(root.path())
            .join(format!(".staging-{}", draft.id))
            .exists());
        let published = publish(&app, &draft.id).await.unwrap();
        assert_eq!(published.state, SkillDraftState::Published);
        assert!(published_root(root.path())
            .join("reviewer")
            .join("SKILL.md")
            .is_file());

        let mut invalid = valid_files();
        invalid.push(valid_files().remove(0));
        assert!(submit(&app, invalid).await.is_err());

        let pending = submit(
            &app,
            vec![DraftInputFile {
                relative_path: "SKILL.md".into(),
                text: Some("invalid".into()),
                base64: None,
            }],
        )
        .await
        .unwrap();
        assert!(!pending.validation.installable);
        let rejected = reject(&app, &pending.id).await.unwrap();
        assert_eq!(rejected.state, SkillDraftState::Rejected);
        assert!(!drafts_root(root.path()).join(pending.id).exists());
    }

    #[tokio::test]
    async fn rejects_non_normalized_duplicate_and_oversized_files() {
        let root = tempfile::tempdir().unwrap();
        let app = state(root.path());
        for path in ["../SKILL.md", "/SKILL.md", "a//b", "a\\b"] {
            assert!(submit(
                &app,
                vec![DraftInputFile {
                    relative_path: path.into(),
                    text: Some("x".into()),
                    base64: None,
                }]
            )
            .await
            .is_err());
        }
        let duplicate = vec![
            DraftInputFile {
                relative_path: "SKILL.md".into(),
                text: Some("x".into()),
                base64: None,
            },
            DraftInputFile {
                relative_path: "SKILL.md".into(),
                text: Some("y".into()),
                base64: None,
            },
        ];
        assert!(submit(&app, duplicate).await.is_err());
        for paths in [
            ["Refs/Guide.md", "refs/guide.md"],
            ["assets", "assets/icon.png"],
        ] {
            assert!(submit(
                &app,
                paths
                    .into_iter()
                    .map(|path| DraftInputFile {
                        relative_path: path.into(),
                        text: Some("x".into()),
                        base64: None,
                    })
                    .collect(),
            )
            .await
            .is_err());
        }
        assert!(submit(
            &app,
            vec![DraftInputFile {
                relative_path: "large".into(),
                text: Some("x".repeat(super::super::MAX_SKILL_FILE_BYTES as usize + 1)),
                base64: None,
            }]
        )
        .await
        .is_err());
        assert!(submit(
            &app,
            vec![DraftInputFile {
                relative_path: "encoded".into(),
                text: None,
                base64: Some(
                    "A".repeat((super::super::MAX_SKILL_FILE_BYTES as usize).div_ceil(3) * 4 + 1,),
                ),
            }]
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn publish_revalidates_staged_content() {
        let root = tempfile::tempdir().unwrap();
        let app = state(root.path());
        let draft = submit(&app, valid_files()).await.unwrap();
        tokio::fs::write(
            drafts_root(root.path())
                .join(&draft.id)
                .join("reviewer")
                .join("SKILL.md"),
            "---\nname: reviewer\ndescription: Changed after review\n---\n",
        )
        .await
        .unwrap();
        assert!(publish(&app, &draft.id).await.is_err());
        assert!(!published_root(root.path()).join("reviewer").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_linked_app_owned_roots() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), root.path().join("skills")).unwrap();
        assert!(submit(&state(root.path()), valid_files()).await.is_err());
        assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none());

        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("skills")).unwrap();
        symlink(outside.path(), root.path().join("skills").join("drafts")).unwrap();
        assert!(submit(&state(root.path()), valid_files()).await.is_err());

        let root = tempfile::tempdir().unwrap();
        let app = state(root.path());
        let draft = submit(&app, valid_files()).await.unwrap();
        symlink(outside.path(), root.path().join("skills").join("published")).unwrap();
        assert!(publish(&app, &draft.id).await.is_err());
        assert!(drafts_root(root.path()).join(draft.id).is_dir());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn publish_rejects_a_draft_root_replaced_by_a_link() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let app = state(root.path());
        let draft = submit(&app, valid_files()).await.unwrap();
        std::fs::remove_dir_all(drafts_root(root.path()).join(&draft.id)).unwrap();
        symlink(outside.path(), drafts_root(root.path()).join(&draft.id)).unwrap();

        assert!(publish(&app, &draft.id).await.is_err());
        assert!(
            std::fs::symlink_metadata(drafts_root(root.path()).join(&draft.id))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none());
    }

    #[tokio::test]
    async fn failed_index_commit_rolls_back_publication_and_new_source() {
        let root = tempfile::tempdir().unwrap();
        let app = state(root.path());
        let draft = submit(&app, valid_files()).await.unwrap();
        schedule_index_saves(root.path(), [true]);

        assert!(publish(&app, &draft.id).await.is_err());
        assert!(drafts_root(root.path())
            .join(&draft.id)
            .join("reviewer")
            .is_dir());
        assert!(!published_root(root.path()).join("reviewer").exists());
        assert!(super::super::load_skill_sources(root.path())
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn failed_submission_restores_transactionally_evicted_draft() {
        let root = tempfile::tempdir().unwrap();
        let app = state(root.path());
        let mut terminal = submit(&app, valid_files()).await.unwrap();
        terminal.state = SkillDraftState::Rejected;
        let terminal_id = terminal.id.clone();
        let mut index = vec![terminal.clone()];
        for _ in 1..MAX_DRAFTS {
            let mut entry = terminal.clone();
            entry.id = Uuid::new_v4().to_string();
            index.push(entry);
        }
        save_index(root.path(), &index).await.unwrap();
        schedule_index_saves(root.path(), [true]);

        assert!(submit(&app, valid_files()).await.is_err());
        let restored = load_index(root.path()).await.unwrap();
        assert_eq!(restored.len(), MAX_DRAFTS);
        assert_eq!(restored[0].id, terminal_id);
        assert!(drafts_root(root.path()).join(terminal_id).is_dir());
    }

    #[tokio::test]
    async fn double_failure_retains_the_last_durable_replacement_state() {
        let root = tempfile::tempdir().unwrap();
        let app = state(root.path());
        let mut terminal = submit(&app, valid_files()).await.unwrap();
        terminal.state = SkillDraftState::Rejected;
        let terminal_id = terminal.id.clone();
        let mut index = vec![terminal.clone()];
        for _ in 1..MAX_DRAFTS {
            let mut entry = terminal.clone();
            entry.id = Uuid::new_v4().to_string();
            index.push(entry);
        }
        save_index(root.path(), &index).await.unwrap();
        let canonical_drafts = std::fs::canonicalize(drafts_root(root.path())).unwrap();
        cleanup_failures()
            .lock()
            .expect("eviction cleanup failure probe")
            .insert(canonical_drafts);
        schedule_index_saves(root.path(), [false, true]);

        let error = submit(&app, valid_files()).await.unwrap_err().to_string();
        assert!(error.contains("retained committed replacement"));
        let durable = load_index(root.path()).await.unwrap();
        assert_eq!(durable.len(), MAX_DRAFTS);
        assert!(!durable.iter().any(|draft| draft.id == terminal_id));
        let replacement = durable.last().unwrap();
        assert_eq!(replacement.state, SkillDraftState::Pending);
        assert!(drafts_root(root.path()).join(&replacement.id).is_dir());
        assert!(!drafts_root(root.path()).join(&terminal_id).exists());
        assert!(std::fs::read_dir(drafts_root(root.path()))
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().starts_with(".evicted-")));
    }
}
