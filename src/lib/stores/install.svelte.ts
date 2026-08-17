/**
 * Install store — drives the Phase 2 install/reconcile backend
 * (install_agent / uninstall_agent / installs_reconcile / tools_list).
 *
 * Singleton: import `install` everywhere. `reconcile()` refreshes the
 * cross-tool installed view (the seven-state Library model); `install()` /
 * `uninstall()` mutate then re-reconcile so the UI reflects truth.
 *
 * Backend-not-ready posture (matches corpus store): every invoke is wrapped
 * so a missing command degrades to empty state rather than throwing.
 */
import { invoke } from "@tauri-apps/api/core";

import { activity, safeActivityDetail } from "$lib/stores/activity.svelte";
import type { ActivityReceiptItem, ActivityReceiptOperation } from "$lib/stores/activity.svelte";
import { i18n } from "$lib/stores/i18n.svelte";
import { corpus } from "$lib/stores/corpus.svelte";
import { skillSources } from "$lib/stores/skillSources.svelte";
import { wiredTools } from "$lib/data/toolRegistry";
import {
  agentBatchApply,
  agentBatchInstallPlan,
  agentDisable,
  agentCollectionApply,
  agentCollectionPlan,
  agentDiffExact,
  agentEnable,
  agentInstallPlan,
  agentInstallWithDependencies,
  agentRosterApply,
  agentRosterDisable,
  agentRosterEnable,
  agentRosterPlan,
  agentRostersReconcile,
  agentRosterVersionHistory,
  agentRosterVersionRollback,
  agentTrackExact,
  agentUninstallExact,
  agentUninstallPlan,
  agentUpdateExact,
  agentUpdatePlan,
  agentVersionHistory,
  agentVersionRollback,
} from "$lib/api";
import { agentInstallKey } from "$lib/agents/libraryModel";
import { appErrorMessage, isAppError } from "$lib/types";
import type {
  AgentDiff,
  AgentMutationPlan,
  AgentReference,
  AgentRosterInstallRecord,
  AgentRosterMutationPlan,
  AgentVersionSnapshot,
  InstalledAgent,
  InstalledAgentRoster,
  InstallRecord,
  InstallState,
  Tool,
  ToolInfo,
  ToolVersion,
  WorkspacePackApplyResponse,
  WorkspacePackPlan,
  WorkspacePackScope,
} from "$lib/types";

function planReceiptOperation(operation: string): ActivityReceiptOperation {
  if (operation === "install" || operation === "update" || operation === "uninstall") return operation;
  throw new Error(`Unsupported receipt operation: ${operation}`);
}

function planReceiptItems(plan: AgentMutationPlan, records: InstallRecord[]): ActivityReceiptItem[] {
  return records.map((record) => {
    const planned = plan.agents.find((item) =>
      item.reference.sourceId === record.sourceId && item.reference.relativePath === record.relativePath);
    return {
      kind: "agent",
      name: planned?.name ?? record.relativePath,
      destination: record.dest,
      outcome: "ok",
      ...(record.deploymentNotice ? { detail: record.deploymentNotice } : {}),
    };
  });
}

function failedPlanReceiptItems(plan: AgentMutationPlan, error: unknown): ActivityReceiptItem[] {
  const detail = error instanceof Error ? error.message : String(error);
  return plan.agents.map((item) => ({
    kind: "agent",
    name: item.name,
    destination: item.destination,
    outcome: "error",
    detail,
  }));
}

function mutationDeploymentNotice(value: unknown): string | undefined {
  const records = Array.isArray(value) ? value : [value];
  const notice = records.find((record) =>
    record && typeof record === "object" && typeof (record as { deploymentNotice?: unknown }).deploymentNotice === "string",
  ) as { deploymentNotice?: string } | undefined;
  return notice?.deploymentNotice ? safeActivityDetail(notice.deploymentNotice) : undefined;
}

/** The tools Phase 2 can install to. Mirrors the Rust `SUPPORTED` set and the
    `supports_user()`/`supports_project()` capabilities in `render/mod.rs`.
    Order = install-menu order.

    `scope` is the PRIMARY/display scope (global-first for dual-scope tools);
    the `supports*` flags drive the "how × where" UI — a tool can deploy
    user-globally AND/OR into a specific project. Verified per-tool against
    official docs (June 2026): Cursor is the one project-only tool (its global
    rules are UI-only); every other supported tool is dual-scope. */
