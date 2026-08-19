# Active Context — Agency Agents

## Factory Runs MVP [COMPLETE] (2026-08-18)

State: DOCS / IDLE on local `main` after the approved merge. The approved OpenSpec change
`factory-runs-mvp` remains the source of truth. Implementation extends the existing Expert Run,
readiness, MCP, Experts, and Activity authorities without adding an executor, database document,
permission family, route, runtime, or dependency. V10 independent review found one Medium
case-sensitivity false positive and one Medium raw-source class covering call/collection
assignments and semicolon-free mutating SQL/DDL. Exact backend and Activity regressions failed
first; shared-classifier fixes now pass both targeted tests while preserving ordinary `Import`,
`Insert`, `Update`, and `Create table` prose. Fresh QA passes 50/50 focused Factory tests, 843
executable Rust library tests with 5 intentional manual-only ignores, 2 binary tests, strict
format/Clippy, 199/199 frontend tests, zero Svelte diagnostics, production build, npm audit with
zero vulnerabilities, 14/14 Factory MCP protocol tests, exact 137-tool inventory with seven
Factory operations, strict OpenSpec 21/21, and diff checks. Source snapshot
`4cd64e73fe42b5b21eac43ef9af22b08bbd1ea680ea4480ac2435c63cf16681c` received a clean goal audit,
but v11 code/security audits found Python alias, SQL variant, uppercase RHS, DDL, `privateKey`, and
safe SELECT-prose gaps. Exact backend/Activity regressions now pass after shared root fixes. Fresh
QA again passes 50/50 focused Factory tests, 843 executable Rust library tests with 5 intentional
manual-only ignores, 2 binary tests, strict format/Clippy, 199/199 frontend tests, zero Svelte
diagnostics, production build, npm audit with zero vulnerabilities, strict OpenSpec 21/21, and diff
checks. Source snapshot `c004b3643a5f20c9ff4500d776c5ca999efd63fbe3e4958e7b21e91b8bb45d7f`
received a v12 security finding for bare-relative/trailing-comment Python imports and
semicolon-free literal SELECT/CTE SQL. Exact backend/Activity regressions failed first and now pass
after the shared classifier repair. Fresh verification passes 50/50 focused Factory tests, 843
executable Rust library tests with 5 intentional manual-only ignores, 2 binary tests, strict
format/Clippy, 199/199 frontend tests, zero Svelte diagnostics, production build, npm audit with
zero vulnerabilities, 14/14 Factory MCP protocol tests, exact 137-tool inventory with seven
Factory operations, strict OpenSpec 21/21, and diff checks. A frozen v13 code, security, and
goal-backward audit found no lifecycle defect and a clean security result, but the code audit found
three Medium metadata-classifier gaps: constructor assignments, materialized/replaced views plus
frontend SELECT aliases/TOP counts, and false positives for terse sentence-case Select/Delete
prose. Exact backend/Activity tests failed first and now pass after shared root fixes. Fresh QA again
passes 50/50 focused Factory tests, 843 executable Rust library tests with 5 intentional manual-only
ignores, 2 binary tests, strict format/Clippy, 199/199 frontend tests, zero Svelte diagnostics,
production build, npm audit with zero vulnerabilities, 14/14 Factory MCP protocol tests, exact
137-tool inventory with seven Factory operations, strict OpenSpec 21/21, and diff checks. A frozen
v14 code, security, and goal-backward audit is next. OpenSpec progress remains 27/29 until those
audits pass. V14 goal review found one Medium privacy regression in the broad sentence-case prose
exception: valid `Select email from users` / `Delete from users` SQL could pass. Exact backend and
Activity regressions failed first. The exception is now fail-closed and limited to the two reviewed
UI literals that are syntactically ambiguous with SQL; both unsafe sentence-case variants and safe
UI controls pass. Fresh full QA remains 843 executable Rust library tests with 5 intentional
manual-only ignores, 2 binary tests, strict format/Clippy, 199/199 frontend tests, zero Svelte
diagnostics, production build, npm audit with zero vulnerabilities, strict OpenSpec 21/21, and diff
checks. A frozen v15 final audit is next. Local `main` and its unrelated dirty changes remain
untouched. V15 code review found three adjacent Medium syntax classes: generic constructor
assignments, alter/drop/refresh materialized-view DDL, and Activity parity for parenthesized TOP
counts and quoted aliases. Exact backend/Activity tests failed first; root fixes now reject any
constructor RHS, cover the materialized-view lifecycle, and mirror backend SELECT forms. Fresh full
Rust/frontend, strict format/Clippy/Svelte/build, dependency, OpenSpec 21/21, and diff gates pass.
A frozen v16 final audit is next; progress remains 27/29.
V16 security/goal review found two Medium residual raw-source classes: implicit SELECT projection
aliases and uppercase scalar/arrow assignments. Exact backend/Activity tests failed first. SELECT
target parsing now validates the complete table/alias segment, implicit projection aliases are
mirrored, and assignment-shaped metadata fails closed except for two exact reviewed UI sentences.
Fresh full QA again passes 843 executable Rust library tests with 5 intentional manual-only ignores,
2 binary tests, strict format/Clippy, 199/199 frontend tests, zero Svelte diagnostics, production
build, npm audit with zero vulnerabilities, OpenSpec 21/21, and diff checks. A frozen v17 final
audit is next; progress remains 27/29.
V17 independent review found seven Medium metadata syntax families: capitalized YAML bare scalars,
trailing SELECT operators, parenthesized destructuring, complex FROM/TOP/quoted-table forms,
temporary/unlogged/materialized DDL, compound assignments, and C/C++/XML directives. Exact backend
and Activity regressions failed first and now pass after shared fail-closed classifier repairs. Fresh
QA on the exact current source passes 843 executable Rust library tests with 5 intentional
manual-only ignores, 2 binary tests, strict format/Clippy, 199/199 frontend tests, zero Svelte
diagnostics, production build, npm audit with zero vulnerabilities, 14/14 Factory MCP protocol
tests, the exact 137-tool inventory with seven Factory operations, strict OpenSpec 21/21, and diff
checks. A frozen v18 code, security, and goal-backward audit is next; progress remains 27/29.
V18 frozen review passed all lifecycle/control-plane goal coverage but found one High backend URL
credential-alias gap and consolidated Medium backend/Activity syntax gaps in DDL, YAML, directives,
XML, adjacent SQL statements, and destructuring parity. State returned to BUILD / CODING for one
paired-regression repair cycle; progress remains 27/29.
The V18 High/Medium findings and Low safe-prose controls now have paired failing-first regressions
and shared-validator repairs. Fresh QA passes 843 executable Rust library tests with 5 intentional
manual-only ignores, 2 binary tests, strict format/Clippy, 199/199 frontend tests, zero Svelte
diagnostics, production build, npm audit with zero vulnerabilities, 14/14 Factory MCP protocol
tests, the exact 137-tool inventory with seven Factory operations, strict OpenSpec 21/21, and diff
checks. Browser keyboard/axe evidence at 375/1440 remains applicable because the repair changes
only metadata classification and regression fixtures. A frozen v19 code, security, and
goal-backward audit is next; progress remains 27/29.
V19 froze clean lifecycle evidence but code/security review found one High prefixed credential-key
URL gap and Medium structured-YAML, SQL modifier/transaction/DDL, and directive parity gaps. The
source returned to BUILD / CODING for paired regressions and structural classifier repair;
progress remains 27/29.
V19 findings now have paired failing-first regressions and structural repairs: backend credential
suffixes match Activity, YAML uses serialization/config signals instead of every colon sentence,
SQL covers reviewed modifier/transaction/DDL forms, and directive parity includes Objective-C,
Rust visibility, C# static/alias, and macro imports. Fresh QA passes 843 executable Rust library
tests with 5 intentional manual-only ignores, 2 binary tests, strict format/Clippy, 199/199
frontend tests, zero Svelte diagnostics, production build, npm audit with zero vulnerabilities,
14/14 Factory MCP protocol tests, exact 137/seven inventory, strict OpenSpec 21/21, and diff checks.
A frozen v20 final audit is next; progress remains 27/29.
V20 code/security review found one High URL-signature/password-alias gap and consolidated Medium
classifier gaps in capital/block-scalar YAML, transaction variants, combined view modifiers,
Objective-C/C++ imports, plus an EXPLAIN safe-prose false positive. State returned to BUILD /
CODING for paired failing-first regressions and shared backend/Activity repair; progress remains
27/29.
V20 paired regressions failed first and now pass after matching credential aliases, structurally
classifying capital/block-scalar YAML and transaction/import/view variants, and constraining
EXPLAIN to valid modifier grammar. State advanced through DIFF to QA / RUNNING for the complete
verification matrix; progress remains 27/29 pending clean frozen audits.
Fresh V21 QA on the exact repaired source passes 843 Rust library tests with 5 intentional
manual/environment ignores, 2 binary tests, 0 doctests, strict format/Clippy, 199 frontend tests,
zero Svelte diagnostics, production build, npm audit with zero vulnerabilities, 14/14 Factory MCP
protocol tests, exact 137/seven inventory, raw-router boundary smoke, strict OpenSpec 21/21, and
diff checks. Prior 375/1440 keyboard/axe evidence remains applicable because V21 changes only the
shared metadata classifiers and their regressions. A frozen V21 code, security, and goal-backward
audit is next; progress remains 27/29.
V21 goal-backward audit passed with all 21 requirements and 47 scenarios proven, but the frozen
code/security audits found one High JWT URL-credential gap and Medium combined SQL, tagged/commented
YAML, and source-directive gaps, plus Low one-word transaction and Owner prose false positives.
State returned to BUILD / CODING for paired regressions and shared-classifier repair; progress
remains 27/29.
V21 findings now have paired failing-first regressions and shared root repairs for JWT key/value
credentials, valid combined SQL forms, tagged/commented YAML, C#/Rust/Objective-C directives, and
reviewed one-word/Owner prose controls. State advanced through DIFF to QA / RUNNING for V22 full
verification; progress remains 27/29.
Fresh V22 QA passes 843 Rust library tests with 5 intentional ignores, 2 binary tests, 0 doctests,
strict format/Clippy, 199 frontend tests, zero Svelte diagnostics, production build, npm audit with
zero vulnerabilities, 14/14 Factory MCP protocol tests, exact 137/seven inventory, raw-router
boundary smoke, strict OpenSpec 21/21, and diff checks. Retained 375/1440 keyboard/axe evidence is
unchanged because V22 remains classifier/test-only. Frozen V22 audits are next; progress remains
27/29.
V22 code/security review found one High SAS `sig` credential gap and Medium compact-JWT/generic
redaction, semicolon SQL parity, YAML quoted/indentation forms, EXPLAIN modifiers, and additional
Rust/C#/Objective-C directives, plus the Priority prose control. The failing snapshot did not need
a goal verdict; state returned to BUILD / CODING for paired regressions and root repair. Progress
remains 27/29.
V22 findings now have paired failing-first regressions and root fixes across backend, Factory
Activity normalization, and generic Activity redaction. Compact JWTs, SAS signatures, semicolon
SQL, quoted/indentation YAML, EXPLAIN modifiers, directives, and Priority prose controls all pass.
State advanced through DIFF to QA / RUNNING for V23 verification; progress remains 27/29.
Fresh V23 QA passes 843 Rust library tests with 5 intentional ignores, 2 binary tests, 0 doctests,
strict format/Clippy, 199 frontend tests, zero Svelte diagnostics, production build, npm audit with
zero vulnerabilities, 14/14 Factory MCP protocol tests, exact 137/seven inventory, raw-router
boundary smoke, strict OpenSpec 21/21, and diff checks. Retained 375/1440 keyboard/axe evidence is
unchanged because V23 remains classifier/test-only. Frozen V23 code, security, and goal-backward
audits are next; progress remains 27/29.
Frozen V30 security and goal-backward audits found zero Critical/High findings but consolidated
Medium privacy gaps in structured `SG.` credentials, host-indicated encoded multi-segment webhook
tails, mixed-case semicolon-free generic DDL, and exported namespace declarations. State returned
to BUILD / CODING for one shared-classifier repair with paired failing-first regressions; progress
remains 27/29. The code-audit worker returned a stale status instead of an audit and will be rerun
on the next frozen snapshot.
V30 findings now pass paired regressions after structural multipart-token recognition, aggregate
decoded webhook-tail validation for both marker and host contexts, case-independent DDL structure
with punctuation-based prose exclusion, and exported/aliased namespace handling. Focused QA passes
1/1 Rust and 2/2 frontend tests with zero Svelte diagnostics. State advanced through DIFF to
QA / RUNNING for the complete V31 matrix; progress remains 27/29.
Fresh V31 QA passes 844 Rust library tests with 5 intentional ignores, 2 binary tests, 0 doctests,
strict format/Clippy, 203 frontend tests, zero Svelte diagnostics, production build, npm audit with
zero vulnerabilities, all Factory MCP/inventory/router coverage, strict OpenSpec 21/21, and clean
dependency/scope/diff gates. Local `main` remains at the shared base with unrelated dirty changes
untouched; no archive exists. Frozen V31 code, security, and goal-backward audits are next;
progress remains 27/29.
Frozen V31 code-quality, security, and goal-backward audits all pass with zero Critical, High,
Medium, or Low findings against unchanged source hash
`efce76bf56fdaf037bf083d3ccd0d44aea67c8abaa453cb64010c45152051af2`. The goal audit proves all
21/21 delta requirements, 47/47 delta scenarios, and 8/8 canonical compatibility requirements.
OpenSpec tasks are 29/29 complete. The implementation was committed and merged into local `main`.
Its deltas were synchronised into four canonical specs, strict canonical validation passes 22/22,
and the change is archived at `openspec/changes/archive/2026-08-19-factory-runs-mvp/`.
Frozen V23 goal review passed all 21 requirements and 47 scenarios, but code/security review found
one High standard-session credential gap and consolidated Medium gaps in compact JWT/JWE handling,
frontend credential-key parity, quoted YAML separators, SQL/source syntax, and legacy JSON
validation. State returned to BUILD / CODING for paired regressions and root repair; progress
remains 27/29.
V23 findings now have paired failing-first regressions and shared root repairs for standard session
credentials, unsecured JWT/JWE values, key-aware frontend redaction, quoted YAML separators,
additional SQL/source grammar, legacy JSON validation, and reviewed Severity prose. Focused QA
passes 51/51 Rust Factory tests and 3/3 targeted Activity tests. State advanced through DIFF to
QA / RUNNING for V24 full verification; progress remains 27/29.
Fresh V24 QA on the exact repaired source passes 844 Rust library tests with 5 intentional ignores,
2 binary tests, 0 doctests, strict format/Clippy, 199 frontend tests, zero Svelte diagnostics,
production build, npm audit with zero vulnerabilities, 14/14 Factory MCP protocol tests, exact
137/seven inventory, raw-router boundary smoke, strict OpenSpec 21/21, and diff/scope/boundary
checks. Retained 375/1440 keyboard/axe evidence remains applicable because V24 changes only shared
metadata validation/redaction, legacy JSON validation, and regressions. Frozen V24 code, security,
and goal-backward audits are next; progress remains 27/29.
Frozen V24 goal review again proves all 21 requirements and 47 scenarios, but code/security review
found Medium trust-boundary gaps in persisted Activity hydration, `sid` and direct-encryption JWE
credentials, credential-bearing webhook paths, SQL variants, and source annotation/directive
families. State returned to BUILD / CODING for paired regressions and shared backend/Activity root
repairs. No Critical or High finding exists; progress remains 27/29.
V24 findings now have paired failing-first regressions and shared root repairs: persisted Activity
details pass through the existing privacy normalizer, `sid` and direct-encryption JWE values are
credentials without the prior `possession` false positive, known webhook bearer paths fail closed,
and SQL/source families are mirrored across Rust and Activity with reviewed prose controls. The
focused Rust privacy test and four targeted frontend tests pass. State advanced through DIFF to
QA / RUNNING for V25 full verification; progress remains 27/29.
Fresh V25 QA passes 844 Rust library tests with 5 intentional manual/environment ignores, 2 binary
tests, 0 doctests, strict format/Clippy, 199 frontend tests, zero Svelte diagnostics, production
build, npm audit with zero vulnerabilities, 14/14 Factory MCP protocol tests, exact 137/seven tool
inventory, raw-router and router-composition boundaries, strict OpenSpec 21/21, and clean
dependency/scope/diff checks. Retained 375/1440 keyboard/axe evidence remains applicable because
V25 changes only shared metadata validation/redaction, hydration, and regressions. Frozen V25
code, security, and goal-backward audits are next; progress remains 27/29.
Frozen V25 goal review again proves 21/21 requirements and 47/47 scenarios, but code/security
review found one High credential-bearing API/webhook URL gap and consolidated Medium gaps in
durable Activity normalization, percent-encoded credentials, SQL families/case handling, and bare
decorators. State returned to BUILD / CODING for paired regressions and shared root repairs;
progress remains 27/29.
V25 findings now have paired failing-first regressions and shared repairs: every local Activity
entry is normalized before memory and persistence, hydration rewrites safe bytes, URL components
decode before credential checks with Slack/Discord/Telegram bearer paths covered, and mirrored
SQL/decorator families fail closed. The focused Rust privacy test and five targeted frontend tests
pass. State advanced through DIFF to QA / RUNNING for V26 full verification; progress remains
27/29. V26 full Rust and OpenSpec verification pass, while full frontend verification exposed one
obsolete project-instruction Activity assertion that still expected raw private paths. Runtime
redaction matches the approved privacy contract; the fixture is corrected and frontend
reverification passes 200/200 tests, zero Svelte diagnostics, and the production build. V26 also
passes strict format/Clippy, npm audit with zero vulnerabilities, 14/14 Factory MCP protocol tests,
the exact inventory and router boundaries, strict OpenSpec 21/21, and diff/scope checks. The exact
source is ready to freeze for independent code, security, and goal-backward audits; progress
remains 27/29. Frozen V26 goal-backward review passed 21/21 requirements and 47/47 scenarios with
zero Critical/High/Medium findings. Code/security review found one High and consolidated Medium
privacy gaps: common bearer-token values and generic opaque webhook paths, additional raw
SQL/source/CSS forms, and unsanitized additive/receipt fields in legacy Activity entries. State
returned to BUILD / CODING for one paired-regression shared-validator repair cycle; progress
remains 27/29. V26 findings now have failing-first backend/Activity regressions and shared root
repairs: bounded common bearer-token recognition, structural webhook-path detection, mirrored
SQL/source/CSS shapes with reviewed prose controls, and closed Activity reconstruction across
hydration, new writes, persistence, generic receipts, and MCP projection. Display-only local
project paths retain only a safe basename label; canonical generic mutation receipts preserve their
spec-required exact destinations while redacting unsafe content. Focused Rust privacy and six
frontend Activity tests pass. State advanced through DIFF to QA / RUNNING for V27 full
verification; progress remains 27/29. Fresh V27 QA passes 844 Rust library tests with 5
intentional ignores, 2 binary tests, 0 doctests, strict format/Clippy, 201 frontend tests, zero
Svelte diagnostics, production build, npm audit with zero vulnerabilities, 14/14 Factory MCP
protocol tests, exact inventory and router boundaries, strict OpenSpec 21/21, and clean
dependency/scope/diff checks. Prior 375/1440 keyboard/axe evidence remains applicable because V27
does not change Factory control-room or review interaction structure; the Activity change replaces
only persisted display strings and is covered by mount/focus regressions. A frozen V27 code,
security, and goal-backward audit is next; progress remains 27/29.
Frozen V27 goal review passed all 21 requirements and 47 scenarios. Code/security review found one
High common-token family gap plus Medium opaque-webhook character, SQL/source/CSS, generic exact
receipt-destination, and fail-closed Activity-envelope gaps. State returned to BUILD / CODING for
paired failing-first backend/Activity regressions and shared root repairs; progress remains 27/29.
V27 findings now pass the paired regressions after shared fixes for common bearer families,
URL-valid opaque webhook tokens, maintenance/type SQL, namespace/nested-CSS source, fail-closed
Activity envelopes/MCP projection, and exact safe generic destinations through 4096 characters.
Focused QA passes 1/1 Rust and 6/6 frontend tests. State advanced through DIFF to QA / RUNNING for
the complete verification matrix; progress remains 27/29.
Fresh V28 QA passes 844 Rust library tests with 5 intentional ignores, 2 binary tests, 0 doctests,
strict format/Clippy, 203 frontend tests, zero Svelte diagnostics, production build, npm audit with
zero vulnerabilities, 14/14 Factory MCP protocol tests, exact 137/seven inventory, raw-router and
router-composition checks, strict OpenSpec 21/21, and dependency/scope/diff checks. Local `main`
remains at the shared base with its unrelated dirty changes untouched; no archive exists. Prior
375/1440 keyboard/axe evidence remains applicable because V28 changes only shared metadata and
Activity journal normalization, with current mount/focus regressions passing. Frozen V28 code,
security, and goal-backward audits are next; progress remains 27/29.
Frozen V28 code, security, and goal-backward audits reported zero Critical/High findings but five
Medium privacy-classifier gaps: common standalone credential families, literal and encoded webhook
path separators, generic SQL DDL, and C++ inline namespaces. One Low finding requires calendar-
strict Activity timestamps. State returned to BUILD / CODING for one generalized shared-validator
repair with paired failing-first regressions; progress remains 27/29.
V28 findings now pass paired regressions after centralized credential-family matching, RFC-valid
embedded webhook scanning with decoded multi-segment opaque tails, generalized DDL detection,
inline-namespace parity, and calendar-strict Activity timestamps. Focused QA passes 1/1 Rust and
4/4 frontend tests with zero Svelte diagnostics. State advanced through DIFF to QA / RUNNING for
the complete V29 verification matrix; progress remains 27/29.
V29 QA passed, but the pre-freeze boundary self-audit found lowercase semicolon-free generic DDL
was not covered by the generalized rule. State briefly returned to BUILD; paired Rust/Activity
regressions failed first and now pass while reviewed sentence-case UI prose remains safe. State is
QA / RUNNING for the complete V30 verification matrix; progress remains 27/29.
Fresh V30 QA on the formatted source passes 844 Rust library tests with 5 intentional ignores,
2 binary tests, 0 doctests, strict format/Clippy, 203 frontend tests, zero Svelte diagnostics,
production build, npm audit with zero vulnerabilities, 14/14 Factory MCP protocol coverage, exact
137/seven inventory and router boundaries, strict OpenSpec 21/21, and dependency/scope/diff checks.
Prior 375/1440 keyboard/axe evidence remains applicable because V30 changes only shared metadata
classification and Activity timestamp normalization. Frozen V30 code, security, and goal-backward
audits are next; progress remains 27/29.

