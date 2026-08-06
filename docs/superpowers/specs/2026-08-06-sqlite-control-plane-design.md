# SQLite Control Plane Design

**Status:** Approved design
**Date:** 2026-08-06
**Branch:** `feat/sqlite-control-plane`

## Purpose

Make the desktop application and its independent Claude/Codex MCP processes share one reliable,
transactional source of truth for Skills, Agents, Experts, approvals, drafts, installations, and
history without moving or rewriting portable package files.

The user-visible outcome is simple: a change made through MCP appears in the desktop app within one
second, concurrent mutations do not overwrite each other, and an interrupted migration or package
operation is recoverable without losing data.

## Decisions Locked by the User

- Use SQLite rather than a document database.
- Keep Skill packages, Agent Markdown, scripts, references, assets, PDFs, Git checkouts, and
  published package directories on the filesystem.
- Keep the Rust backend as the sole database owner; Svelte continues to use Tauri commands.
- Preserve exact source-aware identities, hashes, destinations, IDs, approval boundaries, and MCP
  authorization behavior.
- Remain offline-first with no accounts, telemetry, or cloud replication.
- Import existing data without loss and leave package files unchanged.
- Retain JSON as an explicit backup/import/export format, not a second live source of truth.
- Deliver incrementally; do not add relational normalization or FTS until measured use requires it.

## Current Architecture and Reuse

The existing backend already supplies the domain rules that must remain authoritative:

- `src-tauri/src/util/fs.rs` owns bounded reads and atomic filesystem replacement.
- `src-tauri/src/skills/mod.rs`, `drafts.rs`, `organize.rs`, and `install.rs` validate Skills,
  trust, sources, draft publication, organization, approvals, and install lifecycle.
- `src-tauri/src/agents/mod.rs`, `drafts.rs`, `organize.rs`, and `install/mod.rs` provide the Agent
  equivalents.
- `src-tauri/src/experts.rs` and `expert_runs.rs` validate Expert definitions, requests,
  activations, contracts, evidence, blockers, and reviews.
- `src-tauri/src/state.rs` owns the application-data root, network/MCP policy, and durable audit.
- `src-tauri/src/lib.rs` is the stable Tauri command boundary.

The migration extends these loaders, validators, and transaction paths. It does not introduce a
generic repository layer or duplicate domain validation inside SQL.

## Chosen Architecture

```text
Svelte UI                 Claude/Codex MCP
    |                           |
    +------ existing commands --+
                    |
              Rust domain code
                    |
         BEGIN IMMEDIATE mutation
                    |
   state/agency-agents.sqlite3       filesystem artifacts
   - app_meta                        - SKILL.md + assets
   - state_documents                 - Agent Markdown
   - filesystem_operations          - published packages
   - legacy_imports                  - Git checkouts
                    |                - install destinations
                    +---- paths + hashes ----+
```

SQLite stores versioned, size-capped JSON documents that reuse the current Serde DTOs and semantic
validators. This first cut delivers one writer-serialized control plane without a large relational
rewrite. Later releases may extract measured query hotspots into normalized tables or FTS5.

### Database ownership

- Database path: `<app-data>/state/agency-agents.sqlite3`.
- Rust opens every connection with the same flags and pragmas.
- No frontend JavaScript receives a database path or SQL capability.
- Blocking SQLite work runs through `tokio::task::spawn_blocking`.
- One operation opens one connection; no connection or transaction crosses `.await`, network, or
  filesystem work.

### Initial schema

```sql
CREATE TABLE app_meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
) STRICT;

CREATE TABLE state_documents (
  name TEXT PRIMARY KEY,
  document_version INTEGER NOT NULL,
  revision INTEGER NOT NULL,
  payload TEXT NOT NULL CHECK (json_valid(payload)),
  updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE filesystem_operations (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  phase TEXT NOT NULL CHECK (phase IN ('prepared','filesystem_applied','committed')),
  payload TEXT NOT NULL CHECK (json_valid(payload)),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE legacy_imports (
  name TEXT PRIMARY KEY,
  relative_path TEXT NOT NULL UNIQUE,
  was_present INTEGER NOT NULL CHECK (was_present IN (0, 1)),
  size_bytes INTEGER NOT NULL,
  sha256 TEXT NOT NULL,
  imported_at TEXT NOT NULL
) STRICT;
```

`PRAGMA user_version` versions the SQL schema. `app_meta` contains the migration state and one
monotonic user-visible revision. A missing document row is different from a present document whose
payload is an empty collection.

### Connection contract

Every connection sets:

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = FULL;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
```

WAL is used only on the OS-local application-data filesystem. Export/import, not a shared database
file, is the cross-machine mechanism. SQLite busy failures map to one stable user-safe error:
“Agency Agents is busy with another desktop or MCP operation. Nothing was changed. Try again.”

## Mutation Contract

Every state mutation uses one `BEGIN IMMEDIATE` transaction that:

1. Loads the current document and verifies its document version and size.
2. Deserializes it into the existing domain type.
3. Runs the existing semantic validator.
4. Applies the domain mutation.
5. Validates and size-checks the result.
6. Updates the document and document revision.
7. Increments the user-visible revision in the same transaction.
8. Commits or changes nothing.

This prevents two MCP processes from reading the same JSON document and silently overwriting each
other. Audit-only and internal reconciliation commits do not increment the user-visible revision.

## Filesystem Operation Recovery

SQLite cannot atomically commit a filesystem rename. Existing staging, quarantine, hash,
containment, symlink/reparse-point, and backup-first rules remain. A small journal closes the crash
window:

```text
DB: insert prepared operation
  -> commit