export interface ToolDef {
  id: Tool;
  label: string;
  installKind: string;
  scope: "user" | "project";
  /** Can deploy user-globally (`~/…`). */
  supportsUser: boolean;
  /** Can deploy into a specific project (`<project>/…`). */
  supportsProject: boolean;
}

// Module-level in-flight guard (NOT a class #private field — those can trip up
// Svelte 5's class-$state transform). Coalesces the many on-mount reconcile()
// callers into one heavy scan.
let reconcileInflight: Promise<void> | null = null;

/** Persisted "Install into…" tool selection — remembered across agents/launches. */
const INSTALL_SELECTION_KEY = "agency-agents:install-selection";

/** Derived from the tool registry (`src-tauri/data/tools/*.json`) — the wired
    tools, in registry order. Adding a tool there flows through here; nothing to
    edit in this file. `scope` = the primary/display scope (user-first). */
export const SUPPORTED_TOOLS: ToolDef[] = wiredTools().map((t) => ({
  id: t.id,
  label: t.label,
  installKind: t.installKind ?? "per-agent",
  scope: t.scope?.user ? "user" : "project",
  supportsUser: t.scope?.user ?? false,
  supportsProject: t.scope?.project ?? false,
}));

class InstallStore {
  /** Reconciled cross-tool installs (the Library model). */
  installed: InstalledAgent[] = $state([]);
  /** Project-scoped aggregate Aider/Windsurf roster truth. */
  rosters: InstalledAgentRoster[] = $state([]);
  rosterReconciling: boolean = $state(false);
  rostersReconciled: boolean = $state(false);
  rosterReconcileError: string | null = $state(null);
  /** Detected tools + counts (the Tools section). */
  tools: ToolInfo[] = $state([]);
  /** `${slug}:${tool}` currently mid-install/uninstall (for spinners). */
  busy: string | null = $state(null);
  /** True while a reconcile is in flight (drives loading states). */
  reconciling: boolean = $state(false);
  /** True once the first reconcile has completed (so we can tell "empty"
      apart from "not scanned yet"). */
  reconciled: boolean = $state(false);
  /** Latest reconcile failure. Retained while retrying, cleared only by success. */
  reconcileError: string | null = $state(null);
  /** Actual scan attempt and latest terminal attempt, for transition announcements. */
  reconcileAttempt = $state(0);
  reconcileTerminal = $state(0);
  /** Tools currently checked in the "Install into…" menu. Persisted so the
      choice is remembered for the next agent and the next launch. */
  selectedTools: Tool[] = $state([]);

  /** Load the remembered tool selection; defaults to Claude Code on first run. */
  loadSelection(): void {
    let parsed: Tool[] = [];
    try {
      const raw = localStorage.getItem(INSTALL_SELECTION_KEY);
      if (raw) {
        const arr = JSON.parse(raw) as unknown;
        if (Array.isArray(arr)) {
          parsed = arr.filter((id): id is Tool => SUPPORTED_TOOLS.some((t) => t.id === id));
        }
      }
    } catch {
      /* ignore */
    }
    this.selectedTools = parsed.length > 0 ? parsed : ["claudeCode"];
  }

  /** Is `tool` checked in the Install-into menu? */
  isSelected(tool: Tool): boolean {
    return this.selectedTools.includes(tool);
  }

  /** Toggle a tool's checked state and persist the selection. */
  toggleSelected(tool: Tool): void {
    const nowSelected = !this.isSelected(tool);
    this.selectedTools = nowSelected
      ? [...this.selectedTools, tool]
      : this.selectedTools.filter((t) => t !== tool);
    try {
      localStorage.setItem(INSTALL_SELECTION_KEY, JSON.stringify(this.selectedTools));
    } catch {
      /* ignore */
    }
    // Journal the default-target switch (purely local; no backend call).
    activity.log({
      action: "switch",
      tool,
      scope: this.scopeOf(null),
      outcome: "ok",
      detail: i18n.t(nowSelected ? "common.defaultTargetAdded" : "common.defaultTargetRemoved"),
    });
  }

