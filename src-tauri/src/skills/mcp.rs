use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use crate::{
    state::{append_mcp_audit, AppState, McpAction, McpProjectAuthorization},
    types::{McpAuditEntry, SkillPackageResult, SkillSourceResult},
};
use axum::{
    extract::{Request, State},
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::post_service,
    Router,
};
use rmcp::{
    handler::server::{
        router::tool::ToolRouter,
        tool::{Extension, ToolCallContext},
        wrapper::Parameters,
    },
    model::{
        CallToolRequestParams, CallToolResponse, ErrorData, ListResourceTemplatesResult,
        ListResourcesResult, ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult,
        Resource, ResourceContents, ResourceTemplate, ServerCapabilities, ServerInfo,
    },
    schemars,
    service::RequestContext,
    tool, tool_router,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    RoleServer, ServerHandler, ServiceExt,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

const MAX_RECOMMEND_TASK_BYTES: usize = 2_048;
const MAX_RECOMMEND_LANGUAGES: usize = 32;
const MAX_RECOMMEND_LANGUAGE_BYTES: usize = 64;
const MIN_HTTP_TOKEN_BYTES: usize = 43;

#[derive(Clone)]
struct HttpAuth([u8; 32]);

impl HttpAuth {
    fn new(token: &str) -> Result<Self, String> {
        if token.as_bytes().len() < MIN_HTTP_TOKEN_BYTES {
            return Err(format!(
                "AGENCY_AGENTS_MCP_TOKEN must be at least {MIN_HTTP_TOKEN_BYTES} bytes"
            ));
        }
        Ok(Self(Sha256::digest(token.as_bytes()).into()))
    }

    fn permits(&self, headers: &HeaderMap) -> bool {
        let Some(candidate) = headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .filter(|value| !value.is_empty())
        else {
            return false;
        };
        let candidate: [u8; 32] = Sha256::digest(candidate.as_bytes()).into();
        bool::from(candidate.ct_eq(&self.0))
    }
}

async fn require_bearer(
    State(auth): State<HttpAuth>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if !auth.permits(request.headers()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(next.run(request).await)
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SearchRequest {
    query: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GetRequest {
    source_id: String,
    relative_path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FileRequest {
    source_id: String,
    relative_path: String,
    file_path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
enum SkillRuntime {
    ClaudeCode,
    Codex,
}

impl SkillRuntime {
    fn as_str(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "claudeCode",
            Self::Codex => "codex",
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SkillLifecycleRequest {
    source_id: String,
    relative_path: String,
    runtime: SkillRuntime,
    project_path: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FindAndInstallRequest {
    name: String,
    runtime: SkillRuntime,
    project_path: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FindAndInstallResult {
    installed: Option<crate::types::InstalledSkill>,
    candidates: Vec<SkillPackageResult>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct InstalledRequest {
    project_paths: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct LocalSourceRequest {
    root: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GithubSourceRequest {
    repository: String,
    git_ref: Option<String>,
    subdirectory: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SourceRequest {
    source_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DraftRequest {
    id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SubmitDraftRequest {
    files: Vec<SubmitDraftFile>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SubmitDraftFile {
    relative_path: String,
    text: Option<String>,
    base64: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RecommendRequest {
    task: String,
    #[serde(default)]
    languages: Vec<String>,
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SkillRecommendation {
    package: SkillPackageResult,
    score: u32,
    reasons: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CatalogResponse {
    catalog_revision: String,
    sources: Vec<SkillSourceResult>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RemoveSourceResponse {
    removed: bool,
    catalog_revision: String,
}

#[derive(Clone)]
pub struct SkillMcpServer {
    state: Arc<AppState>,
    #[allow(dead_code, reason = "tool_handler macro reads the generated router")]
    tool_router: ToolRouter<Self>,
}

impl SkillMcpServer {
    fn new(state: Arc<AppState>) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }

    async fn run_tool<T, F>(
        &self,
        _tool: &'static str,
        _action: McpAction,
        _project_path: Option<String>,
        operation: F,
    ) -> Result<T, String>
    where
        F: std::future::Future<Output = Result<T, String>>,
    {
        operation.await
    }

    async fn append_tool_audit(
        &self,
        id: &str,
        tool: &str,
        action: &str,
        phase: &str,
        success: bool,
        project_path: Option<&str>,
    ) -> Result<(), ErrorData> {
        append_mcp_audit(
            &self.state.app_data_dir,
            McpAuditEntry {
                id: id.into(),
                timestamp: chrono::Utc::now().to_rfc3339(),
                tool: tool.into(),
                action: action.into(),
                phase: phase.into(),
                success,
                project_path: project_path.map(str::to_owned),
            },
        )
        .await
        .map_err(|error| {
            ErrorData::internal_error(format!("MCP audit append failed: {error}"), None)
        })
    }
}

fn action_for_tool(tool: &str) -> Option<McpAction> {
    match tool {
        "skills_search"
        | "skills_get"
        | "skills_list_files"
        | "skills_get_file"
        | "skills_installed"
        | "skills_list_sources"
        | "skills_list_drafts"
        | "skills_get_draft"
        | "skills_source_status"
        | "skills_recommend" => Some(McpAction::Read),
        "skills_add_local_source"
        | "skills_add_github_source"
        | "skills_refresh_source"
        | "skills_refresh_all" => Some(McpAction::Source),
        "skills_submit_draft" => Some(McpAction::Source),
        "skills_install" | "skills_find_and_install" | "skills_update" | "skills_enable" => {
            Some(McpAction::Install)
        }
        "skills_disable" | "skills_uninstall" | "skills_remove_source" => {
            Some(McpAction::Destructive)
        }
        _ => None,
    }
}

fn tool_call_succeeded(result: &Result<CallToolResponse, ErrorData>) -> bool {
    match result {
        Ok(CallToolResponse::Complete(result)) => result.is_error != Some(true),
        Ok(_) => true,
        Err(_) => false,
    }
}

#[tool_router]
impl SkillMcpServer {
    #[tool(description = "Search validated skills by name or description")]
    async fn skills_search(
        &self,
        Parameters(SearchRequest { query }): Parameters<SearchRequest>,
    ) -> Result<String, String> {
        self.run_tool("skills_search", McpAction::Read, None, async {
            let results = super::inspect_skill_sources(&self.state)
                .await
                .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&search_packages(&results, &query))
                .map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "Read the SKILL.md for one validated skill")]
    async fn skills_get(
        &self,
        Parameters(GetRequest {
            source_id,
            relative_path,
        }): Parameters<GetRequest>,
    ) -> Result<String, String> {
        self.run_tool("skills_get", McpAction::Read, None, async {
            let content =
                super::read_skill_file(&self.state, &source_id, &relative_path, "SKILL.md")
                    .await
                    .map_err(|error| error.to_string())?;
            content.text.ok_or_else(|| "SKILL.md must be UTF-8".into())
        })
        .await
    }

    #[tool(description = "List the validated files in one skill package")]
    async fn skills_list_files(
        &self,
        Parameters(GetRequest {
            source_id,
            relative_path,
        }): Parameters<GetRequest>,
    ) -> Result<String, String> {
        self.run_tool("skills_list_files", McpAction::Read, None, async {
            let files = super::list_skill_files(&self.state, &source_id, &relative_path)
                .await
                .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&files).map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "Read one validated file from a skill package")]
    async fn skills_get_file(
        &self,
        Parameters(FileRequest {
            source_id,
            relative_path,
            file_path,
        }): Parameters<FileRequest>,
    ) -> Result<String, String> {
        self.run_tool("skills_get_file", McpAction::Read, None, async {
            let content =
                super::read_skill_file(&self.state, &source_id, &relative_path, &file_path)
                    .await
                    .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&content).map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "List installed skills, reconciling their lifecycle state")]
    async fn skills_installed(
        &self,
        Parameters(InstalledRequest { project_paths }): Parameters<InstalledRequest>,
    ) -> Result<String, String> {
        self.run_tool("skills_installed", McpAction::Read, None, async {
            let installed = super::reconcile_skill_installs(
                &self.state,
                project_paths.as_deref().unwrap_or_default(),
            )
            .await
            .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&installed).map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "Install a validated skill for a runtime and optional project path")]
    async fn skills_install(
        &self,
        Parameters(SkillLifecycleRequest {
            source_id,
            relative_path,
            runtime,
            project_path,
        }): Parameters<SkillLifecycleRequest>,
        Extension(project_authorization): Extension<McpProjectAuthorization>,
    ) -> Result<String, String> {
        self.run_tool(
            "skills_install",
            McpAction::Install,
            project_path.clone(),
            async {
                let installed = super::install_skill_authorized(
                    &self.state,
                    &source_id,
                    &relative_path,
                    runtime.as_str(),
                    project_path.as_deref(),
                    project_authorization.0.as_ref(),
                )
                .await
                .map_err(|error| error.to_string())?;
                serde_json::to_string_pretty(&installed).map_err(|error| error.to_string())
            },
        )
        .await
    }

    #[tool(description = "Install only when a skill name has one exact normalized catalog match")]
    async fn skills_find_and_install(
        &self,
        Parameters(FindAndInstallRequest {
            name,
            runtime,
            project_path,
        }): Parameters<FindAndInstallRequest>,
        Extension(project_authorization): Extension<McpProjectAuthorization>,
    ) -> Result<String, String> {
        self.run_tool(
            "skills_find_and_install",
            McpAction::Install,
            project_path.clone(),
            async {
                let results = super::inspect_skill_sources(&self.state)
                    .await
                    .map_err(|error| error.to_string())?;
                let normalized = normalize_skill_name(&name);
                let mut exact = results
                    .iter()
                    .flat_map(|result| &result.packages)
                    .filter(|package| {
                        package.installable
                            && package
                                .name
                                .as_deref()
                                .is_some_and(|name| normalize_skill_name(name) == normalized)
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                exact.sort_by(|left, right| {
                    (&left.source_id, &left.relative_path)
                        .cmp(&(&right.source_id, &right.relative_path))
                });
                let response = if exact.len() == 1 {
                    let package = &exact[0];
                    let installed = super::install_skill_authorized(
                        &self.state,
                        &package.source_id,
                        &package.relative_path,
                        runtime.as_str(),
                        project_path.as_deref(),
                        project_authorization.0.as_ref(),
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                    FindAndInstallResult {
                        installed: Some(installed),
                        candidates: Vec::new(),
                    }
                } else {
                    FindAndInstallResult {
                        installed: None,
                        candidates: if exact.is_empty() {
                            search_packages(&results, &normalized)
                        } else {
                            exact
                        },
                    }
                };
                serde_json::to_string_pretty(&response).map_err(|error| error.to_string())
            },
        )
        .await
    }

    #[tool(description = "Update a managed skill for a runtime and optional project path")]
    async fn skills_update(
        &self,
        Parameters(SkillLifecycleRequest {
            source_id,
            relative_path,
            runtime,
            project_path,
        }): Parameters<SkillLifecycleRequest>,
        Extension(project_authorization): Extension<McpProjectAuthorization>,
    ) -> Result<String, String> {
        self.run_tool(
            "skills_update",
            McpAction::Install,
            project_path.clone(),
            async {
                let installed = super::update_skill_authorized(
                    &self.state,
                    &source_id,
                    &relative_path,
                    runtime.as_str(),
                    project_path.as_deref(),
                    project_authorization.0.as_ref(),
                )
                .await
                .map_err(|error| error.to_string())?;
                serde_json::to_string_pretty(&installed).map_err(|error| error.to_string())
            },
        )
        .await
    }

    #[tool(description = "Disable a managed skill for a runtime and optional project path")]
    async fn skills_disable(
        &self,
        Parameters(SkillLifecycleRequest {
            source_id,
            relative_path,
            runtime,
            project_path,
        }): Parameters<SkillLifecycleRequest>,
        Extension(project_authorization): Extension<McpProjectAuthorization>,
    ) -> Result<String, String> {
        self.run_tool(
            "skills_disable",
            McpAction::Destructive,
            project_path.clone(),
            async {
                let installed = super::disable_skill_authorized(
                    &self.state,
                    &source_id,
                    &relative_path,
                    runtime.as_str(),
                    project_path.as_deref(),
                    project_authorization.0.as_ref(),
                )
                .await
                .map_err(|error| error.to_string())?;
                serde_json::to_string_pretty(&installed).map_err(|error| error.to_string())
            },
        )
        .await
    }

    #[tool(description = "Enable a managed skill for a runtime and optional project path")]
    async fn skills_enable(
        &self,
        Parameters(SkillLifecycleRequest {
            source_id,
            relative_path,
            runtime,
            project_path,
        }): Parameters<SkillLifecycleRequest>,
        Extension(project_authorization): Extension<McpProjectAuthorization>,
    ) -> Result<String, String> {
        self.run_tool(
            "skills_enable",
            McpAction::Install,
            project_path.clone(),
            async {
                let installed = super::enable_skill_authorized(
                    &self.state,
                    &source_id,
                    &relative_path,
                    runtime.as_str(),
                    project_path.as_deref(),
                    project_authorization.0.as_ref(),
                )
                .await
                .map_err(|error| error.to_string())?;
                serde_json::to_string_pretty(&installed).map_err(|error| error.to_string())
            },
        )
        .await
    }

    #[tool(description = "Uninstall a managed skill for a runtime and optional project path")]
    async fn skills_uninstall(
        &self,
        Parameters(SkillLifecycleRequest {
            source_id,
            relative_path,
            runtime,
            project_path,
        }): Parameters<SkillLifecycleRequest>,
        Extension(project_authorization): Extension<McpProjectAuthorization>,
    ) -> Result<String, String> {
        self.run_tool(
            "skills_uninstall",
            McpAction::Destructive,
            project_path.clone(),
            async {
                let removed = super::uninstall_skill_authorized(
                    &self.state,
                    &source_id,
                    &relative_path,
                    runtime.as_str(),
                    project_path.as_deref(),
                    project_authorization.0.as_ref(),
                )
                .await
                .map_err(|error| error.to_string())?;
                serde_json::to_string_pretty(&removed).map_err(|error| error.to_string())
            },
        )
        .await
    }

    #[tool(description = "List registered skill sources")]
    async fn skills_list_sources(&self) -> Result<String, String> {
        self.run_tool("skills_list_sources", McpAction::Read, None, async {
            let sources = super::load_skill_sources(&self.state.app_data_dir)
                .await
                .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&sources).map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "Register an existing local directory as a skill source")]
    async fn skills_add_local_source(
        &self,
        Parameters(LocalSourceRequest { root }): Parameters<LocalSourceRequest>,
    ) -> Result<String, String> {
        self.run_tool("skills_add_local_source", McpAction::Source, None, async {
            let source = super::add_local_source(&self.state, Path::new(&root))
                .await
                .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&source).map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "Register a GitHub repository as a skill source")]
    async fn skills_add_github_source(
        &self,
        Parameters(GithubSourceRequest {
            repository,
            git_ref,
            subdirectory,
        }): Parameters<GithubSourceRequest>,
    ) -> Result<String, String> {
        self.run_tool("skills_add_github_source", McpAction::Source, None, async {
            let source = super::add_github_source(
                &self.state,
                &repository,
                git_ref.as_deref(),
                subdirectory.as_deref(),
            )
            .await
            .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&source).map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "Refresh and validate a registered skill source")]
    async fn skills_refresh_source(
        &self,
        Parameters(SourceRequest { source_id }): Parameters<SourceRequest>,
    ) -> Result<String, String> {
        self.run_tool("skills_refresh_source", McpAction::Source, None, async {
            let result = super::refresh_skill_source(&self.state, &source_id)
                .await
                .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&result).map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "Unregister a skill source without deleting its source directory")]
    async fn skills_remove_source(
        &self,
        Parameters(SourceRequest { source_id }): Parameters<SourceRequest>,
    ) -> Result<String, String> {
        self.run_tool(
            "skills_remove_source",
            McpAction::Destructive,
            None,
            async {
                let removed = super::remove_skill_source(&self.state, &source_id)
                    .await
                    .map_err(|error| error.to_string())?;
                let sources = super::inspect_skill_sources(&self.state)
                    .await
                    .map_err(|error| error.to_string())?;
                serde_json::to_string_pretty(&RemoveSourceResponse {
                    removed,
                    catalog_revision: catalog_revision(&sources),
                })
                .map_err(|error| error.to_string())
            },
        )
        .await
    }

    #[tool(description = "Refresh and validate all registered skill sources")]
    async fn skills_refresh_all(&self) -> Result<String, String> {
        self.run_tool("skills_refresh_all", McpAction::Source, None, async {
            let sources = super::refresh_all_skill_sources(&self.state)
                .await
                .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&CatalogResponse {
                catalog_revision: catalog_revision(&sources),
                sources,
            })
            .map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "Inspect every source and return its current catalog revision")]
    async fn skills_source_status(&self) -> Result<String, String> {
        self.run_tool("skills_source_status", McpAction::Read, None, async {
            let sources = super::inspect_skill_sources(&self.state)
                .await
                .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&CatalogResponse {
                catalog_revision: catalog_revision(&sources),
                sources,
            })
            .map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(
        description = "Recommend validated skills using exact task and language metadata tokens"
    )]
    async fn skills_recommend(
        &self,
        Parameters(RecommendRequest {
            task,
            languages,
            limit,
        }): Parameters<RecommendRequest>,
    ) -> Result<String, String> {
        self.run_tool("skills_recommend", McpAction::Read, None, async {
            validate_recommend_request(&task, &languages)?;
            let sources = super::inspect_skill_sources(&self.state)
                .await
                .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&recommend_skills(
                &sources,
                &task,
                &languages,
                limit.unwrap_or(10).clamp(1, 50),
            ))
            .map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "Submit a bounded skill package draft for desktop review")]
    async fn skills_submit_draft(
        &self,
        Parameters(SubmitDraftRequest { files }): Parameters<SubmitDraftRequest>,
    ) -> Result<String, String> {
        self.run_tool("skills_submit_draft", McpAction::Source, None, async {
            let files = files
                .into_iter()
                .map(|file| super::drafts::DraftInputFile {
                    relative_path: file.relative_path,
                    text: file.text,
                    base64: file.base64,
                })
                .collect();
            let draft = super::drafts::submit(&self.state, files)
                .await
                .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&draft).map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "List skill drafts and validation diagnostics")]
    async fn skills_list_drafts(&self) -> Result<String, String> {
        self.run_tool("skills_list_drafts", McpAction::Read, None, async {
            let drafts = super::drafts::list(&self.state)
                .await
                .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&drafts).map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "Read one skill draft and its validation diagnostics")]
    async fn skills_get_draft(
        &self,
        Parameters(DraftRequest { id }): Parameters<DraftRequest>,
    ) -> Result<String, String> {
        self.run_tool("skills_get_draft", McpAction::Read, None, async {
            let draft = super::drafts::get(&self.state, &id)
                .await
                .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&draft).map_err(|error| error.to_string())
        })
        .await
    }
}

#[rmcp::tool_handler(router = self.tool_router)]
impl ServerHandler for SkillMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_resources_list_changed()
                .build(),
        )
    }

    async fn call_tool(
        &self,
        mut request: CallToolRequestParams,
        mut context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let tool = request.name.to_string();
        let Some(action) = action_for_tool(&tool) else {
            let id = uuid::Uuid::new_v4().to_string();
            self.append_tool_audit(&id, &tool, "unknown", "attempt", false, None)
                .await?;
            if let Err(audit_error) = self
                .append_tool_audit(&id, &tool, "unknown", "terminal", false, None)
                .await
            {
                tracing::error!(
                    tool,
                    error = %audit_error,
                    "MCP terminal unclassified-tool audit failed"
                );
            }
            return Err(ErrorData::invalid_params(
                "unclassified MCP tool; request denied before dispatch",
                None,
            ));
        };
        let action_name = action.as_str();
        let requested_project = request
            .arguments
            .as_ref()
            .and_then(|arguments| arguments.get("project_path"))
            .and_then(serde_json::Value::as_str);
        let authorization = self
            .state
            .authorize_mcp(action, requested_project)
            .await
            .map_err(|error| error.to_string());
        let project_authorization = match authorization {
            Ok(project) => project,
            Err(error) => {
                let id = uuid::Uuid::new_v4().to_string();
                self.append_tool_audit(&id, &tool, action_name, "attempt", false, None)
                    .await?;
                if let Err(audit_error) = self
                    .append_tool_audit(&id, &tool, action_name, "terminal", false, None)
                    .await
                {
                    tracing::error!(
                        tool,
                        error = %audit_error,
                        "MCP terminal denial audit failed"
                    );
                }
                return Err(ErrorData::invalid_params(error, None));
            }
        };
        let authorized_project = project_authorization
            .as_ref()
            .map(|project| project.identity().to_owned());
        if let Some(project) = &authorized_project {
            request.arguments.get_or_insert_default().insert(
                "project_path".into(),
                serde_json::Value::String(project.clone()),
            );
        }
        context
            .extensions
            .insert(McpProjectAuthorization(project_authorization));

        let id = uuid::Uuid::new_v4().to_string();
        self.append_tool_audit(
            &id,
            &tool,
            action_name,
            "attempt",
            false,
            authorized_project.as_deref(),
        )
        .await?;

        let catalog_before = if is_source_catalog_mutation(&tool) {
            super::inspect_skill_sources(&self.state)
                .await
                .ok()
                .map(|results| catalog_revision(&results))
        } else {
            None
        };
        let peer = context.peer.clone();
        let tool_context = ToolCallContext::new(self, request, context);
        let result = self.tool_router.call(tool_context).await;
        let success = tool_call_succeeded(&result);
        let catalog_after = if catalog_before.is_some() {
            super::inspect_skill_sources(&self.state)
                .await
                .ok()
                .map(|results| catalog_revision(&results))
        } else {
            None
        };
        if resource_list_changed(
            &tool,
            success,
            catalog_before.as_deref(),
            catalog_after.as_deref(),
        ) {
            if let Err(error) = peer.notify_resource_list_changed().await {
                tracing::debug!(tool, %error, "MCP peer does not accept resource list changes");
            }
        }
        if let Err(error) = self
            .append_tool_audit(
                &id,
                &tool,
                action_name,
                "terminal",
                success,
                authorized_project.as_deref(),
            )
            .await
        {
            // The durable attempt proves the mutation was admitted. Returning
            // the completed tool result unchanged is deliberately retry-safe:
            // reporting an audit error after a successful mutation could make
            // an MCP client repeat a non-idempotent operation.
            tracing::error!(tool, error = %error, "MCP terminal audit failed");
        }
        result
    }

    async fn list_resources(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let catalog = super::inspect_skill_sources(&self.state)
            .await
            .map_err(mcp_invalid)?;
        let mut resources = vec![Resource::new("skills://catalog", "Skills catalog")
            .with_description("Registered skill sources and their validated packages")
            .with_mime_type("application/json")];
        for result in catalog {
            for package in result
                .packages
                .into_iter()
                .filter(|package| package.installable)
            {
                for file in package.files {
                    resources.push(
                        Resource::new(
                            package_resource_uri(
                                &result.source.id,
                                &package.relative_path,
                                &file.relative_path,
                            ),
                            format!("{}/{}", package.relative_path, file.relative_path),
                        )
                        .with_size(file.size_bytes),
                    );
                }
            }
        }
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn list_resource_templates(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        Ok(ListResourceTemplatesResult::with_all_items(vec![
            ResourceTemplate::new(
                "skills://packages/~{source_id}/~{relative_path}/~{file_path}",
                "Skill package file",
            )
            .with_description("A validated file from a registered skill package"),
        ]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let uri = request.uri;
        let content = if uri == "skills://catalog" {
            let catalog = super::inspect_skill_sources(&self.state)
                .await
                .map_err(mcp_invalid)?;
            ResourceContents::text(
                serde_json::to_string_pretty(&catalog).map_err(mcp_invalid)?,
                &uri,
            )
            .with_mime_type("application/json")
        } else {
            let (source_id, relative_path, file_path) = parse_package_resource_uri(&uri)?;
            let content =
                super::read_skill_file(&self.state, &source_id, &relative_path, &file_path)
                    .await
                    .map_err(mcp_invalid)?;
            match (content.text, content.base64) {
                (Some(text), None) => {
                    ResourceContents::text(text, &uri).with_mime_type(content.mime_type)
                }
                (None, Some(blob)) => {
                    ResourceContents::blob(blob, &uri).with_mime_type(content.mime_type)
                }
                _ => {
                    return Err(ErrorData::invalid_params(
                        "invalid skill file content",
                        None,
                    ))
                }
            }
        };
        Ok(ReadResourceResult::new(vec![content]).into())
    }
}

pub async fn serve() -> Result<(), String> {
    let state = Arc::new(AppState::build().map_err(|error| error.to_string())?);
    SkillMcpServer::new(state)
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|error| error.to_string())?
        .waiting()
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn http_router(
    state: Arc<AppState>,
    auth: HttpAuth,
    address: SocketAddr,
    config: StreamableHttpServerConfig,
) -> Router {
    let server = SkillMcpServer::new(state);
    let authority = address.to_string();
    let origin_host = match address.ip() {
        std::net::IpAddr::V4(ip) => ip.to_string(),
        std::net::IpAddr::V6(ip) => format!("[{ip}]"),
    };
    let config = config
        .with_allowed_hosts([authority.clone(), format!("localhost:{}", address.port())])
        .with_allowed_origins([
            format!("http://{authority}"),
            format!("http://localhost:{}", address.port()),
            format!("http://{origin_host}:{}", address.port()),
        ]);
    let service: StreamableHttpService<SkillMcpServer, LocalSessionManager> =
        StreamableHttpService::new(move || Ok(server.clone()), Default::default(), config);
    Router::new()
        .route_service("/mcp", post_service(service))
        .layer(middleware::from_fn_with_state(auth, require_bearer))
}

pub async fn serve_http(bind: SocketAddr, token: String) -> Result<(), String> {
    if !bind.ip().is_loopback() {
        return Err("MCP HTTP bind address must be loopback".into());
    }
    let auth = HttpAuth::new(&token)?;
    let state = Arc::new(AppState::build().map_err(|error| error.to_string())?);
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|error| format!("bind {bind}: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("read MCP HTTP address: {error}"))?;
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true);
    let cancellation = config.cancellation_token.clone();
    let router = http_router(state, auth, address, config);
    eprintln!("MCP HTTP listening on http://{address}/mcp");
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            cancellation.cancel();
        })
        .await
        .map_err(|error| format!("MCP HTTP server failed: {error}"))
}

fn search_packages(results: &[SkillSourceResult], query: &str) -> Vec<SkillPackageResult> {
    let query = query.to_lowercase();
    results
        .iter()
        .flat_map(|result| &result.packages)
        .filter(|package| {
            package.installable
                && package
                    .name
                    .iter()
                    .map(String::as_str)
                    .chain(package.description.iter().map(String::as_str))
                    .chain(std::iter::once(package.skill_type.as_str()))
                    .chain(package.group.iter().map(String::as_str))
                    .chain(package.tags.iter().map(String::as_str))
                    .any(|value| value.to_lowercase().contains(&query))
        })
        .cloned()
        .collect()
}

fn validate_recommend_request(task: &str, languages: &[String]) -> Result<(), String> {
    if task.len() > MAX_RECOMMEND_TASK_BYTES {
        return Err(format!(
            "task exceeds the {MAX_RECOMMEND_TASK_BYTES}-byte limit"
        ));
    }
    if languages.len() > MAX_RECOMMEND_LANGUAGES {
        return Err(format!(
            "languages exceeds the {MAX_RECOMMEND_LANGUAGES}-item limit"
        ));
    }
    if languages
        .iter()
        .any(|language| language.len() > MAX_RECOMMEND_LANGUAGE_BYTES)
    {
        return Err(format!(
            "language exceeds the {MAX_RECOMMEND_LANGUAGE_BYTES}-byte limit"
        ));
    }
    Ok(())
}

fn metadata_tokens(value: &str) -> std::collections::BTreeSet<String> {
    value
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn recommend_skills(
    results: &[SkillSourceResult],
    task: &str,
    languages: &[String],
    limit: usize,
) -> Vec<SkillRecommendation> {
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
            let name_tokens = metadata_tokens(package.name.as_deref().unwrap_or_default());
            let description_tokens =
                metadata_tokens(package.description.as_deref().unwrap_or_default());
            let taxonomy_tokens = std::iter::once(package.skill_type.as_str())
                .chain(package.group.iter().map(String::as_str))
                .chain(package.tags.iter().map(String::as_str))
                .flat_map(metadata_tokens)
                .collect::<std::collections::BTreeSet<_>>();
            let mut score = 0;
            let mut reasons = Vec::new();
            for token in &task_tokens {
                if name_tokens.contains(token) {
                    score += 4;
                    reasons.push(format!("task:name:{token}"));
                } else if description_tokens.contains(token) {
                    score += 2;
                    reasons.push(format!("task:description:{token}"));
                } else if taxonomy_tokens.contains(token) {
                    score += 2;
                    reasons.push(format!("task:taxonomy:{token}"));
                }
            }
            for token in &language_tokens {
                if name_tokens.contains(token)
                    || description_tokens.contains(token)
                    || taxonomy_tokens.contains(token)
                {
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
    recommendations.truncate(limit);
    recommendations
}

fn catalog_revision(results: &[SkillSourceResult]) -> String {
    let mut canonical = results.to_vec();
    canonical.sort_by(|left, right| left.source.id.cmp(&right.source.id));
    for source in &mut canonical {
        source.packages.sort_by(|left, right| {
            (&left.source_id, &left.relative_path).cmp(&(&right.source_id, &right.relative_path))
        });
        source.errors.sort_by(|left, right| {
            (&left.path, &left.message)
                .cmp(&(&right.path, &right.message))
                .then_with(|| format!("{:?}", left.code).cmp(&format!("{:?}", right.code)))
        });
        for package in &mut source.packages {
            package
                .files
                .sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
            package.errors.sort_by(|left, right| {
                (&left.path, &left.message)
                    .cmp(&(&right.path, &right.message))
                    .then_with(|| format!("{:?}", left.code).cmp(&format!("{:?}", right.code)))
            });
        }
    }
    let canonical = canonical
        .iter()
        .map(|result| {
            let source = match &result.source.kind {
                crate::types::SkillSourceKind::Local { root } => {
                    serde_json::json!(["local", root])
                }
                crate::types::SkillSourceKind::Github {
                    repository,
                    git_ref,
                    subdirectory,
                    ..
                } => serde_json::json!(["github", repository, git_ref, subdirectory]),
            };
            let packages = result
                .packages
                .iter()
                .map(|package| {
                    let files = package
                        .files
                        .iter()
                        .map(|file| {
                            serde_json::json!([file.relative_path, file.size_bytes, file.sha256])
                        })
                        .collect::<Vec<_>>();
                    let errors = package
                        .errors
                        .iter()
                        .map(|error| {
                            serde_json::json!([
                                format!("{:?}", error.code),
                                error.path,
                                error.message
                            ])
                        })
                        .collect::<Vec<_>>();
                    serde_json::json!([
                        package.relative_path,
                        package.name,
                        package.description,
                        package.installable,
                        files,
                        errors
                    ])
                })
                .collect::<Vec<_>>();
            let errors = result
                .errors
                .iter()
                .map(|error| {
                    serde_json::json!([format!("{:?}", error.code), error.path, error.message])
                })
                .collect::<Vec<_>>();
            serde_json::json!([result.source.id, source, packages, errors])
        })
        .collect::<Vec<_>>();
    let bytes = serde_json::to_vec(&canonical).expect("normalized skill catalog serializes");
    format!("{:x}", Sha256::digest(bytes))
}

fn resource_list_changed(
    tool: &str,
    success: bool,
    before: Option<&str>,
    after: Option<&str>,
) -> bool {
    success
        && is_source_catalog_mutation(tool)
        && matches!((before, after), (Some(before), Some(after)) if before != after)
}

fn is_source_catalog_mutation(tool: &str) -> bool {
    matches!(
        tool,
        "skills_add_local_source"
            | "skills_add_github_source"
            | "skills_refresh_source"
            | "skills_refresh_all"
            | "skills_remove_source"
    )
}

fn normalize_skill_name(value: &str) -> String {
    let mut normalized = String::new();
    let mut separator = false;
    for character in value.trim().chars() {
        if character.is_ascii_alphanumeric() {
            if separator && !normalized.is_empty() {
                normalized.push('-');
            }
            normalized.push(character.to_ascii_lowercase());
            separator = false;
        } else {
            separator = true;
        }
    }
    normalized
}

fn mcp_invalid(error: impl std::fmt::Display) -> ErrorData {
    ErrorData::invalid_params(error.to_string(), None)
}

fn package_resource_uri(source_id: &str, relative_path: &str, file_path: &str) -> String {
    format!(
        "skills://packages/{}/{}/{}",
        encode_resource_component(source_id),
        encode_resource_component(relative_path),
        encode_resource_component(file_path),
    )
}

fn parse_package_resource_uri(uri: &str) -> Result<(String, String, String), ErrorData> {
    let parsed = url::Url::parse(uri).map_err(|error| {
        ErrorData::invalid_params(format!("invalid skill resource URI: {error}"), None)
    })?;
    if parsed.scheme() != "skills"
        || parsed.host_str() != Some("packages")
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port().is_some()
    {
        return Err(ErrorData::invalid_params("unknown skill resource", None));
    }
    let parts = parsed
        .path_segments()
        .ok_or_else(|| ErrorData::invalid_params("invalid skill package resource path", None))
        .and_then(|parts| {
            let parts = parts.collect::<Vec<_>>();
            (parts.len() == 3).then_some(parts).ok_or_else(|| {
                ErrorData::invalid_params("invalid skill package resource path", None)
            })
        })?;
    let source_id = decode_resource_component(parts[0])?;
    let relative_path = decode_resource_component(parts[1])?;
    let file_path = decode_resource_component(parts[2])?;
    if source_id.is_empty() || source_id.contains(['/', '\\']) {
        return Err(ErrorData::invalid_params("invalid skill source id", None));
    }
    let relative_path = normalized_resource_path(&relative_path, true)?;
    let file_path = normalized_resource_path(&file_path, false)?;
    Ok((source_id, relative_path, file_path))
}

fn encode_resource_component(value: &str) -> String {
    let mut output = String::from("~");
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            output.push(byte as char);
        } else {
            output.push('%');
            output.push(char::from(b"0123456789ABCDEF"[(byte >> 4) as usize]));
            output.push(char::from(b"0123456789ABCDEF"[(byte & 0x0f) as usize]));
        }
    }
    output
}

fn decode_resource_component(value: &str) -> Result<String, ErrorData> {
    let value = value
        .strip_prefix('~')
        .ok_or_else(|| ErrorData::invalid_params("invalid skill resource component", None))?;
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                if index + 2 >= bytes.len() {
                    return Err(ErrorData::invalid_params(
                        "invalid percent-encoded skill resource component",
                        None,
                    ));
                }
                let high = hex_value(bytes[index + 1]).ok_or_else(|| {
                    ErrorData::invalid_params(
                        "invalid percent-encoded skill resource component",
                        None,
                    )
                })?;
                let low = hex_value(bytes[index + 2]).ok_or_else(|| {
                    ErrorData::invalid_params(
                        "invalid percent-encoded skill resource component",
                        None,
                    )
                })?;
                decoded.push(high << 4 | low);
                index += 3;
            }
            byte if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') => {
                decoded.push(byte);
                index += 1;
            }
            _ => {
                return Err(ErrorData::invalid_params(
                    "invalid skill resource component",
                    None,
                ))
            }
        }
    }
    String::from_utf8(decoded)
        .map_err(|_| ErrorData::invalid_params("skill resource component must be UTF-8", None))
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn normalized_resource_path(value: &str, allow_root: bool) -> Result<String, ErrorData> {
    if allow_root && value == "." {
        return Ok(value.into());
    }
    if value.is_empty()
        || value.contains('\\')
        || value
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(ErrorData::invalid_params(
            "invalid skill resource path",
            None,
        ));
    }
    Ok(value.into())
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;
    use std::sync::Arc;
    use std::time::Duration;

    use crate::{
        commands::settings::{Settings, SettingsLoadState},
        state::{AppState, McpProjectAuthorization},
        types::{SkillPackageResult, SkillSource, SkillSourceKind, SkillSourceResult},
    };
    use rmcp::{
        handler::server::{tool::Extension, wrapper::Parameters},
        ServiceExt,
    };
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::sync::{oneshot, Mutex, RwLock};

    use super::{
        action_for_tool, catalog_revision, package_resource_uri, parse_package_resource_uri,
        recommend_skills, resource_list_changed, search_packages, validate_recommend_request,
        FindAndInstallRequest, HttpAuth, RecommendRequest, SkillMcpServer, SkillRuntime,
        SourceRequest,
    };

    fn package(name: &str, description: &str, installable: bool) -> SkillPackageResult {
        SkillPackageResult {
            source_id: "source-1".into(),
            relative_path: name.to_lowercase().replace(' ', "-"),
            name: Some(name.into()),
            description: Some(description.into()),
            skill_type: crate::types::SkillType::Other,
            group: Vec::new(),
            tags: Vec::new(),
            files: Vec::new(),
            trust_fingerprint: None,
            errors: Vec::new(),
            installable,
        }
    }

    fn test_state(app_data_dir: &std::path::Path) -> Arc<AppState> {
        Arc::new(AppState {
            app_data_dir: app_data_dir.to_path_buf(),
            corpus_cache: Arc::new(Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(Mutex::new(())),
            skill_sources_write_lock: Arc::new(Mutex::new(())),
            skill_installs_write_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(RwLock::new(SettingsLoadState::FirstLaunch)),
            updater_state: crate::commands::updater::empty_state(),
        })
    }

    #[test]
    fn http_bearer_auth_rejects_missing_malformed_short_and_wrong_values() {
        let token = "a".repeat(43);
        let auth = HttpAuth::new(&token).expect("valid token");
        assert!(HttpAuth::new("short").is_err());
        for value in [
            None,
            Some("Basic abc"),
            Some("Bearer "),
            Some("Bearer wrong"),
        ] {
            let mut headers = axum::http::HeaderMap::new();
            if let Some(value) = value {
                headers.insert(
                    axum::http::header::AUTHORIZATION,
                    value.parse().expect("header"),
                );
            }
            assert!(!auth.permits(&headers));
        }
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {token}").parse().expect("header"),
        );
        assert!(auth.permits(&headers));
    }

    #[tokio::test]
    async fn http_transport_authenticates_and_enforces_http_boundaries() {
        let app = tempfile::tempdir().expect("app data");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        let token = "t".repeat(43);
        let config = rmcp::transport::streamable_http_server::StreamableHttpServerConfig::default()
            .with_legacy_session_mode(false)
            .with_json_response(true);
        let router = super::http_router(
            test_state(app.path()),
            HttpAuth::new(&token).expect("auth"),
            address,
            config,
        );
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("serve");
        });
        let client = reqwest::Client::new();
        let url = format!("http://{address}/mcp");
        let initialize = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "http-test", "version": "1"}
            }
        });

        assert_eq!(
            client
                .post(&url)
                .json(&initialize)
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            client
                .get(&url)
                .bearer_auth(&token)
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::METHOD_NOT_ALLOWED
        );
        assert_eq!(
            client
                .post(&url)
                .bearer_auth(&token)
                .header("Host", "evil.example")
                .json(&initialize)
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::FORBIDDEN
        );
        assert_eq!(
            client
                .post(&url)
                .bearer_auth(&token)
                .header("Origin", "https://evil.example")
                .json(&initialize)
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::FORBIDDEN
        );
        assert_eq!(
            client
                .post(&url)
                .bearer_auth(&token)
                .header("Content-Type", "application/json")
                .header("Accept", "application/json, text/event-stream")
                .body(vec![b' '; 4 * 1024 * 1024 + 1])
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::PAYLOAD_TOO_LARGE
        );

        let response = client
            .post(&url)
            .bearer_auth(&token)
            .header("Accept", "application/json, text/event-stream")
            .json(&initialize)
            .send()
            .await
            .expect("initialize");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert!(
            response.json::<serde_json::Value>().await.unwrap()["result"]["serverInfo"]["name"]
                .is_string()
        );

        let tools = client
            .post(&url)
            .bearer_auth(&token)
            .header("Accept", "application/json, text/event-stream")
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/list"
            }))
            .send()
            .await
            .expect("tools/list");
        assert_eq!(tools.status(), reqwest::StatusCode::OK);
        assert!(
            !tools.json::<serde_json::Value>().await.unwrap()["result"]["tools"]
                .as_array()
                .unwrap()
                .is_empty()
        );

        shutdown_tx.send(()).expect("shutdown");
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("graceful shutdown timeout")
            .expect("server task");
    }

    async fn call_tools_over_stdio(
        state: Arc<AppState>,
        calls: Vec<serde_json::Value>,
    ) -> Vec<serde_json::Value> {
        let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
        let server = SkillMcpServer::new(state);
        let server_task = tokio::spawn(async move {
            server
                .serve(server_transport)
                .await
                .expect("start server")
                .waiting()
                .await
                .expect("wait for server");
        });
        let (read, mut write) = tokio::io::split(client_transport);
        let mut read = BufReader::new(read);
        let mut line = String::new();
        write
            .write_all(
                br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#,
            )
            .await
            .expect("initialize request");
        write.write_all(b"\n").await.expect("initialize newline");
        tokio::time::timeout(Duration::from_secs(2), read.read_line(&mut line))
            .await
            .expect("initialize response timeout")
            .expect("initialize response");
        line.clear();
        write
            .write_all(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
            .await
            .expect("initialized notification");
        write.write_all(b"\n").await.expect("notification newline");

        let mut responses = Vec::new();
        for (index, params) in calls.into_iter().enumerate() {
            let request = serde_json::json!({
                "jsonrpc": "2.0",
                "id": index + 2,
                "method": "tools/call",
                "params": params,
            });
            write
                .write_all(request.to_string().as_bytes())
                .await
                .expect("tool request");
            write.write_all(b"\n").await.expect("tool newline");
            tokio::time::timeout(Duration::from_secs(2), read.read_line(&mut line))
                .await
                .expect("tool response timeout")
                .expect("tool response");
            responses.push(serde_json::from_str(&line).expect("tool response JSON"));
            line.clear();
        }
        server_task.abort();
        responses
    }

    #[test]
    fn search_is_case_insensitive_and_excludes_invalid_packages() {
        let mut taxonomy_match = package("Interface Builder", "Builds interfaces", true);
        taxonomy_match.skill_type = crate::types::SkillType::Design;
        taxonomy_match.group = vec!["frontend".into()];
        taxonomy_match.tags = vec!["svelte".into()];
        let results = vec![SkillSourceResult {
            source: SkillSource {
                id: "source-1".into(),
                kind: SkillSourceKind::Local {
                    root: "/skills".into(),
                },
            },
            packages: vec![
                package("Rust Reviewer", "Reviews unsafe Rust", true),
                package("Broken Rust Skill", "Invalid metadata", false),
                taxonomy_match,
            ],
            errors: Vec::new(),
        }];

        let matches = search_packages(&results, "RUST");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name.as_deref(), Some("Rust Reviewer"));
        assert_eq!(search_packages(&results, "frontend").len(), 1);
        assert_eq!(search_packages(&results, "svelte").len(), 1);
        assert_eq!(search_packages(&results, "design").len(), 1);
    }

    #[test]
    fn recommendations_use_exact_tokens_and_stable_catalog_ties() {
        let mut rust = package("Rust Reviewer", "Reviews safe backend changes", true);
        rust.source_id = "source-b".into();
        rust.relative_path = "z-rust".into();
        let mut earlier_rust = rust.clone();
        earlier_rust.source_id = "source-a".into();
        earlier_rust.relative_path = "a-rust".into();
        let results = vec![SkillSourceResult {
            source: SkillSource {
                id: "source-a".into(),
                kind: SkillSourceKind::Local {
                    root: "/skills".into(),
                },
            },
            packages: vec![
                package("Trust Review", "Reviews UI changes", true),
                rust,
                earlier_rust,
                package("Broken Rust", "Rust review", false),
            ],
            errors: Vec::new(),
        }];

        let recommendations = recommend_skills(&results, "review backend", &["rust".into()], 10);

        let expected = [
            (
                "source-a",
                "Rust Reviewer",
                5,
                vec!["task:description:backend", "language:rust"],
            ),
            (
                "source-b",
                "Rust Reviewer",
                5,
                vec!["task:description:backend", "language:rust"],
            ),
            ("source-1", "Trust Review", 4, vec!["task:name:review"]),
        ];
        assert_eq!(recommendations.len(), expected.len());
        for (actual, (source, name, score, reasons)) in recommendations.iter().zip(expected) {
            assert_eq!(actual.package.source_id, source);
            assert_eq!(actual.package.name.as_deref(), Some(name));
            assert_eq!(actual.score, score);
            assert_eq!(actual.reasons, reasons);
        }
    }

    #[test]
    fn catalog_revision_is_order_independent_and_changes_with_catalog_metadata() {
        let source = |id: &str, name: &str| SkillSourceResult {
            source: SkillSource {
                id: id.into(),
                kind: SkillSourceKind::Local {
                    root: format!("/{id}"),
                },
            },
            packages: vec![package(name, "Reviews code", true)],
            errors: Vec::new(),
        };
        let forward = vec![source("b", "Beta"), source("a", "Alpha")];
        let reverse = vec![source("a", "Alpha"), source("b", "Beta")];
        let changed = vec![source("a", "Alpha"), source("b", "Gamma")];

        assert_eq!(catalog_revision(&forward), catalog_revision(&reverse));
        assert_ne!(catalog_revision(&forward), catalog_revision(&changed));
        assert_eq!(catalog_revision(&forward).len(), 64);
    }

    #[test]
    fn catalog_revision_ignores_ephemeral_github_checkout_generations() {
        let result = |checkout: &str| SkillSourceResult {
            source: SkillSource {
                id: "stable-source-id".into(),
                kind: SkillSourceKind::Github {
                    repository: "https://github.com/example/skills.git".into(),
                    git_ref: Some("main".into()),
                    subdirectory: Some("skills".into()),
                    active_checkout: Some(checkout.into()),
                },
            },
            packages: vec![package("Rust Review", "Reviews Rust", true)],
            errors: Vec::new(),
        };

        assert_eq!(
            catalog_revision(&[result(
                "/app/skills/sources/stable-source-id/2b1952ac-3db7-4aaf-a45f-a7ad55c35482"
            )]),
            catalog_revision(&[result(
                "/app/skills/sources/stable-source-id/e1351991-8dc3-4e23-bc3c-2cfbdf707629"
            )])
        );
    }

    #[test]
    fn recommendation_inputs_are_bounded_before_tokenization() {
        assert!(validate_recommend_request(&"x".repeat(2048), &vec!["rust".into(); 32]).is_ok());
        assert!(validate_recommend_request(&"x".repeat(2049), &[]).is_err());
        assert!(validate_recommend_request("review", &vec!["rust".into(); 33]).is_err());
        assert!(validate_recommend_request("review", &["x".repeat(65)]).is_err());
    }

    #[test]
    fn unchanged_or_unknown_source_mutations_do_not_emit_resource_notifications() {
        assert!(!resource_list_changed(
            "skills_remove_source",
            true,
            Some("same"),
            Some("same")
        ));
        assert!(resource_list_changed(
            "skills_remove_source",
            true,
            Some("before"),
            Some("after")
        ));
        assert!(!resource_list_changed(
            "skills_remove_source",
            false,
            Some("before"),
            Some("after")
        ));
    }

    #[test]
    fn task_four_tools_are_classified_at_the_policy_boundary() {
        assert_eq!(
            action_for_tool("skills_source_status"),
            Some(crate::state::McpAction::Read)
        );
        assert_eq!(
            action_for_tool("skills_recommend"),
            Some(crate::state::McpAction::Read)
        );
        assert_eq!(
            action_for_tool("skills_refresh_all"),
            Some(crate::state::McpAction::Source)
        );
        assert_eq!(
            action_for_tool("skills_remove_source"),
            Some(crate::state::McpAction::Destructive)
        );
    }

    #[tokio::test]
    async fn source_lifecycle_responses_carry_pollable_catalog_revisions() {
        let app = tempfile::tempdir().expect("app data");
        let root = tempfile::tempdir().expect("skill source");
        let skill = root.path().join("rust-reviewer");
        std::fs::create_dir(&skill).expect("skill package");
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: rust-reviewer\ndescription: Reviews Rust code\n---\n",
        )
        .expect("skill markdown");
        let state = Arc::new(AppState {
            app_data_dir: app.path().to_path_buf(),
            corpus_cache: Arc::new(Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(Mutex::new(())),
            skill_sources_write_lock: Arc::new(Mutex::new(())),
            skill_installs_write_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(RwLock::new(SettingsLoadState::FirstLaunch)),
            updater_state: crate::commands::updater::empty_state(),
        });
        let registered = super::super::add_local_source(&state, root.path())
            .await
            .expect("register source");
        let server = SkillMcpServer::new(state);

        let refreshed: serde_json::Value =
            serde_json::from_str(&server.skills_refresh_all().await.expect("refresh all"))
                .expect("refresh JSON");
        let status: serde_json::Value =
            serde_json::from_str(&server.skills_source_status().await.expect("source status"))
                .expect("status JSON");
        let recommended: serde_json::Value = serde_json::from_str(
            &server
                .skills_recommend(Parameters(RecommendRequest {
                    task: "review".into(),
                    languages: vec!["rust".into()],
                    limit: Some(5),
                }))
                .await
                .expect("recommend"),
        )
        .expect("recommend JSON");
        assert_eq!(refreshed["catalogRevision"], status["catalogRevision"]);
        assert_eq!(recommended[0]["package"]["name"], "rust-reviewer");

        let removed: serde_json::Value = serde_json::from_str(
            &server
                .skills_remove_source(Parameters(SourceRequest {
                    source_id: registered.id,
                }))
                .await
                .expect("remove source"),
        )
        .expect("remove JSON");
        assert_eq!(removed["removed"], true);
        assert_ne!(removed["catalogRevision"], status["catalogRevision"]);
    }

    #[test]
    fn package_resource_uris_round_trip_root_nested_and_literal_plus_paths() {
        let root = package_resource_uri("source-1", ".", "SKILL.md");
        assert_eq!(root, "skills://packages/~source-1/~./~SKILL.md");
        assert_eq!(
            parse_package_resource_uri(&root).expect("decode root package"),
            ("source-1".into(), ".".into(), "SKILL.md".into())
        );

        let nested = package_resource_uri("source-1", "nested/reviewer", "references/a+b.txt");
        assert!(nested.contains("nested%2Freviewer"));
        assert!(nested.contains("a%2Bb.txt"));
        assert_eq!(
            parse_package_resource_uri(&nested).expect("decode nested package"),
            (
                "source-1".into(),
                "nested/reviewer".into(),
                "references/a+b.txt".into(),
            )
        );

        for malformed in [
            "skills://packages/~source-1/~nested%2Freviewer/~..%2FSKILL.md",
            "skills://packages/~source-1/~nested%ZZreviewer/~SKILL.md",
            "skills://user@packages/~source-1/~reviewer/~SKILL.md",
            "skills://packages:42/~source-1/~reviewer/~SKILL.md",
            "skills://packages/~source-1/~reviewer/~SKILL.md#fragment",
        ] {
            assert!(
                parse_package_resource_uri(malformed).is_err(),
                "malformed resource {malformed:?} was accepted"
            );
        }
    }

    #[tokio::test]
    async fn read_returns_the_validated_skill_markdown() {
        let app = tempfile::tempdir().expect("app data");
        let root = tempfile::tempdir().expect("skill source");
        let skill = root.path().join("rust-reviewer");
        std::fs::create_dir(&skill).expect("skill package");
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: rust-reviewer\ndescription: Reviews unsafe Rust\n---\n# Rust Reviewer\n",
        )
        .expect("skill markdown");
        let state = Arc::new(AppState {
            app_data_dir: app.path().to_path_buf(),
            corpus_cache: Arc::new(Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(Mutex::new(())),
            skill_sources_write_lock: Arc::new(Mutex::new(())),
            skill_installs_write_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(RwLock::new(SettingsLoadState::FirstLaunch)),
            updater_state: crate::commands::updater::empty_state(),
        });
        let registered = super::super::add_local_source(&state, root.path())
            .await
            .expect("register source");

        let markdown =
            super::super::read_skill_file(&state, &registered.id, "rust-reviewer", "SKILL.md")
                .await
                .expect("read skill")
                .text;

        assert!(markdown
            .as_deref()
            .expect("text skill")
            .contains("# Rust Reviewer"));
    }

    #[tokio::test]
    async fn mcp_defaults_allow_reads_deny_each_mutation_class_and_audit_every_call() {
        let app = tempfile::tempdir().expect("app data");
        let source = tempfile::tempdir().expect("skill source");
        let additional_source = tempfile::tempdir().expect("additional source");
        let project = tempfile::tempdir().expect("project");
        let skill = source.path().join("reviewer");
        std::fs::create_dir(&skill).expect("skill package");
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: reviewer\ndescription: Reviews changes\n---\n# never-audit-this-content\n",
        )
        .expect("skill markdown");
        let state = Arc::new(AppState {
            app_data_dir: app.path().to_path_buf(),
            corpus_cache: Arc::new(Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(Mutex::new(())),
            skill_sources_write_lock: Arc::new(Mutex::new(())),
            skill_installs_write_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(RwLock::new(SettingsLoadState::FirstLaunch)),
            updater_state: crate::commands::updater::empty_state(),
        });
        let registered = super::super::add_local_source(&state, source.path())
            .await
            .expect("register source");
        let installed = super::super::install_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(project.path().to_string_lossy().as_ref()),
        )
        .await
        .expect("seed managed install");
        let responses = call_tools_over_stdio(
            Arc::clone(&state),
            vec![
                serde_json::json!({
                    "name": "skills_search",
                    "arguments": {"query": "reviewer"},
                }),
                serde_json::json!({
                    "name": "skills_add_local_source",
                    "arguments": {"root": additional_source.path()},
                }),
                serde_json::json!({
                    "name": "skills_install",
                    "arguments": {
                        "source_id": registered.id,
                        "relative_path": "reviewer",
                        "runtime": "codex",
                    },
                }),
                serde_json::json!({
                    "name": "skills_disable",
                    "arguments": {
                        "source_id": installed.source_id,
                        "relative_path": installed.relative_path,
                        "runtime": "codex",
                        "project_path": installed.project_path,
                    },
                }),
            ],
        )
        .await;
        assert!(responses[0].get("error").is_none(), "{}", responses[0]);
        assert_ne!(responses[0]["result"]["isError"], true, "{}", responses[0]);
        assert!(responses[1..]
            .iter()
            .all(|response| response["error"].is_object()));

        let audit = crate::state::load_mcp_audit(app.path())
            .await
            .expect("load audit");
        assert_eq!(audit.len(), 8);
        assert_eq!(
            audit
                .iter()
                .filter(|entry| entry.phase == "terminal" && entry.success)
                .count(),
            1
        );
        let raw =
            std::fs::read_to_string(app.path().join("state/mcp-audit.jsonl")).expect("audit jsonl");
        assert!(!raw.contains("never-audit-this-content"), "{raw}");
    }

    #[tokio::test]
    async fn raw_tool_boundary_audits_typed_decode_and_unknown_tool_failures() {
        let app = tempfile::tempdir().expect("app data");
        let state = Arc::new(AppState {
            app_data_dir: app.path().to_path_buf(),
            corpus_cache: Arc::new(Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(Mutex::new(())),
            skill_sources_write_lock: Arc::new(Mutex::new(())),
            skill_installs_write_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(RwLock::new(SettingsLoadState::FirstLaunch)),
            updater_state: crate::commands::updater::empty_state(),
        });

        let responses = call_tools_over_stdio(
            state,
            vec![
                serde_json::json!({
                    "name": "skills_search",
                    "arguments": {"query": 42},
                }),
                serde_json::json!({
                    "name": "skills_unknown",
                    "arguments": {},
                }),
            ],
        )
        .await;
        assert_eq!(responses[0]["result"]["isError"], true, "{}", responses[0]);
        assert!(responses[1]["error"].is_object(), "{}", responses[1]);
        assert!(
            responses[1]["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("unclassified MCP tool")),
            "unclassified tools must fail at the policy boundary before router dispatch: {}",
            responses[1]
        );

        let audit = crate::state::load_mcp_audit(app.path())
            .await
            .expect("load audit");
        assert_eq!(audit.len(), 4, "{audit:?}");
        for tool in ["skills_search", "skills_unknown"] {
            let entries = audit
                .iter()
                .filter(|entry| entry.tool == tool)
                .collect::<Vec<_>>();
            assert_eq!(entries.len(), 2, "{tool}: {audit:?}");
            assert_eq!(entries[0].phase, "terminal");
            assert_eq!(entries[1].phase, "attempt");
            assert_eq!(entries[0].id, entries[1].id);
            assert!(!entries[0].success);
        }
    }

    #[tokio::test]
    async fn denied_project_paths_are_never_written_to_the_audit() {
        let app = tempfile::tempdir().expect("app data");
        let allowed = tempfile::tempdir().expect("allowed project");
        let denied = tempfile::tempdir().expect("denied project");
        let allowed = std::fs::canonicalize(allowed.path())
            .expect("canonical allowed project")
            .to_string_lossy()
            .into_owned();
        crate::commands::settings::persist(
            app.path(),
            Settings {
                mcp_install_access: true,
                mcp_project_allowlist: vec![allowed],
                ..Settings::default()
            },
        )
        .await
        .expect("persist MCP settings");
        let state = Arc::new(AppState {
            app_data_dir: app.path().to_path_buf(),
            corpus_cache: Arc::new(Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(Mutex::new(())),
            skill_sources_write_lock: Arc::new(Mutex::new(())),
            skill_installs_write_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(RwLock::new(SettingsLoadState::FirstLaunch)),
            updater_state: crate::commands::updater::empty_state(),
        });
        let denied = denied.path().to_string_lossy().into_owned();

        let responses = call_tools_over_stdio(
            state,
            vec![serde_json::json!({
                "name": "skills_install",
                "arguments": {
                    "source_id": "never-resolved",
                    "relative_path": "reviewer",
                    "runtime": "codex",
                    "project_path": denied,
                },
            })],
        )
        .await;
        assert!(responses[0]["error"].is_object(), "{}", responses[0]);

        let audit = crate::state::load_mcp_audit(app.path())
            .await
            .expect("load audit");
        assert_eq!(audit.len(), 2);
        assert!(audit.iter().all(|entry| entry.project_path.is_none()));
        let raw =
            std::fs::read_to_string(app.path().join("state/mcp-audit.jsonl")).expect("audit JSONL");
        assert!(!raw.contains(&denied), "{raw}");
    }

    #[tokio::test]
    async fn audit_preflight_lock_failure_prevents_the_mutation() {
        let app = tempfile::tempdir().expect("app data");
        let source = tempfile::tempdir().expect("source");
        crate::commands::settings::persist(
            app.path(),
            Settings {
                mcp_source_access: true,
                ..Settings::default()
            },
        )
        .await
        .expect("persist MCP settings");
        let state = Arc::new(AppState {
            app_data_dir: app.path().to_path_buf(),
            corpus_cache: Arc::new(Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(Mutex::new(())),
            skill_sources_write_lock: Arc::new(Mutex::new(())),
            skill_installs_write_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(RwLock::new(SettingsLoadState::FirstLaunch)),
            updater_state: crate::commands::updater::empty_state(),
        });
        let lock_path = app.path().join("state/mcp-audit.lock");
        std::fs::create_dir_all(lock_path.parent().expect("lock parent")).expect("state dir");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(lock_path)
            .expect("open audit lock");
        lock.lock().expect("hold audit lock");

        let responses = call_tools_over_stdio(
            Arc::clone(&state),
            vec![serde_json::json!({
                "name": "skills_add_local_source",
                "arguments": {"root": source.path()},
            })],
        )
        .await;
        assert!(responses[0]["error"].is_object(), "{}", responses[0]);
        drop(lock);
        assert!(
            super::super::load_skill_sources(app.path())
                .await
                .expect("load sources")
                .is_empty(),
            "mutation ran without a durable audit attempt"
        );
    }

    #[tokio::test]
    async fn terminal_audit_failure_returns_the_completed_result_to_prevent_a_retry() {
        let app = tempfile::tempdir().expect("app data");
        let source = tempfile::tempdir().expect("source");
        crate::commands::settings::persist(
            app.path(),
            Settings {
                mcp_source_access: true,
                ..Settings::default()
            },
        )
        .await
        .expect("persist MCP settings");
        let state = Arc::new(AppState {
            app_data_dir: app.path().to_path_buf(),
            corpus_cache: Arc::new(Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(Mutex::new(())),
            skill_sources_write_lock: Arc::new(Mutex::new(())),
            skill_installs_write_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(RwLock::new(SettingsLoadState::FirstLaunch)),
            updater_state: crate::commands::updater::empty_state(),
        });
        crate::state::inject_mcp_audit_failure_after(app.path(), 1);

        let responses = call_tools_over_stdio(
            state,
            vec![serde_json::json!({
                "name": "skills_add_local_source",
                "arguments": {"root": source.path()},
            })],
        )
        .await;
        assert!(responses[0].get("error").is_none(), "{}", responses[0]);
        assert_ne!(responses[0]["result"]["isError"], true, "{}", responses[0]);
        assert_eq!(
            super::super::load_skill_sources(app.path())
                .await
                .expect("load sources")
                .len(),
            1
        );
        let audit = crate::state::load_mcp_audit(app.path())
            .await
            .expect("load audit");
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].phase, "attempt");
        assert!(!audit[0].success);
    }

    #[tokio::test]
    async fn find_and_install_requires_one_exact_normalized_match() {
        let app = tempfile::tempdir().expect("app data");
        let first_source = tempfile::tempdir().expect("first source");
        let second_source = tempfile::tempdir().expect("second source");
        let project = tempfile::tempdir().expect("project");
        for source in [first_source.path(), second_source.path()] {
            let skill = source.join("reviewer");
            std::fs::create_dir(&skill).expect("skill package");
            std::fs::write(
                skill.join("SKILL.md"),
                "---\nname: reviewer\ndescription: Reviews changes\n---\n# Reviewer\n",
            )
            .expect("skill markdown");
        }
        let canonical_project = std::fs::canonicalize(project.path())
            .expect("canonical project")
            .to_string_lossy()
            .into_owned();
        let state = Arc::new(AppState {
            app_data_dir: app.path().to_path_buf(),
            corpus_cache: Arc::new(Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(Mutex::new(())),
            skill_sources_write_lock: Arc::new(Mutex::new(())),
            skill_installs_write_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(RwLock::new(SettingsLoadState::Loaded(
                crate::commands::settings::Settings {
                    mcp_install_access: true,
                    mcp_project_allowlist: vec![canonical_project.clone()],
                    ..crate::commands::settings::Settings::default()
                },
            ))),
            updater_state: crate::commands::updater::empty_state(),
        });
        super::super::add_local_source(&state, first_source.path())
            .await
            .expect("register first source");
        let server = SkillMcpServer::new(Arc::clone(&state));
        let installed: serde_json::Value = serde_json::from_str(
            &server
                .skills_find_and_install(
                    Parameters(FindAndInstallRequest {
                        name: " Reviewer ".into(),
                        runtime: SkillRuntime::Codex,
                        project_path: Some(canonical_project.clone()),
                    }),
                    Extension(McpProjectAuthorization(None)),
                )
                .await
                .expect("unique exact install"),
        )
        .expect("install response");
        assert_eq!(installed["installed"]["state"], "current");
        assert_eq!(installed["candidates"], serde_json::json!([]));

        super::super::add_local_source(&state, second_source.path())
            .await
            .expect("register second source");
        let ambiguous: serde_json::Value = serde_json::from_str(
            &server
                .skills_find_and_install(
                    Parameters(FindAndInstallRequest {
                        name: "reviewer".into(),
                        runtime: SkillRuntime::Codex,
                        project_path: Some(canonical_project.clone()),
                    }),
                    Extension(McpProjectAuthorization(None)),
                )
                .await
                .expect("ambiguous response"),
        )
        .expect("ambiguous JSON");
        assert!(ambiguous["installed"].is_null());
        assert_eq!(
            ambiguous["candidates"]
                .as_array()
                .expect("candidates")
                .len(),
            2
        );

        let missing: serde_json::Value = serde_json::from_str(
            &server
                .skills_find_and_install(
                    Parameters(FindAndInstallRequest {
                        name: "missing".into(),
                        runtime: SkillRuntime::Codex,
                        project_path: Some(canonical_project),
                    }),
                    Extension(McpProjectAuthorization(None)),
                )
                .await
                .expect("missing response"),
        )
        .expect("missing JSON");
        assert!(missing["installed"].is_null());
        assert!(missing["candidates"].is_array());
    }

    #[test]
    fn mcp_exposes_the_skill_catalog_and_source_tools() {
        let root = tempfile::tempdir().expect("app data");
        let server = SkillMcpServer::new(Arc::new(AppState {
            app_data_dir: root.path().to_path_buf(),
            corpus_cache: Arc::new(Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(Mutex::new(())),
            skill_sources_write_lock: Arc::new(Mutex::new(())),
            skill_installs_write_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(RwLock::new(SettingsLoadState::FirstLaunch)),
            updater_state: crate::commands::updater::empty_state(),
        }));
        let mut names = server
            .tool_router
            .list_all()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect::<Vec<_>>();
        names.sort();
        assert!(
            names.iter().all(|name| action_for_tool(name).is_some()),
            "every routed tool must have an explicit audit/policy class"
        );

        assert_eq!(
            names,
            [
                "skills_add_github_source",
                "skills_add_local_source",
                "skills_disable",
                "skills_enable",
                "skills_find_and_install",
                "skills_get",
                "skills_get_draft",
                "skills_get_file",
                "skills_install",
                "skills_installed",
                "skills_list_drafts",
                "skills_list_files",
                "skills_list_sources",
                "skills_recommend",
                "skills_refresh_all",
                "skills_refresh_source",
                "skills_remove_source",
                "skills_search",
                "skills_source_status",
                "skills_submit_draft",
                "skills_uninstall",
                "skills_update",
            ]
        );
    }

    #[tokio::test]
    async fn mcp_installs_a_skill_at_the_requested_project_runtime() {
        let app = tempfile::tempdir().expect("app data");
        let source = tempfile::tempdir().expect("skill source");
        let project = tempfile::tempdir().expect("project");
        let skill = source.path().join("reviewer");
        std::fs::create_dir(&skill).expect("skill package");
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: reviewer\ndescription: Reviews changes\n---\n# Reviewer\n",
        )
        .expect("skill markdown");
        let canonical_project = std::fs::canonicalize(project.path())
            .expect("canonical project")
            .to_string_lossy()
            .into_owned();
        let state = Arc::new(AppState {
            app_data_dir: app.path().to_path_buf(),
            corpus_cache: Arc::new(Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(Mutex::new(())),
            skill_sources_write_lock: Arc::new(Mutex::new(())),
            skill_installs_write_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(RwLock::new(SettingsLoadState::FirstLaunch)),
            updater_state: crate::commands::updater::empty_state(),
        });
        crate::commands::settings::persist(
            app.path(),
            Settings {
                mcp_install_access: true,
                mcp_destructive_access: true,
                mcp_project_allowlist: vec![canonical_project.clone()],
                ..Settings::default()
            },
        )
        .await
        .expect("persist MCP settings");
        let registered = super::super::add_local_source(&state, source.path())
            .await
            .expect("register source");
        let (server_transport, client_transport) = tokio::io::duplex(4096);
        let server = SkillMcpServer::new(Arc::clone(&state));
        let server_task = tokio::spawn(async move {
            server
                .serve(server_transport)
                .await
                .expect("start server")
                .waiting()
                .await
                .expect("wait for server");
        });
        let (read, mut write) = tokio::io::split(client_transport);
        let mut read = BufReader::new(read);
        let mut line = String::new();
        write
            .write_all(
                br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#,
            )
            .await
            .expect("initialize request");
        write.write_all(b"\n").await.expect("initialize newline");
        tokio::time::timeout(Duration::from_secs(2), read.read_line(&mut line))
            .await
            .expect("initialize response timeout")
            .expect("initialize response");
        line.clear();
        write
            .write_all(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
            .await
            .expect("initialized notification");
        write.write_all(b"\n").await.expect("notification newline");

        write
            .write_all(br#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#)
            .await
            .expect("tool list request");
        write.write_all(b"\n").await.expect("tool list newline");
        tokio::time::timeout(Duration::from_secs(2), read.read_line(&mut line))
            .await
            .expect("tool list response timeout")
            .expect("tool list response");
        let tools: serde_json::Value = serde_json::from_str(&line).expect("tools JSON");
        let install = tools["result"]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .find(|tool| tool["name"] == "skills_install")
            .expect("skills_install tool");
        assert_eq!(
            install["inputSchema"]["required"],
            serde_json::json!(["source_id", "relative_path", "runtime"])
        );
        assert!(install["inputSchema"]["properties"]["project_path"]["type"]
            .as_array()
            .expect("optional project path types")
            .iter()
            .any(|value| value == "string"));
        assert_eq!(
            install["inputSchema"]["$defs"]["SkillRuntime"]["enum"],
            serde_json::json!(["claudeCode", "codex"])
        );
        line.clear();

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "skills_install",
                "arguments": {
                    "source_id": registered.id,
                    "relative_path": "reviewer",
                    "runtime": "codex",
                    "project_path": project.path(),
                }
            }
        });
        write
            .write_all(request.to_string().as_bytes())
            .await
            .expect("install request");
        write.write_all(b"\n").await.expect("install newline");
        tokio::time::timeout(Duration::from_secs(2), read.read_line(&mut line))
            .await
            .expect("install response timeout")
            .expect("install response");
        let response: serde_json::Value = serde_json::from_str(&line).expect("install JSON");
        assert!(response.get("error").is_none(), "{response}");
        assert!(project
            .path()
            .join(".agents/skills/reviewer/SKILL.md")
            .is_file());
        line.clear();

        for (id, name, arguments) in [
            (
                4,
                "skills_installed",
                serde_json::json!({"project_paths": [project.path()]}),
            ),
            (
                5,
                "skills_update",
                serde_json::json!({
                    "source_id": registered.id,
                    "relative_path": "reviewer",
                    "runtime": "codex",
                    "project_path": project.path(),
                }),
            ),
            (
                6,
                "skills_disable",
                serde_json::json!({
                    "source_id": registered.id,
                    "relative_path": "reviewer",
                    "runtime": "codex",
                    "project_path": project.path(),
                }),
            ),
            (
                7,
                "skills_enable",
                serde_json::json!({
                    "source_id": registered.id,
                    "relative_path": "reviewer",
                    "runtime": "codex",
                    "project_path": project.path(),
                }),
            ),
            (
                8,
                "skills_uninstall",
                serde_json::json!({
                    "source_id": registered.id,
                    "relative_path": "reviewer",
                    "runtime": "codex",
                    "project_path": project.path(),
                }),
            ),
        ] {
            let request = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": {"name": name, "arguments": arguments},
            });
            write
                .write_all(request.to_string().as_bytes())
                .await
                .expect("lifecycle request");
            write.write_all(b"\n").await.expect("lifecycle newline");
            tokio::time::timeout(Duration::from_secs(2), read.read_line(&mut line))
                .await
                .expect("lifecycle response timeout")
                .expect("lifecycle response");
            let response: serde_json::Value = serde_json::from_str(&line).expect("lifecycle JSON");
            assert!(response.get("error").is_none(), "{name}: {response}");
            assert_ne!(response["result"]["isError"], true, "{name}: {response}");
            assert!(
                response["result"]["content"].is_array(),
                "{name}: {response}"
            );
            let content: serde_json::Value = serde_json::from_str(
                response["result"]["content"][0]["text"]
                    .as_str()
                    .expect("tool text content"),
            )
            .expect("tool JSON content");
            match name {
                "skills_installed" => assert_eq!(content[0]["state"], "current"),
                "skills_update" | "skills_enable" => assert_eq!(content["state"], "current"),
                "skills_disable" => assert_eq!(content["state"], "disabled"),
                "skills_uninstall" => assert_eq!(content, true),
                _ => unreachable!("unexpected lifecycle tool"),
            }
            line.clear();
        }
        assert!(!project.path().join(".agents/skills/reviewer").exists());

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "tools/call",
            "params": {"name": "skills_disable", "arguments": {
                "source_id": registered.id,
                "relative_path": "reviewer",
                "runtime": "codex",
                "project_path": project.path(),
            }},
        });
        write
            .write_all(request.to_string().as_bytes())
            .await
            .expect("failure request");
        write.write_all(b"\n").await.expect("failure newline");
        tokio::time::timeout(Duration::from_secs(2), read.read_line(&mut line))
            .await
            .expect("failure response timeout")
            .expect("failure response");
        let failure: serde_json::Value = serde_json::from_str(&line).expect("failure JSON");
        assert_eq!(failure["result"]["isError"], true, "{failure}");

        server_task.abort();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn mcp_project_capability_survives_root_retarget_before_uninstall_use() {
        use std::os::unix::fs::symlink;

        let app = tempfile::tempdir().expect("app data");
        let source = tempfile::tempdir().expect("skill source");
        let projects = tempfile::tempdir().expect("projects");
        let project = projects.path().join("project");
        let displaced_project = projects.path().join("project-original");
        let attacker = projects.path().join("attacker");
        std::fs::create_dir(&project).expect("project");
        std::fs::create_dir_all(attacker.join(".agents/skills/reviewer"))
            .expect("attacker destination");
        std::fs::write(
            attacker.join(".agents/skills/reviewer/SKILL.md"),
            b"attacker content must remain",
        )
        .expect("attacker content");
        let skill = source.path().join("reviewer");
        std::fs::create_dir(&skill).expect("skill package");
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: reviewer\ndescription: Reviews changes\n---\n# Reviewer\n",
        )
        .expect("skill markdown");
        let canonical_project = std::fs::canonicalize(&project)
            .expect("canonical project")
            .to_string_lossy()
            .into_owned();
        crate::commands::settings::persist(
            app.path(),
            Settings {
                mcp_destructive_access: true,
                mcp_project_allowlist: vec![canonical_project.clone()],
                ..Settings::default()
            },
        )
        .await
        .expect("persist MCP settings");
        let state = Arc::new(AppState {
            app_data_dir: app.path().to_path_buf(),
            corpus_cache: Arc::new(Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(Mutex::new(())),
            skill_sources_write_lock: Arc::new(Mutex::new(())),
            skill_installs_write_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(RwLock::new(SettingsLoadState::FirstLaunch)),
            updater_state: crate::commands::updater::empty_state(),
        });
        let registered = super::super::add_local_source(&state, source.path())
            .await
            .expect("register source");
        let installed = super::super::install_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&canonical_project),
        )
        .await
        .expect("seed project install");
        let target = std::path::PathBuf::from(&installed.path);
        let project_for_probe = project.clone();
        let displaced_for_probe = displaced_project.clone();
        let attacker_for_probe = attacker.clone();
        super::super::set_uninstall_before_quarantine_probe(
            target,
            Box::new(move |_| {
                std::fs::rename(&project_for_probe, &displaced_for_probe)
                    .expect("displace authorized project");
                symlink(&attacker_for_probe, &project_for_probe).expect("retarget project path");
            }),
        );

        let responses = call_tools_over_stdio(
            state,
            vec![serde_json::json!({
                "name": "skills_uninstall",
                "arguments": {
                    "source_id": registered.id,
                    "relative_path": "reviewer",
                    "runtime": "codex",
                    "project_path": canonical_project,
                },
            })],
        )
        .await;
        assert!(responses[0].get("error").is_none(), "{}", responses[0]);
        assert_ne!(responses[0]["result"]["isError"], true, "{}", responses[0]);
        assert_eq!(
            std::fs::read(attacker.join(".agents/skills/reviewer/SKILL.md"))
                .expect("attacker content remains"),
            b"attacker content must remain",
            "project mutation escaped the verified directory capability"
        );
        assert!(
            !displaced_project.join(".agents/skills/reviewer").exists(),
            "the authorized original project should be mutated through its stable capability"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn mcp_project_capability_survives_internal_ancestor_retarget() {
        use std::os::unix::fs::symlink;

        let app = tempfile::tempdir().expect("app data");
        let source = tempfile::tempdir().expect("skill source");
        let project = tempfile::tempdir().expect("project");
        let attacker = tempfile::tempdir().expect("attacker");
        std::fs::create_dir_all(attacker.path().join("skills/reviewer"))
            .expect("attacker destination");
        std::fs::write(
            attacker.path().join("skills/reviewer/SKILL.md"),
            b"attacker content must remain",
        )
        .expect("attacker content");
        let skill = source.path().join("reviewer");
        std::fs::create_dir(&skill).expect("skill package");
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: reviewer\ndescription: Reviews changes\n---\n# Reviewer\n",
        )
        .expect("skill markdown");
        let canonical_project = std::fs::canonicalize(project.path())
            .expect("canonical project")
            .to_string_lossy()
            .into_owned();
        crate::commands::settings::persist(
            app.path(),
            Settings {
                mcp_destructive_access: true,
                mcp_project_allowlist: vec![canonical_project.clone()],
                ..Settings::default()
            },
        )
        .await
        .expect("persist MCP settings");
        let state = Arc::new(AppState {
            app_data_dir: app.path().to_path_buf(),
            corpus_cache: Arc::new(Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(Mutex::new(())),
            skill_sources_write_lock: Arc::new(Mutex::new(())),
            skill_installs_write_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(RwLock::new(SettingsLoadState::FirstLaunch)),
            updater_state: crate::commands::updater::empty_state(),
        });
        let registered = super::super::add_local_source(&state, source.path())
            .await
            .expect("register source");
        let installed = super::super::install_skill(
            &state,
            &registered.id,
            "reviewer",
            "codex",
            Some(&canonical_project),
        )
        .await
        .expect("seed project install");
        let target = std::path::PathBuf::from(&installed.path);
        let agents = project.path().join(".agents");
        let displaced_agents = project.path().join(".agents-original");
        let attacker_path = attacker.path().to_path_buf();
        let agents_for_probe = agents.clone();
        let displaced_for_probe = displaced_agents.clone();
        super::super::set_uninstall_before_quarantine_probe(
            target,
            Box::new(move |_| {
                std::fs::rename(&agents_for_probe, &displaced_for_probe)
                    .expect("displace authorized internal ancestor");
                symlink(&attacker_path, &agents_for_probe).expect("retarget internal ancestor");
            }),
        );

        let responses = call_tools_over_stdio(
            state,
            vec![serde_json::json!({
                "name": "skills_uninstall",
                "arguments": {
                    "source_id": registered.id,
                    "relative_path": "reviewer",
                    "runtime": "codex",
                    "project_path": canonical_project,
                },
            })],
        )
        .await;
        assert_ne!(responses[0]["result"]["isError"], true, "{}", responses[0]);
        assert_eq!(
            std::fs::read(attacker.path().join("skills/reviewer/SKILL.md"))
                .expect("attacker content remains"),
            b"attacker content must remain"
        );
        assert!(
            !displaced_agents.join("skills/reviewer").exists(),
            "the retained no-follow parent handle must target the original tree"
        );
    }

    #[tokio::test]
    async fn mcp_lists_and_reads_the_validated_skill_resources() {
        let app = tempfile::tempdir().expect("app data");
        let source_parent = tempfile::tempdir().expect("source parent");
        let source = source_parent.path().join("root-skill");
        std::fs::create_dir_all(&source).expect("root skill package");
        std::fs::write(
            source.join("SKILL.md"),
            "---\nname: root-skill\ndescription: Reads root resources\n---\n# Root Skill\n",
        )
        .expect("root skill markdown");
        std::fs::write(source.join("blob.png"), [0, 159, 146, 150]).expect("binary file");
        let nested = source.join("nested/reviewer");
        std::fs::create_dir_all(nested.join("references")).expect("nested package");
        std::fs::write(
            nested.join("SKILL.md"),
            "---\nname: reviewer\ndescription: Reviews changes\n---\n# Reviewer\n",
        )
        .expect("nested skill markdown");
        std::fs::write(nested.join("references/a+b.txt"), b"literal plus\n")
            .expect("literal plus file");
        let state = Arc::new(AppState {
            app_data_dir: app.path().to_path_buf(),
            corpus_cache: Arc::new(Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(Mutex::new(())),
            skill_sources_write_lock: Arc::new(Mutex::new(())),
            skill_installs_write_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(RwLock::new(SettingsLoadState::FirstLaunch)),
            updater_state: crate::commands::updater::empty_state(),
        });
        let registered = super::super::add_local_source(&state, &source)
            .await
            .expect("register source");
        let (server_transport, client_transport) = tokio::io::duplex(4096);
        let server = SkillMcpServer::new(Arc::clone(&state));
        let server_task = tokio::spawn(async move {
            server
                .serve(server_transport)
                .await
                .expect("start server")
                .waiting()
                .await
                .expect("wait for server");
        });
        let (read, mut write) = tokio::io::split(client_transport);
        let mut read = BufReader::new(read);
        let mut line = String::new();
        write
            .write_all(
                br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#,
            )
            .await
            .expect("initialize request");
        write.write_all(b"\n").await.expect("initialize newline");
        tokio::time::timeout(Duration::from_secs(2), read.read_line(&mut line))
            .await
            .expect("initialize response timeout")
            .expect("initialize response");
        let initialize: serde_json::Value = serde_json::from_str(&line).expect("initialize JSON");
        assert!(initialize["result"]["capabilities"]["resources"].is_object());
        line.clear();
        write
            .write_all(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
            .await
            .expect("initialized notification");
        write.write_all(b"\n").await.expect("notification newline");

        write
            .write_all(br#"{"jsonrpc":"2.0","id":2,"method":"resources/list","params":{}}"#)
            .await
            .expect("resource list request");
        write.write_all(b"\n").await.expect("list newline");
        tokio::time::timeout(Duration::from_secs(2), read.read_line(&mut line))
            .await
            .expect("resource list response timeout")
            .expect("resource list response");
        let resources: serde_json::Value = serde_json::from_str(&line).expect("resource list JSON");
        assert!(resources["result"]["resources"]
            .as_array()
            .expect("resources array")
            .iter()
            .any(|resource| resource["uri"] == "skills://catalog"));
        assert!(resources["result"]["resources"]
            .as_array()
            .expect("resources array")
            .iter()
            .any(|resource| {
                resource["uri"] == package_resource_uri(&registered.id, ".", "SKILL.md")
            }));
        line.clear();

        write
            .write_all(
                br#"{"jsonrpc":"2.0","id":3,"method":"resources/templates/list","params":{}}"#,
            )
            .await
            .expect("resource templates request");
        write.write_all(b"\n").await.expect("templates newline");
        tokio::time::timeout(Duration::from_secs(2), read.read_line(&mut line))
            .await
            .expect("resource templates response timeout")
            .expect("resource templates response");
        let templates: serde_json::Value = serde_json::from_str(&line).expect("templates JSON");
        let advertised_template = templates["result"]["resourceTemplates"]
            .as_array()
            .expect("templates array")
            .iter()
            .find_map(|template| template["uriTemplate"].as_str())
            .expect("skill package resource template");
        let template_read_uri = advertised_template
            .replace("{source_id}", &registered.id)
            .replace("{relative_path}", ".")
            .replace("{file_path}", "SKILL.md");
        line.clear();

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "resources/read",
            "params": {"uri": template_read_uri},
        });
        write
            .write_all(request.to_string().as_bytes())
            .await
            .expect("resource read request");
        write.write_all(b"\n").await.expect("read newline");
        tokio::time::timeout(Duration::from_secs(2), read.read_line(&mut line))
            .await
            .expect("resource read response timeout")
            .expect("resource read response");
        let response: serde_json::Value = serde_json::from_str(&line).expect("resource read JSON");
        assert!(response["result"]["contents"][0]["text"]
            .as_str()
            .expect("skill resource text")
            .contains("# Root Skill"));
        line.clear();

        for (id, uri, expected_text) in [(
            5,
            package_resource_uri(&registered.id, "nested/reviewer", "references/a+b.txt"),
            "literal plus",
        )] {
            let request = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "resources/read",
                "params": {"uri": uri},
            });
            write
                .write_all(request.to_string().as_bytes())
                .await
                .expect("nested resource read request");
            write.write_all(b"\n").await.expect("nested read newline");
            tokio::time::timeout(Duration::from_secs(2), read.read_line(&mut line))
                .await
                .expect("nested resource read timeout")
                .expect("nested resource read response");
            let response: serde_json::Value =
                serde_json::from_str(&line).expect("nested resource read JSON");
            assert!(response["result"]["contents"][0]["text"]
                .as_str()
                .expect("nested resource text")
                .contains(expected_text));
            line.clear();
        }

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "resources/read",
            "params": {"uri": package_resource_uri(&registered.id, ".", "blob.png")},
        });
        write
            .write_all(request.to_string().as_bytes())
            .await
            .expect("binary resource read request");
        write.write_all(b"\n").await.expect("binary read newline");
        tokio::time::timeout(Duration::from_secs(2), read.read_line(&mut line))
            .await
            .expect("binary resource read timeout")
            .expect("binary resource read response");
        let binary: serde_json::Value = serde_json::from_str(&line).expect("binary resource JSON");
        assert_eq!(
            binary["result"]["contents"][0]["mimeType"],
            "application/octet-stream"
        );
        assert_eq!(binary["result"]["contents"][0]["blob"], "AJ+Slg==");
        line.clear();

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "resources/read",
            "params": {"uri": package_resource_uri(&registered.id, ".", "../SKILL.md")},
        });
        write
            .write_all(request.to_string().as_bytes())
            .await
            .expect("traversal resource read request");
        write
            .write_all(b"\n")
            .await
            .expect("traversal read newline");
        tokio::time::timeout(Duration::from_secs(2), read.read_line(&mut line))
            .await
            .expect("traversal resource read timeout")
            .expect("traversal resource read response");
        let traversal: serde_json::Value =
            serde_json::from_str(&line).expect("traversal resource JSON");
        assert!(traversal["error"].is_object());

        server_task.abort();
    }
}
