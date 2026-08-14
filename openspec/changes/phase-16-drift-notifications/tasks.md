## 1. Opt-in and Platform Authority

- [x] 1.1 Add failing Rust/frontend tests for a false-by-default persisted `driftNotifications` setting and bounded general-settings mutation.
- [x] 1.2 Extend the existing settings DTO/store and Network settings toggle; request OS permission only on explicit enable and retain false on denial/error.
- [x] 1.3 Register the official notification plugin with only check, request, notify, and activation-listener permissions.

## 2. Quiet Drift Detection

- [x] 2.1 Add failing frontend tests for initial-baseline silence, background-only timing, exact tracked actionable identity, deduplication, resolved-then-returned drift, and complete-scan failure retention.
- [x] 2.2 Extend the root reconcile lifecycle with one background interval and complete Agent/Skill drift snapshots; add no new runtime store or scheduler.
- [x] 2.3 Emit one bounded path-private count summary only for newly actionable tracked drift and re-check permission before sending.

## 3. Action and Lifecycle

- [x] 3.1 Add failing tests for notification activation routing Agent-inclusive drift to the attention lens and Skill-only drift to Skills without mutation.
- [x] 3.2 Register and clean up the notification action listener in the existing root lifecycle.

## 4. Verification and Integration

- [x] 4.1 Run focused/full frontend and Rust tests, strict Clippy, formatting, Svelte diagnostics, production build, and strict OpenSpec validation.
- [x] 4.2 Audit for opt-in default, prompt context, background-only/local-only checks, complete-baseline dedupe, bounded path-private copy, least-privilege ACL, no network, no telemetry, no automatic repair, no new route/store, and no unrelated user-file changes.
- [ ] 4.3 Sync and archive the canonical spec, update approved Memory Bank/roadmap evidence, merge the verified branch, and repeat post-merge smoke checks.
