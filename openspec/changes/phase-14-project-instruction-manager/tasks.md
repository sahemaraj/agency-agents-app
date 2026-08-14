## 1. Bounded Instruction Contract

- [x] 1.1 Add failing Rust tests for the four-target allowlist, exact registered-project validation, link/special/oversize/UTF-8 blockers, passive inspection, and deterministic classification.
- [x] 1.2 Implement bounded target inspection and strict versioned ownership-marker parsing in the existing project/install module.
- [x] 1.3 Add failing tests and implement create, replace, remove, adoption, duplicate/malformed marker, count/string, credential, injection, and exact unowned-byte preservation behavior.

## 2. Review and Revision Boundary

- [x] 2.1 Add failing Rust tests for complete current/proposed plans, deterministic revisions, no-op blockers, and zero-write planning.
- [x] 2.2 Implement exact plan creation and fresh apply-time replanning for one supported target.
- [x] 2.3 Prove registration, target bytes, path identity, or request drift causes zero project and backup writes and returns refreshed evidence.

## 3. Backup, Apply, and Recovery

- [x] 3.1 Add failing Rust tests for verified existing-file backup, atomic create/replace/remove, write failure rollback, retained recovery errors, and idempotent prepared/applied recovery.
- [x] 3.2 Reuse the existing private backup directory, atomic writer, and filesystem-operation journal for `project_instruction_apply`.
- [x] 3.3 Connect startup recovery, allowlist the operation kind, and prove unrelated project bytes remain untouched.

## 4. Existing Projects Workflow

- [x] 4.1 Add failing frontend tests for inspect, compose, adopt, replace, remove, exact diff, blockers, explicit approval, progress, retained result, announcements, and focus restoration.
- [x] 4.2 Extend existing types and projects store with inspect, plan, and apply commands plus one existing-format redacted local Activity entry.
- [x] 4.3 Extend the existing Projects detail surface with an inline instruction manager and reuse the current diff utility without a route or standalone editor component.

## 5. Verification and Integration

- [x] 5.1 Run focused and full Rust/frontend tests, crash-recovery checks, strict Clippy, formatting, Svelte diagnostics, production build, and diff checks; fix only Phase 14 regressions.
- [x] 5.2 Audit for no arbitrary paths, links, secret retention, network, execution, dependency, MCP mutation, telemetry, notification, new route, or unrelated state document.
- [ ] 5.3 Sync and archive OpenSpec, update Phase records and the canonical roadmap, commit and merge locally while preserving user-owned main changes, and rerun integration gates.
