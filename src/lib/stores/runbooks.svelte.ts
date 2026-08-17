/**
 * Runbooks store — the NEXUS scenario runbooks from the catalog's
 * `strategy/runbooks.json` (catalog PR #664). Each runbook names its roster BY
 * SLUG; the UI resolves those against the loaded corpus to deploy the set.
 *
 * `strategy/` only ships in a synced catalog (not the bundled snapshot), so an
 * empty list is the normal "not synced yet" state — the UI shows a
 * "sync to unlock" nudge rather than an error. Backend-not-ready posture matches
 * the corpus/install stores: a failed invoke degrades to empty.
 *
 * Singleton: import `runbooks` everywhere.
 */
import { invoke } from "@tauri-apps/api/core";
import { playbookRead, playbooksList } from "$lib/api";
import { appErrorMessage, isAppError, type PlaybookCatalogEntry, type PlaybookDocument, type Runbook } from "$lib/types";

const errorMessage = (error: unknown) => isAppError(error) ? appErrorMessage(error) : String(error);

export function filterPlaybooks(
  entries: PlaybookCatalogEntry[],
  query: string,
): PlaybookCatalogEntry[] {
  const needle = query.trim().toLowerCase();
  return entries
    .filter((entry) => !needle || `${entry.title}\n${entry.relativePath}`.toLowerCase().includes(needle))
    .toSorted((left, right) => left.relativePath < right.relativePath ? -1 : left.relativePath > right.relativePath ? 1 : 0);
}

class RunbooksStore {
  /** The scenario runbooks, in manifest order. Empty until loaded / when unsynced. */
  list: Runbook[] = $state([]);
  /** True once the first load resolves (so "empty" ≠ "not fetched yet"). */
  loaded: boolean = $state(false);
  /** True while a load is in flight. */
  loading: boolean = $state(false);
  runbooksError: string | null = $state(null);
  playbooks: PlaybookCatalogEntry[] = $state([]);
  selected: PlaybookDocument | null = $state(null);
  error: string | null = $state(null);
  reading: boolean = $state(false);
  readError: string | null = $state(null);

  /** Load the manifest from the active catalog. Safe to call on mount. */
  async load(): Promise<void> {
    if (this.loading) return;
    this.loading = true;
    this.runbooksError = null;
    this.error = null;
    const [runbookResult, playbookResult] = await Promise.allSettled([
      invoke<Runbook[]>("runbooks_list"),
      playbooksList(),
    ]);
    if (runbookResult.status === "fulfilled") this.list = runbookResult.value;
    else this.runbooksError = errorMessage(runbookResult.reason);
    if (playbookResult.status === "fulfilled") this.playbooks = playbookResult.value;
    else this.error = errorMessage(playbookResult.reason);
    this.loaded = true;
    this.loading = false;
  }

  async retryRunbooks(): Promise<void> {
    this.runbooksError = null;
    try {
      this.list = await invoke<Runbook[]>("runbooks_list");
    } catch (error) {
      this.runbooksError = errorMessage(error);
    }
  }

  async retryPlaybooks(): Promise<void> {
    this.error = null;
    try {
      this.playbooks = await playbooksList();
    } catch (error) {
      this.error = errorMessage(error);
    }
  }

  async read(relativePath: string): Promise<void> {
    if (this.reading) return;
    this.reading = true;
    this.readError = null;
    this.selected = null;
    try {
      this.selected = await playbookRead(relativePath);
    } catch (error) {
      this.readError = errorMessage(error);
    } finally {
      this.reading = false;
    }
  }
}

export const runbooks = new RunbooksStore();
