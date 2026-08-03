# System Patterns — Agency Agents

## 1. Corpus-copy model (source of truth)

```
~/Library/Application Support/com.zerologic.agency-agents-app/
├── corpus/                  # our maintained copy of the agency-agents repo (251 .md)
│   └── <category>/<slug>.md
├── state/
│   ├── corpus-index.json    # slug → CorpusEntry (hashes, category, version)
│   └── installs.json        # the ledger: InstallRecord[]
└── settings.json
```

- **Seed**: a bundled baseline corpus ships inside the app bundle (resources) so first
  launch works offline. On first run it is extracted to `corpus/`.
- **Refresh**: fetch the GitHub tarball
  `https://codeload.github.com/msitarzewski/agency-agents/tar.gz/refs/heads/main`,
  extract, atomically swap `corpus/`, rebuild `corpus-index.json`. Record the resolved
  commit SHA / tag as `corpus_version`. **No runtime git dependency.**
- This mirrors brew-browser's bundled-catalog + live-refresh pattern, but the served
  tree is the GitHub repo itself.

## 2. Two indexes

### corpus-index.json (upstream truth — rebuilt on every refresh)
Per agent (`slug`): `source_hash` (SHA-256 of canonical `.md`), `frontmatter_hash`,
`body_hash` (split so updates can be classified cosmetic vs substantive), `category`,
`name`, `emoji`, `color`, `vibe`, `description`, `corpus_version`.

### installs.json — the ledger (local-action truth)
Per install action: `slug`, `tool`, `scope` (user|project), `project_path` (for project
scope), `dest` (absolute path written), `source_hash` (corpus version installed from),
`rendered_hash` (SHA-256 of exact bytes written after conversion), `installed_at`,
`corpus_version`.

### skill-installs.json — exact multi-file skill ledger

Skill packages use a separate ledger because one install owns a directory tree rather than one
rendered agent file. Each record stores source identity, package-relative path, runtime, scope,
canonical project path, exact destination, source-tree hash, installed-tree hash, timestamp, and
an optional disabled path reserved for Phase 4.

Installation revalidates the source package, rejects links and special entries, copies into a
same-parent staging directory, verifies its deterministic tree hash, backs up managed content,
then publishes by rename. Foreign or locally modified destinations are never replaced.
Reconciliation classifies Current, Outdated, Modified, Missing, Foreign, Disabled, and
SourceUnavailable without mutating destination content.

Lifecycle mutations remain ledger-driven. Update reuses the validated installation transaction;
Disable moves a byte-identical tracked directory to a hidden sibling and records `disabledPath`;
Enable refuses any occupied destination before moving it back. Uninstall removes only a matching
tracked row and exact directory, copying modified trees to `skill-backups/` first. Source removal
only unregisters the source, so installed content survives as SourceUnavailable.

## 3. Deterministic renderer (Plan B — load-bearing)

Install = **transform frontmatter + write file**, done natively in Rust (no shell/python
at runtime). Each tool has a deterministic converter: `render(agent, tool) -> (bytes, rendered_hash)`.
Determinism is REQUIRED so `rendered_hash` is stable and provenance reconciliation works.
`scripts/convert.sh` in the agency-agents repo is the **reference spec** (color→hex maps,
`.md`→`.mdc`/TOML, etc.), not a runtime dependency.

## 4. Reconciliation — the 5 states (this is our `brew list`/`brew outdated`)

`reconcile(disk_scan, ledger, corpus_index) -> InstalledAgent[]` classifies each
on-disk agent file:

| State | In ledger | On disk | Test | UI |
|---|---|---|---|---|
| **Current** | ✓ | ✓ | re-render(slug,tool)@current == on-disk bytes | ✓ green |
| **Outdated** | ✓ | ✓ | matches an OLDER render; corpus source_hash advanced | "Update" badge |
| **Modified** | ✓ | ✓ | slug known but bytes match no known render | "Modified — protect" |
| **Removed** | ✓ | ✗ | ledger entry, file gone | "Reinstall?" |
| **Foreign** | ✗ | ✓ | slug unknown to corpus-index | "Adopt?" |