filesystem: revalidate + apply existing atomic move/copy
DB: mark filesystem_applied + apply metadata mutation
  -> commit
filesystem: remove obsolete staging/quarantine when safe
DB: mark committed (later prune)
```

At startup, recovery handles each phase idempotently. It never trusts a stored path without
re-running canonical containment, inventory, hash, and link checks. Running approvals carry the
operation ID and return to the correct terminal or reviewable state after recovery.

## Migration and Cutover

### Process exclusivity

Every new desktop/MCP process holds a shared OS lock on `state/storage.lock` for its lifetime.
Migration and restore require the exclusive lock. The maintenance screen asks the user to close
Claude and Codex sessions, then the backend attempts the exclusive lock. If another current process
is alive, migration refuses to begin.

Older binaries cannot honor the new lock. Therefore the cutover also fingerprints every imported
legacy file and requires users to reconnect MCP clients after completion. Reappearing or modified
legacy files produce a persistent conflict warning and are never merged automatically.

### State machine

```text
never -> in_progress -> complete
                 \----> corrupt
newer schema ----------> unsupported
```

- `never`: database has not become authoritative.
- `in_progress`: retry is safe because the import transaction did not publish `complete`.
- `complete`: SQLite is the sole live source of truth; JSON is never an automatic fallback.
- `corrupt`: validation, trust verification, backup verification, or integrity checking failed.
- `unsupported`: the database schema/document version is newer than the running app.

Migration inventories every legacy document, enforces its existing byte/count bounds, parses and
semantically validates it, and verifies every signed Skill trust record against the keychain-held
HMAC key. Secrets and keys are never inserted into SQLite or export files.

Before import, the app creates a timestamped legacy backup and verifies its inventory and hashes.
The import and `complete` marker publish in one SQLite transaction followed by
`PRAGMA integrity_check`. Migration failure leaves the JSON backend and package files unchanged.
After cutover, live database backups use SQLite's Online Backup API and are integrity-checked before
being reported successful.

## User Experience

Migration is a short, explicit maintenance event rather than an unexplained startup delay.

- Start: “Agency Agents needs a one-time data update.”
- Requirement: close connected Claude/Codex sessions; package files are not moved or changed.
- Stages: checking data, verifying backup, moving records, verifying database, finishing.
- Completion: persistent screen instructs the user to reopen Claude/Codex.
- Failure: reassures that nothing was lost and permits retry or continuing with current JSON
  storage when the schema is not newer/unsupported.
- Interrupted journal recovery is automatic and uses calm, durable status text.
- Technical details stay behind “Show details” or “Copy details”.

Visible approval surfaces refresh after local writes and window focus, and poll the user-visible
revision while open. Changes appear within one second without closing the inbox, stealing focus,
resetting scroll, flashing loaders, or emitting repeated toasts.

## Source-of-Truth Scope

Migrated control-plane documents include:

- Skills: sources, signed trust, drafts, library/folders/approvals, installs.
- Agents: sources, drafts, library/approvals, installs, project registry.
- Experts: custom definitions/change requests, activation requests/history, runs/evidence.
- Cross-cutting: settings/MCP policy, audit journal, and user-visible revision.

Derived/rebuildable corpus indexes and caches remain files. Keychain tokens and signing keys remain
in the OS keychain. Agent/Skill version snapshot directories remain filesystem artifacts whose
indexes may be referenced by SQLite.

## Alternatives Rejected

1. **Document database:** adds a service/runtime without improving this local relational lifecycle.
2. **SQLite-only artifact blobs:** breaks Git/editor/Claude/Codex portability and makes large assets
   harder to inspect and recover.
3. **Big-bang normalized schema:** duplicates current validation and greatly increases migration
   risk before query bottlenecks are measured.
4. **Dual-live SQLite and JSON writes:** cannot be committed atomically and creates split-brain.
5. **Silent JSON fallback after cutover:** can resurrect stale approvals, drafts, or installs.

## Decision Log

| Decision | Alternatives | Review objection | Resolution |
|---|---|---|---|
| SQLite control plane + filesystem artifacts | Document DB; SQLite blobs | Package portability | Keep canonical files and store paths/hashes |
| Versioned JSON documents initially | Full normalization | Phase 1 cannot claim relational/search benefits | Claims limited to durability, serialization, and consistency |
| Transactional mutation closures | Load/mutate/upsert | Last-writer-wins across MCP processes | Entire RMW occurs in `BEGIN IMMEDIATE` |
| Explicit maintenance cutover | Silent startup migration | Old/active MCP writers | Shared lifetime lock, exclusive migration, reconnect requirement |
| SQLite-only truth after cutover | Dual write; fallback | Stale JSON resurrection | Backup/export only; fingerprint conflicts never auto-merge |
| Operation journal | Best-effort rollback only | Crash between DB and filesystem | Three phases + idempotent validated recovery |
| Revision polling | Refresh only on open | SQLite does not update Svelte state | <=1s quiet visible-surface refresh |
| Online Backup API | Copy `.sqlite3` | WAL may contain committed pages | SQLite-consistent verified snapshot |
| Keychain remains separate | Store encrypted secrets in DB | Trust/token exposure | Verify trust during import; never export secrets |
| Defer FTS/normalization | Build now | YAGNI and migration surface | Add only after measured need |

## Review Disposition

Structured review completed in order: Skeptic, Constraint Guardian, User Advocate, Arbiter.
All critical/high objections were accepted and incorporated. Arbiter disposition: **APPROVED**,
conditional on the implementation plan explicitly testing exclusive cutover, exact byte caps,
legacy fingerprints, multi-process contention, and crash boundaries.
