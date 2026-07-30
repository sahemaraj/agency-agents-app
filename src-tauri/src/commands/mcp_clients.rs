use std::{
    env,
    ffi::{OsStr, OsString},
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    process::Stdio,
    thread,
    time::Duration,
};

use tokio::{
    io::AsyncReadExt,
    process::Command,
    time::{timeout, Instant},
};

use crate::{
    error::AppError,
    types::{McpClient, McpClientState, McpClientStatus},
};

const SERVER_NAME: &str = "agency-agents";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const OUTPUT_LIMIT: usize = 64 * 1024;
const LOCK_TIMEOUT: Duration = Duration::from_secs(2);

impl McpClient {
    fn executable(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
        }
    }
}

#[derive(Debug)]
struct ProcessOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Registration {
    command: String,
    args: Vec<String>,
    scope: Option<String>,
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::InvalidArgument {
        message: message.into(),
    }
}

fn app_command(executable: &Path) -> Result<(String, Vec<String>), AppError> {
    let executable = executable
        .canonicalize()
        .map_err(|e| invalid(format!("cannot resolve the app executable: {e}")))?;
    let in_temporary_directory = env::temp_dir()
        .canonicalize()
        .is_ok_and(|temporary| executable.starts_with(temporary));
    if in_temporary_directory || is_ephemeral_executable(&executable) {
        return Err(invalid(
            "move Agency Agents to Applications before connecting an MCP client",
        ));
    }
    Ok((
        executable.to_string_lossy().into_owned(),
        vec!["--mcp".to_string()],
    ))
}

fn is_ephemeral_executable(executable: &Path) -> bool {
    let components = executable
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => {
                Some(value.to_string_lossy().to_ascii_lowercase())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    components.iter().any(|component| {
        matches!(
            component.as_str(),
            "apptranslocation" | ".svelte-kit" | "build"
        )
    }) || components
        .windows(2)
        .any(|pair| pair[0] == "target" && matches!(pair[1].as_str(), "debug" | "release"))
}

fn command_display(command: &str, args: &[String]) -> String {
    std::iter::once(command)
        .chain(args.iter().map(String::as_str))
        .map(|part| {
            if part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c))
            {
                part.to_string()
            } else if cfg!(windows) {
                format!("\"{}\"", part.replace('"', "\\\""))
            } else {
                format!("'{}'", part.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn manual_connect_command(client: McpClient, command: &str, args: &[String]) -> String {
    let argv = add_args(client, command, args)
        .into_iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    command_display(client.executable(), &argv)
}

fn registration(output: &str) -> Option<Registration> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(output) {
        let config = value.get("transport").unwrap_or(&value);
        let nonempty_array = |key: &str| {
            config
                .get(key)
                .and_then(serde_json::Value::as_array)
                .is_some_and(|items| !items.is_empty())
        };
        if value
            .get("enabled")
            .is_some_and(|enabled| enabled.as_bool() != Some(true))
            || value
                .get("disabled_reason")
                .is_some_and(|reason| !reason.is_null())
            || config
                .get("type")
                .is_some_and(|kind| kind.as_str() != Some("stdio"))
            || ["env", "cwd"]
                .iter()
                .any(|key| config.get(*key).is_some_and(|item| !item.is_null()))
            || nonempty_array("env_vars")
            || [
                "enabled_tools",
                "disabled_tools",
                "startup_timeout_sec",
                "tool_timeout_sec",
            ]
            .iter()
            .any(|key| value.get(*key).is_some_and(|item| !item.is_null()))
        {
            return None;
        }
        let command = config.get("command")?.as_str()?.to_string();
        let args = match config.get("args").and_then(serde_json::Value::as_array) {
            Some(items) => items
                .iter()
                .map(|item| item.as_str().map(str::to_string))
                .collect::<Option<Vec<_>>>()?,
            None => Vec::new(),
        };
        return Some(Registration {
            command,
            args,
            scope: None,
        });
    }
    let mut command = None;
    let mut args = Vec::new();
    let mut scope = None;
    for line in output.lines() {
        let Some((key, value)) = line.trim().split_once(':') else {
            continue;
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "command" => command = Some(value.trim().to_string()),
            "scope" => scope = Some(value.trim().to_string()),
            "args" => {
                let value = value.trim();
                args = serde_json::from_str::<Vec<String>>(value)
                    .ok()
                    .or_else(|| {
                        (!value.contains(['\'', '"']))
                            .then(|| value.split_whitespace().map(str::to_string).collect())
                    })?;
            }
            "env" | "environment" | "environment variables" if !value.trim().is_empty() => {
                return None
            }
            _ => {}
        }
    }
    command.map(|command| Registration {
        command,
        args,
        scope,
    })
}

fn is_user_scope(scope: &str) -> bool {
    scope.trim().to_ascii_lowercase().starts_with("user")
}

fn find_client(client: McpClient) -> Option<PathBuf> {
    find_client_in(
        client,
        env::var_os("PATH").as_deref(),
        dirs::home_dir().as_deref(),
    )
}

fn find_client_in(client: McpClient, path: Option<&OsStr>, home: Option<&Path>) -> Option<PathBuf> {
    let names = client_names(client, cfg!(windows));
    let mut dirs = path
        .map(env::split_paths)
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if let Some(home) = home {
        dirs.extend([
            home.join(".local/bin"),
            home.join(".claude/local"),
            home.join(".cargo/bin"),
        ]);
    }
    dirs.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]);
    dirs.into_iter().find_map(|dir| {
        names.iter().find_map(|name| {
            let candidate = dir.join(name);
            is_runnable(&candidate)
                .then(|| candidate.canonicalize().ok())
                .flatten()
        })
    })
}

fn client_names(client: McpClient, windows: bool) -> Vec<String> {
    if windows {
        vec![format!("{}.exe", client.executable())]
    } else {
        vec![client.executable().to_string()]
    }
}

fn is_runnable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn lock_path(client: McpClient) -> Result<PathBuf, AppError> {
    let root = dirs::data_local_dir()
        .ok_or_else(|| invalid("local application data directory is unavailable"))?
        .join("agency-agents-app")
        .join("state");
    std::fs::create_dir_all(&root).map_err(AppError::from)?;
    Ok(root.join(format!("mcp-client-{}.lock", client.executable())))
}

async fn acquire_client_lock(client: McpClient) -> Result<File, AppError> {
    let path = lock_path(client)?;
    tokio::task::spawn_blocking(move || {
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&path)?;
        let deadline = std::time::Instant::now() + LOCK_TIMEOUT;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(25));
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    return Err(invalid(format!(
                        "{} MCP configuration is busy",
                        client.label()
                    )))
                }
                Err(std::fs::TryLockError::Error(error)) => return Err(AppError::from(error)),
            }
        }
    })
    .await
    .map_err(|error| invalid(format!("MCP client lock task failed: {error}")))?
}