**Provenance is hash-match only — never mutate agent content / never stamp files.**
Identity = slug-from-filename, confirmed by re-rendering against our corpus-index.
"Adopt" pulls a recognized Foreign agent into the ledger. Update classification uses
`body_hash` vs `frontmatter_hash` to label an update *cosmetic* (metadata only) or
*substantive* (prompt body changed).

## 5. Both scopes — fully supported

- **User-global tools** → fixed `~/…` dests, single global state:
  claude-code (`~/.claude/agents/`), gemini-cli (`~/.gemini/agents/`),
  copilot (`~/.github/agents/` + `~/.copilot/agents/`), qwen (`~/.qwen/agents/`),
  codex (`~/.codex/agents/`, TOML), openclaw (`~/.openclaw/agency-agents/`),
  antigravity (`~/.gemini/antigravity/skills/`).
- **Project-scoped tools** → install into ANY directory, tracked per `project_path`:
  cursor (`.cursor/rules/*.mdc`), windsurf (`.windsurfrules`), aider (`CONVENTIONS.md`),
  opencode (`.opencode/agents/*.md`). The app keeps a **Projects** list; Library/Tools
  show per-project deployment. One agent in five projects = five tracked rows.

## 6. Domain-layer module map (Rust `src-tauri/src/`)

| brew-browser | Agency Agents | role |
|---|---|---|
| `brew/` | `agency/` | exec/parse/paths for agent ops |
| `catalog/` | `catalog/` | parse corpus → in-memory catalog |
| (new) | `corpus/` | fetch/extract/index the repo copy |
| (new) | `render/` | deterministic per-tool converters |
| (new) | `ledger/` | install records read/write |
| (new) | `reconcile/` | disk ↔ ledger ↔ corpus → 5 states |
| (new) | `projects/` | project-scope registry |
| `enrichment/` | `enrichment/` | optional AI "best for/pairs with" |
| `trending/` | `trending/` | New & Updated / Popular from git history |
| `vulns/` | `quality/` | opt-in lint + originality scan |
| `github/` | `github/` | star/watch/issues (near-identical) |
| `services/` | `tools/` | detected AI tools + per-tool deployment |

## 7. Frontend sections (Svelte) — 1:1 with brew-browser

Dashboard · Discover (18-cat tile grid + search) · Library (installed across tools,
status chips, persona detail slide-over) · Trending · **Loadouts** (Agentfile
save/restore, was Snapshots) · **Tools** (was Services) · **Quality** (was Security) ·
Activity drawer (live install/convert stdout). Cmd+K palette, Cmd+0…6 nav inherited.

## 8. Job/streaming model (inherited)

Long ops stream stdout/stderr via `Channel<Event>` with a global write-lock, exactly
like brew-browser's `run_brew_streaming`. Install/convert/refresh are the streaming ops.

## 9. Expert definition and execution lifecycle

Portable Expert definitions live in `state/experts.json`; they contain roster, skill/runbook
references, starter prompt, version, and quality contract, but never project paths or credentials.
MCP lifecycle changes reuse one owned, idempotent desktop approval inbox for create, update, clone,
archive, and delete. Updates use optimistic `baseVersion` checks; archive is reversible.

Activation resolves and installs through the existing agent/skill transaction paths, then snapshots
the definition into `state/expert-runs.json`. Runs are separately lock-serialized, atomically written,
capped at 500 records / 4 MiB, and scoped to the originating MCP client plus canonical project.
Evidence and blockers append while active; terminal desktop review freezes the run. Acceptance uses
the latest evidence for every required contract check or an explicit human waiver. MCP views expose
that a check was waived but redact the human waiver reason.
