## Context

The Experts store already loads all bounded local runs, and every run snapshots the Expert version and quality contract used at activation. Terminal review states and append-only evidence provide the necessary facts. See `proposal.md` and `specs/expert-improvement-coach/spec.md`.

## Goals / Non-Goals

**Goals:**

- Reuse existing local run evidence and the current Expert detail surface.
- Compare only runs with an identical Expert version and quality contract.
- Keep every metric and suggestion deterministic, explainable, and threshold-gated.

**Non-Goals:**

- LLM coaching, causal claims, cross-Expert benchmarking, telemetry, cloud aggregation, automatic Expert edits, or a configurable analytics subsystem.

## Decisions

### Match the selected Expert's current contract exactly

Use the selected Expert id, version, contract version, and ordered check fields as the cohort identity. This prevents older or structurally different quality contracts from being blended into a misleading trend.

### Exclude cancelled runs

Accepted, rework, and rejected are human quality verdicts. Cancelled runs do not establish quality and therefore do not contribute to the five-run threshold or rates.

### Use latest evidence per check

Evidence is append-only, while review semantics use the latest submission for a check. Aggregation follows that same rule and treats a waived required check as a waiver signal rather than a passing evidence result.

### Keep suggestions deterministic

Display recurring signals only when at least two of five comparable runs, or 40%, exhibit the issue. Suggestions name the observed verdict/check pattern without claiming its cause.

## Risks / Trade-offs

- [Small cohorts can be noisy] → Hide metrics and suggestions until five comparable terminal runs exist.
- [Contract edits split history] → Prefer truthful cohorts over larger but incomparable samples.
- [Deterministic rules are less nuanced than model analysis] → Add model-assisted analysis only after explicit demand and a privacy review.

## Migration Plan

No data migration is required. Removing the helper and UI section fully rolls back the feature.
