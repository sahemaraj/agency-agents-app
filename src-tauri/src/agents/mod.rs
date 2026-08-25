use std::collections::{HashMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::corpus;
use crate::error::AppError;
use crate::library;
use crate::state::AppState;
use crate::types::{
    AgentPackageResult, AgentPreferredSource, AgentReference, AgentSource, AgentSourceKind,
    AgentSourceResult, AgentValidationCode, AgentValidationError, CatalogSource,
};

pub(crate) mod drafts;
pub(crate) mod mcp;
pub(crate) mod organize;

pub(crate) const BUILTIN_AGENT_SOURCE_ID: &str = "builtin:agency-agents";
const MAX_AGENT_SOURCE_FILES: usize = 4096;
const MAX_AGENT_SOURCES: usize = 128;

fn invalid(message: impl Into<String>) -> AppError {
    AppError::InvalidArgument {
        message: message.into(),
    }
}

fn diagnostic(
    code: AgentValidationCode,
    path: impl Into<String>,
    message: impl Into<String>,
) -> AgentValidationError {
    AgentValidationError {
        code,
        path: path.into(),
        message: message.into(),
    }
}

fn sources_path(app_data_dir: &Path) -> PathBuf {
    corpus::state_dir(app_data_dir).join("agent-sources.json")
}

fn lock_sources(app_data_dir: &Path) -> Result<File, AppError> {
    let directory = corpus::state_dir(app_data_dir);
    std::fs::create_dir_all(&directory).map_err(|error| AppError::Io {
        message: format!("create Agent source state directory: {error}"),
    })?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("agent-sources.lock"))
        .map_err(|error| AppError::Io {
            message: format!("open Agent source state lock: {error}"),
        })?;
    file.lock().map_err(|error| AppError::Io {
        message: format!("lock Agent source state: {error}"),
    })?;
    Ok(file)
}

async fn lock_sources_async(app_data_dir: PathBuf) -> Result<File, AppError> {
    tokio::task::spawn_blocking(move || lock_sources(&app_data_dir))
        .await
        .map_err(|error| AppError::Internal {
            message: format!("Agent source lock task failed: {error}"),
        })?
}

async fn load_registered_sources(app_data_dir: &Path) -> Result<Vec<AgentSource>, AppError> {
    if let Some(database) = crate::state_db::StateDatabase::completed(app_data_dir).await? {
        return database.read(agent_sources_spec()).await?.ok_or_else(|| {
            AppError::StorageCorrupt {
                message: "Agent sources are missing after SQLite migration".into(),
            }
        });
    }
    let sources = match tokio::fs::read(sources_path(app_data_dir)).await {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| AppError::JsonParse {
            command: "agent_sources_list".into(),
            message: error.to_string(),
            raw_excerpt: String::from_utf8_lossy(&bytes[..bytes.len().min(256)]).into_owned(),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(AppError::Io {
            message: format!("read Agent sources: {error}"),
        }),
    }?;
    validate_registered_sources(&sources)?;
    Ok(sources)
}

fn validate_registered_sources_document(sources: &[AgentSource]) -> Result<(), AppError> {
    validate_registered_sources(sources)
}

fn agent_sources_spec() -> crate::state_db::DocumentSpec<Vec<AgentSource>> {
    crate::state_db::DocumentSpec::new("agent_sources", 1, 1_048_576, |sources| {
        validate_registered_sources_document(sources)
    })
}

pub(crate) fn agent_sources_import_spec() -> crate::state_db::ImportSpec {
    crate::state_db::ImportSpec::document(agent_sources_spec(), Vec::new())
}

fn validate_registered_sources(sources: &[AgentSource]) -> Result<(), AppError> {
    if sources.len() > MAX_AGENT_SOURCES {
        return Err(invalid("Agent source registry exceeds its limit"));
    }
    let mut ids = std::collections::HashSet::new();
    for source in sources {
        library::validate_reference(&source.id, "source.md")?;
        if source.id == BUILTIN_AGENT_SOURCE_ID
            || !ids.insert(source.id.as_str())
            || source.label.trim() != source.label
            || source.label.is_empty()
            || source.label.chars().count() > 128
        {
            return Err(invalid("Agent source identity or label is invalid"));
        }
        match &source.kind {
            AgentSourceKind::BuiltIn => {
                return Err(invalid("the built-in Agent source is implicit"));
            }
            AgentSourceKind::Local { root } | AgentSourceKind::Published { root } => {
                if !Path::new(root).is_absolute() {
                    return Err(invalid("persisted Agent source roots must be absolute"));
                }
            }
            AgentSourceKind::Github {
                repository,
                git_ref,
                resolved_commit,
                subdirectory,
                active_checkout,
            } => {
                if crate::skills::canonical_github_repository(repository)? != *repository {
                    return Err(invalid(
                        "persisted GitHub Agent repository is not canonical",
                    ));
                }
                crate::skills::validated_git_ref(git_ref.as_deref())?;
                crate::skills::validated_resolved_commit(resolved_commit.as_deref())?;
                crate::skills::validated_subdirectory(subdirectory.as_deref())?;
                if active_checkout
                    .as_deref()
                    .is_some_and(|path| !Path::new(path).is_absolute())
                {
                    return Err(invalid("active Agent checkout path must be absolute"));
                }
            }
        }
    }
    Ok(())
}

async fn save_registered_sources(
    app_data_dir: &Path,
    sources: &[AgentSource],
) -> Result<(), AppError> {
    if sources.len() > MAX_AGENT_SOURCES
        || sources
            .iter()
            .any(|source| source.id == BUILTIN_AGENT_SOURCE_ID)
    {
        return Err(invalid(
            "Agent source registry is invalid or exceeds its limit",
        ));
    }
    validate_registered_sources(sources)?;
    if let Some(database) = crate::state_db::StateDatabase::completed(app_data_dir).await? {
        let replacement = sources.to_vec();
        return database
            .mutate(agent_sources_spec(), Vec::new(), move |current| {
                *current = replacement;
                Ok(())
            })
            .await;
    }
    let directory = corpus::state_dir(app_data_dir);
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| AppError::Io {
            message: format!("create Agent source state directory: {error}"),
        })?;
    let bytes = serde_json::to_vec_pretty(sources).map_err(|error| AppError::Internal {
        message: format!("serialize Agent source registry: {error}"),
    })?;
    crate::util::fs::atomic_write(&sources_path(app_data_dir), &bytes).await
}

fn built_in_source() -> AgentSource {
    AgentSource {
        id: BUILTIN_AGENT_SOURCE_ID.into(),
        label: "Agency Agents".into(),
        enabled: true,
        kind: AgentSourceKind::BuiltIn,
    }
}

pub(crate) async fn load_agent_sources(app_data_dir: &Path) -> Result<Vec<AgentSource>, AppError> {
    let mut sources = vec![built_in_source()];
    let mut registered = load_registered_sources(app_data_dir).await?;
    registered.sort_by(|left, right| left.id.cmp(&right.id));
    sources.extend(registered);
    Ok(sources)
}

