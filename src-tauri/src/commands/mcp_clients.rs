use std::{
    env,
    ffi::{OsStr, OsString},
    fs::{File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
    thread,
    time::Duration,
};

use tauri::State;
use tokio::{
    io::AsyncReadExt,
    process::Command,
    time::{timeout, Instant},
};

use crate::{
    error::AppError,
    state::AppState,
    types::{
        McpClient, McpClientState, McpClientStatus, McpInventoryReport, McpInventoryServer,
        McpInventoryValidation, McpServerScope, McpToolDiscovery, McpTrustedTemplate,
    },
};

const SERVER_NAME: &str = "agency-agents";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const OUTPUT_LIMIT: usize = 64 * 1024;
const LOCK_TIMEOUT: Duration = Duration::from_secs(2);
const INVENTORY_CONFIG_LIMIT: u64 = 1024 * 1024;
const INVENTORY_SERVER_LIMIT: usize = 256;
const INVENTORY_FIELD_LIMIT: usize = 256;
const INVENTORY_NAME_LIMIT: usize = 128;
const INVENTORY_LIST_LIMIT: usize = 256;
const INVENTORY_ISSUE_LIMIT: usize = 64;

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

fn bounded_inventory_name(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= INVENTORY_NAME_LIMIT
        && !value.chars().any(char::is_control))
    .then(|| value.to_owned())
}

fn bounded_inventory_list<'a>(values: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut values = values
        .into_iter()
        .filter_map(|value| {
            let value = value.trim();
            (!value.is_empty()
                && value.len() <= INVENTORY_FIELD_LIMIT
                && !value.chars().any(char::is_control))
            .then(|| value.to_owned())
        })
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values.truncate(INVENTORY_LIST_LIMIT);
    values
}

fn inventory_transport(value: &serde_json::Value) -> &serde_json::Value {
    value.get("transport").unwrap_or(value)
}

fn inventory_registration(value: &serde_json::Value) -> Option<Registration> {
    let config = inventory_transport(value);
    let command = config.get("command")?.as_str()?.to_owned();
    let args = config
        .get("args")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| item.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
        })
        .unwrap_or_else(|| Some(Vec::new()))?;
    Some(Registration {
        command,
        args,
        scope: None,
    })
}

fn environment_keys(config: &serde_json::Value) -> Vec<String> {
    let mut keys = Vec::new();
    for key in ["env", "environment"] {
        if let Some(values) = config.get(key).and_then(serde_json::Value::as_object) {
            keys.extend(values.keys().map(String::as_str));
        }
    }
    if let Some(values) = config.get("env_vars").and_then(serde_json::Value::as_array) {
        keys.extend(values.iter().filter_map(serde_json::Value::as_str));
    }
    let mut normalized = bounded_inventory_list(keys);
    for key in ["headers", "http_headers", "env_http_headers"] {
        if let Some(values) = config.get(key).and_then(serde_json::Value::as_object) {
            normalized.extend(
                bounded_inventory_list(values.keys().map(String::as_str))
                    .into_iter()
                    .map(|name| format!("header:{name}")),
            );
        }
    }
    normalized.sort();
    normalized.dedup();
    normalized.truncate(INVENTORY_LIST_LIMIT);
    normalized
}

fn is_environment_reference(value: &str) -> bool {
    let value = value.trim();
    (value.starts_with("${") && value.ends_with('}'))
        || (!value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'))
}

fn has_inline_inventory_credentials(config: &serde_json::Value) -> bool {
    for key in ["env", "environment", "headers", "http_headers"] {
        if config
            .get(key)
            .and_then(serde_json::Value::as_object)
            .is_some_and(|values| {
                values.values().any(|value| {
                    value
                        .as_str()
                        .is_some_and(|value| !value.is_empty() && !is_environment_reference(value))
                })
            })
        {
            return true;
        }
    }
    config
        .get("args")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|args| {
            args.iter()
                .filter_map(serde_json::Value::as_str)
                .any(|value| {
                    let lower = value.to_ascii_lowercase();
                    [
                        "token=",
                        "api_key=",
                        "api-key=",
                        "apikey=",
                        "authorization:",
                        "password=",
                        "sk-",
                        "ghp_",
                        "github_pat_",
                    ]
                    .iter()
                    .any(|marker| lower.contains(marker))
                })
        })
}

