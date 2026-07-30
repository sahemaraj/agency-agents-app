# 260730_mcp-skills-platform

## Objective

Expose the app's validated Skills library through MCP so Claude Code and Codex can discover,
inspect, install, maintain, recommend, and safely contribute skills, with optional authenticated
loopback HTTP access.

## Outcome

- ✅ Added 22 MCP tools for catalog search, package files/resources, install lifecycle, sources,
  deterministic recommendations, and managed draft submission.
- ✅ Added default-denied mutation policy, canonical project allowlists, durable bounded audit
  history, capability-relative project operations, cross-process locks, and rollback protection.
- ✅ Added a managed draft inbox; MCP can submit/read drafts, while publication and rejection remain
  desktop-only human actions.
- ✅ Added official CLI-based Claude Code and Codex connection management with exact-state checks,
  bounded execution, ownership-safe rollback, and no shell execution.
- ✅ Added bearer-authenticated Streamable HTTP on loopback only, with constant-time token
  comparison, Host/Origin validation, request caps, and graceful shutdown.
- ✅ Rust: 379 library + 2 CLI tests passed; 1 library and 6 external tests ignored.
- ✅ Live stdio and HTTP exchanges exposed all 22 tools; HTTP rejected missing authentication
  with status 401.
- ✅ Svelte check: 0 errors, 1 pre-existing missing `@types/node` warning.
- ✅ Production build and whitespace checks passed.
- ✅ Independent final specification, quality, and security audit passed.

## Files Modified

- `src-tauri/src/skills/mcp.rs` — shared stdio/HTTP MCP server, tools, resources, policy boundary.
- `src-tauri/src/skills/mod.rs` — validated package/source and lifecycle core operations.
- `src-tauri/src/skills/install.rs` — transactional capability-relative skill installation.
- `src-tauri/src/skills/drafts.rs` — bounded, transactional managed draft inbox.
- `src-tauri/src/state.rs` — live policy authorization and durable audit journal.
- `src-tauri/src/commands/mcp_clients.rs` — Claude Code and Codex CLI registration lifecycle.
- `src-tauri/src/commands/settings.rs` — serialized general/MCP policy patches.
- `src-tauri/src/main.rs` — stdio and authenticated loopback HTTP modes.
- `src/lib/components/SettingsSectionMcp.svelte` — accessible MCP client connection controls.
- `src/lib/components/SkillsWorkspace.svelte` — desktop draft review and publication controls.
- `src/lib/stores/activity.svelte.ts` — durable MCP audit rows merged with local Activity.

## Patterns Applied

- Extended the existing validator, source registry, transactional installer, lifecycle ledger,
  Settings, and Activity surfaces.
- Kept one `SkillMcpServer` for both transports.
- Routed all tool calls through one fail-closed policy and durable audit boundary.
- Used stable directory capabilities and basename-relative operations for project mutations.
- Delegated client configuration to official Claude and Codex CLIs.

## Integration Points

- Claude Code/Codex connect to the app executable with `--mcp`, or to authenticated loopback
  `POST /mcp` using `AGENCY_AGENTS_MCP_TOKEN`.
- MCP catalog resources and revision hashes reflect validated source state.
- MCP lifecycle calls reuse the same ledgers, backup rules, and reconciliation states as the UI.
- Draft publication creates or reuses one app-owned local source only after desktop approval.
- MCP audit records appear in Activity but cannot be cleared by the local-history clear action.

## Scope Boundaries

No hosted service, account system, telemetry, background daemon, arbitrary executable execution,
non-loopback listener, in-process TLS, commit, push, PR, or release was added.

## Artifacts

- Plan: `docs/superpowers/plans/2026-07-30-mcp-skills-platform.md`
- Implementation ledger: `.superpowers/sdd/2026-07-30-mcp-skills-platform/progress.md`
