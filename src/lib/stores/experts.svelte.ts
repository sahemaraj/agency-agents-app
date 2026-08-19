import { invoke } from "@tauri-apps/api/core";
import {
  expertActivate,
  expertPlanActivation,
  expertRunFactoryCancel,
  expertRunFactoryFinalDecide,
  expertRunFactoryPlanDecide,
  expertRunFactoryReleaseClaim,
  expertRunFactoryWaiveReview,
  expertRunFactoryResolveBlocker,
  expertRunsList,
} from "$lib/api";
import {
  appErrorMessage,
  isAppError,
  type ExpertActivationPlan,
  type ExpertActivationRecord,
  type ExpertActivationRequest,
  type ExpertClient,
  type ExpertCreationRequest,
  type ExpertDefinition,
  type ExpertProposalInput,
  type ExpertResolved,
  type ExpertRun,
  type FactoryWorkOrderInput,
  type FactoryEvidence,
  type FactoryPhase,
  type FactoryWorkflow,
  type FactoryFinalDecisionInput,
} from "$lib/types";

const FACTORY_ATTEMPT_LIMIT = 3;
const FACTORY_PHASE_LABEL: Record<FactoryPhase, string> = {
  preflight: "Preflight",
  planning: "Planning",
  awaitingPlanApproval: "Awaiting plan approval",
  build: "Build",
  validation: "Validation",
  independentReview: "Independent review",
  delivery: "Delivery",
  awaitingFinalApproval: "Awaiting final approval",
  completed: "Completed",
};

export interface FactoryHumanAction {
  kind: "plan" | "final";
  expectedRevision: number;
  contentRevision: string;
}

export interface FactoryRunProjection {
  workflow: FactoryWorkflow;
  phase: FactoryPhase;
  phaseLabel: string;
  attempt: number;
  maxAttempts: number;
  blocker: string | null;
  claimant: string | null;
  headCommit: string | null;
  humanAction: FactoryHumanAction | null;
  latestEvidence: FactoryEvidence[];
}

function currentFactoryEvidence(workflow: FactoryWorkflow): FactoryEvidence[] {
  const attempt = workflow.attempts.at(-1);
  const attemptNumber = attempt?.number ?? 0;
  const headCommit = attempt?.headCommit ?? workflow.delivery?.headCommit ?? workflow.validation?.headCommit ?? null;
  const planRevision = workflow.planApproval?.planRevision ?? null;
  const baseCommit = workflow.planApproval?.baseCommit ?? null;
  const latest = new Map<string, FactoryEvidence>();
  for (const evidence of workflow.evidence) {
    if (evidence.workContractRevision !== workflow.workContractRevision
      || evidence.attempt !== attemptNumber
      || (planRevision && evidence.approvedPlanRevision !== planRevision)
      || (baseCommit && evidence.baseCommit !== baseCommit)
      || (headCommit && evidence.headCommit !== headCommit)
      || !(["validation", "independentReview", "delivery"] as FactoryPhase[]).includes(evidence.phase)) continue;
    latest.set(evidence.checkName, evidence);
  }
  const contractOrder = new Map(workflow.workContract.qualityContract.checks.map((check, index) => [check.name, index]));
  return [...latest.values()].sort((left, right) =>
    (contractOrder.get(left.checkName) ?? Number.MAX_SAFE_INTEGER)
    - (contractOrder.get(right.checkName) ?? Number.MAX_SAFE_INTEGER));
}

export function projectFactoryRun(run: ExpertRun): FactoryRunProjection | null {
  const workflow = run.factory;
  if (!workflow) return null;
  const attempt = workflow.attempts.at(-1);
  const blocker = workflow.blockers.filter((item) => !item.resolvedAt).at(-1) ?? null;
  const humanAction: FactoryHumanAction | null = workflow.phase === "awaitingPlanApproval" && workflow.plan
    ? { kind: "plan", expectedRevision: workflow.revision, contentRevision: workflow.plan.revision }
    : workflow.phase === "awaitingFinalApproval"
      ? {
          kind: "final",
          expectedRevision: workflow.revision,
          contentRevision: workflow.planApproval?.planRevision ?? workflow.workContractRevision,
        }
      : null;
  return {
    workflow,
    phase: workflow.phase,
    phaseLabel: FACTORY_PHASE_LABEL[workflow.phase],
    attempt: attempt?.number ?? 0,
    maxAttempts: FACTORY_ATTEMPT_LIMIT,
    blocker: blocker?.summary ?? null,
    claimant: workflow.currentClaim?.workerIdentity ?? null,
    headCommit: attempt?.headCommit ?? workflow.delivery?.headCommit ?? workflow.validation?.headCommit ?? null,
    humanAction,
    latestEvidence: currentFactoryEvidence(workflow),
  };
}

