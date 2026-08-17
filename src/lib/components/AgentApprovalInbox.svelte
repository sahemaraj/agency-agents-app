<script lang="ts">
  import { tick, untrack } from "svelte";
  import Button from "./Button.svelte";
  import Modal from "./Modal.svelte";
  import { agentApprovalFacts } from "$lib/agents/libraryModel";
  import { agentLibrary } from "$lib/stores/agentLibrary.svelte";
  import { i18n } from "$lib/stores/i18n.svelte";
  import { ui } from "$lib/stores/ui.svelte";
  import type { MessageKey } from "$lib/i18n/messages";
  import type { AgentApprovalAction } from "$lib/types";

  interface Props { open: boolean; onClose: () => void; focusId?: string | null; }
  let { open, onClose, focusId = null }: Props = $props();

  const actionKeys: Record<AgentApprovalAction["action"], MessageKey> = {
    sourceRemove: "agents.approvalAction.sourceRemove",
    folderDelete: "agents.approvalAction.folderDelete",
    collectionDelete: "agents.approvalAction.collectionDelete",
    smartFolderDelete: "agents.approvalAction.smartFolderDelete",
    profileDelete: "agents.approvalAction.profileDelete",
    updatePolicySet: "agents.approvalAction.updatePolicySet",
    publisherTrustSet: "agents.approvalAction.publisherTrustSet",
    draftPublish: "agents.approvalAction.draftPublish",
    install: "agents.approvalAction.install",
    update: "agents.approvalAction.update",
    uninstall: "agents.approvalAction.uninstall",
    rollback: "agents.approvalAction.rollback",
    batchCollection: "agents.approvalAction.batchCollection",
  };

  const approvals = $derived([...agentLibrary.library.approvals].sort((left, right) =>
    Date.parse(right.submittedAt) - Date.parse(left.submittedAt)
  ));
  let status: HTMLDivElement | undefined = $state();
  let loadedFocusId = $state<string | null>(null);

  $effect(() => {
    if (!open) return;
    const requestedFocusId = focusId;
    untrack(() => void agentLibrary.load(true).then(() => {
      if (open && focusId === requestedFocusId) loadedFocusId = requestedFocusId;
    }));
  });

  $effect(() => {
    if (!open || !focusId || !approvals.some((approval) => approval.id === focusId)) return;
    void tick().then(() => [...document.querySelectorAll<HTMLElement>("[data-agent-approval-id]")]
      .find((candidate) => candidate.dataset.agentApprovalId === focusId)
      ?.querySelector<HTMLElement>("button")?.focus({ preventScroll: true }));
  });

  $effect(() => {
    if (!open || !focusId || loadedFocusId !== focusId) return;
    if (approvals.some((approval) => approval.id === focusId)) return;
    onClose();
  });

  function stale(result: string | null): boolean {
    return !!result && /stale|revision|changed since/i.test(result);
  }

  async function resolve(id: string, decision: "approve" | "reject"): Promise<void> {
    const linked = ui.isReviewIntent("agent", id);
    const succeeded = decision === "approve"
      ? await agentLibrary.approveRequest(id)
      : await agentLibrary.rejectRequest(id);
    if (!linked) return;
    if (succeeded) onClose();
    else {
      await tick();
      status?.focus({ preventScroll: true });
    }
  }
</script>

<Modal {open} title={i18n.t("agents.approvalInbox")} size="wide" {onClose}>
  <p>{i18n.t("agents.approvalInboxHelp")}</p>
  <div class="status" aria-live="polite" tabindex="-1" bind:this={status}>{agentLibrary.error ?? ""}</div>
  {#if approvals.length === 0}
    <p class="empty">{i18n.t("agents.noApprovals")}</p>
  {:else}
    <ul class="requests">
      {#each approvals as approval (approval.id)}
        {@const facts = agentApprovalFacts(approval.request)}
        <li data-agent-approval-id={approval.id}>
          <div class="request-head">
            <strong>{i18n.t(actionKeys[facts.kind])}</strong>
            <span class:stale={stale(approval.result)}>{i18n.t(`agents.approvalState.${approval.state}`)}</span>
          </div>
          <dl>
            <dt>{i18n.t("agents.requestedBy")}</dt><dd>{approval.requestedBy}</dd>
            <dt>{i18n.t("agents.requestedAt")}</dt><dd>{new Date(approval.submittedAt).toLocaleString()}</dd>
            <dt>{i18n.t("agents.approvalSubject")}</dt><dd>{facts.subject}</dd>
            {#if facts.planRevision}
              <dt>{i18n.t("agents.planRevision")}</dt><dd><code>{facts.planRevision}</code></dd>
            {/if}
          </dl>
          {#if approval.result}
            <p class:stale={stale(approval.result)} role={stale(approval.result) ? "alert" : undefined}>
              {stale(approval.result) ? `${i18n.t("agents.staleApproval")}: ` : ""}{approval.result}
            </p>
          {/if}
          {#if approval.state === "pending"}
            <div class="actions">
              <Button size="sm" variant="primary" loading={agentLibrary.busy} onclick={() => void resolve(approval.id, "approve")}>{i18n.t("agents.approve")}</Button>
              <Button size="sm" variant="danger" disabled={agentLibrary.busy} onclick={() => void resolve(approval.id, "reject")}>{i18n.t("agents.reject")}</Button>
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</Modal>

<style>
  .status { min-height: 1px; color: var(--color-danger); }
  .empty { color: var(--color-text-muted); }
  .requests { display: flex; flex-direction: column; gap: var(--space-3); max-height: 58vh; overflow-y: auto; }
  .requests > li { display: flex; flex-direction: column; gap: var(--space-2); padding: var(--space-3); border: 1px solid var(--color-border); border-radius: var(--radius-md); }
  .request-head, .actions { display: flex; align-items: center; justify-content: space-between; gap: var(--space-2); }
  .request-head span { color: var(--color-text-muted); }
  dl { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 4px var(--space-3); font-size: var(--text-body-sm); }
  dt { color: var(--color-text-muted); } dd { min-width: 0; overflow-wrap: anywhere; }
  .actions { justify-content: flex-end; }
  .stale { color: var(--color-danger) !important; }
</style>
