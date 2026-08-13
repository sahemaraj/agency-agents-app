## Context

See `proposal.md` for motivation and `specs/unified-task-search/spec.md` for behavior. The desktop palette currently filters ten navigation commands in memory. Separate exact-token rankers already power the Agent and Skill MCP recommendation tools, but they are private to their MCP modules. Agent selection is routable through `ui.agentsSelected`; Skill selection is currently local to `SkillsWorkspace.svelte`.

## Goals / Non-Goals

**Goals:**

- Make one bounded desktop request return both exact Agent and Skill recommendations from one consistent local catalog snapshot.
- Keep existing MCP recommendation behavior compatible while removing duplicated ranking authority.
- Deep-link recommendations into current workspaces without bypassing inspection or mutation plans.
- Preserve existing palette keyboard and accessibility contracts during asynchronous search.

**Non-Goals:**

- Semantic embeddings, fuzzy model inference, API keys, network search, or catalog refresh.
- Automatic team generation, activation-prompt generation, installation, or agent execution.
- A new navigation destination, recommendation database, or persisted search history.

## Decisions

### Expose one combined read-only desktop recommendation command

Move the existing pure Agent and Skill ranking logic to reusable domain-visible functions and compose it behind one bounded Tauri command that reads current validated sources and preferred Agent-source state. Return a typed union containing artifact kind, exact reference, sanitized package metadata, score, and structured reasons.

This reuses the established rankers and avoids two racing frontend calls or a third TypeScript ranking implementation. Calling MCP tools internally was rejected because desktop commands should not traverse transport, policy, JSON-string, or audit layers for a local read.

### Keep ranking semantics stable

Preserve current exact ASCII-alphanumeric tokenization, weights, installable-only filtering, and per-kind stable ties. Combine kinds by score, then a fixed kind order and exact-reference order. The palette translates structured reasons into user-facing labels; the backend remains locale-neutral.

Semantic/fuzzy ranking was rejected because it adds opaque behavior and likely dependencies without evidence the current deterministic metadata cannot satisfy the P0 discovery goal.

### Extend the existing palette state machine

`CommandPalette.svelte` remains the only global search surface. Existing command filtering stays synchronous. A short minimum task length and debounce trigger the bounded command; a monotonically increasing request generation discards stale responses. Empty, loading, and error states affect only recommendation groups, so navigation commands remain operable.

A separate task-search modal or page was rejected because it duplicates Cmd+K, navigation, focus, and keyboard behavior.

### Add exact Skill deep-link state beside existing Agent deep links

Extend `UiStore` with an exact Skill reference selection and include it in navigation restoration. Agent activation reuses `openAgents` plus exact selection; Skill activation switches to Skills and seeds the exact package key. `SkillsWorkspace.svelte` consumes that state rather than owning an unrelated local selection.

Name-only or slug-only navigation was rejected because duplicate names across sources are already supported and exact source-relative identity is required.

### Preserve inspection-before-mutation

Palette activation only navigates. Existing Agent deployment matrices and Skill trust/plan controls remain the sole mutation paths. No new approval or lifecycle code is introduced.

## Risks / Trade-offs

- Exact token matching misses synonyms and natural-language paraphrases → retain deterministic explanations and measure real misses before considering a more complex local index.
- Large local catalogs could make repeated inspection noticeable → debounce queries and reuse one backend read; optimize only if profiling shows a user-visible delay.
- Adding Skill selection to navigation state can strand a reference after source refresh → resolve against current packages and clear unavailable selections without mutation.
- Cross-kind scores were originally designed independently → keep scores visible only as ordering inputs and use fixed deterministic ties rather than claiming calibrated relevance.

## Migration Plan

1. Add the combined read-only command and focused deterministic tests without changing MCP output contracts.
2. Add exact Skill navigation state and workspace consumption.
3. Extend the palette and locale catalog with asynchronous grouped results and accessible lifecycle states.
4. Verify no network, persistence, or mutation command is invoked by task search.
5. Roll back by removing the combined command and UI additions; no data migration or stored state requires cleanup.
