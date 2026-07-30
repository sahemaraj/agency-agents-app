use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::corpus::state_dir;
use crate::error::AppError;
use crate::types::{SkillInstallRecord, SkillInstallState, SkillPackageFile};
use crate::util::fs::atomic_write;

pub fn ledger_path(app_data_dir: &Path) -> PathBuf {
    state_dir(app_data_dir).join("skill-installs.json")
}

pub async fn load_ledger(app_data_dir: &Path) -> Result<Vec<SkillInstallRecord>, AppError> {
    let path = ledger_path(app_data_dir);
    match tokio::fs::read(&path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| AppError::JsonParse {
            command: "skill_installs_reconcile".into(),
            message: error.to_string(),
            raw_excerpt: String::from_utf8_lossy(&bytes[..bytes.len().min(256)]).into_owned(),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(AppError::Io {
            message: format!("read {}: {error}", path.display()),
        }),
    }
}

pub async fn save_ledger(
    app_data_dir: &Path,
    records: &[SkillInstallRecord],
) -> Result<(), AppError> {
    let directory = state_dir(app_data_dir);
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| AppError::Io {
            message: format!("create state directory {}: {error}", directory.display()),
        })?;
    let bytes = serde_json::to_vec_pretty(records).map_err(|error| AppError::Internal {
        message: format!("serialize skill-installs.json: {error}"),
    })?;
    atomic_write(&ledger_path(app_data_dir), &bytes).await
}

pub fn target_path(
    home: &Path,
    project: Option<&Path>,
    runtime: &str,
    name: &str,
) -> Result<PathBuf, AppError> {
    if name.is_empty()
        || !name.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
    {
        return Err(AppError::InvalidArgument {
            message: format!("invalid skill package name: {name}"),
        });
    }
    let base = project.unwrap_or(home);
    let relative = match runtime {
        "claudeCode" => ".claude/skills",
        "codex" => ".agents/skills",
        _ => {
            return Err(AppError::InvalidArgument {
                message: format!("unsupported skill runtime: {runtime}"),
            })
        }
    };
    Ok(base.join(relative).join(name))
}

pub fn project_target_path(runtime: &str, name: &str) -> Result<PathBuf, AppError> {
    target_path(Path::new(""), None, runtime, name)
}

fn validate_project_relative(path: &Path) -> Result<(), AppError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(AppError::InvalidArgument {
            message: format!(
                "project capability path must be a non-empty normalized relative path: {}",
                path.display()
            ),
        });
    }
    Ok(())
}

fn cap_io(action: &str, path: &Path, error: std::io::Error) -> AppError {
    AppError::Io {
        message: format!("{action} {}: {error}", path.display()),
    }
}

fn open_project_dir(root: &fs::File, relative: &Path, create: bool) -> Result<fs::File, AppError> {
    if relative.as_os_str().is_empty() {
        return root
            .try_clone()
            .map_err(|error| cap_io("clone project directory", relative, error));
    }
    validate_project_relative(relative)?;
    let mut directory = root
        .try_clone()
        .map_err(|error| cap_io("clone project directory", relative, error))?;
    for component in relative.components() {
        let Component::Normal(name) = component else {
            unreachable!("validated project-relative component")
        };
        match cap_primitives::fs::open_dir_nofollow(&directory, Path::new(name)) {
            Ok(next) => directory = next,
            Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
                match cap_primitives::fs::create_dir(
                    &directory,
                    Path::new(name),
                    &cap_primitives::fs::DirOptions::new(),
                ) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => {
                        return Err(cap_io("create project directory", relative, error));
                    }
                }
                directory = cap_primitives::fs::open_dir_nofollow(&directory, Path::new(name))
                    .map_err(|error| cap_io("open project directory", relative, error))?;
            }
            Err(error) => return Err(cap_io("open project directory", relative, error)),
        }
    }
    Ok(directory)
}

fn open_project_parent(
    root: &fs::File,
    relative: &Path,
    create: bool,
) -> Result<(fs::File, OsString), AppError> {
    validate_project_relative(relative)?;
    let parent = relative.parent().ok_or_else(|| AppError::InvalidArgument {
        message: "project capability path has no parent".into(),
    })?;
    let name = relative
        .file_name()
        .ok_or_else(|| AppError::InvalidArgument {
            message: "project capability path has no name".into(),
        })?
        .to_os_string();
    Ok((open_project_dir(root, parent, create)?, name))
}

pub struct ProjectDirectoryCapability {
    parent: fs::File,
    name: OsString,
    display: PathBuf,
}

pub fn project_directory_capability(
    root: &fs::File,
    relative: &Path,
) -> Result<ProjectDirectoryCapability, AppError> {
    let (parent, name) = open_project_parent(root, relative, false)?;
    Ok(ProjectDirectoryCapability {
        parent,
        name,
        display: relative.to_path_buf(),
    })
}

pub fn project_capability_tree_hash(
    capability: &ProjectDirectoryCapability,
) -> Result<Option<String>, AppError> {
    project_tree_hash(&capability.parent, Path::new(&capability.name))
}

