/**
 * Typed `invoke()` wrappers for the agency backend command surface.
 *
 * Convention: each function resolves with the typed result, or *throws* a
 * `AppErrorPayload`-shaped object on backend error. Callers should use
 * `try/catch` and `isAppError(e)` to narrow.
 *
 * Covers the cross-cutting infrastructure the agency shell relies on:
 * app version, settings persistence, GitHub integration, and the in-app
 * updater. (Agent catalog/install commands live in their own modules —
 * `corpus` / `install`.)
 */

import { invoke } from "@tauri-apps/api/core";

import type {
  CreatedIssue,
  DeviceFlowPoll,
  DeviceFlowStart,
  GithubStatus,
  GeneralSettingsPatch,
  RepoStats,
  Settings,
  SkillDraft,
  SkillFolderState,
  SkillCollection,
  SkillReference,
  SkillSmartFolder,
  SkillWorkspaceProfile,
  SkillUpdatePolicy,
  SkillApproval,
  SkillPackageResult,
  SkillSource,
  SkillDestinationPresence,
  InstalledSkill,
  McpAuditEntry,
  McpClient,
  McpClientStatus,
  SkillSourceResult,
  SkillVersionSnapshot,
  SkillMutationPlan,
  SkillPublisherTrust,
  SkillPreferredSource,
  SkillBatchResult,
  UpdateCheckOutcome,
} from "./types";

export function skillInstallPlan(
  sourceId: string,
  relativePath: string,
  runtime: "claudeCode" | "codex",
  projectPath: string | null,
): Promise<SkillMutationPlan> {
  return invoke<SkillMutationPlan>("skill_install_plan", {
    sourceId,
    relativePath,
    runtime,
    projectPath,
  });
}

export function skillPublisherTrustSet(trust: SkillPublisherTrust): Promise<SkillFolderState> {
  return invoke<SkillFolderState>("skill_publisher_trust_set", { trust });
}

export function skillPreferredSourceSet(preference: SkillPreferredSource): Promise<SkillFolderState> {
  return invoke<SkillFolderState>("skill_preferred_source_set", { preference });
}

export function skillCollectionBatch(
  collectionName: string,
  operation: "install" | "update" | "uninstall",
  runtime: "claudeCode" | "codex",
  projectPath: string | null,
): Promise<SkillBatchResult> {
  return invoke<SkillBatchResult>("skill_collection_batch", {
    collectionName,
    operation,
    runtime,
    projectPath,
  });
}

// ============================================================
// App version (from tauri::App::package_info)
// ============================================================

/**
 * App version string from `tauri::App::package_info()` — the source of
 * truth is `Cargo.toml` (mirrored by `tauri.conf.json`). Cheaper and
 * more honest than reading `package.json` from the renderer.
 */
export function appVersion(): Promise<string> {
  return invoke<string>("app_version");
}

// ============================================================
// Settings persistence
// ============================================================

/**
 * Read the currently-loaded settings.
 *
 * Throws a `AppErrorPayload` with `code === "internal"` when the
 * settings file on disk is unparseable — in that case the backend is
 * already failing closed (`require_network` denies all outbound calls
 * until the user resets). The Settings UI should catch the throw and
 * show a "Settings file unreadable — Reset to defaults?" affordance
 * that calls `settingsReset()`.
 */
export function settingsGet(): Promise<Settings> {
  return invoke<Settings>("settings_get");
}

/**
 * Persist general settings. MCP policy fields are preserved from the latest
 * disk state; use `mcpPolicySet` for an intentional policy change.
 */
export function settingsSet(patch: GeneralSettingsPatch): Promise<Settings> {
  return invoke<Settings>("settings_set", { patch });
}

export function mcpPolicySet(
  sourceAccess: boolean,
  installAccess: boolean,
  destructiveAccess: boolean,
  projectAllowlist: string[],
): Promise<Settings> {
  return invoke<Settings>("mcp_policy_set", {
    sourceAccess,
    installAccess,
    destructiveAccess,
    projectAllowlist,
  });
}

export function mcpClientPolicySet(
  client: McpClient,
  sourceAccess: boolean,
  installAccess: boolean,
  destructiveAccess: boolean,
): Promise<Settings> {
  return invoke<Settings>("mcp_client_policy_set", {
    client,
    sourceAccess,
    installAccess,
    destructiveAccess,
  });
}

/**
 * Overwrite `settings.json` with defaults. Used by the "Reset to
 * defaults" button in Settings → Network when the file is corrupt or
 * the user wants to start fresh.
 */
export function settingsReset(): Promise<Settings> {
  return invoke<Settings>("settings_reset");
}

export function mcpAuditList(): Promise<McpAuditEntry[]> {
  return invoke<McpAuditEntry[]>("mcp_audit_list");
}

