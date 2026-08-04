<script lang="ts">
  import Button from "./Button.svelte";
  import Input from "./Input.svelte";
  import Modal from "./Modal.svelte";
  import { agentLibrary } from "$lib/stores/agentLibrary.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { i18n } from "$lib/stores/i18n.svelte";

  interface Props {
    open: boolean;
    initial?: { relativePath: string; text: string } | null;
    onClose: () => void;
  }
  let { open, initial = null, onClose }: Props = $props();
  const DEFAULT_PATH = "custom-agent.md";
  const DEFAULT_TEXT = "---\nname: Custom Agent\ndescription: Describe what this Agent does.\n---\n\nAdd instructions here.\n";
  let relativePath = $state(DEFAULT_PATH);
  let text = $state(DEFAULT_TEXT);
  let status = $state("");

  $effect(() => {
    if (open) {
      relativePath = initial?.relativePath ?? DEFAULT_PATH;
      text = initial?.text ?? DEFAULT_TEXT;
      status = "";
    }
  });

  async function create() {
    if (await agentLibrary.createDraft({ relativePath, text })) {
      toast.success(i18n.t("agents.draftSaved"));
      onClose();
    }
  }
  async function publish(id: string) {
    if (await agentLibrary.publishDraft(id)) status = i18n.t("agents.draftPublished");
  }
  async function reject(id: string) {
    if (await agentLibrary.rejectDraft(id)) status = i18n.t("agents.draftRejected");
  }
  async function importFile(event: Event) {
    const file = (event.currentTarget as HTMLInputElement).files?.[0];
    if (!file) return;
    relativePath = file.name;
    text = await file.text();
  }
</script>

<Modal {open} title={i18n.t("agents.createAgent")} size="wide" defaultFocus="first" {onClose}>
  <div class="form">
    <Input bind:value={relativePath} ariaLabel={i18n.t("agents.relativePath")} placeholder={i18n.t("agents.relativePath")} />
    <label>{i18n.t("agents.importMarkdown")}<input type="file" accept=".md,text/markdown" onchange={importFile} /></label>
    <textarea bind:value={text} aria-label={i18n.t("agents.agentMarkdown")} rows="14"></textarea>
    {#if agentLibrary.error}<p class="error" role="alert">{agentLibrary.error}</p>{/if}
    <p class="status" aria-live="polite">{status}</p>
    {#each agentLibrary.drafts.filter((draft) => draft.state === "pending") as draft (draft.id)}
      <div class="draft">
        <span><strong>{draft.validation.agent?.name ?? draft.relativePath}</strong><small>{draft.validation.installable ? i18n.t("agents.readyToPublish") : i18n.t("agents.invalidDraft")}</small></span>
        <Button size="sm" disabled={!draft.validation.installable} onclick={() => publish(draft.id)}>{i18n.t("agents.publishDraft")}</Button>
        <Button size="sm" variant="ghost" onclick={() => reject(draft.id)}>{i18n.t("agents.rejectDraft")}</Button>
      </div>
    {/each}
  </div>
  {#snippet actions()}
    <Button modalAction="cancel" onclick={onClose}>{i18n.t("common.cancel")}</Button>
    <Button variant="primary" modalAction="confirm" loading={agentLibrary.busy} onclick={create}>{i18n.t("agents.saveDraft")}</Button>
  {/snippet}
</Modal>

<style>
  .form { display: flex; flex-direction: column; gap: var(--space-3); }
  label { display: flex; align-items: center; justify-content: space-between; gap: var(--space-3); }
  textarea { width: 100%; resize: vertical; border: 1px solid var(--color-border); border-radius: var(--radius-md); background: var(--color-surface-sunken); color: var(--color-text-primary); padding: var(--space-3); font-family: var(--font-mono); }
  .draft { display: flex; align-items: center; gap: var(--space-2); padding: var(--space-2); border: 1px solid var(--color-border); border-radius: var(--radius-md); }
  .draft > span { display: flex; flex: 1; min-width: 0; flex-direction: column; }
  small { color: var(--color-text-muted); }
  .error { color: var(--color-danger); }
  .status { min-height: 1px; color: var(--color-success); }
</style>
