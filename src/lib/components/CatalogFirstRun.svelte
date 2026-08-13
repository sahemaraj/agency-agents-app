<script lang="ts">
  /**
   * CatalogFirstRun.svelte — first-launch catalog-source picker (#1).
   *
   * Shown once, before anything else, when the user hasn't chosen where the
   * agent catalog lives (`catalog.configured === false`). Three paths:
   *   1. Use my own clone  — detect/Find + folder picker (manage-with-permission)
   *   2. Set one up for me  — provision ~/.agency-agents (git clone or snapshot)
   *   3. Bundled snapshot   — the always-works default
   *
   * Posture: nothing is written until the user picks. Choosing any option
   * persists the choice (configured → true), which dismisses this modal.
   */
  import { onMount, tick } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import FolderGit2 from "@lucide/svelte/icons/folder-git-2";
  import Sparkles from "@lucide/svelte/icons/sparkles";
  import Package from "@lucide/svelte/icons/package";
  import Search from "@lucide/svelte/icons/search";
  import Check from "@lucide/svelte/icons/check";
  import Bot from "@lucide/svelte/icons/bot";

  import { catalog } from "$lib/stores/catalog.svelte";
  import { corpus } from "$lib/stores/corpus.svelte";
  import { install, SUPPORTED_TOOLS } from "$lib/stores/install.svelte";
  import { agentLibrary } from "$lib/stores/agentLibrary.svelte";
  import { i18n } from "$lib/stores/i18n.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import InstallModal from "./InstallModal.svelte";
  import StarterPrompt from "./StarterPrompt.svelte";
  import { PRESET_TEAMS } from "$lib/data/presetTeams";
  import { TEAM_EXAMPLES } from "$lib/data/playbook";
  import {
    FIRST_DEPLOYMENT_COMPLETION,
    FIRST_DEPLOYMENT_STORAGE_KEY,
    defaultFirstDeploymentTool,
    recommendFirstDeploymentPreset,
  } from "$lib/firstDeployment";
  import {
    appErrorMessage,
    isAppError,
    type AgentMutationPlan,
    type AgentReference,
    type CatalogCandidate,
  } from "$lib/types";

  interface Props { onFinish?: () => void }
  let { onFinish = () => {} }: Props = $props();

  let expanded = $state<"clone" | null>(null);
  let manage = $state(true);
  let stage = $state<"catalog" | "prepare" | "success">(catalog.configured ? "prepare" : "catalog");
  let installOpen = $state(false);
  let guideError = $state<string | null>(null);
  let destinations = $state<string[]>([]);
  let dialog: HTMLDivElement;

  const recommendation = $derived(
    recommendFirstDeploymentPreset(new Set(corpus.agents.map((agent) => agent.slug)), PRESET_TEAMS),
  );
  const references = $derived<AgentReference[]>(recommendation
    ? recommendation.agents.flatMap((slug) => {
        const pkg = agentLibrary.packages.find((candidate) =>
          candidate.reference.sourceId === "builtin:agency-agents"
          && candidate.installable
          && candidate.agent?.slug === slug
        );
        return pkg ? [pkg.reference] : [];
      })
    : []);
  const target = $derived(defaultFirstDeploymentTool(install.tools));
  const targetInfo = $derived(install.tools.find((tool) => tool.tool === target) ?? null);
  const supportedTargets = $derived(SUPPORTED_TOOLS
    .filter((tool) => tool.id === "claudeCode" || tool.id === "codex")
    .map((tool) => ({
      ...tool,
      detected: install.tools.some((info) => info.tool === tool.id && info.detected),
    })));
  const ready = $derived(
    !!recommendation
    && references.length === recommendation.agents.length
    && !!target
    && install.reconciled
    && !install.reconciling
    && !install.reconcileError,
  );

  $effect(() => {
    stage;
    void tick().then(() => dialog?.focus());
  });

  onMount(() => {
    if (stage === "catalog") void catalog.detect(false);
    else {
      void corpus.ensureLoaded();
      void agentLibrary.load();
    }
    void install.loadTools();
  });

  async function choose(fn: () => Promise<unknown>, ok: string) {
    try {
      await fn();
      await agentLibrary.load(true);
      stage = "prepare";
      toast.success(ok);
    } catch (e) {
      toast.error(i18n.t("firstRun.error"), isAppError(e) ? appErrorMessage(e) : String(e));
    }
  }

  async function pickFolder() {
    const picked = await openDialog({ directory: true, multiple: false, title: i18n.t("catalog.chooseCloneTitle") });
    if (typeof picked === "string") {
      await choose(() => catalog.useClone(picked, manage), i18n.t("firstRun.usingClone"));
    }
  }

  function useCandidate(c: CatalogCandidate) {
    void choose(() => catalog.useClone(c.path, manage), i18n.t("firstRun.usingPath", { path: c.path }));
  }

  function complete() {
    localStorage.setItem(FIRST_DEPLOYMENT_STORAGE_KEY, FIRST_DEPLOYMENT_COMPLETION);
    onFinish();
  }

  function deployed(plan: AgentMutationPlan) {
    const present = plan.agents.every((item) => install.installed.some((row) =>
      row.tracked
      && row.state === "current"
      && row.sourceId === item.reference.sourceId
      && row.relativePath === item.reference.relativePath
      && row.tool === plan.tool
      && row.projectPath === plan.projectPath
    ));
    if (!present || install.reconcileError) {
      guideError = install.reconcileError ?? i18n.t("firstRun.verifyFailed");
      installOpen = false;
      return;
    }
    destinations = plan.agents.map((item) => item.destination);
    installOpen = false;
    stage = "success";
  }
