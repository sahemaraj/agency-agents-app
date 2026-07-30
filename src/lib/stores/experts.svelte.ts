import { invoke } from "@tauri-apps/api/core";
import type {
  ExpertActivationPlan,
  ExpertActivationRecord,
  ExpertActivationRequest,
  ExpertClient,
  ExpertCreationRequest,
  ExpertDefinition,
  ExpertProposalInput,
  ExpertResolved,
} from "$lib/types";

class ExpertsStore {
  list: ExpertResolved[] = $state([]);
  loading = $state(false);
  error: string | null = $state(null);
  selectedId: string | null = $state(null);
  requests: ExpertActivationRequest[] = $state([]);
  creationRequests: ExpertCreationRequest[] = $state([]);
  history: ExpertActivationRecord[] = $state([]);

  selected = $derived(this.list.find((expert) => expert.id === this.selectedId) ?? this.list[0] ?? null);

  async load(): Promise<void> {
    if (this.loading) return;
    this.loading = true;
    this.error = null;
    try {
      [this.list, this.requests, this.creationRequests, this.history] = await Promise.all([
        invoke<ExpertResolved[]>("experts_list"),
        invoke<ExpertActivationRequest[]>("expert_activation_requests"),
        invoke<ExpertCreationRequest[]>("expert_creation_requests"),
        invoke<ExpertActivationRecord[]>("expert_activation_history", { projectPath: null }),
      ]);
      if (!this.selectedId || !this.list.some((expert) => expert.id === this.selectedId)) {
        this.selectedId = this.list[0]?.id ?? null;
      }
    } catch (error) {
      this.error = String(error);
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
}

export const experts = new ExpertsStore();