## ✅ Nightly Control Plane Program [COMPLETE] (2026-08-18)

The isolated `feat/nightly-control-plane` branch now delivers Unified Review, Project Readiness,
bounded Playbooks, a durable Catalog Change Feed, explicit project subscriptions, Recovery, atomic
security presets, and exact Antigravity/Aider/Windsurf/OpenClaw/Kimi lifecycle coverage. Final
independent audits closed roster integration/removal/retry, passive evidence, managed/clone parity,
security, accessibility, mobile navigation, and narrow-layout defects. Verification passed with 182
frontend tests, 790 executed Rust library tests plus 5 intentional ignores, 2 binary tests, strict
Clippy/format/Svelte/build/diff gates, 3/3 live upstream parity suites, browser E2E/axe coverage for
all eight capabilities at 375 and 1440 CSS pixels, and strict OpenSpec validation. The eight delta
specs are canonical and the completed change is archived at
`openspec/changes/archive/2026-08-18-nightly-control-plane/`. The original `main` checkout remains
untouched and unmerged. Record: `tasks/2026-08/260818_nightly-control-plane.md`.

## ✅ v2.0 Phase 18 — Local Ollama System-Prompt Deployment [COMPLETE] (2026-08-16)

The approved `feat/phase-18-ollama-system-prompt` branch now deploys an exact installable Agent body as an app-owned local Ollama model after explicit revision-bound review. It uses only the fixed default loopback API and already-installed non-cloud bases; create, update, remove, reconciliation, rollback, and prompt-free Activity receipts remain separate from file-tool installs and MCP authority. Verification: OpenSpec 13/13 and strict validation passed, canonical specs validate 12/12, 116 frontend tests and 580 Rust library tests passed with 3 existing environment-gated ignores, 2 binary tests passed, strict Clippy/format/Svelte/build/diff and security gates passed, npm and RustSec reported zero vulnerable dependencies, and a live 98-byte prompt round trip against `qwen2.5-coder:14b` preserved triple quotes/template syntax before removing the unique target with zero recovery residue. Implementation commit: `722e579`. The approved native Windows/Linux and artificial 375px platform waiver remains explicit. Record: `tasks/2026-08/260816_phase18-ollama-system-prompt-deployment.md`.

