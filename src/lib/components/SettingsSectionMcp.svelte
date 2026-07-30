<script lang="ts">
  import { onMount } from "svelte";
  import Check from "@lucide/svelte/icons/check";
  import Copy from "@lucide/svelte/icons/copy";
  import FolderPlus from "@lucide/svelte/icons/folder-plus";
  import RefreshCw from "@lucide/svelte/icons/refresh-cw";
  import X from "@lucide/svelte/icons/x";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import {
    mcpClientConnect,
    mcpClientDisconnect,
    mcpClientRepair,
    mcpClientPolicySet,
    mcpClientsStatus,
    mcpPolicySet,
  } from "$lib/api";
  import { i18n } from "$lib/stores/i18n.svelte";
  import { settings } from "$lib/stores/settings.svelte";
  import { appErrorMessage, isAppError, type McpClient, type McpClientStatus } from "$lib/types";

  let statuses: McpClientStatus[] = $state([]);
  let loading = $state(false);
  let error = $state("");
  let copied: McpClient | null = $state(null);
  let policySaving = $state(false);

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

  async function savePolicy(next: {
    source?: boolean;
    install?: boolean;
    destructive?: boolean;
    projects?: string[];
  }) {
    const current = settings.effective;
    policySaving = true;
    error = "";
    try {
      settings.data = await mcpPolicySet(
        next.source ?? current.mcpSourceAccess,
        next.install ?? current.mcpInstallAccess,
        next.destructive ?? current.mcpDestructiveAccess,
        next.projects ?? current.mcpProjectAllowlist,
      );
    } catch (cause) {
      error = isAppError(cause) ? appErrorMessage(cause) : i18n.t("settings.mcp.policyFailed");
    } finally {
      policySaving = false;
    }
  }

  async function saveClientPolicy(
    client: McpClient,
    next: Partial<{ sourceAccess: boolean; installAccess: boolean; destructiveAccess: boolean }>,
  ) {
    const global = settings.effective;
    const current = global.mcpClientPolicies[client] ?? {
      sourceAccess: global.mcpSourceAccess,
      installAccess: global.mcpInstallAccess,
      destructiveAccess: global.mcpDestructiveAccess,
    };
    policySaving = true;
    error = "";
    try {
      settings.data = await mcpClientPolicySet(
        client,
        next.sourceAccess ?? current.sourceAccess,
        next.installAccess ?? current.installAccess,
        next.destructiveAccess ?? current.destructiveAccess,
      );
    } catch (cause) {
      error = isAppError(cause) ? appErrorMessage(cause) : i18n.t("settings.mcp.policyFailed");
    } finally {
      policySaving = false;
    }
  }

  async function addProject() {
    const selected = await openDialog({ directory: true, multiple: false });
    if (typeof selected !== "string") return;
    const projects = [...new Set([...settings.effective.mcpProjectAllowlist, selected])];
    await savePolicy({ projects });
  }

  onMount(() => {
    void refresh();
    if (!settings.data) void settings.load();
  });
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
      {@const clientPolicy = settings.effective.mcpClientPolicies[status.client] ?? {
        sourceAccess: settings.effective.mcpSourceAccess,
        installAccess: settings.effective.mcpInstallAccess,
        destructiveAccess: settings.effective.mcpDestructiveAccess,
      }}
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
        <fieldset class="client-policy" disabled={policySaving || settings.loading}>
          <legend>Client permissions</legend>
          <label><input type="checkbox" checked={clientPolicy.sourceAccess} onchange={(event) => saveClientPolicy(status.client, { sourceAccess: event.currentTarget.checked })} /> Sources and organization</label>
          <label><input type="checkbox" checked={clientPolicy.installAccess} onchange={(event) => saveClientPolicy(status.client, { installAccess: event.currentTarget.checked })} /> Install and update</label>
          <label><input type="checkbox" checked={clientPolicy.destructiveAccess} onchange={(event) => saveClientPolicy(status.client, { destructiveAccess: event.currentTarget.checked })} /> Destructive actions</label>
        </fieldset>
        {#if status.command}
          <details>
            <summary>{i18n.t("settings.mcp.manual")}</summary>
            <div class="command">
              <code>{status.command}</code>
              <button type="button" onclick={() => copyCommand(status)} aria-label={`${i18n.t("settings.mcp.copy")} ${clientLabel(status.client)}`}>
                {#if copied === status.client}<Check size={14} />{:else}<Copy size={14} />{/if}
              </button>
            </div>
          </details>
        {/if}
      </article>
    {/each}
  </div>

  <div class="policy" aria-busy={policySaving || settings.loading}>
    <div>
      <h3>{i18n.t("settings.mcp.policy.title")}</h3>
      <p>{i18n.t("settings.mcp.policy.help")}</p>
    </div>
    <label>
      <input type="checkbox" checked={settings.effective.mcpSourceAccess} disabled={policySaving || settings.loading} onchange={(event) => savePolicy({ source: event.currentTarget.checked })} />
      <span>{i18n.t("settings.mcp.policy.sources")}</span>
    </label>
    <label>
      <input type="checkbox" checked={settings.effective.mcpInstallAccess} disabled={policySaving || settings.loading} onchange={(event) => savePolicy({ install: event.currentTarget.checked })} />
      <span>{i18n.t("settings.mcp.policy.installs")}</span>
    </label>
    <label>
      <input type="checkbox" checked={settings.effective.mcpDestructiveAccess} disabled={policySaving || settings.loading} onchange={(event) => savePolicy({ destructive: event.currentTarget.checked })} />
      <span>{i18n.t("settings.mcp.policy.destructive")}</span>
    </label>
    <div class="allowlist">
      <div class="allowlist-head">
        <strong>{i18n.t("settings.mcp.policy.projects")}</strong>
        <button type="button" disabled={policySaving || settings.loading} onclick={addProject}>
          <FolderPlus size={14} /> {i18n.t("settings.mcp.policy.addProject")}
        </button>
      </div>
      {#if settings.effective.mcpProjectAllowlist.length === 0}
        <p>{i18n.t("settings.mcp.policy.noProjects")}</p>
      {:else}
        <ul>
          {#each settings.effective.mcpProjectAllowlist as project (project)}
            <li>
              <code>{project}</code>
              <button
                type="button"
                disabled={policySaving || settings.loading}
                aria-label={`${i18n.t("settings.mcp.policy.removeProject")} ${project}`}
                onclick={() => savePolicy({ projects: settings.effective.mcpProjectAllowlist.filter((item) => item !== project) })}
              ><X size={14} /></button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  </div>
  <span class="sr-only" aria-live="polite">
    {copied ? `${clientLabel(copied)} ${i18n.t("settings.mcp.copied")}` : ""}
  </span>
</section>

<style>
  .section { display: flex; flex-direction: column; gap: var(--space-4); max-width: 650px; }
  .heading, .card-head, .actions, .command, .allowlist-head, .allowlist li { display: flex; align-items: center; }
  .heading, .card-head { justify-content: space-between; gap: var(--space-3); }
  h2 { margin: 0 0 var(--space-1); font-size: var(--text-h1); }
  p { margin: 0; color: var(--color-text-muted); font-size: var(--text-body-sm); }
  .cards { display: grid; gap: var(--space-3); }
  .card { padding: var(--space-4); border: 1px solid var(--color-border); border-radius: var(--radius-md); display: grid; gap: var(--space-3); }
  .policy { padding-top: var(--space-4); border-top: 1px solid var(--color-border); display: grid; gap: var(--space-3); }
  .policy h3 { margin: 0 0 var(--space-1); font-size: var(--text-body); }
  .policy > label { display: flex; align-items: center; gap: var(--space-2); font-size: var(--text-body-sm); }
  .allowlist { display: grid; gap: var(--space-2); }
  .allowlist-head { justify-content: space-between; gap: var(--space-3); }
  .allowlist ul { list-style: none; display: grid; gap: var(--space-2); margin: 0; padding: 0; }
  .allowlist li { gap: var(--space-2); }
  .allowlist li code { flex: 1; }
  .card-head span { font-size: var(--text-caption); color: var(--color-text-muted); }
  .card-head span.ok { color: var(--color-success); }
  .card-head span.warn, .error { color: var(--color-danger); }
  .actions { gap: var(--space-2); }
  .client-policy { display: grid; gap: var(--space-2); margin: 0; padding: var(--space-3); border: 1px solid var(--color-border); border-radius: var(--radius-sm); }
  .client-policy legend { padding: 0 var(--space-1); color: var(--color-text-secondary); font-size: var(--text-caption); }
  .client-policy label { display: flex; align-items: center; gap: var(--space-2); font-size: var(--text-body-sm); }
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
