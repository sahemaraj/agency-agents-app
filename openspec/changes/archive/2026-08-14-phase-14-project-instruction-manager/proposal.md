## Why

Registered projects can receive Agents and Skills, but users still have to edit each AI tool's project instruction file manually and risk overwriting existing governance. A local inspect-review-approve workflow closes that gap while preserving the app's non-runtime, filesystem-safe boundary.

## What Changes

- Inspect four bounded known project instruction targets without following links or modifying content.
- Compose app-owned named snippets into existing files, preserving all unowned bytes and treating first insertion as explicit adoption.
- Produce a complete deterministic diff and revision before any write; reject stale or unregistered projects at apply time.
- Back up every existing file before atomic replacement and journal the mutation for crash-safe rollback.
- Extend the existing Projects detail surface with inspect, compose, review, approval, retained result, and Activity evidence.
- Do not execute instruction text, create arbitrary paths, manage whole files, or infer changes from project contents.

## Capabilities

### New Capabilities

- `project-instructions`: Safe inspection, adoption, diff planning, and revision-bound application of bounded project instruction snippets.

### Modified Capabilities

None.

## Impact

- Backend: existing project registry and install/recovery command surface.
- Frontend: existing Projects component, projects store, shared types, and Activity journal.
- Storage: existing filesystem-operation journal and app-data backup directory; no new dependency or state document.
- Filesystem: only `AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, and `.github/copilot-instructions.md` beneath an exact registered project.
