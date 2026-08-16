<script lang="ts">
  /**
   * InstallModal — the ONE place agents get deployed. A destinations × tools
   * grid: rows are destinations (Global + each registered project), columns are
   * the detected tools, and every cell is a toggle that installs/removes the
   * agent set into that (scope, tool).
   *
   * Driven by an agent SET (`agentSlugs`) so it serves a single agent (from the
   * detail pane), a whole division, or a team — same component. For a single
   * agent a cell is on/off; for a set it's tri-state (all / some / none), and
   * toggling fills the missing ones or removes the whole set.
   *
   * "Global" only offers user-capable tools (Cursor's global cell is blank — its
   * global rules are UI-only). Removal of `foreign` files asks first.
   */
  import { onMount } from "svelte";
  import FolderPlus from "@lucide/svelte/icons/folder-plus";
  import FolderIcon from "@lucide/svelte/icons/folder";
  import Modal from "./Modal.svelte";
  import Button from "./Button.svelte";
  import DestructiveConfirm from "./DestructiveConfirm.svelte";
  import DiffModal from "./DiffModal.svelte";
  import DeploymentTargetGrid, {
    type DeploymentCell,
    type DeploymentColumn,
    type DeploymentRow,
  } from "./DeploymentTargetGrid.svelte";
  import { corpus } from "$lib/stores/corpus.svelte";
  import { install, SUPPORTED_TOOLS, type ToolDef } from "$lib/stores/install.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { ui } from "$lib/stores/ui.svelte";
  import { i18n } from "$lib/stores/i18n.svelte";
  import { agentLibrary } from "$lib/stores/agentLibrary.svelte";
  import { canApplyAgentPlan, installStateMessageKey, sameAgent } from "$lib/agents/libraryModel";
  import {
    appErrorMessage,
    isAppError,
    type Tool,
    type Agent,
    type AgentMutationPlan,
    type AgentPackageResult,
    type AgentReference,
    type AgentUpdatePolicy,
    type AgentVersionSnapshot,
    type InstalledAgent,
    type InstallState,
  } from "$lib/types";

  interface Props {
    title: string;
    agentSlugs?: string[];
    /** Exact source-aware single-Agent mode. Collections use `collectionName`. */
    agentPackage?: AgentPackageResult;
    /** Exact source-aware multi-Agent mode used by guided deployment. */
    agentReferences?: AgentReference[];
    allowedTools?: Tool[];
    collectionName?: string;
    reviewIntent?: {
      operation: "install" | "update";
      reference: AgentReference;
      tool: Tool;
      projectPath: string;
    };
    onClose: () => void;
    onApplied?: (plan: AgentMutationPlan) => void;
  }
  let { title, agentSlugs = [], agentPackage, agentReferences = [], allowedTools, collectionName, reviewIntent, onClose, onApplied }: Props = $props();
  const installTruthFresh = $derived(install.reconciled && !install.reconciling && !install.reconcileError);

  onMount(() => {
    projects.refresh();
    // Refresh detection so the columns reflect tools ACTUALLY on this device.
    void install.loadTools();
  });

  // The agents in this set that exist in the corpus (stale slugs skipped).
  const collection = $derived(
    collectionName
      ? agentLibrary.library.collections.find((item) => item.name === collectionName) ?? null
      : null,
  );
  const exactReferences = $derived(
    agentPackage ? [agentPackage.reference] : agentReferences.length ? agentReferences : collection?.agents ?? [],
  );
  const collectionPackages = $derived(
    exactReferences.length
      ? exactReferences.flatMap((reference) => {
          const pkg = agentLibrary.packages.find((item) => sameAgent(item.reference, reference));
          return pkg ? [pkg] : [];
        })
      : [],
  );
  const slugSet = $derived(new Set(
    agentPackage?.agent
      ? [agentPackage.agent.slug]
      : collectionPackages.length
        ? collectionPackages.flatMap((pkg) => pkg.agent ? [pkg.agent.slug] : [])
        : agentSlugs,
  ));
  const agents = $derived<Agent[]>(
    agentPackage?.agent
      ? [agentPackage.agent]
      : collectionPackages.length
        ? collectionPackages.flatMap((pkg) => pkg.agent ? [pkg.agent] : [])
        : corpus.agents.filter((a) => slugSet.has(a.slug)),
  );
  const total = $derived(agents.length);
  const exactReference = $derived(agentPackage?.reference ?? null);
  function matchesExact(row: InstalledAgent): boolean {
    return exactReferences.some(
      (reference) => row.sourceId === reference.sourceId && row.relativePath === reference.relativePath,
    ) || !!(
      agentPackage?.agent && row.state === "foreign" && !row.sourceId &&
      row.slug === agentPackage.agent.slug
    );
  }

  // Columns = tools present on this device (detected, or already holding an
  // install of this set), that can take an agent in SOME scope.
  function detected(t: ToolDef): boolean {
    return (
      install.tools.length === 0 ||
      install.tools.some((ti) => ti.tool === t.id && ti.detected) ||
      install.installed.some((r) => r.tool === t.id && r.state !== "missing" && (
        exactReferences.length
          ? matchesExact(r)
          : slugSet.has(r.slug)
      ))
    );
  }
  const cols = $derived(SUPPORTED_TOOLS.filter((t) =>
    (t.supportsUser || t.supportsProject)
    && (!allowedTools || allowedTools.includes(t.id))
    && detected(t)
  ));

  // Rows = Global + each registered/used project.
  type Row = { kind: "global" } | { kind: "project"; path: string; label: string };
  const rows = $derived<Row[]>([
    { kind: "global" },
    ...projects.list.map((p) => ({ kind: "project" as const, path: p.path, label: p.label })),
  ]);

  function targetOf(row: Row): string | null {
    return row.kind === "global" ? null : row.path;
  }
  // Tools in this grid that ONLY install per-project (no global scope) — e.g.
  // Cursor, whose "global" rules are a UI setting, not a writable file. With no
  // projects registered their only cell is the Global-row "—", a dead end (#40),
  // so we name them and steer the user to add a project.
  const projectOnlyCols = $derived(cols.filter((t) => t.supportsProject && !t.supportsUser));
  const noProjects = $derived(projects.list.length === 0);

  /** Why a cell shows "—" (not installable there), for its tooltip + a11y. */
  function naReason(row: Row, t: ToolDef): string {
    return row.kind === "global"
      ? i18n.t("install.naProjectOnly", { tool: t.label })
      : i18n.t("install.naUserOnly", { tool: t.label });
  }

  function gridCell(column: DeploymentColumn, row: DeploymentRow): DeploymentCell {
    const t = column as ToolDef;
    const destination = targetOf(row);
    const cov = cover(t.id, destination);
    const isBusy = busy === cellKey(t.id, destination);
    const unavailable = i18n.optional("reconcile.unavailableLabel", "Installation status unavailable");
    return {
      state: cov.all ? "on" : cov.some ? "partial" : "off",
      busy: isBusy,
      disabled: !installTruthFresh || total === 0,
      title: installTruthFresh ? i18n.t("install.cellTitle", { tool: t.label, target: row.kind === "global" ? i18n.t("common.global") : row.label }) : unavailable,
      ariaLabel: installTruthFresh ? i18n.t(cov.all ? "install.removeFromAria" : "install.installIntoAria", {
        tool: t.label,
        target: row.kind === "global" ? i18n.t("install.globally") : i18n.t("install.inProject", { project: row.label }),
      }) : unavailable,
    };
  }

  // Coverage of the set in one (tool, target) cell.
  function cover(tool: Tool, target: string | null) {
    const rs = install.installed.filter(
      (r) => r.state !== "missing" &&
        (exactReferences.length
          ? matchesExact(r)
          : slugSet.has(r.slug)) &&
        r.tool === tool && (r.projectPath ?? null) === target,
    );
    const present = new Set(rs.map((r) => exactReferences.length ? `${r.sourceId}:${r.relativePath}` : r.slug));
    return {
      rows: rs,
      count: present.size,
      all: total > 0 && present.size === total,
      some: present.size > 0 && present.size < total,
      hasForeign: rs.some((r) => r.state === "foreign"),
    };
  }

  let busy = $state<string | null>(null);
  const cellKey = (tool: Tool, target: string | null) => `${tool}:${target ?? ""}`;
  let confirm = $state<{ tool: Tool; target: string | null; rows: InstalledAgent[] } | null>(null);
  let pending = $state<{
    plan: AgentMutationPlan;
    operation: "install" | "update" | "uninstall";
    reference: AgentReference | null;
    collectionName: string | null;
    batchReferences: AgentReference[];
  } | null>(null);
  let planLoading = $state(false);
  let actionError = $state<string | null>(null);
  let historyRow = $state<InstalledAgent | null>(null);
  let snapshots = $state<AgentVersionSnapshot[]>([]);
  let rollbackConfirm = $state<string | null>(null);
  let diffRow = $state<InstalledAgent | null>(null);

  const exactRows = $derived(
    exactReference
      ? install.installed.filter(matchesExact).slice().sort((a, b) => a.dest.localeCompare(b.dest))
      : [],
  );
  const collectionTargets = $derived.by(() => {
    if (!collectionName) return [];
    const targets = new Map<string, { tool: Tool; projectPath: string | null; states: Set<InstallState> }>();
    for (const row of install.installed.filter(matchesExact)) {
      const key = cellKey(row.tool, row.projectPath);
      const target = targets.get(key) ?? { tool: row.tool, projectPath: row.projectPath, states: new Set<InstallState>() };
      target.states.add(row.state);
      targets.set(key, target);
    }
    return [...targets.values()];
  });
  const updatePolicy = $derived.by<AgentUpdatePolicy>(() => {
    if (!exactReference) return "notify";
    return agentLibrary.library.updatePolicies.find((entry) => sameAgent(entry.agent, exactReference))?.policy ?? "notify";
  });

  async function reviewPlan(
    operation: "install" | "update" | "uninstall",
    reference: AgentReference,
    tool: Tool,
    target: string | null,
  ) {
    if (!installTruthFresh) return;
    planLoading = true;
    actionError = null;
    try {
      pending = {
        plan: await install.plan(operation, reference, tool, target),
        operation,
        reference,
        collectionName: null,
        batchReferences: [],
      };
    } catch (error) {
      actionError = isAppError(error) ? appErrorMessage(error) : String(error);
    } finally {
      planLoading = false;
    }
  }

  let startedReviewIntent = $state("");
  $effect(() => {
    if (!reviewIntent || !installTruthFresh) return;
    const key = `${reviewIntent.operation}:${reviewIntent.reference.sourceId}:${reviewIntent.reference.relativePath}:${reviewIntent.tool}:${reviewIntent.projectPath}`;
    if (startedReviewIntent === key) return;
    startedReviewIntent = key;
    void reviewPlan(
      reviewIntent.operation,
      reviewIntent.reference,
      reviewIntent.tool,
      reviewIntent.projectPath,
    );
  });

  async function reviewCollection(
    operation: "install" | "update" | "uninstall",
    tool: Tool,
    target: string | null,
  ) {
    if (!installTruthFresh || !collectionName) return;
    planLoading = true;
    actionError = null;
    try {
      pending = {
        plan: await install.planCollection(collectionName, operation, tool, target),
        operation,
        reference: null,
        collectionName,
        batchReferences: [],
      };
    } catch (error) {
      actionError = isAppError(error) ? appErrorMessage(error) : String(error);
    } finally {
      planLoading = false;
    }
  }

  async function reviewBatch(tool: Tool, target: string | null) {
    if (!installTruthFresh || agentReferences.length === 0) return;
    planLoading = true;
    actionError = null;
    try {
      pending = {
        plan: await install.planBatch(agentReferences, tool, target),
        operation: "install",
        reference: null,
        collectionName: null,
        batchReferences: agentReferences,
      };
    } catch (error) {
      actionError = isAppError(error) ? appErrorMessage(error) : String(error);
    } finally {
      planLoading = false;
    }
  }

  async function applyPlan() {
    if (!installTruthFresh || !pending || !canApplyAgentPlan(pending.plan)) return;
    const { operation, reference, collectionName: pendingCollection, batchReferences, plan } = pending;
    busy = cellKey(plan.tool, plan.projectPath);
    actionError = null;
    try {
      let receiptId: string | null = null;
      if (pendingCollection) {
        ({ receiptId } = await install.applyCollection(pendingCollection, plan));
      } else if (batchReferences.length) {
        ({ receiptId } = await install.applyBatch(plan));
      } else if (operation === "install" && reference) {
        await install.installReference(reference, plan.tool, plan.projectPath, true);
      } else if (operation === "update" && reference) {
        await install.updateReference(reference, plan.tool, plan.projectPath, true);
      } else if (reference) {
        await install.uninstallReference(reference, plan.tool, plan.projectPath);
      } else {
        throw new Error("Agent mutation plan has no exact target");
      }
      const receiptAction = receiptId
        ? { label: i18n.t("activity.viewReceipt"), onClick: () => ui.openActivityReceipt(receiptId) }
        : undefined;
      toast.success(i18n.t("agents.lifecycleApplied", { operation }), undefined, receiptAction);
      pending = null;
      onApplied?.(plan);
    } catch (error) {
      actionError = isAppError(error) ? appErrorMessage(error) : String(error);
    } finally {
      busy = null;
    }
  }

  async function runLifecycle(
    action: "track" | "disable" | "enable",
    row: InstalledAgent,
  ) {
    if (!installTruthFresh || !exactReference) return;
    busy = cellKey(row.tool, row.projectPath);
    actionError = null;
    try {
      if (action === "track") await install.trackReference(exactReference, row.tool, row.projectPath);
      else if (action === "disable") await install.disableReference(exactReference, row.tool, row.projectPath);
      else await install.enableReference(exactReference, row.tool, row.projectPath);
      toast.success(i18n.t("agents.lifecycleApplied", { operation: action }));
    } catch (error) {
      actionError = isAppError(error) ? appErrorMessage(error) : String(error);
    } finally {
      busy = null;
    }
  }

  async function showHistory(row: InstalledAgent) {
    if (!exactReference) return;
    historyRow = row;
    snapshots = [];
    rollbackConfirm = null;
    actionError = null;
    try {
      snapshots = await install.history(exactReference, row.tool, row.projectPath);
    } catch (error) {
      actionError = isAppError(error) ? appErrorMessage(error) : String(error);
    }
  }

  async function rollback(snapshotId: string) {
    if (!install.reconciled || install.reconciling || install.reconcileError || !exactReference || !historyRow) return;
    if (rollbackConfirm !== snapshotId) {
      rollbackConfirm = snapshotId;
      return;
    }
    busy = cellKey(historyRow.tool, historyRow.projectPath);
    try {
      await install.rollbackReference(
        exactReference, historyRow.tool, historyRow.projectPath, snapshotId,
      );
      toast.success(i18n.t("agents.rollbackSucceeded"));
      historyRow = null;
      rollbackConfirm = null;
    } catch (error) {
      actionError = isAppError(error) ? appErrorMessage(error) : String(error);
    } finally {
      busy = null;
    }
  }

  async function setPolicy(event: Event) {
    if (!exactReference) return;
    await agentLibrary.setUpdatePolicy(
      exactReference,
      (event.currentTarget as HTMLSelectElement).value as AgentUpdatePolicy,
    );
  }

  async function toggle(tool: Tool, target: string | null) {
    if (!installTruthFresh || busy) return;
    const cov = cover(tool, target);
    if (cov.all) {
      if (collectionName) {
        await reviewCollection("uninstall", tool, target);
        return;
      }
      if (exactReference) {
        await reviewPlan("uninstall", exactReference, tool, target);
        return;
      }
      if (cov.hasForeign) {
        confirm = { tool, target, rows: cov.rows };
        return;
      }
      await remove(tool, target, cov.rows);
      return;
    }
    if (collectionName) {
      await reviewCollection("install", tool, target);
      return;
    }
    if (agentReferences.length) {
      await reviewBatch(tool, target);
      return;
    }
    if (exactReference) {
      await reviewPlan("install", exactReference, tool, target);
      return;
    }
    const present = new Set(cov.rows.map((r) => r.slug));
    const missing = agents.filter((a) => !present.has(a.slug));
    if (missing.length === 0) return;
    busy = cellKey(tool, target);
    try {
      const { ok, fail, receiptId } = await install.bulk(
        "install",
        missing.map((a) => ({ slug: a.slug, tool, projectPath: target })),
      );
      const receiptAction = { label: i18n.t("activity.viewReceipt"), onClick: () => ui.openActivityReceipt(receiptId) };
      const where = target ? labelOf(target) : i18n.t("common.global");
      if (fail === 0) toast.success(i18n.t("install.installedToast", { count: ok, tool: install.toolLabel(tool), where }), undefined, receiptAction);
      else toast.error(i18n.t("install.installFailedToast", { tool: install.toolLabel(tool), ok, fail }), undefined, receiptAction);
    } finally {
      busy = null;
    }
  }

  async function remove(tool: Tool, target: string | null, rs: InstalledAgent[]) {
    if (!install.reconciled || install.reconciling || install.reconcileError) return;
    busy = cellKey(tool, target);
    try {
      const { ok, fail, receiptId } = await install.bulk(
        "uninstall",
        rs.map((r) => ({ slug: r.slug, tool: r.tool, projectPath: r.projectPath })),
      );
      const receiptAction = { label: i18n.t("activity.viewReceipt"), onClick: () => ui.openActivityReceipt(receiptId) };
      if (fail === 0) toast.success(i18n.t("install.removedToast", { count: ok, tool: install.toolLabel(tool) }), undefined, receiptAction);
      else toast.error(i18n.t("install.removeFailedToast", { tool: install.toolLabel(tool), ok, fail }), undefined, receiptAction);
    } finally {
      busy = null;
    }
  }

  async function confirmRemove() {
    if (!install.reconciled || install.reconciling || install.reconcileError || !confirm) return;
    const { tool, target, rows: rs } = confirm;
    confirm = null;
    await remove(tool, target, rs);
  }

  function labelOf(path: string): string {
    return path.replace(/\/+$/, "").split("/").pop() || path;
  }

  // ── "Add project…" popover ──
  // Instead of firing a native folder dialog on click (which feels broken in the
  // dev shim and is easy to spam), open a list of the projects you already manage
  // — clicking one jumps to its grid row — with an explicit "New Project…" that
  // opens the picker only when you mean it.
  let addOpen = $state(false);
  let flashPath = $state<string | null>(null);
  const destEls: Record<string, HTMLElement> = {};

  function regDest(node: HTMLElement, path: string | null) {
    if (path) destEls[path] = node;
    return { destroy() { if (path) delete destEls[path]; } };
  }

  function flash(path: string) {
    flashPath = path;
    setTimeout(() => { if (flashPath === path) flashPath = null; }, 1200);
  }

  function jumpTo(path: string) {
    addOpen = false;
    destEls[path]?.scrollIntoView({ block: "nearest" });
    flash(path);
  }

  async function newProject() {
    addOpen = false;
    const p = await projects.addViaPicker();
    if (p) {
      await projects.refresh();
      flash(p);
    }
  }
