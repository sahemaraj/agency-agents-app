# Tasks — 2026-08

## In Progress

### 2026-08-18: Factory Runs MVP

[COMPLETE] Implemented the approved `factory-runs-mvp` OpenSpec change on isolated branch
`feat/factory-runs-mvp`. Core lifecycle, shared readiness/activation, the seven-tool pull MCP
protocol, desktop control room, Activity review, and bounded terminal receipts are implemented.
V10 review found two Medium raw-source classifier gaps. Exact backend/Activity regressions failed
first and now pass after shared case-sensitive import, assignment, and semicolon-free SQL/DDL
repairs. Full QA and fresh v11 code/security/goal audits remain pending; progress is 27/29.
V11 found Python alias, SQL table-alias/modifier, broader DDL, uppercase RHS, safe SELECT prose, and
camel-case credential gaps. Exact regressions and root fixes now pass the full Rust/frontend,
strict lint/build, dependency, OpenSpec, and diff gates. V12 independent audits are running.
V12 security review found one final Medium class covering bare-relative/trailing-comment Python
imports and semicolon-free literal SELECT/CTE SQL. Exact backend/Activity tests failed first and
the shared-classifier repair now passes the focused and full suites. Fresh QA passes 843 executable
Rust library tests plus 5 intentional manual-only ignores, 2 binary tests, strict format/Clippy,
199 frontend tests, zero Svelte diagnostics, production build, npm audit with zero vulnerabilities,
14 Factory MCP protocol checks, exact 137-tool/seven-Factory inventory, OpenSpec 21/21, and diff
checks. Frozen v13 independent audits remain before tasks 5.6/5.7 can close; progress is 27/29.
V13 security passed at 0 C/H/M and code review found no lifecycle defect, but reported three Medium
metadata classes: constructor assignments, materialized/replaced views plus frontend SELECT
aliases/TOP counts, and terse sentence-case Select/Delete prose false positives. Exact regressions
failed first and shared root fixes now pass the focused/full Rust and frontend suites plus every
strict quality, dependency, MCP inventory/protocol, OpenSpec 21/21, and diff gate. Frozen v14 audits
remain before progress can advance beyond 27/29.
V14 goal review caught one Medium privacy regression in the broad sentence-case prose exception:
valid `Select email from users` / `Delete from users` SQL could pass. Exact red tests now prove the
gap closed while the two reviewed ambiguous UI phrases remain safe. Fresh full Rust/frontend,
strict quality, dependency, OpenSpec 21/21, and diff gates pass. Frozen v15 audits remain; progress
is still 27/29.
V15 code review found three adjacent Medium syntax classes: generic constructor assignments,
materialized-view alter/drop/refresh, and Activity SELECT parity for `TOP (n)` plus quoted aliases.
Exact regressions failed first and shared root fixes now pass full Rust/frontend, strict quality,
dependency, OpenSpec 21/21, and diff gates. Frozen v16 audits remain; progress is 27/29.
V16 security/goal review found two Medium residual raw-source classes: implicit SELECT projection
aliases and uppercase scalar/arrow assignments. Exact regressions failed first; complete table
reference parsing, mirrored implicit aliases, and fail-closed assignment handling now pass full
Rust/frontend, strict quality, dependency, OpenSpec 21/21, and diff gates. Frozen v17 audits remain;
progress is 27/29.
Frozen V25 goal review passed, while code/security review found one High API/webhook bearer-path
gap and Medium Activity persistence/encoding, SQL-family/case, and decorator gaps. Paired
regressions and shared root repair are in progress; progress remains 27/29.
V25 findings now pass paired Rust/Activity regressions after shared durable-Activity, decoded URL
credential, known bearer-path, SQL-family/case, and generic decorator repairs. V26 full QA and
frozen audits remain; progress is 27/29.
Latest review findings now have focused regressions and root fixes for generic metadata privacy,
terminal audit serialization/idempotency, ordinary-setup-only Factory activation, and recovery
payload rejection before any repository write. Fresh QA passes 842 executable Rust library tests
with 5 intentional manual-only ignores, 2 binary tests, strict Clippy/format, 199 frontend tests,
Svelte/build, npm audit, browser keyboard/axe E2E at 375/1440, 13 Factory MCP protocol checks,
exact 137-tool inventory with seven Factory tools, strict OpenSpec 21/21, and diff/boundary scans.
The fresh security audit found one remaining Medium syntax-variant privacy bypass and the goal audit
found an adjacent quality-check metadata boundary. Exact regressions and shared-validator fixes now
pass for newline-split credentials, no-space source statements, triple-slash private URL paths, and
Factory quality-check names/kinds. Fresh full Rust QA again passes 842 executable library tests with
5 intentional manual-only ignores, 2 binary tests, strict Clippy/format, and diff checks. The
goal-backward audit is clean at 0 C/H/M with 21/21 requirements and 47/47 scenarios proven. The
narrow code recheck found two remaining Medium gaps in Factory read policy-lease serialization and
clear Python/SQL/serialized metadata detection. Exact regressions now prove both failures and the
shared root fixes pass 50/50 focused Factory tests, 843 executable Rust library tests with 5
intentional manual-only ignores plus 2 binary tests, strict format/Clippy, 199 frontend tests,
Svelte/build, and npm audit. Security/OpenSpec/diff gates pass, but the fresh code audit found one
remaining Medium structured metadata class spanning whole-value JSON/config/SQL and Python typed,
tuple, async, and control forms. Exact backend and Activity fallback regressions now cover every
reported variant plus safe multiline prose controls. Full QA again passes 843 executable Rust
library tests with 5 intentional manual-only ignores, 2 binary tests, 199 frontend tests, strict
format/Clippy/Svelte/build, npm audit, OpenSpec 21/21, and diff checks. Fresh v8 code, security, and
goal-backward audits found one Medium overcorrection: SQL detection rejects ordinary imperative
product prose such as “Select a project from the list.” Exact backend/Activity safe controls are in
progress before tightening the shared detector and rerunning the full gates.
V17 independent review then found seven Medium raw-source syntax families spanning capitalized YAML
bare scalars, trailing SELECT operators, parenthesized destructuring, complex SELECT sources,
temporary/unlogged/materialized DDL, compound assignments, and C/C++/XML directives. Paired red
tests and shared classifier fixes now cover each family. Fresh verification passes 843 executable
Rust library tests plus 5 intentional manual-only ignores, 2 binary tests, strict format/Clippy,
199 frontend tests, zero Svelte diagnostics, production build, npm audit with zero vulnerabilities,
14/14 Factory MCP protocol checks, the exact 137-tool/seven-Factory inventory, strict OpenSpec
21/21, and diff checks. Frozen v18 independent audits remain; progress is 27/29.
V18 review found one High credential-bearing URL alias gap plus Medium backend/Activity parity gaps
for concurrent/combined DDL, multiword YAML, directives/imports, XML constructs, adjacent SQL, and
destructuring defaults. Paired red tests now prove the failures, and the shared classifier repairs
pass fresh full Rust/frontend, strict quality/build, dependency, 14/14 MCP, 137/seven inventory,
OpenSpec 21/21, and diff gates. Frozen v19 audits remain; progress is 27/29.
V19 review found a High prefixed URL-credential suffix mismatch and Medium structured YAML,
SQL/transaction/DDL modifier, and directive parity gaps. Paired regressions now pass after matching
backend/Activity credential suffixes, replacing broad colon detection with structural YAML signals,
and extending the existing SQL/directive classifiers. Fresh full Rust/frontend, strict quality,
dependency, MCP inventory/protocol, OpenSpec 21/21, and diff gates pass. Frozen v20 audits remain;
progress is 27/29.
V20 review found one High URL-signature/password-alias gap and Medium YAML, transaction, combined
view-modifier, adjacent import, and EXPLAIN safe-prose classifier defects. Paired backend/Activity
regressions and root fixes now pass for credential aliases, capital/block YAML, transaction and
view variants, adjacent imports, and EXPLAIN prose controls. Full QA and fresh frozen audits
remain; progress is 27/29.
Fresh V31 QA passes the complete Rust/frontend/strict-quality/dependency/MCP/OpenSpec/diff matrix:
844 library tests plus 5 intentional ignores, 2 binary tests, 203 frontend tests, strict format and
Clippy, zero Svelte diagnostics, production build, npm audit zero, and strict OpenSpec 21/21.
Frozen audits remain; progress is 27/29.
Fresh V21 QA passes 843 Rust library tests with 5 intentional ignores, 2 binary tests, 0 doctests,
strict format/Clippy, 199 frontend tests, zero Svelte diagnostics, production build, npm audit with
zero vulnerabilities, 14/14 Factory MCP protocol tests, exact 137/seven inventory, raw-router
boundary smoke, strict OpenSpec 21/21, and diff checks. Frozen V21 audits remain; progress is 27/29.
V21 goal-backward review passed, while code/security review found one High JWT credential gap and
Medium SQL, YAML, and source-directive variants plus Low safe-prose controls. Paired regressions and
shared-classifier repair are in progress; progress remains 27/29.
V21 findings now have paired failing-first regressions and shared classifier repairs for JWT
key/value credentials, SQL/YAML/directive variants, and the reviewed safe-prose controls. V22 full
QA and frozen audits remain; progress is 27/29.
Fresh V22 QA passes the complete Rust/frontend/strict-quality/MCP/dependency/OpenSpec/diff matrix;
retained 375/1440 keyboard/axe evidence remains applicable to the classifier-only repair. Frozen
V22 audits remain; progress is 27/29.
V22 code/security review found one High SAS signature credential gap and Medium compact-JWT,
SQL-terminator, YAML, EXPLAIN, generic redaction, and directive variants plus one safe-prose control.
Paired regressions and shared repair are in progress; progress remains 27/29.
V22 findings now pass paired backend/Activity regressions after shared fixes for credentials,
generic redaction, SQL terminators, YAML grammar, EXPLAIN modifiers, directives, and safe prose.
Fresh V23 full QA passes 843 Rust library tests with 5 intentional ignores, 2 binary tests, 0
doctests, strict format/Clippy, 199 frontend tests, zero Svelte diagnostics, production build, npm
audit with zero vulnerabilities, 14/14 Factory MCP protocol tests, exact 137/seven inventory,
raw-router boundary smoke, strict OpenSpec 21/21, and diff checks. Frozen V23 independent audits
remain; progress is 27/29.
V23 goal review proved 21/21 requirements and 47/47 scenarios, while code/security review found one
High standard-session credential gap and Medium compact-token, frontend parity, YAML, SQL/source,
and legacy JSON validation gaps. Paired regressions and shared root repair are in progress;
progress remains 27/29.
V23 findings now pass paired backend/Activity regressions after shared repairs for session and
compact-token credentials, frontend key parity, quoted YAML, SQL/source grammar, legacy JSON
validation, and safe Severity prose. V24 full QA and frozen audits remain; progress is 27/29.
Fresh V24 full QA passes 844 Rust library tests with 5 intentional ignores, 2 binary tests, 0
doctests, strict format/Clippy, 199 frontend tests, zero Svelte diagnostics, production build, npm
audit with zero vulnerabilities, 14/14 Factory MCP protocol tests, exact 137/seven inventory,
raw-router boundary smoke, strict OpenSpec 21/21, and diff/scope/boundary checks. Frozen V24
independent audits remain; progress is 27/29.
Frozen V26 goal review again proves all 21 requirements and 47 scenarios, but code/security review
found one High common bearer-token gap and consolidated Medium opaque-webhook, SQL/source/CSS, and
legacy Activity-field normalization gaps. Paired backend/Activity regressions and shared root
repairs now pass focused tests. Common bearer tokens, structural webhook paths, SQL/source/CSS
forms, closed Activity fields/receipts/MCP projection, and safe local project labels are covered;
fresh V27 QA passes the full Rust/frontend/strict-quality/dependency/MCP/OpenSpec/diff matrix. Prior
375/1440 keyboard/axe evidence remains applicable because Factory interaction structure is
unchanged and Activity display-string behavior has direct mount/focus coverage. Frozen V27 audits
remain; progress is 27/29.
V27 goal review passed 21/21 requirements and 47/47 scenarios. Code/security review found one High
common bearer-token gap and Medium opaque-webhook, SQL/source/CSS, generic exact-destination, and
Activity-envelope gaps. Paired failing-first regressions and shared root repair are in progress;
progress remains 27/29.
V27 findings now pass paired Rust/Activity regressions after shared token, webhook, SQL/source/CSS,
exact generic destination, strict Activity-envelope, and safe MCP projection fixes. Full QA and a
new frozen audit cycle remain; progress is 27/29.
Fresh V28 QA passes the complete Rust/frontend/strict-quality/dependency/MCP/OpenSpec/diff matrix:
844 library tests plus 5 intentional ignores, 2 binary tests, 203 frontend tests, 14 Factory MCP
protocol checks, and exact 137/seven inventory. Frozen code/security/goal audits remain; progress
is 27/29.
Frozen V28 audits found zero Critical/High findings and five Medium privacy-classifier gaps across
standalone credential families, webhook path parsing, generic SQL DDL, and inline namespaces, plus
one Low calendar-timestamp gap. A generalized shared-validator repair with paired failing-first
regressions is in progress; progress remains 27/29.
V28 findings now pass paired Rust/Activity regressions after shared credential, webhook, generic
DDL, inline-namespace, and strict timestamp repairs. Full V29 QA and frozen audits remain; progress
is 27/29.
The V29 pre-freeze self-audit found and repaired lowercase semicolon-free generic DDL parity with
paired failing-first Rust/Activity regressions. Full V30 QA and frozen audits remain; progress is
27/29.
Fresh V30 QA passes 844 Rust library tests with 5 intentional ignores, 2 binary tests, strict
format/Clippy, 203 frontend tests, zero Svelte diagnostics, production build, npm audit zero,
14/14 Factory MCP coverage, exact inventory/router boundaries, strict OpenSpec 21/21, and clean
dependency/scope/diff gates. Frozen audits remain; progress is 27/29.
Frozen V30 security/goal review found Medium structured-SendGrid-token, host-webhook aggregate,
mixed-case DDL, and exported-namespace privacy gaps. Paired shared-classifier regressions are in
progress; 5.6/5.7 remain unchecked.
V30 findings now pass paired Rust/Activity regressions after shared structured-token, aggregate
webhook-tail, mixed-case DDL, and exported/aliased-namespace repairs. Full V31 QA and frozen audits
remain; progress is 27/29.
Frozen V24 goal review proves 21/21 requirements and 47/47 scenarios, while code/security review
found Medium persisted-Activity, compact-credential, webhook-path, SQL-variant, and source-family
privacy gaps. Paired regressions and shared root repair are in progress; progress remains 27/29.
V24 findings now pass paired Rust/Activity regressions after shared repairs for persisted hydration,
compact/session credentials, known webhook bearer paths, SQL variants, source families, and safe
prose controls. V25 full QA and frozen audits remain; progress is 27/29.
Fresh V25 full QA passes 844 Rust library tests with 5 intentional ignores, 2 binary tests, strict
format/Clippy, 199 frontend tests, zero Svelte diagnostics, production build, npm audit with zero
vulnerabilities, 14/14 Factory MCP protocol tests, exact 137/seven inventory, raw-router/router
composition, strict OpenSpec 21/21, and clean dependency/scope/diff gates. Frozen V25 audits remain;
progress is 27/29.
V26 full Rust (844 plus 5 intentional ignores and 2 binary tests) and strict OpenSpec 21/21 pass.
Full frontend verification found one stale project-instruction Activity assertion expecting raw
private paths; the runtime correctly redacts them under the approved privacy contract. The fixture
is corrected. Fresh V26 QA passes 200/200 frontend tests, zero Svelte diagnostics, production
build, strict format/Clippy, npm audit with zero vulnerabilities, 14/14 Factory MCP protocol tests,
the exact inventory and router boundaries, strict OpenSpec 21/21, and diff/scope checks. Frozen V26
independent audits remain; progress is 27/29.

