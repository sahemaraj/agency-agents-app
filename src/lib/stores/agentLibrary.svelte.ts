import {
  agentApprovalApprove,
  agentApprovalReject,
  agentCollectionDelete,
  agentCollectionSave,
  agentDraftCreate,
  agentDraftDuplicate,
  agentDraftEdit,
  agentDraftPublish,
  agentDraftReject,
  agentDraftsList,
  agentFolderCreate,
  agentFolderDelete,
  agentFolderAssign,
  agentFolderMove,
  agentFolderRename,
  agentFavoriteSet,
  agentLibraryExport,
  agentLibraryImport,
  agentLibraryList,
  agentPreferredSourceSet,
  agentProfileDelete,
  agentProfileSave,
  agentPublisherTrustSet,
  agentRecentTouch,
  agentSmartFolderDelete,
  agentSmartFolderSave,
  agentSourceAddGithub,
  agentSourceAddLocal,
  agentSourceRefresh,
  agentSourceRemove,
  agentSourcesInspect,
  agentUpdatePolicySet,
} from "$lib/api";
import { activity, safeActivityDetail, type JournalEntry } from "$lib/stores/activity.svelte";
import { appErrorMessage, isAppError } from "$lib/types";
import type {
  AgentCollection,
  AgentDraft,
  AgentDraftInput,
  AgentLibraryState,
  AgentPackageResult,
  AgentPreferredSource,
  AgentPublisherTrust,
  AgentReference,
  AgentSource,
  AgentSourceResult,
  AgentSmartFolder,
  AgentUpdatePolicy,
  AgentWorkspaceProfile,
} from "$lib/types";

const EMPTY_LIBRARY: AgentLibraryState = {
  folders: [], assignments: [], favorites: [], recent: [], collections: [], smartFolders: [],
  profiles: [], updatePolicies: [], publisherTrust: [], preferredSources: [], usage: [], approvals: [],
};

const message = (error: unknown) => isAppError(error) ? appErrorMessage(error) : String(error);
type AgentJournal = Omit<JournalEntry, "id" | "ts" | "outcome" | "detail"> & { detail?: string };

class AgentLibraryStore {
  results: AgentSourceResult[] = $state([]);
  drafts: AgentDraft[] = $state([]);
  library: AgentLibraryState = $state(EMPTY_LIBRARY);
  selectedReference: AgentReference | null = $state(null);
  loading = $state(false);
  busy = $state(false);
  error: string | null = $state(null);
  private loaded = false;
  private loadPromise: Promise<void> | null = null;

  get sources(): AgentSource[] {
    return this.results.map((result) => result.source);
  }

  get packages(): AgentPackageResult[] {
    return this.results.flatMap((result) => result.agents);
  }

  async load(force = false): Promise<void> {
    if (this.loadPromise) {
      await this.loadPromise;
      if (!force) return;
    }
    if (this.loaded && !force) return;
    this.loadPromise = (async () => {
      this.loading = true;
      this.error = null;
      try {
        [this.results, this.drafts, this.library] = await Promise.all([
          agentSourcesInspect(), agentDraftsList(), agentLibraryList(),
        ]);
        this.loaded = true;
      } catch (error) {
        this.error = message(error);
      } finally {
        this.loading = false;
      }
    })();
    try {
      await this.loadPromise;
    } finally {
      this.loadPromise = null;
    }
  }

  private async mutation<T>(operation: () => Promise<T>, journal?: AgentJournal): Promise<T | null> {
    this.busy = true;
    this.error = null;
    try {
      const result = await operation();
      await this.load(true);
      if (journal) activity.log({ ...journal, outcome: "ok" });
      return result;
    } catch (error) {
      this.error = message(error);
      if (journal) activity.log({ ...journal, outcome: "error", detail: safeActivityDetail(this.error) });
      return null;
    } finally {
      this.busy = false;
    }
  }

