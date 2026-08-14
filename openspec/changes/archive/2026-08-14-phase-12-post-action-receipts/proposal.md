## Why

Bulk Agent and Skill actions currently leave a count summary and, in some paths, failure rows, but users cannot later verify every destination that changed or jump from the transient completion message to the durable record. Post-action receipts close that audit gap before multi-asset Workspace Packs increase the size and consequence of mutations.

## What Changes

- Extend the existing local Activity journal with a bounded structured receipt for each completed bulk mutation, including every attempted Agent or Skill item, every destination actually changed, and each terminal outcome.
- Show receipt destinations and bounded failure detail inside the existing Activity surface without adding another history store or workspace.
- Add a localized `View Activity` action to bulk-completion toasts and the safe-repair results surface; the action opens and focuses the exact durable receipt.
- Keep receipt creation post-action only: it does not change planning, approval, reconciliation, rollback, or mutation behavior.
- Preserve local-first privacy and storage bounds; no telemetry, cloud audit, new backend database, notification, or route is added.

## Capabilities

### New Capabilities

- `post-action-receipts`: Durable, bounded, destination-exact receipts and deep links for completed bulk Agent and Skill mutations.

### Modified Capabilities

- `safe-bulk-repair`: Require the existing combined repair workflow to retain its exact per-destination terminal outcomes in one focusable Activity receipt.

## Impact

- Frontend Activity journal contract and persisted localStorage shape, retaining backward compatibility with existing v2 entries.
- Existing Activity history rendering and UI navigation state for receipt focus.
- Existing Agent bulk mutation helper/callers, safe-repair completion flow, localized toast action, and focused frontend tests.
- No backend command, mutation semantics, dependency, route, network, Keychain, or telemetry change.