async fn run(executable: &Path, args: &[OsString]) -> Result<ProcessOutput, AppError> {
    let mut child = Command::new(executable)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(AppError::from)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| invalid("missing stdout pipe"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| invalid("missing stderr pipe"))?;
    let stdout_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stdout
            .take((OUTPUT_LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .await
            .map(|_| bytes)
    });
    let stderr_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stderr
            .take((OUTPUT_LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .await
            .map(|_| bytes)
    });
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    let status = match timeout(COMMAND_TIMEOUT, child.wait()).await {
        Ok(result) => result.map_err(AppError::from)?,
        Err(_) => {
            let _ = child.kill().await;
            return Err(invalid("MCP client command timed out after 10 seconds"));
        }
    };
    let remaining = deadline.saturating_duration_since(Instant::now());
    let (stdout, stderr) = timeout(remaining.max(Duration::from_millis(100)), async {
        tokio::try_join!(
            async { stdout_task.await.map_err(std::io::Error::other)? },
            async { stderr_task.await.map_err(std::io::Error::other)? }
        )
    })
    .await
    .map_err(|_| invalid("MCP client output timed out"))?
    .map_err(AppError::from)?;
    if stdout.len() > OUTPUT_LIMIT || stderr.len() > OUTPUT_LIMIT {
        return Err(invalid("MCP client output exceeded 64 KiB"));
    }
    Ok(ProcessOutput {
        success: status.success(),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

fn get_args(client: McpClient) -> Vec<OsString> {
    let values: &[&str] = match client {
        McpClient::Claude => &["mcp", "get", SERVER_NAME],
        McpClient::Codex => &["mcp", "get", SERVER_NAME, "--json"],
    };
    values.iter().map(OsString::from).collect()
}

fn add_args(client: McpClient, command: &str, command_args: &[String]) -> Vec<OsString> {
    let mut args = match client {
        McpClient::Claude => vec!["mcp", "add", "--scope", "user", SERVER_NAME, "--"],
        McpClient::Codex => vec!["mcp", "add", SERVER_NAME, "--"],
    }
    .into_iter()
    .map(OsString::from)
    .collect::<Vec<_>>();
    args.push(command.into());
    args.extend(command_args.iter().map(OsString::from));
    args
}

fn remove_args(client: McpClient) -> Vec<OsString> {
    let values: &[&str] = match client {
        McpClient::Claude => &["mcp", "remove", "--scope", "user", SERVER_NAME],
        McpClient::Codex => &["mcp", "remove", SERVER_NAME],
    };
    values.iter().map(OsString::from).collect()
}

fn is_missing_registration(client: McpClient, result: &ProcessOutput) -> bool {
    if result.success {
        return false;
    }
    let message = format!("{}\n{}", result.stdout, result.stderr).to_ascii_lowercase();
    match client {
        McpClient::Claude => {
            message.contains("no mcp server named") || message.contains("server not found")
        }
        McpClient::Codex => message.contains("no mcp server named"),
    }
}

fn parsed_registration(client: McpClient, result: &ProcessOutput) -> Option<Registration> {
    // Codex's --json contract is stdout-only. Never let warnings/errors on
    // stderr turn into configuration.
    match client {
        McpClient::Claude | McpClient::Codex => registration(&result.stdout),
    }
}

fn classify(
    client: McpClient,
    client_path: Option<&Path>,
    app_command: &str,
    app_args: &[String],
    result: Option<&ProcessOutput>,
) -> McpClientStatus {
    let expected = manual_connect_command(client, app_command, app_args);
    let Some(client_path) = client_path else {
        return McpClientStatus {
            client,
            installed: false,
            state: McpClientState::Unavailable,
            command: expected,
            detail: format!("{} CLI was not found", client.label()),
        };
    };
    let Some(result) = result else {
        return McpClientStatus {
            client,
            installed: false,
            state: McpClientState::Unavailable,
            command: expected,
            detail: "Unable to inspect the client registration".into(),
        };
    };
    if is_missing_registration(client, result) {
        return McpClientStatus {
            client,
            installed: false,
            state: McpClientState::Missing,
            command: expected,
            detail: format!("Not connected ({})", client_path.display()),
        };
    }
    let exact = parsed_registration(client, result).is_some_and(|registration| {
        registration.command == app_command
            && registration.args == app_args
            && (!matches!(client, McpClient::Claude)
                || registration
                    .scope
                    .is_some_and(|scope| is_user_scope(&scope)))
    });
    McpClientStatus {
        client,
        installed: true,
        state: if exact {
            McpClientState::Exact
        } else {
            McpClientState::Conflict
        },
        command: expected,
        detail: if exact {
            "Connected to this Agency Agents app".into()
        } else {
            "The existing agency-agents registration points elsewhere".into()
        },
    }
}

async fn status_for(client: McpClient, app_exe: &Path) -> Result<McpClientStatus, AppError> {
    let (command, args) = app_command(app_exe)?;
    let client_path = find_client(client);
    match client_path {
        Some(path) => inspect_at(client, &path, &command, &args)
            .await
            .map(|(status, _)| status),
        None => Ok(classify(client, None, &command, &args, None)),
    }
}

async fn inspect_at(
    client: McpClient,
    client_path: &Path,
    command: &str,
    args: &[String],
) -> Result<(McpClientStatus, Option<Registration>), AppError> {
    let result = run(client_path, &get_args(client)).await?;
    if !result.success && !is_missing_registration(client, &result) {
        return Err(invalid(format!(
            "{} failed to inspect its MCP registration",
            client.label()
        )));
    }
    let parsed = result
        .success
        .then(|| parsed_registration(client, &result))
        .flatten();
    Ok((
        classify(client, Some(client_path), command, args, Some(&result)),
        parsed,
    ))
}

async fn restore_registration(
    client: McpClient,
    client_path: &Path,
    expected_command: &str,
    expected_args: &[String],
    previous: Option<&Registration>,
) -> bool {
    let Ok((current_status, current)) =
        inspect_at(client, client_path, expected_command, expected_args).await
    else {
        return false;
    };
    match previous {
        Some(previous) if current.as_ref() == Some(previous) => return true,
        None if current_status.state == McpClientState::Missing => return true,
        _ => {}
    }
    // Ownership boundary: another process may have changed the registration
    // after our command returned. Only undo our exact desired value or fill a
    // still-missing slot; never remove an unrelated current value.
    let current_is_desired = current.as_ref().is_some_and(|registration| {
        registration.command == expected_command
            && registration.args == expected_args
            && (!matches!(client, McpClient::Claude)
                || registration.scope.as_deref().is_some_and(is_user_scope))
    });
    if current.is_some() && !current_is_desired {
        return false;
    }
    if current_is_desired
        && !run(client_path, &remove_args(client))
            .await
            .is_ok_and(|result| result.success)
    {
        return false;
    }
    if let Some(previous) = previous {
        if !run(
            client_path,
            &add_args(client, &previous.command, &previous.args),
        )
        .await
        .is_ok_and(|result| result.success)
        {
            return false;
        }
    } else if current_status.state != McpClientState::Missing && current.is_none() {
        return false;
    }
    match inspect_at(client, client_path, expected_command, expected_args).await {
        Ok((status, restored)) => match previous {
            Some(previous) => restored.as_ref() == Some(previous),
            None => status.state == McpClientState::Missing,
        },
        Err(_) => false,
    }
}

fn mutation_failed(client: McpClient, action: &str, restored: bool) -> AppError {
    invalid(if restored {
        format!(
            "{} {action} failed; the previous registration was restored",
            client.label()
        )
    } else {
        format!(
            "{} {action} failed and automatic restoration failed",
            client.label()
        )
    })
}

#[tauri::command]
pub async fn mcp_clients_status() -> Result<Vec<McpClientStatus>, AppError> {
    let app_exe = env::current_exe().map_err(AppError::from)?;
    statuses_for_app(&app_exe).await
}

async fn statuses_for_app(app_exe: &Path) -> Result<Vec<McpClientStatus>, AppError> {
    let (command, args) = match app_command(app_exe) {
        Ok(command) => command,
        Err(error) => {
            let detail = error.to_string();
            return Ok([McpClient::Claude, McpClient::Codex]
                .into_iter()
                .map(|client| McpClientStatus {
                    client,
                    installed: false,
                    state: McpClientState::Unavailable,
                    command: String::new(),
                    detail: detail.clone(),
                })
                .collect());
        }
    };
    async fn isolated(
        client: McpClient,
        app_exe: &Path,
        command: &str,
        args: &[String],
    ) -> McpClientStatus {
        match status_for(client, app_exe).await {
            Ok(status) => status,
            Err(error) => McpClientStatus {
                client,
                installed: false,
                state: McpClientState::Unavailable,
                command: manual_connect_command(client, command, args),
                detail: error.to_string(),
            },
        }
    }
    Ok(vec![
        isolated(McpClient::Claude, app_exe, &command, &args).await,
        isolated(McpClient::Codex, app_exe, &command, &args).await,
    ])
}

#[tauri::command]
pub async fn mcp_client_connect(client: McpClient) -> Result<McpClientStatus, AppError> {
    let app_exe = env::current_exe().map_err(AppError::from)?;
    let client_path = find_client(client)
        .ok_or_else(|| invalid(format!("{} CLI was not found", client.label())))?;
    let (command, args) = app_command(&app_exe)?;
    connect_transaction_at(client, &client_path, &command, &args).await
}

async fn connect_transaction_at(
    client: McpClient,
    client_path: &Path,
    command: &str,
    args: &[String],
) -> Result<McpClientStatus, AppError> {
    let _lock = acquire_client_lock(client).await?;
    connect_at(client, client_path, command, args).await
}

async fn connect_at(
    client: McpClient,
    client_path: &Path,
    command: &str,
    args: &[String],
) -> Result<McpClientStatus, AppError> {
    let (before, _) = inspect_at(client, &client_path, &command, &args).await?;
    match before.state {
        McpClientState::Exact => return Ok(before),
        McpClientState::Conflict => {
            return Err(invalid(
                "an agency-agents registration already exists; use Repair to replace it",
            ))
        }
        McpClientState::Unavailable => return Err(invalid(before.detail)),
        McpClientState::Missing => {}
    }
    let added = run(&client_path, &add_args(client, &command, &args))
        .await
        .is_ok_and(|result| result.success);
    let verified = added
        && inspect_at(client, &client_path, &command, &args)
            .await
            .is_ok_and(|(status, _)| status.state == McpClientState::Exact);
    if !verified {
        let restored = restore_registration(client, &client_path, &command, &args, None).await;
        return Err(mutation_failed(client, "connection", restored));
    }
    inspect_at(client, &client_path, &command, &args)
        .await
        .map(|(status, _)| status)
}

#[tauri::command]
pub async fn mcp_client_disconnect(client: McpClient) -> Result<McpClientStatus, AppError> {
    let _lock = acquire_client_lock(client).await?;
    let app_exe = env::current_exe().map_err(AppError::from)?;
    let client_path = find_client(client)
        .ok_or_else(|| invalid(format!("{} CLI was not found", client.label())))?;
    let (command, args) = app_command(&app_exe)?;
    disconnect_at(client, &client_path, &command, &args).await
}

async fn disconnect_at(
    client: McpClient,
    client_path: &Path,
    command: &str,
    args: &[String],
) -> Result<McpClientStatus, AppError> {
    let (before, previous) = inspect_at(client, &client_path, &command, &args).await?;
    match before.state {
        McpClientState::Missing => return Ok(before),
        McpClientState::Unavailable => return Err(invalid(before.detail)),
        McpClientState::Conflict => {
            return Err(invalid(
                "refusing to remove a registration that points to another command",
            ))
        }
        McpClientState::Exact => {}
    }
    let removed = run(&client_path, &remove_args(client))
        .await
        .is_ok_and(|result| result.success);
    let verified = removed
        && inspect_at(client, &client_path, &command, &args)
            .await
            .is_ok_and(|(status, _)| status.state == McpClientState::Missing);
    if !verified {
        let restored =
            restore_registration(client, &client_path, &command, &args, previous.as_ref()).await;
        return Err(mutation_failed(client, "disconnection", restored));
    }
    inspect_at(client, &client_path, &command, &args)
        .await
        .map(|(status, _)| status)
}

#[tauri::command]
pub async fn mcp_client_repair(client: McpClient) -> Result<McpClientStatus, AppError> {
    let _lock = acquire_client_lock(client).await?;
    let app_exe = env::current_exe().map_err(AppError::from)?;
    let client_path = find_client(client)
        .ok_or_else(|| invalid(format!("{} CLI was not found", client.label())))?;
    let (command, args) = app_command(&app_exe)?;
    repair_at(client, &client_path, &command, &args).await
}

async fn repair_at(
    client: McpClient,
    client_path: &Path,
    command: &str,
    args: &[String],
) -> Result<McpClientStatus, AppError> {
    let (before, previous) = inspect_at(client, &client_path, &command, &args).await?;
    match before.state {
        McpClientState::Exact => return Ok(before),
        McpClientState::Missing => {
            let added = run(&client_path, &add_args(client, &command, &args))
                .await
                .is_ok_and(|result| result.success);
            let verified = added
                && inspect_at(client, &client_path, &command, &args)
                    .await
                    .is_ok_and(|(status, _)| status.state == McpClientState::Exact);
            if !verified {
                let restored =
                    restore_registration(client, &client_path, &command, &args, None).await;
                return Err(mutation_failed(client, "repair", restored));
            }
            return inspect_at(client, &client_path, &command, &args)
                .await
                .map(|(status, _)| status);
        }
        McpClientState::Unavailable => return Err(invalid(before.detail)),
        McpClientState::Conflict => {}
    }
    let previous = previous.ok_or_else(|| {
        invalid("the existing registration cannot be read safely; remove it in the client")
    })?;
    if matches!(client, McpClient::Claude) && !previous.scope.as_deref().is_some_and(is_user_scope)
    {
        return Err(invalid(
            "only user-scoped Claude registrations can be repaired safely",
        ));
    }
    let removed = run(&client_path, &remove_args(client))
        .await
        .is_ok_and(|result| result.success);
    let added = removed
        && run(&client_path, &add_args(client, &command, &args))
            .await
            .is_ok_and(|output| output.success);
    let verified = added
        && inspect_at(client, &client_path, &command, &args)
            .await
            .is_ok_and(|(status, _)| status.state == McpClientState::Exact);
    if !verified {
        let restored =
            restore_registration(client, &client_path, &command, &args, Some(&previous)).await;
        return Err(mutation_failed(client, "repair", restored));
    }
    inspect_at(client, &client_path, &command, &args)
        .await
        .map(|(status, _)| status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(path, permissions).unwrap();
    }

    #[cfg(not(unix))]
    fn executable(_path: &Path) {}

    #[test]
    fn argv_preserves_app_paths_with_spaces() {
        let args = add_args(
            McpClient::Claude,
            "/Applications/Agency Agents.app/Contents/MacOS/agency-agents-app",
            &["--mcp".into()],
        );
        assert_eq!(
            args,
            vec![
                "mcp",
                "add",
                "--scope",
                "user",
                "agency-agents",
                "--",
                "/Applications/Agency Agents.app/Contents/MacOS/agency-agents-app",
                "--mcp",
            ]
            .into_iter()
            .map(OsString::from)
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn windows_resolution_never_executes_shell_shims() {
        assert_eq!(
            client_names(McpClient::Codex, true),
            vec!["codex.exe".to_string()]
        );
        assert!(!client_names(McpClient::Claude, true)
            .iter()
            .any(|name| name.ends_with(".cmd") || name.ends_with(".bat")));
    }

    #[test]
    fn detection_uses_path_then_known_locations() {
        let temp = std::env::temp_dir().join(format!("mcp-client-{}", uuid::Uuid::new_v4()));
        let path_bin = temp.join("path");
        let home = temp.join("home");
        std::fs::create_dir_all(&path_bin).unwrap();
        std::fs::create_dir_all(home.join(".local/bin")).unwrap();
        let name = if cfg!(windows) { "codex.exe" } else { "codex" };
        std::fs::write(home.join(".local/bin").join(name), b"").unwrap();
        executable(&home.join(".local/bin").join(name));
        assert_eq!(
            find_client_in(McpClient::Codex, Some(path_bin.as_os_str()), Some(&home)),
            Some(home.join(".local/bin").join(name).canonicalize().unwrap())
        );
        std::fs::write(path_bin.join(name), b"").unwrap();
        executable(&path_bin.join(name));
        assert_eq!(
            find_client_in(McpClient::Codex, Some(path_bin.as_os_str()), Some(&home)),
            Some(path_bin.join(name).canonicalize().unwrap())
        );
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn detection_skips_non_executable_path_poisoning() {
        let temp = std::env::temp_dir().join(format!("mcp-path-{}", uuid::Uuid::new_v4()));
        let poisoned = temp.join("poisoned");
        let home = temp.join("home");
        std::fs::create_dir_all(&poisoned).unwrap();
        std::fs::create_dir_all(home.join(".local/bin")).unwrap();
        std::fs::write(poisoned.join("claude"), b"not executable").unwrap();
        std::fs::write(home.join(".local/bin/claude"), b"real").unwrap();
        executable(&home.join(".local/bin/claude"));
        assert_eq!(
            find_client_in(McpClient::Claude, Some(poisoned.as_os_str()), Some(&home)),
            Some(home.join(".local/bin/claude").canonicalize().unwrap())
        );
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn classifies_missing_exact_and_conflict() {
        let path = Path::new("/usr/bin/codex");
        let missing = ProcessOutput {
            success: false,
            stdout: String::new(),
            stderr: "No MCP server named 'agency-agents' found.".into(),
        };
        assert_eq!(
            classify(
                McpClient::Codex,
                Some(path),
                "/Applications/Agency Agents.app/app",
                &["--mcp".into()],
                Some(&missing)
            )
            .state,
            McpClientState::Missing
        );
        let exact = ProcessOutput {
            success: true,
            stdout: "command: /Applications/Agency Agents.app/app\nargs: --mcp".into(),
            stderr: String::new(),
        };
        assert_eq!(
            classify(
                McpClient::Codex,
                Some(path),
                "/Applications/Agency Agents.app/app",
                &["--mcp".into()],
                Some(&exact)
            )
            .state,
            McpClientState::Exact
        );
        let conflict = ProcessOutput {
            success: true,
            stdout: "command: /tmp/other --mcp".into(),
            stderr: String::new(),
        };
        assert_eq!(
            classify(
                McpClient::Codex,
                Some(path),
                "/Applications/Agency Agents.app/app",
                &["--mcp".into()],
                Some(&conflict)
            )
            .state,
            McpClientState::Conflict
        );
    }

    #[test]
    fn parses_safe_text_and_json_registrations() {
        assert_eq!(
            registration("Command: /Applications/Agency Agents.app/app\nArgs: [\"--mcp\"]"),
            Some(Registration {
                command: "/Applications/Agency Agents.app/app".into(),
                args: vec!["--mcp".into()],
                scope: None,
            })
        );
        assert_eq!(
            registration(r#"{"command":"/tmp/app","args":["--mcp"]}"#),
            Some(Registration {
                command: "/tmp/app".into(),
                args: vec!["--mcp".into()],
                scope: None,
            })
        );
        assert_eq!(
            registration(
                r#"{"transport":{"type":"stdio","command":"/tmp/app","args":["--mcp"],"env":null}}"#
            ),
            Some(Registration {
                command: "/tmp/app".into(),
                args: vec!["--mcp".into()],
                scope: None,
            })
        );
        assert_eq!(
            registration("Command: /tmp/app\nArgs: \"unsafe arg\""),
            None
        );
        assert_eq!(
            registration(
                r#"{"enabled":false,"transport":{"type":"stdio","command":"/tmp/app","args":[]}}"#
            ),
            None
        );
        assert_eq!(
            registration(
                r#"{"enabled":true,"enabled_tools":["one"],"transport":{"type":"stdio","command":"/tmp/app","args":[]}}"#
            ),
            None
        );
    }

    #[test]
    fn rejects_ephemeral_app_executable() {
        let temp = std::env::temp_dir().join(format!("ephemeral-app-{}", uuid::Uuid::new_v4()));
        std::fs::write(&temp, b"app").unwrap();
        let error = app_command(&temp).unwrap_err();
        assert!(error.to_string().contains("Applications"));
        std::fs::remove_file(temp).unwrap();
    }

    #[test]
    fn rejects_development_build_executable() {
        let current = std::env::current_exe().unwrap();
        assert!(current.to_string_lossy().contains("/target/debug/"));
        assert!(app_command(&current).is_err());
        assert!(is_ephemeral_executable(Path::new(
            "/work/project/target/debug/app"
        )));
        assert!(is_ephemeral_executable(Path::new("/work/build/app")));
        assert!(!is_ephemeral_executable(Path::new(
            "/Applications/Agency Agents.app/Contents/MacOS/app"
        )));
    }

    #[tokio::test]
    async fn development_status_returns_unavailable_cards_instead_of_a_global_error() {
        let current = std::env::current_exe().unwrap();
        assert!(is_ephemeral_executable(&current));

        let statuses = statuses_for_app(&current).await.unwrap();

        assert_eq!(statuses.len(), 2);
        assert!(statuses
            .iter()
            .all(|status| status.state == McpClientState::Unavailable));
        assert!(statuses
            .iter()
            .all(|status| status.detail.contains("Applications")));
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn runner_caps_time_and_output() {
        let timeout_err = run(Path::new("/bin/sh"), &["-c".into(), "sleep 11".into()])
            .await
            .unwrap_err();
        assert!(timeout_err.to_string().contains("timed out"));

        let output_err = run(
            Path::new("/bin/sh"),
            &["-c".into(), "yes x | head -c 70000".into()],
        )
        .await
        .unwrap_err();
        assert!(output_err.to_string().contains("64 KiB"));
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn fake_client_receives_literal_argv() {
        use std::os::unix::fs::PermissionsExt;

        let temp = std::env::temp_dir().join(format!("fake-mcp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).unwrap();
        let script = temp.join("fake client");
        let log = temp.join("argv");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nprintf 'Command: /Applications/Agency Agents.app/app\\nArgs: [\"--mcp\"]\\n'\n",
                log.display()
            ),
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&script, permissions).unwrap();
        let result = run(
            &script,
            &[
                "mcp".into(),
                "add".into(),
                "agency-agents".into(),
                "--".into(),
                "/Applications/Agency Agents.app/app".into(),
                "--mcp".into(),
            ],
        )
        .await
        .unwrap();
        assert!(result.success);
        assert_eq!(
            std::fs::read_to_string(log).unwrap(),
            "mcp\nadd\nagency-agents\n--\n/Applications/Agency Agents.app/app\n--mcp\n"
        );
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn fake_client_lifecycle_is_idempotent_and_rolls_back() {
        let temp = std::env::temp_dir().join(format!("fake-life-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).unwrap();
        let script = temp.join("codex");
        let state = temp.join("state.json");
        let mode = temp.join("mode");
        let log = temp.join("log");
        std::fs::write(
            &script,
            format!(
                r#"#!/bin/sh
printf '%s\n' "$*" >> '{log}'
case "$2" in
  get)
    if [ -f '{state}' ]; then cat '{state}'; else echo "No MCP server named 'agency-agents' found." >&2; exit 1; fi ;;
  add)
    if [ -f '{mode}' ] && [ "$(cat '{mode}')" = "external-on-fail-new" ] && [ "$5" = "/new/app" ]; then
      printf '{{"transport":{{"type":"stdio","command":"/external/app","args":["--external"],"env":null,"env_vars":[],"cwd":null}}}}\n' > '{state}'
      exit 8
    fi
    if [ -f '{mode}' ] && [ "$(cat '{mode}')" = "fail-new" ] && [ "$5" = "/new/app" ]; then exit 7; fi
    printf '{{"transport":{{"type":"stdio","command":"%s","args":["%s"],"env":null,"env_vars":[],"cwd":null}}}}\n' "$5" "$6" > '{state}' ;;
  remove) rm -f '{state}' ;;
esac
"#,
                log = log.display(),
                state = state.display(),
                mode = mode.display(),
            ),
        )
        .unwrap();
        executable(&script);
        let args = vec!["--mcp".to_string()];

        let connected = connect_at(McpClient::Codex, &script, "/new/app", &args)
            .await
            .unwrap();
        assert_eq!(connected.state, McpClientState::Exact);
        let before = std::fs::read_to_string(&log).unwrap();
        let idempotent = connect_at(McpClient::Codex, &script, "/new/app", &args)
            .await
            .unwrap();
        assert_eq!(idempotent.state, McpClientState::Exact);
        assert!(!std::fs::read_to_string(&log).unwrap()[before.len()..].contains(" add "));

        let disconnected = disconnect_at(McpClient::Codex, &script, "/new/app", &args)
            .await
            .unwrap();
        assert_eq!(disconnected.state, McpClientState::Missing);

        std::fs::write(
            &state,
            r#"{"transport":{"type":"stdio","command":"/old/app","args":["--old"],"env":null,"env_vars":[],"cwd":null}}"#,
        )
        .unwrap();
        assert!(connect_at(McpClient::Codex, &script, "/new/app", &args)
            .await
            .unwrap_err()
            .to_string()
            .contains("Repair"));

        std::fs::write(&mode, "fail-new").unwrap();
        let repair_error = repair_at(McpClient::Codex, &script, "/new/app", &args)
            .await
            .unwrap_err();
        assert!(repair_error.to_string().contains("restored"));
        let restored = registration(&std::fs::read_to_string(&state).unwrap()).unwrap();
        assert_eq!(restored.command, "/old/app");

        std::fs::write(&mode, "external-on-fail-new").unwrap();
        let ownership_error = repair_at(McpClient::Codex, &script, "/new/app", &args)
            .await
            .unwrap_err();
        assert!(ownership_error.to_string().contains("restoration failed"));
        let external = registration(&std::fs::read_to_string(&state).unwrap()).unwrap();
        assert_eq!(external.command, "/external/app");

        std::fs::remove_file(&state).unwrap();
        std::fs::write(&mode, "fail-new").unwrap();
        let connect_error = connect_at(McpClient::Codex, &script, "/new/app", &args)
            .await
            .unwrap_err();
        assert!(connect_error.to_string().contains("restored"));
        assert!(!state.exists());

        std::fs::remove_file(&mode).unwrap();
        std::fs::write(&log, b"").unwrap();
        let (first, second) = tokio::join!(
            connect_transaction_at(McpClient::Codex, &script, "/new/app", &args),
            connect_transaction_at(McpClient::Codex, &script, "/new/app", &args)
        );
        assert_eq!(first.unwrap().state, McpClientState::Exact);
        assert_eq!(second.unwrap().state, McpClientState::Exact);
        assert_eq!(
            std::fs::read_to_string(&log)
                .unwrap()
                .lines()
                .filter(|line| line.contains("mcp add"))
                .count(),
            1
        );
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[tokio::test]
    async fn per_client_lock_serializes_mutations() {
        let first = acquire_client_lock(McpClient::Codex).await.unwrap();
        let waiter = tokio::spawn(acquire_client_lock(McpClient::Codex));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!waiter.is_finished());
        drop(first);
        assert!(waiter.await.unwrap().is_ok());
    }
}
