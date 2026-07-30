// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

const DEFAULT_MCP_HTTP_BIND: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8765);

#[derive(Debug, PartialEq)]
enum Mode {
    App,
    Stdio,
    Http(SocketAddr),
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
    if args == ["--mcp"] {
        return Ok(Mode::Stdio);
    }
    if args.first().map(String::as_str) != Some("--mcp-http") {
        return Err("expected --mcp or --mcp-http [--bind LOOPBACK:PORT]".into());
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

#[tokio::main]
async fn main() {
    let result = match parse_mode(std::env::args()) {
        Ok(Mode::App) => {
            agency_agents_lib::run();
            return;
        }
        Ok(Mode::Stdio) => agency_agents_lib::run_mcp().await,
        Ok(Mode::Http(bind)) => match std::env::var("AGENCY_AGENTS_MCP_TOKEN") {
            Ok(token) => agency_agents_lib::run_mcp_http(bind, token).await,
            Err(_) => Err("AGENCY_AGENTS_MCP_TOKEN is required".into()),
        },
        Err(error) => Err(error),
    };
    if let Err(error) = result {
        eprintln!("MCP server failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_mode, Mode, DEFAULT_MCP_HTTP_BIND};

    #[test]
    fn parses_mcp_modes() {
        assert_eq!(
            parse_mode(["agency-agents-app", "--mcp"]).unwrap(),
            Mode::Stdio
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
}
