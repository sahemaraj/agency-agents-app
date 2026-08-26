//! Deterministic per-tool agent renderers + destination-path resolution.
//!
//! Ports agency-agents `scripts/convert.sh`. Every renderer is a PURE function
//! of `(Agent, raw source)` — no timestamps, no randomness, stable key order —
//! so `rendered_hash` is reproducible. That reproducibility is the load-bearing
//! requirement for install-state reconciliation (`reconcile/`): we identify an
//! installed file as "ours" by re-rendering its slug for its tool and matching
//! bytes. See `memory-bank/contracts.md` §B/§E.
//!
//! Identity tools (claude-code, copilot) ship the agent `.md` verbatim, so their
//! "render" is the raw corpus source. Transform tools (cursor/.mdc, codex/TOML,
//! gemini-cli, opencode, qwen) rebuild the file from frontmatter fields + body.
//! Skill tools (osaurus, antigravity) emit an Agent-Skills `SKILL.md` directory.
//! Multi-file per-agent tools use [`render_artifacts`]; the original [`render`]
//! API remains the single-file contract.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::registry;
use crate::types::{Agent, Scope};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedArtifact {
    pub content: String,
}

/// Whether `tool` can deploy USER-GLOBALLY (`~/…`). Most CLIs read a user-level
/// agents dir; Cursor is the exception — its global rules live in the Settings
/// UI, with no file path, so it's project-only. Sourced from the registry's
/// `scope` caps. Unknown ids → false.
pub fn supports_user(tool: &str) -> bool {
    registry::get(tool).is_some_and(|m| m.supports_user())
}

/// Whether `tool` can deploy into a SPECIFIC PROJECT (`<project>/…`). Sourced
/// from the registry's `scope` caps. Unknown ids → false.
pub fn supports_project(tool: &str) -> bool {
    registry::get(tool).is_some_and(|m| m.supports_project())
}

/// The scope an install lands in, derived from whether a project root was
/// chosen — NOT a fixed property of the tool. A project path ⇒ project scope.
pub fn scope_for(project_root: Option<&Path>) -> Scope {
    if project_root.is_some() {
        Scope::Project
    } else {
        Scope::User
    }
}

/// Human label for the UI, from the registry. Falls back to the raw id for an
/// unknown tool so callers always get a printable string.
pub fn label(tool: &str) -> String {
    registry::get(tool)
        .map(|m| m.label.clone())
        .unwrap_or_else(|| tool.to_string())
}

/// SHA-256, lowercase hex — the canonical hash for the ledger + reconcile.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let d = h.finalize();
    let mut s = String::with_capacity(64);
    for b in d {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Match `scripts/lib.sh#get_field`: return the first literal `field: value`
/// line between exact `---` fences. The shell helper does not parse YAML, so
/// quotes and other source spelling must be preserved for byte parity.
fn source_field<'a>(source: &'a str, field: &str) -> &'a str {
    let prefix = format!("{field}: ");
    let mut fences = 0;
    for line in source.lines() {
        if line == "---" {
            fences += 1;
            continue;
        }
        if fences == 1 {
            if let Some(value) = line.strip_prefix(&prefix) {
                return value;
            }
        } else if fences >= 2 {
            break;
        }
    }
    ""
}

/// Match `body="$(get_body "$file")"` from the upstream converter. `awk`
/// emits one newline per body line and command substitution strips every
/// trailing newline before the heredoc adds exactly one back.
fn source_body(source: &str) -> String {
    let mut fences = 0;
    let mut body = String::new();
    for line in source.lines() {
        if line == "---" {
            fences += 1;
            continue;
        }
        if fences >= 2 {
            body.push_str(line);
            body.push('\n');
        }
    }
    while body.ends_with('\n') {
        body.pop();
    }
    body
}

