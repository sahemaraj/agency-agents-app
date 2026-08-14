## Why

Users can export an Agent-only installation list, but they cannot carry a complete reusable workspace definition across projects or machines and review it safely before anything changes. Workspace Packs are the next packaging layer after reliable planning, repair, search, health, and receipts: one local portable contract for exact Agents, Skills, runbook context, tool targets, project scope, instruction requirements, and optional MCP requirements.

## What Changes

- Replace the unsafe one-click Agentfile restore experience with bounded Workspace Pack inspection, validation, complete destination planning, and one explicit approval before mutation.
- Export the current managed Agent and Skill deployment as a deterministic versioned local JSON pack using exact source-aware references; retain Agentfile v1 read compatibility.
- Carry optional runbook, instruction, and MCP requirements as portable declarative metadata. Phase 13 displays and validates these requirements but does not edit instruction files, configure or install MCP servers, or execute a runbook.
- Apply only installable Agent and Skill entries through existing exact-reference lifecycle, backup, rollback, reconciliation, and receipt boundaries; a blocker causes zero writes and an apply failure removes only artifacts created by that pack run.
- Keep the feature inside the existing Teams file controls and review UI without adding a top-level destination, cloud sharing, marketplace, network access, or runtime execution.

## Capabilities

### New Capabilities

- `workspace-packs`: Versioned local pack export, backward-compatible import, bounded validation, declarative requirements, complete read-only planning, explicit approval, recoverable application, and exact completion evidence.

### Modified Capabilities

None.

## Impact

- Extends the existing Agentfile/loadout commands and Teams import/export surface, exact Agent and Skill source/plan primitives, runbook catalog, transactional filesystem-operation pattern, reconciliation stores, and post-action receipts.
- Adds no dependency, database authority, network path, telemetry, native notification, route, arbitrary instruction write, MCP server installation, or agent runtime.
