## Why

Installation state can change while Agency Agents is in the background, but the app currently refreshes Agent installs only at startup and Skill installs only after opening the Skills workspace. Returning to the app must refresh both local ledgers quietly so later repair and search features build on current, truthful state.

## What Changes

- Reconcile Agent and Skill installation state after a short debounce when the application returns to the foreground.
- Coalesce rapid foreground signals and reuse each store's existing in-flight reconciliation guard so focus and mount work do not create duplicate scans or UI churn.
- Keep foreground reconciliation strictly local and read-only: no catalog/source refresh, network request, or managed-content mutation.
- Preserve the last-known Agent and Skill installation rows when a scan fails, while retaining the existing visible error and retry behavior.
- Keep existing view-mount reconciliation compatible while the root adds foreground ownership for both ledgers.

## Capabilities

### New Capabilities

- `foreground-reconciliation`: Debounced, coalesced, read-only foreground refresh of Agent and Skill installation truth with retained stale data and retryable failures.

### Modified Capabilities

None.

## Impact

- Root lifecycle and browser event handling in `src/routes/+layout.svelte`.
- Existing Agent reconciliation in `src/lib/stores/install.svelte.ts` and Skill reconciliation in `src/lib/stores/skillSources.svelte.ts` are reused; their public contracts remain compatible.
- Existing view-mount reconciliation remains compatible and shares the stores' in-flight work with root requests.
- Focus behavior remains local Tauri IPC only. No dependency, catalog API, source-refresh, persistence-format, or managed-file change is introduced.
