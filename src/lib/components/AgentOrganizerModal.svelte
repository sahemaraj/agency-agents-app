<script lang="ts">
  import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";

  import Button from "./Button.svelte";
  import DestructiveConfirm from "./DestructiveConfirm.svelte";
  import Input from "./Input.svelte";
  import Modal from "./Modal.svelte";
  import { agentLibrary } from "$lib/stores/agentLibrary.svelte";
  import { i18n } from "$lib/stores/i18n.svelte";
  import type { AgentPackageResult, AgentUpdatePolicy } from "$lib/types";

  interface Props { open: boolean; pkg: AgentPackageResult | null; onClose: () => void; }
  let { open, pkg, onClose }: Props = $props();

  let collectionName = $state("");
  let smartFolderName = $state("");
  let profileName = $state("");
  let selectedFolder = $state("");
  let renamedFolder = $state("");
  let parentFolder = $state("");
  let deleting: { kind: "folder" | "collection" | "smart" | "profile"; name: string } | null = $state(null);

  const reference = $derived(pkg?.reference ?? null);
  const favorite = $derived(reference ? agentLibrary.isFavorite(reference) : false);
  const assignedFolder = $derived(reference ? agentLibrary.folderFor(reference) ?? "" : "");
  const policy = $derived(reference ? agentLibrary.updatePolicy(reference) : "notify");

  function saveCollection() {
    if (!reference || !collectionName.trim()) return;
    const existing = agentLibrary.library.collections.find((item) => item.name === collectionName.trim());
    const agents = existing?.agents.some((item) => item.sourceId === reference.sourceId && item.relativePath === reference.relativePath)
      ? existing.agents
      : [...(existing?.agents ?? []), reference];
    void agentLibrary.saveCollection({ name: collectionName.trim(), agents }).then((result) => {
      if (result) collectionName = "";
    });
  }

  function saveSmartFolder() {
    if (!pkg?.agent || !smartFolderName.trim()) return;
    void agentLibrary.saveSmartFolder({
      name: smartFolderName.trim(),
      rule: {
        query: null,
        division: pkg.agent.category,
        sourceId: pkg.reference.sourceId,
        capability: null,
        lifecycleState: null,
        installable: null,
        favorite: null,
      },
    }).then((result) => { if (result) smartFolderName = ""; });
  }

  function saveProfile() {
    if (!profileName.trim()) return;
    void agentLibrary.saveProfile({
      name: profileName.trim(),
      folders: [...agentLibrary.library.folders],
      collections: agentLibrary.library.collections.map((item) => item.name),
    }).then((result) => { if (result) profileName = ""; });
  }

  async function exportLibrary() {
    const path = await saveDialog({ defaultPath: "agent-library.json" });
    if (path) await agentLibrary.exportLibrary(path);
  }

  async function importLibrary() {
    const path = await openDialog({ multiple: false, directory: false, filters: [{ name: "JSON", extensions: ["json"] }] });
    if (typeof path === "string") await agentLibrary.importLibrary(path);
  }

  function renameFolder() {
    if (!selectedFolder || !renamedFolder.trim()) return;
    void agentLibrary.renameFolder(selectedFolder, renamedFolder.trim()).then((result) => {
      if (result) renamedFolder = "";
    });
  }

  function moveFolder() {
    if (!selectedFolder) return;
    void agentLibrary.moveFolder(selectedFolder, parentFolder || null);
  }

  function confirmDelete() {
    if (!deleting) return;
    const item = deleting;
    deleting = null;
    if (item.kind === "folder") void agentLibrary.deleteFolder(item.name, true);
    else if (item.kind === "collection") void agentLibrary.deleteCollection(item.name);
    else if (item.kind === "smart") void agentLibrary.deleteSmartFolder(item.name);
    else void agentLibrary.deleteProfile(item.name);
  }
</script>

