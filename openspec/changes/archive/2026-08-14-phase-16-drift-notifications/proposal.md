## Why

Reliable foreground reconciliation already identifies local Agent and Skill drift, but users receive no signal while Agency Agents is backgrounded. An opt-in, quiet native notification closes that loop without expanding the app into a scheduler, runtime, or telemetry product.

## What Changes

- Add an off-by-default persisted drift-notification preference and request OS permission only from that explicit user action.
- Reuse the existing bounded local Agent and Skill reconciliation every 15 minutes only while the renderer is backgrounded.
- Establish an initial baseline, then emit one native notification only for newly actionable tracked drift after a completely successful scan.
- Route notification activation to the existing Agent attention lens or Skills workspace.
- Add no network request, filesystem mutation, telemetry, notification history, arbitrary scheduling, or new top-level route.

## Capabilities

### New Capabilities

- `drift-notifications`: Opt-in permission, quiet background detection, deduplication, truthful summaries, and existing-surface navigation for local Agent and Skill drift.

### Modified Capabilities

None.

## Impact

- Extends the existing persisted settings DTO and Network settings section.
- Extends the root renderer lifecycle that already owns foreground reconciliation.
- Registers Tauri's official notification plugin with only the permissions needed to check/request permission, send a notification, and receive its activation callback.
- Adds the official JavaScript and Rust notification plugin packages; no other dependency or authority is introduced.
