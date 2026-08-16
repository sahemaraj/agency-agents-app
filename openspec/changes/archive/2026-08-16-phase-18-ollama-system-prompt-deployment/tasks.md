## 1. Contract and Runtime Core

- [x] 1.1 Add failing Rust tests for bounded deployment-record validation, deterministic collision-resistant target names, non-cloud local inventory filtering, exact state classification, and mutation-plan revision changes.
- [x] 1.2 Add the minimum separate Ollama lifecycle module, validated state document, fixed-loopback client boundary, shared Agent-source resolution, and Tauri/frontend contract types without adding a dependency or changing file-tool semantics.

## 2. Recoverable Lifecycle

- [x] 2.1 Add failing fake-loopback tests for read-only planning, exact arbitrary prompt transport, unmanaged collision blocking, stale revision rejection, vanished-base blocking, and the absence of pull, push, generate, chat, or remote requests.
- [x] 2.2 Implement bounded inventory, plan, create/update/remove, and reconciliation commands with one mutation lock, exact revalidation, last-known-good error behavior, and no automatic repair.
- [x] 2.3 Add failing fault-injection tests for preservation failure, runtime failure, state-commit failure, update/remove restoration, first-create cleanup, and temporary recovery-model cleanup.
- [x] 2.4 Reuse Ollama copy/delete plus atomic deployment-state persistence to make every failure preserve or restore runtime and ledger truth.

## 3. Existing-Surface Presentation

- [x] 3.1 Add failing frontend tests for the Agent-detail local-model action, unavailable/stale states, eligible base selection, device-wide scope, exact plan preview, blockers, revision-bound confirmation, reconciliation states, and receipt links.
- [x] 3.2 Extend the existing Agent detail with a focused Ollama lifecycle modal and typed API calls; keep the file installation modal and MCP surface unchanged.
- [x] 3.3 Reuse Activity receipts, semantic errors, focus management, reduced-motion behavior, localization fallbacks, and the established mutation-busy/stale-truth guards.

## 4. Verification and Integration

- [x] 4.1 Run focused and full frontend/Rust tests, Svelte diagnostics, production build, strict Clippy/formatting, dependency audit, diff checks, canonical OpenSpec validation, and strict change validation.
- [x] 4.2 Exercise one uniquely named temporary deployment against the available local Ollama service, verify the exact system prompt and reconciliation state, remove it, and prove no temporary or recovery model remains; record unavailable rather than fabricate evidence if the daemon is absent.
- [x] 4.3 Audit fixed-loopback enforcement, response/prompt bounds, no shell or remote endpoints, no pull/push/inference, exact source ownership, stale-plan denial, rollback ordering, prompt-free receipts, no MCP authority, and preservation of unrelated user changes.
- [ ] 4.4 Sync and archive the canonical spec, update approved Memory Bank and roadmap evidence, merge the verified branch, and repeat post-merge smoke and protected-change checks.
