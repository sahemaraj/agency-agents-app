## 1. Bounded Receipt Contract

- [x] 1.1 Add failing Activity-store tests for additive v2 hydration, one normalized structured receipt, exact aggregate counts, destination bounds, secret/control-character redaction, generated receipt ID return, and retention compatibility.
- [x] 1.2 Extend the existing journal entry with the minimum closed receipt/item types and normalize them at `activity.log` before local persistence.
- [x] 1.3 Verify old entries without receipts remain unchanged and malformed or oversized receipt data cannot escape the journal boundary.

## 2. Exact Activity Navigation

- [x] 2.1 Add failing frontend checks for accessible native receipt disclosure, textual outcomes, exact transient receipt targeting, focus/scroll handoff, and a retained-receipt-missing announcement.
- [x] 2.2 Extend the existing UI navigation state and Activity history row to open, reveal, and focus an exact receipt without adding a route, modal, or receipt store.
- [x] 2.3 Add only the English baseline receipt labels so existing locale fallback supplies all other locales, then verify keyboard and assistive-technology semantics.

## 3. Agent Bulk Receipts

- [x] 3.1 Add failing tests that require one Agent bulk receipt with every attempted item, exact changed or known planned destination, and terminal outcome across install, update, track, uninstall, planned batch, and collection application, including partial failure and a failed fresh install with no returned destination.
- [x] 3.2 Reuse returned install records, existing plans, and pre-action reconciled rows to collect exact terminal facts, explicitly retain null when a failed fresh install returned no destination, return the receipt ID with existing counts, and preserve mutation/reconciliation behavior.
- [x] 3.3 Add the existing toast action to Agent bulk completion call sites so `View Activity` targets the returned receipt; verify no duplicate journal rows or mutation calls.

## 4. Combined Safe-Repair Receipt

- [x] 4.1 Add failing tests for one mixed Agent/Skill receipt with exact destinations, matching counts, bounded failure detail, continued execution, and retained-result navigation.
- [x] 4.2 Replace the safe-repair aggregate-only journal write with one structured receipt built from its existing terminal results and add `View Activity` to the results surface.
- [x] 4.3 Verify receipt creation remains post-action, does not alter approval/preflight/reconcile flow, and handles journal persistence failure without retrying mutation.

## 5. Verification and Integration

- [x] 5.1 Run focused/full frontend tests, Svelte diagnostics, production build, Rust formatting, strict Clippy, focused/full backend tests, and diff checks; fix only Phase 12 regressions.
- [x] 5.2 Run strict OpenSpec validation and audit the diff for no dependency, backend audit table, network, telemetry, notification, new route/modal, or mutation-authority expansion.
- [ ] 5.3 Sync and archive the OpenSpec change, update the existing Phase records, commit the branch, merge locally while preserving user-owned main changes, and re-run integration smoke gates.