  /**
   * Reconcile installs against disk + corpus. Called from many views on mount,
   * so it COALESCES via a module-level in-flight promise: concurrent callers
   * share one scan (the command reads every installed file + sweeps each tool
   * dir). On error we KEEP the previous result rather than blanking the UI.
   */
  async reconcile(): Promise<void> {
    if (reconcileInflight) return reconcileInflight;
    const attempt = ++this.reconcileAttempt;
    this.reconciling = true;
    reconcileInflight = (async () => {
      try {
        const result = await invoke<InstalledAgent[]>("installs_reconcile", { projectRoots: [] });
        this.installed = result;
        this.reconciled = true;
        this.reconcileError = null;
      } catch (error) {
        this.reconcileError = isAppError(error) ? appErrorMessage(error) : String(error);
      } finally {
        this.reconcileTerminal = attempt;
        this.reconciling = false;
        reconcileInflight = null;
      }
    })();
    return reconcileInflight;
  }

  async reconcileRosters(): Promise<void> {
    this.rosterReconciling = true;
    try {
      const rosters = await agentRostersReconcile();
      if (!Array.isArray(rosters)) throw new Error("Agent roster reconciliation returned no rows");
      this.rosters = rosters;
      this.rostersReconciled = true;
      this.rosterReconcileError = null;
    } catch (error) {
      this.rosterReconcileError = isAppError(error) ? appErrorMessage(error) : String(error);
    } finally {
      this.rosterReconciling = false;
    }
  }

  async loadTools(): Promise<void> {
    try {
      this.tools = await invoke<ToolInfo[]>("tools_list");
    } catch {
      this.tools = [];
    }
  }

  /** Best-effort detected tool versions (`<bin> --version`), keyed by tool id.
      Populated by `loadVersions()`; absent/unknown tools just don't appear. */
  versions: Record<string, string | null> = $state({});

  /** Probe tool versions in the background (slow-ish; spawns processes). */
  async loadVersions(): Promise<void> {
    try {
      const list = await invoke<ToolVersion[]>("tool_versions");
      const m: Record<string, string | null> = {};
      for (const v of list) m[v.tool] = v.version;
      this.versions = m;
    } catch {
      /* leave prior versions */
    }
  }

  /** Detected version string for a tool, or null if unknown. */
  versionOf(tool: Tool): string | null {
    return this.versions[tool] ?? null;
  }

  /** Reveal a path in the OS file manager (Finder / Explorer / xdg-open). */
  async revealPath(path: string): Promise<void> {
    await invoke("reveal_path", { path });
  }

  /** All installed rows for an agent across tools/projects. */
  forSlug(slug: string): InstalledAgent[] {
    return this.installed.filter((i) => i.slug === slug);
  }

  /** Whether `slug` is installed in `tool` (matching project for project tools). */
  isInstalled(slug: string, tool: Tool, projectPath: string | null = null): boolean {
    return this.installed.some(
      (i) =>
        i.slug === slug &&
        i.tool === tool &&
        (i.projectPath ?? null) === (projectPath ?? null),
    );
  }

  /** The reconciled state for `slug` in `tool` (current/outdated/modified/
      missing/foreign/disabled/source-unavailable), or null if there's no install on disk. Lets the UI show
      the SAME truth everywhere instead of a flat "installed". */
  stateFor(slug: string, tool: Tool, projectPath: string | null = null): InstallState | null {
    const row = this.installed.find(
      (i) =>
        i.slug === slug &&
        i.tool === tool &&
        (i.projectPath ?? null) === (projectPath ?? null),
    );
    return row?.state ?? null;
  }

  forReference(reference: AgentReference): InstalledAgent[] {
    return this.installed.filter(
      (row) => row.sourceId === reference.sourceId && row.relativePath === reference.relativePath,
    );
  }

  stateForReference(
    reference: AgentReference,
    tool: Tool,
    projectPath: string | null = null,
  ): InstallState | null {
    return this.installed.find(
      (row) =>
        row.sourceId === reference.sourceId &&
        row.relativePath === reference.relativePath &&
        row.tool === tool &&
        (row.projectPath ?? null) === projectPath,
    )?.state ?? null;
  }