/// Match `scripts/lib.sh#slugify`.
pub fn slugify(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut previous_dash = false;
    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            out.push(ch);
            previous_dash = false;
        } else if !out.is_empty() && !previous_dash {
            out.push('-');
            previous_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Filename stem emitted by `convert.sh`. Identity tools preserve the source
/// filename; transform tools derive it from frontmatter `name`.
pub fn output_slug(agent: &Agent, raw_source: &str, tool: &str) -> String {
    // Identity tools (`slugFrom: "source"`) keep the corpus filename; transform
    // tools derive the stem from frontmatter `name`, with an optional namespace
    // prefix (skill-dir tools share a global folder, so we prefix "agency-").
    let meta = registry::get(tool);
    if meta.and_then(|m| m.slug_from.as_deref()) == Some("source") {
        agent.slug.clone()
    } else {
        let prefix = meta.and_then(|m| m.slug_prefix.as_deref()).unwrap_or("");
        let source_name = source_field(raw_source, "name");
        let name = if source_name.is_empty() {
            agent.name.as_str()
        } else {
            source_name
        };
        format!("{prefix}{}", slugify(name))
    }
}

fn unsupported(tool: &str) -> AppError {
    // Error messages use the kebab id (matching `scripts/install.sh`); fall back
    // to the raw id for an unrecognized tool.
    let kebab = registry::get(tool)
        .map(|m| m.kebab.as_str())
        .unwrap_or(tool);
    AppError::Io {
        message: format!("tool '{kebab}' is not supported for install yet (multi-file format)"),
    }
}

/// Render the file content for `tool` from `agent` (+ the raw corpus `.md`
/// source, used verbatim by identity tools). Deterministic.
pub fn render(_agent: &Agent, raw_source: &str, tool: &str) -> Result<String, AppError> {
    let name = source_field(raw_source, "name");
    let description = source_field(raw_source, "description");
    let body = source_body(raw_source);
    let slug = slugify(name);
    // Dispatch on the registry's render `format` key rather than a Rust variant.
    let format = registry::get(tool).and_then(|m| m.format.as_deref());
    let out = match format {
        // Identity — ship the corpus `.md` exactly as authored.
        Some("identity") => raw_source.to_string(),

        // Cursor `.mdc`: description + globs + alwaysApply frontmatter.
        Some("cursor-mdc") => format!(
            "---\ndescription: {desc}\nglobs: \"\"\nalwaysApply: false\n---\n{body}\n",
            desc = description,
        ),

        // Codex TOML: minimal required fields, control chars escaped.
        Some("codex-toml") => format!(
            "name = \"{name}\"\ndescription = \"{desc}\"\ndeveloper_instructions = \"{body}\"\n",
            name = toml_escape(name),
            desc = toml_escape(description),
            body = toml_escape(&body),
        ),

        // Gemini CLI subagent `.md`: name(=slug) + description frontmatter.
        Some("gemini-md") => format!(
            "---\nname: {slug}\ndescription: {desc}\n---\n{body}\n",
            desc = description,
        ),

        // Qwen Code SubAgent `.md`: optional tools line is preserved literally.
        Some("qwen-md") => {
            let tools = source_field(raw_source, "tools");
            if tools.is_empty() {
                format!("---\nname: {slug}\ndescription: {description}\n---\n{body}\n")
            } else {
                format!(
                    "---\nname: {slug}\ndescription: {description}\ntools: {tools}\n---\n{body}\n"
                )
            }
        }

        // ZCode agent `.md` (Z.ai GLM harness): name + description frontmatter,
        // optional `tools` list preserved literally, persona as the body. Read
        // from `.zcode/agents/` (project) or `~/.config/zcode/agents/` (global).
        Some("zcode-md") => {
            let tools = source_field(raw_source, "tools");
            if tools.is_empty() {
                format!("---\nname: {slug}\ndescription: {description}\n---\n{body}\n")
            } else {
                format!(
                    "---\nname: {slug}\ndescription: {description}\ntools: {tools}\n---\n{body}\n"
                )
            }
        }

        // Agent-Skills `SKILL.md`: name (namespaced) + description frontmatter,
        // persona as the body. Mirrors upstream convert.sh `convert_osaurus`
        // (~/.osaurus/skills/<name>/SKILL.md). The `agency-` prefix on `name`
        // comes from the tool's `slugPrefix`.
        Some("skill-md") => {
            let prefix = registry::get(tool).and_then(|m| m.slug_prefix.as_deref()).unwrap_or("");
            format!(
                "---\nname: {prefix}{slug}\ndescription: {desc}\n---\n{body}\n",
                desc = description,
            )
        }

        // OpenCode `.md`: name + description + mode + hex color frontmatter.
        Some("opencode-md") => format!(
            "---\nname: {name}\ndescription: {desc}\nmode: subagent\ncolor: '{color}'\n---\n{body}\n",
            desc = description,
            color = resolve_opencode_color(source_field(raw_source, "color")),
        ),

        // No format (recognized-only) or an unknown renderer ⇒ not installable.
        _ => return Err(unsupported(tool)),
    };
    Ok(out)
}

/// Render the ordered artifact set corresponding to the registry's ordered
/// destination templates. Single-file tools preserve their existing renderer.
pub fn render_artifacts(
    agent: &Agent,
    raw_source: &str,
    tool: &str,
) -> Result<Vec<RenderedArtifact>, AppError> {
    let format = registry::get(tool).and_then(|meta| meta.format.as_deref());
    let contents = match format {
        Some("kimi-agent") => {
            let name = source_field(raw_source, "name");
            let description = source_field(raw_source, "description");
            let body = source_body(raw_source);
            let slug = slugify(name);
            vec![
                format!(
                    "version: 1\nagent:\n  name: {slug}\n  extend: default\n  system_prompt_path: ./system.md\n"
                ),
                format!("# {name}\n\n{description}\n\n{body}\n"),
            ]
        }
        Some("openclaw-workspace") => render_openclaw(raw_source),
        _ => vec![render(agent, raw_source, tool)?],
    };
    Ok(contents
        .into_iter()
        .map(|content| RenderedArtifact { content })
        .collect())
}

/// Render one reviewed project roster into the aggregate file consumed by
/// Aider or Windsurf. Callers own exact-reference ordering; this function
/// preserves it byte-for-byte to keep the reviewed plan revision authoritative.
pub fn render_roster(raw_sources: &[String], tool: &str) -> Result<String, AppError> {
    let mut output = match registry::get(tool).and_then(|meta| meta.format.as_deref()) {
        Some("aider-conventions") => "# The Agency — AI Agent Conventions\n#\n# This file provides Aider with the full roster of specialized AI agents from\n# The Agency (https://github.com/msitarzewski/agency-agents).\n#\n# To activate an agent, reference it by name in your Aider session prompt, e.g.:\n#   \"Use the Frontend Developer agent to review this component.\"\n#\n# Generated by scripts/convert.sh — do not edit manually.\n\n".to_string(),
        Some("windsurf-rules") => "# The Agency — AI Agent Rules for Windsurf\n#\n# Full roster of specialized AI agents from The Agency.\n# To activate an agent, reference it by name in your Windsurf conversation.\n#\n# Generated by scripts/convert.sh — do not edit manually.\n\n".to_string(),
        _ => return Err(unsupported(tool)),
    };
    for raw in raw_sources {
        let name = source_field(raw, "name");
        if name.is_empty() {
            return Err(AppError::InvalidArgument {
                message: "Agent roster member has no name".into(),
            });
        }
        let description = source_field(raw, "description");
        let body = source_body(raw);
        match registry::get(tool).and_then(|meta| meta.format.as_deref()) {
            Some("aider-conventions") => output.push_str(&format!(
                "\n---\n\n## {name}\n\n> {description}\n\n{body}\n"
            )),
            Some("windsurf-rules") => output.push_str(&format!(
                "\n================================================================================\n## {name}\n{description}\n================================================================================\n\n{body}\n\n"
            )),
            _ => unreachable!(),
        }
    }
    Ok(output)
}

fn render_openclaw(raw_source: &str) -> Vec<String> {
    let body = source_body(raw_source);
    let mut soul = String::new();
    let mut agents = String::new();
    let mut section = String::new();
    let mut soul_target = false;

    for line in body.lines() {
        if line.starts_with("## ") {
            if soul_target {
                soul.push_str(&section);
            } else {
                agents.push_str(&section);
            }
            section.clear();
            let header = line.to_ascii_lowercase();
            soul_target = header.contains("identity")
                || (header.contains("learning") && header.contains("memory"))
                || header.contains("communication")
                || header.contains("style")
                || (header.contains("critical") && header.contains("rule"))
                || (header.contains("rules")
                    && header.contains("you")
                    && header.contains("must")
                    && header.contains("follow"));
        }
        section.push_str(line);
        section.push('\n');
    }
    if soul_target {
        soul.push_str(&section);
    } else {
        agents.push_str(&section);
    }
    // Upstream writes each accumulated shell variable through a heredoc. The
    // heredoc contributes one newline in addition to the section's own newline.
    soul.push('\n');
    agents.push('\n');

    let name = source_field(raw_source, "name");
    let description = source_field(raw_source, "description");
    let emoji = source_field(raw_source, "emoji");
    let vibe = source_field(raw_source, "vibe");
    let identity = if !emoji.is_empty() && !vibe.is_empty() {
        format!("# {emoji} {name}\n{vibe}\n")
    } else {
        format!("# {name}\n{description}\n")
    };
    vec![soul, agents, identity]
}

/// Render + hash in one shot.
#[cfg(test)]
pub fn render_with_hash(
    agent: &Agent,
    raw_source: &str,
    tool: &str,
) -> Result<(String, String), AppError> {
    let bytes = render(agent, raw_source, tool)?;
    let hash = sha256_hex(bytes.as_bytes());
    Ok((bytes, hash))
}

/// Absolute destination path(s) for an installed agent. Most tools write a
/// single file; Copilot dual-writes to `~/.github` and `~/.copilot`.
///
/// `home` is the user's home dir (user-scoped tools). `project_root` is required
/// for project-scoped tools (cursor, opencode) and ignored otherwise.
pub fn dests(
    tool: &str,
    slug: &str,
    home: &Path,
    project_root: Option<&Path>,
) -> Result<Vec<PathBuf>, AppError> {
    // A tool with no `dest` templates (and no renderer) is recognized-only.
    let dest = registry::get(tool)
        .and_then(|m| m.dest.as_ref())
        .ok_or_else(|| unsupported(tool))?;

    // Pick the scope-appropriate template array + its root. USER paths are rooted
    // at `$HOME`; PROJECT paths at the project root. Dual-scope same-path tools
    // (claude/codex/gemini/qwen) just re-root the identical relative template;
    // tools whose user/project dirs differ (opencode, copilot) carry separate
    // arrays in the JSON, so this picks the right one.
    let (templates, root): (&[String], &Path) = match project_root {
        Some(p) => (&dest.project, p),
        None => (&dest.user, home),
    };

    // An empty array for the requested scope means this tool can't deploy there.
    // The only such case today is Cursor: project-only, so a user-scoped (no
    // project root) request must surface the existing "project path required"
    // error rather than a multi-file `unsupported`.
    if templates.is_empty() {
        let kebab = registry::get(tool)
            .map(|m| m.kebab.as_str())
            .unwrap_or(tool);
        return Err(AppError::Io {
            message: format!("tool '{kebab}' is project-scoped; a project path is required"),
        });
    }

    Ok(templates
        .iter()
        .map(|t| root.join(t.replace("{slug}", slug)))
        .collect())
}

/// Map an agency-agents `color` (named or hex) to an OpenCode-safe `#RRGGBB`
/// (uppercase). Unknown → neutral gray. Ported from `resolve_opencode_color`.
fn resolve_opencode_color(color: &str) -> String {
    let c = color.trim().to_ascii_lowercase();
    let mapped = match c.as_str() {
        "cyan" => "#00FFFF",
        "blue" => "#3498DB",
        "green" => "#2ECC71",
        "red" => "#E74C3C",
        "purple" => "#9B59B6",
        "orange" => "#F39C12",
        "teal" => "#008080",
        "indigo" => "#6366F1",
        "pink" => "#E84393",
        "gold" => "#EAB308",
        "amber" => "#F59E0B",
        "neon-green" => "#10B981",
        "neon-cyan" => "#06B6D4",
        "metallic-blue" => "#3B82F6",
        "yellow" => "#EAB308",
        "violet" => "#8B5CF6",
        "rose" => "#F43F5E",
        "lime" => "#84CC16",
        "gray" => "#6B7280",
        "fuchsia" => "#D946EF",
        other => other,
    };
    let hex = mapped.strip_prefix('#').unwrap_or(mapped);
    let is_hex6 = hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit());
    if is_hex6 {
        format!("#{}", hex.to_ascii_uppercase())
    } else {
        "#6B7280".to_string()
    }
}

