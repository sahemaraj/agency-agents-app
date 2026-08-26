use std::collections::HashSet;
use std::ffi::OsString;

use serde::Serialize;

use crate::error::AppError;

const MAX_PROJECT_ROOT_ENTRIES: usize = 256;
const STACK_MANIFESTS: [(&str, &str); 9] = [
    ("package.json", "typescript"),
    ("Cargo.toml", "rust"),
    ("pyproject.toml", "python"),
    ("requirements.txt", "python"),
    ("go.mod", "go"),
    ("pom.xml", "java"),
    ("build.gradle", "java"),
    ("Gemfile", "ruby"),
    ("tsconfig.json", "typescript"),
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectStackEvidence {
    pub file: String,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectStackDetection {
    pub languages: Vec<String>,
    pub evidence: Vec<ProjectStackEvidence>,
}

fn detect_stack_from_names(names: impl IntoIterator<Item = OsString>) -> ProjectStackDetection {
    let names = names
        .into_iter()
        .take(MAX_PROJECT_ROOT_ENTRIES)
        .filter_map(|name| name.into_string().ok())
        .collect::<HashSet<_>>();
    let mut languages = Vec::new();
    let mut evidence = Vec::new();
    for (file, token) in STACK_MANIFESTS {
        if names.contains(file) {
            if !languages.iter().any(|language| language == token) {
                languages.push(token.into());
            }
            evidence.push(ProjectStackEvidence {
                file: file.into(),
                token: token.into(),
            });
        }
    }
    ProjectStackDetection {
        languages,
        evidence,
    }
}

pub(crate) fn detect_project_stack(project_path: &str) -> Result<ProjectStackDetection, AppError> {
    let root = std::fs::canonicalize(project_path).map_err(|error| AppError::Io {
        message: format!("open recommendation project: {error}"),
    })?;
    if !root.is_dir() {
        return Err(AppError::InvalidArgument {
            message: "recommendation project must be a directory".into(),
        });
    }
    let entries = std::fs::read_dir(&root).map_err(|error| AppError::Io {
        message: format!("read recommendation project: {error}"),
    })?;
    Ok(detect_stack_from_names(
        entries
            .take(MAX_PROJECT_ROOT_ENTRIES)
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .map(|entry| entry.file_name()),
    ))
}

#[tauri::command]
pub fn project_detect_stack(project_path: String) -> Result<ProjectStackDetection, AppError> {
    detect_project_stack(&project_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_every_supported_root_manifest() {
        for (file, token) in STACK_MANIFESTS {
            let project = tempfile::tempdir().unwrap();
            std::fs::write(project.path().join(file), "").unwrap();
            assert_eq!(
                detect_project_stack(project.path().to_str().unwrap()).unwrap(),
                ProjectStackDetection {
                    languages: vec![token.into()],
                    evidence: vec![ProjectStackEvidence {
                        file: file.into(),
                        token: token.into(),
                    }],
                },
                "failed to detect {file}",
            );
        }
    }

    #[test]
    fn bounds_root_directory_entries() {
        let names = (0..MAX_PROJECT_ROOT_ENTRIES)
            .map(|index| OsString::from(format!("file-{index}")))
            .chain([OsString::from("Cargo.toml")]);
        assert_eq!(
            detect_stack_from_names(names),
            ProjectStackDetection {
                languages: Vec::new(),
                evidence: Vec::new(),
            }
        );
    }

    #[test]
    fn empty_directory_has_no_stack_evidence() {
        let project = tempfile::tempdir().unwrap();
        assert_eq!(
            detect_project_stack(project.path().to_str().unwrap()).unwrap(),
            ProjectStackDetection {
                languages: Vec::new(),
                evidence: Vec::new(),
            }
        );
    }

    #[test]
    fn directories_named_like_manifests_are_not_evidence() {
        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir(project.path().join("Cargo.toml")).unwrap();
        assert!(detect_project_stack(project.path().to_str().unwrap())
            .unwrap()
            .evidence
            .is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_named_like_manifests_are_not_evidence() {
        use std::os::unix::fs::symlink;

        let project = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("real.toml"), "").unwrap();
        symlink("real.toml", project.path().join("Cargo.toml")).unwrap();
        assert!(detect_project_stack(project.path().to_str().unwrap())
            .unwrap()
            .evidence
            .is_empty());
    }

    #[test]
    fn evidence_preserves_each_manifest_and_deduplicates_languages() {
        let project = tempfile::tempdir().unwrap();
        for file in ["package.json", "tsconfig.json", "Cargo.toml"] {
            std::fs::write(project.path().join(file), "").unwrap();
        }
        assert_eq!(
            detect_project_stack(project.path().to_str().unwrap()).unwrap(),
            ProjectStackDetection {
                languages: vec!["typescript".into(), "rust".into()],
                evidence: vec![
                    ProjectStackEvidence {
                        file: "package.json".into(),
                        token: "typescript".into(),
                    },
                    ProjectStackEvidence {
                        file: "Cargo.toml".into(),
                        token: "rust".into(),
                    },
                    ProjectStackEvidence {
                        file: "tsconfig.json".into(),
                        token: "typescript".into(),
                    },
                ],
            }
        );
    }
}