  private referenceLabel(reference: AgentReference): string {
    const pkg = this.packages.find((candidate) =>
      candidate.reference.sourceId === reference.sourceId
      && candidate.reference.relativePath === reference.relativePath
    );
    const source = this.sources.find((candidate) => candidate.id === reference.sourceId);
    return `${pkg?.agent?.name ?? reference.relativePath} · ${source?.label ?? reference.sourceId}`;
  }

  folderFor(reference: AgentReference): string | null {
    return this.library.assignments.find((item) =>
      item.sourceId === reference.sourceId && item.relativePath === reference.relativePath
    )?.folderPath ?? null;
  }

  isFavorite(reference: AgentReference): boolean {
    return this.library.favorites.some((item) =>
      item.sourceId === reference.sourceId && item.relativePath === reference.relativePath
    );
  }

  updatePolicy(reference: AgentReference): AgentUpdatePolicy {
    return this.library.updatePolicies.find((item) =>
      item.agent.sourceId === reference.sourceId
      && item.agent.relativePath === reference.relativePath
    )?.policy ?? "notify";
  }

  addLocal(root: string) {
    return this.mutation(() => agentSourceAddLocal(root), {
      action: "sourceAdd", subject: "agentSource", subjectName: root,
    });
  }
  async addGithub(repository: string, gitRef: string | null, subdirectory: string | null) {
    const source = await this.mutation(() => agentSourceAddGithub(repository, gitRef, subdirectory), {
      action: "sourceAdd", subject: "agentSource", subjectName: repository,
    });
    if (source) await this.refreshSource(source.id);
    return source;
  }
  refreshSource(sourceId: string) {
    const label = this.sources.find((source) => source.id === sourceId)?.label ?? sourceId;
    return this.mutation(() => agentSourceRefresh(sourceId), {
      action: "sourceRefresh", subject: "agentSource", subjectName: label,
    });
  }
  removeSource(sourceId: string) {
    const label = this.sources.find((source) => source.id === sourceId)?.label ?? sourceId;
    return this.mutation(() => agentSourceRemove(sourceId), {
      action: "sourceRemove", subject: "agentSource", subjectName: label,
    });
  }
  createDraft(input: AgentDraftInput) {
    return this.mutation(() => agentDraftCreate(input), {
      action: "draftCreate", subject: "agentDraft", subjectName: input.relativePath,
    });
  }
  editDraft(id: string, input: AgentDraftInput) {
    return this.mutation(() => agentDraftEdit(id, input), {
      action: "draftEdit", subject: "agentDraft", subjectName: input.relativePath,
    });
  }
  duplicateDraft(reference: AgentReference) {
    return this.mutation(() => agentDraftDuplicate(reference), {
      action: "draftCreate", subject: "agentDraft", subjectName: this.referenceLabel(reference),
    });
  }
  publishDraft(id: string) {
    const label = this.drafts.find((draft) => draft.id === id)?.validation.agent?.name ?? id;
    return this.mutation(() => agentDraftPublish(id), {
      action: "draftPublish", subject: "agentDraft", subjectName: label,
    });
  }
  rejectDraft(id: string) {
    const label = this.drafts.find((draft) => draft.id === id)?.validation.agent?.name ?? id;
    return this.mutation(() => agentDraftReject(id), {
      action: "draftReject", subject: "agentDraft", subjectName: label,
    });
  }
  createFolder(path: string) {
    return this.mutation(() => agentFolderCreate(path), {
      action: "organize", subject: "agentLibrary", subjectName: path,
    });
  }
  renameFolder(path: string, newName: string) {
    return this.mutation(() => agentFolderRename(path, newName), {
      action: "organize", subject: "agentLibrary", subjectName: path,
    });
  }
  moveFolder(path: string, newParent: string | null) {
    return this.mutation(() => agentFolderMove(path, newParent), {
      action: "organize", subject: "agentLibrary", subjectName: path,
    });
  }
  deleteFolder(path: string, recursive: boolean) {
    return this.mutation(() => agentFolderDelete(path, recursive), {
      action: "organize", subject: "agentLibrary", subjectName: path,
    });
  }
  assignFolder(reference: AgentReference, folderPath: string | null) {
    return this.mutation(() => agentFolderAssign(reference, folderPath), {
      action: "organize", subject: "agent", subjectName: this.referenceLabel(reference),
    });
  }
  setFavorite(reference: AgentReference, favorite: boolean) {
    return this.mutation(() => agentFavoriteSet(reference, favorite), {
      action: "organize", subject: "agent", subjectName: this.referenceLabel(reference),
    });
  }
  touchRecent(reference: AgentReference) { return this.mutation(() => agentRecentTouch(reference)); }
  saveCollection(collection: AgentCollection) {
    return this.mutation(() => agentCollectionSave(collection), {
      action: "organize", subject: "agentLibrary", subjectName: collection.name,
    });
  }
  deleteCollection(name: string) {
    return this.mutation(() => agentCollectionDelete(name), {
      action: "organize", subject: "agentLibrary", subjectName: name,
    });
  }
  saveSmartFolder(smartFolder: AgentSmartFolder) {
    return this.mutation(() => agentSmartFolderSave(smartFolder), {
      action: "organize", subject: "agentLibrary", subjectName: smartFolder.name,
    });
  }
  deleteSmartFolder(name: string) {
    return this.mutation(() => agentSmartFolderDelete(name), {
      action: "organize", subject: "agentLibrary", subjectName: name,
    });
  }
  saveProfile(profile: AgentWorkspaceProfile) {
    return this.mutation(() => agentProfileSave(profile), {
      action: "organize", subject: "agentLibrary", subjectName: profile.name,
    });
  }
  deleteProfile(name: string) {
    return this.mutation(() => agentProfileDelete(name), {
      action: "organize", subject: "agentLibrary", subjectName: name,
    });
  }
  importLibrary(path: string) {
    return this.mutation(() => agentLibraryImport(path), {
      action: "organize", subject: "agentLibrary", subjectName: path,
    });
  }
  exportLibrary(path: string) { return agentLibraryExport(path); }
  setPublisherTrust(trust: AgentPublisherTrust) {
    return this.mutation(() => agentPublisherTrustSet(trust), {
      action: "organize", subject: "agentLibrary", subjectName: trust.name,
    });
  }
  setPreferredSource(preferred: AgentPreferredSource) {
    return this.mutation(() => agentPreferredSourceSet(preferred), {
      action: "organize", subject: "agentLibrary", subjectName: preferred.agentName,
    });
  }
  setUpdatePolicy(reference: AgentReference, policy: AgentUpdatePolicy) {
    return this.mutation(() => agentUpdatePolicySet(reference, policy), {
      action: "organize", subject: "agent", subjectName: this.referenceLabel(reference),
    });
  }
  async approveRequest(id: string): Promise<boolean> {
    this.busy = true;
    this.error = null;
    try {
      const approval = await agentApprovalApprove(id);
      await this.load(true);
      await activity.refreshMcpAudit();
      const approved = approval.state === "approved";
      activity.log({
        action: "approvalApprove",
        subject: "agentApproval",
        subjectName: id,
        outcome: approved ? "ok" : "error",
        detail: approval.result ? safeActivityDetail(approval.result) : undefined,
      });
      return approved;
    } catch (error) {
      this.error = message(error);
      activity.log({
        action: "approvalApprove", subject: "agentApproval", subjectName: id,
        outcome: "error", detail: safeActivityDetail(this.error),
      });
      return false;
    } finally {
      this.busy = false;
    }
  }
  async rejectRequest(id: string): Promise<boolean> {
    const rejected = await this.mutation(() => agentApprovalReject(id), {
      action: "approvalReject", subject: "agentApproval", subjectName: id,
    });
    if (rejected) await activity.refreshMcpAudit();
    return rejected !== null;
  }
}

export const agentLibrary = new AgentLibraryStore();
