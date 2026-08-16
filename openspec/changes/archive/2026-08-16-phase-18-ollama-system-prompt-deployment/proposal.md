## Why

Agency Agents can deploy an Agent to file-based coding assistants, but it cannot turn the same reviewed prompt into a reusable local Ollama model. Users currently have to hand-author a Modelfile and then lose the app's preview, approval, rollback, receipt, and drift guarantees.

## What Changes

- Detect a usable local Ollama CLI and list only models already present on the device.
- Let the user select one Agent and one installed base model, then preview the exact derived model name and system prompt before any mutation.
- Create, update, or remove an app-managed Ollama model only after revision-bound confirmation.
- Reconcile the managed model's system prompt with the exact Agent source and classify it as current, outdated, modified, missing, or source-unavailable.
- Preserve the prior managed model definition before replacement and restore it if creation or ledger persistence fails.
- Record successful and failed actions through the existing Activity receipt surface.
- Never pull a model, run inference, contact a remote service, manage the Ollama daemon, overwrite an unmanaged model, or expose this mutation through MCP.

## Capabilities

### New Capabilities

- `ollama-system-prompt-deployment`: Explicit, local-only deployment and reconciliation of an Agent as an Ollama model system prompt.

### Modified Capabilities

None.

## Impact

- Extends the existing Agent detail and mutation-review surfaces with a separate local-runtime target class.
- Adds bounded Tauri commands and app-owned deployment state for Ollama discovery, planning, application, and reconciliation.
- Reuses existing Agent source references, deterministic hashes, atomic state persistence, backup helpers, Activity receipts, and semantic error handling.
- Reuses the installed HTTP client only against Ollama's fixed default loopback API, with bounded payloads and timeouts; no shell, remote host, model inference, or new dependency is added.