## ✅ v2.0 Phase 17 — Expert Improvement Coach [COMPLETE] (2026-08-14)

The approved `feat/phase-17-improvement-coach` branch now derives local performance summaries from existing Expert run verdicts, evidence, and waivers. Cohorts require the exact Expert version and ordered quality contract, exclude cancelled and non-terminal runs, and expose no rate or suggestion before five comparable quality verdicts. Eligible summaries report deterministic acceptance, rework/rejection, waiver, and latest-evidence signals without model inference, network activity, telemetry, persistence, or mutation. Verification: OpenSpec 7/7 and strict validation passed, canonical specs validate 11/11, 113 frontend tests and 567 executed Rust library tests passed with 3 existing environment-gated ignores, 2 binary tests passed, and strict Clippy/format/Svelte/build/dependency/diff and safety gates passed. Implementation commit: `64fe1f8`. Record: `tasks/2026-08/260814_phase17-expert-improvement-coach.md`.

## ✅ v2.0 Phase 16 — Drift Notifications [COMPLETE] (2026-08-14)

The approved `feat/phase-16-drift-notifications` branch now adds explicit permission-backed native alerts for newly actionable tracked Agent and Skill drift while the running app is backgrounded. It reuses the existing bounded local reconciliation every 15 minutes, creates a silent complete baseline, deduplicates exact logical identities, retains the prior baseline after partial failure, omits private paths and content, and routes activation to existing review surfaces without repair. Verification: OpenSpec 11/11 and strict validation passed, canonical specs validate 10/10, 111 frontend tests and 567 executed Rust library tests passed with 3 existing environment-gated ignores, 2 binary tests passed, strict Clippy/format/Svelte/build/dependency/diff and safety gates passed. Implementation commit: `0f6a269`. Record: `tasks/2026-08/260814_phase16-drift-notifications.md`.