pub(crate) async fn add_local_source(
    app_data_dir: &Path,
    root: &Path,
) -> Result<AgentSource, AppError> {
    if !root.is_absolute() {
        return Err(invalid("local Agent source root must be absolute"));
    }
    let metadata = std::fs::symlink_metadata(root).map_err(|_| {
        invalid(format!(
            "local Agent source root must be an existing directory: {}",
            root.display()
        ))
    })?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || crate::skills::metadata_is_reparse_point(&metadata)
    {
        return Err(invalid(format!(
            "local Agent source root must be a real directory: {}",
            root.display()
        )));
    }
    let root = std::fs::canonicalize(root).map_err(|error| {
        invalid(format!(
            "could not resolve local Agent source root {}: {error}",
            root.display()
        ))
    })?;
    let root_string = root.to_string_lossy().into_owned();
    let _guard = lock_sources_async(app_data_dir.to_path_buf()).await?;
    let mut sources = load_registered_sources(app_data_dir).await?;
    if let Some(source) = sources.iter().find(
        |source| matches!(&source.kind, AgentSourceKind::Local { root } if root == &root_string),
    ) {
        return Ok(source.clone());
    }
    if sources.len() == MAX_AGENT_SOURCES {
        return Err(invalid(format!(
            "at most {MAX_AGENT_SOURCES} Agent sources are allowed"
        )));
    }
    let label = root
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("Local Agents")
        .to_owned();
    let source = AgentSource {
        id: Uuid::new_v4().to_string(),
        label,
        enabled: true,
        kind: AgentSourceKind::Local { root: root_string },
    };
    sources.push(source.clone());
    save_registered_sources(app_data_dir, &sources).await?;
    Ok(source)
}

#[cfg(test)]
pub(crate) async fn add_test_github_source(
    app_data_dir: &Path,
    root: &Path,
) -> Result<AgentSource, AppError> {
    let source = add_github_source(
        app_data_dir,
        &format!(
            "https://github.com/agency-agents-test/{}.git",
            Uuid::new_v4()
        ),
        None,
        None,
    )
    .await?;
    let mut sources = load_registered_sources(app_data_dir).await?;
    let portable = sources
        .iter_mut()
        .find(|candidate| candidate.id == source.id)
        .ok_or_else(|| invalid("registered test Agent source disappeared"))?;
    if let AgentSourceKind::Github {
        resolved_commit,
        active_checkout,
        ..
    } = &mut portable.kind
    {
        *resolved_commit = Some("a".repeat(40));
        *active_checkout = Some(std::fs::canonicalize(root)?.to_string_lossy().into_owned());
    }
    let portable = portable.clone();
    save_registered_sources(app_data_dir, &sources).await?;
    Ok(portable)
}

pub(crate) async fn add_github_source(
    app_data_dir: &Path,
    repository: &str,
    git_ref: Option<&str>,
    subdirectory: Option<&str>,
) -> Result<AgentSource, AppError> {
    let repository = crate::skills::canonical_github_repository(repository)?;
    let git_ref = crate::skills::validated_git_ref(git_ref)?;
    let subdirectory = crate::skills::validated_subdirectory(subdirectory)?;
    let _guard = lock_sources_async(app_data_dir.to_path_buf()).await?;
    let mut sources = load_registered_sources(app_data_dir).await?;
    if let Some(source) = sources.iter().find(|source| {
        matches!(
            &source.kind,
            AgentSourceKind::Github {
                repository: existing_repository,
                git_ref: existing_ref,
                subdirectory: existing_subdirectory,
                ..
            } if existing_repository == &repository
                && existing_ref == &git_ref
                && existing_subdirectory == &subdirectory
        )
    }) {
        return Ok(source.clone());
    }
    if sources.len() == MAX_AGENT_SOURCES {
        return Err(invalid(format!(
            "at most {MAX_AGENT_SOURCES} Agent sources are allowed"
        )));
    }
    let label = repository
        .trim_end_matches(".git")
        .rsplit('/')
        .next()
        .unwrap_or("GitHub Agents")
        .to_owned();
    let source = AgentSource {
        id: Uuid::new_v4().to_string(),
        label,
        enabled: true,
        kind: AgentSourceKind::Github {
            repository,
            git_ref,
            resolved_commit: None,
            subdirectory,
            active_checkout: None,
        },
    };
    sources.push(source.clone());
    save_registered_sources(app_data_dir, &sources).await?;
    Ok(source)
}

pub(crate) async fn remove_agent_source(
    app_data_dir: &Path,
    source_id: &str,
) -> Result<bool, AppError> {
    if source_id == BUILTIN_AGENT_SOURCE_ID {
        return Err(invalid("the built-in Agent source cannot be removed"));
    }
    let _guard = lock_sources_async(app_data_dir.to_path_buf()).await?;
    let mut sources = load_registered_sources(app_data_dir).await?;
    let original_len = sources.len();
    sources.retain(|source| source.id != source_id);
    if sources.len() == original_len {
        return Ok(false);
    }
    save_registered_sources(app_data_dir, &sources).await?;
    Ok(true)
}