  private async exactMutation<T>(
    action: "install" | "update" | "track" | "uninstall" | "disable" | "enable" | "rollback",
    reference: AgentReference,
    tool: Tool,
    projectPath: string | null,
    run: () => Promise<T>,
  ): Promise<T> {
    this.busy = agentInstallKey(reference, tool, projectPath);
    try {
      const result = await run();
      await this.reconcile();
      void this.loadTools();
      const detail = mutationDeploymentNotice(result);
      activity.log({
        action,
        subject: "agent",
        subjectName: `${reference.relativePath} · ${reference.sourceId}`,
        agentSlug: reference.relativePath.replace(/\.md$/, "").split("/").pop(),
        tool,
        scope: this.scopeOf(projectPath),
        projectPath: projectPath ?? undefined,
        outcome: "ok",
        ...(detail ? { detail } : {}),
      });
      return result;
    } catch (error) {
      activity.log({
        action,
        subject: "agent",
        subjectName: `${reference.relativePath} · ${reference.sourceId}`,
        tool,
        scope: this.scopeOf(projectPath),
        projectPath: projectPath ?? undefined,
        outcome: "error",
        detail: safeActivityDetail(error instanceof Error ? error.message : error),
      });
      throw error;
    } finally {
      this.busy = null;
    }
  }

  plan(
    operation: "install" | "update" | "uninstall",
    reference: AgentReference,
    tool: Tool,
    projectPath: string | null,
  ): Promise<AgentMutationPlan> {
    if (operation === "install") return agentInstallPlan(reference, tool, projectPath);
    if (operation === "update") return agentUpdatePlan(reference, tool, projectPath);
    return agentUninstallPlan(reference, tool, projectPath);
  }

  installReference(
    reference: AgentReference,
    tool: Tool,
    projectPath: string | null,
    confirmed: boolean,
  ): Promise<InstallRecord[]> {
    return this.exactMutation("install", reference, tool, projectPath, () =>
      agentInstallWithDependencies(reference, tool, projectPath, confirmed));
  }

  updateReference(
    reference: AgentReference,
    tool: Tool,
    projectPath: string | null,
    confirmed: boolean,
  ): Promise<InstallRecord> {
    return this.exactMutation("update", reference, tool, projectPath, () =>
      agentUpdateExact(reference, tool, projectPath, confirmed));
  }

  trackReference(reference: AgentReference, tool: Tool, projectPath: string | null): Promise<InstallRecord> {
    return this.exactMutation("track", reference, tool, projectPath, () =>
      agentTrackExact(reference, tool, projectPath));
  }

  uninstallReference(reference: AgentReference, tool: Tool, projectPath: string | null): Promise<void> {
    return this.exactMutation("uninstall", reference, tool, projectPath, () =>
      agentUninstallExact(reference, tool, projectPath));
  }

  disableReference(reference: AgentReference, tool: Tool, projectPath: string | null): Promise<InstallRecord> {
    return this.exactMutation("disable", reference, tool, projectPath, () =>
      agentDisable(reference, tool, projectPath));
  }

  enableReference(reference: AgentReference, tool: Tool, projectPath: string | null): Promise<InstallRecord> {
    return this.exactMutation("enable", reference, tool, projectPath, () =>
      agentEnable(reference, tool, projectPath));
  }

  history(reference: AgentReference, tool: Tool, projectPath: string | null): Promise<AgentVersionSnapshot[]> {
    return agentVersionHistory(reference, tool, projectPath);
  }

  diffReference(reference: AgentReference, tool: Tool, projectPath: string | null): Promise<AgentDiff> {
    return agentDiffExact(reference, tool, projectPath);
  }

  rollbackReference(
    reference: AgentReference,
    tool: Tool,
    projectPath: string | null,
    snapshotId: string,
  ): Promise<InstallRecord> {
    return this.exactMutation("rollback", reference, tool, projectPath, () =>
      agentVersionRollback(reference, tool, projectPath, snapshotId));
  }

  planCollection(
    name: string,
    operation: "install" | "update" | "uninstall",
    tool: Tool,
    projectPath: string | null,
  ): Promise<AgentMutationPlan> {
    return agentCollectionPlan(name, operation, tool, projectPath);
  }

  planBatch(
    references: AgentReference[],
    tool: Tool,
    projectPath: string | null,
  ): Promise<AgentMutationPlan> {
    return agentBatchInstallPlan(references, tool, projectPath);
  }

