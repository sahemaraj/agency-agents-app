## 1. Shared Recommendation Contract

- [x] 1.1 Add failing Rust checks for bounded task input, installable-only filtering, preserved Agent/Skill ranking semantics, stable cross-kind ordering, exact duplicate-name identity, and read-only local execution.
- [x] 1.2 Extract the existing Agent and Skill rankers into reusable domain functions and add one typed combined recommendation result without changing MCP response behavior.
- [x] 1.3 Register one bounded read-only Tauri recommendation command and typed frontend API wrapper; verify focused Rust checks pass with no new dependency, persistence, network, or mutation path.

## 2. Exact Workspace Handoff

- [x] 2.1 Add failing frontend checks for exact Agent and Skill recommendation activation, navigation restoration, duplicate display names, and unavailable references.
- [x] 2.2 Reuse Agent deep-link navigation and extend the existing UI navigation state with exact Skill selection consumed by `SkillsWorkspace.svelte`.
- [x] 2.3 Verify activating recommendations opens the exact existing detail workflow and never invokes an install, update, execution, or approval command.

## 3. Unified Command Palette Experience

- [x] 3.1 Add failing palette checks for mixed command/Agent/Skill grouping, minimum query threshold, debounce, stale-response rejection, loading, error, empty, reason, provenance, and keyboard behavior.
- [x] 3.2 Extend `CommandPalette.svelte` to request and render bounded combined recommendations while preserving synchronous existing command filtering and operability during recommendation failures.
- [x] 3.3 Translate structured match reasons into human-readable English-baseline locale messages, preserve fallback localization, and add accessible live announcements, labels, result counts, and input-bound feedback.
- [x] 3.4 Verify focused palette checks pass, including Escape, arrows, Enter, pointer activation, focus restoration, reduced motion, and no stale asynchronous result replacement.

## 4. Verification and Review

- [x] 4.1 Run Rust formatting and focused/full backend tests, Svelte diagnostics, focused/full frontend tests, and the production build; fix only Phase 10 regressions.
- [x] 4.2 Run strict OpenSpec validation and audit the diff for no dependency, persistence, network, direct mutation, execution, or parallel search-surface additions.
- [x] 4.3 Present the implementation diff and evidence for human approval before archive, integration, or documentation.
