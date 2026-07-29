use std::collections::VecDeque;
use std::fs::{File, Metadata};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use tauri::State;
use uuid::Uuid;

use crate::corpus::state_dir;
use crate::error::AppError;
use crate::state::AppState;
use crate::types::{
    SkillPackageFile, SkillPackageResult, SkillSource, SkillSourceKind, SkillSourceResult,
    SkillValidationCode, SkillValidationError,
};
use crate::util::fs::atomic_write;

pub const MAX_SKILL_FILES: usize = 512;
pub const MAX_SKILL_FILE_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_SKILL_TOTAL_BYTES: u64 = 32 * 1024 * 1024;

pub(crate) fn skill_sources_path(app_data_dir: &Path) -> PathBuf {
    state_dir(app_data_dir).join("skill-sources.json")
}

pub(crate) async fn load_skill_sources(app_data_dir: &Path) -> Result<Vec<SkillSource>, AppError> {
    let path = skill_sources_path(app_data_dir);
    match tokio::fs::read(&path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| AppError::JsonParse {
            command: "skill_sources_list".into(),
            message: error.to_string(),
            raw_excerpt: String::from_utf8_lossy(&bytes[..bytes.len().min(256)]).into_owned(),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(AppError::Io {
            message: format!("read {}: {error}", path.display()),
        }),
    }
}

async fn save_skill_sources(app_data_dir: &Path, sources: &[SkillSource]) -> Result<(), AppError> {
    let directory = state_dir(app_data_dir);
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| AppError::Io {
            message: format!("create state dir {}: {error}", directory.display()),
        })?;
    let bytes = serde_json::to_vec_pretty(sources).map_err(|error| AppError::Internal {
        message: format!("serialize skill-sources.json: {error}"),
    })?;
    atomic_write(&skill_sources_path(app_data_dir), &bytes).await
}

pub(crate) async fn add_local_source(
    state: &AppState,
    root: &Path,
) -> Result<SkillSource, AppError> {
    if !root.is_absolute() {
        return Err(AppError::InvalidArgument {
            message: "local skill source root must be absolute".into(),
        });
    }
    let metadata = std::fs::symlink_metadata(root).map_err(|_| AppError::InvalidArgument {
        message: format!(
            "local skill source root must be an existing directory: {}",
            root.display()
        ),
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AppError::InvalidArgument {
            message: format!(
                "local skill source root must be a real directory: {}",
                root.display()
            ),
        });
    }
    let canonical_root =
        std::fs::canonicalize(root).map_err(|error| AppError::InvalidArgument {
            message: format!(
                "could not resolve local skill source root {}: {error}",
                root.display()
            ),
        })?;
    let root_string = canonical_root.to_string_lossy().into_owned();

    let _guard = state.skill_sources_write_lock.lock().await;
    let mut sources = load_skill_sources(&state.app_data_dir).await?;
    if let Some(existing) = sources.iter().find(
        |source| matches!(&source.kind, SkillSourceKind::Local { root } if root == &root_string),
    ) {
        return Ok(existing.clone());
    }

    let source = SkillSource {
        id: Uuid::new_v4().to_string(),
        kind: SkillSourceKind::Local { root: root_string },
    };
    sources.push(source.clone());
    save_skill_sources(&state.app_data_dir, &sources).await?;
    Ok(source)
}

pub(crate) async fn discover_source(source: SkillSource) -> Result<SkillSourceResult, AppError> {
    tokio::task::spawn_blocking(move || discover_source_blocking(source))
        .await
        .map_err(|error| AppError::Internal {
            message: format!("skill source discovery task failed: {error}"),
        })?
}

