<script lang="ts">
  import DownloadIcon from "@lucide/svelte/icons/download";

  import Button from "./Button.svelte";
  import DeploymentMatrix from "./DeploymentMatrix.svelte";
  import PersonaBody from "./PersonaBody.svelte";
  import { agentRenderPreview, agentTextRead } from "$lib/api";
  import { nextAgentDetailTab, type AgentDetailTab } from "$lib/agents/libraryModel";
  import { SUPPORTED_TOOLS } from "$lib/stores/install.svelte";
  import { agentLibrary } from "$lib/stores/agentLibrary.svelte";
  import { i18n } from "$lib/stores/i18n.svelte";
  import { appErrorMessage, isAppError, type Agent, type AgentPackageResult, type AgentSource, type Tool } from "$lib/types";

  interface Props {
    agent: Agent;
    pkg: AgentPackageResult | null;
    source: AgentSource | null;
    loading?: boolean;
    catalogDeployment?: boolean;
    onCategory: (slug: string) => void;
    onInstall: () => void;
    onEdit?: () => void;
    onDuplicate?: () => void;
    onDiff: (target: { slug: string; tool: Tool; projectPath: string | null; name: string }) => void;
  }

  let {
    agent,
    pkg,
    source,
    loading = false,
    catalogDeployment = false,
    onCategory,
    onInstall,
    onEdit,
    onDuplicate,
    onDiff,
  }: Props = $props();

  let active = $state<AgentDetailTab>("overview");
  let sourceText = $state("");
  let rendered = $state("");
  let sourceError = $state<string | null>(null);
  let renderError = $state<string | null>(null);
  let selectedTool = $state<Tool>("claudeCode");
  let tablist: HTMLDivElement | undefined = $state();

  const tabs: { id: AgentDetailTab; key: "agents.overview" | "agents.source" | "agents.security" }[] = [
    { id: "overview", key: "agents.overview" },
    { id: "source", key: "agents.source" },
    { id: "security", key: "agents.security" },
  ];

  function selectTab(tab: AgentDetailTab, event?: KeyboardEvent) {
    active = tab;
    if (event) {
      event.preventDefault();
      requestAnimationFrame(() => tablist?.querySelector<HTMLButtonElement>(`[data-agent-tab="${tab}"]`)?.focus());
    }
  }

  function tabKeydown(event: KeyboardEvent) {
    const next = nextAgentDetailTab(active, event.key);
    if (next !== active || event.key === "Home" || event.key === "End") selectTab(next, event);
  }

  $effect(() => {
    const reference = pkg?.reference;
    if (active !== "source" || !reference) return;
    const key = `${reference.sourceId}\0${reference.relativePath}`;
    sourceText = "";
    sourceError = null;
    void agentTextRead(reference).then((text) => {
      if (`${pkg?.reference.sourceId}\0${pkg?.reference.relativePath}` === key) sourceText = text;
    }).catch((error) => {
      sourceError = isAppError(error) ? appErrorMessage(error) : String(error);
    });
  });

  $effect(() => {
    const reference = pkg?.reference;
    const tool = selectedTool;
    if (active !== "source" || !reference || !pkg.installable) return;
    const key = `${reference.sourceId}\0${reference.relativePath}\0${tool}`;
    rendered = "";
    renderError = null;
    void agentRenderPreview(reference, tool).then((text) => {
      if (`${pkg?.reference.sourceId}\0${pkg?.reference.relativePath}\0${selectedTool}` === key) rendered = text;
    }).catch((error) => {
      renderError = isAppError(error) ? appErrorMessage(error) : String(error);
      rendered = "";
    });
  });

  function trustPublisher() {
    if (!pkg?.publisher || !pkg.publisherKey) return;
    void agentLibrary.setPublisherTrust({
      name: pkg.publisher,
      publicKey: pkg.publisherKey,
      trusted: true,
      revoked: false,
    });
  }

  const publisherTrust = $derived(pkg?.publisherKey
    ? agentLibrary.library.publisherTrust.find((item) => item.publicKey === pkg?.publisherKey) ?? null
    : null);
</script>

