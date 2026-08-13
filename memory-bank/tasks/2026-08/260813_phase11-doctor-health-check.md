# 260813_phase11-doctor-health-check

## Objective

Give users one truthful, privacy-safe view of local Agency Agents health and route every actionable result to an existing safe recovery surface.

## Outcome

- Added one bounded, read-only Doctor report for storage, settings, catalog, Agent and Skill sources, installation truth, tools, MCP clients, and cached update state.
- Classified each check independently as Healthy, Needs attention, or Unavailable so one failed authority does not hide successful evidence or produce a false healthy result.
- Added deterministic redacted report copying, including secret, authenticated-URL, control-character, and private home-prefix protection.
- Reused the existing Settings modal and recovery surfaces; Doctor never repairs, installs, refreshes network state, prompts for credentials, or mutates state.
- Removed write-capable database opening from passive migration/completion inspection and proved Doctor leaves empty and existing app-data directories unchanged.

## Verification

- OpenSpec: 15/15 tasks complete; strict change validation passed; canonical specs validate 5/5 after sync and archive.
- Backend: 542 tests discovered; 539 passed and 3 existing environment-gated tests ignored; 2/2 binary tests passed.
- Doctor-focused backend: 8/8 tests passed, including mutation-spy and redaction coverage.
- Rust quality: formatting and strict Clippy passed.
- Frontend: 91/91 tests passed.
- Svelte: 0 errors and 0 warnings.
- Build: production frontend build passed.
- Dependency, route, prohibited-call, and `git diff --check` audits passed.

## Integration Points

- `src-tauri/src/commands/doctor.rs` owns independent local inspection, classification, bounded evidence, and deterministic report formatting.
- `src-tauri/src/state_db.rs` exposes read-only existing-database inspection for passive health and completion checks.
- `src/lib/components/SettingsSectionDoctor.svelte` extends the existing Settings modal with grouped results, Refresh, Copy Report, announcements, and safe navigation actions.
- `openspec/specs/doctor-health-check/spec.md` is the canonical capability contract.

## Security and Safety

- Doctor performs no network, Keychain, telemetry, notification, installation, execution, reconciliation, or direct repair action.
- Visible and copied evidence is bounded and normalized before leaving the backend.
- Existing data and filesystem state are unchanged by report generation.

## Artifacts

- Implementation commit: `45d8430`
- Branch: `feat/phase-11-doctor-health-check`
- OpenSpec archive: `openspec/changes/archive/2026-08-13-phase-11-doctor-health-check/`
