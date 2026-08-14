## Why

Expert runs already capture versioned quality contracts, terminal human verdicts, evidence, and waivers, but users must inspect runs individually to spot recurring quality problems. A small local summary can surface repeat signals without introducing telemetry or an AI inference service.

## What Changes

- Compare only accepted, rework, and rejected runs for the same Expert version and identical quality contract.
- Require at least five comparable terminal runs before showing metrics or suggestions.
- Derive acceptance, rework/rejection, waiver, and per-check issue rates from existing local run data.
- Show deterministic, non-causal improvement suggestions on the existing Expert detail surface.
- Add no model call, network request, telemetry, new persistence, backend command, route, or configurable analytics framework.

## Capabilities

### New Capabilities

- `expert-improvement-coach`: Threshold-gated local performance summaries and evidence-based suggestions for comparable Expert runs.

### Modified Capabilities

None.

## Impact

- Extends the existing Experts store with a pure aggregation helper.
- Extends the existing Expert detail view with a compact performance section.
- Reuses the existing frontend smoke suite and immutable Expert-run evidence.
