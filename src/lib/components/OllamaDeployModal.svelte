<script lang="ts">
  import { onMount } from "svelte";
  import Modal from "./Modal.svelte";
  import Button from "./Button.svelte";
  import { ollamaApply, ollamaPlan, ollamaStatus } from "$lib/api";
  import { activity, safeActivityDetail } from "$lib/stores/activity.svelte";
  import { i18n } from "$lib/stores/i18n.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import { ui } from "$lib/stores/ui.svelte";
  import { appErrorMessage, isAppError } from "$lib/types";
  import type { Agent, AgentPackageResult, OllamaMutationPlan, OllamaStatus } from "$lib/types";

  interface Props {
    pkg: AgentPackageResult;
    agent: Agent;
    onClose: () => void;
  }

  let { pkg, agent, onClose }: Props = $props();
  let status = $state<OllamaStatus | null>(null);
  let statusError = $state<string | null>(null);
  let loading = $state(false);
  let busy = $state(false);
  let baseModel = $state<string | null>(null);
  let plan = $state<OllamaMutationPlan | null>(null);

  const deployment = $derived(status?.deployments.find(({ record }) =>
    record.reference.sourceId === pkg.reference.sourceId
    && record.reference.relativePath === pkg.reference.relativePath
  ) ?? null);
  const operation = $derived<OllamaMutationPlan["operation"]>(deployment ? "update" : "create");
  const truthReady = $derived(Boolean(status) && !loading && !statusError && !busy);

  function message(error: unknown): string {
    return isAppError(error) ? appErrorMessage(error) : error instanceof Error ? error.message : String(error);
  }

  function stateLabel(state: string): string {
    return i18n.optional(`state.${state}`, state);
  }

  async function refresh() {
    loading = true;
    try {
      const next = await ollamaStatus();
      status = next;
      statusError = null;
      const trackedBase = next.deployments.find(({ record }) =>
        record.reference.sourceId === pkg.reference.sourceId
        && record.reference.relativePath === pkg.reference.relativePath
      )?.record.baseModel;
      if (!baseModel || !next.models.some((model) => model.name === baseModel)) {
        baseModel = trackedBase && next.models.some((model) => model.name === trackedBase)
          ? trackedBase
          : next.models[0]?.name ?? null;
      }
      plan = null;
    } catch (error) {
      statusError = message(error);
    } finally {
      loading = false;
    }
  }

  function selectBase(event: Event) {
    baseModel = (event.currentTarget as HTMLSelectElement).value || null;
    plan = null;
  }

  async function review(nextOperation: OllamaMutationPlan["operation"]) {
    if (!truthReady || (nextOperation !== "remove" && !baseModel)) return;
    busy = true;
    try {
      const operation = nextOperation;
      plan = await ollamaPlan(pkg.reference, operation, baseModel);
    } catch (error) {
      toast.error(i18n.optional("ollama.planFailed", "Could not review local model change"), message(error));
    } finally {
      busy = false;
    }
  }

  function logResult(
    operation: OllamaMutationPlan["operation"],
    targetName: string | null,
    outcome: "ok" | "error",
    error?: unknown,
  ): string {
    const action = operation === "create" ? "install" : operation === "remove" ? "uninstall" : "update";
    const detail = error == null ? `${operation} · ${targetName ?? "unknown target"}` : safeActivityDetail(error);
    return activity.log({
      action,
      subject: "agent",
      subjectName: agent.name,
      agentSlug: agent.slug,
      agentName: agent.name,
      outcome,
      detail,
      receipt: {
        operation: action,
        succeeded: outcome === "ok" ? 1 : 0,
        failed: outcome === "error" ? 1 : 0,
        items: [{
          kind: "agent",
          name: agent.name,
          destination: targetName,
          outcome,
          ...(error == null ? {} : { detail }),
        }],
      },
    });
  }

  async function apply() {
    if (!plan || !truthReady || plan.blockers.length) return;
    busy = true;
    try {
      const result = await ollamaApply(pkg.reference, plan.operation, plan.baseModel, plan.revision);
      const receiptId = logResult(plan.operation, result.targetName, "ok");
      toast.success(
        i18n.optional("ollama.applied", "Local model change applied"),
        result.targetName,
        { label: i18n.t("activity.viewReceipt"), onClick: () => ui.openActivityReceipt(receiptId) },
      );
      await refresh();
    } catch (error) {
      const receiptId = logResult(plan.operation, plan.targetName, "error", error);
      toast.error(
        i18n.optional("ollama.applyFailed", "Local model change failed"),
        message(error),
        { label: i18n.t("activity.viewReceipt"), onClick: () => ui.openActivityReceipt(receiptId) },
      );
      plan = null;
    } finally {
      busy = false;
    }
  }

  onMount(refresh);
</script>

<Modal
  open
  size="wide"
  dismissible={!busy}
  title={i18n.optional("ollama.title", "Local Ollama model", { name: agent.name })}
  onClose={onClose}
