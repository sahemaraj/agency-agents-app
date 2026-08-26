<script lang="ts">
  import { onMount } from "svelte";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import Search from "@lucide/svelte/icons/search";
  import Sparkles from "@lucide/svelte/icons/sparkles";
  import Copy from "@lucide/svelte/icons/copy";
  import Plus from "@lucide/svelte/icons/plus";
  import Modal from "./Modal.svelte";
  import Runbooks from "./Runbooks.svelte";
  import { experts, projectFactoryRun, summarizeExpertPerformance } from "$lib/stores/experts.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { corpus } from "$lib/stores/corpus.svelte";
  import { ui } from "$lib/stores/ui.svelte";
  import { teams } from "$lib/stores/teams.svelte";
  import { activity } from "$lib/stores/activity.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import {
    appErrorMessage,
    isAppError,
    type ExpertActivationPlan,
    type ExpertClient,
    type ExpertCreationRequest,
    type ExpertDefinition,
    type ExpertProposalInput,
    type ExpertResolved,
    type ExpertRun,
    type FactoryWorkOrderInput,
  } from "$lib/types";

  onMount(() => {
    void Promise.all([experts.load(), projects.refresh(), corpus.ensureLoaded()])
      .then(() => recordObservedAttemptExhaustion());
  });

  let tab = $state<"experts" | "drafts" | "runs" | "runbooks">("experts");
  let query = $state("");
  let filter = $state<"all" | "ready" | "setup" | "custom" | "recent">("all");
  let projectPath = $state("");
  let client = $state<ExpertClient | "">("");
  let plan = $state<ExpertActivationPlan | null>(null);
  let planning = $state(false);
  let activating = $state(false);
  let builder = $state<ExpertDefinition | null>(null);
  let creationReview = $state<ExpertCreationRequest | null>(null);
  let runReview = $state<ExpertRun | null>(null);
  let linkedActivationId = $state<string | null>(null);
  let waiverReason = $state("");
  let factoryBuilder = $state<FactoryWorkOrderInput | null>(null);
  let factoryWorkOrder = $state<FactoryWorkOrderInput | null>(null);
  let factoryActionBusy = $state(false);
  let factoryAnnouncement = $state("");
  let factoryWaiverReason = $state("");
  let factoryReviewWaiverReason = $state("");
  const pendingRequests = $derived(experts.requests.filter((request) => request.state === "pending"));

  const knownAgents = $derived(new Map(corpus.agents.map((agent) => [agent.slug, agent])));
  function ready(expert: ExpertResolved): boolean {
    return expert.unresolvedAgents.length === 0 && expert.unresolvedSkills.length === 0 && !expert.unresolvedRunbook;
  }
  const visible = $derived(experts.list.filter((expert) => {
    const hay = `${expert.name} ${expert.summary} ${expert.category} ${expert.tags.join(" ")}`.toLowerCase();
    if (query && !hay.includes(query.trim().toLowerCase())) return false;
    if (filter === "ready" && !ready(expert)) return false;
    if (filter === "setup" && ready(expert)) return false;
    if (filter === "custom" && expert.source !== "custom") return false;
    if (filter === "recent" && !experts.history.some((record) => record.expertId === expert.id)) return false;
    return true;
  }));

  function newExpert(): ExpertDefinition {
    return {
      id: `custom-${Date.now()}`,
      name: "New Expert",
      summary: "",
      category: "Custom",
      tags: [],
      version: 1,
      leadAgent: corpus.agents[0]?.slug ?? "",
      supportingAgents: [],
      requiredSkills: [],
      optionalSkills: [],
      runbook: null,
      preferredClient: null,
      starterPrompt: "Use {{expert}} for {{project}}. Lead with {{leadAgent}} and verify the outcome before completion.",
      qualityContract: { version: 1, checks: [] },
      source: "custom",
    };
  }

  function cloneExpert(expert: ExpertResolved): ExpertDefinition {
    return {
      ...structuredClone(expert),
      id: `custom-${Date.now()}`,
      name: `${expert.name} Copy`,
      source: "custom",
    };
  }

  async function review(): Promise<boolean> {
    const expert = experts.selected;
    if (!expert || !projectPath) return false;
    planning = true;
    try {
      plan = await experts.plan(expert.id, projectPath, client || null);
      return true;
    } catch (error) {
      toast.error("Could not plan activation", isAppError(error) ? appErrorMessage(error) : String(error));
      return false;
    } finally {
      planning = false;
    }
  }

  async function activate() {
    if (!plan || plan.blockers.length) return;
    const reviewedActivationId = linkedActivationId;
    activating = true;
    try {
      const pending = pendingRequests.find((item) => item.expertId === plan?.expert.id && item.projectPath === plan?.projectPath);
      const activation = await experts.activate(plan.expert.id, plan.projectPath, plan.client, factoryWorkOrder);
      if (pending) await experts.resolveRequest(pending.id, true);
      await experts.load();
      activity.log({
        action: "bulk",
        subject: "agent",
        subjectName: plan.expert.name,
        projectPath: plan.projectPath,
        outcome: "ok",
        detail: `Activated Expert with ${plan.rollbackScope.length} new component(s)`,
      });
      const prompt = activation.runId
        ? factoryWorkOrder
          ? `${plan.promptPreview}\n\nFactory Run ID: ${activation.runId}. External workers discover exact project-scoped work through the configured Agency Agents MCP Factory tools. Agency Agents remains the control plane and does not launch or execute the worker.`
          : `${plan.promptPreview}\n\nRun ID: ${activation.runId}. Read the quality contract with expert_runs_get_contract, then submit evidence for each check before requesting review.`
        : plan.promptPreview;
      await navigator.clipboard.writeText(prompt);
      toast.success(factoryWorkOrder
        ? `${plan.expert.name} Factory Run created; starter prompt copied`
        : `${plan.expert.name} activated; starter prompt copied`);
      closeLinkedReview(() => { plan = null; linkedActivationId = null; factoryWorkOrder = null; }, "activation", reviewedActivationId);
    } catch (error) {
      activity.log({
        action: "bulk",
        subject: "agent",
        subjectName: plan?.expert.name,
        projectPath: plan?.projectPath,
        outcome: "error",
        detail: isAppError(error) ? appErrorMessage(error) : String(error),
      });
      toast.error("Activation failed", isAppError(error) ? appErrorMessage(error) : String(error));
    } finally {
      activating = false;
    }
  }

  function newFactoryWorkOrder(): FactoryWorkOrderInput {
    return {
      ticketReference: "",
      title: "",
      objective: "",
      acceptanceCriteria: [],
      nonGoals: [],
      playbook: null,
      workspacePackRevision: null,
      risk: "medium",
    };
  }

  function validFactoryWorkOrder(workOrder: FactoryWorkOrderInput | null): boolean {
    return !!workOrder
      && !!workOrder.ticketReference.trim()
      && !!workOrder.title.trim()
      && !!workOrder.objective.trim()
      && workOrder.acceptanceCriteria.some((item) => item.trim());
  }

  async function reviewFactoryCreation() {
    if (!validFactoryWorkOrder(factoryBuilder)) return;
    factoryWorkOrder = {
      ...factoryBuilder!,
      ticketReference: factoryBuilder!.ticketReference.trim(),
      title: factoryBuilder!.title.trim(),
      objective: factoryBuilder!.objective.trim(),
      acceptanceCriteria: factoryBuilder!.acceptanceCriteria.map((item) => item.trim()).filter(Boolean),
      nonGoals: factoryBuilder!.nonGoals.map((item) => item.trim()).filter(Boolean),
      playbook: factoryBuilder!.playbook?.trim() || null,
      workspacePackRevision: null,
    };
    factoryBuilder = null;
    const opened = await review();
    if (!opened) factoryWorkOrder = null;
  }

  async function copyPrompt(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      toast.success("Starter prompt copied");
    } catch (error) {
      toast.error("Copy failed", String(error));
    }
  }

  async function saveBuilder() {
    if (!builder) return;
    const reviewedCreationId = creationReview?.id ?? null;
    try {
      if (creationReview) {
        await experts.approveCreation(creationReview.id, proposalFrom(builder));
        activity.log({
          action: "bulk",
          subject: "agent",
          subjectName: builder.name,
          projectPath: creationReview.projectPath,
          outcome: "ok",
          detail: "Approved Expert proposal",
        });
        creationReview = null;
      } else {
        await experts.save(builder);
      }
      closeLinkedReview(() => (builder = null), "change", reviewedCreationId);
      toast.success("Expert saved");
    } catch (error) {
      toast.error("Could not save Expert", isAppError(error) ? appErrorMessage(error) : String(error));
    }
  }

  function proposalFrom(expert: ExpertDefinition): ExpertProposalInput {
    const { id: _id, version: _version, source: _source, ...proposal } = expert;
    return proposal;
  }

  function reviewCreation(request: ExpertCreationRequest, preserveIntent = false) {
    if (!preserveIntent) ui.clearReviewIntent();
    creationReview = request;
    builder = {
      id: `proposal-${request.id}`,
      version: 1,
      source: "custom",
      ...structuredClone($state.snapshot(request.proposal)),
    };
  }

  async function rejectCreation(request: ExpertCreationRequest) {
    try {
      await experts.rejectCreation(request.id);
      activity.log({
        action: "bulk",
        subject: "agent",
        subjectName: request.proposal.name,
        projectPath: request.projectPath,
        outcome: "ok",
        detail: "Rejected Expert proposal",
      });
    } catch (error) {
      toast.error("Could not reject Expert proposal", isAppError(error) ? appErrorMessage(error) : String(error));
    }
  }

  function missingRequired(run: ExpertRun | null): string[] {
    if (!run) return [];
    return run.contract.checks.filter((check) => check.required && !run.evidence.some((evidence) => evidence.checkName === check.name && evidence.result === "pass")).map((check) => check.name);
  }

  function reportedChecks(run: ExpertRun): number {
    return new Set(run.evidence.map((item) => item.checkName)).size;
  }

  async function finishRun(verdict: "accepted" | "rework" | "rejected" | "cancelled", waive = false) {
    if (!runReview) return;
    const reviewedRunId = runReview.id;
    const waivers = waive ? missingRequired(runReview).map((checkName) => ({ checkName, reason: waiverReason.trim() })) : [];
    try {
      await experts.reviewRun(runReview.id, verdict, waivers);
      closeLinkedReview(() => (runReview = null), "run", reviewedRunId);
      waiverReason = "";
      toast.success(`Run marked ${verdict}`);
    } catch (error) {
      toast.error("Could not review run", isAppError(error) ? appErrorMessage(error) : String(error));
    }
  }

  function projectLabel(run: ExpertRun): string {
    return projects.list.find((project) => project.path === run.projectPath)?.label ?? "Registered project";
  }

  function recordObservedAttemptExhaustion() {
    for (const run of experts.runs) {
      if (run.factory?.terminal?.outcome === "attemptExhausted") {
        activity.recordFactoryRunReceipt(run, projectLabel(run));
      }
    }
  }

  async function reloadExpertRuns() {
    await experts.load();
    recordObservedAttemptExhaustion();
  }

  function factoryElapsed(createdAt: string, endedAt: string | null): string {
    const start = Date.parse(createdAt);
    const end = endedAt ? Date.parse(endedAt) : Date.now();
    if (!Number.isFinite(start) || !Number.isFinite(end)) return "unknown";
    const minutes = Math.max(0, Math.floor((end - start) / 60_000));
    if (minutes < 1) return "<1m";
    if (minutes < 60) return `${minutes}m`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `${hours}h ${minutes % 60}m`;
    return `${Math.floor(hours / 24)}d ${hours % 24}h`;
  }

  async function refreshFactoryReview(id: string, error: unknown) {
    try {
      await experts.refreshRuns();
      recordObservedAttemptExhaustion();
      const refreshed = experts.runs.find((run) => run.id === id && run.factory);
      if (refreshed) runReview = refreshed;
      factoryAnnouncement = refreshed
        ? "Factory run changed. Current revision loaded; review the refreshed evidence before deciding."
        : "Factory run is no longer available.";
    } catch {
      factoryAnnouncement = "Factory action failed and current state could not be refreshed.";
    }
    toast.error("Factory action failed", isAppError(error) ? appErrorMessage(error) : String(error));
  }

  function recordFactoryReceipt(run: ExpertRun): string | null {
    return activity.recordFactoryRunReceipt(run, projectLabel(run));
  }

  async function decideFactoryPlan(decision: "approve" | "reject") {
    if (!runReview) return;
    const id = runReview.id;
    const projection = projectFactoryRun(runReview);
    if (projection?.humanAction?.kind !== "plan" || !projection.workflow.plan) return;
    factoryActionBusy = true;
    factoryAnnouncement = decision === "approve" ? "Approving current Factory plan." : "Rejecting current Factory plan.";
    try {
      await experts.factoryPlanDecide(
        id,
        projection.humanAction.expectedRevision,
        projection.workflow.plan.revision,
        decision,
      );
      factoryAnnouncement = decision === "approve" ? "Factory plan approved." : "Factory plan rejected.";
      closeLinkedReview(() => (runReview = null), "run", id);
    } catch (error) {
      await refreshFactoryReview(id, error);
    } finally {
      factoryActionBusy = false;
    }
  }

  function factoryMissingChecks(run: ExpertRun | null): string[] {
    if (!run) return [];
    const projection = projectFactoryRun(run);
    if (!projection) return [];
    const latest = new Map(projection.latestEvidence.map((item) => [item.checkName, item.result]));
    return projection.workflow.workContract.qualityContract.checks
      .filter((check) => check.required && latest.get(check.name) !== "pass")
      .map((check) => check.name);
  }

  async function decideFactoryFinal(outcome: "accepted" | "rework" | "rejected") {
    if (!runReview) return;
    const id = runReview.id;
    const projection = projectFactoryRun(runReview);
    if (projection?.humanAction?.kind !== "final"
      || !projection.workflow.planApproval
      || !projection.headCommit) return;
    const missingChecks = factoryMissingChecks(runReview);
    const missingReview = projection.workflow.review?.verdict !== "pass"
      && !projection.workflow.humanWaivers.some((waiver) => waiver.kind === "independentReview");
    if (outcome === "accepted" && (missingReview || (missingChecks.length > 0 && !factoryWaiverReason.trim()))) return;
    factoryActionBusy = true;
    factoryAnnouncement = "Recording Factory " + outcome + " decision.";
    try {
      const updated = await experts.factoryFinalDecide(id, {
        expectedRevision: projection.humanAction.expectedRevision,
        outcome,
        approvedPlanRevision: projection.workflow.planApproval.planRevision,
        headCommit: projection.headCommit,
        checkWaivers: outcome === "accepted"
          ? missingChecks.map((checkName) => ({ checkName, reason: factoryWaiverReason.trim() }))
          : [],
        independentReviewWaiverReason: null,
        safeDetail: null,
      });
      const receiptId = recordFactoryReceipt(updated);
      factoryAnnouncement = "Factory result marked " + outcome + ".";
      factoryWaiverReason = "";
      closeLinkedReview(() => (runReview = null), "run", id);
      if (receiptId) ui.openActivityReceipt(receiptId);
    } catch (error) {
      await refreshFactoryReview(id, error);
    } finally {
      factoryActionBusy = false;
    }
  }

  async function cancelFactoryRun() {
    if (!runReview) return;
    const id = runReview.id;
    const projection = projectFactoryRun(runReview);
    if (!projection || projection.workflow.terminal) return;
    factoryActionBusy = true;
    factoryAnnouncement = "Cancelling Factory control-plane authority.";
    try {
      const updated = await experts.factoryCancel(id, projection.workflow.revision, null);
      const receiptId = recordFactoryReceipt(updated);
      factoryAnnouncement = "Factory Run cancelled. External work may still need to be stopped separately.";
      closeLinkedReview(() => (runReview = null), "run", id);
      if (receiptId) ui.openActivityReceipt(receiptId);
    } catch (error) {
      await refreshFactoryReview(id, error);
    } finally {
      factoryActionBusy = false;
    }
  }

  async function releaseFactoryClaim() {
    if (!runReview) return;
    const id = runReview.id;
    const projection = projectFactoryRun(runReview);
    if (!projection?.workflow.currentClaim) return;
    factoryActionBusy = true;
    factoryAnnouncement = "Releasing current Factory claim.";
    try {
      runReview = await experts.factoryReleaseClaim(id, projection.workflow.revision);
      factoryAnnouncement = "Factory claim released. External work was not stopped.";
    } catch (error) {
      await refreshFactoryReview(id, error);
    } finally {
      factoryActionBusy = false;
    }
  }

  async function waiveFactoryReview() {
    if (!runReview || !factoryReviewWaiverReason.trim()) return;
    const id = runReview.id;
    const projection = projectFactoryRun(runReview);
    if (!projection || projection.phase !== "independentReview" || !projection.workflow.validation) return;
    factoryActionBusy = true;
    factoryAnnouncement = "Waiving independent review for the current validated Factory head.";
    try {
      runReview = await experts.factoryWaiveReview(
        id,
        projection.workflow.revision,
        factoryReviewWaiverReason.trim(),
      );
      factoryReviewWaiverReason = "";
      factoryAnnouncement = "Independent review waived; Factory Run advanced to Delivery.";
    } catch (error) {
      await refreshFactoryReview(id, error);
    } finally {
      factoryActionBusy = false;
    }
  }

  async function resolveFactoryBlocker(blockerId: string) {
    if (!runReview) return;
    const id = runReview.id;
    const projection = projectFactoryRun(runReview);
    if (!projection) return;
    factoryActionBusy = true;
    factoryAnnouncement = "Resolving Factory blocker.";
    try {
      runReview = await experts.factoryResolveBlocker(id, projection.workflow.revision, blockerId);
      factoryAnnouncement = "Factory blocker resolved; current claim was released.";
    } catch (error) {
      await refreshFactoryReview(id, error);
    } finally {
      factoryActionBusy = false;
    }
  }

  function normalizedSkill(value: string): string {
    return value.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
  }

  const pendingRequiredSkill = $derived(creationReview !== null && builder !== null && builder.requiredSkills.some((name) => {
    const normalized = normalizedSkill(name);
    return creationReview!.linkedSkillStates.some((link) =>
      normalizedSkill(link.skillName) === normalized && link.state === "pending"
    );
  }));

  async function reviewRequest(request: (typeof experts.requests)[number], preserveIntent = false) {
    if (!preserveIntent) ui.clearReviewIntent();
    projectPath = request.projectPath;
    client = request.client ?? "";
    experts.selectedId = request.expertId;
    const opened = await review();
    if (!opened && preserveIntent) {
      linkedActivationId = null;
      ui.returnToActivityReview("expert-activation", request.id);
    }
  }

  let handledReviewLink = $state("");
  $effect(() => {
    const link = ui.expertReview;
    if (!link || experts.loading || handledReviewLink === `${link.kind}:${link.id}`) return;
    if (link.kind === "change") {
      const request = experts.creationRequests.find((item) => item.id === link.id);
      if (!request) return;
      tab = "drafts";
      reviewCreation(request, true);
    } else if (link.kind === "run") {
      const run = experts.runs.find((item) => item.id === link.id
        && (item.state === "awaitingReview" || !!projectFactoryRun(item)?.humanAction));
      if (!run) return;
      tab = "runs";
      runReview = run;
      waiverReason = "";
      factoryWaiverReason = "";
      factoryReviewWaiverReason = "";
    } else {
      const request = experts.requests.find((item) => item.id === link.id && item.state === "pending");
      if (!request) return;
      tab = "experts";
      linkedActivationId = link.id;
      void reviewRequest(request, true);
    }
    handledReviewLink = `${link.kind}:${link.id}`;
    ui.expertReview = null;
  });

  $effect(() => {
    const link = ui.expertReview;
    if (!link || experts.loading) return;
    const found = link.kind === "change"
      ? experts.creationRequests.some((item) => item.id === link.id)
      : link.kind === "run"
        ? experts.runs.some((item) => item.id === link.id
          && (item.state === "awaitingReview" || !!projectFactoryRun(item)?.humanAction))
        : experts.requests.some((item) => item.id === link.id && item.state === "pending");
    if (found) return;
    ui.expertReview = null;
    ui.returnToActivityReview(`expert-${link.kind}`, link.id);
  });

  function closeLinkedReview(close: () => void, kind: "change" | "run" | "activation" | null = null, id: string | null = null) {
    close();
    if (kind && id && ui.returnToActivityReview(`expert-${kind}`, id)) return;
    if (ui.reviewIntent) ui.clearReviewIntent();
  }

  async function importExperts() {
    const path = await open({ multiple: false, filters: [{ name: "Expert JSON", extensions: ["json"] }] });
    if (typeof path !== "string") return;
    try { toast.success(`Imported ${await experts.importFile(path)} Expert(s)`); }
    catch (error) { toast.error("Import failed", isAppError(error) ? appErrorMessage(error) : String(error)); }
  }

  async function exportExperts() {
    const path = await save({ defaultPath: "experts.json", filters: [{ name: "Expert JSON", extensions: ["json"] }] });
    if (!path) return;
    try { toast.success(`Exported ${await experts.exportFile(path)} Expert(s)`); }
    catch (error) { toast.error("Export failed", isAppError(error) ? appErrorMessage(error) : String(error)); }
  }

  function saveRoster(expert: ExpertResolved) {
    teams.hydrate();
    teams.save(expert.name, [expert.leadAgent, ...expert.supportingAgents]);
    toast.success("Roster saved as Team");
  }
