<script lang="ts">
  import Folder from "@lucide/svelte/icons/folder";
  import FolderPlus from "@lucide/svelte/icons/folder-plus";
  import Library from "@lucide/svelte/icons/library";

  import { agentPackageLabel, buildAgentFolderTree, matchesAgentSmartFolder, sameAgent, type AgentFolderNode } from "$lib/agents/libraryModel";
  import { agentLibrary } from "$lib/stores/agentLibrary.svelte";
  import { i18n } from "$lib/stores/i18n.svelte";
  import type { AgentPackageResult } from "$lib/types";

  interface Props {
    onSelectAgent: (pkg: AgentPackageResult) => void;
    onSelectCollection: (name: string) => void;
  }
  let { onSelectAgent, onSelectCollection }: Props = $props();
  let activeView = $state("all");
  let creating = $state(false);
  let folderName = $state("");

  const tree = $derived(buildAgentFolderTree(agentLibrary.library.folders));
  const flattened = $derived.by(() => {
    const output: { node: AgentFolderNode; depth: number }[] = [];
    const visit = (nodes: AgentFolderNode[], depth: number) => {
      for (const node of nodes) { output.push({ node, depth }); visit(node.children, depth + 1); }
    };
    visit(tree, 1);
    return output;
  });
  const packageViews = $derived(agentLibrary.packages.map((pkg) => ({
    pkg,
    source: agentLibrary.sources.find((source) => source.id === pkg.reference.sourceId)!,
  })).filter((view) => !!view.source));
  const visible = $derived(packageViews.filter(({ pkg, source }) => {
    if (activeView === "all") return true;
    if (activeView === "favorites") return agentLibrary.library.favorites.some((item) => sameAgent(item, pkg.reference));
    if (activeView === "recent") return agentLibrary.library.recent.some((item) => sameAgent(item.agent, pkg.reference));
    if (activeView.startsWith("smart:")) {
      const smart = agentLibrary.library.smartFolders.find((item) => item.name === activeView.slice(6));
      return !!smart && matchesAgentSmartFolder({ pkg, source }, smart.rule, agentLibrary.library.favorites);
    }
    const activeFolder = activeView.slice(7);
    const assigned = agentLibrary.library.assignments.find((item) =>
      item.sourceId === pkg.reference.sourceId && item.relativePath === pkg.reference.relativePath
    )?.folderPath;
    return assigned === activeFolder || assigned?.startsWith(`${activeFolder}/`);
  }).map(({ pkg }) => pkg));

  function sourceLabel(sourceId: string): string {
    return agentLibrary.sources.find((source) => source.id === sourceId)?.label ?? sourceId;
  }

  function packageLabel(pkg: AgentPackageResult): string {
    const view = packageViews.find(({ pkg: candidate }) => sameAgent(candidate.reference, pkg.reference));
    return view ? agentPackageLabel(view, packageViews) : pkg.agent?.name ?? pkg.reference.relativePath;
  }

  function navigate(event: KeyboardEvent) {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    const buttons = [...(event.currentTarget as HTMLElement)
      .closest('[role="tree"]')!
      .querySelectorAll<HTMLButtonElement>('[role="treeitem"]')];
    const index = buttons.indexOf(event.currentTarget as HTMLButtonElement);
    const target = event.key === "Home" ? buttons[0]
      : event.key === "End" ? buttons[buttons.length - 1]
      : buttons[index + (event.key === "ArrowDown" ? 1 : -1)];
    target?.focus();
    event.preventDefault();
  }

  async function createFolder() {
    const parent = activeView.startsWith("folder:") ? activeView.slice(7) : null;
    const name = folderName.trim();
    if (!name) return;
    if (await agentLibrary.createFolder(parent ? `${parent}/${name}` : name)) {
      creating = false;
      folderName = "";
    }
  }
</script>

