## 1. Bounded Inventory Contract

- [x] 1.1 Add failing Rust tests for Claude user/project/local sources, Codex JSON list, deterministic deduplication/sorting/caps, and isolated malformed/link/oversize failures.
- [x] 1.2 Extend existing MCP DTOs and `commands/mcp_clients.rs` with one live inventory command using only exact supported sources.
- [x] 1.3 Preserve and re-prove exactly one Claude and one Codex status row with independent unavailable evidence.

## 2. Privacy and Passive Validation

- [x] 2.1 Add failing tests proving raw environment/header/token/URL/argument values never serialize and unsafe remote endpoints block.
- [x] 2.2 Implement bounded redacted endpoint/key summaries, inline-secret detection, supported-transport validation, and launcher warnings.
- [x] 2.3 Prove Claude inventory never invokes its health-checking list command and Codex invokes only literal `mcp list --json` through the existing bounded no-shell runner.

## 3. Honest Tool and Template Evidence

- [x] 3.1 Add failing tests for exact sorted unique Agency Agents tools, declared foreign filters, and explicit unavailable discovery.
- [x] 3.2 Expose tool names from the existing composed router and add Agency Agents as the sole trusted auto-configurable template.
- [x] 3.3 Reuse existing connect/repair/disconnect, per-client policy, and canonical project allowlist; add no generic foreign mutation.

## 4. Existing MCP Settings Workflow

- [x] 4.1 Add failing frontend tests for inventory loading, source/scope/project evidence, validation, redaction, known/declared/unavailable tools, partial issues, refresh, announcements, and no foreign actions.
- [x] 4.2 Extend existing frontend types/API and `SettingsSectionMcp.svelte` with inventory and trusted-template evidence.
- [x] 4.3 Keep existing policy controls and mutation focus behavior intact without a route, standalone component, or new state store.

## 5. Verification and Integration

- [x] 5.1 Run focused/full Rust/frontend tests, strict Clippy, formatting, Svelte diagnostics, production build, OpenSpec, and diff gates.
- [x] 5.2 Audit for no secret retention, arbitrary path, unsafe link, network, external-server execution, generic install/edit/remove/login, dependency, persistence, route, telemetry, notification, or unrelated mutation.
- [ ] 5.3 Sync/archive OpenSpec, update Phase records and roadmap rank 10, commit/merge locally with protected fingerprints unchanged, and rerun integration gates.
