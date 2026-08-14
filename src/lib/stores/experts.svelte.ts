import { invoke } from "@tauri-apps/api/core";
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
} from "$lib/types";

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
    issueRuns: comparable.filter((run) => run.evidence.filter((item) => item.checkName === check.name).at(-1)?.result !== "pass").length,
    waiverRuns: comparable.filter((run) => run.waivers.some((waiver) => waiver.checkName === check.name)).length,
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
    waiverRate: Math.round(comparable.filter((run) => run.waivers.length > 0).length / comparableRuns * 100),
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
    return invoke("expert_plan_activation", { id, projectPath, client });
  }

  activate(id: string, projectPath: string, client: ExpertClient | null): Promise<ExpertActivationRecord> {
    return invoke("expert_activate", { id, projectPath, client });
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
}

export const experts = new ExpertsStore();