export function mcpClientsStatus(): Promise<McpClientStatus[]> {
  return invoke<McpClientStatus[]>("mcp_clients_status");
}

export function mcpClientConnect(client: McpClient): Promise<McpClientStatus> {
  return invoke<McpClientStatus>("mcp_client_connect", { client });
}

export function mcpClientDisconnect(client: McpClient): Promise<McpClientStatus> {
  return invoke<McpClientStatus>("mcp_client_disconnect", { client });
}

export function mcpClientRepair(client: McpClient): Promise<McpClientStatus> {
  return invoke<McpClientStatus>("mcp_client_repair", { client });
}

// ============================================================
// Skill sources
// ============================================================

export function skillSourcesList(): Promise<SkillSource[]> {
  return invoke<SkillSource[]>("skill_sources_list");
}

export function skillSourcesInspect(): Promise<SkillSourceResult[]> {
  return invoke<SkillSourceResult[]>("skill_sources_inspect");
}

export function skillTrustGrant(
  sourceId: string,
  relativePath: string,
): Promise<SkillPackageResult> {
  return invoke<SkillPackageResult>("skill_trust_grant", { sourceId, relativePath });
}

export function skillTrustRevoke(sourceId: string, relativePath: string): Promise<boolean> {
  return invoke<boolean>("skill_trust_revoke", { sourceId, relativePath });
}

export function skillPackageDestinations(
  sourceId: string,
  relativePath: string,
  projectPaths: string[],
): Promise<SkillDestinationPresence[]> {
  return invoke<SkillDestinationPresence[]>("skill_package_destinations", {
    sourceId,
    relativePath,
    projectPaths,
  });
}

export function skillInstall(
  sourceId: string,
  relativePath: string,
  runtime: "claudeCode" | "codex",
  projectPath: string | null,
): Promise<InstalledSkill> {
  return invoke<InstalledSkill>("skill_install", {
    sourceId,
    relativePath,
    runtime,
    projectPath,
  });
}

export function skillInstallWithDependencies(
  sourceId: string,
  relativePath: string,
  runtime: "claudeCode" | "codex",
  projectPath: string | null,
): Promise<InstalledSkill[]> {
  return invoke<InstalledSkill[]>("skill_install_with_dependencies", {
    sourceId,
    relativePath,
    runtime,
    projectPath,
  });
}

export function skillInstallsReconcile(projectPaths: string[]): Promise<InstalledSkill[]> {
  return invoke<InstalledSkill[]>("skill_installs_reconcile", { projectPaths });
}

function skillLifecycle(
  command: "skill_update" | "skill_disable" | "skill_enable",
  installed: InstalledSkill,
): Promise<InstalledSkill> {
  return invoke<InstalledSkill>(command, {
    sourceId: installed.sourceId,
    relativePath: installed.relativePath,
    runtime: installed.runtime,
    projectPath: installed.projectPath,
  });
}

export const skillUpdate = (installed: InstalledSkill) =>
  skillLifecycle("skill_update", installed);

export const skillDisable = (installed: InstalledSkill) =>
  skillLifecycle("skill_disable", installed);

export const skillEnable = (installed: InstalledSkill) =>
  skillLifecycle("skill_enable", installed);

export function skillUninstall(installed: InstalledSkill): Promise<boolean> {
  return invoke<boolean>("skill_uninstall", {
    sourceId: installed.sourceId,
    relativePath: installed.relativePath,
    runtime: installed.runtime,
    projectPath: installed.projectPath,
  });
}

export function skillBackupsList(): Promise<string[]> {
  return invoke<string[]>("skill_backups_list");
}

export function skillVersionHistory(installed: InstalledSkill): Promise<SkillVersionSnapshot[]> {
  return invoke<SkillVersionSnapshot[]>("skill_version_history_list", {
    sourceId: installed.sourceId,
    relativePath: installed.relativePath,
    runtime: installed.runtime,
    projectPath: installed.projectPath,
  });
}

export function skillVersionRollback(
  installed: InstalledSkill,
  snapshotPath: string,
): Promise<InstalledSkill> {
  return invoke<InstalledSkill>("skill_version_rollback", {
    sourceId: installed.sourceId,
    relativePath: installed.relativePath,
    runtime: installed.runtime,
    projectPath: installed.projectPath,
    snapshotPath,
  });
}

export function skillSourceAddLocal(root: string): Promise<SkillSource> {
  return invoke<SkillSource>("skill_source_add_local", { root });
}

export function skillSourceAddGithub(
  repository: string,
  gitRef: string | null = null,
  subdirectory: string | null = null,
): Promise<SkillSource> {
  return invoke<SkillSource>("skill_source_add_github", {
    repository,
    gitRef,
    subdirectory,
  });
}