fn discover_source_blocking(source: SkillSource) -> Result<SkillSourceResult, AppError> {
    let root = match &source.kind {
        SkillSourceKind::Local { root } => PathBuf::from(root),
        SkillSourceKind::Github {
            active_checkout: Some(root),
            ..
        } => PathBuf::from(root),
        SkillSourceKind::Github { .. } => {
            return Err(AppError::InvalidArgument {
                message: "GitHub source has no active checkout".into(),
            });
        }
    };
    let root_metadata =
        std::fs::symlink_metadata(&root).map_err(|error| AppError::InvalidArgument {
            message: format!(
                "could not inspect skill source root {}: {error}",
                root.display()
            ),
        })?;
    if root_metadata.file_type().is_symlink() || metadata_is_reparse_point(&root_metadata) {
        return Ok(SkillSourceResult {
            source,
            packages: Vec::new(),
            errors: vec![unsafe_entry_error(
                ".".into(),
                "The registered source root is a link or reparse point. Register its real directory instead.",
            )],
        });
    }
    let canonical_root =
        std::fs::canonicalize(&root).map_err(|error| AppError::InvalidArgument {
            message: format!(
                "could not resolve skill source root {}: {error}",
                root.display()
            ),
        })?;

    let mut package_roots = Vec::new();
    let mut errors = Vec::new();
    let mut directories = VecDeque::from([canonical_root.clone()]);

    while let Some(directory) = directories.pop_front() {
        let directory_metadata =
            std::fs::symlink_metadata(&directory).map_err(|error| AppError::Io {
                message: format!("inspect {} before descent: {error}", directory.display()),
            })?;
        if directory_metadata.file_type().is_symlink()
            || metadata_is_reparse_point(&directory_metadata)
        {
            errors.push(unsafe_entry_error(
                relative_path(&canonical_root, &directory),
                "Links and reparse points are not allowed in skill sources. Remove the link and refresh.",
            ));
            continue;
        }
        let mut entries = read_directory_sorted(&directory)?;
        for (path, metadata) in entries.drain(..) {
            let relative = relative_path(&canonical_root, &path);
            if metadata.file_type().is_symlink() {
                errors.push(unsafe_entry_error(
                    relative,
                    "Symbolic links are not allowed in skill sources. Remove the link and refresh.",
                ));
                continue;
            }
            if metadata_is_reparse_point(&metadata) {
                errors.push(unsafe_entry_error(
                    relative,
                    "Windows reparse points are not allowed in skill sources. Remove the link and refresh.",
                ));
                continue;
            }
            if metadata.is_dir() {
                directories.push_back(path);
            } else if metadata.is_file()
                && path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md")
            {
                package_roots.push(directory.clone());
            } else if !metadata.is_file() {
                errors.push(unsafe_entry_error(
                    relative,
                    "Special filesystem entries are not allowed in skill sources. Remove the entry and refresh.",
                ));
            }
        }
    }

    package_roots.sort();
    package_roots.dedup();
    errors.sort_by(|left, right| left.path.cmp(&right.path));
    let mut packages = Vec::with_capacity(package_roots.len());
    for package_root in package_roots {
        let relative = relative_path(&canonical_root, &package_root);
        let canonical_package = match std::fs::canonicalize(&package_root) {
            Ok(path) if path.starts_with(&canonical_root) => path,
            Ok(_) => {
                errors.push(unsafe_entry_error(
                    relative,
                    "Skill package resolves outside its registered source. Remove the link and refresh.",
                ));
                continue;
            }
            Err(error) => {
                errors.push(SkillValidationError {
                    code: SkillValidationCode::Io,
                    path: relative,
                    message: format!("Could not resolve skill package: {error}"),
                });
                continue;
            }
        };
        packages.push(validate_package(
            &source.id,
            &canonical_root,
            &canonical_package,
        ));
    }
    packages.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    errors.sort_by(|left, right| left.path.cmp(&right.path));

    Ok(SkillSourceResult {
        source,
        packages,
        errors,
    })
}