fn command_endpoint(command: &str) -> Option<String> {
    let command = command.trim();
    if command.is_empty()
        || command.len() > INVENTORY_FIELD_LIMIT
        || command.chars().any(char::is_control)
    {
        return None;
    }
    Path::new(command)
        .file_name()
        .and_then(OsStr::to_str)
        .and_then(bounded_inventory_name)
}

fn safe_remote_endpoint(raw: &str, blockers: &mut Vec<String>) -> Option<String> {
    let parsed = match url::Url::parse(raw) {
        Ok(parsed) => parsed,
        Err(_) => {
            blockers.push("Remote MCP URL is invalid".into());
            return None;
        }
    };
    let Some(host) = parsed.host_str() else {
        blockers.push("Remote MCP URL has no host".into());
        return None;
    };
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        blockers.push("Remote MCP URL contains inline credential or private parameters".into());
    }
    let loopback = matches!(host, "localhost" | "127.0.0.1" | "::1");
    if parsed.scheme() != "https" && !(parsed.scheme() == "http" && loopback) {
        blockers.push("Remote MCP URL must use HTTPS or loopback HTTP".into());
    }
    let host_summary = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    let port = parsed
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    let endpoint = format!("{}://{host_summary}{port}", parsed.scheme());
    if endpoint.len() > INVENTORY_FIELD_LIMIT {
        blockers.push("Remote MCP endpoint exceeds its limit".into());
        return None;
    }
    Some(endpoint)
}

fn declared_tool_names(value: &serde_json::Value) -> Vec<String> {
    let values = ["enabled_tools", "disabled_tools"]
        .into_iter()
        .filter_map(|key| value.get(key).and_then(serde_json::Value::as_array))
        .flatten()
        .filter_map(serde_json::Value::as_str);
    bounded_inventory_list(values)
}

fn normalize_inventory_server(
    client: McpClient,
    name: &str,
    scope: McpServerScope,
    project_path: Option<String>,
    value: &serde_json::Value,
    expected: Option<&Registration>,
    trusted_tools: &[String],
) -> McpInventoryServer {
    let mut warnings = Vec::new();
    let mut blockers = Vec::new();
    let name = bounded_inventory_name(name).unwrap_or_else(|| {
        blockers.push("MCP server name is invalid or exceeds its limit".into());
        "invalid-server".into()
    });
    let enabled = value
        .get("enabled")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    if !enabled {
        warnings.push("MCP server is disabled".into());
    }
    let config = inventory_transport(value);
    let raw_transport = config
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| {
            if config.get("url").is_some() {
                "http"
            } else {
                "stdio"
            }
        });
    let transport = match raw_transport {
        "stdio" => "stdio",
        "http" | "streamable_http" => "http",
        "sse" => "sse",
        _ => {
            blockers.push("MCP transport is unsupported".into());
            "unsupported"
        }
    }
    .to_owned();
    let endpoint = if transport == "stdio" {
        let command = config
            .get("command")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let endpoint = command_endpoint(command).unwrap_or_else(|| {
            blockers.push("Stdio MCP command is missing or invalid".into());
            "unknown-command".into()
        });
        if matches!(
            endpoint.to_ascii_lowercase().as_str(),
            "npx" | "npx.cmd" | "uvx" | "docker" | "podman" | "bunx"
        ) {
            warnings.push("Configured runtime launcher may fetch or start external code".into());
        }
        endpoint
    } else if matches!(transport.as_str(), "http" | "sse") {
        safe_remote_endpoint(
            config
                .get("url")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default(),
            &mut blockers,
        )
        .unwrap_or_else(|| "invalid-url".into())
    } else {
        "unsupported".into()
    };
    if has_inline_inventory_credentials(config) {
        warnings.push("Configuration contains inline credential material".into());
    }
    let exact_trusted = name == SERVER_NAME
        && scope == McpServerScope::User
        && expected.is_some_and(|expected| {
            inventory_registration(value).is_some_and(|registration| {
                registration.command == expected.command && registration.args == expected.args
            })
        });
    let declared_tools = declared_tool_names(value);
    let (tool_names, tool_discovery) = if exact_trusted {
        (trusted_tools.to_vec(), McpToolDiscovery::Known)
    } else if declared_tools.is_empty() {
        (Vec::new(), McpToolDiscovery::Unavailable)
    } else {
        (declared_tools, McpToolDiscovery::Declared)
    };
    warnings.sort();
    warnings.dedup();
    blockers.sort();
    blockers.dedup();
    let validation = if !blockers.is_empty() {
        McpInventoryValidation::Blocked
    } else if !warnings.is_empty() {
        McpInventoryValidation::Warning
    } else {
        McpInventoryValidation::Valid
    };
    McpInventoryServer {
        client,
        name,
        scope,
        project_path,
        transport,
        endpoint,
        enabled,
        environment_keys: environment_keys(config),
        tool_names,
        tool_discovery,
        validation,
        warnings,
        blockers,
        trusted_template: exact_trusted,
    }
}

