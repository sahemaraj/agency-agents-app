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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    let path = path(state);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = read_capped(&path, MAX_RUN_BYTES).await?;
    serde_json::from_slice(&raw).map_err(|error| invalid(format!("parse Expert runs: {error}")))
}

async fn save(state: &AppState, runs: &[ExpertRun]) -> Result<(), AppError> {
    let bytes = serde_json::to_vec_pretty(runs)
        .map_err(|error| invalid(format!("serialize Expert runs: {error}")))?;
    if bytes.len() as u64 > MAX_RUN_BYTES {
        return Err(invalid("Expert run state capacity reached"));
    }
    atomic_write(&path(state), &bytes).await
}

fn validate_text(value: &str, field: &str) -> Result<(), AppError> {
    if value.trim().is_empty() || value.len() > MAX_TEXT {
        return Err(invalid(format!("{field} is empty or oversized")));
    }
    Ok(())
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
    validate_text(&create.expert_id, "expertId")?;
    validate_text(&create.project_path, "projectPath")?;
    validate_text(&create.client, "client")?;
    validate_contract(&create.contract)?;
    let _lock = lock(state)?;
    let mut runs = load(state).await?;
    if runs.len() >= MAX_RUNS {
        let index = runs
            .iter()
            .position(|run| run.state.terminal())
            .ok_or_else(|| invalid("Expert run state capacity reached"))?;
        runs.remove(index);
    }
    let run = ExpertRun {
        id: uuid::Uuid::new_v4().to_string(),
        snapshot: create,
        state: ExpertRunState::InProgress,
        started_at: chrono::Utc::now().to_rfc3339(),
        ended_at: None,
        evidence: Vec::new(),
        blockers: Vec::new(),
        waivers: Vec::new(),
    };
    runs.push(run.clone());
    save(state, &runs).await?;
    Ok(run)
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

pub fn mcp_view(run: &ExpertRun) -> serde_json::Value {
    let mut value = serde_json::to_value(run).unwrap_or_else(|_| serde_json::json!({}));
    value["waivers"] = serde_json::Value::Array(
        run.waivers
            .iter()
            .map(|waiver| serde_json::json!({ "checkName": waiver.check_name, "waived": true }))
            .collect(),
    );
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