Final V31 verification passes 844 Rust library tests with 5 intentional ignores, 2 binary tests,
strict format and Clippy, 203 frontend tests, zero Svelte diagnostics, production build, npm audit
with zero vulnerabilities, Factory MCP inventory/protocol boundaries, strict OpenSpec 21/21, and
diff/scope checks. Independent code-quality, security, and goal-backward audits all report zero
Critical/High/Medium/Low findings against the unchanged frozen source hash. All 21 delta
requirements, 47 delta scenarios, and affected canonical receipt/review requirements are proven;
OpenSpec progress is 29/29. Commit `c96afea` was merged into local `main` by `6d4ca5f`. The two new
Factory capabilities and the receipt/review deltas are synchronised into canonical specs, strict
canonical validation passes 22/22, and the change is archived under
`openspec/changes/archive/2026-08-19-factory-runs-mvp/`.

## Completed

### 2026-08-18: Nightly Control Plane Program

Completed the approved eight-capability local control plane with exact target lifecycle coverage,
durable catalog/readiness state, existing-domain review and recovery, bounded Playbooks, atomic
security presets, independent audits, upstream parity, and browser accessibility/E2E evidence. The
isolated branch is integration-ready and the original `main` checkout remains untouched. See
[260818_nightly-control-plane.md](./260818_nightly-control-plane.md).

