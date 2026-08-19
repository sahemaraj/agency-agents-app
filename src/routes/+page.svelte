<script lang="ts">
  import { onMount } from "svelte";

  import Sidebar from "$lib/components/Sidebar.svelte";
  import ResizeHandle from "$lib/components/ResizeHandle.svelte";
  import Experts from "$lib/components/Experts.svelte";
  import LoadingState from "$lib/components/LoadingState.svelte";
  import CommandPalette from "$lib/components/CommandPalette.svelte";
  import Settings from "$lib/components/Settings.svelte";
  import AboutModal from "$lib/components/AboutModal.svelte";
  import PlaybookModal from "$lib/components/PlaybookModal.svelte";
  import DeviceFlowModal from "$lib/components/DeviceFlowModal.svelte";
  import Toast from "$lib/components/Toast.svelte";
  import TitlebarControls from "$lib/components/TitlebarControls.svelte";
  import UpdateIndicator from "$lib/components/UpdateIndicator.svelte";
  import StorageMigrationGate from "$lib/components/StorageMigrationGate.svelte";
  import PanelLeftClose from "@lucide/svelte/icons/panel-left-close";
  import PanelLeftOpen from "@lucide/svelte/icons/panel-left-open";
  import ArrowLeft from "@lucide/svelte/icons/arrow-left";
  import ArrowRight from "@lucide/svelte/icons/arrow-right";

  import {
    ui,
    SIDEBAR_MIN_WIDTH,
    SIDEBAR_MAX_WIDTH,
    SIDEBAR_DEFAULT_WIDTH,
  } from "$lib/stores/ui.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { skillSources } from "$lib/stores/skillSources.svelte";
  import { agentLibrary } from "$lib/stores/agentLibrary.svelte";
  import { experts } from "$lib/stores/experts.svelte";
  import { activity } from "$lib/stores/activity.svelte";
  import { i18n } from "$lib/stores/i18n.svelte";
  import {
    storageMigrationRetry,
    storageMigrationStart,
    storageMigrationStatus,
    storageLegacyConflictsDismiss,
    storageOpenDataDirectory,
    storageVisibleRevision,
  } from "$lib/api";
  import { isMac, shortcut } from "$lib/util/platform";
  import { appErrorMessage, isAppError, type SidebarSection, type StorageMigrationStatus, type ThemePreference } from "$lib/types";

  let migrationStatus = $state<StorageMigrationStatus | null>(null);
  let migrationBusy = $state(false);
  let migrationError = $state<string | null>(null);
  let showMigrationCompletion = $state(false);
  let narrowViewport = $state(false);
  let mobileSidebarOpen = $state(false);
  let visibleRevision = 0;
  const sidebarCollapsed = $derived(narrowViewport ? !mobileSidebarOpen : ui.sidebarCollapsed);
  const needsMigrationGate = $derived(
    migrationStatus !== null
      && (migrationStatus.state !== "complete"
        || migrationStatus.stage === "recovery"
        || showMigrationCompletion),
  );

  async function loadMigrationStatus() {
    try {
      migrationStatus = await storageMigrationStatus();
      if (migrationStatus.state === "complete") visibleRevision = await storageVisibleRevision();
    } catch (error) {
      migrationError = isAppError(error) ? appErrorMessage(error) : String(error);
      if (isAppError(error) && error.code === "storage_unsupported") {
        migrationStatus = { state: "unsupported", stage: "unsupported", detail: migrationError, legacyConflicts: [] };
      } else {
        migrationStatus = { state: "corrupt", stage: "failed", detail: migrationError, legacyConflicts: [] };
      }
    }
  }

  async function runMigration(retry: boolean) {
    migrationBusy = true;
    migrationError = null;
    if (migrationStatus) migrationStatus = { ...migrationStatus, stage: "verifyingBackup" };
    try {
      migrationStatus = retry ? await storageMigrationRetry() : await storageMigrationStart();
      showMigrationCompletion = migrationStatus.state === "complete";
      visibleRevision = await storageVisibleRevision();
    } catch (error) {
      migrationError = isAppError(error) ? appErrorMessage(error) : String(error);
      await loadMigrationStatus();
    } finally {
      migrationBusy = false;
    }
  }

  async function refreshVisibleSurface() {
    if (ui.section === "skills") await skillSources.load();
    else if (ui.section === "personas") await agentLibrary.load(true);
    else if (ui.section === "experts") await experts.load();
  }

  async function dismissLegacyConflicts() {
    try {
      await storageLegacyConflictsDismiss();
      if (migrationStatus) migrationStatus = { ...migrationStatus, legacyConflicts: [] };
    } catch {
      // Keep the durable warning visible when acknowledgement fails.
    }
  }

  async function refreshIfChanged() {
    try {
      if (document.visibilityState !== "visible" || migrationStatus?.state !== "complete") return;
      const revision = await storageVisibleRevision();
      if (revision === visibleRevision) return;
      visibleRevision = revision;
      await activity.refreshFactoryReceipts();
      await refreshVisibleSurface();
    } catch {
      // A later focus/poll retries; foreground work must not be interrupted.
    }
  }

  function themeLabel(t: ThemePreference): string {
    return i18n.t(t === "light" ? "app.theme.light" : t === "dark" ? "app.theme.dark" : "app.theme.system");
  }

  function sectionTitle(s: SidebarSection): string {
    if (s === "dashboard") return i18n.t("nav.dashboard");
    if (s === "personas") return i18n.t("nav.agents");
    if (s === "skills") return "Skills";
    if (s === "tools") return i18n.t("nav.tools");
    if (s === "teams") return i18n.t("nav.teams");
    if (s === "projects") return i18n.t("nav.projects");
    if (s === "experts") return i18n.t("nav.experts");
    return i18n.t("nav.activity");
  }

  function navigateSection(section: SidebarSection) {
    ui.setSection(section);
    if (narrowViewport) mobileSidebarOpen = false;
  }

  function toggleSidebar() {
    if (narrowViewport) mobileSidebarOpen = !mobileSidebarOpen;
    else ui.toggleSidebarCollapsed();
  }

  function isTextInput(el: EventTarget | null): boolean {
    if (!(el instanceof HTMLElement)) return false;
    return el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable;
  }

  function onKeydown(e: KeyboardEvent) {
    const meta = e.metaKey || e.ctrlKey;

    // Cmd+K: open palette (always)
    if (meta && e.key.toLowerCase() === "k") {
      e.preventDefault();
      ui.openPalette();
      return;
    }

    // Cmd+, : open Settings (macOS preferences convention)
    if (meta && e.key === ",") {
      e.preventDefault();
      ui.openSettings();
      return;
    }

    // Cmd+Shift+L: cycle theme (must precede Cmd+L)
    if (meta && e.shiftKey && e.key.toLowerCase() === "l") {
      e.preventDefault();
      const order = ["light", "dark", "system"] as const;
      const next = order[(order.indexOf(ui.theme) + 1) % order.length];
      ui.setTheme(next);
      toast.info(i18n.t("toast.theme", { theme: themeLabel(next) }));
      return;
    }

    // Cmd+L (no shift): toggle drawer
    if (meta && !e.shiftKey && e.key.toLowerCase() === "l") {
      e.preventDefault();
      ui.toggleDrawer();
      return;
    }

    // Cmd+[ / Cmd+] : back / forward through nav history (browser convention).
    if (meta && e.key === "[") { e.preventDefault(); ui.back(); return; }
    if (meta && e.key === "]") { e.preventDefault(); ui.forward(); return; }

    // Cmd+0..7: agency section nav. Matches the sidebar.
    if (meta && ["0","1","2","3","4","5","6","7"].includes(e.key)) {
      e.preventDefault();
      const map: Record<string, SidebarSection> = {
        "0": "dashboard",
        "1": "personas",
        "2": "skills",
        "3": "tools",
        "4": "teams",
        "5": "projects",
        "6": "experts",
        "7": "activity",
      };
      navigateSection(map[e.key]);
      return;
    }

    // Esc: priority: settings (handles its own Esc) → palette.
    // Settings.svelte handles its own Esc listener with stopPropagation, so
    // we never reach this branch while Settings is open. Belt-and-suspenders
    // gate kept to make the priority intent obvious.
    if (e.key === "Escape") {
      if (ui.settingsOpen) return; // Settings handles its own Esc
      if (ui.paletteOpen) { ui.closePalette(); return; }
    }

    // "/": focus the in-view filter input (unless typing)
    if (e.key === "/" && !isTextInput(e.target)) {
      const input = document.querySelector<HTMLInputElement>('input[type="text"], input[type="search"], input:not([type])');
      if (input) {
        e.preventDefault();
        input.focus();
      }
    }
  }

  // Mouse back/forward (buttons 3/4) — the dedicated nav buttons on many mice.
  function onMouseUp(e: MouseEvent) {
    if (e.button === 3) { e.preventDefault(); ui.back(); }
    else if (e.button === 4) { e.preventDefault(); ui.forward(); }
  }

  onMount(() => {
    void loadMigrationStatus();
    const narrowMedia = window.matchMedia("(max-width: 600px)");
    const syncViewport = () => {
      narrowViewport = narrowMedia.matches;
      if (!narrowViewport) mobileSidebarOpen = false;
    };
    syncViewport();
    narrowMedia.addEventListener("change", syncViewport);
    window.addEventListener("keydown", onKeydown);
    window.addEventListener("mouseup", onMouseUp);
    window.addEventListener("focus", refreshIfChanged);
    const revisionPoll = window.setInterval(() => void refreshIfChanged(), 750);
    return () => {
      window.removeEventListener("keydown", onKeydown);
      narrowMedia.removeEventListener("change", syncViewport);
      window.removeEventListener("mouseup", onMouseUp);
      window.removeEventListener("focus", refreshIfChanged);
      window.clearInterval(revisionPoll);
    };
  });
