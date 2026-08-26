use std::path::Path;

use rmcp::{
    handler::server::{tool::Extension, wrapper::Parameters},
    model::{ErrorData, Resource, ResourceContents, ResourceTemplate},
    schemars, tool, tool_router,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    library,
    skills::mcp::{decode_resource_component, encode_resource_component, SkillMcpServer},
    state::{AppState, McpAction, McpProjectAuthorization},
    types::{
        AgentApprovalAction, AgentCollection, AgentDraftInput, AgentPackageResult,
        AgentPreferredSource, AgentReference, AgentSmartFolder, AgentSmartFolderRule,
        AgentSourceKind, AgentSourceResult, AgentUpdatePolicy, AgentWorkspaceProfile,
    },
};

pub(crate) const AGENT_CATALOG_URI: &str = "agents://catalog";
const MAX_AGENT_CATALOG_BYTES: usize = 8 * 1024 * 1024;
const MAX_AGENT_MCP_RESOURCES: usize = 16_384;
const MAX_AGENT_RENDER_BYTES: usize = 4 * 1024 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentCatalogResponse {
    catalog_revision: String,
    sources: Vec<AgentSourceResult>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchRequest {
    query: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReferenceRequest {
    source_id: String,
    relative_path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct FileRequest {
    source_id: String,
    relative_path: String,
    file_path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct LocalSourceRequest {
    root: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct GithubSourceRequest {
    repository: String,
    git_ref: Option<String>,
    subdirectory: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SourceRequest {
    source_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SourceApprovalRequest {
    source_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct DraftInputRequest {
    relative_path: String,
    text: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateDraftRequest {
    relative_path: String,
    name: String,
    description: String,
    body: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct EditDraftRequest {
    id: String,
    relative_path: String,
    text: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct DraftRequest {
    id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateFromSkillRequest {
    source_id: String,
    relative_path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RecommendRequest {
    task: String,
    limit: Option<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentFileEntry {
    relative_path: String,
    size_bytes: u64,
    sha256: String,
}

#[derive(Serialize)]
struct DraftMetadata<'a> {
    name: &'a str,
    description: &'a str,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReferenceInput {
    source_id: String,
    relative_path: String,
}

impl From<ReferenceInput> for AgentReference {
    fn from(value: ReferenceInput) -> Self {
        Self {
            source_id: value.source_id,
            relative_path: value.relative_path,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct FolderRequest {
    path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RenameFolderRequest {
    path: String,
    new_name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct MoveFolderRequest {
    path: String,
    new_parent: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct DeleteFolderRequest {
    path: String,
    recursive: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct AssignFolderRequest {
    source_id: String,
    relative_path: String,
    folder_path: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct FavoriteRequest {
    source_id: String,
    relative_path: String,
    favorite: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct CollectionRequest {
    name: String,
    #[serde(default)]
    agents: Vec<ReferenceInput>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SmartFolderRequest {
    name: String,
    query: Option<String>,
    division: Option<String>,
    source_id: Option<String>,
    capability: Option<String>,
    lifecycle_state: Option<String>,
    installable: Option<bool>,
    favorite: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ProfileRequest {
    name: String,
    #[serde(default)]
    folders: Vec<String>,
    #[serde(default)]
    collections: Vec<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct NamedApprovalRequest {
    name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdatePolicyRequest {
    source_id: String,
    relative_path: String,
    policy: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct PreferredSourceRequest {
    agent_name: String,
    source_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct PublisherTrustRequest {
    name: String,
    public_key: String,
    trusted: bool,
    revoked: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SubmitApprovalRequest {
    request: ApprovalActionRequest,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct AgentLifecycleRequest {
    source_id: String,
    relative_path: String,
    tool: String,
    project_path: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct AgentInstallRequest {
    source_id: String,
    relative_path: String,
    tool: String,
    project_path: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct AgentPlanRequest {
    source_id: String,
    relative_path: String,
    tool: String,
    project_path: Option<String>,
    include_dependencies: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct AgentFindRequest {
    name: String,
    tool: String,
    project_path: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct AgentRollbackRequest {
    source_id: String,
    relative_path: String,
    tool: String,
    project_path: Option<String>,
    snapshot_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct AgentBatchRequest {
    collection_name: String,
    operation: String,
    tool: String,
    project_path: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct LockRequest {
    project_path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct LockApplyRequest {
    project_path: String,
    revision: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(
    tag = "action",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum ApprovalActionRequest {
    SourceRemove {
        source_id: String,
    },
    FolderDelete {
        path: String,
        recursive: bool,
    },
    CollectionDelete {
        name: String,
    },
    SmartFolderDelete {
        name: String,
    },
    ProfileDelete {
        name: String,
    },
    UpdatePolicySet {
        source_id: String,
        relative_path: String,
        policy: String,
    },
    PublisherTrustSet {
        name: String,
        public_key: String,
        trusted: bool,
        revoked: bool,
    },
    Install {
        source_id: String,
        relative_path: String,
        tool: String,
        project_path: Option<String>,
        include_dependencies: bool,
        plan_revision: String,
    },
    Update {
        source_id: String,
        relative_path: String,
        tool: String,
        project_path: Option<String>,
        plan_revision: String,
    },
    Uninstall {
        source_id: String,
        relative_path: String,
        tool: String,
        project_path: Option<String>,
        plan_revision: String,
    },
    Rollback {
        source_id: String,
        relative_path: String,
        tool: String,
        project_path: Option<String>,
        snapshot_id: String,
        plan_revision: String,
    },
    BatchCollection {
        collection_name: String,
        operation: String,
        tool: String,
        project_path: Option<String>,
        plan_revision: String,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AgentResource {
    Catalog,
    Source {
        source_id: String,
        relative_path: String,
    },
    Render {
        source_id: String,
        relative_path: String,
        tool: String,
    },
}

pub(crate) fn agent_resource_uri(source_id: &str, relative_path: &str) -> String {
    format!(
        "agents://agents/{}/{}",
        encode_resource_component(source_id),
        encode_resource_component(relative_path),
    )
}

#[allow(
    dead_code,
    reason = "canonical builder for templated Agent render resources"
)]
pub(crate) fn render_resource_uri(source_id: &str, relative_path: &str, tool: &str) -> String {
    format!(
        "agents://renders/{}/{}/{}",
        encode_resource_component(source_id),
        encode_resource_component(relative_path),
        encode_resource_component(tool),
    )
}

pub(crate) fn parse_agent_resource_uri(uri: &str) -> Result<AgentResource, ErrorData> {
    if uri == AGENT_CATALOG_URI {
        return Ok(AgentResource::Catalog);
    }
    let parsed = url::Url::parse(uri).map_err(|error| {
        ErrorData::invalid_params(format!("invalid Agent resource URI: {error}"), None)
    })?;
    if parsed.scheme() != "agents"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port().is_some()
    {
        return Err(ErrorData::invalid_params("unknown Agent resource", None));
    }
    let parts = parsed
        .path_segments()
        .ok_or_else(|| ErrorData::invalid_params("invalid Agent resource path", None))?
        .collect::<Vec<_>>();
    let expected = match parsed.host_str() {
        Some("agents") if parts.len() == 2 => false,
        Some("renders") if parts.len() == 3 => true,
        _ => return Err(ErrorData::invalid_params("unknown Agent resource", None)),
    };
    let source_id = decode_resource_component(parts[0])?;
    let relative_path = decode_resource_component(parts[1])?;
    if source_id.contains(['/', '\\']) {
        return Err(ErrorData::invalid_params("invalid Agent source id", None));
    }
    library::validate_reference(&source_id, &relative_path)
        .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
    if !expected {
        return Ok(AgentResource::Source {
            source_id,
            relative_path,
        });
    }
    let tool = decode_resource_component(parts[2])?;
    if tool.is_empty()
        || tool.contains(['/', '\\'])
        || !crate::registry::get(&tool).is_some_and(crate::registry::ToolMeta::installable)
    {
        return Err(ErrorData::invalid_params(
            "unsupported Agent render tool",
            None,
        ));
    }
    Ok(AgentResource::Render {
        source_id,
        relative_path,
        tool,
    })
}

pub(crate) fn agent_catalog_revision(results: &[AgentSourceResult]) -> String {
    let mut sources = results
        .iter()
        .map(|result| {
            let errors = result
                .errors
                .iter()
                .map(|error| (&error.code, &error.path, &error.message))
                .collect::<Vec<_>>();
            (&result.source.id, &result.revision, errors)
        })
        .collect::<Vec<_>>();
    sources.sort_by(|left, right| left.0.cmp(right.0));
    let bytes = serde_json::to_vec(&sources).expect("normalized Agent catalog serializes");
    format!("{:x}", Sha256::digest(bytes))
}

async fn catalog_response(state: &AppState) -> Result<AgentCatalogResponse, ErrorData> {
    let mut sources = super::inspect_agent_sources(&state.app_data_dir)
        .await
        .map_err(mcp_invalid)?;
    let catalog_revision = agent_catalog_revision(&sources);
    for source in &mut sources {
        for package in &mut source.agents {
            if let Some(agent) = &mut package.agent {
                agent.body.clear();
            }
        }
    }
    Ok(AgentCatalogResponse {
        catalog_revision,
        sources,
    })
}

pub(crate) async fn list_agent_resources(state: &AppState) -> Result<Vec<Resource>, ErrorData> {
    let catalog = super::inspect_agent_sources(&state.app_data_dir)
        .await
        .map_err(mcp_invalid)?;
    let count = catalog
        .iter()
        .map(|source| {
            source
                .agents
                .iter()
                .filter(|agent| agent.installable)
                .count()
        })
        .sum::<usize>();
    if count >= MAX_AGENT_MCP_RESOURCES {
        return Err(ErrorData::invalid_params(
            format!("Agent resource catalog exceeds the {MAX_AGENT_MCP_RESOURCES}-item limit"),
            None,
        ));
    }
    let mut resources = Vec::with_capacity(count + 1);
    resources.push(
        Resource::new(AGENT_CATALOG_URI, "Agents catalog")
            .with_description("Registered Agent sources and validated package metadata")
            .with_mime_type("application/json"),
    );
    for source in catalog {
        for package in source.agents.into_iter().filter(|agent| agent.installable) {
            resources.push(
                Resource::new(
                    agent_resource_uri(
                        &package.reference.source_id,
                        &package.reference.relative_path,
                    ),
                    package.reference.relative_path,
                )
                .with_mime_type("text/markdown"),
            );
        }
    }
    Ok(resources)
}

pub(crate) fn agent_resource_templates() -> Vec<ResourceTemplate> {
    vec![
        ResourceTemplate::new(
            "agents://agents/~{source_id}/~{relative_path}",
            "Agent source file",
        )
        .with_description("One exact validated Agent Markdown source"),
        ResourceTemplate::new(
            "agents://renders/~{source_id}/~{relative_path}/~{tool}",
            "Agent render preview",
        )
        .with_description("A deterministic preview for one supported tool"),
    ]
}

pub(crate) async fn read_agent_resource(
    state: &AppState,
    uri: &str,
) -> Result<ResourceContents, ErrorData> {
    match parse_agent_resource_uri(uri)? {
        AgentResource::Catalog => {
            let response = catalog_response(state).await?;
            let bytes = serde_json::to_vec_pretty(&response).map_err(mcp_invalid)?;
            if bytes.len() > MAX_AGENT_CATALOG_BYTES {
                return Err(ErrorData::invalid_params(
                    format!("Agent catalog exceeds the {MAX_AGENT_CATALOG_BYTES}-byte limit"),
                    None,
                ));
            }
            let text = String::from_utf8(bytes).expect("JSON is UTF-8");
            Ok(ResourceContents::text(text, uri).with_mime_type("application/json"))
        }
        AgentResource::Source {
            source_id,
            relative_path,
        } => {
            let reference = AgentReference {
                source_id,
                relative_path,
            };
            let package = super::resolve_agent_package(&state.app_data_dir, &reference)
                .await
                .map_err(mcp_invalid)?;
            if !package.installable {
                return Err(ErrorData::invalid_params(
                    "Agent source is not installable",
                    None,
                ));
            }
            let text = super::read_agent_text(&state.app_data_dir, &reference)
                .await
                .map_err(mcp_invalid)?;
            Ok(ResourceContents::text(text, uri).with_mime_type("text/markdown"))
        }
        AgentResource::Render {
            source_id,
            relative_path,
            tool,
        } => {
            let reference = AgentReference {
                source_id,
                relative_path,
            };
            let package = super::resolve_agent_package(&state.app_data_dir, &reference)
                .await
                .map_err(mcp_invalid)?;
            if !package.installable {
                return Err(ErrorData::invalid_params(
                    "Agent source is not installable",
                    None,
                ));
            }
            let agent = package.agent.ok_or_else(|| {
                ErrorData::invalid_params("Agent source has no validated metadata", None)
            })?;
            let source = super::read_agent_text(&state.app_data_dir, &reference)
                .await
                .map_err(mcp_invalid)?;
            let rendered = crate::render::render(&agent, &source, &tool).map_err(mcp_invalid)?;
            if rendered.len() > MAX_AGENT_RENDER_BYTES {
                return Err(ErrorData::invalid_params(
                    format!("Agent render exceeds the {MAX_AGENT_RENDER_BYTES}-byte limit"),
                    None,
                ));
            }
            Ok(ResourceContents::text(rendered, uri).with_mime_type("text/plain"))
        }
    }
}

fn mcp_invalid(error: impl std::fmt::Display) -> ErrorData {
    ErrorData::invalid_params(error.to_string(), None)
}

fn parse_update_policy(value: &str) -> Result<AgentUpdatePolicy, String> {
    match value {
        "notify" => Ok(AgentUpdatePolicy::Notify),
        "autoTrusted" => Ok(AgentUpdatePolicy::AutoTrusted),
        "pin" => Ok(AgentUpdatePolicy::Pin),
        "reviewScripts" => Ok(AgentUpdatePolicy::ReviewScripts),
        _ => Err("policy must be notify, autoTrusted, pin, or reviewScripts".into()),
    }
}

fn approval_action(request: ApprovalActionRequest) -> Result<AgentApprovalAction, String> {
    Ok(match request {
        ApprovalActionRequest::SourceRemove { source_id } => {
            AgentApprovalAction::SourceRemove { source_id }
        }
        ApprovalActionRequest::FolderDelete { path, recursive } => {
            AgentApprovalAction::FolderDelete { path, recursive }
        }
        ApprovalActionRequest::CollectionDelete { name } => {
            AgentApprovalAction::CollectionDelete { name }
        }
        ApprovalActionRequest::SmartFolderDelete { name } => {
            AgentApprovalAction::SmartFolderDelete { name }
        }
        ApprovalActionRequest::ProfileDelete { name } => {
            AgentApprovalAction::ProfileDelete { name }
        }
        ApprovalActionRequest::UpdatePolicySet {
            source_id,
            relative_path,
            policy,
        } => AgentApprovalAction::UpdatePolicySet {
            reference: AgentReference {
                source_id,
                relative_path,
            },
            policy: parse_update_policy(&policy)?,
        },
        ApprovalActionRequest::PublisherTrustSet {
            name,
            public_key,
            trusted,
            revoked,
        } => AgentApprovalAction::PublisherTrustSet {
            name,
            public_key,
            trusted,
            revoked,
        },
        ApprovalActionRequest::Install {
            source_id,
            relative_path,
            tool,
            project_path,
            include_dependencies,
            plan_revision,
        } => AgentApprovalAction::Install {
            reference: AgentReference {
                source_id,
                relative_path,
            },
            tool,
            project_path,
            include_dependencies,
            plan_revision,
        },
        ApprovalActionRequest::Update {
            source_id,
            relative_path,
            tool,
            project_path,
            plan_revision,
        } => AgentApprovalAction::Update {
            reference: AgentReference {
                source_id,
                relative_path,
            },
            tool,
            project_path,
            plan_revision,
        },
        ApprovalActionRequest::Uninstall {
            source_id,
            relative_path,
            tool,
            project_path,
            plan_revision,
        } => AgentApprovalAction::Uninstall {
            reference: AgentReference {
                source_id,
                relative_path,
            },
            tool,
            project_path,
            plan_revision,
        },
        ApprovalActionRequest::Rollback {
            source_id,
            relative_path,
            tool,
            project_path,
            snapshot_id,
            plan_revision,
        } => AgentApprovalAction::Rollback {
            reference: AgentReference {
                source_id,
                relative_path,
            },
            tool,
            project_path,
            snapshot_id,
            plan_revision,
        },
        ApprovalActionRequest::BatchCollection {
            collection_name,
            operation,
            tool,
            project_path,
            plan_revision,
        } => AgentApprovalAction::BatchCollection {
            collection_name,
            operation,
            tool,
            project_path,
            plan_revision,
        },
    })
}

fn authorized_project_matches(
    project_path: &Option<String>,
    authorization: &McpProjectAuthorization,
) -> Result<(), String> {
    match (project_path.as_deref(), authorization.0.as_ref()) {
        (None, None) => Ok(()),
        (Some(path), Some(capability)) if path == capability.identity() => Ok(()),
        _ => Err("MCP project capability does not match the requested scope".into()),
    }
}

fn sanitized_package(mut package: AgentPackageResult) -> AgentPackageResult {
    if let Some(agent) = &mut package.agent {
        agent.body.clear();
    }
    package
}

fn sanitized_source(mut source: AgentSourceResult) -> AgentSourceResult {
    source.agents = source.agents.into_iter().map(sanitized_package).collect();
    source
}

fn search_agents(
    results: &[AgentSourceResult],
    query: &str,
) -> Result<Vec<AgentPackageResult>, String> {
    if query.len() > 2_048 {
        return Err("query exceeds the 2048-byte limit".into());
    }
    let query = query.to_lowercase();
    Ok(results
        .iter()
        .flat_map(|source| &source.agents)
        .filter(|package| package.installable)
        .filter(|package| {
            package
                .agent
                .iter()
                .flat_map(|agent| [&agent.name, &agent.description, &agent.category])
                .chain(package.groups.iter())
                .chain(package.tags.iter())
                .chain(package.capabilities.iter())
                .any(|value| value.to_lowercase().contains(&query))
        })
        .cloned()
        .map(sanitized_package)
        .collect())
}

async fn refresh_agent_source(
    state: &AppState,
    source_id: &str,
) -> Result<AgentSourceResult, crate::error::AppError> {
    let source = super::source_by_id(&state.app_data_dir, source_id).await?;
    if matches!(source.kind, AgentSourceKind::Github { .. }) {
        super::refresh_git_source(state, source_id).await
    } else {
        super::discover_agent_source(&state.app_data_dir, source).await
    }
}

async fn validate_approval_target(
    state: &AppState,
    action: &AgentApprovalAction,
) -> Result<(), String> {
    match action {
        AgentApprovalAction::SourceRemove { source_id } => {
            super::source_by_id(&state.app_data_dir, source_id)
                .await
                .map(|_| ())
                .map_err(|error| error.to_string())
        }
        AgentApprovalAction::FolderDelete { path, .. } => {
            let value = super::organize::list(state)
                .await
                .map_err(|error| error.to_string())?;
            value
                .folders
                .contains(path)
                .then_some(())
                .ok_or_else(|| format!("Agent folder not found: {path}"))
        }
        AgentApprovalAction::CollectionDelete { name }
        | AgentApprovalAction::SmartFolderDelete { name }
        | AgentApprovalAction::ProfileDelete { name } => {
            let value = super::organize::list(state)
                .await
                .map_err(|error| error.to_string())?;
            let exists = match action {
                AgentApprovalAction::CollectionDelete { .. } => {
                    value.collections.iter().any(|item| item.name == *name)
                }
                AgentApprovalAction::SmartFolderDelete { .. } => {
                    value.smart_folders.iter().any(|item| item.name == *name)
                }
                AgentApprovalAction::ProfileDelete { .. } => {
                    value.profiles.iter().any(|item| item.name == *name)
                }
                _ => unreachable!(),
            };
            exists
                .then_some(())
                .ok_or_else(|| format!("Agent library item not found: {name}"))
        }
        AgentApprovalAction::UpdatePolicySet { reference, .. }
        | AgentApprovalAction::Install { reference, .. }
        | AgentApprovalAction::Update { reference, .. } => {
            super::resolve_agent_package(&state.app_data_dir, reference)
                .await
                .map(|_| ())
                .map_err(|error| error.to_string())
        }
        AgentApprovalAction::Uninstall {
            reference,
            tool,
            project_path,
            ..
        } => crate::install::mcp_agent_is_tracked(state, reference, tool, project_path.as_deref())
            .await
            .map_err(|error| error.to_string())?
            .then_some(())
            .ok_or_else(|| "Agent install is not tracked".into()),
        AgentApprovalAction::Rollback {
            reference,
            tool,
            project_path,
            snapshot_id,
            ..
        } => crate::install::mcp_rollback_revision(
            state,
            reference,
            tool,
            project_path.as_deref(),
            snapshot_id,
        )
        .await
        .map(|_| ())
        .map_err(|error| error.to_string()),
        AgentApprovalAction::BatchCollection {
            collection_name, ..
        } => {
            let value = super::organize::list(state)
                .await
                .map_err(|error| error.to_string())?;
            value
                .collections
                .iter()
                .any(|item| item.name == *collection_name)
                .then_some(())
                .ok_or_else(|| format!("Agent collection not found: {collection_name}"))
        }
        AgentApprovalAction::PublisherTrustSet { .. } => Ok(()),
        AgentApprovalAction::DraftPublish { id, plan_revision } => {
            let draft = super::drafts::get(state, id)
                .await
                .map_err(|error| error.to_string())?;
            if draft.state != crate::types::AgentDraftState::Pending
                || !draft.validation.installable
                || draft.source_hash != *plan_revision
            {
                return Err("Agent draft is not a current valid pending draft".into());
            }
            Ok(())
        }
    }
}

#[tool_router(router = agents_tool_router, vis = "pub(crate)")]
impl SkillMcpServer {
    async fn submit_agent_approval_json(
        &self,
        request: crate::types::AgentApprovalAction,
    ) -> Result<String, String> {
        validate_approval_target(self.state(), &request).await?;
        let approval = super::organize::submit_approval(
            self.state(),
            self.client_identity().to_owned(),
            request,
        )
        .await
        .map_err(|error| error.to_string())?;
        serde_json::to_string_pretty(&approval).map_err(|error| error.to_string())
    }

    async fn install_agent_or_request_approval(
        &self,
        reference: AgentReference,
        tool: String,
        project_path: Option<String>,
        include_dependencies: bool,
        project_authorization: &McpProjectAuthorization,
    ) -> Result<String, String> {
        let plan = crate::install::mcp_agent_plan(
            self.state(),
            reference.clone(),
            tool.clone(),
            project_path.clone(),
            "install",
            include_dependencies,
            project_authorization.0.as_ref(),
        )
        .await
        .map_err(|error| error.to_string())?;
        if !plan.blockers.is_empty() {
            return Err(format!(
                "Agent install plan is blocked: {}",
                plan.blockers.join("; ")
            ));
        }
        let tracked = crate::install::mcp_agent_is_tracked(
            self.state(),
            &reference,
            &tool,
            project_path.as_deref(),
        )
        .await
        .map_err(|error| error.to_string())?;
        if include_dependencies || tracked {
            return self
                .submit_agent_approval_json(AgentApprovalAction::Install {
                    reference,
                    tool,
                    project_path,
                    include_dependencies,
                    plan_revision: plan.revision,
                })
                .await;
        }
        let record = crate::install::mcp_install_agent_clean(
            self.state(),
            reference,
            tool,
            project_path,
            project_authorization.0.as_ref(),
        )
        .await
        .map_err(|error| error.to_string())?;
        serde_json::to_string_pretty(&record).map_err(|error| error.to_string())
    }

    #[tool(description = "Search validated Agents by metadata and exact source identity")]
    async fn agents_search(
        &self,
        Parameters(SearchRequest { query }): Parameters<SearchRequest>,
    ) -> Result<String, String> {
        self.run_tool("agents_search", McpAction::Read, None, async {
            let results = super::inspect_agent_sources(&self.state().app_data_dir)
                .await
                .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&search_agents(&results, &query)?)
                .map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "Read one exact validated Agent inspection record")]
    async fn agents_get(
        &self,
        Parameters(ReferenceRequest {
            source_id,
            relative_path,
        }): Parameters<ReferenceRequest>,
    ) -> Result<String, String> {
        self.run_tool("agents_get", McpAction::Read, None, async {
            let reference = AgentReference {
                source_id,
                relative_path,
            };
            let package = super::resolve_agent_package(&self.state().app_data_dir, &reference)
                .await
                .map_err(|error| error.to_string())?;
            let _ = super::organize::touch_recent(self.state(), reference.clone()).await;
            let _ = super::organize::record_usage(self.state(), reference, "fetch".into()).await;
            serde_json::to_string_pretty(&package).map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "List the one canonical source file for an exact Agent")]
    async fn agents_list_files(
        &self,
        Parameters(ReferenceRequest {
            source_id,
            relative_path,
        }): Parameters<ReferenceRequest>,
    ) -> Result<String, String> {
        self.run_tool("agents_list_files", McpAction::Read, None, async {
            let reference = AgentReference {
                source_id,
                relative_path,
            };
            let package = super::resolve_agent_package(&self.state().app_data_dir, &reference)
                .await
                .map_err(|error| error.to_string())?;
            let text = super::read_agent_text(&self.state().app_data_dir, &reference)
                .await
                .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&[AgentFileEntry {
                relative_path: reference.relative_path,
                size_bytes: text.len() as u64,
                sha256: package.source_hash,
            }])
            .map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "Read the bounded canonical source file for an exact Agent")]
    async fn agents_get_file(
        &self,
        Parameters(FileRequest {
            source_id,
            relative_path,
            file_path,
        }): Parameters<FileRequest>,
    ) -> Result<String, String> {
        self.run_tool("agents_get_file", McpAction::Read, None, async {
            if file_path != relative_path {
                return Err("single-file Agents expose only their canonical relativePath".into());
            }
            super::read_agent_text(
                &self.state().app_data_dir,
                &AgentReference {
                    source_id,
                    relative_path,
                },
            )
            .await
            .map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "List managed Agent installs with reconciled lifecycle state")]
    async fn agents_installed(&self) -> Result<String, String> {
        self.run_tool("agents_installed", McpAction::Read, None, async {
            let installed = crate::install::mcp_reconcile_agent_installs(self.state())
                .await
                .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&installed).map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "List registered Agent sources")]
    async fn agents_list_sources(&self) -> Result<String, String> {
        self.run_tool("agents_list_sources", McpAction::Read, None, async {
            let sources = super::load_agent_sources(&self.state().app_data_dir)
                .await
                .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&sources).map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "Register an existing local directory as an Agent source")]
    async fn agents_add_local_source(
        &self,
        Parameters(LocalSourceRequest { root }): Parameters<LocalSourceRequest>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_add_local_source",
            McpAction::AgentSource,
            None,
            async {
                let source = super::add_local_source(&self.state().app_data_dir, Path::new(&root))
                    .await
                    .map_err(|error| error.to_string())?;
                serde_json::to_string_pretty(&source).map_err(|error| error.to_string())
            },
        )
        .await
    }

    #[tool(description = "Register a GitHub repository as an Agent source")]
    async fn agents_add_github_source(
        &self,
        Parameters(GithubSourceRequest {
            repository,
            git_ref,
            subdirectory,
        }): Parameters<GithubSourceRequest>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_add_github_source",
            McpAction::AgentSource,
            None,
            async {
                let source = super::add_github_source(
                    &self.state().app_data_dir,
                    &repository,
                    git_ref.as_deref(),
                    subdirectory.as_deref(),
                )
                .await
                .map_err(|error| error.to_string())?;
                serde_json::to_string_pretty(&source).map_err(|error| error.to_string())
            },
        )
        .await
    }

    #[tool(description = "Refresh and validate one registered Agent source")]
    async fn agents_refresh_source(
        &self,
        Parameters(SourceRequest { source_id }): Parameters<SourceRequest>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_refresh_source",
            McpAction::AgentSource,
            None,
            async {
                let result = refresh_agent_source(self.state(), &source_id)
                    .await
                    .map_err(|error| error.to_string())?;
                serde_json::to_string_pretty(&sanitized_source(result))
                    .map_err(|error| error.to_string())
            },
        )
        .await
    }

    #[tool(description = "Request desktop approval to unregister an Agent source")]
    async fn agents_remove_source(
        &self,
        Parameters(SourceApprovalRequest { source_id }): Parameters<SourceApprovalRequest>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_remove_source",
            McpAction::AgentDestructive,
            None,
            async {
                super::source_by_id(&self.state().app_data_dir, &source_id)
                    .await
                    .map_err(|error| error.to_string())?;
                self.submit_agent_approval_json(crate::types::AgentApprovalAction::SourceRemove {
                    source_id,
                })
                .await
            },
        )
        .await
    }

    #[tool(description = "Refresh and validate every registered Agent source")]
    async fn agents_refresh_all(&self) -> Result<String, String> {
        self.run_tool("agents_refresh_all", McpAction::AgentSource, None, async {
            let sources = super::load_agent_sources(&self.state().app_data_dir)
                .await
                .map_err(|error| error.to_string())?;
            let mut results = Vec::with_capacity(sources.len());
            for source in sources {
                results.push(
                    refresh_agent_source(self.state(), &source.id)
                        .await
                        .map(sanitized_source)
                        .map_err(|error| error.to_string())?,
                );
            }
            serde_json::to_string_pretty(&AgentCatalogResponse {
                catalog_revision: agent_catalog_revision(&results),
                sources: results,
            })
            .map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "Inspect Agent sources and return the aggregate revision")]
    async fn agents_source_status(&self) -> Result<String, String> {
        self.run_tool("agents_source_status", McpAction::Read, None, async {
            serde_json::to_string_pretty(
                &catalog_response(self.state())
                    .await
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "Recommend exact validated Agents for a bounded task description")]
    async fn agents_recommend(
        &self,
        Parameters(RecommendRequest { task, limit }): Parameters<RecommendRequest>,
    ) -> Result<String, String> {
        self.run_tool("agents_recommend", McpAction::Read, None, async {
            let sources = super::inspect_agent_sources(&self.state().app_data_dir)
                .await
                .map_err(|error| error.to_string())?;
            let library = super::organize::list(self.state())
                .await
                .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(
                &crate::library::recommend_agents(
                    &sources,
                    &library.preferred_sources,
                    &task,
                    &[],
                    limit.unwrap_or(10),
                )
                .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "Submit a bounded raw Agent draft for desktop review")]
    async fn agents_submit_draft(
        &self,
        Parameters(DraftInputRequest {
            relative_path,
            text,
        }): Parameters<DraftInputRequest>,
    ) -> Result<String, String> {
        self.run_tool("agents_submit_draft", McpAction::AgentSource, None, async {
            let draft = super::drafts::create(
                self.state(),
                AgentDraftInput {
                    relative_path,
                    text,
                },
            )
            .await
            .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&draft).map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "List Agent drafts and validation diagnostics")]
    async fn agents_list_drafts(&self) -> Result<String, String> {
        self.run_tool("agents_list_drafts", McpAction::Read, None, async {
            let drafts = super::drafts::list(self.state())
                .await
                .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&drafts).map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "Read one Agent draft and its validation diagnostics")]
    async fn agents_get_draft(
        &self,
        Parameters(DraftRequest { id }): Parameters<DraftRequest>,
    ) -> Result<String, String> {
        self.run_tool("agents_get_draft", McpAction::Read, None, async {
            let draft = super::drafts::get(self.state(), &id)
                .await
                .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&draft).map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "Create a structured Agent draft for desktop review")]
    async fn agents_create_draft(
        &self,
        Parameters(CreateDraftRequest {
            relative_path,
            name,
            description,
            body,
        }): Parameters<CreateDraftRequest>,
    ) -> Result<String, String> {
        self.run_tool("agents_create_draft", McpAction::AgentSource, None, async {
            let metadata = serde_yaml::to_string(&DraftMetadata {
                name: &name,
                description: &description,
            })
            .map_err(|error| error.to_string())?;
            let text = format!("---\n{metadata}---\n{}", body.trim_start());
            let draft = super::drafts::create(
                self.state(),
                AgentDraftInput {
                    relative_path,
                    text,
                },
            )
            .await
            .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&draft).map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "Create a validated Agent draft from one exact Skill for desktop review")]
    async fn agents_create_from_skill(
        &self,
        Parameters(CreateFromSkillRequest {
            source_id,
            relative_path,
        }): Parameters<CreateFromSkillRequest>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_create_from_skill",
            McpAction::AgentSource,
            None,
            async {
                let draft = super::drafts::create_from_skill(
                    self.state(),
                    crate::types::SkillReference {
                        source_id,
                        relative_path,
                    },
                )
                .await
                .map_err(|error| error.to_string())?;
                serde_json::to_string_pretty(&draft).map_err(|error| error.to_string())
            },
        )
        .await
    }

    #[tool(description = "Request desktop approval to publish one current valid Agent draft")]
    async fn agents_request_publish_draft(
        &self,
        Parameters(DraftRequest { id }): Parameters<DraftRequest>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_request_publish_draft",
            McpAction::AgentSource,
            None,
            async {
                let draft = super::drafts::get(self.state(), &id)
                    .await
                    .map_err(|error| error.to_string())?;
                self.submit_agent_approval_json(AgentApprovalAction::DraftPublish {
                    id,
                    plan_revision: draft.source_hash,
                })
                .await
            },
        )
        .await
    }

    #[tool(description = "Edit one pending Agent draft")]
    async fn agents_edit_draft(
        &self,
        Parameters(EditDraftRequest {
            id,
            relative_path,
            text,
        }): Parameters<EditDraftRequest>,
    ) -> Result<String, String> {
        self.run_tool("agents_edit_draft", McpAction::AgentSource, None, async {
            let draft = super::drafts::edit(
                self.state(),
                &id,
                AgentDraftInput {
                    relative_path,
                    text,
                },
            )
            .await
            .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&draft).map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "Return bounded Agent library usage and organization insights")]
    async fn agents_get_insights(&self) -> Result<String, String> {
        self.run_tool("agents_get_insights", McpAction::Read, None, async {
            let sources = super::inspect_agent_sources(&self.state().app_data_dir)
                .await
                .map_err(|error| error.to_string())?;
            let library = super::organize::list(self.state())
                .await
                .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&serde_json::json!({
                "sources": sources.len(),
                "agents": sources.iter().map(|source| source.agents.iter().filter(|agent| agent.installable).count()).sum::<usize>(),
                "favorites": library.favorites.len(),
                "folders": library.folders.len(),
                "collections": library.collections.len(),
                "usage": library.usage,
            }))
            .map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "Read the complete Agent personal-library state")]
    async fn agents_get_library(&self) -> Result<String, String> {
        self.run_tool("agents_get_library", McpAction::Read, None, async {
            let value = super::organize::list(self.state())
                .await
                .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&value).map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "List nested Agent library folder paths")]
    async fn agents_list_folders(&self) -> Result<String, String> {
        self.run_tool("agents_list_folders", McpAction::Read, None, async {
            let value = super::organize::list(self.state())
                .await
                .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&value.folders).map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "Create one validated nested Agent library folder")]
    async fn agents_create_folder(
        &self,
        Parameters(FolderRequest { path }): Parameters<FolderRequest>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_create_folder",
            McpAction::AgentSource,
            None,
            async {
                let value = super::organize::create_folder(self.state(), path)
                    .await
                    .map_err(|error| error.to_string())?;
                serde_json::to_string_pretty(&value).map_err(|error| error.to_string())
            },
        )
        .await
    }

    #[tool(description = "Rename one Agent folder and its nested descendants")]
    async fn agents_rename_folder(
        &self,
        Parameters(RenameFolderRequest { path, new_name }): Parameters<RenameFolderRequest>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_rename_folder",
            McpAction::AgentSource,
            None,
            async {
                let value = super::organize::rename_folder(self.state(), path, new_name)
                    .await
                    .map_err(|error| error.to_string())?;
                serde_json::to_string_pretty(&value).map_err(|error| error.to_string())
            },
        )
        .await
    }

    #[tool(description = "Move one Agent folder and its nested descendants")]
    async fn agents_move_folder(
        &self,
        Parameters(MoveFolderRequest { path, new_parent }): Parameters<MoveFolderRequest>,
    ) -> Result<String, String> {
        self.run_tool("agents_move_folder", McpAction::AgentSource, None, async {
            let value = super::organize::move_folder(self.state(), path, new_parent)
                .await
                .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&value).map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "Request desktop approval to delete an Agent folder")]
    async fn agents_delete_folder(
        &self,
        Parameters(DeleteFolderRequest { path, recursive }): Parameters<DeleteFolderRequest>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_delete_folder",
            McpAction::AgentDestructive,
            None,
            async {
                self.submit_agent_approval_json(AgentApprovalAction::FolderDelete {
                    path,
                    recursive,
                })
                .await
            },
        )
        .await
    }

    #[tool(description = "Assign or unassign one exact Agent to a nested folder")]
    async fn agents_assign_folder(
        &self,
        Parameters(AssignFolderRequest {
            source_id,
            relative_path,
            folder_path,
        }): Parameters<AssignFolderRequest>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_assign_folder",
            McpAction::AgentSource,
            None,
            async {
                let value = super::organize::assign_folder(
                    self.state(),
                    AgentReference {
                        source_id,
                        relative_path,
                    },
                    folder_path,
                )
                .await
                .map_err(|error| error.to_string())?;
                serde_json::to_string_pretty(&value).map_err(|error| error.to_string())
            },
        )
        .await
    }

    #[tool(description = "Set the favorite state for one exact Agent")]
    async fn agents_set_favorite(
        &self,
        Parameters(FavoriteRequest {
            source_id,
            relative_path,
            favorite,
        }): Parameters<FavoriteRequest>,
    ) -> Result<String, String> {
        self.run_tool("agents_set_favorite", McpAction::AgentSource, None, async {
            let value = super::organize::set_favorite(
                self.state(),
                AgentReference {
                    source_id,
                    relative_path,
                },
                favorite,
            )
            .await
            .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&value).map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "Create or replace a named exact-Agent collection")]
    async fn agents_save_collection(
        &self,
        Parameters(CollectionRequest { name, agents }): Parameters<CollectionRequest>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_save_collection",
            McpAction::AgentSource,
            None,
            async {
                let value = super::organize::save_collection(
                    self.state(),
                    AgentCollection {
                        name,
                        agents: agents.into_iter().map(Into::into).collect(),
                    },
                )
                .await
                .map_err(|error| error.to_string())?;
                serde_json::to_string_pretty(&value).map_err(|error| error.to_string())
            },
        )
        .await
    }

    #[tool(description = "Create or replace a named Agent smart folder")]
    async fn agents_save_smart_folder(
        &self,
        Parameters(SmartFolderRequest {
            name,
            query,
            division,
            source_id,
            capability,
            lifecycle_state,
            installable,
            favorite,
        }): Parameters<SmartFolderRequest>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_save_smart_folder",
            McpAction::AgentSource,
            None,
            async {
                let value = super::organize::save_smart_folder(
                    self.state(),
                    AgentSmartFolder {
                        name,
                        rule: AgentSmartFolderRule {
                            query,
                            division,
                            source_id,
                            capability,
                            lifecycle_state,
                            installable,
                            favorite,
                        },
                    },
                )
                .await
                .map_err(|error| error.to_string())?;
                serde_json::to_string_pretty(&value).map_err(|error| error.to_string())
            },
        )
        .await
    }

    #[tool(description = "Create or replace a named Agent workspace profile")]
    async fn agents_save_profile(
        &self,
        Parameters(ProfileRequest {
            name,
            folders,
            collections,
        }): Parameters<ProfileRequest>,
    ) -> Result<String, String> {
        self.run_tool("agents_save_profile", McpAction::AgentSource, None, async {
            let value = super::organize::save_profile(
                self.state(),
                AgentWorkspaceProfile {
                    name,
                    folders,
                    collections,
                },
            )
            .await
            .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&value).map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "Request desktop approval to delete an Agent collection")]
    async fn agents_delete_collection(
        &self,
        Parameters(NamedApprovalRequest { name }): Parameters<NamedApprovalRequest>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_delete_collection",
            McpAction::AgentDestructive,
            None,
            async {
                self.submit_agent_approval_json(AgentApprovalAction::CollectionDelete { name })
                    .await
            },
        )
        .await
    }

    #[tool(description = "Request desktop approval to delete an Agent smart folder")]
    async fn agents_delete_smart_folder(
        &self,
        Parameters(NamedApprovalRequest { name }): Parameters<NamedApprovalRequest>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_delete_smart_folder",
            McpAction::AgentDestructive,
            None,
            async {
                self.submit_agent_approval_json(AgentApprovalAction::SmartFolderDelete { name })
                    .await
            },
        )
        .await
    }

    #[tool(description = "Request desktop approval to delete an Agent workspace profile")]
    async fn agents_delete_profile(
        &self,
        Parameters(NamedApprovalRequest { name }): Parameters<NamedApprovalRequest>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_delete_profile",
            McpAction::AgentDestructive,
            None,
            async {
                self.submit_agent_approval_json(AgentApprovalAction::ProfileDelete { name })
                    .await
            },
        )
        .await
    }

    #[tool(description = "Request desktop approval to change an exact Agent update policy")]
    async fn agents_set_update_policy(
        &self,
        Parameters(UpdatePolicyRequest {
            source_id,
            relative_path,
            policy,
        }): Parameters<UpdatePolicyRequest>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_set_update_policy",
            McpAction::AgentSource,
            None,
            async {
                self.submit_agent_approval_json(AgentApprovalAction::UpdatePolicySet {
                    reference: AgentReference {
                        source_id,
                        relative_path,
                    },
                    policy: parse_update_policy(&policy)?,
                })
                .await
            },
        )
        .await
    }

    #[tool(description = "Set the preferred registered source for an Agent name")]
    async fn agents_set_preferred_source(
        &self,
        Parameters(PreferredSourceRequest {
            agent_name,
            source_id,
        }): Parameters<PreferredSourceRequest>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_set_preferred_source",
            McpAction::AgentSource,
            None,
            async {
                let value = super::organize::set_preferred_source(
                    self.state(),
                    AgentPreferredSource {
                        agent_name,
                        source_id,
                    },
                )
                .await
                .map_err(|error| error.to_string())?;
                serde_json::to_string_pretty(&value).map_err(|error| error.to_string())
            },
        )
        .await
    }

    #[tool(description = "Request desktop approval to change Agent publisher trust")]
    async fn agents_request_publisher_trust(
        &self,
        Parameters(PublisherTrustRequest {
            name,
            public_key,
            trusted,
            revoked,
        }): Parameters<PublisherTrustRequest>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_request_publisher_trust",
            McpAction::AgentDestructive,
            None,
            async {
                self.submit_agent_approval_json(AgentApprovalAction::PublisherTrustSet {
                    name,
                    public_key,
                    trusted,
                    revoked,
                })
                .await
            },
        )
        .await
    }

    #[tool(description = "Submit one bounded typed Agent action for desktop approval")]
    async fn agents_submit_approval(
        &self,
        Parameters(SubmitApprovalRequest { request }): Parameters<SubmitApprovalRequest>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_submit_approval",
            McpAction::AgentDestructive,
            None,
            async {
                self.submit_agent_approval_json(approval_action(request)?)
                    .await
            },
        )
        .await
    }

    #[tool(description = "List typed Agent approval requests and desktop decisions")]
    async fn agents_list_approvals(&self) -> Result<String, String> {
        self.run_tool("agents_list_approvals", McpAction::Read, None, async {
            let value = super::organize::list(self.state())
                .await
                .map_err(|error| error.to_string())?;
            serde_json::to_string_pretty(&value.approvals).map_err(|error| error.to_string())
        })
        .await
    }

    #[tool(description = "Plan one exact Agent install and its optional dependencies")]
    async fn agents_plan_install(
        &self,
        Parameters(AgentPlanRequest {
            source_id,
            relative_path,
            tool,
            project_path,
            include_dependencies,
        }): Parameters<AgentPlanRequest>,
        Extension(project_authorization): Extension<McpProjectAuthorization>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_plan_install",
            McpAction::Read,
            project_path.clone(),
            async {
                let plan = crate::install::mcp_agent_plan(
                    self.state(),
                    AgentReference {
                        source_id,
                        relative_path,
                    },
                    tool,
                    project_path,
                    "install",
                    include_dependencies.unwrap_or(true),
                    project_authorization.0.as_ref(),
                )
                .await
                .map_err(|error| error.to_string())?;
                serde_json::to_string_pretty(&plan).map_err(|error| error.to_string())
            },
        )
        .await
    }

    #[tool(
        description = "Install a clean non-conflicting Agent or request desktop approval for replacement"
    )]
    async fn agents_install(
        &self,
        Parameters(AgentInstallRequest {
            source_id,
            relative_path,
            tool,
            project_path,
        }): Parameters<AgentInstallRequest>,
        Extension(project_authorization): Extension<McpProjectAuthorization>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_install",
            McpAction::AgentInstall,
            project_path.clone(),
            async {
                self.install_agent_or_request_approval(
                    AgentReference {
                        source_id,
                        relative_path,
                    },
                    tool,
                    project_path,
                    false,
                    &project_authorization,
                )
                .await
            },
        )
        .await
    }

    #[tool(description = "Request desktop approval for an Agent dependency-graph install")]
    async fn agents_install_with_dependencies(
        &self,
        Parameters(AgentInstallRequest {
            source_id,
            relative_path,
            tool,
            project_path,
        }): Parameters<AgentInstallRequest>,
        Extension(project_authorization): Extension<McpProjectAuthorization>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_install_with_dependencies",
            McpAction::AgentInstall,
            project_path.clone(),
            async {
                self.install_agent_or_request_approval(
                    AgentReference {
                        source_id,
                        relative_path,
                    },
                    tool,
                    project_path,
                    true,
                    &project_authorization,
                )
                .await
            },
        )
        .await
    }

    #[tool(
        description = "Install only one exact normalized Agent name or valid preferred-source match"
    )]
    async fn agents_find_and_install(
        &self,
        Parameters(AgentFindRequest {
            name,
            tool,
            project_path,
        }): Parameters<AgentFindRequest>,
        Extension(project_authorization): Extension<McpProjectAuthorization>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_find_and_install",
            McpAction::AgentInstall,
            project_path.clone(),
            async {
                if name.len() > 256 {
                    return Err("Agent name exceeds the 256-byte limit".into());
                }
                let normalized = crate::skills::mcp::normalize_skill_name(&name);
                let sources = super::inspect_agent_sources(&self.state().app_data_dir)
                    .await
                    .map_err(|error| error.to_string())?;
                let library = super::organize::list(self.state())
                    .await
                    .map_err(|error| error.to_string())?;
                let mut matches = sources
                    .iter()
                    .flat_map(|source| &source.agents)
                    .filter(|package| package.installable)
                    .filter(|package| {
                        package.agent.as_ref().is_some_and(|agent| {
                            crate::skills::mcp::normalize_skill_name(&agent.name) == normalized
                                || crate::skills::mcp::normalize_skill_name(&agent.slug)
                                    == normalized
                        })
                    })
                    .collect::<Vec<_>>();
                if matches.len() > 1 {
                    if let Some(preferred) = library.preferred_sources.iter().find(|preferred| {
                        crate::skills::mcp::normalize_skill_name(&preferred.agent_name)
                            == normalized
                    }) {
                        matches
                            .retain(|package| package.reference.source_id == preferred.source_id);
                    }
                }
                if matches.len() != 1 {
                    return Err(format!(
                        "Agent name must resolve to one exact or preferred-source match; found {}",
                        matches.len()
                    ));
                }
                self.install_agent_or_request_approval(
                    matches[0].reference.clone(),
                    tool,
                    project_path,
                    false,
                    &project_authorization,
                )
                .await
            },
        )
        .await
    }

    #[tool(description = "Request desktop approval for one exact Agent update")]
    async fn agents_update(
        &self,
        Parameters(AgentInstallRequest {
            source_id,
            relative_path,
            tool,
            project_path,
        }): Parameters<AgentInstallRequest>,
        Extension(project_authorization): Extension<McpProjectAuthorization>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_update",
            McpAction::AgentInstall,
            project_path.clone(),
            async {
                let reference = AgentReference {
                    source_id,
                    relative_path,
                };
                let plan = crate::install::mcp_agent_plan(
                    self.state(),
                    reference.clone(),
                    tool.clone(),
                    project_path.clone(),
                    "update",
                    false,
                    project_authorization.0.as_ref(),
                )
                .await
                .map_err(|error| error.to_string())?;
                if !plan.blockers.is_empty() {
                    return Err(format!(
                        "Agent update plan is blocked: {}",
                        plan.blockers.join("; ")
                    ));
                }
                self.submit_agent_approval_json(AgentApprovalAction::Update {
                    reference,
                    tool,
                    project_path,
                    plan_revision: plan.revision,
                })
                .await
            },
        )
        .await
    }

    #[tool(description = "Disable one exact managed Agent using a reversible move")]
    async fn agents_disable(
        &self,
        Parameters(AgentLifecycleRequest {
            source_id,
            relative_path,
            tool,
            project_path,
        }): Parameters<AgentLifecycleRequest>,
        Extension(project_authorization): Extension<McpProjectAuthorization>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_disable",
            McpAction::AgentDestructive,
            project_path.clone(),
            async {
                let record = crate::install::mcp_move_agent_install(
                    self.state(),
                    AgentReference {
                        source_id,
                        relative_path,
                    },
                    tool,
                    project_path,
                    false,
                    project_authorization.0.as_ref(),
                )
                .await
                .map_err(|error| error.to_string())?;
                serde_json::to_string_pretty(&record).map_err(|error| error.to_string())
            },
        )
        .await
    }

    #[tool(description = "Enable one exact disabled Agent using a reversible move")]
    async fn agents_enable(
        &self,
        Parameters(AgentLifecycleRequest {
            source_id,
            relative_path,
            tool,
            project_path,
        }): Parameters<AgentLifecycleRequest>,
        Extension(project_authorization): Extension<McpProjectAuthorization>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_enable",
            McpAction::AgentInstall,
            project_path.clone(),
            async {
                let record = crate::install::mcp_move_agent_install(
                    self.state(),
                    AgentReference {
                        source_id,
                        relative_path,
                    },
                    tool,
                    project_path,
                    true,
                    project_authorization.0.as_ref(),
                )
                .await
                .map_err(|error| error.to_string())?;
                serde_json::to_string_pretty(&record).map_err(|error| error.to_string())
            },
        )
        .await
    }

    #[tool(description = "Request desktop approval to uninstall one exact managed Agent")]
    async fn agents_uninstall(
        &self,
        Parameters(AgentInstallRequest {
            source_id,
            relative_path,
            tool,
            project_path,
        }): Parameters<AgentInstallRequest>,
        Extension(project_authorization): Extension<McpProjectAuthorization>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_uninstall",
            McpAction::AgentDestructive,
            project_path.clone(),
            async {
                let reference = AgentReference {
                    source_id,
                    relative_path,
                };
                let plan = crate::install::mcp_agent_plan(
                    self.state(),
                    reference.clone(),
                    tool.clone(),
                    project_path.clone(),
                    "uninstall",
                    false,
                    project_authorization.0.as_ref(),
                )
                .await
                .map_err(|error| error.to_string())?;
                if !plan.blockers.is_empty() {
                    return Err(format!(
                        "Agent uninstall plan is blocked: {}",
                        plan.blockers.join("; ")
                    ));
                }
                self.submit_agent_approval_json(AgentApprovalAction::Uninstall {
                    reference,
                    tool,
                    project_path,
                    plan_revision: plan.revision,
                })
                .await
            },
        )
        .await
    }

    #[tool(description = "List verified version snapshots for one exact managed Agent")]
    async fn agents_version_history(
        &self,
        Parameters(AgentLifecycleRequest {
            source_id,
            relative_path,
            tool,
            project_path,
        }): Parameters<AgentLifecycleRequest>,
        Extension(project_authorization): Extension<McpProjectAuthorization>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_version_history",
            McpAction::Read,
            project_path.clone(),
            async {
                let history = crate::install::mcp_agent_version_history(
                    self.state(),
                    AgentReference {
                        source_id,
                        relative_path,
                    },
                    tool,
                    project_path,
                    project_authorization.0.as_ref(),
                )
                .await
                .map_err(|error| error.to_string())?;
                serde_json::to_string_pretty(&history).map_err(|error| error.to_string())
            },
        )
        .await
    }

    #[tool(description = "Request desktop approval to roll back one exact Agent snapshot")]
    async fn agents_request_rollback(
        &self,
        Parameters(AgentRollbackRequest {
            source_id,
            relative_path,
            tool,
            project_path,
            snapshot_id,
        }): Parameters<AgentRollbackRequest>,
        Extension(project_authorization): Extension<McpProjectAuthorization>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_request_rollback",
            McpAction::AgentDestructive,
            project_path.clone(),
            async {
                authorized_project_matches(&project_path, &project_authorization)?;
                let reference = AgentReference {
                    source_id,
                    relative_path,
                };
                let plan_revision = crate::install::mcp_rollback_revision(
                    self.state(),
                    &reference,
                    &tool,
                    project_path.as_deref(),
                    &snapshot_id,
                )
                .await
                .map_err(|error| error.to_string())?;
                self.submit_agent_approval_json(AgentApprovalAction::Rollback {
                    reference,
                    tool,
                    project_path,
                    snapshot_id,
                    plan_revision,
                })
                .await
            },
        )
        .await
    }

    #[tool(description = "Request desktop approval for an exact Agent collection mutation plan")]
    async fn agents_request_batch_collection(
        &self,
        Parameters(AgentBatchRequest {
            collection_name,
            operation,
            tool,
            project_path,
        }): Parameters<AgentBatchRequest>,
        Extension(project_authorization): Extension<McpProjectAuthorization>,
    ) -> Result<String, String> {
        self.run_tool(
            "agents_request_batch_collection",
            McpAction::AgentDestructive,
            project_path.clone(),
            async {
                let plan = crate::install::mcp_collection_plan(
                    self.state(),
                    &collection_name,
                    tool.clone(),
                    project_path.clone(),
                    &operation,
                    project_authorization.0.as_ref(),
                )
                .await
                .map_err(|error| error.to_string())?;
                if !plan.blockers.is_empty() {
                    return Err(format!(
                        "Agent collection plan is blocked: {}",
                        plan.blockers.join("; ")
                    ));
                }
                self.submit_agent_approval_json(AgentApprovalAction::BatchCollection {
                    collection_name,
                    operation,
                    tool,
                    project_path,
                    plan_revision: plan.revision,
                })
                .await
            },
        )
        .await
    }

    #[tool(description = "Compare a project's disk and install ledgers with shikigami.lock.json")]
    async fn lock_check(
        &self,
        Parameters(LockRequest { project_path }): Parameters<LockRequest>,
        Extension(_project_authorization): Extension<McpProjectAuthorization>,
    ) -> Result<String, String> {
        self.run_tool(
            "lock_check",
            McpAction::Read,
            Some(project_path.clone()),
            async {
                let result = crate::install::lockfile::mcp_lock_check(self.state(), &project_path)
                    .await
                    .map_err(|error| error.to_string())?;
                serde_json::to_string_pretty(&result).map_err(|error| error.to_string())
            },
        )
        .await
    }

    #[tool(description = "Plan project installs and updates required by shikigami.lock.json")]
    async fn lock_plan(
        &self,
        Parameters(LockRequest { project_path }): Parameters<LockRequest>,
        Extension(_project_authorization): Extension<McpProjectAuthorization>,
    ) -> Result<String, String> {
        self.run_tool(
            "lock_plan",
            McpAction::Read,
            Some(project_path.clone()),
            async {
                let result = crate::install::lockfile::mcp_lock_plan(self.state(), &project_path)
                    .await
                    .map_err(|error| error.to_string())?;
                serde_json::to_string_pretty(&result).map_err(|error| error.to_string())
            },
        )
        .await
    }

    #[tool(description = "Apply an unchanged, unblocked shikigami.lock.json plan")]
    async fn lock_apply(
        &self,
        Parameters(LockApplyRequest {
            project_path,
            revision,
        }): Parameters<LockApplyRequest>,
        Extension(project_authorization): Extension<McpProjectAuthorization>,
    ) -> Result<String, String> {
        self.run_tool(
            "lock_apply",
            McpAction::AgentInstall,
            Some(project_path.clone()),
            async {
                let result = crate::install::lockfile::mcp_lock_apply(
                    self.state(),
                    &project_path,
                    &revision,
                    project_authorization.0.as_ref(),
                )
                .await
                .map_err(|error| error.to_string())?;
                serde_json::to_string_pretty(&result).map_err(|error| error.to_string())
            },
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rmcp::model::ResourceContents;
    use tokio::sync::{Mutex, RwLock};

    use crate::{commands::settings::SettingsLoadState, state::AppState};

    use super::{
        agent_resource_uri, list_agent_resources, parse_agent_resource_uri, read_agent_resource,
        render_resource_uri, AgentResource, SkillMcpServer, AGENT_CATALOG_URI,
    };

    fn test_state(app_data_dir: &std::path::Path) -> AppState {
        AppState {
            app_data_dir: app_data_dir.to_path_buf(),
            corpus_cache: Arc::new(Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(Mutex::new(())),
            skill_sources_write_lock: Arc::new(Mutex::new(())),
            skill_installs_write_lock: Arc::new(Mutex::new(())),
            skill_folders_write_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(RwLock::new(SettingsLoadState::FirstLaunch)),
            updater_state: crate::commands::updater::empty_state(),
        }
    }

    fn text(content: ResourceContents) -> String {
        match content {
            ResourceContents::TextResourceContents { text, .. } => text,
            ResourceContents::BlobResourceContents { .. } => panic!("expected text resource"),
            _ => panic!("unexpected resource content"),
        }
    }

    #[test]
    fn resource_uris_round_trip_exact_agent_identity_and_render_tool() {
        let agent = agent_resource_uri("source:one", "nested/agent name-λ.md");
        assert_eq!(
            parse_agent_resource_uri(&agent).expect("Agent URI"),
            AgentResource::Source {
                source_id: "source:one".into(),
                relative_path: "nested/agent name-λ.md".into(),
            }
        );

        let render = render_resource_uri("source:one", "nested/agent name-λ.md", "claudeCode");
        assert_eq!(
            parse_agent_resource_uri(&render).expect("render URI"),
            AgentResource::Render {
                source_id: "source:one".into(),
                relative_path: "nested/agent name-λ.md".into(),
                tool: "claudeCode".into(),
            }
        );
    }

    #[test]
    fn resource_uris_reject_malformed_non_normalized_and_traversal_paths() {
        for uri in [
            "agents://agents/~source/~bad%ZZ.md",
            "agents://agents/~source/~../agent.md",
            "agents://agents/~source/~nested%5Cagent.md",
            "agents://agents/~source/agent.md",
            "agents://renders/~source/~agent.md/~unknown",
            "agents://agents/~source/~agent.md?query=1",
        ] {
            assert!(parse_agent_resource_uri(uri).is_err(), "accepted {uri}");
        }
    }

    #[tokio::test]
    async fn resources_list_and_read_exact_source_catalog_and_render() {
        let app = tempfile::tempdir().expect("app data");
        let source_root = tempfile::tempdir().expect("Agent source");
        std::fs::create_dir_all(source_root.path().join("nested")).expect("nested source");
        let source_text = "---\nname: Reviewer\ndescription: Reviews code.\n---\nWork carefully.\n";
        std::fs::write(source_root.path().join("nested/reviewer.md"), source_text)
            .expect("Agent source file");
        let source = super::super::add_local_source(app.path(), source_root.path())
            .await
            .expect("register local source");
        let state = test_state(app.path());

        let uri = agent_resource_uri(&source.id, "nested/reviewer.md");
        let resources = list_agent_resources(&state).await.expect("list resources");
        assert!(resources
            .iter()
            .any(|resource| resource.uri == AGENT_CATALOG_URI));
        assert!(resources.iter().any(|resource| resource.uri == uri));
        assert_eq!(
            text(
                read_agent_resource(&state, &uri)
                    .await
                    .expect("read source")
            ),
            source_text
        );

        let render_uri = render_resource_uri(&source.id, "nested/reviewer.md", "codex");
        let rendered = text(
            read_agent_resource(&state, &render_uri)
                .await
                .expect("read render"),
        );
        assert!(rendered.contains("name = \"Reviewer\""));

        let catalog = text(
            read_agent_resource(&state, AGENT_CATALOG_URI)
                .await
                .expect("read catalog"),
        );
        assert!(catalog.contains("catalogRevision"));
        assert!(!catalog.contains("Work carefully."));

        let unknown = agent_resource_uri("local:missing", "nested/reviewer.md");
        assert!(read_agent_resource(&state, &unknown).await.is_err());
    }

    #[test]
    fn all_agent_tools_are_registered_exactly_once() {
        let mut names = SkillMcpServer::agents_tool_router()
            .list_all()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(
            names,
            [
                "agents_add_github_source",
                "agents_add_local_source",
                "agents_assign_folder",
                "agents_create_draft",
                "agents_create_folder",
                "agents_create_from_skill",
                "agents_delete_collection",
                "agents_delete_folder",
                "agents_delete_profile",
                "agents_delete_smart_folder",
                "agents_disable",
                "agents_edit_draft",
                "agents_enable",
                "agents_find_and_install",
                "agents_get",
                "agents_get_draft",
                "agents_get_file",
                "agents_get_insights",
                "agents_get_library",
                "agents_install",
                "agents_install_with_dependencies",
                "agents_installed",
                "agents_list_approvals",
                "agents_list_drafts",
                "agents_list_files",
                "agents_list_folders",
                "agents_list_sources",
                "agents_move_folder",
                "agents_plan_install",
                "agents_recommend",
                "agents_refresh_all",
                "agents_refresh_source",
                "agents_remove_source",
                "agents_rename_folder",
                "agents_request_batch_collection",
                "agents_request_publish_draft",
                "agents_request_publisher_trust",
                "agents_request_rollback",
                "agents_save_collection",
                "agents_save_profile",
                "agents_save_smart_folder",
                "agents_search",
                "agents_set_favorite",
                "agents_set_preferred_source",
                "agents_set_update_policy",
                "agents_source_status",
                "agents_submit_approval",
                "agents_submit_draft",
                "agents_uninstall",
                "agents_update",
                "agents_version_history",
                "lock_apply",
                "lock_check",
                "lock_plan",
            ]
        );
    }
}