fn source_root(app_data_dir: &Path, source: &AgentSource) -> Result<PathBuf, AppError> {
    match &source.kind {
        AgentSourceKind::BuiltIn => {
            let catalog = std::fs::read(corpus::state_dir(app_data_dir).join("catalog.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<CatalogSource>(&bytes).ok())
                .unwrap_or_default();
            Ok(corpus::catalog_root(app_data_dir, &catalog))
        }
        AgentSourceKind::Local { root } | AgentSourceKind::Published { root } => {
            Ok(PathBuf::from(root))
        }
        AgentSourceKind::Github {
            active_checkout: Some(root),
            ..
        } => Ok(PathBuf::from(root)),
        AgentSourceKind::Github { .. } => {
            Err(invalid("GitHub Agent source has no active checkout"))
        }
    }
}

fn validate_source_root(root: &Path) -> Result<(), AppError> {
    let metadata = std::fs::symlink_metadata(root).map_err(|error| {
        invalid(format!(
            "Agent source root must be an existing directory {}: {error}",
            root.display()
        ))
    })?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || crate::skills::metadata_is_reparse_point(&metadata)
    {
        return Err(invalid(format!(
            "Agent source root must be a real directory: {}",
            root.display()
        )));
    }
    Ok(())
}

fn relative_path(root: &Path, path: &Path) -> Result<String, AppError> {
    let relative = path.strip_prefix(root).map_err(|_| {
        invalid(format!(
            "Agent source entry resolves outside its source: {}",
            path.display()
        ))
    })?;
    let value = relative
        .components()
        .map(|component| {
            component
                .as_os_str()
                .to_str()
                .ok_or_else(|| invalid("Agent source paths must be valid UTF-8"))
        })
        .collect::<Result<Vec<_>, _>>()?
        .join("/");
    library::normalize_relative_path(&value)
}

fn read_directory_sorted(path: &Path) -> Result<Vec<(PathBuf, std::fs::Metadata)>, AppError> {
    let mut entries = std::fs::read_dir(path)
        .map_err(|error| AppError::Io {
            message: format!("read Agent source directory {}: {error}", path.display()),
        })?
        .map(|entry| {
            let entry = entry.map_err(|error| AppError::Io {
                message: format!("read Agent source entry: {error}"),
            })?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).map_err(|error| AppError::Io {
                message: format!("inspect Agent source entry {}: {error}", path.display()),
            })?;
            Ok((path, metadata))
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(entries)
}

fn quality(
    agent: &crate::types::Agent,
    metadata: &corpus::parse::AgentMetadata,
) -> (u8, Vec<String>) {
    let mut score = 25;
    let mut checks = vec!["Valid required metadata".into()];
    if agent.description.chars().count() >= 80 {
        score += 25;
        checks.push("Detailed description".into());
    }
    if !metadata.groups.is_empty() || !metadata.tags.is_empty() {
        score += 25;
        checks.push("Discoverability metadata".into());
    }
    if agent.body.to_lowercase().contains("example") || agent.body.contains("```") {
        score += 25;
        checks.push("Examples or references".into());
    }
    (score, checks)
}

fn derived_permissions(body: &str, declared: &[String]) -> Vec<String> {
    let lower = body.to_lowercase();
    let mut permissions = declared.to_vec();
    for (permission, tokens) in [
        ("network", &["https://", "http://", "curl ", "wget "][..]),
        (
            "filesystem",
            &["~/", "/users/", "filesystem", "read file", "write file"][..],
        ),
        (
            "external-tools",
            &["mcp", "command line", "shell command"][..],
        ),
    ] {
        if tokens.iter().any(|token| lower.contains(token)) {
            permissions.push(permission.into());
        }
    }
    permissions.sort();
    permissions.dedup();
    permissions
}

fn inspect_file(
    source_id: &str,
    root: &Path,
    path: &Path,
) -> Result<Option<AgentPackageResult>, AppError> {
    let relative = relative_path(root, path)?;
    let reference = AgentReference {
        source_id: source_id.into(),
        relative_path: relative.clone(),
    };
    library::validate_reference(&reference.source_id, &reference.relative_path)?;
    let metadata = std::fs::symlink_metadata(path).map_err(|error| AppError::Io {
        message: format!("inspect Agent file {}: {error}", path.display()),
    })?;
    if metadata.len() > corpus::MAX_AGENT_BYTES {
        return Ok(Some(AgentPackageResult {
            reference,
            agent: None,
            source_hash: String::new(),
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
            diagnostics: vec![diagnostic(
                AgentValidationCode::Oversize,
                relative,
                "Agent source exceeds the 1 MiB limit",
            )],
            installable: false,
        }));
    }
    let bytes = std::fs::read(path).map_err(|error| AppError::Io {
        message: format!("read Agent file {}: {error}", path.display()),
    })?;
    let source = match String::from_utf8(bytes) {
        Ok(source) => source,
        Err(_) => {
            return Ok(Some(AgentPackageResult {
                reference,
                agent: None,
                source_hash: String::new(),
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
                diagnostics: vec![diagnostic(
                    AgentValidationCode::InvalidMetadata,
                    relative,
                    "Agent source must be UTF-8",
                )],
                installable: false,
            }));
        }
    };
    let slug = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| invalid("Agent filename must be valid UTF-8"))?;
    let category = reference
        .relative_path
        .split('/')
        .next()
        .unwrap_or("custom");
    let parsed = match corpus::parse::parse_agent_package(slug, category, &source) {
        Ok(Some(parsed)) => parsed,
        Ok(None) => return Ok(None),
        Err(message) => {
            let source_hash = hex::encode(Sha256::digest(source.as_bytes()));
            return Ok(Some(AgentPackageResult {
                reference,
                agent: None,
                source_hash,
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
                diagnostics: vec![diagnostic(
                    AgentValidationCode::InvalidMetadata,
                    relative,
                    message,
                )],
                installable: false,
            }));
        }
    };
    let (quality_score, quality_checks) = quality(&parsed.agent, &parsed.metadata);
    let permissions = derived_permissions(&parsed.agent.body, &parsed.metadata.permissions);
    let publisher_verified = match (
        parsed.metadata.publisher.as_deref(),
        parsed.metadata.publisher_key.as_deref(),
        parsed.metadata.publisher_signature.as_deref(),
    ) {
        (Some(publisher), Some(key), Some(signature)) => {
            let version = parsed.metadata.version.as_deref().unwrap_or("0.0.0");
            let channel = parsed.metadata.channel.as_deref().unwrap_or("stable");
            library::verify_publisher_signature(
                publisher,
                key,
                signature,
                &[
                    parsed.agent.name.as_bytes(),
                    version.as_bytes(),
                    channel.as_bytes(),
                    parsed.agent.body.as_bytes(),
                ],
            )
        }
        _ => false,
    };
    let diagnostics = if parsed.metadata.publisher_signature.is_some() && !publisher_verified {
        vec![diagnostic(
            AgentValidationCode::InvalidMetadata,
            &relative,
            "Publisher signature verification failed",
        )]
    } else {
        Vec::new()
    };
    let installable = diagnostics.is_empty();
    Ok(Some(AgentPackageResult {
        reference,
        source_hash: parsed.entry.source_hash,
        frontmatter_hash: parsed.entry.frontmatter_hash,
        body_hash: parsed.entry.body_hash,
        version: parsed.metadata.version,
        channel: parsed.metadata.channel,
        changelog: parsed.metadata.changelog,
        publisher: parsed.metadata.publisher,
        publisher_key: parsed.metadata.publisher_key,
        publisher_verified,
        required_agents: parsed.metadata.required_agents,
        required_skills: parsed.metadata.required_skills,
        recommended_agents: parsed.metadata.recommended_agents,
        groups: parsed.metadata.groups,
        tags: parsed.metadata.tags,
        capabilities: parsed.metadata.capabilities,
        permissions,
        quality_score,
        quality_checks,
        diagnostics,
        installable,
        agent: Some(parsed.agent),
    }))
}

fn discover_source_blocking(
    app_data_dir: &Path,
    source: AgentSource,
) -> Result<AgentSourceResult, AppError> {
    let root = source_root(app_data_dir, &source)?;
    validate_source_root(&root)?;
    let root = std::fs::canonicalize(&root).map_err(|error| {
        invalid(format!(
            "could not resolve Agent source root {}: {error}",
            root.display()
        ))
    })?;
    let mut directories = VecDeque::from([root.clone()]);
    let mut files = Vec::new();
    let mut errors = Vec::new();
    while let Some(directory) = directories.pop_front() {
        for (path, metadata) in read_directory_sorted(&directory)? {
            let display_path = relative_path(&root, &path).unwrap_or_else(|_| ".".into());
            if metadata.is_dir() && path.file_name().is_some_and(|name| name == ".git") {
                continue;
            }
            if metadata.file_type().is_symlink()
                || crate::skills::metadata_is_reparse_point(&metadata)
            {
                errors.push(diagnostic(
                    AgentValidationCode::UnsafeEntry,
                    display_path,
                    "Links and reparse points are not allowed in Agent sources",
                ));
            } else if metadata.is_dir() {
                directories.push_back(path);
            } else if metadata.is_file()
                && path.extension().and_then(|extension| extension.to_str()) == Some("md")
            {
                if files.len() == MAX_AGENT_SOURCE_FILES {
                    errors.push(diagnostic(
                        AgentValidationCode::Oversize,
                        ".",
                        format!("Agent source exceeds {MAX_AGENT_SOURCE_FILES} Markdown files"),
                    ));
                    directories.clear();
                    break;
                }
                files.push(path);
            } else if !metadata.is_file() {
                errors.push(diagnostic(
                    AgentValidationCode::UnsafeEntry,
                    display_path,
                    "Special filesystem entries are not allowed in Agent sources",
                ));
            }
        }
    }
    files.sort();
    let mut agents = files
        .iter()
        .filter_map(|path| match inspect_file(&source.id, &root, path) {
            Ok(package) => package,
            Err(error) => {
                errors.push(diagnostic(
                    AgentValidationCode::Io,
                    relative_path(&root, path).unwrap_or_else(|_| ".".into()),
                    error.to_string(),
                ));
                None
            }
        })
        .collect::<Vec<_>>();
    agents.sort_by(|left, right| left.reference.cmp(&right.reference));

    let mut identities: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, package) in agents.iter().enumerate() {
        let key = library::portable_path_key(&package.reference.relative_path)?;
        identities.entry(key).or_default().push(index);
    }
    for indexes in identities.into_values().filter(|indexes| indexes.len() > 1) {
        for index in indexes {
            let path = agents[index].reference.relative_path.clone();
            agents[index].diagnostics.push(diagnostic(
                AgentValidationCode::DuplicateIdentity,
                path,
                "Agent source contains a portable path collision",
            ));
            agents[index].installable = false;
        }
    }

    let mut revision_hasher = Sha256::new();
    for package in &agents {
        revision_hasher.update(package.reference.relative_path.as_bytes());
        revision_hasher.update([0]);
        revision_hasher.update(package.source_hash.as_bytes());
        revision_hasher.update([u8::from(package.installable)]);
    }
    errors.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(AgentSourceResult {
        source,
        agents,
        errors,
        revision: hex::encode(revision_hasher.finalize()),
    })
}

pub(crate) async fn discover_agent_source(
    app_data_dir: &Path,
    source: AgentSource,
) -> Result<AgentSourceResult, AppError> {
    let app_data_dir = app_data_dir.to_path_buf();
    tokio::task::spawn_blocking(move || discover_source_blocking(&app_data_dir, source))
        .await
        .map_err(|error| AppError::Internal {
            message: format!("Agent source discovery task failed: {error}"),
        })?
}

async fn cleanup_checkout(path: &Path) {
    if let Err(error) = tokio::fs::remove_dir_all(path).await {
        if error.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(path = %path.display(), %error, "could not remove inactive Agent checkout");
        }
    }
}