pub fn rename_project_capability(
    capability: &mut ProjectDirectoryCapability,
    destination_name: OsString,
) -> Result<(), AppError> {
    if project_directory_present(&capability.parent, Path::new(&destination_name))? {
        return Err(AppError::InvalidArgument {
            message: format!(
                "project skill path is occupied: {}",
                destination_name.to_string_lossy()
            ),
        });
    }
    cap_primitives::fs::rename(
        &capability.parent,
        Path::new(&capability.name),
        &capability.parent,
        Path::new(&destination_name),
    )
    .map_err(|error| cap_io("rename project skill", &capability.display, error))?;
    capability.display.set_file_name(&destination_name);
    capability.name = destination_name;
    Ok(())
}

pub fn uninstall_project_capability(
    capability: &ProjectDirectoryCapability,
    backups: &Path,
    modified: bool,
) -> Result<Option<PathBuf>, AppError> {
    let backup = if modified {
        fs::create_dir_all(backups)
            .map_err(|error| cap_io("create skill backups", backups, error))?;
        let backup = backups.join(format!(
            "{}-{}",
            capability.name.to_string_lossy(),
            Uuid::new_v4()
        ));
        copy_project_tree_to_ambient(&capability.parent, Path::new(&capability.name), &backup)?;
        Some(backup)
    } else {
        None
    };
    remove_project_tree(&capability.parent, Path::new(&capability.name))?;
    Ok(backup)
}

fn project_directory_present(root: &fs::File, relative: &Path) -> Result<bool, AppError> {
    let (parent, name) = open_project_parent(root, relative, false)?;
    let metadata = match cap_primitives::fs::stat(
        &parent,
        Path::new(&name),
        cap_primitives::fs::FollowSymlinks::No,
    ) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(cap_io("inspect project skill directory", relative, error)),
    };
    if !metadata.is_dir() || metadata.is_symlink() {
        return Err(AppError::InvalidArgument {
            message: format!(
                "project skill path must be a real directory: {}",
                relative.display()
            ),
        });
    }
    cap_primitives::fs::open_dir_nofollow(&parent, Path::new(&name))
        .map_err(|error| cap_io("open project skill directory", relative, error))?;
    Ok(true)
}

