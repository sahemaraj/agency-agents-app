<script lang="ts">
  import { onMount } from "svelte";
  import Copy from "@lucide/svelte/icons/copy";
  import RefreshCw from "@lucide/svelte/icons/refresh-cw";
  import { doctorReport } from "$lib/api";
  import { i18n } from "$lib/stores/i18n.svelte";
  import { ui } from "$lib/stores/ui.svelte";
  import { appErrorMessage, isAppError, type DoctorAction, type DoctorCategory, type DoctorClassification, type DoctorReport } from "$lib/types";

  const CATEGORIES: DoctorCategory[] = ["core", "library", "installations", "tools", "integrations", "updates"];
  let report: DoctorReport | null = $state(null);
  let loading = $state(false);
  let error = $state("");
  let announcement = $state("");

  const label = (classification: DoctorClassification) => i18n.t(`settings.doctor.${classification}`);
  const categoryLabel = (category: DoctorCategory) => i18n.t(`settings.doctor.category.${category}`);
  const actionLabel = (action: DoctorAction) => action === "retryDoctor"
    ? i18n.t("settings.doctor.retry")
    : i18n.t(`settings.doctor.${action}`);

  async function refresh() {
    if (loading) return;
    loading = true;
    error = "";
    announcement = report ? i18n.t("settings.doctor.prior") : i18n.t("settings.doctor.running");
    try {
      report = await doctorReport();
      announcement = `${label("healthy")} ${report.counts.healthy}. ${label("needsAttention")} ${report.counts.needsAttention}. ${label("unavailable")} ${report.counts.unavailable}.`;
    } catch (cause) {
      const message = isAppError(cause) ? appErrorMessage(cause) : String(cause);
      error = i18n.t("settings.doctor.loadFailed", { message });
      announcement = error;
    } finally {
      loading = false;
    }
  }

  async function copyReport() {
    if (!report) return;
    error = "";
    try {
      await navigator.clipboard.writeText(report.copyText);
      announcement = i18n.t("settings.doctor.copied");
    } catch {
      error = i18n.t("settings.doctor.copyFailed");
      announcement = error;
    }
  }

  function runAction(action: DoctorAction) {
    if (action === "retryDoctor") return void refresh();
    if (action === "openCatalog" || action === "openMcp" || action === "openNetwork") {
      ui.openSettings(action === "openCatalog" ? "catalog" : action === "openMcp" ? "mcp" : "network");
      return;
    }
    ui.closeSettings();
    if (action === "openAgents") ui.openAgents(null, "attention");
    else if (action === "openSkills") ui.setSection("skills");
    else if (action === "openTools") ui.openTools();
    requestAnimationFrame(() => {
      const target = document.querySelector<HTMLElement>("main h1, main h2, main button, main [tabindex]");
      if (!target) return;
      if (!target.matches("button, [href], input, select, textarea, [tabindex]")) target.tabIndex = -1;
      target.focus();
    });
  }

  onMount(() => void refresh());
</script>