pub(crate) async fn refresh_git_source(
    state: &AppState,
    source_id: &str,
) -> Result<AgentSourceResult, AppError> {
    let source = source_by_id(&state.app_data_dir, source_id).await?;
    let repository = match source.kind {
        AgentSourceKind::Github { repository, .. } => repository,
        _ => return Err(invalid("only GitHub Agent sources use Git refresh")),
    };
    refresh_git_source_from(state, source_id, &repository).await
}

async fn refresh_git_source_from(
    state: &AppState,
    source_id: &str,
    clone_source: &str,
) -> Result<AgentSourceResult, AppError> {
    refresh_git_source_from_ref(state, source_id, clone_source, None).await
}

async fn refresh_git_source_from_ref(
    state: &AppState,
    source_id: &str,
    clone_source: &str,
    checkout_override: Option<&str>,
) -> Result<AgentSourceResult, AppError> {
    state.require_network("agent_source_refresh").await?;
    let _guard = lock_sources_async(state.app_data_dir.clone()).await?;
    let mut sources = load_registered_sources(&state.app_data_dir).await?;
    let source_index = sources
        .iter()
        .position(|source| source.id == source_id)
        .ok_or_else(|| invalid(format!("unknown Agent source: {source_id}")))?;
    let (git_ref, subdirectory) = match &sources[source_index].kind {
        AgentSourceKind::Github {
            git_ref,
            subdirectory,
            ..
        } => (git_ref.clone(), subdirectory.clone()),
        _ => return Err(invalid("only GitHub Agent sources use Git refresh")),
    };

    let managed_root = state.app_data_dir.join("agents/sources");
    tokio::fs::create_dir_all(&managed_root)
        .await
        .map_err(|error| AppError::Io {
            message: format!(
                "create managed Agent source directory {}: {error}",
                managed_root.display()
            ),
        })?;
    let staging = managed_root.join(format!(".staging-{}", Uuid::new_v4()));
    let staging_arg = staging.to_string_lossy().into_owned();
    if let Err(error) = corpus::run_git(
        &["clone", "--no-checkout", "--", clone_source, &staging_arg],
        None,
    )
    .await
    {
        cleanup_checkout(&staging).await;
        return Err(error);
    }
    if let Err(error) = corpus::run_git(
        &[
            "checkout",
            "--detach",
            checkout_override.or(git_ref.as_deref()).unwrap_or("HEAD"),
            "--",
        ],
        Some(&staging),
    )
    .await
    {
        cleanup_checkout(&staging).await;
        return Err(error);
    }
    let resolved_commit = match corpus::run_git(&["rev-parse", "HEAD"], Some(&staging)).await {
        Ok(value) => Some(value.trim().to_ascii_lowercase()),
        Err(error) => {
            cleanup_checkout(&staging).await;
            return Err(error);
        }
    };

    let checkout_root = std::fs::canonicalize(&staging).map_err(|error| AppError::Io {
        message: format!(
            "resolve staged Agent checkout {}: {error}",
            staging.display()
        ),
    })?;
    let selected = subdirectory
        .as_deref()
        .map(|path| checkout_root.join(path))
        .unwrap_or_else(|| checkout_root.clone());
    let selected_metadata = match std::fs::symlink_metadata(&selected) {
        Ok(metadata) => metadata,
        Err(error) => {
            cleanup_checkout(&staging).await;
            return Err(AppError::Io {
                message: format!(
                    "inspect staged Agent source {}: {error}",
                    selected.display()
                ),
            });
        }
    };
    if !selected_metadata.is_dir()
        || selected_metadata.file_type().is_symlink()
        || crate::skills::metadata_is_reparse_point(&selected_metadata)
    {
        cleanup_checkout(&staging).await;
        return Err(invalid(
            "GitHub Agent source subdirectory must be a real directory",
        ));
    }
    let selected = std::fs::canonicalize(&selected).map_err(|error| AppError::Io {
        message: format!(
            "resolve staged Agent source {}: {error}",
            selected.display()
        ),
    })?;
    if !selected.starts_with(&checkout_root) {
        cleanup_checkout(&staging).await;
        return Err(invalid(
            "GitHub Agent source subdirectory resolves outside the staged checkout",
        ));
    }

    let mut staged_source = sources[source_index].clone();
    if let AgentSourceKind::Github {
        active_checkout, ..
    } = &mut staged_source.kind
    {
        *active_checkout = Some(selected.to_string_lossy().into_owned());
    }
    let candidate = match discover_agent_source(&state.app_data_dir, staged_source).await {
        Ok(candidate) => candidate,
        Err(error) => {
            cleanup_checkout(&staging).await;
            return Err(error);
        }
    };

    let generation = managed_root
        .join(source_id)
        .join(Uuid::new_v4().to_string());
    if let Some(parent) = generation.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| AppError::Io {
                message: format!("create Agent source generation directory: {error}"),
            })?;
    }
    if let Err(error) = tokio::fs::rename(&staging, &generation).await {
        cleanup_checkout(&staging).await;
        return Err(AppError::Io {
            message: format!(
                "activate staged Agent checkout {} -> {}: {error}",
                staging.display(),
                generation.display()
            ),
        });
    }

    let active_checkout = subdirectory
        .as_deref()
        .map(|path| generation.join(path))
        .unwrap_or_else(|| generation.clone());
    let mut active_source = candidate.source;
    if let AgentSourceKind::Github {
        active_checkout: active,
        resolved_commit: commit,
        ..
    } = &mut active_source.kind
    {
        *active = Some(active_checkout.to_string_lossy().into_owned());
        *commit = resolved_commit;
    }
    sources[source_index] = active_source.clone();
    if let Err(error) = save_registered_sources(&state.app_data_dir, &sources).await {
        cleanup_checkout(&generation).await;
        return Err(error);
    }

    Ok(AgentSourceResult {
        source: active_source,
        agents: candidate.agents,
        errors: candidate.errors,
        revision: candidate.revision,
    })
}