export function skillSourceRefresh(sourceId: string): Promise<SkillSourceResult> {
  return invoke<SkillSourceResult>("skill_source_refresh", { sourceId });
}

export function skillSourceRemove(sourceId: string): Promise<boolean> {
  return invoke<boolean>("skill_source_remove", { sourceId });
}

export function skillDraftsList(): Promise<SkillDraft[]> {
  return invoke<SkillDraft[]>("skill_drafts_list");
}

export function skillDraftPublish(id: string): Promise<SkillDraft> {
  return invoke<SkillDraft>("skill_draft_publish", { id });
}

export function skillDraftReject(id: string): Promise<SkillDraft> {
  return invoke<SkillDraft>("skill_draft_reject", { id });
}

export function skillDraftCreate(
  name: string,
  description: string,
  skillType: import("./types").SkillType,
  group: string[],
  tags: string[],
  body: string,
): Promise<SkillDraft> {
  return invoke<SkillDraft>("skill_draft_create", { name, description, skillType, group, tags, body });
}

export function skillDraftEdit(
  sourceId: string,
  relativePath: string,
  skillMd: string,
): Promise<SkillDraft> {
  return invoke<SkillDraft>("skill_draft_edit", { sourceId, relativePath, skillMd });
}

export function skillTextRead(sourceId: string, relativePath: string): Promise<string> {
  return invoke<string>("skill_text_read", { sourceId, relativePath });
}

export function skillFoldersList(): Promise<SkillFolderState> {
  return invoke<SkillFolderState>("skill_folders_list");
}

export function skillFolderCreate(path: string): Promise<SkillFolderState> {
  return invoke<SkillFolderState>("skill_folder_create", { path });
}

export function skillFolderRename(path: string, newName: string): Promise<SkillFolderState> {
  return invoke<SkillFolderState>("skill_folder_rename", { path, newName });
}

export function skillFolderMove(path: string, newParent: string | null): Promise<SkillFolderState> {
  return invoke<SkillFolderState>("skill_folder_move", { path, newParent });
}

export function skillFolderDelete(path: string, recursive: boolean): Promise<SkillFolderState> {
  return invoke<SkillFolderState>("skill_folder_delete", { path, recursive });
}

export function skillFolderAssign(
  sourceId: string,
  relativePath: string,
  folderPath: string | null,
): Promise<SkillFolderState> {
  return invoke<SkillFolderState>("skill_folder_assign", { sourceId, relativePath, folderPath });
}

export function skillFoldersImport(imported: SkillFolderState): Promise<SkillFolderState> {
  return invoke<SkillFolderState>("skill_folders_import", { imported });
}

export function skillFavoriteSet(skill: SkillReference, favorite: boolean): Promise<SkillFolderState> {
  return invoke<SkillFolderState>("skill_favorite_set", { skill, favorite });
}

export function skillRecentTouch(skill: SkillReference): Promise<SkillFolderState> {
  return invoke<SkillFolderState>("skill_recent_touch", { skill });
}

export function skillCollectionSave(collection: SkillCollection): Promise<SkillFolderState> {
  return invoke<SkillFolderState>("skill_collection_save", { collection });
}

export function skillCollectionDelete(name: string): Promise<SkillFolderState> {
  return invoke<SkillFolderState>("skill_collection_delete", { name });
}

export function skillSmartFolderSave(smartFolder: SkillSmartFolder): Promise<SkillFolderState> {
  return invoke<SkillFolderState>("skill_smart_folder_save", { smartFolder });
}

export function skillSmartFolderDelete(name: string): Promise<SkillFolderState> {
  return invoke<SkillFolderState>("skill_smart_folder_delete", { name });
}

export function skillProfileSave(profile: SkillWorkspaceProfile): Promise<SkillFolderState> {
  return invoke<SkillFolderState>("skill_profile_save", { profile });
}

export function skillProfileDelete(name: string): Promise<SkillFolderState> {
  return invoke<SkillFolderState>("skill_profile_delete", { name });
}

export function skillLibraryReplace(replacement: SkillFolderState): Promise<SkillFolderState> {
  return invoke<SkillFolderState>("skill_library_replace", { replacement });
}

export function skillLibraryExport(path: string): Promise<number> {
  return invoke<number>("skill_library_export", { path });
}

export function skillLibraryImport(path: string): Promise<SkillFolderState> {
  return invoke<SkillFolderState>("skill_library_import", { path });
}

export function skillUpdatePolicySet(
  skill: SkillReference,
  policy: SkillUpdatePolicy,
): Promise<SkillFolderState> {
  return invoke<SkillFolderState>("skill_update_policy_set", { skill, policy });
}

