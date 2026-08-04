# 260804_agents-mcp

## Objective

Expose the source-aware Agent platform through the existing Skills MCP server with the exact
approved 49-tool Agent surface, shared resources and transports, separate default-denied Agent
permissions, typed desktop approvals, and durable redacted auditing.

## Outcome

- ✅ Composed 49 exact `agents_*` tools with the unchanged 49 `skills_*` tools in one MCP server,
  router, stdio transport, bearer-authenticated loopback HTTP transport, and audit boundary.
- ✅ Added `agents://catalog`, exact Agent source resources, deterministic per-tool render previews,
  resource templates, subscriptions, and list/update notifications.
- ✅ Added Agent catalog, source, draft, nested-folder, personal-library, approval, install,
  dependency, batch, history, rollback, disable/enable, update, and uninstall tools.
- ✅ Added separate global and per-client Agent source/install/destructive permissions that remain
  false when older settings grant every Skills permission.
- ✅ Kept draft publication and approval execution desktop-only. Overwrite/destructive requests
  remain Pending until desktop execution revalidates exact target and plan revision.
- ✅ Required exact canonical project allowlisting and capability-relative project filesystem
  operations, including project paths nested inside generic approval requests.
- ✅ Bound approval ownership to the authenticated MCP server identity and recorded approval
  attempt/terminal outcomes under the same approval ID.
- ✅ Hardened audit persistence so mutation success is never returned without a durable terminal
  row; request bodies, source content, credentials, bearer tokens, and private-key material are not
  stored.
- ✅ Rust: 445 passed, 0 failed, 2 environment-dependent tests ignored; 2 CLI tests passed.
- ✅ Strict Clippy and Rust formatting passed; frontend check reported 0 errors and 0 warnings,
  13 frontend tests passed, production build passed, and `git diff --check` passed.
- ✅ Live stdio and HTTP verified 49 Skills + 49 Agents tools, Agent resource list/read, an allowed
  Agent read, a default-denied Agent mutation, and HTTP 401 without bearer authentication.

## Files Modified

- `src-tauri/src/agents/mcp.rs` — Agent router, 49 tools, resources, render previews, URI handling,
  catalog revisioning, and Agent-specific request validation.
- `src-tauri/src/skills/mcp.rs` — merged router, shared dispatch/audit boundary, Agent resource
  delegation and subscriptions, nested project authorization, and transport integration tests.
- `src-tauri/src/state.rs` — Agent authorization classes, exact allowlist capability opening, and
  durable MCP audit behavior.
- `src-tauri/src/commands/settings.rs` — separate global/per-client Agent MCP policy persistence.
- `src-tauri/src/install/mod.rs` and `src-tauri/src/install/history.rs` — state-only Agent plans,
  clean capability-bound installs, lifecycle approval execution, history, and stale-plan checks.
- `src-tauri/src/skills/install.rs` — reused capability-relative project file primitives.
- `src-tauri/src/agents/organize.rs` — typed Agent approvals, desktop execution/rejection, and
  approval-ID audit lifecycle.
- `src-tauri/src/{types.rs,lib.rs}` — approval/plan contracts and desktop command registration.
- `src/lib/{api.ts,types.ts}` — Agent MCP policy and approval wire contracts.
- `src/lib/components/SettingsSectionMcp.svelte` — separate Skills and Agents permission groups.
- `src/lib/i18n/locales/en.ts` and `src/lib/i18n/messages.test.ts` — Agent policy copy and fallback
  coverage.

## Patterns Applied

- Extended the existing MCP server and Skills capability primitives rather than adding a second
  server, port, token, connection registry, filesystem authority, or audit log.
- Kept reads enabled and every Agent mutation class independently default-denied.
- Used exact `(sourceId, relativePath)` identity and revision-bound structured approvals; no slug or
  filename fallback is used for mutation targets.
- Preserved the existing Skills wire behavior and tool inventory while adding an independently
  classified Agent tool family.

## Integration Points

- MCP dispatch authorizes and opens a canonical project capability before Agent planning or
  mutation and passes the capability through the existing handler extension.
- Clean installs execute immediately only with AgentInstall permission and a free destination;
  replacement, dependency, batch, update, rollback, and uninstall flows enter the desktop inbox.
- Agent source refresh/draft operations reuse the Stage 1 source and draft cores; lifecycle tools
  reuse the Stage 2 transactional install/history cores.
- Agent and Skills resources coexist through one `ServerHandler` on both transports.

## Security Evidence

- First-launch, corrupt, paranoid, and non-allowlisted mutation policies fail closed.
- Project capability tests survive root and internal-ancestor retarget attempts.
- Nested generic approval project paths cannot bypass the allowlist or leak into denied audit rows.
- Audit preflight/terminal failure injection prevents unsafe dispatch or false mutation success.
- Redaction fixtures cover prompt/source text, credentials, bearer tokens, and private-key-shaped
  input.

## Scope Boundaries

Desktop workflow completion, Activity coverage, final localization/accessibility checks,
cross-platform rehearsal, and the final evidence-backed parity audit remain Stage 4. Execution,
cloud sync, collaboration, and public marketplace capabilities remain approved non-goals.

## Artifacts

- Design: `docs/superpowers/specs/2026-08-04-agents-skills-feature-parity-design.md`
- Plan: `docs/superpowers/plans/2026-08-04-agents-mcp.md`
