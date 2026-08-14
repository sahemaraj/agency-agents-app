## 1. Contract and Aggregation

- [x] 1.1 Add failing frontend tests for exact comparable-run selection, the five-run threshold, latest evidence, verdict rates, waivers, and recurring-signal rules.
- [x] 1.2 Extend the existing Experts store with the minimum pure aggregation helper and no new persistence or backend authority.

## 2. Existing-Surface Presentation

- [x] 2.1 Add a failing component test for below-threshold disclosure and eligible metrics/suggestions.
- [x] 2.2 Extend the existing Expert detail surface with bounded, non-causal performance copy.

## 3. Verification and Integration

- [x] 3.1 Run focused/full frontend tests, Svelte diagnostics, production build, Rust regression tests, strict Clippy/formatting, and strict OpenSpec validation.
- [x] 3.2 Audit for exact cohort identity, cancelled/non-terminal exclusion, five-run gating, latest-evidence semantics, local-only derivation, no mutation, no telemetry, no model/network call, and no unrelated user-file changes.
- [ ] 3.3 Sync and archive the canonical spec, update approved Memory Bank/roadmap evidence, merge the verified branch, and repeat post-merge smoke checks.
