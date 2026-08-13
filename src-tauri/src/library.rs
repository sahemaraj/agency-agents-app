use std::path::{Component, Path};

use base64::Engine as _;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::state::AppState;
use crate::types::{
    AgentPackageResult, AgentPreferredSource, AgentRecommendation, AgentSourceResult,
    SkillRecommendation, SkillSourceResult, TaskRecommendation,
};
use tauri::State;

pub(crate) const MAX_RECOMMEND_TASK_BYTES: usize = 2_048;
pub(crate) const MAX_RECOMMEND_LANGUAGES: usize = 32;
pub(crate) const MAX_RECOMMEND_LANGUAGE_BYTES: usize = 64;

pub(crate) const MAX_LIBRARY_FOLDERS: usize = 256;
pub(crate) const MAX_LIBRARY_FOLDER_DEPTH: usize = 8;
pub(crate) const MAX_LIBRARY_FOLDER_SEGMENT_CHARS: usize = 64;
pub(crate) const MAX_LIBRARY_RELATIVE_PATH_BYTES: usize = 1024;

fn invalid(message: impl Into<String>) -> AppError {
    AppError::InvalidArgument {
        message: message.into(),
    }
}

fn metadata_tokens(value: &str) -> std::collections::BTreeSet<String> {
    value
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

pub(crate) fn validate_recommend_request(task: &str, languages: &[String]) -> Result<(), AppError> {
    if task.len() > MAX_RECOMMEND_TASK_BYTES {
        return Err(invalid(format!(
            "task exceeds the {MAX_RECOMMEND_TASK_BYTES}-byte limit"
        )));
    }
    if languages.len() > MAX_RECOMMEND_LANGUAGES {
        return Err(invalid(format!(
            "languages exceeds the {MAX_RECOMMEND_LANGUAGES}-item limit"
        )));
    }
    if languages
        .iter()
        .any(|language| language.len() > MAX_RECOMMEND_LANGUAGE_BYTES)
    {
        return Err(invalid(format!(
            "language exceeds the {MAX_RECOMMEND_LANGUAGE_BYTES}-byte limit"
        )));
    }
    Ok(())
}

fn sanitized_agent_package(mut package: AgentPackageResult) -> AgentPackageResult {
    if let Some(agent) = &mut package.agent {
        agent.body.clear();
    }
    package
}

pub(crate) fn recommend_agents(
    results: &[AgentSourceResult],
    preferred: &[AgentPreferredSource],
    task: &str,
    limit: usize,
) -> Result<Vec<AgentRecommendation>, AppError> {
    validate_recommend_request(task, &[])?;
    let task_tokens = metadata_tokens(task);
    let mut recommendations = results
        .iter()
        .flat_map(|source| &source.agents)
        .filter(|package| package.installable)
        .filter_map(|package| {
            let agent = package.agent.as_ref()?;
            let name = metadata_tokens(&agent.name);
            let description = metadata_tokens(&agent.description);
            let taxonomy = std::iter::once(agent.category.as_str())
                .chain(package.groups.iter().map(String::as_str))
                .chain(package.tags.iter().map(String::as_str))
                .chain(package.capabilities.iter().map(String::as_str))
                .flat_map(metadata_tokens)
                .collect::<std::collections::BTreeSet<_>>();
            let mut score = 0;
            let mut reasons = Vec::new();
            for token in &task_tokens {
                if name.contains(token) {
                    score += 4;
                    reasons.push(format!("task:name:{token}"));
                } else if description.contains(token) {
                    score += 2;
                    reasons.push(format!("task:description:{token}"));
                } else if taxonomy.contains(token) {
                    score += 2;
                    reasons.push(format!("task:taxonomy:{token}"));
                }
            }
            if preferred.iter().any(|item| {
                item.agent_name.eq_ignore_ascii_case(&agent.name)
                    && item.source_id == package.reference.source_id
            }) {
                score += 1;
                reasons.push("preferred-source".into());
            }
            (score > 0).then(|| AgentRecommendation {
                package: sanitized_agent_package(package.clone()),
                score,
                reasons,
            })
        })
        .collect::<Vec<_>>();
    recommendations.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.package.reference.cmp(&right.package.reference))
    });
    recommendations.truncate(limit.clamp(1, 50));
    Ok(recommendations)
}

