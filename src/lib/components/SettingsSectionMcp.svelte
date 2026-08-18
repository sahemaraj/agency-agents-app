<script lang="ts">
  import { onMount, tick } from "svelte";
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
    mcpAgentClientPolicySet,
    mcpAgentPolicySet,
    mcpClientPolicySet,
    mcpClientsStatus,
    mcpInventory,
    mcpPolicySet,
  } from "$lib/api";
  import { i18n } from "$lib/stores/i18n.svelte";
  import { settings } from "$lib/stores/settings.svelte";
  import { appErrorMessage, classifySecurityPosture, isAppError, previewSecurityPosture, type McpClient, type McpClientStatus, type McpInventoryReport, type McpInventoryServer, type SecurityPosturePreset, type Settings } from "$lib/types";

  let statuses: McpClientStatus[] = $state([]);
  let inventory: McpInventoryReport = $state({ servers: [], trustedTemplates: [], issues: [] });
  let loading = $state(false);
  let error = $state("");
  let announcement = $state("");
  let copied: McpClient | null = $state(null);
  let policySaving = $state(false);
  let selectedPreset: SecurityPosturePreset = $state("strict");
  let presetAnnouncement = $state("");
  let presetApplyButton: HTMLButtonElement | undefined = $state();
  let currentPosture = $derived(classifySecurityPosture(settings.effective));
  let presetPreview = $derived(previewSecurityPosture(settings.effective, selectedPreset));

  const postureLabels = {
    strict: "Strict",
    localDevelopment: "Local Development",
    custom: "Custom",
  } as const;

  function onOff(value: boolean): string {
    return value ? "On" : "Off";
  }

  function clientPolicySummary(
    source: Settings,
    client: McpClient,
    kind: "skills" | "agents",
  ): string {
    const policy = source.mcpClientPolicies[client];
    if (!policy) return "Inherit";
    const values = kind === "skills"
      ? [policy.sourceAccess, policy.installAccess, policy.destructiveAccess]
      : [policy.agentSourceAccess, policy.agentInstallAccess, policy.agentDestructiveAccess];
    return values.map(onOff).join(" / ");
  }

  async function applyPreset() {
    const preset = selectedPreset;
    error = "";
    presetAnnouncement = "";
    await settings.applySecurityPosture(preset);
    if (settings.error) {
      error = settings.error;
      presetAnnouncement = `${postureLabels[preset]} security posture failed: ${settings.error}`;
    } else {
      presetAnnouncement = `${postureLabels[preset]} security posture applied`;
    }
    await tick();
    presetApplyButton?.focus({ preventScroll: true });
  }

  function clientId(status: McpClientStatus): McpClient {
    return status.client;
  }

  function clientLabel(client: McpClient): string {
    return client === "claude" ? "Claude Code" : "Codex";
  }

  function stateLabel(state: McpClientStatus["state"]): string {
    return i18n.t(`settings.mcp.status.${state}`);
  }

  function scopeLabel(scope: McpInventoryServer["scope"]): string {
    return i18n.t(`settings.mcp.inventory.source.${scope}`);
  }

  function validationLabel(validation: McpInventoryServer["validation"]): string {
    return i18n.t(`settings.mcp.inventory.validation.${validation}`);
  }

  async function refresh() {
    loading = true;
    error = "";
    announcement = "";
    const [statusResult, inventoryResult] = await Promise.allSettled([
      mcpClientsStatus(),
      mcpInventory(),
    ]);
    if (statusResult.status === "fulfilled") statuses = statusResult.value;
    else error = i18n.t("settings.mcp.loadFailed");
    if (inventoryResult.status === "fulfilled") {
      inventory = inventoryResult.value;
      announcement = i18n.t(inventory.issues.length
        ? "settings.mcp.inventory.completeWithIssues"
        : "settings.mcp.inventory.complete");
    } else {
      error = error || i18n.t("settings.mcp.inventory.loadFailed");
      announcement = i18n.t("settings.mcp.inventory.loadFailed");
    }
    loading = false;
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

  async function saveAgentPolicy(next: {
    source?: boolean;
    install?: boolean;
    destructive?: boolean;
  }) {
    const current = settings.effective;
    policySaving = true;
    error = "";
    try {
      settings.data = await mcpAgentPolicySet(
        next.source ?? current.mcpAgentSourceAccess,
        next.install ?? current.mcpAgentInstallAccess,
        next.destructive ?? current.mcpAgentDestructiveAccess,
      );
    } catch (cause) {
      error = isAppError(cause) ? appErrorMessage(cause) : i18n.t("settings.mcp.policyFailed");
    } finally {
      policySaving = false;
    }
  }

  async function saveAgentClientPolicy(
    client: McpClient,
    next: Partial<{ sourceAccess: boolean; installAccess: boolean; destructiveAccess: boolean }>,
  ) {
    const global = settings.effective;
    const current = global.mcpClientPolicies[client] ?? {
      sourceAccess: global.mcpSourceAccess,
      installAccess: global.mcpInstallAccess,
      destructiveAccess: global.mcpDestructiveAccess,
      agentSourceAccess: global.mcpAgentSourceAccess,
      agentInstallAccess: global.mcpAgentInstallAccess,
      agentDestructiveAccess: global.mcpAgentDestructiveAccess,
    };
    policySaving = true;
    error = "";
    try {
      settings.data = await mcpAgentClientPolicySet(
        client,
        next.sourceAccess ?? current.agentSourceAccess,
        next.installAccess ?? current.agentInstallAccess,
        next.destructiveAccess ?? current.agentDestructiveAccess,
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
    <button type="button" class="icon-button" onclick={refresh} disabled={loading} aria-label={i18n.t("settings.mcp.inventory.refresh")}>
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
        agentSourceAccess: settings.effective.mcpAgentSourceAccess,
        agentInstallAccess: settings.effective.mcpAgentInstallAccess,
        agentDestructiveAccess: settings.effective.mcpAgentDestructiveAccess,
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
          <legend>{i18n.t("settings.mcp.policy.skills")}</legend>
          <label><input type="checkbox" checked={clientPolicy.sourceAccess} onchange={(event) => saveClientPolicy(status.client, { sourceAccess: event.currentTarget.checked })} /> {i18n.t("settings.mcp.policy.sources")}</label>
          <label><input type="checkbox" checked={clientPolicy.installAccess} onchange={(event) => saveClientPolicy(status.client, { installAccess: event.currentTarget.checked })} /> {i18n.t("settings.mcp.policy.installs")}</label>
          <label><input type="checkbox" checked={clientPolicy.destructiveAccess} onchange={(event) => saveClientPolicy(status.client, { destructiveAccess: event.currentTarget.checked })} /> {i18n.t("settings.mcp.policy.destructive")}</label>
        </fieldset>
        <fieldset class="client-policy" disabled={policySaving || settings.loading}>
          <legend>{i18n.t("settings.mcp.policy.agents")}</legend>
          <label><input type="checkbox" checked={clientPolicy.agentSourceAccess} onchange={(event) => saveAgentClientPolicy(status.client, { sourceAccess: event.currentTarget.checked })} /> {i18n.t("settings.mcp.policy.agentSources")}</label>
          <label><input type="checkbox" checked={clientPolicy.agentInstallAccess} onchange={(event) => saveAgentClientPolicy(status.client, { installAccess: event.currentTarget.checked })} /> {i18n.t("settings.mcp.policy.agentInstalls")}</label>
          <label><input type="checkbox" checked={clientPolicy.agentDestructiveAccess} onchange={(event) => saveAgentClientPolicy(status.client, { destructiveAccess: event.currentTarget.checked })} /> {i18n.t("settings.mcp.policy.agentDestructive")}</label>
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

  <section class="inventory" aria-labelledby="mcp-inventory-title" aria-busy={loading}>
    <div>
      <h3 id="mcp-inventory-title">{i18n.t("settings.mcp.inventory.title")}</h3>
      <p>{i18n.t("settings.mcp.inventory.help")}</p>
    </div>
    {#if inventory.issues.length}
      <ul class="issues" aria-label={i18n.t("settings.mcp.inventory.issues")}>
        {#each inventory.issues as issue (issue)}<li>{issue}</li>{/each}
      </ul>
    {/if}
    {#if inventory.servers.length === 0}
      <p>{i18n.t("settings.mcp.inventory.empty")}</p>
    {:else}
      <div class="inventory-cards">
        {#each inventory.servers as server (`${server.client}:${server.scope}:${server.projectPath ?? ""}:${server.name}`)}
          <article class="inventory-card" data-server={server.name}>
            <div class="card-head">
              <strong>{server.name}</strong>
              <span class:ok={server.validation === "valid"} class:warn={server.validation !== "valid"}>
                {validationLabel(server.validation)}
              </span>
            </div>
            <p>{clientLabel(server.client)} · {scopeLabel(server.scope)} · {server.transport}</p>
            {#if server.projectPath}<code>{server.projectPath}</code>{/if}
            <dl>
              <div><dt>{i18n.t("settings.mcp.inventory.endpoint")}</dt><dd>{server.endpoint}</dd></div>
              {#if server.environmentKeys.length}
                <div><dt>{i18n.t("settings.mcp.inventory.environment")}</dt><dd>{server.environmentKeys.join(", ")}</dd></div>
              {/if}
              <div>
                <dt>{i18n.t("settings.mcp.inventory.tools")}</dt>
                <dd>{server.toolDiscovery === "unavailable" ? i18n.t("settings.mcp.inventory.toolsUnavailable") : server.toolNames.join(", ")}</dd>
              </div>
            </dl>
            {#if server.warnings.length || server.blockers.length}
              <ul class="findings">
                {#each [...server.blockers, ...server.warnings] as finding (finding)}<li>{finding}</li>{/each}
              </ul>
            {/if}
          </article>
        {/each}
      </div>
    {/if}
    {#each inventory.trustedTemplates as template (template.id)}
      <article class="template">
        <div class="card-head"><strong>{template.name}</strong><span>{i18n.t("settings.mcp.inventory.template")}</span></div>
        {#if template.automaticConfiguration}<p>{i18n.t("settings.mcp.inventory.automatic")}</p>{/if}
        <p>{template.toolNames.join(", ")}</p>
      </article>
    {/each}
  </section>

  <section class="preset" aria-labelledby="security-posture-title" aria-busy={settings.loading}>
    <div>
      <h3 id="security-posture-title">Security posture</h3>
      <p>Apply the complete network and MCP mutation policy as one atomic change.</p>
      <p data-security-posture-current>Current: <strong>{postureLabels[currentPosture]}</strong></p>
    </div>
    <div class="preset-options" role="group" aria-label="Security posture preset">
      {#each ["strict", "localDevelopment"] as preset (preset)}
        <button
          type="button"
          data-security-posture={preset}
          aria-pressed={selectedPreset === preset}
          disabled={settings.loading}
          onclick={() => selectedPreset = preset as SecurityPosturePreset}
        >{postureLabels[preset as SecurityPosturePreset]}</button>
      {/each}
    </div>
    <table class="preset-preview" data-security-posture-preview>
      <thead><tr><th scope="col">Setting</th><th scope="col">Before</th><th scope="col">After</th></tr></thead>
      <tbody>
        {#each [
          ["Offline mode", settings.effective.paranoidMode, presetPreview.paranoidMode],
          ["GitHub access", settings.effective.githubEnabled, presetPreview.githubEnabled],
          ["Automatic update checks", settings.effective.updateAutoCheck, presetPreview.updateAutoCheck],
          ["Drift notifications", settings.effective.driftNotifications, presetPreview.driftNotifications],
          ["Skill source mutations", settings.effective.mcpSourceAccess, presetPreview.mcpSourceAccess],
          ["Skill install mutations", settings.effective.mcpInstallAccess, presetPreview.mcpInstallAccess],
          ["Skill destructive mutations", settings.effective.mcpDestructiveAccess, presetPreview.mcpDestructiveAccess],
          ["Agent source mutations", settings.effective.mcpAgentSourceAccess, presetPreview.mcpAgentSourceAccess],
          ["Agent install mutations", settings.effective.mcpAgentInstallAccess, presetPreview.mcpAgentInstallAccess],
          ["Agent destructive mutations", settings.effective.mcpAgentDestructiveAccess, presetPreview.mcpAgentDestructiveAccess],
        ] as row (row[0] as string)}
          <tr><th scope="row">{row[0]}</th><td>{onOff(row[1] as boolean)}</td><td>{onOff(row[2] as boolean)}</td></tr>
        {/each}
        <tr><th scope="row">Client overrides</th><td>{Object.keys(settings.effective.mcpClientPolicies).length} configured</td><td>None</td></tr>
        <tr><th scope="row">Claude Skill override</th><td>{clientPolicySummary(settings.effective, "claude", "skills")}</td><td>{clientPolicySummary(presetPreview, "claude", "skills")}</td></tr>
        <tr><th scope="row">Claude Agent override</th><td>{clientPolicySummary(settings.effective, "claude", "agents")}</td><td>{clientPolicySummary(presetPreview, "claude", "agents")}</td></tr>
        <tr><th scope="row">Codex Skill override</th><td>{clientPolicySummary(settings.effective, "codex", "skills")}</td><td>{clientPolicySummary(presetPreview, "codex", "skills")}</td></tr>
        <tr><th scope="row">Codex Agent override</th><td>{clientPolicySummary(settings.effective, "codex", "agents")}</td><td>{clientPolicySummary(presetPreview, "codex", "agents")}</td></tr>
        <tr><th scope="row">Project allowlist</th><td>{settings.effective.mcpProjectAllowlist.length} retained</td><td>{presetPreview.mcpProjectAllowlist.length} retained</td></tr>
      </tbody>
    </table>
    <button bind:this={presetApplyButton} type="button" class="primary" data-security-posture-apply disabled={settings.loading} onclick={applyPreset}>
      Apply {postureLabels[selectedPreset]}
    </button>
    <span class="sr-only" role="status" aria-live="polite" aria-atomic="true" data-security-posture-announcement>{presetAnnouncement}</span>
  </section>

  <div class="policy" aria-busy={policySaving || settings.loading}>
    <div>
      <h3>{i18n.t("settings.mcp.policy.title")}</h3>
      <p>{i18n.t("settings.mcp.policy.help")}</p>
    </div>
    <fieldset class="client-policy" disabled={policySaving || settings.loading}>
      <legend>{i18n.t("settings.mcp.policy.skills")}</legend>
      <label><input type="checkbox" checked={settings.effective.mcpSourceAccess} onchange={(event) => savePolicy({ source: event.currentTarget.checked })} /> {i18n.t("settings.mcp.policy.sources")}</label>
      <label><input type="checkbox" checked={settings.effective.mcpInstallAccess} onchange={(event) => savePolicy({ install: event.currentTarget.checked })} /> {i18n.t("settings.mcp.policy.installs")}</label>
      <label><input type="checkbox" checked={settings.effective.mcpDestructiveAccess} onchange={(event) => savePolicy({ destructive: event.currentTarget.checked })} /> {i18n.t("settings.mcp.policy.destructive")}</label>
    </fieldset>
    <fieldset class="client-policy" disabled={policySaving || settings.loading}>
      <legend>{i18n.t("settings.mcp.policy.agents")}</legend>
      <label><input type="checkbox" checked={settings.effective.mcpAgentSourceAccess} onchange={(event) => saveAgentPolicy({ source: event.currentTarget.checked })} /> {i18n.t("settings.mcp.policy.agentSources")}</label>
      <label><input type="checkbox" checked={settings.effective.mcpAgentInstallAccess} onchange={(event) => saveAgentPolicy({ install: event.currentTarget.checked })} /> {i18n.t("settings.mcp.policy.agentInstalls")}</label>
      <label><input type="checkbox" checked={settings.effective.mcpAgentDestructiveAccess} onchange={(event) => saveAgentPolicy({ destructive: event.currentTarget.checked })} /> {i18n.t("settings.mcp.policy.agentDestructive")}</label>
    </fieldset>
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
  <span class="sr-only" aria-live="polite" data-inventory-announcement>{announcement}</span>
</section>

<style>
  .section { display: flex; flex-direction: column; gap: var(--space-4); max-width: 650px; }
  .heading, .card-head, .actions, .command, .allowlist-head, .allowlist li { display: flex; align-items: center; }
  .heading, .card-head { justify-content: space-between; gap: var(--space-3); }
  h2 { margin: 0 0 var(--space-1); font-size: var(--text-h1); }
  p { margin: 0; color: var(--color-text-muted); font-size: var(--text-body-sm); }
  .cards { display: grid; gap: var(--space-3); }
  .card { padding: var(--space-4); border: 1px solid var(--color-border); border-radius: var(--radius-md); display: grid; gap: var(--space-3); }
  .policy, .inventory, .preset { padding-top: var(--space-4); border-top: 1px solid var(--color-border); display: grid; gap: var(--space-3); }
  .policy h3, .inventory h3, .preset h3 { margin: 0 0 var(--space-1); font-size: var(--text-body); }
  .preset-options { display: flex; gap: var(--space-2); }
  .preset-options button[aria-pressed="true"] { border-color: var(--color-accent); color: var(--color-accent); }
  .preset-preview { width: 100%; border-collapse: separate; border-spacing: 0; border: 1px solid var(--color-border); border-radius: var(--radius-sm); overflow: hidden; font-size: var(--text-caption); }
  .preset-preview th, .preset-preview td { padding: var(--space-2) var(--space-3); text-align: left; font-weight: 400; }
  .preset-preview thead th { background: var(--color-surface-sunken); font-weight: 600; }
  .preset-preview tbody th { width: 55%; }
  .preset-preview tbody tr > * { border-top: 1px solid var(--color-border); }
  .inventory-cards { display: grid; gap: var(--space-2); }
  .inventory-card, .template { padding: var(--space-3); border: 1px solid var(--color-border); border-radius: var(--radius-sm); display: grid; gap: var(--space-2); }
  dl, dl div { margin: 0; display: grid; gap: var(--space-1); }
  dt { color: var(--color-text-muted); font-size: var(--text-caption); }
  dd { margin: 0; overflow-wrap: anywhere; font-size: var(--text-body-sm); }
  .issues, .findings { margin: 0; padding-left: var(--space-5); color: var(--color-danger); font-size: var(--text-body-sm); }
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
