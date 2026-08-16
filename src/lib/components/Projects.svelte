<script lang="ts">
  /**
   * Projects — the 4th pillar (Agents / Tools / Teams / Projects). The home for
   * per-project deployments: every folder you've installed agents into.
   *
   * Master/detail (not inline disclosures — a big roster gets unruly):
   *  • List: a row per project (folder · label · path · count). Clicking a row
   *    navigates into its detail via `ui.selectProject(path)` — a nav location,
   *    so the title-bar Back button returns to the list.
   *  • Detail: that project's path, actions (Deploy… · Reveal · Remove from
   *    list), and its roster grouped by division (collapsible), reusing the
   *    division-group pattern from Tools/Teams.
   *
   * "Deploy…" opens the two-pane DeployBrowser scoped to the project (pick an
   * agent, division, or team on the left; install into the project's tools on
   * the right) — so an empty project can be filled.
   */
  import { onMount, tick, untrack } from "svelte";
  import EmptyState from "./EmptyState.svelte";
  import Pill from "./Pill.svelte";
  import Button from "./Button.svelte";
  import Modal from "./Modal.svelte";
  import DeployBrowser from "./DeployBrowser.svelte";
  import InstallModal from "./InstallModal.svelte";
  import FolderIcon from "@lucide/svelte/icons/folder";
  import FolderPlus from "@lucide/svelte/icons/folder-plus";
  import FolderOpen from "@lucide/svelte/icons/folder-open";
  import ChevronRight from "@lucide/svelte/icons/chevron-right";
  import ChevronDown from "@lucide/svelte/icons/chevron-down";
  import LayersIcon from "@lucide/svelte/icons/layers";
  import Trash2 from "@lucide/svelte/icons/trash-2";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";

  import { install } from "$lib/stores/install.svelte";
  import { agentLibrary } from "$lib/stores/agentLibrary.svelte";
  import { corpus } from "$lib/stores/corpus.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { skillSources } from "$lib/stores/skillSources.svelte";
  import { ui } from "$lib/stores/ui.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { resolveCategoryIcon } from "$lib/util/categoryIcon";
  import { i18n } from "$lib/stores/i18n.svelte";
  import { appErrorMessage, isAppError, type AgentPackageResult, type InstalledAgent, type ProjectReadinessBaseline, type ProjectReadinessReport, type ProjectRecommendation } from "$lib/types";
  import type { ProjectInstructionApplyResult, ProjectInstructionOperation, ProjectInstructionPlan, ProjectInstructionSnippet, ProjectInstructionTarget } from "$lib/types";
  import { diffLines, diffStat, type DiffRow } from "$lib/util/diff";

  const installTruthFresh = $derived(install.reconciled && !install.reconcileError);
  const mutationTruthFresh = $derived(installTruthFresh && skillSources.reconciled && !skillSources.reconcileError);
  const installTruthMessage = $derived(install.reconcileError ? i18n.optional("reconcile.unavailable", "Installation status is unavailable until a retry succeeds.") : i18n.optional("reconcile.checking", "Checking installation status…"));
  let projectsRoot: HTMLElement | undefined = $state();
  let reconcileAnnouncement = $state("");
  let priorReconcileError: string | null = $state(null);
  let priorReconcileTerminal = $state(0);
  let priorReconciling = $state(false);
  $effect(() => {
    const { reconcileError: error, reconciling, reconcileTerminal: terminal } = install;
    if (reconciling && !priorReconciling) reconcileAnnouncement = i18n.optional("reconcile.refreshing", error ? "Refreshing installation status…" : "Checking installation status…");
    else if (error && (error !== priorReconcileError || terminal !== priorReconcileTerminal)) reconcileAnnouncement = priorReconcileError ? i18n.optional("reconcile.stillOutOfDate", "Installation status is still out of date. {message}", { message: error }) : i18n.optional("reconcile.outOfDate", "Installation status may be out of date. {message}", { message: error });
    else if (!error && priorReconcileError && !reconciling) reconcileAnnouncement = i18n.optional("reconcile.upToDate", "Installation status is up to date.");
    priorReconcileError = error; priorReconcileTerminal = terminal; priorReconciling = reconciling;
  });
  async function retryReconcile(event: MouseEvent): Promise<void> {
    const restoreFocus = event.currentTarget === document.activeElement;
    await install.reconcile();
    if (!install.reconcileError && restoreFocus) { await tick(); projectsRoot?.focus({ preventScroll: true }); }
  }

  onMount(() => {
    corpus.ensureLoaded();
    void (async () => {
      await projects.refresh();
      await skillSources.reconcileInstalls(projects.list.map((project) => project.path));
    })();
  });

  // ── Per-project roster: rows we (or anyone) deployed into that exact path. ──
  // Keyed by the project's absolute path; null projectPath = global, excluded.
  const rowsByProject = $derived.by(() => {
    const m = new Map<string, InstalledAgent[]>();
    for (const r of install.installed) {
      if (r.state === "missing") continue; // ledger says installed but file gone
      const p = r.projectPath;
      if (p == null) continue; // global scope lives in Teams/Tools, not here
      const arr = m.get(p);
      if (arr) arr.push(r);
      else m.set(p, [r]);
    }
    return m;
  });

  function rosterFor(path: string): InstalledAgent[] {
    return rowsByProject.get(path) ?? [];
  }

  function skillsFor(path: string) {
    return skillSources.installed.filter(
      (installed) => installed.tracked && installed.projectPath === path,
    );
  }

  // ── Selected project (detail pane). Resolve against the live list so a stale
  //    path (e.g. removed) falls back to the list rather than an empty detail. ──
  const selected = $derived(projects.list.find((p) => p.path === ui.projectsSelected) ?? null);

  // ── Group the selected project's roster by division (collapsible). ──
  const OTHER = "__other";
  const detailGroups = $derived.by(() => {
    if (!selected) return [];
    const divOf = new Map(corpus.agents.map((a) => [a.slug, a.category]));
    const m = new Map<string, InstalledAgent[]>();
    for (const r of rosterFor(selected.path)) {
      const div = divOf.get(r.slug) ?? OTHER;
      const arr = m.get(div);
      if (arr) arr.push(r);
      else m.set(div, [r]);
    }
    const out = [...m.entries()].map(([slug, rows]) => ({
      slug,
      label: slug === OTHER ? i18n.t("common.other") : corpus.labelOf(slug),
      color: slug === OTHER ? "#94A3B8" : corpus.colorOf(slug),
      icon: slug === OTHER ? "HelpCircle" : corpus.iconOf(slug),
      rows: rows.slice().sort((a, b) => a.name.localeCompare(b.name)),
    }));
    out.sort((a, b) => (a.slug === OTHER ? 1 : b.slug === OTHER ? -1 : a.label.localeCompare(b.label)));
    return out;
  });

  // Division groups are CLOSED by default; initialize the collapse set to every
  // division once per project (guarded so installs/removes don't re-collapse).
  let collapsed = $state<Set<string>>(new Set());
  function toggleGroup(slug: string) {
    const next = new Set(collapsed);
    if (next.has(slug)) next.delete(slug);
    else next.add(slug);
    collapsed = next;
  }
  let collapseInitFor: string | null = null;
  $effect(() => {
    const p = ui.projectsSelected;
    // Wait for the roster to populate before seeding (cold corpus load) so the
    // divisions stay closed by default rather than seeding from an empty list.
    if (p === collapseInitFor || detailGroups.length === 0) return;
    collapseInitFor = p;
    collapsed = new Set(detailGroups.map((g) => g.slug));
  });

  // ── Deploy into a project: the two-pane DeployBrowser. ──
  let browseFor = $state<string | null>(null); // project path, or null = closed

  // ── Readiness baseline + opt-in catalog recommendations. ──
  let readiness = $state<ProjectReadinessReport | null>(null);
  let recommendations = $state<ProjectRecommendation[]>([]);
  let readinessBusy = $state(false);
  let readinessError = $state<string | null>(null);
  let readinessAnnouncement = $state("");
  let recommendationPlan = $state<ProjectRecommendation | null>(null);
  let recommendationPackage = $state<AgentPackageResult | null>(null);
  let recommendationTargetIndex = $state(0);
  let recommendationTrigger: HTMLButtonElement | undefined = $state();
  let recommendationTriggerId = $state("");
  let instructionManager: HTMLElement | undefined = $state();
  let readinessGeneration = 0;
  const newRecommendationCount = $derived(
    recommendations.filter((recommendation) => recommendation.lifecycle === "new").length,
  );

  function readinessCategoryLabel(category: ProjectReadinessReport["categories"][number]["category"]): string {
    return ({ agentRoster: "Agent roster", skills: "Skills", instructions: "Instructions", mcp: "MCP", tools: "Tools" })[category];
  }

  async function refreshReadiness(): Promise<void> {
    if (!selected || readinessBusy) return;
    const projectPath = selected.path;
    const generation = ++readinessGeneration;
    readinessBusy = true;
    readinessError = null;
    readinessAnnouncement = "Checking project readiness…";
    try {
      const report = await projects.readiness(projectPath);
      const nextRecommendations = report.subscribed
        ? await projects.recommendations(projectPath)
        : [];
      if (generation !== readinessGeneration) return;
      readiness = report;
      recommendations = nextRecommendations;
      const surfaced = nextRecommendations.filter((item) => item.lifecycle === "new");
      readinessAnnouncement = `Readiness ${report.overall}. ${surfaced.length} new recommendations.`;
      await tick();
      if (generation !== readinessGeneration) return;
      if (surfaced.length > 0) {
        const cursor = surfaced.reduce((latest, item) =>
          Date.parse(item.batchAt) > Date.parse(latest) ? item.batchAt : latest,
        surfaced[0].batchAt);
        try {
          await projects.acknowledgeRecommendations(
            projectPath,
            cursor,
            surfaced.map((item) => item.id),
          );
        } catch (error) {
          if (generation === readinessGeneration) {
            readinessError = isAppError(error) ? appErrorMessage(error) : String(error);
            readinessAnnouncement = `Recommendations are visible but could not be acknowledged. ${readinessError}`;
          }
        }
      }
    } catch (error) {
      if (generation === readinessGeneration) {
        readinessError = isAppError(error) ? appErrorMessage(error) : String(error);
        readinessAnnouncement = `Readiness unavailable. ${readinessError}`;
      }
    } finally {
      if (generation === readinessGeneration) readinessBusy = false;
    }
  }

  $effect(() => {
    const path = selected?.path;
    untrack(() => {
      readinessGeneration += 1;
      readinessBusy = false;
      readiness = null;
      recommendations = [];
      readinessError = null;
      if (path) void refreshReadiness();
    });
  });

  async function importReadinessPack(): Promise<void> {
    if (!selected || readinessBusy) return;
    const projectPath = selected.path;
    const generation = readinessGeneration;
    const picked = await openDialog({
      title: "Import Workspace Pack readiness baseline",
      multiple: false,
      filters: [{ name: "Workspace Pack", extensions: ["json"] }],
    });
    if (typeof picked !== "string" || generation !== readinessGeneration || selected?.path !== projectPath) return;
    readinessBusy = true;
    readinessError = null;
    try {
      const plan = await install.inspectWorkspacePack(picked, projectPath);
      if (generation !== readinessGeneration || selected?.path !== projectPath) return;
      if (plan.blockers.length > 0) throw new Error(plan.blockers.join(" "));
      await projects.importPackBaseline(projectPath, plan.pack);
      if (generation !== readinessGeneration || selected?.path !== projectPath) return;
      readinessAnnouncement = "Workspace Pack baseline saved.";
    } catch (error) {
      readinessError = isAppError(error) ? appErrorMessage(error) : String(error);
    } finally {
      if (generation === readinessGeneration) readinessBusy = false;
    }
    if (generation === readinessGeneration && selected?.path === projectPath) await refreshReadiness();
  }

  async function setReadinessSubscription(enabled: boolean): Promise<void> {
    if (!selected || readinessBusy) return;
    const projectPath = selected.path;
    const generation = readinessGeneration;
    readinessBusy = true;
    readinessError = null;
    try {
      await projects.subscribe(projectPath, enabled);
      if (generation !== readinessGeneration || selected?.path !== projectPath) return;
      readinessAnnouncement = enabled ? "Catalog recommendations enabled." : "Catalog recommendations disabled.";
    } catch (error) {
      readinessError = isAppError(error) ? appErrorMessage(error) : String(error);
    } finally {
      if (generation === readinessGeneration) readinessBusy = false;
    }
    if (generation === readinessGeneration && selected?.path === projectPath) await refreshReadiness();
  }

  async function openRecommendation(event: MouseEvent, recommendation: ProjectRecommendation): Promise<void> {
    if (!selected || readinessBusy) return;
    const projectPath = selected.path;
    const generation = readinessGeneration;
    recommendationTrigger = event.currentTarget as HTMLButtonElement;
    recommendationTriggerId = recommendation.id;
    readinessBusy = true;
    readinessError = null;
    try {
      const opened = await projects.openRecommendation(projectPath, recommendation.id);
      await agentLibrary.load(true);
      if (generation !== readinessGeneration || selected?.path !== projectPath) return;
      if (agentLibrary.error) throw new Error(agentLibrary.error);
      const references = [...opened.agentReferences, ...opened.targets.map((target) => target.reference)]
        .filter((reference, index, all) => all.findIndex((candidate) =>
          candidate.sourceId === reference.sourceId && candidate.relativePath === reference.relativePath,
        ) === index);
      const packages = references.map((reference) => agentLibrary.packages.find(
        (item) => item.reference.sourceId === reference.sourceId
          && item.reference.relativePath === reference.relativePath,
      ));
      if (packages.some((item) => !item)) {
        throw new Error("Recommendation references are absent from the refreshed Agent library");
      }
      const target = opened.targets.find((item) => item.operation !== "informational");
      if (!target) throw new Error("Recommendation has no safe deployment review target");
      recommendationPackage = packages.find((item) => item?.reference.sourceId === target.reference.sourceId
        && item.reference.relativePath === target.reference.relativePath) ?? null;
      if (!recommendationPackage) throw new Error("Recommendation target is absent from the refreshed Agent library");
      recommendationTargetIndex = 0;
      recommendationPlan = opened;
    } catch (error) {
      readinessError = isAppError(error) ? appErrorMessage(error) : String(error);
      readinessAnnouncement = `Recommendation could not be opened. ${readinessError}`;
    } finally {
      if (generation === readinessGeneration) readinessBusy = false;
    }
  }

  async function dismissRecommendation(recommendation: ProjectRecommendation): Promise<void> {
    if (!selected || readinessBusy) return;
    const projectPath = selected.path;
    const generation = readinessGeneration;
    readinessBusy = true;
    readinessError = null;
    try {
      await projects.dismissRecommendation(projectPath, recommendation.id);
      if (generation !== readinessGeneration || selected?.path !== projectPath) return;
      readinessAnnouncement = "Recommendation dismissed.";
    } catch (error) {
      if (generation === readinessGeneration) {
        readinessError = isAppError(error) ? appErrorMessage(error) : String(error);
        readinessAnnouncement = `Recommendation could not be dismissed. ${readinessError}`;
      }
    } finally {
      if (generation === readinessGeneration) readinessBusy = false;
    }
    if (generation === readinessGeneration && selected?.path === projectPath) await refreshReadiness();
  }

  async function closeRecommendation(): Promise<void> {
    recommendationPlan = null;
    recommendationPackage = null;
    await tick();
    const trigger = [...(projectsRoot?.querySelectorAll<HTMLButtonElement>("button[data-recommendation-id]") ?? [])]
      .find((button) => button.dataset.recommendationId === recommendationTriggerId)
      ?? recommendationTrigger;
    trigger?.focus({ preventScroll: true });
  }

  function recommendationApplied(): void {
    if (!recommendationPlan) return;
    const targets = recommendationPlan.targets.filter((target) => target.operation !== "informational");
    if (recommendationTargetIndex + 1 < targets.length) {
      recommendationTargetIndex += 1;
      return;
    }
    recommendationPlan = null;
    recommendationPackage = null;
    void refreshReadiness();
  }

  function readinessRepairLabel(
    category: ProjectReadinessReport["categories"][number]["category"],
    label: string,
  ): string {
    return ({
      agentRoster: `Review Agent readiness: ${label}`,
      skills: `Review Skill readiness: ${label}`,
      instructions: `Review project instructions: ${label}`,
      mcp: `Open MCP settings for ${label}`,
      tools: `Open Tools for ${label}`,
    })[category];
  }

  async function repairReadiness(
    category: ProjectReadinessReport["categories"][number]["category"],
    row: ProjectReadinessReport["categories"][number]["rows"][number],
  ): Promise<void> {
    if (!readiness?.baseline) return;
    if (category === "agentRoster") {
      const reference = readiness.baseline.agentRequirements.find(
        (item) => `${item.reference.sourceId}:${item.reference.relativePath}:${item.tool}` === row.id,
      )?.reference ?? readiness.baseline.agents.find(
        (item) => row.id.startsWith(`${item.sourceId}:${item.relativePath}:`),
      );
      if (reference) ui.openAgentReference(reference);
      return;
    }
    if (category === "skills") {
      const reference = readiness.baseline.skillRequirements.find(
        (item) => `${item.reference.sourceId}:${item.reference.relativePath}:${item.runtime}` === row.id,
      )?.reference ?? readiness.baseline.skills.find(
        (item) => row.id.startsWith(`${item.sourceId}:${item.relativePath}:`),
      );
      if (reference) ui.openSkill(reference);
      return;
    }
    if (category === "instructions") {
      readinessAnnouncement = `Project instruction configuration focused for ${row.label}.`;
      await tick();
      instructionManager?.scrollIntoView?.({ block: "nearest" });
      instructionManager?.focus({ preventScroll: true });
      return;
    }
    if (category === "mcp") {
      ui.openSettings("mcp");
      return;
    }
    ui.openTools(row.id as ProjectReadinessBaseline["tools"][number]);
  }

  // ── Project instructions: inspect → compose/remove → review → approve. ──
  type InstructionDraft = {
    target: ProjectInstructionTarget;
    operation: ProjectInstructionOperation;
    snippetId: string;
    content: string;
  };
  let instructionTargets = $state<ProjectInstructionTarget[]>([]);
  let instructionLoading = $state(false);
  let instructionBusy = $state(false);
  let instructionError = $state<string | null>(null);
  let instructionAnnouncement = $state("");
  let instructionDraft = $state<InstructionDraft | null>(null);
  let instructionPlan = $state<ProjectInstructionPlan | null>(null);
  let instructionResult = $state<ProjectInstructionApplyResult | null>(null);
  let instructionRestoreFocus: HTMLElement | null = null;
  let instructionLoadGeneration = 0;

  const instructionDiffRows = $derived<DiffRow[]>(
    instructionPlan ? diffLines(instructionPlan.current, instructionPlan.proposed) : [],
  );
  const instructionDiffSummary = $derived(diffStat(instructionDiffRows));
  const instructionDraftReady = $derived(
    !!instructionDraft?.snippetId
      && (instructionDraft.operation === "remove" || !!instructionDraft.content),
  );

  async function refreshProjectInstructions(): Promise<void> {
    if (!selected) return;
    const generation = ++instructionLoadGeneration;
    instructionLoading = true;
    instructionError = null;
    try {
      const inspected = await projects.inspectInstructions(selected.path);
      if (generation === instructionLoadGeneration) instructionTargets = inspected;
    } catch (error) {
      if (generation === instructionLoadGeneration) {
        instructionError = isAppError(error) ? appErrorMessage(error) : String(error);
      }
    } finally {
      if (generation === instructionLoadGeneration) instructionLoading = false;
    }
  }

  $effect(() => {
    const path = selected?.path;
    untrack(() => {
      if (!path) {
        instructionLoadGeneration += 1;
        instructionTargets = [];
        return;
      }
      void refreshProjectInstructions();
    });
  });

  function openInstructionEditor(
    event: MouseEvent,
    target: ProjectInstructionTarget,
    snippet?: ProjectInstructionSnippet,
  ): void {
    instructionRestoreFocus = event.currentTarget as HTMLElement;
    instructionDraft = {
      target,
      operation: "upsert",
      snippetId: snippet?.id ?? "",
      content: snippet?.content ?? "",
    };
    instructionPlan = null;
    instructionResult = null;
    instructionError = null;
  }

  async function removeInstruction(
    event: MouseEvent,
    target: ProjectInstructionTarget,
    snippet: ProjectInstructionSnippet,
  ): Promise<void> {
    openInstructionEditor(event, target, snippet);
    if (!instructionDraft || !selected) return;
    instructionDraft.operation = "remove";
    instructionDraft.content = "";
    await reviewInstruction();
  }

  async function reviewInstruction(): Promise<void> {
    if (!selected || !instructionDraft || instructionBusy) return;
    instructionBusy = true;
    instructionError = null;
    instructionAnnouncement = "Preparing exact instruction diff…";
    try {
      instructionPlan = await projects.planInstruction(
        selected.path,
        instructionDraft.target.id,
        instructionDraft.operation,
        instructionDraft.snippetId,
        instructionDraft.content,
      );
      instructionAnnouncement = instructionPlan.blockers.length > 0
        ? `Instruction change is blocked. ${instructionPlan.blockers.join(" ")}`
        : "Instruction diff is ready for review.";
    } catch (error) {
      instructionError = isAppError(error) ? appErrorMessage(error) : String(error);
      instructionAnnouncement = `Could not prepare instruction diff. ${instructionError}`;
    } finally {
      instructionBusy = false;
    }
  }

  async function applyInstruction(): Promise<void> {
    if (!instructionPlan || !instructionDraft || instructionBusy) return;
    instructionBusy = true;
    instructionError = null;
    instructionAnnouncement = "Applying reviewed instruction change…";
    try {
      const response = await projects.applyInstruction(instructionPlan, instructionDraft.content);
      instructionPlan = response.plan;
      instructionResult = response.result;
      if (!response.result) {
        instructionAnnouncement = "Instruction content changed. Review the refreshed diff before applying.";
      } else {
        instructionAnnouncement = response.result.outcome === "succeeded"
          ? "Instruction change applied."
          : `Instruction change ${response.result.outcome}.`;
        await refreshProjectInstructions();
      }
    } catch (error) {
      instructionError = isAppError(error) ? appErrorMessage(error) : String(error);
      instructionAnnouncement = `Instruction change failed. ${instructionError}`;
    } finally {
      instructionBusy = false;
    }
  }

  async function closeInstructionModal(): Promise<void> {
    instructionDraft = null;
    instructionPlan = null;
    instructionResult = null;
    instructionError = null;
    const restoreInstructionFocus = instructionRestoreFocus;
    instructionRestoreFocus = null;
    await tick();
    restoreInstructionFocus?.focus({ preventScroll: true });
  }

  async function reveal(path: string) {
    try {
      await install.revealPath(path);
    } catch (e) {
      toast.error(i18n.t("common.couldNotOpenFolder"), isAppError(e) ? appErrorMessage(e) : String(e));
    }
  }

  // ── Remove a project: a confirm dialog with two choices (#44). ──
  // Snapshot label/count at open time so they stay stable across the async op
  // (reconcile mutates projects.list mid-flight).
  let confirm = $state<{ path: string; label: string; agentCount: number; skillCount: number } | null>(null);
  let deleteBusy = $state(false);

  function finishRemove(path: string) {
    confirm = null;
    if (ui.projectsSelected === path) ui.selectProject(null);
  }

  // "Remove from app only" — forget the project; installed files stay on disk.
  async function forgetProject() {
    if (!confirm || deleteBusy) return;
    const { path, label } = confirm;
    deleteBusy = true;
    try {
      await install.forgetProject(path, label);
      await projects.unregister(path);
      finishRemove(path);
    } catch (e) {
      toast.error(i18n.t("common.actionFailed"), isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      deleteBusy = false;
    }
  }

  // "Remove & uninstall" — delete the agents THIS APP installed here (never the
  // user's own files; uninstall backs up modified files first), then forget.
  async function uninstallAndRemove() {
    if (!mutationTruthFresh || !confirm || deleteBusy) return;
    const { path } = confirm;
    deleteBusy = true;
    try {
      const targets = rosterFor(path).map((r) => ({ slug: r.slug, tool: r.tool, projectPath: path }));
      if (targets.length > 0) await install.bulk("uninstall", targets);
      for (const skill of skillsFor(path)) {
        const removed = await skillSources.lifecycle(
          "uninstall",
          skill,
          projects.list.map((project) => project.path),
        );
        if (!removed) throw new Error(`Could not uninstall skill ${skill.name}`);
      }
      await projects.unregister(path);
      finishRemove(path);
    } catch (e) {
      toast.error(i18n.t("common.actionFailed"), isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      deleteBusy = false;
    }
  }

  let adding = $state(false);
  async function addProject() {
    if (adding) return;
    adding = true;
    try {
      const p = await projects.addViaPicker();
      if (!p) return;
      await projects.refresh();
      ui.selectProject(p); // land in the new project's detail (Deploy… from there)
    } finally {
      adding = false;
    }
  }
</script>

<section class="pr" bind:this={projectsRoot} tabindex="-1">
  <div class="sr-only" role="status" aria-live="polite" aria-atomic="true">{reconcileAnnouncement}</div>
  {#if install.reconcileError}
    <aside class="install-truth-warning" aria-busy={install.reconciling}>
      <div class="reconcile-copy"><strong>{i18n.optional("reconcile.heading", "Installation status may be out of date")}</strong><p><span>{install.reconcileError}</span> {install.reconciled ? i18n.optional("reconcile.retained", "Your last known installation data is still shown.") : installTruthMessage}</p></div>
      <Button size="sm" loading={install.reconciling} onclick={(event) => void retryReconcile(event)}>{install.reconciling ? i18n.optional("reconcile.retrying", "Retrying…") : i18n.optional("reconcile.retry", "Retry status check")}</Button>
    </aside>
  {:else if !install.reconciled}
    <p class="install-truth-checking">{installTruthMessage}</p>
  {/if}
  {#if selected}
    <!-- ── Detail ── -->
    <header class="pr-head detail">
      <span class="dh-ic"><FolderIcon size={20} /></span>
      <div class="dh-id">
        <h2 class="dh-label">{selected.label}</h2>
        <button class="dh-path" title={selected.path} onclick={() => reveal(selected.path)}>{selected.path}</button>
      </div>
      <span class="dh-count">{install.reconciled ? i18n.count(rosterFor(selected.path).length, "common.agent.one", "common.agent.many") : installTruthMessage}</span>
      <button class="btn" onclick={() => reveal(selected.path)}><FolderOpen size={15} /><span>{i18n.t("common.reveal")}</span></button>
      <button class="btn primary" disabled={!installTruthFresh} onclick={() => (browseFor = selected.path)}>{i18n.t("teams.deploy")}</button>
      <button class="btn danger-ic" disabled={!mutationTruthFresh} title={i18n.t("projects.removeTitle")} aria-label={i18n.t("projects.removeAria")} onclick={() => (confirm = { path: selected.path, label: selected.label, agentCount: rosterFor(selected.path).length, skillCount: skillsFor(selected.path).length })}><Trash2 size={15} /></button>
    </header>

    <section class="readiness" aria-labelledby="project-readiness-heading" aria-busy={readinessBusy}>
      <div class="readiness-heading">
        <div>
          <h3 id="project-readiness-heading">Readiness</h3>
          <p>{readiness?.baseline?.label ?? "No baseline configured"}{#if readiness} · {readiness.overall}{/if}</p>
        </div>
        <Button size="sm" disabled={readinessBusy} onclick={() => void refreshReadiness()}>Retry</Button>
        <Button size="sm" disabled={readinessBusy} onclick={() => void importReadinessPack()}>Import Workspace Pack baseline</Button>
        {#if readiness?.baseline}
          <label class="readiness-opt-in"><input type="checkbox" checked={readiness.subscribed} disabled={readinessBusy} onchange={(event) => void setReadinessSubscription(event.currentTarget.checked)} /> Catalog recommendations</label>
        {/if}
      </div>
      <div class="sr-only" role="status" aria-live="polite" aria-atomic="true">{readinessAnnouncement}</div>
      {#if readinessError}<p class="readiness-error">{readinessError}</p>{/if}
      {#if readiness?.categories?.length}
        <div class="readiness-categories">
          {#each readiness.categories as category (category.category)}
            <section class="readiness-category">
              <h4>{readinessCategoryLabel(category.category)} <span>{category.state}</span></h4>
              {#if category.rows.length}
                <ul>{#each category.rows as row (row.id)}<li><strong>{row.label}</strong><span>{row.state} · {row.evidence}</span>{#if row.state !== "ready"}<Button variant="link" size="sm" ariaLabel={readinessRepairLabel(category.category, row.label)} onclick={() => void repairReadiness(category.category, row)}>Review</Button>{/if}</li>{/each}</ul>
              {:else}<p>Not required</p>{/if}
            </section>
          {/each}
        </div>
      {:else if !readinessBusy}
        <p class="readiness-empty">Import a reviewed Workspace Pack or save an explicitly selected Team as this project's baseline.</p>
      {/if}
      {#if readiness?.subscribed}
        <div class="recommendations">
          <h4>Catalog recommendations <span>{newRecommendationCount} new</span></h4>
          {#if recommendations.length}
            <ul>
              {#each recommendations as recommendation (recommendation.id)}
                <li>
                  <div><strong>{recommendation.lifecycle}</strong><span>{recommendation.summary}</span></div>
                  <button class="btn" data-recommendation-id={recommendation.id} disabled={readinessBusy || recommendation.lifecycle !== "new" || recommendation.changeKind === "removed"} onclick={(event) => void openRecommendation(event, recommendation)}>Open review</button>
                  {#if recommendation.lifecycle !== "dismissed"}<button class="btn" disabled={readinessBusy} onclick={() => void dismissRecommendation(recommendation)}>Dismiss</button>{/if}
                </li>
              {/each}
            </ul>
          {:else}<p>No catalog recommendations.</p>{/if}
        </div>
      {/if}
    </section>

    <section class="instruction-manager" bind:this={instructionManager} tabindex="-1" aria-labelledby="project-instructions-heading" aria-busy={instructionLoading}>
      <div class="instruction-heading">
        <div>
          <h3 id="project-instructions-heading">Project instructions</h3>
          <p>Compose reviewed snippets without replacing existing project rules.</p>
        </div>
        <button class="btn" disabled={instructionLoading} onclick={() => void refreshProjectInstructions()}>Refresh</button>
      </div>
      <div class="sr-only" role="status" aria-live="polite" aria-atomic="true">{instructionAnnouncement}</div>
      {#if instructionError}<p class="instruction-error">{instructionError}</p>{/if}
      {#if instructionLoading}
        <p class="instruction-muted">Inspecting known instruction files…</p>
      {:else}
        <ul class="instruction-targets">
          {#each instructionTargets as target (target.id)}
            <li class="instruction-target">
              <div class="instruction-target-copy">
                <strong>{target.label}</strong>
                <span title={target.destination}>{target.relativePath} · {target.state === "existingUnmanaged" ? "Existing · adoption required" : target.state}</span>
                {#each target.blockers as blocker}<span class="instruction-blocker">{blocker}</span>{/each}
              </div>
              <div class="instruction-snippets">
                {#each target.snippets as snippet (snippet.id)}
                  <button class="instruction-chip" onclick={(event) => openInstructionEditor(event, target, snippet)}>{snippet.id}</button>
                  <button class="instruction-remove" aria-label={`Remove ${snippet.id} from ${target.label}`} onclick={(event) => void removeInstruction(event, target, snippet)}>Remove</button>
                {/each}
              </div>
              <button class="btn" disabled={target.blockers.length > 0} onclick={(event) => openInstructionEditor(event, target)}>Add snippet</button>
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <div class="scroll">
      {#if !install.reconciled}
        <p class="install-truth-unavailable">{installTruthMessage}</p>
      {:else if detailGroups.length === 0}
        <div class="d-empty">
          <LayersIcon size={40} />
          <p>{i18n.t("projects.emptyHere")}</p>
          <Button variant="primary" disabled={!installTruthFresh} onclick={() => (browseFor = selected.path)}>{i18n.t("projects.deployDivisionTeam")}</Button>
        </div>
      {:else}
        <div class="groups">
          {#each detailGroups as g (g.slug)}
            {@const Icon = resolveCategoryIcon(g.icon)}
            {@const isOpen = !collapsed.has(g.slug)}
            <section class="grp">
              <button class="grp-head" onclick={() => toggleGroup(g.slug)} aria-expanded={isOpen}>
                <ChevronDown size={15} class={isOpen ? "pr-chev open" : "pr-chev"} />
                <span class="grp-ic" style="color:{g.color}"><Icon size={15} /></span>
                <span class="grp-label">{g.label}</span>
                <span class="grp-count">{g.rows.length}</span>
              </button>
              {#if isOpen}
                <ul class="roster">
                  {#each g.rows as r (r.slug + r.tool)}
                    <li class="r-row">
                      <span class="r-name">{r.name}</span>
                      <Pill tone="neutral">{install.toolLabel(r.tool)}</Pill>
                    </li>
                  {/each}
                </ul>
              {/if}
            </section>
          {/each}
        </div>
      {/if}
    </div>
  {:else}
    <!-- ── List ── -->
    <header class="pr-head">
      <p class="pr-count">{i18n.t("projects.count", { count: projects.list.length })}</p>
      <div class="pr-actions">
        <button class="btn primary" disabled={adding} onclick={addProject}>
          <FolderPlus size={15} /><span>{i18n.t("projects.add")}</span>
        </button>
      </div>
    </header>

    {#if projects.list.length === 0}
      <div class="scroll">
        <EmptyState title={i18n.t("projects.emptyTitle")}>
          {#snippet icon()}<FolderIcon size={48} />{/snippet}
          {i18n.t("projects.emptyBody")}
          {#snippet cta()}
            <div class="empty-cta">
              <Button variant="primary" disabled={adding} onclick={addProject}>
                {#snippet icon()}<FolderPlus size={15} />{/snippet}
                {i18n.t("projects.add")}
              </Button>
              <button class="link-btn" onclick={() => ui.openPlaybook()}>{i18n.t("projects.openPlaybook")}</button>
            </div>
          {/snippet}
        </EmptyState>
      </div>
    {:else}
      <ul class="rows">
        {#each projects.list as project (project.path)}
          <li class="proj">
            <button class="proj-row" onclick={() => ui.selectProject(project.path)}>
              <span class="proj-ic"><FolderIcon size={18} /></span>
              <span class="proj-body">
                <span class="proj-label">{project.label}</span>
                <span class="proj-path" title={project.path}>{project.path}</span>
              </span>
              <span class="proj-count">{install.reconciled ? i18n.count(rosterFor(project.path).length, "common.agent.one", "common.agent.many") : "—"}</span>
              <ChevronRight size={16} class="proj-go" />
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
</section>

{#if browseFor !== null}
  <DeployBrowser projectPath={browseFor} onClose={() => (browseFor = null)} />
{/if}

{#if recommendationPlan && recommendationPackage}
  {@const recommendationTarget = recommendationPlan.targets.filter((target) => target.operation !== "informational")[recommendationTargetIndex]}
  <InstallModal
    title="Review catalog recommendation"
    agentPackage={recommendationPackage}
    reviewIntent={{
      operation: recommendationTarget.operation as "install" | "update",
      reference: recommendationTarget.reference,
      tool: recommendationTarget.tool,
      projectPath: recommendationTarget.projectPath,
    }}
    allowedTools={[recommendationTarget.tool]}
    onClose={closeRecommendation}
    onApplied={recommendationApplied}
  />
{/if}

{#if confirm}
  <Modal open title={i18n.t("projects.deleteTitle", { project: confirm.label })} defaultFocus="cancel" onClose={() => (confirm = null)}>
    <p class="del-body">
      {#if (confirm?.agentCount ?? 0) + (confirm?.skillCount ?? 0) > 0}
        {i18n.t("projects.deleteBodyWithSkills", { project: confirm.label, agents: confirm.agentCount, skills: confirm.skillCount })}
      {:else}
        {i18n.t("projects.deleteEmptyBody", { project: confirm.label })}
      {/if}
    </p>
    {#if (confirm?.agentCount ?? 0) + (confirm?.skillCount ?? 0) > 0}
      <p class="del-note">{i18n.t("projects.deleteExplain")}</p>
    {/if}
    {#snippet actions()}
      <Button variant="secondary" modalAction="cancel" onclick={() => (confirm = null)}>{i18n.t("common.cancel")}</Button>
      {#if (confirm?.agentCount ?? 0) + (confirm?.skillCount ?? 0) > 0}
        <Button variant="danger" disabled={!mutationTruthFresh || deleteBusy} onclick={uninstallAndRemove}>
          {i18n.t("projects.deleteUninstall", { count: (confirm?.agentCount ?? 0) + (confirm?.skillCount ?? 0) })}
        </Button>
      {/if}
      <Button variant="primary" modalAction="confirm" disabled={deleteBusy} onclick={forgetProject}>{i18n.t("projects.deleteKeep")}</Button>
    {/snippet}
  </Modal>
{/if}

{#if instructionDraft}
  <Modal open title={`${instructionDraft.operation === "remove" ? "Remove" : "Manage"} ${instructionDraft.target.label} snippet`} defaultFocus="cancel" onClose={closeInstructionModal}>
    {#if instructionResult}
      <div class="instruction-result" data-outcome={instructionResult.outcome}>
        <strong>{instructionResult.outcome === "succeeded" ? "Instruction change applied" : "Instruction change did not complete"}</strong>
        <code title={instructionResult.destination}>{instructionResult.destination}</code>
        {#if instructionResult.backupPath}<p>Backup: <code title={instructionResult.backupPath}>{instructionResult.backupPath}</code></p>{/if}
        {#if instructionResult.message}<p class="instruction-error">{instructionResult.message}</p>{/if}
      </div>
    {:else if instructionPlan}
      <div class="instruction-review">
        <p class="instruction-destination" title={instructionPlan.destination}>{instructionPlan.destination}</p>
        <p class="instruction-stat"><span>+{instructionDiffSummary.added}</span> <span>−{instructionDiffSummary.removed}</span></p>
        {#if instructionPlan.adoption}<p class="instruction-warning">Existing user content remains unowned and byte-preserved.</p>{/if}
        {#if instructionPlan.backupRequired}<p class="instruction-warning">The exact current file will be backed up before apply.</p>{/if}
        {#each instructionPlan.warnings as warning}<p class="instruction-warning">{warning}</p>{/each}
        {#each instructionPlan.blockers as blocker}<p class="instruction-blocker">{blocker}</p>{/each}
        <pre class="instruction-diff" aria-label="Complete instruction file diff">{#each instructionDiffRows as row (`${row.oldNo ?? "x"}:${row.newNo ?? "x"}:${row.tag}`)}<span class:instruction-added={row.tag === "+"} class:instruction-removed={row.tag === "-"}><span class="instruction-line-number">{row.oldNo ?? ""}</span><span class="instruction-line-number">{row.newNo ?? ""}</span><span>{row.tag} {row.text}</span></span>{/each}</pre>
      </div>
    {:else if instructionBusy}
      <p class="instruction-muted">Preparing exact instruction diff…</p>
    {:else}
      <form class="instruction-form" onsubmit={(event) => { event.preventDefault(); void reviewInstruction(); }}>
        <label for="instruction-snippet-id">Snippet id</label>
        <input id="instruction-snippet-id" bind:value={instructionDraft.snippetId} disabled={instructionDraft.operation === "remove"} maxlength="64" pattern="[a-z0-9](?:[a-z0-9-]*[a-z0-9])?" required />
        {#if instructionDraft.operation === "upsert"}
          <label for="instruction-snippet-content">Instruction text</label>
          <textarea id="instruction-snippet-content" bind:value={instructionDraft.content} maxlength="65536" rows="12" required></textarea>
        {:else}
          <p>Only the app-owned <strong>{instructionDraft.snippetId}</strong> block will be removed.</p>
        {/if}
      </form>
    {/if}
    {#if instructionError}<p class="instruction-error">{instructionError}</p>{/if}
    {#snippet actions()}
      {#if instructionResult}
        <Button variant="primary" modalAction="cancel" onclick={closeInstructionModal}>Close</Button>
      {:else if instructionPlan}
        <Button variant="secondary" modalAction="cancel" disabled={instructionBusy} onclick={() => (instructionPlan = null)}>Back</Button>
        <Button variant="primary" modalAction="confirm" loading={instructionBusy} disabled={instructionBusy || instructionPlan.blockers.length > 0 || instructionPlan.noOp} onclick={applyInstruction}>Apply reviewed change</Button>
      {:else}
        <Button variant="secondary" modalAction="cancel" disabled={instructionBusy} onclick={closeInstructionModal}>Cancel</Button>
        <Button variant="primary" modalAction="confirm" loading={instructionBusy} disabled={instructionBusy || !instructionDraftReady} onclick={reviewInstruction}>Review exact diff</Button>
      {/if}
    {/snippet}
  </Modal>
{/if}

<style>
  .pr { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .install-truth-warning { flex: none; display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2); padding: var(--space-2) var(--space-4); border-bottom: 1px solid var(--color-warning); background: var(--color-warning-subtle); color: var(--color-warning-strong); font-size: var(--text-body-sm); }
  .install-truth-warning span { flex: 1 1 auto; min-width: 0; overflow-wrap: anywhere; text-wrap: pretty; }
  .install-truth-warning button { flex: none; padding: var(--space-2) var(--space-3); border: 1px solid var(--color-border); border-radius: var(--radius-md); background: var(--color-surface-raised); }
  .install-truth-unavailable { height: 100%; display: grid; place-items: center; padding: var(--space-6); color: var(--color-warning-strong); font-size: var(--text-body-sm); text-wrap: pretty; }
  .pr-head {
    flex: none; display: flex; align-items: center; justify-content: space-between; gap: var(--space-3);
    padding: var(--space-3) var(--space-4); border-bottom: 1px solid var(--color-border);
  }
  .pr-count { color: var(--color-text-secondary); font-size: var(--text-body-sm); }
  .pr-actions { display: flex; gap: var(--space-2); }
  .readiness, .instruction-manager { flex: none; max-height: 280px; overflow-y: auto; padding: var(--space-3) var(--space-4); border-bottom: 1px solid var(--color-border); }
  .readiness-heading, .instruction-heading { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2); }
  .readiness-heading > div, .instruction-heading > div { flex: 1; min-width: 12rem; }
  .readiness-heading h3, .instruction-heading h3, .readiness-category h4, .recommendations h4 { margin: 0; }
  .readiness-heading p, .readiness-category p, .recommendations p { margin: 2px 0 0; color: var(--color-text-muted); font-size: var(--text-caption); }
  .readiness-opt-in { display: inline-flex; align-items: center; gap: var(--space-2); font-size: var(--text-body-sm); }
  .readiness-categories { display: grid; grid-template-columns: repeat(auto-fit, minmax(10rem, 1fr)); gap: var(--space-2); margin-top: var(--space-3); }
  .readiness-category { padding: var(--space-2); border: 1px solid var(--color-border); border-radius: var(--radius-md); }
  .readiness-category h4, .recommendations h4 { display: flex; justify-content: space-between; gap: var(--space-2); font-size: var(--text-body-sm); }
  .readiness-category h4 span, .recommendations h4 span { color: var(--color-text-muted); font-weight: var(--fw-regular); }
  .readiness-category ul, .recommendations ul { list-style: none; margin: var(--space-2) 0 0; padding: 0; display: grid; gap: var(--space-2); }
  .readiness-category li { display: grid; gap: 2px; min-width: 0; }
  .readiness-category li strong, .readiness-category li span { overflow-wrap: anywhere; font-size: var(--text-caption); }
  .readiness-category li span { color: var(--color-text-muted); }
  .recommendations { margin-top: var(--space-3); }
  .recommendations li { display: flex; align-items: center; gap: var(--space-2); }
  .recommendations li > div { flex: 1; display: grid; min-width: 0; font-size: var(--text-caption); }
  .readiness-error { color: var(--color-danger); overflow-wrap: anywhere; }

  /* ── Detail header ── */
  .pr-head.detail { justify-content: flex-start; }
  .dh-ic {
    flex: none; display: inline-flex; align-items: center; justify-content: center;
    width: 40px; height: 40px; border-radius: var(--radius-md);
    background: var(--color-surface-sunken); color: var(--color-text-secondary);
  }
  .dh-id { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .dh-label { font-size: var(--text-h2); font-weight: var(--fw-semibold); color: var(--color-text-primary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .dh-path {
    font-size: var(--text-caption); color: var(--color-text-muted); background: transparent;
    text-align: left; cursor: pointer; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 100%;
  }
  .dh-path:hover { color: var(--color-brand); text-decoration: underline; }
  .dh-count { flex: none; font-size: var(--text-body-sm); color: var(--color-text-muted); font-variant-numeric: tabular-nums; white-space: nowrap; }

  .btn {
    display: inline-flex; align-items: center; gap: 6px;
    height: 32px; padding: 0 var(--space-3);
    border: 1px solid var(--color-border); border-radius: var(--radius-md);
    background: transparent; color: var(--color-text-secondary);
    font-size: var(--text-body-sm); cursor: pointer; flex: none;
  }
  .btn:hover:not(:disabled) { color: var(--color-text-primary); background: var(--color-surface-sunken); }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .btn.primary { background: var(--color-brand); color: var(--color-text-inverse); border-color: transparent; }
  .btn.primary:hover:not(:disabled) { filter: brightness(1.08); background: var(--color-brand); }
  .btn.danger-ic { padding: 0; width: 32px; justify-content: center; }
  .btn.danger-ic:hover { color: var(--color-danger); border-color: var(--color-danger); background: color-mix(in srgb, var(--color-danger) 10%, transparent); }

  .scroll { flex: 1; min-height: 0; overflow-y: auto; }

  .empty-cta { display: flex; flex-direction: column; align-items: center; gap: var(--space-2); }
  .link-btn { background: transparent; color: var(--color-text-link, var(--color-brand)); font-size: var(--text-body-sm); cursor: pointer; padding: 2px; }
  .link-btn:hover { text-decoration: underline; }

  /* ── Project list rows ── */
  .rows { flex: 1; min-height: 0; overflow-y: auto; list-style: none; margin: 0; padding: var(--space-3); display: flex; flex-direction: column; gap: var(--space-2); }
  .proj { border: 1px solid var(--color-border); border-radius: var(--radius-lg); background: var(--color-surface-raised); overflow: hidden; }
  .proj:hover { border-color: var(--color-border-strong, var(--color-text-muted)); }
  .proj-row {
    width: 100%; display: flex; align-items: center; gap: var(--space-3);
    padding: var(--space-3); background: transparent; cursor: pointer; text-align: left;
  }
  .proj-row:hover { background: var(--color-surface-sunken); }
  .proj-ic { flex: none; display: inline-flex; align-items: center; justify-content: center; width: 36px; height: 36px; border-radius: var(--radius-md); background: var(--color-surface-sunken); color: var(--color-text-secondary); }
  .proj-body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .proj-label { font-weight: var(--fw-semibold); color: var(--color-text-primary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .proj-path { font-size: var(--text-caption); color: var(--color-text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .proj-count { flex: none; font-size: var(--text-body-sm); color: var(--color-text-muted); font-variant-numeric: tabular-nums; white-space: nowrap; }
  :global(.proj-go) { flex: none; color: var(--color-text-muted); }

  /* ── Division groups (detail roster) ── */
  .groups { padding: var(--space-3); display: flex; flex-direction: column; gap: 2px; }
  .grp { display: flex; flex-direction: column; }
  .grp-head {
    display: flex; align-items: center; gap: var(--space-2);
    width: 100%; padding: var(--space-2); border-radius: var(--radius-sm);
    background: transparent; cursor: pointer; text-align: left;
  }
  .grp-head:hover { background: var(--color-surface-sunken); }
  :global(.pr-chev) { color: var(--color-text-muted); transition: transform var(--motion-duration-fast, 120ms) ease; transform: rotate(-90deg); flex: none; }
  :global(.pr-chev.open) { transform: rotate(0deg); }
  .grp-ic { flex: none; display: inline-flex; }
  .grp-label { flex: 1; min-width: 0; font-weight: var(--fw-semibold); color: var(--color-text-primary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .grp-count { flex: none; min-width: 20px; text-align: center; font-size: var(--text-caption); color: var(--color-text-muted); font-variant-numeric: tabular-nums; background: var(--color-surface-sunken); border-radius: var(--radius-full); padding: 1px 7px; }

  .roster { list-style: none; margin: 0; padding: 2px 0 var(--space-2) var(--space-4); display: flex; flex-direction: column; gap: 1px; }
  .r-row { display: flex; align-items: center; gap: var(--space-3); padding: var(--space-2) var(--space-3); border-radius: var(--radius-md); }
  .r-row:hover { background: var(--color-surface-sunken); }
  .r-name { flex: 1; min-width: 0; font-weight: var(--fw-medium); color: var(--color-text-primary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .d-empty { height: 100%; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: var(--space-3); color: var(--color-text-muted); padding: var(--space-6); }
  .d-empty p { font-size: var(--text-body-sm); }

  /* ── Bounded project instruction manager ── */
  .instruction-manager { flex: none; max-height: 240px; overflow-y: auto; padding: var(--space-3) var(--space-4); border-bottom: 1px solid var(--color-border); background: var(--color-surface-sunken); }
  .instruction-heading { display: flex; align-items: start; justify-content: space-between; gap: var(--space-3); }
  .instruction-heading h3 { color: var(--color-text-primary); font-size: var(--text-body); font-weight: var(--fw-semibold); }
  .instruction-heading p, .instruction-muted { color: var(--color-text-muted); font-size: var(--text-caption); }
  .instruction-targets { display: grid; gap: var(--space-2); margin: var(--space-3) 0 0; padding: 0; list-style: none; }
  .instruction-target { display: flex; align-items: center; gap: var(--space-2); min-width: 0; padding: var(--space-2); border: 1px solid var(--color-border); border-radius: var(--radius-md); background: var(--color-surface-raised); }
  .instruction-target-copy { display: flex; flex: 1; min-width: 0; flex-direction: column; }
  .instruction-target-copy span { overflow: hidden; color: var(--color-text-muted); font-size: var(--text-caption); text-overflow: ellipsis; white-space: nowrap; }
  .instruction-snippets { display: flex; flex-wrap: wrap; align-items: center; gap: 4px; }
  .instruction-chip, .instruction-remove { border: 1px solid var(--color-border); border-radius: var(--radius-full); background: transparent; padding: 3px 7px; color: var(--color-text-secondary); font-size: var(--text-caption); cursor: pointer; }
  .instruction-remove { color: var(--color-danger); }
  .instruction-form { display: grid; gap: var(--space-2); }
  .instruction-form label { color: var(--color-text-secondary); font-size: var(--text-body-sm); font-weight: var(--fw-semibold); }
  .instruction-form input, .instruction-form textarea { width: 100%; border: 1px solid var(--color-border); border-radius: var(--radius-md); background: var(--color-surface-sunken); padding: var(--space-2); color: var(--color-text-primary); font: inherit; }
  .instruction-form textarea { resize: vertical; font-family: var(--font-mono); font-size: var(--text-mono); }
  .instruction-review { display: grid; gap: var(--space-2); min-width: 0; }
  .instruction-destination, .instruction-result code { overflow: hidden; color: var(--color-text-muted); font-family: var(--font-mono); font-size: var(--text-mono); text-overflow: ellipsis; white-space: nowrap; }
  .instruction-stat { display: flex; gap: var(--space-2); color: var(--color-text-secondary); font-family: var(--font-mono); font-size: var(--text-caption); }
  .instruction-warning { color: var(--color-warning-strong); font-size: var(--text-body-sm); }
  .instruction-blocker, .instruction-error { color: var(--color-danger); font-size: var(--text-body-sm); overflow-wrap: anywhere; }
  .instruction-diff { max-height: 45vh; overflow: auto; margin: 0; border: 1px solid var(--color-border); border-radius: var(--radius-md); background: var(--color-surface-sunken); padding: var(--space-2); font-family: var(--font-mono); font-size: var(--text-mono); white-space: pre-wrap; }
  .instruction-diff > span { display: block; }
  .instruction-added { color: var(--color-success); background: color-mix(in srgb, var(--color-success) 12%, transparent); }
  .instruction-removed { color: var(--color-danger); background: color-mix(in srgb, var(--color-danger) 12%, transparent); }
  .instruction-line-number { display: inline-block; width: 3em; padding-right: 6px; color: var(--color-text-muted); text-align: right; user-select: none; }
  .instruction-result { display: grid; gap: var(--space-2); }

  /* ── Remove-project confirm dialog ── */
  .del-body { color: var(--color-text-primary); font-size: var(--text-body); }
  .del-note { margin-top: var(--space-3); color: var(--color-text-muted); font-size: var(--text-body-sm); line-height: 1.5; }

</style>