export function skillApprovalApprove(id: string): Promise<SkillApproval> {
  return invoke<SkillApproval>("skill_approval_approve", { id });
}

export function skillApprovalReject(id: string): Promise<SkillApproval> {
  return invoke<SkillApproval>("skill_approval_reject", { id });
}

// ============================================================
// GitHub integration
// ============================================================

/**
 * Fetch repo stats for `homepage`. Returns `null` when the user hasn't
 * enabled GitHub stats, the URL doesn't parse as a github repo, or the
 * repo 404s.
 *
 * Throws `AppErrorPayload` with `code === "paranoid_mode_blocked"`
 * when paranoid mode is on, or `"github_rate_limited"` on the anonymous
 * 60/hr per-IP cap.
 */
export function githubRepoStats(homepage: string): Promise<RepoStats | null> {
  return invoke<RepoStats | null>("github_repo_stats", { homepage });
}

/**
 * Read the current sign-in status. Reads from the macOS Keychain only —
 * no network call. The DTO contains `{ signedIn, username, scopes }`,
 * never the token.
 */
export function githubStatus(): Promise<GithubStatus> {
  return invoke<GithubStatus>("github_status");
}

/**
 * Begin a GitHub Device Flow sign-in. POSTs to
 * `github.com/login/device/code` and returns the user code +
 * verification URI to show in the DeviceFlowModal. Subject to the
 * paranoid-mode gate.
 */
export function githubSigninStart(): Promise<DeviceFlowStart> {
  return invoke<DeviceFlowStart>("github_signin_start");
}

/**
 * Poll the token endpoint once with the opaque `deviceCode` returned
 * by `githubSigninStart`. Caller drives the polling loop using the
 * `interval` from the start response.
 */
export function githubSigninPoll(deviceCode: string): Promise<DeviceFlowPoll> {
  return invoke<DeviceFlowPoll>("github_signin_poll", { deviceCode });
}

/**
 * Delete the stored OAuth token (and cached username/scopes) from the
 * macOS Keychain. Idempotent.
 */
export function githubSignout(): Promise<void> {
  return invoke<void>("github_signout");
}

/**
 * Star the repo whose URL matches `homepage`. The backend validates
 * the URL is `github.com/<owner>/<repo>` before any network call.
 */
export function githubStar(homepage: string): Promise<void> {
  return invoke<void>("github_star", { homepage });
}

/** Unstar — idempotent on the GitHub side. */
export function githubUnstar(homepage: string): Promise<void> {
  return invoke<void>("github_unstar", { homepage });
}

/**
 * Check whether the signed-in user has starred `homepage`. Backend maps
 * 204 → true, 404 → false.
 */
export function githubIsStarred(homepage: string): Promise<boolean> {
  return invoke<boolean>("github_is_starred", { homepage });
}

/** Watch the repo (`subscribed: true, ignored: false`). */
export function githubWatch(homepage: string): Promise<void> {
  return invoke<void>("github_watch", { homepage });
}

/** Stop watching — idempotent. */
export function githubUnwatch(homepage: string): Promise<void> {
  return invoke<void>("github_unwatch", { homepage });
}

/**
 * File an issue against the repo. Backend sanitises and caps title,
 * body, and labels. Returns the new issue's `{ number, htmlUrl }`.
 */
export function githubCreateIssue(
  homepage: string,
  title: string,
  body: string,
  labels: string[],
): Promise<CreatedIssue> {
  return invoke<CreatedIssue>("github_create_issue", {
    homepage,
    title,
    body,
    labels,
  });
}

// ============================================================
// In-app updater
// ============================================================

/**
 * Check the manifest for a newer release. Backend handles the version
 * comparison, the skip-list consultation, and the URL allowlist.
 *
 * Throws `AppErrorPayload` with `code === "paranoid_mode_blocked"`
 * (feature: "update_check") when Offline Mode is on.
 */
export function updateCheckNow(): Promise<UpdateCheckOutcome> {
  return invoke<UpdateCheckOutcome>("update_check_now");
}

/**
 * Download, verify, and install the named version. The backend
 * cross-checks `version` against the cached "available" entry from the
 * most recent `update_check_now` call.
 */
export function updateInstall(version: string): Promise<void> {
  return invoke<void>("update_install", { version });
}

/**
 * Add `version` to the skip-list so the title-bar indicator stops
 * surfacing for this release.
 */
export function updateSkip(version: string): Promise<void> {
  return invoke<void>("update_skip", { version });
}

/**
 * Restart the running process so the freshly-installed .app picks up.
 * Called from the "Relaunch now" affordance after `updateInstall`.
 */
export function updateRelaunch(): Promise<void> {
  return invoke<void>("update_relaunch");
}
