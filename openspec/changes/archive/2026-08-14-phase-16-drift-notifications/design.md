## Context

The root layout already owns a coalesced, debounced local reconcile for both install ledgers, the settings backend already persists bounded forward-compatible booleans, and the navigation store already opens Agent and Skill review surfaces. Native notification support is the only missing platform primitive. See `proposal.md` and `specs/drift-notifications/spec.md`.

## Goals / Non-Goals

**Goals:**

- Extend the existing settings, reconcile, and navigation paths.
- Keep the check local, background-only, deduplicated, and failure-safe.
- Keep permission prompting contextual to the opt-in toggle.

**Non-Goals:**

- A daemon, launch agent, closed-app scheduler, notification history, configurable interval, cloud sync, or automatic repair.
- Notifications for foreign, disabled, or source-unavailable rows; those states are not newly drifted managed bytes.

## Decisions

### Use the official Tauri notification plugin with least privilege

Register the official JavaScript/Rust plugin and grant only permission checking, permission requesting, notification sending, and activation-listener registration. The browser API alone is not the project's supported cross-platform Tauri contract; a custom native bridge would be more code and platform risk.

### Keep orchestration in the existing root layout

The layout already owns lifecycle cleanup, project-path selection, and both reconcile calls. One 15-minute interval is enough; it checks the persisted opt-in and renderer visibility before doing work. A backend scheduler or new store would duplicate state and introduce unnecessary lifetime coordination.

### Compare complete in-memory snapshots

Represent each actionable row by kind plus exact logical identity, retain only the last complete Agent-and-Skill snapshot, and notify for set difference. Do not advance the baseline after partial failure. This naturally suppresses repeats and permits a resolved item to notify if it later drifts again.

### Navigate without repair

The notification activation callback uses the existing UI store. Agent-inclusive notifications open the Agent attention lens; Skill-only notifications open Skills. No mutation plan is prepared or applied.

## Risks / Trade-offs

- [The renderer cannot reconcile after the app is fully closed] → This phase intentionally covers only a running backgrounded app; add a native service only after measured demand.
- [Platform permission may be denied or revoked] → Keep the preference false when enablement fails and re-check permission before every send.
- [A background timer may be throttled] → Accept best-effort timing; the next focus reconcile still refreshes visible truth.
- [Mixed Agent and Skill drift has one activation target] → Prefer the Agent attention lens when any Agent drift exists and include both counts in the notification body.

## Migration Plan

The new persisted boolean defaults false for existing and fresh settings files. Rollback removes plugin registration and UI orchestration; older binaries ignore the extra settings key.