<section class="section" aria-labelledby="doctor-title">
  <div class="heading">
    <div>
      <h2 id="doctor-title">{i18n.t("settings.doctor.title")}</h2>
      <p>{i18n.t("settings.doctor.help")}</p>
    </div>
    <div class="controls">
      <button type="button" data-doctor-refresh aria-keyshortcuts="Enter Space" onclick={refresh} disabled={loading}>
        <span class:spin={loading} aria-hidden="true"><RefreshCw size={14} /></span>
        {i18n.t("settings.doctor.refresh")}
      </button>
      <button type="button" data-doctor-copy onclick={copyReport} disabled={!report}>
        <Copy size={14} aria-hidden="true" />
        {i18n.t("settings.doctor.copy")}
      </button>
    </div>
  </div>

  <p class="sr-only" role="status" aria-live="polite">{announcement}</p>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if loading && report}<p class="prior">{i18n.t("settings.doctor.prior")}</p>{/if}
  {#if loading && !report}<p class="loading" role="status">{i18n.t("settings.doctor.running")}</p>{/if}

  {#if report}
    <div class="summary" aria-label={announcement}>
      <span class="healthy">{label("healthy")} {report.counts.healthy}</span>
      <span class="needsAttention">{label("needsAttention")} {report.counts.needsAttention}</span>
      <span class="unavailable">{label("unavailable")} {report.counts.unavailable}</span>
    </div>
    <p class="generated">{i18n.t("settings.doctor.generated", { timestamp: report.generatedAt })}</p>
    {#if report.overall === "healthy"}<p class="all-healthy">{i18n.t("settings.doctor.allHealthy")}</p>{/if}

    {#each CATEGORIES as category}
      {@const checks = report.checks.filter((check) => check.category === category)}
      {#if checks.length}
        <div class="group" data-doctor-category={category}>
          <h3>{categoryLabel(category)}</h3>
          {#each checks as check (check.id)}
            <article class="check {check.classification}">
              <div class="check-heading">
                <h4>{check.title}</h4>
                <span>{label(check.classification)}</span>
              </div>
              <p>{check.evidence}</p>
              {#if check.guidance}<p class="guidance">{check.guidance}</p>{/if}
              {#if check.action}
                <button type="button" class="action" data-doctor-action={check.action} onclick={() => runAction(check.action!)}>
                  {actionLabel(check.action)}
                </button>
              {/if}
            </article>
          {/each}
        </div>
      {/if}
    {/each}
  {/if}
</section>

<style>
  .section { display: flex; flex-direction: column; gap: var(--space-4); max-width: 620px; }
  .heading, .controls, .summary, .check-heading { display: flex; align-items: center; }
  .heading { justify-content: space-between; gap: var(--space-4); }
  .heading h2 { font-size: var(--text-h1); font-weight: var(--fw-semibold); color: var(--color-text-primary); }
  .heading p, .generated, .prior, .loading { margin-top: var(--space-1); color: var(--color-text-muted); font-size: var(--text-body-sm); line-height: var(--lh-normal); }
  .controls, .summary { gap: var(--space-2); flex-wrap: wrap; }
  button { display: inline-flex; align-items: center; gap: var(--space-1); padding: 6px var(--space-3); border: 1px solid var(--color-border); border-radius: var(--radius-md); color: var(--color-text-primary); background: var(--color-surface-raised); font-size: var(--text-body-sm); }
  button:disabled { opacity: .55; cursor: not-allowed; }
  button:not(:disabled):hover { background: var(--color-surface-sunken); }
  .summary span { padding: 4px var(--space-2); border-radius: var(--radius-pill); font-size: var(--text-body-sm); font-weight: var(--fw-semibold); }
  .summary .healthy { color: var(--color-success-on-subtle); background: var(--color-success-subtle); }
  .summary .needsAttention { color: var(--color-warning-on-subtle); background: var(--color-warning-subtle); }
  .summary .unavailable { color: var(--color-text-secondary); background: var(--color-surface-sunken); }
  .group { display: flex; flex-direction: column; gap: var(--space-2); }
  .group > h3 { color: var(--color-text-secondary); font-size: var(--text-body-sm); font-weight: var(--fw-semibold); }
  .check { border: 1px solid var(--color-border); border-left-width: 3px; border-radius: var(--radius-md); padding: var(--space-3); background: var(--color-surface-raised); }
  .check.healthy { border-left-color: var(--color-success); }
  .check.needsAttention { border-left-color: var(--color-warning); }
  .check.unavailable { border-left-color: var(--color-text-muted); }
  .check-heading { justify-content: space-between; gap: var(--space-3); }
  .check h4 { font-size: var(--text-body); font-weight: var(--fw-semibold); }
  .check-heading span, .check p { color: var(--color-text-muted); font-size: var(--text-body-sm); }
  .check p { margin-top: var(--space-1); line-height: var(--lh-normal); overflow-wrap: anywhere; }
  .guidance { color: var(--color-text-secondary) !important; }
  .action { margin-top: var(--space-2); }
  .error { color: var(--color-danger); font-size: var(--text-body-sm); }
  .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; }
  .spin { animation: spin 800ms linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) { .spin { animation: none; } }
</style>
