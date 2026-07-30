import {
  skillPackageDestinations,
  skillInstall,
  skillInstallsReconcile,
  skillBackupsList,
  skillDisable,
  skillEnable,
  skillUninstall,
  skillUpdate,
  skillSourceAddGithub,
  skillSourceAddLocal,
  skillSourceRefresh,
  skillSourceRemove,
  skillSourcesInspect,
  skillDraftPublish,
  skillDraftReject,
  skillDraftsList,
} from "$lib/api";
import {
  appErrorMessage,
  isAppError,
  type InstalledSkill,
  type SkillDraft,
  type SkillDestinationPresence,
  type SkillPackageResult,
  type SkillSource,
  type SkillSourceResult,
} from "$lib/types";
import { activity } from "$lib/stores/activity.svelte";

export interface AddSourceResult {
  registrationSucceeded: boolean;
  initialRefreshSucceeded: boolean;
}

function errorMessage(error: unknown): string {
  return isAppError(error) ? appErrorMessage(error) : String(error);
}

function sourceLabel(source: SkillSource | undefined, fallback: string): string {
  if (!source) return fallback;
  return source.kind.kind === "local" ? source.kind.root : source.kind.repository;
}

class SkillSourcesStore {
  sources: SkillSource[] = $state([]);
  results: Record<string, SkillSourceResult> = $state({});
  destinations: Record<string, SkillDestinationPresence[]> = $state({});
  installed: InstalledSkill[] = $state([]);
  drafts: SkillDraft[] = $state([]);
  backups: string[] = $state([]);
  installErrors: Record<string, string> = $state({});
  destinationErrors: Record<string, string> = $state({});
  refreshErrors: Record<string, string> = $state({});
  removeErrors: Record<string, string> = $state({});
  addError: string | null = $state(null);
  loading = $state(false);
  adding = $state(false);
  refreshing: Record<string, boolean> = $state({});
  removing: Record<string, boolean> = $state({});
  loadingDestinations: Record<string, boolean> = $state({});
  installing: Record<string, boolean> = $state({});

  async load(): Promise<void> {
    if (this.loading) return;
    this.loading = true;
    try {
      const results = await skillSourcesInspect();
      this.drafts = await skillDraftsList();
      this.sources = results.map((result) => result.source);
      this.results = Object.fromEntries(results.map((result) => [result.source.id, result]));
    } catch (error) {
      this.addError = errorMessage(error);
    } finally {
      this.loading = false;
    }
  }

  async publishDraft(draft: SkillDraft): Promise<boolean> {
    try {
      await skillDraftPublish(draft.id);
      await this.load();
      activity.log({
        action: "sourceAdd",
        subject: "skillSource",
        subjectName: draft.validation.name ?? draft.id,
        outcome: "ok",
        detail: "Published skill draft",
      });
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      activity.log({
        action: "sourceAdd",
        subject: "skillSource",
        subjectName: draft.validation.name ?? draft.id,
        outcome: "error",
        detail: errorMessage(error),
      });
      return false;
    }
  }

  async rejectDraft(draft: SkillDraft): Promise<boolean> {
    try {
      await skillDraftReject(draft.id);
      this.drafts = this.drafts.map((entry) =>
        entry.id === draft.id ? { ...entry, state: "rejected" } : entry
      );
      activity.log({
        action: "sourceRemove",
        subject: "skillSource",
        subjectName: draft.validation.name ?? draft.id,
        outcome: "ok",
        detail: "Rejected skill draft",
      });
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      activity.log({
        action: "sourceRemove",
        subject: "skillSource",
        subjectName: draft.validation.name ?? draft.id,
        outcome: "error",
        detail: errorMessage(error),
      });
      return false;
    }
  }

  packageKey(pkg: SkillPackageResult): string {
    return `${pkg.sourceId}\0${pkg.relativePath}`;
  }

  installKey(pkg: SkillPackageResult, runtime: string, projectPath: string | null): string {
    return `${this.packageKey(pkg)}\0${runtime}\0${projectPath ?? ""}`;
  }

  async reconcileInstalls(projectPaths: string[]): Promise<void> {
    try {
      this.installed = await skillInstallsReconcile(projectPaths);
      this.backups = await skillBackupsList();
    } catch (error) {
      this.addError = errorMessage(error);
    }
  }