fn parse_server_map(
    client: McpClient,
    servers: &serde_json::Map<String, serde_json::Value>,
    scope: McpServerScope,
    project_path: Option<String>,
    expected: Option<&Registration>,
    trusted_tools: &[String],
    output: &mut Vec<McpInventoryServer>,
) {
    let mut entries = servers.iter().collect::<Vec<_>>();
    entries.sort_by_key(|(name, _)| *name);
    for (name, value) in entries.into_iter().take(INVENTORY_SERVER_LIMIT) {
        output.push(normalize_inventory_server(
            client,
            name,
            scope,
            project_path.clone(),
            value,
            expected,
            trusted_tools,
        ));
    }
}

fn parse_codex_inventory(
    output: &str,
    expected: Option<&Registration>,
    trusted_tools: &[String],
) -> Result<Vec<McpInventoryServer>, AppError> {
    let values = serde_json::from_str::<serde_json::Value>(output)
        .map_err(|_| invalid("Codex MCP inventory JSON is invalid"))?;
    let values = values
        .as_array()
        .ok_or_else(|| invalid("Codex MCP inventory must be a JSON array"))?;
    let mut servers = values
        .iter()
        .map(|value| {
            let name = value
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            normalize_inventory_server(
                McpClient::Codex,
                name,
                McpServerScope::User,
                None,
                value,
                expected,
                trusted_tools,
            )
        })
        .collect::<Vec<_>>();
    servers.sort_by(|left, right| left.name.cmp(&right.name));
    servers.dedup_by(|left, right| left.name == right.name);
    servers.truncate(INVENTORY_SERVER_LIMIT);
    Ok(servers)
}

