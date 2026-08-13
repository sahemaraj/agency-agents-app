## Why

Agency Agents already has a global command palette and deterministic Agent and Skill recommendation engines, but those capabilities are disconnected: users must know which library to browse before they can find the right reusable configuration for a task. Phase 10 closes the final P0 discovery gap by turning Cmd+K into one local, explainable task search without adding an AI runtime, API key, or network dependency.

## What Changes

- Extend the existing command palette so a bounded task description returns combined exact Agent and Skill recommendations alongside existing navigation commands.
- Reuse one shared deterministic ranking contract for desktop and MCP callers, including stable ordering, installable-only results, and structured match reasons.
- Present human-readable reasons and provenance for every recommendation rather than an opaque relevance score.
- Open the exact Agent or Skill in its existing workspace so inspection, deployment planning, trust checks, and approval remain on the established lifecycle path.
- Preserve keyboard navigation, accessible result grouping, loading/error/empty states, and command filtering.
- Keep recommendation local and read-only; do not generate teams, install directly from the palette, execute Agents, call models, or access the network.

## Capabilities

### New Capabilities

- `unified-task-search`: Local task-to-Agent/Skill recommendations in the existing command palette, with explainable ranking and exact safe handoff to current workspaces.

### Modified Capabilities

None.

## Impact

- Extends `src/lib/components/CommandPalette.svelte`, the existing UI navigation state, and focused frontend tests.
- Extracts or exposes the current Agent and Skill exact-token rankers through bounded read-only Tauri commands while keeping MCP behavior compatible.
- Adds typed recommendation results and English-baseline locale keys with existing fallback behavior.
- Adds no dependency, persistence format, network call, installation shortcut, or execution surface.