### 2026-08-03: Expert MCP Lifecycle — Release 1

Added the human-approved Expert MCP lifecycle for discovery, portable change proposals,
activation requests, immutable run contracts, evidence, blockers, waivers, and desktop review.
See [260803_expert-mcp-release1.md](./260803_expert-mcp-release1.md).

### 2026-08-04: Agent Foundation

Added source-aware Agent identity, built-in/local/GitHub/published sources, validated one-file
drafts, nested personal-library organization, and the first source-aware Agents workspace surfaces.
See [260804_agent-foundation.md](./260804_agent-foundation.md).

### 2026-08-04: Agent Lifecycle Parity

Added source-aware Agent install migration, seven lifecycle states, transactional
history/rollback/disable/enable, exact mutations, dependency and collection plans, and desktop
lifecycle controls. See [260804_agent-lifecycle-parity.md](./260804_agent-lifecycle-parity.md).

### 2026-08-04: Agents MCP

Added the exact 49 Agent MCP tools, Agent resources/subscriptions, separate default-denied Agent
permissions, capability-bound project mutations, typed desktop approvals, and durable redacted
audit through the existing Skills MCP server. See [260804_agents-mcp.md](./260804_agents-mcp.md).

### 2026-08-04: Agents–Skills Feature Parity

Completed the desktop workflow, Activity coverage, localization/accessibility, recovery and
security rehearsals, and the evidence-backed Skills-to-Agents parity audit. See
[260804_agents-skills-feature-parity.md](./260804_agents-skills-feature-parity.md).