fn parse_claude_user_config(
    root: &serde_json::Value,
    projects: &[PathBuf],
    expected: Option<&Registration>,
    trusted_tools: &[String],
    output: &mut Vec<McpInventoryServer>,
) -> Result<(), AppError> {
    let root = root
        .as_object()
        .ok_or_else(|| invalid("Claude MCP user config must be a JSON object"))?;
    let configured_projects = root.get("projects").and_then(serde_json::Value::as_object);
    let mut projects = projects.to_vec();
    projects.sort();
    for project in projects {
        let identity = project.to_string_lossy().into_owned();
        let servers = configured_projects
            .and_then(|configured| configured.get(&identity))
            .and_then(|value| value.get("mcpServers"))
            .and_then(serde_json::Value::as_object);
        if let Some(servers) = servers {
            parse_server_map(
                McpClient::Claude,
                servers,
                McpServerScope::Local,
                Some(identity),
                expected,
                trusted_tools,
                output,
            );
        }
    }
    if let Some(servers) = root
        .get("mcpServers")
        .and_then(serde_json::Value::as_object)
    {
        parse_server_map(
            McpClient::Claude,
            servers,
            McpServerScope::User,
            None,
            expected,
            trusted_tools,
            output,
        );
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::InvalidArgument {
        message: message.into(),
    }
}

fn app_command(executable: &Path, client: McpClient) -> Result<(String, Vec<String>), AppError> {
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
        vec![
            "--mcp".to_string(),
            "--client".to_string(),
            match client {
                McpClient::Claude => "claude",
                McpClient::Codex => "codex",
            }
            .to_string(),
        ],
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
            .truncate(false)
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
    let (command, args) = app_command(app_exe, client)?;
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
    async fn isolated(client: McpClient, app_exe: &Path) -> McpClientStatus {
        match status_for(client, app_exe).await {
            Ok(status) => status,
            Err(error) => McpClientStatus {
                client,
                installed: false,
                state: McpClientState::Unavailable,
                command: app_command(app_exe, client)
                    .map(|(command, args)| manual_connect_command(client, &command, &args))
                    .unwrap_or_default(),
                detail: error.to_string(),
            },
        }
    }
    Ok(vec![
        isolated(McpClient::Claude, app_exe).await,
        isolated(McpClient::Codex, app_exe).await,
    ])
}

fn expected_inventory_registration(app_exe: &Path, client: McpClient) -> Option<Registration> {
    app_command(app_exe, client)
        .ok()
        .map(|(command, args)| Registration {
            command,
            args,
            scope: None,
        })
}

fn push_inventory_issue(issues: &mut Vec<String>, message: impl Into<String>) {
    if issues.len() >= INVENTORY_ISSUE_LIMIT {
        return;
    }
    let message = message.into();
    let bounded = message
        .chars()
        .filter(|character| !character.is_control())
        .take(INVENTORY_FIELD_LIMIT)
        .collect::<String>();
    if !bounded.is_empty() {
        issues.push(bounded);
    }
}

async fn read_inventory_json(path: &Path) -> Result<Option<serde_json::Value>, AppError> {
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || {
        let parent = path
            .parent()
            .ok_or_else(|| invalid("MCP inventory source has no parent"))?;
        let name = path
            .file_name()
            .ok_or_else(|| invalid("MCP inventory source has no file name"))?;
        let directory =
            cap_primitives::fs::open_ambient_dir(parent, cap_primitives::ambient_authority())
                .map_err(AppError::from)?;
        let mut options = cap_primitives::fs::OpenOptions::new();
        options
            .read(true)
            ._cap_fs_ext_follow(cap_primitives::fs::FollowSymlinks::No);
        let file = match cap_primitives::fs::open(&directory, Path::new(name), &options) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(AppError::from(error)),
        };
        let metadata = file.metadata().map_err(AppError::from)?;
        if !metadata.is_file() || crate::skills::metadata_is_reparse_point(&metadata) {
            return Err(invalid("MCP inventory source must be a regular file"));
        }
        if metadata.len() > INVENTORY_CONFIG_LIMIT {
            return Err(invalid("MCP inventory source exceeds its size limit"));
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(INVENTORY_CONFIG_LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(AppError::from)?;
        if bytes.len() as u64 > INVENTORY_CONFIG_LIMIT {
            return Err(invalid("MCP inventory source exceeds its size limit"));
        }
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| invalid("MCP inventory source contains invalid JSON"))
    })
    .await
    .map_err(|error| invalid(format!("MCP inventory read task failed: {error}")))?
}

