import {
  skillPackageDestinations,
  skillInstall,
  skillInstallWithDependencies,
  skillInstallsReconcile,
  skillBackupsList,
  skillVersionHistory,
  skillVersionRollback,
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
  skillDraftCreate,
  skillDraftEdit,
  skillTextRead,
  skillDraftsList,
  skillFolderAssign,
  skillFolderCreate,
  skillFolderDelete,
  skillFolderMove,
  skillFolderRename,
  skillFoldersImport,
  skillFoldersList,
  skillFavoriteSet,
  skillRecentTouch,
  skillCollectionSave,
  skillCollectionDelete,
  skillSmartFolderSave,
  skillSmartFolderDelete,
  skillProfileSave,
  skillProfileDelete,
  skillLibraryReplace,
  skillLibraryExport,
  skillLibraryImport,
  skillUpdatePolicySet,
  skillPublisherTrustSet,
  skillPreferredSourceSet,
  skillApprovalApprove,
  skillApprovalReject,
  skillTrustGrant,
  skillTrustRevoke,
} from "$lib/api";
import {
  appErrorMessage,
  isAppError,
  type InstalledSkill,
  type SkillDraft,
  type SkillFolderState,
  type SkillCollection,
  type SkillReference,
  type SkillSmartFolder,
  type SkillWorkspaceProfile,
  type SkillUpdatePolicy,
  type SkillVersionSnapshot,
  type SkillPublisherTrust,
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

type ReconcileRequest = { key: string; paths: string[]; generation: number };
let reconcileInstallsInflight: Promise<void> | null = null;
let reconcileInstallsExecuting: ReconcileRequest | null = null;
let reconcileInstallsPending: ReconcileRequest | null = null;
let reconcileInstallsGeneration = 0;

class SkillSourcesStore {
  sources: SkillSource[] = $state([]);
  results: Record<string, SkillSourceResult> = $state({});
  destinations: Record<string, SkillDestinationPresence[]> = $state({});
  installed: InstalledSkill[] = $state([]);
  reconciling = $state(false);
  reconciled = $state(false);
  reconcileError: string | null = $state(null);
  reconcileAttempt = $state(0);
  reconcileTerminal = $state(0);
  drafts: SkillDraft[] = $state([]);
  backups: string[] = $state([]);
  folderState: SkillFolderState = $state({
    folders: [],
    assignments: [],
    favorites: [],
    recent: [],
    collections: [],
    smartFolders: [],
    profiles: [],
    updatePolicies: [],
    publisherTrust: [],
    preferredSources: [],
    usage: [],
    approvals: [],
  });
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
  trusting: Record<string, boolean> = $state({});

  async load(): Promise<void> {
    if (this.loading) return;
    this.loading = true;
    try {
      const results = await skillSourcesInspect();
      this.drafts = await skillDraftsList();
      this.folderState = await skillFoldersList();
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

  async createDraft(
    name: string,
    description: string,
    skillType: import("$lib/types").SkillType,
    group: string[],
    tags: string[],
    body: string,
  ): Promise<boolean> {
    try {
      const draft = await skillDraftCreate(name, description, skillType, group, tags, body);
      this.drafts = [...this.drafts, draft];
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      return false;
    }
  }

  readSkillText(pkg: SkillPackageResult): Promise<string> {
    return skillTextRead(pkg.sourceId, pkg.relativePath);
  }

  async editDraft(pkg: SkillPackageResult, skillMd: string): Promise<boolean> {
    try {
      const draft = await skillDraftEdit(pkg.sourceId, pkg.relativePath, skillMd);
      this.drafts = [...this.drafts, draft];
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      return false;
    }
  }

  packageKey(pkg: SkillPackageResult): string {
    return `${pkg.sourceId}\0${pkg.relativePath}`;
  }

  folderFor(pkg: SkillPackageResult): string | null {
    return this.folderState.assignments.find((assignment) =>
      assignment.sourceId === pkg.sourceId && assignment.relativePath === pkg.relativePath
    )?.folderPath ?? null;
  }

  reference(pkg: SkillPackageResult): SkillReference {
    return { sourceId: pkg.sourceId, relativePath: pkg.relativePath };
  }

  isFavorite(pkg: SkillPackageResult): boolean {
    return this.folderState.favorites.some((skill) =>
      skill.sourceId === pkg.sourceId && skill.relativePath === pkg.relativePath
    );
  }

  updatePolicy(pkg: SkillPackageResult): SkillUpdatePolicy {
    return this.folderState.updatePolicies.find((record) =>
      record.skill.sourceId === pkg.sourceId && record.skill.relativePath === pkg.relativePath
    )?.policy ?? "notify";
  }

  async setUpdatePolicy(pkg: SkillPackageResult, policy: SkillUpdatePolicy): Promise<boolean> {
    try {
      this.folderState = await skillUpdatePolicySet(this.reference(pkg), policy);
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      return false;
    }
  }

  async setPublisherTrust(trust: SkillPublisherTrust): Promise<boolean> {
    try {
      this.folderState = await skillPublisherTrustSet(trust);
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      return false;
    }
  }

  async setPreferredSource(pkg: SkillPackageResult): Promise<boolean> {
    if (!pkg.name) return false;
    try {
      this.folderState = await skillPreferredSourceSet({
        skillName: pkg.name,
        sourceId: pkg.sourceId,
      });
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      return false;
    }
  }

  async setFavorite(pkg: SkillPackageResult, favorite: boolean): Promise<boolean> {
    try {
      this.folderState = await skillFavoriteSet(this.reference(pkg), favorite);
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      return false;
    }
  }

  async touchRecent(pkg: SkillPackageResult): Promise<void> {
    try {
      this.folderState = await skillRecentTouch(this.reference(pkg));
    } catch (error) {
      this.addError = errorMessage(error);
    }
  }

  async saveCollection(collection: SkillCollection): Promise<boolean> {
    try {
      this.folderState = await skillCollectionSave(collection);
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      return false;
    }
  }

  async deleteCollection(name: string): Promise<boolean> {
    try {
      this.folderState = await skillCollectionDelete(name);
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      return false;
    }
  }

  async saveSmartFolder(smartFolder: SkillSmartFolder): Promise<boolean> {
    try {
      this.folderState = await skillSmartFolderSave(smartFolder);
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      return false;
    }
  }

  async deleteSmartFolder(name: string): Promise<boolean> {
    try {
      this.folderState = await skillSmartFolderDelete(name);
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      return false;
    }
  }

  async saveProfile(profile: SkillWorkspaceProfile): Promise<boolean> {
    try {
      this.folderState = await skillProfileSave(profile);
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      return false;
    }
  }

  async deleteProfile(name: string): Promise<boolean> {
    try {
      this.folderState = await skillProfileDelete(name);
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      return false;
    }
  }

  async replaceLibrary(replacement: SkillFolderState): Promise<boolean> {
    try {
      this.folderState = await skillLibraryReplace(replacement);
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      return false;
    }
  }

  async exportLibrary(path: string): Promise<number> {
    return skillLibraryExport(path);
  }

  async importLibrary(path: string): Promise<boolean> {
    try {
      this.folderState = await skillLibraryImport(path);
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      return false;
    }
  }

  async approveRequest(id: string): Promise<boolean> {
    try {
      await skillApprovalApprove(id);
      this.folderState = await skillFoldersList();
      await this.load();
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      return false;
    }
  }

  async rejectRequest(id: string): Promise<boolean> {
    try {
      await skillApprovalReject(id);
      this.folderState = await skillFoldersList();
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      return false;
    }
  }

  async createFolder(path: string): Promise<boolean> {
    try {
      this.folderState = await skillFolderCreate(path);
      this.addError = null;
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      return false;
    }
  }

  async renameFolder(path: string, newName: string): Promise<boolean> {
    try {
      this.folderState = await skillFolderRename(path, newName);
      this.addError = null;
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      return false;
    }
  }

  async moveFolder(path: string, newParent: string | null): Promise<boolean> {
    try {
      this.folderState = await skillFolderMove(path, newParent);
      this.addError = null;
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      return false;
    }
  }

  async deleteFolder(path: string, recursive: boolean): Promise<boolean> {
    try {
      this.folderState = await skillFolderDelete(path, recursive);
      this.addError = null;
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      return false;
    }
  }

  async assignFolder(pkg: SkillPackageResult, folderPath: string | null): Promise<boolean> {
    try {
      this.folderState = await skillFolderAssign(pkg.sourceId, pkg.relativePath, folderPath);
      this.addError = null;
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      return false;
    }
  }

  async importFolders(imported: SkillFolderState): Promise<boolean> {
    try {
      this.folderState = await skillFoldersImport(imported);
      this.addError = null;
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      return false;
    }
  }

  private replacePackage(pkg: SkillPackageResult): void {
    const result = this.results[pkg.sourceId];
    if (!result) return;
    this.results = {
      ...this.results,
      [pkg.sourceId]: {
        ...result,
        packages: result.packages.map((current) =>
          current.relativePath === pkg.relativePath ? pkg : current
        ),
      },
    };
  }

  async grantTrust(pkg: SkillPackageResult): Promise<boolean> {
    const key = this.packageKey(pkg);
    if (this.trusting[key]) return false;
    this.trusting = { ...this.trusting, [key]: true };
    try {
      this.replacePackage(await skillTrustGrant(pkg.sourceId, pkg.relativePath));
      this.addError = null;
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      return false;
    } finally {
      const { [key]: _cleared, ...trusting } = this.trusting;
      this.trusting = trusting;
    }
  }

  async revokeTrust(pkg: SkillPackageResult): Promise<boolean> {
    const key = this.packageKey(pkg);
    if (this.trusting[key]) return false;
    this.trusting = { ...this.trusting, [key]: true };
    try {
      await skillTrustRevoke(pkg.sourceId, pkg.relativePath);
      await this.load();
      this.addError = null;
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      return false;
    } finally {
      const { [key]: _cleared, ...trusting } = this.trusting;
      this.trusting = trusting;
    }
  }

  installKey(pkg: SkillPackageResult, runtime: string, projectPath: string | null): string {
    return `${this.packageKey(pkg)}\0${runtime}\0${projectPath ?? ""}`;
  }

  async reconcileInstalls(projectPaths: string[]): Promise<void> {
    const canonicalPaths = [...new Set(projectPaths)].sort();
    const key = canonicalPaths.join("\0");
    if (reconcileInstallsInflight) {
      if (reconcileInstallsPending?.key === key) return reconcileInstallsInflight;
      if (!reconcileInstallsPending && reconcileInstallsExecuting?.key === key) return reconcileInstallsInflight;
    }
    reconcileInstallsPending = { key, paths: canonicalPaths, generation: ++reconcileInstallsGeneration };
    if (reconcileInstallsInflight) return reconcileInstallsInflight;

    this.reconciling = true;
    reconcileInstallsInflight = (async () => {
      while (reconcileInstallsPending) {
        const request = reconcileInstallsPending;
        reconcileInstallsPending = null;
        reconcileInstallsExecuting = request;
        const attempt = ++this.reconcileAttempt;
        try {
          const installed = await skillInstallsReconcile(request.paths);
          const backups = await skillBackupsList();
          if (request.generation === reconcileInstallsGeneration) {
            this.installed = installed;
            this.backups = backups;
            this.reconciled = true;
            this.reconcileError = null;
          }
        } catch (error) {
          if (request.generation === reconcileInstallsGeneration) this.reconcileError = errorMessage(error);
        } finally {
          if (request.generation === reconcileInstallsGeneration) this.reconcileTerminal = attempt;
        }
      }
      reconcileInstallsExecuting = null;
      this.reconciling = false;
      reconcileInstallsInflight = null;
    })();
    return reconcileInstallsInflight;
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

  history(installed: InstalledSkill): Promise<SkillVersionSnapshot[]> {
    return skillVersionHistory(installed);
  }

  async rollback(installed: InstalledSkill, snapshotPath: string, projectPaths: string[]): Promise<boolean> {
    try {
      await skillVersionRollback(installed, snapshotPath);
      await this.reconcileInstalls(projectPaths);
      return true;
    } catch (error) {
      this.addError = errorMessage(error);
      return false;
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
      await skillInstallWithDependencies(pkg.sourceId, pkg.relativePath, runtime, projectPath);
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