  planRoster(
    references: AgentReference[],
    operation: "install" | "update" | "uninstall",
    tool: Tool,
    projectPath: string,
  ): Promise<AgentRosterMutationPlan> {
    return agentRosterPlan(references, operation, tool, projectPath);
  }

  async applyRoster(plan: AgentRosterMutationPlan): Promise<AgentRosterInstallRecord> {
    this.busy = JSON.stringify(["roster", plan.tool, plan.projectPath]);
    try {
      const record = await agentRosterApply(plan);
      await this.reconcileRosters();
      activity.log({
        action: plan.operation,
        subject: "agentLibrary",
        subjectName: `${this.toolLabel(plan.tool)} roster`,
        tool: plan.tool,
        scope: "project",
        projectPath: plan.projectPath,
        outcome: "ok",
        detail: `${plan.members.length} exact agents · ${plan.destination}`,
      });
      return record;
    } finally {
      this.busy = null;
    }
  }

  async moveRoster(
    tool: Tool,
    projectPath: string,
    enable: boolean,
  ): Promise<AgentRosterInstallRecord> {
    const record = enable
      ? await agentRosterEnable(tool, projectPath)
      : await agentRosterDisable(tool, projectPath);
    await this.reconcileRosters();
    return record;
  }

  rosterHistory(tool: Tool, projectPath: string): Promise<AgentVersionSnapshot[]> {
    return agentRosterVersionHistory(tool, projectPath);
  }

  async rollbackRoster(
    tool: Tool,
    projectPath: string,
    snapshotId: string,
  ): Promise<AgentRosterInstallRecord> {
    const record = await agentRosterVersionRollback(tool, projectPath, snapshotId);
    await this.reconcileRosters();
    return record;
  }

  async applyBatch(plan: AgentMutationPlan): Promise<{ records: InstallRecord[]; receiptId: string }> {
    this.busy = JSON.stringify(["batch", plan.revision]);
    try {
      const records = await agentBatchApply(
        plan.agents.filter((item) => !item.dependency).map((item) => item.reference),
        plan.tool,
        plan.projectPath,
        plan.revision,
      );
      await this.reconcile();
      void this.loadTools();
      const receiptId = activity.log({
        action: "bulk",
        subject: "agentLibrary",
        subjectName: i18n.t("firstRun.deployTitle"),
        tool: plan.tool,
        scope: this.scopeOf(plan.projectPath),
        projectPath: plan.projectPath ?? undefined,
        outcome: "ok",
        detail: `${i18n.t("common.install")} · ${records.length}`,
        receipt: {
          operation: planReceiptOperation(plan.operation),
          succeeded: records.length,
          failed: 0,
          items: planReceiptItems(plan, records),
        },
      });
      return { records, receiptId };
    } catch (error) {
      activity.log({
        action: "bulk",
        subject: "agentLibrary",
        subjectName: i18n.t("firstRun.deployTitle"),
        tool: plan.tool,
        scope: this.scopeOf(plan.projectPath),
        projectPath: plan.projectPath ?? undefined,
        outcome: "error",
        detail: safeActivityDetail(error instanceof Error ? error.message : error),
        receipt: {
          operation: planReceiptOperation(plan.operation),
          succeeded: 0,
          failed: plan.agents.length,
          items: failedPlanReceiptItems(plan, error),
        },
      });
      throw error;
    } finally {
      this.busy = null;
    }
  }

