# 260816_skill-review-cleanup

## Objective

Remove false Skills review entries caused by hidden runtime mirrors and prevent untracked foreign
installs from being presented as cleanup candidates.

## Outcome

- Source discovery now skips hidden descendant directories such as `.cursor`, `.agents`, and
  `.opencode` while still discovering visible nested Skill packages.
- Cleanup suggestions now require an app-tracked, non-missing install with no recorded fetch or
  install usage.
- Existing script trust, invalid metadata, and unsafe-entry validation remain unchanged.
- No Skill package was trusted, deleted, or modified.

## Files Modified

- `src-tauri/src/skills/mod.rs` — hidden runtime-mirror discovery guard and regression test.
- `src/lib/skills/libraryModel.ts` — shared tracked-install cleanup predicate.
- `src/lib/skills/libraryModel.test.ts` — tracked-versus-foreign cleanup regression coverage.

## Integration Points

- Existing source inspection and MCP catalog reads use the corrected discovery result.
- Existing Skills workspace filtering and metric counts share the corrected cleanup predicate.

## Verification

- Live Agency Agents MCP: Library 2,180 → 1,690; Needs review 672 → 202; repeated
  `context-restore` entries → 1; Cleanup suggestions → 0.
- Frontend: 117 tests passed, Svelte check reported 0 errors and 0 warnings, and the production
  build passed.
- Backend: 582 tests passed with 3 documented environment-gated ignores.
- Quality: strict Clippy, Rust format, and `git diff --check` passed.

## Deliberate Limit

The remaining 202 review entries retain genuine package validation or exact-version script trust
requirements. They require package-owner correction or explicit human review and were not bypassed.