  async lifecycle(
    action: "update" | "disable" | "enable" | "uninstall",
    installed: InstalledSkill,
    projectPaths: string[],
  ): Promise<boolean> {
    const key = `${installed.sourceId}\0${installed.relativePath}\0${installed.runtime}\0${installed.projectPath ?? ""}`;
    if (this.installing[key]) return false;
    this.installing = { ...this.installing, [key]: true };
    try {
      if (action === "update") await skillUpdate(installed);
      else if (action === "disable") await skillDisable(installed);
      else if (action === "enable") await skillEnable(installed);
      else await skillUninstall(installed);
      const { [key]: _cleared, ...errors } = this.installErrors;
      this.installErrors = errors;
      await this.reconcileInstalls(projectPaths);
      activity.log({
        action,
        subject: "skill",
        subjectName: installed.name,
        tool: installed.runtime,
        scope: installed.scope,
        projectPath: installed.projectPath ?? undefined,
        outcome: "ok",
      });
      return true;
    } catch (error) {
      this.installErrors = { ...this.installErrors, [key]: errorMessage(error) };
      activity.log({
        action,
        subject: "skill",
        subjectName: installed.name,
        tool: installed.runtime,
        scope: installed.scope,
        projectPath: installed.projectPath ?? undefined,
        outcome: "error",
        detail: errorMessage(error),
      });
      return false;
    } finally {
      const { [key]: _cleared, ...installing } = this.installing;
      this.installing = installing;
    }
  }

  async installPackage(
    pkg: SkillPackageResult,
    runtime: "claudeCode" | "codex",
    projectPath: string | null,
    projectPaths: string[],
  ): Promise<boolean> {
    const key = this.installKey(pkg, runtime, projectPath);
    if (!pkg.installable || this.installing[key]) return false;
    this.installing = { ...this.installing, [key]: true };
    try {
      await skillInstall(pkg.sourceId, pkg.relativePath, runtime, projectPath);
      const { [key]: _cleared, ...errors } = this.installErrors;
      this.installErrors = errors;
      await this.reconcileInstalls(projectPaths);
      activity.log({
        action: "install",
        subject: "skill",
        subjectName: pkg.name ?? pkg.relativePath,
        tool: runtime,
        scope: projectPath ? "project" : "user",
        projectPath: projectPath ?? undefined,
        outcome: "ok",
      });
      return true;
    } catch (error) {
      this.installErrors = { ...this.installErrors, [key]: errorMessage(error) };
      activity.log({
        action: "install",
        subject: "skill",
        subjectName: pkg.name ?? pkg.relativePath,
        tool: runtime,
        scope: projectPath ? "project" : "user",
        projectPath: projectPath ?? undefined,
        outcome: "error",
        detail: errorMessage(error),
      });
      return false;
    } finally {
      const { [key]: _cleared, ...installing } = this.installing;
      this.installing = installing;
    }
  }

  async loadDestinations(pkg: SkillPackageResult, projectPaths: string[]): Promise<void> {
    const key = this.packageKey(pkg);
    if (!pkg.installable || this.loadingDestinations[key]) return;
    this.loadingDestinations = { ...this.loadingDestinations, [key]: true };
    try {
      const destinations = await skillPackageDestinations(
        pkg.sourceId,
        pkg.relativePath,
        projectPaths,
      );
      this.destinations = { ...this.destinations, [key]: destinations };
      const { [key]: _cleared, ...remainingErrors } = this.destinationErrors;
      this.destinationErrors = remainingErrors;
    } catch (error) {
      this.destinationErrors = {
        ...this.destinationErrors,
        [key]: errorMessage(error),
      };
    } finally {
      const { [key]: _cleared, ...remainingBusy } = this.loadingDestinations;
      this.loadingDestinations = remainingBusy;
    }
  }

  async addLocal(root: string): Promise<AddSourceResult> {
    if (this.loading || this.adding) {
      return { registrationSucceeded: false, initialRefreshSucceeded: false };
    }

    this.adding = true;
    this.addError = null;
    try {
      const source = await skillSourceAddLocal(root);
      this.mergeSource(source);
      this.addError = null;
      const initialRefreshSucceeded = await this.refresh(source.id);
      activity.log({ action: "sourceAdd", subject: "skillSource", subjectName: root, outcome: "ok" });
      return { registrationSucceeded: true, initialRefreshSucceeded };
    } catch (error) {
      this.addError = errorMessage(error);
      activity.log({ action: "sourceAdd", subject: "skillSource", subjectName: root, outcome: "error", detail: errorMessage(error) });
      return { registrationSucceeded: false, initialRefreshSucceeded: false };
    } finally {
      this.adding = false;
    }
  }