  async applyCollection(
    name: string,
    plan: AgentMutationPlan,
  ): Promise<{ records: InstallRecord[]; receiptId: string }> {
    const operation = planReceiptOperation(plan.operation);
    if (operation === "track" || operation === "repair") throw new Error(`Unsupported collection operation: ${operation}`);
    this.busy = JSON.stringify(["collection", name, operation, plan.tool, plan.projectPath]);
    try {
      const records = await agentCollectionApply(name, operation, plan.tool, plan.projectPath, true);
      await this.reconcile();
      void this.loadTools();
      const receiptId = activity.log({
        action: "bulk",
        subject: "agentLibrary",
        subjectName: name,
        tool: plan.tool,
        scope: this.scopeOf(plan.projectPath),
        projectPath: plan.projectPath ?? undefined,
        outcome: "ok",
        detail: `${i18n.t(`common.${operation}`)} · ${records.length}`,
        receipt: {
          operation,
          succeeded: records.length,
          failed: 0,
          items: planReceiptItems(plan, records),
        },
      });
      return { records, receiptId };
    } catch (error) {
      activity.log({
        action: "bulk",
        subject: "agentLibrary",
        subjectName: name,
        tool: plan.tool,
        scope: this.scopeOf(plan.projectPath),
        projectPath: plan.projectPath ?? undefined,
        outcome: "error",
        detail: safeActivityDetail(error instanceof Error ? error.message : error),
        receipt: {
          operation,
          succeeded: 0,
          failed: plan.agents.length,
          items: failedPlanReceiptItems(plan, error),
        },
      });
      throw error;
    } finally {
      this.busy = null;
    }
  }

  /** Resolve an agent's friendly name from the loaded corpus, if available.
      Returns undefined when the corpus list hasn't loaded the slug — the
      journal then falls back to the slug alone. */
  private agentName(slug: string): string | undefined {
    return corpus.agents.find((a) => a.slug === slug)?.name;
  }

  /** Deployment scope of an INSTALL — derived from whether it targets a project,
      not from the tool (tools are dual-scope now). Mirrors Rust `scope_for()`. */
  private scopeOf(projectPath: string | null): "user" | "project" {
    return projectPath ? "project" : "user";
  }

  async install(slug: string, tool: Tool, projectPath: string | null = null): Promise<InstallRecord> {
    this.busy = `${slug}:${tool}`;
    try {
      const rec = await invoke<InstallRecord>("install_agent", { slug, tool, projectPath });
      await this.reconcile();
      void this.loadTools();
      activity.log({
        action: "install",
        agentSlug: slug,
        agentName: this.agentName(slug),
        tool,
        scope: this.scopeOf(projectPath),
        projectPath: projectPath ?? undefined,
        outcome: "ok",
      });
      return rec;
    } catch (e) {
      activity.log({
        action: "install",
        agentSlug: slug,
        agentName: this.agentName(slug),
        tool,
        scope: this.scopeOf(projectPath),
        projectPath: projectPath ?? undefined,
        outcome: "error",
        detail: safeActivityDetail(e instanceof Error ? e.message : e),
      });
      throw e;
    } finally {
      this.busy = null;
    }
  }

  async uninstall(slug: string, tool: Tool, projectPath: string | null = null): Promise<void> {
    this.busy = `${slug}:${tool}`;
    try {
      await invoke("uninstall_agent", { slug, tool, projectPath });
      await this.reconcile();
      void this.loadTools();
      activity.log({
        action: "uninstall",
        agentSlug: slug,
        agentName: this.agentName(slug),
        tool,
        scope: this.scopeOf(projectPath),
        projectPath: projectPath ?? undefined,
        outcome: "ok",
      });
    } catch (e) {
      activity.log({
        action: "uninstall",
        agentSlug: slug,
        agentName: this.agentName(slug),
        tool,
        scope: this.scopeOf(projectPath),
        projectPath: projectPath ?? undefined,
        outcome: "error",
        detail: safeActivityDetail(e instanceof Error ? e.message : e),
      });
      throw e;
    } finally {
      this.busy = null;
    }
  }

  /** Update an Outdated install to the current corpus version. */
  async update(slug: string, tool: Tool, projectPath: string | null = null): Promise<void> {
    this.busy = `${slug}:${tool}`;
    try {
      await invoke("update_agent", { slug, tool, projectPath });
      await this.reconcile();
      activity.log({
        action: "update",
        agentSlug: slug,
        agentName: this.agentName(slug),
        tool,
        scope: this.scopeOf(projectPath),
        projectPath: projectPath ?? undefined,
        outcome: "ok",
      });
    } catch (e) {
      activity.log({
        action: "update",
        agentSlug: slug,
        agentName: this.agentName(slug),
        tool,
        scope: this.scopeOf(projectPath),
        projectPath: projectPath ?? undefined,
        outcome: "error",
        detail: safeActivityDetail(e instanceof Error ? e.message : e),
      });
      throw e;
    } finally {
      this.busy = null;
    }
  }