async fn collect_claude_inventory(
    home: &Path,
    projects: &[PathBuf],
    expected: Option<&Registration>,
    trusted_tools: &[String],
    servers: &mut Vec<McpInventoryServer>,
    issues: &mut Vec<String>,
) {
    match read_inventory_json(&home.join(".claude.json")).await {
        Ok(Some(root)) => {
            if parse_claude_user_config(&root, projects, expected, trusted_tools, servers).is_err()
            {
                push_inventory_issue(issues, "Claude user MCP configuration is malformed");
            }
        }
        Ok(None) => {}
        Err(_) => push_inventory_issue(issues, "Claude user MCP configuration is unavailable"),
    }

    let mut sorted_projects = projects.to_vec();
    sorted_projects.sort();
    for project in sorted_projects {
        let exact_project = std::fs::symlink_metadata(&project).is_ok_and(|metadata| {
            metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && !crate::skills::metadata_is_reparse_point(&metadata)
        }) && std::fs::canonicalize(&project)
            .is_ok_and(|canonical| canonical == project);
        if !exact_project {
            push_inventory_issue(
                issues,
                format!(
                    "Claude project MCP source is unavailable for {}",
                    project.display()
                ),
            );
            continue;
        }
        match read_inventory_json(&project.join(".mcp.json")).await {
            Ok(Some(root)) => match root
                .get("mcpServers")
                .and_then(serde_json::Value::as_object)
            {
                Some(configured) => parse_server_map(
                    McpClient::Claude,
                    configured,
                    McpServerScope::Project,
                    Some(project.to_string_lossy().into_owned()),
                    expected,
                    trusted_tools,
                    servers,
                ),
                None => push_inventory_issue(
                    issues,
                    format!(
                        "Claude project MCP configuration is malformed for {}",
                        project.display()
                    ),
                ),
            },
            Ok(None) => {}
            Err(_) => push_inventory_issue(
                issues,
                format!(
                    "Claude project MCP configuration is unavailable for {}",
                    project.display()
                ),
            ),
        }
    }
}

async fn collect_codex_inventory(
    expected: Option<&Registration>,
    trusted_tools: &[String],
    servers: &mut Vec<McpInventoryServer>,
    issues: &mut Vec<String>,
) {
    let Some(client_path) = find_client(McpClient::Codex) else {
        push_inventory_issue(issues, "Codex CLI is unavailable for MCP inventory");
        return;
    };
    collect_codex_inventory_at(&client_path, expected, trusted_tools, servers, issues).await;
}

async fn collect_codex_inventory_at(
    client_path: &Path,
    expected: Option<&Registration>,
    trusted_tools: &[String],
    servers: &mut Vec<McpInventoryServer>,
    issues: &mut Vec<String>,
) {
    let result = match run(client_path, &["mcp".into(), "list".into(), "--json".into()]).await {
        Ok(result) if result.success => result,
        _ => {
            push_inventory_issue(issues, "Codex MCP inventory command failed");
            return;
        }
    };
    match parse_codex_inventory(&result.stdout, expected, trusted_tools) {
        Ok(mut parsed) => servers.append(&mut parsed),
        Err(_) => push_inventory_issue(issues, "Codex MCP inventory output is malformed"),
    }
}

pub(crate) async fn mcp_inventory_for_state(
    state: &AppState,
) -> Result<McpInventoryReport, AppError> {
    let trusted_tools = crate::skills::mcp::agency_agents_tool_names();
    let app_exe = env::current_exe().map_err(AppError::from)?;
    let claude_expected = expected_inventory_registration(&app_exe, McpClient::Claude);
    let codex_expected = expected_inventory_registration(&app_exe, McpClient::Codex);
    let projects = crate::install::registered_projects(&state.app_data_dir).await?;
    let mut servers = Vec::new();
    let mut issues = Vec::new();
    if let Some(home) = dirs::home_dir() {
        collect_claude_inventory(
            &home,
            &projects,
            claude_expected.as_ref(),
            &trusted_tools,
            &mut servers,
            &mut issues,
        )
        .await;
    } else {
        push_inventory_issue(&mut issues, "Claude home is unavailable for MCP inventory");
    }
    collect_codex_inventory(
        codex_expected.as_ref(),
        &trusted_tools,
        &mut servers,
        &mut issues,
    )
    .await;
    servers.sort_by(|left, right| {
        (
            left.client,
            left.scope,
            left.project_path.as_deref(),
            left.name.as_str(),
        )
            .cmp(&(
                right.client,
                right.scope,
                right.project_path.as_deref(),
                right.name.as_str(),
            ))
    });
    servers.dedup_by(|left, right| {
        left.client == right.client
            && left.scope == right.scope
            && left.project_path == right.project_path
            && left.name == right.name
    });
    if servers.len() > INVENTORY_SERVER_LIMIT {
        servers.truncate(INVENTORY_SERVER_LIMIT);
        push_inventory_issue(&mut issues, "MCP inventory reached its server limit");
    }
    issues.sort();
    issues.dedup();
    Ok(McpInventoryReport {
        servers,
        trusted_templates: vec![McpTrustedTemplate {
            id: SERVER_NAME.into(),
            name: "Agency Agents".into(),
            clients: vec![McpClient::Claude, McpClient::Codex],
            tool_names: trusted_tools,
            automatic_configuration: true,
        }],
        issues,
    })
}