  async addGithub(
    repository: string,
    gitRef: string,
    subdirectory: string,
  ): Promise<AddSourceResult> {
    if (this.loading || this.adding) {
      return { registrationSucceeded: false, initialRefreshSucceeded: false };
    }

    const trimmedRepository = repository.trim();
    if (!trimmedRepository) {
      this.addError = "GitHub repository URL is required.";
      return { registrationSucceeded: false, initialRefreshSucceeded: false };
    }

    this.adding = true;
    this.addError = null;
    try {
      const source = await skillSourceAddGithub(
        trimmedRepository,
        gitRef.trim() || null,
        subdirectory.trim() || null,
      );
      this.mergeSource(source);
      const initialRefreshSucceeded = await this.refresh(source.id);
      activity.log({ action: "sourceAdd", subject: "skillSource", subjectName: trimmedRepository, outcome: "ok" });
      return { registrationSucceeded: true, initialRefreshSucceeded };
    } catch (error) {
      this.addError = errorMessage(error);
      activity.log({ action: "sourceAdd", subject: "skillSource", subjectName: trimmedRepository, outcome: "error", detail: errorMessage(error) });
      return { registrationSucceeded: false, initialRefreshSucceeded: false };
    } finally {
      this.adding = false;
    }
  }

  async refresh(sourceId: string): Promise<boolean> {
    if (this.refreshing[sourceId]) return false;
    const label = sourceLabel(this.sources.find((source) => source.id === sourceId), sourceId);
    this.refreshing = { ...this.refreshing, [sourceId]: true };
    try {
      const result = await skillSourceRefresh(sourceId);
      this.results = { ...this.results, [sourceId]: result };
      this.mergeSource(result.source);
      const { [sourceId]: _cleared, ...remainingErrors } = this.refreshErrors;
      this.refreshErrors = remainingErrors;
      activity.log({ action: "sourceRefresh", subject: "skillSource", subjectName: label, outcome: "ok" });
      return true;
    } catch (error) {
      this.refreshErrors = {
        ...this.refreshErrors,
        [sourceId]: errorMessage(error),
      };
      activity.log({ action: "sourceRefresh", subject: "skillSource", subjectName: label, outcome: "error", detail: errorMessage(error) });
      return false;
    } finally {
      const { [sourceId]: _cleared, ...remainingBusy } = this.refreshing;
      this.refreshing = remainingBusy;
    }
  }

  isRefreshing(sourceId: string): boolean {
    return this.refreshing[sourceId] === true;
  }

  async remove(sourceId: string): Promise<boolean> {
    if (this.removing[sourceId]) return false;
    const label = sourceLabel(this.sources.find((source) => source.id === sourceId), sourceId);
    this.removing = { ...this.removing, [sourceId]: true };
    try {
      await skillSourceRemove(sourceId);
      const removed = this.sources.find((source) => source.id === sourceId);
      this.sources = this.sources.filter((source) => source.id !== sourceId);
      const { [sourceId]: _result, ...results } = this.results;
      const { [sourceId]: _refreshError, ...refreshErrors } = this.refreshErrors;
      const { [sourceId]: _removeError, ...removeErrors } = this.removeErrors;
      this.results = results;
      this.refreshErrors = refreshErrors;
      this.removeErrors = removeErrors;
      activity.log({ action: "sourceRemove", subject: "skillSource", subjectName: sourceLabel(removed, label), outcome: "ok" });
      return true;
    } catch (error) {
      this.removeErrors = { ...this.removeErrors, [sourceId]: errorMessage(error) };
      activity.log({ action: "sourceRemove", subject: "skillSource", subjectName: label, outcome: "error", detail: errorMessage(error) });
      return false;
    } finally {
      const { [sourceId]: _removed, ...removing } = this.removing;
      this.removing = removing;
    }
  }

  isRemoving(sourceId: string): boolean {
    return this.removing[sourceId] === true;
  }

  private mergeSource(source: SkillSource): void {
    const index = this.sources.findIndex((candidate) => candidate.id === source.id);
    this.sources =
      index === -1
        ? [...this.sources, source]
        : this.sources.map((candidate) => candidate.id === source.id ? source : candidate);
  }
}

export const skillSources = new SkillSourcesStore();
