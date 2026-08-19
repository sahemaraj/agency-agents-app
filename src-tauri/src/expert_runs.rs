use std::collections::HashSet;
use std::fs::OpenOptions;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::error::AppError;
use crate::state::AppState;
use crate::util::fs::{atomic_write, read_capped};

const MAX_RUN_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RUNS: usize = 500;
const MAX_TEXT: usize = 4096;
const MAX_FACTORY_TITLE: usize = 240;
const MAX_FACTORY_REFERENCE: usize = 512;
const MAX_FACTORY_ITEMS: usize = 32;
const MAX_FACTORY_ITEM_TEXT: usize = 1024;
const MAX_FACTORY_READINESS_AGE_SECONDS: i64 = 300;
const MAX_FACTORY_CLAIMS: usize = 32;
const MAX_FACTORY_BLOCKERS: usize = 32;
const MAX_FACTORY_ARTIFACTS: usize = 64;
const MAX_FACTORY_EVIDENCE: usize = 128;
const MAX_FACTORY_IDEMPOTENCY: usize = 256;
const MAX_FACTORY_ARTIFACT_BYTES: u64 = 1024 * 1024 * 1024 * 1024;
pub const MAX_FACTORY_ATTEMPTS: u8 = 3;
pub const FACTORY_CLAIM_LEASE_SECONDS: i64 = 2 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpertCheck {
    pub name: String,
    pub kind: String,
    pub required: bool,
    pub evidence_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QualityContract {
    pub version: u32,
    #[serde(default)]
    pub checks: Vec<ExpertCheck>,
}

impl Default for QualityContract {
    fn default() -> Self {
        Self {
            version: 1,
            checks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceResult {
    Pass,
    Fail,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceSubmission {
    pub idempotency_key: String,
    pub check_name: String,
    pub result: EvidenceResult,
    pub command_label: Option<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpertEvidence {
    pub id: String,
    #[serde(flatten)]
    pub submission: EvidenceSubmission,
    pub submitted_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpertBlocker {
    pub kind: String,
    pub summary: String,
    pub reported_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpertWaiverInput {
    pub check_name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpertWaiver {
    pub check_name: String,
    pub reason: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FactoryRiskClass {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FactoryReadinessOverall {
    NotConfigured,
    Ready,
    NeedsAttention,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryReadinessSnapshot {
    pub checked_at: String,
    pub overall: FactoryReadinessOverall,
    pub evidence_revision: String,
    #[serde(default)]
    pub summary: Vec<String>,
}

/// Server-constructed Factory activation input. The desktop supplies the work-order
/// fields to `experts.rs`; that module attaches current readiness evidence before
/// calling the creation helpers in this module.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryRunCreate {
    pub ticket_reference: String,
    pub title: String,
    pub objective: String,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub non_goals: Vec<String>,
    pub playbook: Option<String>,
    pub workspace_pack_revision: Option<String>,
    pub risk: FactoryRiskClass,
    pub readiness: FactoryReadinessSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryWorkContract {
    pub ticket_reference: String,
    pub title: String,
    pub objective: String,
    pub acceptance_criteria: Vec<String>,
    pub non_goals: Vec<String>,
    pub project_path: String,
    pub expert_id: String,
    pub expert_version: u32,
    pub playbook: Option<String>,
    pub runbook: Option<String>,
    pub workspace_pack_revision: Option<String>,
    pub quality_contract: QualityContract,
    pub risk: FactoryRiskClass,
    pub readiness: FactoryReadinessSnapshot,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FactoryPhase {
    Preflight,
    Planning,
    AwaitingPlanApproval,
    Build,
    Validation,
    IndependentReview,
    Delivery,
    AwaitingFinalApproval,
    Completed,
}

impl FactoryPhase {
    pub fn worker_claimable(self) -> bool {
        matches!(
            self,
            Self::Planning
                | Self::Build
                | Self::Validation
                | Self::IndependentReview
                | Self::Delivery
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryPlan {
    pub revision: String,
    pub content: String,
    #[serde(default)]
    pub citations: Vec<String>,
    #[serde(default)]
    pub declared_checks: Vec<String>,
    #[serde(default)]
    pub risks: Vec<String>,
    #[serde(default)]
    pub known_limitations: Vec<String>,
    pub base_commit: String,
    pub submitted_by: String,
    pub submitted_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryPlanApproval {
    pub plan_revision: String,
    pub base_commit: String,
    pub approved_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryAttempt {
    pub number: u8,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub head_commit: Option<String>,
    pub builder_identity: Option<String>,
    pub result: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryClaim {
    pub id: String,
    pub idempotency_key: String,
    pub generation: u64,
    pub worker_identity: String,
    pub phase: FactoryPhase,
    pub run_revision: u64,
    pub claimed_at: String,
    pub last_renewed_at: String,
    pub expires_at: String,
    pub released_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryBlocker {
    pub id: String,
    pub run_id: String,
    pub idempotency_key: String,
    pub claim_id: String,
    pub claim_generation: u64,
    pub kind: String,
    pub summary: String,
    pub phase: FactoryPhase,
    pub attempt: u8,
    pub reported_by: String,
    pub reported_at: String,
    pub resolved_at: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FactoryProvenance {
    ClientReported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryArtifact {
    pub id: String,
    pub run_id: String,
    pub idempotency_key: String,
    pub claim_id: String,
    pub kind: String,
    pub label: String,
    pub reference: String,
    pub digest: String,
    pub byte_size: u64,
    pub summary: String,
    pub phase: FactoryPhase,
    pub attempt: u8,
    pub claim_generation: u64,
    pub work_contract_revision: String,
    pub approved_plan_revision: Option<String>,
    pub base_commit: Option<String>,
    pub head_commit: Option<String>,
    pub provenance: FactoryProvenance,
    pub submitted_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryEvidence {
    pub id: String,
    pub run_id: String,
    pub idempotency_key: String,
    pub claim_id: String,
    pub check_name: String,
    pub result: EvidenceResult,
    pub command_label: Option<String>,
    pub exit_code: Option<i32>,
    pub summary: String,
    #[serde(default)]
    pub artifact_ids: Vec<String>,
    pub phase: FactoryPhase,
    pub attempt: u8,
    pub claim_generation: u64,
    pub work_contract_revision: String,
    pub approved_plan_revision: Option<String>,
    pub base_commit: Option<String>,
    pub head_commit: Option<String>,
    pub provenance: FactoryProvenance,
    pub submitted_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryValidation {
    pub attempt: u8,
    pub head_commit: String,
    pub check_names: Vec<String>,
    pub phase: FactoryPhase,
    pub claim_id: String,
    pub claim_generation: u64,
    pub validated_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FactoryReviewVerdict {
    Pass,
    Rework,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FactoryReviewSeverity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryReviewFinding {
    pub severity: FactoryReviewSeverity,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryReview {
    pub attempt: u8,
    pub head_commit: String,
    pub phase: FactoryPhase,
    pub claim_id: String,
    pub claim_generation: u64,
    pub reviewer_identity: String,
    pub verdict: FactoryReviewVerdict,
    pub summary: String,
    #[serde(default)]
    pub findings: Vec<FactoryReviewFinding>,
    pub submitted_at: String,
    pub provenance: FactoryProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryDelivery {
    pub reference: String,
    pub attempt: u8,
    pub head_commit: String,
    pub phase: FactoryPhase,
    pub claim_id: String,
    pub claim_generation: u64,
    pub evidence_summary: String,
    pub known_limitations: Vec<String>,
    pub submitted_at: String,
    pub provenance: FactoryProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryHumanWaiver {
    pub kind: String,
    pub check_name: Option<String>,
    pub reason: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FactoryImprovementTarget {
    Test,
    Rule,
    Skill,
    Expert,
    Playbook,
    Instruction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryImprovementProposal {
    pub failure_class: String,
    pub target: FactoryImprovementTarget,
    pub proposal: String,
    pub suggested_test: Option<String>,
    pub provenance: FactoryProvenance,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FactoryTerminalOutcome {
    Accepted,
    Rework,
    Rejected,
    Cancelled,
    AttemptExhausted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryTerminalDecision {
    pub outcome: FactoryTerminalOutcome,
    pub decided_at: String,
    pub safe_detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryIdempotencyRecord {
    pub key: String,
    pub run_id: String,
    pub request_digest: String,
    pub result_id: String,
    pub result_revision: u64,
    pub result_phase: FactoryPhase,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_result: Option<FactoryClaim>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryWorkflow {
    pub work_contract: FactoryWorkContract,
    pub work_contract_revision: String,
    pub phase: FactoryPhase,
    pub revision: u64,
    pub created_at: String,
    pub preflight_completed_at: String,
    #[serde(default)]
    pub attempts: Vec<FactoryAttempt>,
    pub plan: Option<FactoryPlan>,
    pub plan_approval: Option<FactoryPlanApproval>,
    pub current_claim: Option<FactoryClaim>,
    #[serde(default)]
    pub prior_claims: Vec<FactoryClaim>,
    #[serde(default)]
    pub blockers: Vec<FactoryBlocker>,
    #[serde(default)]
    pub artifacts: Vec<FactoryArtifact>,
    #[serde(default)]
    pub evidence: Vec<FactoryEvidence>,
    pub validation: Option<FactoryValidation>,
    pub review: Option<FactoryReview>,
    pub delivery: Option<FactoryDelivery>,
    #[serde(default)]
    pub human_waivers: Vec<FactoryHumanWaiver>,
    pub terminal: Option<FactoryTerminalDecision>,
    pub improvement_proposal: Option<FactoryImprovementProposal>,
    #[serde(default)]
    pub idempotency: Vec<FactoryIdempotencyRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryClaimRequest {
    pub expected_revision: u64,
    pub phase: FactoryPhase,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryWorkerContext {
    pub expected_revision: u64,
    pub phase: FactoryPhase,
    pub attempt: u8,
    pub claim_id: String,
    pub claim_generation: u64,
    pub work_contract_revision: String,
    pub approved_plan_revision: Option<String>,
    pub base_commit: Option<String>,
    pub head_commit: Option<String>,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryPlanInput {
    pub content: String,
    #[serde(default)]
    pub citations: Vec<String>,
    #[serde(default)]
    pub declared_checks: Vec<String>,
    #[serde(default)]
    pub risks: Vec<String>,
    #[serde(default)]
    pub known_limitations: Vec<String>,
    pub base_commit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryArtifactInput {
    pub kind: String,
    pub label: String,
    pub reference: String,
    pub digest: String,
    pub byte_size: u64,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryEvidenceInput {
    pub check_name: String,
    pub result: EvidenceResult,
    pub command_label: Option<String>,
    pub exit_code: Option<i32>,
    pub summary: String,
    #[serde(default)]
    pub artifact_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryBlockerInput {
    pub kind: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryReviewInput {
    pub verdict: FactoryReviewVerdict,
    pub summary: String,
    #[serde(default)]
    pub findings: Vec<FactoryReviewFinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryDeliveryInput {
    pub reference: String,
    pub head_commit: String,
    pub evidence_summary: String,
    pub known_limitations: Vec<String>,
    pub improvement_proposal: Option<FactoryImprovementProposal>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FactoryPhaseCompletion {
    Planning { plan: FactoryPlanInput },
    Build { head_commit: String },
    Validation,
    IndependentReview { review: FactoryReviewInput },
    Delivery { delivery: FactoryDeliveryInput },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryMutationReceipt {
    pub id: String,
    pub revision: u64,
    pub phase: FactoryPhase,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FactoryPlanDecision {
    Approve,
    Reject,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryCheckWaiverInput {
    pub check_name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryFinalDecisionInput {
    pub expected_revision: u64,
    pub outcome: FactoryTerminalOutcome,
    pub approved_plan_revision: String,
    pub head_commit: String,
    #[serde(default)]
    pub check_waivers: Vec<FactoryCheckWaiverInput>,
    pub independent_review_waiver_reason: Option<String>,
    pub safe_detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryWorkSummary {
    pub run_id: String,
    pub ticket_reference: String,
    pub title: String,
    pub phase: FactoryPhase,
    pub revision: u64,
    pub attempt: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryClaimContract {
    pub run_id: String,
    pub project_path: String,
    pub expert_id: String,
    pub expert_version: u32,
    pub work_contract: FactoryWorkContract,
    pub work_contract_revision: String,
    pub phase: FactoryPhase,
    pub attempt: u8,
    pub attempt_limit: u8,
    pub run_revision: u64,
    pub approved_plan: Option<FactoryPlan>,
    pub base_commit: Option<String>,
    pub head_commit: Option<String>,
    pub required_checks: Vec<ExpertCheck>,
    pub permitted_submissions: Vec<FactoryPermittedSubmissionShape>,
    pub claim_id: String,
    pub claim_generation: u64,
    pub expires_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FactoryPermittedSubmissionShape {
    Artifact,
    Evidence,
    Blocker,
    PlanningCompletion,
    BuildCompletion,
    ValidationCompletion,
    IndependentReviewCompletion,
    DeliveryCompletion,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExpertRunState {
    InProgress,
    AwaitingReview,
    Accepted,
    Rework,
    Rejected,
    Cancelled,
}

impl ExpertRunState {
    fn terminal(self) -> bool {
        matches!(
            self,
            Self::Accepted | Self::Rework | Self::Rejected | Self::Cancelled
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpertRunCreate {
    pub expert_id: String,
    pub expert_version: u32,
    pub project_path: String,
    pub client: String,
    pub lead_agent: String,
    pub supporting_agents: Vec<String>,
    pub required_skills: Vec<String>,
    pub optional_skills: Vec<String>,
    pub runbook: Option<String>,
    pub contract: QualityContract,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpertRun {
    pub id: String,
    #[serde(flatten)]
    pub snapshot: ExpertRunCreate,
    pub state: ExpertRunState,
    pub started_at: String,
    pub ended_at: Option<String>,
    #[serde(default)]
    pub evidence: Vec<ExpertEvidence>,
    #[serde(default)]
    pub blockers: Vec<ExpertBlocker>,
    #[serde(default)]
    pub waivers: Vec<ExpertWaiver>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub factory: Option<FactoryWorkflow>,
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::InvalidArgument {
        message: message.into(),
    }
}

fn path(state: &AppState) -> std::path::PathBuf {
    state.app_data_dir.join("state").join("expert-runs.json")
}

fn lock(state: &AppState) -> Result<std::fs::File, AppError> {
    let directory = state.app_data_dir.join("state");
    std::fs::create_dir_all(&directory).map_err(|error| AppError::Io {
        message: format!("create Expert run state directory: {error}"),
    })?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("expert-runs.lock"))
        .map_err(|error| AppError::Io {
            message: format!("open Expert run lock: {error}"),
        })?;
    file.lock().map_err(|error| AppError::Io {
        message: format!("lock Expert runs: {error}"),
    })?;
    Ok(file)
}

async fn load(state: &AppState) -> Result<Vec<ExpertRun>, AppError> {
    if let Some(database) = state.completed_state_database().await? {
        return database
            .read(document_spec())
            .await?
            .ok_or_else(|| AppError::StorageCorrupt {
                message: "Expert runs are missing after SQLite migration".into(),
            });
    }
    let path = path(state);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = read_capped(&path, MAX_RUN_BYTES).await?;
    let runs: Vec<ExpertRun> = serde_json::from_slice(&raw)
        .map_err(|error| invalid(format!("parse Expert runs: {error}")))?;
    validate_runs(&runs)?;
    Ok(runs)
}

async fn save(state: &AppState, runs: &[ExpertRun]) -> Result<(), AppError> {
    validate_runs(runs)?;
    if let Some(database) = state.completed_state_database().await? {
        let replacement = runs.to_vec();
        return database
            .mutate(document_spec(), Vec::new(), move |current| {
                *current = replacement;
                Ok(())
            })
            .await;
    }
    let bytes = serde_json::to_vec_pretty(runs)
        .map_err(|error| invalid(format!("serialize Expert runs: {error}")))?;
    if bytes.len() as u64 > MAX_RUN_BYTES {
        return Err(invalid("Expert run state capacity reached"));
    }
    atomic_write(&path(state), &bytes).await
}

fn validate_runs(runs: &[ExpertRun]) -> Result<(), AppError> {
    let mut ids = HashSet::new();
    if runs.len() > MAX_RUNS {
        return Err(invalid("Expert run state capacity reached"));
    }
    for run in runs {
        if uuid::Uuid::parse_str(&run.id).is_err()
            || !ids.insert(run.id.as_str())
            || run.snapshot.expert_version == 0
        {
            return Err(invalid("Expert run identity is invalid"));
        }
        validate_text(&run.snapshot.expert_id, "expertId")?;
        validate_text(&run.snapshot.project_path, "projectPath")?;
        validate_text(&run.snapshot.client, "client")?;
        validate_contract(&run.snapshot.contract)?;
        if let Some(factory) = &run.factory {
            validate_factory_workflow(factory, &run.snapshot)?;
            validate_factory_persisted_trust(factory, run)?;
            if factory.terminal.is_some() != run.state.terminal() {
                return Err(invalid("Factory terminal state is inconsistent"));
            }
        }
    }
    Ok(())
}

fn document_spec() -> crate::state_db::DocumentSpec<Vec<ExpertRun>> {
    crate::state_db::DocumentSpec::new("expert_runs", 1, MAX_RUN_BYTES, |runs| validate_runs(runs))
}

pub(crate) fn import_spec() -> crate::state_db::ImportSpec {
    crate::state_db::ImportSpec::document(document_spec(), Vec::new())
}

fn validate_text(value: &str, field: &str) -> Result<(), AppError> {
    if value.trim().is_empty() || value.len() > MAX_TEXT {
        return Err(invalid(format!("{field} is empty or oversized")));
    }
    Ok(())
}

fn validate_factory_text(value: &str, field: &str, max: usize) -> Result<(), AppError> {
    if value.trim().is_empty() || value.len() > max || value.chars().any(|ch| ch == '\0') {
        return Err(invalid(format!("{field} is empty or oversized")));
    }
    Ok(())
}

fn validate_factory_optional_text(
    value: Option<&str>,
    field: &str,
    max: usize,
) -> Result<(), AppError> {
    if let Some(value) = value {
        validate_factory_text(value, field, max)?;
    }
    Ok(())
}

fn factory_credential_key(value: &str) -> bool {
    let normalized = value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    matches!(
        normalized.as_str(),
        "token"
            | "accesstoken"
            | "refreshtoken"
            | "idtoken"
            | "jwt"
            | "sig"
            | "signature"
            | "apikey"
            | "password"
            | "passwd"
            | "pwd"
            | "secret"
            | "credential"
            | "credentials"
            | "auth"
            | "privatekey"
            | "clientsecret"
            | "authorization"
            | "xamzcredential"
            | "xamzsignature"
            | "session"
            | "sid"
            | "sessionid"
            | "sessionkey"
            | "sessiontoken"
            | "sessionsecret"
            | "sessioncookie"
            | "phpsessid"
            | "jsessionid"
            | "aspnetsessionid"
            | "connectsid"
    ) || [
        "accesstoken",
        "refreshtoken",
        "idtoken",
        "jwt",
        "sig",
        "signature",
        "token",
        "apikey",
        "accesskey",
        "secretkey",
        "privatekey",
        "clientsecret",
        "password",
        "passwd",
        "pwd",
        "credential",
        "credentials",
        "auth",
        "secret",
        "signature",
        "authorization",
        "sessionid",
        "sessionkey",
        "sessiontoken",
        "sessionsecret",
        "sessioncookie",
    ]
    .iter()
    .any(|suffix| normalized.len() > suffix.len() && normalized.ends_with(suffix))
}

fn factory_url_component_has_credential(value: &str) -> bool {
    let Some(decoded) = decode_factory_url_component(value) else {
        return true;
    };
    let decoded_separators = decoded
        .replace("%5F", "_")
        .replace("%5f", "_")
        .replace("%2D", "-")
        .replace("%2d", "-")
        .replace("%2E", ".")
        .replace("%2e", ".");
    decoded_separators
        .split(|character: char| {
            character.is_ascii_whitespace()
                || matches!(character, '/' | '\\' | ':' | '?' | '#' | '&' | '=')
        })
        .any(factory_credential_key)
}

fn decode_factory_url_component(value: &str) -> Option<String> {
    let mut decoded = value.to_owned();
    for _ in 0..=value.len() {
        let next = decode_factory_percent_encoding(&decoded)?;
        if next == decoded {
            return Some(decoded);
        }
        decoded = next;
    }
    None
}

fn factory_value_looks_like_jwt(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    let encoded = |part: &str| {
        part.chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    };
    match parts.as_slice() {
        [header, payload, signature] => {
            header.starts_with("eyJ")
                && encoded(header)
                && !payload.is_empty()
                && encoded(payload)
                && encoded(signature)
        }
        [header, encrypted_key, iv, ciphertext, tag] => {
            header.starts_with("eyJ")
                && [header, iv, ciphertext, tag]
                    .iter()
                    .all(|part| !part.is_empty() && encoded(part))
                && encoded(encrypted_key)
        }
        _ => false,
    }
}

fn factory_value_looks_like_bearer(value: &str) -> bool {
    let candidate = value.trim_matches(|character: char| {
        !character.is_ascii_alphanumeric() && !matches!(character, '_' | '-')
    });
    let prefixes = [
        "xoxb-",
        "xoxp-",
        "xoxa-",
        "xoxr-",
        "xoxs-",
        "xapp-",
        "xwfp-",
        "glpat-",
        "glsoat-",
        "glffct-",
        "gloas-",
        "gldt-",
        "glrt-",
        "glcbt-",
        "glptt-",
        "glft-",
        "glimt-",
        "glagent-",
        "glwt-",
        "hf_",
        "npm_",
        "dckr_pat_",
        "pypi-",
        "dop_v1_",
        "AIzaSy",
        "ya29.",
        "sk_live_",
        "sk_test_",
        "rk_live_",
        "rk_test_",
        "whsec_",
    ];
    prefixes.iter().any(|prefix| {
        candidate.strip_prefix(prefix).is_some_and(|tail| {
            tail.len() >= 16
                && tail.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
                })
        })
    }) || {
        let parts = candidate.split('.').collect::<Vec<_>>();
        matches!(parts.as_slice(), [prefix, first, second]
        if (2..=8).contains(&prefix.len())
            && prefix.chars().all(|character| character.is_ascii_uppercase())
            && first.len() >= 16
            && second.len() >= 16
            && [first, second].iter().all(|part| part.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
            })))
    } || candidate
        .rsplit_once(['_', '-'])
        .is_some_and(|(prefix, tail)| {
            let normalized_prefix = prefix
                .to_ascii_lowercase()
                .replace(['_', '-'].as_slice(), "");
            tail.len() >= 16
                && (normalized_prefix.contains("api") || normalized_prefix.contains("pat"))
                && tail.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
                })
        })
}

fn contains_factory_credential(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "authorization:",
        "bearer ",
        "token=",
        "token:",
        "access_token",
        "api_key",
        "api-key",
        "apikey",
        "password=",
        "password:",
        "secret=",
        "secret:",
        "-----begin private key",
        "ghp_",
        "gho_",
        "ghu_",
        "ghs_",
        "ghr_",
        "sk-",
        "x-amz-credential",
        "x-amz-signature",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
        || value
            .split(|character: char| {
                character.is_ascii_whitespace()
                    || matches!(
                        character,
                        '/' | '\\'
                            | ':'
                            | '?'
                            | '#'
                            | '&'
                            | '='
                            | '"'
                            | '\''
                            | '`'
                            | '('
                            | ')'
                            | '['
                            | ']'
                            | '{'
                            | '}'
                            | '<'
                            | '>'
                            | ','
                            | ';'
                    )
            })
            .any(factory_value_looks_like_bearer)
        || value
            .split(|character: char| {
                character.is_ascii_whitespace()
                    || matches!(character, '/' | '\\' | ':' | '?' | '#' | '&' | '=')
            })
            .any(factory_value_looks_like_jwt)
        || lower.split_whitespace().any(|token| {
            token
                .split_once('=')
                .or_else(|| token.split_once(':'))
                .is_some_and(|(key, _)| factory_credential_key(key))
        })
        || lower.lines().any(|line| {
            line.char_indices().any(|(index, character)| {
                matches!(character, ':' | '=')
                    && line[..index]
                        .split_whitespace()
                        .next_back()
                        .is_some_and(factory_credential_key)
                    && !line[index + character.len_utf8()..].trim_start().is_empty()
            })
        })
        || {
            let compact = lower
                .chars()
                .filter(|character| !character.is_ascii_whitespace())
                .collect::<String>();
            compact.char_indices().any(|(index, character)| {
                matches!(character, ':' | '=')
                    && factory_credential_key(&compact[..index])
                    && !compact[index + character.len_utf8()..].is_empty()
            })
        }
}

fn contains_factory_unsafe_url(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    ["https://", "http://"].iter().any(|scheme| {
        let mut offset = 0;
        while let Some(relative) = lower[offset..].find(scheme) {
            let start = offset + relative;
            let suffix = &value[start..];
            let end = suffix
                .find(|character: char| {
                    character.is_ascii_whitespace()
                        || matches!(character, '"' | '`' | '[' | ']' | '{' | '}' | '<' | '>')
                })
                .unwrap_or(suffix.len());
            if url::Url::parse(&suffix[..end]).ok().is_some_and(|parsed| {
                !parsed.username().is_empty()
                    || parsed.password().is_some()
                    || factory_url_has_credentials(&parsed)
                    || factory_url_has_private_path(&parsed)
            }) {
                return true;
            }
            offset = start + scheme.len();
        }
        false
    })
}

fn contains_private_absolute_path(value: &str) -> bool {
    value
        .split(|character: char| {
            character.is_ascii_whitespace()
                || matches!(
                    character,
                    '"' | '\''
                        | '`'
                        | '('
                        | ')'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                        | '<'
                        | '>'
                        | ','
                        | ';'
                        | '='
                )
        })
        .any(|candidate| {
            let absolute = |path: &str| {
                let bytes = path.as_bytes();
                (path.starts_with('/') && path.len() > 1)
                    || path.starts_with("\\\\")
                    || path.starts_with("//")
                    || (bytes.len() >= 3
                        && bytes[0].is_ascii_alphabetic()
                        && bytes[1] == b':'
                        && matches!(bytes[2], b'/' | b'\\'))
            };
            absolute(candidate)
                || candidate.rsplit_once(':').is_some_and(|(prefix, suffix)| {
                    !matches!(prefix.to_ascii_lowercase().as_str(), "http" | "https")
                        && absolute(suffix)
                })
        })
}

fn split_factory_yaml_pair(value: &str) -> Option<(&str, &str)> {
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in value.char_indices() {
        if let Some(active_quote) = quote {
            if active_quote == '"' && escaped {
                escaped = false;
            } else if active_quote == '"' && character == '\\' {
                escaped = true;
            } else if character == active_quote {
                quote = None;
            }
        } else if matches!(character, '"' | '\'') {
            quote = Some(character);
        } else if character == ':' {
            return Some((&value[..index], &value[index + 1..]));
        }
    }
    None
}

fn contains_factory_source_snippet(value: &str) -> bool {
    let is_sql = |source: &str| {
        let source = source.trim();
        let source_statement = source.trim_end_matches(';').trim();
        let statement = source_statement.to_ascii_lowercase();
        let words = statement.split_whitespace().collect::<Vec<_>>();
        let identifier = |value: &str| {
            !value.is_empty()
                && !value.starts_with('.')
                && !value.ends_with('.')
                && value.chars().all(|character| {
                    character.is_ascii_alphanumeric()
                        || matches!(character, '_' | '$' | '.' | '`' | '"' | '[' | ']')
                })
        };
        let table_reference = |value: &str| {
            let parts = value.split_whitespace().collect::<Vec<_>>();
            match parts.as_slice() {
                [table] => identifier(table),
                [table, alias] => identifier(table) && identifier(alias),
                [table, "as", alias] => identifier(table) && identifier(alias),
                _ => false,
            }
        };
        // ponytail: these reviewed UI phrases are also valid SQL; add literals only with paired
        // unsafe SQL regressions so the metadata boundary remains fail-closed.
        let known_sql_shaped_prose = !source.ends_with(';')
            && matches!(
                source_statement,
                "Select items from catalog"
                    | "Select items from catalog for review."
                    | "Delete from history"
            );
        let select_shape = !known_sql_shaped_prose
            && words.first() == Some(&"select")
            && words
                .iter()
                .position(|word| *word == "from")
                .is_some_and(|from| {
                    let clause = words[from + 1..]
                        .iter()
                        .position(|word| {
                            matches!(
                                *word,
                                "where"
                                    | "join"
                                    | "left"
                                    | "right"
                                    | "inner"
                                    | "outer"
                                    | "group"
                                    | "order"
                                    | "limit"
                                    | "offset"
                                    | "having"
                                    | "union"
                                    | "intersect"
                                    | "except"
                                    | "fetch"
                                    | "for"
                            )
                        })
                        .map(|position| from + 1 + position)
                        .unwrap_or(words.len());
                    let source = words[from + 1..clause].join(" ");
                    from > 1
                        && from + 1 < words.len()
                        && (table_reference(&source)
                            || source.contains(',')
                            || source.contains('('))
                        && (matches!(from, 2 | 3)
                            || matches!(words[1], "distinct" | "all" | "distinctrow" | "top")
                            || words[1..from].iter().any(|word| {
                                *word == "as" || word.contains([',', '*', '.', '('].as_slice())
                            }))
                });
        let select_literal_shape = statement.strip_prefix("select ").is_some_and(|projection| {
            !projection.contains(" from ")
                && (projection.parse::<f64>().is_ok()
                    || matches!(projection, "null" | "true" | "false")
                    || projection.starts_with(['\'', '"'].as_slice())
                    || projection.contains(['(', ')', ',', '+', '-', '*', '/'].as_slice()))
        });
        let with_shape = statement
            .strip_prefix("with ")
            .map(|rest| rest.strip_prefix("recursive ").unwrap_or(rest))
            .is_some_and(|rest| {
                rest.contains(" as (")
                    && [") select ", ") insert into ", ") update ", ") delete from "]
                        .iter()
                        .any(|continuation| rest.contains(continuation))
            });
        let insert_shape = statement.strip_prefix("insert into ").is_some_and(|rest| {
            let object_end = rest
                .find(|character: char| character.is_ascii_whitespace() || character == '(')
                .unwrap_or(rest.len());
            let object = &rest[..object_end];
            let mut body = rest[object_end..].trim_start();
            if body.starts_with('(') {
                body = body
                    .split_once(')')
                    .map_or("", |(_, remainder)| remainder.trim_start());
            }
            identifier(object)
                && (body.starts_with("values(")
                    || body.starts_with("values (")
                    || body.starts_with("select ")
                    || body == "default values"
                    || body.starts_with("set ") && body.contains('='))
        });
        let update_shape = statement
            .strip_prefix("update ")
            .and_then(|rest| rest.split_once(" set "))
            .is_some_and(|(target, assignments)| {
                table_reference(target.trim()) && assignments.contains('=')
            });
        let delete_shape = !known_sql_shaped_prose
            && statement.strip_prefix("delete from ").is_some_and(|rest| {
                let parts = rest.split_whitespace().collect::<Vec<_>>();
                let clause = parts
                    .iter()
                    .position(|word| matches!(*word, "where" | "using" | "returning"));
                let target_end = clause.unwrap_or(parts.len());
                table_reference(&parts[..target_end].join(" "))
            });
        let ddl_prefixes = [
            "create unique index concurrently ",
            "create index concurrently ",
            "drop index concurrently ",
            "refresh materialized view concurrently ",
            "create or replace temporary view ",
            "create or replace temp view ",
            "create or replace temporary recursive view ",
            "create or replace temp recursive view ",
            "create or replace recursive view ",
            "create temporary recursive view ",
            "create temp recursive view ",
            "create global temporary table ",
            "create local temporary table ",
            "create table ",
            "create temp table ",
            "create temporary table ",
            "create unlogged table ",
            "create index ",
            "create unique index ",
            "create view ",
            "create materialized view ",
            "create or replace view ",
            "create or replace materialized view ",
            "create recursive view ",
            "create temp view ",
            "create temporary view ",
            "create database ",
            "create schema ",
            "create type ",
            "create function ",
            "create or replace function ",
            "create procedure ",
            "create or replace procedure ",
            "create trigger ",
            "create or replace trigger ",
            "create temporary sequence ",
            "create temp sequence ",
            "create unlogged sequence ",
            "create sequence ",
            "alter table ",
            "alter index ",
            "alter view ",
            "alter materialized view ",
            "alter database ",
            "alter schema ",
            "alter sequence ",
            "alter function ",
            "alter procedure ",
            "alter trigger ",
            "drop table ",
            "drop index ",
            "drop view ",
            "drop materialized view ",
            "drop database ",
            "drop schema ",
            "drop sequence ",
            "drop function ",
            "drop procedure ",
            "drop trigger ",
            "truncate table ",
            "truncate ",
            "refresh materialized view ",
        ];
        let ddl_shape = ddl_prefixes
            .iter()
            .find_map(|prefix| statement.strip_prefix(prefix))
            .is_some_and(|rest| {
                let rest = rest
                    .strip_prefix("if not exists ")
                    .or_else(|| rest.strip_prefix("if exists "))
                    .unwrap_or(rest);
                let object_end = rest
                    .find(|character: char| character.is_ascii_whitespace() || character == '(')
                    .unwrap_or(rest.len());
                let object = &rest[..object_end];
                let suffix = rest[object_end..].trim_start();
                identifier(object)
                    && (suffix.is_empty()
                        || suffix.starts_with('(')
                        || [
                            "add ",
                            "drop ",
                            "rename ",
                            "alter ",
                            "as ",
                            "on ",
                            "using ",
                            "with ",
                            "like ",
                            "enable ",
                            "disable ",
                            "owner ",
                            "set ",
                            "reset ",
                            "validate ",
                            "attach ",
                            "detach ",
                            "cluster ",
                            "without ",
                            "cascade",
                            "restrict",
                            "returns ",
                            "language ",
                            "before ",
                            "after ",
                            "instead ",
                            "execute ",
                        ]
                        .iter()
                        .any(|clause| suffix.starts_with(clause)))
            });
        let generic_ddl_shape = !source.ends_with(['.', '!', '?'].as_slice())
            && matches!(
                words.as_slice(),
                ["create" | "alter" | "drop", _, _, ..] | ["comment", "on", _, _, ..]
            );
        let explain_shape = statement.strip_prefix("explain ").is_some_and(|rest| {
            let mut remainder = rest.trim_start();
            if let Some(tail) = remainder.strip_prefix("plan for ") {
                remainder = tail.trim_start();
            }
            loop {
                if let Some(options) = remainder.strip_prefix('(') {
                    let Some((_, tail)) = options.split_once(')') else {
                        return false;
                    };
                    remainder = tail.trim_start();
                    continue;
                }
                if let Some(tail) = remainder.strip_prefix("query plan ") {
                    remainder = tail.trim_start();
                    continue;
                }
                if let Some(tail) = remainder
                    .strip_prefix("analyze ")
                    .or_else(|| remainder.strip_prefix("analyse "))
                    .or_else(|| remainder.strip_prefix("verbose "))
                    .or_else(|| remainder.strip_prefix("extended "))
                    .or_else(|| remainder.strip_prefix("partitions "))
                {
                    remainder = tail.trim_start();
                    continue;
                }
                if let Some(tail) = remainder.strip_prefix("format=") {
                    remainder = tail
                        .split_once(char::is_whitespace)
                        .map_or("", |(_, statement)| statement.trim_start());
                    continue;
                }
                if let Some(tail) = remainder.strip_prefix("format ") {
                    remainder = tail
                        .split_once(char::is_whitespace)
                        .map_or("", |(_, statement)| statement.trim_start());
                    continue;
                }
                break;
            }
            [
                "select ", "with ", "insert ", "update ", "delete ", "create ", "alter ", "drop ",
                "values ",
            ]
            .iter()
            .any(|prefix| remainder.starts_with(prefix))
        });
        let pragma_shape = statement.strip_prefix("pragma ").is_some_and(|rest| {
            !rest.is_empty()
                && (rest.contains('=')
                    || rest.split_once('(').is_some_and(|(name, arguments)| {
                        identifier(name) && arguments.ends_with(')')
                    })
                    || identifier(rest))
        });
        let values_shape = statement
            .strip_prefix("values ")
            .or_else(|| statement.strip_prefix("values"))
            .is_some_and(|rest| {
                rest.trim_start().starts_with('(') && rest.trim_end().ends_with(')')
            });
        let transaction_words = statement.split_whitespace().collect::<Vec<_>>();
        let known_transaction_prose =
            matches!(source, "Begin" | "Commit" | "Rollback" | "Abort" | "End");
        let transaction_unit = |value: &str| matches!(value, "transaction" | "work");
        let transaction_mode =
            |value: &str| matches!(value, "deferred" | "immediate" | "exclusive");
        let transaction_shape = match transaction_words.as_slice() {
            ["begin"] | ["commit"] | ["end"] | ["rollback"] | ["abort"] => !known_transaction_prose,
            ["start", "transaction"] => true,
            ["begin", value] => transaction_unit(value) || transaction_mode(value),
            ["begin", mode, unit] => transaction_mode(mode) && transaction_unit(unit),
            [operation, unit] if matches!(*operation, "commit" | "end" | "rollback" | "abort") => {
                transaction_unit(unit)
            }
            [operation, "and", "chain"] | [operation, "and", "no", "chain"]
                if matches!(*operation, "commit" | "end" | "rollback" | "abort") =>
            {
                true
            }
            [operation, unit, "and", "chain"] | [operation, unit, "and", "no", "chain"]
                if matches!(*operation, "commit" | "end" | "rollback" | "abort") =>
            {
                transaction_unit(unit)
            }
            ["rollback", "to", name] | ["savepoint", name] | ["release", name] => identifier(name),
            ["rollback", "to", "savepoint", name] | ["release", "savepoint", name] => {
                identifier(name)
            }
            ["rollback", unit, "to", name] if transaction_unit(unit) => identifier(name),
            ["rollback", unit, "to", "savepoint", name] if transaction_unit(unit) => {
                identifier(name)
            }
            _ => statement
                .strip_prefix("set transaction ")
                .or_else(|| statement.strip_prefix("set session characteristics as transaction "))
                .is_some_and(|mode| {
                    mode.starts_with("isolation level ")
                        || matches!(
                            mode,
                            "read only" | "read write" | "deferrable" | "not deferrable"
                        )
                }),
        };
        let object_reference = |value: &str| {
            let parts = value.split_whitespace().collect::<Vec<_>>();
            match parts.as_slice() {
                [object] => identifier(object),
                [kind, object]
                    if matches!(
                        *kind,
                        "table" | "sequence" | "function" | "procedure" | "database" | "schema"
                    ) =>
                {
                    identifier(object)
                }
                _ => false,
            }
        };
        let principals = |value: &str| {
            let value = value
                .strip_suffix(" with grant option")
                .or_else(|| value.strip_suffix(" cascade"))
                .or_else(|| value.strip_suffix(" restrict"))
                .unwrap_or(value);
            value
                .split(',')
                .all(|principal| identifier(principal.trim()))
        };
        let grant_shape = statement.strip_prefix("grant ").is_some_and(|rest| {
            if let Some((privileges, object_and_grantee)) = rest.split_once(" on ") {
                object_and_grantee
                    .split_once(" to ")
                    .is_some_and(|(object, grantee)| {
                        !privileges.trim().is_empty()
                            && object_reference(object.trim())
                            && principals(grantee.trim())
                    })
            } else {
                rest.split_once(" to ").is_some_and(|(roles, grantees)| {
                    principals(roles.trim()) && principals(grantees.trim())
                })
            }
        });
        let revoke_shape = statement.strip_prefix("revoke ").is_some_and(|rest| {
            if let Some((privileges, object_and_grantee)) = rest.split_once(" on ") {
                object_and_grantee
                    .split_once(" from ")
                    .is_some_and(|(object, grantee)| {
                        !privileges.trim().is_empty()
                            && object_reference(object.trim())
                            && principals(grantee.trim())
                    })
            } else {
                rest.split_once(" from ").is_some_and(|(roles, grantees)| {
                    principals(roles.trim()) && principals(grantees.trim())
                })
            }
        });
        let show_shape = match words.as_slice() {
            ["show", "tables"]
            | ["show", "databases"]
            | ["show", "variables"]
            | ["show", "status"] => true,
            ["show", "tables" | "databases", "from" | "in" | "like", value]
            | ["show", "variables" | "status", "like", value] => identifier(value),
            ["show", "columns" | "fields", "from" | "in", object] => identifier(object),
            ["show", "columns" | "fields", "from" | "in", object, "from" | "in", database] => {
                identifier(object) && identifier(database)
            }
            ["show", "create", "table", object] => identifier(object),
            ["show", value] => {
                identifier(value)
                    && (source.ends_with(';') || value.contains(['_', '.'].as_slice()))
            }
            _ => false,
        };
        let describe_shape = statement
            .strip_prefix("describe ")
            .or_else(|| statement.strip_prefix("desc "))
            .is_some_and(|description| {
                let parts = description.split_whitespace().collect::<Vec<_>>();
                matches!(parts.as_slice(), [object] if identifier(object))
                    || matches!(parts.as_slice(), [object, column] if identifier(object) && identifier(column))
            });
        let merge_shape = statement.strip_prefix("merge into ").is_some_and(|rest| {
            rest.split_once(" using ")
                .is_some_and(|(target, remainder)| {
                    table_reference(target.trim())
                        && remainder.contains(" on ")
                        && remainder.contains(" when ")
                })
        });
        let replace_shape = statement.strip_prefix("replace into ").is_some_and(|rest| {
            let object_end = rest
                .find(|character: char| character.is_ascii_whitespace() || character == '(')
                .unwrap_or(rest.len());
            let object = &rest[..object_end];
            let mut body = rest[object_end..].trim_start();
            if body.starts_with('(') {
                body = body
                    .split_once(')')
                    .map_or("", |(_, remainder)| remainder.trim_start());
            }
            identifier(object)
                && (body.starts_with("values(")
                    || body.starts_with("values (")
                    || body.starts_with("select ")
                    || body == "default values"
                    || body.starts_with("set ") && body.contains('='))
        });
        let call_shape = statement.strip_prefix("call ").is_some_and(|rest| {
            rest.split_once('(').is_some_and(|(procedure, arguments)| {
                identifier(procedure.trim()) && arguments.ends_with(')')
            })
        });
        let execute_shape = statement
            .strip_prefix("exec ")
            .or_else(|| statement.strip_prefix("execute "))
            .and_then(|rest| rest.split_whitespace().next())
            .is_some_and(identifier);
        let maintenance_shape = (source.ends_with(';')
            && matches!(statement.as_str(), "vacuum" | "analyze" | "analyse"))
            || ["vacuum ", "analyze ", "analyse "]
                .iter()
                .find_map(|prefix| statement.strip_prefix(prefix))
                .is_some_and(|target| identifier(target.trim()));
        let copy_shape = statement.strip_prefix("copy ").is_some_and(|rest| {
            rest.split_once(" to ")
                .or_else(|| rest.split_once(" from "))
                .is_some_and(|(source, target)| {
                    !source.trim().is_empty() && !target.trim().is_empty()
                })
        });
        let upsert_shape = statement.strip_prefix("upsert into ").is_some_and(|rest| {
            let object_end = rest
                .find(|character: char| character.is_ascii_whitespace() || character == '(')
                .unwrap_or(rest.len());
            identifier(&rest[..object_end]) && !rest[object_end..].trim().is_empty()
        });
        select_shape
            || select_literal_shape
            || with_shape
            || insert_shape
            || update_shape
            || delete_shape
            || ddl_shape
            || generic_ddl_shape
            || explain_shape
            || pragma_shape
            || values_shape
            || transaction_shape
            || grant_shape
            || revoke_shape
            || show_shape
            || describe_shape
            || merge_shape
            || replace_shape
            || call_shape
            || execute_shape
            || maintenance_shape
            || copy_shape
            || upsert_shape
            || (source.ends_with(';')
                && ((statement.starts_with("select ") && statement.contains(" from "))
                    || (statement.starts_with("insert ") && statement.contains(" into "))
                    || (statement.starts_with("update ") && statement.contains(" set "))
                    || (statement.starts_with("delete ") && statement.contains(" from "))
                    || ddl_prefixes
                        .iter()
                        .any(|prefix| statement.starts_with(prefix))
                    || (statement.starts_with("with ") && statement.contains(" select "))))
    };
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let collapsed_lower = collapsed.to_ascii_lowercase();
    let collapsed_call = collapsed_lower
        .strip_prefix("await ")
        .unwrap_or(collapsed_lower.as_str());
    let multiline_call = collapsed_call.ends_with(')')
        && collapsed_call
            .split_once('(')
            .is_some_and(|(callee, rest)| {
                !rest.is_empty()
                    && !callee.is_empty()
                    && callee.chars().all(|character| {
                        character.is_ascii_alphanumeric()
                            || matches!(character, '_' | '$' | '.' | ':' | '!' | '?')
                    })
            });
    let css_shape = collapsed
        .split_once('{')
        .and_then(|(selector, body)| body.rsplit_once('}').map(|(body, _)| (selector, body)))
        .is_some_and(|(selector, body)| {
            !selector.trim().is_empty()
                && body.split(';').any(|declaration| {
                    declaration
                        .split_once(':')
                        .is_some_and(|(property, value)| {
                            let property = property.trim();
                            !property.is_empty()
                                && !value.trim().is_empty()
                                && property.chars().all(|character| {
                                    character.is_ascii_alphanumeric()
                                        || matches!(character, '_' | '-')
                                })
                        })
                })
        });
    let nested_css_shape = collapsed.starts_with('@')
        && collapsed.matches('{').count() >= 2
        && collapsed.matches('}').count() >= 2
        && collapsed.contains(':');
    if is_sql(&collapsed)
        || multiline_call
        || css_shape
        || nested_css_shape
        || serde_json::from_str::<serde_json::Value>(value)
            .ok()
            .is_some_and(|parsed| parsed.is_array() || parsed.is_object())
        || (value.trim().starts_with('<') && value.trim().ends_with('>'))
    {
        return true;
    }

    let known_yaml_shaped_prose = matches!(
        value.trim(),
        "Database:\nHost details remain client-reported."
            | "Status: ready for desktop review."
            | "Status: ready"
            | "Client-reported: shown only as bounded metadata."
    );
    value.lines().any(|line| {
        let line = line.trim();
        let lower = line.to_ascii_lowercase();
        let declaration_line = if let Some(visibility) = lower.strip_prefix("pub(") {
            visibility
                .split_once(") ")
                .map_or(lower.as_str(), |(_, declaration)| declaration)
        } else {
            lower.as_str()
        };
        let declaration = [
            "fn ",
            "pub fn ",
            "async fn ",
            "def ",
            "class ",
            "struct ",
            "pub struct ",
            "enum ",
            "pub enum ",
            "trait ",
            "pub trait ",
            "impl ",
            "interface ",
            "export interface ",
            "type ",
            "export type ",
            "function ",
            "#include ",
            "import ",
            "package ",
            "public class ",
            "private class ",
            "const ",
            "let ",
            "var ",
        ]
        .iter()
        .any(|prefix| declaration_line.starts_with(prefix));
        let source_identifier = |candidate: &str| {
            !candidate.is_empty()
                && candidate.split('.').all(|segment| {
                    segment.chars().next().is_some_and(|character| {
                        character.is_ascii_alphabetic() || matches!(character, '_' | '$')
                    }) && segment.chars().all(|character| {
                        character.is_ascii_alphanumeric() || matches!(character, '_' | '$')
                    })
                })
        };
        let go_package = declaration_line
            .strip_prefix("package ")
            .is_some_and(|name| source_identifier(name.trim()));
        let shell_set = declaration_line
            .strip_prefix("set ")
            .and_then(|options| options.split_whitespace().next())
            .is_some_and(|options| options.starts_with(['-', '+'].as_slice()) && options.len() > 1);
        let namespace = [
            "export inline namespace ",
            "export namespace ",
            "inline namespace ",
            "namespace ",
        ]
        .iter()
        .find_map(|prefix| declaration_line.strip_prefix(prefix))
        .is_some_and(|name| {
            let terminated = name.trim_end().ends_with(['{', ';'].as_slice());
            let name = name.trim_end_matches(['{', ';'].as_slice()).trim();
            let qualified = |candidate: &str| {
                candidate
                    .split("::")
                    .all(|segment| source_identifier(segment.trim()))
            };
            terminated
                && (name.is_empty()
                    || qualified(name)
                    || name.split_once('=').is_some_and(|(alias, target)| {
                        qualified(alias.trim()) && qualified(target.trim())
                    }))
        });
        let python_import = line
            .strip_prefix("import ")
            .is_some_and(|imports| !imports.trim().is_empty())
            || line
                .strip_prefix("from ")
                .and_then(|import| import.split_once(" import "))
                .is_some_and(|(module, names)| {
                    !module.trim().is_empty() && !names.trim().is_empty()
                });
        let control_flow = [
            "if",
            "elif",
            "else",
            "for",
            "while",
            "switch",
            "match",
            "try",
            "catch",
            "with",
            "async with",
            "async for",
        ]
        .iter()
        .any(|keyword| {
            declaration_line
                .strip_prefix(keyword)
                .is_some_and(|suffix| {
                    suffix.chars().next().is_some_and(|character| {
                        character.is_ascii_whitespace() || matches!(character, '(' | '{' | ':')
                    })
                })
        }) && (line.contains('(') || line.contains('{') || line.ends_with(':'));
        let statement = line.trim_end_matches(';').trim_end();
        // ponytail: these reviewed UI sentences contain `=` but are not assignments; keep the
        // exception exact so every other valid assignment-shaped line fails closed.
        let known_assignment_shaped_prose = !line.ends_with(';')
            && matches!(
                statement,
                "Status = ready when all checks pass." | "Result = output only after validation."
            );
        let assignment = [
            "+=", "-=", "*=", "/=", "%=", "|=", "&=", "^=", "??=", "<<=", ">>=", "&&=", "||=",
            "**=",
        ]
        .iter()
        .any(|operator| line.contains(operator))
            || statement.ends_with("++")
            || statement.ends_with("--")
            || line.split_once('=').is_some_and(|(lhs, rhs)| {
                let lhs = lhs.trim();
                let rhs = rhs.trim();
                !lhs.is_empty()
                    && !rhs.is_empty()
                    && !lhs.ends_with(['!', '<', '>', '='].as_slice())
                    && !rhs.starts_with(['=', '>'].as_slice())
                    && lhs.chars().all(|character| {
                        character.is_ascii_alphanumeric()
                            || character.is_ascii_whitespace()
                            || matches!(
                                character,
                                '_' | '$'
                                    | '.'
                                    | '['
                                    | ']'
                                    | '('
                                    | ')'
                                    | '{'
                                    | '}'
                                    | '\''
                                    | '"'
                                    | '-'
                                    | ':'
                                    | ','
                            )
                    })
                    && !known_assignment_shaped_prose
            });
        let call_line = declaration_line
            .strip_prefix("await ")
            .unwrap_or(declaration_line);
        let call = (line.ends_with(';') || line.ends_with(')'))
            && call_line.split_once('(').is_some_and(|(callee, rest)| {
                !rest.is_empty()
                    && !callee.is_empty()
                    && callee.chars().all(|character| {
                        character.is_ascii_alphanumeric()
                            || matches!(character, '_' | '$' | '.' | ':' | '!' | '?')
                    })
            });
        let sql = is_sql(line);
        let serialized = (line.starts_with('{') && line.ends_with('}') && line.contains(':'))
            || (line.starts_with('[') && line.ends_with(']') && line.len() > 2);
        let yaml_list_item = line.starts_with("- ");
        let yaml_line = line.strip_prefix("- ").unwrap_or(line);
        let yaml = split_factory_yaml_pair(yaml_line).is_some_and(|(raw_key, value)| {
            let raw_key = raw_key.trim();
            let quoted_key = (raw_key.starts_with('"') && raw_key.ends_with('"'))
                || (raw_key.starts_with('\'') && raw_key.ends_with('\''));
            let key = raw_key.trim_matches(['"', '\''].as_slice());
            let value = value.trim();
            let readiness_status = matches!(
                (
                    key.to_ascii_lowercase().as_str(),
                    value.to_ascii_lowercase().as_str()
                ),
                ("agents" | "skills" | "project", "ready")
            );
            let scalar = matches!(
                value.to_ascii_lowercase().as_str(),
                "true" | "false" | "null" | "~"
            ) || value.parse::<f64>().is_ok()
                || ((value.starts_with('"') && value.ends_with('"'))
                    || (value.starts_with('\'') && value.ends_with('\''))
                    || value.starts_with('[')
                    || value.starts_with('{'));
            let plain_key = key.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
            });
            let config_key = !key.is_empty()
                && !matches!(key.to_ascii_lowercase().as_str(), "http" | "https" | "urn")
                && (quoted_key || plain_key);
            let key_lower = key.to_ascii_lowercase();
            let narrative_key = matches!(
                key_lower.as_str(),
                "risk" | "result" | "note" | "status" | "owner" | "priority" | "severity"
            );
            let technical_key = matches!(
                key_lower.as_str(),
                "database"
                    | "datasource"
                    | "host"
                    | "hostname"
                    | "server"
                    | "port"
                    | "url"
                    | "uri"
                    | "endpoint"
                    | "schema"
                    | "table"
                    | "username"
                    | "repository"
                    | "registry"
                    | "environment"
                    | "config"
                    | "configuration"
            );
            let lowercase_config = key == key_lower
                && (key.contains(['_', '-', '.'].as_slice())
                    || value.is_empty()
                    || !value.chars().any(char::is_whitespace));
            let capital_bare_scalar = key.chars().next().is_some_and(char::is_uppercase)
                && !narrative_key
                && !value.is_empty()
                && !value.chars().any(char::is_whitespace);
            let block_scalar = value.split_whitespace().next().is_some_and(|header| {
                let mut characters = header.chars();
                let indicator = characters.next();
                let suffix = characters.collect::<Vec<_>>();
                matches!(indicator, Some('|' | '>'))
                    && suffix.len() <= 2
                    && suffix
                        .iter()
                        .all(|character| matches!(character, '+' | '-' | '1'..='9'))
                    && suffix
                        .iter()
                        .copied()
                        .filter(|character| matches!(character, '1'..='9'))
                        .count()
                        <= 1
                    && suffix
                        .iter()
                        .copied()
                        .filter(|character| matches!(character, '+' | '-'))
                        .count()
                        <= 1
            });
            !known_yaml_shaped_prose
                && !key.is_empty()
                && (quoted_key || plain_key)
                && !readiness_status
                && config_key
                && (scalar
                    || quoted_key
                    || yaml_list_item
                    || value.starts_with(['&', '*', '!'].as_slice())
                    || technical_key
                    || lowercase_config
                    || capital_bare_scalar
                    || block_scalar)
        });
        let xml = line.starts_with('<') && line.ends_with('>');
        let preprocessor = line.strip_prefix('#').is_some_and(|directive| {
            let directive = directive.trim_start();
            [
                "include",
                "import",
                "define",
                "pragma",
                "if",
                "ifdef",
                "ifndef",
                "elif",
                "else",
                "endif",
                "undef",
                "error",
                "warning",
                "line",
                "nullable",
                "region",
                "endregion",
                "r",
                "load",
                "checksum",
            ]
            .iter()
            .any(|keyword| {
                directive.strip_prefix(keyword).is_some_and(|suffix| {
                    suffix.is_empty()
                        || suffix.chars().next().is_some_and(|character| {
                            character.is_ascii_whitespace() || matches!(character, '<' | '"')
                        })
                })
            })
        });
        let rust_attribute =
            (line.starts_with("#[") || line.starts_with("#![")) && line.ends_with(']');
        let objective_c_declaration = [
            "@interface ",
            "@implementation ",
            "@protocol ",
            "@class ",
            "@property ",
            "@synthesize ",
            "@dynamic ",
            "@compatibility_alias ",
            "@end",
        ]
        .iter()
        .any(|prefix| declaration_line.starts_with(prefix));
        let rust_module = declaration_line
            .strip_prefix("mod ")
            .or_else(|| declaration_line.strip_prefix("pub mod "))
            .is_some_and(|remainder| {
                let (module, terminator) = remainder
                    .split_once(char::is_whitespace)
                    .map_or((remainder.trim_end_matches(';'), ";"), |(module, tail)| {
                        (module, tail.trim_start())
                    });
                !module.is_empty()
                    && module
                        .chars()
                        .all(|character| character.is_ascii_alphanumeric() || character == '_')
                    && (line.ends_with(';') || terminator.starts_with('{'))
            });
        let rust_macro = declaration_line.starts_with("macro_rules!") && line.contains('{');
        let annotation_identifier = |value: &str| {
            !value.is_empty()
                && value.split('.').all(|segment| {
                    segment.chars().next().is_some_and(|character| {
                        character.is_ascii_alphabetic() || matches!(character, '_' | '$')
                    }) && segment.chars().all(|character| {
                        character.is_ascii_alphanumeric() || matches!(character, '_' | '$')
                    })
                })
        };
        let source_annotation = line.strip_prefix('@').is_some_and(|annotation| {
            if let Some((name, arguments)) = annotation.split_once('(') {
                annotation_identifier(name.trim()) && arguments.ends_with(')')
            } else {
                annotation_identifier(annotation)
            }
        });
        let objective_c_control = [
            "@autoreleasepool",
            "@try",
            "@catch",
            "@finally",
            "@synchronized",
        ]
        .iter()
        .any(|prefix| {
            declaration_line
                .strip_prefix(prefix)
                .is_some_and(|suffix| suffix.trim_start().starts_with(['(', '{'].as_slice()))
        });
        let objective_c_throw = declaration_line.starts_with("@throw ") && line.ends_with(';');
        let php_opening =
            declaration_line.starts_with("<?php") || declaration_line.starts_with("<?=");
        let shebang = line.starts_with("#!");
        let directive = preprocessor
            || rust_attribute
            || objective_c_declaration
            || rust_module
            || rust_macro
            || source_annotation
            || objective_c_control
            || objective_c_throw
            || php_opening
            || shebang
            || (line.ends_with(';')
                && (declaration_line.starts_with("using ")
                    || declaration_line.starts_with("global using ")
                    || declaration_line.starts_with("use ")
                    || declaration_line.starts_with("pub use ")
                    || declaration_line.starts_with("@import ")
                    || declaration_line.starts_with("export import ")));
        (declaration
            && (line.contains('(')
                || line.contains('{')
                || line.ends_with(';')
                || line.contains(" = ")))
            || (declaration_line.starts_with("return ") && line.ends_with(';'))
            || control_flow
            || python_import
            || go_package
            || shell_set
            || namespace
            || directive
            || assignment
            || call
            || sql
            || serialized
            || yaml
            || xml
            || line.contains("=>")
            || (line.contains("::") && (line.contains('(') || line.ends_with(';')))
    })
}

fn contains_forbidden_factory_external_content(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    contains_factory_credential(value)
        || contains_factory_unsafe_url(value)
        || ["diff --git", "raw output:", "stdout:", "stderr:", "```"]
            .iter()
            .any(|marker| lower.contains(marker))
        || contains_private_absolute_path(value)
        || contains_factory_source_snippet(value)
        || lower.lines().any(|line| {
            let line = line.trim_start();
            line.starts_with("@@ ") || line.starts_with("--- a/") || line.starts_with("+++ b/")
        })
}

fn factory_url_has_credentials(parsed: &url::Url) -> bool {
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    let Some(decoded_path) = decode_factory_url_component(parsed.path()) else {
        return true;
    };
    let segments = decoded_path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let normalized_segments = segments
        .iter()
        .map(|segment| segment.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let discord_webhook = (segments.first() == Some(&"api")
        && segments.get(1) == Some(&"webhooks")
        && segments.len() >= 4)
        || (segments.first() == Some(&"api")
            && segments
                .get(1)
                .and_then(|segment| segment.strip_prefix('v'))
                .is_some_and(|version| {
                    !version.is_empty()
                        && version.chars().all(|character| character.is_ascii_digit())
                })
            && segments.get(2) == Some(&"webhooks")
            && segments.len() >= 5);
    let telegram_bot = host == "api.telegram.org"
        && (segments
            .first()
            .is_some_and(|segment| segment.starts_with("bot") && segment.len() > 3)
            || (segments.first() == Some(&"file")
                && segments
                    .get(1)
                    .is_some_and(|segment| segment.starts_with("bot") && segment.len() > 3)));
    let webhook_marker = |segment: &str| {
        matches!(
            segment,
            "hook" | "hooks" | "webhook" | "webhooks" | "webhookb2" | "incomingwebhook"
        )
    };
    let host_webhook_context = host
        .split('.')
        .any(|label| label.contains("hook") || label.contains("webhook"));
    let opaque_tail = |tail: &[String]| {
        !tail.is_empty()
            && tail.iter().all(|segment| {
                segment.chars().all(|character| {
                    character.is_ascii_alphanumeric()
                        || matches!(
                            character,
                            '_' | '-'
                                | '.'
                                | '~'
                                | '!'
                                | '$'
                                | '&'
                                | '\''
                                | '('
                                | ')'
                                | '*'
                                | '+'
                                | ','
                                | ';'
                                | '='
                                | ':'
                                | '@'
                        )
                })
            })
            && tail.iter().map(String::len).sum::<usize>() >= 16
    };
    let generic_webhook = normalized_segments
        .iter()
        .enumerate()
        .any(|(index, segment)| {
            webhook_marker(segment) && opaque_tail(&normalized_segments[index + 1..])
        })
        || (host_webhook_context && opaque_tail(&normalized_segments));
    let credential_bearing_webhook = ((host == "hooks.slack.com" || host == "hooks.slack-gov.com")
        && segments.first() == Some(&"services")
        && segments.len() >= 4)
        || ((host == "discord.com" || host == "discordapp.com") && discord_webhook)
        || telegram_bot
        || generic_webhook;
    if credential_bearing_webhook
        || factory_url_component_has_credential(parsed.path())
        || parsed.query_pairs().any(|(key, value)| {
            factory_credential_key(&key)
                || factory_url_component_has_credential(&key)
                || contains_factory_credential(&value)
                || factory_url_component_has_credential(&value)
                || factory_value_looks_like_jwt(&value)
        })
    {
        return true;
    }
    parsed.fragment().is_some_and(|fragment| {
        contains_factory_credential(fragment)
            || factory_url_component_has_credential(fragment)
            || url::form_urlencoded::parse(fragment.as_bytes()).any(|(key, value)| {
                factory_credential_key(&key)
                    || contains_factory_credential(&value)
                    || factory_url_component_has_credential(&value)
                    || factory_value_looks_like_jwt(&value)
            })
    })
}

fn factory_url_has_private_path(parsed: &url::Url) -> bool {
    let path = parsed.path().replace('\\', "/");
    let relative = path.trim_start_matches('/');
    let relative_lower = relative.to_ascii_lowercase();
    let private_roots = ["users/", "home/", "opt/", "srv/", "tmp/", "etc/"];
    path.starts_with("//")
        || private_roots
            .iter()
            .any(|root| relative_lower.starts_with(root))
        || path.split("//").skip(1).any(|suffix| {
            let suffix = suffix.trim_start_matches('/').to_ascii_lowercase();
            private_roots.iter().any(|root| suffix.starts_with(root))
        })
        || (relative.len() >= 3
            && relative.as_bytes()[0].is_ascii_alphabetic()
            && relative.as_bytes()[1] == b':'
            && relative.as_bytes()[2] == b'/')
        || parsed
            .query_pairs()
            .any(|(_, value)| contains_private_absolute_path(&value))
        || parsed.fragment().is_some_and(|fragment| {
            contains_private_absolute_path(fragment)
                || url::form_urlencoded::parse(fragment.as_bytes())
                    .any(|(_, value)| contains_private_absolute_path(&value))
        })
}

fn validate_factory_external_text(value: &str, field: &str, max: usize) -> Result<(), AppError> {
    validate_factory_text(value, field, max)?;
    let mut inspected = value.to_owned();
    loop {
        if contains_forbidden_factory_external_content(&inspected) {
            return Err(invalid(format!(
                "{field} must not contain credentials, raw output, diffs, repository content, or private absolute paths"
            )));
        }
        let Some(decoded) = decode_factory_percent_encoding_for_inspection(&inspected)
            .map_err(|_| invalid(format!("{field} contains invalid encoded text")))?
        else {
            break;
        };
        inspected = decoded;
    }
    Ok(())
}

fn decode_factory_percent_encoding_for_inspection(value: &str) -> Result<Option<String>, ()> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut changed = false;
    let mut index = 0;
    while index < bytes.len() {
        let nibble = |byte: u8| match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        };
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && nibble(bytes[index + 1]).is_some()
            && nibble(bytes[index + 2]).is_some()
        {
            decoded
                .push(nibble(bytes[index + 1]).unwrap() * 16 + nibble(bytes[index + 2]).unwrap());
            index += 3;
            changed = true;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    if !changed {
        return Ok(None);
    }
    String::from_utf8(decoded).map(Some).map_err(|_| ())
}

fn decode_factory_percent_encoding(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return None;
            }
            let nibble = |byte: u8| match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                b'A'..=b'F' => Some(byte - b'A' + 10),
                _ => None,
            };
            decoded.push(nibble(bytes[index + 1])? * 16 + nibble(bytes[index + 2])?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn validate_factory_external_url_text(value: &str, field: &str) -> Result<(), AppError> {
    validate_factory_external_text(value, field, MAX_FACTORY_REFERENCE)?;
    let mut decoded = value.to_owned();
    loop {
        if url::Url::parse(&decoded)
            .ok()
            .is_some_and(|parsed| factory_url_has_private_path(&parsed))
        {
            return Err(invalid(format!(
                "{field} must not encode credentials, raw content, or private absolute paths"
            )));
        }
        let next = decode_factory_percent_encoding(&decoded)
            .ok_or_else(|| invalid(format!("{field} has invalid percent encoding")))?;
        if next == decoded {
            break;
        }
        if contains_forbidden_factory_external_content(&next)
            || url::Url::parse(&next)
                .ok()
                .is_some_and(|parsed| factory_url_has_private_path(&parsed))
        {
            return Err(invalid(format!(
                "{field} must not encode credentials, raw content, or private absolute paths"
            )));
        }
        decoded = next;
    }
    Ok(())
}

fn validate_factory_external_optional_text(
    value: Option<&str>,
    field: &str,
    max: usize,
) -> Result<(), AppError> {
    if let Some(value) = value {
        validate_factory_external_text(value, field, max)?;
    }
    Ok(())
}

fn validate_factory_external_list(
    values: &[String],
    field: &str,
    required: bool,
) -> Result<(), AppError> {
    if values.len() > MAX_FACTORY_ITEMS || (required && values.is_empty()) {
        return Err(invalid(format!("{field} count is invalid")));
    }
    for value in values {
        validate_factory_external_text(value, field, MAX_FACTORY_ITEM_TEXT)?;
    }
    Ok(())
}

fn validate_factory_list(values: &[String], field: &str, required: bool) -> Result<(), AppError> {
    if values.len() > MAX_FACTORY_ITEMS || (required && values.is_empty()) {
        return Err(invalid(format!("{field} count is invalid")));
    }
    for value in values {
        validate_factory_text(value, field, MAX_FACTORY_ITEM_TEXT)?;
    }
    Ok(())
}

fn parse_factory_timestamp(
    value: &str,
    field: &str,
) -> Result<chrono::DateTime<chrono::Utc>, AppError> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|value| value.to_utc())
        .map_err(|_| invalid(format!("{field} is not an RFC 3339 timestamp")))
}

fn factory_timestamp(value: chrono::DateTime<chrono::Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn factory_digest(value: &impl Serialize) -> Result<String, AppError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| invalid(format!("serialize Factory digest input: {error}")))?;
    Ok(crate::render::sha256_hex(&bytes))
}

fn validate_factory_digest(value: &str, field: &str) -> Result<(), AppError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(format!("{field} is not a SHA-256 digest")));
    }
    Ok(())
}

fn validate_factory_contract(
    contract: &FactoryWorkContract,
    snapshot: &ExpertRunCreate,
) -> Result<(), AppError> {
    validate_factory_external_text(
        &contract.ticket_reference,
        "Factory ticketReference",
        MAX_FACTORY_ITEM_TEXT,
    )?;
    validate_factory_external_text(&contract.title, "Factory title", MAX_FACTORY_TITLE)?;
    validate_factory_external_text(&contract.objective, "Factory objective", MAX_TEXT)?;
    validate_factory_external_list(
        &contract.acceptance_criteria,
        "Factory acceptanceCriteria",
        true,
    )?;
    validate_factory_external_list(&contract.non_goals, "Factory nonGoals", false)?;
    validate_factory_optional_text(
        contract.playbook.as_deref(),
        "Factory playbook",
        MAX_FACTORY_ITEM_TEXT,
    )?;
    validate_factory_optional_text(
        contract.runbook.as_deref(),
        "Factory runbook",
        MAX_FACTORY_ITEM_TEXT,
    )?;
    if contract.playbook.is_none() && contract.runbook.is_none() {
        return Err(invalid("Factory work requires a playbook or runbook"));
    }
    if let Some(revision) = contract.workspace_pack_revision.as_deref() {
        validate_factory_digest(revision, "Factory workspacePackRevision")?;
    }
    validate_factory_text(
        &contract.readiness.evidence_revision,
        "Factory readiness evidenceRevision",
        MAX_FACTORY_ITEM_TEXT,
    )?;
    validate_factory_external_list(
        &contract.readiness.summary,
        "Factory readiness summary",
        false,
    )?;
    parse_factory_timestamp(
        &contract.readiness.checked_at,
        "Factory readiness checkedAt",
    )?;
    validate_contract(&contract.quality_contract)?;
    for check in &contract.quality_contract.checks {
        validate_factory_external_text(&check.name, "Factory quality check name", 160)?;
        validate_factory_external_text(&check.kind, "Factory quality check kind", 160)?;
    }
    if contract.readiness.overall != FactoryReadinessOverall::Ready {
        return Err(invalid("Factory project readiness is not ready"));
    }
    if contract.project_path != snapshot.project_path
        || contract.expert_id != snapshot.expert_id
        || contract.expert_version != snapshot.expert_version
        || contract.runbook != snapshot.runbook
        || contract.quality_contract != snapshot.contract
    {
        return Err(invalid(
            "Factory work contract does not match the Expert run",
        ));
    }
    Ok(())
}

fn validate_factory_workflow(
    workflow: &FactoryWorkflow,
    snapshot: &ExpertRunCreate,
) -> Result<(), AppError> {
    validate_factory_contract(&workflow.work_contract, snapshot)?;
    validate_factory_digest(
        &workflow.work_contract_revision,
        "Factory workContractRevision",
    )?;
    if factory_digest(&workflow.work_contract)? != workflow.work_contract_revision {
        return Err(invalid("Factory work contract revision is invalid"));
    }
    parse_factory_timestamp(&workflow.created_at, "Factory createdAt")?;
    parse_factory_timestamp(
        &workflow.preflight_completed_at,
        "Factory preflightCompletedAt",
    )?;
    if workflow.revision == 0
        || workflow.attempts.len() > MAX_FACTORY_ATTEMPTS as usize
        || workflow.prior_claims.len() > MAX_FACTORY_CLAIMS
        || workflow.blockers.len() > MAX_FACTORY_BLOCKERS
        || workflow.artifacts.len() > MAX_FACTORY_ARTIFACTS
        || workflow.evidence.len() > MAX_FACTORY_EVIDENCE
        || workflow.idempotency.len() > MAX_FACTORY_IDEMPOTENCY
        || (workflow.phase == FactoryPhase::Completed) != workflow.terminal.is_some()
    {
        return Err(invalid(
            "Factory workflow bounds or terminal state are invalid",
        ));
    }
    Ok(())
}

fn validate_factory_persisted_trust(
    workflow: &FactoryWorkflow,
    run: &ExpertRun,
) -> Result<(), AppError> {
    if let Some(plan) = &workflow.plan {
        let input = FactoryPlanInput {
            content: plan.content.clone(),
            citations: plan.citations.clone(),
            declared_checks: plan.declared_checks.clone(),
            risks: plan.risks.clone(),
            known_limitations: plan.known_limitations.clone(),
            base_commit: plan.base_commit.clone(),
        };
        validate_factory_plan_input(&input, &run.snapshot.contract)?;
        validate_factory_digest(&plan.revision, "Factory plan revision")?;
        validate_factory_text(
            &plan.submitted_by,
            "Factory plan submittedBy",
            MAX_FACTORY_ITEM_TEXT,
        )?;
        parse_factory_timestamp(&plan.submitted_at, "Factory plan submittedAt")?;
        if factory_plan_revision(&workflow.work_contract_revision, &input)? != plan.revision {
            return Err(invalid("Factory persisted plan revision is invalid"));
        }
    } else if workflow.plan_approval.is_some() {
        return Err(invalid("Factory plan approval has no plan"));
    }

    if let Some(approval) = &workflow.plan_approval {
        validate_factory_digest(&approval.plan_revision, "Factory approved plan revision")?;
        validate_factory_commit(&approval.base_commit, "Factory approved baseCommit")?;
        parse_factory_timestamp(&approval.approved_at, "Factory plan approvedAt")?;
        if !workflow.plan.as_ref().is_some_and(|plan| {
            plan.revision == approval.plan_revision && plan.base_commit == approval.base_commit
        }) {
            return Err(invalid(
                "Factory persisted plan approval binding is invalid",
            ));
        }
    }

    for (index, attempt) in workflow.attempts.iter().enumerate() {
        if attempt.number as usize != index + 1 || attempt.number > MAX_FACTORY_ATTEMPTS {
            return Err(invalid("Factory persisted attempt sequence is invalid"));
        }
        let started_at = parse_factory_timestamp(&attempt.started_at, "Factory attempt startedAt")?;
        let ended_at = attempt
            .ended_at
            .as_deref()
            .map(|value| parse_factory_timestamp(value, "Factory attempt endedAt"))
            .transpose()?;
        if ended_at.is_some() != attempt.result.is_some()
            || ended_at.is_some_and(|ended_at| ended_at < started_at)
            || attempt.head_commit.is_some() != attempt.builder_identity.is_some()
            || attempt.result.as_deref().is_some_and(|result| {
                !matches!(
                    result,
                    "validationRework"
                        | "reviewRework"
                        | "reviewEvidenceRework"
                        | "deliveryEvidenceRework"
                )
            })
        {
            return Err(invalid("Factory persisted attempt binding is invalid"));
        }
        if let Some(head) = attempt.head_commit.as_deref() {
            validate_factory_commit(head, "Factory attempt headCommit")?;
        }
        if let Some(builder) = attempt.builder_identity.as_deref() {
            validate_factory_text(
                builder,
                "Factory attempt builderIdentity",
                MAX_FACTORY_ITEM_TEXT,
            )?;
        }
        if index + 1 < workflow.attempts.len() && ended_at.is_none() {
            return Err(invalid("Factory prior attempt is not complete"));
        }
    }
    let attempt_exhausted = workflow
        .terminal
        .as_ref()
        .is_some_and(|terminal| terminal.outcome == FactoryTerminalOutcome::AttemptExhausted);
    if workflow
        .attempts
        .last()
        .is_some_and(|attempt| attempt.ended_at.is_some())
        != attempt_exhausted
        || (workflow.attempts.is_empty() && workflow.plan_approval.is_some())
        || (!workflow.attempts.is_empty() && workflow.plan_approval.is_none())
    {
        return Err(invalid("Factory persisted attempt lifecycle is invalid"));
    }

    let mut claim_ids = HashSet::new();
    let mut claim_generations = HashSet::new();
    for (claim, current) in workflow
        .current_claim
        .iter()
        .map(|claim| (claim, true))
        .chain(workflow.prior_claims.iter().map(|claim| (claim, false)))
    {
        if uuid::Uuid::parse_str(&claim.id).is_err()
            || !claim_ids.insert(claim.id.as_str())
            || claim.generation == 0
            || !claim_generations.insert(claim.generation)
            || !claim.phase.worker_claimable()
            || claim.run_revision == 0
            || claim.run_revision > workflow.revision
        {
            return Err(invalid("Factory persisted claim identity is invalid"));
        }
        validate_factory_external_text(
            &claim.idempotency_key,
            "Factory claim idempotencyKey",
            MAX_FACTORY_ITEM_TEXT,
        )?;
        validate_factory_text(
            &claim.worker_identity,
            "Factory claim workerIdentity",
            MAX_FACTORY_ITEM_TEXT,
        )?;
        let claimed_at = parse_factory_timestamp(&claim.claimed_at, "Factory claim claimedAt")?;
        let renewed_at =
            parse_factory_timestamp(&claim.last_renewed_at, "Factory claim lastRenewedAt")?;
        let expires_at = parse_factory_timestamp(&claim.expires_at, "Factory claim expiresAt")?;
        if renewed_at < claimed_at || expires_at <= renewed_at {
            return Err(invalid("Factory persisted claim lease is invalid"));
        }
        match (current, claim.released_at.as_deref()) {
            (true, None)
                if claim.phase == workflow.phase && claim.run_revision == workflow.revision => {}
            (false, Some(released_at)) => {
                if parse_factory_timestamp(released_at, "Factory claim releasedAt")? < claimed_at {
                    return Err(invalid("Factory persisted claim release is invalid"));
                }
            }
            _ => return Err(invalid("Factory persisted claim ownership is invalid")),
        }
    }

    let retained_claim = |claim_id: &str, generation: u64| {
        workflow
            .current_claim
            .iter()
            .chain(workflow.prior_claims.iter())
            .chain(
                workflow
                    .idempotency
                    .iter()
                    .filter_map(|record| record.claim_result.as_ref()),
            )
            .find(|claim| claim.id == claim_id && claim.generation == generation)
    };
    let valid_claim_lineage = |claim_id: &str, generation: u64, phase: FactoryPhase| {
        uuid::Uuid::parse_str(claim_id).is_ok()
            && retained_claim(claim_id, generation).is_some_and(|claim| claim.phase == phase)
    };
    let mut blocker_ids = HashSet::new();
    for blocker in &workflow.blockers {
        if uuid::Uuid::parse_str(&blocker.id).is_err()
            || !blocker_ids.insert(blocker.id.as_str())
            || blocker.run_id != run.id
            || !valid_claim_lineage(&blocker.claim_id, blocker.claim_generation, blocker.phase)
            || !blocker.phase.worker_claimable()
            || (blocker.phase == FactoryPhase::Planning && blocker.attempt != 0)
            || (blocker.phase != FactoryPhase::Planning
                && !workflow
                    .attempts
                    .iter()
                    .any(|attempt| attempt.number == blocker.attempt))
        {
            return Err(invalid("Factory persisted blocker binding is invalid"));
        }
        validate_factory_external_text(
            &blocker.idempotency_key,
            "Factory blocker idempotencyKey",
            MAX_FACTORY_ITEM_TEXT,
        )?;
        validate_factory_external_text(
            &blocker.kind,
            "Factory blocker kind",
            MAX_FACTORY_ITEM_TEXT,
        )?;
        validate_factory_external_text(&blocker.summary, "Factory blocker summary", MAX_TEXT)?;
        validate_factory_text(
            &blocker.reported_by,
            "Factory blocker reportedBy",
            MAX_FACTORY_ITEM_TEXT,
        )?;
        let reported_at =
            parse_factory_timestamp(&blocker.reported_at, "Factory blocker reportedAt")?;
        let resolved_at = blocker
            .resolved_at
            .as_deref()
            .map(|value| parse_factory_timestamp(value, "Factory blocker resolvedAt"))
            .transpose()?;
        if resolved_at.is_some_and(|resolved_at| resolved_at < reported_at)
            || (resolved_at.is_none()
                && workflow.phase != FactoryPhase::Completed
                && blocker.phase != workflow.phase)
        {
            return Err(invalid("Factory persisted blocker lifecycle is invalid"));
        }
    }
    let approval = workflow.plan_approval.as_ref();
    let artifact_ids = workflow
        .artifacts
        .iter()
        .map(|artifact| artifact.id.as_str())
        .collect::<HashSet<_>>();
    if artifact_ids.len() != workflow.artifacts.len() {
        return Err(invalid("Factory persisted artifact identity is invalid"));
    }
    for artifact in &workflow.artifacts {
        if uuid::Uuid::parse_str(&artifact.id).is_err()
            || artifact.run_id != run.id
            || !valid_claim_lineage(
                &artifact.claim_id,
                artifact.claim_generation,
                artifact.phase,
            )
            || artifact.work_contract_revision != workflow.work_contract_revision
            || !artifact.phase.worker_claimable()
            || artifact.byte_size == 0
            || artifact.byte_size > MAX_FACTORY_ARTIFACT_BYTES
        {
            return Err(invalid("Factory persisted artifact binding is invalid"));
        }
        validate_factory_external_text(
            &artifact.idempotency_key,
            "Factory artifact idempotencyKey",
            MAX_FACTORY_ITEM_TEXT,
        )?;
        validate_factory_external_text(
            &artifact.kind,
            "Factory artifact kind",
            MAX_FACTORY_ITEM_TEXT,
        )?;
        validate_factory_external_text(
            &artifact.label,
            "Factory artifact label",
            MAX_FACTORY_ITEM_TEXT,
        )?;
        validate_factory_artifact_reference(&artifact.reference)?;
        validate_factory_digest(&artifact.digest, "Factory artifact digest")?;
        validate_factory_external_text(&artifact.summary, "Factory artifact summary", MAX_TEXT)?;
        parse_factory_timestamp(&artifact.submitted_at, "Factory artifact submittedAt")?;
        if artifact.phase == FactoryPhase::Planning {
            if artifact.attempt != 0
                || artifact.approved_plan_revision.is_some()
                || artifact.base_commit.is_some()
                || artifact.head_commit.is_some()
            {
                return Err(invalid(
                    "Factory persisted planning artifact binding is invalid",
                ));
            }
        } else if artifact.attempt == 0
            || artifact.approved_plan_revision.as_deref()
                != approval.map(|approval| approval.plan_revision.as_str())
            || artifact.base_commit.as_deref()
                != approval.map(|approval| approval.base_commit.as_str())
        {
            return Err(invalid(
                "Factory persisted artifact plan binding is invalid",
            ));
        }
        if artifact.phase != FactoryPhase::Planning {
            let attempt = workflow
                .attempts
                .iter()
                .find(|attempt| attempt.number == artifact.attempt)
                .ok_or_else(|| invalid("Factory persisted artifact attempt is invalid"))?;
            let expected_head = if artifact.phase == FactoryPhase::Build {
                None
            } else {
                attempt.head_commit.as_deref()
            };
            if artifact.head_commit.as_deref() != expected_head {
                return Err(invalid(
                    "Factory persisted artifact head binding is invalid",
                ));
            }
        }
        if let Some(head) = artifact.head_commit.as_deref() {
            validate_factory_commit(head, "Factory artifact headCommit")?;
        }
    }

    let mut evidence_ids = HashSet::new();
    for evidence in &workflow.evidence {
        if uuid::Uuid::parse_str(&evidence.id).is_err()
            || !evidence_ids.insert(evidence.id.as_str())
            || evidence.run_id != run.id
            || !valid_claim_lineage(
                &evidence.claim_id,
                evidence.claim_generation,
                evidence.phase,
            )
            || evidence.work_contract_revision != workflow.work_contract_revision
            || evidence.attempt == 0
            || !matches!(
                evidence.phase,
                FactoryPhase::Build
                    | FactoryPhase::Validation
                    | FactoryPhase::IndependentReview
                    | FactoryPhase::Delivery
            )
            || evidence.approved_plan_revision.as_deref()
                != approval.map(|approval| approval.plan_revision.as_str())
            || evidence.base_commit.as_deref()
                != approval.map(|approval| approval.base_commit.as_str())
        {
            return Err(invalid("Factory persisted evidence binding is invalid"));
        }
        validate_factory_external_text(
            &evidence.idempotency_key,
            "Factory evidence idempotencyKey",
            MAX_FACTORY_ITEM_TEXT,
        )?;
        validate_factory_evidence_input(&FactoryEvidenceInput {
            check_name: evidence.check_name.clone(),
            result: evidence.result.clone(),
            command_label: evidence.command_label.clone(),
            exit_code: evidence.exit_code,
            summary: evidence.summary.clone(),
            artifact_ids: evidence.artifact_ids.clone(),
        })?;
        if !run
            .snapshot
            .contract
            .checks
            .iter()
            .any(|check| check.name == evidence.check_name)
            || evidence.artifact_ids.iter().any(|artifact_id| {
                workflow
                    .artifacts
                    .iter()
                    .find(|artifact| artifact.id == *artifact_id)
                    .is_none_or(|artifact| {
                        artifact.phase != evidence.phase
                            || artifact.attempt != evidence.attempt
                            || artifact.claim_id != evidence.claim_id
                            || artifact.claim_generation != evidence.claim_generation
                            || artifact.work_contract_revision != evidence.work_contract_revision
                            || artifact.approved_plan_revision != evidence.approved_plan_revision
                            || artifact.base_commit != evidence.base_commit
                            || artifact.head_commit != evidence.head_commit
                    })
            })
        {
            return Err(invalid("Factory persisted evidence references are invalid"));
        }
        if let Some(head) = evidence.head_commit.as_deref() {
            validate_factory_commit(head, "Factory evidence headCommit")?;
        }
        let attempt = workflow
            .attempts
            .iter()
            .find(|attempt| attempt.number == evidence.attempt)
            .ok_or_else(|| invalid("Factory persisted evidence attempt is invalid"))?;
        let expected_head = if evidence.phase == FactoryPhase::Build {
            None
        } else {
            attempt.head_commit.as_deref()
        };
        if evidence.head_commit.as_deref() != expected_head {
            return Err(invalid(
                "Factory persisted evidence head binding is invalid",
            ));
        }
        parse_factory_timestamp(&evidence.submitted_at, "Factory evidence submittedAt")?;
    }

    if let Some(validation) = &workflow.validation {
        validate_factory_commit(&validation.head_commit, "Factory validation headCommit")?;
        parse_factory_timestamp(&validation.validated_at, "Factory validation validatedAt")?;
        if validation.phase != FactoryPhase::Validation
            || !valid_claim_lineage(
                &validation.claim_id,
                validation.claim_generation,
                validation.phase,
            )
            || validation.attempt != current_factory_attempt(workflow)
            || current_factory_head(workflow) != Some(validation.head_commit.as_str())
            || validation.check_names.iter().collect::<HashSet<_>>().len()
                != validation.check_names.len()
        {
            return Err(invalid("Factory persisted validation binding is invalid"));
        }
        let (check_names, failed) = factory_required_evidence(
            workflow,
            &run.snapshot.contract,
            validation.phase,
            &validation.claim_id,
            validation.claim_generation,
        )?;
        if failed || check_names != validation.check_names {
            return Err(invalid("Factory persisted validation evidence is invalid"));
        }
    } else if matches!(
        workflow.phase,
        FactoryPhase::IndependentReview
            | FactoryPhase::Delivery
            | FactoryPhase::AwaitingFinalApproval
    ) {
        return Err(invalid("Factory persisted validation snapshot is missing"));
    }

    if let Some(review) = &workflow.review {
        validate_factory_review_input(&FactoryReviewInput {
            verdict: review.verdict,
            summary: review.summary.clone(),
            findings: review.findings.clone(),
        })?;
        validate_factory_commit(&review.head_commit, "Factory review headCommit")?;
        validate_factory_text(
            &review.reviewer_identity,
            "Factory review reviewerIdentity",
            MAX_FACTORY_ITEM_TEXT,
        )?;
        parse_factory_timestamp(&review.submitted_at, "Factory review submittedAt")?;
        let attempt = workflow
            .attempts
            .iter()
            .find(|attempt| attempt.number == review.attempt)
            .ok_or_else(|| invalid("Factory persisted review attempt is invalid"))?;
        if review.phase != FactoryPhase::IndependentReview
            || !valid_claim_lineage(&review.claim_id, review.claim_generation, review.phase)
            || review.attempt != current_factory_attempt(workflow)
            || attempt.head_commit.as_deref() != Some(review.head_commit.as_str())
            || attempt.builder_identity.as_deref() == Some(review.reviewer_identity.as_str())
            || (review.verdict == FactoryReviewVerdict::Rework && !attempt_exhausted)
            || (review.verdict == FactoryReviewVerdict::Pass
                && !matches!(
                    workflow.phase,
                    FactoryPhase::Delivery
                        | FactoryPhase::AwaitingFinalApproval
                        | FactoryPhase::Completed
                ))
        {
            return Err(invalid("Factory persisted review binding is invalid"));
        }
    }

    let independent_review_waived = workflow
        .human_waivers
        .iter()
        .any(|waiver| waiver.kind == "independentReview");
    if let Some(delivery) = &workflow.delivery {
        validate_factory_https(&delivery.reference, "Factory delivery reference")?;
        validate_factory_commit(&delivery.head_commit, "Factory delivery headCommit")?;
        validate_factory_external_text(
            &delivery.evidence_summary,
            "Factory delivery evidenceSummary",
            MAX_TEXT,
        )?;
        validate_factory_external_list(
            &delivery.known_limitations,
            "Factory delivery knownLimitations",
            true,
        )?;
        parse_factory_timestamp(&delivery.submitted_at, "Factory delivery submittedAt")?;
        let review_passes = workflow.review.as_ref().is_some_and(|review| {
            review.verdict == FactoryReviewVerdict::Pass
                && review.attempt == delivery.attempt
                && review.head_commit == delivery.head_commit
        });
        if delivery.phase != FactoryPhase::Delivery
            || !valid_claim_lineage(
                &delivery.claim_id,
                delivery.claim_generation,
                delivery.phase,
            )
            || delivery.attempt != current_factory_attempt(workflow)
            || current_factory_head(workflow) != Some(delivery.head_commit.as_str())
            || (!review_passes && !independent_review_waived)
            || !matches!(
                workflow.phase,
                FactoryPhase::AwaitingFinalApproval | FactoryPhase::Completed
            )
        {
            return Err(invalid("Factory persisted delivery binding is invalid"));
        }
    } else if workflow.phase == FactoryPhase::AwaitingFinalApproval {
        return Err(invalid("Factory persisted delivery is missing"));
    }

    let mut waiver_bindings = HashSet::new();
    for waiver in &workflow.human_waivers {
        validate_factory_external_text(&waiver.reason, "Factory waiver reason", MAX_TEXT)?;
        parse_factory_timestamp(&waiver.created_at, "Factory waiver createdAt")?;
        let binding = (waiver.kind.as_str(), waiver.check_name.as_deref());
        if !waiver_bindings.insert(binding)
            || match waiver.kind.as_str() {
                "independentReview" => waiver.check_name.is_some(),
                "qualityCheck" => {
                    !workflow.terminal.as_ref().is_some_and(|terminal| {
                        terminal.outcome == FactoryTerminalOutcome::Accepted
                    }) || waiver.check_name.as_deref().is_none_or(|check_name| {
                        !run.snapshot
                            .contract
                            .checks
                            .iter()
                            .any(|check| check.required && check.name == check_name)
                    })
                }
                _ => true,
            }
        {
            return Err(invalid("Factory persisted waiver binding is invalid"));
        }
    }

    if let Some(proposal) = &workflow.improvement_proposal {
        validate_factory_improvement(proposal)?;
        if workflow.delivery.is_none() {
            return Err(invalid("Factory persisted improvement has no delivery"));
        }
    }

    match (&workflow.terminal, run.state) {
        (None, ExpertRunState::InProgress) if run.ended_at.is_none() => {}
        (Some(terminal), state) => {
            let decided_at =
                parse_factory_timestamp(&terminal.decided_at, "Factory terminal decidedAt")?;
            validate_factory_external_optional_text(
                terminal.safe_detail.as_deref(),
                "Factory terminal safeDetail",
                MAX_FACTORY_ITEM_TEXT,
            )?;
            let expected_state = match terminal.outcome {
                FactoryTerminalOutcome::Accepted => ExpertRunState::Accepted,
                FactoryTerminalOutcome::Rework | FactoryTerminalOutcome::AttemptExhausted => {
                    ExpertRunState::Rework
                }
                FactoryTerminalOutcome::Rejected => ExpertRunState::Rejected,
                FactoryTerminalOutcome::Cancelled => ExpertRunState::Cancelled,
            };
            let ended_at = parse_factory_timestamp(
                run.ended_at.as_deref().unwrap_or_default(),
                "Factory run endedAt",
            )?;
            if state != expected_state || ended_at < decided_at || workflow.current_claim.is_some()
            {
                return Err(invalid(
                    "Factory terminal outcome and run state are inconsistent",
                ));
            }
            match terminal.outcome {
                FactoryTerminalOutcome::Accepted
                | FactoryTerminalOutcome::Rework
                | FactoryTerminalOutcome::Rejected => {
                    let approval = workflow.plan_approval.as_ref();
                    let validation = workflow.validation.as_ref();
                    let delivery = workflow.delivery.as_ref();
                    if approval.is_none()
                        || validation.is_none()
                        || delivery.is_none()
                        || validation.is_some_and(|validation| {
                            delivery.is_none_or(|delivery| {
                                validation.attempt != delivery.attempt
                                    || validation.head_commit != delivery.head_commit
                            })
                        })
                    {
                        return Err(invalid(
                            "Factory final terminal decision bindings are incomplete",
                        ));
                    }
                    let review_passes = workflow.review.as_ref().is_some_and(|review| {
                        review.verdict == FactoryReviewVerdict::Pass
                            && delivery.is_some_and(|delivery| {
                                review.attempt == delivery.attempt
                                    && review.head_commit == delivery.head_commit
                            })
                    });
                    if !review_passes && !independent_review_waived {
                        return Err(invalid(
                            "Factory final terminal review binding is incomplete",
                        ));
                    }
                    let quality_waivers = workflow
                        .human_waivers
                        .iter()
                        .filter(|waiver| waiver.kind == "qualityCheck")
                        .filter_map(|waiver| waiver.check_name.clone())
                        .collect::<HashSet<_>>();
                    if terminal.outcome == FactoryTerminalOutcome::Accepted {
                        if quality_waivers
                            != factory_missing_required_checks(workflow, &run.snapshot.contract)
                        {
                            return Err(invalid(
                                "Factory accepted terminal quality bindings are incomplete",
                            ));
                        }
                    } else if !quality_waivers.is_empty() {
                        return Err(invalid(
                            "Factory non-accepted terminal decision has quality waivers",
                        ));
                    }
                }
                FactoryTerminalOutcome::AttemptExhausted => {
                    if workflow.attempts.len() != MAX_FACTORY_ATTEMPTS as usize
                        || workflow.delivery.is_some()
                        || workflow.improvement_proposal.is_some()
                        || !workflow.human_waivers.is_empty()
                        || workflow
                            .review
                            .as_ref()
                            .is_some_and(|review| review.verdict != FactoryReviewVerdict::Rework)
                    {
                        return Err(invalid(
                            "Factory exhausted terminal attempt binding is inconsistent",
                        ));
                    }
                }
                FactoryTerminalOutcome::Cancelled => {}
            }
        }
        _ => return Err(invalid("Factory non-terminal run state is inconsistent")),
    }

    let mut idempotency_keys = HashSet::new();
    for record in &workflow.idempotency {
        validate_factory_external_text(
            &record.key,
            "Factory idempotency key",
            MAX_FACTORY_ITEM_TEXT,
        )?;
        validate_factory_digest(&record.request_digest, "Factory idempotency requestDigest")?;
        if !idempotency_keys.insert(record.key.as_str())
            || record.run_id != run.id
            || uuid::Uuid::parse_str(&record.result_id).is_err()
            || record.result_revision == 0
            || record.result_revision > workflow.revision
        {
            return Err(invalid("Factory persisted idempotency record is invalid"));
        }
        parse_factory_timestamp(&record.created_at, "Factory idempotency createdAt")?;
        if let Some(claim) = record.claim_result.as_ref() {
            validate_factory_external_text(
                &claim.idempotency_key,
                "Factory idempotent claim key",
                MAX_FACTORY_ITEM_TEXT,
            )?;
            validate_factory_text(
                &claim.worker_identity,
                "Factory idempotent claim workerIdentity",
                MAX_FACTORY_ITEM_TEXT,
            )?;
            let claimed_at =
                parse_factory_timestamp(&claim.claimed_at, "Factory idempotent claim claimedAt")?;
            let renewed_at = parse_factory_timestamp(
                &claim.last_renewed_at,
                "Factory idempotent claim lastRenewedAt",
            )?;
            let expires_at =
                parse_factory_timestamp(&claim.expires_at, "Factory idempotent claim expiresAt")?;
            if claim.id != record.result_id
                || claim.idempotency_key != record.key
                || claim.phase != record.result_phase
                || claim.run_revision != record.result_revision
                || claim.generation == 0
                || renewed_at < claimed_at
                || expires_at <= renewed_at
            {
                return Err(invalid(
                    "Factory persisted idempotent claim result is invalid",
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn prepare_factory_workflow(
    snapshot: &ExpertRunCreate,
    mut create: FactoryRunCreate,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<FactoryWorkflow, AppError> {
    let checked_at =
        parse_factory_timestamp(&create.readiness.checked_at, "Factory readiness checkedAt")?;
    let age = now.signed_duration_since(checked_at).num_seconds();
    if !(0..=MAX_FACTORY_READINESS_AGE_SECONDS).contains(&age) {
        return Err(invalid("Factory project readiness evidence is stale"));
    }
    create.readiness.checked_at = factory_timestamp(checked_at);
    let contract = FactoryWorkContract {
        ticket_reference: create.ticket_reference,
        title: create.title,
        objective: create.objective,
        acceptance_criteria: create.acceptance_criteria,
        non_goals: create.non_goals,
        project_path: snapshot.project_path.clone(),
        expert_id: snapshot.expert_id.clone(),
        expert_version: snapshot.expert_version,
        playbook: create.playbook,
        runbook: snapshot.runbook.clone(),
        workspace_pack_revision: create.workspace_pack_revision,
        quality_contract: snapshot.contract.clone(),
        risk: create.risk,
        readiness: create.readiness,
    };
    validate_factory_contract(&contract, snapshot)?;
    let work_contract_revision = factory_digest(&contract)?;
    let created_at = factory_timestamp(now);
    let workflow = FactoryWorkflow {
        work_contract: contract,
        work_contract_revision,
        phase: FactoryPhase::Planning,
        revision: 1,
        created_at: created_at.clone(),
        preflight_completed_at: created_at,
        attempts: Vec::new(),
        plan: None,
        plan_approval: None,
        current_claim: None,
        prior_claims: Vec::new(),
        blockers: Vec::new(),
        artifacts: Vec::new(),
        evidence: Vec::new(),
        validation: None,
        review: None,
        delivery: None,
        human_waivers: Vec::new(),
        terminal: None,
        improvement_proposal: None,
        idempotency: Vec::new(),
    };
    validate_factory_workflow(&workflow, snapshot)?;
    Ok(workflow)
}

pub(crate) fn validate_contract(contract: &QualityContract) -> Result<(), AppError> {
    if contract.version == 0 || contract.checks.len() > 64 {
        return Err(invalid(
            "quality contract version or check count is invalid",
        ));
    }
    let mut names = HashSet::new();
    for check in &contract.checks {
        if check.name.trim().is_empty()
            || check.name.len() > 160
            || check.kind.trim().is_empty()
            || check.kind.len() > 160
            || !matches!(
                check.evidence_mode.as_str(),
                "clientReported" | "userConfirmed"
            )
            || !names.insert(check.name.as_str())
        {
            return Err(invalid(
                "quality contract contains an invalid or duplicate check",
            ));
        }
    }
    Ok(())
}

fn scoped<'a>(
    runs: &'a mut [ExpertRun],
    id: &str,
    client: &str,
    project: &str,
) -> Result<&'a mut ExpertRun, AppError> {
    runs.iter_mut()
        .find(|run| {
            run.id == id && run.snapshot.client == client && run.snapshot.project_path == project
        })
        .ok_or_else(|| invalid("Expert run does not exist"))
}

pub async fn create_run(state: &AppState, create: ExpertRunCreate) -> Result<ExpertRun, AppError> {
    create_run_record_with_id(
        state,
        &uuid::Uuid::new_v4().to_string(),
        create,
        None,
        chrono::Utc::now(),
    )
    .await
}

pub(crate) async fn create_run_with_id(
    state: &AppState,
    id: &str,
    create: ExpertRunCreate,
) -> Result<ExpertRun, AppError> {
    create_run_record_with_id(state, id, create, None, chrono::Utc::now()).await
}

#[cfg(test)]
pub async fn create_factory_run(
    state: &AppState,
    create: ExpertRunCreate,
    factory: FactoryRunCreate,
) -> Result<ExpertRun, AppError> {
    create_factory_run_with_id(state, &uuid::Uuid::new_v4().to_string(), create, factory).await
}

#[cfg(test)]
pub(crate) async fn create_factory_run_with_id(
    state: &AppState,
    id: &str,
    create: ExpertRunCreate,
    factory: FactoryRunCreate,
) -> Result<ExpertRun, AppError> {
    create_factory_run_with_id_at(state, id, create, factory, chrono::Utc::now()).await
}

#[cfg(test)]
pub(crate) async fn create_factory_run_with_id_at(
    state: &AppState,
    id: &str,
    create: ExpertRunCreate,
    factory: FactoryRunCreate,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<ExpertRun, AppError> {
    let workflow = prepare_factory_workflow(&create, factory, now)?;
    create_run_record_with_id(state, id, create, Some(workflow), now).await
}

pub(crate) fn validate_prepared_factory_workflow(
    create: &ExpertRunCreate,
    workflow: &FactoryWorkflow,
) -> Result<(), AppError> {
    validate_factory_workflow(workflow, create)
}

pub(crate) async fn create_prepared_factory_run_with_id(
    state: &AppState,
    id: &str,
    create: ExpertRunCreate,
    workflow: FactoryWorkflow,
) -> Result<ExpertRun, AppError> {
    validate_prepared_factory_workflow(&create, &workflow)?;
    let started_at = parse_factory_timestamp(&workflow.created_at, "Factory createdAt")?;
    create_run_record_with_id(state, id, create, Some(workflow), started_at).await
}

fn same_factory_work_order(existing: &FactoryWorkflow, requested: &FactoryWorkflow) -> bool {
    let existing = &existing.work_contract;
    let requested = &requested.work_contract;
    existing.ticket_reference == requested.ticket_reference
        && existing.title == requested.title
        && existing.objective == requested.objective
        && existing.acceptance_criteria == requested.acceptance_criteria
        && existing.non_goals == requested.non_goals
        && existing.playbook == requested.playbook
        && existing.workspace_pack_revision == requested.workspace_pack_revision
        && existing.risk == requested.risk
}

async fn create_run_record_with_id(
    state: &AppState,
    id: &str,
    create: ExpertRunCreate,
    factory: Option<FactoryWorkflow>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<ExpertRun, AppError> {
    uuid::Uuid::parse_str(id).map_err(|_| invalid("Expert run id is invalid"))?;
    validate_text(&create.expert_id, "expertId")?;
    validate_text(&create.project_path, "projectPath")?;
    validate_text(&create.client, "client")?;
    validate_contract(&create.contract)?;
    let policy_lease = crate::state_db::SecurityPolicyLease::exclusive(&state.app_data_dir).await?;
    let _lock = lock(state)?;
    let mut runs = load(state).await?;
    if let Some(existing) = runs.iter().find(|run| run.id == id) {
        let same_factory = match (&existing.factory, &factory) {
            (None, None) => true,
            (Some(existing), Some(requested)) => same_factory_work_order(existing, requested),
            _ => false,
        };
        if existing.snapshot == create && same_factory {
            return Ok(existing.clone());
        }
        return Err(invalid("Expert run id conflicts with another run"));
    }
    let run = ExpertRun {
        id: id.to_owned(),
        snapshot: create,
        state: ExpertRunState::InProgress,
        started_at: factory_timestamp(now),
        ended_at: None,
        evidence: Vec::new(),
        blockers: Vec::new(),
        waivers: Vec::new(),
        factory,
    };
    if add_run_with_retention(&mut runs, run.clone())? {
        crate::skills::mcp::reconcile_factory_terminal_audits_under_policy_lease(
            state,
            &policy_lease,
        )
        .await?;
    }
    save(state, &runs).await?;
    Ok(run)
}

fn add_run_with_retention(
    runs: &mut Vec<ExpertRun>,
    candidate: ExpertRun,
) -> Result<bool, AppError> {
    let candidate_id = candidate.id.clone();
    let mut next = runs.clone();
    next.push(candidate);
    let mut pruned_factory = false;
    loop {
        let bytes = serde_json::to_vec_pretty(&next)
            .map_err(|error| invalid(format!("serialize Expert runs: {error}")))?;
        if next.len() <= MAX_RUNS && bytes.len() as u64 <= MAX_RUN_BYTES {
            *runs = next;
            return Ok(pruned_factory);
        }
        let index = next
            .iter()
            .position(|run| run.id != candidate_id && run.state.terminal())
            .ok_or_else(|| invalid("Expert run state capacity reached"))?;
        pruned_factory |= next.remove(index).factory.is_some();
    }
}

pub async fn list_runs(
    state: &AppState,
    client: &str,
    project: Option<&str>,
) -> Result<Vec<ExpertRun>, AppError> {
    Ok(load(state)
        .await?
        .into_iter()
        .filter(|run| {
            run.snapshot.client == client && project.is_none_or(|p| run.snapshot.project_path == p)
        })
        .collect())
}

pub async fn get_run(
    state: &AppState,
    id: &str,
    client: &str,
    project: &str,
) -> Result<ExpertRun, AppError> {
    let mut runs = load(state).await?;
    Ok(scoped(&mut runs, id, client, project)?.clone())
}

fn factory_mut(run: &mut ExpertRun) -> Result<&mut FactoryWorkflow, AppError> {
    if run.state.terminal() {
        return Err(invalid("Factory run is terminal"));
    }
    run.factory
        .as_mut()
        .ok_or_else(|| invalid("Expert run is not Factory-enabled"))
}

fn reject_factory_legacy_mutation(run: &ExpertRun) -> Result<(), AppError> {
    if run.factory.is_some() {
        return Err(invalid(
            "Factory-enabled Expert runs require the Factory worker and desktop decision protocol",
        ));
    }
    Ok(())
}

fn factory_by_id_mut<'a>(
    runs: &'a mut [ExpertRun],
    id: &str,
) -> Result<&'a mut ExpertRun, AppError> {
    runs.iter_mut()
        .find(|run| run.id == id)
        .ok_or_else(|| invalid("Expert run does not exist"))
}

async fn mutate_factory_scoped_idempotent<R>(
    state: &AppState,
    id: &str,
    client: &str,
    project: &str,
    idempotency_key: &str,
    request_digest: &str,
    mutation: impl FnOnce(&mut ExpertRun) -> Result<R, AppError>,
) -> Result<R, AppError> {
    let _lock = lock(state)?;
    let mut runs = load(state).await?;
    if runs
        .iter()
        .filter(|run| run.snapshot.client == client)
        .any(|run| {
            run.factory.as_ref().is_some_and(|workflow| {
                workflow.idempotency.iter().any(|record| {
                    record.key == idempotency_key
                        && (run.id != id || record.request_digest != request_digest)
                })
            })
        })
    {
        return Err(invalid(
            "Factory idempotency key conflicts with a different request, run, or project",
        ));
    }
    let result = mutation(scoped(&mut runs, id, client, project)?)?;
    save(state, &runs).await?;
    Ok(result)
}

async fn mutate_factory_by_id<R>(
    state: &AppState,
    id: &str,
    mutation: impl FnOnce(&mut ExpertRun) -> Result<R, AppError>,
) -> Result<R, AppError> {
    let _lock = lock(state)?;
    let mut runs = load(state).await?;
    let result = mutation(factory_by_id_mut(&mut runs, id)?)?;
    save(state, &runs).await?;
    Ok(result)
}

fn current_factory_attempt(workflow: &FactoryWorkflow) -> u8 {
    workflow
        .attempts
        .last()
        .map(|attempt| attempt.number)
        .unwrap_or(0)
}

fn current_factory_head(workflow: &FactoryWorkflow) -> Option<&str> {
    workflow
        .attempts
        .last()
        .and_then(|attempt| attempt.head_commit.as_deref())
}

fn has_active_factory_blocker(workflow: &FactoryWorkflow) -> bool {
    workflow
        .blockers
        .iter()
        .any(|blocker| blocker.resolved_at.is_none())
}

fn factory_claim_is_current(
    claim: &FactoryClaim,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<bool, AppError> {
    Ok(claim.released_at.is_none()
        && parse_factory_timestamp(&claim.expires_at, "Factory claim expiresAt")? > now)
}

fn release_factory_claim(
    workflow: &mut FactoryWorkflow,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<FactoryClaim, AppError> {
    if workflow.prior_claims.len() >= MAX_FACTORY_CLAIMS {
        workflow.prior_claims.remove(0);
    }
    let mut claim = workflow
        .current_claim
        .take()
        .ok_or_else(|| invalid("Factory phase has no current claim"))?;
    claim.released_at = Some(factory_timestamp(now));
    workflow.prior_claims.push(claim.clone());
    Ok(claim)
}

fn factory_idempotency(
    workflow: &FactoryWorkflow,
    key: &str,
    request_digest: &str,
) -> Result<Option<FactoryIdempotencyRecord>, AppError> {
    let Some(existing) = workflow.idempotency.iter().find(|item| item.key == key) else {
        return Ok(None);
    };
    if existing.request_digest != request_digest {
        return Err(invalid(
            "Factory idempotency key conflicts with a different request",
        ));
    }
    Ok(Some(existing.clone()))
}

fn push_factory_idempotency(
    workflow: &mut FactoryWorkflow,
    record: FactoryIdempotencyRecord,
) -> Result<(), AppError> {
    if workflow.idempotency.len() >= MAX_FACTORY_IDEMPOTENCY {
        return Err(invalid("Factory idempotency capacity reached"));
    }
    workflow.idempotency.push(record);
    Ok(())
}

fn factory_idempotency_record(
    key: String,
    run_id: &str,
    request_digest: String,
    result_id: String,
    workflow: &FactoryWorkflow,
    now: chrono::DateTime<chrono::Utc>,
) -> FactoryIdempotencyRecord {
    FactoryIdempotencyRecord {
        key,
        run_id: run_id.into(),
        request_digest,
        result_id,
        result_revision: workflow.revision,
        result_phase: workflow.phase,
        created_at: factory_timestamp(now),
        claim_result: None,
    }
}

fn validate_factory_commit(value: &str, field: &str) -> Result<(), AppError> {
    if !(7..=64).contains(&value.len())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(format!(
            "{field} is not a lowercase commit identifier"
        )));
    }
    Ok(())
}

fn validate_worker_context(
    workflow: &FactoryWorkflow,
    worker_identity: &str,
    context: &FactoryWorkerContext,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), AppError> {
    validate_factory_external_text(
        &context.idempotency_key,
        "Factory idempotencyKey",
        MAX_FACTORY_ITEM_TEXT,
    )?;
    validate_factory_text(
        worker_identity,
        "Factory worker identity",
        MAX_FACTORY_ITEM_TEXT,
    )?;
    let approval = workflow.plan_approval.as_ref();
    let plan_revision = approval.map(|item| item.plan_revision.as_str());
    let base_commit = approval.map(|item| item.base_commit.as_str());
    if workflow.revision != context.expected_revision
        || workflow.phase != context.phase
        || current_factory_attempt(workflow) != context.attempt
        || workflow.work_contract_revision != context.work_contract_revision
        || plan_revision != context.approved_plan_revision.as_deref()
        || base_commit != context.base_commit.as_deref()
        || current_factory_head(workflow) != context.head_commit.as_deref()
    {
        return Err(invalid("Factory worker binding is stale"));
    }
    let claim = workflow
        .current_claim
        .as_ref()
        .ok_or_else(|| invalid("Factory phase has no current claim"))?;
    if claim.id != context.claim_id
        || claim.generation != context.claim_generation
        || claim.worker_identity != worker_identity
        || claim.phase != context.phase
        || claim.run_revision != workflow.revision
        || !factory_claim_is_current(claim, now)?
    {
        return Err(invalid("Factory claim binding is stale"));
    }
    Ok(())
}

fn renew_factory_claim(
    workflow: &mut FactoryWorkflow,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), AppError> {
    let claim = workflow
        .current_claim
        .as_mut()
        .ok_or_else(|| invalid("Factory phase has no current claim"))?;
    claim.run_revision = workflow.revision;
    claim.last_renewed_at = factory_timestamp(now);
    claim.expires_at =
        factory_timestamp(now + chrono::Duration::seconds(FACTORY_CLAIM_LEASE_SECONDS));
    Ok(())
}

pub async fn list_factory_work(
    state: &AppState,
    client: &str,
    project: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<FactoryWorkSummary>, AppError> {
    let mut summaries = Vec::new();
    for run in list_runs(state, client, Some(project)).await? {
        let Some(factory) = run.factory.as_ref() else {
            continue;
        };
        let held_by_current_claim = match factory.current_claim.as_ref() {
            Some(claim) => factory_claim_is_current(claim, now)?,
            None => false,
        };
        if run.state.terminal()
            || !factory.phase.worker_claimable()
            || has_active_factory_blocker(factory)
            || held_by_current_claim
        {
            continue;
        }
        summaries.push(FactoryWorkSummary {
            run_id: run.id,
            ticket_reference: factory.work_contract.ticket_reference.clone(),
            title: factory.work_contract.title.clone(),
            phase: factory.phase,
            revision: factory.revision,
            attempt: current_factory_attempt(factory),
        });
    }
    Ok(summaries)
}

pub async fn factory_claim_phase(
    state: &AppState,
    id: &str,
    client: &str,
    project: &str,
    worker_identity: &str,
    request: FactoryClaimRequest,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<FactoryClaim, AppError> {
    validate_factory_external_text(
        &request.idempotency_key,
        "Factory idempotencyKey",
        MAX_FACTORY_ITEM_TEXT,
    )?;
    validate_factory_text(
        worker_identity,
        "Factory worker identity",
        MAX_FACTORY_ITEM_TEXT,
    )?;
    let request_digest = factory_digest(&serde_json::json!({
        "kind": "claim",
        "runId": id,
        "client": client,
        "projectPath": project,
        "workerIdentity": worker_identity,
        "request": &request,
    }))?;
    let idempotency_key = request.idempotency_key.clone();
    let mutation_digest = request_digest.clone();
    mutate_factory_scoped_idempotent(
        state,
        id,
        client,
        project,
        &idempotency_key,
        &request_digest,
        move |run| {
            let run_id = run.id.clone();
            if let Some(workflow) = run.factory.as_ref() {
                if let Some(existing) =
                    factory_idempotency(workflow, &request.idempotency_key, &mutation_digest)?
                {
                    return existing
                        .claim_result
                        .ok_or_else(|| invalid("Factory idempotent claim result is missing"));
                }
            }
            let workflow = factory_mut(run)?;
            if workflow.revision != request.expected_revision || workflow.phase != request.phase {
                return Err(invalid("Factory claim request is stale"));
            }
            if !workflow.phase.worker_claimable() || has_active_factory_blocker(workflow) {
                return Err(invalid("Factory phase is not claimable"));
            }
            if let Some(claim) = workflow.current_claim.as_ref() {
                if factory_claim_is_current(claim, now)? {
                    return Err(invalid("Factory phase is already claimed"));
                }
                release_factory_claim(workflow, now)?;
            }
            if workflow.phase == FactoryPhase::IndependentReview
                && workflow
                    .attempts
                    .last()
                    .and_then(|attempt| attempt.builder_identity.as_deref())
                    == Some(worker_identity)
            {
                return Err(invalid(
                    "Factory reviewer must be a distinct worker session",
                ));
            }
            let generation = workflow
                .prior_claims
                .iter()
                .map(|claim| claim.generation)
                .max()
                .unwrap_or(0)
                + 1;
            workflow.revision += 1;
            let timestamp = factory_timestamp(now);
            let claim = FactoryClaim {
                id: uuid::Uuid::new_v4().to_string(),
                idempotency_key: request.idempotency_key.clone(),
                generation,
                worker_identity: worker_identity.into(),
                phase: workflow.phase,
                run_revision: workflow.revision,
                claimed_at: timestamp.clone(),
                last_renewed_at: timestamp,
                expires_at: factory_timestamp(
                    now + chrono::Duration::seconds(FACTORY_CLAIM_LEASE_SECONDS),
                ),
                released_at: None,
            };
            workflow.current_claim = Some(claim.clone());
            let mut record = factory_idempotency_record(
                request.idempotency_key,
                &run_id,
                mutation_digest,
                claim.id.clone(),
                workflow,
                now,
            );
            record.claim_result = Some(claim.clone());
            push_factory_idempotency(workflow, record)?;
            Ok(claim)
        },
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn factory_claim_contract(
    state: &AppState,
    id: &str,
    client: &str,
    project: &str,
    worker_identity: &str,
    claim_id: &str,
    claim_generation: u64,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<FactoryClaimContract, AppError> {
    let run = get_run(state, id, client, project).await?;
    let factory = run
        .factory
        .as_ref()
        .ok_or_else(|| invalid("Expert run is not Factory-enabled"))?;
    let claim = factory
        .current_claim
        .as_ref()
        .filter(|claim| {
            claim.id == claim_id
                && claim.generation == claim_generation
                && claim.worker_identity == worker_identity
        })
        .ok_or_else(|| invalid("Factory claim does not exist"))?;
    if claim.run_revision != factory.revision || !factory_claim_is_current(claim, now)? {
        return Err(invalid("Factory claim is stale"));
    }
    let permitted_submissions = factory_permitted_submission_shapes(factory.phase)?;
    Ok(FactoryClaimContract {
        run_id: run.id,
        project_path: run.snapshot.project_path,
        expert_id: run.snapshot.expert_id,
        expert_version: run.snapshot.expert_version,
        work_contract: factory.work_contract.clone(),
        work_contract_revision: factory.work_contract_revision.clone(),
        phase: factory.phase,
        attempt: current_factory_attempt(factory),
        attempt_limit: MAX_FACTORY_ATTEMPTS,
        run_revision: factory.revision,
        approved_plan: factory.plan.clone().filter(|plan| {
            factory
                .plan_approval
                .as_ref()
                .is_some_and(|approval| approval.plan_revision == plan.revision)
        }),
        base_commit: factory
            .plan_approval
            .as_ref()
            .map(|approval| approval.base_commit.clone()),
        head_commit: current_factory_head(factory).map(str::to_owned),
        required_checks: run
            .snapshot
            .contract
            .checks
            .iter()
            .filter(|check| check.required)
            .cloned()
            .collect(),
        permitted_submissions,
        claim_id: claim.id.clone(),
        claim_generation: claim.generation,
        expires_at: claim.expires_at.clone(),
    })
}

fn factory_permitted_submission_shapes(
    phase: FactoryPhase,
) -> Result<Vec<FactoryPermittedSubmissionShape>, AppError> {
    let mut permitted_submissions = vec![
        FactoryPermittedSubmissionShape::Artifact,
        FactoryPermittedSubmissionShape::Blocker,
    ];
    let completion = match phase {
        FactoryPhase::Planning => FactoryPermittedSubmissionShape::PlanningCompletion,
        FactoryPhase::Build => {
            permitted_submissions.push(FactoryPermittedSubmissionShape::Evidence);
            FactoryPermittedSubmissionShape::BuildCompletion
        }
        FactoryPhase::Validation => {
            permitted_submissions.push(FactoryPermittedSubmissionShape::Evidence);
            FactoryPermittedSubmissionShape::ValidationCompletion
        }
        FactoryPhase::IndependentReview => {
            permitted_submissions.push(FactoryPermittedSubmissionShape::Evidence);
            FactoryPermittedSubmissionShape::IndependentReviewCompletion
        }
        FactoryPhase::Delivery => {
            permitted_submissions.push(FactoryPermittedSubmissionShape::Evidence);
            FactoryPermittedSubmissionShape::DeliveryCompletion
        }
        _ => return Err(invalid("Factory claim phase is not worker-claimable")),
    };
    permitted_submissions.push(completion);
    Ok(permitted_submissions)
}

pub async fn factory_release_claim(
    state: &AppState,
    id: &str,
    expected_revision: u64,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<ExpertRun, AppError> {
    mutate_factory_by_id(state, id, move |run| {
        let workflow = factory_mut(run)?;
        if workflow.revision != expected_revision {
            return Err(invalid("Factory release request is stale"));
        }
        release_factory_claim(workflow, now)?;
        workflow.revision += 1;
        Ok(run.clone())
    })
    .await
}

pub async fn factory_resolve_blocker(
    state: &AppState,
    id: &str,
    expected_revision: u64,
    blocker_id: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<ExpertRun, AppError> {
    validate_factory_text(blocker_id, "Factory blockerId", MAX_FACTORY_ITEM_TEXT)?;
    mutate_factory_by_id(state, id, move |run| {
        let workflow = factory_mut(run)?;
        if workflow.revision != expected_revision {
            return Err(invalid("Factory blocker resolution is stale"));
        }
        let blocker = workflow
            .blockers
            .iter_mut()
            .find(|blocker| blocker.id == blocker_id && blocker.resolved_at.is_none())
            .ok_or_else(|| invalid("Factory blocker does not exist"))?;
        blocker.resolved_at = Some(factory_timestamp(now));
        if workflow.current_claim.is_some() {
            release_factory_claim(workflow, now)?;
        }
        workflow.revision += 1;
        Ok(run.clone())
    })
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn factory_submit_blocker(
    state: &AppState,
    id: &str,
    client: &str,
    project: &str,
    worker_identity: &str,
    context: FactoryWorkerContext,
    input: FactoryBlockerInput,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<FactoryBlocker, AppError> {
    validate_factory_external_text(&input.kind, "Factory blocker kind", MAX_FACTORY_ITEM_TEXT)?;
    validate_factory_external_text(&input.summary, "Factory blocker summary", MAX_TEXT)?;
    let request_digest = factory_digest(&serde_json::json!({
        "kind": "blocker",
        "runId": id,
        "client": client,
        "projectPath": project,
        "workerIdentity": worker_identity,
        "context": &context,
        "input": &input,
    }))?;
    let idempotency_key = context.idempotency_key.clone();
    let mutation_digest = request_digest.clone();
    mutate_factory_scoped_idempotent(
        state,
        id,
        client,
        project,
        &idempotency_key,
        &request_digest,
        move |run| {
            let run_id = run.id.clone();
            if let Some(workflow) = run.factory.as_ref() {
                if let Some(existing) =
                    factory_idempotency(workflow, &context.idempotency_key, &mutation_digest)?
                {
                    return workflow
                        .blockers
                        .iter()
                        .find(|blocker| blocker.id == existing.result_id)
                        .cloned()
                        .ok_or_else(|| invalid("Factory idempotent blocker result is missing"));
                }
            }
            let workflow = factory_mut(run)?;
            validate_worker_context(workflow, worker_identity, &context, now)?;
            if workflow.blockers.len() >= MAX_FACTORY_BLOCKERS {
                return Err(invalid("Factory blocker capacity reached"));
            }
            let blocker = FactoryBlocker {
                id: uuid::Uuid::new_v4().to_string(),
                run_id: run_id.clone(),
                idempotency_key: context.idempotency_key.clone(),
                claim_id: context.claim_id.clone(),
                claim_generation: context.claim_generation,
                kind: input.kind,
                summary: input.summary,
                phase: workflow.phase,
                attempt: current_factory_attempt(workflow),
                reported_by: worker_identity.into(),
                reported_at: factory_timestamp(now),
                resolved_at: None,
            };
            workflow.blockers.push(blocker.clone());
            workflow.revision += 1;
            renew_factory_claim(workflow, now)?;
            let record = factory_idempotency_record(
                context.idempotency_key,
                &run_id,
                mutation_digest,
                blocker.id.clone(),
                workflow,
                now,
            );
            push_factory_idempotency(workflow, record)?;
            Ok(blocker)
        },
    )
    .await
}

fn validate_factory_artifact_reference(value: &str) -> Result<(), AppError> {
    validate_factory_external_url_text(value, "Factory artifact reference")?;
    let parsed = url::Url::parse(value)
        .map_err(|_| invalid("Factory artifact reference is not a safe URL or URN"))?;
    let safe_https = parsed.scheme() == "https"
        && parsed.host_str().is_some()
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && !factory_url_has_credentials(&parsed);
    let safe_urn = parsed.scheme() == "urn"
        && !parsed.path().is_empty()
        && parsed.query().is_none()
        && parsed.fragment().is_none()
        && !factory_url_has_credentials(&parsed);
    if !safe_https && !safe_urn {
        return Err(invalid(
            "Factory artifact reference must be a credential-free HTTPS URL or URN",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn factory_submit_artifact(
    state: &AppState,
    id: &str,
    client: &str,
    project: &str,
    worker_identity: &str,
    context: FactoryWorkerContext,
    input: FactoryArtifactInput,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<FactoryArtifact, AppError> {
    validate_factory_external_text(&input.kind, "Factory artifact kind", MAX_FACTORY_ITEM_TEXT)?;
    validate_factory_external_text(
        &input.label,
        "Factory artifact label",
        MAX_FACTORY_ITEM_TEXT,
    )?;
    validate_factory_artifact_reference(&input.reference)?;
    validate_factory_digest(&input.digest, "Factory artifact digest")?;
    validate_factory_external_text(&input.summary, "Factory artifact summary", MAX_TEXT)?;
    if input.byte_size == 0 || input.byte_size > MAX_FACTORY_ARTIFACT_BYTES {
        return Err(invalid("Factory artifact byteSize is invalid"));
    }
    let request_digest = factory_digest(&serde_json::json!({
        "kind": "artifact",
        "runId": id,
        "client": client,
        "projectPath": project,
        "workerIdentity": worker_identity,
        "context": &context,
        "input": &input,
    }))?;
    let idempotency_key = context.idempotency_key.clone();
    let mutation_digest = request_digest.clone();
    mutate_factory_scoped_idempotent(
        state,
        id,
        client,
        project,
        &idempotency_key,
        &request_digest,
        move |run| {
            let run_id = run.id.clone();
            if let Some(workflow) = run.factory.as_ref() {
                if let Some(existing) =
                    factory_idempotency(workflow, &context.idempotency_key, &mutation_digest)?
                {
                    return workflow
                        .artifacts
                        .iter()
                        .find(|artifact| artifact.id == existing.result_id)
                        .cloned()
                        .ok_or_else(|| invalid("Factory idempotent artifact result is missing"));
                }
            }
            let workflow = factory_mut(run)?;
            validate_worker_context(workflow, worker_identity, &context, now)?;
            if workflow.artifacts.len() >= MAX_FACTORY_ARTIFACTS {
                return Err(invalid("Factory artifact capacity reached"));
            }
            let artifact = FactoryArtifact {
                id: uuid::Uuid::new_v4().to_string(),
                run_id: run_id.clone(),
                idempotency_key: context.idempotency_key.clone(),
                claim_id: context.claim_id.clone(),
                kind: input.kind,
                label: input.label,
                reference: input.reference,
                digest: input.digest,
                byte_size: input.byte_size,
                summary: input.summary,
                phase: context.phase,
                attempt: context.attempt,
                claim_generation: context.claim_generation,
                work_contract_revision: context.work_contract_revision.clone(),
                approved_plan_revision: context.approved_plan_revision.clone(),
                base_commit: context.base_commit.clone(),
                head_commit: context.head_commit.clone(),
                provenance: FactoryProvenance::ClientReported,
                submitted_at: factory_timestamp(now),
            };
            workflow.artifacts.push(artifact.clone());
            workflow.revision += 1;
            renew_factory_claim(workflow, now)?;
            let record = factory_idempotency_record(
                context.idempotency_key,
                &run_id,
                mutation_digest,
                artifact.id.clone(),
                workflow,
                now,
            );
            push_factory_idempotency(workflow, record)?;
            Ok(artifact)
        },
    )
    .await
}

fn validate_factory_evidence_input(input: &FactoryEvidenceInput) -> Result<(), AppError> {
    validate_factory_text(
        &input.check_name,
        "Factory evidence checkName",
        MAX_FACTORY_ITEM_TEXT,
    )?;
    validate_factory_external_optional_text(
        input.command_label.as_deref(),
        "Factory evidence commandLabel",
        256,
    )?;
    validate_factory_external_text(&input.summary, "Factory evidence summary", MAX_TEXT)?;
    if input.artifact_ids.len() > MAX_FACTORY_ITEMS
        || input.artifact_ids.iter().collect::<HashSet<_>>().len() != input.artifact_ids.len()
        || input
            .artifact_ids
            .iter()
            .any(|id| uuid::Uuid::parse_str(id).is_err())
    {
        return Err(invalid("Factory evidence artifactIds are invalid"));
    }
    match (&input.command_label, input.exit_code, &input.result) {
        (None, None, _) => {}
        (Some(_), Some(0), EvidenceResult::Pass) => {}
        (Some(_), Some(_), EvidenceResult::Fail | EvidenceResult::Skipped) => {}
        _ => {
            return Err(invalid(
                "Factory evidence command result and exitCode are inconsistent",
            ))
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn factory_submit_evidence(
    state: &AppState,
    id: &str,
    client: &str,
    project: &str,
    worker_identity: &str,
    context: FactoryWorkerContext,
    input: FactoryEvidenceInput,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<FactoryEvidence, AppError> {
    validate_factory_evidence_input(&input)?;
    let request_digest = factory_digest(&serde_json::json!({
        "kind": "evidence",
        "runId": id,
        "client": client,
        "projectPath": project,
        "workerIdentity": worker_identity,
        "context": &context,
        "input": &input,
    }))?;
    let idempotency_key = context.idempotency_key.clone();
    let mutation_digest = request_digest.clone();
    mutate_factory_scoped_idempotent(
        state,
        id,
        client,
        project,
        &idempotency_key,
        &request_digest,
        move |run| {
            let run_id = run.id.clone();
            let contract = run.snapshot.contract.clone();
            if let Some(workflow) = run.factory.as_ref() {
                if let Some(existing) =
                    factory_idempotency(workflow, &context.idempotency_key, &mutation_digest)?
                {
                    return workflow
                        .evidence
                        .iter()
                        .find(|evidence| evidence.id == existing.result_id)
                        .cloned()
                        .ok_or_else(|| invalid("Factory idempotent evidence result is missing"));
                }
            }
            let workflow = factory_mut(run)?;
            validate_worker_context(workflow, worker_identity, &context, now)?;
            if !matches!(
                workflow.phase,
                FactoryPhase::Build
                    | FactoryPhase::Validation
                    | FactoryPhase::IndependentReview
                    | FactoryPhase::Delivery
            ) {
                return Err(invalid("Factory evidence is not allowed in this phase"));
            }
            if !contract
                .checks
                .iter()
                .any(|check| check.name == input.check_name)
            {
                return Err(invalid("Factory evidence names an unknown check"));
            }
            if workflow.evidence.len() >= MAX_FACTORY_EVIDENCE {
                return Err(invalid("Factory evidence capacity reached"));
            }
            for artifact_id in &input.artifact_ids {
                let artifact = workflow
                    .artifacts
                    .iter()
                    .find(|artifact| artifact.id == *artifact_id)
                    .ok_or_else(|| invalid("Factory evidence artifact does not exist"))?;
                if artifact.run_id != run_id
                    || artifact.phase != context.phase
                    || artifact.attempt != context.attempt
                    || artifact.claim_id != context.claim_id
                    || artifact.claim_generation != context.claim_generation
                    || artifact.work_contract_revision != context.work_contract_revision
                    || artifact.approved_plan_revision != context.approved_plan_revision
                    || artifact.base_commit != context.base_commit
                    || artifact.head_commit != context.head_commit
                {
                    return Err(invalid("Factory evidence artifact binding is stale"));
                }
            }
            let evidence = FactoryEvidence {
                id: uuid::Uuid::new_v4().to_string(),
                run_id: run_id.clone(),
                idempotency_key: context.idempotency_key.clone(),
                claim_id: context.claim_id.clone(),
                check_name: input.check_name,
                result: input.result,
                command_label: input.command_label,
                exit_code: input.exit_code,
                summary: input.summary,
                artifact_ids: input.artifact_ids,
                phase: context.phase,
                attempt: context.attempt,
                claim_generation: context.claim_generation,
                work_contract_revision: context.work_contract_revision.clone(),
                approved_plan_revision: context.approved_plan_revision.clone(),
                base_commit: context.base_commit.clone(),
                head_commit: context.head_commit.clone(),
                provenance: FactoryProvenance::ClientReported,
                submitted_at: factory_timestamp(now),
            };
            workflow.evidence.push(evidence.clone());
            workflow.revision += 1;
            renew_factory_claim(workflow, now)?;
            let record = factory_idempotency_record(
                context.idempotency_key,
                &run_id,
                mutation_digest,
                evidence.id.clone(),
                workflow,
                now,
            );
            push_factory_idempotency(workflow, record)?;
            Ok(evidence)
        },
    )
    .await
}

fn validate_factory_plan_input(
    input: &FactoryPlanInput,
    contract: &QualityContract,
) -> Result<(), AppError> {
    validate_factory_external_text(&input.content, "Factory plan content", MAX_TEXT)?;
    validate_factory_external_list(&input.citations, "Factory plan citations", true)?;
    validate_factory_list(&input.declared_checks, "Factory plan declaredChecks", false)?;
    validate_factory_external_list(&input.risks, "Factory plan risks", false)?;
    validate_factory_external_list(
        &input.known_limitations,
        "Factory plan knownLimitations",
        true,
    )?;
    validate_factory_commit(&input.base_commit, "Factory plan baseCommit")?;
    let declared = input
        .declared_checks
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    if declared.len() != input.declared_checks.len()
        || declared
            .iter()
            .any(|name| !contract.checks.iter().any(|check| check.name == *name))
        || contract
            .checks
            .iter()
            .filter(|check| check.required)
            .any(|check| !declared.contains(check.name.as_str()))
    {
        return Err(invalid("Factory plan declared checks are invalid"));
    }
    Ok(())
}

fn factory_plan_revision(
    work_contract_revision: &str,
    input: &FactoryPlanInput,
) -> Result<String, AppError> {
    factory_digest(&serde_json::json!({
        "workContractRevision": work_contract_revision,
        "plan": input,
    }))
}

fn finish_factory_attempt(
    workflow: &mut FactoryWorkflow,
    result: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<bool, AppError> {
    let attempt = workflow
        .attempts
        .last_mut()
        .ok_or_else(|| invalid("Factory run has no current attempt"))?;
    attempt.ended_at = Some(factory_timestamp(now));
    attempt.result = Some(result.into());
    let exhausted = attempt.number >= MAX_FACTORY_ATTEMPTS;
    let retain_decisive_review = exhausted
        && workflow
            .review
            .as_ref()
            .is_some_and(|review| review.verdict == FactoryReviewVerdict::Rework);
    workflow.validation = None;
    if !retain_decisive_review {
        workflow.review = None;
    }
    workflow.delivery = None;
    workflow.human_waivers.clear();
    if exhausted {
        workflow.phase = FactoryPhase::Completed;
        workflow.terminal = Some(FactoryTerminalDecision {
            outcome: FactoryTerminalOutcome::AttemptExhausted,
            decided_at: factory_timestamp(now),
            safe_detail: Some("Automated build and review attempts were exhausted".into()),
        });
        return Ok(true);
    }
    let number = attempt.number + 1;
    workflow.attempts.push(FactoryAttempt {
        number,
        started_at: factory_timestamp(now),
        ended_at: None,
        head_commit: None,
        builder_identity: None,
        result: None,
    });
    workflow.phase = FactoryPhase::Build;
    Ok(false)
}

fn latest_factory_phase_evidence<'a>(
    workflow: &'a FactoryWorkflow,
    check_name: &str,
    phase: FactoryPhase,
    claim_id: &str,
    claim_generation: u64,
) -> Option<&'a FactoryEvidence> {
    let approval = workflow.plan_approval.as_ref()?;
    let attempt = workflow.attempts.last()?;
    let head = attempt.head_commit.as_deref()?;
    workflow.evidence.iter().rev().find(|evidence| {
        evidence.check_name == check_name
            && evidence.phase == phase
            && evidence.claim_id == claim_id
            && evidence.claim_generation == claim_generation
            && evidence.attempt == attempt.number
            && evidence.work_contract_revision == workflow.work_contract_revision
            && evidence.approved_plan_revision.as_deref() == Some(approval.plan_revision.as_str())
            && evidence.base_commit.as_deref() == Some(approval.base_commit.as_str())
            && evidence.head_commit.as_deref() == Some(head)
    })
}

fn latest_factory_current_attempt_evidence<'a>(
    workflow: &'a FactoryWorkflow,
    check_name: &str,
) -> Option<&'a FactoryEvidence> {
    let approval = workflow.plan_approval.as_ref()?;
    let attempt = workflow.attempts.last()?;
    let head = attempt.head_commit.as_deref()?;
    workflow.evidence.iter().rev().find(|evidence| {
        evidence.check_name == check_name
            && matches!(
                evidence.phase,
                FactoryPhase::Validation | FactoryPhase::IndependentReview | FactoryPhase::Delivery
            )
            && evidence.attempt == attempt.number
            && evidence.work_contract_revision == workflow.work_contract_revision
            && evidence.approved_plan_revision.as_deref() == Some(approval.plan_revision.as_str())
            && evidence.base_commit.as_deref() == Some(approval.base_commit.as_str())
            && evidence.head_commit.as_deref() == Some(head)
    })
}

fn factory_required_evidence(
    workflow: &FactoryWorkflow,
    contract: &QualityContract,
    phase: FactoryPhase,
    claim_id: &str,
    claim_generation: u64,
) -> Result<(Vec<String>, bool), AppError> {
    let mut passed = Vec::new();
    let mut failed = false;
    for check in contract.checks.iter().filter(|check| check.required) {
        if let Some(evidence) =
            latest_factory_phase_evidence(workflow, &check.name, phase, claim_id, claim_generation)
        {
            if evidence.result == EvidenceResult::Pass {
                passed.push(check.name.clone());
            } else {
                failed = true;
            }
        }
    }
    Ok((passed, failed))
}

fn factory_required_current_attempt_evidence(
    workflow: &FactoryWorkflow,
    contract: &QualityContract,
) -> Result<(Vec<String>, bool), AppError> {
    let mut passed = Vec::new();
    let mut failed = false;
    for check in contract.checks.iter().filter(|check| check.required) {
        if let Some(evidence) = latest_factory_current_attempt_evidence(workflow, &check.name) {
            if evidence.result == EvidenceResult::Pass {
                passed.push(check.name.clone());
            } else {
                failed = true;
            }
        }
    }
    Ok((passed, failed))
}

fn factory_required_validated_evidence(
    workflow: &FactoryWorkflow,
    contract: &QualityContract,
) -> Result<(Vec<String>, bool), AppError> {
    workflow
        .validation
        .as_ref()
        .ok_or_else(|| invalid("Factory validation snapshot is missing"))?;
    factory_required_current_attempt_evidence(workflow, contract)
}

fn validate_factory_review_input(input: &FactoryReviewInput) -> Result<(), AppError> {
    validate_factory_external_text(&input.summary, "Factory review summary", MAX_TEXT)?;
    if input.findings.len() > MAX_FACTORY_ITEMS {
        return Err(invalid("Factory review finding count is invalid"));
    }
    for finding in &input.findings {
        validate_factory_external_text(
            &finding.summary,
            "Factory review finding",
            MAX_FACTORY_ITEM_TEXT,
        )?;
    }
    Ok(())
}

fn validate_factory_https(value: &str, field: &str) -> Result<(), AppError> {
    validate_factory_external_url_text(value, field)?;
    let parsed = url::Url::parse(value).map_err(|_| invalid(format!("{field} is not a URL")))?;
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.host_str().is_none()
        || factory_url_has_credentials(&parsed)
        || factory_url_has_private_path(&parsed)
    {
        return Err(invalid(format!(
            "{field} must be a credential-free HTTPS URL"
        )));
    }
    Ok(())
}

fn validate_factory_improvement(proposal: &FactoryImprovementProposal) -> Result<(), AppError> {
    validate_factory_external_text(
        &proposal.failure_class,
        "Factory improvement failureClass",
        MAX_FACTORY_ITEM_TEXT,
    )?;
    validate_factory_external_text(&proposal.proposal, "Factory improvement proposal", MAX_TEXT)?;
    validate_factory_external_optional_text(
        proposal.suggested_test.as_deref(),
        "Factory improvement suggestedTest",
        MAX_TEXT,
    )
}

#[allow(clippy::too_many_arguments)]
pub async fn factory_complete_phase(
    state: &AppState,
    id: &str,
    client: &str,
    project: &str,
    worker_identity: &str,
    context: FactoryWorkerContext,
    completion: FactoryPhaseCompletion,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<FactoryMutationReceipt, AppError> {
    let request_digest = factory_digest(&serde_json::json!({
        "kind": "completePhase",
        "runId": id,
        "client": client,
        "projectPath": project,
        "workerIdentity": worker_identity,
        "context": &context,
        "completion": &completion,
    }))?;
    let idempotency_key = context.idempotency_key.clone();
    let mutation_digest = request_digest.clone();
    mutate_factory_scoped_idempotent(
        state,
        id,
        client,
        project,
        &idempotency_key,
        &request_digest,
        move |run| {
            let run_id = run.id.clone();
            let contract = run.snapshot.contract.clone();
            let mut terminal_state = None;
            let receipt = {
                if let Some(workflow) = run.factory.as_ref() {
                    if let Some(existing) =
                        factory_idempotency(workflow, &context.idempotency_key, &mutation_digest)?
                    {
                        return Ok(FactoryMutationReceipt {
                            id: existing.result_id,
                            revision: existing.result_revision,
                            phase: existing.result_phase,
                        });
                    }
                }
                let workflow = factory_mut(run)?;
                validate_worker_context(workflow, worker_identity, &context, now)?;
                if has_active_factory_blocker(workflow) {
                    return Err(invalid("Factory phase is blocked"));
                }
                match (workflow.phase, completion) {
                    (FactoryPhase::Planning, FactoryPhaseCompletion::Planning { plan }) => {
                        validate_factory_plan_input(&plan, &contract)?;
                        let revision =
                            factory_plan_revision(&workflow.work_contract_revision, &plan)?;
                        workflow.plan = Some(FactoryPlan {
                            revision,
                            content: plan.content,
                            citations: plan.citations,
                            declared_checks: plan.declared_checks,
                            risks: plan.risks,
                            known_limitations: plan.known_limitations,
                            base_commit: plan.base_commit,
                            submitted_by: worker_identity.into(),
                            submitted_at: factory_timestamp(now),
                        });
                        workflow.plan_approval = None;
                        workflow.phase = FactoryPhase::AwaitingPlanApproval;
                    }
                    (FactoryPhase::Build, FactoryPhaseCompletion::Build { head_commit }) => {
                        validate_factory_commit(&head_commit, "Factory build headCommit")?;
                        let attempt = workflow
                            .attempts
                            .last_mut()
                            .ok_or_else(|| invalid("Factory run has no current attempt"))?;
                        if attempt.head_commit.is_some() {
                            return Err(invalid("Factory build is already complete"));
                        }
                        attempt.head_commit = Some(head_commit);
                        attempt.builder_identity = Some(worker_identity.into());
                        workflow.validation = None;
                        workflow.review = None;
                        workflow.delivery = None;
                        workflow.phase = FactoryPhase::Validation;
                    }
                    (FactoryPhase::Validation, FactoryPhaseCompletion::Validation) => {
                        let (checks, failed) =
                            factory_required_current_attempt_evidence(workflow, &contract)?;
                        if failed {
                            if finish_factory_attempt(workflow, "validationRework", now)? {
                                terminal_state = Some(ExpertRunState::Rework);
                            }
                        } else {
                            let attempt = workflow
                                .attempts
                                .last()
                                .ok_or_else(|| invalid("Factory run has no current attempt"))?;
                            let head_commit = attempt
                                .head_commit
                                .clone()
                                .ok_or_else(|| invalid("Factory validation has no head commit"))?;
                            workflow.validation = Some(FactoryValidation {
                                attempt: attempt.number,
                                head_commit,
                                check_names: checks,
                                phase: context.phase,
                                claim_id: context.claim_id.clone(),
                                claim_generation: context.claim_generation,
                                validated_at: factory_timestamp(now),
                            });
                            workflow.phase = FactoryPhase::IndependentReview;
                        }
                    }
                    (
                        FactoryPhase::IndependentReview,
                        FactoryPhaseCompletion::IndependentReview { review },
                    ) => {
                        validate_factory_review_input(&review)?;
                        let attempt = workflow
                            .attempts
                            .last()
                            .ok_or_else(|| invalid("Factory run has no current attempt"))?;
                        let head_commit = attempt
                            .head_commit
                            .clone()
                            .ok_or_else(|| invalid("Factory review has no head commit"))?;
                        if attempt.builder_identity.as_deref() == Some(worker_identity) {
                            return Err(invalid(
                                "Factory reviewer must be a distinct worker session",
                            ));
                        }
                        let verdict = review.verdict;
                        workflow.review = Some(FactoryReview {
                            attempt: attempt.number,
                            head_commit,
                            phase: context.phase,
                            claim_id: context.claim_id.clone(),
                            claim_generation: context.claim_generation,
                            reviewer_identity: worker_identity.into(),
                            verdict,
                            summary: review.summary,
                            findings: review.findings,
                            submitted_at: factory_timestamp(now),
                            provenance: FactoryProvenance::ClientReported,
                        });
                        if verdict == FactoryReviewVerdict::Rework {
                            if finish_factory_attempt(workflow, "reviewRework", now)? {
                                terminal_state = Some(ExpertRunState::Rework);
                            }
                        } else {
                            let (_, failed) =
                                factory_required_validated_evidence(workflow, &contract)?;
                            if failed {
                                if finish_factory_attempt(workflow, "reviewEvidenceRework", now)? {
                                    terminal_state = Some(ExpertRunState::Rework);
                                }
                            } else {
                                workflow.phase = FactoryPhase::Delivery;
                            }
                        }
                    }
                    (FactoryPhase::Delivery, FactoryPhaseCompletion::Delivery { delivery }) => {
                        validate_factory_https(&delivery.reference, "Factory delivery reference")?;
                        validate_factory_commit(
                            &delivery.head_commit,
                            "Factory delivery headCommit",
                        )?;
                        validate_factory_external_text(
                            &delivery.evidence_summary,
                            "Factory delivery evidenceSummary",
                            MAX_TEXT,
                        )?;
                        validate_factory_external_list(
                            &delivery.known_limitations,
                            "Factory delivery knownLimitations",
                            true,
                        )?;
                        if let Some(proposal) = &delivery.improvement_proposal {
                            validate_factory_improvement(proposal)?;
                        }
                        let review_passes = workflow.review.as_ref().is_some_and(|review| {
                            review.verdict == FactoryReviewVerdict::Pass
                                && review.head_commit == delivery.head_commit
                        });
                        let review_waived = workflow
                            .human_waivers
                            .iter()
                            .any(|waiver| waiver.kind == "independentReview");
                        if current_factory_head(workflow) != Some(delivery.head_commit.as_str())
                            || (!review_passes && !review_waived)
                        {
                            return Err(invalid(
                                "Factory delivery is not bound to the reviewed head",
                            ));
                        }
                        let (_, failed) = factory_required_validated_evidence(workflow, &contract)?;
                        if failed {
                            if finish_factory_attempt(workflow, "deliveryEvidenceRework", now)? {
                                terminal_state = Some(ExpertRunState::Rework);
                            }
                        } else {
                            workflow.delivery = Some(FactoryDelivery {
                                reference: delivery.reference,
                                attempt: current_factory_attempt(workflow),
                                head_commit: delivery.head_commit,
                                phase: context.phase,
                                claim_id: context.claim_id.clone(),
                                claim_generation: context.claim_generation,
                                evidence_summary: delivery.evidence_summary,
                                known_limitations: delivery.known_limitations,
                                submitted_at: factory_timestamp(now),
                                provenance: FactoryProvenance::ClientReported,
                            });
                            workflow.improvement_proposal = delivery.improvement_proposal;
                            workflow.phase = FactoryPhase::AwaitingFinalApproval;
                        }
                    }
                    _ => return Err(invalid("Factory phase completion is not allowed")),
                }
                workflow.revision += 1;
                release_factory_claim(workflow, now)?;
                let receipt = FactoryMutationReceipt {
                    id: uuid::Uuid::new_v4().to_string(),
                    revision: workflow.revision,
                    phase: workflow.phase,
                };
                let record = factory_idempotency_record(
                    context.idempotency_key,
                    &run_id,
                    mutation_digest,
                    receipt.id.clone(),
                    workflow,
                    now,
                );
                push_factory_idempotency(workflow, record)?;
                receipt
            };
            if let Some(state) = terminal_state {
                run.state = state;
                run.ended_at = Some(factory_timestamp(now));
            }
            Ok(receipt)
        },
    )
    .await
}

pub async fn factory_decide_plan(
    state: &AppState,
    id: &str,
    expected_revision: u64,
    plan_revision: &str,
    decision: FactoryPlanDecision,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<ExpertRun, AppError> {
    validate_factory_digest(plan_revision, "Factory planRevision")?;
    mutate_factory_by_id(state, id, move |run| {
        let workflow = factory_mut(run)?;
        if workflow.revision != expected_revision
            || workflow.phase != FactoryPhase::AwaitingPlanApproval
        {
            return Err(invalid("Factory plan decision is stale"));
        }
        let plan = workflow
            .plan
            .as_ref()
            .filter(|plan| plan.revision == plan_revision)
            .ok_or_else(|| invalid("Factory plan revision does not exist"))?;
        match decision {
            FactoryPlanDecision::Approve => {
                if !workflow.attempts.is_empty() {
                    return Err(invalid("Factory build attempt already exists"));
                }
                workflow.plan_approval = Some(FactoryPlanApproval {
                    plan_revision: plan.revision.clone(),
                    base_commit: plan.base_commit.clone(),
                    approved_at: factory_timestamp(now),
                });
                workflow.attempts.push(FactoryAttempt {
                    number: 1,
                    started_at: factory_timestamp(now),
                    ended_at: None,
                    head_commit: None,
                    builder_identity: None,
                    result: None,
                });
                workflow.phase = FactoryPhase::Build;
            }
            FactoryPlanDecision::Reject => {
                workflow.plan_approval = None;
                workflow.phase = FactoryPhase::Planning;
            }
        }
        workflow.revision += 1;
        Ok(run.clone())
    })
    .await
}

pub async fn factory_cancel(
    state: &AppState,
    id: &str,
    expected_revision: u64,
    safe_detail: Option<String>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<ExpertRun, AppError> {
    validate_factory_external_optional_text(
        safe_detail.as_deref(),
        "Factory cancellation detail",
        MAX_FACTORY_ITEM_TEXT,
    )?;
    mutate_factory_by_id(state, id, move |run| {
        {
            let workflow = factory_mut(run)?;
            if workflow.revision != expected_revision {
                return Err(invalid("Factory cancellation is stale"));
            }
            if workflow.current_claim.is_some() {
                release_factory_claim(workflow, now)?;
            }
            workflow.phase = FactoryPhase::Completed;
            workflow.terminal = Some(FactoryTerminalDecision {
                outcome: FactoryTerminalOutcome::Cancelled,
                decided_at: factory_timestamp(now),
                safe_detail,
            });
            workflow.revision += 1;
        }
        run.state = ExpertRunState::Cancelled;
        run.ended_at = Some(factory_timestamp(now));
        Ok(run.clone())
    })
    .await
}

pub async fn factory_waive_independent_review(
    state: &AppState,
    id: &str,
    expected_revision: u64,
    reason: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<ExpertRun, AppError> {
    validate_factory_external_text(reason, "Factory independent review waiver", MAX_TEXT)?;
    mutate_factory_by_id(state, id, move |run| {
        let workflow = factory_mut(run)?;
        if workflow.revision != expected_revision
            || workflow.phase != FactoryPhase::IndependentReview
            || workflow.validation.is_none()
        {
            return Err(invalid("Factory independent review waiver is stale"));
        }
        if workflow
            .human_waivers
            .iter()
            .any(|waiver| waiver.kind == "independentReview")
        {
            return Err(invalid("Factory independent review is already waived"));
        }
        if workflow.current_claim.is_some() {
            release_factory_claim(workflow, now)?;
        }
        workflow.human_waivers.push(FactoryHumanWaiver {
            kind: "independentReview".into(),
            check_name: None,
            reason: reason.into(),
            created_at: factory_timestamp(now),
        });
        workflow.phase = FactoryPhase::Delivery;
        workflow.revision += 1;
        Ok(run.clone())
    })
    .await
}

fn factory_missing_required_checks(
    workflow: &FactoryWorkflow,
    contract: &QualityContract,
) -> HashSet<String> {
    if workflow.validation.is_none() {
        return contract
            .checks
            .iter()
            .filter(|check| check.required)
            .map(|check| check.name.clone())
            .collect();
    }
    contract
        .checks
        .iter()
        .filter(|check| check.required)
        .filter(|check| {
            latest_factory_current_attempt_evidence(workflow, &check.name)
                .is_none_or(|evidence| evidence.result != EvidenceResult::Pass)
        })
        .map(|check| check.name.clone())
        .collect()
}

pub async fn factory_decide_final(
    state: &AppState,
    id: &str,
    input: FactoryFinalDecisionInput,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<ExpertRun, AppError> {
    validate_factory_digest(
        &input.approved_plan_revision,
        "Factory final approvedPlanRevision",
    )?;
    validate_factory_commit(&input.head_commit, "Factory final headCommit")?;
    validate_factory_external_optional_text(
        input.safe_detail.as_deref(),
        "Factory final safeDetail",
        MAX_FACTORY_ITEM_TEXT,
    )?;
    validate_factory_external_optional_text(
        input.independent_review_waiver_reason.as_deref(),
        "Factory independent review waiver",
        MAX_TEXT,
    )?;
    if input.check_waivers.len() > MAX_FACTORY_ITEMS {
        return Err(invalid("Factory check waiver count is invalid"));
    }
    for waiver in &input.check_waivers {
        validate_factory_text(
            &waiver.check_name,
            "Factory check waiver checkName",
            MAX_FACTORY_ITEM_TEXT,
        )?;
        validate_factory_external_text(&waiver.reason, "Factory check waiver reason", MAX_TEXT)?;
    }
    if !matches!(
        input.outcome,
        FactoryTerminalOutcome::Accepted
            | FactoryTerminalOutcome::Rework
            | FactoryTerminalOutcome::Rejected
    ) {
        return Err(invalid("Factory final outcome is not supported"));
    }
    mutate_factory_by_id(state, id, move |run| {
        let contract = run.snapshot.contract.clone();
        let state = {
            let workflow = factory_mut(run)?;
            if workflow.revision != input.expected_revision
                || workflow.phase != FactoryPhase::AwaitingFinalApproval
            {
                return Err(invalid("Factory final decision is stale"));
            }
            let approval = workflow
                .plan_approval
                .as_ref()
                .filter(|approval| approval.plan_revision == input.approved_plan_revision)
                .ok_or_else(|| invalid("Factory final plan binding is stale"))?;
            let delivery = workflow
                .delivery
                .as_ref()
                .filter(|delivery| delivery.head_commit == input.head_commit)
                .ok_or_else(|| invalid("Factory final delivery binding is stale"))?;
            if current_factory_head(workflow) != Some(input.head_commit.as_str())
                || approval.base_commit
                    != workflow
                        .plan
                        .as_ref()
                        .map(|plan| plan.base_commit.clone())
                        .unwrap_or_default()
                || delivery.reference.is_empty()
            {
                return Err(invalid("Factory final evidence binding is stale"));
            }
            if input.outcome == FactoryTerminalOutcome::Accepted {
                let review_passes = workflow.review.as_ref().is_some_and(|review| {
                    review.verdict == FactoryReviewVerdict::Pass
                        && review.head_commit == input.head_commit
                        && review.attempt == current_factory_attempt(workflow)
                });
                let review_waived = workflow
                    .human_waivers
                    .iter()
                    .any(|waiver| waiver.kind == "independentReview");
                match (
                    review_passes,
                    review_waived,
                    input.independent_review_waiver_reason.as_deref(),
                ) {
                    (true, _, Some(_)) | (false, true, Some(_)) => {
                        return Err(invalid("Factory independent review waiver is unnecessary"))
                    }
                    (false, false, Some(reason)) => {
                        workflow.human_waivers.push(FactoryHumanWaiver {
                            kind: "independentReview".into(),
                            check_name: None,
                            reason: reason.into(),
                            created_at: factory_timestamp(now),
                        });
                    }
                    (false, false, None) => {
                        return Err(invalid(
                            "Factory final acceptance requires review or an explicit waiver",
                        ))
                    }
                    _ => {}
                }
                let missing = factory_missing_required_checks(workflow, &contract);
                let waived = input
                    .check_waivers
                    .iter()
                    .map(|waiver| waiver.check_name.clone())
                    .collect::<HashSet<_>>();
                if waived.len() != input.check_waivers.len() || waived != missing {
                    return Err(invalid(
                        "Factory final required checks are missing or improperly waived",
                    ));
                }
                workflow
                    .human_waivers
                    .retain(|waiver| waiver.kind != "qualityCheck");
                workflow
                    .human_waivers
                    .extend(input.check_waivers.iter().map(|waiver| FactoryHumanWaiver {
                        kind: "qualityCheck".into(),
                        check_name: Some(waiver.check_name.clone()),
                        reason: waiver.reason.clone(),
                        created_at: factory_timestamp(now),
                    }));
            } else if !input.check_waivers.is_empty()
                || input.independent_review_waiver_reason.is_some()
            {
                return Err(invalid("Factory waivers are only valid for acceptance"));
            }
            workflow.phase = FactoryPhase::Completed;
            workflow.terminal = Some(FactoryTerminalDecision {
                outcome: input.outcome,
                decided_at: factory_timestamp(now),
                safe_detail: input.safe_detail,
            });
            workflow.revision += 1;
            match input.outcome {
                FactoryTerminalOutcome::Accepted => ExpertRunState::Accepted,
                FactoryTerminalOutcome::Rework => ExpertRunState::Rework,
                FactoryTerminalOutcome::Rejected => ExpertRunState::Rejected,
                _ => unreachable!("unsupported final outcome rejected before mutation"),
            }
        };
        run.state = state;
        run.ended_at = Some(factory_timestamp(now));
        Ok(run.clone())
    })
    .await
}

pub fn mcp_view(run: &ExpertRun) -> serde_json::Value {
    let mut value = serde_json::to_value(run).unwrap_or_else(|_| serde_json::json!({}));
    value["waivers"] = serde_json::Value::Array(
        run.waivers
            .iter()
            .map(|waiver| serde_json::json!({ "checkName": waiver.check_name, "waived": true }))
            .collect(),
    );
    if let Some(factory) = &run.factory {
        value["factory"] = serde_json::json!({
            "runId": run.id,
            "phase": factory.phase,
            "revision": factory.revision,
            "attempt": factory.attempts.last().map_or(0, |attempt| attempt.number),
            "blockerCount": factory
                .blockers
                .iter()
                .filter(|blocker| blocker.resolved_at.is_none())
                .count(),
            "terminalOutcome": factory.terminal.as_ref().map(|terminal| terminal.outcome),
            "provenance": FactoryProvenance::ClientReported,
        });
    }
    value
}

pub async fn submit_evidence(
    state: &AppState,
    id: &str,
    client: &str,
    project: &str,
    submission: EvidenceSubmission,
) -> Result<ExpertEvidence, AppError> {
    validate_text(&submission.idempotency_key, "idempotencyKey")?;
    validate_text(&submission.check_name, "checkName")?;
    validate_text(&submission.summary, "summary")?;
    if submission
        .command_label
        .as_ref()
        .is_some_and(|value| value.len() > 256)
    {
        return Err(invalid("commandLabel is oversized"));
    }
    let _lock = lock(state)?;
    let mut runs = load(state).await?;
    let run = scoped(&mut runs, id, client, project)?;
    reject_factory_legacy_mutation(run)?;
    if run.state.terminal() {
        return Err(invalid("Expert run is terminal"));
    }
    if !run
        .snapshot
        .contract
        .checks
        .iter()
        .any(|check| check.name == submission.check_name)
    {
        return Err(invalid("unknown quality contract check"));
    }
    if let Some(existing) = run
        .evidence
        .iter()
        .find(|item| item.submission.idempotency_key == submission.idempotency_key)
    {
        if existing.submission == submission {
            return Ok(existing.clone());
        }
        return Err(invalid("idempotency key conflicts with different evidence"));
    }
    let evidence = ExpertEvidence {
        id: uuid::Uuid::new_v4().to_string(),
        submission,
        submitted_at: chrono::Utc::now().to_rfc3339(),
    };
    run.evidence.push(evidence.clone());
    save(state, &runs).await?;
    Ok(evidence)
}

pub async fn report_blocker(
    state: &AppState,
    id: &str,
    client: &str,
    project: &str,
    kind: &str,
    summary: &str,
) -> Result<ExpertBlocker, AppError> {
    validate_text(kind, "kind")?;
    validate_text(summary, "summary")?;
    let _lock = lock(state)?;
    let mut runs = load(state).await?;
    let run = scoped(&mut runs, id, client, project)?;
    reject_factory_legacy_mutation(run)?;
    if run.state.terminal() {
        return Err(invalid("Expert run is terminal"));
    }
    let blocker = ExpertBlocker {
        kind: kind.into(),
        summary: summary.into(),
        reported_at: chrono::Utc::now().to_rfc3339(),
    };
    run.blockers.push(blocker.clone());
    save(state, &runs).await?;
    Ok(blocker)
}

pub async fn request_review(
    state: &AppState,
    id: &str,
    client: &str,
    project: &str,
) -> Result<ExpertRun, AppError> {
    let _lock = lock(state)?;
    let mut runs = load(state).await?;
    let run = scoped(&mut runs, id, client, project)?;
    reject_factory_legacy_mutation(run)?;
    if run.state != ExpertRunState::InProgress {
        return Err(invalid("Expert run is not in progress"));
    }
    run.state = ExpertRunState::AwaitingReview;
    let result = run.clone();
    save(state, &runs).await?;
    Ok(result)
}

pub async fn review_run_with_waivers(
    state: &AppState,
    id: &str,
    verdict: ExpertRunState,
    waiver_inputs: Vec<ExpertWaiverInput>,
) -> Result<ExpertRun, AppError> {
    if !verdict.terminal() {
        return Err(invalid("review verdict must be terminal"));
    }
    for waiver in &waiver_inputs {
        validate_text(&waiver.check_name, "waiver checkName")?;
        validate_text(&waiver.reason, "waiver reason")?;
    }
    let _lock = lock(state)?;
    let mut runs = load(state).await?;
    let run = runs
        .iter_mut()
        .find(|run| run.id == id)
        .ok_or_else(|| invalid("Expert run does not exist"))?;
    reject_factory_legacy_mutation(run)?;
    if run.state != ExpertRunState::AwaitingReview {
        return Err(invalid("Expert run is not awaiting review"));
    }
    if verdict == ExpertRunState::Accepted {
        let missing = run
            .snapshot
            .contract
            .checks
            .iter()
            .filter(|check| check.required)
            .filter(|check| {
                run.evidence
                    .iter()
                    .rev()
                    .find(|evidence| evidence.submission.check_name == check.name)
                    .is_none_or(|evidence| evidence.submission.result != EvidenceResult::Pass)
            })
            .map(|check| check.name.as_str())
            .collect::<HashSet<_>>();
        let waived = waiver_inputs
            .iter()
            .map(|waiver| waiver.check_name.as_str())
            .collect::<HashSet<_>>();
        if waived.len() != waiver_inputs.len()
            || !waived.is_subset(&missing)
            || !missing.is_subset(&waived)
        {
            return Err(invalid("required checks are missing or failed"));
        }
    } else if !waiver_inputs.is_empty() {
        return Err(invalid("waivers are only valid for accepted runs"));
    }
    run.waivers = waiver_inputs
        .into_iter()
        .map(|waiver| ExpertWaiver {
            check_name: waiver.check_name,
            reason: waiver.reason,
            created_at: chrono::Utc::now().to_rfc3339(),
        })
        .collect();
    run.state = verdict;
    run.ended_at = Some(chrono::Utc::now().to_rfc3339());
    let result = run.clone();
    save(state, &runs).await?;
    Ok(result)
}

#[tauri::command]
pub async fn expert_runs_list(
    state: State<'_, AppState>,
    project_path: Option<String>,
) -> Result<Vec<ExpertRun>, AppError> {
    Ok(load(&state)
        .await?
        .into_iter()
        .filter(|run| {
            project_path
                .as_ref()
                .is_none_or(|path| &run.snapshot.project_path == path)
        })
        .collect())
}

#[tauri::command]
pub async fn expert_run_get(state: State<'_, AppState>, id: String) -> Result<ExpertRun, AppError> {
    load(&state)
        .await?
        .into_iter()
        .find(|run| run.id == id)
        .ok_or_else(|| invalid("Expert run does not exist"))
}

#[tauri::command]
pub async fn expert_run_review(
    state: State<'_, AppState>,
    id: String,
    verdict: String,
    waivers: Vec<ExpertWaiverInput>,
) -> Result<ExpertRun, AppError> {
    let verdict = match verdict.as_str() {
        "accepted" => ExpertRunState::Accepted,
        "rework" => ExpertRunState::Rework,
        "rejected" => ExpertRunState::Rejected,
        "cancelled" => ExpertRunState::Cancelled,
        _ => return Err(invalid("unsupported Expert run verdict")),
    };
    review_run_with_waivers(&state, &id, verdict, waivers).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn factory_expert_create() -> ExpertRunCreate {
        ExpertRunCreate {
            expert_id: "factory-reviewer".into(),
            expert_version: 3,
            project_path: "/tmp/factory-project".into(),
            client: "codex".into(),
            lead_agent: "lead".into(),
            supporting_agents: vec!["reviewer".into()],
            required_skills: vec!["testing".into()],
            optional_skills: Vec::new(),
            runbook: Some("factory-runbook".into()),
            contract: QualityContract {
                version: 1,
                checks: vec![ExpertCheck {
                    name: "tests".into(),
                    kind: "tests".into(),
                    required: true,
                    evidence_mode: "clientReported".into(),
                }],
            },
        }
    }

    fn factory_create(checked_at: &str) -> FactoryRunCreate {
        FactoryRunCreate {
            ticket_reference: "APP-42".into(),
            title: "Add Factory lifecycle".into(),
            objective: "Carry one bounded work order through independent review.".into(),
            acceptance_criteria: vec!["The exact approved revision reaches delivery.".into()],
            non_goals: vec!["Do not execute Git or tests inside the app.".into()],
            playbook: Some("factory-build-review".into()),
            workspace_pack_revision: Some("a".repeat(64)),
            risk: FactoryRiskClass::Medium,
            readiness: FactoryReadinessSnapshot {
                checked_at: checked_at.into(),
                overall: FactoryReadinessOverall::Ready,
                evidence_revision: "readiness-v7".into(),
                summary: vec!["agents:ready".into(), "skills:ready".into()],
            },
        }
    }

    fn factory_now(value: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(value)
            .unwrap()
            .to_utc()
    }

    async fn create_factory_test_run(
        state: &AppState,
        now: chrono::DateTime<chrono::Utc>,
    ) -> ExpertRun {
        create_factory_run_with_id_at(
            state,
            &uuid::Uuid::new_v4().to_string(),
            factory_expert_create(),
            factory_create(&now.to_rfc3339()),
            now,
        )
        .await
        .unwrap()
    }

    fn worker_context(run: &ExpertRun, claim: &FactoryClaim, key: &str) -> FactoryWorkerContext {
        let factory = run.factory.as_ref().unwrap();
        let plan = factory.plan_approval.as_ref();
        FactoryWorkerContext {
            expected_revision: factory.revision,
            phase: factory.phase,
            attempt: factory
                .attempts
                .last()
                .map(|attempt| attempt.number)
                .unwrap_or(0),
            claim_id: claim.id.clone(),
            claim_generation: claim.generation,
            work_contract_revision: factory.work_contract_revision.clone(),
            approved_plan_revision: plan.map(|approval| approval.plan_revision.clone()),
            base_commit: plan.map(|approval| approval.base_commit.clone()),
            head_commit: factory
                .attempts
                .last()
                .and_then(|attempt| attempt.head_commit.clone()),
            idempotency_key: key.into(),
        }
    }

    fn planning_completion() -> FactoryPhaseCompletion {
        FactoryPhaseCompletion::Planning {
            plan: FactoryPlanInput {
                content: "1. Implement the bounded state machine.\n2. Verify every gate.".into(),
                citations: vec!["src-tauri/src/expert_runs.rs".into()],
                declared_checks: vec!["tests".into()],
                risks: vec!["Concurrent workers can race for one phase.".into()],
                known_limitations: vec!["Execution remains client-reported.".into()],
                base_commit: "0123456789abcdef0123456789abcdef01234567".into(),
            },
        }
    }

    async fn advance_factory_to_build(
        state: &AppState,
        run: &ExpertRun,
        now: chrono::DateTime<chrono::Utc>,
    ) -> ExpertRun {
        let factory = run.factory.as_ref().unwrap();
        let claim = factory_claim_phase(
            state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-plan",
            FactoryClaimRequest {
                expected_revision: factory.revision,
                phase: FactoryPhase::Planning,
                idempotency_key: format!("claim-plan-{}", factory.revision),
            },
            now,
        )
        .await
        .unwrap();
        let claimed = get_run(state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        factory_complete_phase(
            state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-plan",
            worker_context(
                &claimed,
                &claim,
                &format!("complete-plan-{}", factory.revision),
            ),
            planning_completion(),
            now,
        )
        .await
        .unwrap();
        let awaiting = get_run(state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let workflow = awaiting.factory.as_ref().unwrap();
        factory_decide_plan(
            state,
            &run.id,
            workflow.revision,
            &workflow.plan.as_ref().unwrap().revision,
            FactoryPlanDecision::Approve,
            now,
        )
        .await
        .unwrap()
    }

    async fn advance_factory_to_validation(
        state: &AppState,
        run: &ExpertRun,
        worker: &str,
        head: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> ExpertRun {
        let workflow = run.factory.as_ref().unwrap();
        let claim = factory_claim_phase(
            state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            worker,
            FactoryClaimRequest {
                expected_revision: workflow.revision,
                phase: FactoryPhase::Build,
                idempotency_key: format!("claim-build-{}", workflow.revision),
            },
            now,
        )
        .await
        .unwrap();
        let claimed = get_run(state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        factory_complete_phase(
            state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            worker,
            worker_context(
                &claimed,
                &claim,
                &format!("complete-build-{}", workflow.revision),
            ),
            FactoryPhaseCompletion::Build {
                head_commit: head.into(),
            },
            now,
        )
        .await
        .unwrap();
        get_run(state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap()
    }

    async fn advance_factory_to_independent_review(
        state: &AppState,
        run: &ExpertRun,
        head: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> ExpertRun {
        let validation =
            advance_factory_to_validation(state, run, "codex/session-build", head, now).await;
        let validation_claim = factory_claim_phase(
            state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validate",
            FactoryClaimRequest {
                expected_revision: validation.factory.as_ref().unwrap().revision,
                phase: FactoryPhase::Validation,
                idempotency_key: format!("claim-validation-{}", run.id),
            },
            now,
        )
        .await
        .unwrap();
        let claimed = get_run(state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        factory_submit_evidence(
            state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validate",
            worker_context(
                &claimed,
                &validation_claim,
                &format!("tests-pass-{}", run.id),
            ),
            FactoryEvidenceInput {
                check_name: "tests".into(),
                result: EvidenceResult::Pass,
                command_label: Some("cargo test".into()),
                exit_code: Some(0),
                summary: "All required tests passed for the exact head.".into(),
                artifact_ids: Vec::new(),
            },
            now,
        )
        .await
        .unwrap();
        let evidenced = get_run(state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let current_claim = evidenced
            .factory
            .as_ref()
            .unwrap()
            .current_claim
            .as_ref()
            .unwrap();
        factory_complete_phase(
            state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validate",
            worker_context(
                &evidenced,
                current_claim,
                &format!("complete-validation-{}", run.id),
            ),
            FactoryPhaseCompletion::Validation,
            now,
        )
        .await
        .unwrap();
        get_run(state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap()
    }

    #[test]
    fn legacy_run_deserializes_without_factory_workflow() {
        let value = serde_json::json!({
            "id": uuid::Uuid::new_v4().to_string(),
            "expertId": "reviewer",
            "expertVersion": 1,
            "projectPath": "/tmp/project",
            "client": "codex",
            "leadAgent": "reviewer",
            "supportingAgents": [],
            "requiredSkills": [],
            "optionalSkills": [],
            "runbook": null,
            "contract": { "version": 1, "checks": [] },
            "state": "inProgress",
            "startedAt": "2026-08-18T10:00:00Z",
            "endedAt": null,
            "evidence": [],
            "blockers": [],
            "waivers": []
        });

        let run: ExpertRun = serde_json::from_value(value).unwrap();
        assert!(run.factory.is_none());
        assert!(serde_json::to_value(run).unwrap().get("factory").is_none());
    }

    #[test]
    fn factory_workflow_round_trips_exactly() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-18T10:00:00Z")
            .unwrap()
            .to_utc();
        let create = factory_expert_create();
        let workflow =
            prepare_factory_workflow(&create, factory_create(&now.to_rfc3339()), now).unwrap();
        let run = ExpertRun {
            id: uuid::Uuid::new_v4().to_string(),
            snapshot: create,
            state: ExpertRunState::InProgress,
            started_at: now.to_rfc3339(),
            ended_at: None,
            evidence: Vec::new(),
            blockers: Vec::new(),
            waivers: Vec::new(),
            factory: Some(workflow),
        };

        validate_runs(std::slice::from_ref(&run)).unwrap();
        let encoded = serde_json::to_vec(&run).unwrap();
        let decoded: ExpertRun = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, run);
    }

    #[test]
    fn factory_work_order_must_be_bounded_fresh_and_ready() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-18T10:00:00Z")
            .unwrap()
            .to_utc();
        let create = factory_expert_create();

        let mut oversized = factory_create(&now.to_rfc3339());
        oversized.title = "x".repeat(MAX_FACTORY_TITLE + 1);
        assert!(prepare_factory_workflow(&create, oversized, now).is_err());

        let mut stale = factory_create("2026-08-18T09:54:59Z");
        assert!(prepare_factory_workflow(&create, stale.clone(), now).is_err());

        stale.readiness.checked_at = now.to_rfc3339();
        stale.readiness.overall = FactoryReadinessOverall::NeedsAttention;
        assert!(prepare_factory_workflow(&create, stale, now).is_err());

        let mut unbound_workspace_pack = factory_create(&now.to_rfc3339());
        unbound_workspace_pack.workspace_pack_revision = Some("free-text-pack-v2".into());
        assert!(prepare_factory_workflow(&create, unbound_workspace_pack, now).is_err());

        let mut secret_bearing = factory_create(&now.to_rfc3339());
        secret_bearing.objective = "Use AWS_SECRET_ACCESS_KEY=secret-value".into();
        assert!(prepare_factory_workflow(&create, secret_bearing, now).is_err());

        let mut no_configuration = factory_create(&now.to_rfc3339());
        no_configuration.playbook = None;
        let mut no_runbook = create;
        no_runbook.runbook = None;
        assert!(prepare_factory_workflow(&no_runbook, no_configuration, now).is_err());
    }

    #[tokio::test]
    async fn factory_persisted_plan_and_work_contract_digest_drift_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.path().to_path_buf();
        let now = factory_now("2026-08-18T10:00:00Z");
        let run = create_factory_test_run(&state, now).await;
        let claim = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-drift-plan",
            FactoryClaimRequest {
                expected_revision: 1,
                phase: FactoryPhase::Planning,
                idempotency_key: "drift-claim-plan".into(),
            },
            now,
        )
        .await
        .unwrap();
        let claimed = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        factory_complete_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-drift-plan",
            worker_context(&claimed, &claim, "drift-complete-plan"),
            planning_completion(),
            now,
        )
        .await
        .unwrap();
        let awaiting = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        validate_runs(std::slice::from_ref(&awaiting)).unwrap();

        let mut contract_drift = awaiting.clone();
        contract_drift
            .factory
            .as_mut()
            .unwrap()
            .work_contract
            .objective
            .push_str(" Changed after hashing.");
        assert!(validate_runs(std::slice::from_ref(&contract_drift)).is_err());

        let mut plan_drift = awaiting;
        plan_drift
            .factory
            .as_mut()
            .unwrap()
            .plan
            .as_mut()
            .unwrap()
            .content
            .push_str("\n3. Unreviewed plan mutation.");
        assert!(validate_runs(std::slice::from_ref(&plan_drift)).is_err());
    }

    #[tokio::test]
    async fn factory_persisted_claim_evidence_and_idempotency_trust_records_are_validated() {
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.path().to_path_buf();
        let now = factory_now("2026-08-18T10:00:00Z");
        let run = create_factory_test_run(&state, now).await;
        let build = advance_factory_to_build(&state, &run, now).await;
        let validation = advance_factory_to_validation(
            &state,
            &build,
            "codex/session-persisted-build",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            now,
        )
        .await;
        let claim = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-persisted-validation",
            FactoryClaimRequest {
                expected_revision: validation.factory.as_ref().unwrap().revision,
                phase: FactoryPhase::Validation,
                idempotency_key: "persisted-claim-validation".into(),
            },
            now,
        )
        .await
        .unwrap();
        let claimed = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        factory_submit_evidence(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-persisted-validation",
            worker_context(&claimed, &claim, "persisted-tests-pass"),
            FactoryEvidenceInput {
                check_name: "tests".into(),
                result: EvidenceResult::Pass,
                command_label: Some("cargo test".into()),
                exit_code: Some(0),
                summary: "Persisted test evidence passed.".into(),
                artifact_ids: Vec::new(),
            },
            now,
        )
        .await
        .unwrap();
        let evidenced = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        validate_runs(std::slice::from_ref(&evidenced)).unwrap();

        let mut bad_attempt = evidenced.clone();
        bad_attempt.factory.as_mut().unwrap().attempts[0].ended_at = Some(factory_timestamp(now));
        assert!(validate_runs(std::slice::from_ref(&bad_attempt)).is_err());

        let mut bad_blocker = evidenced.clone();
        let blocker_claim = bad_blocker
            .factory
            .as_ref()
            .unwrap()
            .current_claim
            .as_ref()
            .unwrap()
            .clone();
        bad_blocker
            .factory
            .as_mut()
            .unwrap()
            .blockers
            .push(FactoryBlocker {
                id: uuid::Uuid::new_v4().to_string(),
                run_id: uuid::Uuid::new_v4().to_string(),
                idempotency_key: "tampered-blocker".into(),
                claim_id: blocker_claim.id,
                claim_generation: blocker_claim.generation,
                kind: "access".into(),
                summary: "Waiting for bounded access".into(),
                phase: blocker_claim.phase,
                attempt: 1,
                reported_by: blocker_claim.worker_identity,
                reported_at: factory_timestamp(now),
                resolved_at: None,
            });
        assert!(validate_runs(std::slice::from_ref(&bad_blocker)).is_err());

        let mut bad_claim = evidenced.clone();
        bad_claim
            .factory
            .as_mut()
            .unwrap()
            .current_claim
            .as_mut()
            .unwrap()
            .worker_identity
            .clear();
        assert!(validate_runs(std::slice::from_ref(&bad_claim)).is_err());

        let mut bad_evidence = evidenced.clone();
        bad_evidence.factory.as_mut().unwrap().evidence[0].exit_code = Some(1);
        assert!(validate_runs(std::slice::from_ref(&bad_evidence)).is_err());

        let mut bad_idempotency = evidenced;
        bad_idempotency.factory.as_mut().unwrap().idempotency[0].request_digest =
            "not-a-digest".into();
        assert!(validate_runs(std::slice::from_ref(&bad_idempotency)).is_err());
    }

    #[tokio::test]
    async fn factory_phase_edges_are_revision_bound_and_terminal_runs_freeze() {
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.path().to_path_buf();
        let now = factory_now("2026-08-18T10:00:00Z");
        let run = create_factory_test_run(&state, now).await;
        assert_eq!(run.factory.as_ref().unwrap().phase, FactoryPhase::Planning);
        assert_eq!(run.factory.as_ref().unwrap().revision, 1);

        let claim = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-plan",
            FactoryClaimRequest {
                expected_revision: 1,
                phase: FactoryPhase::Planning,
                idempotency_key: "claim-plan".into(),
            },
            now,
        )
        .await
        .unwrap();
        let claimed = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let context = worker_context(&claimed, &claim, "complete-plan");
        let receipt = factory_complete_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-plan",
            context.clone(),
            planning_completion(),
            now,
        )
        .await
        .unwrap();
        assert_eq!(receipt.phase, FactoryPhase::AwaitingPlanApproval);
        assert_eq!(receipt.revision, 3);

        let retry = factory_complete_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-plan",
            context,
            planning_completion(),
            now,
        )
        .await
        .unwrap();
        assert_eq!(retry, receipt);

        let awaiting = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let plan_revision = awaiting
            .factory
            .as_ref()
            .unwrap()
            .plan
            .as_ref()
            .unwrap()
            .revision
            .clone();
        assert!(factory_decide_plan(
            &state,
            &run.id,
            2,
            &plan_revision,
            FactoryPlanDecision::Approve,
            now,
        )
        .await
        .is_err());
        let build = factory_decide_plan(
            &state,
            &run.id,
            3,
            &plan_revision,
            FactoryPlanDecision::Approve,
            now,
        )
        .await
        .unwrap();
        assert_eq!(build.factory.as_ref().unwrap().phase, FactoryPhase::Build);
        assert_eq!(build.factory.as_ref().unwrap().attempts[0].number, 1);

        let cancelled = factory_cancel(
            &state,
            &run.id,
            build.factory.as_ref().unwrap().revision,
            Some("User stopped the control-plane run".into()),
            now,
        )
        .await
        .unwrap();
        assert_eq!(cancelled.state, ExpertRunState::Cancelled);
        assert_eq!(
            cancelled.factory.as_ref().unwrap().phase,
            FactoryPhase::Completed
        );
        assert!(factory_cancel(
            &state,
            &run.id,
            cancelled.factory.as_ref().unwrap().revision,
            None,
            now,
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn factory_blocker_pauses_and_exact_desktop_resolution_resumes_the_phase() {
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.path().to_path_buf();
        let now = factory_now("2026-08-18T10:00:00Z");
        let run = create_factory_test_run(&state, now).await;
        let claim = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-plan",
            FactoryClaimRequest {
                expected_revision: 1,
                phase: FactoryPhase::Planning,
                idempotency_key: "claim-plan".into(),
            },
            now,
        )
        .await
        .unwrap();
        let claimed = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let blocker = factory_submit_blocker(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-plan",
            worker_context(&claimed, &claim, "blocker-1"),
            FactoryBlockerInput {
                kind: "access".into(),
                summary: "Waiting for the bounded project credential.".into(),
            },
            now,
        )
        .await
        .unwrap();
        let blocked = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        assert_eq!(
            blocked.factory.as_ref().unwrap().phase,
            FactoryPhase::Planning
        );
        assert!(factory_complete_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-plan",
            worker_context(
                &blocked,
                blocked
                    .factory
                    .as_ref()
                    .unwrap()
                    .current_claim
                    .as_ref()
                    .unwrap(),
                "complete-blocked"
            ),
            planning_completion(),
            now,
        )
        .await
        .is_err());
        assert!(factory_resolve_blocker(
            &state,
            &run.id,
            blocked.factory.as_ref().unwrap().revision,
            "unknown-blocker",
            now,
        )
        .await
        .is_err());
        let resumed = factory_resolve_blocker(
            &state,
            &run.id,
            blocked.factory.as_ref().unwrap().revision,
            &blocker.id,
            now,
        )
        .await
        .unwrap();
        assert_eq!(
            resumed.factory.as_ref().unwrap().phase,
            FactoryPhase::Planning
        );
        assert!(resumed.factory.as_ref().unwrap().current_claim.is_none());
        assert!(resumed.factory.as_ref().unwrap().blockers[0]
            .resolved_at
            .is_some());
    }

    #[tokio::test]
    async fn factory_claims_are_exclusive_idempotent_expiring_and_generation_bound() {
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.path().to_path_buf();
        let now = factory_now("2026-08-18T10:00:00Z");
        let run = create_factory_test_run(&state, now).await;
        let request = FactoryClaimRequest {
            expected_revision: 1,
            phase: FactoryPhase::Planning,
            idempotency_key: "claim-once".into(),
        };
        let first = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-a",
            request.clone(),
            now,
        )
        .await
        .unwrap();
        let retry = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-a",
            request,
            now,
        )
        .await
        .unwrap();
        assert_eq!(retry.id, first.id);
        assert!(factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-b",
            FactoryClaimRequest {
                expected_revision: 1,
                phase: FactoryPhase::Planning,
                idempotency_key: "claim-once".into(),
            },
            now,
        )
        .await
        .is_err());

        let expired_at = now + chrono::Duration::seconds(FACTORY_CLAIM_LEASE_SECONDS + 1);
        let second = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-b",
            FactoryClaimRequest {
                expected_revision: 2,
                phase: FactoryPhase::Planning,
                idempotency_key: "claim-after-expiry".into(),
            },
            expired_at,
        )
        .await
        .unwrap();
        assert_eq!(second.generation, first.generation + 1);
        let current = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let stale_context = FactoryWorkerContext {
            expected_revision: current.factory.as_ref().unwrap().revision,
            phase: FactoryPhase::Planning,
            attempt: 0,
            claim_id: first.id,
            claim_generation: first.generation,
            work_contract_revision: current
                .factory
                .as_ref()
                .unwrap()
                .work_contract_revision
                .clone(),
            approved_plan_revision: None,
            base_commit: None,
            head_commit: None,
            idempotency_key: "stale-blocker".into(),
        };
        assert!(factory_submit_blocker(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-a",
            stale_context,
            FactoryBlockerInput {
                kind: "stale".into(),
                summary: "This expired claimant cannot report.".into(),
            },
            expired_at,
        )
        .await
        .is_err());
        let released = factory_release_claim(
            &state,
            &run.id,
            current.factory.as_ref().unwrap().revision,
            expired_at,
        )
        .await
        .unwrap();
        assert!(released.factory.as_ref().unwrap().current_claim.is_none());
    }

    #[tokio::test]
    async fn factory_claim_idempotency_key_conflicts_across_runs_and_projects() {
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.path().to_path_buf();
        let now = factory_now("2026-08-18T10:00:00Z");
        let first = create_factory_test_run(&state, now).await;
        factory_claim_phase(
            &state,
            &first.id,
            "codex",
            "/tmp/factory-project",
            "codex/shared-key-session",
            FactoryClaimRequest {
                expected_revision: 1,
                phase: FactoryPhase::Planning,
                idempotency_key: "globally-unique-claim".into(),
            },
            now,
        )
        .await
        .unwrap();

        let mut second_create = factory_expert_create();
        second_create.project_path = "/tmp/other-factory-project".into();
        let second = create_factory_run_with_id_at(
            &state,
            &uuid::Uuid::new_v4().to_string(),
            second_create,
            factory_create(&now.to_rfc3339()),
            now,
        )
        .await
        .unwrap();
        assert!(factory_claim_phase(
            &state,
            &second.id,
            "codex",
            "/tmp/other-factory-project",
            "codex/shared-key-session",
            FactoryClaimRequest {
                expected_revision: 1,
                phase: FactoryPhase::Planning,
                idempotency_key: "globally-unique-claim".into(),
            },
            now,
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn factory_successful_submission_renews_claim_for_exactly_two_hours() {
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.path().to_path_buf();
        let now = factory_now("2026-08-18T10:00:00Z");
        let submitted_at = now + chrono::Duration::minutes(37);
        let run = create_factory_test_run(&state, now).await;
        let claim = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-renew",
            FactoryClaimRequest {
                expected_revision: 1,
                phase: FactoryPhase::Planning,
                idempotency_key: "claim-renew".into(),
            },
            now,
        )
        .await
        .unwrap();
        let claimed = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        factory_submit_blocker(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-renew",
            worker_context(&claimed, &claim, "renew-with-blocker"),
            FactoryBlockerInput {
                kind: "dependency".into(),
                summary: "A bounded dependency needs user input.".into(),
            },
            submitted_at,
        )
        .await
        .unwrap();

        let renewed = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let workflow = renewed.factory.as_ref().unwrap();
        let renewed_claim = workflow.current_claim.as_ref().unwrap();
        assert_eq!(
            renewed_claim.last_renewed_at,
            factory_timestamp(submitted_at)
        );
        assert_eq!(
            renewed_claim.expires_at,
            factory_timestamp(
                submitted_at + chrono::Duration::seconds(FACTORY_CLAIM_LEASE_SECONDS)
            )
        );
        assert_eq!(renewed_claim.run_revision, workflow.revision);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn factory_same_revision_plan_race_advances_exactly_once() {
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.path().to_path_buf();
        let state = std::sync::Arc::new(state);
        let now = factory_now("2026-08-18T10:00:00Z");
        let run = create_factory_test_run(&state, now).await;
        let claim = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-race-plan",
            FactoryClaimRequest {
                expected_revision: 1,
                phase: FactoryPhase::Planning,
                idempotency_key: "race-claim-plan".into(),
            },
            now,
        )
        .await
        .unwrap();
        let claimed = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        factory_complete_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-race-plan",
            worker_context(&claimed, &claim, "race-complete-plan"),
            planning_completion(),
            now,
        )
        .await
        .unwrap();
        let awaiting = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let workflow = awaiting.factory.as_ref().unwrap();
        let expected_revision = workflow.revision;
        let plan_revision = workflow.plan.as_ref().unwrap().revision.clone();

        let left_state = std::sync::Arc::clone(&state);
        let left_id = run.id.clone();
        let left_plan = plan_revision.clone();
        let right_state = std::sync::Arc::clone(&state);
        let right_id = run.id.clone();
        let right_plan = plan_revision;
        let (left, right) = tokio::join!(
            tokio::spawn(async move {
                factory_decide_plan(
                    &left_state,
                    &left_id,
                    expected_revision,
                    &left_plan,
                    FactoryPlanDecision::Approve,
                    now,
                )
                .await
            }),
            tokio::spawn(async move {
                factory_decide_plan(
                    &right_state,
                    &right_id,
                    expected_revision,
                    &right_plan,
                    FactoryPlanDecision::Approve,
                    now,
                )
                .await
            }),
        );
        let outcomes = [left.unwrap(), right.unwrap()];
        assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(outcomes.iter().filter(|result| result.is_err()).count(), 1);

        let stored = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let stored = stored.factory.as_ref().unwrap();
        assert_eq!(stored.phase, FactoryPhase::Build);
        assert_eq!(stored.revision, expected_revision + 1);
        assert_eq!(stored.attempts.len(), 1, "the race must not double-advance");
    }

    #[tokio::test]
    async fn factory_cancellation_revokes_new_work_but_preserves_exact_idempotent_retries() {
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.path().to_path_buf();
        let now = factory_now("2026-08-18T10:00:00Z");
        let run = create_factory_test_run(&state, now).await;
        let build = advance_factory_to_build(&state, &run, now).await;
        let claim_request = FactoryClaimRequest {
            expected_revision: build.factory.as_ref().unwrap().revision,
            phase: FactoryPhase::Build,
            idempotency_key: "cancel-claim-build".into(),
        };
        let claim = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-cancelled-build",
            claim_request.clone(),
            now,
        )
        .await
        .unwrap();
        let claimed = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let artifact_context = worker_context(&claimed, &claim, "cancel-retained-artifact");
        let artifact_input = FactoryArtifactInput {
            kind: "testReport".into(),
            label: "Intermediate test report".into(),
            reference: "urn:factory:cancel-retained-artifact".into(),
            digest: "a".repeat(64),
            byte_size: 128,
            summary: "Bounded intermediate artifact metadata.".into(),
        };
        let artifact = factory_submit_artifact(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-cancelled-build",
            artifact_context.clone(),
            artifact_input.clone(),
            now,
        )
        .await
        .unwrap();
        let with_artifact = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let blocker_context = worker_context(
            &with_artifact,
            with_artifact
                .factory
                .as_ref()
                .unwrap()
                .current_claim
                .as_ref()
                .unwrap(),
            "cancel-retained-blocker",
        );
        let blocker_input = FactoryBlockerInput {
            kind: "dependency".into(),
            summary: "An external dependency is temporarily unavailable.".into(),
        };
        let blocker = factory_submit_blocker(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-cancelled-build",
            blocker_context.clone(),
            blocker_input.clone(),
            now,
        )
        .await
        .unwrap();
        let with_blocker = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let evidence_context = worker_context(
            &with_blocker,
            with_blocker
                .factory
                .as_ref()
                .unwrap()
                .current_claim
                .as_ref()
                .unwrap(),
            "cancel-retained-evidence",
        );
        let evidence_input = FactoryEvidenceInput {
            check_name: "tests".into(),
            result: EvidenceResult::Pass,
            command_label: Some("cargo test".into()),
            exit_code: Some(0),
            summary: "The external worker reported a passing intermediate check.".into(),
            artifact_ids: vec![artifact.id.clone()],
        };
        let evidence = factory_submit_evidence(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-cancelled-build",
            evidence_context.clone(),
            evidence_input.clone(),
            now + chrono::Duration::minutes(1),
        )
        .await
        .unwrap();
        let before_cancel = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let stale_context = worker_context(
            &before_cancel,
            before_cancel
                .factory
                .as_ref()
                .unwrap()
                .current_claim
                .as_ref()
                .unwrap(),
            "cancelled-worker-retry",
        );
        let cancel_time = now + chrono::Duration::minutes(2);
        let cancelled = factory_cancel(
            &state,
            &run.id,
            before_cancel.factory.as_ref().unwrap().revision,
            Some("Control-plane authority revoked; external work may still be running.".into()),
            cancel_time,
        )
        .await
        .unwrap();
        let workflow = cancelled.factory.as_ref().unwrap();
        assert_eq!(cancelled.state, ExpertRunState::Cancelled);
        assert!(workflow.current_claim.is_none());
        assert_eq!(
            workflow.evidence,
            before_cancel.factory.as_ref().unwrap().evidence
        );
        assert_eq!(
            workflow.idempotency,
            before_cancel.factory.as_ref().unwrap().idempotency
        );
        let released = workflow.prior_claims.last().unwrap();
        assert_eq!(released.id, claim.id);
        assert_eq!(
            released.released_at.as_deref(),
            Some(factory_timestamp(cancel_time).as_str())
        );

        assert_eq!(
            factory_claim_phase(
                &state,
                &run.id,
                "codex",
                "/tmp/factory-project",
                "codex/session-cancelled-build",
                claim_request,
                cancel_time,
            )
            .await
            .unwrap(),
            claim
        );
        assert_eq!(
            factory_submit_artifact(
                &state,
                &run.id,
                "codex",
                "/tmp/factory-project",
                "codex/session-cancelled-build",
                artifact_context,
                artifact_input,
                cancel_time,
            )
            .await
            .unwrap(),
            artifact
        );
        assert_eq!(
            factory_submit_blocker(
                &state,
                &run.id,
                "codex",
                "/tmp/factory-project",
                "codex/session-cancelled-build",
                blocker_context,
                blocker_input,
                cancel_time,
            )
            .await
            .unwrap(),
            blocker
        );
        assert_eq!(
            factory_submit_evidence(
                &state,
                &run.id,
                "codex",
                "/tmp/factory-project",
                "codex/session-cancelled-build",
                evidence_context,
                evidence_input,
                cancel_time,
            )
            .await
            .unwrap(),
            evidence
        );

        assert!(factory_submit_evidence(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-cancelled-build",
            stale_context,
            FactoryEvidenceInput {
                check_name: "tests".into(),
                result: EvidenceResult::Pass,
                command_label: Some("cargo test".into()),
                exit_code: Some(0),
                summary: "This stale report must not change the terminal run.".into(),
                artifact_ids: Vec::new(),
            },
            cancel_time,
        )
        .await
        .is_err());
        assert_eq!(
            get_run(&state, &run.id, "codex", "/tmp/factory-project")
                .await
                .unwrap(),
            cancelled
        );
        validate_runs(std::slice::from_ref(&cancelled)).unwrap();

        let planning = create_factory_test_run(&state, now).await;
        let cancelled_planning = factory_cancel(
            &state,
            &planning.id,
            planning.factory.as_ref().unwrap().revision,
            None,
            now,
        )
        .await
        .unwrap();
        validate_runs(std::slice::from_ref(&cancelled_planning)).unwrap();
    }

    #[tokio::test]
    async fn factory_evidence_is_bound_idempotent_and_latest_result_wins() {
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.path().to_path_buf();
        let now = factory_now("2026-08-18T10:00:00Z");
        let run = create_factory_test_run(&state, now).await;
        let build = advance_factory_to_build(&state, &run, now).await;
        let validation = advance_factory_to_validation(
            &state,
            &build,
            "codex/session-build",
            "1111111111111111111111111111111111111111",
            now,
        )
        .await;
        let workflow = validation.factory.as_ref().unwrap();
        let claim = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validate",
            FactoryClaimRequest {
                expected_revision: workflow.revision,
                phase: FactoryPhase::Validation,
                idempotency_key: "claim-validation".into(),
            },
            now,
        )
        .await
        .unwrap();
        let claimed = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let artifact_context = worker_context(&claimed, &claim, "artifact-tests");
        let artifact_input = FactoryArtifactInput {
            kind: "testReport".into(),
            label: "Cargo test summary".into(),
            reference: "urn:factory:test-report:42".into(),
            digest: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            byte_size: 1200,
            summary: "Bounded test report metadata".into(),
        };
        let artifact = factory_submit_artifact(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validate",
            artifact_context.clone(),
            artifact_input.clone(),
            now,
        )
        .await
        .unwrap();
        let artifact_retry = factory_submit_artifact(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validate",
            artifact_context,
            artifact_input,
            now,
        )
        .await
        .unwrap();
        assert_eq!(artifact_retry.id, artifact.id);

        let after_artifact = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        assert!(factory_submit_evidence(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validate",
            worker_context(
                &after_artifact,
                after_artifact
                    .factory
                    .as_ref()
                    .unwrap()
                    .current_claim
                    .as_ref()
                    .unwrap(),
                "bad-pass",
            ),
            FactoryEvidenceInput {
                check_name: "tests".into(),
                result: EvidenceResult::Pass,
                command_label: Some("cargo test".into()),
                exit_code: Some(1),
                summary: "Command failed".into(),
                artifact_ids: vec![artifact.id.clone()],
            },
            now,
        )
        .await
        .is_err());
        let pass = factory_submit_evidence(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validate",
            worker_context(
                &after_artifact,
                after_artifact
                    .factory
                    .as_ref()
                    .unwrap()
                    .current_claim
                    .as_ref()
                    .unwrap(),
                "tests-pass",
            ),
            FactoryEvidenceInput {
                check_name: "tests".into(),
                result: EvidenceResult::Pass,
                command_label: Some("cargo test".into()),
                exit_code: Some(0),
                summary: "Tests passed".into(),
                artifact_ids: vec![artifact.id],
            },
            now,
        )
        .await
        .unwrap();
        assert_eq!(pass.run_id, run.id);
        let after_pass = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        factory_submit_evidence(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validate",
            worker_context(
                &after_pass,
                after_pass
                    .factory
                    .as_ref()
                    .unwrap()
                    .current_claim
                    .as_ref()
                    .unwrap(),
                "tests-late-fail",
            ),
            FactoryEvidenceInput {
                check_name: "tests".into(),
                result: EvidenceResult::Fail,
                command_label: Some("cargo test".into()),
                exit_code: Some(1),
                summary: "A later rerun failed".into(),
                artifact_ids: Vec::new(),
            },
            now,
        )
        .await
        .unwrap();
        let failed = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let failed_claim = failed
            .factory
            .as_ref()
            .unwrap()
            .current_claim
            .as_ref()
            .unwrap();
        let receipt = factory_complete_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validate",
            worker_context(&failed, failed_claim, "complete-failed-validation"),
            FactoryPhaseCompletion::Validation,
            now,
        )
        .await
        .unwrap();
        assert_eq!(receipt.phase, FactoryPhase::Build);
        let rework = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        assert_eq!(rework.factory.as_ref().unwrap().attempts.len(), 2);
    }

    #[tokio::test]
    async fn factory_independent_review_rejects_the_build_worker_and_rework_advances_attempt() {
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.path().to_path_buf();
        let now = factory_now("2026-08-18T10:00:00Z");
        let run = create_factory_test_run(&state, now).await;
        let build = advance_factory_to_build(&state, &run, now).await;
        let validation = advance_factory_to_validation(
            &state,
            &build,
            "codex/session-build",
            "2222222222222222222222222222222222222222",
            now,
        )
        .await;
        let validation_workflow = validation.factory.as_ref().unwrap();
        let validation_claim = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validate",
            FactoryClaimRequest {
                expected_revision: validation_workflow.revision,
                phase: FactoryPhase::Validation,
                idempotency_key: "claim-validation-review-test".into(),
            },
            now,
        )
        .await
        .unwrap();
        let claimed = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        factory_submit_evidence(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validate",
            worker_context(&claimed, &validation_claim, "review-test-pass"),
            FactoryEvidenceInput {
                check_name: "tests".into(),
                result: EvidenceResult::Pass,
                command_label: Some("cargo test".into()),
                exit_code: Some(0),
                summary: "Tests passed".into(),
                artifact_ids: Vec::new(),
            },
            now,
        )
        .await
        .unwrap();
        let evidenced = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let evidence_claim = evidenced
            .factory
            .as_ref()
            .unwrap()
            .current_claim
            .as_ref()
            .unwrap();
        factory_complete_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validate",
            worker_context(
                &evidenced,
                evidence_claim,
                "complete-validation-review-test",
            ),
            FactoryPhaseCompletion::Validation,
            now,
        )
        .await
        .unwrap();
        let review = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        assert!(factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-build",
            FactoryClaimRequest {
                expected_revision: review.factory.as_ref().unwrap().revision,
                phase: FactoryPhase::IndependentReview,
                idempotency_key: "self-review".into(),
            },
            now,
        )
        .await
        .is_err());
        let reviewer_claim = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-review",
            FactoryClaimRequest {
                expected_revision: review.factory.as_ref().unwrap().revision,
                phase: FactoryPhase::IndependentReview,
                idempotency_key: "distinct-review".into(),
            },
            now,
        )
        .await
        .unwrap();
        let claimed_review = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let receipt = factory_complete_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-review",
            worker_context(&claimed_review, &reviewer_claim, "review-rework"),
            FactoryPhaseCompletion::IndependentReview {
                review: FactoryReviewInput {
                    verdict: FactoryReviewVerdict::Rework,
                    summary: "One high-severity issue remains.".into(),
                    findings: vec![FactoryReviewFinding {
                        severity: FactoryReviewSeverity::High,
                        summary: "The stale binding path needs a regression test.".into(),
                    }],
                },
            },
            now,
        )
        .await
        .unwrap();
        assert_eq!(receipt.phase, FactoryPhase::Build);
        let rework = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        assert_eq!(rework.factory.as_ref().unwrap().attempts.len(), 2);
    }

    #[tokio::test]
    async fn factory_late_review_evidence_failure_overrides_validation_pass() {
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.path().to_path_buf();
        let now = factory_now("2026-08-18T10:00:00Z");
        let run = create_factory_test_run(&state, now).await;
        let build = advance_factory_to_build(&state, &run, now).await;
        let review = advance_factory_to_independent_review(
            &state,
            &build,
            "3333333333333333333333333333333333333333",
            now,
        )
        .await;
        let review_claim = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-review",
            FactoryClaimRequest {
                expected_revision: review.factory.as_ref().unwrap().revision,
                phase: FactoryPhase::IndependentReview,
                idempotency_key: "late-fail-claim-review".into(),
            },
            now,
        )
        .await
        .unwrap();
        let claimed = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        factory_submit_evidence(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-review",
            worker_context(&claimed, &review_claim, "late-review-tests-fail"),
            FactoryEvidenceInput {
                check_name: "tests".into(),
                result: EvidenceResult::Fail,
                command_label: Some("cargo test".into()),
                exit_code: Some(1),
                summary: "A later rerun failed during independent review.".into(),
                artifact_ids: Vec::new(),
            },
            now,
        )
        .await
        .unwrap();
        let failed = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let receipt = factory_complete_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-review",
            worker_context(
                &failed,
                failed
                    .factory
                    .as_ref()
                    .unwrap()
                    .current_claim
                    .as_ref()
                    .unwrap(),
                "late-review-complete",
            ),
            FactoryPhaseCompletion::IndependentReview {
                review: FactoryReviewInput {
                    verdict: FactoryReviewVerdict::Pass,
                    summary: "The code review itself found no separate issues.".into(),
                    findings: Vec::new(),
                },
            },
            now,
        )
        .await
        .unwrap();
        assert_eq!(receipt.phase, FactoryPhase::Build);
    }

    #[tokio::test]
    async fn factory_late_delivery_evidence_failure_prevents_final_approval() {
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.path().to_path_buf();
        let now = factory_now("2026-08-18T10:00:00Z");
        let run = create_factory_test_run(&state, now).await;
        let build = advance_factory_to_build(&state, &run, now).await;
        let review = advance_factory_to_independent_review(
            &state,
            &build,
            "4444444444444444444444444444444444444444",
            now,
        )
        .await;
        let review_claim = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-review",
            FactoryClaimRequest {
                expected_revision: review.factory.as_ref().unwrap().revision,
                phase: FactoryPhase::IndependentReview,
                idempotency_key: "late-delivery-claim-review".into(),
            },
            now,
        )
        .await
        .unwrap();
        let claimed_review = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        factory_complete_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-review",
            worker_context(
                &claimed_review,
                &review_claim,
                "late-delivery-complete-review",
            ),
            FactoryPhaseCompletion::IndependentReview {
                review: FactoryReviewInput {
                    verdict: FactoryReviewVerdict::Pass,
                    summary: "Independent review passed for the exact head.".into(),
                    findings: Vec::new(),
                },
            },
            now,
        )
        .await
        .unwrap();
        let delivery = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let delivery_claim = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-delivery",
            FactoryClaimRequest {
                expected_revision: delivery.factory.as_ref().unwrap().revision,
                phase: FactoryPhase::Delivery,
                idempotency_key: "late-delivery-claim".into(),
            },
            now,
        )
        .await
        .unwrap();
        let claimed_delivery = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        factory_submit_evidence(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-delivery",
            worker_context(
                &claimed_delivery,
                &delivery_claim,
                "late-delivery-tests-fail",
            ),
            FactoryEvidenceInput {
                check_name: "tests".into(),
                result: EvidenceResult::Fail,
                command_label: Some("cargo test".into()),
                exit_code: Some(1),
                summary: "The delivery rerun failed after review.".into(),
                artifact_ids: Vec::new(),
            },
            now,
        )
        .await
        .unwrap();
        let failed = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let receipt = factory_complete_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-delivery",
            worker_context(
                &failed,
                failed
                    .factory
                    .as_ref()
                    .unwrap()
                    .current_claim
                    .as_ref()
                    .unwrap(),
                "late-delivery-complete",
            ),
            FactoryPhaseCompletion::Delivery {
                delivery: FactoryDeliveryInput {
                    reference: "https://github.com/example/project/pull/44".into(),
                    head_commit: "4444444444444444444444444444444444444444".into(),
                    evidence_summary: "Delivery was prepared, but the latest check failed.".into(),
                    known_limitations: vec!["Execution evidence is client-reported.".into()],
                    improvement_proposal: None,
                },
            },
            now,
        )
        .await
        .unwrap();
        assert_eq!(receipt.phase, FactoryPhase::Build);
    }

    #[tokio::test]
    async fn factory_full_lifecycle_binds_delivery_and_accepts_only_the_exact_final_revision() {
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.path().to_path_buf();
        let now = factory_now("2026-08-18T10:00:00Z");
        let run = create_factory_test_run(&state, now).await;
        let build = advance_factory_to_build(&state, &run, now).await;
        let validation = advance_factory_to_validation(
            &state,
            &build,
            "codex/session-build",
            "3333333333333333333333333333333333333333",
            now,
        )
        .await;
        let validation_claim = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validate",
            FactoryClaimRequest {
                expected_revision: validation.factory.as_ref().unwrap().revision,
                phase: FactoryPhase::Validation,
                idempotency_key: "full-claim-validation".into(),
            },
            now,
        )
        .await
        .unwrap();
        let claimed_validation = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        factory_submit_evidence(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validate",
            worker_context(&claimed_validation, &validation_claim, "full-tests-pass"),
            FactoryEvidenceInput {
                check_name: "tests".into(),
                result: EvidenceResult::Pass,
                command_label: Some("cargo test".into()),
                exit_code: Some(0),
                summary: "All tests passed".into(),
                artifact_ids: Vec::new(),
            },
            now,
        )
        .await
        .unwrap();
        let evidenced = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        factory_complete_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validate",
            worker_context(
                &evidenced,
                evidenced
                    .factory
                    .as_ref()
                    .unwrap()
                    .current_claim
                    .as_ref()
                    .unwrap(),
                "full-complete-validation",
            ),
            FactoryPhaseCompletion::Validation,
            now,
        )
        .await
        .unwrap();
        let review = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let review_claim = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-review",
            FactoryClaimRequest {
                expected_revision: review.factory.as_ref().unwrap().revision,
                phase: FactoryPhase::IndependentReview,
                idempotency_key: "full-claim-review".into(),
            },
            now,
        )
        .await
        .unwrap();
        let claimed_review = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        factory_complete_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-review",
            worker_context(&claimed_review, &review_claim, "full-complete-review"),
            FactoryPhaseCompletion::IndependentReview {
                review: FactoryReviewInput {
                    verdict: FactoryReviewVerdict::Pass,
                    summary: "Independent review passed for the exact head.".into(),
                    findings: Vec::new(),
                },
            },
            now,
        )
        .await
        .unwrap();
        let delivery = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let delivery_claim = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-delivery",
            FactoryClaimRequest {
                expected_revision: delivery.factory.as_ref().unwrap().revision,
                phase: FactoryPhase::Delivery,
                idempotency_key: "full-claim-delivery".into(),
            },
            now,
        )
        .await
        .unwrap();
        let claimed_delivery = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        factory_complete_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-delivery",
            worker_context(&claimed_delivery, &delivery_claim, "full-complete-delivery"),
            FactoryPhaseCompletion::Delivery {
                delivery: FactoryDeliveryInput {
                    reference: "https://github.com/example/project/pull/42".into(),
                    head_commit: "3333333333333333333333333333333333333333".into(),
                    evidence_summary: "Required checks and independent review passed.".into(),
                    known_limitations: vec!["Execution evidence is client-reported.".into()],
                    improvement_proposal: None,
                },
            },
            now,
        )
        .await
        .unwrap();
        let awaiting = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let workflow = awaiting.factory.as_ref().unwrap();
        let input = FactoryFinalDecisionInput {
            expected_revision: workflow.revision,
            outcome: FactoryTerminalOutcome::Accepted,
            approved_plan_revision: workflow
                .plan_approval
                .as_ref()
                .unwrap()
                .plan_revision
                .clone(),
            head_commit: "3333333333333333333333333333333333333333".into(),
            check_waivers: Vec::new(),
            independent_review_waiver_reason: None,
            safe_detail: None,
        };
        validate_runs(std::slice::from_ref(&awaiting)).unwrap();
        let mut bad_validation = awaiting.clone();
        bad_validation
            .factory
            .as_mut()
            .unwrap()
            .validation
            .as_mut()
            .unwrap()
            .claim_generation = 0;
        assert!(validate_runs(std::slice::from_ref(&bad_validation)).is_err());
        let mut bad_review = awaiting.clone();
        bad_review
            .factory
            .as_mut()
            .unwrap()
            .review
            .as_mut()
            .unwrap()
            .head_commit = "9999999999999999999999999999999999999999".into();
        assert!(validate_runs(std::slice::from_ref(&bad_review)).is_err());
        let mut bad_delivery = awaiting.clone();
        bad_delivery
            .factory
            .as_mut()
            .unwrap()
            .delivery
            .as_mut()
            .unwrap()
            .attempt = 0;
        assert!(validate_runs(std::slice::from_ref(&bad_delivery)).is_err());
        let mut bad_waiver = awaiting.clone();
        bad_waiver
            .factory
            .as_mut()
            .unwrap()
            .human_waivers
            .push(FactoryHumanWaiver {
                kind: "unknown".into(),
                check_name: None,
                reason: "Unsupported waiver binding".into(),
                created_at: factory_timestamp(now),
            });
        assert!(validate_runs(std::slice::from_ref(&bad_waiver)).is_err());
        let mut bad_improvement = awaiting.clone();
        bad_improvement
            .factory
            .as_mut()
            .unwrap()
            .improvement_proposal = Some(FactoryImprovementProposal {
            failure_class: "delivery".into(),
            target: FactoryImprovementTarget::Test,
            proposal: "Persist token=unsafe-value".into(),
            suggested_test: None,
            provenance: FactoryProvenance::ClientReported,
        });
        assert!(validate_runs(std::slice::from_ref(&bad_improvement)).is_err());
        let mut stale = input.clone();
        stale.expected_revision -= 1;
        assert!(factory_decide_final(&state, &run.id, stale, now)
            .await
            .is_err());
        let accepted = factory_decide_final(&state, &run.id, input, now)
            .await
            .unwrap();
        assert_eq!(accepted.state, ExpertRunState::Accepted);
        assert_eq!(
            accepted
                .factory
                .as_ref()
                .unwrap()
                .terminal
                .as_ref()
                .unwrap()
                .outcome,
            FactoryTerminalOutcome::Accepted
        );
        validate_runs(std::slice::from_ref(&accepted)).unwrap();
        for field in ["plan", "validation", "review", "delivery"] {
            let mut incomplete = accepted.clone();
            let workflow = incomplete.factory.as_mut().unwrap();
            match field {
                "plan" => workflow.plan_approval = None,
                "validation" => workflow.validation = None,
                "review" => workflow.review = None,
                "delivery" => workflow.delivery = None,
                _ => unreachable!(),
            }
            assert!(
                validate_runs(std::slice::from_ref(&incomplete)).is_err(),
                "accepted terminal must reject a missing {field} binding"
            );
        }
        for (outcome, state) in [
            (FactoryTerminalOutcome::Rework, ExpertRunState::Rework),
            (FactoryTerminalOutcome::Rejected, ExpertRunState::Rejected),
        ] {
            let mut exact = accepted.clone();
            exact.state = state;
            exact
                .factory
                .as_mut()
                .unwrap()
                .terminal
                .as_mut()
                .unwrap()
                .outcome = outcome;
            validate_runs(std::slice::from_ref(&exact)).unwrap();
            exact.factory.as_mut().unwrap().delivery = None;
            assert!(validate_runs(std::slice::from_ref(&exact)).is_err());
        }
        let mut bad_terminal = accepted;
        bad_terminal
            .factory
            .as_mut()
            .unwrap()
            .terminal
            .as_mut()
            .unwrap()
            .outcome = FactoryTerminalOutcome::Cancelled;
        assert!(validate_runs(std::slice::from_ref(&bad_terminal)).is_err());
    }

    #[tokio::test]
    async fn factory_review_and_missing_check_waivers_are_explicit_desktop_decisions() {
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.path().to_path_buf();
        let now = factory_now("2026-08-18T10:00:00Z");
        let run = create_factory_test_run(&state, now).await;
        let build = advance_factory_to_build(&state, &run, now).await;
        let validation = advance_factory_to_validation(
            &state,
            &build,
            "codex/session-build",
            "4444444444444444444444444444444444444444",
            now,
        )
        .await;
        let claim = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validate",
            FactoryClaimRequest {
                expected_revision: validation.factory.as_ref().unwrap().revision,
                phase: FactoryPhase::Validation,
                idempotency_key: "waiver-claim-validation".into(),
            },
            now,
        )
        .await
        .unwrap();
        let claimed = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        factory_complete_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validate",
            worker_context(&claimed, &claim, "waiver-complete-validation"),
            FactoryPhaseCompletion::Validation,
            now,
        )
        .await
        .unwrap();
        let review = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let waived = factory_waive_independent_review(
            &state,
            &run.id,
            review.factory.as_ref().unwrap().revision,
            "No distinct worker session is available; user reviewed the exact head.",
            now,
        )
        .await
        .unwrap();
        assert_eq!(
            waived.factory.as_ref().unwrap().phase,
            FactoryPhase::Delivery
        );
        assert_eq!(
            waived.factory.as_ref().unwrap().human_waivers[0].kind,
            "independentReview"
        );
        let mcp = mcp_view(&waived);
        let factory = mcp["factory"].as_object().expect("Factory MCP status");
        assert_eq!(factory["runId"], waived.id);
        assert_eq!(factory["phase"], "delivery");
        assert_eq!(
            factory["revision"],
            waived.factory.as_ref().unwrap().revision
        );
        assert_eq!(factory["attempt"], 1);
        assert_eq!(factory["blockerCount"], 0);
        assert!(factory["terminalOutcome"].is_null());
        assert_eq!(factory["provenance"], "clientReported");
        assert_eq!(
            factory.keys().cloned().collect::<HashSet<_>>(),
            HashSet::from([
                "runId".to_string(),
                "phase".to_string(),
                "revision".to_string(),
                "attempt".to_string(),
                "blockerCount".to_string(),
                "terminalOutcome".to_string(),
                "provenance".to_string(),
            ])
        );
        assert!(!mcp.to_string().contains("No distinct worker session"));

        let delivery_claim = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-delivery-waived-check",
            FactoryClaimRequest {
                expected_revision: waived.factory.as_ref().unwrap().revision,
                phase: FactoryPhase::Delivery,
                idempotency_key: "waiver-claim-delivery".into(),
            },
            now,
        )
        .await
        .unwrap();
        let claimed_delivery = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        factory_complete_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-delivery-waived-check",
            worker_context(
                &claimed_delivery,
                &delivery_claim,
                "waiver-complete-delivery",
            ),
            FactoryPhaseCompletion::Delivery {
                delivery: FactoryDeliveryInput {
                    reference: "https://github.example.test/pull/42".into(),
                    head_commit: "4444444444444444444444444444444444444444".into(),
                    evidence_summary: "Delivery is ready for an explicit desktop check waiver."
                        .into(),
                    known_limitations: vec!["The required tests check is missing.".into()],
                    improvement_proposal: None,
                },
            },
            now,
        )
        .await
        .unwrap();
        let awaiting = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let workflow = awaiting.factory.as_ref().unwrap();
        let accepted = factory_decide_final(
            &state,
            &run.id,
            FactoryFinalDecisionInput {
                expected_revision: workflow.revision,
                outcome: FactoryTerminalOutcome::Accepted,
                approved_plan_revision: workflow
                    .plan_approval
                    .as_ref()
                    .unwrap()
                    .plan_revision
                    .clone(),
                head_commit: "4444444444444444444444444444444444444444".into(),
                check_waivers: vec![FactoryCheckWaiverInput {
                    check_name: "tests".into(),
                    reason: "The desktop user accepts the missing client-reported test evidence."
                        .into(),
                }],
                independent_review_waiver_reason: None,
                safe_detail: None,
            },
            now,
        )
        .await
        .unwrap();
        assert_eq!(accepted.state, ExpertRunState::Accepted);
        assert!(accepted
            .factory
            .as_ref()
            .unwrap()
            .human_waivers
            .iter()
            .any(|waiver| waiver.kind == "qualityCheck"
                && waiver.check_name.as_deref() == Some("tests")));
    }

    #[tokio::test]
    async fn factory_attempt_exhaustion_is_terminal_after_three_failed_validations() {
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.path().to_path_buf();
        let now = factory_now("2026-08-18T10:00:00Z");
        let run = create_factory_test_run(&state, now).await;
        let mut current = advance_factory_to_build(&state, &run, now).await;
        for attempt in 1..=MAX_FACTORY_ATTEMPTS {
            current = advance_factory_to_validation(
                &state,
                &current,
                &format!("codex/session-build-{attempt}"),
                &format!("{attempt}{attempt}{attempt}{attempt}{attempt}{attempt}{attempt}"),
                now,
            )
            .await;
            let claim = factory_claim_phase(
                &state,
                &run.id,
                "codex",
                "/tmp/factory-project",
                &format!("codex/session-validate-{attempt}"),
                FactoryClaimRequest {
                    expected_revision: current.factory.as_ref().unwrap().revision,
                    phase: FactoryPhase::Validation,
                    idempotency_key: format!("exhaust-claim-{attempt}"),
                },
                now,
            )
            .await
            .unwrap();
            let claimed = get_run(&state, &run.id, "codex", "/tmp/factory-project")
                .await
                .unwrap();
            factory_submit_evidence(
                &state,
                &run.id,
                "codex",
                "/tmp/factory-project",
                &format!("codex/session-validate-{attempt}"),
                worker_context(&claimed, &claim, &format!("exhaust-fail-{attempt}")),
                FactoryEvidenceInput {
                    check_name: "tests".into(),
                    result: EvidenceResult::Fail,
                    command_label: Some("cargo test".into()),
                    exit_code: Some(1),
                    summary: "Validation failed".into(),
                    artifact_ids: Vec::new(),
                },
                now,
            )
            .await
            .unwrap();
            let failed = get_run(&state, &run.id, "codex", "/tmp/factory-project")
                .await
                .unwrap();
            let completion_context = worker_context(
                &failed,
                failed
                    .factory
                    .as_ref()
                    .unwrap()
                    .current_claim
                    .as_ref()
                    .unwrap(),
                &format!("exhaust-complete-{attempt}"),
            );
            let receipt = factory_complete_phase(
                &state,
                &run.id,
                "codex",
                "/tmp/factory-project",
                &format!("codex/session-validate-{attempt}"),
                completion_context.clone(),
                FactoryPhaseCompletion::Validation,
                now,
            )
            .await
            .unwrap();
            current = get_run(&state, &run.id, "codex", "/tmp/factory-project")
                .await
                .unwrap();
            assert_eq!(receipt.phase, current.factory.as_ref().unwrap().phase);
            if attempt == MAX_FACTORY_ATTEMPTS {
                let retry = factory_complete_phase(
                    &state,
                    &run.id,
                    "codex",
                    "/tmp/factory-project",
                    &format!("codex/session-validate-{attempt}"),
                    completion_context.clone(),
                    FactoryPhaseCompletion::Validation,
                    now,
                )
                .await
                .unwrap();
                assert_eq!(retry, receipt);
                assert!(factory_complete_phase(
                    &state,
                    &run.id,
                    "codex",
                    "/tmp/factory-project",
                    &format!("codex/session-validate-{attempt}"),
                    completion_context,
                    FactoryPhaseCompletion::Build {
                        head_commit: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".into(),
                    },
                    now,
                )
                .await
                .is_err());
            }
        }
        assert_eq!(current.state, ExpertRunState::Rework);
        assert_eq!(
            current.factory.as_ref().unwrap().phase,
            FactoryPhase::Completed
        );
        assert_eq!(
            current
                .factory
                .as_ref()
                .unwrap()
                .terminal
                .as_ref()
                .unwrap()
                .outcome,
            FactoryTerminalOutcome::AttemptExhausted
        );
        validate_runs(std::slice::from_ref(&current)).unwrap();
        let mut short_exhaustion = current.clone();
        short_exhaustion.factory.as_mut().unwrap().attempts.pop();
        assert!(validate_runs(std::slice::from_ref(&short_exhaustion)).is_err());
        let mut active_exhaustion = current;
        let attempt = active_exhaustion
            .factory
            .as_mut()
            .unwrap()
            .attempts
            .last_mut()
            .unwrap();
        attempt.ended_at = None;
        attempt.result = None;
        assert!(validate_runs(std::slice::from_ref(&active_exhaustion)).is_err());
    }

    #[tokio::test]
    async fn sqlite_restart_restores_exact_factory_phase_claim_and_revision() {
        let root = tempfile::tempdir().unwrap();
        let database = crate::state_db::StateDatabase::open(root.path()).unwrap();
        database
            .mutate(document_spec(), Vec::new(), |_| Ok(()))
            .await
            .unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let mut first_state = AppState::build().unwrap();
        first_state.app_data_dir = root.path().to_path_buf();
        let now = factory_now("2026-08-18T10:00:00Z");
        let run = create_factory_test_run(&first_state, now).await;
        factory_claim_phase(
            &first_state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-restart",
            FactoryClaimRequest {
                expected_revision: 1,
                phase: FactoryPhase::Planning,
                idempotency_key: "restart-claim".into(),
            },
            now,
        )
        .await
        .unwrap();
        let before = get_run(&first_state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let mut restarted_state = AppState::build().unwrap();
        restarted_state.app_data_dir = root.path().to_path_buf();
        let after = get_run(&restarted_state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        assert_eq!(after, before);
    }

    #[tokio::test]
    async fn legacy_json_load_rejects_invalid_factory_state() {
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.path().to_path_buf();
        let mut run = create_factory_test_run(&state, factory_now("2026-08-18T10:00:00Z")).await;
        run.factory.as_mut().unwrap().work_contract.title = "token=secret-value".into();
        std::fs::write(path(&state), serde_json::to_vec(&vec![run]).unwrap()).unwrap();

        assert!(load(&state).await.is_err());
    }

    #[test]
    fn retention_prunes_only_oldest_terminal_and_fails_closed_for_all_active_runs() {
        let create = factory_expert_create();
        let mut active = (0..MAX_RUNS)
            .map(|index| ExpertRun {
                id: uuid::Uuid::new_v4().to_string(),
                snapshot: create.clone(),
                state: ExpertRunState::InProgress,
                started_at: format!("2026-08-18T10:{:02}:00Z", index % 60),
                ended_at: None,
                evidence: Vec::new(),
                blockers: Vec::new(),
                waivers: Vec::new(),
                factory: None,
            })
            .collect::<Vec<_>>();
        let candidate = ExpertRun {
            id: uuid::Uuid::new_v4().to_string(),
            snapshot: create,
            state: ExpertRunState::InProgress,
            started_at: "2026-08-18T11:00:00Z".into(),
            ended_at: None,
            evidence: Vec::new(),
            blockers: Vec::new(),
            waivers: Vec::new(),
            factory: None,
        };
        let original_ids = active.iter().map(|run| run.id.clone()).collect::<Vec<_>>();
        assert!(add_run_with_retention(&mut active, candidate.clone()).is_err());
        assert_eq!(
            active.iter().map(|run| run.id.clone()).collect::<Vec<_>>(),
            original_ids
        );

        active[0].state = ExpertRunState::Accepted;
        active[0].ended_at = Some("2026-08-18T10:00:01Z".into());
        let evicted = active[0].id.clone();
        add_run_with_retention(&mut active, candidate.clone()).unwrap();
        assert_eq!(active.len(), MAX_RUNS);
        assert!(!active.iter().any(|run| run.id == evicted));
        assert!(active.iter().any(|run| run.id == candidate.id));
    }

    #[tokio::test]
    async fn completed_sqlite_persists_expert_runs() {
        let root = tempfile::tempdir().unwrap();
        let database = crate::state_db::StateDatabase::open(root.path()).unwrap();
        database
            .mutate(document_spec(), Vec::new(), |_| Ok(()))
            .await
            .unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.path().to_path_buf();

        let created = create_run(
            &state,
            ExpertRunCreate {
                expert_id: "reviewer".into(),
                expert_version: 1,
                project_path: "/tmp/project".into(),
                client: "codex".into(),
                lead_agent: "reviewer".into(),
                supporting_agents: Vec::new(),
                required_skills: Vec::new(),
                optional_skills: Vec::new(),
                runbook: None,
                contract: QualityContract::default(),
            },
        )
        .await
        .unwrap();

        assert_eq!(load(&state).await.unwrap()[0].id, created.id);
        assert!(!path(&state).exists());
    }

    #[tokio::test]
    async fn fixed_run_id_is_idempotent_for_recovery() {
        let root = tempfile::tempdir().unwrap();
        let database = crate::state_db::StateDatabase::open(root.path()).unwrap();
        database
            .mutate(document_spec(), Vec::new(), |_| Ok(()))
            .await
            .unwrap();
        database
            .set_migration_state(crate::types::StorageMigrationState::Complete)
            .await
            .unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.path().to_path_buf();
        let id = uuid::Uuid::new_v4().to_string();
        let create = ExpertRunCreate {
            expert_id: "reviewer".into(),
            expert_version: 1,
            project_path: "/tmp/project".into(),
            client: "codex".into(),
            lead_agent: "reviewer".into(),
            supporting_agents: Vec::new(),
            required_skills: Vec::new(),
            optional_skills: Vec::new(),
            runbook: None,
            contract: QualityContract::default(),
        };

        let first = create_run_with_id(&state, &id, create.clone())
            .await
            .unwrap();
        let second = create_run_with_id(&state, &id, create).await.unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(load(&state).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn fixed_factory_run_id_restores_committed_preflight_after_readiness_refresh() {
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.path().to_path_buf();
        let id = uuid::Uuid::new_v4().to_string();
        let now = factory_now("2026-08-18T10:00:00Z");
        let first = create_factory_run_with_id_at(
            &state,
            &id,
            factory_expert_create(),
            factory_create("2026-08-18T10:00:00Z"),
            now,
        )
        .await
        .unwrap();

        let mut refreshed = factory_create("2026-08-18T10:01:00Z");
        refreshed.readiness.evidence_revision = "readiness-v8".into();
        let recovered = create_factory_run_with_id_at(
            &state,
            &id,
            factory_expert_create(),
            refreshed,
            now + chrono::Duration::minutes(1),
        )
        .await
        .unwrap();

        assert_eq!(
            recovered, first,
            "the committed preflight remains authoritative"
        );
        assert_eq!(load(&state).await.unwrap().len(), 1);

        let mut changed_work = factory_create("2026-08-18T10:01:00Z");
        changed_work.title = "A different work order".into();
        assert!(create_factory_run_with_id_at(
            &state,
            &id,
            factory_expert_create(),
            changed_work,
            now + chrono::Duration::minutes(1),
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn factory_runs_reject_every_legacy_mutation_path() {
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.path().to_path_buf();
        let now = factory_now("2026-08-18T10:00:00Z");

        let evidence_run = create_factory_test_run(&state, now).await;
        assert!(submit_evidence(
            &state,
            &evidence_run.id,
            "codex",
            "/tmp/factory-project",
            EvidenceSubmission {
                idempotency_key: "legacy-evidence".into(),
                check_name: "tests".into(),
                result: EvidenceResult::Pass,
                command_label: Some("cargo test".into()),
                summary: "Tests passed".into(),
            },
        )
        .await
        .is_err());

        let blocker_run = create_factory_test_run(&state, now).await;
        assert!(report_blocker(
            &state,
            &blocker_run.id,
            "codex",
            "/tmp/factory-project",
            "access",
            "Waiting for access",
        )
        .await
        .is_err());

        let request_run = create_factory_test_run(&state, now).await;
        assert!(
            request_review(&state, &request_run.id, "codex", "/tmp/factory-project",)
                .await
                .is_err()
        );

        let review_run = create_factory_test_run(&state, now).await;
        let error =
            review_run_with_waivers(&state, &review_run.id, ExpertRunState::Rework, Vec::new())
                .await
                .unwrap_err();
        assert!(matches!(
            error,
            AppError::InvalidArgument { message }
                if message.contains("Factory-enabled Expert runs require")
        ));
    }

    #[tokio::test]
    async fn factory_validation_requires_evidence_from_the_exact_current_claim_lineage() {
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.path().to_path_buf();
        let now = factory_now("2026-08-18T10:00:00Z");
        let run = create_factory_test_run(&state, now).await;
        let build = advance_factory_to_build(&state, &run, now).await;
        let validation = advance_factory_to_validation(
            &state,
            &build,
            "codex/session-build-lineage",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            now,
        )
        .await;
        let first_claim = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validation-first",
            FactoryClaimRequest {
                expected_revision: validation.factory.as_ref().unwrap().revision,
                phase: FactoryPhase::Validation,
                idempotency_key: "lineage-claim-first".into(),
            },
            now,
        )
        .await
        .unwrap();
        let first_claimed = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        factory_submit_evidence(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validation-first",
            worker_context(&first_claimed, &first_claim, "lineage-pass-first"),
            FactoryEvidenceInput {
                check_name: "tests".into(),
                result: EvidenceResult::Pass,
                command_label: Some("cargo test".into()),
                exit_code: Some(0),
                summary: "Tests passed".into(),
                artifact_ids: Vec::new(),
            },
            now,
        )
        .await
        .unwrap();
        let evidenced = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        factory_release_claim(
            &state,
            &run.id,
            evidenced.factory.as_ref().unwrap().revision,
            now,
        )
        .await
        .unwrap();
        let released = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let second_claim = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validation-second",
            FactoryClaimRequest {
                expected_revision: released.factory.as_ref().unwrap().revision,
                phase: FactoryPhase::Validation,
                idempotency_key: "lineage-claim-second".into(),
            },
            now,
        )
        .await
        .unwrap();
        let second_claimed = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();

        assert!(factory_complete_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validation-second",
            worker_context(&second_claimed, &second_claim, "lineage-complete-second"),
            FactoryPhaseCompletion::Validation,
            now,
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn factory_validation_rejects_build_phase_evidence_and_prior_claim_artifacts() {
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.path().to_path_buf();
        let now = factory_now("2026-08-18T10:00:00Z");
        let run = create_factory_test_run(&state, now).await;
        let build = advance_factory_to_build(&state, &run, now).await;
        let workflow = build.factory.as_ref().unwrap();
        let build_claim = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-build-evidence",
            FactoryClaimRequest {
                expected_revision: workflow.revision,
                phase: FactoryPhase::Build,
                idempotency_key: "phase-evidence-build-claim".into(),
            },
            now,
        )
        .await
        .unwrap();
        let claimed_build = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        factory_submit_evidence(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-build-evidence",
            worker_context(&claimed_build, &build_claim, "phase-evidence-build-pass"),
            FactoryEvidenceInput {
                check_name: "tests".into(),
                result: EvidenceResult::Pass,
                command_label: Some("cargo test".into()),
                exit_code: Some(0),
                summary: "Build-phase tests passed".into(),
                artifact_ids: Vec::new(),
            },
            now,
        )
        .await
        .unwrap();
        let evidenced_build = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        factory_complete_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-build-evidence",
            worker_context(
                &evidenced_build,
                evidenced_build
                    .factory
                    .as_ref()
                    .unwrap()
                    .current_claim
                    .as_ref()
                    .unwrap(),
                "phase-evidence-complete-build",
            ),
            FactoryPhaseCompletion::Build {
                head_commit: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            now,
        )
        .await
        .unwrap();
        let validation = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let first_validation_claim = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validation-artifact-first",
            FactoryClaimRequest {
                expected_revision: validation.factory.as_ref().unwrap().revision,
                phase: FactoryPhase::Validation,
                idempotency_key: "phase-evidence-validation-claim-first".into(),
            },
            now,
        )
        .await
        .unwrap();
        let claimed_validation = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let artifact = factory_submit_artifact(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validation-artifact-first",
            worker_context(
                &claimed_validation,
                &first_validation_claim,
                "phase-evidence-artifact-first",
            ),
            FactoryArtifactInput {
                kind: "testReport".into(),
                label: "Test report".into(),
                reference: "urn:factory:test-report:first".into(),
                digest: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".into(),
                byte_size: 128,
                summary: "Bounded test report metadata".into(),
            },
            now,
        )
        .await
        .unwrap();
        let with_artifact = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        factory_release_claim(
            &state,
            &run.id,
            with_artifact.factory.as_ref().unwrap().revision,
            now,
        )
        .await
        .unwrap();
        let released = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let second_validation_claim = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validation-artifact-second",
            FactoryClaimRequest {
                expected_revision: released.factory.as_ref().unwrap().revision,
                phase: FactoryPhase::Validation,
                idempotency_key: "phase-evidence-validation-claim-second".into(),
            },
            now,
        )
        .await
        .unwrap();
        let second_claimed = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        assert!(factory_submit_evidence(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validation-artifact-second",
            worker_context(
                &second_claimed,
                &second_validation_claim,
                "phase-evidence-pass-second",
            ),
            FactoryEvidenceInput {
                check_name: "tests".into(),
                result: EvidenceResult::Pass,
                command_label: Some("cargo test".into()),
                exit_code: Some(0),
                summary: "Tests passed".into(),
                artifact_ids: vec![artifact.id],
            },
            now,
        )
        .await
        .is_err());

        let without_validation_evidence = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        factory_complete_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-validation-artifact-second",
            worker_context(
                &without_validation_evidence,
                &second_validation_claim,
                "phase-evidence-complete-validation",
            ),
            FactoryPhaseCompletion::Validation,
            now,
        )
        .await
        .unwrap();
        let review = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let workflow = review.factory.as_ref().unwrap();
        assert_eq!(workflow.phase, FactoryPhase::IndependentReview);
        assert!(workflow.validation.as_ref().unwrap().check_names.is_empty());
    }

    #[tokio::test]
    async fn factory_claim_history_remains_bounded_without_stranding_release_or_cancel() {
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.path().to_path_buf();
        let now = factory_now("2026-08-18T10:00:00Z");
        let run = create_factory_test_run(&state, now).await;
        let mut current = run;
        let mut first_claim = None;
        let mut first_request = None;

        for index in 0..(MAX_FACTORY_CLAIMS + 4) {
            let workflow = current.factory.as_ref().unwrap();
            let request = FactoryClaimRequest {
                expected_revision: workflow.revision,
                phase: FactoryPhase::Planning,
                idempotency_key: format!("history-claim-{index}"),
            };
            let claim = factory_claim_phase(
                &state,
                &current.id,
                "codex",
                "/tmp/factory-project",
                &format!("codex/session-history-{index}"),
                request.clone(),
                now,
            )
            .await
            .unwrap();
            if index == 0 {
                first_claim = Some(claim);
                first_request = Some(request);
            }
            current = get_run(&state, &current.id, "codex", "/tmp/factory-project")
                .await
                .unwrap();
            current = factory_release_claim(
                &state,
                &current.id,
                current.factory.as_ref().unwrap().revision,
                now,
            )
            .await
            .unwrap();
        }

        let retry = factory_claim_phase(
            &state,
            &current.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-history-0",
            first_request.unwrap(),
            now,
        )
        .await
        .unwrap();
        assert_eq!(retry, first_claim.unwrap());

        assert_eq!(
            current.factory.as_ref().unwrap().prior_claims.len(),
            MAX_FACTORY_CLAIMS
        );
        let claimed = factory_claim_phase(
            &state,
            &current.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-history-cancel",
            FactoryClaimRequest {
                expected_revision: current.factory.as_ref().unwrap().revision,
                phase: FactoryPhase::Planning,
                idempotency_key: "history-claim-cancel".into(),
            },
            now,
        )
        .await
        .unwrap();
        let claimed_run = get_run(&state, &current.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        assert!(claimed.generation > MAX_FACTORY_CLAIMS as u64);
        let cancelled = factory_cancel(
            &state,
            &current.id,
            claimed_run.factory.as_ref().unwrap().revision,
            None,
            now,
        )
        .await
        .unwrap();
        assert_eq!(cancelled.state, ExpertRunState::Cancelled);
        assert_eq!(
            cancelled.factory.as_ref().unwrap().prior_claims.len(),
            MAX_FACTORY_CLAIMS
        );
    }

    #[test]
    fn factory_exhaustion_retains_the_decisive_review_rework_history() {
        let now = factory_now("2026-08-18T10:00:00Z");
        let create = factory_expert_create();
        let mut workflow =
            prepare_factory_workflow(&create, factory_create(&now.to_rfc3339()), now).unwrap();
        workflow.phase = FactoryPhase::IndependentReview;
        workflow.attempts = vec![FactoryAttempt {
            number: MAX_FACTORY_ATTEMPTS,
            started_at: factory_timestamp(now),
            ended_at: None,
            head_commit: Some("dddddddddddddddddddddddddddddddddddddddd".into()),
            builder_identity: Some("codex/session-builder".into()),
            result: None,
        }];
        workflow.review = Some(FactoryReview {
            attempt: MAX_FACTORY_ATTEMPTS,
            head_commit: "dddddddddddddddddddddddddddddddddddddddd".into(),
            phase: FactoryPhase::IndependentReview,
            claim_id: uuid::Uuid::new_v4().to_string(),
            claim_generation: 1,
            reviewer_identity: "codex/session-reviewer".into(),
            verdict: FactoryReviewVerdict::Rework,
            summary: "A decisive high-severity issue remains.".into(),
            findings: vec![FactoryReviewFinding {
                severity: FactoryReviewSeverity::High,
                summary: "The current head violates the approved contract.".into(),
            }],
            submitted_at: factory_timestamp(now),
            provenance: FactoryProvenance::ClientReported,
        });

        assert!(finish_factory_attempt(&mut workflow, "reviewRework", now).unwrap());
        assert_eq!(
            workflow.review.as_ref().unwrap().verdict,
            FactoryReviewVerdict::Rework
        );
        assert_eq!(workflow.review.as_ref().unwrap().findings.len(), 1);
    }

    #[test]
    fn factory_external_metadata_rejects_secrets_raw_content_and_private_paths() {
        let contract = factory_expert_create().contract;
        for unsafe_summary in [
            "authorization: Bearer secret-value",
            "password : hunter2",
            "password\n:\nhunter2",
            "sid=0123456789abcdef0123456789abcdef",
            "data=eyJlbmMiOiJBMTI4R0NNIn0..aXY.Y2lwaGVydGV4dA.dGFn",
            "AWS_SECRET_ACCESS_KEY=secret-value",
            "Observed sk-proj-1234567890abcdef",
            concat!("xox", "b-123456789012-123456789012-abcdefghijklmnopqrstuvwxyz"),
            "glpat-abcdefghijklmnopqrstuvwxyz",
            "glsoat-abcdefghijklmnopqrstuvwxyz",
            "glffct-abcdefghijklmnopqrstuvwxyz",
            "hf_abcdefghijklmnopqrstuvwxyz",
            "npm_abcdefghijklmnopqrstuvwxyz",
            "dckr_pat_abcdefghijklmnopqrstuvwxyz",
            "pypi-abcdefghijklmnopqrstuvwxyz",
            "lin_api_abcdefghijklmnopqrstuvwxyz",
            "shpat_abcdefghijklmnopqrstuvwxyz",
            "dop_v1_abcdefghijklmnopqrstuvwxyz",
            "AIzaSyabcdefghijklmnopqrstuvwxyz",
            "ya29.abcdefghijklmnopqrstuvwxyz",
            "SG.abcdefghijklmnopqrstuv.abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNO",
            "Observed https://chat.example.test/hooks/abcdefgh;ijklmnop",
            "Observed https://chat.example.test/hooks/abcdefgh,ijklmnop",
            "Observed https://chat.example.test/hooks/abcdefgh(ijklmnop)",
            "Observed https://chat.example.test/hooks/abcdefgh'ijklmnop",
            "Observed https://chat.example.test/hooks/abcdefgh%2Fijklmnop",
            concat!("sk_", "live_abcdefghijklmnopqrstuvwxyz"),
            concat!("rk_", "live_abcdefghijklmnopqrstuvwxyz"),
            "fn leaked_repository_content() {}",
            "pub(crate) fn leaked_repository_content() {}",
            "return account.balance;",
            "account.balance += 1;",
            "account.balance=1;",
            "account.balance = 1",
            "deploy();",
            "deploy()",
            "if (ready) deploy();",
            "if (authorized) { reveal(account.balance); }",
            "if(authorized){reveal(account.balance);}",
            "if authorized:\n    reveal(account.balance)",
            "elif ready:\n    await deploy()",
            "else:\n    await deploy()",
            "with open(\"settings.json\") as file:\n    load(file)",
            "async with client:\n    await deploy()",
            "async for item in items:\n    await deploy()",
            "await deploy()",
            "await deploy(\n  production\n)",
            "import os",
            "import os # platform-specific",
            "from pathlib import Path",
            "from pathlib import Path as P",
            "from pathlib import *",
            "from . import settings",
            "user_id: int = 42",
            "result = output",
            "result = load_config()",
            "result = [1, 2, 3]",
            r#"result = {"ok": true}"#,
            "Result = load_config()",
            "Result = output",
            "Handler = () => deploy()",
            "(x, y) = (1, 2)",
            "({x} = source)",
            "({ x = 1 } = source)",
            "([x = 1] = source)",
            "[x, y] = source",
            "flags |= ADMIN",
            "mask &= allowed",
            "cache ??= build()",
            "value <<= 1",
            "Result = new Foo()",
            "Result = new Foo<string>()",
            "Result = [1, 2, 3]",
            r#"Config = {"ok": true}"#,
            "x, y = 1, 2",
            "console.log(account.balance);",
            "SELECT email FROM users;",
            "SELECT email FROM users",
            "Select email from users",
            "SELECT 1",
            "SELECT DISTINCT email FROM users",
            "SELECT email AS address FROM users",
            "Select email address from users",
            "SELECT email FROM users AS u",
            "SELECT TOP 10 email FROM users",
            "SELECT TOP (10) email FROM users",
            "SELECT email FROM users AS \"u\"",
            "SELECT email FROM users UNION SELECT email FROM admins",
            "SELECT email FROM users FOR UPDATE",
            "SELECT * FROM users, roles",
            "SELECT value FROM generate_series(1, 10)",
            "SELECT TOP 10 PERCENT email FROM users",
            "SELECT * FROM \"users\"",
            "WITH active AS (SELECT id FROM users) SELECT id FROM active",
            "INSERT INTO users(email) VALUES ('alice@example.test')",
            "INSERT INTO users(email)VALUES('alice@example.test')",
            "UPDATE users SET email = 'alice@example.test' WHERE id = 1",
            "UPDATE users u SET email = 'alice@example.test' WHERE u.id = 1",
            "DELETE FROM users WHERE id = 1",
            "Delete from users",
            "DELETE FROM users AS u WHERE u.id = 1",
            "CREATE TABLE users (id INTEGER)",
            "CREATE TEMP TABLE users (id INTEGER)",
            "CREATE GLOBAL TEMPORARY TABLE users (id INTEGER)",
            "CREATE LOCAL TEMPORARY TABLE users (id INTEGER)",
            "CREATE MATERIALIZED VIEW active_users AS SELECT id FROM users",
            "CREATE OR REPLACE VIEW active_users AS SELECT id FROM users",
            "CREATE TEMP VIEW active_users AS SELECT id FROM users",
            "CREATE TEMPORARY VIEW active_users AS SELECT id FROM users",
            "CREATE UNLOGGED TABLE audit_log (id INTEGER)",
            "CREATE INDEX CONCURRENTLY idx_users_email ON users(email)",
            "CREATE UNIQUE INDEX CONCURRENTLY idx_users_email ON users(email)",
            "DROP INDEX CONCURRENTLY idx_users_email",
            "CREATE OR REPLACE TEMP VIEW active_users AS SELECT id FROM users",
            "CREATE OR REPLACE TEMPORARY VIEW active_users AS SELECT id FROM users",
            "CREATE RECURSIVE VIEW active_users AS SELECT id FROM users",
            "CREATE OR REPLACE TEMP RECURSIVE VIEW active_users AS SELECT id FROM users",
            "CREATE TEMP RECURSIVE VIEW active_users AS SELECT id FROM users",
            "CREATE TEMPORARY RECURSIVE VIEW active_users AS SELECT id FROM users",
            "ALTER MATERIALIZED VIEW active_users RENAME TO archived_users",
            "DROP MATERIALIZED VIEW active_users",
            "REFRESH MATERIALIZED VIEW active_users",
            "REFRESH MATERIALIZED VIEW CONCURRENTLY active_users",
            "CREATE SEQUENCE internal_ids",
            "CREATE TEMPORARY SEQUENCE internal_ids",
            "CREATE UNLOGGED SEQUENCE internal_ids",
            "EXPLAIN SELECT email FROM users",
            "EXPLAIN VERBOSE SELECT email FROM users",
            "EXPLAIN ANALYZE VERBOSE SELECT email FROM users",
            "EXPLAIN QUERY PLAN SELECT email FROM users",
            "EXPLAIN FORMAT=JSON SELECT email FROM users",
            "EXPLAIN EXTENDED SELECT email FROM users",
            "EXPLAIN PARTITIONS SELECT email FROM users",
            "EXPLAIN PLAN FOR SELECT email FROM users",
            "GRANT SELECT ON users TO analyst;",
            "GRANT SELECT ON TABLE users TO analyst;",
            "GRANT analyst TO reviewer;",
            "REVOKE SELECT ON users FROM analyst;",
            "REVOKE SELECT ON TABLE users FROM analyst;",
            "REVOKE analyst FROM reviewer;",
            "grant analyst to reviewer",
            "revoke analyst from reviewer",
            "SHOW TABLES;",
            "SHOW VARIABLES;",
            "SHOW STATUS;",
            "SHOW COLUMNS FROM users;",
            "DESCRIBE users;",
            "DESCRIBE users email;",
            "MERGE INTO target USING source ON target.id = source.id WHEN MATCHED THEN UPDATE SET value = source.value;",
            "MERGE INTO target t USING source s ON t.id = s.id WHEN MATCHED THEN UPDATE SET value = s.value;",
            "REPLACE INTO users(id) VALUES (1)",
            "CALL refresh_cache()",
            "EXEC refresh_cache",
            "EXECUTE refresh_cache",
            "VACUUM users;",
            "VACUUM;",
            "ANALYZE users;",
            "ANALYZE;",
            "SHOW search_path;",
            "TRUNCATE users;",
            "COPY users TO STDOUT;",
            "UPSERT INTO users(id) VALUES (1);",
            "CREATE FUNCTION f() RETURNS int AS 'SELECT 1' LANGUAGE SQL;",
            "CREATE PROCEDURE refresh_cache() LANGUAGE SQL AS 'SELECT 1';",
            "CREATE TRIGGER audit_insert AFTER INSERT ON users EXECUTE FUNCTION audit();",
            "CREATE TYPE mood AS ENUM ('happy', 'sad');",
            "CREATE EXTENSION IF NOT EXISTS pgcrypto;",
            "CREATE ROLE analyst;",
            "CREATE POLICY tenant_policy ON accounts;",
            "ALTER TYPE mood ADD VALUE 'happy';",
            "COMMENT ON TABLE accounts IS 'internal';",
            "create extension pgcrypto",
            "alter type mood add value 'sad'",
            "drop role analyst",
            "comment on table accounts is 'internal'",
            "Create Role analyst",
            "Comment On Table accounts IS 'internal'",
            "CrEaTe ExTeNsIoN pgcrypto",
            "BEGIN IMMEDIATE TRANSACTION",
            "BEGIN EXCLUSIVE",
            "ROLLBACK TO SAVEPOINT checkpoint",
            "END TRANSACTION",
            "ABORT TRANSACTION",
            "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE",
            "COMMIT WORK AND CHAIN",
            "ROLLBACK TRANSACTION AND NO CHAIN",
            "PRAGMA table_info(users)",
            "VALUES (1)",
            "BEGIN TRANSACTION",
            "COMMIT",
            "ROLLBACK TRANSACTION",
            "ALTER TABLE users ENABLE ROW LEVEL SECURITY",
            "ALTER TABLE users OWNER TO admin",
            "TRUNCATE TABLE users",
            "DROP TABLE users",
            "SELECT email\nFROM users;",
            r#"{"email":"alice@example.test"}"#,
            "{\n  \"email\": \"alice@example.test\"\n}",
            r#"["internal-host","admin"]"#,
            "[database]",
            "[database]\nurl: postgres://example.test/app",
            "feature_enabled: true",
            "database_url: postgres://internal.example.test/app",
            "server_host: internal.example.test",
            "privateKey: hunter2",
            "Database: internal-db",
            "Database: internal database",
            "Database: &primary internal-db",
            "Mode: production",
            "Profile: release",
            "Region: us-east-1",
            "Namespace: internal",
            "Logging: |\n  verbose output enabled",
            "Payload: | # internal output\n  repository source",
            "Mode: !Ref Environment",
            "\"db host\": internal",
            "\"db:host\": internal database",
            "\"db\\\":host\": internal database",
            "Payload: |2 # internal output\n  repository source",
            "Payload: >2- # internal output\n  repository source",
            "\"Database\": internal database",
            "'Database': &primary internal-db",
            "- Database: internal-db",
            "database:\n  host: internal-db",
            "<setting>true</setting>",
            r#"<setting enabled="true" />"#,
            "<?xml version=\"1.0\"?>",
            "<!DOCTYPE html>",
            "#include <stdio.h>",
            "#include<stdio.h>",
            "# include <stdio.h>",
            "#define FEATURE 1",
            "#pragma once",
            "#import <Foundation/Foundation.h>",
            "@import Foundation;",
            "export import std;",
            "#nullable enable",
            "#[derive(Debug)]",
            "@interface Foo : NSObject",
            "#![allow(dead_code)]",
            "#checksum \"source.cs\" \"{00000000-0000-0000-0000-000000000000}\" \"00\"",
            "@synthesize property = _property;",
            "@dynamic property;",
            "@compatibility_alias Alias Original;",
            "mod internal;",
            "mod internal {}",
            "macro_rules! example {}",
            "@autoreleasepool {",
            "@try {",
            "@Override",
            "@dataclass",
            "@staticmethod",
            "@cache",
            "@contextmanager",
            "@abstractmethod",
            "package main",
            "set -euo pipefail",
            "namespace Acme {",
            "namespace {",
            "namespace Acme::Core {",
            "inline namespace v1 {",
            "export namespace v1 {",
            "export inline namespace v1 {",
            "namespace current = Acme::Core;",
            "body { color: red; }",
            "@media (max-width: 600px) { body { color: red; } }",
            "@Inject",
            "@Test",
            "@app.route(\"/x\")",
            "@throw exception;",
            "@synchronized(obj) {",
            "<?php echo \"value\";",
            "#!/usr/bin/env bash",
            "#include HEADER_FILE",
            "using System;",
            "global using System;",
            "using static System.Math;",
            "using Foo = Namespace.Type;",
            "use foo;",
            "pub use foo;",
            "<!-- internal repository note -->",
            "<![CDATA[internal_repository]]>",
            "<?xml-stylesheet type=\"text/xsl\" href=\"style.xsl\"?>",
            "<!DOCTYPE note [<!ENTITY writer \"internal\">]>",
            "database.url = postgres://example.test/app",
            "struct Account { id: u64 }",
            "interface Account { id: number }",
            "diff --git a/src/lib.rs b/src/lib.rs",
            "raw output: test process environment",
            "/Users/alice/private/project/src/lib.rs",
            "[spec](/Users/alice/private/spec.md)",
            "see(/Users/alice/private/spec.md)",
            "See %252FUsers%252Falice%252Fprivate%252Fresult.json",
            "[source](https://alice:p4ss@example.test/spec)",
            "source=https://alice:p4ss@example.test/spec",
            "Read /opt/company/project/config.toml",
            "Read /srv/company/project/config.toml",
            "Read /tmp/company/project/config.toml",
            "Read D:/company/project/config.toml",
            r"Read \\server\private\project\config.toml",
        ] {
            assert!(
                validate_factory_evidence_input(&FactoryEvidenceInput {
                    check_name: "tests".into(),
                    result: EvidenceResult::Pass,
                    command_label: None,
                    exit_code: None,
                    summary: unsafe_summary.into(),
                    artifact_ids: Vec::new(),
                })
                .is_err(),
                "unsafe summary was accepted: {unsafe_summary:?}"
            );
        }

        for safe_summary in [
            "Validation completed successfully.",
            "Status: ready for desktop review.",
            "The [database] section was reviewed.",
            "Use <setting> as the documented label.",
            "Two internal hosts were checked.",
            "Validation completed successfully.\nAll required checks passed.",
            "Review notes:\nThe database section was reviewed.",
            "- Inspect the API.\n- Run the declared checks.",
            "Select a project from the list.",
            "Select items from catalog.",
            "Select items from catalog",
            "Select items from catalog for review.",
            "Delete old entries from history.",
            "Delete from history",
            "Status = ready when all checks pass.",
            "Status: ready",
            "Risk: rollout remains manual.",
            "Result: passed.",
            "Note: review with the client.",
            "Status: ready\nAll systems nominal.",
            "Client-reported: shown only as bounded metadata.",
            "C++ support remains unchanged.",
            "Import settings only after approval.",
            "Import data",
            "Import users",
            "From planning, continue to review.",
            "Result = output only after validation.",
            "Await deployment only after approval.",
            "Database:\nHost details remain client-reported.",
            "Insert users into the selected team.",
            "Update users after approval.",
            "Create table views in the dashboard.",
            "Explain the select option to reviewers.",
            "Explain how to update the dashboard.",
            "Begin",
            "Commit",
            "Rollback",
            "Abort",
            "End",
            "Owner: Alice",
            "Priority: High",
            "Severity: High",
            "Show tables in the dashboard.",
            "@alice please review the delivery.",
            "Call reviewers after approval.",
            "Package main changes for release.",
            "Set deployment rules before approval.",
            "Namespace review remains pending.",
            "Vacuum the workspace after approval.",
            "Analyze reported evidence.",
            "Show readiness after validation.",
            "Truncate labels in the UI.",
            "Copy the summary for review.",
            "Create function descriptions for users.",
            "Create type descriptions for users.",
            "Create role assignments for users.",
            "Inline namespace review remains pending.",
            "Use body color in the report.",
            "Media queries remain review notes.",
        ] {
            assert!(
                validate_factory_evidence_input(&FactoryEvidenceInput {
                    check_name: "tests".into(),
                    result: EvidenceResult::Pass,
                    command_label: None,
                    exit_code: None,
                    summary: safe_summary.into(),
                    artifact_ids: Vec::new(),
                })
                .is_ok(),
                "safe summary was rejected: {safe_summary:?}"
            );
        }

        let mut plan = match planning_completion() {
            FactoryPhaseCompletion::Planning { plan } => plan,
            _ => unreachable!(),
        };
        plan.known_limitations = vec!["Repository file: /home/alice/project/.env".into()];
        assert!(validate_factory_plan_input(&plan, &contract).is_err());
        plan.known_limitations = vec!["Execution remains client-reported.".into()];
        plan.content = "Use authorization: Bearer secret-value during validation".into();
        assert!(validate_factory_plan_input(&plan, &contract).is_err());
        plan.content = "Verify the approved implementation plan.".into();
        plan.citations = vec!["source:/Users/alice/private/project/src/lib.rs".into()];
        assert!(validate_factory_plan_input(&plan, &contract).is_err());
        plan.citations = vec!["https://alice:supersecret@example.test/private".into()];
        assert!(validate_factory_plan_input(&plan, &contract).is_err());

        let mut work_order = factory_create("2026-08-18T10:00:00Z");
        work_order.title = "account.balance += 1;".into();
        assert!(prepare_factory_workflow(
            &factory_expert_create(),
            work_order,
            factory_now("2026-08-18T10:00:00Z"),
        )
        .is_err());

        let mut unsafe_contract = factory_expert_create();
        unsafe_contract.contract.checks[0].name = "password\n:\nhunter2".into();
        assert!(prepare_factory_workflow(
            &unsafe_contract,
            factory_create("2026-08-18T10:00:00Z"),
            factory_now("2026-08-18T10:00:00Z"),
        )
        .is_err());
        unsafe_contract.contract.checks[0].name = "tests".into();
        unsafe_contract.contract.checks[0].kind = "account.balance=1;".into();
        assert!(prepare_factory_workflow(
            &unsafe_contract,
            factory_create("2026-08-18T10:00:00Z"),
            factory_now("2026-08-18T10:00:00Z"),
        )
        .is_err());

        assert!(validate_factory_review_input(&FactoryReviewInput {
            verdict: FactoryReviewVerdict::Rework,
            summary: "Review completed".into(),
            findings: vec![FactoryReviewFinding {
                severity: FactoryReviewSeverity::High,
                summary: "```rust\nfn leaked_repository_content() {}\n```".into(),
            }],
        })
        .is_err());
        assert!(validate_factory_improvement(&FactoryImprovementProposal {
            failure_class: "validation".into(),
            target: FactoryImprovementTarget::Test,
            proposal: "Store api_key=secret-value for the next run".into(),
            suggested_test: None,
            provenance: FactoryProvenance::ClientReported,
        })
        .is_err());
        assert!(validate_factory_artifact_reference(
            "https://ci.example.test/report?access_token=secret-value"
        )
        .is_err());
        for unsafe_reference in [
            "https://ci.example.test/report?jwt=eyJhbGciOiJIUzI1NiJ9.payload.signature",
            "https://ci.example.test/report#jwt=eyJhbGciOiJIUzI1NiJ9.payload.signature",
            "https://ci.example.test/report?session=eyJhbGciOiJIUzI1NiJ9.cHJpdmF0ZS1wYXlsb2Fk.c2VjcmV0LXNpZ25hdHVyZQ",
            "https://ci.example.test/report?session=eyJhbGciOiJIUzI1NiJ9.e30.c2ln",
            "https://ci.example.test/report?session=eyJhbGciOiJub25lIn0.e30.",
            "https://ci.example.test/report?session=eyJlbmMiOiJBMTI4R0NNIn0.ZW5jcnlwdGVk.aXY.Y2lwaGVydGV4dA.dGFn",
            "https://ci.example.test/report?sid=0123456789abcdef0123456789abcdef",
            "https://ci.example.test/report?data=eyJlbmMiOiJBMTI4R0NNIn0..aXY.Y2lwaGVydGV4dA.dGFn",
            "https://ci.example.test/report?session=0123456789abcdef0123456789abcdef",
            "https://ci.example.test/report?PHPSESSID=0123456789abcdef0123456789abcdef",
            "https://storage.example.test/blob?sv=2026-01-01&sig=azure-secret",
            "urn:factory:signature:hunter2",
            "https://ci.example.test/report?pwd=hunter2",
            "https://ci.example.test/report#pwd=hunter2",
            "https://ci.example.test/report?passwd=hunter2",
            "https://ci.example.test/report?auth=hunter2",
            "https://ci.example.test/report?credential=hunter2",
            "https://ci.example.test/report?credentials=hunter2",
            "https://ci.example.test/report?db_passwd=hunter2",
            "https://ci.example.test/report?basic_auth=hunter2",
            "https://ci.example.test/report?service_credentials=hunter2",
            "https://ci.example.test/report?db_secret=hunter2",
            "https://ci.example.test/report#service_credentials=hunter2",
            "https://ci.example.test/reports/client-secret/value",
            concat!(
                "https://hooks.slack.",
                "com/services/T00000000/B00000000/abcdefghijklmnopqrstuvwx"
            ),
            "https://hooks.slack.com/%73ervices/T00000000/B00000000/abcdefghijklmnopqrstuvwx",
            "https://discord.com/api/v10/webhooks/123456789/abcdefghijklmnopqrstuvwx",
            "https://api.telegram.org/bot123456789:ABCDEFGHIJKLMNOPQRSTUVWXYZ/getMe",
            "https://canary.discord.com/api/webhooks/123456789/abcdefghijklmnopqrstuvwx",
            "https://tenant.webhook.office.com/webhookb2/tenant/IncomingWebhook/abcdefghijklmnopqrstuvwx/channel",
            "https://chat.example.test/hooks/abcdefghijklmnopqrstuvwx",
            "https://chat.example.test/hooks/abcdefghijkl%7Emnopqrstuvwx",
            "https://chat.example.test/hooks/abcdefghijkl+mnopqrstuvwx",
            "https://chat.example.test/hooks/abcdefghijkl=mnopqrstuvwx",
            "https://chat.example.test/hooks/abcdefgh;ijklmnop",
            "https://chat.example.test/hooks/abcdefgh%2Fijklmnop",
            "https://tenant.webhook.example/abcdefgh%2Fijklmnop",
            "https://ci.example.test/report/ya29.abcdefghijklmnopqrstuvwxyz",
            "https://ci.example.test/report?AWS_SECRET_ACCESS_KEY=secret-value",
            "https://ci.example.test/report?refresh.token=secret-value",
            "https://ci.example.test/report#id-token=secret-value",
            "urn:factory:refresh_token:secret-value",
            "urn:file:%2FUsers%2Falice%2Fprivate%2Fresult.json",
            "urn:file:%252FUsers%252Falice%252Fprivate%252Fresult.json",
        ] {
            assert!(validate_factory_artifact_reference(unsafe_reference).is_err());
        }
        assert!(validate_factory_artifact_reference(
            "urn:factory:report:/Users/alice/private/project/result.json"
        )
        .is_err());
        for safe_reference in [
            "https://ci.example.test/report?assignee=alice",
            "https://ci.example.test/report?assignment=ready",
            "https://ci.example.test/report?design=compact",
            "https://ci.example.test/report?possession=ready",
        ] {
            assert!(validate_factory_artifact_reference(safe_reference).is_ok());
        }
        assert!(validate_factory_https(
            "https://github.example.test/pull/42#token=secret-value",
            "Factory delivery reference",
        )
        .is_err());
        assert!(validate_factory_https(
            "https://github.example.test/%252FUsers%252Falice%252Fprivate",
            "Factory delivery reference",
        )
        .is_err());
        assert!(validate_factory_https(
            "https://example.test/view//Users/alice/private",
            "Factory delivery reference",
        )
        .is_err());
        assert!(validate_factory_https(
            "https://example.test/view///Users/alice/private",
            "Factory delivery reference",
        )
        .is_err());
        assert!(validate_factory_https(
            "https://github.example.test/pull/42?view=summary#checks",
            "Factory delivery reference",
        )
        .is_ok());
    }

    #[tokio::test]
    async fn factory_worker_labels_reject_external_secrets_and_private_paths() {
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.path().to_path_buf();
        let now = factory_now("2026-08-18T10:00:00Z");
        let run = create_factory_test_run(&state, now).await;
        assert!(factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-label-validation",
            FactoryClaimRequest {
                expected_revision: run.factory.as_ref().unwrap().revision,
                phase: FactoryPhase::Planning,
                idempotency_key: "AWS_SECRET_ACCESS_KEY=secret-value".into(),
            },
            now,
        )
        .await
        .is_err());
        let claim = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-label-validation",
            FactoryClaimRequest {
                expected_revision: run.factory.as_ref().unwrap().revision,
                phase: FactoryPhase::Planning,
                idempotency_key: "claim-label-validation".into(),
            },
            now,
        )
        .await
        .unwrap();
        let claimed = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let mut unsafe_context = worker_context(&claimed, &claim, "unsafe-worker-context");
        unsafe_context.idempotency_key = "/Users/alice/private/idempotency".into();
        assert!(factory_submit_blocker(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-label-validation",
            unsafe_context,
            FactoryBlockerInput {
                kind: "access".into(),
                summary: "A bounded blocker summary.".into(),
            },
            now,
        )
        .await
        .is_err());
        assert!(factory_submit_artifact(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-label-validation",
            worker_context(&claimed, &claim, "unsafe-artifact-label"),
            FactoryArtifactInput {
                kind: "testReport".into(),
                label: "source:/Users/alice/private/project/result.json".into(),
                reference: "urn:factory:test-report:unsafe-label".into(),
                digest: "a".repeat(64),
                byte_size: 10,
                summary: "Bounded metadata only.".into(),
            },
            now,
        )
        .await
        .is_err());
        assert!(factory_submit_blocker(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-label-validation",
            worker_context(&claimed, &claim, "unsafe-blocker-kind"),
            FactoryBlockerInput {
                kind: "authorization: Bearer secret-value".into(),
                summary: "A bounded blocker summary.".into(),
            },
            now,
        )
        .await
        .is_err());
    }

    #[test]
    fn factory_claim_contract_exposes_only_typed_phase_specific_submission_shapes() {
        let planning = factory_permitted_submission_shapes(FactoryPhase::Planning).unwrap();
        assert_eq!(
            planning,
            vec![
                FactoryPermittedSubmissionShape::Artifact,
                FactoryPermittedSubmissionShape::Blocker,
                FactoryPermittedSubmissionShape::PlanningCompletion,
            ]
        );
        let validation = factory_permitted_submission_shapes(FactoryPhase::Validation).unwrap();
        assert_eq!(
            validation,
            vec![
                FactoryPermittedSubmissionShape::Artifact,
                FactoryPermittedSubmissionShape::Blocker,
                FactoryPermittedSubmissionShape::Evidence,
                FactoryPermittedSubmissionShape::ValidationCompletion,
            ]
        );
        assert_eq!(
            serde_json::to_value(&validation).unwrap(),
            serde_json::json!([
                { "kind": "artifact" },
                { "kind": "blocker" },
                { "kind": "evidence" },
                { "kind": "validationCompletion" },
            ])
        );
        assert!(factory_permitted_submission_shapes(FactoryPhase::Completed).is_err());
    }

    #[tokio::test]
    async fn factory_evicted_claim_lineage_keeps_retained_artifact_and_evidence_valid() {
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::build().unwrap();
        state.app_data_dir = root.path().to_path_buf();
        let now = factory_now("2026-08-18T10:00:00Z");
        let run = create_factory_test_run(&state, now).await;
        let build = advance_factory_to_build(&state, &run, now).await;
        let validation = advance_factory_to_validation(
            &state,
            &build,
            "codex/session-eviction-build",
            "ffffffffffffffffffffffffffffffffffffffff",
            now,
        )
        .await;
        let first_claim = factory_claim_phase(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-eviction-first",
            FactoryClaimRequest {
                expected_revision: validation.factory.as_ref().unwrap().revision,
                phase: FactoryPhase::Validation,
                idempotency_key: "eviction-claim-first".into(),
            },
            now,
        )
        .await
        .unwrap();
        let claimed = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        let artifact = factory_submit_artifact(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-eviction-first",
            worker_context(&claimed, &first_claim, "eviction-artifact-first"),
            FactoryArtifactInput {
                kind: "testReport".into(),
                label: "Test report".into(),
                reference: "urn:factory:test-report:evicted-claim".into(),
                digest: "abababababababababababababababababababababababababababababababab".into(),
                byte_size: 256,
                summary: "Retained bounded test report metadata".into(),
            },
            now,
        )
        .await
        .unwrap();
        let with_artifact = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        factory_submit_evidence(
            &state,
            &run.id,
            "codex",
            "/tmp/factory-project",
            "codex/session-eviction-first",
            worker_context(
                &with_artifact,
                with_artifact
                    .factory
                    .as_ref()
                    .unwrap()
                    .current_claim
                    .as_ref()
                    .unwrap(),
                "eviction-evidence-first",
            ),
            FactoryEvidenceInput {
                check_name: "tests".into(),
                result: EvidenceResult::Pass,
                command_label: Some("cargo test".into()),
                exit_code: Some(0),
                summary: "Tests passed".into(),
                artifact_ids: vec![artifact.id],
            },
            now,
        )
        .await
        .unwrap();
        let mut current = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        current = factory_release_claim(
            &state,
            &run.id,
            current.factory.as_ref().unwrap().revision,
            now,
        )
        .await
        .unwrap();

        for index in 0..(MAX_FACTORY_CLAIMS + 1) {
            let claim = factory_claim_phase(
                &state,
                &run.id,
                "codex",
                "/tmp/factory-project",
                &format!("codex/session-eviction-{index}"),
                FactoryClaimRequest {
                    expected_revision: current.factory.as_ref().unwrap().revision,
                    phase: FactoryPhase::Validation,
                    idempotency_key: format!("eviction-claim-{index}"),
                },
                now,
            )
            .await
            .unwrap();
            current = get_run(&state, &run.id, "codex", "/tmp/factory-project")
                .await
                .unwrap();
            assert!(claim.generation > first_claim.generation);
            current = factory_release_claim(
                &state,
                &run.id,
                current.factory.as_ref().unwrap().revision,
                now,
            )
            .await
            .unwrap();
        }

        let workflow = current.factory.as_ref().unwrap();
        assert_eq!(workflow.prior_claims.len(), MAX_FACTORY_CLAIMS);
        assert!(!workflow
            .prior_claims
            .iter()
            .any(|claim| claim.id == first_claim.id));
        validate_runs(std::slice::from_ref(&current)).unwrap();
        let restored = get_run(&state, &run.id, "codex", "/tmp/factory-project")
            .await
            .unwrap();
        assert_eq!(restored.factory.as_ref().unwrap().artifacts.len(), 1);
        assert_eq!(restored.factory.as_ref().unwrap().evidence.len(), 1);

        let mut tampered = restored;
        let recent_generation = tampered.factory.as_ref().unwrap().prior_claims[0].generation;
        let evidence = &mut tampered.factory.as_mut().unwrap().evidence[0];
        evidence.claim_id = uuid::Uuid::new_v4().to_string();
        evidence.claim_generation = recent_generation;
        assert!(validate_runs(std::slice::from_ref(&tampered)).is_err());

        let mut tampered_evicted = current;
        let evidence = &mut tampered_evicted.factory.as_mut().unwrap().evidence[0];
        evidence.claim_id = uuid::Uuid::new_v4().to_string();
        evidence.claim_generation = first_claim.generation;
        assert!(validate_runs(std::slice::from_ref(&tampered_evicted)).is_err());
    }
}