#[tauri::command]
pub async fn mcp_inventory(state: State<'_, AppState>) -> Result<McpInventoryReport, AppError> {
    mcp_inventory_for_state(&state).await
}

#[tauri::command]
pub async fn mcp_client_connect(client: McpClient) -> Result<McpClientStatus, AppError> {
    let app_exe = env::current_exe().map_err(AppError::from)?;
    let client_path = find_client(client)
        .ok_or_else(|| invalid(format!("{} CLI was not found", client.label())))?;
    let (command, args) = app_command(&app_exe, client)?;
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
    let (before, _) = inspect_at(client, client_path, command, args).await?;
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
    let added = run(client_path, &add_args(client, command, args))
        .await
        .is_ok_and(|result| result.success);
    let verified = added
        && inspect_at(client, client_path, command, args)
            .await
            .is_ok_and(|(status, _)| status.state == McpClientState::Exact);
    if !verified {
        let restored = restore_registration(client, client_path, command, args, None).await;
        return Err(mutation_failed(client, "connection", restored));
    }
    inspect_at(client, client_path, command, args)
        .await
        .map(|(status, _)| status)
}

#[tauri::command]
pub async fn mcp_client_disconnect(client: McpClient) -> Result<McpClientStatus, AppError> {
    let _lock = acquire_client_lock(client).await?;
    let app_exe = env::current_exe().map_err(AppError::from)?;
    let client_path = find_client(client)
        .ok_or_else(|| invalid(format!("{} CLI was not found", client.label())))?;
    let (command, args) = app_command(&app_exe, client)?;
    disconnect_at(client, &client_path, &command, &args).await
}

async fn disconnect_at(
    client: McpClient,
    client_path: &Path,
    command: &str,
    args: &[String],
) -> Result<McpClientStatus, AppError> {
    let (before, previous) = inspect_at(client, client_path, command, args).await?;
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
    let removed = run(client_path, &remove_args(client))
        .await
        .is_ok_and(|result| result.success);
    let verified = removed
        && inspect_at(client, client_path, command, args)
            .await
            .is_ok_and(|(status, _)| status.state == McpClientState::Missing);
    if !verified {
        let restored =
            restore_registration(client, client_path, command, args, previous.as_ref()).await;
        return Err(mutation_failed(client, "disconnection", restored));
    }
    inspect_at(client, client_path, command, args)
        .await
        .map(|(status, _)| status)
}

#[tauri::command]
pub async fn mcp_client_repair(client: McpClient) -> Result<McpClientStatus, AppError> {
    let _lock = acquire_client_lock(client).await?;
    let app_exe = env::current_exe().map_err(AppError::from)?;
    let client_path = find_client(client)
        .ok_or_else(|| invalid(format!("{} CLI was not found", client.label())))?;
    let (command, args) = app_command(&app_exe, client)?;
    repair_at(client, &client_path, &command, &args).await
}

