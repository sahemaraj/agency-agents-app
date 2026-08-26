<script lang="ts">
  import { tick } from "svelte";
  import Modal from "./Modal.svelte";
  import Button from "./Button.svelte";
  import DiffModal from "./DiffModal.svelte";
  import { skillInstallPlan, skillSourcesInspect } from "$lib/api";
  import { install } from "$lib/stores/install.svelte";
  import { skillSources } from "$lib/stores/skillSources.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { activity, safeActivityDetail } from "$lib/stores/activity.svelte";
  import { i18n } from "$lib/stores/i18n.svelte";
  import { ui } from "$lib/stores/ui.svelte";
  import { appErrorMessage, isAppError } from "$lib/types";
  import type { AgentMergePreview, AgentMutationPlan, InstalledAgent, InstalledSkill, InstallState, SkillInstallState, SkillMutationPlan } from "$lib/types";

  interface Props {
    onClose: () => void;
  }
  let { onClose }: Props = $props();

  type RepairCandidate =
    | { kind: "agent"; key: string; row: InstalledAgent }
    | { kind: "skill"; key: string; row: InstalledSkill };
  type ReviewPlan =
    | { kind: "agent"; candidate: Extract<RepairCandidate, { kind: "agent" }>; plan: AgentMutationPlan | null; mergePreview: AgentMergePreview | null; error: string | null }
    | { kind: "skill"; candidate: Extract<RepairCandidate, { kind: "skill" }>; plan: SkillMutationPlan | null; sourceFiles: { relativePath: string; sizeBytes: number; sha256: string }[]; error: string | null };
  interface RepairResult {
    candidate: RepairCandidate;
    ok: boolean;
    stale: boolean;
    error: string | null;
  }
  type RepairAction = "update" | "merge" | "overwrite" | "skip";
  type AgentReviewPlan = Extract<ReviewPlan, { kind: "agent" }>;
  type AgentCandidate = Extract<RepairCandidate, { kind: "agent" }>;
  type DiffView = { candidate: AgentCandidate; mergedPreview?: string; conflictSummaries?: string[] };

  const repairableAgent = (state: InstallState) => state === "outdated" || state === "missing" || state === "modified";
  const repairableSkill = (state: SkillInstallState) => state === "outdated" || state === "missing";
  const agentKey = (row: InstalledAgent) => ["agent", row.sourceId, row.relativePath, row.tool, row.projectPath ?? ""].join("\0");
  const skillKey = (row: InstalledSkill) => ["skill", row.sourceId, row.relativePath, row.runtime, row.projectPath ?? ""].join("\0");

  const collectCandidates = (): RepairCandidate[] => [
    ...install.installed
      .filter((row) => row.tracked && repairableAgent(row.state))
      .map((row) => ({ kind: "agent" as const, key: agentKey(row), row })),
    ...skillSources.installed
      .filter((row) => row.tracked && repairableSkill(row.state))
      .map((row) => ({ kind: "skill" as const, key: skillKey(row), row })),
  ];
  const candidates = $derived.by(collectCandidates);
  const unsafe = $derived.by<RepairCandidate[]>(() => [
    ...install.installed
      .filter((row) => ["foreign", "disabled", "sourceUnavailable"].includes(row.state))
      .map((row) => ({ kind: "agent" as const, key: row.tracked ? agentKey(row) : ["agent", "unsafe", row.tool, row.dest].join("\0"), row })),
    ...skillSources.installed
      .filter((row) => ["modified", "foreign", "disabled", "sourceUnavailable"].includes(row.state))
      .map((row) => ({ kind: "skill" as const, key: row.tracked ? skillKey(row) : ["skill", "unsafe", row.runtime, row.path].join("\0"), row })),
  ]);

  let deselected = $state<Set<string>>(new Set());
  let stage = $state<"select" | "review" | "results">("select");
  let planning = $state(false);
  let applying = $state(false);
  let progress = $state(0);
  let applyTotal = $state(0);
  let results = $state<RepairResult[]>([]);
  let receiptId = $state<string | null>(null);
  let reviewPlans = $state<ReviewPlan[]>([]);
  let reviewedSignature = $state("");
  let staleMessage = $state<string | null>(null);
  let actions = $state<Record<string, RepairAction>>({});
  let diffView = $state<DiffView | null>(null);
  const selected = $derived(candidates.filter((candidate) => !deselected.has(candidate.key)));
  const allSelected = $derived(candidates.length > 0 && selected.length === candidates.length);
  const someSelected = $derived(selected.length > 0 && !allSelected);
  const truthReady = $derived(
    install.reconciled && !install.reconciling && !install.reconcileError
    && skillSources.reconciled && !skillSources.reconciling && !skillSources.reconcileError,
  );

  function toggle(key: string) {
    const next = new Set(deselected);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    deselected = next;
  }

  function toggleAll() {
    deselected = allSelected ? new Set(candidates.map((candidate) => candidate.key)) : new Set();
  }

  function target(candidate: RepairCandidate): string {
    return candidate.kind === "agent" ? candidate.row.tool : candidate.row.runtime;
  }

  function destination(candidate: RepairCandidate): string {
    return candidate.kind === "agent" ? candidate.row.dest : candidate.row.path;
  }

  function unsafeReason(state: InstallState | SkillInstallState): string {
    if (state === "modified") return i18n.optional("agentUpdates.reasonModified", "Local changes require manual review");
    if (state === "foreign") return i18n.optional("agentUpdates.reasonForeign", "Untracked content requires manual review");
    if (state === "disabled") return i18n.optional("agentUpdates.reasonDisabled", "Enable this installation before repair");
    return i18n.optional("agentUpdates.reasonSourceUnavailable", "Restore its source before repair");
  }

  function finalState(candidate: RepairCandidate): string {
    const row = candidate.kind === "agent"
      ? install.installed.find((entry) => agentKey(entry) === candidate.key)
      : skillSources.installed.find((entry) => skillKey(entry) === candidate.key);
    return row ? i18n.t(`state.${row.state}`) : i18n.optional("agentUpdates.notDetected", "Not detected");
  }

  function ledgerError(): string | null {
    if (!install.reconciled || install.reconciling || install.reconcileError) {
      return i18n.optional("agentUpdates.agentTruthUnavailable", "Agent installation status is unavailable. Retry reconciliation before repair.");
    }
    if (!skillSources.reconciled || skillSources.reconciling || skillSources.reconcileError) {
      return i18n.optional("agentUpdates.skillTruthUnavailable", "Skill installation status is unavailable. Retry reconciliation before repair.");
    }
    return null;
  }

  function errorMessage(error: unknown): string {
    return isAppError(error) ? appErrorMessage(error) : error instanceof Error ? error.message : String(error);
  }

  function mergePreviewIsStale(error: unknown): boolean {
    const message = errorMessage(error);
    // Backend `agent_merge_apply` reports both branches as invalid_argument,
    // so these messages are the only available discriminants.
    return message.includes("Agent merge preview is stale")
      || message.includes("Agent merge is no longer clean");
  }

  function viewReceipt() {
    if (!receiptId) return;
    const id = receiptId;
    onClose();
    ui.openActivityReceipt(id);
  }

  async function buildPlans(items: RepairCandidate[]): Promise<ReviewPlan[]> {
    const needsSkills = items.some((candidate) => candidate.kind === "skill");
    let inspectedSkills: Awaited<ReturnType<typeof skillSourcesInspect>> | null = null;
    let inspectError: string | null = null;
    if (needsSkills) {
      try {
        inspectedSkills = await skillSourcesInspect();
      } catch (error) {
        inspectError = errorMessage(error);
      }
    }
    return Promise.all(items.map(async (candidate): Promise<ReviewPlan> => {
      try {
        if (candidate.kind === "agent") {
          const plan = await install.plan(
            "update",
            { sourceId: candidate.row.sourceId, relativePath: candidate.row.relativePath },
            candidate.row.tool,
            candidate.row.projectPath,
          );
          if (candidate.row.state === "modified" && plan.mergeOutcome?.status === "clean") {
            try {
              const mergePreview = await install.mergePreviewReference(
                { sourceId: candidate.row.sourceId, relativePath: candidate.row.relativePath },
                candidate.row.tool,
                candidate.row.projectPath,
              );
              return { kind: "agent", candidate, plan, mergePreview, error: null };
            } catch (error) {
              return { kind: "agent", candidate, plan, mergePreview: null, error: errorMessage(error) };
            }
          }
          return { kind: "agent", candidate, plan, mergePreview: null, error: null };
        }
        const plan = await skillInstallPlan(
          candidate.row.sourceId,
          candidate.row.relativePath,
          candidate.row.runtime,
          candidate.row.projectPath,
        );
        if (inspectError) return { kind: "skill", candidate, plan, sourceFiles: [], error: inspectError };
        const pkg = inspectedSkills
          ?.flatMap((source) => source.packages)
          .find((entry) => entry.sourceId === candidate.row.sourceId && entry.relativePath === candidate.row.relativePath);
        if (!pkg) return { kind: "skill", candidate, plan, sourceFiles: [], error: i18n.optional("agentUpdates.sourceInspectionMissing", "Skill source could not be inspected") };
        return { kind: "skill", candidate, plan, sourceFiles: pkg.files, error: null };
      } catch (error) {
        return candidate.kind === "agent"
          ? { kind: "agent", candidate, plan: null, mergePreview: null, error: errorMessage(error) }
          : { kind: "skill", candidate, plan: null, sourceFiles: [], error: errorMessage(error) };
      }
    }));
  }

  function planSignature(plans: ReviewPlan[]): string {
    return JSON.stringify(plans.map((item) => ({
      key: item.candidate.key,
      state: item.candidate.row.state,
      plan: item.plan,
      mergePreview: item.kind === "agent" ? item.mergePreview : undefined,
      sourceFiles: item.kind === "skill" ? item.sourceFiles : undefined,
      error: item.error,
    })));
  }

  async function reverifyReviewedPlan(item: ReviewPlan): Promise<ReviewPlan | null> {
    const [fresh] = await buildPlans([item.candidate]);
    return fresh && planSignature([fresh]) === planSignature([item]) ? fresh : null;
  }

  function defaultAction(item: ReviewPlan): RepairAction {
    if (item.candidate.row.state !== "modified") return "update";
    return item.kind === "agent" && item.plan?.mergeOutcome?.status === "clean" && item.mergePreview
      ? "merge"
      : "skip";
  }

  function actionFor(item: ReviewPlan): RepairAction {
    return actions[item.candidate.key] ?? defaultAction(item);
  }

  function chooseAction(item: ReviewPlan, action: RepairAction) {
    actions = { ...actions, [item.candidate.key]: action };
  }

  function plainUnavailableReason(reason: string): string {
    if (reason.includes("no canonical base snapshot")) {
      return i18n.optional("agentUpdates.mergeNoBase", "No recorded base exists for this install yet. Overwrite once to enable merging next time.");
    }
    if (reason.includes("multi-artifact") || reason.includes("aggregate roster") || reason.includes("multi-Agent")) {
      return i18n.optional("agentUpdates.mergeMultipleFiles", "This install writes more than one file, so its changes cannot be combined here.");
    }
    if (reason.includes("UTF-8") || reason.includes("only md, toml, and mdc")) {
      return i18n.optional("agentUpdates.mergeNonText", "This is not a supported text file, so its changes cannot be combined here.");
    }
    if (reason.includes("enable the Agent")) {
      return i18n.optional("agentUpdates.mergeDisabled", "Enable this Agent before combining changes.");
    }
    return reason;
  }

  function showMerged(item: AgentReviewPlan) {
    if (!item.mergePreview) return;
    chooseAction(item, "merge");
    diffView = { candidate: item.candidate, mergedPreview: item.mergePreview.preview };
  }

  function showConflicts(item: AgentReviewPlan) {
    const outcome = item.plan?.mergeOutcome;
    if (outcome?.status !== "conflicts") return;
    diffView = { candidate: item.candidate, conflictSummaries: outcome.hunkSummaries };
  }

  const reviewBlocked = $derived.by(() => {
    const actionable = reviewPlans.filter((item) => actionFor(item) !== "skip");
    return actionable.length === 0
      || actionable.some((item) => !item.plan
        || item.plan.blockers.length > 0
        || (Boolean(item.error) && (item.kind === "skill" || actionFor(item) === "merge")));
  });

  async function reviewSelected() {
    if (!truthReady || selected.length === 0 || planning) return;
    planning = true;
    staleMessage = null;
    try {
      reviewPlans = await buildPlans(selected);
      actions = Object.fromEntries(reviewPlans.map((item) => [item.candidate.key, defaultAction(item)]));
      reviewedSignature = planSignature(reviewPlans);
      stage = "review";
      await tick();
    } finally {
      planning = false;
    }
  }

  async function approveReviewed() {
    if (reviewBlocked || planning) return;
    planning = true;
    staleMessage = null;
    try {
      await Promise.all([
        install.reconcile(),
        skillSources.reconcileInstalls(projects.list.map((project) => project.path)),
      ]);
      await tick();
      if (install.reconcileError || skillSources.reconcileError) {
        staleMessage = i18n.optional("agentUpdates.freshReconcileFailed", "Fresh reconciliation failed. Retry before approving repairs.");
        return;
      }
      const selectedKeys = new Set(reviewPlans.map(({ candidate }) => candidate.key));
      const freshCandidates = collectCandidates().filter((candidate) => selectedKeys.has(candidate.key));
      const freshPlans = await buildPlans(freshCandidates);
      const freshSignature = planSignature(freshPlans);
      if (freshCandidates.length !== selectedKeys.size || freshSignature !== reviewedSignature) {
        reviewPlans = freshPlans;
        actions = Object.fromEntries(freshPlans.map((item) => [item.candidate.key, defaultAction(item)]));
        reviewedSignature = freshSignature;
        staleMessage = i18n.optional("agentUpdates.planChanged", "Repair plan changed. Review the updated plan before approving again.");
        return;
      }
      applying = true;
      progress = 0;
      results = [];
      receiptId = null;
      const actionable = freshPlans.filter((item) => actionFor(item) !== "skip");
      applyTotal = actionable.length;
      for (const item of actionable) {
        let result: RepairResult;
        const verified = await reverifyReviewedPlan(item);
        if (!verified) {
          result = {
            candidate: item.candidate,
            ok: false,
            stale: true,
            error: i18n.optional("agentUpdates.contentChanged", "This installation changed since review. Review the updated content before trying again."),
          };
        } else try {
          if (verified.kind === "agent") {
            const reference = { sourceId: verified.candidate.row.sourceId, relativePath: verified.candidate.row.relativePath };
            if (actionFor(verified) === "merge") {
              if (verified.plan?.mergeOutcome?.status !== "clean" || !verified.mergePreview) {
                throw new Error("A fresh merge preview is required");
              }
              await install.mergeApplyReference(
                reference,
                verified.candidate.row.tool,
                verified.candidate.row.projectPath,
                verified.mergePreview.previewHash,
              );
            } else {
              await install.updateReference(
                reference,
                verified.candidate.row.tool,
                verified.candidate.row.projectPath,
                true,
              );
            }
            result = { candidate: verified.candidate, ok: true, stale: false, error: null };
          } else {
            const ok = await skillSources.lifecycle(
              "update",
              verified.candidate.row,
              projects.list.map((project) => project.path),
            );
            result = {
              candidate: verified.candidate,
              ok,
              stale: false,
              error: ok ? null : skillSources.installErrors[verified.candidate.key.slice("skill\0".length)] ?? i18n.optional("agentUpdates.unknownFailure", "Repair failed"),
            };
          }
        } catch (error) {
          const stale = mergePreviewIsStale(error);
          result = {
            candidate: item.candidate,
            ok: false,
            stale,
            error: stale
              ? i18n.optional("agentUpdates.mergeStale", "This file changed on disk after the preview. Review a fresh preview before trying again.")
              : safeActivityDetail(errorMessage(error)),
          };
        }
        results = [...results, result];
        progress += 1;
      }
      await Promise.all([
        install.reconcile(),
        skillSources.reconcileInstalls(projects.list.map((project) => project.path)),
      ]);
      const repaired = results.filter((result) => result.ok).length;
      const failed = results.length - repaired;
      receiptId = activity.log({
        action: "bulk",
        subject: "agentLibrary",
        subjectName: i18n.optional("agentUpdates.repairActivity", "Safe repair"),
        outcome: failed === 0 ? "ok" : "error",
        detail: `${repaired} repaired · ${failed} failed`,
        receipt: {
          operation: "repair",
          succeeded: repaired,
          failed,
          items: results.map((result) => ({
            kind: result.candidate.kind,
            name: result.candidate.row.name,
            destination: destination(result.candidate),
            outcome: result.ok ? "ok" : "error",
            ...(result.error ? { detail: result.error } : {}),
          })),
        },
      });
      stage = "results";
    } finally {
      applying = false;
      planning = false;
    }
  }

  async function repreview(result: RepairResult) {
    if (!result.stale || planning) return;
    planning = true;
    try {
      const [fresh] = await buildPlans([result.candidate]);
      if (!fresh) return;
      reviewPlans = [fresh];
      actions = { [fresh.candidate.key]: defaultAction(fresh) };
      reviewedSignature = planSignature(reviewPlans);
      staleMessage = fresh.kind === "agent"
        ? i18n.optional("agentUpdates.mergeRepreviewed", "The preview was refreshed from the file now on disk. Review it before applying.")
        : i18n.optional("agentUpdates.reviewRefreshed", "The repair review was refreshed from the current content. Review it before applying.");
      stage = "review";
      if (fresh.kind === "agent" && fresh.mergePreview) showMerged(fresh);
      else if (fresh.kind === "agent" && fresh.plan?.mergeOutcome?.status === "conflicts") showConflicts(fresh);
    } finally {
      planning = false;
    }
  }