### 2026-08-05: Create Agent from Skill

Added deterministic editable Skill-to-Agent drafts in the desktop app and MCP, structured
`required-skills` metadata, and hash-bound desktop approval for MCP publication requests. See
[260805_create-agent-from-skill.md](./260805_create-agent-from-skill.md).

### 2026-08-05: Skill Publishing MCP and Skills UI Fixes

Added revision-bound Skill publication requests through the existing desktop approval boundary,
published the 59-file Primavera hybrid, made Skills popovers dismiss on outside clicks, and
contained filters within the package-list column. Same-name app-owned revisions now replace with a
rollback backup, stale exact approvals reconcile, and the inbox shows one action per revision. See
[260805_skill-publishing-mcp.md](./260805_skill-publishing-mcp.md).

### 2026-08-06: SQLite Control Plane

Moved the desktop and MCP mutable control plane to a shared transactional SQLite authority with a
verified one-time migration, private backups, crash-recoverable filesystem journals, exact approval
reconciliation, and foreground revision refresh. Package artifacts and Keychain secrets remain
outside the database. See [260806_sqlite-control-plane.md](./260806_sqlite-control-plane.md).

### 2026-08-13: Phase 6 Reliability Gate

Added semantic application errors, retained and retryable Agent/Skill reconciliation truth, stale-mutation guards, and backend-authorized canonical filesystem reveal. Verification passed with zero implementation blockers; the user explicitly waived unavailable 375px geometry and native Linux/Windows evidence without treating them as green. See [260813_phase6-reliability-gate.md](./260813_phase6-reliability-gate.md).

