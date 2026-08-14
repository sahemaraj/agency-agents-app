# 260814_phase16-drift-notifications

## Objective

Notify users about newly actionable local Agent and Skill drift while the running app is backgrounded, without adding automatic repair, network work, telemetry, or a standalone scheduler.

## Outcome

- Added an off-by-default persisted `driftNotifications` preference in the existing bounded settings document.
- Added a Network settings toggle that checks and requests native permission only from explicit enablement and retains disabled state after denial or error.
- Reused root-owned Agent and Skill reconciliation on one 15-minute interval only while the renderer is hidden.
- Established a complete silent baseline, compared exact logical tracked-install identities, retained the prior baseline after partial failure, and suppressed unchanged drift.
- Added one bounded path-private count notification for newly outdated, modified, or missing managed items; resolved drift can notify if it later returns.
- Routed notification activation to the existing Agent attention lens or Skills workspace without preparing or applying repair.

## Verification

- OpenSpec: 11/11 tasks complete; strict change validation passed; canonical specs validate 10/10 after sync.
- Frontend: 111/111 tests passed; Svelte diagnostics reported 0 errors and 0 warnings; production build passed.
- Backend: 570 library tests discovered; 567 passed with 3 existing environment-gated ignores; 2/2 binary tests passed.
- Rust quality: formatting and strict Clippy passed.
- Dependency audit: production npm dependency audit reported 0 vulnerabilities.
- Safety: opt-in default, contextual permission request, background-only/local-only scan, complete-baseline failure retention, exact-identity dedupe, path-private copy, least-privilege ACL, listener cleanup, no-network, no-telemetry, no-auto-repair, no-route, no-store, and diff audits passed.

## Integration Points

- `src/routes/+layout.svelte` extends the existing root reconciliation lifecycle with background eligibility, complete drift snapshots, native send, and activation routing.
- `src/lib/components/SettingsSectionNetwork.svelte` owns explicit permission-backed enablement beside existing local/network policy controls.
- `src-tauri/src/commands/settings.rs` persists the default-off preference through the existing settings authority.
- `openspec/specs/drift-notifications/spec.md` is the canonical capability contract.

## Security and Safety

- Notifications contain only bounded aggregate Agent and Skill counts; no path, source content, destination, or credential crosses the notification boundary.
- The interval performs existing local reconciliation only and skips while the app is visible, disabled, or incompletely reconciled.
- The official Tauri plugin receives only permission-check, permission-request, notify, and activation-listener capabilities.
- Activation changes UI location only; it never installs, updates, repairs, or removes content.

## Artifacts

- Implementation commit: `0f6a269`
- Branch: `feat/phase-16-drift-notifications`
- OpenSpec archive: `openspec/changes/archive/2026-08-14-phase-16-drift-notifications/`
