## Why

Agency Agents has reliable domain-specific installers, approvals, history, and health checks, but users still have to assemble project readiness, catalog awareness, recovery, and policy state across separate surfaces. This change adds one bounded local control plane and completes the requested install-target coverage without introducing runtime execution, telemetry, silent mutation, or a second approval authority.

## What Changes

- Add a durable bounded catalog change feed with deterministic add, update, remove, and unambiguous rename events.
- Add exact source-aware project readiness baselines, opt-in subscriptions, and local recommendations that always return to existing reviewed plans.
- Aggregate all existing approval shapes into Activity Review while preserving each domain's authority and keeping recommendations separate.
- Aggregate existing Agent, Skill, and storage recovery actions while keeping database restore explicitly offline/manual.
- Add a bounded read-only Playbook library over approved catalog roots with safe plain-text display.
- Add atomic Strict and Local Development security posture presets; every non-exact policy remains Custom.
- Add complete multi-artifact lifecycle truth for Kimi and OpenClaw, aggregate project roster lifecycle for Aider and Windsurf, and unchanged-flow parity proof for Antigravity.
- Preserve local-first, explicit-review, exact-path, no-link/reparse, durable-journal, rollback, and bounded-data guarantees across every new read and write path.

## Capabilities

### New Capabilities

- `catalog-change-feed`: Durable bounded active-catalog snapshots, deterministic typed changes, stale retention, and refresh evidence.
- `project-readiness`: Exact project baselines and explainable readiness derived from independent Agent, Skill, instruction, MCP, and tool evidence.
- `catalog-subscriptions`: Explicit project subscriptions, durable cursors, bounded recommendations, dismissal/supersession, and reviewed-plan handoff.
- `unified-review`: One partial-failure-tolerant Activity review projection over existing Agent, Skill, and Expert approval authorities.
- `recovery-center`: One read-only aggregation of existing exact rollback, backup, reveal, and offline database recovery guidance.
- `playbook-library`: Bounded path-contained catalog playbook discovery, local search, safe reads, and plain-text presentation.
- `security-posture-presets`: Atomic complete-policy classification, preview, and application for Strict, Local Development, and Custom.
- `agent-install-targets`: Exact multi-artifact, aggregate roster, and existing Antigravity lifecycle contracts for Kimi, OpenClaw, Aider, Windsurf, and Antigravity.

### Modified Capabilities

None.

## Impact

- Extends the existing SQLite control-center document, corpus refresh path, project registry/readiness commands, settings authority, install ledgers/journals/history, Activity, Projects, Runbooks, Settings, and existing plan/apply UI.
- Adds no production dependency, route, cloud service, telemetry path, Agent runtime, arbitrary shell boundary, new approval engine, or hot database restore.
- Existing single-artifact tools and Antigravity behavior remain backward compatible; roster targets are project-scoped aggregate artifacts and are never represented as one per-Agent install.