pub(crate) fn recommend_skills(
    results: &[SkillSourceResult],
    task: &str,
    languages: &[String],
    limit: usize,
) -> Result<Vec<SkillRecommendation>, AppError> {
    validate_recommend_request(task, languages)?;
    let task_tokens = metadata_tokens(task);
    let language_tokens = languages
        .iter()
        .flat_map(|language| metadata_tokens(language))
        .collect::<std::collections::BTreeSet<_>>();
    let mut recommendations = results
        .iter()
        .flat_map(|result| &result.packages)
        .filter(|package| package.installable)
        .filter_map(|package| {
            let name = metadata_tokens(package.name.as_deref().unwrap_or_default());
            let description = metadata_tokens(package.description.as_deref().unwrap_or_default());
            let taxonomy = std::iter::once(package.skill_type.as_str())
                .chain(package.group.iter().map(String::as_str))
                .chain(package.tags.iter().map(String::as_str))
                .flat_map(metadata_tokens)
                .collect::<std::collections::BTreeSet<_>>();
            let mut score = 0;
            let mut reasons = Vec::new();
            for token in &task_tokens {
                if name.contains(token) {
                    score += 4;
                    reasons.push(format!("task:name:{token}"));
                } else if description.contains(token) {
                    score += 2;
                    reasons.push(format!("task:description:{token}"));
                } else if taxonomy.contains(token) {
                    score += 2;
                    reasons.push(format!("task:taxonomy:{token}"));
                }
            }
            for token in &language_tokens {
                if name.contains(token) || description.contains(token) || taxonomy.contains(token) {
                    score += 3;
                    reasons.push(format!("language:{token}"));
                }
            }
            (score > 0).then(|| SkillRecommendation {
                package: package.clone(),
                score,
                reasons,
            })
        })
        .collect::<Vec<_>>();
    recommendations.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| {
                left.package
                    .name
                    .as_deref()
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .cmp(
                        &right
                            .package
                            .name
                            .as_deref()
                            .unwrap_or_default()
                            .to_ascii_lowercase(),
                    )
            })
            .then_with(|| left.package.name.cmp(&right.package.name))
            .then_with(|| left.package.source_id.cmp(&right.package.source_id))
            .then_with(|| left.package.relative_path.cmp(&right.package.relative_path))
    });
    recommendations.truncate(limit.clamp(1, 50));
    Ok(recommendations)
}

pub(crate) fn recommend_catalog(
    agents: &[AgentSourceResult],
    preferred: &[AgentPreferredSource],
    skills: &[SkillSourceResult],
    task: &str,
    languages: &[String],
    limit: usize,
) -> Result<Vec<TaskRecommendation>, AppError> {
    let mut combined = recommend_agents(agents, preferred, task, limit)?
        .into_iter()
        .map(|item| TaskRecommendation::Agent {
            package: item.package,
            score: item.score,
            reasons: item.reasons,
        })
        .chain(
            recommend_skills(skills, task, languages, limit)?
                .into_iter()
                .map(|item| TaskRecommendation::Skill {
                    package: item.package,
                    score: item.score,
                    reasons: item.reasons,
                }),
        )
        .collect::<Vec<_>>();
    combined.sort_by(|left, right| {
        right
            .score()
            .cmp(&left.score())
            .then_with(|| match (left, right) {
                (
                    TaskRecommendation::Agent { package: left, .. },
                    TaskRecommendation::Agent { package: right, .. },
                ) => left.reference.cmp(&right.reference),
                (
                    TaskRecommendation::Skill { package: left, .. },
                    TaskRecommendation::Skill { package: right, .. },
                ) => (&left.source_id, &left.relative_path)
                    .cmp(&(&right.source_id, &right.relative_path)),
                (TaskRecommendation::Agent { .. }, TaskRecommendation::Skill { .. }) => {
                    std::cmp::Ordering::Less
                }
                (TaskRecommendation::Skill { .. }, TaskRecommendation::Agent { .. }) => {
                    std::cmp::Ordering::Greater
                }
            })
    });
    combined.truncate(limit.clamp(1, 50));
    Ok(combined)
}

