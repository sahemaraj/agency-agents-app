<script lang="ts">
  import { onMount } from "svelte";
  import Check from "@lucide/svelte/icons/check";
  import Copy from "@lucide/svelte/icons/copy";
  import RefreshCw from "@lucide/svelte/icons/refresh-cw";
  import {
    mcpClientConnect,
    mcpClientDisconnect,
    mcpClientRepair,
    mcpClientsStatus,
  } from "$lib/api";
  import { i18n } from "$lib/stores/i18n.svelte";
  import { appErrorMessage, isAppError, type McpClient, type McpClientStatus } from "$lib/types";

  let statuses: McpClientStatus[] = $state([]);
  let loading = $state(false);
  let error = $state("");
  let copied: McpClient | null = $state(null);

  function clientId(status: McpClientStatus): McpClient {
    return status.client;
  }

  function clientLabel(client: McpClient): string {
    return client === "claude" ? "Claude Code" : "Codex";
  }

  function stateLabel(state: McpClientStatus["state"]): string {
    return i18n.t(`settings.mcp.status.${state}`);
  }

  async function refresh() {
    loading = true;
    error = "";
    try {
      statuses = await mcpClientsStatus();
    } catch {
      error = i18n.t("settings.mcp.loadFailed");
    } finally {
      loading = false;
    }
  }

  async function mutate(status: McpClientStatus, action: "connect" | "disconnect" | "repair") {
    loading = true;
    error = "";
    try {
      const client = clientId(status);
      if (action === "connect") await mcpClientConnect(client);
      else if (action === "disconnect") await mcpClientDisconnect(client);
      else await mcpClientRepair(client);
      await refresh();
    } catch (cause) {
      error = isAppError(cause) ? appErrorMessage(cause) : i18n.t("settings.mcp.loadFailed");
      loading = false;
    }
  }

  async function copyCommand(status: McpClientStatus) {
    error = "";
    try {
      await navigator.clipboard.writeText(status.command);
      copied = status.client;
      setTimeout(() => {
        if (copied === status.client) copied = null;
      }, 1500);
    } catch {
      error = i18n.t("settings.mcp.copyFailed");
    }
  }

  onMount(() => void refresh());
</script>

<section class="section" aria-labelledby="mcp-title">
  <div class="heading">
    <div>
      <h2 id="mcp-title">{i18n.t("settings.mcp.title")}</h2>
      <p>{i18n.t("settings.mcp.help")}</p>
    </div>
    <button type="button" class="icon-button" onclick={refresh} disabled={loading} aria-label={i18n.t("settings.mcp.refresh")}>
      <span class:spin={loading}><RefreshCw size={15} /></span>
    </button>
  </div>

  {#if error}<p class="error" role="alert">{error}</p>{/if}

  <div class="cards" aria-live="polite" aria-busy={loading}>
    {#each statuses as status (status.client)}
      <article class="card">
        <div class="card-head">
          <strong>{clientLabel(status.client)}</strong>
          <span class:ok={status.state === "exact"} class:warn={status.state === "conflict"}>
            {stateLabel(status.state)}
          </span>
        </div>
        <p>{status.detail}</p>
        <div class="actions">
          {#if status.state === "missing"}
            <button type="button" class="primary" disabled={loading} onclick={() => mutate(status, "connect")}>{i18n.t("settings.mcp.connect")}</button>
          {:else if status.state === "exact"}
            <button type="button" disabled={loading} onclick={() => mutate(status, "disconnect")}>{i18n.t("settings.mcp.disconnect")}</button>
          {:else if status.state === "conflict"}
            <button type="button" class="primary" disabled={loading} onclick={() => mutate(status, "repair")}>{i18n.t("settings.mcp.repair")}</button>
          {/if}
        </div>
        <details>
          <summary>{i18n.t("settings.mcp.manual")}</summary>
          <div class="command">
            <code>{status.command}</code>
            <button type="button" onclick={() => copyCommand(status)} aria-label={`${i18n.t("settings.mcp.copy")} ${clientLabel(status.client)}`}>
              {#if copied === status.client}<Check size={14} />{:else}<Copy size={14} />{/if}
            </button>
          </div>
        </details>
      </article>
    {/each}
  </div>
  <span class="sr-only" aria-live="polite">
    {copied ? `${clientLabel(copied)} ${i18n.t("settings.mcp.copied")}` : ""}
  </span>
</section>

<style>
  .section { display: flex; flex-direction: column; gap: var(--space-4); max-width: 650px; }
  .heading, .card-head, .actions, .command { display: flex; align-items: center; }
  .heading, .card-head { justify-content: space-between; gap: var(--space-3); }
  h2 { margin: 0 0 var(--space-1); font-size: var(--text-h1); }
  p { margin: 0; color: var(--color-text-muted); font-size: var(--text-body-sm); }
  .cards { display: grid; gap: var(--space-3); }
  .card { padding: var(--space-4); border: 1px solid var(--color-border); border-radius: var(--radius-md); display: grid; gap: var(--space-3); }
  .card-head span { font-size: var(--text-caption); color: var(--color-text-muted); }
  .card-head span.ok { color: var(--color-success); }
  .card-head span.warn, .error { color: var(--color-danger); }
  .actions { gap: var(--space-2); }
  button { min-height: 32px; padding: 0 var(--space-3); border: 1px solid var(--color-border); border-radius: var(--radius-sm); background: var(--color-surface-raised); color: var(--color-text-primary); cursor: pointer; }
  button:disabled { opacity: .5; cursor: default; }
  button.primary { background: var(--color-accent); color: white; border-color: var(--color-accent); }
  .icon-button { width: 32px; padding: 0; display: grid; place-items: center; }
  details { font-size: var(--text-body-sm); }
  summary { cursor: pointer; color: var(--color-text-secondary); }
  .command { margin-top: var(--space-2); gap: var(--space-2); }
  code { flex: 1; min-width: 0; overflow-wrap: anywhere; padding: var(--space-2); background: var(--color-surface-sunken); border-radius: var(--radius-sm); font-size: var(--text-caption); }
  .command button { width: 32px; padding: 0; display: grid; place-items: center; }
  :global(.spin) { animation: spin .8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>
