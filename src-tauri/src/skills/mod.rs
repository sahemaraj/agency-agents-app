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
        std::fs::write(source.path().join("nested/skill.md"), b"not exact")
            .expect("write decoy");
        let state = test_state(app.path());

        let registered = add_local_source(&state, source.path())
            .await
            .expect("register source");
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
        write_skill(external.path(), "escaped", "escaped", "Must not be discovered");
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
        let second_source = second_task
            .await
            .expect("second join")
            .expect("second add");
        let persisted = load_skill_sources(app.path())
            .await
            .expect("load sources");

        assert_eq!(persisted.len(), 2);
        assert!(persisted.iter().any(|source| source.id == first_source.id));
        assert!(persisted.iter().any(|source| source.id == second_source.id));
    }
}
