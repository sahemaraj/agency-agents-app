use std::path::{Component, Path};

use base64::Engine as _;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

use crate::error::AppError;

pub(crate) const MAX_LIBRARY_FOLDERS: usize = 256;
pub(crate) const MAX_LIBRARY_FOLDER_DEPTH: usize = 8;
pub(crate) const MAX_LIBRARY_FOLDER_SEGMENT_CHARS: usize = 64;
pub(crate) const MAX_LIBRARY_RELATIVE_PATH_BYTES: usize = 1024;

fn invalid(message: impl Into<String>) -> AppError {
    AppError::InvalidArgument {
        message: message.into(),
    }
}

pub(crate) fn verify_publisher_signature(
    publisher_name: &str,
    public_key: &str,
    signature: &str,
    signed_parts: &[&[u8]],
) -> bool {
    if publisher_name.trim().is_empty() || publisher_name.chars().count() > 128 {
        return false;
    }
    let Ok(public_key) = base64::engine::general_purpose::STANDARD.decode(public_key) else {
        return false;
    };
    let Ok(signature) = base64::engine::general_purpose::STANDARD.decode(signature) else {
        return false;
    };
    let (Ok(public_key), Ok(signature)) = (
        <[u8; 32]>::try_from(public_key),
        <[u8; 64]>::try_from(signature),
    ) else {
        return false;
    };
    let Ok(key) = VerifyingKey::from_bytes(&public_key) else {
        return false;
    };
    let mut hasher = Sha256::new();
    hasher.update(publisher_name.as_bytes());
    for part in signed_parts {
        hasher.update([0]);
        hasher.update(part);
    }
    key.verify(&hasher.finalize(), &Signature::from_bytes(&signature))
        .is_ok()
}