## ✅ v2.0 Phase 15 — MCP Inventory Manager [COMPLETE] (2026-08-14)

The approved `feat/phase-15-mcp-inventory` branch now extends the existing MCP client command and Settings section with bounded privacy-safe Claude Code/Codex inventory, exact Agency Agents router tool evidence, passive validation, isolated source failures, and one trusted auto-configurable template. Foreign servers remain read-only and unknown tools remain explicitly unavailable; Claude inventory reads only no-follow supported config files and Codex uses only literal `mcp list --json` through the existing no-shell bounded runner. Verification: OpenSpec 15/15 and strict validation passed, canonical specs validate 9/9, 108 frontend tests and 566 executed Rust library tests passed with 3 existing environment-gated ignores, 2 binary tests passed, and strict Clippy/format/Svelte/build/diff and safety gates passed. Implementation commit: `4d4d89d`. Record: `tasks/2026-08/260814_phase15-mcp-inventory-manager.md`.

## ✅ v2.0 Phase 14 — Project Instruction Manager [COMPLETE] (2026-08-14)

The approved `feat/phase-14-project-instructions` branch now inspects four fixed instruction targets beneath exact registered projects and manages bounded versioned app-owned snippets without taking ownership of existing content. The existing Projects detail surface provides composition/removal, complete diff review, explicit revision-bound approval, progress, retained results, accessible announcements, focus restoration, and redacted local Activity. Existing bytes are revalidated, backed up exactly, verified, and atomically changed through the durable filesystem journal; startup recovery rejects unsafe path retargeting and retains honest errors. Verification: OpenSpec 15/15 and strict validation passed, canonical specs validate 8/8, 107 frontend tests and 559 executed Rust library tests passed with 3 existing environment-gated ignores, 2 binary tests passed, strict Clippy/format and Svelte checks passed, and production build, safety, and diff gates passed. Implementation commit: `ccb6408`. Record: `tasks/2026-08/260814_phase14-project-instruction-manager.md`.

## ✅ v2.0 Phase 13 — Portable Workspace Packs [COMPLETE] (2026-08-14)

The approved `feat/phase-13-workspace-packs` branch now exports deterministic path-private Workspace Pack v1 files containing exact Agent and Skill references plus declarative runbook, target, instruction, and MCP requirements. Existing Teams controls provide bounded local inspection, explicit project binding, complete dependency/destination/blocker review, revision-bound approval, cross-domain recoverable apply, retained outcomes, and exact mixed Activity receipts. Legacy Agentfiles now convert to the same read-only review and block ambiguity. Verification: OpenSpec 17/17 and strict validation passed, canonical specs validate 7/7, 105 frontend tests and 547 executed Rust library tests passed with 3 existing environment-gated ignores, 2 binary tests passed, strict Clippy/format and Svelte checks passed, and production build, deterministic/path-private, safety, and diff gates passed. Implementation commit: `2e86fb8`. Record: `tasks/2026-08/260814_phase13-portable-workspace-packs.md`.

## ✅ v2.0 Phase 12 — Post-Action Receipts [COMPLETE] (2026-08-14)

The approved `feat/phase-12-post-action-receipts` branch now records one bounded local receipt for completed Agent bulk install/update/track/uninstall, reviewed batch/collection application, and mixed Agent/Skill safe repair. Receipts preserve every attempted item, exact changed or known planned destination, terminal outcome, and redacted bounded failure detail; failed fresh installs without a returned destination claim no path. Existing completion surfaces deep-link to the exact accessible Activity disclosure without adding a route, modal, backend table, network path, telemetry, notification, or mutation authority. Verification: OpenSpec 15/15 and strict validation passed, canonical specs validate 6/6, 103 frontend tests and 539 executed Rust library tests passed with 3 existing environment-gated ignores, 2 binary tests passed, strict Clippy/format and Svelte checks passed, and production build, dependency, route, safety, and diff gates passed. Implementation commit: `e3a30fb`. Record: `tasks/2026-08/260814_phase12-post-action-receipts.md`.

## ✅ v2.0 Phase 11 — Doctor Health Check [COMPLETE] (2026-08-13)

The approved `feat/phase-11-doctor-health-check` branch now provides one bounded read-only Doctor report for local storage, settings, catalog, Agent and Skill sources, installation truth, tools, MCP clients, and cached update state. Checks fail independently, use honest Healthy/Needs attention/Unavailable classifications, redact copied and visible evidence, and hand off only to existing safe recovery controls. Passive database inspection no longer opens a write-capable schema lock, and mutation-spy tests prove empty and existing app data remain unchanged. Verification: OpenSpec 15/15 and strict validation passed, canonical specs validate 5/5, 91 frontend tests and 539 executed Rust library tests passed with 3 existing environment-gated ignores, 2 binary tests passed, strict Clippy/format and Svelte checks passed, and production build, dependency, route, prohibited-call, and diff gates passed. Implementation commit: `45d8430`. Record: `tasks/2026-08/260813_phase11-doctor-health-check.md`.

## ✅ v2.0 Phase 10 — Unified Task Search [COMPLETE] (2026-08-13)

The approved `feat/phase-10-unified-task-search` branch now reuses the deterministic Agent and Skill rankers behind one bounded local read-only desktop command and exposes results through the existing Cmd+K palette. Results retain exact source identity, provenance, and human-readable reasons; activation opens the existing workspaces without installing, executing, or bypassing approvals. The palette preserves commands and adds debouncing, stale-response rejection, retryable errors, input bounds, keyboard navigation, announcements, and focus restoration. Verification: OpenSpec 13/13 and strict validation passed, canonical specs validate 4/4, 89 frontend and 530 Rust tests passed with 3 existing environment-gated ignores, strict Clippy/format and Svelte checks passed, and the production build and diff gates passed. Implementation commit: `42256d5`. Record: `tasks/2026-08/260813_phase10-unified-task-search.md`.

## ✅ v2.0 Phase 9 — Safe Bulk Repair [COMPLETE] (2026-08-13)

The approved `feat/phase-9-safe-bulk-repair` branch now combines exact tracked outdated and missing Agent and Skill installations into one reviewed repair workflow. Unsafe states remain visible but cannot be selected. Approval is bound to fresh dual-ledger reconciliation plus normalized plan and Skill source-byte signatures; drift causes zero writes. Execution reuses the existing sequential, backed-up exact lifecycle operations, continues after individual failure, records bounded Activity, and reconciles final truth. Verification: OpenSpec 16/16 and strict validation passed, canonical specs validate 3/3, 84 frontend tests and 529 executed Rust tests passed, Svelte check reported 0 errors and 0 warnings, and production build and diff gates passed. Implementation commit: `e36ec10`. Record: `tasks/2026-08/260813_phase9-safe-bulk-repair.md`.

## ✅ v2.0 Phase 8 — Foreground Reconciliation [COMPLETE] (2026-08-13)

The approved `feat/phase-8-foreground-reconciliation` branch now refreshes Agent and Skill installation truth after a root-owned 250 ms focus debounce, reuses each store's in-flight guard, includes registered project paths, performs only local read commands, and preserves stale rows plus existing Retry behavior after failure. Verification: OpenSpec 10/10 and strict validation passed, 74 frontend tests and 529 executed Rust tests passed, Svelte check reported 0 errors and 0 warnings, and the production build and diff gates passed. The existing manual-platform waiver remains explicit. Implementation commit: `af9aa2b`. Record: `tasks/2026-08/260813_phase8-foreground-reconciliation.md`.

## ✅ v2.0 Phase 7 — Guided First Deployment [COMPLETE] (2026-08-13)

