## Context

See `proposal.md` for motivation and `specs/doctor-health-check/spec.md` for behavior. Health evidence already exists behind separate commands and stores: storage migration, settings, catalog and corpus status, Agent and Skill source inspection, Agent and Skill install reconciliation, tool detection/version state, MCP client registration, and cached update settings/state. Existing UI recovery controls live primarily in Settings, Catalog, Tools, Agents, and Skills.

The feature crosses Rust domain modules and the Settings UI, but introduces no new persistence or dependency. The trust boundary is diagnostic output: a copied support report must not leak credentials or private absolute paths, and an unknown check must never be promoted to healthy.

## Goals / Non-Goals

**Goals:**

- Establish one canonical classification and redaction authority for health evidence.
- Isolate failures so partial reports remain useful.
- Reuse current local inspection functions and existing recovery UI.
- Make output bounded, deterministic, testable, and safe to copy.

**Non-Goals:**

- A repair engine, scheduler, notification system, telemetry pipeline, benchmark, network monitor, or replacement for detailed domain workspaces.
- Probing GitHub Keychain credentials or remote update/catalog services.
- Persisting reports or health history.

## Decisions

### Aggregate and classify in one backend Doctor module

Add a typed `doctor_report` command that composes current domain inspection functions and returns report metadata, ordered checks, classification, evidence, and action identifiers. Each subsystem inspection is captured independently so one error becomes an Unavailable check instead of failing the entire command.

This is preferred over frontend-only aggregation because it centralizes security redaction and classification rules and prevents copied and rendered reports from drifting. A generic plugin/check framework was rejected: the covered checks are fixed and one implementation does not justify registration abstractions.

### Reuse pure inspection cores, not existing Tauri command transport

Extract or expose only the smallest existing local read functions needed by Doctor. Doctor does not call Tauri commands through transport and does not reuse functions that refresh, reconcile, prompt, or persist as part of inspection. Existing command behavior remains unchanged.

Live probes were rejected because catalog/update checks can perform network I/O, GitHub status can prompt through Keychain, and reconciliation is broader state refresh rather than a passive health read. Doctor reports their already-known configuration/evidence and links users to explicit refresh controls.

### Use three public classifications with deterministic severity

Checks use `healthy`, `needsAttention`, or `unavailable`. Overall severity is Needs attention if any actionable problem exists, otherwise Unavailable if any evidence is unknown, otherwise Healthy. Stable category/check IDs and explicit ordering make output and tests deterministic.

Missing optional integrations such as an uninstalled MCP client are Unavailable rather than Needs attention unless current configuration says the integration is expected. Corrupt state, conflicts, stale reconciliation, invalid packages, and absent required destinations are Needs attention.

### Return structured safe-action identifiers

The backend returns a closed action enum containing navigation intents such as Settings subsection, Agents/Skills reconciliation, Tools, or Catalog. The frontend maps these to existing navigation/store operations; action activation never calls a mutation command.

Free-form shell commands and arbitrary URLs were rejected because they expand the trust boundary and make copied guidance unsafe.

### Put Doctor in existing Settings navigation

Extend the Settings section union, navigation list, localization catalog, and existing modal pane with one Doctor section. The section loads on mount, supports Refresh and Copy Report, renders summary counts and grouped checks, and uses current Settings deep links for actions.

A new sidebar destination was rejected because Doctor is configuration/support functionality, not a daily workspace. Embedding individual health cards across existing screens remains useful but does not solve the consolidated-report requirement.

### Redact before data crosses the backend boundary

Bound and sanitize every evidence and guidance string in Rust. Reuse the established Activity redaction concepts for token/secret/authenticated-URL patterns, replace the current home prefix with `~`, strip control characters, and cap per-field/report sizes. The backend also supplies the canonical copy text so the frontend never reconstructs an unsafe variant.

Frontend escaping remains defense in depth. Returning raw evidence and redacting only during copy was rejected because visible output would still leak data.

## Risks / Trade-offs

- Passive evidence can become stale → include report time and per-check limitations; link to explicit existing refresh/reconcile controls.
- Optional integrations can look broken → reserve Needs attention for configured or core failures and use Unavailable for absent optional clients.
- Existing read helpers may mix reads with initialization → factor pure inspection at the narrowest shared boundary and add mutation/network-spy tests.
- Redaction can remove useful path detail → preserve safe relative suffixes and stable check IDs while hiding home prefixes and secrets.
- A large report can overwhelm users → group by fixed categories, summarize counts first, and keep evidence/actions concise and bounded.

## Migration Plan

1. Add the typed report contract, pure classification/redaction functions, and focused backend tests.
2. Compose the existing local authorities into the read-only command and register it.
3. Add the Settings Doctor pane and action mapping with frontend tests.
4. Run full backend/frontend/build/OpenSpec gates and audit command call paths for network or mutation behavior.
5. Roll back by removing the command and Settings section; no stored data or migration requires cleanup.