### 2026-08-13: Phase 7 Guided First Deployment

Continued catalog setup into a deterministic, approval-gated Claude Code/Codex team deployment with exact-reference transactional rollback and reconciliation-backed success. OpenSpec, frontend, backend, build, formatting, and security-sensitive transaction gates passed; manual platform evidence remains explicitly unavailable under the approved waiver. See [260813_phase7-guided-first-deployment.md](./260813_phase7-guided-first-deployment.md).

### 2026-08-13: Phase 8 Foreground Reconciliation

Added debounced root-owned Agent and Skill foreground reconciliation, reused existing in-flight guards, retained stale data and Retry after failures, and restricted focus work to local reads. OpenSpec, frontend, backend, Svelte, build, and diff gates passed. See [260813_phase8-foreground-reconciliation.md](./260813_phase8-foreground-reconciliation.md).

### 2026-08-13: Phase 9 Safe Bulk Repair

Added one approval-bound repair workflow for exact tracked outdated and missing Agent and Skill installations, kept unsafe states in manual review, reused the existing recoverable lifecycle paths, and reported every terminal outcome. OpenSpec, frontend, backend, Svelte, build, and diff gates passed. See [260813_phase9-safe-bulk-repair.md](./260813_phase9-safe-bulk-repair.md).

### 2026-08-13: Phase 10 Unified Task Search

Added bounded, deterministic, local Agent and Skill recommendations to the existing Cmd+K palette with explanations, provenance, exact workspace handoff, async lifecycle safety, and no mutation or network path. OpenSpec, frontend, backend, Rust quality, Svelte, build, and diff gates passed. See [260813_phase10-unified-task-search.md](./260813_phase10-unified-task-search.md).

