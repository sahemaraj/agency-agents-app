<script lang="ts">
  import "../app.css";
  import { onMount } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import type { PluginListener } from "@tauri-apps/api/core";
  import {
    isPermissionGranted,
    onAction,
    sendNotification,
  } from "@tauri-apps/plugin-notification";
  import { ui, watchSystemTheme } from "$lib/stores/ui.svelte";
  import { install } from "$lib/stores/install.svelte";
  import { skillSources } from "$lib/stores/skillSources.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { activity } from "$lib/stores/activity.svelte";
  import { settings } from "$lib/stores/settings.svelte";
  import { catalog } from "$lib/stores/catalog.svelte";
  import { i18n } from "$lib/stores/i18n.svelte";
  import CatalogFirstRun from "$lib/components/CatalogFirstRun.svelte";
  import {
    FIRST_DEPLOYMENT_COMPLETION,
    FIRST_DEPLOYMENT_STORAGE_KEY,
    shouldShowFirstDeployment,
  } from "$lib/firstDeployment";

  let { children } = $props();
  let firstDeploymentCompletion = $state<string | null>(null);
  let firstDeploymentActive = $state(false);
  type DriftKind = "agents" | "skills";
  let driftBaseline: Map<string, DriftKind> | null = null;
  const DRIFT_RECONCILE_INTERVAL_MS = 15 * 60 * 1000;
  const managedInstallCount = $derived(
    install.installed.filter((row) => row.tracked && row.state !== "missing").length,
  );
  const showFirstDeployment = $derived(shouldShowFirstDeployment({
    catalogLoaded: catalog.loaded,
    catalogConfigured: catalog.configured,
    completion: firstDeploymentCompletion,
    reconciled: install.reconciled,
    reconcileError: install.reconcileError,
    managedInstallCount,
  }));
  $effect(() => {
    if (showFirstDeployment) firstDeploymentActive = true;
  });

  function finishFirstDeployment() {
    firstDeploymentCompletion = FIRST_DEPLOYMENT_COMPLETION;
    firstDeploymentActive = false;
  }

  function projectPaths(): string[] {
    return projects.list.map((project) => project.path);
  }

  function driftSnapshot(): Map<string, DriftKind> {
    const snapshot = new Map<string, DriftKind>();
    for (const row of install.installed) {
      if (!row.tracked || !["outdated", "modified", "missing"].includes(row.state)) continue;
      snapshot.set(JSON.stringify(["agent", row.sourceId, row.relativePath, row.tool, row.projectPath]), "agents");
    }
    for (const row of skillSources.installed) {
      if (!row.tracked || !["outdated", "modified", "missing"].includes(row.state)) continue;
      snapshot.set(JSON.stringify(["skill", row.sourceId, row.relativePath, row.runtime, row.projectPath]), "skills");
    }
    return snapshot;
  }

  function rememberCurrentDrift(): void {
    if (!install.reconcileError && !skillSources.reconcileError) driftBaseline = driftSnapshot();
  }

  async function reconcileInstallationTruth(): Promise<boolean> {
    await Promise.all([
      install.reconcile(),
      skillSources.reconcileInstalls(projectPaths()),
    ]);
    return !install.reconcileError && !skillSources.reconcileError;
  }

  async function reconcileVisibleTruth(): Promise<void> {
    if (await reconcileInstallationTruth()) rememberCurrentDrift();
  }

  async function reconcileBackgroundDrift(): Promise<void> {
    if (!settings.effective.driftNotifications || document.visibilityState !== "hidden") return;
    if (!await reconcileInstallationTruth()) return;
    const next = driftSnapshot();
    if (driftBaseline === null) {
      driftBaseline = next;
      return;
    }
    const added = [...next].filter(([identity]) => !driftBaseline?.has(identity));
    driftBaseline = next;
    if (added.length === 0) return;
    try {
      if (!await isPermissionGranted()) return;
      const agents = added.filter(([, kind]) => kind === "agents").length;
      const skills = added.length - agents;
      const counts = [
        agents > 0 ? `${agents} Agent${agents === 1 ? "" : "s"}` : null,
        skills > 0 ? `${skills} Skill${skills === 1 ? "" : "s"}` : null,
      ].filter(Boolean).join(" and ");
      sendNotification({
        title: agents > 0 ? "Agent drift needs review" : "Skill drift needs review",
        body: `${counts} newly need attention. Open Shikigami to review.`,
        extra: { review: agents > 0 ? "agents" : "skills" },
      });
    } catch {
      // Permission may be revoked while the app is backgrounded; stay quiet.
    }
  }

  onMount(() => {
    i18n.init();
    firstDeploymentCompletion = localStorage.getItem(FIRST_DEPLOYMENT_STORAGE_KEY);
    ui.loadThemeFromStorage();
    // Settings (Phase 12b) — all read with enum/numeric validation so a
    // corrupt or hostile localStorage entry can't poison runtime state.
    ui.loadDefaultSectionFromStorage();
    ui.loadVibrancyMaterialFromStorage();
    ui.loadConfirmDestructiveFromStorage();
    ui.loadActivitySettingsFromStorage();
    ui.loadSidebarCollapsedFromStorage();
    ui.loadSidebarWidthFromStorage();
    ui.loadDetailPaneWidthFromStorage();
    // Seed back/forward history with the landing location (after the default
    // section has been resolved above), so the first entry is real.
    ui.initNav();
    activity.hydrate();
    // Install state — reconcile ONCE here at the app root, not inside the view
    // components. A view that both reads install.* state AND triggers a mutation
    // (reconcile) during its own setup froze its reactivity (Library bug). Views
    // are now pure readers; Rescan buttons re-trigger on user action.
    void reconcileVisibleTruth();
    const initialProjectPaths = projectPaths();
    void projects.refresh().then(() => {
      const refreshedProjectPaths = projectPaths();
      if (refreshedProjectPaths.join("\0") !== initialProjectPaths.join("\0")) {
        void skillSources.reconcileInstalls(refreshedProjectPaths).then(rememberCurrentDrift);
      }
    }).catch(() => {
      // Existing install state remains usable; a later view or focus retries.
    });
    void install.loadTools();
    install.loadSelection();
    // Phase 12d — hydrate the persisted settings.json into the renderer
    // so the Network section, the Catalog stale banner, and the cask
    // icon mode all read from one source of truth.
    void settings.load();
    // Catalog source (#1) — load the persisted choice; if none has been made
    // the first-run picker renders over the app until the user chooses.
    void catalog.load();
    // NOTE: GitHub sign-in status is intentionally NOT hydrated here.
    // `github.loadStatus()` reads from macOS Keychain, which prompts
    // the user the first time a new app binary tries to access an
    // existing entry. Probing on every app launch trains users to
    // dismiss the prompt without reading it, and is intrusive when
    // they have no intention of using GitHub features.
    // Instead: probe lazily — `requireGithubSignIn()` (in PackageDetail)
    // awaits loadStatus on first action click, and Settings → GitHub
    // calls loadStatus when its panel mounts. Both contexts are
    // user-initiated, so a Keychain prompt is contextual and expected.

    // Native macOS menu bridge — Rust emits `menu:about` / `menu:settings`
    // when the user picks those items from the App menu in the system menu
    // bar; we just open the corresponding modal. The Cmd+, accelerator is
    // also bound on the Settings menu item so both surfaces stay in sync
    // with the in-app shortcut already handled in `+page.svelte`.
    let unlistenAbout: UnlistenFn | undefined;
    let unlistenSettings: UnlistenFn | undefined;
    let notificationActionListener: PluginListener | undefined;
    let disposed = false;
    let foregroundTimer: ReturnType<typeof setTimeout> | undefined;
    const scheduleForegroundReconcile = () => {
      if (foregroundTimer) clearTimeout(foregroundTimer);
      foregroundTimer = setTimeout(() => void reconcileVisibleTruth(), 250);
    };
    const driftTimer = setInterval(() => void reconcileBackgroundDrift(), DRIFT_RECONCILE_INTERVAL_MS);
    window.addEventListener("focus", scheduleForegroundReconcile);
    void listen("menu:about", () => { ui.openAbout(); }).then((u) => { unlistenAbout = u; });
    void listen("menu:settings", () => { ui.openSettings(); }).then((u) => { unlistenSettings = u; });
    void onAction((notification) => {
      const review = notification.extra?.review;
      if (review === "agents") ui.openAgents(null, "attention");
      else if (review === "skills") ui.setSection("skills");
    }).then((listener) => {
      if (disposed) void listener.unregister();
      else notificationActionListener = listener;
    });

    const unwatch = watchSystemTheme(() => ui.theme);
    return () => {
      unwatch();
      disposed = true;
      window.removeEventListener("focus", scheduleForegroundReconcile);
      if (foregroundTimer) clearTimeout(foregroundTimer);
      clearInterval(driftTimer);
      unlistenAbout?.();
      unlistenSettings?.();
      void notificationActionListener?.unregister();
    };
  });
</script>

<!--
  Window dragging in Tauri 2 with titleBarStyle: "Overlay" is wired via the
  `data-tauri-drag-region` attribute on regular DOM elements (Sidebar brand
  area + each panel-head). Tauri's WebView handles click-vs-drag detection
  natively, so interactive children inside drag regions still receive their
  clicks. Avoids the fixed-overlay pattern (which intercepts scroll-wheel
  events at the top of the window).
-->

{@render children()}

{#if firstDeploymentActive}
  <CatalogFirstRun onFinish={finishFirstDeployment} />
{/if}