pub(crate) async fn materialize_github_source(
    state: &AppState,
    repository: &str,
    requested_ref: Option<&str>,
    resolved_commit: Option<&str>,
    subdirectory: Option<&str>,
) -> Result<AgentSourceResult, AppError> {
    let source =
        add_github_source(&state.app_data_dir, repository, requested_ref, subdirectory).await?;
    refresh_git_source_from_ref(state, &source.id, repository, resolved_commit).await
}

pub(crate) async fn inspect_agent_sources(
    app_data_dir: &Path,
) -> Result<Vec<AgentSourceResult>, AppError> {
    let sources = load_agent_sources(app_data_dir).await?;
    let mut results = Vec::with_capacity(sources.len());
    for source in sources.into_iter().filter(|source| source.enabled) {
        match discover_agent_source(app_data_dir, source.clone()).await {
            Ok(result) => results.push(result),
            Err(error) => results.push(AgentSourceResult {
                source,
                agents: Vec::new(),
                errors: vec![diagnostic(AgentValidationCode::Io, ".", error.to_string())],
                revision: String::new(),
            }),
        }
    }
    Ok(results)
}

pub(crate) async fn inspect_builtin_agent_source(
    app_data_dir: &Path,
) -> Result<AgentSourceResult, AppError> {
    discover_agent_source(app_data_dir, built_in_source()).await
}

pub(crate) async fn source_by_id(
    app_data_dir: &Path,
    source_id: &str,
) -> Result<AgentSource, AppError> {
    load_agent_sources(app_data_dir)
        .await?
        .into_iter()
        .find(|source| source.id == source_id && source.enabled)
        .ok_or_else(|| invalid(format!("unknown or disabled Agent source: {source_id}")))
}

pub(crate) async fn read_agent_text(
    app_data_dir: &Path,
    reference: &AgentReference,
) -> Result<String, AppError> {
    library::validate_reference(&reference.source_id, &reference.relative_path)?;
    let source = source_by_id(app_data_dir, &reference.source_id).await?;
    let root = source_root(app_data_dir, &source)?;
    validate_source_root(&root)?;
    let root = std::fs::canonicalize(&root).map_err(|error| {
        invalid(format!(
            "could not resolve Agent source root {}: {error}",
            root.display()
        ))
    })?;
    let candidate = root.join(&reference.relative_path);
    let metadata = std::fs::symlink_metadata(&candidate).map_err(|error| AppError::Io {
        message: format!("inspect Agent source {}: {error}", candidate.display()),
    })?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || crate::skills::metadata_is_reparse_point(&metadata)
    {
        return Err(invalid(
            "Agent source entry must be a regular unlinked file",
        ));
    }
    let candidate = std::fs::canonicalize(&candidate).map_err(|error| AppError::Io {
        message: format!("resolve Agent source {}: {error}", candidate.display()),
    })?;
    if !candidate.starts_with(&root) {
        return Err(invalid("Agent source entry resolves outside its source"));
    }
    let bytes = crate::util::fs::read_capped(&candidate, corpus::MAX_AGENT_BYTES).await?;
    String::from_utf8(bytes).map_err(|_| invalid("Agent source must be UTF-8"))
}

pub(crate) async fn resolve_agent_package(
    app_data_dir: &Path,
    reference: &AgentReference,
) -> Result<AgentPackageResult, AppError> {
    let source = source_by_id(app_data_dir, &reference.source_id).await?;
    discover_agent_source(app_data_dir, source)
        .await?
        .agents
        .into_iter()
        .find(|package| package.reference == *reference)
        .ok_or_else(|| {
            invalid(format!(
                "unknown Agent reference: {}:{}",
                reference.source_id, reference.relative_path
            ))
        })
}

#[derive(Debug, Default)]
pub(crate) struct AgentDependencyResolution {
    pub ordered: Vec<AgentReference>,
    pub warnings: Vec<String>,
    pub blockers: Vec<String>,
}

