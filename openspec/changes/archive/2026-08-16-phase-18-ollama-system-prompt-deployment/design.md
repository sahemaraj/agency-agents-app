## Context

The project already records that model runtimes are a separate target class from file-based Agent hosts because Ollama serves model weights and has no Agent destination directory. Existing Agent installation code assumes filesystem destinations, per-tool renderers, and `installs` ledger rows, while the app already provides reusable source references, deterministic hashing, revision-bound plans, atomic state documents, Activity receipts, and last-known-good reconciliation behavior.

Ollama's fixed local API supports inventory, show, create, copy, and delete. Its JSON create payload carries the system prompt as data; this matters because valid catalog prompts contain literal triple quotes that cannot be safely interpolated into a generated multiline Modelfile.

## Goals / Non-Goals

**Goals:**

- Add one truthful device-local runtime target without weakening the existing mutation contract.
- Preserve the exact Agent body, source identity, and runtime state across preview, application, reconciliation, and recovery.
- Reuse installed dependencies and existing UI/state patterns with the smallest separate implementation surface.

**Non-Goals:**

- LM Studio or other runtimes, model download, daemon lifecycle, inference, prompt testing, parameter tuning, quantization, publishing, remote Ollama hosts, MCP mutation, bulk deployment, project-scoped models, or user-defined target names.

## Decisions

### Keep Ollama outside the file-tool registry

Add a narrow Ollama lifecycle module and deployment record rather than adding `ollama` to `Tool`. File-tool reconciliation assumes paths, rendered files, user/project scope, and install destinations; pretending a model is a file target would spread exceptions through every caller. The new module reuses source inspection, hashes, state documents, errors, and receipts but owns runtime-specific truth.

Alternative: extend the existing install ledger with virtual `ollama://` destinations. Rejected because existing mutation, reveal, diff, disable, project, and reconciliation paths dereference destinations as files.

### Use the fixed loopback JSON API

Use the already-installed HTTP client against exact `http://127.0.0.1:11434/api/...` endpoints with response-size limits and operation-specific timeouts. JSON create preserves arbitrary prompt text and avoids shell or Modelfile parsing. Discovery and apply re-check inventory; cloud-tagged bases are excluded, and the implementation never calls pull, push, generate, or chat.

Alternative: invoke `ollama create` with a generated Modelfile. Rejected because existing Agent prompts can contain the same triple-quote delimiter used by multiline `SYSTEM` values, making exact rendering impossible without an undocumented escape convention.

### Derive a collision-resistant app-owned model name

Generate `agency-agents/<agent-slug>-<reference-digest>:latest`, where the short digest comes from the full source id and relative path. The name is stable, previewable, shell-free, and distinguishes equal slugs from different sources. An existing target is mutable only when the deployment document contains the same full source reference and target name.

Alternative: accept an arbitrary target name. Rejected because it adds validation, collision, typo, and ownership ambiguity without improving the first useful version.

### Store a separate bounded deployment document

Persist one validated state document containing the full Agent reference, target name, base-model name and digest, source snapshot hash, prompt hash, and timestamp. Do not store prompt text; reconcile obtains current source text and the runtime-reported system prompt, then hashes both.

Alternative: store prompt text for offline comparison. Rejected because the source remains authoritative and prompt persistence would duplicate potentially sensitive content.

### Rebuild and compare every plan at apply time

Planning is read-only. Apply repeats source resolution, base inventory, target ownership, and runtime state inspection, then requires the derived revision to match the reviewed revision. No API mutation occurs when blockers exist, reconciliation is stale, or the revision changed.

### Preserve managed targets with Ollama copy

Before update or removal, copy the managed target to a temporary app-namespaced recovery model. If target mutation or deployment-state commit fails, restore from that copy and restore the original state document. For a failed first create after runtime success, delete the new target. Cleanup of the temporary recovery model is best-effort only after both runtime and state commits succeed; a cleanup failure is logged and the deterministic recovery target is removed before the next explicit mutation. Passive reconciliation remains read-only.

Alternative: reconstruct a backup from show output. Rejected because show does not provide a stable lossless contract for every model attribute, while copy preserves Ollama's own model manifest.

### Extend the existing Agent detail action surface

Add a secondary local-model action beside the existing installation action for an exact `AgentPackageResult`. A focused modal owns runtime inventory, current deployment state, base selection, plan preview, confirmation, and terminal receipt link. This keeps the existing file deployment modal unchanged and makes device-wide scope explicit.

## Risks / Trade-offs

- [Ollama API is unavailable or bound to a non-default host] → report unavailable; do not add configurable hosts or start the daemon in this phase.
- [Create can take longer than ordinary configuration writes] → use a bounded long-running timeout, busy state, and one in-flight mutation lock.
- [Ollama changes response shapes] → deny unknown/oversized mutation-critical data and retain last-known-good reconciliation state.
- [Process interruption leaves a temporary recovery model] → use a deterministic app-only recovery name and remove that exact target before the next explicit mutation without touching user models.
- [A short digest could theoretically collide] → validate full source identity in the deployment document and block rather than adopt any mismatch.

## Migration Plan

No existing Agent install state changes. The new deployment document starts empty. Rollback removes the desktop entry point and runtime commands; app-managed models already created remain usable in Ollama, while removal stays available until users clear tracked deployments.
