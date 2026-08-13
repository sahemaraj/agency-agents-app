<script lang="ts">
  import Search from "@lucide/svelte/icons/search";

  import { taskRecommendations } from "$lib/api";
  import { ui } from "$lib/stores/ui.svelte";
  import { i18n } from "$lib/stores/i18n.svelte";
  import { shortcut } from "$lib/util/platform";
  import { appErrorMessage, isAppError, type PaletteItem, type TaskRecommendation } from "$lib/types";

  let query = $state("");
  let selectedIdx = $state(0);
  let inputEl: HTMLInputElement | undefined = $state();
  let recommendations: TaskRecommendation[] = $state([]);
  let recommendationLoading = $state(false);
  let recommendationError: string | null = $state(null);
  let retry = $state(0);
  let focusReturn: HTMLElement | null = null;
  let wasOpen = false;

  $effect(() => {
    if (ui.paletteOpen) {
      if (!wasOpen) focusReturn = document.activeElement as HTMLElement | null;
      query = "";
      selectedIdx = 0;
      // focus after mount/render
      setTimeout(() => inputEl?.focus(), 0);
    } else if (wasOpen) {
      setTimeout(() => focusReturn?.focus(), 0);
    }
    wasOpen = ui.paletteOpen;
  });

  const commands = $derived<PaletteItem[]>([
    { kind: "command", id: "dashboard", label: i18n.t("palette.openDashboard"), shortcut: shortcut("0"), section: i18n.t("palette.nav"), run: () => ui.setSection("dashboard") },
    { kind: "command", id: "personas",  label: i18n.t("palette.openAgents"),    shortcut: shortcut("1"), section: i18n.t("palette.nav"), run: () => ui.openAgents() },
    { kind: "command", id: "skills",    label: "Open Skills",                  shortcut: shortcut("2"), section: i18n.t("palette.nav"), run: () => ui.setSection("skills") },
    { kind: "command", id: "tools",     label: i18n.t("palette.openTools"),     shortcut: shortcut("3"), section: i18n.t("palette.nav"), run: () => ui.setSection("tools") },
    { kind: "command", id: "teams",     label: i18n.t("palette.openTeams"),     shortcut: shortcut("4"), section: i18n.t("palette.nav"), run: () => ui.setSection("teams") },
    { kind: "command", id: "projects",  label: i18n.t("palette.openProjects"),  shortcut: shortcut("5"), section: i18n.t("palette.nav"), run: () => ui.setSection("projects") },
    { kind: "command", id: "experts",   label: i18n.t("palette.openExperts"),   shortcut: shortcut("6"), section: i18n.t("palette.nav"), run: () => ui.setSection("experts") },
    { kind: "command", id: "activity",  label: i18n.t("palette.openActivity"),  shortcut: shortcut("7"), section: i18n.t("palette.nav"), run: () => ui.setSection("activity") },
    { kind: "command", id: "drawer",    label: i18n.t("palette.toggleActivityDrawer"), shortcut: shortcut("L"), section: i18n.t("palette.view"), run: () => ui.toggleDrawer() },
    { kind: "command", id: "playbook",  label: i18n.t("palette.openPlaybook"), section: i18n.t("palette.help"), run: () => ui.openPlaybook() },
  ]);

  let commandHits = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return commands;
    return commands.filter((c) => c.kind === "command" && c.label.toLowerCase().includes(q));
  });

  function explain(reasons: string[]): string {
    return reasons.map((reason) => {
      const [scope, field, token] = reason.split(":");
      if (scope === "task" && field === "name") return i18n.optional("palette.reasonName", "Name matches “{token}”", { token });
      if (scope === "task" && field === "description") return i18n.optional("palette.reasonDescription", "Description matches “{token}”", { token });
      if (scope === "task" && field === "taxonomy") return i18n.optional("palette.reasonTaxonomy", "Category matches “{token}”", { token });
      if (scope === "language") return i18n.optional("palette.reasonLanguage", "Language matches “{token}”", { token: field });
      if (scope === "preferred-source") return i18n.optional("palette.reasonPreferredSource", "Preferred source");
      return reason;
    }).join(" · ");
  }

  let recommendationItems = $derived<PaletteItem[]>(recommendations.map((recommendation) => {
    if (recommendation.kind === "agent") {
      const pkg = recommendation.package;
      return {
        kind: "agent",
        id: `agent:${pkg.reference.sourceId}:${pkg.reference.relativePath}`,
        label: pkg.agent?.name ?? pkg.reference.relativePath,
        description: pkg.agent?.description ?? "Agent",
        reason: explain(recommendation.reasons),
        source: `${pkg.reference.sourceId} · ${pkg.reference.relativePath}`,
        run: () => ui.openAgentReference(pkg.reference),
      };
    }
    const pkg = recommendation.package;
    return {
      kind: "skill",
      id: `skill:${pkg.sourceId}:${pkg.relativePath}`,
      label: pkg.name ?? pkg.relativePath,
      description: pkg.description ?? "Skill",
      reason: explain(recommendation.reasons),
      source: `${pkg.sourceId} · ${pkg.relativePath}`,
      run: () => ui.openSkill({ sourceId: pkg.sourceId, relativePath: pkg.relativePath }),
    };
  }));

  $effect(() => {
    retry;
    const task = query.trim();
    if (!ui.paletteOpen || task.length < 3) {
      recommendations = [];
      recommendationLoading = false;
      recommendationError = null;
      return;
    }
    let current = true;
    recommendations = [];
    recommendationLoading = true;
    recommendationError = null;
    const timer = setTimeout(() => {
      void taskRecommendations(task).then((results) => {
        if (current && query.trim() === task && ui.paletteOpen) recommendations = results;
      }).catch((error: unknown) => {
        if (current && query.trim() === task && ui.paletteOpen) {
          recommendations = [];
          recommendationError = isAppError(error) ? appErrorMessage(error) : "Unexpected local search error";
        }
      }).finally(() => {
        if (current && query.trim() === task && ui.paletteOpen) recommendationLoading = false;
      });
    }, 200);
    return () => {
      current = false;
      clearTimeout(timer);
    };
  });

  type Group = { label: string; items: Array<{ item: PaletteItem; idx: number }> };
  let groups = $derived.by<Group[]>(() => {
    const out: Group[] = [];
    let idx = 0;
    if (commandHits.length > 0) {
      out.push({
        label: i18n.t("palette.commands"),
        items: commandHits.map((c) => ({ item: c, idx: idx++ })),
      });
    }
    const agents = recommendationItems.filter((item) => item.kind === "agent");
    if (agents.length > 0) {
      out.push({ label: i18n.optional("palette.agents", "Agents"), items: agents.map((item) => ({ item, idx: idx++ })) });
    }
    const skills = recommendationItems.filter((item) => item.kind === "skill");
    if (skills.length > 0) {
      out.push({ label: i18n.optional("palette.skills", "Skills"), items: skills.map((item) => ({ item, idx: idx++ })) });
    }
    return out;
  });

  let totalItems = $derived(groups.reduce((n, g) => n + g.items.length, 0));

  $effect(() => {
    if (selectedIdx >= totalItems) selectedIdx = Math.max(0, totalItems - 1);
  });

  function activate(item: PaletteItem) {
    void item.run();
    ui.closePalette();
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") { e.preventDefault(); ui.closePalette(); return; }
    if (e.key === "ArrowDown") { e.preventDefault(); selectedIdx = Math.max(0, Math.min(totalItems - 1, selectedIdx + 1)); }
    if (e.key === "ArrowUp") { e.preventDefault(); selectedIdx = Math.max(0, selectedIdx - 1); }
    if (e.key === "Enter") {
      e.preventDefault();
      let found: PaletteItem | undefined;
      for (const g of groups) {
        const hit = g.items.find((x) => x.idx === selectedIdx);
        if (hit) { found = hit.item; break; }
      }
      if (found) activate(found);
    }
  }
