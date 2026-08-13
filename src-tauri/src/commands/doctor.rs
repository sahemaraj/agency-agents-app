use std::path::Path;

use serde::Serialize;
use tauri::State;

use crate::commands::settings::SettingsLoadState;
use crate::error::AppError;
use crate::state::AppState;
use crate::types::{CatalogSource, StorageMigrationState};

pub(crate) const MAX_FIELD_CHARS: usize = 512;
pub(crate) const MAX_REPORT_CHARS: usize = 32 * 1024;
const MAX_CHECKS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DoctorClassification {
    Healthy,
    Unavailable,
    NeedsAttention,
}

impl DoctorClassification {
    fn label(self) -> &'static str {
        match self {
            Self::Healthy => "Healthy",
            Self::NeedsAttention => "Needs attention",
            Self::Unavailable => "Unavailable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DoctorCategory {
    Core,
    Library,
    Installations,
    Tools,
    Integrations,
    Updates,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DoctorAction {
    RetryDoctor,
    OpenCatalog,
    OpenAgents,
    OpenSkills,
    OpenTools,
    OpenMcp,
    OpenNetwork,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorCheck {
    pub id: String,
    pub category: DoctorCategory,
    pub title: String,
    pub classification: DoctorClassification,
    pub evidence: String,
    pub guidance: Option<String>,
    pub action: Option<DoctorAction>,
}

impl DoctorCheck {
    fn new(
        id: impl Into<String>,
        category: DoctorCategory,
        title: impl AsRef<str>,
        classification: DoctorClassification,
        evidence: impl AsRef<str>,
        guidance: Option<String>,
        action: Option<DoctorAction>,
    ) -> Self {
        let home = dirs::home_dir();
        Self {
            id: sanitize_field(&id.into(), home.as_deref()),
            category,
            title: sanitize_field(title.as_ref(), home.as_deref()),
            classification,
            evidence: sanitize_field(evidence.as_ref(), home.as_deref()),
            guidance: guidance.map(|value| sanitize_field(&value, home.as_deref())),
            action,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorCounts {
    pub healthy: u32,
    pub needs_attention: u32,
    pub unavailable: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorReport {
    pub generated_at: String,
    pub overall: DoctorClassification,
    pub counts: DoctorCounts,
    pub checks: Vec<DoctorCheck>,
    pub copy_text: String,
}

#[derive(Debug)]
struct DoctorAuthorityFailure {
    id: String,
    category: DoctorCategory,
    title: String,
    detail: String,
    action: DoctorAction,
}

#[derive(Debug)]
struct DoctorEvidenceState {
    classification: DoctorClassification,
    evidence: String,
    guidance: Option<String>,
}

impl DoctorEvidenceState {
    fn healthy(evidence: impl Into<String>) -> Self {
        Self {
            classification: DoctorClassification::Healthy,
            evidence: evidence.into(),
            guidance: None,
        }
    }

    fn unavailable(evidence: impl Into<String>) -> Self {
        Self {
            classification: DoctorClassification::Unavailable,
            evidence: evidence.into(),
            guidance: Some(
                "Open the related surface for current evidence, then retry Doctor.".into(),
            ),
        }
    }

    fn needs_attention(evidence: impl Into<String>) -> Self {
        Self {
            classification: DoctorClassification::NeedsAttention,
            evidence: evidence.into(),
            guidance: Some(
                "Open the related surface and review its existing recovery controls.".into(),
            ),
        }
    }
}

type EvidenceResult = Result<DoctorEvidenceState, String>;

#[derive(Debug)]
struct DoctorEvidence {
    storage: EvidenceResult,
    settings: EvidenceResult,
    catalog: EvidenceResult,
    agent_sources: EvidenceResult,
    skill_sources: EvidenceResult,
    agent_installs: EvidenceResult,
    skill_installs: EvidenceResult,
    tools: EvidenceResult,
    mcp_clients: EvidenceResult,
    updates: EvidenceResult,
}

fn evidence_check(
    id: &'static str,
    category: DoctorCategory,
    title: &'static str,
    action: DoctorAction,
    state: EvidenceResult,
) -> Result<DoctorCheck, DoctorAuthorityFailure> {
    state
        .map(|state| {
            DoctorCheck::new(
                id,
                category,
                title,
                state.classification,
                state.evidence,
                state.guidance,
                (state.classification != DoctorClassification::Healthy).then_some(action),
            )
        })
        .map_err(|detail| DoctorAuthorityFailure::new(id, category, title, detail, action))
}

fn report_from_evidence(generated_at: &str, evidence: DoctorEvidence) -> DoctorReport {
    build_report(
        generated_at,
        vec![
            evidence_check(
                "storage",
                DoctorCategory::Core,
                "Storage",
                DoctorAction::RetryDoctor,
                evidence.storage,
            ),
            evidence_check(
                "settings",
                DoctorCategory::Core,
                "Settings",
                DoctorAction::OpenNetwork,
                evidence.settings,
            ),
            evidence_check(
                "catalog",
                DoctorCategory::Library,
                "Catalog",
                DoctorAction::OpenCatalog,
                evidence.catalog,
            ),
            evidence_check(
                "agent-sources",
                DoctorCategory::Library,
                "Agent sources",
                DoctorAction::OpenAgents,
                evidence.agent_sources,
            ),
            evidence_check(
                "skill-sources",
                DoctorCategory::Library,
                "Skill sources",
                DoctorAction::OpenSkills,
                evidence.skill_sources,
            ),
            evidence_check(
                "agent-installs",
                DoctorCategory::Installations,
                "Agent installations",
                DoctorAction::OpenAgents,
                evidence.agent_installs,
            ),
            evidence_check(
                "skill-installs",
                DoctorCategory::Installations,
                "Skill installations",
                DoctorAction::OpenSkills,
                evidence.skill_installs,
            ),
            evidence_check(
                "tools",
                DoctorCategory::Tools,
                "Deployment tools",
                DoctorAction::OpenTools,
                evidence.tools,
            ),
            evidence_check(
                "mcp-clients",
                DoctorCategory::Integrations,
                "MCP clients",
                DoctorAction::OpenMcp,
                evidence.mcp_clients,
            ),
            evidence_check(
                "updates",
                DoctorCategory::Updates,
                "Update state",
                DoctorAction::OpenNetwork,
                evidence.updates,
            ),
        ],
    )
}

fn storage_evidence(status: crate::types::StorageMigrationStatus) -> DoctorEvidenceState {
    match status.state {
        StorageMigrationState::Complete if status.legacy_conflicts.is_empty() => {
            DoctorEvidenceState::healthy("SQLite control-plane storage is complete")
        }
        StorageMigrationState::Complete => DoctorEvidenceState::needs_attention(format!(
            "Storage is complete with {} unresolved legacy conflict(s)",
            status.legacy_conflicts.len()
        )),
        StorageMigrationState::Legacy | StorageMigrationState::InProgress => {
            DoctorEvidenceState::needs_attention(
                status
                    .detail
                    .unwrap_or_else(|| "The data update is incomplete".into()),
            )
        }
        StorageMigrationState::Corrupt | StorageMigrationState::Unsupported => {
            DoctorEvidenceState::needs_attention(
                status
                    .detail
                    .unwrap_or_else(|| "Storage cannot be used safely".into()),
            )
        }
    }
}

fn settings_evidence(state: &SettingsLoadState) -> DoctorEvidenceState {
    match state {
        SettingsLoadState::FirstLaunch => {
            DoctorEvidenceState::healthy("No saved settings; safe defaults are active")
        }
        SettingsLoadState::Loaded(settings) => DoctorEvidenceState::healthy(format!(
            "Settings loaded; offline mode {}; automatic update checks {}",
            if settings.paranoid_mode { "on" } else { "off" },
            if settings.update_auto_check {
                "on"
            } else {
                "off"
            }
        )),
        SettingsLoadState::Corrupt { message } => {
            DoctorEvidenceState::needs_attention(format!("Settings cannot be read: {message}"))
        }
    }
}

async fn catalog_evidence(state: &AppState) -> Result<DoctorEvidenceState, String> {
    let source = crate::corpus::load_catalog_source_checked(&state.app_data_dir)
        .await
        .map_err(|error| error.to_string())?;
    let root = crate::corpus::catalog_root(&state.app_data_dir, &source);
    let label = match &source {
        CatalogSource::Bundled => "bundled",
        CatalogSource::Managed { .. } => "managed",
        CatalogSource::UserClone { .. } => "user clone",
    };
    match tokio::fs::metadata(&root).await {
        Ok(metadata) if metadata.is_dir() => Ok(DoctorEvidenceState::healthy(format!(
            "The configured {label} catalog is available locally"
        ))),
        Ok(_) => Ok(DoctorEvidenceState::needs_attention(format!(
            "The configured {label} catalog path is not a directory"
        ))),
        Err(error) if matches!(source, CatalogSource::Bundled) => {
            Ok(DoctorEvidenceState::unavailable(format!(
                "The bundled catalog has not been initialized locally: {error}"
            )))
        }
        Err(error) => Ok(DoctorEvidenceState::needs_attention(format!(
            "The configured {label} catalog is unavailable: {error}"
        ))),
    }
}

async fn agent_sources_evidence(state: &AppState) -> Result<DoctorEvidenceState, String> {
    let results = crate::agents::inspect_agent_sources(&state.app_data_dir)
        .await
        .map_err(|error| error.to_string())?;
    let packages = results
        .iter()
        .map(|result| result.agents.len())
        .sum::<usize>();
    let errors = results
        .iter()
        .map(|result| result.errors.len())
        .sum::<usize>();
    if errors > 0 {
        Ok(DoctorEvidenceState::needs_attention(format!(
            "{} enabled source(s), {packages} Agent(s), {errors} validation error(s)",
            results.len()
        )))
    } else if packages == 0 {
        Ok(DoctorEvidenceState::unavailable(format!(
            "{} enabled source(s) returned no Agents",
            results.len()
        )))
    } else {
        Ok(DoctorEvidenceState::healthy(format!(
            "{} enabled source(s), {packages} Agent(s)",
            results.len()
        )))
    }
}

async fn skill_sources_evidence(state: &AppState) -> Result<DoctorEvidenceState, String> {
    let sources = crate::skills::load_skill_sources_for_state(state)
        .await
        .map_err(|error| error.to_string())?;
    if sources.is_empty() {
        return Ok(DoctorEvidenceState::unavailable(
            "No Skill source is registered",
        ));
    }
    let mut packages = 0usize;
    let mut errors = 0usize;
    for source in sources.iter().cloned() {
        match crate::skills::discover_source(source).await {
            Ok(result) => {
                packages += result.packages.len();
                errors += result.errors.len();
            }
            Err(_) => errors += 1,
        }
    }
    if errors > 0 {
        Ok(DoctorEvidenceState::needs_attention(format!(
            "{} source(s), {packages} Skill(s), {errors} validation error(s); trust credentials were not read",
            sources.len()
        )))
    } else {
        Ok(DoctorEvidenceState::healthy(format!(
            "{} source(s), {packages} Skill(s); trust credentials were not read",
            sources.len()
        )))
    }
}

fn destination_presence<'a>(
    destinations: impl Iterator<Item = (&'a str, Option<&'a str>)>,
) -> (usize, usize) {
    destinations.fold((0, 0), |(present, missing), (active, disabled)| {
        if Path::new(active).exists() || disabled.is_some_and(|path| Path::new(path).exists()) {
            (present + 1, missing)
        } else {
            (present, missing + 1)
        }
    })
}

async fn agent_installs_evidence(state: &AppState) -> Result<DoctorEvidenceState, String> {
    let records = crate::install::load_ledger_read_only(state)
        .await
        .map_err(|error| error.to_string())?;
    let (present, missing) = destination_presence(
        records
            .iter()
            .map(|record| (record.dest.as_str(), record.disabled_path.as_deref())),
    );
    if missing > 0 {
        Ok(DoctorEvidenceState::needs_attention(format!(
            "{present} tracked Agent destination(s) present; {missing} missing; run reconciliation for full hash and source truth"
        )))
    } else if records.is_empty() {
        Ok(DoctorEvidenceState::healthy(
            "The Agent install ledger is readable and empty",
        ))
    } else {
        Ok(DoctorEvidenceState::unavailable(format!(
            "All {present} tracked Agent destination(s) exist; run reconciliation for current hash and source truth"
        )))
    }
}

async fn skill_installs_evidence(state: &AppState) -> Result<DoctorEvidenceState, String> {
    let records = if state
        .completed_state_database()
        .await
        .map_err(|error| error.to_string())?
        .is_some()
    {
        crate::skills::install::load_ledger_for_state(state).await
    } else {
        crate::skills::install::load_ledger_checked(&state.app_data_dir).await
    }
    .map_err(|error| error.to_string())?;
    let (present, missing) = destination_presence(
        records
            .iter()
            .map(|record| (record.dest.as_str(), record.disabled_path.as_deref())),
    );
    if missing > 0 {
        Ok(DoctorEvidenceState::needs_attention(format!(
            "{present} tracked Skill destination(s) present; {missing} missing; run reconciliation for full hash and source truth"
        )))
    } else if records.is_empty() {
        Ok(DoctorEvidenceState::healthy(
            "The Skill install ledger is readable and empty",
        ))
    } else {
        Ok(DoctorEvidenceState::unavailable(format!(
            "All {present} tracked Skill destination(s) exist; run reconciliation for current hash and source truth"
        )))
    }
}

async fn tools_evidence(state: &AppState) -> Result<DoctorEvidenceState, String> {
    let mut detected = 0usize;
    for tool in crate::registry::all() {
        if crate::install::tool_detected(state, &tool.id)
            .await
            .map_err(|error| error.to_string())?
        {
            detected += 1;
        }
    }
    if detected == 0 {
        Ok(DoctorEvidenceState::unavailable(format!(
            "No deployment tool detected from {} known local definitions",
            crate::registry::all().len()
        )))
    } else {
        Ok(DoctorEvidenceState::healthy(format!(
            "{detected} of {} known deployment tools detected locally",
            crate::registry::all().len()
        )))
    }
}

fn mcp_evidence() -> Result<DoctorEvidenceState, String> {
    Ok(DoctorEvidenceState::unavailable(
        "MCP registration is not passively cached; Doctor does not run client commands. Open MCP clients for an explicit local check",
    ))
}

async fn update_evidence(state: &AppState) -> Result<DoctorEvidenceState, String> {
    let auto = state
        .settings
        .read()
        .await
        .effective_settings()
        .map(|settings| settings.update_auto_check);
    let cached = state.updater_state.read().await;
    match (&cached.last_outcome, cached.last_checked_at, auto) {
        (Some(crate::commands::UpdateCheckOutcome::UpToDate), Some(stamp), _) => {
            Ok(DoctorEvidenceState::healthy(format!(
                "Cached update check at Unix time {stamp}: up to date"
            )))
        }
        (Some(crate::commands::UpdateCheckOutcome::Available { version, .. }), Some(stamp), _) => {
            Ok(DoctorEvidenceState::needs_attention(format!(
                "Cached update check at Unix time {stamp}: version {version} is available"
            )))
        }
        (_, _, Some(auto)) => Ok(DoctorEvidenceState::unavailable(format!(
            "Automatic update checks are {}; no cached result is available",
            if auto { "on" } else { "off" }
        ))),
        _ => Ok(DoctorEvidenceState::unavailable(
            "Update configuration or cached state cannot be verified",
        )),
    }
}

async fn collect_evidence(state: &AppState) -> DoctorEvidence {
    let storage = crate::state::migration_status(state)
        .await
        .map(storage_evidence)
        .map_err(|error| error.to_string());
    let settings = {
        let settings = state.settings.read().await;
        Ok(settings_evidence(&settings))
    };
    let catalog = catalog_evidence(state).await;
    let agent_sources = agent_sources_evidence(state).await;
    let skill_sources = skill_sources_evidence(state).await;
    let agent_installs = agent_installs_evidence(state).await;
    let skill_installs = skill_installs_evidence(state).await;
    let tools = tools_evidence(state).await;
    let mcp_clients = mcp_evidence();
    let updates = update_evidence(state).await;

    DoctorEvidence {
        storage,
        settings,
        catalog,
        agent_sources,
        skill_sources,
        agent_installs,
        skill_installs,
        tools,
        mcp_clients,
        updates,
    }
}

#[tauri::command]
pub async fn doctor_report(state: State<'_, AppState>) -> Result<DoctorReport, AppError> {
    Ok(report_from_evidence(
        &chrono::Utc::now().to_rfc3339(),
        collect_evidence(&state).await,
    ))
}

impl DoctorAuthorityFailure {
    fn new(
        id: impl Into<String>,
        category: DoctorCategory,
        title: impl Into<String>,
        detail: impl Into<String>,
        action: DoctorAction,
    ) -> Self {
        Self {
            id: id.into(),
            category,
            title: title.into(),
            detail: detail.into(),
            action,
        }
    }

    fn into_check(self) -> DoctorCheck {
        DoctorCheck::new(
            self.id,
            self.category,
            self.title,
            DoctorClassification::Unavailable,
            self.detail,
            Some("Retry Doctor or open the related settings to inspect the source.".into()),
            Some(self.action),
        )
    }
}

fn take_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn redact_private_key(mut value: String) -> String {
    loop {
        let upper = value.to_ascii_uppercase();
        let Some(start) = upper.find("-----BEGIN ") else {
            break;
        };
        let Some(relative_end) = upper[start..].find("-----END ") else {
            value.replace_range(start.., "[redacted]");
            break;
        };
        let end_start = start + relative_end;
        let marker_body = end_start + 5;
        let end = value[marker_body..]
            .find("-----")
            .map(|offset| marker_body + offset + 5)
            .unwrap_or(value.len());
        value.replace_range(start..end, "[redacted]");
    }
    value
}

fn redact_token(token: &str, home: Option<&Path>) -> String {
    let lower = token.to_ascii_lowercase();
    if lower.starts_with("ghp_")
        || lower.starts_with("gho_")
        || lower.starts_with("ghu_")
        || lower.starts_with("ghs_")
        || lower.starts_with("ghr_")
        || (lower.starts_with("sk-") && token.len() >= 10)
    {
        return "[redacted]".into();
    }
    if let Some((key, separator)) = token
        .split_once('=')
        .map(|(key, _)| (key, '='))
        .or_else(|| token.split_once(':').map(|(key, _)| (key, ':')))
    {
        if matches!(
            key.trim_matches(|character: char| !character.is_ascii_alphanumeric()
                && character != '_'
                && character != '-')
                .to_ascii_lowercase()
                .as_str(),
            "token" | "secret" | "password" | "apikey" | "api_key" | "api-key"
        ) {
            return format!("{key}{separator}[redacted]");
        }
    }
    let mut safe = token.to_string();
    if let Some(scheme) = safe.find("://") {
        let authority = scheme + 3;
        if let Some(at) = safe[authority..].find('@') {
            safe.replace_range(authority..authority + at, "[redacted]");
        }
    }
    if let Some(home) = home.and_then(Path::to_str) {
        safe = safe.replace(home, "~");
    }
    for prefix in ["/Users/", "/home/"] {
        while let Some(start) = safe.find(prefix) {
            let name_start = start + prefix.len();
            let suffix_start = safe[name_start..]
                .find('/')
                .map(|offset| name_start + offset)
                .unwrap_or(safe.len());
            safe.replace_range(start..suffix_start, "~");
        }
    }
    safe
}

fn sanitize_field(value: &str, home: Option<&Path>) -> String {
    let filtered: String = value
        .chars()
        .filter(|character| !character.is_control() || character.is_whitespace())
        .collect();
    let mut words = Vec::new();
    let mut redact_next = false;
    for word in redact_private_key(filtered).split_whitespace() {
        if redact_next {
            words.push("[redacted]".to_string());
            redact_next = false;
            continue;
        }
        if word.eq_ignore_ascii_case("bearer") {
            words.push("[redacted]".to_string());
            redact_next = true;
            continue;
        }
        if word.eq_ignore_ascii_case("authorization:") {
            words.push("Authorization:".to_string());
            continue;
        }
        words.push(redact_token(word, home));
    }
    take_chars(&words.join(" "), MAX_FIELD_CHARS)
}

fn format_report(report: &DoctorReport) -> String {
    let mut output = format!(
        "Agency Agents Doctor\nGenerated: {}\nOverall: {}\nHealthy: {} | Needs attention: {} | Unavailable: {}\n",
        report.generated_at,
        report.overall.label(),
        report.counts.healthy,
        report.counts.needs_attention,
        report.counts.unavailable
    );
    for check in &report.checks {
        output.push_str(&format!(
            "\n[{}] {} — {}\nEvidence: {}\n",
            check.id,
            check.title,
            check.classification.label(),
            check.evidence
        ));
        if let Some(guidance) = &check.guidance {
            output.push_str(&format!("Next: {guidance}\n"));
        }
    }
    take_chars(&output, MAX_REPORT_CHARS)
}

fn build_report(
    generated_at: impl AsRef<str>,
    checks: Vec<Result<DoctorCheck, DoctorAuthorityFailure>>,
) -> DoctorReport {
    let mut checks = checks
        .into_iter()
        .take(MAX_CHECKS)
        .map(|result| result.unwrap_or_else(DoctorAuthorityFailure::into_check))
        .collect::<Vec<_>>();
    checks.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut counts = DoctorCounts::default();
    for check in &checks {
        match check.classification {
            DoctorClassification::Healthy => counts.healthy += 1,
            DoctorClassification::NeedsAttention => counts.needs_attention += 1,
            DoctorClassification::Unavailable => counts.unavailable += 1,
        }
    }
    let overall = if counts.needs_attention > 0 {
        DoctorClassification::NeedsAttention
    } else if counts.unavailable > 0 {
        DoctorClassification::Unavailable
    } else {
        DoctorClassification::Healthy
    };
    let mut report = DoctorReport {
        generated_at: sanitize_field(generated_at.as_ref(), None),
        overall,
        counts,
        checks,
        copy_text: String::new(),
    };
    report.copy_text = format_report(&report);
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state(app_data_dir: &Path) -> AppState {
        AppState {
            app_data_dir: app_data_dir.to_path_buf(),
            corpus_cache: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            corpus_refresh_in_flight: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            skill_sources_write_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            skill_installs_write_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            skill_folders_write_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            settings: std::sync::Arc::new(tokio::sync::RwLock::new(SettingsLoadState::FirstLaunch)),
            updater_state: crate::commands::updater::empty_state(),
        }
    }

    fn check(
        id: &'static str,
        category: DoctorCategory,
        classification: DoctorClassification,
    ) -> DoctorCheck {
        DoctorCheck::new(
            id,
            category,
            id,
            classification,
            format!("evidence for {id}"),
            None,
            None,
        )
    }

    #[test]
    fn report_order_and_overall_severity_are_deterministic() {
        let report = build_report(
            "2026-08-13T12:00:00Z",
            vec![
                Ok(check(
                    "mcp-clients",
                    DoctorCategory::Integrations,
                    DoctorClassification::Unavailable,
                )),
                Ok(check(
                    "catalog",
                    DoctorCategory::Library,
                    DoctorClassification::Healthy,
                )),
                Ok(check(
                    "settings",
                    DoctorCategory::Core,
                    DoctorClassification::NeedsAttention,
                )),
                Ok(check(
                    "storage",
                    DoctorCategory::Core,
                    DoctorClassification::Healthy,
                )),
            ],
        );

        assert_eq!(report.overall, DoctorClassification::NeedsAttention);
        assert_eq!(report.counts.healthy, 2);
        assert_eq!(report.counts.needs_attention, 1);
        assert_eq!(report.counts.unavailable, 1);
        assert_eq!(
            report
                .checks
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["settings", "storage", "catalog", "mcp-clients"]
        );
        assert_eq!(report.copy_text, format_report(&report));
    }

    #[test]
    fn failed_authority_becomes_one_unavailable_check() {
        let report = build_report(
            "2026-08-13T12:00:00Z",
            vec![
                Ok(check(
                    "storage",
                    DoctorCategory::Core,
                    DoctorClassification::Healthy,
                )),
                Err(DoctorAuthorityFailure::new(
                    "catalog",
                    DoctorCategory::Library,
                    "Catalog",
                    "token=secret123 /Users/alice/private/catalog.json",
                    DoctorAction::OpenCatalog,
                )),
            ],
        );

        assert_eq!(report.overall, DoctorClassification::Unavailable);
        assert_eq!(report.checks.len(), 2);
        assert_eq!(
            report.checks[0].classification,
            DoctorClassification::Healthy
        );
        assert_eq!(
            report.checks[1].classification,
            DoctorClassification::Unavailable
        );
        assert_eq!(report.checks[1].action, Some(DoctorAction::OpenCatalog));
        assert!(!report.copy_text.contains("secret123"));
        assert!(!report.copy_text.contains("/Users/alice"));
    }

    #[test]
    fn sanitization_bounds_and_redacts_sensitive_evidence() {
        let home = std::path::Path::new("/Users/alice");
        let raw = format!(
            "\u{0007}/Users/alice/private/config token=secret123 api_key:also-secret Authorization: Bearer abc123 \
             https://user:password@example.test/path ghp_abcdefghijklmnopqrstuvwxyz \
             -----BEGIN PRIVATE KEY----- hidden -----END PRIVATE KEY----- {}",
            "x".repeat(2_000)
        );
        let safe = sanitize_field(&raw, Some(home));

        assert!(safe.chars().count() <= MAX_FIELD_CHARS);
        assert!(safe.contains("~/private/config"));
        for forbidden in [
            "\u{0007}",
            "secret123",
            "also-secret",
            "Bearer abc123",
            "user:password",
            "ghp_abcdefghijklmnopqrstuvwxyz",
            "hidden",
            "END PRIVATE KEY",
            "/Users/alice",
        ] {
            assert!(!safe.contains(forbidden), "leaked {forbidden:?}: {safe}");
        }
        assert!(safe.contains("[redacted]"));
    }

    #[test]
    fn report_and_safe_actions_are_bounded_and_closed() {
        let checks = (0..200)
            .map(|index| {
                Ok(DoctorCheck::new(
                    format!("check-{index:03}"),
                    DoctorCategory::Updates,
                    "x".repeat(2_000),
                    DoctorClassification::Unavailable,
                    "y".repeat(2_000),
                    Some("z".repeat(2_000)),
                    Some(DoctorAction::RetryDoctor),
                ))
            })
            .collect();
        let report = build_report("2026-08-13T12:00:00Z", checks);

        assert!(report.copy_text.chars().count() <= MAX_REPORT_CHARS);
        assert!(serde_json::to_vec(&report).unwrap().len() <= MAX_REPORT_CHARS * 2);
        assert_eq!(
            serde_json::to_string(&DoctorAction::OpenSkills).unwrap(),
            "\"openSkills\""
        );
        assert_eq!(
            serde_json::to_string(&DoctorAction::OpenNetwork).unwrap(),
            "\"openNetwork\""
        );
    }

    #[test]
    fn local_evidence_covers_every_authority_and_isolates_failure() {
        let evidence = DoctorEvidence {
            storage: Ok(DoctorEvidenceState::healthy("SQLite state is complete")),
            settings: Ok(DoctorEvidenceState::healthy("Settings loaded")),
            catalog: Err("catalog metadata cannot be read".into()),
            agent_sources: Ok(DoctorEvidenceState::healthy("2 enabled sources; 41 Agents")),
            skill_sources: Ok(DoctorEvidenceState::healthy("3 sources; 120 Skills")),
            agent_installs: Ok(DoctorEvidenceState::unavailable(
                "12 tracked installs; run reconciliation for current disk truth",
            )),
            skill_installs: Ok(DoctorEvidenceState::needs_attention(
                "1 tracked destination is missing",
            )),
            tools: Ok(DoctorEvidenceState::healthy("2 of 15 tools detected")),
            mcp_clients: Ok(DoctorEvidenceState::unavailable(
                "No MCP client is connected",
            )),
            updates: Ok(DoctorEvidenceState::unavailable(
                "Automatic checks are off; no cached check exists",
            )),
        };

        let report = report_from_evidence("2026-08-13T12:00:00Z", evidence);
        assert_eq!(report.checks.len(), 10);
        assert_eq!(report.overall, DoctorClassification::NeedsAttention);
        assert_eq!(
            report
                .checks
                .iter()
                .map(|check| check.id.as_str())
                .collect::<Vec<_>>(),
            [
                "settings",
                "storage",
                "agent-sources",
                "catalog",
                "skill-sources",
                "agent-installs",
                "skill-installs",
                "tools",
                "mcp-clients",
                "updates",
            ]
        );
        let catalog = report
            .checks
            .iter()
            .find(|check| check.id == "catalog")
            .unwrap();
        assert_eq!(catalog.classification, DoctorClassification::Unavailable);
        assert!(report.checks.iter().any(|check| {
            check.id == "storage" && check.classification == DoctorClassification::Healthy
        }));
    }

    #[test]
    fn mcp_registration_stays_unavailable_without_executing_a_client() {
        let evidence = mcp_evidence().unwrap();
        assert_eq!(evidence.classification, DoctorClassification::Unavailable);
        assert!(evidence.evidence.contains("does not run client commands"));
    }

    #[tokio::test]
    async fn evidence_collection_does_not_create_application_state() {
        let app = tempfile::tempdir().unwrap();
        let state = test_state(app.path());

        let _ = collect_evidence(&state).await;

        assert!(!app.path().join("state").exists());
    }

    #[tokio::test]
    async fn evidence_collection_does_not_create_storage_locks() {
        let app = tempfile::tempdir().unwrap();
        drop(crate::state_db::StateDatabase::open(app.path()).unwrap());
        let schema_lock = app.path().join("state/schema.lock");
        std::fs::remove_file(&schema_lock).unwrap();
        let database_path = app.path().join("state/agency-agents.sqlite3");
        let database_before = std::fs::read(&database_path).unwrap();
        let state = test_state(app.path());

        let _ = collect_evidence(&state).await;

        assert!(!schema_lock.exists());
        assert_eq!(std::fs::read(database_path).unwrap(), database_before);
    }
}
