<script lang="ts">
  import { onMount, tick } from "svelte";
  import Copy from "@lucide/svelte/icons/copy";
  import RefreshCw from "@lucide/svelte/icons/refresh-cw";
  import {
    agentInstallsReconcile,
    agentVersionHistory,
    doctorReport,
    projectsList,
    skillInstallsReconcile,
    skillVersionHistory,
    storageBackup,
    storageMigrationStatus,
  } from "$lib/api";
  import { i18n } from "$lib/stores/i18n.svelte";
  import { safeActivityDetail } from "$lib/stores/activity.svelte";
  import { install } from "$lib/stores/install.svelte";
  import { ui } from "$lib/stores/ui.svelte";
  import { appErrorMessage, isAppError, type AgentVersionSnapshot, type DoctorAction, type DoctorCategory, type DoctorClassification, type DoctorReport, type InstalledAgent, type InstalledSkill, type SkillVersionSnapshot } from "$lib/types";

  type RecoveryStatus = "loading" | "ready" | "unavailable";
  type AgentRecoveryRow = { installed: InstalledAgent; snapshots: AgentVersionSnapshot[] };
  type SkillRecoveryRow = { installed: InstalledSkill; snapshots: SkillVersionSnapshot[] };
  const RECOVERY_INSTALL_LIMIT = 100;

  const CATEGORIES: DoctorCategory[] = ["core", "library", "installations", "tools", "integrations", "updates"];
  let report: DoctorReport | null = $state(null);
  let loading = $state(false);
  let error = $state("");
  let announcement = $state("");
  let recoveryAnnouncement = $state("");
  let agentRecoveryStatus = $state<RecoveryStatus>("loading");
  let agentRecoveryError = $state("");
  let agentRecoveryRows = $state<AgentRecoveryRow[]>([]);
  let skillRecoveryStatus = $state<RecoveryStatus>("loading");
  let skillRecoveryError = $state("");
  let skillRecoveryRows = $state<SkillRecoveryRow[]>([]);
  const skillRollbackCount = $derived(skillRecoveryRows.reduce((count, row) => count + row.snapshots.length, 0));
  let storageRecoveryStatus = $state<RecoveryStatus>("loading");
  let storageRecoveryError = $state("");
  let backupPath = $state<string | null>(null);
  let backupBusy = $state(false);
  let recoveryRoot: HTMLElement | undefined = $state();

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

  function recoveryError(error: unknown): string {
    return safeActivityDetail(isAppError(error) ? appErrorMessage(error) : error);
  }

  async function loadAgentRecovery() {
    agentRecoveryStatus = "loading";
    agentRecoveryError = "";
    recoveryAnnouncement = "Agent recovery loading.";
    try {
      const installed = (await agentInstallsReconcile())
        .filter((item) => item.tracked && item.sourceId && item.relativePath)
        .slice(0, RECOVERY_INSTALL_LIMIT);
      agentRecoveryRows = (await Promise.all(installed.map(async (item) => ({
        installed: item,
        snapshots: await agentVersionHistory(
          { sourceId: item.sourceId, relativePath: item.relativePath }, item.tool, item.projectPath,
        ),
      })))).filter((row) => row.snapshots.length > 0);
      agentRecoveryStatus = "ready";
      recoveryAnnouncement = `Agent recovery ready. ${agentRecoveryRows.reduce((count, row) => count + row.snapshots.length, 0)} rollback points.`;
    } catch (error) {
      agentRecoveryStatus = "unavailable";
      agentRecoveryError = recoveryError(error);
      recoveryAnnouncement = `Agent recovery unavailable. ${agentRecoveryError}`;
    }
  }

  async function loadSkillRecovery() {
    skillRecoveryStatus = "loading";
    skillRecoveryError = "";
    recoveryAnnouncement = "Skill recovery loading.";
    try {
      const registered = await projectsList();
      const installed = (await skillInstallsReconcile(registered.map((project) => project.path)))
        .filter((item) => item.tracked)
        .slice(0, RECOVERY_INSTALL_LIMIT);
      skillRecoveryRows = (await Promise.all(installed.map(async (item) => ({
        installed: item,
        snapshots: await skillVersionHistory(item),
      })))).filter((row) => row.snapshots.length > 0);
      skillRecoveryStatus = "ready";
      recoveryAnnouncement = `Skill recovery ready. ${skillRecoveryRows.reduce((count, row) => count + row.snapshots.length, 0)} rollback points.`;
    } catch (error) {
      skillRecoveryStatus = "unavailable";
      skillRecoveryError = recoveryError(error);
      recoveryAnnouncement = `Skill recovery unavailable. ${skillRecoveryError}`;
    }
  }

  async function loadStorageRecovery() {
    storageRecoveryStatus = "loading";
    storageRecoveryError = "";
    recoveryAnnouncement = "Database backup recovery loading.";
    try {
      const status = await storageMigrationStatus();
      if (status.state !== "complete") throw new Error("SQLite migration must complete before verified backups are available.");
      storageRecoveryStatus = "ready";
      recoveryAnnouncement = "Database backup recovery is ready.";
    } catch (error) {
      storageRecoveryStatus = "unavailable";
      storageRecoveryError = recoveryError(error);
      recoveryAnnouncement = `Database backup recovery unavailable. ${storageRecoveryError}`;
    }
  }

  function agentRecoveryTrigger(index: number): string {
    return `agent-recovery-${index}`;
  }

  function skillRecoveryTrigger(index: number): string {
    return `skill-recovery-${index}`;
  }

  function openAgentRecovery(row: AgentRecoveryRow, index: number) {
    ui.closeSettings();
    ui.openAgentRecovery({
      reference: { sourceId: row.installed.sourceId, relativePath: row.installed.relativePath },
      tool: row.installed.tool,
      projectPath: row.installed.projectPath,
    }, agentRecoveryTrigger(index));
  }

  function openSkillRecovery(row: SkillRecoveryRow, index: number) {
    ui.closeSettings();
    ui.openSkillRecovery({
      reference: { sourceId: row.installed.sourceId, relativePath: row.installed.relativePath },
      runtime: row.installed.runtime,
      projectPath: row.installed.projectPath,
    }, skillRecoveryTrigger(index));
  }

  async function createBackup() {
    if (backupBusy || storageRecoveryStatus !== "ready") return;
    backupBusy = true;
    storageRecoveryError = "";
    try {
      backupPath = await storageBackup();
      recoveryAnnouncement = `Verified backup created: ${backupName(backupPath)}.`;
      await tick();
      document.querySelector<HTMLButtonElement>("[data-storage-reveal]")?.focus({ preventScroll: true });
    } catch (error) {
      storageRecoveryError = recoveryError(error);
      recoveryAnnouncement = `Verified backup failed. ${storageRecoveryError}`;
    } finally {
      backupBusy = false;
    }
  }

  async function revealBackup() {
    if (!backupPath) return;
    storageRecoveryError = "";
    try {
      await install.revealPath(backupPath);
      recoveryAnnouncement = "Verified backup revealed in the system file manager.";
    } catch (error) {
      storageRecoveryError = recoveryError(error);
      recoveryAnnouncement = `Could not reveal verified backup. ${storageRecoveryError}`;
    }
  }

  function backupName(path: string): string {
    return path.split(/[\\/]/).filter(Boolean).at(-1) ?? "database backup";
  }

  async function retryRecovery(source: "agents" | "skills" | "storage") {
    if (source === "agents") await loadAgentRecovery();
    else if (source === "skills") await loadSkillRecovery();
    else await loadStorageRecovery();
    await tick();
    document.querySelector<HTMLElement>(`[data-recovery-source="${source}"] h3`)?.focus({ preventScroll: true });
    const ready = source === "agents"
      ? agentRecoveryStatus === "ready"
      : source === "skills"
        ? skillRecoveryStatus === "ready"
        : storageRecoveryStatus === "ready";
    if (ready) recoveryAnnouncement = `${source === "agents" ? "Agent" : source === "skills" ? "Skill" : "Database backup"} recovery refreshed.`;
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

  onMount(() => {
    void refresh();
    void Promise.all([loadAgentRecovery(), loadSkillRecovery(), loadStorageRecovery()]);
  });

  $effect(() => {
    const id = ui.recoveryReturnId;
    if (!ui.settingsOpen || !id) return;
    void tick().then(() => setTimeout(() => {
      const trigger = [...(recoveryRoot?.querySelectorAll<HTMLButtonElement>("[data-recovery-trigger]") ?? [])]
        .find((candidate) => candidate.dataset.recoveryTrigger === id);
      if (!trigger) return;
      trigger.focus({ preventScroll: true });
      ui.consumeRecoveryReturn();
      recoveryAnnouncement = "Returned to Recovery.";
    }, 0));
  });
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

  <section class="recovery" bind:this={recoveryRoot} aria-labelledby="recovery-title">
    <div><h2 id="recovery-title">Recovery</h2><p>Use the existing exact rollback controls and verified app-owned backups.</p></div>
    <p class="sr-only" role="status" aria-live="polite" aria-atomic="true">{recoveryAnnouncement}</p>

    <article data-recovery-source="agents" aria-busy={agentRecoveryStatus === "loading"}>
      <div class="recovery-head"><h3 tabindex="-1">Agent versions</h3><strong>{agentRecoveryStatus === "ready" ? `${agentRecoveryRows.reduce((count, row) => count + row.snapshots.length, 0)} rollback ${agentRecoveryRows.reduce((count, row) => count + row.snapshots.length, 0) === 1 ? "point" : "points"}` : agentRecoveryStatus === "loading" ? "Loading" : "Unavailable"}</strong></div>
      {#if agentRecoveryStatus === "unavailable"}<p role="alert" class="error">{agentRecoveryError}</p><button type="button" data-recovery-retry onclick={() => void retryRecovery("agents")}>Retry</button>
      {:else if agentRecoveryStatus === "ready"}
        {#if agentRecoveryRows.length === 0}<p>No Agent installs with version history.</p>{:else}<ul>{#each agentRecoveryRows as row, index (`${row.installed.sourceId}:${row.installed.relativePath}:${row.installed.tool}:${row.installed.projectPath ?? ""}`)}<li><span>{row.installed.name} · {row.installed.tool} · {row.snapshots.length} snapshots</span><button type="button" data-agent-recovery data-recovery-trigger={agentRecoveryTrigger(index)} onclick={() => openAgentRecovery(row, index)}>Open rollback</button></li>{/each}</ul>{/if}
      {/if}
    </article>

    <article data-recovery-source="skills" aria-busy={skillRecoveryStatus === "loading"}>
      <div class="recovery-head"><h3 tabindex="-1">Skill versions</h3><strong>{skillRecoveryStatus === "ready" ? `${skillRollbackCount} rollback ${skillRollbackCount === 1 ? "point" : "points"}` : skillRecoveryStatus === "loading" ? "Loading" : "Unavailable"}</strong></div>
      {#if skillRecoveryStatus === "unavailable"}<p role="alert" class="error">{skillRecoveryError}</p><button type="button" data-recovery-retry onclick={() => void retryRecovery("skills")}>Retry</button>
      {:else if skillRecoveryStatus === "ready"}
        {#if skillRecoveryRows.length === 0}<p>No Skill installs with version history.</p>{:else}<ul>{#each skillRecoveryRows as row, index (`${row.installed.sourceId}:${row.installed.relativePath}:${row.installed.runtime}:${row.installed.projectPath ?? ""}`)}<li><span>{row.installed.name} · {row.installed.runtime} · {row.snapshots.length} snapshots</span><button type="button" data-skill-recovery data-recovery-trigger={skillRecoveryTrigger(index)} onclick={() => openSkillRecovery(row, index)}>Open rollback</button></li>{/each}</ul>{/if}
      {/if}
    </article>

    <article data-recovery-source="storage" aria-busy={storageRecoveryStatus === "loading"}>
      <div class="recovery-head"><h3 tabindex="-1">Database backup</h3><strong>{storageRecoveryStatus === "ready" ? "Ready" : storageRecoveryStatus === "loading" ? "Loading" : "Unavailable"}</strong></div>
      <p>Creates a verified SQLite backup. Backup creation is not restore.</p>
      <p>Database restore is offline/manual because the running app owns the WAL, process lease, and caches. Close the app before restoring.</p>
      {#if storageRecoveryError}<p role="alert" class="error">{storageRecoveryError}</p>{/if}
      {#if storageRecoveryStatus === "unavailable"}<button type="button" data-recovery-retry onclick={() => void retryRecovery("storage")}>Retry</button>
      {:else if storageRecoveryStatus === "ready"}<div class="recovery-actions"><button type="button" data-storage-backup disabled={backupBusy} onclick={() => void createBackup()}>{backupBusy ? "Creating…" : "Create verified backup"}</button>{#if backupPath}<span>Verified backup created: {backupName(backupPath)}</span><button type="button" data-storage-reveal onclick={() => void revealBackup()}>Reveal backup</button>{/if}</div>{/if}
    </article>
  </section>
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
  .recovery { display: grid; gap: var(--space-3); padding-top: var(--space-4); border-top: 1px solid var(--color-border); }
  .recovery > div > p, .recovery article p, .recovery li { color: var(--color-text-muted); font-size: var(--text-body-sm); }
  .recovery article { display: grid; gap: var(--space-2); padding: var(--space-3); border: 1px solid var(--color-border); border-radius: var(--radius-md); }
  .recovery-head, .recovery-actions, .recovery li { display: flex; align-items: center; justify-content: space-between; gap: var(--space-2); }
  .recovery ul { display: grid; gap: var(--space-2); }
  .recovery button { width: fit-content; }
  .recovery button:focus-visible { outline: 2px solid var(--color-brand); outline-offset: 2px; }
  .recovery-actions { justify-content: flex-start; flex-wrap: wrap; }
  @keyframes spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) { .spin { animation: none; } }
</style>