fn project_entries(root: &fs::File, relative: &Path) -> Result<Vec<(PathBuf, Vec<u8>)>, AppError> {
    let (parent, name) = open_project_parent(root, relative, false)?;
    let start = cap_primitives::fs::open_dir_nofollow(&parent, Path::new(&name))
        .map_err(|error| cap_io("open project skill directory", relative, error))?;
    let mut pending = vec![(PathBuf::new(), start)];
    let mut files = Vec::new();
    while let Some((directory_relative, directory)) = pending.pop() {
        let mut children = cap_primitives::fs::read_base_dir(&directory)
            .map_err(|error| cap_io("read project skill directory", relative, error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| cap_io("read project skill entry", relative, error))?;
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            let name = child.file_name();
            let entry_relative = directory_relative.join(&name);
            let file_type = child
                .file_type()
                .map_err(|error| cap_io("inspect project skill entry", &entry_relative, error))?;
            if file_type.is_symlink() {
                return Err(AppError::InvalidArgument {
                    message: format!(
                        "project skill contains a linked entry: {}",
                        entry_relative.display()
                    ),
                });
            }
            if file_type.is_dir() {
                let child_dir = cap_primitives::fs::open_dir_nofollow(&directory, Path::new(&name))
                    .map_err(|error| {
                        cap_io("open project skill directory", &entry_relative, error)
                    })?;
                pending.push((entry_relative, child_dir));
            } else if file_type.is_file() {
                let mut options = cap_primitives::fs::OpenOptions::new();
                options
                    .read(true)
                    ._cap_fs_ext_follow(cap_primitives::fs::FollowSymlinks::No);
                let mut file = cap_primitives::fs::open(&directory, Path::new(&name), &options)
                    .map_err(|error| cap_io("open project skill file", &entry_relative, error))?;
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)
                    .map_err(|error| cap_io("read project skill file", &entry_relative, error))?;
                files.push((entry_relative, bytes));
            } else {
                return Err(AppError::InvalidArgument {
                    message: format!(
                        "project skill contains a special entry: {}",
                        entry_relative.display()
                    ),
                });
            }
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

pub fn project_tree_hash(root: &fs::File, relative: &Path) -> Result<Option<String>, AppError> {
    if !project_directory_present(root, relative)? {
        return Ok(None);
    }
    let mut digest = Sha256::new();
    for (path, bytes) in project_entries(root, relative)? {
        digest.update(path.to_string_lossy().as_bytes());
        digest.update([0]);
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(&bytes);
    }
    Ok(Some(format!("{:x}", digest.finalize())))
}

fn write_project_tree(
    root: &fs::File,
    relative: &Path,
    files: &[(String, Vec<u8>)],
) -> Result<(), AppError> {
    open_project_dir(root, relative, true)?;
    for (file_relative, bytes) in files {
        let full_relative = relative.join(file_relative);
        let parent = full_relative.parent().ok_or_else(|| AppError::Internal {
            message: "project skill staging file has no parent".into(),
        })?;
        let directory = open_project_dir(root, parent, true)?;
        let name = full_relative
            .file_name()
            .ok_or_else(|| AppError::Internal {
                message: "project skill staging file has no name".into(),
            })?;
        let mut options = cap_primitives::fs::OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            ._cap_fs_ext_follow(cap_primitives::fs::FollowSymlinks::No);
        let mut file = cap_primitives::fs::open(&directory, Path::new(name), &options)
            .map_err(|error| cap_io("create project skill file", &full_relative, error))?;
        file.write_all(bytes)
            .map_err(|error| cap_io("write project skill file", &full_relative, error))?;
    }
    Ok(())
}

fn copy_project_tree_to_ambient(
    root: &fs::File,
    relative: &Path,
    destination: &Path,
) -> Result<(), AppError> {
    fs::create_dir_all(destination)
        .map_err(|error| cap_io("create project skill backup directory", destination, error))?;
    for (path, bytes) in project_entries(root, relative)? {
        let target = destination.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| cap_io("create project skill backup directory", parent, error))?;
        }
        fs::write(&target, bytes)
            .map_err(|error| cap_io("write project skill backup", &target, error))?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn remove_project_tree(parent: &fs::File, name: &Path) -> Result<(), AppError> {
    cap_primitives::fs::remove_dir_all(parent, name)
        .map_err(|error| cap_io("remove project skill", name, error))
}

#[cfg(windows)]
fn remove_project_tree(_: &fs::File, name: &Path) -> Result<(), AppError> {
    // ponytail: cap-primitives 4 reopens an ambient path on Windows; retain
    // the quarantined tree until it offers handle-relative recursive delete.
    Err(AppError::Internal {
        message: format!(
            "project skill quarantine retained at {}; safe recursive deletion is unavailable on Windows",
            name.display()
        ),
    })
}

fn ensure_project_replacement_cleanup(
    had_destination: bool,
    safe_recursive_delete: bool,
) -> Result<(), AppError> {
    if had_destination && !safe_recursive_delete {
        return Err(AppError::InvalidArgument {
            message:
                "managed project skill replacement is unavailable because safe cleanup is unsupported"
                    .into(),
        });
    }
    Ok(())
}

pub fn install_validated_directory_in_project(
    root: &fs::File,
    source: &Path,
    files: &[SkillPackageFile],
    destination: &Path,
    backups: &Path,
    replace_managed: bool,
) -> Result<String, AppError> {
    validate_project_relative(destination)?;
    let verified = verified_inventory_files(source, files)?;
    let parent = destination
        .parent()
        .ok_or_else(|| AppError::InvalidArgument {
            message: "project skill destination has no parent".into(),
        })?;
    let parent_dir = open_project_dir(root, parent, true)?;
    let destination_name = destination
        .file_name()
        .ok_or_else(|| AppError::InvalidArgument {
            message: "project skill destination has no name".into(),
        })?;
    let had_destination = project_directory_present(&parent_dir, Path::new(destination_name))?;
    if had_destination && !replace_managed {
        return Err(AppError::InvalidArgument {
            message: format!(
                "project skill destination already exists and is not replaceable: {}",
                destination.display()
            ),
        });
    }
    ensure_project_replacement_cleanup(had_destination, cfg!(not(windows)))?;

    let transaction_id = Uuid::new_v4();
    let stage = PathBuf::from(format!(".agency-skill-{transaction_id}.stage"));
    let retired = PathBuf::from(format!(".agency-skill-{transaction_id}.previous"));
    write_project_tree(&parent_dir, &stage, &verified)?;
    let installed_hash =
        project_tree_hash(&parent_dir, &stage)?.ok_or_else(|| AppError::Internal {
            message: "project skill staging directory disappeared".into(),
        })?;

    let mut backup_path = None;
    if had_destination {
        fs::create_dir_all(backups)
            .map_err(|error| cap_io("create skill backups", backups, error))?;
        let backup = backups.join(format!(
            "{}-{transaction_id}",
            destination
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("skill")
        ));
        copy_project_tree_to_ambient(&parent_dir, Path::new(destination_name), &backup)?;
        backup_path = Some(backup);
        cap_primitives::fs::rename(
            &parent_dir,
            Path::new(destination_name),
            &parent_dir,
            &retired,
        )
        .map_err(|error| cap_io("stage existing project skill", destination, error))?;
    }
    if let Err(error) = cap_primitives::fs::rename(
        &parent_dir,
        &stage,
        &parent_dir,
        Path::new(destination_name),
    ) {
        let _ = remove_project_tree(&parent_dir, &stage);
        if had_destination {
            if let Err(restore) = cap_primitives::fs::rename(
                &parent_dir,
                &retired,
                &parent_dir,
                Path::new(destination_name),
            ) {
                return Err(AppError::Internal {
                    message: format!(
                        "publish project skill {} failed: {error}; restore failed: {restore}; recovery retained at {}{}",
                        destination.display(),
                        retired.display(),
                        backup_path
                            .map(|path| format!("; backup at {}", path.display()))
                            .unwrap_or_default(),
                    ),
                });
            }
        }
        return Err(cap_io("publish project skill", destination, error));
    }
    if had_destination {
        let _ = remove_project_tree(&parent_dir, &retired);
    }
    Ok(installed_hash)
}

pub fn rename_project_directory(
    root: &fs::File,
    source: &Path,
    destination: &Path,
) -> Result<(), AppError> {
    validate_project_relative(source)?;
    validate_project_relative(destination)?;
    let (source_parent, source_name) = open_project_parent(root, source, false)?;
    let (destination_parent, destination_name) = open_project_parent(root, destination, true)?;
    if project_directory_present(&destination_parent, Path::new(&destination_name))? {
        return Err(AppError::InvalidArgument {
            message: format!("project skill path is occupied: {}", destination.display()),
        });
    }
    cap_primitives::fs::rename(
        &source_parent,
        Path::new(&source_name),
        &destination_parent,
        Path::new(&destination_name),
    )
    .map_err(|error| cap_io("rename project skill", source, error))
}

pub fn uninstall_project_directory(
    root: &fs::File,
    destination: &Path,
    backups: &Path,
    modified: bool,
) -> Result<Option<PathBuf>, AppError> {
    let (parent, name) = open_project_parent(root, destination, false)?;
    if !project_directory_present(&parent, Path::new(&name))? {
        return Ok(None);
    }
    let backup = if modified {
        fs::create_dir_all(backups)
            .map_err(|error| cap_io("create skill backups", backups, error))?;
        let backup = backups.join(format!(
            "{}-{}",
            destination
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("skill"),
            Uuid::new_v4()
        ));
        copy_project_tree_to_ambient(&parent, Path::new(&name), &backup)?;
        Some(backup)
    } else {
        None
    };
    remove_project_tree(&parent, Path::new(&name))?;
    Ok(backup)
}

pub fn classify(
    disk_hash: Option<&str>,
    installed_hash: &str,
    source_available: bool,
    source_current: bool,
    disabled: bool,
) -> SkillInstallState {
    if !disabled && !source_available {
        return SkillInstallState::SourceUnavailable;
    }
    match disk_hash {
        None => SkillInstallState::Missing,
        Some(hash) if hash != installed_hash => SkillInstallState::Modified,
        Some(_) if disabled => SkillInstallState::Disabled,
        Some(_) if !source_available => SkillInstallState::SourceUnavailable,
        Some(_) if !source_current => SkillInstallState::Outdated,
        Some(_) => SkillInstallState::Current,
    }
}

fn entries(root: &Path) -> Result<Vec<PathBuf>, AppError> {
    let root_metadata = fs::symlink_metadata(root).map_err(|error| AppError::Io {
        message: format!("inspect skill directory {}: {error}", root.display()),
    })?;
    if !root_metadata.is_dir()
        || root_metadata.file_type().is_symlink()
        || super::metadata_is_reparse_point(&root_metadata)
    {
        return Err(AppError::InvalidArgument {
            message: format!(
                "skill directory must be a real directory: {}",
                root.display()
            ),
        });
    }
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        let mut children = fs::read_dir(&directory)
            .map_err(|error| AppError::Io {
                message: format!("read skill directory {}: {error}", directory.display()),
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| AppError::Io {
                message: format!("read skill directory entry: {error}"),
            })?;
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            let path = child.path();
            let metadata = fs::symlink_metadata(&path).map_err(|error| AppError::Io {
                message: format!("inspect skill entry {}: {error}", path.display()),
            })?;
            if metadata.file_type().is_symlink() {
                return Err(AppError::InvalidArgument {
                    message: format!("skill package contains a linked entry: {}", path.display()),
                });
            }
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file() {
                files.push(path);
            } else {
                return Err(AppError::InvalidArgument {
                    message: format!("skill package contains a special entry: {}", path.display()),
                });
            }
        }
    }
    files.sort();
    Ok(files)
}

pub fn tree_hash(root: &Path) -> Result<String, AppError> {
    let mut digest = Sha256::new();
    for path in entries(root)? {
        let relative = path.strip_prefix(root).map_err(|_| AppError::Internal {
            message: format!("skill entry escaped package root: {}", path.display()),
        })?;
        let bytes = fs::read(&path).map_err(|error| AppError::Io {
            message: format!("read skill entry {}: {error}", path.display()),
        })?;
        digest.update(relative.to_string_lossy().as_bytes());
        digest.update([0]);
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(&bytes);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), AppError> {
    fs::create_dir_all(destination).map_err(|error| AppError::Io {
        message: format!("create skill directory {}: {error}", destination.display()),
    })?;
    for path in entries(source)? {
        let relative = path.strip_prefix(source).map_err(|_| AppError::Internal {
            message: format!("skill entry escaped package root: {}", path.display()),
        })?;
        let target = destination.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| AppError::Io {
                message: format!("create skill directory {}: {error}", parent.display()),
            })?;
        }
        fs::copy(&path, &target).map_err(|error| AppError::Io {
            message: format!(
                "copy skill entry {} to {}: {error}",
                path.display(),
                target.display()
            ),
        })?;
    }
    Ok(())
}

