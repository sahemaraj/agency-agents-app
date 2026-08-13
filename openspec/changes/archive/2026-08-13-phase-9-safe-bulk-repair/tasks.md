## 1. Candidate Truth and Selection

- [x] 1.1 Add failing focused frontend checks for exact Agent/Skill candidate keys, default eligibility, state-specific exclusion reasons, and unavailable-ledger gating.
- [x] 1.2 Extend `UpdatesModal.svelte` to derive combined exact candidates from both reconciled ledgers, select only tracked outdated/missing rows, and show all unsafe rows as non-selectable manual review.
- [x] 1.3 Verify the focused candidate checks pass without adding a helper module unless the existing test setup requires one.

## 2. Complete Review and Approval Binding

- [x] 2.1 Add failing checks for combined Agent/Skill plan presentation, blocker handling, and stable reviewed-plan invalidation.
- [x] 2.2 Add the review step using existing Agent update plans, Skill install plans, exact destinations, warnings, blockers, rollback availability, and the existing Agent diff modal.
- [x] 2.3 Add fresh dual-ledger reconciliation and normalized plan comparison so any changed eligibility or plan returns to review before the first mutation.
- [x] 2.4 Verify review remains read-only, approval is disabled for incomplete or blocked plans, and the focused review checks pass.

## 3. Safe Execution and Results

- [x] 3.1 Add failing checks that one explicit approval runs exact repairs sequentially, continues after individual failure, retains every terminal outcome, and produces bounded summary counts.
- [x] 3.2 Execute Agents through `install.updateReference(...)` and Skills through `skillSources.lifecycle("update", ...)`, preserving existing per-item backups, reconciliation, and Activity logging.
- [x] 3.3 Add the terminal results view, one aggregate Activity summary, and a final reconciliation of both ledgers; verify the focused execution checks pass.

## 4. Existing Entry Points and Accessibility

- [x] 4.1 Update the Agents landing action and sidebar badge to count eligible repairs across both ledgers.
- [x] 4.2 Extend the existing `agentUpdates.*` locale catalog for selection, review, unsafe reasons, progress, stale review, and results while preserving keyboard focus and accessible labels.
- [x] 4.3 Add or update focused checks for combined counts, complete locale keys, modal actions, focus behavior, and live progress announcements.

## 5. Verification and Review

- [x] 5.1 Run Svelte diagnostics, focused frontend checks, the full frontend suite, production build, and Rust test suite; fix only Phase 9 regressions.
- [x] 5.2 Run strict OpenSpec validation and verify the implementation diff changes no backend command, dependency, persistence format, network behavior, or unsafe-state mutation path.
- [x] 5.3 Present the implementation diff and evidence for human approval before archive or integration.
