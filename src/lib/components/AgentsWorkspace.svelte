<script lang="ts">
  /**
   * AgentsWorkspace — the unified Agents surface. Replaces the old split between
   * PersonaDiscover (catalog browse) and AgentLibrary (installed view): an agent
   * and its cross-tool deployment are ONE object now.
   *
   * Three panes: the app sidebar (Nav, in +page) · a list pane (filter lens +
   * search + category + bulk select) · a persistent, resizable detail pane
   * (persona + the DeploymentMatrix). Install state is a FILTER over one list,
   * not a separate destination — so "what an agent does" and "where it's
   * installed" are finally visible together.
   */
  import { onMount, tick } from "svelte";
  import SearchIcon from "@lucide/svelte/icons/search";
  import RefreshIcon from "@lucide/svelte/icons/refresh-cw";
  import ChevronDown from "@lucide/svelte/icons/chevron-down";
  import TrashIcon from "@lucide/svelte/icons/trash-2";
  import PlusIcon from "@lucide/svelte/icons/plus";
  import XIcon from "@lucide/svelte/icons/x";
  import AlertTriangle from "@lucide/svelte/icons/triangle-alert";
  import LayersIcon from "@lucide/svelte/icons/layers";

  import Input from "./Input.svelte";
  import Button from "./Button.svelte";
  import Pill from "./Pill.svelte";
  import EmptyState from "./EmptyState.svelte";
  import LoadingState from "./LoadingState.svelte";
  import ResizeHandle from "./ResizeHandle.svelte";
  import DiffModal from "./DiffModal.svelte";
  import DivisionsLanding from "./DivisionsLanding.svelte";
  import InstallModal from "./InstallModal.svelte";
  import OllamaDeployModal from "./OllamaDeployModal.svelte";
  import StarterPrompt from "./StarterPrompt.svelte";
  import AgentLibrarySidebar from "./AgentLibrarySidebar.svelte";
  import AgentSourceManager from "./AgentSourceManager.svelte";
  import AgentCreatorModal from "./AgentCreatorModal.svelte";
  import AgentApprovalInbox from "./AgentApprovalInbox.svelte";
  import AgentDetailTabs from "./AgentDetailTabs.svelte";
  import AgentOrganizerModal from "./AgentOrganizerModal.svelte";
  import { buildAgentBrowseViews, findAgentPackage, sameAgent } from "$lib/agents/libraryModel";
  import { divisionPrompt } from "$lib/data/playbook";
  import DownloadIcon from "@lucide/svelte/icons/download";

  import { corpus } from "$lib/stores/corpus.svelte";
  import { install } from "$lib/stores/install.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import {
    ui,
    DETAIL_PANE_MIN_WIDTH,
    DETAIL_PANE_DEFAULT_WIDTH,
    clampDetailPaneWidth,
  } from "$lib/stores/ui.svelte";
  import { resolveCategoryIcon } from "$lib/util/categoryIcon";
  import { i18n } from "$lib/stores/i18n.svelte";
  import { agentLibrary } from "$lib/stores/agentLibrary.svelte";
  import { agentTextRead } from "$lib/api";
  import { installStateMessageKey } from "$lib/agents/libraryModel";
  import type { MessageKey } from "$lib/i18n/messages";
  import type { Agent, AgentPackageResult, InstalledAgent, InstallState, Tool } from "$lib/types";

  let agentCatalogLoaded = $state(false);
  onMount(() => {
    void Promise.all([corpus.ensureLoaded(), agentLibrary.load()]).then(() => (agentCatalogLoaded = true));
  });

  // ── OS-style dropdown dismissal: click anywhere outside (or Escape) closes the
  //    open menu. Each trigger button is excluded so clicking it just toggles. ──
  let catBtn = $state<HTMLElement>();
  let catMenu = $state<HTMLElement>();
  let bulkBtn = $state<HTMLElement>();
  let bulkMenu = $state<HTMLElement>();
  function onDocClick(e: MouseEvent) {
    const t = e.target as Node | null;
    if (!t) return;
    if (catMenuOpen && !catBtn?.contains(t) && !catMenu?.contains(t)) catMenuOpen = false;
    if (menuOpen && !bulkBtn?.contains(t) && !bulkMenu?.contains(t)) menuOpen = false;
  }
  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      catMenuOpen = false;
      menuOpen = false;
    }
  }
  onMount(() => {
    document.addEventListener("click", onDocClick);
    window.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("click", onDocClick);
      window.removeEventListener("keydown", onKey);
    };
  });

  // ── Install rows grouped by agent slug (reactive over the reconcile) ──
  const installsBySlug = $derived.by(() => {
    const m = new Map<string, InstalledAgent[]>();
    for (const r of install.installed) {
      const a = m.get(r.slug);
      if (a) a.push(r);
      else m.set(r.slug, [r]);
    }
    return m;
  });


  // ── List state ── (category lives in ui so back/forward + division
  // deep-links drive it; search query stays local — not a navigation.)
  let query = $state("");
  let catMenuOpen = $state(false);

  // ── Install-state lens ── (filter the agent list by deployment state, the
  // same dot tones shown per row). Local + localStorage like the Tools lens —
  // not nav state. An agent matches a bucket if ANY of its install rows is in
  // it; "none" = no install rows anywhere.
  type Lens = "all" | "attention" | InstallState | "none";
  // The lens lives in nav (ui.agentsLens) — the single source of truth — so the
  // Dashboard can deep-link a filter ("5 need attention" → the flat attention list)
  // and back/forward restores it. It no longer persists across launches: a sticky
  // filter would hijack the divisions landing, since an active lens now switches the
  // list to a flat all-divisions view (see showDivisions).
  const lens: Lens = $derived(ui.agentsLens as Lens);
  function setLens(l: Lens) { ui.setAgentsLens(l); }

  /** Which state buckets an agent falls into, across all its install rows. */
  function buckets(slug: string): Set<Lens> {
    const rows = installsBySlug.get(slug) ?? [];
    const s = new Set<Lens>();
    if (!install.reconciled) return s;
    if (rows.length === 0) { s.add("none"); return s; }
    for (const r of rows) {
      if (r.state === "current") s.add("current");
      else if (r.state === "outdated" || r.state === "modified") { s.add(r.state); s.add("attention"); }
      else if (r.state === "foreign") s.add("foreign");
      else if (r.state === "missing" || r.state === "sourceUnavailable") { s.add(r.state); s.add("attention"); }
      else if (r.state === "disabled") s.add("disabled");
    }
    return s;
  }

  // Division + search filtered (pre-lens) — lens counts are computed over THIS
  // so they reflect the current division/search, not the selected lens.
  const sourcePackages = $derived.by(() => agentLibrary.results.flatMap((result) =>
    result.agents.map((pkg) => ({ pkg, source: result.source }))
  ));
  const base = $derived(buildAgentBrowseViews(corpus.agents, sourcePackages, ui.agentsCategory, query));
  const visible = $derived(lens === "all" ? base : base.filter(({ agent }) => buckets(agent.slug).has(lens)));

  const lensCounts = $derived.by<Record<Lens, number>>(() => {
    const c: Record<Lens, number> = {
      all: base.length, attention: 0, current: 0, outdated: 0, modified: 0,
      missing: 0, foreign: 0, disabled: 0, sourceUnavailable: 0, none: 0,
    };
    for (const { agent } of base) for (const b of buckets(agent.slug)) c[b]++;
    return c;
  });

  // Lens definitions paired with the row dot tones so the color story matches.
  const LENSES: { id: Lens; key: MessageKey; tone: string }[] = [
    { id: "all", key: "state.all", tone: "" },
    { id: "attention", key: "state.attention", tone: "warn" },
    { id: "current", key: "state.current", tone: "ok" },
    { id: "outdated", key: "state.outdated", tone: "warn" },
    { id: "modified", key: "state.modified", tone: "warn" },
    { id: "foreign", key: "state.foreign", tone: "info" },
    { id: "missing", key: "state.missing", tone: "danger" },
    { id: "disabled", key: "state.disabled", tone: "none" },
    { id: "sourceUnavailable", key: "state.sourceUnavailable", tone: "danger" },
    { id: "none", key: "state.none", tone: "none" },
  ];
  // Show "All" plus any bucket present in the current view (zero-count lenses hide).
  const visibleLenses = $derived(LENSES.filter((f) => f.id === "all" || lensCounts[f.id] > 0));
  // If the active lens empties out (e.g. switching to a division with none of
  // that state), fall back to All so the selection isn't an invisible filter.
  $effect(() => {
    if (lens !== "all" && lensCounts[lens] === 0) setLens("all");
  });

  // Close the detail pane when the open agent falls out of the list — e.g.
  // switching divisions or refining search leaves the detail showing an agent
  // you can no longer see in the picker. Only strands a real (in-corpus) agent;
  // an unresolved slug is left to the loader below.
  $effect(() => {
    const slug = ui.agentsSelected;
    if (!slug) return;
    const inCorpus = corpus.agents.some((a) => a.slug === slug);
    if (inCorpus && !base.some(({ agent }) => agent.slug === slug)) ui.selectAgent(null);
  });

  // The Agents tab LANDS on the division list (not a flat agent list): only when
  // no division is drilled into AND there's no active search. Picking a division
  // or typing a query switches to the agent list below.
  const showDivisions = $derived(ui.agentsCategory === null && !query.trim() && lens === "all");
  // Leaving the landing for the agent list shouldn't carry a stale agent-select
  // session; entering it shouldn't either (the landing has its own selection).
  $effect(() => {
    if (showDivisions && selectMode) exitSelect();
  });

  function pickCategory(slug: string | null) {
    ui.setAgentsCategory(slug);
    catMenuOpen = false;
  }
  const categoryLabel = $derived(ui.agentsCategory ? corpus.labelOf(ui.agentsCategory) : i18n.t("agents.allDivisions"));

  // ── Division overview banner — shown atop a division's agent list (not while
  //    searching or selecting): what the division is for + deploy-the-whole-
  //    division + a starter prompt. null = hidden. ──
  const divisionMeta = $derived.by(() => {
    const slug = ui.agentsCategory;
    if (!slug || query.trim() || selectMode) return null;
    const slugs = corpus.agents.filter((a) => a.category === slug).map((a) => a.slug);
    if (slugs.length === 0) return null;
    const label = corpus.labelOf(slug);
    const fallback = divisionPrompt(slug, label);
    const prompt = i18n.optional(`divisionPrompt.${slug}`, fallback, { division: label });
    return { slug, label, color: corpus.colorOf(slug), icon: corpus.iconOf(slug), slugs, prompt };
  });
  let divisionInstallOpen = $state(false);

  // Compact per-row state dots (one per install row, colored by state).
  function dotTone(s: InstallState): string {
    if (s === "current") return "ok";
    if (s === "outdated" || s === "modified") return "warn";
    if (s === "foreign") return "info";
    if (s === "disabled") return "none";
    return "danger";
  }

  // ── Detail selection (persistent pane) ──
  // Driven by ui.agentsSelected so back/forward + deep-links restore the open
  // agent. The effect shows the list-view stub instantly, then loads the body.
  let detailStub = $state<Agent | null>(null);
  let detail = $state<Agent | null>(null);
  let libraryDetail = $state<Agent | null>(null);
  let libraryPackage = $state<AgentPackageResult | null>(null);
  let detailLoading = $state(false);
  const selectedLibraryPackage = $derived(findAgentPackage(
    agentLibrary.packages,
    ui.agentsReference,
  ));
  $effect(() => {
    if (agentCatalogLoaded && ui.agentsReference && !selectedLibraryPackage) ui.selectAgent(null);
  });
  const panelAgent = $derived(selectedLibraryPackage?.agent ?? libraryDetail ?? detail ?? detailStub);
  const panelPackage = $derived(selectedLibraryPackage ?? libraryPackage ?? (panelAgent
    ? agentLibrary.packages.find((pkg) =>
        pkg.agent?.slug === panelAgent.slug
        && agentLibrary.sources.find((source) => source.id === pkg.reference.sourceId)?.kind.kind === "builtIn"
      ) ?? null
    : null));
  const panelSource = $derived(panelPackage
    ? agentLibrary.sources.find((source) => source.id === panelPackage.reference.sourceId) ?? null
    : null);

  $effect(() => {
    const slug = ui.agentsSelected;
    if (!slug) {
      detailStub = null;
      detail = null;
      detailLoading = false;
      return;
    }
    const stub = corpus.agents.find((a) => a.slug === slug) ?? null;
    detailStub = stub;
    if (stub?.body) {
      detail = stub;
      detailLoading = false;
      return;
    }
    detail = null;
    detailLoading = true;
    void corpus.get(slug).then((full) => {
      if (ui.agentsSelected === slug) {
        detail = full;
        detailLoading = false;
      }
    });
  });

  function openAgent(a: Agent) {
    libraryDetail = null;
    libraryPackage = null;
    agentLibrary.selectedReference = null;
    ui.selectAgent(a.slug);
  }
  function openLibraryAgent(pkg: AgentPackageResult) {
    if (!pkg.agent) return;
    ui.openAgentReference(pkg.reference);
    libraryPackage = pkg;
    libraryDetail = pkg.agent;
    agentLibrary.selectedReference = pkg.reference;
    void agentLibrary.touchRecent(pkg.reference);
  }
  function closeDetail() {
    libraryDetail = null;
    libraryPackage = null;
    agentLibrary.selectedReference = null;
    ui.selectAgent(null);
  }

  let sourceManagerOpen = $state(false);
  let creatorOpen = $state(false);
  let organizerOpen = $state(false);
  let approvalsOpen = $state(false);
  let collectionInstall = $state<string | null>(null);
  let creatorInitial = $state<{ relativePath: string; text: string } | null>(null);
  let sourceButton: HTMLButtonElement | undefined = $state();
  let creatorButton: HTMLButtonElement | undefined = $state();
  let organizerButton: HTMLButtonElement | undefined = $state();
  let approvalsButton: HTMLButtonElement | undefined = $state();
  let rescanButton: HTMLButtonElement | undefined = $state();
  async function editLibraryAgent() {
    if (!panelPackage) return;
    const text = await agentTextRead(panelPackage.reference);
    creatorInitial = { relativePath: panelPackage.reference.relativePath, text };
    creatorOpen = true;
  }
  function closeSourceManager() {
    sourceManagerOpen = false;
    setTimeout(() => sourceButton?.focus());
  }
  function closeCreator() {
    creatorOpen = false;
    creatorInitial = null;
    setTimeout(() => creatorButton?.focus());
  }
  function closeOrganizer() {
    organizerOpen = false;
    setTimeout(() => organizerButton?.focus());
  }
  function closeApprovals() {
    const returnToReview = !!ui.reviewReturnId;
    ui.agentApprovalId = null;
    approvalsOpen = false;
    if (returnToReview) ui.returnToActivityReview();
    else setTimeout(() => approvalsButton?.focus());
  }

  const pendingApprovals = $derived(agentLibrary.library.approvals.filter((approval) => approval.state === "pending").length);

  // ── Diff modal (opened from the deployment pills) ──
  let diffTarget = $state<{ slug: string; tool: Tool; projectPath: string | null; name: string } | null>(null);
  // ── Install modal (the shared destinations × tools grid) for the open agent ──
  let installOpen = $state(false);
  let ollamaOpen = $state(false);

  $effect(() => {
    if (ui.agentApprovalId) approvalsOpen = true;
  });

  $effect(() => {
    const recovery = ui.agentRecovery;
    if (!recovery || !panelPackage || !install.reconciled) return;
    if (panelPackage.reference.sourceId !== recovery.reference.sourceId
      || panelPackage.reference.relativePath !== recovery.reference.relativePath) return;
    installOpen = true;
  });

  function closeInstall(): void {
    const returnToRecovery = !!ui.agentRecovery && !!ui.recoveryReturnId;
    installOpen = false;
    ui.agentRecovery = null;
    if (returnToRecovery) ui.returnToSettingsRecovery();
  }

  // ── Bulk select (lifted from the old Library, now over the unified list) ──
  let selectMode = $state(false);
  let selected = $state<Set<string>>(new Set());
  let menuOpen = $state(false);
  let bulkBusy = $state(false);
  let confirmDelete = $state(false);
  // Bulk deploy: the shared InstallModal opened over the current selection.
  let bulkInstallOpen = $state(false);

  function enterSelect() { selectMode = true; }
  function exitSelect() { selectMode = false; menuOpen = false; selected = new Set(); }
  function toggleRow(slug: string) {
    const next = new Set(selected);
    if (next.has(slug)) next.delete(slug);
    else next.add(slug);
    selected = next;
  }
  const allVisibleSelected = $derived(visible.length > 0 && visible.every(({ agent }) => selected.has(agent.slug)));
  const someSelected = $derived(selected.size > 0 && !allVisibleSelected);
  function toggleAll() {
    if (allVisibleSelected) selected = new Set();
    else selected = new Set(visible.map(({ agent }) => agent.slug));
  }
  // Prune selection to agents that still exist after a reconcile/reload.
  $effect(() => {
    const live = new Set([
      ...corpus.agents.map((agent) => agent.slug),
      ...sourcePackages.flatMap(({ pkg, source }) =>
        source.kind.kind !== "builtIn" && pkg.agent ? [pkg.agent.slug] : []
      ),
    ]);
    if ([...selected].some((s) => !live.has(s))) {
      selected = new Set([...selected].filter((s) => live.has(s)));
    }
  });

  const selInstalls = $derived([...selected].flatMap((slug) => installsBySlug.get(slug) ?? []));
  const installTruthFresh = $derived(install.reconciled && !install.reconcileError);
  const canBulkUpdate = $derived(installTruthFresh && selInstalls.some((i) => i.state !== "current"));
  const canBulkTrack = $derived(installTruthFresh && selInstalls.some((i) => i.state === "foreign"));
  // Foreign rows = files we don't manage ("not ours"). When the selection has any,
  // the destructive action is a genuine delete; otherwise it's a reversible
  // uninstall (catalog agents re-install; any edits are backed up first).
  const selHasForeign = $derived(selInstalls.some((i) => i.state === "foreign"));

  async function runBulk(action: "update" | "track" | "uninstall", verbKey: MessageKey) {
    if (!installTruthFresh) return;
    let picked = selInstalls;
    if (action === "update") picked = selInstalls.filter((i) => i.state !== "current");
    else if (action === "track") picked = selInstalls.filter((i) => i.state === "foreign");
    const targets = picked.map((i) => ({ slug: i.slug, tool: i.tool, projectPath: i.projectPath }));
    if (targets.length === 0) return;
    menuOpen = false;
    bulkBusy = true;
    try {
      const { ok, fail, receiptId } = await install.bulk(action, targets);
      const verb = i18n.t(verbKey);
      const receiptAction = { label: i18n.t("activity.viewReceipt"), onClick: () => ui.openActivityReceipt(receiptId) };
      if (fail === 0) toast.success(i18n.t("agents.bulkSuccess", { verb, count: ok }), undefined, receiptAction);
      else toast.error(i18n.t("agents.bulkError", { verb, ok, fail }), undefined, receiptAction);
      selected = new Set();
    } finally {
      bulkBusy = false;
    }
  }

  const scanning = $derived(install.reconciling && !install.reconciled);
  let reconcileAnnouncement = $state("");
  let priorReconcileError: string | null = $state(null);
  let priorReconcileTerminal = $state(0);
  let priorReconciling = $state(false);

  $effect(() => {
    const error = install.reconcileError;
    const reconciling = install.reconciling;
    const terminal = install.reconcileTerminal;
    if (reconciling && !priorReconciling) {
      reconcileAnnouncement = i18n.optional(
        "reconcile.refreshing",
        error ? "Refreshing installation status…" : "Checking installation status…",
      );
    } else if (error && (error !== priorReconcileError || terminal !== priorReconcileTerminal)) {
      reconcileAnnouncement = priorReconcileError
        ? i18n.optional("reconcile.stillOutOfDate", "Installation status is still out of date. {message}", { message: error })
        : i18n.optional("reconcile.outOfDate", "Installation status may be out of date. {message}", { message: error });
    } else if (!error && priorReconcileError && !reconciling) {
      reconcileAnnouncement = i18n.optional("reconcile.upToDate", "Installation status is up to date.");
    }
    priorReconcileError = error;
    priorReconcileTerminal = terminal;
    priorReconciling = reconciling;
  });

  async function retryReconcile(event: MouseEvent): Promise<void> {
    const restoreFocus = event.currentTarget === document.activeElement;
    reconcileAnnouncement = i18n.optional("reconcile.refreshing", "Refreshing installation status…");
    await install.reconcile();
    if (install.reconcileError) {
      reconcileAnnouncement = i18n.optional(
        "reconcile.stillOutOfDate",
        "Installation status is still out of date. {message}",
        { message: install.reconcileError },
      );
    } else if (restoreFocus) {
      await tick();
      rescanButton?.focus({ preventScroll: true });
    }
  }