fn read_directory_sorted(directory: &Path) -> Result<Vec<(PathBuf, Metadata)>, AppError> {
    let mut entries = std::fs::read_dir(directory)
        .map_err(|error| AppError::Io {
            message: format!("read directory {}: {error}", directory.display()),
        })?
        .map(|entry| {
            let entry = entry.map_err(|error| AppError::Io {
                message: format!("read entry in {}: {error}", directory.display()),
            })?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).map_err(|error| AppError::Io {
                message: format!("inspect {}: {error}", path.display()),
            })?;
            Ok((path, metadata))
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(entries)
}

fn validate_package(
    source_id: &str,
    source_root: &Path,
    package_root: &Path,
) -> SkillPackageResult {
    let relative = relative_path(source_root, package_root);
    let mut result = SkillPackageResult {
        source_id: source_id.into(),
        relative_path: relative,
        name: None,
        description: None,
        files: Vec::new(),
        errors: Vec::new(),
        installable: false,
    };

    match inventory_package(package_root) {
        Ok(files) => result.files = files,
        Err(error) => result.errors.push(error),
    }

    match read_bounded(&package_root.join("SKILL.md"), MAX_SKILL_FILE_BYTES) {
        Ok(bytes) => match parse_skill_metadata(&bytes) {
            Ok(metadata) => {
                result.name = Some(metadata.name.clone());
                result.description = Some(metadata.description);
                let directory_name = package_root.file_name().and_then(|name| name.to_str());
                if directory_name != Some(metadata.name.as_str()) {
                    result.errors.push(SkillValidationError {
                        code: SkillValidationCode::InvalidMetadata,
                        path: "SKILL.md".into(),
                        message: format!(
                            "Skill name '{}' must match directory '{}'.",
                            metadata.name,
                            directory_name.unwrap_or_default()
                        ),
                    });
                }
            }
            Err(message) => result.errors.push(SkillValidationError {
                code: SkillValidationCode::InvalidMetadata,
                path: "SKILL.md".into(),
                message,
            }),
        },
        Err(message) => result.errors.push(SkillValidationError {
            code: SkillValidationCode::Io,
            path: "SKILL.md".into(),
            message,
        }),
    }

    result.installable = result.errors.is_empty();
    result
}

fn inventory_package(package_root: &Path) -> Result<Vec<SkillPackageFile>, SkillValidationError> {
    let mut files = Vec::new();
    let mut total_bytes = 0_u64;
    let mut directories = VecDeque::from([package_root.to_path_buf()]);

    while let Some(directory) = directories.pop_front() {
        let entries = read_directory_sorted(&directory).map_err(|error| SkillValidationError {
            code: SkillValidationCode::Io,
            path: relative_path(package_root, &directory),
            message: error.to_string(),
        })?;
        for (path, metadata) in entries {
            let relative = relative_path(package_root, &path);
            if metadata.file_type().is_symlink() || metadata_is_reparse_point(&metadata) {
                return Err(unsafe_entry_error(
                    relative,
                    "Links and reparse points are not allowed in skill packages. Remove the entry and refresh.",
                ));
            }
            if metadata.is_dir() {
                directories.push_back(path);
                continue;
            }
            if !metadata.is_file() {
                return Err(unsafe_entry_error(
                    relative,
                    "Special filesystem entries are not allowed in skill packages. Remove the entry and refresh.",
                ));
            }
            if files.len() == MAX_SKILL_FILES {
                return Err(SkillValidationError {
                    code: SkillValidationCode::UnsafeEntry,
                    path: relative,
                    message: format!("Skill package exceeds the {MAX_SKILL_FILES}-file limit."),
                });
            }
            let bytes = read_bounded(&path, MAX_SKILL_FILE_BYTES).map_err(|message| {
                SkillValidationError {
                    code: SkillValidationCode::UnsafeEntry,
                    path: relative.clone(),
                    message,
                }
            })?;
            total_bytes += bytes.len() as u64;
            if total_bytes > MAX_SKILL_TOTAL_BYTES {
                return Err(SkillValidationError {
                    code: SkillValidationCode::UnsafeEntry,
                    path: relative,
                    message: format!(
                        "Skill package exceeds the {}-byte total limit.",
                        MAX_SKILL_TOTAL_BYTES
                    ),
                });
            }
            files.push(SkillPackageFile {
                relative_path: relative,
                size_bytes: bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(&bytes)),
            });
        }
    }
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(files)
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|error| format!("Could not open {}: {error}", path.display()))?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    if bytes.len() as u64 > limit {
        return Err(format!(
            "{} exceeds the {limit}-byte file limit.",
            path.display()
        ));
    }
    Ok(bytes)
}

#[derive(Deserialize)]
struct SkillMetadata {
    name: String,
    description: String,
}