</script>

<section class="experts">
  <div class="sr-only" role="status" aria-live="polite" aria-atomic="true">{factoryAnnouncement}</div>
  <header class="head">
    <div><h1>Experts</h1><p>Project-ready specialist workspaces.</p></div>
    <div class="tabs" role="tablist">
      <button role="tab" aria-selected={tab === "experts"} class:active={tab === "experts"} onclick={() => (tab = "experts")}>Experts</button>
      <button role="tab" aria-selected={tab === "drafts"} class:active={tab === "drafts"} onclick={() => { tab = "drafts"; void experts.load(); }}>Changes ({experts.creationRequests.filter((request) => request.state === "pending").length})</button>
      <button role="tab" aria-selected={tab === "runs"} class:active={tab === "runs"} onclick={() => { tab = "runs"; void reloadExpertRuns(); }}>Runs ({experts.runs.length})</button>
      <button role="tab" aria-selected={tab === "runbooks"} class:active={tab === "runbooks"} onclick={() => (tab = "runbooks")}>Runbooks</button>
    </div>
  </header>

  {#if tab === "runbooks"}
    <div class="runbooks"><Runbooks /></div>
  {:else if tab === "runs"}
    <div class="drafts" aria-label="Expert runs">
      {#if experts.runs.length === 0}<p class="empty">No Expert runs.</p>
      {:else}{#each experts.runs as run (run.id)}
        {@const factory = projectFactoryRun(run)}
        <article class="draft-card">
          {#if factory}
            <div class="title">
              <div><h2>{factory.workflow.workContract.title}</h2><p>{projectLabel(run)} · {run.client} · {new Date(factory.workflow.createdAt).toLocaleString()} · Elapsed {factoryElapsed(factory.workflow.createdAt, factory.workflow.terminal?.decidedAt ?? null)}</p></div>
              <span class:ready={!!factory.workflow.terminal} class="status">{factory.phaseLabel}</span>
            </div>
            <p>Attempt {factory.attempt} of {factory.maxAttempts} · Head {factory.headCommit ?? "not reported"} · Validation {factory.workflow.validation ? "reported" : "pending"} · {factory.latestEvidence.length} current client-reported check(s) · {factory.workflow.blockers.filter((item) => !item.resolvedAt).length} blockers</p>
            {#if factory.claimant}<p>Claimed by {factory.claimant}</p>{/if}
            {#if factory.blocker}<div class="warning">{factory.blocker}</div>{/if}
            <footer><button onclick={() => { ui.clearReviewIntent(); runReview = run; factoryWaiverReason = ""; factoryReviewWaiverReason = ""; }}>
              {factory.humanAction?.kind === "plan" ? "Review plan" : factory.humanAction?.kind === "final" ? "Review final result" : "View Factory run"}
            </button></footer>
          {:else}
            <div class="title"><div><h2>{experts.list.find((expert) => expert.id === run.expertId)?.name ?? run.expertId}</h2><p>{run.projectPath} · {run.client} · {new Date(run.startedAt).toLocaleString()}</p></div><span class:ready={run.state === "accepted"} class="status">{run.state}</span></div>
            <p>{reportedChecks(run)}/{run.contract.checks.length} checks reported · {run.blockers.length} blockers</p>
            {#if run.state === "awaitingReview"}<footer><button onclick={() => { ui.clearReviewIntent(); runReview = run; waiverReason = ""; }}>Review run</button></footer>{/if}
          {/if}
        </article>
      {/each}{/if}
    </div>
  {:else if tab === "drafts"}
    <div class="drafts" aria-label="Expert change requests">
      {#if experts.loading}
        <p class="empty">Loading Expert changes…</p>
      {:else if experts.error}
        <p class="empty">{experts.error}</p>
      {:else if experts.creationRequests.length === 0}
        <p class="empty">No Expert change requests.</p>
      {:else}
        {#each experts.creationRequests as request (request.id)}
          <article class="draft-card">
            <div class="title">
              <div>
                <h2>{request.kind}: {request.proposal.name}</h2>
                <p>{request.requestedBy} · {request.projectPath} · {new Date(request.requestedAt).toLocaleString()}</p>
              </div>
              <span class:ready={request.readiness === "ready"} class="status">{request.readiness}</span>
            </div>
            {#if request.blockers.length}
              <div class="warning">{request.blockers.join(" · ")}</div>
            {/if}
            {#if request.agentSubstitutions.length}
              <section>
                <h3>Agent substitutions</h3>
                {#each request.agentSubstitutions as substitution}
                  <p>{substitution.neededCapability} → {substitution.selectedCatalogSlug}: {substitution.rationale}</p>
                {/each}
              </section>
            {/if}
            {#if request.linkedSkillStates.length}
              <section>
                <h3>Linked skills</h3>
                <div class="chips">
                  {#each request.linkedSkillStates as skill}
                    <button onclick={() => ui.setSection("skills")}>{skill.skillName} · {skill.state ?? "missing"}</button>
                  {/each}
                </div>
              </section>
            {/if}
            <footer>
              {#if request.state === "pending"}
                <button onclick={() => rejectCreation(request)}>Reject</button>
                <button class="primary" onclick={() => reviewCreation(request)}>Review</button>
              {:else}
                <span>{request.state}{request.savedExpertId ? ` · ${request.savedExpertId}` : ""}</span>
              {/if}
            </footer>
          </article>
        {/each}
      {/if}
    </div>
  {:else}
    <div class="toolbar">
      <label class="search"><Search size={14} /><input aria-label="Search Experts" placeholder="Search Experts" bind:value={query} /></label>
      <select aria-label="Project" bind:value={projectPath}>
        <option value="">Select project…</option>
        {#each projects.list as project (project.path)}<option value={project.path}>{project.label}</option>{/each}
      </select>
      <button class="primary" onclick={() => (builder = newExpert())}><Plus size={14} /> New Expert</button>
      <button onclick={importExperts}>Import</button>
      <button onclick={exportExperts}>Export</button>
    </div>
    <div class="filters">
      {#each [["all","All"],["ready","Ready"],["setup","Needs setup"],["custom","Custom"],["recent","Recently used"]] as item}
        <button class:active={filter === item[0]} onclick={() => (filter = item[0] as typeof filter)}>{item[1]}</button>
      {/each}
    </div>
    {#if pendingRequests.length}
      <div class="inbox">
        <strong>Activation requests</strong>
        {#each pendingRequests as request (request.id)}
          <span>{request.requestedBy} requested {experts.list.find((item) => item.id === request.expertId)?.name ?? request.expertId}</span>
          <button onclick={() => reviewRequest(request)}>Review</button>
          <button onclick={() => experts.resolveRequest(request.id, false)}>Reject</button>
        {/each}
      </div>
    {/if}

    <div class="workspace">
      <aside class="list" aria-label="Experts">
        {#if experts.loading}<p class="empty">Loading…</p>
        {:else if experts.error}<p class="empty">{experts.error}</p>
        {:else if visible.length === 0}<p class="empty">No Experts match this view.</p>
        {:else}
          {#each visible as expert (expert.id)}
            <button class:selected={experts.selected?.id === expert.id} onclick={() => (experts.selectedId = expert.id)}>
              <span class="row-title">{expert.name}</span>
              <span class="row-meta">{knownAgents.get(expert.leadAgent)?.name ?? expert.leadAgent} · {expert.preferredClient ?? "Choose client"}</span>
              <span class:ready={ready(expert)} class="status">{ready(expert) ? "Ready" : "Needs setup"}</span>
            </button>
          {/each}
        {/if}
      </aside>

      <article class="detail">
        {#if experts.selected}
          {@const expert = experts.selected}
          {@const performance = summarizeExpertPerformance(expert, experts.runs)}
          <div class="title"><div><h2>{expert.name}</h2><p>{expert.summary}</p></div><span class="source">{expert.source}</span></div>
          <section><h3>Expected outcome</h3><p>{expert.summary}</p></section>
          <section><h3>Quality contract</h3><p>{expert.qualityContract.checks.length ? expert.qualityContract.checks.map((check) => `${check.name} · ${check.required ? "required" : "optional"}`).join(" · ") : "No checks configured"}</p></section>
          <section class="performance">
            <h3>Performance and Improvement Coach</h3>
            {#if performance.eligible}
              <p>Based on {performance.comparableRuns} comparable terminal runs with this Expert version and quality contract.</p>
              <div class="metrics">
                <strong>Acceptance rate {performance.acceptanceRate}%</strong>
                <span>Rework {performance.rework} · Rejected {performance.rejected} · Runs with waivers {performance.waiverRate}%</span>
              </div>
              <ul>{#each performance.suggestions as suggestion}<li>{suggestion}</li>{/each}</ul>
            {:else}
              <p>{performance.comparableRuns} of 5 comparable terminal runs. Metrics and suggestions appear after five quality verdicts for this exact Expert version and contract.</p>
            {/if}
          </section>
          <section>
            <h3>Roster</h3>
            <button class="link" onclick={() => ui.openAgents(null)}>{knownAgents.get(expert.leadAgent)?.name ?? expert.leadAgent} · lead</button>
            {#each expert.supportingAgents as slug}<button class="link" onclick={() => ui.openAgents(null)}>{knownAgents.get(slug)?.name ?? slug}</button>{/each}
          </section>
          <section>
            <h3>Skills</h3>
            <div class="chips">
              {#each expert.requiredSkills as skill}<button onclick={() => ui.setSection("skills")}>{skill} · required</button>{/each}
              {#each expert.optionalSkills as skill}<button onclick={() => ui.setSection("skills")}>{skill} · optional</button>{/each}
            </div>
          </section>
          <section class="grid">
            <label>Client<select bind:value={client}><option value="">Use preference</option><option value="claudeCode">Claude Code</option><option value="codex">Codex</option></select></label>
            <div><h3>Readiness</h3><strong class:ok={ready(expert)}>{ready(expert) ? "Ready" : "Blocked"}</strong></div>
          </section>
          {#if expert.unresolvedAgents.length || expert.unresolvedSkills.length || expert.unresolvedRunbook}
            <div class="warning">Unresolved: {[...expert.unresolvedAgents, ...expert.unresolvedSkills, ...(expert.unresolvedRunbook ? [expert.runbook ?? "runbook"] : [])].join(", ")}</div>
          {/if}
          <section><h3>Starter prompt</h3><pre>{expert.starterPrompt}</pre><button onclick={() => copyPrompt(expert.starterPrompt)}><Copy size={14} /> Copy</button></section>
          <footer>
            {#if expert.source === "custom"}<button onclick={() => experts.remove(expert.id)}>Delete</button>{/if}
            <button onclick={() => (builder = cloneExpert(expert))}>Clone and customize</button>
            <button onclick={() => saveRoster(expert)}>Save roster as Team</button>
            <button disabled={!projectPath || planning} onclick={() => { ui.clearReviewIntent(); factoryBuilder = newFactoryWorkOrder(); }}>Create Factory Run</button>
            <button class="primary" disabled={!projectPath || planning} onclick={() => { ui.clearReviewIntent(); factoryWorkOrder = null; void review(); }}><Sparkles size={14} /> {planning ? "Planning…" : "Activate Expert"}</button>
          </footer>
        {:else}<p class="empty">Select an Expert.</p>{/if}
      </article>
    </div>
  {/if}
</section>

{#if plan}
  <Modal open title={`Review ${plan.expert.name}`} size="wide" onClose={() => closeLinkedReview(() => { plan = null; linkedActivationId = null; factoryWorkOrder = null; }, "activation", linkedActivationId)}>
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div class="review" role="region" aria-label="Activation review details" tabindex="0">
      <p><strong>Destination:</strong> {plan.projectPath}</p>
      <p><strong>Client:</strong> {plan.client}</p>
      <h3>Agents</h3>
      {#each plan.agents as agent}<p>{agent.slug} — {agent.status}{agent.destination ? ` → ${agent.destination}` : ""}</p>{/each}
      <h3>Skills and dependencies</h3>
      {#each plan.skills as skill}{#each skill.packages as pkg}<p>{pkg.dependency ? "Dependency" : "Skill"}: {pkg.name} → {pkg.destination}</p>{/each}{/each}
      {#if plan.blockers.length}<div class="warning"><strong>Blocked</strong>{#each plan.blockers as blocker}<p>{blocker}</p>{/each}</div>{/if}
      <h3>Rollback scope</h3><p>{plan.rollbackScope.join(", ") || "No new components"}</p>
      <h3>Quality contract</h3><p>{plan.expert.qualityContract.checks.length ? plan.expert.qualityContract.checks.map((check) => check.name).join(", ") : "No checks configured"}</p>
      {#if factoryWorkOrder}
        <h3>Factory work order</h3>
        <p><strong>{factoryWorkOrder.ticketReference} · {factoryWorkOrder.title}</strong></p>
        <p>{factoryWorkOrder.objective}</p>
        <p><strong>Acceptance criteria:</strong> {factoryWorkOrder.acceptanceCriteria.join(" · ")}</p>
        <p><strong>Non-goals:</strong> {factoryWorkOrder.nonGoals.join(" · ")}</p>
        <p><strong>Risk:</strong> {factoryWorkOrder.risk}</p>
        {#if factoryWorkOrder.playbook}<p><strong>Playbook:</strong> {factoryWorkOrder.playbook}</p>{/if}
        {#if factoryWorkOrder.workspacePackRevision}<p><strong>Workspace Pack revision:</strong> {factoryWorkOrder.workspacePackRevision}</p>{/if}
      {/if}
      <h3>Generated starter prompt</h3><pre>{plan.promptPreview}</pre>
    </div>
    {#snippet actions()}
      <button onclick={() => closeLinkedReview(() => { plan = null; linkedActivationId = null; factoryWorkOrder = null; }, "activation", linkedActivationId)}>Cancel</button>
      <button class="primary" disabled={(plan?.blockers.length ?? 1) > 0 || activating} onclick={activate}>{activating ? "Activating…" : factoryWorkOrder ? "Create Factory Run" : "Approve activation"}</button>
    {/snippet}
  </Modal>
{/if}

{#if factoryBuilder}
  <Modal open title="Create Factory Run" size="wide" defaultFocus="first" onClose={() => (factoryBuilder = null)}>
    <form class="builder" onsubmit={(event) => { event.preventDefault(); void reviewFactoryCreation(); }}>
      <p>Agency Agents governs the contract and approvals. External Claude Code or Codex workers perform implementation work.</p>
      <label>Ticket reference<input aria-label="Ticket reference" required maxlength="160" bind:value={factoryBuilder.ticketReference} /></label>
      <label>Work-order title<input aria-label="Work-order title" required maxlength="160" bind:value={factoryBuilder.title} /></label>
      <label>Objective<textarea aria-label="Objective" required maxlength="4096" rows="4" bind:value={factoryBuilder.objective}></textarea></label>
      <label>Acceptance criteria<textarea aria-label="Acceptance criteria" required maxlength="8192" rows="5" placeholder="One criterion per line" value={factoryBuilder.acceptanceCriteria.join("\n")} oninput={(event) => (factoryBuilder!.acceptanceCriteria = event.currentTarget.value.split("\n"))}></textarea></label>
      <label>Non-goals (optional)<textarea aria-label="Non-goals" maxlength="4096" rows="4" placeholder="One non-goal per line" value={factoryBuilder.nonGoals.join("\n")} oninput={(event) => (factoryBuilder!.nonGoals = event.currentTarget.value.split("\n"))}></textarea></label>
      <label>Playbook (optional)<input aria-label="Playbook" maxlength="512" value={factoryBuilder.playbook ?? ""} oninput={(event) => (factoryBuilder!.playbook = event.currentTarget.value || null)} /></label>
      <p class="field-help">Workspace Pack binding is unavailable until the app can verify a selected pack revision.</p>
      <label>Risk<select aria-label="Risk" bind:value={factoryBuilder.risk}><option value="low">Low</option><option value="medium">Medium</option><option value="high">High</option></select></label>
    </form>
    {#snippet actions()}
      <button data-modal-action="cancel" onclick={() => (factoryBuilder = null)}>Cancel</button>
      <button data-modal-action="confirm" class="primary" disabled={!validFactoryWorkOrder(factoryBuilder)} onclick={() => void reviewFactoryCreation()}>Review Factory Run</button>
    {/snippet}
  </Modal>
{/if}

{#if runReview}
  {@const factoryReview = projectFactoryRun(runReview)}
  <Modal open title={`Review run ${runReview.id.slice(0, 8)}`} size="wide" onClose={() => closeLinkedReview(() => (runReview = null), "run", runReview?.id ?? null)}>
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div class="review" role="region" aria-label="Run review details" tabindex="0">
      {#if factoryReview}
        <p><strong>{factoryReview.workflow.workContract.ticketReference} · {factoryReview.workflow.workContract.title}</strong></p>
        <p><strong>Project:</strong> {projectLabel(runReview)}</p>
        <p><strong>Phase:</strong> {factoryReview.phaseLabel} · revision {factoryReview.workflow.revision}</p>
        <p><strong>Attempt:</strong> {factoryReview.attempt} of {factoryReview.maxAttempts}</p>
        <p><strong>Elapsed:</strong> {factoryElapsed(factoryReview.workflow.createdAt, factoryReview.workflow.terminal?.decidedAt ?? null)}</p>
        <p><strong>Head:</strong> {factoryReview.headCommit ?? "Not reported"} · <strong>Validation:</strong> {factoryReview.workflow.validation ? `reported ${new Date(factoryReview.workflow.validation.validatedAt).toLocaleString()}` : "pending"}</p>
        <p><strong>Provenance:</strong> External execution and evidence are client-reported.</p>
        {#if factoryReview.workflow.currentClaim}
          <p><strong>Claim:</strong> {factoryReview.workflow.currentClaim.workerIdentity} · generation {factoryReview.workflow.currentClaim.generation} · expires {new Date(factoryReview.workflow.currentClaim.expiresAt).toLocaleString()}</p>
        {/if}
        {#if factoryReview.workflow.blockers.some((blocker) => !blocker.resolvedAt)}
          <h3>Active blockers</h3>
          {#each factoryReview.workflow.blockers.filter((blocker) => !blocker.resolvedAt) as blocker (blocker.id)}
            <div class="warning"><p>{blocker.kind}: {blocker.summary}</p><button disabled={factoryActionBusy} onclick={() => void resolveFactoryBlocker(blocker.id)}>Resolve blocker</button></div>
          {/each}
        {/if}
        {#if factoryReview.workflow.plan}
          <h3>Plan revision {factoryReview.workflow.plan.revision}</h3>
          <p><strong>Client-reported base:</strong> {factoryReview.workflow.plan.baseCommit}</p>
          <pre>{factoryReview.workflow.plan.content}</pre>
          {#if factoryReview.workflow.plan.risks.length}<p><strong>Risks:</strong> {factoryReview.workflow.plan.risks.join(" · ")}</p>{/if}
          {#if factoryReview.workflow.plan.knownLimitations.length}<p><strong>Known limitations:</strong> {factoryReview.workflow.plan.knownLimitations.join(" · ")}</p>{/if}
        {/if}
        <h3>Current evidence</h3>
        {#if factoryReview.latestEvidence.length === 0}<p>No current bound evidence.</p>{/if}
        {#each factoryReview.latestEvidence as evidence (evidence.checkName)}
          <p>{evidence.checkName} — {evidence.result}: {evidence.summary} · client-reported{evidence.commandLabel ? ` · ${evidence.commandLabel}` : ""}</p>
        {/each}
        {#if factoryReview.workflow.review}
          <h3>Independent review</h3><p>{factoryReview.workflow.review.verdict}: {factoryReview.workflow.review.summary} · client-reported by a distinct worker session</p>
        {:else if factoryReview.phase === "independentReview"}
          <h3>Independent review</h3>
          <p>No distinct worker-session review has been reported for the current validated head.</p>
          <label>Independent Review waiver reason<textarea aria-label="Independent Review waiver reason" maxlength="4096" bind:value={factoryReviewWaiverReason}></textarea></label>
          <p>This desktop waiver advances the current validated head to Delivery. The reason is not copied into Activity.</p>
        {/if}
        {#if factoryReview.workflow.delivery}
          <h3>Delivery</h3>
          <p><strong>Client-reported head:</strong> {factoryReview.workflow.delivery.headCommit}</p>
          <p class="delivery-reference"><strong>HTTPS reference:</strong> {factoryReview.workflow.delivery.reference}</p>
          <p><strong>Evidence summary:</strong> {factoryReview.workflow.delivery.evidenceSummary}</p>
          <p><strong>Known limitations:</strong> {factoryReview.workflow.delivery.knownLimitations.join(" · ") || "None reported"}</p>
        {/if}
        {#if factoryReview.workflow.improvementProposal}
          <h3>Improvement coach: client-reported proposal</h3>
          <p>{factoryReview.workflow.improvementProposal.failureClass} · {factoryReview.workflow.improvementProposal.target}</p>
          <p>{factoryReview.workflow.improvementProposal.proposal}</p>
          {#if factoryReview.workflow.improvementProposal.suggestedTest}<p>Suggested test: {factoryReview.workflow.improvementProposal.suggestedTest}</p>{/if}
          <p>This proposal is inert; Agency Agents will not apply, publish, install, approve, or share it automatically.</p>
        {/if}
        {#if !factoryReview.workflow.terminal}
          <div class="warning"><strong>Cancellation scope</strong><p>Agency Agents cannot stop an external process or delete its branch, worktree, artifacts, evidence, or repository changes. Stop external work separately if needed.</p></div>
        {/if}
        {#if factoryReview.humanAction?.kind === "final" && factoryMissingChecks(runReview).length}
          <label>Final waiver reason<textarea maxlength="4096" bind:value={factoryWaiverReason}></textarea></label>
        {/if}
        {#if factoryReview.humanAction?.kind === "final"
          && factoryReview.workflow.review?.verdict !== "pass"
          && !factoryReview.workflow.humanWaivers.some((waiver) => waiver.kind === "independentReview")}
          <div class="warning">Acceptance is blocked until independent review passes or is waived during Independent Review.</div>
        {/if}
      {:else}
      <p><strong>Expert:</strong> {runReview.expertId} v{runReview.expertVersion}</p>
      <p><strong>Project:</strong> {runReview.projectPath}</p>
      <h3>Checks</h3>
      {#each runReview.contract.checks as check}
        {@const evidence = runReview.evidence.filter((item) => item.checkName === check.name).at(-1)}
        <p>{check.name} · {check.required ? "required" : "optional"} — {evidence?.result ?? "missing"}{evidence?.summary ? `: ${evidence.summary}` : ""}</p>
      {/each}
      {#if runReview.blockers.length}<h3>Blockers</h3>{#each runReview.blockers as blocker}<p>{blocker.kind}: {blocker.summary}</p>{/each}{/if}
      {#if missingRequired(runReview).length}<label>Waiver reason<textarea maxlength="4096" bind:value={waiverReason}></textarea></label>{/if}
      {/if}
    </div>
    {#snippet actions()}
      {#if factoryReview}
        <button data-modal-action="cancel" disabled={factoryActionBusy} onclick={() => closeLinkedReview(() => (runReview = null), "run", runReview?.id ?? null)}>Close</button>
        {#if factoryReview.workflow.currentClaim}<button disabled={factoryActionBusy} onclick={() => void releaseFactoryClaim()}>Release claim</button>{/if}
        {#if !factoryReview.workflow.terminal}<button disabled={factoryActionBusy} onclick={() => void cancelFactoryRun()}>Cancel run</button>{/if}
        {#if factoryReview.phase === "independentReview" && !factoryReview.workflow.review}
          <button class="primary" disabled={factoryActionBusy || !factoryReviewWaiverReason.trim()} onclick={() => void waiveFactoryReview()}>Waive independent review</button>
        {/if}
        {#if factoryReview.humanAction?.kind === "plan"}
          <button disabled={factoryActionBusy} onclick={() => void decideFactoryPlan("reject")}>Reject plan</button>
          <button class="primary" disabled={factoryActionBusy} onclick={() => void decideFactoryPlan("approve")}>Approve plan</button>
        {:else if factoryReview.humanAction?.kind === "final"}
          <button disabled={factoryActionBusy} onclick={() => void decideFactoryFinal("rejected")}>Reject result</button>
          <button disabled={factoryActionBusy} onclick={() => void decideFactoryFinal("rework")}>Request rework</button>
          <button class="primary" disabled={factoryActionBusy
            || (factoryMissingChecks(runReview).length > 0 && !factoryWaiverReason.trim())
            || (factoryReview.workflow.review?.verdict !== "pass"
              && !factoryReview.workflow.humanWaivers.some((waiver) => waiver.kind === "independentReview"))} onclick={() => void decideFactoryFinal("accepted")}>Accept result</button>
        {/if}
      {:else}
      <button onclick={() => finishRun("cancelled")}>Cancel run</button>
      <button onclick={() => finishRun("rejected")}>Reject</button>
      <button onclick={() => finishRun("rework")}>Request rework</button>
      {#if missingRequired(runReview).length}
        <button class="primary" disabled={!waiverReason.trim()} onclick={() => finishRun("accepted", true)}>Accept with waiver</button>
      {:else}<button class="primary" onclick={() => finishRun("accepted")}>Accept</button>{/if}
      {/if}
    {/snippet}
  </Modal>
{/if}

{#if builder}
  <Modal open title={creationReview ? `Review ${creationReview.kind} request` : builder.source === "custom" ? "Edit Expert" : "Clone Expert"} size="wide" onClose={() => closeLinkedReview(() => { builder = null; creationReview = null; }, "change", creationReview?.id ?? null)}>
    <form class="builder" onsubmit={(event) => { event.preventDefault(); void saveBuilder(); }}>
      <label>Name<input required maxlength="120" bind:value={builder.name} /></label>
      <label>Summary<textarea required maxlength="1000" bind:value={builder.summary}></textarea></label>
      <label>Category<input required maxlength="80" bind:value={builder.category} /></label>
      <label>Tags<input maxlength="500" value={builder.tags.join(", ")} oninput={(event) => (builder!.tags = event.currentTarget.value.split(",").map((item) => item.trim()).filter(Boolean))} /></label>
      <label>Lead agent<select bind:value={builder.leadAgent}>{#each corpus.agents as agent}<option value={agent.slug}>{agent.name}</option>{/each}</select></label>
      <label>Supporting agents<select multiple size="7" value={builder.supportingAgents} onchange={(event) => (builder!.supportingAgents = [...event.currentTarget.selectedOptions].map((item) => item.value))}>{#each corpus.agents as agent}<option value={agent.slug}>{agent.name}</option>{/each}</select></label>
      <label>Required skills<input maxlength="1000" value={builder.requiredSkills.join(", ")} oninput={(event) => (builder!.requiredSkills = event.currentTarget.value.split(",").map((item) => item.trim()).filter(Boolean))} /></label>
      <label>Optional skills<input maxlength="1000" value={builder.optionalSkills.join(", ")} oninput={(event) => (builder!.optionalSkills = event.currentTarget.value.split(",").map((item) => item.trim()).filter(Boolean))} /></label>
      <label>Runbook slug<input maxlength="160" value={builder.runbook ?? ""} oninput={(event) => (builder!.runbook = event.currentTarget.value.trim() || null)} /></label>
      <label>Preferred client<select bind:value={builder.preferredClient}><option value={null}>Ask during activation</option><option value="claudeCode">Claude Code</option><option value="codex">Codex</option></select></label>
      <label>Starter prompt<textarea required maxlength="4096" rows="6" bind:value={builder.starterPrompt}></textarea></label>
      <label>Required quality checks<input maxlength="500" value={builder.qualityContract.checks.filter((check) => check.required).map((check) => check.name).join(", ")} oninput={(event) => { const optional = builder!.qualityContract.checks.filter((check) => !check.required); builder!.qualityContract = { version: 1, checks: [...event.currentTarget.value.split(",").map((name) => name.trim()).filter(Boolean).map((name) => ({ name, kind: name, required: true, evidenceMode: "clientReported" as const })), ...optional] }; }} /></label>
      <label>Optional quality checks<input maxlength="500" value={builder.qualityContract.checks.filter((check) => !check.required).map((check) => check.name).join(", ")} oninput={(event) => { const required = builder!.qualityContract.checks.filter((check) => check.required); builder!.qualityContract = { version: 1, checks: [...required, ...event.currentTarget.value.split(",").map((name) => name.trim()).filter(Boolean).map((name) => ({ name, kind: name, required: false, evidenceMode: "clientReported" as const }))] }; }} /></label>
    </form>
    {#snippet actions()}<button onclick={() => closeLinkedReview(() => { builder = null; creationReview = null; }, "change", creationReview?.id ?? null)}>Cancel</button><button class="primary" disabled={pendingRequiredSkill} onclick={saveBuilder}>Save</button>{/snippet}
  </Modal>
{/if}

<style>
  .experts { height: 100%; min-height: 0; display: flex; flex-direction: column; }
  .head,.toolbar,.filters,footer,.title,.tabs,.chips,.grid { display: flex; align-items: center; gap: var(--space-2); }
  .head { justify-content: space-between; padding: var(--space-3) var(--space-4); border-bottom: 1px solid var(--color-border); }
  h1 { font-size: var(--text-h2); } h2 { font-size: var(--text-h2); } h3 { font-size: var(--text-body); margin-bottom: 6px; }
  p,.row-meta { color: var(--color-text-secondary); font-size: var(--text-body-sm); }
  button,select,input,textarea { border: 1px solid var(--color-border); border-radius: var(--radius-md); background: var(--color-surface-raised); color: var(--color-text-primary); padding: 7px 10px; }
  button { cursor: pointer; display: inline-flex; align-items: center; gap: 5px; } button:disabled { opacity: .5; cursor: default; }
  .primary,.tabs .active,.filters .active { background: var(--color-brand); color: var(--color-text-inverse); border-color: var(--color-brand); }
  .runbooks { flex: 1; min-height: 0; } .toolbar { padding: var(--space-3) var(--space-4); } .toolbar .search { flex: 1; display:flex;align-items:center;gap:6px; }
  .drafts { flex:1; min-height:0; overflow:auto; padding:var(--space-4); display:grid; gap:var(--space-3); align-content:start; }
  .draft-card { border:1px solid var(--color-border); border-radius:var(--radius-lg); padding:var(--space-4); background:var(--color-surface-raised); }
  .draft-card section { margin-top:var(--space-3); }.draft-card footer { justify-content:flex-end; margin-top:var(--space-3); }
  .inbox { display:grid; grid-template-columns:auto 1fr auto auto; align-items:center; gap:var(--space-2); padding:var(--space-2) var(--space-4); background:var(--color-brand-subtle); border-top:1px solid var(--color-border); }
  .search input { flex: 1; border: 0; padding: 0; background: transparent; } .filters { padding: 0 var(--space-4) var(--space-3); }
  .workspace { flex: 1; min-height: 0; display:grid; grid-template-columns:minmax(280px,34%) 1fr; border-top:1px solid var(--color-border); }
  .list { overflow:auto; border-right:1px solid var(--color-border); padding:var(--space-2); }
  .list>button { width:100%; display:flex; flex-direction:column; align-items:flex-start; margin-bottom:4px; text-align:left; }
  .list>button.selected { border-color:var(--color-brand); background:var(--color-brand-subtle); } .row-title { font-weight:var(--fw-semibold); }
  .status { font-size:var(--text-caption); color:var(--color-warning-strong); }.status.ready,.ok { color:var(--color-success-on-subtle); }
  .detail { overflow:auto; padding:var(--space-4); } .detail section { margin-top:var(--space-4); } .title { justify-content:space-between; }.source { text-transform:capitalize; }
  .metrics { display:flex; gap:var(--space-2); flex-wrap:wrap; align-items:center; margin-top:var(--space-2); }.performance ul { margin:var(--space-2) 0 0; padding-left:var(--space-4); }.performance li { margin-top:4px; font-size:var(--text-body-sm); }
  .link { display:block; border:0; background:transparent; color:var(--color-brand); padding-left:0; }.chips { flex-wrap:wrap; }.grid { align-items:flex-start; justify-content:space-between; }
  label { display:flex; flex-direction:column; gap:5px; font-size:var(--text-body-sm); }.detail footer { justify-content:flex-end; margin-top:var(--space-4); }
  pre { white-space:pre-wrap; padding:var(--space-3); background:var(--color-surface-sunken); border-radius:var(--radius-md); font-size:var(--text-body-sm); }
  .warning { padding:var(--space-3); margin-top:var(--space-3); border:1px solid var(--color-warning); border-radius:var(--radius-md); background:var(--color-warning-subtle); color:var(--color-warning-strong); }
  .review { max-height:60vh; overflow:auto; }.builder { display:grid; gap:var(--space-3); max-height:60vh; overflow:auto; }.builder input,.builder textarea,.builder select { width:100%; }.field-help { color:var(--color-text-secondary); }
  .empty { padding:var(--space-4); }
  @media (max-width: 600px) {
    .head,.toolbar { align-items:stretch; flex-wrap:wrap; }
    .tabs,.filters { max-width:100%; overflow-x:auto; }
    .toolbar .search { flex-basis:100%; }
    .workspace { grid-template-columns:minmax(0,1fr); overflow:auto; }
    .list { max-height:180px; border-right:0; border-bottom:1px solid var(--color-border); }
    .detail { overflow:visible; }
    .title,.grid { align-items:flex-start; flex-direction:column; }
    footer { flex-wrap:wrap; }
  }
</style>