<aside class="library" aria-label={i18n.t("agents.libraryAria")}>
  <div class="heading"><Library size={15} /><strong>{i18n.t("agents.library")}</strong></div>
  <div class="folders" role="tree" aria-label={i18n.t("agents.foldersAria")}>
    <button role="treeitem" aria-level="1" aria-selected={activeView === "all"} class:on={activeView === "all"}
      onkeydown={navigate} onclick={() => (activeView = "all")}>
      <Library size={13} /> {i18n.t("agents.allSources")}
    </button>
    <button role="treeitem" aria-level="1" aria-selected={activeView === "favorites"} class:on={activeView === "favorites"}
      onkeydown={navigate} onclick={() => (activeView = "favorites")}>
      <span aria-hidden="true">★</span> {i18n.t("agents.favorites")}
    </button>
    <button role="treeitem" aria-level="1" aria-selected={activeView === "recent"} class:on={activeView === "recent"}
      onkeydown={navigate} onclick={() => (activeView = "recent")}>
      <span aria-hidden="true">↻</span> {i18n.t("agents.recent")}
    </button>
    {#each flattened as item (item.node.path)}
      <button role="treeitem" aria-level={item.depth + 1} aria-selected={activeView === `folder:${item.node.path}`}
        class:on={activeView === `folder:${item.node.path}`} style={`padding-left:${8 + item.depth * 14}px`}
        onkeydown={navigate} onclick={() => (activeView = `folder:${item.node.path}`)}>
        <Folder size={13} /> <span class="truncate">{item.node.label}</span>
      </button>
    {/each}
    {#each agentLibrary.library.smartFolders as smartFolder (smartFolder.name)}
      <button role="treeitem" aria-level="1" aria-selected={activeView === `smart:${smartFolder.name}`}
        class:on={activeView === `smart:${smartFolder.name}`} onkeydown={navigate} onclick={() => (activeView = `smart:${smartFolder.name}`)}>
        <span aria-hidden="true">◇</span> <span class="truncate">{smartFolder.name}</span>
      </button>
    {/each}
  </div>
  {#if creating}
    <form class="new-folder" onsubmit={(event) => { event.preventDefault(); void createFolder(); }}>
      <input bind:value={folderName} aria-label={i18n.t("agents.folderName")} maxlength="64" />
      <button type="submit">{i18n.t("agents.createFolder")}</button>
    </form>
  {:else}
    <button class="add" onclick={() => (creating = true)}><FolderPlus size={13} /> {i18n.t("agents.newFolder")}</button>
  {/if}
  {#if agentLibrary.library.collections.length > 0}
    <div class="collections" aria-label={i18n.t("agents.collections")}>
      <strong>{i18n.t("agents.collections")}</strong>
      {#each agentLibrary.library.collections as collection (collection.name)}
        <button onclick={() => onSelectCollection(collection.name)}>
          <span class="truncate">{collection.name}</span>
          <small>{i18n.t("agents.agentCount", { count: collection.agents.length })}</small>
        </button>
      {/each}
    </div>
  {/if}
  <div class="packages" aria-label={i18n.t("agents.folderAgentsAria")}>
    {#each visible as pkg (`${pkg.reference.sourceId}:${pkg.reference.relativePath}`)}
      <button disabled={!pkg.agent} onclick={() => onSelectAgent(pkg)}>
        <span class="truncate">{packageLabel(pkg)}</span>
        <small class="truncate">{sourceLabel(pkg.reference.sourceId)}</small>
      </button>
    {/each}
  </div>
</aside>

<style>
  .library { width: 220px; flex: none; border-right: 1px solid var(--color-border); background: var(--color-surface-raised); display: flex; flex-direction: column; min-height: 0; }
  .heading { display: flex; align-items: center; gap: 7px; padding: var(--space-3); border-bottom: 1px solid var(--color-border); font-size: var(--text-body-sm); }
  .folders { padding: var(--space-2); display: flex; flex-direction: column; gap: 1px; }
  .folders button, .add { display: flex; align-items: center; gap: 6px; width: 100%; min-height: 28px; padding: 4px 8px; border-radius: var(--radius-sm); color: var(--color-text-secondary); text-align: left; font-size: var(--text-body-sm); }
  .folders button:hover, .folders button.on, .add:hover { background: var(--color-surface-sunken); color: var(--color-text-primary); }
  .folders button.on { color: var(--color-brand); }
  .add { margin: 0 var(--space-2); width: calc(100% - var(--space-4)); }
  .new-folder { display: flex; gap: 4px; padding: 0 var(--space-2) var(--space-2); }
  .new-folder input { min-width: 0; flex: 1; border: 1px solid var(--color-border); border-radius: var(--radius-sm); padding: 4px 6px; }
  .new-folder button { color: var(--color-brand); font-size: var(--text-caption); }
  .collections { border-top: 1px solid var(--color-border); padding: var(--space-2); display: flex; flex-direction: column; gap: 2px; }
  .collections > strong { padding: 3px 8px; font-size: var(--text-caption); color: var(--color-text-muted); }
  .collections button { display: flex; align-items: center; justify-content: space-between; gap: var(--space-2); padding: 6px 8px; border-radius: var(--radius-sm); text-align: left; }
  .collections button:hover { background: var(--color-surface-sunken); }
  .collections small { flex: none; color: var(--color-text-muted); font-size: var(--text-caption); }
  .packages { border-top: 1px solid var(--color-border); padding: var(--space-2); overflow-y: auto; display: flex; flex-direction: column; gap: 2px; }
  .packages button { display: flex; flex-direction: column; padding: 6px 8px; border-radius: var(--radius-sm); text-align: left; }
  .packages button:hover { background: var(--color-surface-sunken); }
  .packages small { color: var(--color-text-muted); font-size: var(--text-caption); }
  .truncate { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  @media (max-width: 900px) { .library { display: none; } }
</style>
