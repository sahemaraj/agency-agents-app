/**
 * Projects store — the registered project directories used for project-scoped
 * installs. Backs the Projects nav section (the 4th pillar).
 *
 * Registered roots are persisted by Tauri. The backend list unions those roots
 * with project paths derived from the install ledger.
 *
 * Singleton: import `projects` everywhere.
 */

import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";

import { i18n } from "$lib/stores/i18n.svelte";
import { install } from "$lib/stores/install.svelte";
import { activity, safeActivityDetail } from "$lib/stores/activity.svelte";
import {
  projectBaselineImportPack,
  projectBaselineSaveTeam,
  projectReadinessGet,
  projectRecommendationDismiss,
  projectRecommendationOpen,
  projectRecommendationsList,
  projectSubscriptionSet,
} from "$lib/api";
import type {
  ProjectInfo,
  ProjectInstructionApplyResponse,
  ProjectInstructionOperation,
  ProjectInstructionPlan,
  ProjectInstructionTarget,
  ProjectReadinessBaseline,
  ProjectReadinessReport,
  ProjectRecommendation,
  WorkspacePack,
} from "$lib/types";

const STORAGE_KEY = "agency-agents:projects:v1";

class ProjectsStore {
  list: ProjectInfo[] = $state([]);
  private migrated = false;

  private async migrateLocalStorage(): Promise<void> {
    if (this.migrated || typeof window === "undefined") return;
    this.migrated = true;
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return;
    try {
      const parsed = JSON.parse(raw) as unknown;
      if (!Array.isArray(parsed)) return;
      const paths = parsed.filter((value): value is string => typeof value === "string");
      for (const path of paths) await invoke<string>("project_register", { path });
      localStorage.removeItem(STORAGE_KEY);
    } catch {
      // Keep the complete legacy payload so a later launch can retry safely.
    }
  }

  /** Ensure the registered set + ledger are loaded (panel calls on mount). */
  async refresh(): Promise<void> {
    await this.migrateLocalStorage();
    await install.reconcile();
    this.list = await invoke<ProjectInfo[]>("projects_list");
  }

  async register(path: string): Promise<string> {
    const canonical = await invoke<string>("project_register", { path });
    this.list = await invoke<ProjectInfo[]>("projects_list");
    return canonical;
  }

  /** Forget a project root. Any agents installed into it remain on disk (and
      keep the project visible via the ledger) until they're removed. */
  async unregister(path: string): Promise<void> {
    await invoke("project_unregister", { path });
    this.list = await invoke<ProjectInfo[]>("projects_list");
  }

  inspectInstructions(projectPath: string): Promise<ProjectInstructionTarget[]> {
    return invoke<ProjectInstructionTarget[]>("project_instructions_inspect", { projectPath });
  }

  saveTeamBaseline(projectPath: string, label: string, slugs: string[]): Promise<ProjectReadinessBaseline> {
    return projectBaselineSaveTeam(projectPath, label, slugs);
  }

  importPackBaseline(projectPath: string, pack: WorkspacePack): Promise<ProjectReadinessBaseline> {
    return projectBaselineImportPack(projectPath, pack);
  }

  readiness(projectPath: string): Promise<ProjectReadinessReport> {
    return projectReadinessGet(projectPath);
  }

  subscribe(projectPath: string, enabled: boolean): Promise<boolean> {
    return projectSubscriptionSet(projectPath, enabled);
  }

  recommendations(projectPath: string): Promise<ProjectRecommendation[]> {
    return projectRecommendationsList(projectPath);
  }

  dismissRecommendation(projectPath: string, recommendationId: string): Promise<void> {
    return projectRecommendationDismiss(projectPath, recommendationId);
  }

  openRecommendation(projectPath: string, recommendationId: string): Promise<ProjectRecommendation> {
    return projectRecommendationOpen(projectPath, recommendationId);
  }

  planInstruction(
    projectPath: string,
    target: string,
    operation: ProjectInstructionOperation,
    snippetId: string,
    content: string,
  ): Promise<ProjectInstructionPlan> {
    return invoke<ProjectInstructionPlan>("project_instruction_plan", {
      projectPath, target, operation, snippetId, content,
    });
  }

  async applyInstruction(
    plan: ProjectInstructionPlan,
    content: string,
  ): Promise<ProjectInstructionApplyResponse> {
    try {
      const response = await invoke<ProjectInstructionApplyResponse>("project_instruction_apply", {
        projectPath: plan.projectPath,
        target: plan.target,
        operation: plan.operation,
        snippetId: plan.snippetId,
        content,
        revision: plan.revision,
        confirmed: true,
      });
      if (response.result?.message) response.result.message = safeActivityDetail(response.result.message);
      if (response.result) {
        const message = response.result.message ? ` · ${safeActivityDetail(response.result.message)}` : "";
        activity.log({
          action: "update",
          subject: "agentLibrary",
          subjectName: response.plan.label,
          scope: "project",
          projectPath: response.plan.projectPath,
          outcome: response.result.outcome === "succeeded" ? "ok" : "error",
          detail: `${response.result.destination}${message}`,
        });
      }
      return response;
    } catch (error) {
      activity.log({
        action: "update",
        subject: "agentLibrary",
        subjectName: plan.label,
        scope: "project",
        projectPath: plan.projectPath,
        outcome: "error",
        detail: `${plan.destination} · ${safeActivityDetail(error)}`,
      });
      throw error;
    }
  }

  /** Native folder picker → registers and returns the chosen path (or null). */
  async addViaPicker(): Promise<string | null> {
    const picked = await openDialog({ directory: true, title: i18n.t("projects.chooseFolderTitle") });
    const path = typeof picked === "string" ? picked : Array.isArray(picked) ? (picked[0] ?? null) : null;
    return path ? this.register(path) : null;
  }
}

export const projects = new ProjectsStore();
