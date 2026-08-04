use std::collections::HashSet;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::error::AppError;
use crate::state::AppState;
use crate::types::SkillMutationPlan;
use crate::util::fs::{atomic_write, read_capped};

const SCHEMA_VERSION: u32 = 1;
const MAX_FILE_BYTES: u64 = 512 * 1024;
const MAX_EXPERT_STATE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_EXPERTS: usize = 200;
const MAX_CREATION_REQUESTS: usize = 200;
const MAX_TEXT: usize = 4096;
const BUNDLED: &str = include_str!("../data/experts.json");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpertDefinition {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub category: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub version: u32,
    pub lead_agent: String,
    #[serde(default)]
    pub supporting_agents: Vec<String>,
    #[serde(default)]
    pub required_skills: Vec<String>,
    #[serde(default)]
    pub optional_skills: Vec<String>,
    pub runbook: Option<String>,
    pub preferred_client: Option<String>,
    pub starter_prompt: String,
    #[serde(default)]
    pub quality_contract: crate::expert_runs::QualityContract,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpertProposalInput {
    pub name: String,
    pub summary: String,
    pub category: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub lead_agent: String,
    #[serde(default)]
    pub supporting_agents: Vec<String>,
    #[serde(default)]
    pub required_skills: Vec<String>,
    #[serde(default)]
    pub optional_skills: Vec<String>,
    pub runbook: Option<String>,
    pub preferred_client: Option<String>,
    pub starter_prompt: String,
    #[serde(default)]
    pub quality_contract: crate::expert_runs::QualityContract,
}

impl ExpertProposalInput {
    fn into_definition(self, id: String) -> ExpertDefinition {
        ExpertDefinition {
            id,
            name: self.name,
            summary: self.summary,
            category: self.category,
            tags: self.tags,
            version: 1,
            lead_agent: self.lead_agent,
            supporting_agents: self.supporting_agents,
            required_skills: self.required_skills,
            optional_skills: self.optional_skills,
            runbook: self.runbook,
            preferred_client: self.preferred_client,
            starter_prompt: self.starter_prompt,
            quality_contract: self.quality_contract,
            source: "custom".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LinkedSkillDraft {
    pub skill_name: String,
    pub draft_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSubstitution {
    pub needed_capability: String,
    pub selected_catalog_slug: String,
    pub rationale: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExpertCreationState {
    Pending,
    Approved,
    Rejected,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExpertChangeKind {
    #[default]
    Create,
    Update,
    Clone,
    Archive,
    Delete,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExpertReadiness {
    WaitingForSkills,
    Ready,
    Blocked,
}

#[derive(Debug, Clone)]
struct CatalogSkill {
    normalized_name: String,
    preferred: bool,
}

#[derive(Debug, Clone)]
struct DraftAvailability {
    id: String,
    name: Option<String>,
    state: crate::types::SkillDraftState,
    installable: bool,
}

#[derive(Debug)]
struct ReadinessEvaluation {
    readiness: ExpertReadiness,
    blockers: Vec<String>,
    warnings: Vec<String>,
}

fn normalize_logical_name(value: &str) -> String {
    let mut out = String::new();
    let mut separator = false;
    for character in value.trim().chars() {
        if character.is_ascii_alphanumeric() {
            if separator && !out.is_empty() {
                out.push('-');
            }
            out.push(character.to_ascii_lowercase());
            separator = false;
        } else {
            separator = true;
        }
    }
    out
}

fn validate_proposal_references(
    proposal: &mut ExpertProposalInput,
    links: &[LinkedSkillDraft],
    substitutions: &[AgentSubstitution],
    known_agents: &HashSet<&str>,
    known_runbooks: &HashSet<&str>,
) -> Result<(), AppError> {
    let mut definition = proposal.clone().into_definition("proposal".into());
    validate(&mut definition, true)?;
    if definition.tags.len() > 64
        || definition.supporting_agents.len() > 64
        || definition.required_skills.len() > 64
        || definition.optional_skills.len() > 64
    {
        return Err(invalid("expert proposal contains too many list items"));
    }
    if definition
        .tags
        .iter()
        .chain(definition.required_skills.iter())
        .chain(definition.optional_skills.iter())
        .any(|item| item.trim().is_empty() || item.len() > 160)
    {
        return Err(invalid("expert proposal contains an invalid list item"));
    }
    if definition
        .required_skills
        .iter()
        .chain(definition.optional_skills.iter())
        .any(|name| normalize_logical_name(name).is_empty())
    {
        return Err(invalid("expert proposal contains an invalid skill name"));
    }
    let roster = std::iter::once(definition.lead_agent.as_str())
        .chain(definition.supporting_agents.iter().map(String::as_str))
        .collect::<HashSet<_>>();
    if let Some(unknown) = roster.iter().find(|slug| !known_agents.contains(**slug)) {
        return Err(invalid(format!("unknown agent: {unknown}")));
    }
    if definition
        .runbook
        .as_deref()
        .is_some_and(|runbook| !known_runbooks.contains(runbook))
    {
        return Err(invalid("unknown runbook"));
    }
    let skills = definition
        .required_skills
        .iter()
        .chain(definition.optional_skills.iter())
        .map(|name| normalize_logical_name(name))
        .collect::<HashSet<_>>();
    let mut linked_names = HashSet::new();
    let mut linked_ids = HashSet::new();
    for link in links {
        let name = normalize_logical_name(&link.skill_name);
        if !skills.contains(&name)
            || !linked_names.insert(name)
            || !linked_ids.insert(link.draft_id.as_str())
            || uuid::Uuid::parse_str(&link.draft_id).is_err()
        {
            return Err(invalid("skill draft link is invalid or mismatched"));
        }
    }
    for substitution in substitutions {
        if substitution.needed_capability.trim().is_empty()
            || substitution.rationale.trim().is_empty()
            || substitution.needed_capability.len() > MAX_TEXT
            || substitution.rationale.len() > MAX_TEXT
            || !known_agents.contains(substitution.selected_catalog_slug.as_str())
            || !roster.contains(substitution.selected_catalog_slug.as_str())
        {
            return Err(invalid("agent substitution is invalid"));
        }
    }
    *proposal = ExpertProposalInput {
        name: definition.name,
        summary: definition.summary,
        category: definition.category,
        tags: definition.tags,
        lead_agent: definition.lead_agent,
        supporting_agents: definition.supporting_agents,
        required_skills: definition.required_skills,
        optional_skills: definition.optional_skills,
        runbook: definition.runbook,
        preferred_client: definition.preferred_client,
        starter_prompt: definition.starter_prompt,
        quality_contract: definition.quality_contract,
    };
    Ok(())
}

fn catalog_skill_resolves(name: &str, catalog: &[CatalogSkill]) -> Result<bool, ()> {
    let matches = catalog
        .iter()
        .filter(|skill| skill.normalized_name == name)
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return Ok(false);
    }
    if matches.len() == 1 || matches.iter().filter(|skill| skill.preferred).count() == 1 {
        Ok(true)
    } else {
        Err(())
    }
}

fn derive_readiness(
    proposal: &ExpertProposalInput,
    links: &[LinkedSkillDraft],
    catalog: &[CatalogSkill],
    drafts: &[DraftAvailability],
) -> ReadinessEvaluation {
    let mut blockers = Vec::new();
    let mut warnings = Vec::new();
    let mut waiting = false;
    for (required, names) in [
        (true, &proposal.required_skills),
        (false, &proposal.optional_skills),
    ] {
        for name in names {
            let normalized = normalize_logical_name(name);
            match catalog_skill_resolves(&normalized, catalog) {
                Ok(true) => continue,
                Err(()) => {
                    let message = format!("ambiguous skill: {name}");
                    if required {
                        blockers.push(message);
                    } else {
                        warnings.push(message);
                    }
                    continue;
                }
                Ok(false) => {}
            }
            let link = links
                .iter()
                .find(|link| normalize_logical_name(&link.skill_name) == normalized);
            let Some(link) = link else {
                let message = format!("missing skill: {name}");
                if required {
                    blockers.push(message);
                } else {
                    warnings.push(message);
                }
                continue;
            };
            let draft = drafts.iter().find(|draft| draft.id == link.draft_id);
            let Some(draft) = draft else {
                let message = format!("missing linked skill draft: {name}");
                if required {
                    blockers.push(message);
                } else {
                    warnings.push(message);
                }
                continue;
            };
            if !draft.installable
                || draft.name.as_deref().map(normalize_logical_name).as_deref()
                    != Some(normalized.as_str())
            {
                let message = format!("invalid or changed linked skill draft: {name}");
                if required {
                    blockers.push(message);
                } else {
                    warnings.push(message);
                }
                continue;
            }
            match draft.state {
                crate::types::SkillDraftState::Pending if required => waiting = true,
                crate::types::SkillDraftState::Pending => {
                    warnings.push(format!("optional skill draft pending: {name}"))
                }
                crate::types::SkillDraftState::Published => {}
                crate::types::SkillDraftState::Rejected if required => {
                    blockers.push(format!("linked skill draft rejected: {name}"))
                }
                crate::types::SkillDraftState::Rejected => {
                    warnings.push(format!("optional skill draft rejected: {name}"))
                }
            }
        }
    }
    blockers.sort();
    blockers.dedup();
    warnings.sort();
    warnings.dedup();
    ReadinessEvaluation {
        readiness: if !blockers.is_empty() {
            ExpertReadiness::Blocked
        } else if waiting {
            ExpertReadiness::WaitingForSkills
        } else {
            ExpertReadiness::Ready
        },
        blockers,
        warnings,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExpertFile {
    schema_version: u32,
    #[serde(default)]
    experts: Vec<ExpertDefinition>,
    #[serde(default)]
    creation_requests: Vec<ExpertCreationRequest>,
    #[serde(default)]
    archived_experts: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpertResolved {
    #[serde(flatten)]
    pub definition: ExpertDefinition,
    pub unresolved_agents: Vec<String>,
    pub unresolved_skills: Vec<String>,
    pub unresolved_runbook: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpertAgentAction {
    pub slug: String,
    pub status: String,
    pub destination: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpertActivationPlan {
    pub expert: ExpertResolved,
    pub project_path: String,
    pub client: String,
    pub agents: Vec<ExpertAgentAction>,
    pub skills: Vec<SkillMutationPlan>,
    pub existing: Vec<String>,
    pub warnings: Vec<String>,
    pub blockers: Vec<String>,
    pub prompt_preview: String,
    pub rollback_scope: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpertActivationRecord {
    pub id: String,
    pub expert_id: String,
    pub expert_version: u32,
    pub project_path: String,
    pub client: String,
    pub activated_at: String,
    pub installed_agents: Vec<String>,
    pub installed_skills: Vec<String>,
    #[serde(default)]
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpertActivationRequest {
    pub id: String,
    pub expert_id: String,
    pub project_path: String,
    pub client: Option<String>,
    pub requested_by: String,
    pub requested_at: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpertCreationRequest {
    pub id: String,
    pub client_request_id: String,
    pub outcome: String,
    pub project_path: String,
    pub requested_by: String,
    pub requested_at: String,
    pub proposal: ExpertProposalInput,
    #[serde(default)]
    pub linked_skill_drafts: Vec<LinkedSkillDraft>,
    #[serde(default)]
    pub agent_substitutions: Vec<AgentSubstitution>,
    pub state: ExpertCreationState,
    pub saved_expert_id: Option<String>,
    #[serde(default)]
    pub kind: ExpertChangeKind,
    #[serde(default)]
    pub target_expert_id: Option<String>,
    #[serde(default)]
    pub base_version: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpertLinkedSkillState {
    pub skill_name: String,
    pub draft_id: String,
    pub state: Option<crate::types::SkillDraftState>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpertCreationRequestView {
    #[serde(flatten)]
    pub request: ExpertCreationRequest,
    pub readiness: ExpertReadiness,
    pub linked_skill_states: Vec<ExpertLinkedSkillState>,
    pub blockers: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExpertContextAgent {
    slug: String,
    name: String,
    category: String,
    description: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExpertContextRunbook {
    slug: String,
    title: String,
    summary: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExpertContextExcerpt {
    file: String,
    excerpt: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExpertCreationContext {
    project_path: String,
    project_name: String,
    languages: Vec<String>,
    manifests: Vec<String>,
    instruction_excerpts: Vec<ExpertContextExcerpt>,
    agents: Vec<ExpertContextAgent>,
    runbooks: Vec<ExpertContextRunbook>,
    detected_clients: Vec<String>,
    similar_experts: Vec<ExpertContextExpert>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExpertContextExpert {
    id: String,
    name: String,
    summary: String,
    category: String,
    tags: Vec<String>,
    lead_agent: String,
    supporting_agents: Vec<String>,
    required_skills: Vec<String>,
    optional_skills: Vec<String>,
    runbook: Option<String>,
    preferred_client: Option<String>,
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::InvalidArgument {
        message: message.into(),
    }
}

fn state_path(state: &AppState, name: &str) -> PathBuf {
    state.app_data_dir.join("state").join(name)
}

fn lock_expert_state(state: &AppState) -> Result<std::fs::File, AppError> {
    let directory = state.app_data_dir.join("state");
    std::fs::create_dir_all(&directory).map_err(|error| AppError::Io {
        message: format!("create Expert state directory: {error}"),
    })?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("experts.lock"))
        .map_err(|error| AppError::Io {
            message: format!("open Expert state lock: {error}"),
        })?;
    file.lock().map_err(|error| AppError::Io {
        message: format!("lock Expert state: {error}"),
    })?;
    Ok(file)
}

fn insert_creation_request(
    requests: &mut Vec<ExpertCreationRequest>,
    request: ExpertCreationRequest,
) -> Result<ExpertCreationRequest, AppError> {
    if let Some(existing) = requests.iter().find(|existing| {
        existing.client_request_id == request.client_request_id
            && existing.requested_by == request.requested_by
    }) {
        return Ok(existing.clone());
    }
    if requests.len() >= MAX_CREATION_REQUESTS {
        let Some(index) = requests
            .iter()
            .position(|request| request.state != ExpertCreationState::Pending)
        else {
            return Err(invalid("Expert creation request inbox is full"));
        };
        requests.remove(index);
    }
    requests.push(request.clone());
    Ok(request)
}

fn revise_pending_change_request(
    request: &mut ExpertCreationRequest,
    requested_by: &str,
    proposal: ExpertProposalInput,
    base_version: Option<u32>,
) -> Result<(), AppError> {
    if request.requested_by != requested_by || request.state != ExpertCreationState::Pending {
        return Err(invalid(
            "caller does not own a pending Expert change request",
        ));
    }
    validate_portable_proposal(&proposal, Path::new(&request.project_path))?;
    request.proposal = proposal;
    request.base_version = base_version;
    Ok(())
}

fn cancel_pending_change_request(
    request: &mut ExpertCreationRequest,
    requested_by: &str,
) -> Result<(), AppError> {
    if request.requested_by != requested_by || request.state != ExpertCreationState::Pending {
        return Err(invalid(
            "caller does not own a pending Expert change request",
        ));
    }
    request.state = ExpertCreationState::Cancelled;
    Ok(())
}

fn cancel_pending_activation_request(
    request: &mut ExpertActivationRequest,
    requested_by: &str,
) -> Result<(), AppError> {
    if request.requested_by != requested_by || request.state != "pending" {
        return Err(invalid(
            "caller does not own a pending Expert activation request",
        ));
    }
    request.state = "cancelled".into();
    Ok(())
}

fn validate(definition: &mut ExpertDefinition, custom: bool) -> Result<(), AppError> {
    let bounded = [
        ("id", &definition.id),
        ("name", &definition.name),
        ("summary", &definition.summary),
        ("category", &definition.category),
        ("leadAgent", &definition.lead_agent),
        ("starterPrompt", &definition.starter_prompt),
    ];
    for (field, value) in bounded {
        if value.trim().is_empty() || value.len() > MAX_TEXT {
            return Err(invalid(format!("{field} is empty or oversized")));
        }
    }
    if definition.version == 0 {
        return Err(invalid("expert version must be positive"));
    }
    crate::expert_runs::validate_contract(&definition.quality_contract)?;
    if !definition
        .id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(invalid("expert id contains unsupported characters"));
    }
    if definition.starter_prompt.matches("{{").count()
        != definition.starter_prompt.matches("}}").count()
    {
        return Err(invalid("starter prompt has malformed template markers"));
    }
    if !matches!(
        definition.preferred_client.as_deref(),
        None | Some("claudeCode") | Some("codex")
    ) {
        return Err(invalid(
            "preferredClient must be claudeCode, codex, or null",
        ));
    }
    definition
        .supporting_agents
        .retain(|slug| slug != &definition.lead_agent);
    definition.supporting_agents.sort();
    definition.supporting_agents.dedup();
    definition.required_skills.sort();
    definition.required_skills.dedup();
    definition
        .optional_skills
        .retain(|name| !definition.required_skills.contains(name));
    definition.optional_skills.sort();
    definition.optional_skills.dedup();
    definition.source = if custom { "custom" } else { "curated" }.into();
    Ok(())
}

fn parse_file(raw: &str, custom: bool) -> Result<Vec<ExpertDefinition>, AppError> {
    let mut file: ExpertFile =
        serde_json::from_str(raw).map_err(|e| invalid(format!("parse experts: {e}")))?;
    if file.schema_version != SCHEMA_VERSION {
        return Err(invalid(format!(
            "unsupported expert schema version: {}",
            file.schema_version
        )));
    }
    if file.experts.len() > MAX_EXPERTS {
        return Err(invalid("too many experts"));
    }
    let mut ids = HashSet::new();
    for expert in &mut file.experts {
        validate(expert, custom)?;
        if !ids.insert(expert.id.clone()) {
            return Err(invalid(format!("duplicate expert id: {}", expert.id)));
        }
    }
    Ok(file.experts)
}

async fn custom_state(state: &AppState) -> Result<ExpertFile, AppError> {
    let path = state_path(state, "experts.json");
    if !path.exists() {
        return Ok(ExpertFile {
            schema_version: SCHEMA_VERSION,
            experts: Vec::new(),
            creation_requests: Vec::new(),
            archived_experts: Vec::new(),
        });
    }
    let raw = read_capped(&path, MAX_EXPERT_STATE_BYTES).await?;
    let text = String::from_utf8(raw).map_err(|_| invalid("custom experts must be UTF-8"))?;
    let mut file: ExpertFile =
        serde_json::from_str(&text).map_err(|error| invalid(format!("parse experts: {error}")))?;
    if file.schema_version != SCHEMA_VERSION
        || file.experts.len() > MAX_EXPERTS
        || file.creation_requests.len() > MAX_CREATION_REQUESTS
    {
        return Err(invalid("invalid Expert state file"));
    }
    let mut ids = HashSet::new();
    for expert in &mut file.experts {
        validate(expert, true)?;
        if !ids.insert(expert.id.clone()) {
            return Err(invalid(format!("duplicate expert id: {}", expert.id)));
        }
    }
    Ok(file)
}

async fn custom_list(state: &AppState) -> Result<Vec<ExpertDefinition>, AppError> {
    Ok(custom_state(state).await?.experts)
}

async fn save_custom(state: &AppState, experts: Vec<ExpertDefinition>) -> Result<(), AppError> {
    let mut file = custom_state(state).await?;
    file.experts = experts;
    save_expert_state(state, &file).await
}

async fn save_expert_state(state: &AppState, file: &ExpertFile) -> Result<(), AppError> {
    let bytes =
        serde_json::to_vec_pretty(file).map_err(|e| invalid(format!("serialize experts: {e}")))?;
    atomic_write(&state_path(state, "experts.json"), &bytes).await
}

fn apply_change_request(
    file: &mut ExpertFile,
    request: &mut ExpertCreationRequest,
    proposal: ExpertProposalInput,
) -> Result<Option<String>, AppError> {
    let target = request.target_expert_id.clone();
    let saved = match request.kind {
        ExpertChangeKind::Create | ExpertChangeKind::Clone => {
            let used = file
                .experts
                .iter()
                .map(|expert| expert.id.clone())
                .collect::<HashSet<_>>();
            let id = collision_safe_custom_id(&proposal.name, &used);
            let mut definition = proposal.clone().into_definition(id.clone());
            validate(&mut definition, true)?;
            file.experts.push(definition);
            Some(id)
        }
        ExpertChangeKind::Update => {
            let target = target.ok_or_else(|| invalid("update requires targetExpertId"))?;
            let expert = file
                .experts
                .iter_mut()
                .find(|expert| expert.id == target)
                .ok_or_else(|| invalid("Expert update target does not exist"))?;
            if request.base_version != Some(expert.version) {
                return Err(invalid("stale Expert version"));
            }
            let mut replacement = proposal.clone().into_definition(target.clone());
            replacement.version = expert.version + 1;
            validate(&mut replacement, true)?;
            *expert = replacement;
            file.archived_experts.retain(|id| id != &target);
            Some(target)
        }
        ExpertChangeKind::Archive => {
            let target = target.ok_or_else(|| invalid("archive requires targetExpertId"))?;
            if !file.archived_experts.contains(&target) {
                file.archived_experts.push(target.clone());
                file.archived_experts.sort();
            }
            Some(target)
        }
        ExpertChangeKind::Delete => {
            let target = target.ok_or_else(|| invalid("delete requires targetExpertId"))?;
            file.experts.retain(|expert| expert.id != target);
            if !file.archived_experts.contains(&target) {
                file.archived_experts.push(target.clone());
                file.archived_experts.sort();
            }
            Some(target)
        }
    };
    request.proposal = proposal;
    request.state = ExpertCreationState::Approved;
    request.saved_expert_id = saved.clone();
    Ok(saved)
}

async fn definitions(app: &AppHandle, state: &AppState) -> Result<Vec<ExpertDefinition>, AppError> {
    let root = crate::corpus::active_catalog_root(app).await?;
    let manifest = root.join("strategy").join("experts.json");
    let curated = if manifest.exists() {
        let raw = read_capped(&manifest, MAX_FILE_BYTES).await?;
        let text = String::from_utf8(raw).map_err(|_| invalid("catalog experts must be UTF-8"))?;
        parse_file(&text, false)?
    } else {
        parse_file(BUNDLED, false)?
    };
    let mut merged = curated;
    for custom in custom_list(state).await? {
        if let Some(existing) = merged.iter_mut().find(|item| item.id == custom.id) {
            *existing = custom;
        } else {
            merged.push(custom);
        }
    }
    let archived = custom_state(state).await?.archived_experts;
    merged.retain(|expert| !archived.contains(&expert.id));
    Ok(merged)
}

pub(crate) async fn mcp_definitions(state: &AppState) -> Result<Vec<ExpertDefinition>, AppError> {
    let root = crate::corpus::active_catalog_root_at(&state.app_data_dir).await;
    let manifest = root.join("strategy").join("experts.json");
    let mut merged = if manifest.exists() {
        let raw = read_capped(&manifest, MAX_FILE_BYTES).await?;
        let text = String::from_utf8(raw).map_err(|_| invalid("catalog experts must be UTF-8"))?;
        parse_file(&text, false)?
    } else {
        parse_file(BUNDLED, false)?
    };
    for custom in custom_list(state).await? {
        if let Some(existing) = merged.iter_mut().find(|item| item.id == custom.id) {
            *existing = custom;
        } else {
            merged.push(custom);
        }
    }
    let archived = custom_state(state).await?.archived_experts;
    merged.retain(|expert| !archived.contains(&expert.id));
    Ok(merged)
}

pub(crate) async fn mcp_plan(
    state: &AppState,
    id: &str,
    project_path: &str,
    client: Option<String>,
) -> Result<serde_json::Value, AppError> {
    let definition = mcp_definitions(state)
        .await?
        .into_iter()
        .find(|expert| expert.id == id)
        .ok_or_else(|| invalid("unknown expert"))?;
    let project = tokio::fs::canonicalize(project_path)
        .await
        .map_err(|e| invalid(format!("invalid project: {e}")))?;
    let client = client
        .or(definition.preferred_client.clone())
        .ok_or_else(|| invalid("activation requires a client"))?;
    if !matches!(client.as_str(), "claudeCode" | "codex") {
        return Err(invalid("unsupported client"));
    }
    Ok(serde_json::json!({
        "expert": definition,
        "projectPath": project,
        "client": client,
        "requiresDesktopReview": true
    }))
}

async fn catalog_agents(state: &AppState) -> Result<Vec<crate::types::CorpusEntry>, AppError> {
    let path = crate::corpus::state_dir(&state.app_data_dir).join("corpus-index.json");
    let raw = read_capped(&path, 4 * 1024 * 1024)
        .await
        .map_err(|_| invalid("agent catalog is unavailable; open the desktop catalog first"))?;
    let index: std::collections::BTreeMap<String, crate::types::CorpusEntry> =
        serde_json::from_slice(&raw)
            .map_err(|error| invalid(format!("parse agent catalog: {error}")))?;
    Ok(index.into_values().collect())
}

async fn catalog_runbooks(state: &AppState) -> Result<Vec<ExpertContextRunbook>, AppError> {
    #[derive(Deserialize)]
    struct File {
        #[serde(default)]
        runbooks: Vec<Row>,
    }
    #[derive(Deserialize)]
    struct Row {
        slug: String,
        title: String,
        summary: String,
    }
    let root = crate::corpus::active_catalog_root_at(&state.app_data_dir).await;
    let path = root.join("strategy").join("runbooks.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = read_capped(&path, MAX_FILE_BYTES).await?;
    let file: File = serde_json::from_slice(&raw)
        .map_err(|error| invalid(format!("parse runbooks: {error}")))?;
    Ok(file
        .runbooks
        .into_iter()
        .map(|row| ExpertContextRunbook {
            slug: row.slug,
            title: row.title,
            summary: row.summary,
        })
        .collect())
}

async fn catalog_skills(state: &AppState) -> Result<Vec<CatalogSkill>, AppError> {
    let preferred = crate::skills::organize::list(state)
        .await?
        .preferred_sources;
    Ok(crate::skills::inspect_skill_sources(state)
        .await?
        .into_iter()
        .flat_map(|source| source.packages)
        .filter(|package| package.installable)
        .filter_map(|package| {
            let name = package.name?;
            let normalized_name = normalize_logical_name(&name);
            let preferred = preferred.iter().any(|preference| {
                normalize_logical_name(&preference.skill_name) == normalized_name
                    && preference.source_id == package.source_id
            });
            Some(CatalogSkill {
                normalized_name,
                preferred,
            })
        })
        .collect())
}

async fn draft_availability(state: &AppState) -> Result<Vec<DraftAvailability>, AppError> {
    Ok(crate::skills::drafts::list(state)
        .await?
        .into_iter()
        .map(|draft| DraftAvailability {
            id: draft.id,
            name: draft.validation.name,
            state: draft.state,
            installable: draft.validation.installable,
        })
        .collect())
}

async fn creation_view(
    state: &AppState,
    request: ExpertCreationRequest,
) -> Result<ExpertCreationRequestView, AppError> {
    let drafts = draft_availability(state).await?;
    let evaluation = derive_readiness(
        &request.proposal,
        &request.linked_skill_drafts,
        &catalog_skills(state).await?,
        &drafts,
    );
    let linked_skill_states = request
        .linked_skill_drafts
        .iter()
        .map(|link| ExpertLinkedSkillState {
            skill_name: link.skill_name.clone(),
            draft_id: link.draft_id.clone(),
            state: drafts
                .iter()
                .find(|draft| draft.id == link.draft_id)
                .map(|draft| draft.state),
        })
        .collect();
    Ok(ExpertCreationRequestView {
        request,
        readiness: evaluation.readiness,
        linked_skill_states,
        blockers: evaluation.blockers,
        warnings: evaluation.warnings,
    })
}

fn validate_request_metadata(
    client_request_id: &str,
    outcome: &str,
    requested_by: &str,
) -> Result<(), AppError> {
    if client_request_id.trim().is_empty()
        || client_request_id.len() > 128
        || outcome.trim().is_empty()
        || outcome.len() > MAX_TEXT
        || requested_by.trim().is_empty()
        || requested_by.len() > 128
    {
        return Err(invalid("invalid Expert creation request metadata"));
    }
    Ok(())
}

fn validate_portable_proposal(
    proposal: &ExpertProposalInput,
    project_path: &Path,
) -> Result<(), AppError> {
    let serialized = serde_json::to_string(proposal)
        .map_err(|error| invalid(format!("serialize Expert proposal: {error}")))?;
    let normalized = serialized.to_ascii_lowercase();
    let project = project_path.to_string_lossy().to_ascii_lowercase();
    if (!project.is_empty() && normalized.contains(&project))
        || normalized.contains("/users/")
        || normalized.contains("c:\\\\users\\\\")
        || ["password=", "token=", "secret=", "api_key", "apikey"]
            .iter()
            .any(|marker| normalized.contains(marker))
    {
        return Err(invalid(
            "portable Expert proposals must not contain project paths or credentials",
        ));
    }
    Ok(())
}

async fn validate_live_proposal(
    state: &AppState,
    proposal: &mut ExpertProposalInput,
    links: &[LinkedSkillDraft],
    substitutions: &[AgentSubstitution],
) -> Result<(), AppError> {
    let agents = catalog_agents(state).await?;
    let known_agents = agents
        .iter()
        .map(|agent| agent.slug.as_str())
        .collect::<HashSet<_>>();
    let runbooks = catalog_runbooks(state).await?;
    let known_runbooks = runbooks
        .iter()
        .map(|runbook| runbook.slug.as_str())
        .collect::<HashSet<_>>();
    validate_proposal_references(
        proposal,
        links,
        substitutions,
        &known_agents,
        &known_runbooks,
    )
}

pub(crate) async fn mcp_validate_proposal(
    state: &AppState,
    project_path: &str,
    mut proposal: ExpertProposalInput,
    links: Vec<LinkedSkillDraft>,
    substitutions: Vec<AgentSubstitution>,
) -> Result<serde_json::Value, AppError> {
    let project = std::fs::canonicalize(project_path)
        .map_err(|error| invalid(format!("invalid project: {error}")))?;
    if Path::new(project_path) != project
        || !crate::install::project_is_registered(&state.app_data_dir, &project).await?
    {
        return Err(invalid(
            "project must be exactly canonical and registered in the desktop app",
        ));
    }
    validate_portable_proposal(&proposal, &project)?;
    validate_live_proposal(state, &mut proposal, &links, &substitutions).await?;
    let evaluation = derive_readiness(
        &proposal,
        &links,
        &catalog_skills(state).await?,
        &draft_availability(state).await?,
    );
    Ok(serde_json::json!({
        "valid": evaluation.readiness != ExpertReadiness::Blocked,
        "readiness": evaluation.readiness,
        "blockers": evaluation.blockers,
        "warnings": evaluation.warnings,
        "proposal": proposal,
    }))
}

pub(crate) async fn mcp_creation_context(
    state: &AppState,
    outcome: &str,
    project_path: &str,
) -> Result<ExpertCreationContext, AppError> {
    if outcome.trim().is_empty() || outcome.len() > MAX_TEXT {
        return Err(invalid("outcome is empty or oversized"));
    }
    let project = std::fs::canonicalize(project_path)
        .map_err(|error| invalid(format!("invalid project: {error}")))?;
    if Path::new(project_path) != project
        || !crate::install::project_is_registered(&state.app_data_dir, &project).await?
    {
        return Err(invalid(
            "project must be exactly canonical and registered in the desktop app",
        ));
    }
    let recognized = [
        "AGENTS.md",
        "CLAUDE.md",
        "README.md",
        "package.json",
        "Cargo.toml",
        "pyproject.toml",
        "requirements.txt",
        "go.mod",
        "pom.xml",
        "build.gradle",
    ];
    let mut manifests = Vec::new();
    let mut languages = HashSet::new();
    let mut instruction_excerpts = Vec::new();
    let mut warnings = Vec::new();
    let mut total = 0usize;
    for name in recognized {
        let path = project.join(name);
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            warnings.push(format!("Skipped linked or non-file root entry: {name}"));
            continue;
        }
        if matches!(name, "AGENTS.md" | "CLAUDE.md" | "README.md") {
            match read_capped(&path, 64 * 1024).await {
                Ok(raw) if total + raw.len() <= 256 * 1024 => {
                    total += raw.len();
                    instruction_excerpts.push(ExpertContextExcerpt {
                        file: name.into(),
                        excerpt: String::from_utf8_lossy(&raw).into_owned(),
                    });
                }
                Ok(_) => warnings.push(format!("Skipped {name}: context scan limit reached")),
                Err(_) => warnings.push(format!("Skipped oversized or unreadable {name}")),
            }
        } else {
            manifests.push(name.into());
            match name {
                "package.json" => {
                    languages.insert("typescript".to_string());
                }
                "Cargo.toml" => {
                    languages.insert("rust".to_string());
                }
                "pyproject.toml" | "requirements.txt" => {
                    languages.insert("python".to_string());
                }
                "go.mod" => {
                    languages.insert("go".to_string());
                }
                "pom.xml" | "build.gradle" => {
                    languages.insert("java".to_string());
                }
                _ => {}
            }
        }
    }
    let agents = catalog_agents(state)
        .await?
        .into_iter()
        .map(|agent| ExpertContextAgent {
            slug: agent.slug,
            name: agent.name,
            category: agent.category,
            description: agent.description,
        })
        .collect();
    let mut detected_clients = Vec::new();
    for client in ["claudeCode", "codex"] {
        if crate::install::tool_detected(state, client).await? {
            detected_clients.push(client.into());
        }
    }
    let terms = outcome
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|term| term.len() > 2)
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    let mut similar = mcp_definitions(state)
        .await?
        .into_iter()
        .filter_map(|expert| {
            let haystack = format!(
                "{} {} {} {}",
                expert.name,
                expert.summary,
                expert.category,
                expert.tags.join(" ")
            )
            .to_ascii_lowercase();
            let score = terms
                .iter()
                .filter(|term| haystack.contains(term.as_str()))
                .count();
            (score > 0).then_some((score, expert))
        })
        .collect::<Vec<_>>();
    similar.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.id.cmp(&right.1.id))
    });
    let similar_experts = similar
        .into_iter()
        .take(5)
        .map(|(_, expert)| ExpertContextExpert {
            id: expert.id,
            name: expert.name,
            summary: expert.summary,
            category: expert.category,
            tags: expert.tags,
            lead_agent: expert.lead_agent,
            supporting_agents: expert.supporting_agents,
            required_skills: expert.required_skills,
            optional_skills: expert.optional_skills,
            runbook: expert.runbook,
            preferred_client: expert.preferred_client,
        })
        .collect();
    let mut languages = languages.into_iter().collect::<Vec<_>>();
    languages.sort();
    Ok(ExpertCreationContext {
        project_path: project.to_string_lossy().into_owned(),
        project_name: project
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| project.to_string_lossy().into_owned()),
        languages,
        manifests,
        instruction_excerpts,
        agents,
        runbooks: catalog_runbooks(state).await?,
        detected_clients,
        similar_experts,
        warnings,
    })
}

async fn requests(state: &AppState) -> Result<Vec<ExpertActivationRequest>, AppError> {
    let path = state_path(state, "expert-activation-requests.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = read_capped(&path, MAX_FILE_BYTES).await?;
    serde_json::from_slice(&raw).map_err(|e| invalid(format!("parse activation requests: {e}")))
}

pub(crate) async fn mcp_request(
    state: &AppState,
    expert_id: String,
    project_path: String,
    client: Option<String>,
    requested_by: String,
) -> Result<ExpertActivationRequest, AppError> {
    let _ = mcp_plan(state, &expert_id, &project_path, client.clone()).await?;
    let canonical = tokio::fs::canonicalize(project_path)
        .await
        .map_err(|e| invalid(format!("invalid project: {e}")))?;
    let mut items = requests(state).await?;
    if items.iter().any(|item| {
        item.expert_id == expert_id
            && item.project_path == canonical.to_string_lossy()
            && item.state == "pending"
    }) {
        return Err(invalid(
            "an identical activation request is already pending",
        ));
    }
    let request = ExpertActivationRequest {
        id: uuid::Uuid::new_v4().to_string(),
        expert_id,
        project_path: canonical.to_string_lossy().into_owned(),
        client,
        requested_by: requested_by.chars().take(128).collect(),
        requested_at: chrono::Utc::now().to_rfc3339(),
        state: "pending".into(),
    };
    items.push(request.clone());
    let bytes = serde_json::to_vec_pretty(&items)
        .map_err(|e| invalid(format!("serialize activation requests: {e}")))?;
    atomic_write(
        &state_path(state, "expert-activation-requests.json"),
        &bytes,
    )
    .await?;
    Ok(request)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn mcp_request_creation(
    state: &AppState,
    client_request_id: String,
    outcome: String,
    project_path: String,
    mut proposal: ExpertProposalInput,
    linked_skill_drafts: Vec<LinkedSkillDraft>,
    agent_substitutions: Vec<AgentSubstitution>,
    requested_by: String,
) -> Result<ExpertCreationRequestView, AppError> {
    validate_request_metadata(&client_request_id, &outcome, &requested_by)?;
    let project = std::fs::canonicalize(&project_path)
        .map_err(|error| invalid(format!("invalid project: {error}")))?;
    if Path::new(&project_path) != project
        || !crate::install::project_is_registered(&state.app_data_dir, &project).await?
    {
        return Err(invalid(
            "project must be exactly canonical and registered in the desktop app",
        ));
    }
    let _lock = lock_expert_state(state)?;
    let mut file = custom_state(state).await?;
    if let Some(existing) = file.creation_requests.iter().find(|request| {
        request.client_request_id == client_request_id && request.requested_by == requested_by
    }) {
        return creation_view(state, existing.clone()).await;
    }
    validate_portable_proposal(&proposal, &project)?;
    validate_live_proposal(
        state,
        &mut proposal,
        &linked_skill_drafts,
        &agent_substitutions,
    )
    .await?;
    let request = ExpertCreationRequest {
        id: uuid::Uuid::new_v4().to_string(),
        client_request_id,
        outcome,
        project_path: project.to_string_lossy().into_owned(),
        requested_by,
        requested_at: chrono::Utc::now().to_rfc3339(),
        proposal,
        linked_skill_drafts,
        agent_substitutions,
        state: ExpertCreationState::Pending,
        saved_expert_id: None,
        kind: ExpertChangeKind::Create,
        target_expert_id: None,
        base_version: None,
    };
    let request = insert_creation_request(&mut file.creation_requests, request)?;
    save_expert_state(state, &file).await?;
    creation_view(state, request).await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn mcp_request_change(
    state: &AppState,
    client_request_id: String,
    outcome: String,
    project_path: String,
    proposal: ExpertProposalInput,
    linked_skill_drafts: Vec<LinkedSkillDraft>,
    agent_substitutions: Vec<AgentSubstitution>,
    requested_by: String,
    kind: ExpertChangeKind,
    target_expert_id: Option<String>,
    base_version: Option<u32>,
) -> Result<ExpertCreationRequestView, AppError> {
    if kind != ExpertChangeKind::Create {
        let target = target_expert_id
            .as_deref()
            .ok_or_else(|| invalid("Expert change requires targetExpertId"))?;
        let current = mcp_definitions(state)
            .await?
            .into_iter()
            .find(|expert| expert.id == target)
            .ok_or_else(|| invalid("Expert change target does not exist"))?;
        if matches!(kind, ExpertChangeKind::Update) && base_version != Some(current.version) {
            return Err(invalid("stale Expert version"));
        }
    }
    let view = mcp_request_creation(
        state,
        client_request_id,
        outcome,
        project_path,
        proposal,
        linked_skill_drafts,
        agent_substitutions,
        requested_by.clone(),
    )
    .await?;
    let _lock = lock_expert_state(state)?;
    let mut file = custom_state(state).await?;
    let request = file
        .creation_requests
        .iter_mut()
        .find(|request| request.id == view.request.id && request.requested_by == requested_by)
        .ok_or_else(|| invalid("Expert change request does not exist"))?;
    request.kind = kind;
    request.target_expert_id = target_expert_id;
    request.base_version = base_version;
    let request = request.clone();
    save_expert_state(state, &file).await?;
    creation_view(state, request).await
}

pub(crate) async fn mcp_revise_change_request(
    state: &AppState,
    id: &str,
    requested_by: &str,
    mut proposal: ExpertProposalInput,
    base_version: Option<u32>,
) -> Result<ExpertCreationRequestView, AppError> {
    let _lock = lock_expert_state(state)?;
    validate_live_proposal(state, &mut proposal, &[], &[]).await?;
    let mut file = custom_state(state).await?;
    let request = file
        .creation_requests
        .iter_mut()
        .find(|request| request.id == id)
        .ok_or_else(|| invalid("Expert change request does not exist"))?;
    revise_pending_change_request(request, requested_by, proposal, base_version)?;
    let request = request.clone();
    save_expert_state(state, &file).await?;
    creation_view(state, request).await
}

pub(crate) async fn mcp_cancel_change_request(
    state: &AppState,
    id: &str,
    requested_by: &str,
) -> Result<ExpertCreationRequestView, AppError> {
    let _lock = lock_expert_state(state)?;
    let mut file = custom_state(state).await?;
    let request = file
        .creation_requests
        .iter_mut()
        .find(|request| request.id == id)
        .ok_or_else(|| invalid("Expert change request does not exist"))?;
    cancel_pending_change_request(request, requested_by)?;
    let request = request.clone();
    save_expert_state(state, &file).await?;
    creation_view(state, request).await
}

pub(crate) async fn mcp_list_activation_requests(
    state: &AppState,
    requested_by: &str,
) -> Result<Vec<ExpertActivationRequest>, AppError> {
    Ok(requests(state)
        .await?
        .into_iter()
        .filter(|request| request.requested_by == requested_by)
        .collect())
}

pub(crate) async fn mcp_get_activation_request(
    state: &AppState,
    id: &str,
    requested_by: &str,
) -> Result<ExpertActivationRequest, AppError> {
    requests(state)
        .await?
        .into_iter()
        .find(|request| request.id == id && request.requested_by == requested_by)
        .ok_or_else(|| invalid("Expert activation request does not exist"))
}

pub(crate) async fn mcp_cancel_activation_request(
    state: &AppState,
    id: &str,
    requested_by: &str,
) -> Result<ExpertActivationRequest, AppError> {
    let _lock = lock_expert_state(state)?;
    let mut items = requests(state).await?;
    let request = items
        .iter_mut()
        .find(|request| request.id == id)
        .ok_or_else(|| invalid("Expert activation request does not exist"))?;
    cancel_pending_activation_request(request, requested_by)?;
    let request = request.clone();
    let bytes = serde_json::to_vec_pretty(&items)
        .map_err(|error| invalid(format!("serialize activation requests: {error}")))?;
    atomic_write(
        &state_path(state, "expert-activation-requests.json"),
        &bytes,
    )
    .await?;
    Ok(request)
}

pub(crate) async fn mcp_list_creation_requests(
    state: &AppState,
    requested_by: &str,
) -> Result<Vec<ExpertCreationRequestView>, AppError> {
    let requests = custom_state(state)
        .await?
        .creation_requests
        .into_iter()
        .filter(|request| request.requested_by == requested_by)
        .collect::<Vec<_>>();
    let mut views = Vec::with_capacity(requests.len());
    for request in requests {
        views.push(creation_view(state, request).await?);
    }
    Ok(views)
}

pub(crate) async fn mcp_get_creation_request(
    state: &AppState,
    id: &str,
    requested_by: &str,
) -> Result<ExpertCreationRequestView, AppError> {
    let request = custom_state(state)
        .await?
        .creation_requests
        .into_iter()
        .find(|request| request.id == id && request.requested_by == requested_by)
        .ok_or_else(|| invalid("Expert creation request does not exist"))?;
    creation_view(state, request).await
}

fn collision_safe_custom_id(name: &str, used: &HashSet<String>) -> String {
    let slug = normalize_logical_name(name);
    let slug = slug.get(..slug.len().min(72)).unwrap_or(&slug);
    let base = format!("custom-{}", if slug.is_empty() { "expert" } else { slug });
    if !used.contains(&base) {
        return base;
    }
    for suffix in 2.. {
        let candidate = format!("{base}-{suffix}");
        if !used.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

async fn approve_creation_request(
    state: &AppState,
    request_id: &str,
    mut proposal: ExpertProposalInput,
) -> Result<ExpertCreationRequestView, AppError> {
    let _lock = lock_expert_state(state)?;
    let mut file = custom_state(state).await?;
    let index = file
        .creation_requests
        .iter()
        .position(|request| {
            request.id == request_id && request.state == ExpertCreationState::Pending
        })
        .ok_or_else(|| invalid("pending Expert creation request does not exist"))?;
    let links = file.creation_requests[index].linked_skill_drafts.clone();
    let substitutions = file.creation_requests[index].agent_substitutions.clone();
    let edited_skills = proposal
        .required_skills
        .iter()
        .chain(proposal.optional_skills.iter())
        .map(|name| normalize_logical_name(name))
        .collect::<HashSet<_>>();
    let active_links = links
        .iter()
        .filter(|link| edited_skills.contains(&normalize_logical_name(&link.skill_name)))
        .cloned()
        .collect::<Vec<_>>();
    validate_live_proposal(state, &mut proposal, &active_links, &substitutions).await?;
    let evaluation = derive_readiness(
        &proposal,
        &links,
        &catalog_skills(state).await?,
        &draft_availability(state).await?,
    );
    if evaluation.readiness != ExpertReadiness::Ready {
        return Err(invalid(if evaluation.blockers.is_empty() {
            "required skill drafts are still pending".into()
        } else {
            evaluation.blockers.join("; ")
        }));
    }
    if matches!(
        file.creation_requests[index].kind,
        ExpertChangeKind::Create | ExpertChangeKind::Clone
    ) {
        let mut used = mcp_definitions(state)
            .await?
            .into_iter()
            .map(|expert| expert.id)
            .collect::<HashSet<_>>();
        used.extend(file.experts.iter().map(|expert| expert.id.clone()));
        let id = collision_safe_custom_id(&proposal.name, &used);
        let mut definition = proposal.clone().into_definition(id.clone());
        validate(&mut definition, true)?;
        file.experts.push(definition);
        file.creation_requests[index].proposal = proposal;
        file.creation_requests[index].state = ExpertCreationState::Approved;
        file.creation_requests[index].saved_expert_id = Some(id);
    } else {
        let mut request = file.creation_requests[index].clone();
        apply_change_request(&mut file, &mut request, proposal)?;
        file.creation_requests[index] = request;
    }
    let request = file.creation_requests[index].clone();
    save_expert_state(state, &file).await?;
    creation_view(state, request).await
}

async fn reject_creation_request(
    state: &AppState,
    request_id: &str,
) -> Result<ExpertCreationRequestView, AppError> {
    let _lock = lock_expert_state(state)?;
    let mut file = custom_state(state).await?;
    let request = file
        .creation_requests
        .iter_mut()
        .find(|request| request.id == request_id && request.state == ExpertCreationState::Pending)
        .ok_or_else(|| invalid("pending Expert creation request does not exist"))?;
    request.state = ExpertCreationState::Rejected;
    let request = request.clone();
    save_expert_state(state, &file).await?;
    creation_view(state, request).await
}

async fn resolve(
    app: &AppHandle,
    state: &AppState,
    definition: ExpertDefinition,
) -> Result<ExpertResolved, AppError> {
    let corpus = crate::corpus::ensure_corpus(app, state).await?;
    let agent_list = corpus.list(None);
    let known_agents = agent_list
        .iter()
        .map(|agent| agent.slug.as_str())
        .collect::<HashSet<_>>();
    let sources = crate::skills::inspect_skill_sources(state).await?;
    let known_skills = sources
        .iter()
        .flat_map(|source| source.packages.iter())
        .filter_map(|package| package.name.as_deref())
        .collect::<HashSet<_>>();
    let runbooks = crate::corpus::runbooks_list(app.clone()).await?;
    let unresolved_agents = std::iter::once(&definition.lead_agent)
        .chain(definition.supporting_agents.iter())
        .filter(|slug| !known_agents.contains(slug.as_str()))
        .cloned()
        .collect();
    let unresolved_skills = definition
        .required_skills
        .iter()
        .filter(|name| !known_skills.contains(name.as_str()))
        .cloned()
        .collect();
    let unresolved_runbook = definition
        .runbook
        .as_ref()
        .is_some_and(|slug| !runbooks.iter().any(|runbook| &runbook.slug == slug));
    Ok(ExpertResolved {
        definition,
        unresolved_agents,
        unresolved_skills,
        unresolved_runbook,
    })
}

#[tauri::command]
pub async fn experts_list(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<ExpertResolved>, AppError> {
    let mut out = Vec::new();
    for definition in definitions(&app, &state).await? {
        out.push(resolve(&app, &state, definition).await?);
    }
    Ok(out)
}

#[tauri::command]
pub async fn experts_get(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<ExpertResolved, AppError> {
    let definition = definitions(&app, &state)
        .await?
        .into_iter()
        .find(|expert| expert.id == id)
        .ok_or_else(|| invalid("unknown expert"))?;
    resolve(&app, &state, definition).await
}

#[tauri::command]
pub async fn expert_save(
    state: State<'_, AppState>,
    mut expert: ExpertDefinition,
) -> Result<(), AppError> {
    validate(&mut expert, true)?;
    let _lock = lock_expert_state(&state)?;
    let mut experts = custom_list(&state).await?;
    experts.retain(|item| item.id != expert.id);
    experts.push(expert);
    save_custom(&state, experts).await
}

#[tauri::command]
pub async fn expert_delete(state: State<'_, AppState>, id: String) -> Result<(), AppError> {
    let _lock = lock_expert_state(&state)?;
    let mut experts = custom_list(&state).await?;
    let before = experts.len();
    experts.retain(|item| item.id != id);
    if before == experts.len() {
        return Err(invalid("custom expert does not exist"));
    }
    save_custom(&state, experts).await
}

#[tauri::command]
pub async fn expert_import(state: State<'_, AppState>, path: String) -> Result<u32, AppError> {
    let raw = read_capped(Path::new(&path), MAX_FILE_BYTES).await?;
    let text = String::from_utf8(raw).map_err(|_| invalid("expert import must be UTF-8"))?;
    let imported = parse_file(&text, true)?;
    let count = imported.len() as u32;
    let _lock = lock_expert_state(&state)?;
    let mut experts = custom_list(&state).await?;
    for expert in imported {
        experts.retain(|item| item.id != expert.id);
        experts.push(expert);
    }
    save_custom(&state, experts).await?;
    Ok(count)
}

#[tauri::command]
pub async fn expert_export(state: State<'_, AppState>, path: String) -> Result<u32, AppError> {
    let experts = custom_list(&state).await?;
    let count = experts.len() as u32;
    let bytes = serde_json::to_vec_pretty(&ExpertFile {
        schema_version: SCHEMA_VERSION,
        experts,
        creation_requests: Vec::new(),
        archived_experts: Vec::new(),
    })
    .map_err(|e| invalid(format!("serialize experts: {e}")))?;
    atomic_write(Path::new(&path), &bytes).await?;
    Ok(count)
}

async fn plan(
    app: &AppHandle,
    state: &AppState,
    id: &str,
    project_path: &str,
    client: Option<String>,
) -> Result<ExpertActivationPlan, AppError> {
    let project = tokio::fs::canonicalize(project_path)
        .await
        .map_err(|e| invalid(format!("invalid project: {e}")))?;
    if !project.is_dir() {
        return Err(invalid("project path must be a directory"));
    }
    let project_path = project.to_string_lossy().into_owned();
    let definition = definitions(app, state)
        .await?
        .into_iter()
        .find(|expert| expert.id == id)
        .ok_or_else(|| invalid("unknown expert"))?;
    let expert = resolve(app, state, definition).await?;
    let client = client
        .or_else(|| expert.definition.preferred_client.clone())
        .ok_or_else(|| invalid("activation requires a client"))?;
    if !matches!(client.as_str(), "claudeCode" | "codex") {
        return Err(invalid("unsupported client"));
    }
    let client_detected = crate::install::tool_detected(state, &client).await?;
    let installed = crate::install::load_ledger(app, state).await?;
    let corpus = crate::corpus::ensure_corpus(app, state).await?;
    let tool_home = crate::install::tool_home(state, &client).await?;
    let project_root = Path::new(&project_path);
    let mut agent_blockers = Vec::new();
    let mut existing = Vec::new();
    let mut agents = Vec::new();
    for slug in std::iter::once(&expert.definition.lead_agent)
        .chain(expert.definition.supporting_agents.iter())
    {
        let row = installed.iter().find(|row| {
            row.slug == *slug
                && row.tool == client
                && row.project_path.as_deref() == Some(project_path.as_str())
        });
        let destination = match row {
            Some(row) => Some(row.dest.clone()),
            None => crate::render::dests(&client, slug, &tool_home, Some(project_root))
                .ok()
                .and_then(|paths| paths.into_iter().next())
                .map(|path| path.to_string_lossy().into_owned()),
        };
        let status = match row {
            Some(row) if !Path::new(&row.dest).exists() => "missing",
            Some(row) => {
                let disk_hash = read_capped(Path::new(&row.dest), 1024 * 1024)
                    .await
                    .ok()
                    .map(|bytes| crate::render::sha256_hex(&bytes));
                if disk_hash.as_deref() != Some(row.rendered_hash.as_str()) {
                    agent_blockers.push(format!("modified agent: {slug}"));
                    "modified"
                } else if corpus
                    .entry(slug)
                    .is_some_and(|entry| entry.source_hash != row.source_hash)
                {
                    agent_blockers.push(format!("outdated agent: {slug}"));
                    "outdated"
                } else {
                    "current"
                }
            }
            None if destination
                .as_ref()
                .is_some_and(|path| Path::new(path).exists()) =>
            {
                agent_blockers.push(format!("foreign agent destination: {slug}"));
                "foreign"
            }
            None => "missing",
        };
        if status == "current" {
            existing.push(format!("agent:{slug}"));
        }
        agents.push(ExpertAgentAction {
            slug: slug.clone(),
            status: status.into(),
            destination,
        });
    }
    let sources = crate::skills::inspect_skill_sources(state).await?;
    let mut skills = Vec::new();
    let mut blockers = expert
        .unresolved_agents
        .iter()
        .map(|item| format!("unknown agent: {item}"))
        .collect::<Vec<_>>();
    blockers.extend(agent_blockers);
    blockers.extend(
        expert
            .unresolved_skills
            .iter()
            .map(|item| format!("unknown skill: {item}")),
    );
    if expert.unresolved_runbook {
        blockers.push("unknown runbook".into());
    }
    if !client_detected {
        blockers.push(format!(
            "{} is not detected",
            if client == "claudeCode" {
                "Claude Code"
            } else {
                "Codex"
            }
        ));
    }
    for name in expert
        .definition
        .required_skills
        .iter()
        .chain(expert.definition.optional_skills.iter())
    {
        let matches = sources
            .iter()
            .flat_map(|source| source.packages.iter())
            .filter(|package| package.installable && package.name.as_deref() == Some(name))
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [package] => {
                let item = crate::skills::plan_skill_install(
                    state,
                    &package.source_id,
                    &package.relative_path,
                    &client,
                    Some(&project_path),
                )
                .await?;
                if expert.definition.required_skills.contains(name) {
                    blockers.extend(item.blockers.iter().cloned());
                }
                skills.push(item);
            }
            [] if expert.definition.required_skills.contains(name) => {
                blockers.push(format!("missing skill: {name}"))
            }
            [] => {}
            _ if expert.definition.required_skills.contains(name) => {
                blockers.push(format!("ambiguous skill: {name}"))
            }
            _ => {}
        }
    }
    blockers.sort();
    blockers.dedup();
    let prompt_preview = expert
        .definition
        .starter_prompt
        .replace("{{expert}}", &expert.definition.name)
        .replace("{{project}}", &project_path)
        .replace("{{leadAgent}}", &expert.definition.lead_agent);
    let rollback_scope = agents
        .iter()
        .filter(|item| item.status == "missing")
        .map(|item| format!("agent:{}", item.slug))
        .chain(skills.iter().flat_map(|plan| {
            plan.packages
                .iter()
                .map(|item| format!("skill:{}", item.name))
        }))
        .collect();
    Ok(ExpertActivationPlan {
        expert,
        project_path,
        client,
        agents,
        skills,
        existing,
        warnings: Vec::new(),
        blockers,
        prompt_preview,
        rollback_scope,
    })
}

#[tauri::command]
pub async fn expert_plan_activation(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    project_path: String,
    client: Option<String>,
) -> Result<ExpertActivationPlan, AppError> {
    plan(&app, &state, &id, &project_path, client).await
}

#[tauri::command]
pub async fn expert_activate(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    project_path: String,
    client: Option<String>,
) -> Result<ExpertActivationRecord, AppError> {
    let plan = plan(&app, &state, &id, &project_path, client).await?;
    if !plan.blockers.is_empty() {
        return Err(invalid(plan.blockers.join("; ")));
    }
    let mut installed_agents = Vec::new();
    for agent in &plan.agents {
        if agent.status == "missing" {
            match crate::install::do_install_legacy(
                &app,
                &state,
                agent.slug.clone(),
                plan.client.clone(),
                Some(plan.project_path.clone()),
            )
            .await
            {
                Ok(_) => installed_agents.push(agent.slug.clone()),
                Err(error) => {
                    for slug in installed_agents.iter().rev() {
                        let _ = crate::install::do_uninstall_legacy(
                            &app,
                            &state,
                            slug.clone(),
                            plan.client.clone(),
                            Some(plan.project_path.clone()),
                        )
                        .await;
                    }
                    return Err(error);
                }
            }
        }
    }
    let mut installed_skills = Vec::new();
    let mut installed_skill_refs = Vec::new();
    for skill in &plan.skills {
        for package in &skill.packages {
            if package.dependency || installed_skills.contains(&package.name) {
                continue;
            }
            match crate::skills::install_skill_with_dependencies(
                state.inner(),
                &package.source_id,
                &package.relative_path,
                &plan.client,
                Some(&plan.project_path),
            )
            .await
            {
                Ok(created) => {
                    for item in created {
                        if !installed_skills.contains(&item.name) {
                            installed_skills.push(item.name.clone());
                            installed_skill_refs.push((item.source_id, item.relative_path));
                        }
                    }
                }
                Err(error) => {
                    for (source_id, relative_path) in installed_skill_refs.iter().rev() {
                        let _ = crate::skills::uninstall_skill(
                            state.inner(),
                            source_id,
                            relative_path,
                            &plan.client,
                            Some(&plan.project_path),
                        )
                        .await;
                    }
                    for slug in installed_agents.iter().rev() {
                        let _ = crate::install::do_uninstall_legacy(
                            &app,
                            &state,
                            slug.clone(),
                            plan.client.clone(),
                            Some(plan.project_path.clone()),
                        )
                        .await;
                    }
                    return Err(error);
                }
            }
        }
    }
    let run = match crate::expert_runs::create_run(
        &state,
        crate::expert_runs::ExpertRunCreate {
            expert_id: plan.expert.definition.id.clone(),
            expert_version: plan.expert.definition.version,
            project_path: plan.project_path.clone(),
            client: plan.client.clone(),
            lead_agent: plan.expert.definition.lead_agent.clone(),
            supporting_agents: plan.expert.definition.supporting_agents.clone(),
            required_skills: plan.expert.definition.required_skills.clone(),
            optional_skills: plan.expert.definition.optional_skills.clone(),
            runbook: plan.expert.definition.runbook.clone(),
            contract: plan.expert.definition.quality_contract.clone(),
        },
    )
    .await
    {
        Ok(run) => run,
        Err(error) => {
            for (source_id, relative_path) in installed_skill_refs.iter().rev() {
                let _ = crate::skills::uninstall_skill(
                    state.inner(),
                    source_id,
                    relative_path,
                    &plan.client,
                    Some(&plan.project_path),
                )
                .await;
            }
            for slug in installed_agents.iter().rev() {
                let _ = crate::install::do_uninstall_legacy(
                    &app,
                    &state,
                    slug.clone(),
                    plan.client.clone(),
                    Some(plan.project_path.clone()),
                )
                .await;
            }
            return Err(error);
        }
    };
    let record = ExpertActivationRecord {
        id: uuid::Uuid::new_v4().to_string(),
        expert_id: plan.expert.definition.id,
        expert_version: plan.expert.definition.version,
        project_path: plan.project_path,
        client: plan.client,
        activated_at: chrono::Utc::now().to_rfc3339(),
        installed_agents,
        installed_skills,
        run_id: Some(run.id),
    };
    let mut history = activation_history(&state).await?;
    history.push(record.clone());
    let bytes = serde_json::to_vec_pretty(&history)
        .map_err(|e| invalid(format!("serialize activation history: {e}")))?;
    atomic_write(&state_path(&state, "expert-activations.json"), &bytes).await?;
    Ok(record)
}

async fn activation_history(state: &AppState) -> Result<Vec<ExpertActivationRecord>, AppError> {
    let path = state_path(state, "expert-activations.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = read_capped(&path, MAX_FILE_BYTES).await?;
    serde_json::from_slice(&raw).map_err(|e| invalid(format!("parse activation history: {e}")))
}

#[tauri::command]
pub async fn expert_activation_history(
    state: State<'_, AppState>,
    project_path: Option<String>,
) -> Result<Vec<ExpertActivationRecord>, AppError> {
    let mut history = activation_history(&state).await?;
    if let Some(project_path) = project_path {
        history.retain(|record| record.project_path == project_path);
    }
    Ok(history)
}

#[tauri::command]
pub async fn expert_activation_requests(
    state: State<'_, AppState>,
) -> Result<Vec<ExpertActivationRequest>, AppError> {
    requests(&state).await
}

#[tauri::command]
pub async fn expert_activation_request_resolve(
    state: State<'_, AppState>,
    request_id: String,
    approved: bool,
) -> Result<(), AppError> {
    let mut items = requests(&state).await?;
    let item = items
        .iter_mut()
        .find(|item| item.id == request_id && item.state == "pending")
        .ok_or_else(|| invalid("pending activation request does not exist"))?;
    item.state = if approved { "approved" } else { "rejected" }.into();
    let bytes = serde_json::to_vec_pretty(&items)
        .map_err(|e| invalid(format!("serialize activation requests: {e}")))?;
    atomic_write(
        &state_path(&state, "expert-activation-requests.json"),
        &bytes,
    )
    .await
}

#[tauri::command]
pub async fn expert_creation_requests(
    state: State<'_, AppState>,
) -> Result<Vec<ExpertCreationRequestView>, AppError> {
    let requests = custom_state(&state).await?.creation_requests;
    let mut views = Vec::with_capacity(requests.len());
    for request in requests {
        views.push(creation_view(&state, request).await?);
    }
    Ok(views)
}

#[tauri::command]
pub async fn expert_creation_request_get(
    state: State<'_, AppState>,
    id: String,
) -> Result<ExpertCreationRequestView, AppError> {
    let request = custom_state(&state)
        .await?
        .creation_requests
        .into_iter()
        .find(|request| request.id == id)
        .ok_or_else(|| invalid("Expert creation request does not exist"))?;
    creation_view(&state, request).await
}

#[tauri::command]
pub async fn expert_creation_request_approve(
    state: State<'_, AppState>,
    request_id: String,
    proposal: ExpertProposalInput,
) -> Result<ExpertCreationRequestView, AppError> {
    approve_creation_request(&state, &request_id, proposal).await
}

#[tauri::command]
pub async fn expert_creation_request_reject(
    state: State<'_, AppState>,
    request_id: String,
) -> Result<ExpertCreationRequestView, AppError> {
    reject_creation_request(&state, &request_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::{Mutex, RwLock};

    fn proposal() -> ExpertProposalInput {
        ExpertProposalInput {
            name: "Rust Review Expert".into(),
            summary: "Review Rust changes safely.".into(),
            category: "Engineering".into(),
            tags: vec!["rust".into()],
            lead_agent: "rust-reviewer".into(),
            supporting_agents: vec!["security-reviewer".into()],
            required_skills: vec!["Rust Review".into()],
            optional_skills: vec!["Release Notes".into()],
            runbook: Some("review-runbook".into()),
            preferred_client: Some("codex".into()),
            starter_prompt: "Review {{project}} with {{leadAgent}}.".into(),
            quality_contract: crate::expert_runs::QualityContract {
                version: 1,
                checks: vec![crate::expert_runs::ExpertCheck {
                    name: "tests".into(),
                    kind: "tests".into(),
                    required: true,
                    evidence_mode: "clientReported".into(),
                }],
            },
        }
    }

    fn test_state(root: &Path) -> AppState {
        AppState {
            app_data_dir: root.to_path_buf(),
            corpus_cache: Arc::new(Mutex::new(None)),
            corpus_refresh_in_flight: Arc::new(Mutex::new(())),
            skill_sources_write_lock: Arc::new(Mutex::new(())),
            skill_installs_write_lock: Arc::new(Mutex::new(())),
            skill_folders_write_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(RwLock::new(
                crate::commands::settings::SettingsLoadState::FirstLaunch,
            )),
            updater_state: crate::commands::updater::empty_state(),
        }
    }

    #[test]
    fn bundled_experts_are_valid_and_unique() {
        let experts = parse_file(BUNDLED, false).unwrap();
        assert_eq!(experts.len(), 3);
        assert_eq!(
            experts
                .iter()
                .map(|item| &item.id)
                .collect::<HashSet<_>>()
                .len(),
            3
        );
    }

    #[test]
    fn malformed_templates_and_duplicate_ids_are_rejected() {
        let mut expert = parse_file(BUNDLED, false).unwrap().remove(0);
        expert.starter_prompt = "{{project}".into();
        assert!(validate(&mut expert, true).is_err());

        let mut expert = proposal().into_definition("quality-contract".into());
        expert
            .quality_contract
            .checks
            .push(expert.quality_contract.checks[0].clone());
        assert!(validate(&mut expert, true).is_err());
        let raw = format!(
            r#"{{"schemaVersion":1,"experts":[{},{}]}}"#,
            serde_json::to_string(&expert).unwrap(),
            serde_json::to_string(&expert).unwrap()
        );
        assert!(parse_file(&raw, true).is_err());
    }

    #[test]
    fn proposal_references_require_known_agents_runbooks_and_matching_draft_names() {
        let known_agents = HashSet::from(["rust-reviewer", "security-reviewer"]);
        let known_runbooks = HashSet::from(["review-runbook"]);
        let mut input = proposal();
        let links = vec![LinkedSkillDraft {
            skill_name: "rust-review".into(),
            draft_id: uuid::Uuid::new_v4().to_string(),
        }];
        let substitutions = vec![AgentSubstitution {
            needed_capability: "Rust ownership review".into(),
            selected_catalog_slug: "rust-reviewer".into(),
            rationale: "Closest catalog specialist".into(),
        }];
        assert!(validate_proposal_references(
            &mut input,
            &links,
            &substitutions,
            &known_agents,
            &known_runbooks,
        )
        .is_ok());

        input.lead_agent = "missing-agent".into();
        assert!(validate_proposal_references(
            &mut input,
            &links,
            &substitutions,
            &known_agents,
            &known_runbooks,
        )
        .is_err());

        let mut input = proposal();
        let mismatched = vec![LinkedSkillDraft {
            skill_name: "different-skill".into(),
            draft_id: links[0].draft_id.clone(),
        }];
        assert!(validate_proposal_references(
            &mut input,
            &mismatched,
            &substitutions,
            &known_agents,
            &known_runbooks,
        )
        .is_err());
    }

    #[test]
    fn required_skill_readiness_transitions_without_blocking_optional_gaps() {
        let input = proposal();
        let draft_id = uuid::Uuid::new_v4().to_string();
        let links = vec![LinkedSkillDraft {
            skill_name: "rust-review".into(),
            draft_id: draft_id.clone(),
        }];
        let pending = vec![DraftAvailability {
            id: draft_id.clone(),
            name: Some("Rust Review".into()),
            state: crate::types::SkillDraftState::Pending,
            installable: true,
        }];

        let waiting = derive_readiness(&input, &links, &[], &pending);
        assert_eq!(waiting.readiness, ExpertReadiness::WaitingForSkills);
        assert!(waiting.blockers.is_empty());
        assert!(waiting
            .warnings
            .iter()
            .any(|warning| warning.contains("Release Notes")));

        let published = vec![DraftAvailability {
            state: crate::types::SkillDraftState::Published,
            ..pending[0].clone()
        }];
        assert_eq!(
            derive_readiness(&input, &links, &[], &published).readiness,
            ExpertReadiness::Ready
        );

        let rejected = vec![DraftAvailability {
            state: crate::types::SkillDraftState::Rejected,
            ..pending[0].clone()
        }];
        assert_eq!(
            derive_readiness(&input, &links, &[], &rejected).readiness,
            ExpertReadiness::Blocked
        );

        let catalog = vec![CatalogSkill {
            normalized_name: "rust-review".into(),
            preferred: true,
        }];
        assert_eq!(
            derive_readiness(&input, &links, &catalog, &rejected).readiness,
            ExpertReadiness::Ready
        );
    }

    fn creation_request(
        client_request_id: &str,
        state: ExpertCreationState,
    ) -> ExpertCreationRequest {
        ExpertCreationRequest {
            id: uuid::Uuid::new_v4().to_string(),
            client_request_id: client_request_id.into(),
            outcome: "Review this project".into(),
            project_path: "/project".into(),
            requested_by: "codex".into(),
            requested_at: "2026-07-30T00:00:00Z".into(),
            proposal: proposal(),
            linked_skill_drafts: Vec::new(),
            agent_substitutions: Vec::new(),
            state,
            saved_expert_id: None,
            kind: ExpertChangeKind::Create,
            target_expert_id: None,
            base_version: None,
        }
    }

    #[test]
    fn creation_request_inbox_is_idempotent_and_evicts_only_terminal_records() {
        let mut requests = vec![creation_request("old", ExpertCreationState::Rejected)];
        for index in 1..MAX_CREATION_REQUESTS {
            requests.push(creation_request(
                &format!("pending-{index}"),
                ExpertCreationState::Pending,
            ));
        }
        let incoming = creation_request("new", ExpertCreationState::Pending);
        let inserted = insert_creation_request(&mut requests, incoming.clone()).unwrap();
        assert_eq!(inserted.id, incoming.id);
        assert_eq!(requests.len(), MAX_CREATION_REQUESTS);
        assert!(!requests
            .iter()
            .any(|request| request.client_request_id == "old"));

        let retry = creation_request("new", ExpertCreationState::Pending);
        assert_eq!(
            insert_creation_request(&mut requests, retry).unwrap().id,
            incoming.id
        );
        assert_eq!(requests.len(), MAX_CREATION_REQUESTS);

        for request in &mut requests {
            request.state = ExpertCreationState::Pending;
        }
        assert!(insert_creation_request(
            &mut requests,
            creation_request("overflow", ExpertCreationState::Pending),
        )
        .is_err());
    }

    #[tokio::test]
    async fn creation_request_round_trip_is_client_scoped_and_saves_a_portable_expert() {
        let app = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let project = std::fs::canonicalize(project.path()).unwrap();
        let state_dir = crate::corpus::state_dir(app.path());
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::write(
            state_dir.join("projects.json"),
            serde_json::to_vec(&vec![project.to_string_lossy().into_owned()]).unwrap(),
        )
        .unwrap();
        let agent = crate::types::CorpusEntry {
            slug: "rust-reviewer".into(),
            name: "Rust Reviewer".into(),
            category: "engineering".into(),
            emoji: None,
            color: None,
            vibe: None,
            description: "Reviews Rust".into(),
            source_hash: "a".repeat(64),
            frontmatter_hash: "b".repeat(64),
            body_hash: "c".repeat(64),
        };
        std::fs::write(
            state_dir.join("corpus-index.json"),
            serde_json::to_vec(&std::collections::BTreeMap::from([(
                agent.slug.clone(),
                agent,
            )]))
            .unwrap(),
        )
        .unwrap();
        let state = test_state(app.path());
        let mut input = proposal();
        input.supporting_agents.clear();
        input.required_skills.clear();
        input.optional_skills.clear();
        input.runbook = None;

        let first = mcp_request_creation(
            &state,
            "request-1".into(),
            "Review this project".into(),
            project.to_string_lossy().into_owned(),
            input.clone(),
            Vec::new(),
            Vec::new(),
            "codex".into(),
        )
        .await
        .unwrap();
        let retry = mcp_request_creation(
            &state,
            "request-1".into(),
            "Different retry body".into(),
            project.to_string_lossy().into_owned(),
            input.clone(),
            Vec::new(),
            Vec::new(),
            "codex".into(),
        )
        .await
        .unwrap();
        assert_eq!(retry.request.id, first.request.id);
        assert!(
            mcp_get_creation_request(&state, &first.request.id, "claude")
                .await
                .is_err()
        );

        let approved = approve_creation_request(&state, &first.request.id, input)
            .await
            .unwrap();
        assert_eq!(approved.request.state, ExpertCreationState::Approved);
        let saved_id = approved.request.saved_expert_id.unwrap();
        let saved = custom_list(&state)
            .await
            .unwrap()
            .into_iter()
            .find(|expert| expert.id == saved_id)
            .unwrap();
        assert_eq!(saved.source, "custom");
        assert_eq!(saved.version, 1);
        assert!(!serde_json::to_string(&saved)
            .unwrap()
            .contains(project.to_string_lossy().as_ref()));
    }

    #[tokio::test]
    async fn creation_context_reads_only_bounded_recognized_root_files() {
        let app = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let project = std::fs::canonicalize(project.path()).unwrap();
        let state_dir = crate::corpus::state_dir(app.path());
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::write(
            state_dir.join("projects.json"),
            serde_json::to_vec(&vec![project.to_string_lossy().into_owned()]).unwrap(),
        )
        .unwrap();
        let agent = crate::types::CorpusEntry {
            slug: "rust-reviewer".into(),
            name: "Rust Reviewer".into(),
            category: "engineering".into(),
            emoji: None,
            color: None,
            vibe: None,
            description: "Reviews Rust".into(),
            source_hash: "a".repeat(64),
            frontmatter_hash: "b".repeat(64),
            body_hash: "c".repeat(64),
        };
        std::fs::write(
            state_dir.join("corpus-index.json"),
            serde_json::to_vec(&std::collections::BTreeMap::from([(
                agent.slug.clone(),
                agent,
            )]))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(project.join("README.md"), "root-only").unwrap();
        std::fs::write(project.join("AGENTS.md"), "x".repeat(64 * 1024 + 1)).unwrap();
        std::fs::create_dir(project.join("nested")).unwrap();
        std::fs::write(project.join("nested/CLAUDE.md"), "nested-secret").unwrap();
        let context = mcp_creation_context(
            &test_state(app.path()),
            "Review Rust",
            project.to_str().unwrap(),
        )
        .await
        .unwrap();
        let json = serde_json::to_string(&context).unwrap();

        assert!(json.contains("root-only"));
        assert!(!json.contains("nested-secret"));
        assert!(json.contains("Skipped oversized or unreadable AGENTS.md"));
    }

    #[test]
    fn portable_proposals_reject_project_paths_and_credentials() {
        let project = Path::new("/Users/client/secret-project");
        let mut input = proposal();
        assert!(validate_portable_proposal(&input, project).is_ok());

        input.summary = format!("Use files from {}", project.display());
        assert!(validate_portable_proposal(&input, project).is_err());

        input.summary = "Review safely".into();
        input.starter_prompt = "token=super-secret".into();
        assert!(validate_portable_proposal(&input, project).is_err());
    }

    #[test]
    fn change_requests_apply_versioned_updates_and_reversible_archive() {
        let mut existing = proposal().into_definition("custom-rust-review".into());
        existing.version = 2;
        let mut file = ExpertFile {
            schema_version: SCHEMA_VERSION,
            experts: vec![existing],
            creation_requests: Vec::new(),
            archived_experts: Vec::new(),
        };
        let mut request = creation_request("update", ExpertCreationState::Pending);
        request.kind = ExpertChangeKind::Update;
        request.target_expert_id = Some("custom-rust-review".into());
        request.base_version = Some(1);
        assert!(apply_change_request(&mut file, &mut request, proposal()).is_err());

        request.base_version = Some(2);
        let saved = apply_change_request(&mut file, &mut request, proposal()).unwrap();
        assert_eq!(saved.as_deref(), Some("custom-rust-review"));
        assert_eq!(file.experts[0].version, 3);

        request.kind = ExpertChangeKind::Archive;
        request.state = ExpertCreationState::Pending;
        assert!(apply_change_request(&mut file, &mut request, proposal()).is_ok());
        assert_eq!(file.archived_experts, vec!["custom-rust-review"]);

        request.kind = ExpertChangeKind::Update;
        request.state = ExpertCreationState::Pending;
        request.base_version = Some(3);
        assert!(apply_change_request(&mut file, &mut request, proposal()).is_ok());
        assert!(file.archived_experts.is_empty());
    }

    #[test]
    fn pending_requests_are_owned_revisable_and_cancellable() {
        let mut change = creation_request("change", ExpertCreationState::Pending);
        let mut leaked = proposal();
        leaked.summary = format!("Read {}", change.project_path);
        assert!(revise_pending_change_request(&mut change, "codex", leaked, Some(1)).is_err());
        let mut revised = proposal();
        revised.summary = "Revised outcome".into();
        revise_pending_change_request(&mut change, "codex", revised.clone(), Some(1)).unwrap();
        assert_eq!(change.proposal.summary, revised.summary);
        assert!(revise_pending_change_request(&mut change, "claude", revised, Some(1)).is_err());
        cancel_pending_change_request(&mut change, "codex").unwrap();
        assert_eq!(change.state, ExpertCreationState::Cancelled);

        let mut activation = ExpertActivationRequest {
            id: "activation-1".into(),
            expert_id: "expert-1".into(),
            project_path: "/project".into(),
            client: Some("codex".into()),
            requested_by: "codex".into(),
            requested_at: "2026-08-03T00:00:00Z".into(),
            state: "pending".into(),
        };
        cancel_pending_activation_request(&mut activation, "codex").unwrap();
        assert_eq!(activation.state, "cancelled");
        assert!(cancel_pending_activation_request(&mut activation, "claude").is_err());
    }

    #[tokio::test]
    async fn runs_scope_idempotent_evidence_and_freeze_after_review() {
        use crate::expert_runs::{
            EvidenceResult, EvidenceSubmission, ExpertCheck, ExpertRunCreate, ExpertRunState,
            QualityContract,
        };

        let app = tempfile::tempdir().unwrap();
        let state = test_state(app.path());
        let run = crate::expert_runs::create_run(
            &state,
            ExpertRunCreate {
                expert_id: "expert-1".into(),
                expert_version: 1,
                project_path: "/project".into(),
                client: "codex".into(),
                lead_agent: "lead".into(),
                supporting_agents: Vec::new(),
                required_skills: Vec::new(),
                optional_skills: Vec::new(),
                runbook: None,
                contract: QualityContract {
                    version: 1,
                    checks: vec![ExpertCheck {
                        name: "tests".into(),
                        kind: "tests".into(),
                        required: true,
                        evidence_mode: "clientReported".into(),
                    }],
                },
            },
        )
        .await
        .unwrap();
        let evidence = EvidenceSubmission {
            idempotency_key: "evidence-1".into(),
            check_name: "tests".into(),
            result: EvidenceResult::Pass,
            command_label: Some("cargo test".into()),
            summary: "All tests passed".into(),
        };
        let first = crate::expert_runs::submit_evidence(
            &state,
            &run.id,
            "codex",
            "/project",
            evidence.clone(),
        )
        .await
        .unwrap();
        let retry = crate::expert_runs::submit_evidence(
            &state,
            &run.id,
            "codex",
            "/project",
            evidence.clone(),
        )
        .await
        .unwrap();
        assert_eq!(first.id, retry.id);
        assert!(crate::expert_runs::submit_evidence(
            &state,
            &run.id,
            "claudeCode",
            "/project",
            evidence.clone(),
        )
        .await
        .is_err());
        let mut changed = evidence;
        changed.summary = "Changed replay".into();
        assert!(
            crate::expert_runs::submit_evidence(&state, &run.id, "codex", "/project", changed,)
                .await
                .is_err()
        );
        crate::expert_runs::submit_evidence(
            &state,
            &run.id,
            "codex",
            "/project",
            EvidenceSubmission {
                idempotency_key: "evidence-fail".into(),
                check_name: "tests".into(),
                result: EvidenceResult::Fail,
                command_label: None,
                summary: "Latest run failed".into(),
            },
        )
        .await
        .unwrap();
        crate::expert_runs::report_blocker(
            &state,
            &run.id,
            "codex",
            "/project",
            "dependency",
            "Waiting for access",
        )
        .await
        .unwrap();
        crate::expert_runs::request_review(&state, &run.id, "codex", "/project")
            .await
            .unwrap();
        assert!(crate::expert_runs::review_run_with_waivers(
            &state,
            &run.id,
            ExpertRunState::Accepted,
            Vec::new(),
        )
        .await
        .is_err());
        crate::expert_runs::review_run_with_waivers(
            &state,
            &run.id,
            ExpertRunState::Accepted,
            vec![crate::expert_runs::ExpertWaiverInput {
                check_name: "tests".into(),
                reason: "Approved exception".into(),
            }],
        )
        .await
        .unwrap();
        assert!(crate::expert_runs::submit_evidence(
            &state,
            &run.id,
            "codex",
            "/project",
            EvidenceSubmission {
                idempotency_key: "late".into(),
                check_name: "tests".into(),
                result: EvidenceResult::Pass,
                command_label: None,
                summary: "late".into(),
            },
        )
        .await
        .is_err());

        let waiver_run = crate::expert_runs::create_run(
            &state,
            ExpertRunCreate {
                expert_id: "expert-2".into(),
                expert_version: 1,
                project_path: "/project".into(),
                client: "codex".into(),
                lead_agent: "lead".into(),
                supporting_agents: Vec::new(),
                required_skills: Vec::new(),
                optional_skills: Vec::new(),
                runbook: None,
                contract: QualityContract {
                    version: 1,
                    checks: vec![ExpertCheck {
                        name: "security".into(),
                        kind: "security".into(),
                        required: true,
                        evidence_mode: "userConfirmed".into(),
                    }],
                },
            },
        )
        .await
        .unwrap();
        crate::expert_runs::request_review(&state, &waiver_run.id, "codex", "/project")
            .await
            .unwrap();
        assert!(crate::expert_runs::review_run_with_waivers(
            &state,
            &waiver_run.id,
            ExpertRunState::Accepted,
            Vec::new(),
        )
        .await
        .is_err());
        assert!(crate::expert_runs::review_run_with_waivers(
            &state,
            &waiver_run.id,
            ExpertRunState::Accepted,
            vec![
                crate::expert_runs::ExpertWaiverInput {
                    check_name: "security".into(),
                    reason: "Approved emergency exception".into(),
                },
                crate::expert_runs::ExpertWaiverInput {
                    check_name: "not-required".into(),
                    reason: "Irrelevant".into(),
                },
            ],
        )
        .await
        .is_err());
        let accepted = crate::expert_runs::review_run_with_waivers(
            &state,
            &waiver_run.id,
            ExpertRunState::Accepted,
            vec![crate::expert_runs::ExpertWaiverInput {
                check_name: "security".into(),
                reason: "Approved emergency exception".into(),
            }],
        )
        .await
        .unwrap();
        assert_eq!(accepted.state, ExpertRunState::Accepted);
        assert_eq!(accepted.waivers.len(), 1);
        let mcp_json = crate::expert_runs::mcp_view(&accepted).to_string();
        assert!(!mcp_json.contains("Approved emergency exception"));
        assert!(mcp_json.contains("security"));
    }
}