fn portable_segment(segment: &str) -> bool {
    if segment.len() > 255
        || segment.ends_with(['.', ' '])
        || segment.chars().any(|value| {
            value.is_control() || matches!(value, '<' | '>' | ':' | '"' | '|' | '?' | '*')
        })
    {
        return false;
    }
    let stem = segment
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        && !matches!(
            stem.strip_prefix("COM"),
            Some("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        )
        && !matches!(
            stem.strip_prefix("LPT"),
            Some("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        )
}

pub(crate) fn normalize_relative_path(value: &str) -> Result<String, AppError> {
    if value.is_empty()
        || value.len() > MAX_LIBRARY_RELATIVE_PATH_BYTES
        || value.starts_with('/')
        || value.contains(['\\', '\0'])
        || value.split('/').any(|segment| {
            segment.is_empty() || matches!(segment, "." | "..") || !portable_segment(segment)
        })
    {
        return Err(invalid(format!(
            "library path must be normalized, relative, and slash-separated: {value}"
        )));
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(invalid(format!(
            "library path must be normalized, relative, and slash-separated: {value}"
        )));
    }
    Ok(value.to_owned())
}

pub(crate) fn portable_path_key(value: &str) -> Result<String, AppError> {
    Ok(normalize_relative_path(value)?.to_lowercase())
}

pub(crate) fn validate_reference(source_id: &str, relative_path: &str) -> Result<(), AppError> {
    if source_id.trim() != source_id
        || source_id.is_empty()
        || source_id.len() > 128
        || source_id.chars().any(char::is_control)
    {
        return Err(invalid("library source identity is invalid"));
    }
    normalize_relative_path(relative_path)?;
    Ok(())
}

pub(crate) fn validate_folder_segment(value: &str) -> Result<(), AppError> {
    let chars = value.chars().count();
    if value.trim() != value
        || chars == 0
        || chars > MAX_LIBRARY_FOLDER_SEGMENT_CHARS
        || value.contains(['/', '\\'])
        || value.chars().any(char::is_control)
    {
        return Err(invalid(
            "folder segments must contain 1-64 non-control characters without separators or surrounding whitespace",
        ));
    }
    Ok(())
}

pub(crate) fn validate_folder_path(value: &str) -> Result<(), AppError> {
    let segments = value.split('/').collect::<Vec<_>>();
    if segments.is_empty() || segments.len() > MAX_LIBRARY_FOLDER_DEPTH {
        return Err(invalid(format!(
            "folder paths must contain 1-{MAX_LIBRARY_FOLDER_DEPTH} segments"
        )));
    }
    segments.into_iter().try_for_each(validate_folder_segment)
}

pub(crate) fn create_folder(folders: &mut Vec<String>, path: String) -> Result<(), AppError> {
    validate_folder_path(&path)?;
    if folders
        .iter()
        .any(|folder| folder.eq_ignore_ascii_case(&path))
    {
        return Err(invalid(format!("folder already exists: {path}")));
    }
    if let Some((parent, _)) = path.rsplit_once('/') {
        if !folders.iter().any(|folder| folder == parent) {
            return Err(invalid(format!("parent folder does not exist: {parent}")));
        }
    }
    if folders.len() >= MAX_LIBRARY_FOLDERS {
        return Err(invalid(format!(
            "at most {MAX_LIBRARY_FOLDERS} folders are allowed"
        )));
    }
    folders.push(path);
    folders.sort();
    Ok(())
}

fn replace_prefix(value: &str, from: &str, to: &str) -> Option<String> {
    if value == from {
        Some(to.to_owned())
    } else {
        value
            .strip_prefix(&format!("{from}/"))
            .map(|suffix| format!("{to}/{suffix}"))
    }
}

pub(crate) fn rewrite_folder_paths(
    folders: &[String],
    path: &str,
    destination: String,
) -> Result<Vec<(String, String)>, AppError> {
    validate_folder_path(path)?;
    validate_folder_path(&destination)?;
    if !folders.iter().any(|folder| folder == path) {
        return Err(invalid(format!("folder does not exist: {path}")));
    }
    if destination == path || destination.starts_with(&format!("{path}/")) {
        return Err(invalid("a folder cannot be moved into itself"));
    }

    let rewrites = folders
        .iter()
        .filter_map(|folder| {
            replace_prefix(folder, path, &destination).map(|updated| (folder.clone(), updated))
        })
        .collect::<Vec<_>>();
    let unaffected = folders
        .iter()
        .filter(|folder| !rewrites.iter().any(|(current, _)| current == *folder));
    if rewrites.iter().enumerate().any(|(index, (_, updated))| {
        unaffected
            .clone()
            .any(|folder| folder.eq_ignore_ascii_case(updated))
            || rewrites[index + 1..]
                .iter()
                .any(|(_, other)| other.eq_ignore_ascii_case(updated))
    }) {
        return Err(invalid(format!("folder already exists: {destination}")));
    }
    Ok(rewrites)
}

pub(crate) fn rename_folder_paths(
    folders: &[String],
    path: &str,
    new_name: &str,
) -> Result<Vec<(String, String)>, AppError> {
    validate_folder_segment(new_name)?;
    let destination = path
        .rsplit_once('/')
        .map(|(parent, _)| format!("{parent}/{new_name}"))
        .unwrap_or_else(|| new_name.to_owned());
    rewrite_folder_paths(folders, path, destination)
}

pub(crate) fn move_folder_paths(
    folders: &[String],
    path: &str,
    new_parent: Option<&str>,
) -> Result<Vec<(String, String)>, AppError> {
    if let Some(parent) = new_parent {
        validate_folder_path(parent)?;
        if !folders.iter().any(|folder| folder == parent) {
            return Err(invalid(format!("parent folder does not exist: {parent}")));
        }
    }
    let name = path
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| invalid("folder path is empty"))?;
    let destination = new_parent
        .map(|parent| format!("{parent}/{name}"))
        .unwrap_or_else(|| name.to_owned());
    rewrite_folder_paths(folders, path, destination)
}

pub(crate) fn deleted_folder_paths(
    folders: &[String],
    path: &str,
    recursive: bool,
) -> Result<Vec<String>, AppError> {
    validate_folder_path(path)?;
    if !folders.iter().any(|folder| folder == path) {
        return Err(invalid(format!("folder does not exist: {path}")));
    }
    let prefix = format!("{path}/");
    let removed = folders
        .iter()
        .filter(|folder| *folder == path || folder.starts_with(&prefix))
        .cloned()
        .collect::<Vec<_>>();
    if removed.len() > 1 && !recursive {
        return Err(invalid(
            "folder is not empty; set recursive=true to remove descendants and assignments",
        ));
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_must_already_be_portable_and_normalized() {
        assert_eq!(
            normalize_relative_path("engineering/ui.md").unwrap(),
            "engineering/ui.md"
        );
        for invalid in [
            "",
            "/ui.md",
            "C:/ui.md",
            "../ui.md",
            "./ui.md",
            "engineering\\ui.md",
            "ui\0.md",
        ] {
            assert!(
                normalize_relative_path(invalid).is_err(),
                "accepted {invalid:?}"
            );
        }
        assert!(
            normalize_relative_path(&format!("{}/ui.md", vec!["nested"; 200].join("/"))).is_err()
        );
    }

    #[test]
    fn portable_keys_make_case_collisions_explicit() {
        assert_eq!(
            portable_path_key("Engineering/UI.md").unwrap(),
            "engineering/ui.md"
        );
        assert_eq!(
            portable_path_key("Engineering/UI.md").unwrap(),
            portable_path_key("engineering/ui.md").unwrap()
        );
    }

    #[test]
    fn references_require_a_source_and_normalized_path() {
        assert!(validate_reference("source", "engineering/ui.md").is_ok());
        assert!(validate_reference("", "engineering/ui.md").is_err());
        assert!(validate_reference("source", "../ui.md").is_err());
    }

    #[test]
    fn folder_mutations_return_deterministic_rewrites() {
        let mut folders = vec!["Engineering".into(), "Engineering/Frontend".into()];
        create_folder(&mut folders, "Engineering/Backend".into()).unwrap();
        assert_eq!(
            folders,
            ["Engineering", "Engineering/Backend", "Engineering/Frontend"]
        );
        assert!(create_folder(&mut folders, "engineering".into()).is_err());

        assert_eq!(
            rename_folder_paths(&folders, "Engineering", "Development").unwrap(),
            [
                ("Engineering".into(), "Development".into()),
                ("Engineering/Backend".into(), "Development/Backend".into()),
                ("Engineering/Frontend".into(), "Development/Frontend".into())
            ]
        );
        assert_eq!(
            move_folder_paths(&folders, "Engineering/Frontend", None).unwrap(),
            [("Engineering/Frontend".into(), "Frontend".into())]
        );
        assert!(deleted_folder_paths(&folders, "Engineering", false).is_err());
        assert_eq!(
            deleted_folder_paths(&folders, "Engineering", true).unwrap(),
            [
                "Engineering".to_string(),
                "Engineering/Backend".to_string(),
                "Engineering/Frontend".to_string()
            ]
        );
    }
}