pub(crate) fn resolve_agent_dependencies(
    sources: &[AgentSourceResult],
    roots: &[AgentReference],
    preferred_sources: &[AgentPreferredSource],
) -> AgentDependencyResolution {
    let packages = sources
        .iter()
        .flat_map(|source| &source.agents)
        .map(|package| (package.reference.clone(), package))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut result = AgentDependencyResolution::default();
    let mut visiting = std::collections::BTreeSet::new();
    let mut visited = std::collections::BTreeSet::new();

    fn resolve_token(
        token: &str,
        owner: &AgentReference,
        packages: &std::collections::BTreeMap<AgentReference, &AgentPackageResult>,
        preferred_sources: &[AgentPreferredSource],
    ) -> Result<AgentReference, String> {
        if token.trim() != token || token.is_empty() || token.chars().any(char::is_control) {
            return Err(format!("invalid Agent dependency: {token}"));
        }
        if token.contains('/') || token.ends_with(".md") {
            crate::library::normalize_relative_path(token)
                .map_err(|_| format!("invalid Agent dependency path: {token}"))?;
        }
        let same_source = AgentReference {
            source_id: owner.source_id.clone(),
            relative_path: token.into(),
        };
        if packages.contains_key(&same_source) {
            return Ok(same_source);
        }
        let mut candidates = packages
            .iter()
            .filter(|(reference, package)| {
                reference.relative_path == token
                    || package.agent.as_ref().is_some_and(|agent| {
                        agent.name.eq_ignore_ascii_case(token)
                            || agent.slug.eq_ignore_ascii_case(token)
                    })
            })
            .map(|(reference, _)| reference.clone())
            .collect::<Vec<_>>();
        candidates.sort();
        candidates.dedup();
        if candidates.len() == 1 {
            return Ok(candidates.remove(0));
        }
        if candidates.len() > 1 {
            let preferred = preferred_sources.iter().find(|preference| {
                preference.agent_name.eq_ignore_ascii_case(token)
                    || candidates.iter().any(|reference| {
                        packages
                            .get(reference)
                            .and_then(|package| package.agent.as_ref())
                            .is_some_and(|agent| {
                                agent.name.eq_ignore_ascii_case(&preference.agent_name)
                            })
                    })
            });
            if let Some(preference) = preferred {
                let mut selected = candidates
                    .into_iter()
                    .filter(|reference| reference.source_id == preference.source_id)
                    .collect::<Vec<_>>();
                if selected.len() == 1 {
                    return Ok(selected.remove(0));
                }
            }
            return Err(format!("ambiguous Agent dependency: {token}"));
        }
        Err(format!("missing Agent dependency: {token}"))
    }

    fn visit(
        reference: &AgentReference,
        packages: &std::collections::BTreeMap<AgentReference, &AgentPackageResult>,
        preferred_sources: &[AgentPreferredSource],
        visiting: &mut std::collections::BTreeSet<AgentReference>,
        visited: &mut std::collections::BTreeSet<AgentReference>,
        result: &mut AgentDependencyResolution,
    ) {
        if visited.contains(reference) {
            return;
        }
        if !visiting.insert(reference.clone()) {
            result.blockers.push(format!(
                "Agent dependency cycle includes {}:{}",
                reference.source_id, reference.relative_path
            ));
            return;
        }
        let Some(package) = packages.get(reference).copied() else {
            result.blockers.push(format!(
                "unknown Agent dependency root: {}:{}",
                reference.source_id, reference.relative_path
            ));
            visiting.remove(reference);
            return;
        };
        if !package.installable {
            result.blockers.push(format!(
                "Agent dependency is not installable: {}:{}",
                reference.source_id, reference.relative_path
            ));
        }
        let mut required = package.required_agents.clone();
        required.sort_by_key(|value| value.to_lowercase());
        required.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        for token in required {
            match resolve_token(&token, reference, packages, preferred_sources) {
                Ok(dependency) => visit(
                    &dependency,
                    packages,
                    preferred_sources,
                    visiting,
                    visited,
                    result,
                ),
                Err(blocker) => result.blockers.push(blocker),
            }
        }
        let mut recommended = package.recommended_agents.clone();
        recommended.sort_by_key(|value| value.to_lowercase());
        recommended.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        for token in recommended {
            let label = resolve_token(&token, reference, packages, preferred_sources)
                .ok()
                .and_then(|resolved| packages.get(&resolved))
                .and_then(|recommended| recommended.agent.as_ref())
                .map(|agent| agent.name.clone())
                .unwrap_or(token);
            result.warnings.push(format!(
                "Recommended Agent not included automatically: {label}"
            ));
        }
        visiting.remove(reference);
        if visited.insert(reference.clone()) {
            result.ordered.push(reference.clone());
        }
    }

    let mut roots = roots.to_vec();
    roots.sort();
    roots.dedup();
    for root in roots {
        visit(
            &root,
            &packages,
            preferred_sources,
            &mut visiting,
            &mut visited,
            &mut result,
        );
    }
    result.warnings.sort();
    result.warnings.dedup();
    result.blockers.sort();
    result.blockers.dedup();
    result
}

#[tauri::command]
pub async fn agent_sources_list(state: State<'_, AppState>) -> Result<Vec<AgentSource>, AppError> {
    load_agent_sources(&state.app_data_dir).await
}

