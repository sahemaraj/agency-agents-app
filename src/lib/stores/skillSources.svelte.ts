import {
  skillSourceAddLocal,
  skillSourceRefresh,
  skillSourcesList,
} from "$lib/api";
import {
  appErrorMessage,
  isAppError,
  type SkillSource,
  type SkillSourceResult,
} from "$lib/types";

export interface AddSourceResult {
  registrationSucceeded: boolean;
  initialRefreshSucceeded: boolean;
}

function errorMessage(error: unknown): string {
  return isAppError(error) ? appErrorMessage(error) : String(error);
}

class SkillSourcesStore {
  sources: SkillSource[] = $state([]);
  results: Record<string, SkillSourceResult> = $state({});
  refreshErrors: Record<string, string> = $state({});
  addError: string | null = $state(null);
  loading = $state(false);
  adding = $state(false);
  refreshing: Record<string, boolean> = $state({});

  async load(): Promise<void> {
    if (this.loading) return;
    this.loading = true;
    try {
      this.sources = await skillSourcesList();
    } catch (error) {
      this.addError = errorMessage(error);
    } finally {
      this.loading = false;
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
      return { registrationSucceeded: true, initialRefreshSucceeded };
    } catch (error) {
      this.addError = errorMessage(error);
      return { registrationSucceeded: false, initialRefreshSucceeded: false };
    } finally {
      this.adding = false;
    }
  }

  async refresh(sourceId: string): Promise<boolean> {
    if (this.refreshing[sourceId]) return false;
    this.refreshing = { ...this.refreshing, [sourceId]: true };
    try {
      const result = await skillSourceRefresh(sourceId);
      this.results = { ...this.results, [sourceId]: result };
      this.mergeSource(result.source);
      const { [sourceId]: _cleared, ...remainingErrors } = this.refreshErrors;
      this.refreshErrors = remainingErrors;
      return true;
    } catch (error) {
      this.refreshErrors = {
        ...this.refreshErrors,
        [sourceId]: errorMessage(error),
      };
      return false;
    } finally {
      const { [sourceId]: _cleared, ...remainingBusy } = this.refreshing;
      this.refreshing = remainingBusy;
    }
  }

  isRefreshing(sourceId: string): boolean {
    return this.refreshing[sourceId] === true;
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