</script>

{#if migrationStatus === null}
  <div class="storage-check"><LoadingState label="Checking data storage" /></div>
{:else}
<div
  class="app"
  class:macos={isMac}
  class:sidebar-collapsed={sidebarCollapsed}
  style="--sidebar-width: {ui.sidebarWidth}px"
>
  <!--
    Window title bar. Spans the full width above the main split so it
    reads as one unified chrome (the Mac unified-toolbar pattern).
    Layout:
      • macOS-rendered traffic lights overlay the far left (~80 px)
      • Sidebar toggle sits just inside the sidebar's right edge when
        expanded; when collapsed, it slides over next to the traffic
        lights (the sidebar can't fit a button at 56 px wide).
      • Page title sits just past where the sidebar divider lands so it
        aligns with the main content column.
    Both the toggle and the title slide via CSS custom properties driven
    by the `.sidebar-collapsed` class on `.app`, so transitions are smooth.
  -->
  <header class="titlebar" data-tauri-drag-region>
    <button
      type="button"
      class="titlebar-btn"
      data-tauri-drag-region="false"
      title={sidebarCollapsed ? i18n.t("titlebar.showSidebar") : i18n.t("titlebar.hideSidebar")}
      aria-label={sidebarCollapsed ? i18n.t("titlebar.showSidebar") : i18n.t("titlebar.hideSidebar")}
      aria-pressed={sidebarCollapsed}
      onclick={toggleSidebar}
    >
      {#if sidebarCollapsed}
        <PanelLeftOpen size={16} />
      {:else}
        <PanelLeftClose size={16} />
      {/if}
    </button>
    <div class="titlebar-nav" data-tauri-drag-region="false">
      <button
        type="button"
        class="titlebar-btn nav"
        title={`${i18n.t("titlebar.back")} (${shortcut("[")})`}
        aria-label={i18n.t("titlebar.back")}
        disabled={!ui.canBack}
        onclick={() => ui.back()}
      >
        <ArrowLeft size={16} />
      </button>
      <button
        type="button"
        class="titlebar-btn nav"
        title={`${i18n.t("titlebar.forward")} (${shortcut("]")})`}
        aria-label={i18n.t("titlebar.forward")}
        disabled={!ui.canForward}
        onclick={() => ui.forward()}
      >
        <ArrowRight size={16} />
      </button>
    </div>
    <h1 class="titlebar-title">{sectionTitle(ui.section)}</h1>
    <div class="titlebar-right">
      <UpdateIndicator />
      <TitlebarControls />
    </div>
  </header>
  <div class="main">
     <Sidebar collapsed={sidebarCollapsed} onNavigate={navigateSection} />
     {#if !sidebarCollapsed}
       <div class="sidebar-resize-region" role="region" aria-label="Sidebar resize control">
         <ResizeHandle
           width={ui.sidebarWidth}
           min={SIDEBAR_MIN_WIDTH}
           max={SIDEBAR_MAX_WIDTH}
           defaultWidth={SIDEBAR_DEFAULT_WIDTH}
           direction="right"
           label={i18n.t("titlebar.resizeSidebar")}
           onChange={(w) => (ui.sidebarWidth = w)}
           onCommit={(w) => ui.setSidebarWidth(w)}
         />
       </div>
    {/if}
    <main class="content">
        <div class="section-pane">
          {#if ui.section === "dashboard"}
            {#await import("$lib/components/AgencyDashboard.svelte")}
              <LoadingState />
            {:then { default: AgencyDashboard }}
              <AgencyDashboard />
            {:catch}
              <p role="alert">Could not load Dashboard.</p>
            {/await}
          {:else if ui.section === "skills"}
            {#await import("$lib/components/SkillsWorkspace.svelte")}
              <LoadingState />
            {:then { default: SkillsWorkspace }}
              <SkillsWorkspace />
            {:catch}
              <p role="alert">Could not load Skills.</p>
            {/await}
          {:else if ui.section === "tools"}
            {#await import("$lib/components/ToolsView.svelte")}
              <LoadingState />
            {:then { default: ToolsView }}
              <ToolsView />
            {:catch}
              <p role="alert">Could not load Tools.</p>
            {/await}
          {:else if ui.section === "teams"}
            {#await import("$lib/components/Teams.svelte")}
              <LoadingState />
            {:then { default: Teams }}
              <Teams />
            {:catch}
              <p role="alert">Could not load Teams.</p>
            {/await}
          {:else if ui.section === "projects"}
            {#await import("$lib/components/Projects.svelte")}
              <LoadingState />
            {:then { default: Projects }}
              <Projects />
            {:catch}
              <p role="alert">Could not load Projects.</p>
            {/await}
          {:else if ui.section === "personas"}
            {#await import("$lib/components/AgentsWorkspace.svelte")}
              <LoadingState />
            {:then { default: AgentsWorkspace }}
              <AgentsWorkspace />
            {:catch}
              <p role="alert">Could not load Personas.</p>
            {/await}
          {:else if ui.section === "experts"}
            <Experts />
          {:else if ui.section === "activity"}
            {#await import("$lib/components/ActivityHistory.svelte")}
              <LoadingState />
            {:then { default: ActivityHistory }}
              <ActivityHistory />
            {:catch}
              <p role="alert">Could not load Activity.</p>
            {/await}
          {/if}
        </div>
    </main>
  </div>
  <CommandPalette />
  <Settings />
  <AboutModal />
  <PlaybookModal />
  <DeviceFlowModal />
  <Toast />
  {#if migrationStatus.legacyConflicts.length > 0}
    <aside class="legacy-warning" role="status">
      Legacy data files changed after the update and were not merged.
      <button type="button" onclick={() => void storageOpenDataDirectory()}>Open data folder</button>
      <button type="button" onclick={() => void dismissLegacyConflicts()}>Dismiss</button>
    </aside>
  {/if}
  {#if needsMigrationGate}
    <StorageMigrationGate
      status={migrationStatus}
      busy={migrationBusy}
      error={migrationError}
      onStart={() => void runMigration(false)}
      onRetry={() => void runMigration(true)}
      onOpenData={() => void storageOpenDataDirectory()}
    />
  {/if}
</div>
{/if}

<style>
  .storage-check { position: fixed; inset: 0; display: grid; place-items: center; background: var(--color-surface); }
  .legacy-warning {
    position: fixed;
    right: var(--space-4);
    bottom: var(--space-4);
    z-index: 900;
    max-width: 440px;
    padding: var(--space-3);
    border: 1px solid var(--color-warning);
    border-radius: var(--radius-md);
    background: var(--color-surface-raised);
    color: var(--color-text-primary);
  }
  .legacy-warning button { margin-left: var(--space-2); color: var(--color-text-link); }
  .app {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--color-surface);
    /* Title bar layout knobs — track the live sidebar width so the toggle
       and page title stay aligned with the (resizable) sidebar's right edge.
       `--sidebar-width` is set inline from `ui.sidebarWidth`. */
    --titlebar-toggle-left: calc(var(--sidebar-width, 200px) - 32px);
    --titlebar-title-left: calc(var(--sidebar-width, 200px) + 20px);
  }
  .app.sidebar-collapsed {
    --titlebar-toggle-left: 12px;
    --titlebar-title-left: 52px;
  }
  .app.macos.sidebar-collapsed {
    --titlebar-toggle-left: 84px;   /* just past the traffic lights */
    --titlebar-title-left: 124px;   /* just past the toggle */
  }
  /* Window-level title bar. Same chrome color as the sidebar so the
     two read as one continuous L-shaped frame around the main content.
     Height tuned with `trafficLightPosition` (tauri.conf.json) so the
     macOS-rendered traffic lights end up vertically centered on the
     same horizontal axis as the toggle and the page title. */
  .titlebar {
    flex: none;
    height: 36px;
    position: relative;
    background: var(--color-surface-raised);
    border-bottom: 1px solid var(--color-border);
  }
  /* Toggle slides between two positions via CSS variables. */
  .titlebar-btn {
    position: absolute;
    top: 50%;
    left: var(--titlebar-toggle-left);
    transform: translateY(-50%);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    background: transparent;
    border-radius: var(--radius-md);
    color: var(--color-text-muted);
    cursor: pointer;
    transition: left var(--motion-duration-base, 180ms) var(--motion-ease-out, ease),
                background-color var(--motion-duration-fast) var(--motion-ease-out),
                color var(--motion-duration-fast) var(--motion-ease-out);
  }
  .titlebar-btn:hover {
    background: var(--color-surface-sunken);
    color: var(--color-text-primary);
  }
  .titlebar-btn:focus-visible {
    outline: 2px solid var(--color-focus, var(--color-brand));
    outline-offset: 2px;
  }
  /* Back/forward cluster — sits just left of the page title and slides with it. */
  .titlebar-nav {
    position: absolute;
    top: 50%;
    left: var(--titlebar-title-left);
    transform: translateY(-50%);
    display: inline-flex;
    align-items: center;
    gap: 2px;
    transition: left var(--motion-duration-base, 180ms) var(--motion-ease-out, ease);
  }
  /* Inside the flex cluster these are in-flow, not absolutely placed. */
  .titlebar-btn.nav {
    position: static;
    left: auto;
    transform: none;
    transition: background-color var(--motion-duration-fast) var(--motion-ease-out),
                color var(--motion-duration-fast) var(--motion-ease-out);
  }
  .titlebar-btn:disabled {
    opacity: 0.32;
    cursor: default;
  }
  .titlebar-btn:disabled:hover {
    background: transparent;
    color: var(--color-text-muted);
  }
  /* Page title also slides so it stays aligned with the start of the
     main content column (just past the sidebar divider). */
  .titlebar-title {
    position: absolute;
    top: 50%;
    /* Shifted right of the back/forward cluster (≈62px wide). */
    left: calc(var(--titlebar-title-left) + 62px);
    transform: translateY(-50%);
    margin: 0;
    font-size: var(--text-h3);
    font-weight: var(--fw-semibold);
    color: var(--color-text-primary);
    white-space: nowrap;
    transition: left var(--motion-duration-base, 180ms) var(--motion-ease-out, ease);
    /* Don't intercept the draggable region: clicks on the title still
       let the user drag the window. */
    pointer-events: none;
  }
  @media (prefers-reduced-motion: reduce) {
    .titlebar-btn, .titlebar-title { transition: none; }
  }
  /* Right-side button cluster — theme dropdown + Settings + Donate.
     Now that the title bar's right half is otherwise empty, this is
     the natural Mac spot for app controls (Mail's right-side toolbar
     uses the same alignment). Nudged 1 px below center to align
     optically with the macOS traffic lights on the left. */
  .titlebar-right {
    position: absolute;
    top: 50%;
    /* Align the cluster's right edge with the main panel's content
       right edge. Panel-head and body both use var(--space-4) of
       horizontal padding, so matching it here lines everything up. */
    right: var(--space-4);
    transform: translateY(calc(-50% + 1px));
    display: flex;
    align-items: center;
    /* 8 px between the optional UpdateIndicator pill and the
       TitlebarControls cluster (Phase 15). The indicator hides
       entirely when there's no update available, so the gap is
       absorbed by the layout — no empty space when nothing to show. */
    gap: 8px;
  }
  .main {
    flex: 1;
    display: flex;
    min-height: 0;
    overflow: hidden;
  }
  .content {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    background: var(--color-surface);
    overflow: hidden;
  }
  .sidebar-resize-region { display: flex; flex: none; align-self: stretch; }
  /* Quiet crossfade when switching sidebar sections.
     Tabs are peers, so we fade content rather than slide (designSystem §6). */
  .section-pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    animation: section-in var(--motion-duration-base) var(--motion-ease-out);
  }
  @keyframes section-in {
    from { opacity: 0; }
    to   { opacity: 1; }
  }
  @media (prefers-reduced-motion: reduce) {
    .section-pane { animation: none; }
  }
  @media (max-width: 600px) {
    .app {
      --titlebar-toggle-left: 12px;
      --titlebar-title-left: 52px;
    }
    .app.macos {
      --titlebar-toggle-left: 84px;
      --titlebar-title-left: 124px;
    }
    .titlebar-nav { display: none; }
    .titlebar-title {
      width: 1px; height: 1px; padding: 0; margin: -1px;
      overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border: 0;
    }
    .titlebar-right { right: var(--space-2); }
    .main :global(.handle) { display: none; }
    .main :global(.sidebar) {
      position: fixed;
      inset: 36px auto 0 0;
      z-index: 42;
      max-width: min(85vw, 280px);
      box-shadow: var(--shadow-lg);
    }
    .app.sidebar-collapsed .main :global(.sidebar) { display: none; }
  }
</style>