fn parse_skill_metadata(bytes: &[u8]) -> Result<SkillMetadata, String> {
    let source =
        std::str::from_utf8(bytes).map_err(|error| format!("SKILL.md must be UTF-8: {error}"))?;
    let rest = source
        .strip_prefix("---\n")
        .ok_or_else(|| "SKILL.md must start with YAML frontmatter.".to_string())?;
    let end = rest
        .find("\n---")
        .ok_or_else(|| "SKILL.md frontmatter must end with '---'.".to_string())?;
    let yaml = &rest[..end];
    let metadata: SkillMetadata = serde_yaml::from_str(&yaml)
        .map_err(|error| format!("SKILL.md frontmatter is invalid: {error}"))?;
    if metadata.name.trim().is_empty() || metadata.description.trim().is_empty() {
        return Err("SKILL.md name and description must be non-empty.".into());
    }
    Ok(metadata)
}

fn unsafe_entry_error(path: String, message: &str) -> SkillValidationError {
    SkillValidationError {
        code: SkillValidationCode::UnsafeEntry,
        path,
        message: message.into(),
    }
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg_attr(not(any(test, windows)), allow(dead_code))]
pub(crate) fn is_windows_reparse_point(attributes: u32) -> bool {
    attributes & 0x400 != 0
}

#[cfg(windows)]
fn metadata_is_reparse_point(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    is_windows_reparse_point(metadata.file_attributes())
}

#[cfg(not(windows))]
fn metadata_is_reparse_point(_: &Metadata) -> bool {
    false
}

#[tauri::command]
pub async fn skill_sources_list(state: State<'_, AppState>) -> Result<Vec<SkillSource>, AppError> {
    load_skill_sources(&state.app_data_dir).await
}

#[tauri::command]
pub async fn skill_source_add_local(
    state: State<'_, AppState>,
    root: String,
) -> Result<SkillSource, AppError> {
    add_local_source(&state, Path::new(&root)).await
}

