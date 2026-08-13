## Context

See `proposal.md` for motivation and `specs/guided-first-deployment/spec.md` for behavior. The current first-run overlay ends when `catalog.configured` becomes true. Tool detection, project registration, bundled presets, exact-reference mutation plans, batch rollback, reconciliation, Activity, and starter prompts already exist, but no component composes them into the first successful deployment.

## Goals / Non-Goals

**Goals:**

- Compose existing primitives into one foreground catalog-to-success guide.
- Preserve exact-reference identity, explicit approval, rollback, and reconciliation truth.
- Keep recommendation and timing behavior deterministic and locally testable.
- Add the smallest backend surface necessary to plan and apply a transient exact-reference batch.

**Non-Goals:**

- Model-backed or personalized recommendations.
- Installing Claude Code or Codex themselves.
- Background deployment, automatic approval, analytics, telemetry, or accounts.
- Replacing the general Teams or InstallModal workflows.
- Expanding the first-deployment target set beyond Claude Code and Codex.

## Decisions

### Use a bounded deterministic pipeline, not an AI loop

Phase 7 has no runtime discovery that requires model reasoning. Its complete contract is:

- **Goal:** reach one verified Claude Code or Codex preset deployment for a new user.
- **Observations:** catalog configuration, active exact Agent references, Claude Code/Codex detection, registered projects, mutation-plan output, and reconciled install rows.
- **States:** catalog → prepare → review → applying → success, with terminal deferred, blocked, and failed states.
- **Decisions:** pure preset fallback, Claude Code-before-Codex defaulting, explicit user/project selection, and mutation-plan applicability.
- **Allowed actions:** choose catalog, create one read-only plan, accept one explicit approval, execute at most one transactional batch, then reconcile once. Retry is user-triggered and starts a new bounded attempt.
- **Success evaluator:** every planned exact reference is present at the approved tool and project path after an error-free reconcile.
- **Hard termination:** success, defer, no compatible target or preset, invalid/blocked plan, apply failure after rollback, reconcile failure, cancellation, or the end of the four-stage interaction. No automatic retry or open-ended loop exists.
- **Budgets:** four user-visible stages, one consequential write attempt per approval, no required network call after catalog selection, a 60-second happy-path interaction target, and zero model calls, tokens, or model cost.
- **Approval and recovery:** approval is bound to the visible plan's tool, scope, destinations, references, warnings, and blockers; apply revalidates current inputs, rollback restores destinations and ledger on failure, and incomplete reconciliation never becomes success.
- **State and privacy:** only a versioned local completion marker is retained; no prompt, timing, project content, telemetry, or additional user memory is stored.
- **Auditability:** existing Activity entries record the approved batch outcome, while the install ledger and reconciliation provide deterministic replayable evidence of final state.

### Extend the existing first-run surface with explicit stages

`CatalogFirstRun.svelte` will remain the root-owned modal and advance through catalog, deployment, review, and success stages. This keeps the flow foregrounded across the `catalog.configured` transition and avoids a second root overlay controller.

Alternative: navigate to Teams after catalog selection. Rejected because it recreates the current unguided handoff and cannot reliably present completion as one flow.

### Store first-run visibility separately from catalog configuration

Catalog configuration remains durable backend truth. A versioned local completion marker will control whether an already-configured but still-new user needs the guide. The marker is written only when the user defers or reconciliation verifies success, never merely because catalog selection succeeded.

Alternative: infer first-run solely from an empty install ledger. Rejected because a returning user may intentionally have no installs, and a foreign installation may appear without an app-managed completion decision.

### Use a pure deterministic recommendation helper

A small pure helper will receive active catalog slugs and ordered preset definitions. It returns AI Builders when complete, otherwise the first complete preset, otherwise no recommendation. One focused unit test will cover ordering and incomplete catalogs.

Alternative: score presets dynamically. Rejected because Phase 7 requires one deterministic recommendation and has no evidence for personalization.

### Reuse the mutation plan schema with transient exact-reference batches

The backend will expose plan/apply commands that accept a bounded list of exact Agent references. They will call the existing mutation-plan builder and transactional install executor. The apply command rebuilds the plan from current sources before executing, just as existing collection apply does. No hidden personal-library collection is created.

Alternative: save the preset as a collection then call collection plan/apply. Rejected because onboarding would mutate unrelated personal-library state before deployment approval and could collide with user collection names.

Alternative: call individual install commands from the UI. Rejected because failure could leave a partial team and would bypass the existing batch rollback guarantee.

### Reuse target, project, plan, and prompt presentation

The guide will reuse existing target detection and project registration stores, the mutation plan's existing fields and applicability check, and `StarterPrompt`. Shared presentation will be extracted from `InstallModal` only if direct reuse is impossible without duplicating behavior; otherwise the guide will pass a transient batch into a narrowly extended modal.

Alternative: build a separate deployment grid and plan UI. Rejected because it would duplicate safety logic and accessibility behavior.

### Verify against exact references after apply

Apply already triggers reconciliation. The guide will compare every planned exact reference, tool, and project path against the reconciled install state and show destinations from those confirmed rows. Any reconcile error, missing row, or non-present lifecycle state keeps the guide out of success.

Alternative: trust the apply return value. Rejected because GUIDE-05 explicitly requires reconciliation-backed success and disk truth can differ from command return state.

### Measure the interaction contract without telemetry

An in-memory monotonic timer starts when deployment preparation becomes usable and is used only by tests/manual verification; no duration is persisted or transmitted. The UI is structured for the four required decisions rather than displaying a countdown.

Alternative: product analytics. Rejected because the app has a no-telemetry contract and the requirement concerns design, not collection of user behavior.

## Risks / Trade-offs

- [A previously configured user never received the new guide] → use a versioned completion marker and show the guide when that marker is absent and reconciled managed installs are empty.
- [Tool detection or reconciliation is still loading] → show a single preparation state and disable plan mutation until both truths are fresh.
- [Preset slugs differ across a custom catalog] → require a complete preset and use deterministic fallback; never silently deploy a partial recommendation.
- [Shared InstallModal extraction grows the diff] → first try a narrow input/output extension; extract only the existing plan presentation if Svelte ownership makes composition impossible.
- [Project choice adds time] → default to user scope while preserving explicit project scope and the existing native picker.
- [Generated OpenSpec Codex skills duplicate globally available skills] → keep them repo-local as the reproducible project workflow; they add no runtime dependency.

## Migration Plan

1. Land OpenSpec project configuration and the validated change artifacts on the feature branch.
2. Add focused recommendation and orchestration tests before implementation.
3. Add the exact-reference batch adapter over existing transaction internals.
4. Extend the first-run UI and locale contract.
5. Verify OpenSpec strictly, frontend checks/tests/build, Rust tests/format, and the manual catalog-to-success path.
6. Roll back by reverting the feature branch; existing catalog configuration, installs, and general deployment flows remain compatible because no stored schema is changed.
