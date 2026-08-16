# 260816_phase18-ollama-system-prompt-deployment

## Objective

Let a user turn one exact installable Agent prompt into an explicitly reviewed, device-local Ollama model without model download, inference, daemon management, remote authority, file-tool coupling, or MCP mutation.

## Outcome

- Added bounded fixed-loopback inventory for already-installed non-cloud Ollama base models.
- Added deterministic app-owned target names and immutable create, update, and remove plans with exact prompt preview, warnings, blockers, rollback availability, and mutation-relevant revisions.
- Preserved prompt characters through Ollama's JSON create API, including literal triple quotes and template-looking syntax.
- Added separate validated deployment persistence and current, outdated, modified, missing, and source-unavailable reconciliation.
- Added backup-first update/removal, first-create cleanup, state-commit rollback, and deterministic recovery-model cleanup before a later explicit mutation.
- Added a focused Agent-detail modal with stale-truth guards, explicit confirmation, semantic errors, accessible focus behavior, localization fallbacks, and prompt-free Activity receipts.
- Kept file-tool installation and every MCP surface unchanged.

## Verification

- OpenSpec: 13/13 tasks complete; strict change validation passed; canonical specs validate 12/12 after sync; archived at `openspec/changes/archive/2026-08-16-phase-18-ollama-system-prompt-deployment/`.
- Frontend: 116/116 tests passed; Svelte diagnostics reported 0 errors and 0 warnings; production build passed.
- Backend: 580/580 Rust library tests passed with 3 existing environment-gated ignores; 2/2 binary tests passed.
- Rust quality: formatting and strict Clippy passed with warnings denied.
- Dependency audit: `npm audit` reported 0 vulnerabilities after the compatible lockfile-only `nanoid` 3.3.18 update; RustSec reported no vulnerable crates and retained existing transitive maintenance/unsoundness warnings.
- Live Ollama: a unique target based on `qwen2.5-coder:14b` round-tripped the exact 98-byte prompt containing triple quotes, template syntax, newlines, and Unicode; show/reconciliation inputs matched, deletion succeeded, the target was absent afterward, and recovery-model count stayed 0.
- Security: fixed loopback only; bounded prompts/responses/state; redirects disabled; no shell, pull, push, inference, remote endpoint, unmanaged adoption, stale-plan apply, prompt-bearing receipt, or MCP Ollama authority.
- Integration: external feature research marks Phase 18 complete; unrelated main-worktree changes retained their exact tracked diff and `.omc`/`.prefs` content hashes. Native Windows/Linux and artificial 375px evidence remain unavailable under the approved manual-platform waiver.

## Integration Points

- `src-tauri/src/ollama.rs` owns bounded runtime inventory, planning, reconciliation, transactional apply, and rollback.
- `src-tauri/src/state.rs` registers `ollama_deployments` with the existing SQLite/legacy migration inventory.
- `src/lib/components/OllamaDeployModal.svelte` adds the reviewed lifecycle to the existing Agent detail surface.
- `src/lib/api.ts` and `src/lib/types.ts` define the typed Tauri contract.
- `openspec/specs/ollama-system-prompt-deployment/spec.md` is the canonical capability contract.

## Architectural Decisions

- Ollama remains outside the file-tool registry because model manifests have device scope and no filesystem destination contract.
- The exact Agent body is transported as JSON system-prompt data rather than interpolated into a Modelfile.
- Recovery reuses Ollama copy/delete plus atomic deployment-state persistence; passive reconciliation never repairs or deletes runtime state.

## Artifacts

- Implementation commit: `722e579`
- Branch: `feat/phase-18-ollama-system-prompt`
- OpenSpec archive: `openspec/changes/archive/2026-08-16-phase-18-ollama-system-prompt-deployment/`