<div bind:this={tablist} class="detail-tabs" role="tablist" aria-label={i18n.t("agents.detailSections")}>
  {#each tabs as tab (tab.id)}
    <button
      data-agent-tab={tab.id}
      role="tab"
      aria-selected={active === tab.id}
      tabindex={active === tab.id ? 0 : -1}
      class:on={active === tab.id}
      onkeydown={tabKeydown}
      onclick={() => selectTab(tab.id)}
    >{i18n.t(tab.key)}</button>
  {/each}
</div>

<div role="tabpanel" aria-label={i18n.t(tabs.find((tab) => tab.id === active)?.key ?? "agents.overview")}>
  {#if active === "overview"}
    <PersonaBody {agent} {loading} {onCategory}>
      {#snippet headerAction()}
        <Button size="sm" variant="primary" onclick={onInstall}>
          {#snippet icon()}<DownloadIcon size={14} />{/snippet}
          {i18n.t(pkg ? "agents.manageInstallations" : "agents.installAgent")}
        </Button>
        {#if pkg && onEdit}<Button size="sm" onclick={onEdit}>{i18n.t("agents.editAsDraft")}</Button>{/if}
        {#if pkg && onDuplicate}<Button size="sm" onclick={onDuplicate}>{i18n.t("agents.duplicateDraft")}</Button>{/if}
      {/snippet}
      {#snippet deploy()}
        {#if catalogDeployment}
          <DeploymentMatrix {agent} onDiff={onDiff} />
        {/if}
      {/snippet}
    </PersonaBody>
  {:else if active === "source"}
    <section class="panel">
      <h2>{i18n.t("agents.sourceProvenance")}</h2>
      {#if pkg && source}
        <dl>
          <dt>{i18n.t("agents.source")}</dt><dd>{source.label} · {i18n.t(`agents.sourceKind.${source.kind.kind}`)}</dd>
          <dt>{i18n.t("agents.identity")}</dt><dd>{pkg.reference.sourceId} · {pkg.reference.relativePath}</dd>
          <dt>{i18n.t("agents.version")}</dt><dd>{pkg.version ?? i18n.t("agents.unversioned")}</dd>
          <dt>{i18n.t("agents.sourceHash")}</dt><dd><code>{pkg.sourceHash}</code></dd>
        </dl>
        <label class="tool-label">
          <span>{i18n.t("agents.renderedPreview")}</span>
          <select bind:value={selectedTool} aria-label={i18n.t("agents.previewTool")}>
            {#each SUPPORTED_TOOLS as tool (tool.id)}<option value={tool.id}>{tool.label}</option>{/each}
          </select>
        </label>
        {#if renderError}<p class="error" role="alert">{i18n.t("agents.renderFailed")}: {renderError}</p>
        {:else}<pre aria-label={i18n.t("agents.renderedPreview")}>{rendered}</pre>{/if}
        <h3>{i18n.t("agents.sourceMarkdown")}</h3>
        {#if sourceError}<p class="error" role="alert">{sourceError}</p>
        {:else}<pre aria-label={i18n.t("agents.sourceMarkdown")}>{sourceText}</pre>{/if}
      {:else}
        <p>{i18n.t("agents.catalogSourceHelp")}</p>
      {/if}
    </section>
  {:else}
    <section class="panel">
      <h2>{i18n.t("agents.security")}</h2>
      {#if pkg}
        <p class:ok={pkg.installable} class:error={!pkg.installable}>
          {i18n.t(pkg.installable ? "agents.validationPassed" : "agents.validationFailed")}
        </p>
        <p>{i18n.t("agents.qualityScore", { score: pkg.qualityScore })}</p>
        {#if pkg.qualityChecks.length}<ul>{#each pkg.qualityChecks as check}<li>{check}</li>{/each}</ul>{/if}
        <h3>{i18n.t("agents.capabilities")}</h3>
        <p>{pkg.capabilities.length ? pkg.capabilities.join(", ") : i18n.t("common.none")}</p>
        <h3>{i18n.t("agents.permissions")}</h3>
        <p>{pkg.permissions.length ? pkg.permissions.join(", ") : i18n.t("agents.noElevatedPermissions")}</p>
        <h3>{i18n.t("agents.dependencies")}</h3>
        <p>{pkg.requiredAgents.length ? pkg.requiredAgents.join(", ") : i18n.t("common.none")}</p>
        <h3>{i18n.t("agents.requiredSkills")}</h3>
        <p>{pkg.requiredSkills.length ? pkg.requiredSkills.join(", ") : i18n.t("common.none")}</p>
        <h3>{i18n.t("agents.recommendations")}</h3>
        <p>{pkg.recommendedAgents.length ? pkg.recommendedAgents.join(", ") : i18n.t("common.none")}</p>
        {#if pkg.publisher}
          <h3>{i18n.t("agents.publisher")}</h3>
          <p>{pkg.publisher} · {pkg.publisherVerified ? i18n.t("agents.signatureVerified") : i18n.t("agents.signatureUnverified")}</p>
          <p>{publisherTrust?.revoked ? i18n.t("agents.publisherRevoked") : publisherTrust?.trusted ? i18n.t("agents.publisherTrusted") : i18n.t("agents.publisherUntrusted")}</p>
          {#if pkg.publisherVerified && pkg.publisherKey && !publisherTrust?.trusted}
            <Button size="sm" onclick={trustPublisher}>{i18n.t("agents.trustPublisher")}</Button>
          {/if}
        {/if}
        {#if pkg.diagnostics.length}
          <h3>{i18n.t("agents.diagnostics")}</h3>
          <ul>{#each pkg.diagnostics as diagnostic}<li>{diagnostic.path}: {diagnostic.message}</li>{/each}</ul>
        {/if}
      {:else}
        <p>{i18n.t("agents.catalogValidationHelp")}</p>
      {/if}
    </section>
  {/if}
</div>

<style>
  .detail-tabs { display: flex; gap: 2px; padding: 0 var(--space-4); border-bottom: 1px solid var(--color-border); }
  .detail-tabs button { padding: var(--space-2) var(--space-3); color: var(--color-text-muted); border-bottom: 2px solid transparent; }
  .detail-tabs button.on { color: var(--color-text-primary); border-bottom-color: var(--color-brand); }
  .panel { display: flex; flex-direction: column; gap: var(--space-3); padding: var(--space-4); }
  h2 { font-size: var(--text-h2); } h3 { font-size: var(--text-body); font-weight: var(--fw-semibold); }
  dl { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: var(--space-2) var(--space-3); }
  dt { color: var(--color-text-muted); } dd { min-width: 0; overflow-wrap: anywhere; }
  .tool-label { display: flex; justify-content: space-between; align-items: center; gap: var(--space-3); }
  select { border: 1px solid var(--color-border); border-radius: var(--radius-md); background: var(--color-surface-sunken); padding: 5px 8px; }
  pre { max-height: 320px; overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; padding: var(--space-3); border: 1px solid var(--color-border); border-radius: var(--radius-md); background: var(--color-surface-sunken); font: var(--text-caption)/1.5 var(--font-mono); }
  ul { padding-left: var(--space-5); list-style: disc; }
  .ok { color: var(--color-success); } .error { color: var(--color-danger); }
</style>