export interface ExpertPerformanceSummary {
  comparableRuns: number;
  eligible: boolean;
  accepted: number;
  rework: number;
  rejected: number;
  acceptanceRate: number | null;
  waiverRate: number | null;
  checks: Array<{ name: string; issueRuns: number; waiverRuns: number }>;
  suggestions: string[];
}

function contractSignature(contract: ExpertDefinition["qualityContract"]): string {
  return JSON.stringify({
    version: contract.version,
    checks: contract.checks.map(({ name, kind, required, evidenceMode }) => ({ name, kind, required, evidenceMode })),
  });
}

function performanceEvidence(run: ExpertRun, checkName: string): FactoryEvidence | ExpertRun["evidence"][number] | undefined {
  return run.factory
    ? currentFactoryEvidence(run.factory).find((item) => item.checkName === checkName)
    : run.evidence.filter((item) => item.checkName === checkName).at(-1);
}

function performanceCheckWaived(run: ExpertRun, checkName: string): boolean {
  return run.factory
    ? run.factory.humanWaivers.some((waiver) => waiver.kind === "qualityCheck" && waiver.checkName === checkName)
    : run.waivers.some((waiver) => waiver.checkName === checkName);
}

function performanceRunWaived(run: ExpertRun): boolean {
  return run.factory ? run.factory.humanWaivers.length > 0 : run.waivers.length > 0;
}

export function summarizeExpertPerformance(
  expert: Pick<ExpertDefinition, "id" | "version" | "qualityContract">,
  runs: ExpertRun[],
): ExpertPerformanceSummary {
  const signature = contractSignature(expert.qualityContract);
  const comparable = runs.filter((run) =>
    run.expertId === expert.id
    && run.expertVersion === expert.version
    && ["accepted", "rework", "rejected"].includes(run.state)
    && contractSignature(run.contract) === signature
  );
  const comparableRuns = comparable.length;
  const eligible = comparableRuns >= 5;
  const accepted = comparable.filter((run) => run.state === "accepted").length;
  const rework = comparable.filter((run) => run.state === "rework").length;
  const rejected = comparable.filter((run) => run.state === "rejected").length;
  const checks = expert.qualityContract.checks.map((check) => ({
    name: check.name,
    issueRuns: comparable.filter((run) => performanceEvidence(run, check.name)?.result !== "pass").length,
    waiverRuns: comparable.filter((run) => performanceCheckWaived(run, check.name)).length,
  }));
  if (!eligible) return {
    comparableRuns, eligible, accepted, rework, rejected,
    acceptanceRate: null, waiverRate: null, checks, suggestions: [],
  };

  const recurring = (count: number) => count >= 2 && count / comparableRuns >= 0.4;
  const suggestions: string[] = [];
  if (recurring(rework + rejected)) {
    suggestions.push(`${rework + rejected} of ${comparableRuns} runs ended in rework or rejection; review the Expert instructions and roster for recurring gaps.`);
  }
  for (const check of checks) {
    if (recurring(check.issueRuns)) {
      suggestions.push(`${check.name} had missing, skipped, or failed evidence in ${check.issueRuns} of ${comparableRuns} runs; review its instructions or tooling.`);
    }
    if (recurring(check.waiverRuns)) {
      suggestions.push(`${check.name} was waived in ${check.waiverRuns} of ${comparableRuns} runs; clarify the check or improve its evidence path.`);
    }
  }
  if (suggestions.length === 0) suggestions.push("No recurring improvement signal was detected.");

  return {
    comparableRuns, eligible, accepted, rework, rejected,
    acceptanceRate: Math.round(accepted / comparableRuns * 100),
    waiverRate: Math.round(comparable.filter(performanceRunWaived).length / comparableRuns * 100),
    checks, suggestions,
  };
}

class ExpertsStore {
  list: ExpertResolved[] = $state([]);
  loading = $state(false);
  error: string | null = $state(null);
  selectedId: string | null = $state(null);
  requests: ExpertActivationRequest[] = $state([]);
  creationRequests: ExpertCreationRequest[] = $state([]);
  history: ExpertActivationRecord[] = $state([]);
  runs: ExpertRun[] = $state([]);

  selected = $derived(this.list.find((expert) => expert.id === this.selectedId) ?? this.list[0] ?? null);