### 2026-08-13: Phase 11 Doctor Health Check

Added one bounded, read-only local health report with independent classifications, privacy-safe deterministic copying, and handoff to existing recovery surfaces. OpenSpec, frontend, backend, Rust quality, Svelte, build, mutation-spy, dependency, route, and diff gates passed. See [260813_phase11-doctor-health-check.md](./260813_phase11-doctor-health-check.md).

### 2026-08-14: Phase 12 Post-Action Receipts

Added one bounded local receipt for completed Agent bulk and mixed Agent/Skill repair operations,
including every attempted item, exact changed or known planned destination, terminal outcome,
privacy-safe failure detail, and exact Activity navigation from existing completion surfaces.
OpenSpec, frontend, backend, Rust quality, Svelte, build, safety, and diff gates passed. See
[260814_phase12-post-action-receipts.md](./260814_phase12-post-action-receipts.md).

### 2026-08-14: Phase 13 Portable Workspace Packs

Added deterministic path-private Workspace Pack export, strict legacy conversion, complete read-only
Agent/Skill planning, revision-bound recoverable apply, Teams review, and mixed Activity receipts.
OpenSpec, frontend, backend, Rust quality, Svelte, build, safety, and diff gates passed. See
[260814_phase13-portable-workspace-packs.md](./260814_phase13-portable-workspace-packs.md).

### 2026-08-14: Phase 14 Project Instruction Manager

Added bounded inspection and byte-preserving app-owned snippets for four known project instruction
files, complete deterministic diff plans, revision-bound atomic apply, verified backup and startup
recovery, plus the existing Projects review and Activity surfaces. OpenSpec, frontend, backend, Rust
quality, Svelte, build, safety, and diff gates passed. See
[260814_phase14-project-instruction-manager.md](./260814_phase14-project-instruction-manager.md).

### 2026-08-14: Phase 15 MCP Inventory Manager

Added bounded privacy-safe Claude Code/Codex MCP inventory, passive validation, exact Agency Agents tool evidence, one trusted template, isolated failures, and read-only foreign-server evidence in the existing Settings workflow. OpenSpec, frontend, backend, Rust quality, Svelte, build, safety, and diff gates passed. See [260814_phase15-mcp-inventory-manager.md](./260814_phase15-mcp-inventory-manager.md).

### 2026-08-14: Phase 16 Drift Notifications

Added explicit permission-backed native drift alerts that reuse bounded local Agent and Skill reconciliation only while the running app is backgrounded, establish a silent complete baseline, deduplicate exact managed identities, retain truth after partial failures, omit private paths, and route activation to existing review surfaces without repair. OpenSpec, frontend, backend, Rust quality, Svelte, build, dependency, safety, and diff gates passed. See [260814_phase16-drift-notifications.md](./260814_phase16-drift-notifications.md).

### 2026-08-14: Phase 17 Expert Improvement Coach

Added five-run-gated local Expert performance summaries for exact versioned quality-contract cohorts, including acceptance, rework/rejection, waiver, and latest-evidence signals plus deterministic non-causal suggestions. No model, network, telemetry, persistence, or mutation authority was added. OpenSpec, frontend, backend regression, Rust quality, Svelte, build, dependency, safety, and diff gates passed. See [260814_phase17-expert-improvement-coach.md](./260814_phase17-expert-improvement-coach.md).

### 2026-08-16: Phase 18 Local Ollama System-Prompt Deployment

Added explicit revision-bound create, update, remove, reconciliation, rollback, and prompt-free receipts for app-owned local Ollama models derived from exact installable Agent prompts and already-installed bases. Fixed-loopback, no-pull, no-inference, no-daemon, no-remote-host, and no-MCP-authority boundaries are enforced. OpenSpec, frontend, backend, Rust quality, Svelte, build, dependency, live Ollama, security, and diff gates passed. See [260816_phase18-ollama-system-prompt-deployment.md](./260816_phase18-ollama-system-prompt-deployment.md).

### 2026-08-16: Skill Review and Cleanup Classification

Removed hidden runtime-mirror packages from Skill discovery and limited Cleanup suggestions to
app-tracked unused installs. Live MCP reduced Needs review from 672 to 202 genuine items and
Cleanup to 0 without trusting, deleting, or modifying any Skill package. Frontend, backend,
Svelte, build, Clippy, format, and diff gates passed. See
[260816_skill-review-cleanup.md](./260816_skill-review-cleanup.md).
