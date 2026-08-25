// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
};

const DEFAULT_MCP_HTTP_BIND: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8765);

#[derive(Debug, PartialEq)]
enum Mode {
    App,
    Stdio(String),
    Http(SocketAddr),
    Cli(CliArgs),
}

#[derive(Debug, PartialEq)]
struct CliArgs {
    command: String,
    project: Option<PathBuf>,
    json: bool,
    dry_run: bool,
    merge: bool,
}

fn parse_mode<I, S>(args: I) -> Result<Mode, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<String> = args
        .into_iter()
        .skip(1)
        .map(|arg| arg.as_ref().to_owned())
        .collect();
    if args.is_empty() {
        return Ok(Mode::App);
    }
    if matches!(
        args.first().map(String::as_str),
        Some("check" | "plan" | "apply" | "list")
    ) {
        return parse_cli(&args).map(Mode::Cli);
    }
    if args == ["--mcp"] {
        return Ok(Mode::Stdio("unknown".into()));
    }
    if let [flag, client_flag, client] = args.as_slice() {
        if flag == "--mcp"
            && client_flag == "--client"
            && matches!(client.as_str(), "claude" | "codex")
        {
            return Ok(Mode::Stdio(client.clone()));
        }
    }
    if args.first().map(String::as_str) != Some("--mcp-http") {
        return Err(
            "expected check, plan, apply, list, --mcp, or --mcp-http [--bind LOOPBACK:PORT]".into(),
        );
    }
    let bind = match args.as_slice() {
        [_] => DEFAULT_MCP_HTTP_BIND,
        [_, flag, value] if flag == "--bind" => value
            .parse::<SocketAddr>()
            .map_err(|_| "--bind must be an IP socket address".to_string())?,
        _ => return Err("expected --mcp-http [--bind LOOPBACK:PORT]".into()),
    };
    if !bind.ip().is_loopback() {
        return Err("--bind must use a loopback IP address".into());
    }
    Ok(Mode::Http(bind))
}

fn parse_cli(args: &[String]) -> Result<CliArgs, String> {
    let command = args[0].clone();
    let mut project = None;
    let mut json = false;
    let mut dry_run = false;
    let mut merge = false;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--project" if project.is_none() => {
                let value = args
                    .get(index + 1)
                    .filter(|value| !value.starts_with("--"))
                    .ok_or_else(|| "--project requires a path".to_string())?;
                project = Some(PathBuf::from(value));
                index += 2;
            }
            "--json" if !json => {
                json = true;
                index += 1;
            }
            "--dry-run" if command == "apply" && !dry_run => {
                dry_run = true;
                index += 1;
            }
            "--merge" if command == "apply" && !merge => {
                merge = true;
                index += 1;
            }
            flag => return Err(format!("invalid {command} option: {flag}")),
        }
    }
    Ok(CliArgs {
        command,
        project,
        json,
        dry_run,
        merge,
    })
}

