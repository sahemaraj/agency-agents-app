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
import type { ProjectInfo } from "$lib/types";

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

  /** Native folder picker → registers and returns the chosen path (or null). */
  async addViaPicker(): Promise<string | null> {
    const picked = await openDialog({ directory: true, title: i18n.t("projects.chooseFolderTitle") });
    const path = typeof picked === "string" ? picked : Array.isArray(picked) ? (picked[0] ?? null) : null;
    return path ? this.register(path) : null;
  }
}

export const projects = new ProjectsStore();