#[tauri::command]
pub async fn agent_sources_inspect(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<AgentSourceResult>, AppError> {
    corpus::ensure_corpus(&app, &state).await?;
    inspect_agent_sources(&state.app_data_dir).await
}

#[tauri::command]
pub async fn agent_source_add_local(
    state: State<'_, AppState>,
    root: String,
) -> Result<AgentSource, AppError> {
    add_local_source(&state.app_data_dir, Path::new(&root)).await
}

#[tauri::command]
pub async fn agent_source_add_github(
    state: State<'_, AppState>,
    repository: String,
    git_ref: Option<String>,
    subdirectory: Option<String>,
) -> Result<AgentSource, AppError> {
    add_github_source(
        &state.app_data_dir,
        &repository,
        git_ref.as_deref(),
        subdirectory.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn agent_source_refresh(
    app: AppHandle,
    state: State<'_, AppState>,
    source_id: String,
) -> Result<AgentSourceResult, AppError> {
    if source_id == BUILTIN_AGENT_SOURCE_ID {
        corpus::ensure_corpus(&app, &state).await?;
    }
    let source = source_by_id(&state.app_data_dir, &source_id).await?;
    if matches!(source.kind, AgentSourceKind::Github { .. }) {
        refresh_git_source(&state, &source_id).await
    } else {
        discover_agent_source(&state.app_data_dir, source).await
    }
}

#[tauri::command]
pub async fn agent_source_remove(
    state: State<'_, AppState>,
    source_id: String,
) -> Result<bool, AppError> {
    remove_agent_source(&state.app_data_dir, &source_id).await
}

#[tauri::command]
pub async fn agent_source_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<AgentSourceResult>, AppError> {
    corpus::ensure_corpus(&app, &state).await?;
    inspect_agent_sources(&state.app_data_dir).await
}

#[tauri::command]
pub async fn agent_get(
    app: AppHandle,
    state: State<'_, AppState>,
    source_id: String,
    relative_path: String,
) -> Result<AgentPackageResult, AppError> {
    if source_id == BUILTIN_AGENT_SOURCE_ID {
        corpus::ensure_corpus(&app, &state).await?;
    }
    resolve_agent_package(
        &state.app_data_dir,
        &AgentReference {
            source_id,
            relative_path,
        },
    )
    .await
}

#[tauri::command]
pub async fn agent_text_read(
    app: AppHandle,
    state: State<'_, AppState>,
    source_id: String,
    relative_path: String,
) -> Result<String, AppError> {
    if source_id == BUILTIN_AGENT_SOURCE_ID {
        corpus::ensure_corpus(&app, &state).await?;
    }
    read_agent_text(
        &state.app_data_dir,
        &AgentReference {
            source_id,
            relative_path,
        },
    )
    .await
}

#[tauri::command]
pub async fn agent_render_preview(
    app: AppHandle,
    state: State<'_, AppState>,
    source_id: String,
    relative_path: String,
    tool: String,
) -> Result<String, AppError> {
    if source_id == BUILTIN_AGENT_SOURCE_ID {
        corpus::ensure_corpus(&app, &state).await?;
    }
    let reference = AgentReference {
        source_id,
        relative_path,
    };
    let package = resolve_agent_package(&state.app_data_dir, &reference).await?;
    let raw = read_agent_text(&state.app_data_dir, &reference).await?;
    let agent = package
        .agent
        .ok_or_else(|| invalid("Rejected Agent packages cannot be rendered"))?;
    crate::render::render(&agent, &raw, &tool)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    use crate::commands::settings::SettingsLoadState;
    use crate::types::AgentSourceKind;
    use base64::Engine as _;
    use ed25519_dalek::{Signer, SigningKey};

    fn run(command: &mut Command) {
        assert!(command.status().unwrap().success());
    }

    #[test]
    fn local_discovery_preserves_nested_duplicate_slugs() {
        let root = tempfile::tempdir().unwrap();
        for engine in ["godot", "unity"] {
            let directory = root.path().join("game-development").join(engine);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(
                directory.join("shader-developer.md"),
                format!("---\nname: {engine} Shader Developer\ndescription: Builds shaders.\n---\nWork carefully.\n"),
            )
            .unwrap();
        }
        let source = AgentSource {
            id: "local:test".into(),
            label: "Test".into(),
            enabled: true,
            kind: AgentSourceKind::Local {
                root: root.path().to_string_lossy().into_owned(),
            },
        };

        let result = discover_source_blocking(root.path(), source).unwrap();
        let references = result
            .agents
            .iter()
            .map(|package| package.reference.relative_path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            references,
            [
                "game-development/godot/shader-developer.md",
                "game-development/unity/shader-developer.md"
            ]
        );
        assert!(result.agents.iter().all(|package| package.installable));
    }

    #[tokio::test]
    async fn publisher_signature_is_verified_and_body_changes_invalidate_it() {
        let app_data = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let body = "Review carefully.\n";
        let mut payload = Sha256::new();
        payload.update(b"Acme");
        for part in ["Reviewer", "1.2.3", "stable", body] {
            payload.update([0]);
            payload.update(part.as_bytes());
        }
        let public_key = base64::engine::general_purpose::STANDARD
            .encode(signing_key.verifying_key().to_bytes());
        let signature = base64::engine::general_purpose::STANDARD
            .encode(signing_key.sign(&payload.finalize()).to_bytes());
        let markdown = |body: &str| {
            format!(
                "---\nname: Reviewer\ndescription: Reviews code\nversion: 1.2.3\nchannel: stable\npublisher: Acme\npublisher-key: {public_key}\npublisher-signature: {signature}\n---\n{body}"
            )
        };
        let path = source.path().join("reviewer.md");
        std::fs::write(&path, markdown(body)).unwrap();
        let registered = add_local_source(app_data.path(), source.path())
            .await
            .unwrap();

        let inspect = || async {
            inspect_agent_sources(app_data.path())
                .await
                .unwrap()
                .into_iter()
                .find(|result| result.source.id == registered.id)
                .unwrap()
                .agents
                .into_iter()
                .find(|agent| agent.reference.relative_path == "reviewer.md")
                .unwrap()
        };
        let verified = inspect().await;
        assert!(verified.publisher_verified);
        assert!(verified.installable);

        std::fs::write(&path, markdown("Review differently.\n")).unwrap();
        let changed = inspect().await;
        assert!(!changed.publisher_verified);
        assert!(!changed.installable);
        assert!(changed
            .diagnostics
            .iter()
            .any(|item| item.message.contains("signature verification failed")));
    }

    #[test]
    fn source_discovery_ignores_git_metadata() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join(".git")).unwrap();
        for path in [
            root.path().join("agent.md"),
            root.path().join(".git/hidden.md"),
        ] {
            std::fs::write(
                path,
                "---\nname: Agent\ndescription: Works.\n---\nWork carefully.\n",
            )
            .unwrap();
        }
        let result = discover_source_blocking(
            root.path(),
            AgentSource {
                id: "local:test".into(),
                label: "Test".into(),
                enabled: true,
                kind: AgentSourceKind::Local {
                    root: root.path().to_string_lossy().into_owned(),
                },
            },
        )
        .unwrap();
        assert_eq!(result.agents.len(), 1);
        assert_eq!(result.agents[0].reference.relative_path, "agent.md");
    }

    #[test]
    fn source_discovery_exposes_required_skills() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("primavera-agent.md"),
            "---\nname: Primavera Agent\ndescription: Advises on P6.\nrequired-skills: [primavera-p6-eppm]\n---\nUse the required skill.\n",
        )
        .unwrap();
        let result = discover_source_blocking(
            root.path(),
            AgentSource {
                id: "local:test".into(),
                label: "Test".into(),
                enabled: true,
                kind: AgentSourceKind::Local {
                    root: root.path().to_string_lossy().into_owned(),
                },
            },
        )
        .unwrap();

        let package = serde_json::to_value(&result.agents[0]).unwrap();
        assert_eq!(
            package["requiredSkills"],
            serde_json::json!(["primavera-p6-eppm"])
        );
    }

    fn dependency_source(
        root: &Path,
        source_id: &str,
        files: &[(&str, &str, &[&str], &[&str])],
    ) -> AgentSourceResult {
        for (path, name, required, recommended) in files {
            let destination = root.join(path);
            std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
            let required = serde_json::to_string(required).unwrap();
            let recommended = serde_json::to_string(recommended).unwrap();
            std::fs::write(
                destination,
                format!(
                    "---\nname: {name}\ndescription: Works.\nrequired-agents: {required}\nrecommended-agents: {recommended}\n---\nWork carefully.\n"
                ),
            )
            .unwrap();
        }
        discover_source_blocking(
            root,
            AgentSource {
                id: source_id.into(),
                label: source_id.into(),
                enabled: true,
                kind: AgentSourceKind::Local {
                    root: root.to_string_lossy().into_owned(),
                },
            },
        )
        .unwrap()
    }

    #[test]
    fn dependency_graph_is_topological_and_recommendations_are_informational() {
        let root = tempfile::tempdir().unwrap();
        let source = dependency_source(
            root.path(),
            "local:test",
            &[
                ("base.md", "Base", &[], &[]),
                ("mid.md", "Mid", &["base.md"], &[]),
                ("optional.md", "Optional", &[], &[]),
                ("root.md", "Root", &["mid.md"], &["optional.md"]),
            ],
        );
        let root_reference = source
            .agents
            .iter()
            .find(|package| package.reference.relative_path == "root.md")
            .unwrap()
            .reference
            .clone();

        let resolution =
            resolve_agent_dependencies(std::slice::from_ref(&source), &[root_reference], &[]);
        assert!(resolution.blockers.is_empty());
        assert_eq!(
            resolution
                .ordered
                .iter()
                .map(|reference| reference.relative_path.as_str())
                .collect::<Vec<_>>(),
            ["base.md", "mid.md", "root.md"]
        );
        assert!(resolution
            .warnings
            .iter()
            .any(|warning| warning.contains("Optional")));
    }

    #[test]
    fn dependency_graph_requires_preferred_source_for_ambiguous_names() {
        let first_root = tempfile::tempdir().unwrap();
        let second_root = tempfile::tempdir().unwrap();
        let root_root = tempfile::tempdir().unwrap();
        let first = dependency_source(
            first_root.path(),
            "local:first",
            &[("reviewer.md", "Reviewer", &[], &[])],
        );
        let second = dependency_source(
            second_root.path(),
            "local:second",
            &[("reviewer.md", "Reviewer", &[], &[])],
        );
        let root_source = dependency_source(
            root_root.path(),
            "local:root",
            &[("root.md", "Root", &["Reviewer"], &[])],
        );
        let root_reference = root_source.agents[0].reference.clone();
        let sources = [first, second, root_source];

        assert!(
            !resolve_agent_dependencies(&sources, std::slice::from_ref(&root_reference), &[])
                .blockers
                .is_empty()
        );
        let preferred = [crate::types::AgentPreferredSource {
            agent_name: "Reviewer".into(),
            source_id: "local:second".into(),
        }];
        let resolution =
            resolve_agent_dependencies(&sources, std::slice::from_ref(&root_reference), &preferred);
        assert!(resolution.blockers.is_empty());
        assert_eq!(resolution.ordered[0].source_id, "local:second");
    }

    #[test]
    fn dependency_graph_blocks_missing_invalid_and_cyclic_edges() {
        let root = tempfile::tempdir().unwrap();
        let source = dependency_source(
            root.path(),
            "local:test",
            &[
                ("a.md", "A", &["b.md"], &[]),
                ("b.md", "B", &["a.md"], &[]),
                ("missing.md", "Missing Root", &["does-not-exist"], &[]),
                ("invalid.md", "Invalid Root", &["../escape.md"], &[]),
            ],
        );
        for path in ["a.md", "missing.md", "invalid.md"] {
            let reference = source
                .agents
                .iter()
                .find(|package| package.reference.relative_path == path)
                .unwrap()
                .reference
                .clone();
            assert!(
                !resolve_agent_dependencies(std::slice::from_ref(&source), &[reference], &[],)
                    .blockers
                    .is_empty()
            );
        }
    }

    #[tokio::test]
    async fn local_source_registration_is_canonical_deduplicated_and_non_destructive() {
        let app_data = tempfile::tempdir().unwrap();
        let source_root = tempfile::tempdir().unwrap();

        let first = add_local_source(app_data.path(), source_root.path())
            .await
            .unwrap();
        let second = add_local_source(app_data.path(), source_root.path())
            .await
            .unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(load_agent_sources(app_data.path()).await.unwrap().len(), 2);

        assert!(remove_agent_source(app_data.path(), &first.id)
            .await
            .unwrap());
        assert!(
            source_root.path().exists(),
            "unregister must not delete source content"
        );
        assert_eq!(load_agent_sources(app_data.path()).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn github_source_registration_reuses_skills_validation_and_deduplicates() {
        let app_data = tempfile::tempdir().unwrap();

        assert!(add_github_source(
            app_data.path(),
            "https://github.com/acme/agents",
            Some("../main"),
            None,
        )
        .await
        .is_err());

        let first = add_github_source(
            app_data.path(),
            "https://github.com/acme/agents",
            Some("main"),
            Some("catalog"),
        )
        .await
        .unwrap();
        let second = add_github_source(
            app_data.path(),
            "https://github.com/acme/agents.git",
            Some("main"),
            Some("catalog"),
        )
        .await
        .unwrap();

        assert_eq!(first.id, second.id);
        assert!(matches!(
            first.kind,
            AgentSourceKind::Github {
                repository,
                active_checkout: None,
                ..
            } if repository == "https://github.com/acme/agents.git"
        ));
    }

    #[tokio::test]
    async fn github_refresh_activates_only_a_valid_checkout_and_rolls_back_on_failure() {
        let app_data = tempfile::tempdir().unwrap();
        let repository = tempfile::tempdir().unwrap();
        run(Command::new("git").args(["init", repository.path().to_str().unwrap()]));
        run(Command::new("git").args([
            "-C",
            repository.path().to_str().unwrap(),
            "config",
            "user.email",
            "test@example.com",
        ]));
        run(Command::new("git").args([
            "-C",
            repository.path().to_str().unwrap(),
            "config",
            "user.name",
            "Test",
        ]));
        std::fs::write(
            repository.path().join("reviewer.md"),
            "---\nname: Reviewer\ndescription: Reviews code.\n---\nReview carefully.\n",
        )
        .unwrap();
        run(Command::new("git").args([
            "-C",
            repository.path().to_str().unwrap(),
            "add",
            "reviewer.md",
        ]));
        run(Command::new("git").args([
            "-C",
            repository.path().to_str().unwrap(),
            "commit",
            "-m",
            "fixture",
        ]));

        let source = AgentSource {
            id: "github:test".into(),
            label: "Test".into(),
            enabled: true,
            kind: AgentSourceKind::Github {
                repository: "https://github.com/acme/agents.git".into(),
                git_ref: None,
                resolved_commit: None,
                subdirectory: None,
                active_checkout: None,
            },
        };
        save_registered_sources(app_data.path(), &[source])
            .await
            .unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = app_data.path().to_path_buf();
        state.settings =
            std::sync::Arc::new(tokio::sync::RwLock::new(SettingsLoadState::FirstLaunch));

        let result =
            refresh_git_source_from(&state, "github:test", repository.path().to_str().unwrap())
                .await
                .unwrap();
        assert_eq!(result.agents.len(), 1);
        let active = match &result.source.kind {
            AgentSourceKind::Github {
                active_checkout: Some(path),
                ..
            } => path.clone(),
            _ => panic!("refresh must activate a checkout"),
        };
        assert!(Path::new(&active).is_dir());

        assert!(
            refresh_git_source_from(&state, "github:test", "/missing/repository")
                .await
                .is_err()
        );
        let persisted = load_registered_sources(app_data.path()).await.unwrap();
        assert!(matches!(
            &persisted[0].kind,
            AgentSourceKind::Github {
                active_checkout: Some(path),
                resolved_commit: Some(commit),
                ..
            } if path == &active && commit.len() == 40
        ));
    }

    #[tokio::test]
    async fn persisted_sources_are_revalidated_before_use() {
        let app_data = tempfile::tempdir().unwrap();
        let invalid_source = AgentSource {
            id: "bad".into(),
            label: "Bad".into(),
            enabled: true,
            kind: AgentSourceKind::Local {
                root: "relative/path".into(),
            },
        };
        let state_dir = corpus::state_dir(app_data.path());
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::write(
            sources_path(app_data.path()),
            serde_json::to_vec(&vec![invalid_source]).unwrap(),
        )
        .unwrap();

        assert!(load_agent_sources(app_data.path()).await.is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn exact_reads_reject_a_source_root_replaced_by_a_link() {
        use std::os::unix::fs::symlink;

        let app_data = tempfile::tempdir().unwrap();
        let parent = tempfile::tempdir().unwrap();
        let source_root = parent.path().join("source");
        let moved_root = parent.path().join("moved");
        std::fs::create_dir(&source_root).unwrap();
        std::fs::write(
            source_root.join("reviewer.md"),
            "---\nname: Reviewer\ndescription: Reviews code.\n---\n",
        )
        .unwrap();
        let source = add_local_source(app_data.path(), &source_root)
            .await
            .unwrap();
        std::fs::rename(&source_root, &moved_root).unwrap();
        symlink(&moved_root, &source_root).unwrap();

        assert!(read_agent_text(
            app_data.path(),
            &AgentReference {
                source_id: source.id,
                relative_path: "reviewer.md".into(),
            },
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn corrupt_source_registry_fails_without_rewriting_it() {
        let app_data = tempfile::tempdir().unwrap();
        let path = sources_path(app_data.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{not-json").unwrap();

        assert!(load_agent_sources(app_data.path()).await.is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"{not-json");
    }
}
