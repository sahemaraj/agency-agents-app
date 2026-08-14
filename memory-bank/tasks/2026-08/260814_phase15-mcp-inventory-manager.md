# 260814_phase15-mcp-inventory-manager

## Objective

Inventory supported Claude Code and Codex MCP configuration with bounded privacy-safe evidence, passive validation, honest tool discovery, and one trusted Agency Agents template without becoming an arbitrary server installer or runtime.

## Outcome

- Added one live inventory command for Claude user/project/local configuration and Codex `mcp list --json` output, with deterministic source identity, sorting, deduplication, and caps.
- Added no-follow bounded config reads, isolated malformed/link/oversize failures, redacted command/remote endpoint summaries, environment/header names only, inline-secret warnings, transport validation, and launcher warnings.
- Exposed the exact sorted unique 130-tool Agency Agents router inventory without starting the MCP server; foreign tools remain declared-only or explicitly unavailable.
- Reused existing Agency Agents connect/repair/disconnect, per-client permission policy, and exact canonical project allowlist; foreign inventory has no mutation action.
- Extended the existing MCP Settings section with inventory, scope/project evidence, validation findings, tool evidence, partial issues, refresh progress, and accessible completion announcements.

## Verification

- OpenSpec: 15/15 tasks complete; strict change validation passed; canonical specs validate 9/9 after sync.
- Frontend: 108/108 tests passed; Svelte diagnostics reported 0 errors and 0 warnings; production build passed.
- Backend: 569 library tests discovered; 566 passed with 3 existing environment-gated ignores; 2/2 binary tests passed; 10/10 focused inventory-related tests passed.
- Rust quality: formatting and strict Clippy passed.
- Safety: secret-retention, endpoint redaction, bounded input/output, no-follow link/reparse, isolated-source, exact-project, literal-argv, no-Claude-health-check, no-network, no-external-server, foreign-read-only, dependency, persistence, route, telemetry, notification, unrelated-mutation, and diff audits passed.

## Integration Points

- `src-tauri/src/commands/mcp_clients.rs` owns supported-source collection, normalization, passive validation, and the existing bounded Codex runner integration.
- `src-tauri/src/skills/mcp.rs` exposes exact tool names from the existing composed router without starting a server.
- `src/lib/components/SettingsSectionMcp.svelte` owns the inventory UI beside the existing trusted Agency Agents and policy controls.
- `openspec/specs/mcp-inventory/spec.md` is the canonical capability contract.

## Security and Safety

- The command accepts no arbitrary path or server input. Claude sources are fixed to the user config and exact registered projects; final config files are opened no-follow and bounded to 1 MiB.
- Raw configuration, arguments, environment/header values, URL paths, userinfo, query strings, and fragments never cross IPC. Only bounded names, key names, and safe endpoint summaries are retained.
- Inventory never invokes Claude's health-checking list command, contacts remote endpoints, or starts foreign servers. The only added process path is literal `codex mcp list --json` through the existing timeout/output-capped no-shell runner.
- Automatic mutation remains limited to the existing exact Agency Agents registration transaction. Foreign entries expose evidence only.

## Artifacts

- Implementation commit: `4d4d89d`
- Branch: `feat/phase-15-mcp-inventory`
- OpenSpec archive: `openspec/changes/archive/2026-08-14-phase-15-mcp-inventory-manager/`