</script>

{#if ui.paletteOpen}
  <div class="scrim" role="presentation" onclick={() => ui.closePalette()}></div>
  <div class="palette" role="dialog" aria-modal="true" aria-label={i18n.t("palette.dialogLabel")}>
    <div class="search">
      <Search size={16} />
      <input
        bind:this={inputEl}
        type="text"
        placeholder={i18n.t("palette.placeholder")}
        bind:value={query}
        onkeydown={onKey}
        aria-label={i18n.t("palette.searchLabel")}
        role="combobox"
        aria-controls="palette-listbox"
        aria-expanded={totalItems > 0}
        aria-activedescendant={totalItems > 0 ? `palette-opt-${selectedIdx}` : undefined}
        aria-autocomplete="list"
        aria-describedby="palette-limit"
        maxlength="2048"
      />
      <span class="kbd">Esc</span>
    </div>

    <div class="results">
      <div class="status" role="status" aria-live="polite">
        {#if recommendationLoading}{i18n.optional("palette.loadingRecommendations", "Finding local Agents and Skills…")}{/if}
        {#if recommendationError}
          {i18n.optional("palette.recommendationsUnavailable", "Recommendations unavailable")}: {recommendationError}
          <button class="retry" onclick={() => (retry += 1)}>{i18n.optional("common.retry", "Retry")}</button>
        {/if}
        {#if !recommendationLoading && !recommendationError && query.trim().length >= 3}
          {recommendations.length} {recommendations.length === 1
            ? i18n.optional("palette.recommendation", "recommendation")
            : i18n.optional("palette.recommendations", "recommendations")}
        {/if}
      </div>
      {#if totalItems === 0 && !recommendationLoading}
        <p class="empty">{i18n.t("palette.empty")}</p>
      {:else}
        <div id="palette-listbox" role="listbox" aria-label={i18n.t("palette.resultsLabel")}>
          {#each groups as g (g.label)}
            <div class="group" role="group" aria-label={g.label}>
              <div class="group-label" aria-hidden="true">{g.label}</div>
              {#each g.items as entry (entry.idx)}
                {@const item = entry.item}
                <button
                  class="result"
                  class:on={entry.idx === selectedIdx}
                  role="option"
                  id="palette-opt-{entry.idx}"
                  aria-selected={entry.idx === selectedIdx}
                  onmouseenter={() => (selectedIdx = entry.idx)}
                  onclick={() => activate(item)}
                >
                  <span class="name">{item.label}</span>
                  {#if item.kind === "command" && item.shortcut}<span class="meta kbd">{item.shortcut}</span>{/if}
                  {#if item.kind !== "command"}
                    <span class="detail">{item.description}</span>
                    <span class="meta">{item.kind === "agent" ? i18n.optional("palette.agent", "Agent") : i18n.optional("palette.skill", "Skill")}</span>
                    <span class="detail reason">{item.reason}</span>
                    <span class="meta source">{item.source}</span>
                  {/if}
                </button>
              {/each}
            </div>
          {/each}
        </div>
      {/if}
    </div>

    <footer class="foot">
      <span id="palette-limit" class="sr-only">{i18n.optional("palette.taskLimit", "Task description limit: 2048 characters.")}</span>
      <span class="kbd">↑↓</span> {i18n.t("palette.navigate")}
      <span class="kbd">⏎</span> {i18n.t("palette.open")}
      <span class="kbd">Esc</span> {i18n.t("palette.close")}
    </footer>
  </div>
{/if}

<style>
  .scrim {
    position: fixed; inset: 0;
    background: rgb(0 0 0 / 0.4);
    z-index: 80;
    animation: fadeIn var(--motion-duration-base) var(--motion-ease-out);
  }
  .palette {
    position: fixed;
    top: 10%;
    left: 50%;
    transform: translateX(-50%);
    width: 640px;
    max-width: calc(100% - 32px);
    max-height: 60vh;
    background: var(--color-surface-raised);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-modal);
    z-index: 81;
    display: flex;
    flex-direction: column;
    animation: pop var(--motion-duration-base) var(--motion-ease-out);
  }
  @keyframes fadeIn { from { opacity: 0; } to { opacity: 1; } }
  @keyframes pop { from { opacity: 0; transform: translate(-50%, -4px) scale(0.98); } to { opacity: 1; transform: translateX(-50%) scale(1); } }
  @media (prefers-reduced-motion: reduce) {
    .scrim, .palette { animation: none; }
  }

  .search {
    display: flex; align-items: center; gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--color-border);
    color: var(--color-text-muted);
  }
  .search input {
    flex: 1;
    background: transparent;
    font-size: var(--text-body);
    color: var(--color-text-primary);
  }
  .search input::placeholder { color: var(--color-text-muted); }

  .results {
    overflow-y: auto;
    flex: 1;
    min-height: 0;
    padding: var(--space-2);
  }

  .group { margin-bottom: var(--space-2); }
  .group-label {
    padding: var(--space-1) var(--space-3);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    font-size: var(--text-caption);
    color: var(--color-text-muted);
    font-weight: var(--fw-semibold);
  }
  .result {
    display: grid;
    grid-template-columns: 1fr auto;
    align-items: center;
    width: 100%;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    color: var(--color-text-primary);
    font-size: var(--text-body);
    gap: var(--space-3);
    text-align: left;
  }
  .name { min-width: 0; }
  .detail { color: var(--color-text-muted); font-size: var(--text-body-sm); min-width: 0; }
  .reason, .source { font-size: var(--text-caption); }
  .result.on { background: var(--color-selection-strong); color: var(--color-text-inverse); }
  .meta { color: var(--color-text-muted); font-size: var(--text-body-sm); }
  .result.on .meta, .result.on .detail { color: var(--color-text-inverse); opacity: 0.85; }

  .status { color: var(--color-text-muted); font-size: var(--text-body-sm); padding: 0 var(--space-3); }
  .status:empty { display: none; }
  .retry { margin-left: var(--space-2); text-decoration: underline; }

  .kbd {
    font-family: var(--font-mono);
    font-size: var(--text-caption);
    color: var(--color-text-muted);
    background: var(--color-surface-sunken);
    border-radius: var(--radius-sm);
    padding: 1px 6px;
  }

  .foot {
    display: flex; gap: var(--space-3); align-items: center;
    padding: var(--space-2) var(--space-3);
    border-top: 1px solid var(--color-border);
    color: var(--color-text-muted);
    font-size: var(--text-caption);
  }
  .empty { padding: var(--space-4); color: var(--color-text-muted); }
  .sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border: 0; }
</style>
