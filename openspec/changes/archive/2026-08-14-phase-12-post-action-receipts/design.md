## Context

See `proposal.md` for motivation. The frontend already has a 500-entry localStorage-backed Activity journal, one Activity workspace, toast actions, and route-free UI navigation. Agent bulk operations currently retain only an aggregate row plus individual failures. Safe repair shows combined Agent and Skill results in memory but persists only an aggregate summary. Exact destinations are already available from returned install records, reconciled rows, mutation plans, or repair candidates.

The change crosses the journal store, Activity rendering, UI navigation, Agent bulk helper/callers, and safe-repair result flow. Mutation commands and their approval/recovery contracts must remain unchanged.

## Goals / Non-Goals

**Goals:**

- Persist one bounded structured receipt per completed bulk operation using existing local Activity retention.
- Preserve every attempted item, every exact changed destination, and terminal outcomes for Agent bulk actions and combined safe repair.
- Reuse the existing Activity workspace and toast action component for exact receipt navigation.
- Keep old journal entries compatible and receipt persistence subordinate to mutation truth.

**Non-Goals:**

- A backend audit table, cloud synchronization, export format, native notification, or new route/modal.
- Changing mutation selection, approval, retry, reconciliation, backup, rollback, or authorization.
- Retrofitting single-item operations, MCP audit entries, or historical entries with synthetic receipts.

## Decisions

### Extend the existing journal entry instead of creating receipt storage

Add an optional structured receipt payload to the current journal entry and keep persisted shape version 2 because the field is additive. The Activity store remains the single retention, hydration, redaction, and size boundary. Its log operation returns the generated entry ID so completion surfaces can link to the exact record.

A new SQLite table or localStorage key was rejected because receipts have the same lifecycle and retention expectations as Activity and do not need cross-process authority.

### Normalize receipt content once at the Activity boundary

The Activity store validates closed item fields, applies the existing secret/error redaction to failure detail, strips control characters, and bounds text before persistence. Callers provide raw terminal facts but cannot bypass journal safety.

Scattered caller sanitization was rejected because new bulk paths could omit it and persisted content is the trust boundary.

### Collect terminal facts where the mutation loop already knows them

The Agent bulk helper records returned destinations for successful install, update, and track operations and the pre-action reconciled destination for uninstall. A failure retains a known pre-action destination when one exists; a failed fresh legacy install records no destination and explicitly claims no changed path. It returns the receipt ID with existing counts. Planned batch and collection applications reuse their existing plan destinations. Safe repair builds its combined receipt from the already terminal `results` list and exact candidates. No backend command or second reconciliation is added.

Reconstructing outcomes from the final reconciled ledger was rejected because uninstalled destinations disappear and a failed attempt may retain its prior state.

### Use native disclosure inside Activity and existing navigation state

Receipt rows use a native disclosure control so destination detail is keyboard-operable without a new component or modal. UI state carries one transient receipt ID. `View Activity` switches to the existing Activity section; the matching disclosure opens, scrolls, and receives focus after render. A missing retained ID produces an accessible bounded notice and then clears the transient target.

A new receipt page was rejected because Activity already owns post-action evidence and the app avoids route proliferation.

### Add receipt actions only to bulk completion surfaces

Existing bulk completion toasts receive the current optional toast action, and safe repair adds the same action to its retained results surface. This meets the roadmap gap without changing single-item toasts or introducing a global notification policy.

## Risks / Trade-offs

- [A large bulk receipt consumes more of the existing localStorage allowance] → Bound each field and reuse the journal's fixed entry retention; do not duplicate successful items as separate rows.
- [A returned receipt ID can outlive journal retention] → Open Activity safely, announce the missing receipt, and never focus a different row.
- [Some failed commands return no destination] → Capture an existing reconciled or planned destination before mutation when available; otherwise retain a null destination and state that no changed path is claimed.
- [Journal persistence can fail after files changed] → Preserve mutation truth, report persistence failure through the existing development warning path, and never retry mutation for receipt creation.
- [Success toast auto-dismiss can limit action time] → The durable receipt remains available through Activity even when the transient action disappears.

## Migration Plan

1. Add the optional receipt field and normalization while continuing to accept persisted v2 entries without it.
2. Add Activity disclosure and exact-focus navigation with focused compatibility tests.
3. Populate receipts from Agent bulk and safe repair terminal loops, then add existing toast/result actions.
4. Roll back by removing receipt writers and rendering; older clients ignore the additive JSON field and existing summaries remain valid.
