## Context

`commands/mcp_clients.rs` already resolves safe Claude/Codex executables, bounds subprocess time/output, parses Agency Agents registrations, serializes mutations with a client lock, verifies outcomes, and restores prior registrations. `SettingsSectionMcp.svelte` already owns connect/repair/disconnect, per-client policy, and canonical project allowlisting. The composed MCP router already knows the exact Agency Agents tool set.

## Goals / Non-Goals

**Goals:**

- Show deterministic privacy-safe MCP configuration inventory for the two supported clients.
- Validate configuration without executing or contacting external servers.
- Make exact known tools visible and unknown tool discovery explicit.
- Keep automatic configuration restricted to the app's existing trusted Agency Agents template.

**Non-Goals:**

- Generic server installation/removal, marketplace metadata, active MCP initialization, OAuth, credential management, arbitrary file paths, or background monitoring.

## Decisions

### Extend existing MCP settings and runner

The feature remains in `SettingsSectionMcp.svelte` and `commands/mcp_clients.rs`. No route, standalone manager component, store, or dependency is justified because the current settings surface already owns all relevant controls and lifecycle state.

### Local config inspection plus one read-only Codex CLI contract

Claude inventory reads bounded regular JSON files only: the user config, exact registered-project `.mcp.json`, and exact registered-project entries in the user config. Codex inventory uses only `codex mcp list --json`, whose current CLI contract emits configuration JSON without starting servers. The existing bounded runner supplies timeout, output, executable, and no-shell guarantees. Claude `mcp list` is not used because it health-checks approved servers.

### Privacy-safe normalized DTO

Raw config never crosses IPC. Each entry retains only bounded server identity, client, scope, exact registered project when applicable, transport, redacted endpoint summary, enabled state, environment/header names, declared tool filters, validation, warnings, and blockers. Inline credential presence becomes a warning without retaining its value. Entries and evidence are sorted and capped.

### Honest tool discovery

The exact Agency Agents tool names come from its existing composed router without starting a server. Foreign entries expose only declared tool filters when present; otherwise the UI states that tools are unavailable without starting the server. The app never active-probes an external MCP process or URL.

### One trusted template

Agency Agents is the only automatic template. Its existing connect/repair/disconnect transaction is reused. Foreign inventory is read-only. Project usage remains the existing exact canonical mutation allowlist rather than a second per-server persistence model.

## Risks / Trade-offs

- Client config formats can evolve → fail one source independently, retain other inventory, and surface a bounded issue rather than guessing.
- Codex CLI can be missing or old → show unavailable client evidence; do not fall back to hand-parsing TOML or add a parser dependency.
- External tools remain unknown → prefer honest unavailable evidence over starting untrusted code.
- A config can contain secrets → inspect in memory, retain only key names and presence warnings, and test serialized IPC output for absence.

## Migration Plan

No migration. The inventory is computed on refresh, and existing settings/registration authorities remain canonical.
