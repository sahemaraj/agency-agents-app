use std::path::PathBuf;

use serde::Serialize;

use crate::install::lockfile::{
    cli_lock_apply, cli_lock_check, cli_lock_plan, LockCheckResult, LockOperation, LockPlan,
};
use crate::state::AppState;

const OUTPUT_VERSION: u8 = 1;

pub struct CliOutcome {
    pub stdout: String,
    pub exit_code: i32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CheckJson<'a> {
    version: u8,
    command: &'static str,
    project_path: &'a str,
    clean: bool,
    entries: &'a [crate::install::lockfile::LockCheckEntry],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanJson<'a> {
    version: u8,
    command: &'static str,
    plan: &'a LockPlan,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ApplyJson<'a> {
    version: u8,
    command: &'static str,
    project_path: &'a str,
    dry_run: bool,
    applied: bool,
    operations: &'a [LockOperation],
    warnings: &'a [String],
    blockers: &'a [String],
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct InventoryItem {
    tool: String,
    kind: &'static str,
    name: String,
    state: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ListJson<'a> {
    version: u8,
    command: &'static str,
    project_path: &'a str,
    items: &'a [InventoryItem],
}

pub async fn run(
    command: &str,
    project: Option<PathBuf>,
    json: bool,
    dry_run: bool,
    merge: bool,
) -> Result<CliOutcome, String> {
    #[cfg(target_os = "windows")]
    return Err("CLI mode is supported on macOS and Linux only".into());

    #[cfg(not(target_os = "windows"))]
    {
        let project = match project {
            Some(project) => project,
            None => std::env::current_dir().map_err(|error| error.to_string())?,
        };
        let project = canonical_project(&project.to_string_lossy())?
            .to_string_lossy()
            .into_owned();
        let state = AppState::build().map_err(|error| error.to_string())?;
        crate::corpus::ensure_corpus_headless(&state)
            .await
            .map_err(|error| error.to_string())?;
        match command {
            "check" => run_check(&state, &project, json).await,
            "plan" => run_plan(&state, &project, json).await,
            "apply" => run_apply(&state, &project, json, dry_run, merge).await,
            "list" => run_list(&state, &project, json).await,
            _ => Err(format!("unknown CLI command: {command}")),
        }
    }
}

async fn run_check(state: &AppState, project: &str, json: bool) -> Result<CliOutcome, String> {
    let check = cli_lock_check(state, project)
        .await
        .map_err(|error| error.to_string())?;
    let stdout = if json {
        json_line(&CheckJson {
            version: OUTPUT_VERSION,
            command: "check",
            project_path: project,
            clean: check.clean,
            entries: &check.entries,
        })?
    } else {
        human_check(&check)
    };
    Ok(CliOutcome {
        stdout,
        exit_code: check_exit(check.clean),
    })
}

async fn run_plan(state: &AppState, project: &str, json: bool) -> Result<CliOutcome, String> {
    let plan = cli_lock_plan(state, project, false)
        .await
        .map_err(|error| error.to_string())?;
    let stdout = if json {
        json_line(&PlanJson {
            version: OUTPUT_VERSION,
            command: "plan",
            plan: &plan,
        })?
    } else {
        human_plan(&plan, "plan")
    };
    Ok(CliOutcome {
        stdout,
        exit_code: 0,
    })
}

async fn run_apply(
    state: &AppState,
    project: &str,
    json: bool,
    dry_run: bool,
    merge: bool,
) -> Result<CliOutcome, String> {
    let plan = cli_lock_plan(state, project, merge)
        .await
        .map_err(|error| error.to_string())?;
    let (applied, blockers) = if dry_run || !plan.blockers.is_empty() {
        (false, plan.blockers.clone())
    } else {
        let response = cli_lock_apply(state, project, &plan.revision, merge)
            .await
            .map_err(|error| error.to_string())?;
        (response.applied, response.plan.blockers)
    };
    let stdout = if json {
        json_line(&ApplyJson {
            version: OUTPUT_VERSION,
            command: "apply",
            project_path: project,
            dry_run,
            applied,
            operations: &plan.operations,
            warnings: &plan.warnings,
            blockers: &blockers,
        })?
    } else if dry_run {
        human_plan(&plan, "dry run")
    } else if blockers.is_empty() && applied {
        format!("applied: {} operation(s)\n", plan.operations.len())
    } else {
        let mut output = format!("apply blocked: {} blocker(s)\n", blockers.len());
        for blocker in &blockers {
            output.push_str(&format!("blocker: {blocker}\n"));
        }
        output
    };
    Ok(CliOutcome {
        stdout,
        exit_code: apply_exit(dry_run || applied, blockers.len()),
    })
}

async fn run_list(state: &AppState, project: &str, json: bool) -> Result<CliOutcome, String> {
    let project = canonical_project(project)?;
    let project_string = project.to_string_lossy().into_owned();
    let agents = crate::install::mcp_reconcile_agent_installs(state)
        .await
        .map_err(|error| error.to_string())?;
    let skills =
        crate::skills::reconcile_skill_installs(state, std::slice::from_ref(&project_string))
            .await
            .map_err(|error| error.to_string())?;
    let mut items = agents
        .into_iter()
        .filter(|item| item.tracked && item.project_path.as_deref() == Some(&project_string))
        .map(|item| InventoryItem {
            tool: item.tool,
            kind: "agent",
            name: item.name,
            state: state_name(item.state),
        })
        .chain(
            skills
                .into_iter()
                .filter(|item| {
                    item.tracked && item.project_path.as_deref() == Some(&project_string)
                })
                .map(|item| InventoryItem {
                    tool: item.runtime,
                    kind: "skill",
                    name: item.name,
                    state: state_name(item.state),
                }),
        )
        .collect::<Vec<_>>();
    items.sort_by(|left, right| {
        (&left.tool, left.kind, &left.name, &left.state).cmp(&(
            &right.tool,
            right.kind,
            &right.name,
            &right.state,
        ))
    });
    let stdout = if json {
        list_json(&project_string, &items)?
    } else {
        let mut output = format!("list: {} installed\n", items.len());
        for item in &items {
            output.push_str(&format!(
                "{} {} {} {}\n",
                item.tool, item.kind, item.name, item.state
            ));
        }
        output
    };
    Ok(CliOutcome {
        stdout,
        exit_code: 0,
    })
}

fn canonical_project(project: &str) -> Result<PathBuf, String> {
    let project = std::fs::canonicalize(project).map_err(|error| error.to_string())?;
    if !project.is_dir() {
        return Err("project must be a directory".into());
    }
    Ok(project)
}

fn human_check(check: &LockCheckResult) -> String {
    if check.clean {
        return "check: in sync\n".into();
    }
    let mut output = format!("check: drift ({} entry(s))\n", check.entries.len());
    for checked in check
        .entries
        .iter()
        .filter(|entry| entry.status != crate::install::lockfile::LockEntryStatus::Current)
    {
        output.push_str(&format!(
            "{} {} {} {}\n",
            state_name(checked.status),
            checked.entry.kind,
            checked.entry.source_relative_path,
            checked.entry.tool
        ));
    }
    output
}

fn human_plan(plan: &LockPlan, label: &str) -> String {
    let mut output = format!(
        "{label}: {} operation(s), {} blocker(s)\n",
        plan.operations.len(),
        plan.blockers.len()
    );
    for operation in &plan.operations {
        output.push_str(&format!(
            "{} {} {} {}\n",
            operation.action, operation.kind, operation.source_relative_path, operation.tool
        ));
    }
    for warning in &plan.warnings {
        output.push_str(&format!("warning: {warning}\n"));
    }
    for blocker in &plan.blockers {
        output.push_str(&format!("blocker: {blocker}\n"));
    }
    output
}

fn state_name<T: Serialize>(state: T) -> String {
    serde_json::to_value(state)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".into())
}

fn json_line<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value)
        .map(|mut output| {
            output.push('\n');
            output
        })
        .map_err(|error| error.to_string())
}

fn list_json(project: &str, items: &[InventoryItem]) -> Result<String, String> {
    json_line(&ListJson {
        version: OUTPUT_VERSION,
        command: "list",
        project_path: project,
        items,
    })
}

fn check_exit(clean: bool) -> i32 {
    i32::from(!clean)
}

fn apply_exit(completed: bool, blocker_count: usize) -> i32 {
    i32::from(!completed || blocker_count > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_follow_check_and_apply_results() {
        assert_eq!(check_exit(true), 0);
        assert_eq!(check_exit(false), 1);
        assert_eq!(apply_exit(true, 0), 0);
        assert_eq!(apply_exit(true, 1), 1);
        assert_eq!(apply_exit(false, 0), 1);
    }

    #[test]
    fn list_json_shape_and_key_order_are_stable() {
        let output = list_json(
            "/tmp/project",
            &[InventoryItem {
                tool: "codex".into(),
                kind: "agent",
                name: "reviewer".into(),
                state: "current".into(),
            }],
        )
        .unwrap();
        assert_eq!(
            output,
            "{\"version\":1,\"command\":\"list\",\"projectPath\":\"/tmp/project\",\"items\":[{\"tool\":\"codex\",\"kind\":\"agent\",\"name\":\"reviewer\",\"state\":\"current\"}]}\n"
        );
    }

    #[test]
    fn check_and_apply_json_envelopes_are_versioned_and_stable() {
        assert_eq!(
            json_line(&CheckJson {
                version: OUTPUT_VERSION,
                command: "check",
                project_path: "/tmp/project",
                clean: true,
                entries: &[],
            })
            .unwrap(),
            "{\"version\":1,\"command\":\"check\",\"projectPath\":\"/tmp/project\",\"clean\":true,\"entries\":[]}\n"
        );
        assert_eq!(
            json_line(&ApplyJson {
                version: OUTPUT_VERSION,
                command: "apply",
                project_path: "/tmp/project",
                dry_run: true,
                applied: false,
                operations: &[],
                warnings: &[],
                blockers: &[],
            })
            .unwrap(),
            "{\"version\":1,\"command\":\"apply\",\"projectPath\":\"/tmp/project\",\"dryRun\":true,\"applied\":false,\"operations\":[],\"warnings\":[],\"blockers\":[]}\n"
        );
    }
}