>
  <div class="stack">
    <dl>
      <dt>{i18n.optional("ollama.scope", "Scope")}</dt><dd>{i18n.optional("ollama.thisDevice", "This device")}</dd>
      <dt>{i18n.optional("ollama.agent", "Agent")}</dt><dd>{agent.name}</dd>
      {#if deployment}
        <dt>{i18n.optional("ollama.target", "Target")}</dt><dd><code>{deployment.record.targetName}</code></dd>
        <dt>{i18n.optional("ollama.state", "State")}</dt><dd>{stateLabel(deployment.state)}</dd>
      {/if}
    </dl>

    {#if statusError}
      <div class="notice error" role="alert">
        <p>{i18n.optional("ollama.stale", "Local model status is stale. Changes are blocked until retry succeeds.")}</p>
        <p>{statusError}</p>
        <Button size="sm" loading={loading} onclick={refresh}>{i18n.optional("common.retry", "Retry")}</Button>
      </div>
    {:else if loading && !status}
      <p role="status">{i18n.optional("ollama.checking", "Checking local Ollama models…")}</p>
    {:else if status}
      {#if status.models.length}
        <label>
          <span>{i18n.optional("ollama.baseModel", "Base model")}</span>
          <select value={baseModel ?? ""} onchange={selectBase} disabled={!truthReady || Boolean(plan)}>
            {#each status.models as model (model.name)}
              <option value={model.name}>{model.name}</option>
            {/each}
          </select>
        </label>
      {:else}
        <p class="notice">{i18n.optional("ollama.noModels", "No eligible local base models are installed. Agency Agents will not download one.")}</p>
      {/if}

      {#if plan}
        <section class="review" aria-label={i18n.optional("ollama.review", "Deployment review")}>
          <dl>
            <dt>{i18n.optional("ollama.operation", "Operation")}</dt><dd>{plan.operation}</dd>
            <dt>{i18n.optional("ollama.target", "Target")}</dt><dd><code>{plan.targetName}</code></dd>
            <dt>{i18n.optional("ollama.baseModel", "Base model")}</dt><dd>{plan.baseModel ?? i18n.optional("common.none", "None")}</dd>
            <dt>{i18n.optional("ollama.state", "State")}</dt><dd>{plan.state ? stateLabel(plan.state) : i18n.optional("ollama.notDeployed", "Not deployed")}</dd>
            <dt>{i18n.optional("ollama.rollback", "Rollback")}</dt><dd>{plan.rollbackAvailable ? i18n.optional("common.available", "Available") : i18n.optional("common.unavailable", "Unavailable")}</dd>
          </dl>
          {#if plan.promptPreview != null}
            <h2>{i18n.optional("ollama.systemPrompt", "Exact system prompt")}</h2>
            <pre>{plan.promptPreview}</pre>
          {/if}
          {#if plan.warnings.length}
            <div class="notice warning"><strong>{i18n.optional("common.warning", "Warning")}</strong><ul>{#each plan.warnings as warning}<li>{warning}</li>{/each}</ul></div>
          {/if}
          {#if plan.blockers.length}
            <div class="notice error" role="alert"><strong>{i18n.optional("ollama.blocked", "Blocked")}</strong><ul>{#each plan.blockers as blocker}<li>{blocker}</li>{/each}</ul></div>
          {/if}
        </section>
      {/if}
    {/if}
  </div>

  {#snippet actions()}
    <Button modalAction="cancel" onclick={onClose} disabled={busy}>{i18n.t("common.close")}</Button>
    {#if plan}
      <Button variant="primary" modalAction="confirm" loading={busy} disabled={!truthReady || plan.blockers.length > 0} onclick={apply}>
        {i18n.optional("ollama.confirm", "Confirm change")}
      </Button>
    {:else if status}
      {#if deployment}
        <Button variant="danger" disabled={!truthReady} onclick={() => review("remove")}>{i18n.optional("ollama.reviewRemoval", "Review removal")}</Button>
      {/if}
      <Button variant="primary" loading={busy} disabled={!truthReady || !baseModel} onclick={() => review(operation)}>
        {i18n.optional("ollama.reviewChange", "Review change")}
      </Button>
    {/if}
  {/snippet}
</Modal>

<style>
  .stack, .review { display: flex; flex-direction: column; gap: var(--space-3); }
  dl { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: var(--space-2) var(--space-3); }
  dt { color: var(--color-text-muted); }
  dd { min-width: 0; overflow-wrap: anywhere; }
  label { display: flex; align-items: center; justify-content: space-between; gap: var(--space-3); }
  select { min-width: 240px; border: 1px solid var(--color-border); border-radius: var(--radius-md); background: var(--color-surface-sunken); padding: 5px 8px; }
  h2 { font-size: var(--text-body); font-weight: var(--fw-semibold); }
  pre { max-height: 300px; overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; padding: var(--space-3); border: 1px solid var(--color-border); border-radius: var(--radius-md); background: var(--color-surface-sunken); font: var(--text-caption)/1.5 var(--font-mono); }
  .notice { padding: var(--space-3); border: 1px solid var(--color-border); border-radius: var(--radius-md); background: var(--color-surface-sunken); }
  .notice.error { border-color: var(--color-danger); color: var(--color-danger); }
  .notice.warning { border-color: var(--color-warning); }
  ul { padding-left: var(--space-5); list-style: disc; }
</style>