  /**
   * Track a recognized Foreign install into the ledger NON-DESTRUCTIVELY — the
   * backend records provenance but never writes to the user's file. After this,
   * reconcile shows Current (file already matches the catalog) or Modified (it
   * differs; an explicit Update reconciles it, backing up first).
   */
  async track(slug: string, tool: Tool, projectPath: string | null = null): Promise<void> {
    this.busy = `${slug}:${tool}`;
    try {
      await invoke("track_agent", { slug, tool, projectPath });
      await this.reconcile();
      activity.log({
        action: "track",
        agentSlug: slug,
        agentName: this.agentName(slug),
        tool,
        scope: this.scopeOf(projectPath),
        projectPath: projectPath ?? undefined,
        outcome: "ok",
      });
    } catch (e) {
      activity.log({
        action: "track",
        agentSlug: slug,
        agentName: this.agentName(slug),
        tool,
        scope: this.scopeOf(projectPath),
        projectPath: projectPath ?? undefined,
        outcome: "error",
        detail: safeActivityDetail(e instanceof Error ? e.message : e),
      });
      throw e;
    } finally {
      this.busy = null;
    }
  }

  /** Diff the on-disk file against the canonical render (review before Update). */
  async diff(slug: string, tool: Tool, projectPath: string | null = null): Promise<AgentDiff> {
    return invoke<AgentDiff>("agent_diff", { slug, tool, projectPath });
  }

  /**
   * Run one action across many installs with a SINGLE reconcile at the end
   * (calling install()/update()/etc. in a loop would reconcile per item).
   *
   * For `update`/`track`/`uninstall` each target is an EXISTING install row, so
   * project tools already know their dest — no folder prompts. For `install`
   * each target is an agent to deploy fresh (used by the divisions landing to
   * deploy a whole division into a user-scoped tool); project-scoped tools are
   * excluded by the caller since they'd each need a folder prompt.
   *
   * Returns {ok, fail} counts.
   */
  async bulk(
    action: "install" | "update" | "track" | "uninstall",
    targets: { slug: string; tool: Tool; projectPath: string | null }[],
  ): Promise<{ ok: number; fail: number; receiptId: string }> {
    const cmd =
      action === "install"
        ? "install_agent"
        : action === "uninstall"
          ? "uninstall_agent"
          : action === "track"
            ? "track_agent"
            : "update_agent";
    let ok = 0;
    let fail = 0;
    const before = new Map(this.installed.map((row) => [
      JSON.stringify([row.slug, row.tool, row.projectPath ?? null]),
      row.dest,
    ]));
    const items: import("$lib/stores/activity.svelte").ActivityReceiptItem[] = [];
    for (const t of targets) {
      const priorDestination = before.get(JSON.stringify([t.slug, t.tool, t.projectPath ?? null])) ?? null;
      try {
        const record = action === "uninstall"
          ? (await invoke<void>(cmd, { slug: t.slug, tool: t.tool, projectPath: t.projectPath }), null)
          : await invoke<InstallRecord>(cmd, { slug: t.slug, tool: t.tool, projectPath: t.projectPath });
        ok++;
        items.push({
          kind: "agent",
          name: this.agentName(t.slug) ?? t.slug,
          destination: record?.dest ?? priorDestination,
          outcome: "ok",
        });
      } catch (error) {
        fail++;
        items.push({
          kind: "agent",
          name: this.agentName(t.slug) ?? t.slug,
          destination: priorDestination,
          outcome: "error",
          detail: safeActivityDetail(error instanceof Error ? error.message : error),
        });
      }
    }
    await this.reconcile();
    void this.loadTools();
    // ONE summarizing journal entry for the whole batch (not one per item). An
    // "update" sweep is a Sync; install/track/uninstall sweeps are generic Bulk
    // ops. `detail` is a self-contained verb phrase so the row reads naturally;
    // no single `tool` since a batch can span tools.
    const summary =
      action === "install"
        ? `${i18n.t("activity.action.install")} ${i18n.count(ok, "common.agent.one", "common.agent.many")}`
        : action === "update"
          ? `${i18n.t("activity.action.update")} ${i18n.count(ok, "common.agent.one", "common.agent.many")}`
          : action === "track"
            ? `${i18n.t("activity.action.track")} ${i18n.count(ok, "common.agent.one", "common.agent.many")}`
            : `${i18n.t("activity.action.uninstall")} ${i18n.count(ok, "common.agent.one", "common.agent.many")}`;
    const receiptId = activity.log({
      action: action === "update" ? "sync" : "bulk",
      outcome: fail > 0 ? "error" : "ok",
      detail: fail > 0 ? i18n.t("activity.bulkFailed", { summary, fail }) : summary,
      receipt: { operation: action, succeeded: ok, failed: fail, items },
    });
    return { ok, fail, receiptId };
  }