The approved `feat/phase-7-guided-first-deployment` branch now continues catalog setup into a deterministic Claude Code/Codex first deployment, recommends only a complete preset, reuses the existing scope and mutation-plan UI, and applies exact references through a bounded revision-bound transaction. Success is shown only after fresh reconciliation confirms every planned Agent; defer and completion use a versioned local marker. Verification: OpenSpec strict validation passed, 69 frontend tests and 529 executed Rust tests passed, Svelte check reported 0 errors and 0 warnings, production build/format/diff gates passed, and the transaction tests prove no-write planning plus file/ledger rollback. The approved manual-platform waiver records 375px timing and native Linux/Windows evidence as UNAVAILABLE. Implementation commit: `7b1c19b`. Record: `tasks/2026-08/260813_phase7-guided-first-deployment.md`.

## ✅ v2.0 Phase 6 — Reliability Gate [COMPLETE] (2026-08-13)

The approved `feat/v2-activation-truthful-state` branch now renders affected application failures semantically, preserves last-known Agent and Skill installation truth through reconciliation failures, exposes safe coalesced Retry, blocks stale mutations, and authorizes filesystem reveal only for existing canonical targets inside backend-derived supported roots. Verification: 65 frontend tests and 528 Rust tests passed, Svelte check reported 0 errors and 0 warnings, production build/format/diff gates passed, Nyquist is compliant, and security has zero blocking threats. The user explicitly waived real 375px geometry and native Linux/Windows evidence; those remain UNAVAILABLE, not green. Implementation commit: `73e7c5d`. Record: `tasks/2026-08/260813_phase6-reliability-gate.md`.

## ✅ SQLite Control Plane [COMPLETE] (2026-08-06)

The approved `feat/sqlite-control-plane` branch now uses a shared transactional SQLite authority
for 17 bounded control-plane documents across the desktop app and MCP processes. Cutover is an
explicit one-time maintenance event with an exclusive process lease, verified private backup,
semantic and cryptographic validation, integrity checking, persistent legacy conflict detection,
and foreground revision refresh. Filesystem mutations use durable recovery journals; package
artifacts and Keychain secrets remain outside SQLite. Verification: 521 backend and 25 frontend
tests passed, strict Clippy/format/Svelte checks and production build passed, and a copied-state
rehearsal preserved all 2,311 package/artifact hashes while importing 17/17 documents. Commit:
`3d3a838`. Record: `tasks/2026-08/260806_sqlite-control-plane.md`.

## ✅ Skill Publishing MCP and Skills UI Fixes [COMPLETE] (2026-08-05)

Local `main` now exposes revision-bound Skill draft publication
requests through the existing authenticated MCP server and desktop approval inbox. It accepts
validated root-level Skill references, rejects stale revisions without leaving approvals Running,
and keeps Agent and Expert publishing on their existing endpoints. The Skills Manage Sources and
Approval Inbox popovers now remain open for inside interactions and close on outside clicks. The
search and filter controls remain inside the package-list column. Same-name app-owned revisions now
replace with a rollback backup, exact approvals reconcile after direct publication, and the inbox
shows one action per revision. The approved flow published all 59 files of
`primavera-p6-eppm-hybrid` in 2.61 seconds. Verification: 480 Rust library tests passed with 2
intentional ignores, 2 CLI tests passed, strict Clippy and format checks passed, frontend check
reported 0 errors and warnings, 22 frontend tests and the production build passed, and
`git diff --check` passed. Record: `tasks/2026-08/260805_skill-publishing-mcp.md`.

## ✅ Create Agent from Skill [COMPLETE] (2026-08-05)

The local `feat/create-agent-from-skill` branch now creates a deterministic editable Agent draft
from an exact validated Skill in both the desktop app and MCP. Generated Agents declare structured
`required-skills` metadata and delegate to the Skill instead of copying it. MCP publication stays
desktop-only through a typed, source-hash-bound approval that rejects stale drafts; successful
publication is re-read through the canonical Agent validator. Verification: 474 Rust library tests
passed with 2 intentional ignores, 2 CLI tests passed, strict Clippy/format/diff checks passed,
frontend check reported 0 errors and warnings, 20 frontend tests and the production build passed,
and live stdio exposed 129 tools: 49 Skills + 51 Agents + 29 Experts. The latest development app
was rebuilt and launched. User approved the final diff; no remote change was authorized. Record:
`tasks/2026-08/260805_create-agent-from-skill.md`.

## ✅ Agents–Skills Feature Parity [COMPLETE] (2026-08-04)

**State**: COMPLETE on local `main`, integrating feature commit `e655d28`.
Stage 1 was approved, applied to the sandbox, and documented in
`tasks/2026-08/260804_agent-foundation.md`. Stage 2 was approved, applied to the sandbox, and
documented in `tasks/2026-08/260804_agent-lifecycle-parity.md`; no commit was created. Its evidence:
434 Rust library tests passed + 2 ignored, 2 CLI tests passed, a copied real pre-feature ledger
migrated without changing either installed destination hash, strict Clippy/format passed, and
frontend check/13 tests/build passed. Stage 3 follows
`docs/superpowers/plans/2026-08-04-agents-mcp.md`: compose Agent tools/resources into the existing MCP
server, add separate default-denied Agent mutation policy, desktop approvals, and durable redacted
audit.

Stage 3 is approved and documented in `tasks/2026-08/260804_agents-mcp.md`. The existing MCP server now
composes the exact 49 Skills and 49 Agent tools, exposes Agent catalog/source/render resources and
subscriptions on stdio and authenticated loopback HTTP, keeps Agent permissions separately
default-denied, and routes overwrite/destructive Agent actions through typed desktop approvals with
plan-revision checks and durable redacted audit rows. Verification: strict format and Clippy passed;
445/445 Rust library tests passed (2 ignored) plus 2/2 CLI tests; frontend check had 0 errors and 0
warnings, 13/13 tests passed, and production build passed; `git diff --check` passed. Live stdio and
HTTP smoke tests each reported 49 Skills + 49 Agents tools, readable Agent resources, successful
Agent reads, default-denied Agent mutation, and HTTP 401 without bearer auth. Stage 4 follows
`docs/superpowers/plans/2026-08-04-agent-parity-integration.md`. Stage 4 now completes the desktop library,
source/draft/organizer/approval flows, Activity coverage, localization/accessibility contracts, recovery and
security rehearsals, and exact parity audit. Final verification: 453/453 Rust library tests passed with 2
intentional external-fixture ignores, 2/2 CLI tests passed, strict Rust format/Clippy passed, frontend check
reported 0 errors and 0 warnings, 19/19 tests passed, production build passed, and `git diff --check` passed.
Live stdio initialization exposed exactly 49 Skills + 49 Agent tools with no duplicates. Host Phase C passed
6/6 including the macOS release build. The optional renderer clone and Ubuntu VM were absent; the online
Windows 11 VM was not a configured runner because it had no shared repo, Node, Rust, or Build Tools, so no
Windows pass is claimed. User approved the final diff; APPLY and DOCS are complete. The integrated
record is `tasks/2026-08/260804_agents-skills-feature-parity.md`. Post-approval verification repeated
the full Rust and frontend suites successfully, and live stdio exposed 98 unique tools: exactly 49
Skills + 49 Agents. Local integration with Expert Release 1 passed 468 Rust library tests with 2
intentional ignores, 2 CLI tests, 19 frontend tests, production build, strict Clippy/format/diff,
and a live inventory of 127 unique tools: 49 Skills + 49 Agents + 29 Experts. No remote change was
authorized.

## ✅ Expert MCP Lifecycle — Release 1 (2026-08-03)

The `feat/experts-hub` branch now provides the human-approved Expert lifecycle for Claude Code and
Codex: discovery, portable create/update/clone/archive/delete proposals, activation requests,
quality-contract runs, idempotent evidence, blockers, waivers, and desktop review. MCP mutations
reuse the existing default-denied policy/audit boundary; callers are scoped by client and canonical
registered project, while terminal verdicts remain desktop-only. Verified: Rust 411 passed / 1
environment-gated parity test ignored, Svelte check 0 errors and 0 warnings, production frontend
build green, native macOS Tauri debug build green, and diff/format checks clean. User approved the
implementation. Task: `tasks/2026-08/260803_expert-mcp-release1.md`.

## ✅ MCP Skills Platform (2026-07-30)

