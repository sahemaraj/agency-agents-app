## Context

See `proposal.md` for motivation. Registered-project validation, capped reads, atomic writes, private backups, revision hashing, filesystem journals, Activity logging, and line-diff rendering already exist. Project instructions have no current ownership contract or persisted state.

## Goals / Non-Goals

**Goals:**

- Make one reviewed instruction-file change safely and recoverably.
- Preserve unowned bytes exactly and expose adoption honestly.
- Keep inspection local, bounded, deterministic, and passive.
- Reuse the Projects detail surface and current mutation/recovery authorities.

**Non-Goals:**

- Arbitrary file editing, whole-file ownership, nested-directory discovery, automatic project inference, template marketplace behavior, instruction execution, or MCP configuration.
- Synchronizing snippets across projects or adding a new persisted snippet library.

## Decisions

### Known-target allowlist rather than arbitrary relative paths

The command accepts a closed target identifier mapped to four fixed relative paths. Every request revalidates the exact canonical registered root and rejects links in existing path components. This is smaller and safer than a generic editor or user-configurable path registry.

### File-embedded ownership markers rather than a second ledger

Each snippet uses a versioned HTML-comment boundary with a portable slug identifier. A non-empty file receives an app-owned leading separator inside the managed span, allowing removal to reproduce the original bytes. Parsing rejects unmatched, nested, duplicate, or user-injected markers. The file remains the source of truth, so external edits are visible immediately and no migration is required.

### Current/proposed bytes are the diff contract

The backend returns complete bounded current and proposed UTF-8 content. The frontend reuses the existing local line-diff utility. The revision hashes normalized request fields, exact current/proposed bytes, target identity, and registration identity; apply fully replans and compares revisions.

### Existing filesystem journal for one-file atomic mutation

The operation payload contains the canonical project, allowlisted target, destination, pre/post hashes, and optional backup. Existing bytes are copied and verified into the current private backup directory before `atomic_write`. Startup recovery rolls an incomplete operation back to the pre-operation bytes (or removes a newly created destination) and retains recovery errors instead of claiming success.

### Existing Projects component and Activity entry

The manager is an inline section/modal state machine within `Projects.svelte`, backed by methods on the existing projects store. Successful or failed attempts use the current Activity journal shape with an `update` action and project scope, avoiding a new route, receipt schema, or durable audit table.

## Risks / Trade-offs

- File-embedded markers can be manually damaged → fail closed and show the exact target blocker; never guess ownership.
- Large existing files could make review unwieldy → enforce the existing 4 MiB read ceiling and bounded snippet/count limits.
- Crash after project write but before journal commit → prepared/applied recovery restores the verified previous bytes or removes the known newly-created target.
- Four targets exclude niche tools → add a reviewed fixed mapping only when a real client contract is established; do not accept arbitrary paths.
- Local Activity persistence can fail after a successful mutation → retain the terminal result in the UI and warn locally without retrying the mutation.

## Migration Plan

No stored-state migration is needed. Existing instruction files remain unmanaged until a user explicitly reviews and adopts one by adding the first owned snippet. Rollback uses the retained exact backup or removes a file created by the operation.