</script>

<section class="ws" class:sel={!!panelAgent}>
  <AgentLibrarySidebar onSelectAgent={openLibraryAgent} onSelectCollection={(name) => (collectionInstall = name)} />
  <!-- ── List pane ── -->
  <div class="list-pane">
    <div class="lp-head">
      <div class="lp-search-row">
        <button class="ghost" bind:this={sourceButton} onclick={() => (sourceManagerOpen = true)}>{i18n.t("agents.sources")}</button>
        <button class="ghost" bind:this={creatorButton} onclick={() => { creatorInitial = null; creatorOpen = true; }}><PlusIcon size={14} /> {i18n.t("agents.create")}</button>
        <button class="ghost" bind:this={organizerButton} onclick={() => (organizerOpen = true)}>{i18n.t("agents.organize")}</button>
        <button class="ghost" bind:this={approvalsButton} onclick={() => (approvalsOpen = true)}>
          {i18n.t("agents.approvals")}{#if pendingApprovals > 0}<span class="badge" aria-label={i18n.t("agents.pendingApprovals", { count: pendingApprovals })}>{pendingApprovals}</span>{/if}
        </button>
        <div class="cat-wrap">
          <button class="ghost cat-btn" bind:this={catBtn} onclick={() => (catMenuOpen = !catMenuOpen)}>
            <span class="truncate">{categoryLabel}</span><ChevronDown size={13} />
          </button>
          {#if catMenuOpen}
            <div class="cat-menu" role="menu" bind:this={catMenu}>
              <button class="cat-opt" role="menuitem" class:on={!ui.agentsCategory} onclick={() => pickCategory(null)}>
                <LayersIcon size={14} /><span class="truncate">{i18n.t("agents.allDivisions")}</span><span class="cat-c">{corpus.agents.length + sourcePackages.filter(({ pkg, source }) => source.kind.kind !== "builtIn" && pkg.agent).length}</span>
              </button>
              {#each corpus.tiles as c (c.slug)}
                {@const Icon = resolveCategoryIcon(c.icon)}
                <button class="cat-opt" role="menuitem" class:on={ui.agentsCategory === c.slug} onclick={() => pickCategory(c.slug)}>
                  <span class="cat-ic" style="color:{corpus.colorOf(c.slug)}"><Icon size={14} /></span><span class="truncate">{c.label}</span><span class="cat-c">{c.count}</span>
                </button>
              {/each}
            </div>
          {/if}
        </div>
        <Input bind:value={query} variant="search" placeholder={i18n.t("agents.searchPlaceholder")} ariaLabel={i18n.t("agents.searchLabel")} />
        {#if visible.length > 0 && !showDivisions}
          {#if selectMode}
            <button class="ghost" onclick={exitSelect}>{i18n.t("common.done")}</button>
          {:else}
            <button class="ghost" onclick={enterSelect}>{i18n.t("common.select")}</button>
          {/if}
        {/if}
        <button class="ghost icon" data-install-rescan bind:this={rescanButton} title={i18n.t("agents.rescanTitle")} aria-label={i18n.t("agents.rescanTitle")} onclick={() => install.reconcile()}>
          <RefreshIcon size={15} />
        </button>
      </div>

      {#if !showDivisions && !selectMode && base.length > 0}
        <div class="seg" role="tablist" aria-label={i18n.t("agents.filterByInstallState")}>
          {#each visibleLenses as f (f.id)}
            <button
              class="seg-btn"
              class:on={lens === f.id}
              role="tab"
              aria-selected={lens === f.id}
              onclick={() => setLens(f.id)}
            >
              {#if f.tone}<span class="seg-dot" data-tone={f.tone}></span>{/if}
              {i18n.t(f.key)}
              <span class="seg-c">{lensCounts[f.id]}</span>
            </button>
          {/each}
        </div>
      {/if}

      {#if selectMode}
        <div class="bulk-bar">
          <input
            type="checkbox"
            class="check"
            checked={allVisibleSelected}
            indeterminate={someSelected}
            onchange={toggleAll}
            aria-label={i18n.t("agents.selectAllVisible")}
          />
          <span class="bulk-count">{i18n.t("common.selected", { count: selected.size })}</span>
          {#if selected.size > 0}
            <div class="bulk-menu-wrap">
              <button class="ghost" bind:this={bulkBtn} disabled={bulkBusy} onclick={() => (menuOpen = !menuOpen)}>
                {bulkBusy ? i18n.t("common.working") : i18n.t("agents.withSelected")}<ChevronDown size={14} />
              </button>
              {#if menuOpen}
                <div class="bulk-menu" role="menu" bind:this={bulkMenu}>
                  <button class="bulk-opt" role="menuitem" onclick={() => { menuOpen = false; bulkInstallOpen = true; }}>
                    <DownloadIcon size={14} /><span>{i18n.t("agents.installSelected")}</span>
                  </button>
                  <div class="bulk-div"></div>
                  <button class="bulk-opt" role="menuitem" disabled={!canBulkUpdate} title={canBulkUpdate ? "" : i18n.t("agents.allSelectedInSync")} onclick={() => runBulk("update", "agents.updatedVerb")}>
                    <RefreshIcon size={14} /><span>{i18n.t("agents.updateSelected")}</span>
                  </button>
                  <button class="bulk-opt" role="menuitem" disabled={!canBulkTrack} title={canBulkTrack ? "" : i18n.t("agents.nothingUntrackedSelection")} onclick={() => runBulk("track", "agents.trackedVerb")}>
                    <PlusIcon size={14} /><span>{i18n.t("agents.trackSelected")}</span>
                  </button>
                  <button class="bulk-opt" class:danger={selHasForeign} role="menuitem" disabled={!installTruthFresh} onclick={() => { menuOpen = false; confirmDelete = true; }}>
                    <TrashIcon size={14} /><span>{i18n.t(selHasForeign ? "agents.deleteSelected" : "agents.uninstallSelected")}</span>
                  </button>
                </div>
              {/if}
            </div>
          {/if}
        </div>
      {/if}
    </div>

    <div class="reconcile-announcement" role="status" aria-live="polite" aria-atomic="true">{reconcileAnnouncement}</div>
    {#if install.reconcileError}
      <aside class="reconcile-warning" aria-busy={install.reconciling}>
        <AlertTriangle size={16} aria-hidden="true" />
        <div class="reconcile-copy">
          <strong>{i18n.optional("reconcile.heading", "Installation status may be out of date")}</strong>
          <p class="reconcile-message">
            <span>{install.reconcileError}</span>
            {install.reconciled
              ? i18n.optional("reconcile.retained", "Your last known installation data is still shown.")
              : i18n.optional("reconcile.unavailable", "Installation status is unavailable until a retry succeeds.")}
          </p>
        </div>
        <Button size="sm" loading={install.reconciling} onclick={(event) => void retryReconcile(event)}>
          {install.reconciling
            ? i18n.optional("reconcile.retrying", "Retrying…")
            : i18n.optional("reconcile.retry", "Retry status check")}
        </Button>
      </aside>
    {/if}

    <div class="lp-list">
      {#if divisionMeta}
        {@const Icon = resolveCategoryIcon(divisionMeta.icon)}
        <div class="dov">
          <div class="dov-head">
            <span class="dov-ic" style="color:{divisionMeta.color}"><Icon size={18} /></span>
            <div class="dov-id">
              <span class="dov-name">{divisionMeta.label}</span>
              <span class="dov-sub">{i18n.t("agents.divisionOverview", { count: divisionMeta.slugs.length })}</span>
            </div>
            <button class="dov-deploy" onclick={() => (divisionInstallOpen = true)}><DownloadIcon size={14} /> {i18n.t("agents.deployDivision")}</button>
          </div>
          <StarterPrompt template={divisionMeta.prompt} />
        </div>
      {/if}
      {#if corpus.loading && corpus.agents.length === 0}
        <LoadingState rows={6} label={i18n.t("agents.loading")} />
      {:else if corpus.error && corpus.agents.length === 0}
        <EmptyState title={i18n.t("agents.corpusUnavailableTitle")} body={i18n.t("agents.corpusUnavailableBody")}>
          {#snippet icon()}<SearchIcon size={48} />{/snippet}
        </EmptyState>
      {:else if showDivisions}
        <DivisionsLanding />
      {:else if visible.length === 0}
        <EmptyState
          title={lens !== "all"
            ? i18n.t("agents.emptyStateLens", { state: i18n.t(LENSES.find((l) => l.id === lens)?.key ?? "state.all").toLowerCase() })
            : query.trim()
              ? i18n.t("agents.emptySearch", { query: query.trim() })
              : i18n.t("agents.emptyDivision")}
          body={lens !== "all" ? i18n.t("agents.emptyFilteredBody") : i18n.t("agents.emptyBody")}
        >
          {#snippet icon()}<SearchIcon size={48} />{/snippet}
        </EmptyState>
      {:else}
        <ul class="rows">
          {#each visible as view (view.key)}
            {@const a = view.agent}
            {@const rows = installsBySlug.get(a.slug) ?? []}
            {@const isSel = view.pkg
              ? !!selectedLibraryPackage && sameAgent(selectedLibraryPackage.reference, view.pkg.reference)
              : ui.agentsSelected === a.slug}
            <li class="row" class:active={isSel} class:picked={selectMode && selected.has(a.slug)}>
              {#if selectMode}
                <input type="checkbox" class="check" checked={selected.has(a.slug)} onchange={() => toggleRow(a.slug)} aria-label={`${i18n.t("common.select")} ${a.name}`} />
              {/if}
              <button class="row-main" onclick={() => view.pkg ? openLibraryAgent(view.pkg) : openAgent(a)} aria-current={isSel ? "true" : undefined}>
                <span class="row-emoji" aria-hidden="true">{a.emoji ?? "🧩"}</span>
                <span class="row-text">
                  <span class="row-name truncate">{a.name}</span>
                  {#if a.vibe}<span class="row-vibe truncate">{a.vibe}</span>{/if}
                </span>
                {#if rows.length > 0}
                  <span class="row-dots">
                    {#each rows as r (r.dest)}
                      <span class="state-chip" title={`${install.toolLabel(r.tool)} · ${i18n.t(installStateMessageKey(r.state))}`}>
                        <span class="dot" data-tone={dotTone(r.state)} aria-hidden="true"></span>
                        <span>{i18n.t(installStateMessageKey(r.state))}</span>
                      </span>
                    {/each}
                  </span>
                {/if}
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  </div>

  {#if panelAgent}
    <!-- ── Resize handle (grows the detail pane when dragged left) ── -->
    <div class="ws-resize">
      <ResizeHandle
        width={ui.detailPaneWidth}
        min={DETAIL_PANE_MIN_WIDTH}
        max={900}
        defaultWidth={DETAIL_PANE_DEFAULT_WIDTH}
        direction="left"
        label={i18n.t("common.resizeDetailPane")}
        onChange={(w) => (ui.detailPaneWidth = clampDetailPaneWidth(w))}
        onCommit={(w) => ui.setDetailPaneWidth(w)}
      />
    </div>

    <!-- ── Detail pane (only when an agent is selected) ── -->
    <aside class="detail-pane" style="width: {ui.detailPaneWidth}px" aria-label={i18n.t("agents.agentDetail")}>
      <div class="dp-bar">
        <button class="dp-close" onclick={closeDetail} aria-label={i18n.t("agents.closeDetail")} title={i18n.t("agents.closeDetail")}><XIcon size={16} /></button>
      </div>
      <div class="dp-scroll">
        <AgentDetailTabs
          agent={panelAgent}
          pkg={panelPackage}
          source={panelSource}
          loading={detailLoading}
          catalogDeployment={!libraryPackage}
          onCategory={(slug) => ui.openDivision(slug)}
          onInstall={() => (installOpen = true)}
          onLocalModel={() => (ollamaOpen = true)}
          onEdit={panelPackage ? editLibraryAgent : undefined}
          onDuplicate={panelPackage ? () => agentLibrary.duplicateDraft(panelPackage!.reference) : undefined}
          onDiff={(target) => (diffTarget = target)}
        />
      </div>
    </aside>

    <!-- Narrow-window overlay scrim: clicking dismisses the overlaid detail pane. -->
    <button class="ws-scrim" aria-label={i18n.t("agents.closeDetail")} onclick={closeDetail}></button>
  {/if}
</section>

{#if diffTarget}
  <DiffModal
    slug={diffTarget.slug}
    tool={diffTarget.tool}
    projectPath={diffTarget.projectPath}
    name={diffTarget.name}
    onClose={() => (diffTarget = null)}
  />
{/if}

<AgentSourceManager open={sourceManagerOpen} onClose={closeSourceManager} />
<AgentCreatorModal open={creatorOpen} initial={creatorInitial} onClose={closeCreator} />
<AgentOrganizerModal open={organizerOpen} pkg={panelPackage} onClose={closeOrganizer} />
<AgentApprovalInbox open={approvalsOpen} focusId={ui.agentApprovalId} onClose={closeApprovals} />

{#if installOpen && panelAgent}
  <InstallModal
    title={i18n.t("agents.installAgentTitle", { name: panelAgent.name })}
    agentSlugs={panelPackage ? [] : [panelAgent.slug]}
    agentPackage={panelPackage ?? undefined}
    historyIntent={ui.agentRecovery ?? undefined}
    onClose={closeInstall}
    onHistoryComplete={closeInstall}
  />
{/if}

{#if ollamaOpen && panelPackage && panelAgent}
  <OllamaDeployModal pkg={panelPackage} agent={panelAgent} onClose={() => (ollamaOpen = false)} />
{/if}

{#if bulkInstallOpen && selected.size > 0}
  <InstallModal
    title={i18n.t("agents.installSelectedTitle", { count: selected.size })}
    agentSlugs={[...selected]}
    onClose={() => (bulkInstallOpen = false)}
  />
{/if}

{#if divisionInstallOpen && divisionMeta}
  <InstallModal
    title={i18n.t("agents.installDivisionTitle", { division: divisionMeta.label })}
    agentSlugs={divisionMeta.slugs}
    onClose={() => (divisionInstallOpen = false)}
  />
{/if}

{#if collectionInstall}
  <InstallModal
    title={i18n.t("agents.collectionInstallTitle", { name: collectionInstall })}
    collectionName={collectionInstall}
    onClose={() => (collectionInstall = null)}
  />
{/if}

{#if confirmDelete}
  <button class="cd-scrim" aria-label={i18n.t("common.cancel")} onclick={() => (confirmDelete = false)}></button>
  <div class="cd-box" role="alertdialog" aria-modal="true" aria-label={i18n.t("agents.confirmDeleteAria")}>
    <div class="cd-head"><AlertTriangle size={20} /><h2>{i18n.t(selHasForeign ? "agents.confirmDelete" : "agents.confirmUninstall", { count: selected.size })}</h2></div>
    <p class="cd-body">
      {#if selHasForeign}
        {i18n.t("agents.deleteBody", { count: selInstalls.length })}
      {:else}
        {i18n.t("agents.uninstallBody", { count: selInstalls.length })}
      {/if}
    </p>
    <p class="cd-note">{i18n.t("agents.confirmTip")}</p>
    <div class="cd-actions">
      <button class="cd-cancel" onclick={() => (confirmDelete = false)}>{i18n.t("common.cancel")}</button>
      <button class="cd-delete" disabled={bulkBusy || !installTruthFresh} onclick={() => { if (!installTruthFresh) return; confirmDelete = false; void runBulk("uninstall", selHasForeign ? "agents.deletedVerb" : "agents.uninstalledVerb"); }}>
        <TrashIcon size={14} /> {i18n.t(selHasForeign ? "common.delete" : "common.uninstall")} {selInstalls.length}
      </button>
    </div>
  </div>
{/if}

<style>
  .ws { display: flex; height: 100%; min-height: 0; }

  /* ── List pane ── */
  .list-pane { flex: 1; min-width: 0; display: flex; flex-direction: column; min-height: 0; }
  .lp-head {
    flex: none; padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--color-border);
    display: flex; flex-direction: column; gap: var(--space-3);
  }
  .lp-search-row { display: flex; align-items: center; gap: var(--space-2); }
  .lp-search-row :global(.wrap) { flex: 1; min-width: 0; }
  .reconcile-announcement { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border: 0; }
  .reconcile-warning {
    flex: none; min-width: 0; display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2);
    padding: var(--space-2) var(--space-4); border-bottom: 1px solid var(--color-warning);
    background: var(--color-warning-subtle); color: var(--color-warning-strong);
  }
  .reconcile-copy { flex: 1 1 auto; min-width: 0; display: grid; gap: var(--space-1); }
  .reconcile-copy strong { font-size: var(--text-body); font-weight: var(--fw-medium); line-height: var(--lh-snug); text-wrap: balance; }
  .reconcile-message { margin: 0; font-size: var(--text-body-sm); line-height: var(--lh-normal); overflow-wrap: anywhere; text-wrap: pretty; }
  .reconcile-message span { margin-right: var(--space-1); }

  .ghost {
    display: inline-flex; align-items: center; gap: 6px; flex: none;
    height: 32px; padding: 0 var(--space-3);
    border: 1px solid var(--color-border); border-radius: var(--radius-md);
    background: transparent; color: var(--color-text-secondary);
    font-size: var(--text-body-sm); cursor: pointer;
  }
  .badge { min-width: 18px; padding: 1px 5px; border-radius: var(--radius-full); background: var(--color-danger); color: white; font-size: var(--text-caption); text-align: center; }
  .ghost:hover:not(:disabled) { color: var(--color-text-primary); background: var(--color-surface-sunken); }
  .ghost:disabled { opacity: 0.6; cursor: default; }
  .ghost.icon { padding: 0; width: 32px; justify-content: center; }

  .cat-wrap { position: relative; }
  .cat-btn { max-width: 180px; }
  .cat-menu {
    position: absolute; top: calc(100% + 4px); left: 0; z-index: 30;
    min-width: 220px; max-height: 320px; overflow-y: auto; padding: 4px;
    background: var(--color-surface-raised); border: 1px solid var(--color-border);
    border-radius: var(--radius-md); box-shadow: var(--shadow-lg);
    display: flex; flex-direction: column; gap: 1px;
  }
  .cat-opt {
    display: flex; align-items: center; gap: var(--space-2);
    padding: 6px 8px; border-radius: var(--radius-sm);
    background: transparent; color: var(--color-text-primary);
    font-size: var(--text-body-sm); text-align: left; cursor: pointer;
  }
  .cat-opt:hover { background: var(--color-surface-sunken); }
  .cat-opt.on { color: var(--color-brand); }
  .cat-opt .truncate { flex: 1; min-width: 0; }
  /* Division icon tinted with the division's brand color; dim to neutral when
     the row is the active selection so the brand-blue "on" state stays legible. */
  .cat-ic { display: inline-flex; flex: none; }
  .cat-opt.on .cat-ic { color: var(--color-brand) !important; }
  .cat-c { font-size: var(--text-caption); color: var(--color-text-muted); }

  .bulk-bar { display: flex; align-items: center; gap: var(--space-2); }
  .bulk-count { font-size: var(--text-body-sm); color: var(--color-brand); font-weight: var(--fw-medium); }

  /* ── Install-state lens (mirrors the Tools view segmented filter) ── */
  .seg {
    display: flex; align-items: center; gap: 2px; flex-wrap: wrap; min-width: 0;
    margin-top: var(--space-2); padding: 2px;
    background: var(--color-surface-sunken);
    border: 1px solid var(--color-border); border-radius: var(--radius-md);
  }
  .seg-btn {
    display: inline-flex; align-items: center; gap: 6px;
    height: 26px; padding: 0 10px; border-radius: var(--radius-sm);
    background: transparent; color: var(--color-text-secondary);
    font-size: var(--text-body-sm); cursor: pointer; white-space: nowrap;
  }
  .seg-btn:hover { color: var(--color-text-primary); }
  .seg-btn.on { background: var(--color-surface-raised); color: var(--color-text-primary); box-shadow: var(--shadow-sm, 0 1px 2px rgba(0,0,0,0.08)); }
  .seg-dot { width: 7px; height: 7px; border-radius: 999px; background: var(--color-text-muted); flex: none; }
  .seg-dot[data-tone="ok"]     { background: var(--color-success); }
  .seg-dot[data-tone="warn"]   { background: var(--color-warning); }
  .seg-dot[data-tone="info"]   { background: var(--color-brand); }
  .seg-dot[data-tone="danger"] { background: var(--color-danger); }
  .seg-c { color: var(--color-text-muted); font-variant-numeric: tabular-nums; }
  .seg-btn.on .seg-c { color: var(--color-text-secondary); }
  .bulk-menu-wrap { position: relative; margin-left: auto; }
  .bulk-menu {
    position: absolute; top: calc(100% + 6px); right: 0; z-index: 30;
    min-width: 280px; padding: 4px;
    background: var(--color-surface-raised); border: 1px solid var(--color-border);
    border-radius: var(--radius-md); box-shadow: var(--shadow-lg);
    display: flex; flex-direction: column; gap: 1px;
  }
  .bulk-opt {
    display: flex; align-items: center; gap: var(--space-2);
    padding: 8px 10px; border-radius: var(--radius-sm);
    background: transparent; color: var(--color-text-primary);
    font-size: var(--text-body-sm); text-align: left; cursor: pointer;
  }
  .bulk-opt:hover:not(:disabled) { background: var(--color-surface-sunken); }
  .bulk-opt:disabled { opacity: 0.4; cursor: default; }
  .bulk-opt.danger { color: var(--color-danger); }
  .bulk-opt.danger:hover { background: color-mix(in srgb, var(--color-danger) 12%, transparent); }
  .bulk-div { height: 1px; margin: 4px 0; background: var(--color-border); }

  .check { accent-color: var(--color-brand); cursor: pointer; width: 15px; height: 15px; flex: none; }

  /* ── Division overview banner ── */
  .dov {
    margin: var(--space-1) var(--space-1) var(--space-3);
    padding: var(--space-3);
    border: 1px solid var(--color-border); border-radius: var(--radius-lg);
    background: var(--color-surface-raised);
    display: flex; flex-direction: column; gap: var(--space-3);
  }
  .dov-head { display: flex; align-items: center; gap: var(--space-3); }
  .dov-ic { flex: none; display: inline-flex; align-items: center; justify-content: center; width: 34px; height: 34px; border-radius: var(--radius-md); background: var(--color-surface-sunken); }
  .dov-id { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .dov-name { font-size: var(--text-body); font-weight: var(--fw-semibold); color: var(--color-text-primary); }
  .dov-sub { font-size: var(--text-caption); color: var(--color-text-muted); }
  .dov-deploy {
    flex: none; display: inline-flex; align-items: center; gap: 6px;
    height: 30px; padding: 0 12px; border-radius: var(--radius-md);
    border: 1px solid var(--color-border); background: var(--color-surface-sunken);
    color: var(--color-text-primary); font-size: var(--text-body-sm); cursor: pointer;
  }
  .dov-deploy:hover { border-color: var(--color-brand); }

  /* ── Rows ── */
  .lp-list { flex: 1; overflow-y: auto; min-height: 0; padding: var(--space-2) var(--space-3); }
  .rows { display: flex; flex-direction: column; gap: 1px; }
  .row { display: flex; align-items: center; gap: var(--space-2); border-radius: var(--radius-md); padding-left: var(--space-2); }
  .row:hover { background: var(--color-surface-sunken); }
  .row.active { background: var(--color-brand-subtle); }
  .row.picked { background: color-mix(in srgb, var(--color-brand) 10%, transparent); }
  .row-main {
    flex: 1; min-width: 0; display: flex; align-items: center; gap: var(--space-3);
    padding: var(--space-2) var(--space-2); background: transparent; cursor: pointer; text-align: left;
  }
  .row-emoji { font-size: 19px; line-height: 1; flex: none; }
  .row-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .row-name { font-size: var(--text-body-sm); font-weight: var(--fw-medium); color: var(--color-text-primary); }
  .row-vibe { font-size: var(--text-caption); color: var(--color-text-muted); }
  .row-dots { display: inline-flex; align-items: center; gap: 4px; flex: none; flex-wrap: wrap; justify-content: flex-end; }
  .state-chip { display: inline-flex; align-items: center; gap: 4px; font-size: var(--text-caption); color: var(--color-text-muted); }
  .row-dots .dot { width: 7px; height: 7px; border-radius: 999px; background: var(--color-text-muted); }
  .dot[data-tone="ok"]     { background: var(--color-success); }
  .dot[data-tone="warn"]   { background: var(--color-warning); }
  .dot[data-tone="info"]   { background: var(--color-brand); }
  .dot[data-tone="danger"] { background: var(--color-danger); }

  /* ── Resize handle wrapper ── */
  .ws-resize { display: flex; flex: none; }

  /* ── Detail pane ── */
  .detail-pane {
    flex: none; max-width: 90vw;
    display: flex; flex-direction: column; min-height: 0;
    background: var(--color-surface-raised);
    border-left: 1px solid var(--color-border);
  }
  .dp-bar { flex: none; display: flex; justify-content: flex-end; padding: 6px 8px 0; }
  .dp-close {
    display: inline-flex; align-items: center; justify-content: center;
    width: 28px; height: 28px; border-radius: var(--radius-sm);
    color: var(--color-text-muted); background: transparent; cursor: pointer;
  }
  .dp-close:hover { background: var(--color-surface-sunken); color: var(--color-text-primary); }
  .dp-scroll { flex: 1; overflow-y: auto; min-height: 0; }

  /* Narrow-window overlay scrim — hidden by default, shown only under the
     breakpoint when a detail is open (see media query). */
  .ws-scrim { display: none; }

  @media (max-width: 860px) {
    .ws-resize { display: none; }
    .detail-pane {
      position: fixed; top: 36px; right: 0; bottom: 0; z-index: 41;
      width: min(var(--detail-w, 420px), 92vw) !important;
      box-shadow: var(--shadow-lg, -8px 0 24px rgba(0,0,0,0.18));
    }
    .ws:not(.sel) .detail-pane { display: none; }
    .ws.sel .ws-scrim {
      display: block; position: fixed; inset: 36px 0 0 0; z-index: 40;
      background: rgba(0,0,0,0.28); border: 0; cursor: default;
    }
  }
</style>