  /**
   * Forget a project WITHOUT deleting any files: the backend drops the
   * project's ledger rows so it leaves the Projects list, but the agent/skill
   * files this app wrote stay on disk. For the "also uninstall" path the caller
   * runs `bulk("uninstall", …)` first (which removes files + rows), so this is
   * only invoked for the keep-the-files choice.
   */
  async forgetProject(projectPath: string, label: string): Promise<void> {
    try {
      await invoke("project_forget", { projectPath });
      await this.reconcile();
      void this.loadTools();
      activity.log({
        action: "bulk",
        outcome: "ok",
        detail: i18n.t("projects.journalForgotten", { project: label }),
      });
    } catch (e) {
      activity.log({
        action: "bulk",
        outcome: "error",
        detail: i18n.t("common.actionFailed"),
      });
      throw e;
    }
  }

  /** Label for a tool id (for view-models that only have the wire value). */
  toolLabel(tool: Tool): string {
    return SUPPORTED_TOOLS.find((t) => t.id === tool)?.label ?? tool;
  }

  /** Export one current logical scope as a path-private Workspace Pack. */
  async exportLoadout(
    path: string,
    name: string,
    scope: WorkspacePackScope,
    projectPath: string | null,
  ): Promise<number> {
    return invoke<number>("loadout_export", { path, name, scope, projectPath });
  }

  /** Parse and completely plan a Workspace Pack without applying it. */
  inspectWorkspacePack(path: string, projectPath: string | null): Promise<WorkspacePackPlan> {
    return invoke<WorkspacePackPlan>("loadout_import", { path, projectPath });
  }

  /** Apply one unchanged reviewed plan, refresh both ledgers, and retain one receipt. */
  async applyWorkspacePack(
    path: string,
    projectPath: string | null,
    plan: WorkspacePackPlan,
    projectPaths: string[],
  ): Promise<{ response: WorkspacePackApplyResponse; receiptId: string | null }> {
    let response = await invoke<WorkspacePackApplyResponse>("loadout_apply", {
      path,
      projectPath,
      revision: plan.revision,
    });
    if (!response.result) return { response, receiptId: null };
    const result = {
      ...response.result,
      rollbackErrors: response.result.rollbackErrors.map(safeActivityDetail),
      items: response.result.items.map((item) => ({
        ...item,
        message: item.message == null ? null : safeActivityDetail(item.message),
      })),
    };
    response = {
      ...response,
      result,
    };
    await Promise.all([this.reconcile(), skillSources.reconcileInstalls(projectPaths)]);
    void this.loadTools();
    const names = new Map<string, string>();
    for (const item of [...response.plan.agents, ...response.plan.skills]) {
      names.set(`${item.reference.sourceId}\0${item.reference.relativePath}`, item.name);
    }
    const receiptItems: ActivityReceiptItem[] = result.items.map((item) => {
      const ok = item.outcome === "installed" || item.outcome === "current";
      return {
        kind: item.kind,
        name: names.get(`${item.sourceId}\0${item.relativePath}`) ?? item.relativePath,
        destination: item.destination,
        outcome: ok ? "ok" : "error",
        ...(!ok ? { detail: safeActivityDetail(item.message ?? item.outcome) } : {}),
      };
    });
    const succeeded = receiptItems.filter((item) => item.outcome === "ok").length;
    const failed = receiptItems.length - succeeded;
    const receiptId = activity.log({
      action: "bulk",
      subject: "agentLibrary",
      outcome: failed === 0 ? "ok" : "error",
      detail: `${succeeded} applied · ${failed} failed`,
      receipt: {
        operation: "install",
        succeeded,
        failed,
        items: receiptItems,
      },
    });
    return { response, receiptId };
  }
}

export const install = new InstallStore();