</script>

<Modal open size="wide" dismissible={!planning} title={stage === "results" ? i18n.optional("agentUpdates.resultsTitle", "Repair results") : i18n.optional("agentUpdates.repairTitle", "Safe repair", { count: candidates.length })} onClose={onClose}>
  <p class="sub">{i18n.optional("agentUpdates.repairSub", "Review recoverable Agent and Skill installations before changing files.")}</p>

  {#if ledgerError() && stage === "select"}
    <p class="error" role="alert">{ledgerError()}</p>
  {:else if candidates.length === 0 && unsafe.length === 0 && stage === "select"}
    <p class="empty">{i18n.t("agentUpdates.empty")}</p>
  {:else if stage === "select"}
    {#if candidates.length > 0}
      <div class="head">
        <label class="all">
          <input type="checkbox" checked={allSelected} indeterminate={someSelected} onchange={toggleAll} />
          {i18n.t("agentUpdates.selectAll")}
        </label>
        <span class="n">{i18n.t("common.selected", { count: selected.length })}</span>
      </div>
      <ul class="items">
        {#each candidates as candidate (candidate.key)}
          <li>
            <label class="candidate">
              <input
                type="checkbox"
                name="repair-item"
                data-candidate-key={candidate.key}
                checked={!deselected.has(candidate.key)}
                onchange={() => toggle(candidate.key)}
              />
              <span class="details">
                <span class="name">{candidate.row.name}</span>
                <span class="meta">{candidate.kind === "agent" ? "Agent" : "Skill"} · {target(candidate)} · {candidate.row.state === "missing" ? i18n.optional("agentUpdates.reinstall", "Reinstall") : candidate.row.state === "modified" ? i18n.optional("agentUpdates.reviewChanges", "Review local edits") : i18n.optional("agentUpdates.update", "Update")}</span>
                <span class="path" title={destination(candidate)}>{destination(candidate)}</span>
              </span>
            </label>
          </li>
        {/each}
      </ul>
    {/if}

    {#if unsafe.length > 0}
      <section class="manual">
        <h2>{i18n.optional("agentUpdates.manualTitle", "Manual review required")}</h2>
        <ul class="items unsafe">
          {#each unsafe as candidate (candidate.key)}
            <li class="candidate">
              <span class="details">
                <span class="name">{candidate.row.name}</span>
                <span class="meta">{candidate.kind === "agent" ? "Agent" : "Skill"} · {target(candidate)} · {unsafeReason(candidate.row.state)}</span>
                <span class="path" title={destination(candidate)}>{destination(candidate)}</span>
              </span>
            </li>
          {/each}
        </ul>
      </section>
    {/if}
  {:else if stage === "review"}
    {#if staleMessage}<p class="error" role="alert">{staleMessage}</p>{/if}
    <p class="sub">{i18n.optional("agentUpdates.reviewSub", "Confirm every destination, warning, blocker, and recovery option before repair.")}</p>
    <ul class="plans">
      {#each reviewPlans as item (item.candidate.key)}
        <li>
          <div class="plan-head">
            <span class="name">{item.candidate.row.name}</span>
            <span class="operation">{item.candidate.row.state === "missing" ? i18n.optional("agentUpdates.reinstall", "Reinstall") : item.candidate.row.state === "modified" ? i18n.optional("agentUpdates.reviewChanges", "Review local edits") : i18n.optional("agentUpdates.update", "Update")}</span>
          </div>
          <p class="meta">{item.candidate.kind === "agent" ? "Agent" : "Skill"} · {item.candidate.row.sourceId} · {item.candidate.row.relativePath}</p>
          {#if !item.plan}
            <p class="error">{item.error}</p>
          {:else}
            {#if item.error}<p class="error">{item.error}</p>{/if}
            <ul class="packages">
              {#if item.kind === "agent"}
                {#each item.plan.agents as pkg}
                  <li>
                    <span class="path" title={pkg.destination}>{pkg.destination}</span>
                    <span class="meta">{pkg.dependency ? i18n.optional("agentUpdates.dependency", "Dependency") : i18n.optional("agentUpdates.primary", "Primary")} · {pkg.renderedFileCount} {pkg.renderedFileCount === 1 ? "file" : "files"}</span>
                  </li>
                {/each}
              {:else}
                {#each item.plan.packages as pkg}
                  <li>
                    <span class="path" title={pkg.destination}>{pkg.destination}</span>
                    <span class="meta">{pkg.dependency ? i18n.optional("agentUpdates.dependency", "Dependency") : i18n.optional("agentUpdates.primary", "Primary")} · {pkg.fileCount} {pkg.fileCount === 1 ? "file" : "files"}{#if pkg.permissions.length} · {pkg.permissions.join(", ")}{/if}</span>
                  </li>
                {/each}
              {/if}
            </ul>
            {#each item.plan.warnings as warning}<p class="warning">{warning}</p>{/each}
            {#each item.plan.blockers as blocker}<p class="error">{blocker}</p>{/each}
            <p class="meta">{item.plan.rollbackAvailable ? i18n.optional("agentUpdates.rollbackAvailable", "Rollback available") : i18n.optional("agentUpdates.rollbackUnavailable", "No existing content requires backup")}</p>
            {#if item.kind === "agent" && item.candidate.row.state === "modified" && item.plan.mergeOutcome}
              <section class="merge-review" data-merge-status={item.plan.mergeOutcome.status}>
                {#if item.plan.mergeOutcome.status === "clean"}
                  <p class="merge-title">{i18n.optional("agentUpdates.mergeClean", "Your edits are kept, and upstream changes are added.")}</p>
                  <div class="merge-actions">
                    <Button size="sm" variant="primary" disabled={!item.mergePreview} onclick={() => showMerged(item)}>{i18n.optional("agentUpdates.mergeAndUpdate", "Merge and update")}</Button>
                    <Button size="sm" variant="danger" onclick={() => chooseAction(item, "overwrite")}>{i18n.optional("agentUpdates.overwriteEdits", "Overwrite local edits")}</Button>
                    <Button size="sm" variant="secondary" onclick={() => chooseAction(item, "skip")}>{i18n.optional("agentUpdates.skip", "Skip")}</Button>
                  </div>
                {:else if item.plan.mergeOutcome.status === "conflicts"}
                  <p class="merge-title">{i18n.optional("agentUpdates.mergeConflicts", "{count} conflicting parts need your choice. Nothing will be merged automatically.", { count: item.plan.mergeOutcome.count })}</p>
                  <div class="merge-actions">
                    <Button size="sm" variant="secondary" onclick={() => showConflicts(item)}>{i18n.optional("agentUpdates.viewConflicts", "View conflicting parts")}</Button>
                    <Button size="sm" variant="danger" onclick={() => chooseAction(item, "overwrite")}>{i18n.optional("agentUpdates.overwriteEdits", "Overwrite local edits")}</Button>
                    <Button size="sm" variant="secondary" onclick={() => chooseAction(item, "skip")}>{i18n.optional("agentUpdates.skip", "Skip")}</Button>
                  </div>
                {:else}
                  <p class="merge-title">{plainUnavailableReason(item.plan.mergeOutcome.reason)}</p>
                  <div class="merge-actions">
                    <Button size="sm" variant="danger" onclick={() => chooseAction(item, "overwrite")}>{i18n.optional("agentUpdates.overwriteEdits", "Overwrite local edits")}</Button>
                    <Button size="sm" variant="secondary" onclick={() => chooseAction(item, "skip")}>{i18n.optional("agentUpdates.skip", "Skip")}</Button>
                  </div>
                {/if}
                <p class="choice-status">{actionFor(item) === "merge" ? i18n.optional("agentUpdates.choiceMerge", "Selected: keep edits and update") : actionFor(item) === "overwrite" ? i18n.optional("agentUpdates.choiceOverwrite", "Selected: overwrite local edits") : i18n.optional("agentUpdates.choiceSkip", "Selected: skip")}</p>
                <button class="diff-link" onclick={() => (diffView = { candidate: item.candidate })}>{i18n.optional("agentUpdates.viewOverwriteDiff", "View overwrite diff")}</button>
              </section>
            {:else if item.kind === "agent"}
              <button class="diff-link" onclick={() => (diffView = { candidate: item.candidate })}>{i18n.optional("agentUpdates.viewDiff", "View diff")}</button>
            {/if}
          {/if}
        </li>
      {/each}
    </ul>
    {#if applying}
      <p class="progress" role="status" aria-live="polite">{i18n.optional("agentUpdates.progress", "Repairing {done} of {total}", { done: progress, total: applyTotal })}</p>
    {/if}
  {:else}
    <p class="sub">{i18n.optional("agentUpdates.resultsSub", "Every selected installation reached a terminal result.")}</p>
    <ul class="results">
      {#each results as result (result.candidate.key)}
        <li>
          <span class="result-mark" class:ok={result.ok}>{result.ok ? "✓" : "!"}</span>
          <span class="details">
            <span class="name">{result.candidate.row.name}</span>
            <span class="meta">{result.ok ? `${i18n.optional("agentUpdates.repaired", "Repaired")} · ${finalState(result.candidate)}` : result.error}</span>
            <span class="path" title={destination(result.candidate)}>{destination(result.candidate)}</span>
            {#if result.stale}<button class="diff-link" onclick={() => void repreview(result)}>{result.candidate.kind === "agent" ? i18n.optional("agentUpdates.repreview", "Re-preview") : i18n.optional("agentUpdates.reviewAgain", "Review again")}</button>{/if}
          </span>
        </li>
      {/each}
    </ul>
  {/if}

  {#snippet actions()}
    {#if stage === "review"}
      <Button variant="secondary" modalAction="cancel" disabled={planning} onclick={() => (stage = "select")}>{i18n.optional("agentUpdates.back", "Back")}</Button>
    {:else}
      <Button variant="secondary" modalAction="cancel" onclick={onClose}>{i18n.t("common.close")}</Button>
    {/if}
    {#if stage === "results" && receiptId}
      <Button variant="primary" onclick={viewReceipt}>{i18n.t("activity.viewReceipt")}</Button>
    {/if}
    {#if stage !== "results"}
      <Button
        variant="primary"
        modalAction="confirm"
        loading={planning}
        disabled={stage === "select" ? !truthReady || selected.length === 0 : reviewBlocked}
        onclick={stage === "select" ? reviewSelected : approveReviewed}
      >
        {stage === "select" ? i18n.optional("agentUpdates.reviewN", "Review {count}", { count: selected.length }) : i18n.optional("agentUpdates.approve", "Approve repairs")}
      </Button>
    {/if}
  {/snippet}
</Modal>

{#if diffView}
  <DiffModal
    slug={diffView.candidate.row.slug}
    name={diffView.candidate.row.name}
    tool={diffView.candidate.row.tool}
    projectPath={diffView.candidate.row.projectPath}
    reference={{ sourceId: diffView.candidate.row.sourceId, relativePath: diffView.candidate.row.relativePath }}
    mergedPreview={diffView.mergedPreview}
    conflictSummaries={diffView.conflictSummaries}
    onClose={() => (diffView = null)}
  />
{/if}

<style>
  .sub, .empty { font-size: var(--text-body-sm); color: var(--color-text-muted); }
  .error { padding: var(--space-3); border: 1px solid var(--color-danger); border-radius: var(--radius-md); color: var(--color-danger); font-size: var(--text-body-sm); }
  .head { display: flex; align-items: center; gap: var(--space-3); margin-bottom: var(--space-2); }
  .all, .candidate { display: flex; align-items: flex-start; gap: var(--space-2); }
  .all { font-size: var(--text-body-sm); color: var(--color-text-secondary); cursor: pointer; }
  input { width: 16px; height: 16px; accent-color: var(--color-brand); }
  .n { margin-left: auto; font-size: var(--text-caption); color: var(--color-text-muted); }
  .items { list-style: none; padding: 0; margin: 0; max-height: 32vh; overflow: auto; border: 1px solid var(--color-border); border-radius: var(--radius-md); }
  .items li { padding: var(--space-2) var(--space-3); border-bottom: 1px solid var(--color-border); }
  .items li:last-child { border-bottom: 0; }
  .candidate { cursor: pointer; }
  .unsafe .candidate { cursor: default; }
  .details { min-width: 0; display: flex; flex: 1; flex-direction: column; gap: 2px; }
  .name { color: var(--color-text-primary); font-size: var(--text-body-sm); font-weight: var(--fw-medium); }
  .meta { color: var(--color-text-secondary); font-size: var(--text-caption); }
  .path { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--color-text-muted); font-family: var(--font-mono); font-size: var(--text-mono); }
  .manual { margin-top: var(--space-4); }
  .manual h2 { margin-bottom: var(--space-2); color: var(--color-text-secondary); font-size: var(--text-body-sm); font-weight: var(--fw-semibold); }
  .plans { list-style: none; padding: 0; margin: 0; max-height: 52vh; overflow: auto; display: flex; flex-direction: column; gap: var(--space-3); }
  .plans > li { padding: var(--space-3); border: 1px solid var(--color-border); border-radius: var(--radius-md); }
  .plan-head { display: flex; align-items: center; gap: var(--space-2); }
  .operation { margin-left: auto; color: var(--color-brand); font-size: var(--text-caption); font-weight: var(--fw-semibold); }
  .packages { list-style: none; margin: var(--space-2) 0; padding: 0; }
  .packages li { display: flex; flex-direction: column; gap: 2px; padding: var(--space-1) 0; }
  .warning { margin: var(--space-1) 0; color: var(--color-warning); font-size: var(--text-caption); }
  .merge-review { margin-top: var(--space-3); padding: var(--space-3); border: 1px solid var(--color-border); border-radius: var(--radius-md); background: var(--color-surface-sunken); }
  .merge-title { margin: 0 0 var(--space-2); color: var(--color-text-primary); font-size: var(--text-body-sm); }
  .merge-actions { display: flex; flex-wrap: wrap; gap: var(--space-2); }
  .choice-status { margin: var(--space-2) 0 0; color: var(--color-text-secondary); font-size: var(--text-caption); }
  .diff-link { margin-top: var(--space-2); padding: 0; background: transparent; color: var(--color-text-link); font-size: var(--text-body-sm); cursor: pointer; }
  .diff-link:hover { text-decoration: underline; }
  .progress { color: var(--color-text-secondary); font-size: var(--text-body-sm); }
  .results { list-style: none; margin: 0; padding: 0; border: 1px solid var(--color-border); border-radius: var(--radius-md); }
  .results li { display: flex; align-items: flex-start; gap: var(--space-2); padding: var(--space-2) var(--space-3); border-bottom: 1px solid var(--color-border); }
  .results li:last-child { border-bottom: 0; }
  .result-mark { color: var(--color-danger); font-weight: var(--fw-bold); }
  .result-mark.ok { color: var(--color-success); }
</style>