</script>

<Modal open {title} size="wide" onClose={onClose}>
  <p class="sub">{i18n.t("install.sub", { count: total })}</p>

  {#if exactReference}
    <section class="provenance" aria-label={i18n.t("agents.sourceProvenance")}>
      <strong>{i18n.t("agents.sourceProvenance")}</strong>
      <code>{exactReference.sourceId}:{exactReference.relativePath}</code>
      <label>
        <span>{i18n.t("agents.updatePolicy")}</span>
        <select value={updatePolicy} onchange={setPolicy}>
          <option value="notify">{i18n.t("agents.policyNotify")}</option>
          <option value="autoTrusted">{i18n.t("agents.policyAutoTrusted")}</option>
          <option value="pin">{i18n.t("agents.policyPin")}</option>
          <option value="reviewScripts">{i18n.t("agents.policyReviewScripts")}</option>
        </select>
      </label>
      <p>{i18n.t(`agents.policyHelp.${updatePolicy}`)}</p>
    </section>
  {:else if collectionName}
    <section class="provenance" aria-label={i18n.t("agents.collectionMembers")}>
      <strong>{i18n.t("agents.collectionMembers")}: {collectionName}</strong>
      {#each exactReferences as reference (`${reference.sourceId}:${reference.relativePath}`)}
        <code>{reference.sourceId}:{reference.relativePath}</code>
      {/each}
    </section>
  {/if}

  {#if actionError}<p class="plan-error" role="alert">{actionError}</p>{/if}
  {#if planLoading}<p class="sub">{i18n.t("agents.loadingPlan")}</p>{/if}

  {#if pending}
    <section class="plan" aria-label={i18n.t("agents.mutationPlan")}>
      <h3>{i18n.t("agents.mutationPlan")}: {pending.operation}</h3>
      {#if pending.plan.blockers.length > 0}
        <div class="plan-blockers" role="alert">
          <strong>{i18n.t("agents.planBlocked")}</strong>
          <ul>{#each pending.plan.blockers as blocker}<li>{blocker}</li>{/each}</ul>
        </div>
      {/if}
      {#if pending.plan.warnings.length > 0}
        <div class="plan-warnings">
          <strong>{i18n.t("agents.planWarnings")}</strong>
          <ul>{#each pending.plan.warnings as warning}<li>{warning}</li>{/each}</ul>
        </div>
      {/if}
      <ul class="plan-agents">
        {#each pending.plan.agents as item (`${item.reference.sourceId}:${item.reference.relativePath}`)}
          <li>
            <strong>{item.name}</strong>{#if item.dependency} · {i18n.t("agents.requiredDependency")}{/if}
            <code>{item.reference.sourceId}:{item.reference.relativePath}</code>
            <span>{item.destination} · {i18n.t("agents.fileCount", { count: item.renderedFileCount })}</span>
            <span>{i18n.t("agents.capabilities")}: {item.capabilities.length ? item.capabilities.join(", ") : i18n.t("common.none")}</span>
          </li>
        {/each}
      </ul>
      <p class="rollback-note">{pending.plan.rollbackAvailable ? i18n.t("agents.rollbackAvailable") : i18n.t("agents.rollbackUnavailable")}</p>
    </section>
  {/if}

  {#if exactReference && exactRows.length > 0 && !pending}
    <section class="lifecycle" aria-label={i18n.t("agents.lifecycleInstalls")}>
      <h3>{i18n.t("agents.lifecycleInstalls")}</h3>
      {#each exactRows as row (row.dest)}
        <article class="install-row">
          <div class="install-facts">
            <strong>{install.toolLabel(row.tool)}{#if row.projectPath} · {labelOf(row.projectPath)}{/if}</strong>
            <span class="state-text" data-state={row.state}>{i18n.t(installStateMessageKey(row.state))}</span>
            <code title={row.dest}>{row.dest}</code>
            <small>{row.sourceId || exactReference.sourceId}:{row.relativePath || exactReference.relativePath}</small>
          </div>
          <div class="row-actions">
            {#if ["foreign", "modified", "outdated"].includes(row.state)}
              <button onclick={() => (diffRow = row)}>{i18n.t("agents.reviewDiff")}</button>
            {/if}
            {#if ["outdated", "modified", "missing"].includes(row.state)}
              <button disabled={!installTruthFresh} onclick={() => reviewPlan("update", exactReference, row.tool, row.projectPath)}>{i18n.t("common.update")}</button>
            {/if}
            {#if row.state === "foreign"}
              <button disabled={!installTruthFresh} onclick={() => runLifecycle("track", row)}>{i18n.t("agents.track")}</button>
            {:else}
              {#if row.state === "disabled"}
                <button disabled={!installTruthFresh} onclick={() => runLifecycle("enable", row)}>{i18n.t("agents.enable")}</button>
              {:else if row.state === "current" || row.state === "outdated"}
                <button disabled={!installTruthFresh} onclick={() => runLifecycle("disable", row)}>{i18n.t("agents.disable")}</button>
              {/if}
              <button onclick={() => showHistory(row)}>{i18n.t("agents.versionHistory")}</button>
              <button class="danger" disabled={!installTruthFresh} onclick={() => reviewPlan("uninstall", exactReference, row.tool, row.projectPath)}>{i18n.t("common.uninstall")}</button>
            {/if}
          </div>
        </article>
      {/each}
    </section>
  {/if}

  {#if collectionName && collectionTargets.length > 0 && !pending}
    <section class="lifecycle" aria-label={i18n.t("agents.collectionLifecycle")}>
      <h3>{i18n.t("agents.collectionLifecycle")}</h3>
      {#each collectionTargets as target (`${target.tool}:${target.projectPath ?? ""}`)}
        <article class="install-row">
          <div class="install-facts">
            <strong>{install.toolLabel(target.tool)}{#if target.projectPath} · {labelOf(target.projectPath)}{/if}</strong>
            <span>{[...target.states].map((state) => i18n.t(installStateMessageKey(state))).join(", ")}</span>
          </div>
          <div class="row-actions">
            <button disabled={!installTruthFresh} onclick={() => reviewCollection("update", target.tool, target.projectPath)}>{i18n.t("common.update")}</button>
            <button class="danger" disabled={!installTruthFresh} onclick={() => reviewCollection("uninstall", target.tool, target.projectPath)}>{i18n.t("common.uninstall")}</button>
          </div>
        </article>
      {/each}
    </section>
  {/if}

  {#if historyRow && !pending}
    <section class="history" aria-label={i18n.t("agents.versionHistory")}>
      <div class="history-head"><h3>{i18n.t("agents.versionHistory")}</h3><button onclick={() => (historyRow = null)}>{i18n.t("common.close")}</button></div>
      {#if snapshots.length === 0}
        <p class="sub">{i18n.t("agents.noVersionHistory")}</p>
      {:else}
        {#each snapshots as snapshot (snapshot.id)}
          <div class="snapshot">
            <span>{new Date(snapshot.createdAt).toLocaleString()}</span>
            <code>{snapshot.sourceHash.slice(0, 12)}</code>
            <button disabled={!installTruthFresh} onclick={() => rollback(snapshot.id)}>
              {rollbackConfirm === snapshot.id ? i18n.t("agents.confirmRollback") : i18n.t("agents.rollback")}
            </button>
          </div>
        {/each}
      {/if}
    </section>
  {/if}

  {#if !pending && cols.length === 0}
    <p class="no-tools">{i18n.t("install.noTools")}</p>
  {:else if !pending}
    <DeploymentTargetGrid
      columns={cols}
      {rows}
      cell={gridCell}
      onToggle={(column, row) => void toggle(column.id, targetOf(row))}
      notApplicable={(column, row) => naReason(row, column as ToolDef)}
      {flashPath}
      registerDestination={regDest}
    />
  {/if}

  {#if cols.length > 0 && projectOnlyCols.length > 0 && noProjects}
    <p class="scope-hint">
      <FolderPlus size={13} />
      <span>{i18n.t("install.projectOnlyHint", { tools: projectOnlyCols.map((t) => t.label).join(", ") })}</span>
    </p>
  {/if}

  <div class="add-wrap">
    <button class="addrow" onclick={() => (addOpen = !addOpen)} aria-haspopup="menu" aria-expanded={addOpen}>
      <FolderPlus size={14} /> {i18n.t("install.addProject")}
    </button>
    {#if addOpen}
      <button class="add-scrim" aria-label={i18n.t("common.close")} onclick={() => (addOpen = false)}></button>
      <div class="add-menu" role="menu">
        {#if projects.list.length > 0}
          <p class="add-head">{i18n.t("install.yourProjects")}</p>
          {#each projects.list as p (p.path)}
            <button class="add-opt" role="menuitem" onclick={() => jumpTo(p.path)}>
              <FolderIcon size={14} />
              <span class="add-body">
                <span class="add-label">{p.label}</span>
                <span class="add-path" title={p.path}>{p.path}</span>
              </span>
            </button>
          {/each}
          <div class="add-div"></div>
        {/if}
        <button class="add-opt new" role="menuitem" onclick={newProject}>
          <FolderPlus size={14} /> <span>{i18n.t("install.newProject")}</span>
        </button>
      </div>
    {/if}
  </div>

  {#snippet actions()}
    {#if pending}
      <Button onclick={() => (pending = null)}>{i18n.t("common.cancel")}</Button>
      <Button variant="primary" disabled={!installTruthFresh || !canApplyAgentPlan(pending.plan) || !!busy} onclick={applyPlan}>{i18n.t("agents.applyPlan")}</Button>
    {:else}
      <span class="legend"><span class="dot full"></span> {i18n.t("common.installed")} <span class="dot half"></span> {i18n.t("common.some")} <span class="dot"></span> {i18n.t("common.none")}</span>
      <Button variant="primary" onclick={onClose}>{i18n.t("common.done")}</Button>
    {/if}
  {/snippet}
</Modal>

{#if diffRow && agentPackage?.agent}
  <DiffModal
    slug={agentPackage.agent.slug}
    reference={agentPackage.reference}
    tool={diffRow.tool}
    projectPath={diffRow.projectPath}
    name={agentPackage.agent.name}
    onClose={() => (diffRow = null)}
  />
{/if}

{#if confirm}
  {@const n = confirm.rows.length}
  {@const label = install.toolLabel(confirm.tool)}
  <DestructiveConfirm
    open
    title={i18n.t("install.deleteTitle", { count: n, label })}
    confirmLabel={i18n.t("install.deleteConfirm", { count: n })}
    cancelLabel={i18n.t("common.cancel")}
    confirmDisabled={!installTruthFresh}
    onConfirm={confirmRemove}
    onCancel={() => (confirm = null)}
  >
    <p>
      {i18n.t("install.deleteBody", { count: n })}
    </p>
  </DestructiveConfirm>
{/if}

<style>
  .sub { font-size: var(--text-body-sm); color: var(--color-text-muted); margin-bottom: var(--space-3); }
  .no-tools { font-size: var(--text-body-sm); color: var(--color-text-muted); }

  .provenance, .plan, .lifecycle, .history {
    display: flex; flex-direction: column; gap: var(--space-2);
    padding: var(--space-3); border: 1px solid var(--color-border);
    border-radius: var(--radius-md); background: var(--color-surface-sunken);
  }
  .provenance code, .install-facts code, .install-facts small, .plan-agents code {
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    font-family: var(--font-mono); font-size: var(--text-caption); color: var(--color-text-muted);
  }
  .provenance label { display: flex; align-items: center; gap: var(--space-2); font-size: var(--text-body-sm); }
  .provenance select { padding: 5px 8px; border: 1px solid var(--color-border); border-radius: var(--radius-sm); background: var(--color-surface-raised); color: var(--color-text-primary); }
  .provenance p, .rollback-note { margin: 0; font-size: var(--text-caption); color: var(--color-text-muted); }
  .plan h3, .lifecycle h3, .history h3 { margin: 0; font-size: var(--text-body); }
  .plan ul { margin: var(--space-1) 0 0; padding-left: var(--space-5); }
  .plan-blockers { color: var(--color-danger); }
  .plan-warnings { color: var(--color-warning); }
  .plan-agents { display: flex; flex-direction: column; gap: var(--space-2); }
  .plan-agents li { display: flex; flex-direction: column; gap: 2px; color: var(--color-text-secondary); }
  .plan-error { padding: var(--space-2); color: var(--color-danger); background: color-mix(in srgb, var(--color-danger) 10%, transparent); border-radius: var(--radius-sm); font-size: var(--text-body-sm); }
  .install-row { display: flex; align-items: center; gap: var(--space-3); padding: var(--space-2) 0; border-top: 1px solid var(--color-border); }
  .install-facts { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; font-size: var(--text-body-sm); }
  .state-text { font-size: var(--text-caption); font-weight: var(--fw-semibold); }
  .state-text[data-state="current"] { color: var(--color-success); }
  .state-text[data-state="outdated"], .state-text[data-state="modified"] { color: var(--color-warning); }
  .state-text[data-state="foreign"] { color: var(--color-brand); }
  .state-text[data-state="missing"], .state-text[data-state="sourceUnavailable"] { color: var(--color-danger); }
  .state-text[data-state="disabled"] { color: var(--color-text-muted); }
  .row-actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 4px; }
  .row-actions button, .history button, .snapshot button {
    padding: 5px 8px; border: 1px solid var(--color-border); border-radius: var(--radius-sm);
    background: var(--color-surface-raised); color: var(--color-text-secondary); font-size: var(--text-caption); cursor: pointer;
  }
  .row-actions button:hover, .history button:hover, .snapshot button:hover { color: var(--color-text-primary); border-color: var(--color-brand); }
  .row-actions button.danger { color: var(--color-danger); }
  .history-head, .snapshot { display: flex; align-items: center; justify-content: space-between; gap: var(--space-2); }
  .snapshot code { font-family: var(--font-mono); font-size: var(--text-caption); color: var(--color-text-muted); }


  .scope-hint {
    display: flex; align-items: center; gap: 7px;
    margin-top: var(--space-3); padding: var(--space-2) var(--space-3);
    background: color-mix(in srgb, var(--color-brand) 8%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-brand) 22%, transparent);
    border-radius: var(--radius-md);
    font-size: var(--text-body-sm); color: var(--color-text-secondary);
  }
  .scope-hint :global(svg) { flex: none; color: var(--color-brand); }

  .add-wrap { position: relative; display: inline-block; margin-top: var(--space-2); }
  .addrow {
    display: inline-flex; align-items: center; gap: 6px;
    padding: var(--space-2);
    background: transparent; color: var(--color-brand); font-size: var(--text-body-sm); cursor: pointer;
  }
  .addrow:hover { text-decoration: underline; }

  /* Backdrop closes the popover on any outside click. */
  .add-scrim { position: fixed; inset: 0; z-index: 1; background: transparent; border: 0; cursor: default; }
  .add-menu {
    position: absolute; bottom: calc(100% + 4px); left: 0; z-index: 2;
    min-width: 260px; max-width: 360px; max-height: 280px; overflow-y: auto; padding: 4px;
    background: var(--color-surface-raised); border: 1px solid var(--color-border);
    border-radius: var(--radius-md); box-shadow: var(--shadow-lg);
    display: flex; flex-direction: column; gap: 1px;
  }
  .add-head {
    padding: 6px 8px 2px; font-size: var(--text-caption); font-weight: var(--fw-semibold);
    color: var(--color-text-muted); text-transform: uppercase; letter-spacing: 0.04em;
  }
  .add-opt {
    display: flex; align-items: center; gap: var(--space-2);
    padding: 6px 8px; border-radius: var(--radius-sm);
    background: transparent; color: var(--color-text-primary);
    font-size: var(--text-body-sm); text-align: left; cursor: pointer; min-width: 0;
  }
  .add-opt:hover { background: var(--color-surface-sunken); }
  .add-opt.new { color: var(--color-brand); font-weight: var(--fw-medium); }
  .add-body { display: flex; flex-direction: column; gap: 0; min-width: 0; }
  .add-label { font-weight: var(--fw-medium); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .add-path { font-size: var(--text-caption); color: var(--color-text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .add-div { height: 1px; margin: 4px 0; background: var(--color-border); }

  .legend { display: inline-flex; align-items: center; gap: 6px; margin-right: auto; font-size: var(--text-caption); color: var(--color-text-muted); }
  .dot { width: 16px; height: 16px; border: 1.5px solid var(--color-border-strong, var(--color-text-muted)); border-radius: 999px; box-sizing: border-box; }
  .dot.full { border-color: var(--color-brand); background: var(--color-brand); }
  .dot.half { border-color: var(--color-brand); background: linear-gradient(90deg, var(--color-brand) 50%, transparent 50%); }
  .legend .dot { width: 13px; height: 13px; }
</style>
