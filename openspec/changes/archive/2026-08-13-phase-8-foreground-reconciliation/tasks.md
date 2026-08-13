## 1. Foreground Lifecycle Contract

- [x] 1.1 Add focused Vitest coverage proving rapid window-focus events debounce to one Agent request and one Skill request.
- [x] 1.2 Add focused coverage proving a focus request overlapping existing mount reconciliation reuses the stores' current in-flight work without an extra visible cycle.
- [x] 1.3 Add focused coverage proving listener cleanup cancels pending work and that foreground reconciliation invokes only the approved local read commands.

## 2. Root Reconciliation

- [x] 2.1 Extend `src/routes/+layout.svelte` to start Agent and Skill install reconciliation at the root using current registered project paths.
- [x] 2.2 Add the 250 ms native window-focus debounce and remove its listener and timer during root cleanup.
- [x] 2.3 Confirm existing Skills workspace mount requests remain compatible with the root and preserve latest project-scope reconciliation.

## 3. Failure and Regression Verification

- [x] 3.1 Verify Agent and Skill focus failures retain last-known rows, expose stale/error state, and recover through existing Retry controls.
- [x] 3.2 Run `npm run check`, `npm run test:frontend`, and `npm run build`.
- [x] 3.3 Run the existing Rust test suite and record the approved manual-platform waiver for native focus behavior not automatable in this environment.
- [x] 3.4 Run strict OpenSpec validation and verify SYNC-01, SYNC-02, and SYNC-03 against the implemented diff.