</script>

<div class="scrim">
  <div class="box" bind:this={dialog} tabindex="-1" role="dialog" aria-modal="true" aria-label={i18n.t("firstRun.dialogAria")}>
    <p class="sr-only" aria-live="polite">{i18n.t(`firstRun.stage.${stage}`)}</p>
    {#if stage === "catalog"}
    <header>
      <h1>{i18n.t("firstRun.title")}</h1>
      <p class="lede">{i18n.t("firstRun.lede")}</p>
    </header>

    <div class="cards">
      <!-- 1. Use my own clone -->
      <div class="card" class:open={expanded === "clone"}>
        <button class="card-head" onclick={() => (expanded = expanded === "clone" ? null : "clone")}>
          <FolderGit2 size={22} />
          <div class="ct">
            <span class="t">{i18n.t("firstRun.useClone")}</span>
            <span class="d">{i18n.t("firstRun.useCloneDesc")}</span>
          </div>
        </button>

        {#if expanded === "clone"}
          <div class="card-body">
            {#if catalog.detection?.candidates.length}
              <ul class="cands">
                {#each catalog.detection.candidates as c (c.path)}
                  <li>
                    <button class="cand" disabled={catalog.busy} onclick={() => useCandidate(c)}>
                      <div class="cand-main">
                        <span class="cand-path">{c.path}</span>
                        <span class="cand-meta">
                          {i18n.count(c.agentCount, "common.agent.one", "common.agent.many")}{c.hasGit ? " · git" : ""} · {c.kind === "managed" ? "~/.agency-agents" : i18n.t("common.detected")}
                        </span>
                      </div>
                      <Check size={15} />
                    </button>
                  </li>
                {/each}
              </ul>
            {:else}
              <p class="empty">{i18n.t("firstRun.noClone")}</p>
            {/if}

            <label class="manage">
              <input type="checkbox" bind:checked={manage} />
              {i18n.t("firstRun.manage")}
            </label>

            <div class="row-actions">
              <button class="ghost" disabled={catalog.scanning} onclick={() => catalog.detect(true)}>
                <Search size={14} /><span>{catalog.scanning ? i18n.t("common.searching") : i18n.t("catalog.find")}</span>
              </button>
              <button class="ghost" disabled={catalog.busy} onclick={pickFolder}>{i18n.t("catalog.chooseFolder")}</button>
            </div>
          </div>
        {/if}
      </div>

      <!-- 2. Set one up for me -->
      <button class="card simple" disabled={catalog.busy} onclick={() => choose(() => catalog.provisionManaged(), i18n.t("catalog.setupManaged"))}>
        <Sparkles size={22} />
        <div class="ct">
          <span class="t">{i18n.t("firstRun.setup")}</span>
          <span class="d">{i18n.t("firstRun.setupDesc")}</span>
        </div>
      </button>

      <!-- 3. Bundled snapshot -->
      <button class="card simple" disabled={catalog.busy} onclick={() => choose(() => catalog.useBundled(), i18n.t("catalog.usingBundled"))}>
        <Package size={22} />
        <div class="ct">
          <span class="t">{i18n.t("firstRun.bundled")}</span>
          <span class="d">{i18n.t("firstRun.bundledDesc")}</span>
        </div>
      </button>
    </div>

    {#if catalog.error}<p class="err">{catalog.error}</p>{/if}
    {:else if stage === "prepare"}
      <header>
        <h1>{i18n.t("firstRun.deployTitle")}</h1>
        <p class="lede">{i18n.t("firstRun.deployLede")}</p>
      </header>

      <div class="guide-facts">
        <div class="fact">
          <Bot size={20} />
          <div class="ct">
            <span class="t">{targetInfo?.label ?? i18n.t("firstRun.noTarget")}</span>
            <span class="d">{targetInfo ? i18n.t("firstRun.detectedTarget") : i18n.t("firstRun.noTargetHelp")}</span>
            <span class="target-states">
              {#each supportedTargets as candidate}
                <span class:available={candidate.detected}>
                  {candidate.label}: {i18n.t(candidate.detected ? "common.detected" : "firstRun.unavailable")}
                </span>
              {/each}
            </span>
          </div>
        </div>
        <div class="fact">
          {#if recommendation}
            {@const RecommendedIcon = recommendation.icon}
            <RecommendedIcon size={20} color={recommendation.color} />
          {:else}
            <Package size={20} />
          {/if}
          <div class="ct">
            <span class="t">{recommendation?.label ?? i18n.t("firstRun.noPreset")}</span>
            <span class="d">{recommendation?.description ?? i18n.t("firstRun.noPresetHelp")}</span>
          </div>
        </div>
      </div>

      {#if recommendation && references.length !== recommendation.agents.length}
        <p class="err" role="alert">{i18n.t("firstRun.presetUnavailable")}</p>
      {/if}
      {#if install.reconcileError}<p class="err" role="alert">{install.reconcileError}</p>{/if}
      {#if guideError}<p class="err" role="alert">{guideError}</p>{/if}

      <div class="guide-actions">
        <button class="ghost" onclick={complete}>{i18n.t("firstRun.later")}</button>
        <button class="primary" disabled={!ready} onclick={() => (installOpen = true)}>
          {i18n.t("firstRun.reviewDeployment")}
        </button>
      </div>
    {:else}
      <header>
        <h1>{i18n.t("firstRun.successTitle")}</h1>
        <p class="lede">{i18n.t("firstRun.successLede")}</p>
      </header>
      <ul class="destinations">
        {#each destinations as destination}<li><code>{destination}</code></li>{/each}
      </ul>
      {#if recommendation}
        <StarterPrompt
          label={i18n.t("firstRun.starterPrompt")}
          template={TEAM_EXAMPLES[recommendation.slug]?.[0] ?? ""}
        />
      {/if}
      <div class="guide-actions"><button class="primary" onclick={complete}>{i18n.t("common.done")}</button></div>
    {/if}
  </div>
</div>

{#if installOpen && recommendation}
  <InstallModal
    title={i18n.t("firstRun.reviewTitle", { team: recommendation.label })}
    agentReferences={references}
    allowedTools={["claudeCode", "codex"]}
    onClose={() => (installOpen = false)}
    onApplied={deployed}
  />
{/if}

<style>
  .scrim {
    position: fixed; inset: 36px 0 0 0; z-index: 40;
    display: flex; align-items: center; justify-content: center;
    background: color-mix(in srgb, var(--color-bg) 70%, transparent);
    backdrop-filter: blur(6px);
  }
  .box {
    width: min(560px, 92vw); max-height: 86vh; overflow-y: auto;
    background: var(--color-surface-raised);
    border: 1px solid var(--color-border); border-radius: var(--radius-lg);
    box-shadow: var(--shadow-lg); padding: var(--space-6);
    display: flex; flex-direction: column; gap: var(--space-5);
  }
  header h1 { font-size: var(--text-h1); font-weight: var(--fw-semibold); color: var(--color-text-primary); }
  .lede { margin-top: var(--space-2); font-size: var(--text-body-sm); color: var(--color-text-secondary); line-height: var(--lh-normal); }
  .cards { display: flex; flex-direction: column; gap: var(--space-3); }
  .card {
    border: 1px solid var(--color-border); border-radius: var(--radius-md);
    background: var(--color-surface); overflow: hidden;
  }
  .card.simple {
    display: flex; align-items: flex-start; gap: var(--space-3);
    padding: var(--space-4); text-align: left; cursor: pointer; color: inherit;
  }
  .card.simple:hover:not(:disabled) { background: var(--color-surface-sunken); border-color: var(--color-brand); }
  .card.simple:disabled { opacity: 0.5; cursor: default; }
  .card-head {
    width: 100%; display: flex; align-items: flex-start; gap: var(--space-3);
    padding: var(--space-4); background: transparent; cursor: pointer; text-align: left; color: inherit;
  }
  .card-head:hover { background: var(--color-surface-sunken); }
  .card.open { border-color: var(--color-brand); }
  .ct { display: flex; flex-direction: column; gap: 3px; min-width: 0; }
  .ct .t { font-weight: var(--fw-medium); color: var(--color-text-primary); }
  .ct .d { font-size: var(--text-caption); color: var(--color-text-muted); line-height: var(--lh-normal); }
  .card-body { padding: 0 var(--space-4) var(--space-4); display: flex; flex-direction: column; gap: var(--space-3); }
  .cands { display: flex; flex-direction: column; gap: 4px; }
  .cand {
    width: 100%; display: flex; align-items: center; gap: var(--space-2);
    padding: var(--space-2) var(--space-3); border: 1px solid var(--color-border);
    border-radius: var(--radius-sm); background: var(--color-surface-sunken); cursor: pointer; color: inherit;
  }
  .cand:hover:not(:disabled) { border-color: var(--color-brand); color: var(--color-brand); }
  .cand-main { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; text-align: left; }
  .cand-path { font-family: var(--font-mono); font-size: var(--text-mono); color: var(--color-text-primary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cand-meta { font-size: var(--text-caption); color: var(--color-text-muted); }
  .empty { font-size: var(--text-body-sm); color: var(--color-text-muted); }
  .manage { display: flex; align-items: center; gap: 8px; font-size: var(--text-caption); color: var(--color-text-secondary); }
  .row-actions { display: flex; gap: var(--space-2); }
  .ghost {
    display: inline-flex; align-items: center; gap: 6px; height: 30px; padding: 0 var(--space-3);
    border: 1px solid var(--color-border); border-radius: var(--radius-md);
    background: transparent; color: var(--color-text-secondary); font-size: var(--text-body-sm); cursor: pointer;
  }
  .ghost:hover:not(:disabled) { color: var(--color-text-primary); background: var(--color-surface-sunken); }
  .ghost:disabled { opacity: 0.5; cursor: default; }
  .err { font-size: var(--text-body-sm); color: var(--color-danger); }
  .sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border: 0; }
  .guide-facts { display: grid; gap: var(--space-3); }
  .fact {
    display: flex; align-items: flex-start; gap: var(--space-3); padding: var(--space-4);
    border: 1px solid var(--color-border); border-radius: var(--radius-md); background: var(--color-surface);
  }
  .guide-actions { display: flex; justify-content: flex-end; gap: var(--space-2); }
  .target-states { display: flex; flex-wrap: wrap; gap: var(--space-2); margin-top: var(--space-1); }
  .target-states span { font-size: var(--text-caption); color: var(--color-text-muted); }
  .target-states .available { color: var(--color-success); }
  .primary {
    height: 32px; padding: 0 var(--space-4); border: 0; border-radius: var(--radius-md);
    background: var(--color-brand); color: var(--color-text-inverse); cursor: pointer;
  }
  .primary:disabled { opacity: 0.5; cursor: default; }
  .destinations { display: flex; flex-direction: column; gap: var(--space-2); }
  .destinations code { font-size: var(--text-mono); overflow-wrap: anywhere; }
</style>