The `feat/skills-library` branch now exposes the validated Skills library to Claude Code and Codex
through 22 MCP tools and package resources. Read operations are available by default; source,
install, and destructive mutations require explicit settings policy and exact canonical project
allowlisting. Both stdio and bearer-authenticated loopback HTTP reuse one server, lifecycle core,
durable audit boundary, and capability-relative project filesystem operations. Managed drafts
require desktop approval to publish. Verified: Rust 379 library + 2 CLI passed, live stdio/HTTP
exposed 22 tools, unauthorized HTTP returned 401, Svelte check 0 errors, production build green,
and independent spec/quality/security audit passed. User approved the implementation.
Task: `tasks/2026-07/260730_mcp-skills-platform.md`.

## ✅ Skills Phase 5 — Project and App Integration (2026-07-30)

The `feat/skills-library` branch now completes the five-phase Skills Library milestone. Project
cleanup removes only tracked project-scoped skills and leaves user-scope installs untouched.
Source, install, update, disable, enable, uninstall, and failure outcomes are recorded in Activity.
Skills workspace copy is routed through the existing English baseline plus partial-locale fallback
catalog, and the content scan found no remaining embedded Skills UI labels. Verified: Rust 305
passed / 1 ignored, Svelte check 0 errors, production build green, native Tauri debug bundle
created, and diff audit clean. User approved the implementation.
Task: `tasks/2026-07/260730_phase5-project-app-integration.md`.

## ✅ Skills Phase 4 — Managed Skill Lifecycle (2026-07-30)

The `feat/skills-library` branch now completes the tracked skill lifecycle: Update repairs Missing
or advances Outdated installs; Disable/Enable use reversible same-filesystem directory moves;
Uninstall confirms intent and preserves modified content under `skill-backups/`. Removing a source
does not touch installed directories or ledger rows—affected installs remain visible as
SourceUnavailable. Backup paths are visible in the Skills workspace, foreign destinations remain
immutable, and lifecycle hashing rejects linked/reparse-point roots. Verified: Rust 305 passed /
1 ignored, Svelte check 0 errors, production build green. User approved the implementation.
Task: `tasks/2026-07/260730_phase4-skill-lifecycle.md`.

## ✅ Skills Phase 3 — Transactional Skill Installation (2026-07-30)

The `feat/skills-library` branch installs exact validated multi-file skills into Claude Code and
Codex user or project destinations. A dedicated atomic ledger reconciles Current, Outdated,
Modified, Missing, Foreign, Disabled, and SourceUnavailable. Installation rejects linked paths,
never overwrites foreign or modified content, and uses staging, backup-first managed replacement,
atomic publication, and rollback. Agent and skill deployment now reuse one destination grid.
Phase 4 retains update, disable, enable, uninstall, source lifecycle, and backup-management UI.
Verified: Rust 301 passed / 1 ignored, Svelte check 0 errors, production build green. User approved
the implementation. Task: `tasks/2026-07/260730_phase3-transactional-skill-installation.md`.

## ✅ Skills Phase 2 — Inspectable Skills Workspace (2026-07-29)

The `feat/skills-library` branch now provides a browse-first, read-only Skills workspace:
search plus Ready/Rejected/source filters, persistent package detail, provenance, validation
diagnostics, exact file inventory, and coarse Claude Code/Codex destination presence. Workspace
loading reads registered local folders and active Git checkouts without network refresh. Install
and lifecycle actions remain Phase 3/4. Verified: Rust 294 passed / 1 ignored, Svelte check 0
errors, production build green, native Tauri launch successful. User approved the implementation.
Task: `tasks/2026-07/260729_phase2-inspectable-skills-workspace.md`.