<Modal {open} title={i18n.t("agents.organize")} size="wide" defaultFocus="first" {onClose}>
  {#if pkg && reference}
    <section>
      <h2>{pkg.agent?.name ?? pkg.reference.relativePath}</h2>
      <p class="provenance">{pkg.reference.sourceId} · {pkg.reference.relativePath}</p>
      <div class="row">
        <Button size="sm" onclick={() => agentLibrary.setFavorite(reference, !favorite)}>
          {i18n.t(favorite ? "agents.removeFavorite" : "agents.addFavorite")}
        </Button>
        <label>{i18n.t("agents.assignFolder")}
          <select value={assignedFolder} onchange={(event) => agentLibrary.assignFolder(reference, event.currentTarget.value || null)}>
            <option value="">{i18n.t("agents.noFolder")}</option>
            {#each agentLibrary.library.folders as folder}<option value={folder}>{folder}</option>{/each}
          </select>
        </label>
        <label>{i18n.t("agents.updatePolicy")}
          <select value={policy} onchange={(event) => agentLibrary.setUpdatePolicy(reference, event.currentTarget.value as AgentUpdatePolicy)}>
            <option value="notify">{i18n.t("agents.policyNotify")}</option>
            <option value="autoTrusted">{i18n.t("agents.policyAutoTrusted")}</option>
            <option value="pin">{i18n.t("agents.policyPin")}</option>
            <option value="reviewScripts">{i18n.t("agents.policyReviewScripts")}</option>
          </select>
        </label>
        {#if pkg.agent}
          <Button size="sm" onclick={() => agentLibrary.setPreferredSource({ agentName: pkg.agent!.name, sourceId: pkg.reference.sourceId })}>
            {i18n.t("agents.preferSource")}
          </Button>
        {/if}
      </div>
    </section>
  {/if}

  <section>
    <h2>{i18n.t("agents.manageFolders")}</h2>
    <div class="row">
      <label>{i18n.t("agents.folder")}
        <select bind:value={selectedFolder}><option value="">{i18n.t("agents.chooseFolder")}</option>{#each agentLibrary.library.folders as folder}<option value={folder}>{folder}</option>{/each}</select>
      </label>
      <Input bind:value={renamedFolder} ariaLabel={i18n.t("agents.renameFolder")} placeholder={i18n.t("agents.newFolderName")} />
      <Button size="sm" disabled={!selectedFolder || !renamedFolder.trim()} onclick={renameFolder}>{i18n.t("agents.renameFolder")}</Button>
    </div>
    <div class="row">
      <label>{i18n.t("agents.parentFolder")}
        <select bind:value={parentFolder}><option value="">{i18n.t("agents.folderRoot")}</option>{#each agentLibrary.library.folders.filter((folder) => folder !== selectedFolder && !folder.startsWith(`${selectedFolder}/`)) as folder}<option value={folder}>{folder}</option>{/each}</select>
      </label>
      <Button size="sm" disabled={!selectedFolder} onclick={moveFolder}>{i18n.t("agents.moveFolder")}</Button>
      <Button size="sm" variant="danger" disabled={!selectedFolder} onclick={() => (deleting = { kind: "folder", name: selectedFolder })}>{i18n.t("common.delete")}</Button>
    </div>
  </section>

  <section class="named-grid">
    <div>
      <h2>{i18n.t("agents.collections")}</h2>
      <div class="row"><Input bind:value={collectionName} ariaLabel={i18n.t("agents.collectionName")} placeholder={i18n.t("agents.collectionName")} /><Button size="sm" disabled={!reference || !collectionName.trim()} onclick={saveCollection}>{i18n.t("agents.saveCollection")}</Button></div>
      <ul>{#each agentLibrary.library.collections as item}<li><span>{item.name} · {item.agents.length}</span><Button size="sm" variant="danger" onclick={() => (deleting = { kind: "collection", name: item.name })}>{i18n.t("common.delete")}</Button></li>{/each}</ul>
    </div>
    <div>
      <h2>{i18n.t("agents.smartFolders")}</h2>
      <div class="row"><Input bind:value={smartFolderName} ariaLabel={i18n.t("agents.smartFolderName")} placeholder={i18n.t("agents.smartFolderName")} /><Button size="sm" disabled={!pkg?.agent || !smartFolderName.trim()} onclick={saveSmartFolder}>{i18n.t("agents.saveSmartFolder")}</Button></div>
      <ul>{#each agentLibrary.library.smartFolders as item}<li><span>{item.name}</span><Button size="sm" variant="danger" onclick={() => (deleting = { kind: "smart", name: item.name })}>{i18n.t("common.delete")}</Button></li>{/each}</ul>
    </div>
    <div>
      <h2>{i18n.t("agents.workspaceProfiles")}</h2>
      <div class="row"><Input bind:value={profileName} ariaLabel={i18n.t("agents.profileName")} placeholder={i18n.t("agents.profileName")} /><Button size="sm" disabled={!profileName.trim()} onclick={saveProfile}>{i18n.t("agents.saveProfile")}</Button></div>
      <ul>{#each agentLibrary.library.profiles as item}<li><span>{item.name}</span><Button size="sm" variant="danger" onclick={() => (deleting = { kind: "profile", name: item.name })}>{i18n.t("common.delete")}</Button></li>{/each}</ul>
    </div>
  </section>

  <section>
    <h2>{i18n.t("agents.portableLibrary")}</h2>
    <div class="row"><Button size="sm" onclick={exportLibrary}>{i18n.t("agents.exportLibrary")}</Button><Button size="sm" onclick={importLibrary}>{i18n.t("agents.importLibrary")}</Button></div>
  </section>
  {#if agentLibrary.error}<p class="error" role="alert" aria-live="polite">{agentLibrary.error}</p>{/if}
</Modal>

<DestructiveConfirm open={deleting !== null} title={i18n.t("agents.deleteOrganizeTitle")}
  confirmLabel={i18n.t("common.delete")} onCancel={() => (deleting = null)} onConfirm={confirmDelete}>
  <p>{i18n.t("agents.deleteOrganizeBody", { name: deleting?.name ?? "" })}</p>
</DestructiveConfirm>

<style>
  section { display: flex; flex-direction: column; gap: var(--space-2); }
  section + section { padding-top: var(--space-3); border-top: 1px solid var(--color-border); }
  h2 { font-size: var(--text-body); font-weight: var(--fw-semibold); }
  .provenance { color: var(--color-text-muted); font-size: var(--text-caption); overflow-wrap: anywhere; }
  .row { display: flex; align-items: end; flex-wrap: wrap; gap: var(--space-2); }
  .row :global(.wrap) { flex: 1; min-width: 160px; }
  label { display: flex; flex-direction: column; gap: 4px; color: var(--color-text-muted); font-size: var(--text-caption); }
  select { min-height: 32px; border: 1px solid var(--color-border); border-radius: var(--radius-md); background: var(--color-surface-sunken); color: var(--color-text-primary); padding: 4px 8px; }
  .named-grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: var(--space-4); }
  .named-grid > div { display: flex; flex-direction: column; gap: var(--space-2); min-width: 0; }
  ul { display: flex; flex-direction: column; gap: 4px; max-height: 140px; overflow-y: auto; }
  li { display: flex; align-items: center; justify-content: space-between; gap: var(--space-2); }
  li span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .error { color: var(--color-danger); }
  @media (max-width: 760px) { .named-grid { grid-template-columns: 1fr; } }
</style>
