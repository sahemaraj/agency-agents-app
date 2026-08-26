use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::error::AppError;
use crate::state::AppState;
use crate::types::AgentReference;
use crate::util::fs::{atomic_write, read_capped};

const OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434/api";
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_DEPLOYMENTS: usize = 1024;
const MAX_DEPLOYMENT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_MODEL_NAME_BYTES: usize = 255;
const RECOVERY_PREFIX: &str = "agency-agents-recovery/";

fn invalid(message: impl Into<String>) -> AppError {
    AppError::InvalidArgument {
        message: message.into(),
    }
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_model_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_MODEL_NAME_BYTES
        && !value.starts_with('-')
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-/:".contains(&byte))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OllamaModel {
    pub name: String,
    pub digest: String,
    pub size: u64,
}

fn eligible_models(models: Vec<OllamaModel>) -> Vec<OllamaModel> {
    let mut eligible = BTreeMap::new();
    for model in models {
        if valid_model_name(&model.name)
            && valid_hash(&model.digest)
            && !model.name.ends_with(":cloud")
            && !model.name.starts_with(RECOVERY_PREFIX)
        {
            eligible.entry(model.name.clone()).or_insert(model);
        }
    }
    eligible.into_values().collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OllamaDeploymentRecord {
    pub reference: AgentReference,
    pub agent_name: String,
    pub agent_slug: String,
    pub target_name: String,
    pub base_model: String,
    pub base_digest: String,
    pub source_hash: String,
    pub prompt_hash: String,
    pub deployed_at: String,
}

fn target_name(agent_slug: &str, reference: &AgentReference) -> String {
    let slug = crate::render::slugify(agent_slug);
    let slug = if slug.is_empty() {
        "agent"
    } else {
        slug.as_str()
    };
    let identity = format!("{}\0{}", reference.source_id, reference.relative_path);
    let digest = crate::render::sha256_hex(identity.as_bytes());
    format!("agency-agents/{slug}-{}:latest", &digest[..12])
}

fn validate_deployments(records: &[OllamaDeploymentRecord]) -> Result<(), AppError> {
    if records.len() > MAX_DEPLOYMENTS {
        return Err(invalid("too many Ollama deployments"));
    }
    let mut targets = HashSet::new();
    let mut references = HashSet::new();
    for record in records {
        crate::library::validate_reference(
            &record.reference.source_id,
            &record.reference.relative_path,
        )?;
        if record.agent_name.is_empty()
            || record.agent_name.len() > 256
            || record.agent_slug.is_empty()
            || !valid_model_name(&record.target_name)
            || record.target_name != target_name(&record.agent_slug, &record.reference)
            || !valid_model_name(&record.base_model)
            || !valid_hash(&record.base_digest)
            || !valid_hash(&record.source_hash)
            || !valid_hash(&record.prompt_hash)
            || chrono::DateTime::parse_from_rfc3339(&record.deployed_at).is_err()
            || !targets.insert(record.target_name.as_str())
            || !references.insert((&record.reference.source_id, &record.reference.relative_path))
        {
            return Err(invalid("persisted Ollama deployment is invalid"));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OllamaDeploymentState {
    Current,
    Outdated,
    Modified,
    Missing,
    SourceUnavailable,
}

fn classify_deployment(
    tracked_prompt_hash: Option<&String>,
    source_prompt_hash: Option<&String>,
    runtime_prompt_hash: Option<&String>,
) -> OllamaDeploymentState {
    let Some(tracked) = tracked_prompt_hash else {
        return OllamaDeploymentState::SourceUnavailable;
    };
    let Some(source) = source_prompt_hash else {
        return OllamaDeploymentState::SourceUnavailable;
    };
    let Some(runtime) = runtime_prompt_hash else {
        return OllamaDeploymentState::Missing;
    };
    if runtime != tracked {
        OllamaDeploymentState::Modified
    } else if source != tracked {
        OllamaDeploymentState::Outdated
    } else {
        OllamaDeploymentState::Current
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OllamaDeployment {
    pub record: OllamaDeploymentRecord,
    pub state: OllamaDeploymentState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OllamaStatus {
    pub models: Vec<OllamaModel>,
    pub deployments: Vec<OllamaDeployment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OllamaMutationPlan {
    pub revision: String,
    pub operation: String,
    pub reference: AgentReference,
    pub agent_name: String,
    pub agent_slug: String,
    pub target_name: String,
    pub base_model: Option<String>,
    pub base_digest: Option<String>,
    pub source_hash: String,
    pub prompt_hash: String,
    pub prompt_preview: Option<String>,
    pub state: Option<OllamaDeploymentState>,
    pub scope: String,
    pub warnings: Vec<String>,
    pub blockers: Vec<String>,
    pub rollback_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OllamaMutationResult {
    pub operation: String,
    pub target_name: String,
    pub deployment: Option<OllamaDeploymentRecord>,
}

fn revision_for(plan: &OllamaMutationPlan) -> Result<String, AppError> {
    let mut canonical = plan.clone();
    canonical.revision.clear();
    let bytes = serde_json::to_vec(&canonical).map_err(|error| AppError::Internal {
        message: format!("serialize Ollama mutation plan: {error}"),
    })?;
    Ok(crate::render::sha256_hex(&bytes))
}

fn require_plan_revision(plan: &OllamaMutationPlan, expected: &str) -> Result<(), AppError> {
    if expected.len() != 64
        || !expected.bytes().all(|byte| byte.is_ascii_hexdigit())
        || plan.revision != expected
    {
        return Err(invalid("Ollama mutation plan changed; review a fresh plan"));
    }
    Ok(())
}

fn deployments_spec() -> crate::state_db::DocumentSpec<Vec<OllamaDeploymentRecord>> {
    crate::state_db::DocumentSpec::new("ollama_deployments", 1, MAX_DEPLOYMENT_BYTES, |records| {
        validate_deployments(records)
    })
}

pub(crate) fn import_spec() -> crate::state_db::ImportSpec {
    crate::state_db::ImportSpec::document(deployments_spec(), Vec::new())
}

fn legacy_path(app_data_dir: &std::path::Path) -> PathBuf {
    app_data_dir.join("state/ollama-deployments.json")
}

async fn load_deployments(state: &AppState) -> Result<Vec<OllamaDeploymentRecord>, AppError> {
    if let Some(database) = state.completed_state_database().await? {
        return Ok(database.read(deployments_spec()).await?.unwrap_or_default());
    }
    let path = legacy_path(&state.app_data_dir);
    let bytes = match read_capped(&path, MAX_DEPLOYMENT_BYTES).await {
        Ok(bytes) => bytes,
        Err(AppError::Io { .. }) if !path.exists() => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let records = serde_json::from_slice::<Vec<OllamaDeploymentRecord>>(&bytes)?;
    validate_deployments(&records)?;
    Ok(records)
}

async fn save_deployments(
    state: &AppState,
    records: Vec<OllamaDeploymentRecord>,
) -> Result<(), AppError> {
    validate_deployments(&records)?;
    if let Some(database) = state.completed_state_database().await? {
        return database
            .mutate(deployments_spec(), Vec::new(), move |current| {
                *current = records;
                Ok(())
            })
            .await;
    }
    let bytes = serde_json::to_vec_pretty(&records).map_err(|error| AppError::Internal {
        message: format!("serialize Ollama deployments: {error}"),
    })?;
    atomic_write(&legacy_path(&state.app_data_dir), &bytes).await
}

#[async_trait]
trait DeploymentStore: Send + Sync {
    async fn save(&self, records: Vec<OllamaDeploymentRecord>) -> Result<(), AppError>;
}

struct StateDeploymentStore<'a>(&'a AppState);

#[async_trait]
impl DeploymentStore for StateDeploymentStore<'_> {
    async fn save(&self, records: Vec<OllamaDeploymentRecord>) -> Result<(), AppError> {
        save_deployments(self.0, records).await
    }
}

#[async_trait]
trait OllamaBackend: Send + Sync {
    async fn models(&self) -> Result<Vec<OllamaModel>, AppError>;
    async fn system_prompt(&self, model: &str) -> Result<Option<String>, AppError>;
    async fn create(&self, model: &str, base: &str, system: &str) -> Result<(), AppError>;
    async fn copy(&self, source: &str, destination: &str) -> Result<(), AppError>;
    async fn delete(&self, model: &str) -> Result<(), AppError>;
}

struct HttpOllamaBackend {
    client: reqwest::Client,
    base_url: String,
}

impl HttpOllamaBackend {
    fn fixed() -> Result<Self, AppError> {
        Self::build(OLLAMA_BASE_URL)
    }

    #[cfg(test)]
    fn new(base_url: &str) -> Result<Self, AppError> {
        Self::build(base_url)
    }

    fn build(base_url: &str) -> Result<Self, AppError> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(120))
            .build()?;
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path)
    }

    async fn send(
        &self,
        request: reqwest::RequestBuilder,
        timeout: Duration,
    ) -> Result<reqwest::Response, AppError> {
        tokio::time::timeout(timeout, request.send())
            .await
            .map_err(|_| AppError::Network {
                url: self.base_url.clone(),
                message: "Ollama request timed out".into(),
            })??
            .error_for_status()
            .map_err(AppError::from)
    }

    async fn decode<T: DeserializeOwned>(
        &self,
        mut response: reqwest::Response,
        label: &str,
    ) -> Result<T, AppError> {
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(invalid(format!("Ollama {label} response is too large")));
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|error| AppError::JsonParse {
            command: format!("ollama_{label}"),
            message: error.to_string(),
            raw_excerpt: String::from_utf8_lossy(&bytes).chars().take(160).collect(),
        })
    }
}

#[derive(Deserialize)]
struct TagsResponse {
    models: Vec<TagModel>,
}

#[derive(Deserialize)]
struct TagModel {
    #[serde(alias = "model")]
    name: String,
    digest: String,
    size: u64,
}

#[derive(Deserialize)]
struct ShowResponse {
    #[serde(default)]
    system: String,
}

#[derive(Deserialize)]
struct CreateResponse {
    status: String,
}

#[async_trait]
impl OllamaBackend for HttpOllamaBackend {
    async fn models(&self) -> Result<Vec<OllamaModel>, AppError> {
        let request = self.client.get(self.url("tags"));
        let response = self.send(request, Duration::from_secs(5)).await?;
        let payload: TagsResponse = self.decode(response, "tags").await?;
        Ok(eligible_models(
            payload
                .models
                .into_iter()
                .map(|model| OllamaModel {
                    name: model.name,
                    digest: model.digest,
                    size: model.size,
                })
                .collect(),
        ))
    }

    async fn system_prompt(&self, model: &str) -> Result<Option<String>, AppError> {
        if !valid_model_name(model) {
            return Err(invalid("invalid Ollama model name"));
        }
        let request = self
            .client
            .post(self.url("show"))
            .json(&serde_json::json!({ "model": model }));
        let response = match tokio::time::timeout(Duration::from_secs(5), request.send()).await {
            Ok(Ok(response)) if response.status() == reqwest::StatusCode::NOT_FOUND => {
                return Ok(None)
            }
            Ok(Ok(response)) => response.error_for_status()?,
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => {
                return Err(AppError::Network {
                    url: self.url("show"),
                    message: "Ollama request timed out".into(),
                })
            }
        };
        let payload: ShowResponse = self.decode(response, "show").await?;
        Ok(Some(payload.system))
    }

    async fn create(&self, model: &str, base: &str, system: &str) -> Result<(), AppError> {
        if !valid_model_name(model)
            || !valid_model_name(base)
            || system.len() > crate::corpus::MAX_AGENT_BYTES as usize
        {
            return Err(invalid("invalid Ollama create request"));
        }
        let request = self
            .client
            .post(self.url("create"))
            .json(&serde_json::json!({
                "model": model, "from": base, "system": system, "stream": false,
            }));
        let response = self.send(request, Duration::from_secs(120)).await?;
        let payload: CreateResponse = self.decode(response, "create").await?;
        if payload.status != "success" {
            return Err(AppError::Io {
                message: "Ollama did not confirm model creation".into(),
            });
        }
        Ok(())
    }

    async fn copy(&self, source: &str, destination: &str) -> Result<(), AppError> {
        if !valid_model_name(source) || !valid_model_name(destination) {
            return Err(invalid("invalid Ollama copy request"));
        }
        let request = self
            .client
            .post(self.url("copy"))
            .json(&serde_json::json!({ "source": source, "destination": destination }));
        self.send(request, Duration::from_secs(30)).await?;
        Ok(())
    }

    async fn delete(&self, model: &str) -> Result<(), AppError> {
        if !valid_model_name(model) {
            return Err(invalid("invalid Ollama delete request"));
        }
        let request = self
            .client
            .delete(self.url("delete"))
            .json(&serde_json::json!({ "model": model }));
        self.send(request, Duration::from_secs(30)).await?;
        Ok(())
    }
}

#[derive(Clone)]
struct AgentFacts {
    reference: AgentReference,
    name: String,
    slug: String,
    source_hash: String,
    prompt: String,
}

async fn resolve_agent_facts(
    state: &AppState,
    reference: &AgentReference,
) -> Result<AgentFacts, AppError> {
    let package = crate::agents::resolve_agent_package(&state.app_data_dir, reference).await?;
    if !package.installable {
        return Err(invalid("Agent is not installable"));
    }
    let agent = package
        .agent
        .ok_or_else(|| invalid("Agent source has no parsed Agent"))?;
    if agent.body.len() > crate::corpus::MAX_AGENT_BYTES as usize {
        return Err(invalid("Agent system prompt is too large"));
    }
    Ok(AgentFacts {
        reference: reference.clone(),
        name: agent.name,
        slug: agent.slug,
        source_hash: package.source_hash,
        prompt: agent.body,
    })
}

async fn build_plan_with<B: OllamaBackend + ?Sized>(
    backend: &B,
    records: &[OllamaDeploymentRecord],
    facts: AgentFacts,
    operation: &str,
    requested_base: Option<String>,
) -> Result<OllamaMutationPlan, AppError> {
    if !matches!(operation, "create" | "update" | "remove") {
        return Err(invalid("unknown Ollama mutation operation"));
    }
    let models = backend.models().await?;
    let model_by_name = models
        .iter()
        .map(|model| (model.name.as_str(), model))
        .collect::<BTreeMap<_, _>>();
    let target = target_name(&facts.slug, &facts.reference);
    let existing = records
        .iter()
        .find(|record| record.reference == facts.reference);
    let target_present = model_by_name.contains_key(target.as_str());
    let runtime_prompt_hash = if target_present {
        backend
            .system_prompt(&target)
            .await?
            .map(|prompt| crate::render::sha256_hex(prompt.as_bytes()))
    } else {
        None
    };
    let prompt_hash = crate::render::sha256_hex(facts.prompt.as_bytes());
    let state = existing.map(|record| {
        classify_deployment(
            Some(&record.prompt_hash),
            Some(&prompt_hash),
            runtime_prompt_hash.as_ref(),
        )
    });
    let mut warnings = Vec::new();
    let mut blockers = Vec::new();

    let (base_model, base_digest) = if operation == "remove" {
        existing
            .map(|record| {
                (
                    Some(record.base_model.clone()),
                    Some(record.base_digest.clone()),
                )
            })
            .unwrap_or((None, None))
    } else {
        match requested_base {
            Some(base) if !valid_model_name(&base) => {
                blockers.push("Selected Ollama base model is invalid".into());
                (Some(base), None)
            }
            Some(base) => {
                let digest = model_by_name
                    .get(base.as_str())
                    .map(|model| model.digest.clone());
                if digest.is_none() {
                    blockers.push("Selected Ollama base model is no longer installed".into());
                }
                (Some(base), digest)
            }
            None => {
                blockers.push("Select an installed Ollama base model".into());
                (None, None)
            }
        }
    };

    match operation {
        "create" => {
            if existing.is_some() {
                blockers.push("This Agent already has a managed Ollama deployment".into());
            } else if target_present {
                blockers.push(
                    "The derived Ollama target exists but is not managed by Shikigami".into(),
                );
            }
        }
        "update" => {
            if existing.is_none() {
                blockers.push("This Agent has no managed Ollama deployment to update".into());
            }
            if state == Some(OllamaDeploymentState::Modified) {
                warnings.push("The Ollama system prompt was changed outside Shikigami and will be preserved before update".into());
            }
        }
        "remove" => {
            if existing.is_none() {
                blockers.push("This Agent has no managed Ollama deployment to remove".into());
            } else if !target_present {
                warnings.push("The managed Ollama target is already missing; removal will clear its tracking record".into());
            }
        }
        _ => unreachable!(),
    }

    warnings.sort();
    blockers.sort();
    let mut plan = OllamaMutationPlan {
        revision: String::new(),
        operation: operation.into(),
        reference: facts.reference,
        agent_name: facts.name,
        agent_slug: facts.slug,
        target_name: target,
        base_model,
        base_digest,
        source_hash: facts.source_hash,
        prompt_hash,
        prompt_preview: Some(facts.prompt),
        state,
        scope: "device".into(),
        warnings,
        blockers,
        rollback_available: existing.is_some() && target_present,
    };
    plan.revision = revision_for(&plan)?;
    Ok(plan)
}

async fn build_plan(
    state: &AppState,
    reference: AgentReference,
    operation: &str,
    base_model: Option<String>,
) -> Result<OllamaMutationPlan, AppError> {
    let backend = HttpOllamaBackend::fixed()?;
    let records = load_deployments(state).await?;
    let facts = resolve_agent_facts(state, &reference).await?;
    build_plan_with(&backend, &records, facts, operation, base_model).await
}

async fn reconcile_with<B: OllamaBackend + ?Sized>(
    backend: &B,
    state: &AppState,
    records: Vec<OllamaDeploymentRecord>,
) -> Result<OllamaStatus, AppError> {
    let models = backend.models().await?;
    let names = models
        .iter()
        .map(|model| model.name.as_str())
        .collect::<HashSet<_>>();
    let mut deployments = Vec::with_capacity(records.len());
    for record in records {
        let source_prompt_hash = match resolve_agent_facts(state, &record.reference).await {
            Ok(facts) => Some(crate::render::sha256_hex(facts.prompt.as_bytes())),
            Err(_) => None,
        };
        let runtime_prompt_hash = if names.contains(record.target_name.as_str()) {
            backend
                .system_prompt(&record.target_name)
                .await?
                .map(|prompt| crate::render::sha256_hex(prompt.as_bytes()))
        } else {
            None
        };
        let state = classify_deployment(
            Some(&record.prompt_hash),
            source_prompt_hash.as_ref(),
            runtime_prompt_hash.as_ref(),
        );
        deployments.push(OllamaDeployment { record, state });
    }
    deployments.sort_by(|left, right| {
        left.record
            .agent_name
            .cmp(&right.record.agent_name)
            .then_with(|| left.record.target_name.cmp(&right.record.target_name))
    });
    Ok(OllamaStatus {
        models,
        deployments,
    })
}

#[tauri::command]
pub async fn ollama_status(state: tauri::State<'_, AppState>) -> Result<OllamaStatus, AppError> {
    let backend = HttpOllamaBackend::fixed()?;
    let records = load_deployments(&state).await?;
    reconcile_with(&backend, &state, records).await
}

#[tauri::command]
pub async fn ollama_plan(
    state: tauri::State<'_, AppState>,
    reference: AgentReference,
    operation: String,
    base_model: Option<String>,
) -> Result<OllamaMutationPlan, AppError> {
    build_plan(&state, reference, &operation, base_model).await
}

async fn apply_with<B: OllamaBackend + ?Sized>(
    backend: &B,
    state: &AppState,
    reviewed_revision: &str,
    reference: AgentReference,
    operation: &str,
    base_model: Option<String>,
) -> Result<OllamaMutationResult, AppError> {
    let records = load_deployments(state).await?;
    let facts = resolve_agent_facts(state, &reference).await?;
    let fresh = build_plan_with(backend, &records, facts, operation, base_model).await?;
    require_plan_revision(&fresh, reviewed_revision)?;
    if !fresh.blockers.is_empty() {
        return Err(invalid(format!(
            "Ollama mutation plan is blocked: {}",
            fresh.blockers.join("; ")
        )));
    }
    execute_plan_with(backend, &StateDeploymentStore(state), records, fresh).await
}

fn recovery_name(target: &str) -> String {
    let digest = crate::render::sha256_hex(target.as_bytes());
    format!("{RECOVERY_PREFIX}{}:latest", &digest[..16])
}

async fn restore_recovery<B: OllamaBackend + ?Sized>(
    backend: &B,
    recovery: &str,
    target: &str,
) -> Result<(), AppError> {
    backend.copy(recovery, target).await?;
    backend.delete(recovery).await
}

async fn execute_plan_with<B: OllamaBackend + ?Sized, S: DeploymentStore + ?Sized>(
    backend: &B,
    store: &S,
    original_records: Vec<OllamaDeploymentRecord>,
    plan: OllamaMutationPlan,
) -> Result<OllamaMutationResult, AppError> {
    if !plan.blockers.is_empty() {
        return Err(invalid(format!(
            "Ollama mutation plan is blocked: {}",
            plan.blockers.join("; ")
        )));
    }
    let previous_index = original_records
        .iter()
        .position(|record| record.reference == plan.reference);
    let target_existed = plan
        .state
        .is_some_and(|state| state != OllamaDeploymentState::Missing);
    let recovery = target_existed.then(|| recovery_name(&plan.target_name));
    if let Some(recovery) = &recovery {
        if backend.system_prompt(recovery).await?.is_some() {
            backend.delete(recovery).await?;
        }
        backend.copy(&plan.target_name, recovery).await?;
    }

    let mut next_records = original_records.clone();
    let mut target_mutated = false;
    let mutation = async {
        let deployment = match plan.operation.as_str() {
            "create" | "update" => {
                let base = plan
                    .base_model
                    .as_deref()
                    .ok_or_else(|| invalid("Ollama plan has no base model"))?;
                let prompt = plan
                    .prompt_preview
                    .as_deref()
                    .ok_or_else(|| invalid("Ollama plan has no system prompt"))?;
                backend.create(&plan.target_name, base, prompt).await?;
                target_mutated = true;
                let record = OllamaDeploymentRecord {
                    reference: plan.reference.clone(),
                    agent_name: plan.agent_name.clone(),
                    agent_slug: plan.agent_slug.clone(),
                    target_name: plan.target_name.clone(),
                    base_model: base.into(),
                    base_digest: plan
                        .base_digest
                        .clone()
                        .ok_or_else(|| invalid("Ollama plan has no base digest"))?,
                    source_hash: plan.source_hash.clone(),
                    prompt_hash: plan.prompt_hash.clone(),
                    deployed_at: chrono::Utc::now().to_rfc3339(),
                };
                if let Some(index) = previous_index {
                    next_records[index] = record.clone();
                } else {
                    next_records.push(record.clone());
                }
                Some(record)
            }
            "remove" => {
                if target_existed {
                    backend.delete(&plan.target_name).await?;
                    target_mutated = true;
                }
                let index =
                    previous_index.ok_or_else(|| invalid("Ollama deployment is not tracked"))?;
                next_records.remove(index);
                None
            }
            _ => return Err(invalid("unknown Ollama mutation operation")),
        };
        store.save(next_records).await?;
        Ok::<_, AppError>(deployment)
    }
    .await;

    match mutation {
        Ok(deployment) => {
            if let Some(recovery) = &recovery {
                if let Err(error) = backend.delete(recovery).await {
                    tracing::warn!("could not remove temporary Ollama recovery model: {error}");
                }
            }
            Ok(OllamaMutationResult {
                operation: plan.operation,
                target_name: plan.target_name,
                deployment,
            })
        }
        Err(error) => {
            let rollback = if let Some(recovery) = &recovery {
                restore_recovery(backend, recovery, &plan.target_name).await
            } else if plan.operation == "create" && target_mutated {
                backend.delete(&plan.target_name).await
            } else {
                Ok(())
            };
            match rollback {
                Ok(()) => Err(error),
                Err(rollback) => Err(AppError::Internal {
                    message: format!(
                        "Ollama mutation failed: {error}; rollback failed: {rollback}"
                    ),
                }),
            }
        }
    }
}

#[tauri::command]
pub async fn ollama_apply(
    state: tauri::State<'_, AppState>,
    reference: AgentReference,
    operation: String,
    base_model: Option<String>,
    plan_revision: String,
) -> Result<OllamaMutationResult, AppError> {
    let _guard = state.skill_installs_write_lock.lock().await;
    let backend = HttpOllamaBackend::fixed()?;
    apply_with(
        &backend,
        &state,
        &plan_revision,
        reference,
        &operation,
        base_model,
    )
    .await
}

#[cfg(test)]
fn test_plan() -> OllamaMutationPlan {
    let reference = AgentReference {
        source_id: "builtin".into(),
        relative_path: "engineering/frontend-developer.md".into(),
    };
    OllamaMutationPlan {
        revision: String::new(),
        operation: "create".into(),
        reference: reference.clone(),
        agent_name: "Frontend Developer".into(),
        agent_slug: "frontend-developer".into(),
        target_name: target_name("frontend-developer", &reference),
        base_model: Some("qwen:latest".into()),
        base_digest: Some("a".repeat(64)),
        source_hash: "b".repeat(64),
        prompt_hash: "c".repeat(64),
        prompt_preview: Some("You are a frontend developer.".into()),
        state: None,
        scope: "device".into(),
        warnings: Vec::new(),
        blockers: Vec::new(),
        rollback_available: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn reference(source_id: &str) -> crate::types::AgentReference {
        crate::types::AgentReference {
            source_id: source_id.into(),
            relative_path: "engineering/frontend-developer.md".into(),
        }
    }

    fn record() -> OllamaDeploymentRecord {
        OllamaDeploymentRecord {
            reference: reference("builtin"),
            agent_name: "Frontend Developer".into(),
            agent_slug: "frontend-developer".into(),
            target_name: target_name("frontend-developer", &reference("builtin")),
            base_model: "qwen2.5-coder:14b".into(),
            base_digest: "a".repeat(64),
            source_hash: "b".repeat(64),
            prompt_hash: "c".repeat(64),
            deployed_at: "2026-08-16T12:00:00Z".into(),
        }
    }

    fn facts() -> AgentFacts {
        AgentFacts {
            reference: reference("builtin"),
            name: "Frontend Developer".into(),
            slug: "frontend-developer".into(),
            source_hash: "b".repeat(64),
            prompt: "System with literal \"\"\" delimiters.".into(),
        }
    }

    #[derive(Default)]
    struct FakeBackend {
        models: Mutex<Vec<OllamaModel>>,
        systems: Mutex<BTreeMap<String, String>>,
        calls: Mutex<Vec<String>>,
    }

    impl FakeBackend {
        fn with_models(models: Vec<OllamaModel>) -> Self {
            Self {
                models: Mutex::new(models),
                ..Self::default()
            }
        }
    }

    #[async_trait]
    impl OllamaBackend for FakeBackend {
        async fn models(&self) -> Result<Vec<OllamaModel>, AppError> {
            self.calls.lock().unwrap().push("tags".into());
            Ok(self.models.lock().unwrap().clone())
        }

        async fn system_prompt(&self, model: &str) -> Result<Option<String>, AppError> {
            self.calls.lock().unwrap().push(format!("show:{model}"));
            Ok(self.systems.lock().unwrap().get(model).cloned())
        }

        async fn create(&self, model: &str, base: &str, system: &str) -> Result<(), AppError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("create:{model}:{base}:{system}"));
            Ok(())
        }

        async fn copy(&self, source: &str, destination: &str) -> Result<(), AppError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("copy:{source}:{destination}"));
            Ok(())
        }

        async fn delete(&self, model: &str) -> Result<(), AppError> {
            self.calls.lock().unwrap().push(format!("delete:{model}"));
            Ok(())
        }
    }

    #[test]
    fn deployment_records_are_bounded_unique_and_hash_validated() {
        assert!(validate_deployments(&[record()]).is_ok());

        let mut duplicate = record();
        duplicate.reference = reference("other");
        assert!(validate_deployments(&[record(), duplicate]).is_err());

        let mut invalid = record();
        invalid.prompt_hash = "short".into();
        assert!(validate_deployments(&[invalid]).is_err());

        let oversized = vec![record(); MAX_DEPLOYMENTS + 1];
        assert!(validate_deployments(&oversized).is_err());
    }

    #[test]
    fn target_names_are_stable_namespaced_and_source_specific() {
        let first = target_name("Frontend Developer", &reference("builtin"));
        let same = target_name("Frontend Developer", &reference("builtin"));
        let other = target_name("Frontend Developer", &reference("other"));

        assert_eq!(first, same);
        assert_ne!(first, other);
        assert!(first.starts_with("agency-agents/frontend-developer-"));
        assert!(first.ends_with(":latest"));
        assert!(valid_model_name(&first));
    }

    #[test]
    fn inventory_is_sorted_deduplicated_and_excludes_cloud_and_recovery_models() {
        let models = vec![
            OllamaModel {
                name: "qwen:latest".into(),
                digest: "2".repeat(64),
                size: 2,
            },
            OllamaModel {
                name: "gemma:latest".into(),
                digest: "1".repeat(64),
                size: 1,
            },
            OllamaModel {
                name: "qwen:latest".into(),
                digest: "2".repeat(64),
                size: 2,
            },
            OllamaModel {
                name: "glm:cloud".into(),
                digest: "3".repeat(64),
                size: 0,
            },
            OllamaModel {
                name: "agency-agents-recovery/123:latest".into(),
                digest: "4".repeat(64),
                size: 1,
            },
        ];

        assert_eq!(
            eligible_models(models)
                .into_iter()
                .map(|model| model.name)
                .collect::<Vec<_>>(),
            vec!["gemma:latest", "qwen:latest"],
        );
    }

    #[test]
    fn reconciliation_classifies_each_supported_state() {
        let tracked = "a".repeat(64);
        let changed = "b".repeat(64);

        assert_eq!(
            classify_deployment(Some(&tracked), Some(&tracked), Some(&tracked)),
            OllamaDeploymentState::Current
        );
        assert_eq!(
            classify_deployment(Some(&tracked), Some(&changed), Some(&tracked)),
            OllamaDeploymentState::Outdated
        );
        assert_eq!(
            classify_deployment(Some(&tracked), Some(&tracked), Some(&changed)),
            OllamaDeploymentState::Modified
        );
        assert_eq!(
            classify_deployment(Some(&tracked), Some(&tracked), None),
            OllamaDeploymentState::Missing
        );
        assert_eq!(
            classify_deployment(Some(&tracked), None, Some(&tracked)),
            OllamaDeploymentState::SourceUnavailable
        );
    }

    #[test]
    fn plan_revision_covers_every_mutation_relevant_fact() {
        let base = test_plan();
        let revision = revision_for(&base).unwrap();
        assert_eq!(revision, revision_for(&base).unwrap());

        let mut changed_source = base.clone();
        changed_source.source_hash = "d".repeat(64);
        assert_ne!(revision, revision_for(&changed_source).unwrap());

        let mut changed_base = base.clone();
        changed_base.base_digest = Some("e".repeat(64));
        assert_ne!(revision, revision_for(&changed_base).unwrap());

        let mut changed_state = base;
        changed_state.state = Some(OllamaDeploymentState::Modified);
        assert_ne!(revision, revision_for(&changed_state).unwrap());
    }

    #[tokio::test]
    async fn planning_is_read_only_and_blocks_an_unmanaged_target_collision() {
        let agent = facts();
        let target = target_name(&agent.slug, &agent.reference);
        let backend = FakeBackend::with_models(vec![
            OllamaModel {
                name: "qwen:latest".into(),
                digest: "a".repeat(64),
                size: 1,
            },
            OllamaModel {
                name: target.clone(),
                digest: "d".repeat(64),
                size: 1,
            },
        ]);
        backend
            .systems
            .lock()
            .unwrap()
            .insert(target, "someone else's prompt".into());

        let plan = build_plan_with(&backend, &[], agent, "create", Some("qwen:latest".into()))
            .await
            .unwrap();

        assert!(plan
            .blockers
            .iter()
            .any(|blocker| blocker.contains("not managed")));
        assert!(backend
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|call| call == "tags" || call.starts_with("show:")));
    }

    #[tokio::test]
    async fn vanished_base_changes_the_plan_and_blocks_application() {
        let backend = FakeBackend::with_models(vec![OllamaModel {
            name: "qwen:latest".into(),
            digest: "a".repeat(64),
            size: 1,
        }]);
        let reviewed =
            build_plan_with(&backend, &[], facts(), "create", Some("qwen:latest".into()))
                .await
                .unwrap();
        *backend.models.lock().unwrap() = Vec::new();
        let fresh = build_plan_with(&backend, &[], facts(), "create", Some("qwen:latest".into()))
            .await
            .unwrap();

        assert!(fresh
            .blockers
            .iter()
            .any(|blocker| blocker.contains("no longer installed")));
        assert!(require_plan_revision(&fresh, &reviewed.revision).is_err());
        assert!(!backend
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| call.starts_with("create:")));
    }

    #[tokio::test]
    async fn loopback_json_create_preserves_prompt_delimiters_and_uses_only_allowed_endpoints() {
        use axum::{
            extract::State,
            http::Method,
            routing::{delete, get, post},
            Json, Router,
        };
        use serde_json::{json, Value};

        #[derive(Clone, Default)]
        struct Log(Arc<Mutex<Vec<(Method, String, Value)>>>);

        async fn tags(State(log): State<Log>) -> Json<Value> {
            log.0
                .lock()
                .unwrap()
                .push((Method::GET, "/api/tags".into(), Value::Null));
            Json(
                json!({"models":[{"name":"qwen:latest","digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","size":1}]}),
            )
        }
        async fn show(State(log): State<Log>, Json(body): Json<Value>) -> Json<Value> {
            log.0
                .lock()
                .unwrap()
                .push((Method::POST, "/api/show".into(), body));
            Json(json!({"system":"existing"}))
        }
        async fn create(State(log): State<Log>, Json(body): Json<Value>) -> Json<Value> {
            log.0
                .lock()
                .unwrap()
                .push((Method::POST, "/api/create".into(), body));
            Json(json!({"status":"success"}))
        }
        async fn copy(State(log): State<Log>, Json(body): Json<Value>) {
            log.0
                .lock()
                .unwrap()
                .push((Method::POST, "/api/copy".into(), body));
        }
        async fn remove(State(log): State<Log>, Json(body): Json<Value>) {
            log.0
                .lock()
                .unwrap()
                .push((Method::DELETE, "/api/delete".into(), body));
        }

        let log = Log::default();
        let app = Router::new()
            .route("/api/tags", get(tags))
            .route("/api/show", post(show))
            .route("/api/create", post(create))
            .route("/api/copy", post(copy))
            .route("/api/delete", delete(remove))
            .with_state(log.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let backend = HttpOllamaBackend::new(&format!("http://{address}/api")).unwrap();
        let prompt = "literal \"\"\" and {{ .System }} remain data";

        backend.models().await.unwrap();
        backend.system_prompt("qwen:latest").await.unwrap();
        backend
            .create("agency-agents/test:latest", "qwen:latest", prompt)
            .await
            .unwrap();
        backend
            .copy(
                "agency-agents/test:latest",
                "agency-agents-recovery/test:latest",
            )
            .await
            .unwrap();
        backend
            .delete("agency-agents-recovery/test:latest")
            .await
            .unwrap();

        let calls = log.0.lock().unwrap();
        let create_body = &calls
            .iter()
            .find(|(_, path, _)| path == "/api/create")
            .unwrap()
            .2;
        assert_eq!(create_body["system"], prompt);
        assert!(calls.iter().all(|(_, path, _)| [
            "/api/tags",
            "/api/show",
            "/api/create",
            "/api/copy",
            "/api/delete"
        ]
        .contains(&path.as_str())));
        server.abort();
    }

    #[derive(Default)]
    struct FakeStore {
        saves: Mutex<Vec<Vec<OllamaDeploymentRecord>>>,
        fail_save: bool,
    }

    #[async_trait]
    impl DeploymentStore for FakeStore {
        async fn save(&self, records: Vec<OllamaDeploymentRecord>) -> Result<(), AppError> {
            if self.fail_save {
                return Err(AppError::Io {
                    message: "injected deployment save failure".into(),
                });
            }
            self.saves.lock().unwrap().push(records);
            Ok(())
        }
    }

    #[derive(Default)]
    struct FaultBackend {
        systems: Mutex<BTreeMap<String, String>>,
        fail: Mutex<HashSet<String>>,
        calls: Mutex<Vec<String>>,
    }

    impl FaultBackend {
        fn failing(self, operation: &str) -> Self {
            self.fail.lock().unwrap().insert(operation.into());
            self
        }

        fn should_fail(&self, operation: &str) -> Result<(), AppError> {
            if self.fail.lock().unwrap().contains(operation) {
                Err(AppError::Io {
                    message: format!("injected {operation} failure"),
                })
            } else {
                Ok(())
            }
        }
    }

    #[async_trait]
    impl OllamaBackend for FaultBackend {
        async fn models(&self) -> Result<Vec<OllamaModel>, AppError> {
            unreachable!()
        }
        async fn system_prompt(&self, model: &str) -> Result<Option<String>, AppError> {
            Ok(self.systems.lock().unwrap().get(model).cloned())
        }
        async fn create(&self, model: &str, _base: &str, system: &str) -> Result<(), AppError> {
            self.calls.lock().unwrap().push("create".into());
            self.should_fail("create")?;
            self.systems
                .lock()
                .unwrap()
                .insert(model.into(), system.into());
            Ok(())
        }
        async fn copy(&self, source: &str, destination: &str) -> Result<(), AppError> {
            self.calls.lock().unwrap().push("copy".into());
            self.should_fail("copy")?;
            let value = self
                .systems
                .lock()
                .unwrap()
                .get(source)
                .cloned()
                .ok_or_else(|| invalid("copy source missing"))?;
            self.systems
                .lock()
                .unwrap()
                .insert(destination.into(), value);
            Ok(())
        }
        async fn delete(&self, model: &str) -> Result<(), AppError> {
            self.calls.lock().unwrap().push("delete".into());
            self.should_fail("delete")?;
            self.systems.lock().unwrap().remove(model);
            Ok(())
        }
    }

    fn update_fixture() -> (Vec<OllamaDeploymentRecord>, OllamaMutationPlan, String) {
        let mut old = record();
        let old_prompt = "old system prompt";
        old.prompt_hash = crate::render::sha256_hex(old_prompt.as_bytes());
        let mut plan = test_plan();
        plan.operation = "update".into();
        plan.state = Some(OllamaDeploymentState::Outdated);
        plan.prompt_preview = Some("new system prompt".into());
        plan.prompt_hash = crate::render::sha256_hex(b"new system prompt");
        plan.target_name = old.target_name.clone();
        plan.reference = old.reference.clone();
        plan.revision = revision_for(&plan).unwrap();
        (vec![old], plan, old_prompt.into())
    }

    #[tokio::test]
    async fn preservation_failure_aborts_before_target_or_state_mutation() {
        let (records, plan, old_prompt) = update_fixture();
        let backend = FaultBackend::default().failing("copy");
        backend
            .systems
            .lock()
            .unwrap()
            .insert(plan.target_name.clone(), old_prompt.clone());
        let store = FakeStore::default();

        assert!(execute_plan_with(&backend, &store, records, plan.clone())
            .await
            .is_err());
        assert_eq!(
            backend.systems.lock().unwrap().get(&plan.target_name),
            Some(&old_prompt)
        );
        assert!(store.saves.lock().unwrap().is_empty());
        assert_eq!(*backend.calls.lock().unwrap(), vec!["copy"]);
    }

    #[tokio::test]
    async fn failed_update_restores_the_preserved_target_and_cleans_recovery() {
        let (records, plan, old_prompt) = update_fixture();
        let backend = FaultBackend::default().failing("create");
        backend
            .systems
            .lock()
            .unwrap()
            .insert(plan.target_name.clone(), old_prompt.clone());
        let store = FakeStore::default();

        assert!(execute_plan_with(&backend, &store, records, plan.clone())
            .await
            .is_err());
        let recovery = recovery_name(&plan.target_name);
        assert_eq!(
            backend.systems.lock().unwrap().get(&plan.target_name),
            Some(&old_prompt)
        );
        assert!(!backend.systems.lock().unwrap().contains_key(&recovery));
        assert!(store.saves.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn failed_first_create_state_commit_removes_the_new_target() {
        let mut plan = test_plan();
        plan.revision = revision_for(&plan).unwrap();
        let backend = FaultBackend::default();
        let store = FakeStore {
            fail_save: true,
            ..FakeStore::default()
        };

        assert!(
            execute_plan_with(&backend, &store, Vec::new(), plan.clone())
                .await
                .is_err()
        );
        assert!(!backend
            .systems
            .lock()
            .unwrap()
            .contains_key(&plan.target_name));
    }

    #[tokio::test]
    async fn failed_remove_state_commit_restores_target_and_record() {
        let (records, mut plan, old_prompt) = update_fixture();
        plan.operation = "remove".into();
        plan.revision = revision_for(&plan).unwrap();
        let backend = FaultBackend::default();
        backend
            .systems
            .lock()
            .unwrap()
            .insert(plan.target_name.clone(), old_prompt.clone());
        let store = FakeStore {
            fail_save: true,
            ..FakeStore::default()
        };

        assert!(execute_plan_with(&backend, &store, records, plan.clone())
            .await
            .is_err());
        assert_eq!(
            backend.systems.lock().unwrap().get(&plan.target_name),
            Some(&old_prompt)
        );
        assert!(!backend
            .systems
            .lock()
            .unwrap()
            .contains_key(&recovery_name(&plan.target_name)));
    }

    #[tokio::test]
    async fn successful_update_commits_state_and_removes_temporary_recovery_model() {
        let (records, plan, old_prompt) = update_fixture();
        let backend = FaultBackend::default();
        backend
            .systems
            .lock()
            .unwrap()
            .insert(plan.target_name.clone(), old_prompt);
        let store = FakeStore::default();

        execute_plan_with(&backend, &store, records, plan.clone())
            .await
            .unwrap();
        assert_eq!(
            backend
                .systems
                .lock()
                .unwrap()
                .get(&plan.target_name)
                .map(String::as_str),
            Some("new system prompt")
        );
        assert!(!backend
            .systems
            .lock()
            .unwrap()
            .contains_key(&recovery_name(&plan.target_name)));
        assert_eq!(store.saves.lock().unwrap().len(), 1);
    }
}