/// Escape a value for a TOML basic string (ported from `toml_escape_string`).
fn toml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) == 0x7F => {
                out.push_str(&format!("\\u{:04X}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::fs;
    use std::process::Command;

    fn agent() -> Agent {
        Agent {
            slug: "frontend-developer".into(),
            name: "Frontend Developer".into(),
            description: "Builds UIs.".into(),
            category: "engineering".into(),
            emoji: Some("🎨".into()),
            color: Some("blue".into()),
            vibe: Some("Ships pixels.".into()),
            body: "You are a frontend dev.\n".into(),
        }
    }

    fn raw() -> &'static str {
        "---\nname: Frontend Developer\ndescription: Builds UIs.\ncolor: blue\nemoji: 🎨\nvibe: Ships pixels.\n---\nYou are a frontend dev.\n"
    }

    #[test]
    fn claude_code_is_identity() {
        let a = agent();
        let raw = "---\nname: Frontend Developer\n---\nORIGINAL BODY\n";
        assert_eq!(render(&a, raw, "claudeCode").unwrap(), raw);
        assert_eq!(render(&a, raw, "copilot").unwrap(), raw);
    }

    #[test]
    fn cursor_mdc_shape() {
        let out = render(&agent(), raw(), "cursor").unwrap();
        assert!(out
            .starts_with("---\ndescription: Builds UIs.\nglobs: \"\"\nalwaysApply: false\n---\n"));
        assert!(out.contains("You are a frontend dev."));
    }

    #[test]
    fn codex_toml_escapes() {
        let mut a = agent();
        a.description = "has \"quotes\" and\nnewline".into();
        let source = "---\nname: Frontend Developer\ndescription: has \"quotes\" and\tcontrols\n---\nline 1\nline \"2\"\n";
        let out = render(&a, source, "codex").unwrap();
        assert!(out.contains("description = \"has \\\"quotes\\\" and\\tcontrols\""));
        assert!(out.contains("developer_instructions = \"line 1\\nline \\\"2\\\"\""));
        assert!(out.starts_with("name = \"Frontend Developer\""));
    }

    #[test]
    fn opencode_color_maps_to_hex() {
        let out = render(&agent(), raw(), "opencode").unwrap();
        assert!(out.contains("color: '#3498DB'"), "blue → #3498DB: {out}");
        assert!(out.contains("mode: subagent"));
    }

    #[test]
    fn osaurus_skill_md_shape_and_dest() {
        // Mirrors upstream convert.sh `convert_osaurus`: a SKILL.md whose `name`
        // carries the `agency-` namespace prefix, persona as the body.
        let out = render(&agent(), raw(), "osaurus").unwrap();
        assert_eq!(
            out,
            "---\nname: agency-frontend-developer\ndescription: Builds UIs.\n---\nYou are a frontend dev.\n"
        );
        // output_slug carries the prefix → it names the skill directory.
        assert_eq!(
            output_slug(&agent(), raw(), "osaurus"),
            "agency-frontend-developer"
        );
        // dest is the nested ~/.osaurus/skills/<name>/SKILL.md (user-scope).
        let d = dests(
            "osaurus",
            "agency-frontend-developer",
            Path::new("/home"),
            None,
        )
        .unwrap();
        assert_eq!(
            d,
            vec![PathBuf::from(
                "/home/.osaurus/skills/agency-frontend-developer/SKILL.md"
            )]
        );
    }

    #[test]
    fn antigravity_skill_md_shape_and_dests() {
        // Antigravity reuses the skill-md renderer (identical shape to osaurus,
        // same `agency-` prefix). Global skills load from ~/.gemini/config/skills/,
        // project skills from <project>/.agents/skills/.
        let out = render(&agent(), raw(), "antigravity").unwrap();
        assert_eq!(
            out,
            "---\nname: agency-frontend-developer\ndescription: Builds UIs.\n---\nYou are a frontend dev.\n"
        );
        assert_eq!(
            output_slug(&agent(), raw(), "antigravity"),
            "agency-frontend-developer"
        );
        // user-scope → ~/.gemini/config/skills/<name>/SKILL.md
        let user = dests(
            "antigravity",
            "agency-frontend-developer",
            Path::new("/home"),
            None,
        )
        .unwrap();
        assert_eq!(
            user,
            vec![PathBuf::from(
                "/home/.gemini/config/skills/agency-frontend-developer/SKILL.md"
            )]
        );
        // project-scope → <project>/.agents/skills/<name>/SKILL.md
        let proj = dests(
            "antigravity",
            "agency-frontend-developer",
            Path::new("/home"),
            Some(Path::new("/proj")),
        )
        .unwrap();
        assert_eq!(
            proj,
            vec![PathBuf::from(
                "/proj/.agents/skills/agency-frontend-developer/SKILL.md"
            )]
        );
    }

    #[test]
    fn opencode_unknown_color_falls_back() {
        let mut a = agent();
        a.color = None;
        let source = "---\nname: Frontend Developer\ndescription: Builds UIs.\n---\nBody\n";
        let out = render(&a, source, "opencode").unwrap();
        assert!(out.contains("color: '#6B7280'"));
    }

    #[test]
    fn gemini_uses_slug_as_name() {
        let out = render(&agent(), raw(), "geminiCli").unwrap();
        assert!(out.starts_with("---\nname: frontend-developer\ndescription: Builds UIs.\n---\n"));
    }

    #[test]
    fn render_is_deterministic() {
        for tool in ["cursor", "codex", "opencode", "geminiCli", "qwen", "zcode"] {
            let a = render(&agent(), raw(), tool).unwrap();
            let b = render(&agent(), raw(), tool).unwrap();
            assert_eq!(a, b, "{tool} must be deterministic");
        }
    }

    #[test]
    fn kimi_artifact_set_matches_upstream_converter() {
        let artifacts = render_artifacts(&agent(), raw(), "kimi").unwrap();
        assert_eq!(artifacts.len(), 2);
        assert_eq!(
            artifacts[0].content,
            "version: 1\nagent:\n  name: frontend-developer\n  extend: default\n  system_prompt_path: ./system.md\n"
        );
        assert_eq!(
            artifacts[1].content,
            "# Frontend Developer\n\nBuilds UIs.\n\nYou are a frontend dev.\n"
        );
    }

    #[test]
    fn openclaw_artifact_set_matches_upstream_converter() {
        let source = "---\nname: Frontend Developer\ndescription: Builds UIs.\nemoji: 🎨\nvibe: Ships pixels.\n---\n## Mission\nBuild reliable UIs.\n## Identity\nYou care about craft.\n## Learning & Memory\nRecord durable lessons.\n## Workflow\nVerify the result.\n## Communication Style\nBe direct.\n";
        let artifacts = render_artifacts(&agent(), source, "openclaw").unwrap();
        assert_eq!(artifacts.len(), 3);
        assert_eq!(
            artifacts[0].content,
            "## Identity\nYou care about craft.\n## Learning & Memory\nRecord durable lessons.\n## Communication Style\nBe direct.\n\n"
        );
        assert_eq!(
            artifacts[1].content,
            "## Mission\nBuild reliable UIs.\n## Workflow\nVerify the result.\n\n"
        );
        assert_eq!(
            artifacts[2].content,
            "# 🎨 Frontend Developer\nShips pixels.\n"
        );
    }

    #[test]
    fn roster_renderers_match_upstream_and_preserve_reviewed_order() {
        let backend = "---\nname: Backend Engineer\ndescription: Builds APIs.\n---\nOwn the API.\n";
        let frontend = raw();
        let sources = vec![backend.to_string(), frontend.to_string()];

        assert_eq!(
            render_roster(&sources, "aider").unwrap(),
            "# The Agency — AI Agent Conventions\n#\n# This file provides Aider with the full roster of specialized AI agents from\n# The Agency (https://github.com/msitarzewski/agency-agents).\n#\n# To activate an agent, reference it by name in your Aider session prompt, e.g.:\n#   \"Use the Frontend Developer agent to review this component.\"\n#\n# Generated by scripts/convert.sh — do not edit manually.\n\n\n---\n\n## Backend Engineer\n\n> Builds APIs.\n\nOwn the API.\n\n---\n\n## Frontend Developer\n\n> Builds UIs.\n\nYou are a frontend dev.\n"
        );
        assert_eq!(
            render_roster(&sources, "windsurf").unwrap(),
            "# The Agency — AI Agent Rules for Windsurf\n#\n# Full roster of specialized AI agents from The Agency.\n# To activate an agent, reference it by name in your Windsurf conversation.\n#\n# Generated by scripts/convert.sh — do not edit manually.\n\n\n================================================================================\n## Backend Engineer\nBuilds APIs.\n================================================================================\n\nOwn the API.\n\n\n================================================================================\n## Frontend Developer\nBuilds UIs.\n================================================================================\n\nYou are a frontend dev.\n\n"
        );
    }

    #[test]
    fn openclaw_fallback_identity_matches_upstream_converter() {
        let source =
            "---\nname: Frontend Developer\ndescription: Builds UIs.\n---\n## Mission\nBuild.\n";
        let artifacts = render_artifacts(&agent(), source, "openclaw").unwrap();
        assert_eq!(artifacts[0].content, "\n");
        assert_eq!(artifacts[1].content, "## Mission\nBuild.\n\n");
        assert_eq!(artifacts[2].content, "# Frontend Developer\nBuilds UIs.\n");
    }

    #[test]
    fn agent_prompt_content_is_rendered_as_data_without_execution() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("must-not-exist");
        let source = format!(
            "---\nname: Frontend Developer\ndescription: Builds UIs.\n---\n$(touch {})\n",
            marker.display()
        );
        let rendered = render(&agent(), &source, "claudeCode").unwrap();
        assert!(rendered.contains(&marker.to_string_lossy().to_string()));
        assert!(!marker.exists());
    }

    #[test]
    fn source_helpers_match_shell_semantics() {
        let source = "---\nname: \"Quoted Name\"\ndescription: has: colon\ntools: Read, Write\n---\nBody\n---\nTail\n\n";
        assert_eq!(source_field(source, "name"), "\"Quoted Name\"");
        assert_eq!(source_field(source, "description"), "has: colon");
        assert_eq!(source_body(source), "Body\nTail");
        assert_eq!(slugify("FP&A / QA"), "fp-a-qa");
    }

    #[test]
    fn qwen_preserves_optional_tools() {
        let source = "---\nname: Frontend Developer\ndescription: Builds UIs.\ntools: Read, Write\n---\nBody\n";
        let out = render(&agent(), source, "qwen").unwrap();
        assert!(out.contains("\ntools: Read, Write\n"));

        let without = render(&agent(), raw(), "qwen").unwrap();
        assert!(!without.contains("\ntools: "));
    }

    #[test]
    fn zcode_uses_slug_name_and_optional_tools() {
        // ZCode agent .md: name(=slug) + description frontmatter, optional tools.
        let out = render(&agent(), raw(), "zcode").unwrap();
        assert!(out.starts_with("---\nname: frontend-developer\ndescription: Builds UIs.\n---\n"));
        assert!(!out.contains("\ntools: "));

        let source = "---\nname: Frontend Developer\ndescription: Builds UIs.\ntools: Read, Write\n---\nBody\n";
        let with = render(&agent(), source, "zcode").unwrap();
        assert!(with.contains("\ntools: Read, Write\n"));
    }

    #[test]
    fn output_slug_matches_converter_identity_rules() {
        let mut a = agent();
        a.slug = "engineering-frontend-developer".into();
        assert_eq!(
            output_slug(&a, raw(), "claudeCode"),
            "engineering-frontend-developer"
        );
        assert_eq!(output_slug(&a, raw(), "codex"), "frontend-developer");
    }

    fn collect_markdown(root: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_markdown(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
                out.push(path);
            }
        }
    }

    #[test]
    #[ignore = "requires AGENCY_AGENTS_PARITY_ROOT and executes upstream convert.sh"]
    fn upstream_convert_sh_is_byte_identical_for_transform_tools() {
        let root = std::env::var("AGENCY_AGENTS_PARITY_ROOT")
            .expect("set AGENCY_AGENTS_PARITY_ROOT to an agency-agents clone");
        let root = PathBuf::from(root);
        let script = root.join("scripts/convert.sh");
        assert!(script.is_file(), "missing {}", script.display());

        let script_text = fs::read_to_string(&script).unwrap();
        let dirs_start = script_text.find("AGENT_DIRS=(").expect("AGENT_DIRS");
        let dirs_tail = &script_text[dirs_start + "AGENT_DIRS=(".len()..];
        let dirs_body = dirs_tail.split(')').next().expect("AGENT_DIRS close");
        let categories: Vec<&str> = dirs_body.split_whitespace().collect();

        let temp = tempfile::tempdir().unwrap();
        // Every single-artifact transform the pinned upstream converter emits.
        // Keep this list exhaustive: silently dropping a tool here turns a
        // parity regression into a green build.
        let tools = [
            ("cursor", "cursor/rules", "mdc"),
            ("geminiCli", "gemini-cli/agents", "md"),
            ("opencode", "opencode/agents", "md"),
            ("qwen", "qwen/agents", "md"),
            ("codex", "codex/agents", "toml"),
            ("zcode", "zcode/agents", "md"),
        ];
        for (_, tool_id, _) in tools {
            let tool = tool_id.split('/').next().unwrap();
            let output = Command::new("bash")
                .arg(&script)
                .args(["--tool", tool, "--out"])
                .arg(temp.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "upstream converter failed for required parity tool '{tool}': {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let mut files = Vec::new();
        for category in categories {
            collect_markdown(&root.join(category), &mut files);
        }
        files.sort();

        let mut conversion_slugs = HashSet::new();
        let mut compared = 0usize;
        for path in files {
            let raw = fs::read_to_string(&path).unwrap();
            let name = source_field(&raw, "name");
            if name.is_empty() || !raw.starts_with("---\n") {
                continue;
            }
            let source_slug = path.file_stem().unwrap().to_string_lossy().to_string();
            let agent = Agent {
                slug: source_slug,
                name: name.to_string(),
                description: String::new(),
                category: String::new(),
                emoji: None,
                color: None,
                vibe: None,
                body: String::new(),
            };
            let converted_slug = output_slug(&agent, &raw, "cursor");
            assert!(
                conversion_slugs.insert(converted_slug.clone()),
                "duplicate conversion slug: {converted_slug}"
            );
            for (tool, subdir, ext) in tools {
                let expected_path = temp
                    .path()
                    .join(subdir)
                    .join(format!("{converted_slug}.{ext}"));
                let expected = fs::read(&expected_path)
                    .unwrap_or_else(|e| panic!("read {}: {e}", expected_path.display()));
                let actual = render(&agent, &raw, tool).unwrap();
                assert_eq!(
                    actual.as_bytes(),
                    expected,
                    "{tool} parity mismatch for {}",
                    path.display()
                );
                compared += 1;
            }
        }
        assert!(compared > 0);
        eprintln!(
            "renderer parity: {} agents, {} byte comparisons",
            conversion_slugs.len(),
            compared
        );
    }

    #[test]
    #[ignore = "requires AGENCY_AGENTS_PARITY_ROOT and executes upstream convert.sh"]
    fn upstream_convert_sh_is_byte_identical_for_multi_artifact_tools() {
        let root = PathBuf::from(
            std::env::var("AGENCY_AGENTS_PARITY_ROOT")
                .expect("set AGENCY_AGENTS_PARITY_ROOT to an agency-agents clone"),
        );
        let script = root.join("scripts/convert.sh");
        let temp = tempfile::tempdir().unwrap();
        for tool in ["kimi", "openclaw", "antigravity"] {
            let status = Command::new("bash")
                .arg(&script)
                .args(["--tool", tool, "--out"])
                .arg(temp.path())
                .status()
                .unwrap();
            assert!(status.success(), "convert.sh failed for {tool}");
        }

        let script_text = fs::read_to_string(&script).unwrap();
        let dirs_start = script_text.find("AGENT_DIRS=(").expect("AGENT_DIRS");
        let dirs_tail = &script_text[dirs_start + "AGENT_DIRS=(".len()..];
        let categories = dirs_tail
            .split(')')
            .next()
            .expect("AGENT_DIRS close")
            .split_whitespace();
        let mut files = Vec::new();
        for category in categories {
            collect_markdown(&root.join(category), &mut files);
        }
        files.sort();

        let mut compared = 0usize;
        for path in files {
            let raw = fs::read_to_string(&path).unwrap();
            let name = source_field(&raw, "name");
            if name.is_empty() || !raw.starts_with("---\n") {
                continue;
            }
            let agent = Agent {
                slug: path.file_stem().unwrap().to_string_lossy().into_owned(),
                name: name.into(),
                description: String::new(),
                category: String::new(),
                emoji: None,
                color: None,
                vibe: None,
                body: String::new(),
            };
            let slug = slugify(name);
            for (tool, leaves) in [
                ("kimi", &["agent.yaml", "system.md"][..]),
                ("openclaw", &["SOUL.md", "AGENTS.md", "IDENTITY.md"][..]),
                ("antigravity", &["SKILL.md"][..]),
            ] {
                let actual = render_artifacts(&agent, &raw, tool).unwrap();
                assert_eq!(actual.len(), leaves.len());
                for (artifact, leaf) in actual.iter().zip(leaves) {
                    let expected_slug = if tool == "antigravity" {
                        format!("agency-{slug}")
                    } else {
                        slug.clone()
                    };
                    let expected_path = temp.path().join(tool).join(expected_slug).join(leaf);
                    let expected = fs::read(&expected_path).unwrap_or_else(|error| {
                        panic!("read {}: {error}", expected_path.display())
                    });
                    assert_eq!(
                        artifact.content.as_bytes(),
                        expected,
                        "{tool}/{leaf} parity mismatch for {}",
                        path.display()
                    );
                    compared += 1;
                }
            }
        }
        assert!(compared > 0);
        eprintln!("multi-artifact renderer parity: {compared} byte comparisons");
    }

    #[test]
    #[ignore = "requires AGENCY_AGENTS_PARITY_ROOT and executes upstream convert.sh"]
    fn upstream_convert_sh_is_byte_identical_for_roster_tools() {
        let root = PathBuf::from(
            std::env::var("AGENCY_AGENTS_PARITY_ROOT")
                .expect("set AGENCY_AGENTS_PARITY_ROOT to an agency-agents clone"),
        );
        let script = root.join("scripts/convert.sh");
        let temp = tempfile::tempdir().unwrap();
        for tool in ["aider", "windsurf"] {
            let status = Command::new("bash")
                .arg(&script)
                .args(["--tool", tool, "--out"])
                .arg(temp.path())
                .status()
                .unwrap();
            assert!(status.success(), "convert.sh failed for {tool}");
        }

        let script_text = fs::read_to_string(&script).unwrap();
        let dirs_start = script_text.find("AGENT_DIRS=(").expect("AGENT_DIRS");
        let dirs_tail = &script_text[dirs_start + "AGENT_DIRS=(".len()..];
        let categories = dirs_tail
            .split(')')
            .next()
            .expect("AGENT_DIRS close")
            .split_whitespace();
        let mut sources = Vec::new();
        for category in categories {
            let mut files = Vec::new();
            collect_markdown(&root.join(category), &mut files);
            files.sort();
            for path in files {
                let raw = fs::read_to_string(path).unwrap();
                if raw.starts_with("---\n") && !source_field(&raw, "name").is_empty() {
                    sources.push(raw);
                }
            }
        }
        assert!(!sources.is_empty());
        for (tool, expected_path) in [
            ("aider", temp.path().join("aider/CONVENTIONS.md")),
            ("windsurf", temp.path().join("windsurf/.windsurfrules")),
        ] {
            let expected = fs::read(&expected_path).unwrap();
            let actual = render_roster(&sources, tool).unwrap();
            assert_eq!(
                actual.as_bytes(),
                expected,
                "{tool} aggregate parity mismatch"
            );
        }
    }

    #[test]
    fn unsupported_tools_error() {
        // These tools are in the catalog (upstream truth: real format + dest), but
        // this app ships no renderer for their format — so render() must refuse.
        // dests() legitimately returns the upstream templates; the install path is
        // gated on render(), and these tools aren't in the installable set anyway.
        for tool in ["windsurf", "aider", "openclaw", "kimi"] {
            assert!(
                render(&agent(), "raw", tool).is_err(),
                "{tool} has no app renderer"
            );
        }
    }

    #[test]
    fn dests_per_tool() {
        let home = Path::new("/Users/x");
        let proj = Path::new("/proj");
        assert_eq!(
            dests("claudeCode", "a", home, None).unwrap(),
            vec![PathBuf::from("/Users/x/.claude/agents/a.md")]
        );
        assert_eq!(dests("copilot", "a", home, None).unwrap().len(), 2);
        assert_eq!(
            dests("codex", "a", home, None).unwrap(),
            vec![PathBuf::from("/Users/x/.codex/agents/a.toml")]
        );
        assert_eq!(
            dests("cursor", "a", home, Some(proj)).unwrap(),
            vec![PathBuf::from("/proj/.cursor/rules/a.mdc")]
        );
        // project-scoped without a project path → error
        assert!(dests("cursor", "a", home, None).is_err());
    }

    #[test]
    fn scope_capabilities() {
        // Dual-scope tools support both global and project; Cursor is project-only.
        assert!(supports_user("claudeCode") && supports_project("claudeCode"));
        assert!(supports_user("opencode") && supports_project("opencode"));
        assert!(supports_user("codex") && supports_project("codex"));
        assert!(!supports_user("cursor") && supports_project("cursor"));
        // An install's scope comes from whether a project root was chosen.
        assert_eq!(scope_for(None), Scope::User);
        assert_eq!(scope_for(Some(Path::new("/p"))), Scope::Project);
    }

    #[test]
    fn dests_are_scope_aware() {
        let home = Path::new("/home/u");
        let proj = Path::new("/work/app");
        // Root-swap tools: same relative path, rooted at home or the project.
        assert_eq!(
            dests("claudeCode", "x", home, None).unwrap()[0],
            home.join(".claude/agents/x.md")
        );
        assert_eq!(
            dests("claudeCode", "x", home, Some(proj)).unwrap()[0],
            proj.join(".claude/agents/x.md")
        );
        // opencode uses DIFFERENT dirs per scope.
        assert_eq!(
            dests("opencode", "x", home, None).unwrap()[0],
            home.join(".config/opencode/agents/x.md")
        );
        assert_eq!(
            dests("opencode", "x", home, Some(proj)).unwrap()[0],
            proj.join(".opencode/agents/x.md")
        );
        // Cursor is project-only: a global (no project root) request errors.
        assert!(dests("cursor", "x", home, None).is_err());
        assert_eq!(
            dests("cursor", "x", home, Some(proj)).unwrap()[0],
            proj.join(".cursor/rules/x.mdc")
        );
    }
}