#[tauri::command]
pub async fn skill_source_refresh(
    state: State<'_, AppState>,
    source_id: String,
) -> Result<SkillSourceResult, AppError> {
    let source = load_skill_sources(&state.app_data_dir)
        .await?
        .into_iter()
        .find(|source| source.id == source_id)
        .ok_or_else(|| AppError::InvalidArgument {
            message: format!("unknown skill source id: {source_id}"),
        })?;
    discover_source(source).await
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use serde_json::json;
    use tempfile::tempdir;

    use super::{
        add_local_source, discover_source, is_windows_reparse_point, load_skill_sources,
        skill_sources_path,
    };
    use crate::error::AppError;
    use crate::state::AppState;
    use crate::types::{SkillSourceKind, SkillValidationCode};

    fn test_state(app_data_dir: &Path) -> AppState {
        let mut state = AppState::build().expect("build app state");
        state.app_data_dir = app_data_dir.to_path_buf();
        state
    }

    fn write_skill(root: &Path, relative_dir: &str, name: &str, description: &str) {
        let package = root.join(relative_dir);
        std::fs::create_dir_all(&package).expect("create package");
        std::fs::write(
            package.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n"),
        )
        .expect("write SKILL.md");
    }

    #[test]
    fn github_source_kind_serializes_camel_case_variant_fields() {
        let kind = SkillSourceKind::Github {
            repository: "owner/repo".into(),
            git_ref: Some("v1.0.0".into()),
            subdirectory: Some("skills".into()),
            active_checkout: Some("/tmp/checkout".into()),
        };

        assert_eq!(
            serde_json::to_value(kind).expect("serialize"),
            json!({
                "kind": "github",
                "repository": "owner/repo",
                "gitRef": "v1.0.0",
                "subdirectory": "skills",
                "activeCheckout": "/tmp/checkout"
            })
        );
    }

    #[tokio::test]
    async fn local_source_tracer() {
        let app = tempdir().expect("app data");
        let source = tempdir().expect("source");
        write_skill(source.path(), "nested/example", "example", "Example skill");
        std::fs::write(
            source.path().join("nested/example/reference.md"),
            b"reference\n",
        )
        .expect("write reference");
        std::fs::write(source.path().join("nested/skill.md"), b"not exact").expect("write decoy");
        let state = test_state(app.path());

        let registered = add_local_source(&state, source.path())
            .await
            .expect("register source");
        let registered_again = add_local_source(&state, source.path())
            .await
            .expect("register source once");
        assert_eq!(registered_again.id, registered.id);
        let persisted = load_skill_sources(app.path())
            .await
            .expect("reload sources");
        assert_eq!(persisted, vec![registered.clone()]);

        let result = discover_source(registered).await.expect("refresh source");
        assert!(result.errors.is_empty());
        assert_eq!(result.packages.len(), 1);
        let package = &result.packages[0];
        assert_eq!(package.relative_path, "nested/example");
        assert_eq!(package.name.as_deref(), Some("example"));
        assert_eq!(package.description.as_deref(), Some("Example skill"));
        assert!(package.installable);
        assert!(package.errors.is_empty());
        assert_eq!(
            package
                .files
                .iter()
                .map(|file| file.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec!["SKILL.md", "reference.md"]
        );
        assert!(package.files.iter().all(|file| file.sha256.len() == 64));

        let reloaded = load_skill_sources(app.path())
            .await
            .expect("reload sources again");
        assert_eq!(reloaded[0].id, result.source.id);
        assert!(skill_sources_path(app.path()).exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn discovery_rejects_symlinked_ancestor_outside_source() {
        use std::os::unix::fs::symlink;

        let app = tempdir().expect("app data");
        let source = tempdir().expect("source");
        let external = tempdir().expect("external");
        write_skill(
            external.path(),
            "escaped",
            "escaped",
            "Must not be discovered",
        );
        symlink(external.path(), source.path().join("linked")).expect("create symlink");
        let state = test_state(app.path());
        let registered = add_local_source(&state, source.path())
            .await
            .expect("register source");

        let result = discover_source(registered).await.expect("refresh source");

        assert!(result.packages.is_empty());
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].code, SkillValidationCode::UnsafeEntry);
        assert_eq!(result.errors[0].path, "linked");
        assert!(result.errors[0].message.contains("Remove the link"));
    }

    #[test]
    fn windows_reparse_attribute_fails_closed() {
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

        assert!(!is_windows_reparse_point(0));
        assert!(is_windows_reparse_point(FILE_ATTRIBUTE_REPARSE_POINT));
        assert!(is_windows_reparse_point(
            FILE_ATTRIBUTE_REPARSE_POINT | 0x10
        ));
    }

    #[tokio::test]
    async fn invalid_local_roots_preserve_state() {
        let app = tempdir().expect("app data");
        let valid = tempdir().expect("valid source");
        let state = test_state(app.path());
        add_local_source(&state, valid.path())
            .await
            .expect("seed valid source");
        let state_path = skill_sources_path(app.path());
        let before = std::fs::read(&state_path).expect("read initial state");

        let relative = add_local_source(&state, Path::new("relative")).await;
        let missing = add_local_source(&state, &app.path().join("missing")).await;
        let file = app.path().join("file");
        std::fs::write(&file, b"x").expect("write file");
        let not_directory = add_local_source(&state, &file).await;

        for result in [relative, missing, not_directory] {
            assert!(matches!(result, Err(AppError::InvalidArgument { .. })));
        }
        assert_eq!(
            std::fs::read(&state_path).expect("read preserved state"),
            before
        );
    }

    #[tokio::test]
    async fn concurrent_local_registration_preserves_both_sources() {
        let app = tempdir().expect("app data");
        let first = tempdir().expect("first source");
        let second = tempdir().expect("second source");
        let state = Arc::new(test_state(app.path()));

        let first_task = {
            let state = Arc::clone(&state);
            let root = first.path().to_path_buf();
            tokio::spawn(async move { add_local_source(&state, &root).await })
        };
        let second_task = {
            let state = Arc::clone(&state);
            let root = second.path().to_path_buf();
            tokio::spawn(async move { add_local_source(&state, &root).await })
        };

        let first_source = first_task.await.expect("first join").expect("first add");
        let second_source = second_task.await.expect("second join").expect("second add");
        let persisted = load_skill_sources(app.path()).await.expect("load sources");

        assert_eq!(persisted.len(), 2);
        assert!(persisted.iter().any(|source| source.id == first_source.id));
        assert!(persisted.iter().any(|source| source.id == second_source.id));
    }
}
