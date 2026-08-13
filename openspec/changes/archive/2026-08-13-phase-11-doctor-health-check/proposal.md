## Why

Agency Agents already exposes catalog, storage, tool, MCP, update, and installation status, but users must inspect several screens and interpret inconsistent failure states to understand whether the app is healthy. A consolidated Doctor report is the next priority because trustworthy self-service diagnosis should precede broader update and workspace-management automation.

## What Changes

- Add one on-demand Doctor report that classifies every check as Healthy, Needs attention, or Unavailable and includes bounded evidence plus a safe manual next action.
- Cover current local authorities for storage, settings, catalog, Agent and Skill sources, installation reconciliation, detected deployment tools, MCP client registration, and cached update configuration/state.
- Add Refresh and Copy Report controls with accessible progress/result announcements and deterministic redacted output.
- Route actions to existing recovery surfaces; Doctor never repairs, installs, updates, executes, prompts for credentials, refreshes a network source, or writes telemetry.
- Keep failed or unverifiable checks visible instead of treating them as healthy or failing the entire report.

## Capabilities

### New Capabilities

- `doctor-health-check`: Local evidence-based system diagnosis, report presentation, safe action handoff, and redacted report export.

### Modified Capabilities

None.

## Impact

- Adds one bounded read-only Tauri command and typed Doctor report contract.
- Extends the existing Settings navigation and modal with a Doctor section rather than creating another top-level workspace.
- Reuses existing storage, settings, catalog, source inspection, install reconciliation, tool detection, MCP status, update-state, navigation, localization, and clipboard patterns.
- Adds no dependency, persistence format, network request, scheduled work, telemetry, notification, automatic repair, or direct mutation path.