fn main() {
    let mode = match parse_mode(std::env::args()) {
        Ok(mode) => mode,
        Err(error) => {
            let is_cli = matches!(
                std::env::args().nth(1).as_deref(),
                Some("check" | "plan" | "apply" | "list")
            );
            eprintln!(
                "{}",
                if is_cli {
                    format!("error: {error}")
                } else {
                    format!("MCP server failed: {error}")
                }
            );
            std::process::exit(if is_cli { 2 } else { 1 });
        }
    };

    if matches!(&mode, Mode::App) {
        agency_agents_lib::run();
        return;
    }

    let cli_mode = matches!(&mode, Mode::Cli(_));
    let runtime = tokio::runtime::Runtime::new().unwrap_or_else(|error| {
        eprintln!(
            "{}",
            if cli_mode {
                format!("error: {error}")
            } else {
                format!("MCP server failed: {error}")
            }
        );
        std::process::exit(if cli_mode { 2 } else { 1 });
    });
    let result = runtime.block_on(async move {
        match mode {
            Mode::Stdio(client) => agency_agents_lib::run_mcp(client).await.map(|_| 0),
            Mode::Http(bind) => match std::env::var("AGENCY_AGENTS_MCP_TOKEN") {
                Ok(token) => agency_agents_lib::run_mcp_http(bind, token)
                    .await
                    .map(|_| 0),
                Err(_) => Err("AGENCY_AGENTS_MCP_TOKEN is required".into()),
            },
            Mode::Cli(args) => agency_agents_lib::run_cli(
                &args.command,
                args.project,
                args.json,
                args.dry_run,
                args.merge,
            )
            .await
            .map(|outcome| {
                print!("{}", outcome.stdout);
                outcome.exit_code
            }),
            Mode::App => unreachable!(),
        }
    });
    match result {
        Ok(0) => {}
        Ok(exit_code) => std::process::exit(exit_code),
        Err(error) => {
            eprintln!(
                "{}",
                if cli_mode {
                    format!("error: {error}")
                } else {
                    format!("MCP server failed: {error}")
                }
            );
            std::process::exit(if cli_mode { 2 } else { 1 });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_mode, CliArgs, Mode, DEFAULT_MCP_HTTP_BIND};

    #[test]
    fn parses_mcp_modes() {
        assert_eq!(
            parse_mode(["agency-agents-app", "--mcp"]).unwrap(),
            Mode::Stdio("unknown".into())
        );
        assert_eq!(
            parse_mode(["agency-agents-app", "--mcp", "--client", "codex"]).unwrap(),
            Mode::Stdio("codex".into())
        );
        assert_eq!(
            parse_mode(["agency-agents-app", "--mcp-http"]).unwrap(),
            Mode::Http(DEFAULT_MCP_HTTP_BIND)
        );
        assert_eq!(
            parse_mode(["agency-agents-app", "--mcp-http", "--bind", "[::1]:0"]).unwrap(),
            Mode::Http("[::1]:0".parse().unwrap())
        );
    }

    #[test]
    fn rejects_non_loopback_or_ambiguous_http_binds() {
        for bind in ["0.0.0.0:8765", "192.168.1.2:8765", "localhost:8765"] {
            assert!(parse_mode(["app", "--mcp-http", "--bind", bind]).is_err());
        }
        assert!(parse_mode(["app", "--mcp", "--mcp-http"]).is_err());
        assert!(parse_mode(["app", "--bind", "127.0.0.1:1"]).is_err());
    }

    #[test]
    fn parses_cli_verbs_and_flags() {
        assert_eq!(
            parse_mode(["app", "check"]).unwrap(),
            Mode::Cli(CliArgs {
                command: "check".into(),
                project: None,
                json: false,
                dry_run: false,
                merge: false,
            })
        );
        assert_eq!(
            parse_mode(["app", "plan", "--json", "--project", "/tmp/project"]).unwrap(),
            Mode::Cli(CliArgs {
                command: "plan".into(),
                project: Some("/tmp/project".into()),
                json: true,
                dry_run: false,
                merge: false,
            })
        );
        assert_eq!(
            parse_mode([
                "app",
                "apply",
                "--project",
                "/tmp/project",
                "--dry-run",
                "--json",
            ])
            .unwrap(),
            Mode::Cli(CliArgs {
                command: "apply".into(),
                project: Some("/tmp/project".into()),
                json: true,
                dry_run: true,
                merge: false,
            })
        );
        assert_eq!(
            parse_mode(["app", "list", "--project", "."]).unwrap(),
            Mode::Cli(CliArgs {
                command: "list".into(),
                project: Some(".".into()),
                json: false,
                dry_run: false,
                merge: false,
            })
        );
        assert_eq!(
            parse_mode(["app", "apply", "--merge"]).unwrap(),
            Mode::Cli(CliArgs {
                command: "apply".into(),
                project: None,
                json: false,
                dry_run: false,
                merge: true,
            })
        );
    }

    #[test]
    fn rejects_invalid_cli_options() {
        assert!(parse_mode(["app", "check", "--dry-run"]).is_err());
        assert!(parse_mode(["app", "apply", "--project"]).is_err());
        assert!(parse_mode(["app", "list", "--json", "--json"]).is_err());
        assert!(parse_mode(["app", "plan", "--merge"]).is_err());
        assert!(parse_mode(["app", "plan", "--unknown"]).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "launches the native desktop binary"]
    fn desktop_app_stays_alive_past_startup() {
        use std::{
            io::Read,
            path::PathBuf,
            process::{Command, Stdio},
            thread,
            time::{Duration, Instant},
        };

        let binary =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/agency-agents-app");
        assert!(
            binary.is_file(),
            "build the desktop binary before running this test"
        );
        let mut child = Command::new(binary)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("launch desktop binary");
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if let Some(status) = child.try_wait().expect("poll desktop binary") {
                let mut stderr = String::new();
                child
                    .stderr
                    .take()
                    .expect("capture desktop stderr")
                    .read_to_string(&mut stderr)
                    .expect("read desktop stderr");
                panic!("desktop app exited during startup ({status}): {stderr}");
            }
            thread::sleep(Duration::from_millis(100));
        }
        child.kill().expect("stop desktop binary");
        child.wait().expect("reap desktop binary");
    }
}
