# Phase 15 — MCP Inventory and Trusted Configuration Manager

## Why

Agency Agents can connect its own MCP server to Claude Code and Codex and already owns per-client mutation policy, but users cannot see the rest of their configured MCP surface, validation risks, project usage, or which tool inventory is actually known. A generic MCP installer would expand the app into an unsafe package/runtime launcher.

## What Changes

- Add one bounded local inventory for Claude Code and Codex MCP configurations, including user, exact registered-project, and known local scopes.
- Report transport, privacy-safe endpoint summary, enabled state, environment/header names, validation, exact project usage, and honest tool-discovery state without retaining secrets.
- Expose the exact Agency Agents tool list from its existing composed router as the sole trusted auto-configurable template.
- Reuse existing connect, repair, disconnect, per-client policy, and canonical project allowlist controls as the configuration manager.
- Extend the existing Settings MCP section with inventory, validation evidence, project usage, trusted-template details, refresh progress, and accessible errors.
- Preserve the existing exactly-one-row-per-client status contract and isolated per-client failures.

## Non-Goals

- Installing arbitrary MCP packages, invoking `npx`/`uvx`/Docker, starting external MCP servers, probing external tools, logging in to servers, or making network requests.
- Editing foreign Claude/Codex configuration entries, storing credentials, adding a registry/marketplace, or introducing a new route, dependency, or persisted state document.

## Impact

- Backend: extend `commands/mcp_clients.rs`, shared MCP DTOs, and the existing Tauri command registry; expose the existing composed Agency Agents tool names.
- Frontend: extend existing API/types and `SettingsSectionMcp.svelte` only.
- Persistence: none; inventory is live local evidence and configuration mutations keep using existing authorities.