async fn repair_at(
    client: McpClient,
    client_path: &Path,
    command: &str,
    args: &[String],
) -> Result<McpClientStatus, AppError> {
    let (before, previous) = inspect_at(client, client_path, command, args).await?;
    match before.state {
        McpClientState::Exact => return Ok(before),
        McpClientState::Missing => {
            let added = run(client_path, &add_args(client, command, args))
                .await
                .is_ok_and(|result| result.success);
            let verified = added
                && inspect_at(client, client_path, command, args)
                    .await
                    .is_ok_and(|(status, _)| status.state == McpClientState::Exact);
            if !verified {
                let restored = restore_registration(client, client_path, command, args, None).await;
                return Err(mutation_failed(client, "repair", restored));
            }
            return inspect_at(client, client_path, command, args)
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
    let removed = run(client_path, &remove_args(client))
        .await
        .is_ok_and(|result| result.success);
    let added = removed
        && run(client_path, &add_args(client, command, args))
            .await
            .is_ok_and(|output| output.success);
    let verified = added
        && inspect_at(client, client_path, command, args)
            .await
            .is_ok_and(|(status, _)| status.state == McpClientState::Exact);
    if !verified {
        let restored =
            restore_registration(client, client_path, command, args, Some(&previous)).await;
        return Err(mutation_failed(client, "repair", restored));
    }
    inspect_at(client, client_path, command, args)
        .await
        .map(|(status, _)| status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_normalization_redacts_secrets_and_blocks_unsafe_remote_urls() {
        let raw = serde_json::json!({
            "name": "remote",
            "enabled": true,
            "transport": {
                "type": "streamable_http",
                "url": "http://user:password@example.com/mcp?token=secret#fragment",
                "http_headers": {"Authorization": "Bearer super-secret"},
                "env_http_headers": {"X-Api-Key": "MCP_API_KEY"}
            }
        });

        let server = normalize_inventory_server(
            McpClient::Codex,
            "remote",
            McpServerScope::User,
            None,
            &raw,
            None,
            &[],
        );
        let serialized = serde_json::to_string(&server).unwrap();

        assert_eq!(server.endpoint, "http://example.com");
        assert_eq!(server.validation, McpInventoryValidation::Blocked);
        assert_eq!(
            server.environment_keys,
            vec!["header:Authorization", "header:X-Api-Key"]
        );
        for secret in ["password", "token=", "super-secret", "Bearer"] {
            assert!(!serialized.contains(secret), "secret leaked: {secret}");
        }
    }

    #[test]
    fn codex_inventory_is_sorted_bounded_and_reports_honest_tools() {
        let output = serde_json::json!([
            {
                "name": "z-foreign",
                "enabled": true,
                "enabled_tools": ["read", "write", "read"],
                "transport": {"type": "stdio", "command": "npx", "args": ["-y", "pkg"], "env": {}}
            },
            {
                "name": "agency-agents",
                "enabled": true,
                "transport": {"type": "stdio", "command": "/Applications/Agency Agents.app/app", "args": ["--mcp", "--client", "codex"], "env": {}}
            },
            {
                "name": "a-foreign",
                "enabled": false,
                "transport": {"type": "stdio", "command": "local-server", "args": [], "env": {}}
            }
        ])
        .to_string();
        let expected = Registration {
            command: "/Applications/Agency Agents.app/app".into(),
            args: vec!["--mcp".into(), "--client".into(), "codex".into()],
            scope: None,
        };
        let trusted_tools = vec!["skills_search".into(), "agents_search".into()];

        let servers = parse_codex_inventory(&output, Some(&expected), &trusted_tools).unwrap();

        assert_eq!(
            servers
                .iter()
                .map(|server| server.name.as_str())
                .collect::<Vec<_>>(),
            vec!["a-foreign", "agency-agents", "z-foreign"]
        );
        assert_eq!(servers[0].tool_discovery, McpToolDiscovery::Unavailable);
        assert_eq!(servers[1].tool_discovery, McpToolDiscovery::Known);
        assert_eq!(servers[1].tool_names, trusted_tools);
        assert_eq!(servers[2].tool_discovery, McpToolDiscovery::Declared);
        assert_eq!(servers[2].tool_names, vec!["read", "write"]);
        assert!(servers[2]
            .warnings
            .iter()
            .any(|warning| warning.contains("launcher")));
    }

    #[test]
    fn claude_inventory_keeps_scope_and_exact_registered_project_identity() {
        let project = "/projects/example";
        let root = serde_json::json!({
            "mcpServers": {
                "user-server": {"command": "user-bin", "args": [], "env": {}}
            },
            "projects": {
                (project): {
                    "mcpServers": {
                        "local-server": {"command": "local-bin", "args": [], "env": {}}
                    }
                }
            }
        });
        let mut servers = Vec::new();
        parse_claude_user_config(&root, &[PathBuf::from(project)], None, &[], &mut servers)
            .unwrap();

        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].scope, McpServerScope::Local);
        assert_eq!(servers[0].project_path.as_deref(), Some(project));
        assert_eq!(servers[1].scope, McpServerScope::User);
        assert_eq!(servers[1].project_path, None);
    }

    #[test]
    fn inventory_caps_after_deterministic_sorting() {
        let values = (0..(INVENTORY_SERVER_LIMIT + 2))
            .rev()
            .map(|index| {
                serde_json::json!({
                    "name": format!("server-{index:03}"),
                    "transport": {"type": "stdio", "command": "server", "args": []}
                })
            })
            .collect::<Vec<_>>();
        let servers =
            parse_codex_inventory(&serde_json::to_string(&values).unwrap(), None, &[]).unwrap();

        assert_eq!(servers.len(), INVENTORY_SERVER_LIMIT);
        assert_eq!(servers.first().unwrap().name, "server-000");
        assert_eq!(servers.last().unwrap().name, "server-255");
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn inventory_sources_reject_links_and_oversize_files() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("config.json");
        let link = temp.path().join("linked.json");
        std::fs::write(&target, b"{}").unwrap();
        symlink(&target, &link).unwrap();
        assert!(read_inventory_json(&link).await.is_err());

        let oversized = temp.path().join("oversized.json");
        std::fs::write(&oversized, vec![b' '; INVENTORY_CONFIG_LIMIT as usize + 1]).unwrap();
        assert!(read_inventory_json(&oversized).await.is_err());
    }

    #[tokio::test]
    async fn claude_source_failure_keeps_independent_inventory() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        std::fs::create_dir(&project).unwrap();
        std::fs::write(
            temp.path().join(".claude.json"),
            br#"{"mcpServers":{"user-server":{"command":"user-bin","args":[]}}}"#,
        )
        .unwrap();
        std::fs::write(project.join(".mcp.json"), b"not JSON").unwrap();
        let mut servers = Vec::new();
        let mut issues = Vec::new();

        collect_claude_inventory(
            temp.path(),
            &[project],
            None,
            &[],
            &mut servers,
            &mut issues,
        )
        .await;

        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "user-server");
        assert_eq!(issues.len(), 1);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn codex_inventory_invokes_only_literal_json_list_argv() {
        let temp = tempfile::tempdir().unwrap();
        let client = temp.path().join("fake codex");
        let argv = temp.path().join("argv");
        std::fs::write(
            &client,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nprintf '[]'\n",
                argv.display()
            ),
        )
        .unwrap();
        executable(&client);
        let mut servers = Vec::new();
        let mut issues = Vec::new();

        collect_codex_inventory_at(&client, None, &[], &mut servers, &mut issues).await;

        assert_eq!(
            std::fs::read_to_string(argv).unwrap(),
            "mcp\nlist\n--json\n"
        );
        assert!(servers.is_empty());
        assert!(issues.is_empty());
    }

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
        let error = app_command(&temp, McpClient::Codex).unwrap_err();
        assert!(error.to_string().contains("Applications"));
        std::fs::remove_file(temp).unwrap();
    }

    #[test]
    fn rejects_development_build_executable() {
        let current = std::env::current_exe().unwrap();
        assert!(current.to_string_lossy().contains("/target/debug/"));
        assert!(app_command(&current, McpClient::Codex).is_err());
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