  async load(): Promise<void> {
    if (this.loading) return;
    this.loading = true;
    this.error = null;
    try {
      [this.list, this.requests, this.creationRequests, this.history, this.runs] = await Promise.all([
        invoke<ExpertResolved[]>("experts_list"),
        invoke<ExpertActivationRequest[]>("expert_activation_requests"),
        invoke<ExpertCreationRequest[]>("expert_creation_requests"),
        invoke<ExpertActivationRecord[]>("expert_activation_history", { projectPath: null }),
        invoke<ExpertRun[]>("expert_runs_list", { projectPath: null }),
      ]);
      if (!this.selectedId || !this.list.some((expert) => expert.id === this.selectedId)) {
        this.selectedId = this.list[0]?.id ?? null;
      }
    } catch (error) {
      this.error = isAppError(error) ? appErrorMessage(error) : String(error);
      this.list = [];
    } finally {
      this.loading = false;
    }
  }

  async save(expert: ExpertDefinition): Promise<void> {
    await invoke("expert_save", { expert });
    await this.load();
    this.selectedId = expert.id;
  }

  async remove(id: string): Promise<void> {
    await invoke("expert_delete", { id });
    this.selectedId = null;
    await this.load();
  }

  async importFile(path: string): Promise<number> {
    const count = await invoke<number>("expert_import", { path });
    await this.load();
    return count;
  }

  exportFile(path: string): Promise<number> {
    return invoke<number>("expert_export", { path });
  }

  plan(id: string, projectPath: string, client: ExpertClient | null): Promise<ExpertActivationPlan> {
    return expertPlanActivation(id, projectPath, client);
  }

  activate(
    id: string,
    projectPath: string,
    client: ExpertClient | null,
    workOrder: FactoryWorkOrderInput | null = null,
  ): Promise<ExpertActivationRecord> {
    return expertActivate(id, projectPath, client, workOrder);
  }

  async resolveRequest(requestId: string, approved: boolean): Promise<void> {
    await invoke("expert_activation_request_resolve", { requestId, approved });
    this.requests = await invoke<ExpertActivationRequest[]>("expert_activation_requests");
  }

  async approveCreation(requestId: string, proposal: ExpertProposalInput): Promise<void> {
    await invoke("expert_creation_request_approve", { requestId, proposal });
    await this.load();
  }

  async rejectCreation(requestId: string): Promise<void> {
    await invoke("expert_creation_request_reject", { requestId });
    this.creationRequests = await invoke<ExpertCreationRequest[]>("expert_creation_requests");
  }

  async reviewRun(id: string, verdict: "accepted" | "rework" | "rejected" | "cancelled", waivers: Array<{ checkName: string; reason: string }> = []): Promise<void> {
    await invoke("expert_run_review", { id, verdict, waivers });
    this.runs = await invoke<ExpertRun[]>("expert_runs_list", { projectPath: null });
  }

  private retainRun(updated: ExpertRun): ExpertRun {
    const index = this.runs.findIndex((run) => run.id === updated.id);
    this.runs = index < 0
      ? [updated, ...this.runs]
      : this.runs.map((run) => run.id === updated.id ? updated : run);
    return updated;
  }

  async refreshRuns(): Promise<ExpertRun[]> {
    this.runs = await expertRunsList();
    return this.runs;
  }

  async factoryPlanDecide(
    id: string,
    expectedRevision: number,
    planRevision: string,
    decision: "approve" | "reject",
  ): Promise<ExpertRun> {
    return this.retainRun(await expertRunFactoryPlanDecide(id, expectedRevision, planRevision, decision));
  }

  async factoryFinalDecide(id: string, input: FactoryFinalDecisionInput): Promise<ExpertRun> {
    return this.retainRun(await expertRunFactoryFinalDecide(id, input));
  }

  async factoryCancel(id: string, expectedRevision: number, safeDetail: string | null = null): Promise<ExpertRun> {
    return this.retainRun(await expertRunFactoryCancel(id, expectedRevision, safeDetail));
  }

  async factoryReleaseClaim(id: string, expectedRevision: number): Promise<ExpertRun> {
    return this.retainRun(await expertRunFactoryReleaseClaim(id, expectedRevision));
  }

  async factoryWaiveReview(id: string, expectedRevision: number, reason: string): Promise<ExpertRun> {
    return this.retainRun(await expertRunFactoryWaiveReview(id, expectedRevision, reason));
  }

  async factoryResolveBlocker(id: string, expectedRevision: number, blockerId: string): Promise<ExpertRun> {
    return this.retainRun(await expertRunFactoryResolveBlocker(id, expectedRevision, blockerId));
  }
}

export const experts = new ExpertsStore();