**State**: 🚀 **v0.2.0 SHIPPED (2026-06-23)** — `main` @ `16182e5`. First feature release since the v0.1.0
launch (the internally-tracked "0.1.1"/"0.1.2" milestones were never cut separately — they ship here), and
**auto-update is now LIVE** at [`agencyagents.app/updater.json`](https://agencyagents.app/updater.json) for
**both Mac arches** (`darwin-aarch64` + `darwin-x86_64`). Release at
[releases/tag/v0.2.0](https://github.com/msitarzewski/agency-agents-app/releases/tag/v0.2.0): **9 assets** (macOS
aarch64+x64 signed/notarized DMGs **+ updater tarballs**, Linux deb/rpm/AppImage, Windows x64/arm64). Homebrew:
`brew tap msitarzewski/agency-agents && brew install --cask agency-agents` (cask @ 0.2.0). Cross-platform CI in
`.github/workflows/` (linux-build, windows-build) fires on `v*` tags; macOS DMGs build locally via
`scripts/release.sh`. Full ship log: `agentLog.md` 2026-06-23; task doc `tasks/2026-06/260623_v0.2.0-ship.md`.

**Workflow (from 2026-06-16):** ALL changes go through a **branch → PR → merge to `main`**. No direct commits to main.

## ✅ v0.2.0 — first feature release + LIVE auto-update — SHIPPED (2026-06-23, PRs #21 + #22; `main` @ 16182e5)
- **Auto-update is on.** Endpoint `agencyagents.app/updater.json` (Caddy on `umacbookpro` from `~/Sites/agency-agents/`,
  sibling vhost to the live `brew-browser.zerologic.com` manifest). **Dedicated agency signing key `ABF5AFD8`**
  (embedded pubkey in `tauri.conf.json`; private key + password in the **macOS Keychain**, services
  `agency-agents-updater-key` / `…-key-pw`; canonical key file backup at `~/.config/agency-agents-app/updater.key`).
  Live path = check → notify → one-click install; full hands-off auto-install is still deferred (the
  "Install updates automatically" toggle ships **present-but-disabled**).
- **Release build gotchas (now fixed + documented in `BUILD.md` / `release.sh` / PR #22):**
  - Updater-on macOS builds **must pass a `--config`** flag — the macos-private-api allowlist check reads only
    base `tauri.conf.json`, so the split `tauri.macos.conf.json` (`macOSPrivateApi:true`) is invisible to it
    (tauri#11142). `release.sh` now always passes `--config '{"app":{"macOSPrivateApi":true}}'`. (Every old
    `SKIP_UPDATER` build worked only because it happened to pass a `--config`.)
  - **Intel cross-compile needs the rustup toolchain** — Homebrew's `rust` is host-only (`can't find crate for
    core`). Build with `PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"`.
  - **Store the updater Keychain key via `$(cat …)`**, never a manual paste — a trailing newline corrupts it and
    signing fails with `incorrect updater private key password: Invalid input`.
- **Asset naming uses underscores** (`Agency_Agents_0.2.0_*`), NOT v0.1.0's auto-sanitized dots — the live updater
  manifest and the brew cask url both depend on this.
- **Post-ship UX (PR #23):** the Agents pane now has a **"Needs attention" filter** (= Outdated ∪ Modified ∪
  Missing). The install-state lens moved into nav (`ui.agentsLens`) so the Dashboard "N need attention" stat +
  health-donut segments **deep-link** to a **flat all-divisions** filtered list (`showDivisions` gates on
  `lens === "all"`); cross-launch lens persistence dropped (would hijack the landing).
- Decisions: `decisions.md` 2026-06-22 (host + dedicated key) and 2026-06-23 (build mechanics). Gates: cargo 264/0,
  svelte-check 0, signed+notarized, CI green.

## ✅ v0.1.2 — Tool registry + Osaurus + Playbook + Projects dashboard — SHIPPED (2026-06-21, PRs #18 + #19; `main` @ 1df932c)
**Tool knowledge is now a single source of truth.** It consolidated to the single canonical `tools.json` the
upstream `agency-agents` repo OWNS (twin of `divisions.json`, CI-guarded by its no-jq `check-tools.sh`); both the
Rust backend (`registry.rs`, `include_str!`) and the frontend (`toolRegistry.ts`) read it. **The Rust `Tool` enum
is GONE** — a tool is a string id; `label`/`detect`/`version`/`dests`/`scope` are registry lookups; `render()`
dispatches on the JSON `format` key. Frontend deleted ACCENTS/ICONS_SVG/SHORT/hardcoded SUPPORTED_TOOLS.
**Adding a tool = editing one JSON file** (+ a Rust formatter only for a brand-new output format).
- **All 13 tools** modeled (incl. Kimi, Osaurus); Tools panel shows installable + recognized-only (dimmed). Real
  brand logos (Lobe Icons, MIT) under `assets/tools/`, letter fallback otherwise.
- **Installability derived, not stored:** `installable(tool) = format ∈ IMPLEMENTED_FORMATS` ({identity,
  codex-toml, gemini-md, qwen-md, cursor-mdc, opencode-md, skill-md}). aa owns upstream truth; renderer coverage
  is app-side + self-maintaining (ship a renderer → add its format → those tools light up).
- **Osaurus wired** via a `skill-md` format (Agent-Skills `SKILL.md`, `slugPrefix:"agency-"`), byte-identical to
  upstream `convert_osaurus` — contributed UPSTREAM first (catalog owns transforms), mirrored here (parity test).
  Verified live: catalog agents run as native Osaurus skills.
- **Playbook** (in-app practices + copyable starter prompts + per-team/division examples; title-bar 📖 + ⌘K) +
  `docs/USING-AGENTS.md`. **Teams & Projects master/detail** via the system back arrow
  (`ui.projectsSelected`/`teamsSelected`); **division overview** + deploy.
- **Dashboard**: two-ring Global-vs-Projects install **sunburst** (`InstallSunburst.svelte`) so totals reconcile;
  cross-tool coverage **merged** with catalog-by-division (linked hover); uniform donuts, equal-height cards.

Task doc: [tasks/2026-06/260621_tool-registry-12-tools-osaurus.md](tasks/2026-06/260621_tool-registry-12-tools-osaurus.md).
Green: cargo 264/0, svelte-check 0, build clean. Upstream `agency-agents` (same machine): Osaurus transformer +
`tools.json` + `check-tools.sh` + `check-tools.yml` landed (aa PRs #605/#606).

## ✅ v0.1.1 IA arc — SHIPPED (2026-06-17 → 06-20, PRs #15 + #16, + the deploy-browser PR)
The whole "how people think about agents" reorganization landed:
- **Divisions landing** — the Agents tab opens on divisions; select-mode → bulk deploy.
- **Install-state lens** — filter the agent list by deployment state (In sync / Outdated / Untracked / Missing /
  Not installed), counts scoped to the division.
- **Teams** (renamed from Loadouts) — "Your team" (current installs, division-grouped) + "Team presets"
  (app-bundled `presetTeams.ts` + your saved teams, `teams.svelte.ts`).
- **how × where engine** (backend, PR #15) — tools are dual-scope; `render::dests()` scope-aware;
  `supports_user()`/`supports_project()`; install scope derived from the chosen project. Verified tool-path
  matrix (June 2026) in the PR. Cursor is project-only; Windsurf/Aider/Antigravity/openclaw deferred.
- **Projects pillar** (4th nav section, ⌘4; Activity → ⌘5) — `projects.svelte.ts` store (registered roots in
  localStorage ∪ the live ledger), `Projects.svelte` panel with rosters.
- **One `InstallModal`** — the destinations × tools GRID (rows = Global + each project + "Add project…",
  columns = detected tools, cells = tri-state toggles). Reused by agent detail, divisions, Teams, Projects.
  Replaced `DeployModal` + the inline switch-matrix. Agent detail: "Install…" in the title, pills on their own row.
- **Two-pane `DeployBrowser.svelte`** (Projects "Deploy…") — System-Settings master/detail: left =
  searchable list of EVERY granularity (agents · divisions · teams incl. saved · current roster); right =
  per-project per-tool install.

**Four-pillar model (drives IA copy):** Agents = *who* · Tools = *how* · Teams = *which* · Projects = *where*.

## 🔵 Backlog (next — full list in `docs/PLAN.md` post-0.2.0 punch list)
- **Opt-in automatic install** — the v0.2.0 "Install updates automatically" toggle is inert; wire it to a real
  off-by-default background download → verify → install (live updater is check → notify → one-click install today).
- **Refresh `tools.json` from the catalog clone** (vs. the bundled baseline) — like the corpus + `divisions.json`;
  cleaner now that aa #605 prunes stale convert output.
- **Foreign-sweep for nested skill dirs** (`…/<dir>/SKILL.md`) so CLI-installed Osaurus/Antigravity skills are
  detected (app-installed ones already are).
- **Antigravity wiring** once upstream makes its skill deterministic (drops the non-deterministic `date_added`).
- **"Auto Updates" subscription** for bulk installs — installing all of a division/team into a project/tool
  offers to auto-deploy newly-added catalog agents.
- **Copilot `.md` → `.agent.md`** (needs reconcile `file_stem` double-extension handling).
- Optionally tighten backend `detect()` to require the tool **binary**, not just a lingering config dir.
- Bonus: a "scaffold AGENT-ZERO into a project" action (every assistant honors repo-root `AGENTS.md`).

**Dev-harness note:** to screenshot-verify the Svelte frontend in a browser (the native Tauri window can't be
auto-driven), a `?shim=1` Tauri-IPC shim is temporarily injected into `src/app.html` then reverted — it never
ships. The shim can't open a real native folder dialog (returns a fixture path), so "Add project…" looks broken
in the shim though it works natively.

**Last updated**: 2026-06-23

## ✅ Pre-release polish (2026-06-15) — committed + pushed on `release-planning`
- **brew vestige cleanup**: error-type rename (`BrewError*`→`AppError*`), removed dead `catalogAutoRefresh`
  setting, removed the dead error codes (`brew_*`, `job_not_found`, `canceled`, `feature_disabled`,
  `vulns_not_installed`), and **deleted the brew-era Python pipeline** (`tools/{catalog,categorize,enrich,
  pipeline,trending-collector}` — they fetched Homebrew formulae, NOT used by AA; the catalog comes from
  `corpus/mod.rs`).
- **Activity Journal** (replaces the inherited, permanently-empty brew streaming "Activity"): pivoted
  `activity.svelte.ts` to a `JournalEntry` store (localStorage), `install.svelte.ts` logs every
  install/uninstall/update/track/bulk + default-target switch, `ActivityHistory.svelte` rewritten as a
  day-grouped clearable journal. Deleted `ActivityDrawer.svelte` + `AppStreamEvent`/`ActivityJob` types.
  Built via a Workflow (planner→builder→Code-Reviewer+UX-Architect team→fix loop); UX nits hand-polished.
- **Tools pane lens**: defaults to **Installed** (detected/in-use) tools; toggle `Installed · Not installed
  · All` (top row beside rescan, no count chips). `ToolsView.svelte`. Bar = **catalog coverage**
  (green installed / gray rest), not sync-state.
- **Agents workspace streamlined**: removed the filter lens (per-row install dots already show count);
  Division dropdown moved onto the search row as the first element (neutral form styling); detail pane
  hidden when no agent is selected (list goes full-width).
- **Cold `cargo test` tauri-gate fix**: `.cargo/config.toml` feeds `TAURI_CONFIG` so bare cargo (tests/CI)
  passes the `macos-private-api` allowlist gate (Tauri CLI overrides it for real builds). `macos-private-api`
  enabled in `Cargo.toml`. Verified `tauri dev` still launches clean.
- **Cross-platform creds FIXED + VM-validated**: GitHub token now persists to the OS-native vault per
  platform (Keychain / Credential Manager / Secret Service) via per-target `keyring` features; also moved
  `macos-private-api` to `[target.macos]` only (was wrongly in base deps → broke the Linux gate). Built +
  tested on Ubuntu (258/0 + deb/rpm/appimage) and Windows x64 via `phase-c.sh` VM matrix.
- **Dead-code/brew pass**: removed dead `agentsFilter` lens plumbing; scrubbed ALL brew comment mentions
  (grep → none); zero cargo dead_code warnings.
- **UX**: adaptive Uninstall/Delete wording by ownership; OS-style click-outside menu dismiss; Tools detail
  closes when the lens hides the tool; CoverageMatrix shades by **coverage-%** (not raw size).
- **Terminology**: user-facing **Category → Division** (catalog repo's term); internal `category` field kept.
- **Dashboard viz DONE**: replaced the cross-tool matrix with **CoverageDonuts** (one donut per tool,
  sliced by division, shared legend, linked hover); established a curated **division color scheme** as catalog
  metadata (PR github.com/msitarzewski/agency-agents/pull/592 = `divisions.json`) read via `corpus.colorOf`;
  Dashboard "Coverage by tool" click now selects the tool (`ui.openTools`). **`CatalogByDivision.svelte`** (NEW)
  replaces the orange bar-list: ONE proportional bar (segment per division, brand-colored), labels across FOUR
  lanes (2 top, 2 bottom) tied to segments by **non-crossing Z-elbow leaders** (rank-staggered rails +
  phase-shifted bottom columns), plus CoverageDonuts-style **linked hover** (dim others). Division **icons
  tinted** with their color in the `Division ▾` dropdown + persona pill (added `corpus.iconOf`); `categoryIcon.ts`
  gained `Map`+`Workflow` so gis/integrations stop falling back to "?". See `agentLog.md` 2026-06-15 (later 4).
- **Green throughout**: svelte-check 0 errors, cargo 258/0 (macOS + Linux), config validation all-pass.

## ✅ Phase C (2026-06-14) — both red items closed
- **Renderer parity VERIFIED.** `render/mod.rs` mirrors the upstream shell converter byte-for-byte
  (`source_field`/`source_body`/`slugify`/`output_slug`); new `--ignored` test diffs the real
  `scripts/convert.sh` → **232 agents × 5 transform tools = 1160/1160 byte-identical**. The
  `current`/Diff/Update model is now proven, not assumed.
- **Uninstall safety RESOLVED.** `remove_agent_files` backs up modified files FIRST (separate pass),
  byte-identical files need no backup, backup failure aborts the delete (original preserved). Tests cover
  every path.
- **Cross-platform chrome DONE.** Config split: base `tauri.conf.json` (decorations, opaque, no
  macOS-only keys) + `tauri.macos.conf.json` override (overlay titlebar/traffic-light/transparency).
- **Cleanup:** brew→Agency rename finished in `lib.rs`; dead `Settings` fields purged; docs overhauled;
  stale release notes removed; new `tools/phase-c/` validation runner. **Catalog now = 232 agents**
  (the re-org landed). Green: cargo 258/0 + parity 1/0, svelte-check 0, build clean.

## 🟣 Tahoe app icon (read first if touching icons)
macOS 26 renders icons from a compiled **`Assets.car`** (Icon Composer Liquid Glass), NOT `.icns` — `.icns`
-only = blank/gray squircle ("icon jail"). FIXED: `actool` (full Xcode only, by path) compiles
`docs/icon/AppIcon.icon` → `src-tauri/Assets.car` (in `bundle.resources`) + Tahoe-aware
`src-tauri/icons/icon.icns`; `src-tauri/Info.plist` adds `CFBundleIconName=AppIcon` (Tauri merges it).
**Don't run `npm run tauri icon`** (clobbers the glass icns). Full recipe: `docs/icon/README-liquid-glass.md`.
Dev Dock hack REMOVED (lib.rs plain `.run()`, objc2 deps dropped).


## Current state (read NEXT-SESSION.md for the full picture + IMMEDIATE backlog)
- **Phase B + nav + Tools (2026-06-09):** Dashboard has 4 dependency-free charts (`HealthDonut`,
  `CoverageMatrix` category×tool, coverage-by-tool bars, category distribution). **Back/forward nav**
  (titlebar ◀▶, ⌘[/], mouse 3/4) over a `ui` NavLocation history; `agentsCategory`+`agentsSelected`
  lifted into `ui`. **Division pills deep-link** everywhere (`ui.openDivision`); lens counts narrow to
  the division; added "Not installed" lens; zero-count lenses/stats hide. **Tools = list/detail console**
  (`ToolsView` rebuilt): badges (`util/toolBadge`), health bars, versions (`tool_versions`), Reveal
  (`reveal_path`), Default-target Switch, Sync-to-catalog/Track-all/Remove-all, projects list. Dev Dock
  icon set on `RunEvent::Ready` (macOS debug). Icon redrawn as a **macOS squircle** (regenerated).
- **UNIFIED Agents workspace (Phase A done).** Agents + Library are ONE three-pane surface
  (`AgentsWorkspace.svelte`): list pane (filter lens All/Installed/Needs-attention/Untracked + search +
  Category ▾ + Select-mode bulk) · `ResizeHandle` · persistent detail pane (`PersonaBody` + the
  `DeploymentMatrix`). `PersonaDiscover.svelte` + `AgentLibrary.svelte` DELETED.
- **Deployment band under the name/division**: summary pills for installed tools + a "USE WITH ⌄"
  disclosure. User tools = `Switch` (on=installed); project tools = Install/Add-project + per-project
  sub-rows. Drift actions (Diff/Track/Update) inline when applicable. New `Switch.svelte` (shared,
  extracted from Settings→Network), `util/platform.ts` (⌘/Ctrl shortcut glyphs).
- Nav: `library` section retired everywhere; `ui.agentsFilter` + `ui.openAgents(filter)` deep-link
  (Dashboard cards + palette use it). Section id stayed `personas`.
- **Byte-identical foreign → `current`**; **recursive indexing**; `agent_diff` + `DiffModal`; Track (safe).
- Active catalog = **userClone** `/Users/michael/Software/AgentLand/agency-agents` (manage:true).
- **Signed + notarized `.app`/`.dmg`** via `scripts/release.sh` (SKIP_UPDATER=1). 247 Rust tests / 0.
- 🔵 NEXT: **Phase B** = 4 Dashboard charts (coverage matrix · health donut · category distribution ·
  per-tool coverage), dependency-free SVG/CSS, cells deep-link into the workspace. Then **Phase C** =
  Windows/Linux titlebar degradation + "this device" copy + home-path display.
- ✅ CLOSED 2026-06-14: (1) **renderer parity** vs convert.sh — VERIFIED 1160/1160 byte-identical;
  (2) **uninstall safety** — RESOLVED (backup-first for modified, none for byte-identical, abort-on-fail).

## (historical) Earlier this arc
- **Adopt → Track**: destructive Adopt gone. `track_agent` records provenance, writes nothing; every
  write backs up first (`<app_data>/backups/`); `agent_diff` for review-before-Update.
- **categories from tooling**: `discover_categories` parses `AGENT_DIRS` from
  `scripts/convert.sh`. **Data fix: `integrations` (convert.sh output) dropped (210→209); `strategy`
  added.** Removed the orphan `integrations/backend-architect-with-memory.md` from the baseline (it's
  a valid-but-misfiled enrichment example; to ship it for real, promote it UPSTREAM into a real
  category — then it flows in via refresh).
- **#1 slices 2–4 — catalog source**: `CatalogSource` (Bundled | Managed{~/.agency-agents} |
  UserClone{path,manage}) in `state/catalog.json`; corpus reads/writes the RESOLVED root. Detect
  (~/.agency-agents + "Find" scan), provision (git clone or snapshot), pull (git pull or tarball).
  First-run picker (`CatalogFirstRun`) + `Settings → Catalog`. Verbs Track/Update, manage-with-
  permission, picker+Find — all as decided. cargo test 275/0; svelte-check 0 err; build green.
- ⚠️ NOTE: existing installs (incl. Michael's) have no catalog.json → the **first-run picker WILL
  appear** on next launch (by design — one-time source choice; pick "Bundled" to keep current).

> Full plan + sequence: `phases/phase-roadmap.md` (the "v2" block). Detailed resume notes +
> gotchas: `NEXT-SESSION.md`. Build spec: `contracts.md`. Architecture: `systemPatterns.md`.

## How to run (dev)
- `npm run tauri dev` from repo root. **Dev server is on port 1430** (NOT 1420 — that's
  brew-browser; sharing it makes one app load the other's frontend). HMR for frontend; Rust changes
  recompile. The app opens on **Agents** (personas).
- Reference clones (read-only): `/tmp/brew-browser-inspect`, `/tmp/agency-agents-inspect`.

## What works (verified)
- **Agents** catalog (210 agents / 16 categories), search, persona detail with an **Install** menu.
- **Library** — flat list of installs; your ~184 `install.sh` agents show as `foreign` with Adopt.
- **Tools**, **Loadouts** (Agentfile), **Dashboard** (agency rollup), Activity, Settings (⌘,).
- Backend: `corpus · render · install · github · util · commands{github,settings,updater}`.
  `cargo test` ~265/0; `vite build` + `svelte-check` green; app boots clean (210 corpus seeded).
- New brain-circuit **app icon** (dark shipped; light master in `docs/icon/`). About window rebranded.

## Immediate next: Michael runs it, then #2 / #3
**#1 slices 1–4 ✅ done.** Remaining for #1 (deferred refinements, non-blocking):
- `aliases.json` (slug renames across catalog versions) — not yet honored.
- Explicit **orphan** surfacing (ledger rows whose slug left the catalog) + unique-slug enforcement.
- `.agency-cache/` convention + add to the agency-agents repo `.gitignore` (cache not yet written).
- Symlink-aware reconcile (the `~/.claude` alias case) — still the old behavior.

Then: **#2 Track-all / Update-all**; **#3 tool-grouped Library IA** (L1 tools+counts → L2 per-tool)
+ wire `agent_diff` into a review-before-Update UI.

## Decisions locked (this session)
- Build order: **Both, Track first** → Track DONE, now #1.
- Clone detection: **picker-primary + a "Find Agency Agents" button** (opt-in scan, not auto).
- Existing clone: **manage-with-permission**. Managed path: **`~/.agency-agents`**.
- Cache dir: `.agency-cache/`. Verbs: **Track / Update**.
- Categories: **parse from repo tooling** (`AGENT_DIRS` in convert.sh), not a frontmatter heuristic.

## ✅ RESOLVED: "Adopt" is no longer unsafe
Adopt → **Track** (non-destructive) + backup-on-write shipped this session. The old clobber path is
gone.
