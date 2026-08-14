# Tasks — 2026-08

## Completed

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
