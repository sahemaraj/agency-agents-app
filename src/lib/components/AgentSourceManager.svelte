<script lang="ts">
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import Refresh from "@lucide/svelte/icons/refresh-cw";
  import Trash from "@lucide/svelte/icons/trash-2";

  import Button from "./Button.svelte";
  import DestructiveConfirm from "./DestructiveConfirm.svelte";
  import Input from "./Input.svelte";
  import Modal from "./Modal.svelte";
  import { agentLibrary } from "$lib/stores/agentLibrary.svelte";
  import { i18n } from "$lib/stores/i18n.svelte";
  import type { AgentSource } from "$lib/types";

  interface Props { open: boolean; onClose: () => void; }
  let { open, onClose }: Props = $props();
  let repository = $state("");
  let gitRef = $state("");
  let subdirectory = $state("");
  let removeCandidate: AgentSource | null = $state(null);
  let status = $state("");

  async function addLocal() {
    const selected = await openDialog({ directory: true, multiple: false });
    if (typeof selected === "string" && await agentLibrary.addLocal(selected)) status = i18n.t("agents.sourceAdded");
  }
  async function addGithub() {
    if (!repository.trim()) return;
    if (await agentLibrary.addGithub(repository.trim(), gitRef.trim() || null, subdirectory.trim() || null)) {
      repository = ""; gitRef = ""; subdirectory = "";
      status = i18n.t("agents.sourceAdded");
    }
  }
  async function refresh(source: AgentSource) {
    if (await agentLibrary.refreshSource(source.id)) status = i18n.t("agents.sourceRefreshed", { source: source.label });
  }
  async function removeSource() {
    const source = removeCandidate;
    removeCandidate = null;
    if (source && await agentLibrary.removeSource(source.id)) status = i18n.t("agents.sourceRemoved", { source: source.label });
  }
  const sourceKind = (source: AgentSource) => i18n.t(`agents.sourceKind.${source.kind.kind}`);
</script>

<Modal {open} title={i18n.t("agents.manageSources")} size="wide" defaultFocus="first" {onClose}>
  <div class="add-grid">
    <Button onclick={addLocal}>{i18n.t("agents.addLocalSource")}</Button>
    <Input bind:value={repository} placeholder={i18n.t("agents.githubRepository")} ariaLabel={i18n.t("agents.githubRepository")} />
    <Input bind:value={gitRef} placeholder={i18n.t("agents.gitRef")} ariaLabel={i18n.t("agents.gitRef")} />
    <Input bind:value={subdirectory} placeholder={i18n.t("agents.subdirectory")} ariaLabel={i18n.t("agents.subdirectory")} />
    <Button variant="primary" disabled={!repository.trim()} loading={agentLibrary.busy} onclick={addGithub}>{i18n.t("agents.addGithubSource")}</Button>
  </div>
  {#if agentLibrary.error}<p class="error" role="alert">{agentLibrary.error}</p>{/if}
  <p class="sr-status" aria-live="polite">{status}</p>
  <ul class="sources">
    {#each agentLibrary.sources as source (source.id)}
      <li>
        <div><strong>{source.label}</strong><small>{sourceKind(source)}</small></div>
        <div class="actions">
          <Button size="sm" onclick={() => refresh(source)}>
            {#snippet icon()}<Refresh size={13} />{/snippet}{i18n.t("agents.refreshSource")}
          </Button>
          <Button size="sm" variant="ghost" disabled={source.kind.kind === "builtIn" || source.kind.kind === "published"} onclick={() => (removeCandidate = source)} ariaLabel={i18n.t("agents.removeSource")}>
            {#snippet icon()}<Trash size={13} />{/snippet}{i18n.t("agents.removeSource")}
          </Button>
        </div>
      </li>
    {/each}
  </ul>
</Modal>

<DestructiveConfirm open={removeCandidate !== null} title={i18n.t("agents.removeSourceTitle")}
  confirmLabel={i18n.t("agents.removeSource")} onCancel={() => (removeCandidate = null)}
  onConfirm={removeSource}>
  <p>{i18n.t("agents.removeSourceBody")}</p>
</DestructiveConfirm>

<style>
  .add-grid { display: grid; grid-template-columns: auto 2fr 1fr 1fr auto; gap: var(--space-2); align-items: center; }
  .sources { display: flex; flex-direction: column; gap: var(--space-2); margin-top: var(--space-4); max-height: 300px; overflow-y: auto; }
  .sources li { display: flex; justify-content: space-between; gap: var(--space-3); padding: var(--space-2); border: 1px solid var(--color-border); border-radius: var(--radius-md); }
  .sources li > div:first-child { display: flex; flex-direction: column; min-width: 0; }
  small { color: var(--color-text-muted); }
  .actions { display: flex; gap: var(--space-2); }
  .error { color: var(--color-danger); margin-top: var(--space-2); }
  .sr-status { min-height: 1px; color: var(--color-success); font-size: var(--text-caption); }
  @media (max-width: 760px) { .add-grid { grid-template-columns: 1fr; } }
</style>