#[tauri::command]
pub async fn task_recommendations(
    state: State<'_, AppState>,
    task: String,
    limit: Option<usize>,
) -> Result<Vec<TaskRecommendation>, AppError> {
    validate_recommend_request(&task, &[])?;
    let agents = crate::agents::inspect_agent_sources(&state.app_data_dir).await?;
    let preferred = crate::agents::organize::list(&state)
        .await?
        .preferred_sources;
    let skills = crate::skills::inspect_skill_sources(&state).await?;
    recommend_catalog(
        &agents,
        &preferred,
        &skills,
        &task,
        &[],
        limit.unwrap_or(10),
    )
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

pub(crate) fn validate_source_id(source_id: &str) -> Result<(), AppError> {
    if source_id.trim() != source_id
        || source_id.is_empty()
        || source_id.len() > 128
        || source_id.chars().any(char::is_control)
    {
        return Err(invalid("library source identity is invalid"));
    }
    Ok(())
}

pub(crate) fn validate_reference(source_id: &str, relative_path: &str) -> Result<(), AppError> {
    validate_source_id(source_id)?;
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
    use crate::types::{
        Agent, AgentPackageResult, AgentReference, AgentSource, AgentSourceKind, AgentSourceResult,
        SkillPackageResult, SkillSource, SkillSourceKind, SkillSourceResult, SkillType,
        TaskRecommendation,
    };

    fn agent_package(source_id: &str, path: &str, installable: bool) -> AgentPackageResult {
        AgentPackageResult {
            reference: AgentReference {
                source_id: source_id.into(),
                relative_path: path.into(),
            },
            agent: Some(Agent {
                slug: "rust-reviewer".into(),
                name: "Rust Reviewer".into(),
                description: "Reviews backend changes".into(),
                category: "engineering".into(),
                emoji: None,
                color: None,
                vibe: None,
                body: "private prompt".into(),
            }),
            source_hash: "source-hash".into(),
            frontmatter_hash: "frontmatter-hash".into(),
            body_hash: "body-hash".into(),
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
            tags: vec!["rust".into()],
            capabilities: Vec::new(),
            permissions: Vec::new(),
            quality_score: 80,
            quality_checks: Vec::new(),
            diagnostics: Vec::new(),
            installable,
        }
    }

    fn skill_package(source_id: &str, path: &str, installable: bool) -> SkillPackageResult {
        SkillPackageResult {
            source_id: source_id.into(),
            relative_path: path.into(),
            name: Some("Rust Reviewer".into()),
            description: Some("Reviews backend changes".into()),
            skill_type: SkillType::Testing,
            group: Vec::new(),
            tags: vec!["rust".into()],
            dependencies: Vec::new(),
            recommended_skills: Vec::new(),
            version: None,
            channel: "stable".into(),
            changelog: None,
            publisher: None,
            publisher_key: None,
            publisher_verified: false,
            validation_results: Vec::new(),
            permissions: Vec::new(),
            quality_score: 80,
            quality_checks: Vec::new(),
            files: Vec::new(),
            trust_fingerprint: None,
            errors: Vec::new(),
            installable,
        }
    }

    #[test]
    fn task_recommendations_are_bounded_stable_exact_and_installable_only() {
        let agents = vec![AgentSourceResult {
            source: AgentSource {
                id: "agents".into(),
                label: "Agent source".into(),
                enabled: true,
                kind: AgentSourceKind::Local {
                    root: "/agents".into(),
                },
            },
            agents: vec![
                agent_package("source-b", "z/reviewer.md", true),
                agent_package("source-a", "a/reviewer.md", true),
                agent_package("source-c", "broken.md", false),
            ],
            errors: Vec::new(),
            revision: "revision".into(),
        }];
        let skills = vec![SkillSourceResult {
            source: SkillSource {
                id: "skills".into(),
                kind: SkillSourceKind::Local {
                    root: "/skills".into(),
                },
            },
            packages: vec![
                skill_package("skill-b", "z-review", true),
                skill_package("skill-a", "a-review", true),
                skill_package("skill-c", "broken", false),
            ],
            errors: Vec::new(),
        }];

        let matches = recommend_catalog(&agents, &[], &skills, "rust review", &[], 10)
            .expect("bounded recommendation");
        let identities = matches
            .iter()
            .map(|item| match item {
                TaskRecommendation::Agent { package, .. } => format!(
                    "agent:{}:{}",
                    package.reference.source_id, package.reference.relative_path
                ),
                TaskRecommendation::Skill { package, .. } => {
                    format!("skill:{}:{}", package.source_id, package.relative_path)
                }
            })
            .collect::<Vec<_>>();

        assert_eq!(
            identities,
            [
                "agent:source-a:a/reviewer.md",
                "agent:source-b:z/reviewer.md",
                "skill:skill-a:a-review",
                "skill:skill-b:z-review",
            ]
        );
        assert_eq!(
            matches
                .iter()
                .map(TaskRecommendation::score)
                .collect::<Vec<_>>(),
            [4, 4, 4, 4]
        );
        assert!(recommend_catalog(&agents, &[], &skills, &"x".repeat(2049), &[], 10).is_err());
    }

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