fn verified_inventory_files(
    source: &Path,
    files: &[SkillPackageFile],
) -> Result<Vec<(String, Vec<u8>)>, AppError> {
    let source_metadata = fs::symlink_metadata(source).map_err(|error| AppError::Io {
        message: format!("inspect skill package {}: {error}", source.display()),
    })?;
    if !source_metadata.is_dir()
        || source_metadata.file_type().is_symlink()
        || super::metadata_is_reparse_point(&source_metadata)
    {
        return Err(AppError::InvalidArgument {
            message: "skill package root must be a real directory".into(),
        });
    }
    let source = fs::canonicalize(source).map_err(|error| AppError::Io {
        message: format!("resolve skill package {}: {error}", source.display()),
    })?;
    let mut seen = BTreeSet::new();
    let mut verified = Vec::with_capacity(files.len());
    for file in files {
        let relative_path = super::normalized_requested_file_path(&file.relative_path)?;
        if !seen.insert(relative_path.clone()) {
            return Err(AppError::InvalidArgument {
                message: format!("duplicate skill inventory entry: {relative_path}"),
            });
        }
        let path = source.join(&relative_path);
        let metadata = fs::symlink_metadata(&path).map_err(|error| AppError::Io {
            message: format!("inspect skill file {}: {error}", path.display()),
        })?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || super::metadata_is_reparse_point(&metadata)
        {
            return Err(AppError::InvalidArgument {
                message: format!("skill inventory entry is not a regular file: {relative_path}"),
            });
        }
        let path = fs::canonicalize(&path).map_err(|error| AppError::Io {
            message: format!("resolve skill file {}: {error}", path.display()),
        })?;
        if !path.starts_with(&source) {
            return Err(AppError::InvalidArgument {
                message: format!("skill inventory entry escaped its package: {relative_path}"),
            });
        }
        let bytes = super::read_bounded(&path, super::MAX_SKILL_FILE_BYTES).map_err(|error| {
            AppError::InvalidArgument {
                message: error.message(),
            }
        })?;
        if bytes.len() as u64 != file.size_bytes
            || format!("{:x}", Sha256::digest(&bytes)) != file.sha256
        {
            return Err(AppError::InvalidArgument {
                message: format!("skill inventory entry changed since validation: {relative_path}"),
            });
        }
        verified.push((relative_path, bytes));
    }
    verified.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(verified)
}

