<script lang="ts">
  /**
   * Teams — your roster of agents, the way you actually think about them.
   *
   * Two tabs:
   *  • "Your team" — what you have installed right now, grouped by division
   *    (collapsible). Save it as a named team, export it as an Agentfile, or
   *    restore one.
   *  • "Team presets" — app-bundled curated squads (PRESET_TEAMS) plus your own
   *    saved teams. Deploy any of them as a unit via the tri-state DeployModal.
   *
   * "Your team" reads the live install ledger (reconciled in +layout). Saved
   * teams live in the teams store (localStorage). Presets are bundled data.
   */
  import { onMount, tick } from "svelte";
  import EmptyState from "./EmptyState.svelte";
  import Pill from "./Pill.svelte";
  import Modal from "./Modal.svelte";
  import Button from "./Button.svelte";
  import Input from "./Input.svelte";
  import InstallModal from "./InstallModal.svelte";
  import UploadIcon from "@lucide/svelte/icons/upload";
  import DownloadIcon from "@lucide/svelte/icons/download";
  import ArchiveIcon from "@lucide/svelte/icons/archive";
  import ChevronDown from "@lucide/svelte/icons/chevron-down";
  import ChevronRight from "@lucide/svelte/icons/chevron-right";
  import SaveIcon from "@lucide/svelte/icons/bookmark-plus";
  import Trash2 from "@lucide/svelte/icons/trash-2";
  import UsersIcon from "@lucide/svelte/icons/users";
  import RocketIcon from "@lucide/svelte/icons/rocket";

  import StarterPrompt from "./StarterPrompt.svelte";
  import { install } from "$lib/stores/install.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { skillSources } from "$lib/stores/skillSources.svelte";
  import { corpus } from "$lib/stores/corpus.svelte";
  import { teams, type SavedTeam } from "$lib/stores/teams.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { ui } from "$lib/stores/ui.svelte";
  import { resolveCategoryIcon } from "$lib/util/categoryIcon";
  import { i18n } from "$lib/stores/i18n.svelte";
  import type { MessageKey } from "$lib/i18n/messages";
  import { PRESET_TEAMS } from "$lib/data/presetTeams";
  import { TEAM_EXAMPLES } from "$lib/data/playbook";
  import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
  import { appErrorMessage, isAppError, type InstalledAgent, type Agent, type WorkspacePackApplyResult, type WorkspacePackPlan, type WorkspacePackScope } from "$lib/types";
  import type { Component } from "svelte";

  const installTruthFresh = $derived(install.reconciled && !install.reconcileError);
  const packTruthFresh = $derived(installTruthFresh && !install.reconciling && skillSources.reconciled && !skillSources.reconciling && !skillSources.reconcileError);
  const installTruthMessage = $derived(install.reconcileError ? i18n.optional("reconcile.unavailable", "Installation status is unavailable until a retry succeeds.") : i18n.optional("reconcile.checking", "Checking installation status…"));
  let currentTab: HTMLButtonElement | undefined = $state();
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
    if (!install.reconcileError && restoreFocus) { await tick(); currentTab?.focus({ preventScroll: true }); }
  }

  onMount(() => {
    corpus.ensureLoaded();
    teams.hydrate();
    void projects.refresh();
  });

  let tab = $state<"current" | "presets">("current");
  let busy = $state(false);
  let baselineProject = $state("");
  let baselineBusy = $state(false);
  let baselineAnnouncement = $state("");
  let baselineGeneration = 0;
  const baselineResolution = $derived.by(() => {
    const requirements: Array<{ reference: { sourceId: string; relativePath: string }; tool: string }> = [];
    const unresolved: string[] = [];
    if (!openedTeam || !baselineProject) return { requirements, unresolved };
    for (const slug of new Set(openedTeam.agents)) {
      const candidates = install.installed
        .filter((row) => row.slug === slug
          && row.projectPath === baselineProject
          && row.tracked
          && row.sourceId
          && row.relativePath
          && !["missing", "foreign", "sourceUnavailable"].includes(row.state))
        .map((row) => ({
          reference: { sourceId: row.sourceId, relativePath: row.relativePath },
          tool: row.tool,
        }));
      const exact = [...new Map(candidates.map((item) => [
        `${item.reference.sourceId}:${item.reference.relativePath}:${item.tool}`,
        item,
      ])).values()];
      if (exact.length === 1) requirements.push(exact[0]);
      else unresolved.push(corpus.agents.find((agent) => agent.slug === slug)?.name ?? slug);
    }
    return { requirements, unresolved };
  });
  const baselineRequirements = $derived(baselineResolution.requirements);
  const baselineBlockingMessage = $derived(
    baselineProject && baselineResolution.unresolved.length > 0
      ? `${baselineResolution.unresolved.length} Team ${baselineResolution.unresolved.length === 1 ? "member" : "members"} cannot be saved: ${baselineResolution.unresolved.join(", ")}. Deploy each member to exactly one target in this project before saving.`
      : "",
  );
  const baselineReady = $derived(Boolean(baselineProject) && !baselineBlockingMessage && baselineRequirements.length > 0);

  async function applyTeamBaseline(subscribe: boolean): Promise<void> {
    if (!openedTeam || !baselineProject || baselineBusy) return;
    if (!baselineReady) {
      baselineAnnouncement = baselineBlockingMessage || "Every Team member needs one reviewed deployment target before saving.";
      return;
    }
    const projectPath = baselineProject;
    const teamKey = openedTeam.key;
    const teamLabel = openedTeam.label;
    const requirements = baselineRequirements.map((item) => ({
      reference: { ...item.reference }, tool: item.tool,
    }));
    const generation = ++baselineGeneration;
    baselineBusy = true;
    baselineAnnouncement = "Saving reviewed Team deployment targets…";
    try {
      await projects.saveTeamBaseline(projectPath, teamLabel, requirements, subscribe);
      if (generation !== baselineGeneration || baselineProject !== projectPath || openedTeam?.key !== teamKey) return;
      baselineAnnouncement = subscribe
        ? "Team baseline saved and catalog recommendations enabled."
        : "Team baseline saved.";
      toast.success("Project readiness updated", baselineAnnouncement);
    } catch (error) {
      baselineAnnouncement = isAppError(error) ? appErrorMessage(error) : String(error);
      toast.error("Could not save Team baseline", baselineAnnouncement);
    } finally {
      baselineBusy = false;
    }
  }

  // ── "Your team" = what WE installed (foreign isn't part of it until tracked). ──
  const managed = $derived(install.installed.filter((i) => i.state !== "foreign"));
  const managedSlugs = $derived([...new Set(managed.map((m) => m.slug))]);
  const portableManagedCount = $derived(managed.length + skillSources.installed.filter((item) => item.tracked).length);

  // Group the loadout by division (collapsible). Agents missing from the current
  // corpus (e.g. removed upstream) fall into "Other".
  const OTHER = "__other";
  const groups = $derived.by(() => {
    const divOf = new Map(corpus.agents.map((a) => [a.slug, a.category]));
    const m = new Map<string, InstalledAgent[]>();
    for (const r of managed) {
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

  let collapsed = $state<Set<string>>(new Set());
  function toggle(slug: string) {
    const next = new Set(collapsed);
    if (next.has(slug)) next.delete(slug);
    else next.add(slug);
    collapsed = next;
  }
  // Division groups are CLOSED by default — seed the collapse set with every
  // division once the roster first loads.
  let collapseSeeded = false;
  $effect(() => {
    if (collapseSeeded || groups.length === 0) return;
    collapseSeeded = true;
    collapsed = new Set(groups.map((g) => g.slug));
  });
  const allCollapsed = $derived(groups.length > 0 && groups.every((g) => collapsed.has(g.slug)));
  function toggleAll() {
    collapsed = allCollapsed ? new Set() : new Set(groups.map((g) => g.slug));
  }

  // ── Deploy modal (presets + saved teams reuse the division tri-state) ──
  let deployTarget = $state<{ title: string; agents: string[] } | null>(null);
  function deploy(title: string, agents: string[]) {
    deployTarget = { title, agents };
  }

  function presetLabel(slug: string): string {
    return i18n.t(`preset.${slug}.label` as MessageKey);
  }

  function presetDescription(slug: string): string {
    return i18n.t(`preset.${slug}.description` as MessageKey);
  }

  function teamExamples(slug: string): string[] {
    const fallback = TEAM_EXAMPLES[slug] ?? [];
    return fallback.map((example, i) => i18n.optional(`teamExample.${slug}.${i + 1}`, example));
  }

  // ── Team detail (master/detail over the presets tab) ──
  type TeamView = {
    key: string;
    label: string;
    description: string;
    color: string | null;
    icon: Component;
    saved: boolean;
    agents: string[];
    examples: string[];
  };
  // The opened team is a nav location (ui.teamsSelected) so the system-wide
  // title-bar back arrow returns to the list — no in-view back button.
  function openPreset(p: (typeof PRESET_TEAMS)[number]) { tab = "presets"; ui.selectTeam(`preset:${p.slug}`); }
  function openSaved(t: SavedTeam) { tab = "presets"; ui.selectTeam(`saved:${t.id}`); }
  const openedTeam = $derived.by<TeamView | null>(() => {
    const key = ui.teamsSelected;
    if (!key) return null;
    if (key.startsWith("preset:")) {
      const slug = key.slice("preset:".length);
      const p = PRESET_TEAMS.find((x) => x.slug === slug);
      return p ? { key, label: presetLabel(p.slug), description: presetDescription(p.slug), color: p.color, icon: p.icon, saved: false, agents: p.agents, examples: teamExamples(p.slug) } : null;
    }
    if (key.startsWith("saved:")) {
      const id = key.slice("saved:".length);
      const t = teams.saved.find((x) => x.id === id);
      return t ? { key, label: t.name, description: i18n.t("teams.savedTeamDesc", { count: t.agents.length }), color: null, icon: UsersIcon as unknown as Component, saved: true, agents: t.agents, examples: [] } : null;
    }
    return null;
  });

  // The opened team's agents, grouped by division (disclosures, closed by default).
  const TOTHER = "__other";
  const detailGroups = $derived.by(() => {
    if (!openedTeam) return [];
    const present = corpus.agents.filter((a) => openedTeam!.agents.includes(a.slug));
    const m = new Map<string, Agent[]>();
    for (const a of present) {
      const d = a.category || TOTHER;
      const arr = m.get(d);
      if (arr) arr.push(a);
      else m.set(d, [a]);
    }
    const out = [...m.entries()].map(([slug, rows]) => ({
      slug,
      label: slug === TOTHER ? i18n.t("common.other") : corpus.labelOf(slug),
      color: slug === TOTHER ? "#94A3B8" : corpus.colorOf(slug),
      icon: slug === TOTHER ? "HelpCircle" : corpus.iconOf(slug),
      rows: rows.slice().sort((a, b) => a.name.localeCompare(b.name)),
    }));
    out.sort((a, b) => (a.slug === TOTHER ? 1 : b.slug === TOTHER ? -1 : a.label.localeCompare(b.label)));
    return out;
  });
  const detailMissing = $derived(openedTeam ? openedTeam.agents.length - detailGroups.reduce((n, g) => n + g.rows.length, 0) : 0);
  // Curated examples when present; otherwise a generic loop seeded with the name.
  const detailExamples = $derived(
    openedTeam
      ? openedTeam.examples.length > 0
        ? openedTeam.examples
        : [i18n.t("teams.genericExample", { team: openedTeam.label })]
      : [],
  );

  let teamCollapsed = $state<Set<string>>(new Set());
  function toggleTeamGroup(slug: string) {
    const next = new Set(teamCollapsed);
    if (next.has(slug)) next.delete(slug);
    else next.add(slug);
    teamCollapsed = next;
  }
  let tcInitFor = "";
  $effect(() => {
    const k = openedTeam?.key ?? "";
    // Guard on an empty group set too: if the corpus loads after the team opens
    // (e.g. a cold back/forward restore), don't seed from an empty list — wait
    // for detailGroups so divisions stay closed by default.
    if (k === tcInitFor || detailGroups.length === 0) return;
    tcInitFor = k;
    teamCollapsed = new Set(detailGroups.map((g) => g.slug));
  });

  // How many of a team's agents exist in the corpus / are currently installed.
  const corpusSlugs = $derived(new Set(corpus.agents.map((a) => a.slug)));
  const installedSlugs = $derived(new Set(install.installed.filter((i) => i.state !== "missing").map((i) => i.slug)));
  function teamStats(agents: string[]) {
    const present = agents.filter((s) => corpusSlugs.has(s));
    const deployed = present.filter((s) => installedSlugs.has(s)).length;
    return { count: present.length, deployed };
  }

  // ── Save current installs as a named team ──
  let saveOpen = $state(false);
  let saveName = $state("");
  function openSave() {
    saveName = "";
    saveOpen = true;
  }
  function confirmSave() {
    if (managedSlugs.length === 0) return;
    const t = teams.save(saveName, managedSlugs);
    saveOpen = false;
    tab = "presets";
    toast.success(i18n.t("teams.savedToast", { name: t.name }), i18n.count(t.agents.length, "common.agent.one", "common.agent.many"));
  }

  function deleteSaved(t: SavedTeam) {
    teams.remove(t.id);
    toast.success(i18n.t("teams.deletedToast", { name: t.name }));
  }

  // ── Portable Workspace Pack export / reviewed apply ──
  let exportOpen = $state(false);
  let exportName = $state("My workspace");
  let exportScope = $state<WorkspacePackScope>("user");
  let exportProject = $state("");
  function openExport() {
    exportName = "My workspace";
    exportScope = "user";
    exportProject = projects.list[0]?.path ?? "";
    exportOpen = true;
  }
  async function exportLoadout() {
    if (!packTruthFresh || !exportName.trim() || (exportScope === "project" && !exportProject)) return;
    const path = await saveDialog({ title: i18n.optional("teams.saveWorkspacePackTitle", "Save Workspace Pack"), defaultPath: "WorkspacePack.json", filters: [{ name: "Workspace Pack", extensions: ["json"] }] });
    if (!path) return;
    busy = true;
    try {
      const n = await install.exportLoadout(path, exportName.trim(), exportScope, exportScope === "project" ? exportProject : null);
      exportOpen = false;
      toast.success(i18n.t("teams.exportedToast", { count: n }), path);
    } catch (e) {
      toast.error(i18n.t("teams.exportFailed"), isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      busy = false;
    }
  }

  let packTrigger: HTMLButtonElement | undefined = $state();
  let packPath = $state("");
  let packProject = $state("");
  let packPlan: WorkspacePackPlan | null = $state(null);
  let packResult: WorkspacePackApplyResult | null = $state(null);
  let packReceiptId: string | null = $state(null);
  let packOpen = $state(false);
  let packBusy = $state(false);
  let packAnnouncement = $state("");
  let packGeneration = 0;
  async function closePack() {
    if (packBusy) return;
    packGeneration += 1;
    packOpen = false;
    await tick();
    packTrigger?.focus({ preventScroll: true });
  }
  async function importLoadout(event: MouseEvent) {
    packTrigger = event.currentTarget as HTMLButtonElement;
    const generation = ++packGeneration;
    const picked = await openDialog({ title: i18n.optional("teams.openWorkspacePackTitle", "Open Workspace Pack"), multiple: false, filters: [{ name: "Workspace Pack or Agentfile", extensions: ["json"] }] });
    if (!picked || Array.isArray(picked) || generation !== packGeneration) return;
    const capturedPath = picked;
    busy = true;
    try {
      packPath = capturedPath;
      packProject = "";
      packResult = null;
      packReceiptId = null;
      const inspected = await install.inspectWorkspacePack(capturedPath, null);
      if (generation !== packGeneration) return;
      packPlan = inspected;
      packOpen = true;
      packAnnouncement = i18n.optional("teams.workspacePackReady", "Workspace Pack review ready.");
    } catch (e) {
      toast.error(i18n.t("teams.restoreFailed"), isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      busy = false;
    }
  }
  async function bindPackProject() {
    if (!packProject) return;
    const capturedPath = packPath;
    const capturedProject = packProject;
    const generation = ++packGeneration;
    packBusy = true;
    packAnnouncement = i18n.optional("teams.workspacePackPlanning", "Planning exact destinations…");
    try {
      const inspected = await install.inspectWorkspacePack(capturedPath, capturedProject);
      if (generation !== packGeneration || packProject !== capturedProject || packPath !== capturedPath) return;
      packPlan = inspected;
      packAnnouncement = i18n.optional("teams.workspacePackReady", "Workspace Pack review ready.");
    } catch (e) {
      toast.error(i18n.t("teams.restoreFailed"), isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      packBusy = false;
    }
  }
  async function applyPack() {
    if (!packPlan || !packTruthFresh || packPlan.blockers.length > 0) return;
    const reviewedPlan = packPlan;
    const capturedPath = packPath;
    const capturedProject = reviewedPlan.projectPath;
    const registeredProjects = projects.list.map((project) => project.path);
    const generation = ++packGeneration;
    packBusy = true;
    packAnnouncement = i18n.optional("teams.workspacePackApplying", "Applying Workspace Pack…");
    try {
      const applied = await install.applyWorkspacePack(capturedPath, capturedProject, reviewedPlan, registeredProjects);
      if (generation !== packGeneration || packPath !== capturedPath) return;
      packPlan = applied.response.plan;
      packResult = applied.response.result;
      packReceiptId = applied.receiptId;
      packAnnouncement = packResult
        ? i18n.optional("teams.workspacePackComplete", "Workspace Pack apply complete.")
        : i18n.optional("teams.workspacePackChanged", "Workspace Pack plan changed. Review the refreshed plan.");
    } catch (e) {
      packAnnouncement = i18n.optional("teams.workspacePackFailed", "Workspace Pack apply failed.");
      toast.error(i18n.t("teams.restoreFailed"), isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      packBusy = false;
    }
  }
  async function viewPackActivity() {
    if (!packReceiptId) return;
    const receiptId = packReceiptId;
    await closePack();
    ui.openActivityReceipt(receiptId);
  }
</script>

<section class="lo">
  <div class="sr-only" role="status" aria-live="polite" aria-atomic="true">{reconcileAnnouncement}</div>
  <header class="lo-head">
    <div class="seg" role="tablist" aria-label={i18n.t("teams.view")}>
      <button bind:this={currentTab} class="seg-btn" class:on={tab === "current"} role="tab" aria-selected={tab === "current"} onclick={() => (tab = "current")}>
        <UsersIcon size={14} /> {i18n.t("teams.yourTeam")}
      </button>
      <button class="seg-btn" class:on={tab === "presets"} role="tab" aria-selected={tab === "presets"} onclick={() => (tab = "presets")}>
        <RocketIcon size={14} /> {i18n.t("teams.presetsTab")}
      </button>
    </div>

    {#if tab === "current"}
      <div class="lo-actions">
        {#if managed.length > 0}
          <button class="btn ghost" onclick={toggleAll}>{i18n.t(allCollapsed ? "common.expandAll" : "common.collapseAll")}</button>
          <button class="btn" disabled={!installTruthFresh} onclick={openSave}><SaveIcon size={15} /><span>{i18n.t("teams.saveAsTeam")}</span></button>
        {/if}
        <button class="btn" disabled={busy} onclick={importLoadout}><DownloadIcon size={15} /><span>{i18n.t("teams.restore")}</span></button>
        <button class="btn primary" disabled={!packTruthFresh || busy || portableManagedCount === 0} onclick={openExport}><UploadIcon size={15} /><span>{i18n.t("teams.export")}</span></button>
      </div>
    {/if}
  </header>

  {#if install.reconcileError}
    <aside class="install-truth-warning" aria-busy={install.reconciling}>
      <div class="reconcile-copy"><strong>{i18n.optional("reconcile.heading", "Installation status may be out of date")}</strong><p><span>{install.reconcileError}</span> {install.reconciled ? i18n.optional("reconcile.retained", "Your last known installation data is still shown.") : installTruthMessage}</p></div>
      <Button size="sm" loading={install.reconciling} onclick={(event) => void retryReconcile(event)}>{install.reconciling ? i18n.optional("reconcile.retrying", "Retrying…") : i18n.optional("reconcile.retry", "Retry status check")}</Button>
    </aside>
  {/if}

  {#if tab === "current"}
    {#if !install.reconciled}
      <p class="install-truth-unavailable">{installTruthMessage}</p>
    {:else if managed.length === 0}
      <EmptyState title={i18n.t("teams.emptyTitle")}>
        {#snippet icon()}<ArchiveIcon size={48} />{/snippet}
        {i18n.t("teams.emptyBody")}
        {#snippet cta()}
          <div class="empty-cta">
            <Button variant="primary" onclick={() => (tab = "presets")}>
              {#snippet icon()}<RocketIcon size={15} />{/snippet}
              {i18n.t("teams.browsePresets")}
            </Button>
            <button class="link-btn" onclick={() => ui.openPlaybook()}>{i18n.t("teams.openPlaybook")}</button>
          </div>
        {/snippet}
      </EmptyState>
    {:else}
      <p class="lo-sub">
        {i18n.count(managed.length, "common.agent.one", "common.agent.many")}{#if groups.length > 1} · {i18n.count(groups.length, "common.division.one", "common.division.many")}{/if}
      </p>
      <div class="groups">
        {#each groups as g (g.slug)}
          {@const Icon = resolveCategoryIcon(g.icon)}
          {@const isOpen = !collapsed.has(g.slug)}
          <section class="grp">
            <button class="grp-head" onclick={() => toggle(g.slug)} aria-expanded={isOpen}>
              <ChevronDown size={15} class={isOpen ? "grp-chev open" : "grp-chev"} />
              <span class="grp-ic" style="color:{g.color}"><Icon size={16} /></span>
              <span class="grp-label">{g.label}</span>
              <span class="grp-count">{g.rows.length}</span>
            </button>
            {#if isOpen}
              <ul class="rows">
                {#each g.rows as r (r.slug + r.tool + (r.projectPath ?? ""))}
                  <li class="row">
                    <span class="r-name">{r.name}</span>
                    <Pill tone="neutral">{install.toolLabel(r.tool)}</Pill>
                    {#if r.projectPath}<span class="r-proj" title={r.projectPath}>{r.projectPath.split("/").pop()}</span>{/if}
                  </li>
                {/each}
              </ul>
            {/if}
          </section>
        {/each}
      </div>
    {/if}
  {:else if openedTeam}
    {@const team = openedTeam}
    {@const st = teamStats(team.agents)}
    {@const HeadIcon = team.icon}
    <div class="tdetail">
      <div class="td-head">
        <span class="td-ic" style={team.color ? `color:${team.color}` : ""}><HeadIcon size={22} /></span>
        <div class="td-id">
          <h2 class="td-name">{team.label}</h2>
          <p class="td-desc">{team.description}</p>
        </div>
        <span class="td-count">{i18n.count(st.count, "common.agent.one", "common.agent.many")}{#if st.deployed > 0} · {i18n.t("common.detectedCount", { count: st.deployed })}{/if}</span>
        <Button variant="primary" onclick={() => deploy(i18n.t("teams.deployTeamTitle", { team: team.label }), team.agents)}>{i18n.t("teams.deploy")}</Button>
      </div>
      <div class="team-baseline" aria-busy={baselineBusy}>
        <label><span>Apply readiness baseline to project</span><select bind:value={baselineProject} disabled={baselineBusy}><option value="">Choose a registered project</option>{#each projects.list as project (project.path)}<option value={project.path}>{project.label}</option>{/each}</select></label>
        <Button size="sm" disabled={!baselineReady || baselineBusy} loading={baselineBusy} onclick={() => void applyTeamBaseline(false)}>Save {baselineRequirements.length} reviewed targets</Button>
        <Button size="sm" variant="primary" disabled={!baselineReady || baselineBusy} loading={baselineBusy} onclick={() => void applyTeamBaseline(true)}>Save targets and subscribe</Button>
        {#if baselineBlockingMessage}<p class="baseline-error" role="alert">{baselineBlockingMessage}</p>{/if}
        <div class="sr-only" role="status" aria-live="polite" aria-atomic="true">{baselineAnnouncement}</div>
      </div>

      <h3 class="td-sec">{i18n.t("teams.tryThese")}</h3>
      <div class="td-examples">
        {#each detailExamples as ex (ex)}
          <StarterPrompt template={ex} />
        {/each}
      </div>

      <h3 class="td-sec">{i18n.count(st.count, "common.agent.one", "common.agent.many")}{#if detailGroups.length > 1} · {i18n.count(detailGroups.length, "common.division.one", "common.division.many")}{/if}</h3>
      <div class="groups">
        {#each detailGroups as g (g.slug)}
          {@const Icon = resolveCategoryIcon(g.icon)}
          {@const isOpen = !teamCollapsed.has(g.slug)}
          <section class="grp">
            <button class="grp-head" onclick={() => toggleTeamGroup(g.slug)} aria-expanded={isOpen}>
              <ChevronDown size={15} class={isOpen ? "grp-chev open" : "grp-chev"} />
              <span class="grp-ic" style="color:{g.color}"><Icon size={15} /></span>
              <span class="grp-label">{g.label}</span>
              <span class="grp-count">{g.rows.length}</span>
            </button>
            {#if isOpen}
              <ul class="rows">
                {#each g.rows as a (a.slug)}
                  <li class="row"><span class="row-emoji">{a.emoji ?? "🧩"}</span><span class="r-name">{a.name}</span></li>
                {/each}
              </ul>
            {/if}
          </section>
        {/each}
        {#if detailMissing > 0}
          <p class="td-missing">{i18n.t("teams.missingAgents", { count: detailMissing })}</p>
        {/if}
      </div>
    </div>
  {:else}
    <div class="cards">
      {#if teams.saved.length > 0}
        <h2 class="cards-h">{i18n.t("teams.savedTeams")}</h2>
        <ul class="card-list">
          {#each teams.saved as t (t.id)}
            {@const st = teamStats(t.agents)}
            <li class="card">
              <button class="card-main" onclick={() => openSaved(t)}>
                <span class="card-ic saved"><UsersIcon size={18} /></span>
                <span class="card-body">
                  <span class="card-title">{t.name}</span>
                  <span class="card-desc">{i18n.count(st.count, "common.agent.one", "common.agent.many")}{#if st.deployed > 0} · {i18n.t("common.detectedCount", { count: st.deployed })}{/if}</span>
                </span>
                <ChevronRight size={16} class="card-go" />
              </button>
              <button class="card-del" title={i18n.t("teams.deleteTeam")} aria-label={i18n.t("teams.deleteTeamAria", { team: t.name })} onclick={() => deleteSaved(t)}><Trash2 size={14} /></button>
            </li>
          {/each}
        </ul>
      {/if}

      <h2 class="cards-h">{i18n.t("teams.presets")}</h2>
      <ul class="card-list">
        {#each PRESET_TEAMS as p (p.slug)}
          {@const st = teamStats(p.agents)}
          {@const Icon = p.icon}
          <li class="card">
            <button class="card-main" onclick={() => openPreset(p)}>
              <span class="card-ic" style="color:{p.color}"><Icon size={18} /></span>
              <span class="card-body">
                <span class="card-title">{presetLabel(p.slug)}</span>
                <span class="card-desc">{presetDescription(p.slug)}</span>
                <span class="card-meta">{i18n.count(st.count, "common.agent.one", "common.agent.many")}{#if st.deployed > 0} · {i18n.t("common.detectedCount", { count: st.deployed })}{/if}</span>
              </span>
              <ChevronRight size={16} class="card-go" />
            </button>
          </li>
        {/each}
      </ul>
    </div>
  {/if}
</section>

{#if deployTarget}
  <InstallModal title={deployTarget.title} agentSlugs={deployTarget.agents} onClose={() => (deployTarget = null)} />
{/if}

{#if exportOpen}
  <Modal open title={i18n.optional("teams.exportWorkspacePack", "Export Workspace Pack")} defaultFocus="first" onClose={() => (exportOpen = false)}>
    <Input bind:value={exportName} placeholder={i18n.optional("teams.workspacePackName", "Workspace name")} ariaLabel={i18n.optional("teams.workspacePackName", "Workspace name")} />
    <label class="pack-field"><span>{i18n.optional("teams.workspacePackScope", "Scope")}</span><select bind:value={exportScope}><option value="user">{i18n.optional("teams.workspacePackGlobal", "Global")}</option><option value="project">{i18n.optional("teams.workspacePackProject", "Project")}</option></select></label>
    {#if exportScope === "project"}
      <label class="pack-field"><span>{i18n.optional("teams.workspacePackProjectBinding", "Project")}</span><select bind:value={exportProject}><option value="">{i18n.optional("teams.workspacePackChooseProject", "Choose a registered project")}</option>{#each projects.list as project (project.path)}<option value={project.path}>{project.label}</option>{/each}</select></label>
    {/if}
    {#snippet actions()}
      <Button variant="secondary" modalAction="cancel" onclick={() => (exportOpen = false)}>{i18n.t("common.cancel")}</Button>
      <Button variant="primary" modalAction="confirm" loading={busy} disabled={!packTruthFresh || !exportName.trim() || (exportScope === "project" && !exportProject)} onclick={exportLoadout}>{i18n.t("teams.export")}</Button>
    {/snippet}
  </Modal>
{/if}

{#if packOpen && packPlan}
  {@const reviewedPack = packPlan}
  <Modal open size="wide" title={i18n.optional("teams.workspacePackReview", "Review Workspace Pack")} dismissible={!packBusy} defaultFocus="cancel" onClose={closePack}>
    <div class="sr-only" role="status" aria-live="polite" aria-atomic="true">{packAnnouncement}</div>
    <div class="pack-summary"><strong>{packPlan.pack.name}</strong><span>{packPlan.pack.scope === "project" ? i18n.optional("teams.workspacePackProject", "Project") : i18n.optional("teams.workspacePackGlobal", "Global")}</span></div>
    {#if packPlan.pack.scope === "project" && !packPlan.projectPath}
      <div class="pack-bind"><label class="pack-field"><span>{i18n.optional("teams.workspacePackProjectBinding", "Bind to a registered project")}</span><select bind:value={packProject} disabled={packBusy}><option value="">{i18n.optional("teams.workspacePackChooseProject", "Choose a registered project")}</option>{#each projects.list as project (project.path)}<option value={project.path}>{project.label}</option>{/each}</select></label><Button size="sm" loading={packBusy} disabled={!packProject || packBusy} onclick={bindPackProject}>{i18n.optional("teams.workspacePackPlan", "Review destinations")}</Button></div>
    {:else}
      <div class="pack-plan" aria-busy={packBusy}>
        <h2>{i18n.optional("teams.workspacePackPlanHeading", "Complete plan")}</h2>
        <ul>{#each packPlan.agents as item (`agent:${item.reference.sourceId}:${item.reference.relativePath}:${item.tool}`)}<li><strong>{item.name}</strong><span>Agent · {item.tool} · {item.state}{item.dependency ? " · dependency" : ""}</span>{#each item.destinations as destination}<code>{destination}</code>{/each}</li>{/each}{#each packPlan.skills as item (`skill:${item.reference.sourceId}:${item.reference.relativePath}:${item.runtime}`)}<li><strong>{item.name}</strong><span>Skill · {item.runtime} · {item.state}{item.dependency ? " · dependency" : ""}</span>{#each item.destinations as destination}<code>{destination}</code>{/each}</li>{/each}</ul>
        {#if packPlan.pack.runbook}<p><strong>Runbook requirement:</strong> {packPlan.pack.runbook} — not executed automatically.</p>{/if}
        {#if packPlan.pack.instructions.length}<p><strong>Instruction requirements:</strong> {packPlan.pack.instructions.join(", ")} — not applied automatically.</p>{/if}
        {#if packPlan.pack.mcpServers.length}<p><strong>MCP requirements:</strong> {packPlan.pack.mcpServers.join(", ")} — not configured automatically.</p>{/if}
        {#if packPlan.warnings.length}<aside class="pack-warning"><strong>Warnings</strong><ul>{#each packPlan.warnings as warning (warning)}<li>{warning}</li>{/each}</ul></aside>{/if}
        {#if packPlan.blockers.length}<aside class="pack-blocker"><strong>Approval blocked</strong><ul>{#each packPlan.blockers as blocker (blocker)}<li>{blocker}</li>{/each}</ul></aside>{/if}
        {#if packResult}<section class="pack-results"><h2>Results</h2><p>{packResult.outcome}</p><ul>{#each packResult.items as item (`${item.kind}:${item.sourceId}:${item.relativePath}:${item.target}:${item.destination}`)}<li><strong>{item.kind} · {item.outcome}</strong><code>{item.destination}</code>{#if item.message}<span>{item.message}</span>{/if}</li>{/each}</ul></section>{/if}
      </div>
    {/if}
    {#snippet actions()}
      <Button variant="secondary" modalAction="cancel" disabled={packBusy} onclick={closePack}>{i18n.t("common.close")}</Button>
      {#if packResult && packReceiptId}<Button variant="secondary" onclick={viewPackActivity}>View Activity</Button>{/if}
      {#if !packResult && (reviewedPack.pack.scope === "user" || reviewedPack.projectPath !== null)}<Button variant="primary" modalAction="confirm" loading={packBusy} disabled={!packTruthFresh || reviewedPack.blockers.length > 0} onclick={applyPack}>Approve and apply</Button>{/if}
    {/snippet}
  </Modal>
{/if}

{#if saveOpen}
  <Modal open title={i18n.t("teams.saveModalTitle")} defaultFocus="first" onClose={() => (saveOpen = false)}>
    <p class="save-sub">{i18n.t("teams.saveModalBody", { count: managedSlugs.length })}</p>
    <Input bind:value={saveName} placeholder={i18n.t("teams.namePlaceholder")} ariaLabel={i18n.t("teams.nameAria")} />
    {#snippet actions()}
      <Button variant="secondary" modalAction="cancel" onclick={() => (saveOpen = false)}>{i18n.t("common.cancel")}</Button>
      <Button variant="primary" modalAction="confirm" disabled={!installTruthFresh || managedSlugs.length === 0} onclick={confirmSave}>{i18n.t("teams.saveTeam")}</Button>
    {/snippet}
  </Modal>
{/if}

<style>
  .lo { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .install-truth-warning { flex: none; display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2); padding: var(--space-2) var(--space-4); border-bottom: 1px solid var(--color-warning); background: var(--color-warning-subtle); color: var(--color-warning-strong); font-size: var(--text-body-sm); }
  .install-truth-warning span { flex: 1 1 auto; min-width: 0; overflow-wrap: anywhere; text-wrap: pretty; }
  .install-truth-warning button { flex: none; padding: var(--space-2) var(--space-3); border: 1px solid var(--color-border); border-radius: var(--radius-md); background: var(--color-surface-raised); }
  .install-truth-unavailable { flex: 1; display: grid; place-items: center; padding: var(--space-6); color: var(--color-warning-strong); font-size: var(--text-body-sm); text-wrap: pretty; }
  .lo-head {
    flex: none; display: flex; align-items: center; justify-content: space-between; gap: var(--space-3);
    padding: var(--space-3) var(--space-4); border-bottom: 1px solid var(--color-border);
  }
  .team-baseline { display: flex; flex-wrap: wrap; align-items: end; gap: var(--space-2); padding: var(--space-3) var(--space-4); border-bottom: 1px solid var(--color-border); }
  .team-baseline label { flex: 1; min-width: 14rem; display: grid; gap: var(--space-1); color: var(--color-text-secondary); font-size: var(--text-caption); }
  .team-baseline select { min-height: 32px; padding: 0 var(--space-2); border: 1px solid var(--color-border); border-radius: var(--radius-md); background: var(--color-surface-raised); color: var(--color-text-primary); }
  .baseline-error { flex-basis: 100%; margin: 0; color: var(--color-danger); font-size: var(--text-caption); }
  .lo-sub { flex: none; padding: var(--space-2) var(--space-4) 0; color: var(--color-text-secondary); font-size: var(--text-body-sm); }
  .lo-actions { display: flex; gap: var(--space-2); }

  /* tabs */
  .seg { display: flex; align-items: center; gap: 2px; padding: 2px; background: var(--color-surface-sunken); border: 1px solid var(--color-border); border-radius: var(--radius-md); }
  .seg-btn { display: inline-flex; align-items: center; gap: 6px; height: 28px; padding: 0 12px; border-radius: var(--radius-sm); background: transparent; color: var(--color-text-secondary); font-size: var(--text-body-sm); cursor: pointer; white-space: nowrap; }
  .seg-btn:hover { color: var(--color-text-primary); }
  .seg-btn.on { background: var(--color-surface-raised); color: var(--color-text-primary); box-shadow: var(--shadow-sm, 0 1px 2px rgba(0,0,0,0.08)); }

  .btn {
    display: inline-flex; align-items: center; gap: 6px;
    height: 32px; padding: 0 var(--space-3);
    border: 1px solid var(--color-border); border-radius: var(--radius-md);
    background: transparent; color: var(--color-text-secondary);
    font-size: var(--text-body-sm); cursor: pointer;
  }
  .btn:hover:not(:disabled) { color: var(--color-text-primary); background: var(--color-surface-sunken); }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .btn.primary { background: var(--color-brand); color: var(--color-text-inverse); border-color: transparent; }
  .btn.primary:hover:not(:disabled) { filter: brightness(1.08); background: var(--color-brand); }
  .btn.ghost { border-color: transparent; }

  /* ── Division groups (collapsible) ── */
  .groups { flex: 1; min-height: 0; overflow-y: auto; padding: var(--space-2) var(--space-3); }
  .grp { display: flex; flex-direction: column; }
  .grp-head {
    position: sticky; top: 0; z-index: 1;
    display: flex; align-items: center; gap: var(--space-2);
    width: 100%; padding: var(--space-2) var(--space-2);
    background: var(--color-surface); cursor: pointer; text-align: left;
    border-bottom: 1px solid var(--color-border);
  }
  .grp-head:hover { background: var(--color-surface-sunken); }
  :global(.grp-chev) { color: var(--color-text-muted); transition: transform var(--motion-duration-fast, 120ms) ease; transform: rotate(-90deg); flex: none; }
  :global(.grp-chev.open) { transform: rotate(0deg); }
  .grp-ic { flex: none; display: inline-flex; }
  .grp-label { flex: 1; min-width: 0; font-weight: var(--fw-semibold); color: var(--color-text-primary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .grp-count { flex: none; min-width: 20px; text-align: center; font-size: var(--text-caption); color: var(--color-text-muted); font-variant-numeric: tabular-nums; background: var(--color-surface-sunken); border-radius: var(--radius-full); padding: 1px 7px; }

  .rows { display: flex; flex-direction: column; gap: 1px; padding: 2px 0 var(--space-2) var(--space-4); }
  .row { display: flex; align-items: center; gap: var(--space-3); padding: var(--space-2) var(--space-3); border-radius: var(--radius-md); }
  .row:hover { background: var(--color-surface-sunken); }
  .r-name { flex: 1; min-width: 0; font-weight: var(--fw-medium); color: var(--color-text-primary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .r-proj { font-size: var(--text-caption); color: var(--color-text-muted); }

  /* ── Preset / saved team cards ── */
  .cards { flex: 1; min-height: 0; overflow-y: auto; padding: var(--space-3) var(--space-4); }
  .cards-h { font-size: var(--text-body-sm); font-weight: var(--fw-semibold); color: var(--color-text-muted); text-transform: uppercase; letter-spacing: 0.04em; margin: var(--space-3) 0 var(--space-2); }
  .cards-h:first-child { margin-top: 0; }
  .card-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--space-2); }
  .card {
    display: flex; align-items: stretch; gap: 0;
    border: 1px solid var(--color-border); border-radius: var(--radius-lg);
    background: var(--color-surface-raised); overflow: hidden;
  }
  .card:hover { border-color: var(--color-border-strong, var(--color-text-muted)); }
  .card-main {
    flex: 1; min-width: 0; display: flex; align-items: center; gap: var(--space-3);
    padding: var(--space-3); background: transparent; cursor: pointer; text-align: left;
  }
  .card-main:hover { background: var(--color-surface-sunken); }
  :global(.card-go) { flex: none; color: var(--color-text-muted); }
  .card-ic { flex: none; display: inline-flex; align-items: center; justify-content: center; width: 36px; height: 36px; border-radius: var(--radius-md); background: var(--color-surface-sunken); }
  .card-ic.saved { color: var(--color-text-secondary); }
  .card-body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .card-title { font-weight: var(--fw-semibold); color: var(--color-text-primary); }
  .card-desc { font-size: var(--text-body-sm); color: var(--color-text-secondary); }
  .card-meta { font-size: var(--text-caption); color: var(--color-text-muted); }
  .card-del { flex: none; align-self: center; display: inline-flex; align-items: center; justify-content: center; width: 28px; height: 28px; margin-right: var(--space-2); border-radius: var(--radius-md); color: var(--color-text-muted); cursor: pointer; }
  .card-del:hover { background: var(--color-surface-sunken); color: var(--color-danger); }

  .save-sub { font-size: var(--text-body-sm); color: var(--color-text-secondary); margin-bottom: var(--space-3); }
  .pack-field { display: flex; flex-direction: column; gap: var(--space-1); font-size: var(--text-body-sm); }
  .pack-field select { min-height: 36px; padding: 0 var(--space-2); border: 1px solid var(--color-border); border-radius: var(--radius-md); background: var(--color-surface); color: var(--color-text-primary); }
  .pack-summary, .pack-bind { display: flex; align-items: end; justify-content: space-between; gap: var(--space-3); }
  .pack-summary span { color: var(--color-text-muted); }
  .pack-bind .pack-field { flex: 1; }
  .pack-plan { display: flex; flex-direction: column; gap: var(--space-3); max-height: min(62vh, 640px); overflow-y: auto; }
  .pack-plan h2 { font-size: var(--text-body); color: var(--color-text-primary); }
  .pack-plan > ul, .pack-results ul { display: flex; flex-direction: column; gap: var(--space-2); }
  .pack-plan li { display: flex; flex-direction: column; gap: 2px; }
  .pack-plan code { overflow-wrap: anywhere; color: var(--color-text-secondary); font-size: var(--text-caption); }
  .pack-warning, .pack-blocker, .pack-results { padding: var(--space-3); border: 1px solid var(--color-border); border-radius: var(--radius-md); background: var(--color-surface-sunken); }
  .pack-blocker { border-color: var(--color-danger); }

  .empty-cta { display: flex; flex-direction: column; align-items: center; gap: var(--space-2); }
  .link-btn { background: transparent; color: var(--color-text-link, var(--color-brand)); font-size: var(--text-body-sm); cursor: pointer; padding: 2px; }
  .link-btn:hover { text-decoration: underline; }

  /* ── Team detail (master/detail over the presets tab) ── */
  .tdetail { flex: 1; min-height: 0; overflow-y: auto; padding: var(--space-3) var(--space-4) var(--space-5); display: flex; flex-direction: column; gap: var(--space-3); }
  .td-head { display: flex; align-items: center; gap: var(--space-3); }
  .td-ic { flex: none; display: inline-flex; align-items: center; justify-content: center; width: 44px; height: 44px; border-radius: var(--radius-md); background: var(--color-surface-sunken); }
  .td-id { flex: 1; min-width: 0; }
  .td-name { font-size: var(--text-h2); font-weight: var(--fw-semibold); color: var(--color-text-primary); }
  .td-desc { font-size: var(--text-body-sm); color: var(--color-text-secondary); }
  .td-count { flex: none; font-size: var(--text-body-sm); color: var(--color-text-muted); font-variant-numeric: tabular-nums; white-space: nowrap; }
  .td-sec { font-size: var(--text-caption); font-weight: var(--fw-semibold); color: var(--color-text-muted); text-transform: uppercase; letter-spacing: 0.04em; }
  .td-examples { display: flex; flex-direction: column; gap: var(--space-2); }
  .td-missing { font-size: var(--text-caption); color: var(--color-text-muted); padding: var(--space-2) var(--space-2) 0; }
  .row-emoji { flex: none; font-size: 15px; line-height: 1; }
  /* In the detail, the whole pane scrolls — don't make the group list its own
     scroll region (it inherits .groups from the "Your team" tab). */
  .tdetail .groups { flex: none; overflow: visible; padding: 0; }
</style>
