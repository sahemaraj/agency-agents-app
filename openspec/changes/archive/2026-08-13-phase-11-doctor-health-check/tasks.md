## 1. Canonical Doctor Contract

- [x] 1.1 Add failing Rust tests for deterministic classification and ordering, partial authority failure, overall severity, bounded fields/report size, secret and authenticated-URL redaction, home-prefix normalization, and safe-action typing.
- [x] 1.2 Add the minimal typed Doctor report, check, classification, category, and closed safe-action DTOs plus pure report formatting/redaction helpers.
- [x] 1.3 Verify the pure report checks pass and serialized output contains no raw secret, credential, control-character, or private home-prefix content.

## 2. Read-Only Local Evidence Composition

- [x] 2.1 Add failing backend checks that cover storage, settings, catalog, Agent sources, Skill sources, Agent and Skill installation truth, tool detection, MCP clients, and cached update state while proving one failing authority does not discard other checks.
- [x] 2.2 Reuse or minimally expose existing pure local inspection functions and implement one `doctor_report` command with independent failure capture and no network, Keychain prompt, persistence, reconciliation, install, execution, or approval path.
- [x] 2.3 Register the command and typed frontend wrapper; verify focused and compatibility tests, including optional-integration classification and no changes to existing command behavior.

## 3. Existing-Surface Action Handoff

- [x] 3.1 Add failing frontend checks for every closed Doctor action, exact Settings subsection or workspace navigation, unavailable/manual-only guidance, and zero mutation-command invocation.
- [x] 3.2 Extend the existing Settings-section navigation state and map Doctor actions to existing Catalog, Network, MCP, Tools, Agents, and Skills recovery surfaces without implementing repair logic.
- [x] 3.3 Verify action activation closes or preserves Settings as appropriate, restores keyboard focus, and never performs the diagnosed mutation automatically.

## 4. Doctor Settings Experience

- [x] 4.1 Add failing component checks for initial loading, refresh with stale prior evidence, non-overlapping requests, global retryable failure, grouped results, summary counts, deterministic Copy Report, copy failure, accessible announcements, and keyboard access.
- [x] 4.2 Extend the existing Settings modal with one localized Doctor section that renders the canonical report, Refresh and Copy Report controls, fixed category groups, evidence, limitations, and safe actions.
- [x] 4.3 Verify all focused Doctor UI checks pass, including partial reports, no false all-healthy state, clipboard parity with backend copy text, reduced motion, and locale fallback.

## 5. Verification and Review

- [x] 5.1 Run Rust formatting, strict Clippy, focused/full backend tests, Svelte diagnostics, focused/full frontend tests, production build, and diff checks; fix only Phase 11 regressions.
- [x] 5.2 Run strict OpenSpec validation and audit the call graph/diff for no dependency, persistence, report history, network, credential prompt, telemetry, scheduled work, notification, direct mutation, or new top-level workspace.
- [x] 5.3 Present the implementation diff, report coverage matrix, and fresh verification evidence for human approval before archive, integration, or documentation.