pub fn validated_tree_hash(source: &Path, files: &[SkillPackageFile]) -> Result<String, AppError> {
    let mut digest = Sha256::new();
    for (relative_path, bytes) in verified_inventory_files(source, files)? {
        digest.update(relative_path.as_bytes());
        digest.update([0]);
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(&bytes);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn publish_failure(
    error: std::io::Error,
    destination: &Path,
    retired: &Path,
    backup: Option<&Path>,
    had_destination: bool,
) -> AppError {
    if had_destination {
        if let Err(restore) = fs::rename(retired, destination) {
            return AppError::Internal {
                message: format!(
                    "publish skill {} failed: {error}; restore {} -> {} failed: {restore}; recovery retained at {}{}",
                    destination.display(),
                    retired.display(),
                    destination.display(),
                    retired.display(),
                    backup.map(|path| format!("; backup at {}", path.display())).unwrap_or_default(),
                ),
            };
        }
    }
    AppError::Io {
        message: format!("publish skill {}: {error}", destination.display()),
    }
}

pub fn install_validated_directory(
    source: &Path,
    files: &[SkillPackageFile],
    destination: &Path,
    backups: &Path,
    replace_managed: bool,
) -> Result<String, AppError> {
    let verified = verified_inventory_files(source, files)?;
    let parent = destination
        .parent()
        .ok_or_else(|| AppError::InvalidArgument {
            message: "skill destination has no parent directory".into(),
        })?;
    fs::create_dir_all(parent).map_err(|error| AppError::Io {
        message: format!(
            "create skill destination parent {}: {error}",
            parent.display()
        ),
    })?;
    if fs::symlink_metadata(destination).is_ok() && !replace_managed {
        return Err(AppError::InvalidArgument {
            message: format!(
                "skill destination already exists and is not replaceable: {}",
                destination.display()
            ),
        });
    }

    let transaction_id = Uuid::new_v4();
    let stage = parent.join(format!(".agency-skill-{transaction_id}.stage"));
    let retired = parent.join(format!(".agency-skill-{transaction_id}.previous"));
    fs::create_dir_all(&stage).map_err(|error| AppError::Io {
        message: format!(
            "create skill staging directory {}: {error}",
            stage.display()
        ),
    })?;
    for (relative_path, bytes) in verified {
        let target = stage.join(&relative_path);
        let target_parent = target.parent().ok_or_else(|| AppError::Internal {
            message: "skill staging file has no parent".into(),
        })?;
        fs::create_dir_all(target_parent).map_err(|error| AppError::Io {
            message: format!(
                "create skill staging directory {}: {error}",
                target_parent.display()
            ),
        })?;
        fs::write(&target, bytes).map_err(|error| AppError::Io {
            message: format!("write skill staging file {}: {error}", target.display()),
        })?;
    }
    let installed_hash = tree_hash(&stage)?;

    let had_destination = fs::symlink_metadata(destination).is_ok();
    let mut backup_path = None;
    if had_destination {
        fs::create_dir_all(backups).map_err(|error| AppError::Io {
            message: format!("create skill backups {}: {error}", backups.display()),
        })?;
        let backup = backups.join(format!(
            "{}-{transaction_id}",
            destination
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("skill")
        ));
        copy_tree(destination, &backup)?;
        backup_path = Some(backup);
        fs::rename(destination, &retired).map_err(|error| AppError::Io {
            message: format!("stage existing skill {}: {error}", destination.display()),
        })?;
    }
    if let Err(error) = fs::rename(&stage, destination) {
        let _ = fs::remove_dir_all(&stage);
        return Err(publish_failure(
            error,
            destination,
            &retired,
            backup_path.as_deref(),
            had_destination,
        ));
    }
    if had_destination {
        let _ = fs::remove_dir_all(retired);
    }
    Ok(installed_hash)
}

pub fn install_directory(
    source: &Path,
    destination: &Path,
    backups: &Path,
    replace_managed: bool,
) -> Result<String, AppError> {
    let source_metadata = fs::symlink_metadata(source).map_err(|error| AppError::Io {
        message: format!("inspect skill package {}: {error}", source.display()),
    })?;
    if !source_metadata.is_dir() || source_metadata.file_type().is_symlink() {
        return Err(AppError::InvalidArgument {
            message: "skill package root must be a real directory".into(),
        });
    }
    let parent = destination
        .parent()
        .ok_or_else(|| AppError::InvalidArgument {
            message: "skill destination has no parent directory".into(),
        })?;
    fs::create_dir_all(parent).map_err(|error| AppError::Io {
        message: format!(
            "create skill destination parent {}: {error}",
            parent.display()
        ),
    })?;
    if fs::symlink_metadata(destination).is_ok() && !replace_managed {
        return Err(AppError::InvalidArgument {
            message: format!(
                "skill destination already exists and is not replaceable: {}",
                destination.display()
            ),
        });
    }

    let transaction_id = Uuid::new_v4();
    let stage = parent.join(format!(".agency-skill-{transaction_id}.stage"));
    let retired = parent.join(format!(".agency-skill-{transaction_id}.previous"));
    copy_tree(source, &stage)?;
    let installed_hash = tree_hash(&stage)?;

    let had_destination = fs::symlink_metadata(destination).is_ok();
    let mut backup_path = None;
    if had_destination {
        fs::create_dir_all(backups).map_err(|error| AppError::Io {
            message: format!("create skill backups {}: {error}", backups.display()),
        })?;
        let backup = backups.join(format!(
            "{}-{transaction_id}",
            destination
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("skill")
        ));
        copy_tree(destination, &backup)?;
        backup_path = Some(backup);
        fs::rename(destination, &retired).map_err(|error| AppError::Io {
            message: format!("stage existing skill {}: {error}", destination.display()),
        })?;
    }

    if let Err(error) = fs::rename(&stage, destination) {
        let _ = fs::remove_dir_all(&stage);
        return Err(publish_failure(
            error,
            destination,
            &retired,
            backup_path.as_deref(),
            had_destination,
        ));
    }
    if had_destination {
        let _ = fs::remove_dir_all(retired);
    }
    Ok(installed_hash)
}

pub fn disable_directory(source: &Path, disabled: &Path) -> Result<(), AppError> {
    if fs::symlink_metadata(disabled).is_ok() {
        return Err(AppError::InvalidArgument {
            message: format!("disabled skill path is occupied: {}", disabled.display()),
        });
    }
    let parent = disabled.parent().ok_or_else(|| AppError::InvalidArgument {
        message: "disabled skill path has no parent".into(),
    })?;
    fs::create_dir_all(parent).map_err(|error| AppError::Io {
        message: format!(
            "create disabled skill directory {}: {error}",
            parent.display()
        ),
    })?;
    fs::rename(source, disabled).map_err(|error| AppError::Io {
        message: format!("disable skill {}: {error}", source.display()),
    })
}

pub fn enable_directory(disabled: &Path, destination: &Path) -> Result<(), AppError> {
    if fs::symlink_metadata(destination).is_ok() {
        return Err(AppError::InvalidArgument {
            message: format!("skill destination is occupied: {}", destination.display()),
        });
    }
    let parent = destination
        .parent()
        .ok_or_else(|| AppError::InvalidArgument {
            message: "skill destination has no parent".into(),
        })?;
    fs::create_dir_all(parent).map_err(|error| AppError::Io {
        message: format!(
            "create skill destination directory {}: {error}",
            parent.display()
        ),
    })?;
    fs::rename(disabled, destination).map_err(|error| AppError::Io {
        message: format!("enable skill {}: {error}", destination.display()),
    })
}

pub fn uninstall_directory(
    destination: &Path,
    backups: &Path,
    modified: bool,
) -> Result<Option<PathBuf>, AppError> {
    if fs::symlink_metadata(destination).is_err() {
        return Ok(None);
    }
    if modified {
        fs::create_dir_all(backups).map_err(|error| AppError::Io {
            message: format!("create skill backups {}: {error}", backups.display()),
        })?;
        let backup = backups.join(format!(
            "{}-{}",
            destination
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("skill"),
            Uuid::new_v4()
        ));
        copy_tree(destination, &backup)?;
        fs::remove_dir_all(destination).map_err(|error| AppError::Io {
            message: format!("remove backed-up skill {}: {error}", destination.display()),
        })?;
        return Ok(Some(backup));
    }
    fs::remove_dir_all(destination).map_err(|error| AppError::Io {
        message: format!("remove skill {}: {error}", destination.display()),
    })?;
    Ok(None)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use sha2::Digest;
    use tempfile::tempdir;

    use super::{
        classify, disable_directory, enable_directory, ensure_project_replacement_cleanup,
        install_directory, install_validated_directory, load_ledger, publish_failure, save_ledger,
        target_path, tree_hash, uninstall_directory,
    };
    use crate::types::{SkillInstallRecord, SkillInstallState, SkillPackageFile};

    fn write_package(root: &Path, body: &str) {
        std::fs::create_dir_all(root.join("references")).expect("create package");
        std::fs::write(
            root.join("SKILL.md"),
            format!("---\nname: reviewer\ndescription: Reviews changes\n---\n{body}"),
        )
        .expect("write skill");
        std::fs::write(root.join("references/checklist.md"), b"# Checklist\n")
            .expect("write reference");
    }

    #[test]
    fn install_publishes_the_exact_package_directory() {
        let source = tempdir().expect("source");
        let target_root = tempdir().expect("target");
        let backups = tempdir().expect("backups");
        write_package(source.path(), "# Review\n");
        let destination = target_root.path().join("reviewer");

        let installed_hash = install_directory(source.path(), &destination, backups.path(), false)
            .expect("install package");

        assert_eq!(
            std::fs::read(destination.join("SKILL.md")).expect("installed skill"),
            std::fs::read(source.path().join("SKILL.md")).expect("source skill")
        );
        assert_eq!(
            std::fs::read(destination.join("references/checklist.md"))
                .expect("installed reference"),
            b"# Checklist\n"
        );
        assert_eq!(
            installed_hash,
            tree_hash(&destination).expect("destination hash")
        );
    }

    #[test]
    fn validated_install_copies_only_verified_inventory_files() {
        let source = tempdir().expect("source");
        let destination = tempdir().expect("destination");
        let backups = tempdir().expect("backups");
        std::fs::write(source.path().join("SKILL.md"), b"# Skill\n").expect("skill file");
        std::fs::create_dir_all(source.path().join("references")).expect("references");
        std::fs::write(source.path().join("references/guide.md"), b"# Guide\n")
            .expect("guide file");
        std::fs::write(source.path().join("not-in-inventory.txt"), b"do not copy")
            .expect("unlisted file");
        let files = [
            ("SKILL.md", b"# Skill\n".as_slice()),
            ("references/guide.md", b"# Guide\n".as_slice()),
        ]
        .into_iter()
        .map(|(relative_path, bytes)| SkillPackageFile {
            relative_path: relative_path.into(),
            size_bytes: bytes.len() as u64,
            sha256: format!("{:x}", sha2::Sha256::digest(bytes)),
        })
        .collect::<Vec<_>>();

        install_validated_directory(
            source.path(),
            &files,
            &destination.path().join("reviewer"),
            backups.path(),
            false,
        )
        .expect("publish validated package");

        assert!(destination.path().join("reviewer/SKILL.md").is_file());
        assert!(destination
            .path()
            .join("reviewer/references/guide.md")
            .is_file());
        assert!(!destination
            .path()
            .join("reviewer/not-in-inventory.txt")
            .exists());

        std::fs::write(source.path().join("SKILL.md"), b"changed").expect("mutate source");
        assert!(install_validated_directory(
            source.path(),
            &files,
            &destination.path().join("changed"),
            backups.path(),
            false,
        )
        .is_err());
    }

    #[test]
    fn install_refuses_to_replace_an_unmanaged_destination() {
        let source = tempdir().expect("source");
        let target_root = tempdir().expect("target");
        let backups = tempdir().expect("backups");
        write_package(source.path(), "# Review\n");
        let destination = target_root.path().join("reviewer");
        std::fs::create_dir(&destination).expect("create foreign destination");
        std::fs::write(destination.join("LOCAL.md"), b"keep me").expect("write foreign content");

        let result = install_directory(source.path(), &destination, backups.path(), false);

        assert!(result.is_err());
        assert_eq!(
            std::fs::read(destination.join("LOCAL.md")).expect("foreign content preserved"),
            b"keep me"
        );
        assert_eq!(
            std::fs::read_dir(backups.path()).expect("backups").count(),
            0
        );
    }

    #[test]
    fn managed_replacement_is_backed_up_before_publication() {
        let source = tempdir().expect("source");
        let target_root = tempdir().expect("target");
        let backups = tempdir().expect("backups");
        write_package(source.path(), "# New\n");
        let destination = target_root.path().join("reviewer");
        std::fs::create_dir(&destination).expect("create managed destination");
        std::fs::write(destination.join("SKILL.md"), b"old bytes").expect("write old content");

        install_directory(source.path(), &destination, backups.path(), true)
            .expect("replace managed destination");

        let backup = std::fs::read_dir(backups.path())
            .expect("backups")
            .next()
            .expect("one backup")
            .expect("backup entry")
            .path();
        assert_eq!(
            std::fs::read(backup.join("SKILL.md")).expect("backed up bytes"),
            b"old bytes"
        );
        assert!(std::fs::read_to_string(destination.join("SKILL.md"))
            .expect("new skill")
            .contains("# New"));
    }

    #[cfg(unix)]
    #[test]
    fn install_rejects_linked_package_entries() {
        use std::os::unix::fs::symlink;

        let source = tempdir().expect("source");
        let outside = tempdir().expect("outside");
        let target_root = tempdir().expect("target");
        let backups = tempdir().expect("backups");
        write_package(source.path(), "# Review\n");
        std::fs::write(outside.path().join("secret"), b"secret").expect("outside file");
        symlink(
            outside.path().join("secret"),
            source.path().join("references/linked"),
        )
        .expect("create source link");

        let result = install_directory(
            source.path(),
            &target_root.path().join("reviewer"),
            backups.path(),
            false,
        );

        assert!(result.is_err());
        assert!(!target_root.path().join("reviewer").exists());
    }

    #[cfg(unix)]
    #[test]
    fn tree_hash_rejects_a_linked_root() {
        use std::os::unix::fs::symlink;

        let real = tempdir().expect("real package");
        let links = tempdir().expect("links");
        write_package(real.path(), "# Review\n");
        let linked = links.path().join("reviewer");
        symlink(real.path(), &linked).expect("link package root");

        assert!(tree_hash(&linked).is_err());
    }

    #[test]
    fn reconciliation_classifies_every_phase_three_state() {
        assert_eq!(
            classify(Some("same"), "same", true, true, false),
            SkillInstallState::Current
        );
        assert_eq!(
            classify(Some("same"), "same", true, false, false),
            SkillInstallState::Outdated
        );
        assert_eq!(
            classify(Some("edited"), "same", true, true, false),
            SkillInstallState::Modified
        );
        assert_eq!(
            classify(None, "same", true, true, false),
            SkillInstallState::Missing
        );
        assert_eq!(
            classify(Some("same"), "same", false, false, false),
            SkillInstallState::SourceUnavailable
        );
        assert_eq!(
            classify(None, "same", false, false, false),
            SkillInstallState::SourceUnavailable
        );
        assert_eq!(
            classify(Some("edited"), "same", false, false, false),
            SkillInstallState::SourceUnavailable
        );
        assert_eq!(
            classify(None, "same", true, true, true),
            SkillInstallState::Missing
        );
        assert_eq!(
            classify(Some("edited"), "same", true, true, true),
            SkillInstallState::Modified
        );
        assert_eq!(
            classify(Some("same"), "same", false, false, true),
            SkillInstallState::Disabled
        );
    }

    #[test]
    fn publish_failure_retains_the_recovery_and_backup_paths() {
        let error = publish_failure(
            std::io::Error::other("publish failed"),
            Path::new("/target/reviewer"),
            Path::new("/target/.previous"),
            Some(Path::new("/backups/reviewer")),
            true,
        );
        assert!(error.to_string().contains("publish failed"));
        assert!(error.to_string().contains("/target/.previous"));
        assert!(error.to_string().contains("/backups/reviewer"));
    }

    #[test]
    fn replacement_is_rejected_before_staging_without_safe_recursive_cleanup() {
        assert!(ensure_project_replacement_cleanup(true, false).is_err());
        assert!(ensure_project_replacement_cleanup(false, false).is_ok());
        assert!(ensure_project_replacement_cleanup(true, true).is_ok());
    }

    #[test]
    fn target_paths_cover_both_runtimes_and_scopes() {
        let home = Path::new("/home/dev");
        let project = Path::new("/work/app");

        assert_eq!(
            target_path(home, None, "claudeCode", "reviewer").expect("Claude user"),
            Path::new("/home/dev/.claude/skills/reviewer")
        );
        assert_eq!(
            target_path(home, None, "codex", "reviewer").expect("Codex user"),
            Path::new("/home/dev/.agents/skills/reviewer")
        );
        assert_eq!(
            target_path(home, Some(project), "claudeCode", "reviewer").expect("Claude project"),
            Path::new("/work/app/.claude/skills/reviewer")
        );
        assert_eq!(
            target_path(home, Some(project), "codex", "reviewer").expect("Codex project"),
            Path::new("/work/app/.agents/skills/reviewer")
        );
        assert!(target_path(home, None, "cursor", "reviewer").is_err());
    }

    #[tokio::test]
    async fn skill_ledger_round_trips_atomically() {
        let app = tempdir().expect("app data");
        let records = vec![SkillInstallRecord {
            source_id: "source-1".into(),
            relative_path: "skills/reviewer".into(),
            name: "reviewer".into(),
            runtime: "claudeCode".into(),
            scope: "user".into(),
            project_path: None,
            dest: "/home/dev/.claude/skills/reviewer".into(),
            source_hash: "source-hash".into(),
            installed_hash: "installed-hash".into(),
            installed_at: "2026-07-30T00:00:00Z".into(),
            disabled_path: None,
        }];

        save_ledger(app.path(), &records)
            .await
            .expect("save ledger");

        assert_eq!(load_ledger(app.path()).await.expect("load ledger"), records);
    }

    #[test]
    fn disable_and_enable_move_the_exact_directory_without_copying() {
        let root = tempdir().expect("root");
        let destination = root.path().join("active/reviewer");
        let disabled = root.path().join("disabled/reviewer");
        write_package(&destination, "# Review\n");
        let expected = tree_hash(&destination).expect("active hash");

        disable_directory(&destination, &disabled).expect("disable");
        assert!(!destination.exists());
        assert_eq!(tree_hash(&disabled).expect("disabled hash"), expected);

        enable_directory(&disabled, &destination).expect("enable");
        assert!(!disabled.exists());
        assert_eq!(tree_hash(&destination).expect("restored hash"), expected);
    }

    #[test]
    fn enable_refuses_to_overwrite_an_occupied_destination() {
        let root = tempdir().expect("root");
        let destination = root.path().join("active/reviewer");
        let disabled = root.path().join("disabled/reviewer");
        write_package(&disabled, "# Disabled\n");
        write_package(&destination, "# Foreign\n");

        assert!(enable_directory(&disabled, &destination).is_err());
        assert!(disabled.exists());
        assert!(std::fs::read_to_string(destination.join("SKILL.md"))
            .expect("foreign content")
            .contains("# Foreign"));
    }

    #[test]
    fn uninstall_backs_up_modified_content_before_removal() {
        let root = tempdir().expect("root");
        let destination = root.path().join("active/reviewer");
        let backups = root.path().join("backups");
        write_package(&destination, "# User edit\n");

        let backup = uninstall_directory(&destination, &backups, true)
            .expect("safe uninstall")
            .expect("backup path");

        assert!(!destination.exists());
        assert!(std::fs::read_to_string(backup.join("SKILL.md"))
            .expect("backup content")
            .contains("# User edit"));
    }
}
