<script lang="ts">
  import { onMount } from "svelte";
  import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
  import FolderPlus from "@lucide/svelte/icons/folder-plus";
  import Settings2 from "@lucide/svelte/icons/settings-2";
  import Star from "@lucide/svelte/icons/star";
  import Search from "@lucide/svelte/icons/search";

  import Button from "$lib/components/Button.svelte";
  import AgentCreatorModal from "$lib/components/AgentCreatorModal.svelte";
  import DestructiveConfirm from "$lib/components/DestructiveConfirm.svelte";
  import DeploymentTargetGrid, {
    type DeploymentCell,
    type DeploymentColumn,
    type DeploymentRow,
  } from "$lib/components/DeploymentTargetGrid.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import Input from "$lib/components/Input.svelte";
  import LoadingState from "$lib/components/LoadingState.svelte";
  import Modal from "$lib/components/Modal.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { skillSources } from "$lib/stores/skillSources.svelte";
  import { i18n } from "$lib/stores/i18n.svelte";
  import { skillCollectionBatch, skillInstallPlan } from "$lib/api";
  import type { InstalledSkill, SkillApprovalAction, SkillDraft, SkillMutationPlan, SkillPackageResult, SkillReference, SkillSmartFolderRule, SkillSource, SkillSourceResult, SkillType, SkillUpdatePolicy, SkillVersionSnapshot } from "$lib/types";
  import {
    buildPersonalFolderTree,
    filterPackages,
    groupPackages,
    isInstalled as packageIsInstalled,
    libraryMetrics,
    packageConflicts as findPackageConflicts,
    requiresTrust,
    sourceLabel,
    taxonomyLabel,
    trustedScripts,
    typeLabel,
    type PackageView,
    type PersonalFolderNode,
    type SkillGroupNode,
    type SortOrder,
    type StatusFilter,
  } from "$lib/skills/libraryModel";

  type DetailTab = "overview" | "files" | "security";

  const PERSONAL_FOLDERS_KEY = "agency-agents:skill-folders:v1";

  let localPath = $state("");
  let announcement = $state("");
  let removeCandidate: SkillSource | null = $state(null);
  let githubRepository = $state("");
  let githubRef = $state("");
  let githubSubdirectory = $state("");
  let githubRegistrationRejected = $state(false);
  let query = $state("");
  let searchInput: HTMLInputElement | undefined = $state();
  let sourceManager: HTMLDetailsElement | undefined = $state();
  let approvalInbox: HTMLDetailsElement | undefined = $state();
  let statusFilter: StatusFilter = $state("all");
  let sourceFilter = $state("all");
  let libraryFilter = $state("all");
  let sortOrder: SortOrder = $state("name");
  let detailTab: DetailTab = $state("overview");
  let selectedKey: string | null = $state(null);
  let uninstallCandidate: InstalledSkill | null = $state(null);
  let rejectDraftCandidate: SkillDraft | null = $state(null);
  let trustCandidate: PackageView | null = $state(null);
  let folderModalOpen = $state(false);
  let folderName = $state("");
  let folderParent = $state("");
  let folderError = $state("");
  let organizeOpen = $state(false);
  let collectionName = $state("");
  let smartFolderName = $state("");
  let profileName = $state("");
  let organizeDelete: { kind: "collection" | "smart" | "profile"; name: string } | null = $state(null);
  let versionHistory: Record<string, SkillVersionSnapshot[]> = $state({});
  let collectionInstallOpen = $state(false);
  let collectionRuntime: "claudeCode" | "codex" = $state("codex");
  let collectionProject = $state("");
  let collectionOperation: "install" | "update" | "uninstall" = $state("install");
  let creatorOpen = $state(false);
  let agentCreatorSkill: SkillReference | null = $state(null);
  let creatorName = $state("");
  let creatorDescription = $state("");
  let creatorType: SkillType = $state("other");
  let creatorGroup = $state("");
  let creatorTags = $state("");
  let creatorBody = $state("");
  let editorOpen = $state(false);
  let editorText = $state("");
  let editorLoading = $state(false);
  let installPlan: SkillMutationPlan | null = $state(null);
  let plannedInstall: { pkg: SkillPackageResult; runtime: "claudeCode" | "codex"; projectPath: string | null; destination: string } | null = $state(null);

  const packages = $derived.by<PackageView[]>(() =>
    Object.values(skillSources.results).flatMap((result) =>
      result.packages.map((pkg) => ({ pkg, source: result.source })),
    ),
  );
  const personalFolders = $derived(skillSources.folderState.folders);
  const filtered = $derived(filterPackages({
    packages,
    installed: skillSources.installed,
    folderState: skillSources.folderState,
    query,
    statusFilter,
    sourceFilter,
    libraryFilter,
    sortOrder,
  }));
  const grouped = $derived(groupPackages(packages));
  const selected = $derived(
    selectedKey === null
      ? null
      : packages.find(({ pkg }) => skillSources.packageKey(pkg) === selectedKey) ?? null,
  );
  const personalFolderTree = $derived(buildPersonalFolderTree(personalFolders));
  const deploymentColumns: DeploymentColumn[] = [
    { id: "claudeCode", label: "Claude Code", supportsUser: true, supportsProject: true },
    { id: "codex", label: "Codex", supportsUser: true, supportsProject: true },
  ];
  const deploymentRows = $derived<DeploymentRow[]>([
    { kind: "global" },
    ...projects.list.map((project) => ({ kind: "project" as const, path: project.path, label: project.label })),
  ]);
  const selectedInstalls = $derived(
    selected
      ? skillSources.installed.filter((record) =>
          record.sourceId === selected.pkg.sourceId
          && record.relativePath === selected.pkg.relativePath,
        )
      : [],
  );
  const unavailableInstalls = $derived(
    skillSources.installed.filter((record) => record.state === "sourceUnavailable"),
  );
  const pendingDrafts = $derived(skillSources.drafts.filter((draft) => draft.state === "pending"));
  const pendingApprovals = $derived(skillSources.folderState.approvals.filter((approval) => approval.state === "pending"));
  const metrics = $derived(libraryMetrics(packages, skillSources.installed, skillSources.folderState));
  const installedCount = $derived(metrics.installed);
  const trustedCount = $derived(metrics.trusted);
  const reviewCount = $derived(metrics.review);
  const recommendationCount = $derived(metrics.recommendations);
  const duplicateCount = $derived(metrics.duplicates);
  const cleanupCount = $derived(metrics.cleanup);
  const libraryTitle = $derived.by(() => {
    if (libraryFilter === "installed") return i18n.t("skills.installed");
    if (libraryFilter === "trusted") return i18n.t("skills.trustedScripts");
    if (libraryFilter === "review") return i18n.t("skills.needsReview");
    if (libraryFilter === "favorites") return i18n.t("skills.favorites");
    if (libraryFilter === "recent") return i18n.t("skills.recent");
    if (libraryFilter === "recommendations") return "Recommendations";
    if (libraryFilter === "duplicates") return "Duplicates";
    if (libraryFilter === "cleanup") return "Cleanup suggestions";
    if (libraryFilter.startsWith("collection:") || libraryFilter.startsWith("smart:")) {
      return libraryFilter.slice(libraryFilter.indexOf(":") + 1);
    }
    if (libraryFilter.startsWith("personal:")) {
      return libraryFilter.slice("personal:".length).split("/").at(-1) ?? i18n.t("skills.allSkills");
    }
    if (libraryFilter.startsWith("taxonomy:")) {
      const node = findGroup(grouped, libraryFilter.slice("taxonomy:".length));
      return node?.label ?? i18n.t("skills.allSkills");
    }
    return i18n.t("skills.allSkills");
  });

  $effect(() => {
    if (selectedKey !== null && !filtered.some(({ pkg }) => skillSources.packageKey(pkg) === selectedKey)) {
      selectedKey = null;
    }
  });

  onMount(() => {
    const focusSearch = (event: KeyboardEvent): void => {
      const target = event.target as HTMLElement | null;
      if (event.key !== "/" || target?.matches("input, textarea, select, [contenteditable='true']")) return;
      event.preventDefault();
      searchInput?.focus();
    };
    const closePopovers = (event: MouseEvent): void => {
      const target = event.target as Node | null;
      if (!target) return;
      if (sourceManager?.open && !sourceManager.contains(target)) sourceManager.open = false;
      if (approvalInbox?.open && !approvalInbox.contains(target)) approvalInbox.open = false;
    };
    document.addEventListener("keydown", focusSearch);
    document.addEventListener("click", closePopovers);
    void (async () => {
      await projects.refresh();
      await skillSources.load();
      await migratePersonalFolders();
      await skillSources.reconcileInstalls(projects.list.map((project) => project.path));
    })();
    return () => {
      document.removeEventListener("keydown", focusSearch);
      document.removeEventListener("click", closePopovers);
    };
  });

  function sourceKind(source: SkillSource): string {
    return source.kind.kind === "local" ? i18n.t("skills.localFolder") : i18n.t("skills.github");
  }

  function packageCount(result: SkillSourceResult | undefined): number {
    return result?.packages.filter((pkg) => pkg.installable).length ?? 0;
  }

  function statusLabel(status: StatusFilter): string {
    if (status === "ready") return i18n.t("skills.ready");
    if (status === "rejected") return i18n.t("skills.rejected");
    return i18n.t("skills.all");
  }

  function groupCount(node: SkillGroupNode): number {
    return node.packages.length + node.children.reduce((count, child) => count + groupCount(child), 0);
  }

  function findGroup(nodes: SkillGroupNode[], key: string): SkillGroupNode | undefined {
    for (const node of nodes) {
      if (node.key === key) return node;
      const child = findGroup(node.children, key);
      if (child) return child;
    }
    return undefined;
  }

  async function migratePersonalFolders(): Promise<void> {
    try {
      const parsed = JSON.parse(localStorage.getItem(PERSONAL_FOLDERS_KEY) ?? "{}") as {
        folders?: unknown;
        assignments?: unknown;
      };
      const folders = Array.isArray(parsed.folders)
        ? parsed.folders.filter((value): value is string => typeof value === "string" && value.length > 0)
        : [];
      const assignments = parsed.assignments && typeof parsed.assignments === "object"
        ? Object.entries(parsed.assignments).flatMap(([key, folderPath]) => {
            const separator = key.indexOf("\0");
            return separator > 0 && typeof folderPath === "string"
              ? [{
                  sourceId: key.slice(0, separator),
                  relativePath: key.slice(separator + 1),
                  folderPath,
                }]
              : [];
          })
        : [];
      if (folders.length === 0 && assignments.length === 0) return;
      if (await skillSources.importFolders({
        folders,
        assignments,
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
      })) {
        localStorage.removeItem(PERSONAL_FOLDERS_KEY);
      }
    } catch {
      folderError = i18n.t("skills.folderSaveError");
    }
  }

  function openFolderModal(parent = ""): void {
    folderParent = parent;
    folderName = "";
    folderError = "";
    folderModalOpen = true;
  }

  async function createPersonalFolder(): Promise<void> {
    const name = folderName.trim();
    if (!name || name.includes("/") || name.length > 64) {
      folderError = i18n.t("skills.folderNameError");
      return;
    }
    const path = folderParent ? `${folderParent}/${name}` : name;
    if (!(await skillSources.createFolder(path))) {
      folderError = skillSources.addError ?? i18n.t("skills.folderSaveError");
      return;
    }
    libraryFilter = `personal:${path}`;
    folderModalOpen = false;
  }

  async function assignSelectedFolder(folder: string): Promise<void> {
    if (!selected) return;
    if (!(await skillSources.assignFolder(selected.pkg, folder || null))) {
      folderError = skillSources.addError ?? i18n.t("skills.folderSaveError");
    }
  }

  function personalFolderCount(path: string): number {
    return packages.filter(({ pkg }) => {
      const assigned = skillSources.folderFor(pkg);
      return assigned === path || assigned?.startsWith(`${path}/`);
    }).length;
  }

  function missingDependencies(pkg: SkillPackageResult): string[] {
    const available = new Set(packages.flatMap(({ pkg }) => pkg.name ? [pkg.name] : []));
    return pkg.dependencies.filter((name) => !available.has(name));
  }

  function packageConflicts(pkg: SkillPackageResult): PackageView[] {
    return findPackageConflicts(pkg, packages);
  }

  async function saveCurrentCollection(): Promise<void> {
    const name = collectionName.trim();
    if (!name) return;
    const skills = filtered.map(({ pkg }) => skillSources.reference(pkg));
    if (await skillSources.saveCollection({ name, skills })) {
      collectionName = "";
      libraryFilter = `collection:${name}`;
    }
  }

  async function saveCurrentSmartFolder(): Promise<void> {
    const name = smartFolderName.trim();
    if (!name) return;
    const taxonomyType = libraryFilter.startsWith("taxonomy:")
      ? libraryFilter.slice("taxonomy:".length).split("/")[0] as SkillType
      : null;
    const rule: SkillSmartFolderRule = {
      query: query.trim() || null,
      skillType: taxonomyType,
      tag: null,
      sourceId: sourceFilter === "all" ? null : sourceFilter,
      installable: statusFilter === "all" ? null : statusFilter === "ready",
      favorite: libraryFilter === "favorites" ? true : null,
    };
    if (Object.values(rule).every((value) => value === null)) {
      folderError = i18n.t("skills.smartFolderNeedsFilter");
      return;
    }
    if (await skillSources.saveSmartFolder({ name, rule })) {
      smartFolderName = "";
      libraryFilter = `smart:${name}`;
    }
  }

  async function saveCurrentProfile(): Promise<void> {
    const name = profileName.trim();
    if (!name) return;
    if (await skillSources.saveProfile({
      name,
      folders: [...skillSources.folderState.folders],
      collections: skillSources.folderState.collections.map((collection) => collection.name),
      runtime: null,
      projectPath: null,
    })) profileName = "";
  }

  async function exportLibrary(): Promise<void> {
    const path = await saveDialog({
      title: i18n.t("skills.exportLibrary"),
      defaultPath: "skills-library.json",
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (!path) return;
    const count = await skillSources.exportLibrary(path);
    announcement = i18n.t("skills.libraryExported", { count });
  }

  async function importLibrary(): Promise<void> {
    const path = await openDialog({
      title: i18n.t("skills.importLibrary"),
      multiple: false,
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (!path || Array.isArray(path)) return;
    const succeeded = await skillSources.importLibrary(path);
    announcement = succeeded
      ? i18n.t("skills.libraryImported")
      : skillSources.addError ?? i18n.t("skills.folderSaveError");
  }

  async function deleteOrganizeItem(): Promise<void> {
    if (!organizeDelete) return;
    const { kind, name } = organizeDelete;
    const succeeded = kind === "collection"
      ? await skillSources.deleteCollection(name)
      : kind === "smart"
        ? await skillSources.deleteSmartFolder(name)
        : await skillSources.deleteProfile(name);
    if (succeeded) organizeDelete = null;
  }

  async function loadVersionHistory(installed: InstalledSkill): Promise<void> {
    try {
      versionHistory = {
        ...versionHistory,
        [lifecycleKey(installed)]: await skillSources.history(installed),
      };
    } catch {
      announcement = skillSources.addError ?? i18n.t("skills.historyFailed");
    }
  }

  async function rollbackVersion(installed: InstalledSkill, snapshotPath: string): Promise<void> {
    const succeeded = await skillSources.rollback(
      installed,
      snapshotPath,
      projects.list.map((project) => project.path),
    );
    announcement = succeeded
      ? i18n.t("skills.rollbackSucceeded")
      : skillSources.addError ?? i18n.t("skills.rollbackFailed");
    if (succeeded) await loadVersionHistory(installed);
  }

  async function installCurrentCollection(): Promise<void> {
    const collectionName = libraryFilter.startsWith("collection:")
      ? libraryFilter.slice("collection:".length)
      : "";
    try {
      const result = await skillCollectionBatch(
        collectionName,
        collectionOperation,
        collectionRuntime,
        collectionProject || null,
      );
      await skillSources.reconcileInstalls(projects.list.map((project) => project.path));
      announcement = `${collectionOperation} completed for ${result.completed.length} skill(s).`;
      collectionInstallOpen = false;
    } catch (error) {
      announcement = String(error);
    }
  }

  async function createSkillDraft(): Promise<void> {
    const succeeded = await skillSources.createDraft(
      creatorName.trim(),
      creatorDescription.trim(),
      creatorType,
      creatorGroup.split("/").map((value) => value.trim()).filter(Boolean),
      creatorTags.split(",").map((value) => value.trim()).filter(Boolean),
      creatorBody,
    );
    if (succeeded) {
      creatorOpen = false;
      announcement = i18n.t("skills.creatorSubmitted");
    } else {
      folderError = skillSources.addError ?? i18n.t("skills.creatorFailed");
    }
  }

  async function openEditor(): Promise<void> {
    if (!selected) return;
    editorLoading = true;
    try {
      editorText = await skillSources.readSkillText(selected.pkg);
      editorOpen = true;
    } catch (error) {
      folderError = String(error);
    } finally {
      editorLoading = false;
    }
  }

  async function submitEditDraft(): Promise<void> {
    if (!selected) return;
    const succeeded = await skillSources.editDraft(selected.pkg, editorText);
    if (succeeded) {
      editorOpen = false;
      announcement = i18n.t("skills.editorSubmitted");
    } else {
      folderError = skillSources.addError ?? i18n.t("skills.creatorFailed");
    }
  }

  function isInstalled(pkg: SkillPackageResult): boolean {
    return packageIsInstalled(pkg, skillSources.installed);
  }

  function formatBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
  }

  function packageStatus(pkg: SkillPackageResult): string {
    if (requiresTrust(pkg)) return i18n.t("skills.trustRequired");
    if (trustedScripts(pkg)) return i18n.t("skills.trusted");
    return pkg.installable ? i18n.t("skills.ready") : i18n.t("skills.rejected");
  }

  function approvalLabel(action: SkillApprovalAction): string {
    if (action.action === "install") return `Install ${action.relativePath} in ${action.runtime}`;
    if (action.action === "folderCreate") return `Create folder ${action.path}`;
    if (action.action === "folderRename") return `Rename ${action.path} to ${action.newName}`;
    if (action.action === "folderMove") return `Move ${action.path}`;
    if (action.action === "folderDelete") return `Delete folder ${action.path}`;
    if (action.action === "folderAssign") return `Assign ${action.relativePath} to ${action.folderPath ?? "no folder"}`;
    if (action.action === "collectionDelete") return `Delete collection ${action.name}`;
    if (action.action === "smartFolderDelete") return `Delete smart folder ${action.name}`;
    if (action.action === "profileDelete") return `Delete profile ${action.name}`;
    if (action.action === "updatePolicySet") return `Set ${action.relativePath} policy to ${action.policy}`;
    if (action.action === "publisherTrustSet") return `${action.revoked ? "Revoke" : "Trust"} publisher ${action.name}`;
    if (action.action === "draftPublish") return `Publish Skill draft ${action.id}`;
    if (action.action === "batchCollection") return `${action.operation} collection ${action.collectionName} in ${action.runtime}`;
    return `Roll back ${action.relativePath} in ${action.runtime}`;
  }

  async function grantTrust(): Promise<void> {
    if (!trustCandidate) return;
    const name = trustCandidate.pkg.name ?? trustCandidate.pkg.relativePath;
    const succeeded = await skillSources.grantTrust(trustCandidate.pkg);
    announcement = succeeded
      ? i18n.t("skills.trustSucceeded", { name })
      : skillSources.addError ?? i18n.t("skills.trustFailed");
    if (succeeded) trustCandidate = null;
  }

  async function revokeTrust(pkg: SkillPackageResult): Promise<void> {
    const name = pkg.name ?? pkg.relativePath;
    const succeeded = await skillSources.revokeTrust(pkg);
    announcement = succeeded
      ? i18n.t("skills.trustRevoked", { name })
      : skillSources.addError ?? i18n.t("skills.trustFailed");
  }

  function selectPackage(view: PackageView): void {
    selectedKey = skillSources.packageKey(view.pkg);
    detailTab = "overview";
    void skillSources.touchRecent(view.pkg);
  }

  function installedAt(pkg: SkillPackageResult, runtime: string, projectPath: string | null): InstalledSkill | undefined {
    return skillSources.installed.find((record) =>
      record.runtime === runtime
      && (record.projectPath ?? null) === projectPath
      && record.sourceId === pkg.sourceId
      && record.relativePath === pkg.relativePath,
    );
  }

  function deploymentCell(column: DeploymentColumn, row: DeploymentRow): DeploymentCell {
    if (!selected) {
      return { state: "off", disabled: true, title: i18n.t("skills.select"), ariaLabel: i18n.t("skills.select") };
    }
    const projectPath = row.kind === "global" ? null : row.path;
    const runtime = column.id as "claudeCode" | "codex";
    const record = installedAt(selected.pkg, runtime, projectPath);
    const key = skillSources.installKey(selected.pkg, runtime, projectPath);
    const where = row.kind === "global" ? "globally" : `in ${row.label}`;
    const state = record?.state ?? "missing";
    const canInstall = !record || state === "missing";
    return {
      state: record && state !== "missing" ? (state === "current" ? "on" : "partial") : "off",
      busy: skillSources.installing[key] === true,
      disabled: !selected.pkg.installable || !canInstall,
      title: record ? `${column.label}: ${state}` : `Install ${column.label} ${where}`,
      ariaLabel: canInstall ? `Install ${selected.pkg.name} for ${column.label} ${where}` : `${column.label} ${where}: ${state}`,
    };
  }

  async function installSelected(column: DeploymentColumn, row: DeploymentRow): Promise<void> {
    if (!selected) return;
    const projectPath = row.kind === "global" ? null : row.path;
    const runtime = column.id as "claudeCode" | "codex";
    try {
      installPlan = await skillInstallPlan(selected.pkg.sourceId, selected.pkg.relativePath, runtime, projectPath);
      plannedInstall = { pkg: selected.pkg, runtime, projectPath, destination: column.label };
    } catch (error) {
      announcement = String(error);
    }
  }

  async function confirmPlannedInstall(): Promise<void> {
    if (!plannedInstall || !installPlan || installPlan.blockers.length > 0) return;
    const target = plannedInstall;
    const succeeded = await skillSources.installPackage(
      target.pkg,
      target.runtime,
      target.projectPath,
      projects.list.map((project) => project.path),
    );
    announcement = succeeded
      ? i18n.t("skills.installSucceeded", { name: target.pkg.name ?? target.pkg.relativePath, destination: target.destination })
      : skillSources.installErrors[skillSources.installKey(target.pkg, target.runtime, target.projectPath)] ?? i18n.t("skills.installFailed");
    if (succeeded) {
      installPlan = null;
      plannedInstall = null;
    }
  }

  function lifecycleKey(installed: InstalledSkill): string {
    return `${installed.sourceId}\0${installed.relativePath}\0${installed.runtime}\0${installed.projectPath ?? ""}`;
  }

  function destinationLabel(installed: InstalledSkill): string {
    const runtime = installed.runtime === "claudeCode" ? "Claude Code" : "Codex";
    if (!installed.projectPath) return `${runtime} · Global`;
    return `${runtime} · ${projects.list.find((project) => project.path === installed.projectPath)?.label ?? installed.projectPath}`;
  }

  async function runLifecycle(
    action: "update" | "disable" | "enable" | "uninstall",
    installed: InstalledSkill,
  ): Promise<void> {
    const succeeded = await skillSources.lifecycle(
      action,
      installed,
      projects.list.map((project) => project.path),
    );
    announcement = succeeded
      ? `${installed.name} ${action === "disable" ? "disabled" : action === "enable" ? "enabled" : action === "uninstall" ? "uninstalled" : "updated"}.`
      : skillSources.installErrors[lifecycleKey(installed)] ?? `Could not ${action} skill.`;
    if (succeeded && action === "uninstall") uninstallCandidate = null;
  }

  async function addProjectDestination(): Promise<void> {
    const added = await projects.addViaPicker();
    if (added) {
      await skillSources.reconcileInstalls(projects.list.map((project) => project.path));
    }
  }

  async function chooseLocalFolder(): Promise<void> {
    if (skillSources.loading || skillSources.adding) return;
    const picked = await openDialog({
      directory: true,
      multiple: false,
      title: i18n.t("skills.chooseSourceFolder"),
    });
    if (typeof picked !== "string") return;

    localPath = picked;
    const outcome = await skillSources.addLocal(picked);
    if (outcome.registrationSucceeded && outcome.initialRefreshSucceeded) {
      const source = skillSources.sources.find(
        (candidate) => candidate.kind.kind === "local" && candidate.kind.root === picked,
      );
      announcement = i18n.t("skills.sourceAdded", { count: packageCount(source ? skillSources.results[source.id] : undefined) });
      localPath = "";
    }
  }

  async function refresh(source: SkillSource): Promise<void> {
    const succeeded = await skillSources.refresh(source.id);
    if (succeeded) {
      announcement = i18n.t("skills.sourceRefreshed", { source: sourceLabel(source), count: packageCount(skillSources.results[source.id]) });
    }
  }

  async function addGithub(): Promise<void> {
    const outcome = await skillSources.addGithub(
      githubRepository,
      githubRef,
      githubSubdirectory,
    );
    githubRegistrationRejected = !outcome.registrationSucceeded;
    if (outcome.registrationSucceeded && outcome.initialRefreshSucceeded) {
      announcement = i18n.t("skills.githubAdded");
      githubRepository = "";
      githubRef = "";
      githubSubdirectory = "";
      githubRegistrationRejected = false;
    }
  }

  async function removeSource(): Promise<void> {
    if (!removeCandidate) return;
    const source = removeCandidate;
    if (await skillSources.remove(source.id)) {
      await skillSources.reconcileInstalls(projects.list.map((project) => project.path));
      announcement = i18n.t("skills.sourceRemoved", { source: sourceLabel(source) });
      if (sourceFilter === source.id) sourceFilter = "all";
      removeCandidate = null;
    }
  }

  async function publishDraft(draft: SkillDraft): Promise<void> {
    const succeeded = await skillSources.publishDraft(draft);
    announcement = succeeded
      ? `${draft.validation.name ?? draft.id} published.`
      : skillSources.addError ?? "Could not publish draft.";
  }

  async function rejectDraft(): Promise<void> {
    if (!rejectDraftCandidate) return;
    const draft = rejectDraftCandidate;
    const succeeded = await skillSources.rejectDraft(draft);
    announcement = succeeded
      ? `${draft.validation.name ?? draft.id} rejected.`
      : skillSources.addError ?? "Could not reject draft.";
    if (succeeded) rejectDraftCandidate = null;
  }
</script>

{#snippet packageRow(view: PackageView)}
  {@const key = skillSources.packageKey(view.pkg)}
  <li class="package-row">
    <button class:selected={selectedKey === key} aria-pressed={selectedKey === key} onclick={() => selectPackage(view)}>
      <span class="row-top">
        <strong>{view.pkg.name ?? view.pkg.relativePath}</strong>
        <span class:ready={view.pkg.installable} class:rejected={!view.pkg.installable} class="status-badge">
          {packageStatus(view.pkg)}
        </span>
      </span>
      <span class="description">{view.pkg.description ?? i18n.t("skills.packageValidationFailed")}</span>
      <span class="provenance">{typeLabel(view.pkg.skillType)} · {sourceKind(view.source)}</span>
    </button>
  </li>
{/snippet}

{#snippet groupNode(node: SkillGroupNode, depth: number)}
  <li class="skill-group">
    <details open>
      <summary
        class:active={libraryFilter === `taxonomy:${node.key}`}
        aria-current={libraryFilter === `taxonomy:${node.key}` ? "page" : undefined}
        style:--tree-depth={depth}
        onclick={() => (libraryFilter = `taxonomy:${node.key}`)}
      >
        <span>{node.label}</span>
        <span>{groupCount(node)}</span>
      </summary>
      <ul>
        {#each node.children as child (child.key)}
          {@render groupNode(child, depth + 1)}
        {/each}
      </ul>
    </details>
  </li>
{/snippet}

{#snippet personalFolderNode(node: PersonalFolderNode, depth: number)}
  <li class="personal-folder">
    <button
      class:active={libraryFilter === `personal:${node.path}`}
      aria-pressed={libraryFilter === `personal:${node.path}`}
      style:--tree-depth={depth}
      onclick={() => (libraryFilter = `personal:${node.path}`)}
    >
      <span>{node.label}</span>
      <span>{personalFolderCount(node.path)}</span>
    </button>
    {#if node.children.length > 0}
      <ul>
        {#each node.children as child (child.path)}
          {@render personalFolderNode(child, depth + 1)}
        {/each}
      </ul>
    {/if}
  </li>
{/snippet}

<div class="workspace">
  <header>
    <div>
      <h2>{i18n.t("skills.title")}</h2>
      <p>{i18n.t("skills.subtitle")}</p>
    </div>
    <div class="header-actions">
    <Button size="sm" onclick={() => (creatorOpen = true)}>{i18n.t("skills.createSkill")}</Button>
    <details class="source-manager" bind:this={sourceManager}>
      <summary>{i18n.t("skills.manageSources")} <span>{skillSources.sources.length}</span></summary>
      <div class="source-popover">
        <section class="local-add" aria-label={i18n.t("skills.addLocalAria")} aria-busy={skillSources.loading || skillSources.adding}>
          {#if localPath}<span class="picked" title={localPath}>{localPath}</span>{/if}
          <Button
            size="sm"
            loading={skillSources.adding}
            disabled={skillSources.loading || skillSources.adding}
            ariaLabel={i18n.t("skills.chooseLocal")}
            onclick={() => void chooseLocalFolder()}
          >
            {#snippet icon()}<FolderPlus size={16} />{/snippet}
            {i18n.t("skills.addLocal")}
          </Button>
        </section>

        <form
          class="github-add"
          aria-label={i18n.t("skills.addGithubAria")}
          aria-busy={skillSources.adding}
          onsubmit={(event) => {
            event.preventDefault();
            void addGithub();
          }}
        >
          <Input bind:value={githubRepository} placeholder="https://github.com/owner/repository" ariaLabel={i18n.t("skills.githubRepository")} ariaDescribedby="github-source-help skill-source-add-error" invalid={githubRegistrationRejected} disabled={skillSources.adding} />
          <div class="github-options">
            <Input bind:value={githubRef} placeholder={i18n.t("skills.gitRef")} ariaLabel={i18n.t("skills.gitRef")} ariaDescribedby="github-source-help skill-source-add-error" invalid={githubRegistrationRejected && githubRef.trim().length > 0} disabled={skillSources.adding} />
            <Input bind:value={githubSubdirectory} placeholder={i18n.t("skills.subdirectory")} ariaLabel={i18n.t("skills.subdirectory")} ariaDescribedby="github-source-help skill-source-add-error" invalid={githubRegistrationRejected && githubSubdirectory.trim().length > 0} disabled={skillSources.adding} />
          </div>
          <Button type="submit" size="sm" loading={skillSources.adding} disabled={skillSources.loading || skillSources.adding} ariaLabel={i18n.t("skills.addGithub")}>{i18n.t("skills.addGithub")}</Button>
          <p id="github-source-help">{i18n.t("skills.githubHelp")}</p>
        </form>

        {#if skillSources.addError}
          <div id="skill-source-add-error" class="alert" role="alert">{skillSources.addError}</div>
        {/if}

        <div class="source-list">
          {#each skillSources.sources as source (source.id)}
            {@const result = skillSources.results[source.id]}
            {@const sourceError = skillSources.refreshErrors[source.id] ?? skillSources.removeErrors[source.id]}
            <article class="source" aria-busy={skillSources.isRefreshing(source.id) || skillSources.isRemoving(source.id)}>
              <div>
                <strong title={sourceLabel(source)}>{sourceLabel(source)}</strong>
                <span>{sourceKind(source)} · {i18n.t("skills.packages", { count: result?.packages.length ?? 0 })}</span>
              </div>
              <div class="source-actions">
                <Button size="sm" loading={skillSources.isRefreshing(source.id)} disabled={skillSources.isRemoving(source.id)} ariaLabel={`${i18n.t("skills.refresh")} ${sourceLabel(source)}`} onclick={() => void refresh(source)}>{i18n.t("skills.refresh")}</Button>
                <Button size="sm" variant="danger" disabled={skillSources.isRefreshing(source.id) || skillSources.isRemoving(source.id)} ariaLabel={`${i18n.t("skills.remove")} ${sourceLabel(source)}`} onclick={() => (removeCandidate = source)}>{i18n.t("skills.remove")}</Button>
              </div>
              {#if sourceError}<div class="alert" role="alert">{sourceError}</div>{/if}
              {#if result && result.errors.length > 0}
                <ul class="diagnostics" aria-label={`Errors for ${sourceLabel(source)}`}>
                  {#each result.errors as error}
                    <li><code>{error.code}</code> · {error.message}</li>
                  {/each}
                </ul>
              {/if}
            </article>
          {/each}
        </div>
      </div>
    </details>
    <details class="draft-inbox" bind:this={approvalInbox}>
      <summary>{i18n.t("skills.approvalInbox")} <span>{pendingDrafts.length + pendingApprovals.length}</span></summary>
      <div class="draft-popover" aria-label={i18n.t("skills.draftInboxAria")}>
        {#if pendingDrafts.length === 0 && pendingApprovals.length === 0}
          <p class="quiet">{i18n.t("skills.noDrafts")}</p>
        {:else}
          {#each pendingApprovals as approval (approval.id)}
            <article class="draft">
              <div>
                <strong>{approvalLabel(approval.request)}</strong>
                <span>{approval.requestedBy} · {new Date(approval.submittedAt).toLocaleString()}</span>
              </div>
              <div class="draft-actions">
                <Button size="sm" onclick={() => void skillSources.approveRequest(approval.id)}>{i18n.t("skills.approve")}</Button>
                <Button size="sm" variant="danger" onclick={() => void skillSources.rejectRequest(approval.id)}>{i18n.t("skills.reject")}</Button>
              </div>
              {#if approval.result}<p class="draft-error">{approval.result}</p>{/if}
            </article>
          {/each}
          {#each pendingDrafts as draft (draft.id)}
            <article class="draft">
              <div>
                <strong>{draft.validation.name ?? draft.id}</strong>
                <span>{draft.files.length} files · {draft.treeHash.slice(0, 12)}</span>
              </div>
              <div class="draft-actions">
                <Button
                  size="sm"
                  disabled={!draft.validation.installable}
                  ariaLabel={`${i18n.t("skills.publishDraft")} ${draft.validation.name ?? draft.id}`}
                  onclick={() => void publishDraft(draft)}
                >{i18n.t("skills.publishDraft")}</Button>
                <Button
                  size="sm"
                  variant="danger"
                  ariaLabel={`${i18n.t("skills.rejectDraft")} ${draft.validation.name ?? draft.id}`}
                  onclick={() => (rejectDraftCandidate = draft)}
                >{i18n.t("skills.rejectDraft")}</Button>
              </div>
              {#if !draft.validation.installable}
                <p class="draft-error">{i18n.t("skills.invalidDraft")}</p>
                <ul class="diagnostics">
                  {#each draft.validation.errors as error}
                    <li><code>{error.path}</code> · {error.message}</li>
                  {/each}
                </ul>
              {/if}
            </article>
          {/each}
        {/if}
      </div>
    </details>
    </div>
  </header>

  <div class="announcement" role="status" aria-live="polite">{announcement}</div>
  <div class="announcement" role="status" aria-live="polite">
    {i18n.t("skills.packagesShown", { count: filtered.length })}
  </div>

  {#if unavailableInstalls.length > 0}
    <aside class="unavailable" aria-label={i18n.t("skills.unavailableAria")}>
      <strong>{i18n.t("skills.sourceUnavailable")}</strong>
      <span>{i18n.t("skills.sourceUnavailableBody", { count: unavailableInstalls.length })}</span>
      {#each unavailableInstalls as installed (lifecycleKey(installed))}
        <div>
          <code>{installed.name} · {destinationLabel(installed)}</code>
          <Button size="sm" ariaLabel={`${i18n.t("skills.disable")} ${destinationLabel(installed)}`} onclick={() => void runLifecycle("disable", installed)}>{i18n.t("skills.disable")}</Button>
          <Button size="sm" variant="danger" ariaLabel={`${i18n.t("skills.uninstall")} ${destinationLabel(installed)}`} onclick={() => (uninstallCandidate = installed)}>{i18n.t("skills.uninstall")}</Button>
        </div>
      {/each}
    </aside>
  {/if}

  {#if skillSources.loading && skillSources.sources.length === 0}
    <LoadingState rows={6} label={i18n.t("skills.loading")} />
  {:else if skillSources.sources.length === 0}
    <EmptyState title={i18n.t("skills.noSources")} body={i18n.t("skills.noSourcesBody")}>
      {#snippet icon()}<FolderPlus size={28} />{/snippet}
    </EmptyState>
  {:else}
    <div class="browser">
      <nav class="library-pane" aria-label={i18n.t("skills.library")}>
        <div class="pane-heading">
          <span>{i18n.t("skills.library")}</span>
          <span class="pane-tools">
            <span>{packages.length}</span>
            <button aria-label={i18n.t("skills.organize")} onclick={() => (organizeOpen = true)}>
              <Settings2 size={15} />
            </button>
          </span>
        </div>
        <div class="quick-filters">
          {#each [
            ["all", i18n.t("skills.allSkills"), packages.length],
            ["installed", i18n.t("skills.installed"), installedCount],
            ["trusted", i18n.t("skills.trustedScripts"), trustedCount],
            ["review", i18n.t("skills.needsReview"), reviewCount],
            ["favorites", i18n.t("skills.favorites"), skillSources.folderState.favorites.length],
            ["recent", i18n.t("skills.recent"), skillSources.folderState.recent.length],
            ["recommendations", "Recommendations", recommendationCount],
            ["duplicates", "Duplicates", duplicateCount],
            ["cleanup", "Cleanup", cleanupCount],
          ] as item}
            <button
              class:active={libraryFilter === item[0]}
              aria-pressed={libraryFilter === item[0]}
              onclick={() => (libraryFilter = item[0] as string)}
            >
              <span>{item[1]}</span><span>{item[2]}</span>
            </button>
          {/each}
        </div>
        <div class="tree-heading folder-heading">
          <span>{i18n.t("skills.myFolders")}</span>
          <button class="add-folder" aria-label={i18n.t("skills.newFolder")} onclick={() => openFolderModal()}>
            <FolderPlus size={15} />
          </button>
        </div>
        {#if personalFolderTree.length === 0}
          <button class="empty-folder-action" onclick={() => openFolderModal()}>{i18n.t("skills.createFolder")}</button>
        {:else}
          <ul class="personal-folder-tree">
            {#each personalFolderTree as node (node.path)}
              {@render personalFolderNode(node, 0)}
            {/each}
          </ul>
        {/if}
        {#if skillSources.folderState.collections.length > 0}
          <div class="tree-heading">{i18n.t("skills.collections")}</div>
          <ul class="named-views">
            {#each skillSources.folderState.collections as collection (collection.name)}
              <li>
                <button
                  class:active={libraryFilter === `collection:${collection.name}`}
                  aria-pressed={libraryFilter === `collection:${collection.name}`}
                  onclick={() => (libraryFilter = `collection:${collection.name}`)}
                ><span>{collection.name}</span><span>{collection.skills.length}</span></button>
              </li>
            {/each}
          </ul>
        {/if}
        {#if skillSources.folderState.smartFolders.length > 0}
          <div class="tree-heading">{i18n.t("skills.smartFolders")}</div>
          <ul class="named-views">
            {#each skillSources.folderState.smartFolders as smartFolder (smartFolder.name)}
              <li>
                <button
                  class:active={libraryFilter === `smart:${smartFolder.name}`}
                  aria-pressed={libraryFilter === `smart:${smartFolder.name}`}
                  onclick={() => (libraryFilter = `smart:${smartFolder.name}`)}
                ><span>{smartFolder.name}</span></button>
              </li>
            {/each}
          </ul>
        {/if}
        <div class="tree-heading">{i18n.t("skills.typesAndFolders")}</div>
        <ul class="taxonomy-tree">
          {#each grouped as node (node.key)}
            {@render groupNode(node, 0)}
          {/each}
        </ul>
      </nav>

      <section class="package-list" aria-label={i18n.t("skills.packagesAria")}>
        <div class="pane-heading">
          <span>{libraryTitle}</span>
          <span class="pane-tools">
            <span>{filtered.length}</span>
            {#if libraryFilter.startsWith("collection:") && filtered.length > 0}
              <Button size="sm" onclick={() => (collectionInstallOpen = true)}>{i18n.t("skills.manageCollection")}</Button>
            {/if}
          </span>
        </div>
        <div class="filters">
          <label class="search">
            <Search size={15} aria-hidden="true" />
            <input bind:this={searchInput} bind:value={query} type="search" placeholder={i18n.t("skills.search")} aria-label={i18n.t("skills.search")} />
          </label>
          <div class="filter-row">
            <div class="segments" aria-label={i18n.t("skills.validationStatus")}>
              {#each ["all", "ready", "rejected"] as status}
                <button class:active={statusFilter === status} aria-pressed={statusFilter === status} onclick={() => (statusFilter = status as StatusFilter)}>
                  {statusLabel(status as StatusFilter)}
                </button>
              {/each}
            </div>
            <select bind:value={sourceFilter} aria-label={i18n.t("skills.filterSource")}>
              <option value="all">{i18n.t("skills.allSources")}</option>
              {#each skillSources.sources as source (source.id)}
                <option value={source.id}>{sourceLabel(source)}</option>
              {/each}
            </select>
            <select bind:value={sortOrder} aria-label={i18n.t("skills.sortBy")}>
              <option value="name">{i18n.t("skills.sortName")}</option>
              <option value="type">{i18n.t("skills.sortType")}</option>
              <option value="source">{i18n.t("skills.sortSource")}</option>
            </select>
          </div>
        </div>

        <div class="result-count" aria-hidden="true">{i18n.t("skills.results", { count: filtered.length })}</div>
        {#if filtered.length === 0}
          <EmptyState title={i18n.t("skills.noMatches")} body={i18n.t("skills.noMatchesBody", { query: query || statusLabel(statusFilter) })} />
        {:else}
          <ul class="results" aria-label={i18n.t("skills.resultsAria")}>
            {#each filtered as view (skillSources.packageKey(view.pkg))}
              {@render packageRow(view)}
            {/each}
          </ul>
        {/if}
      </section>

      <section class="detail" aria-label={i18n.t("skills.detailAria")}>
        {#if selected}
          <div class="detail-scroll">
            <div class="detail-title">
              <div>
                <span class="eyebrow">{selected.pkg.installable ? i18n.t("skills.readySkill") : i18n.t("skills.rejectedPackage")}</span>
                <h3>{selected.pkg.name ?? selected.pkg.relativePath}</h3>
                <p>{selected.pkg.description ?? i18n.t("skills.invalidMetadata")}</p>
              </div>
              <div class="detail-actions">
                <Button size="sm" disabled={!selected.pkg.installable} onclick={() => (agentCreatorSkill = { sourceId: selected.pkg.sourceId, relativePath: selected.pkg.relativePath })}>{i18n.t("skills.createAgent")}</Button>
                <Button size="sm" loading={editorLoading} onclick={() => void openEditor()}>{i18n.t("skills.editAsDraft")}</Button>
                <button
                  class="favorite-button"
                  class:active={skillSources.isFavorite(selected.pkg)}
                  aria-label={skillSources.isFavorite(selected.pkg) ? i18n.t("skills.removeFavorite") : i18n.t("skills.addFavorite")}
                  aria-pressed={skillSources.isFavorite(selected.pkg)}
                  onclick={() => void skillSources.setFavorite(selected.pkg, !skillSources.isFavorite(selected.pkg))}
                ><Star size={17} fill={skillSources.isFavorite(selected.pkg) ? "currentColor" : "none"} /></button>
                <span class:ready={selected.pkg.installable} class:rejected={!selected.pkg.installable} class="status-badge">
                  {selected.pkg.installable ? i18n.t("skills.validationPassed") : i18n.t("skills.validationFailed")}
                </span>
              </div>
            </div>

            <div class="detail-tabs" role="tablist" aria-label={i18n.t("skills.detailSections")}>
              {#each ["overview", "files", "security"] as tab}
                <button
                  role="tab"
                  aria-selected={detailTab === tab}
                  class:active={detailTab === tab}
                  onclick={() => (detailTab = tab as DetailTab)}
                >{i18n.t(`skills.${tab}` as "skills.overview" | "skills.files" | "skills.security")}</button>
              {/each}
            </div>

            {#if detailTab === "overview"}
            <section class="detail-section folder-assignment">
              <div class="section-title-row">
                <h4>{i18n.t("skills.myFolder")}</h4>
                <Button size="sm" onclick={() => openFolderModal(skillSources.folderFor(selected.pkg) ?? "")}>
                  {i18n.t("skills.newFolder")}
                </Button>
              </div>
              <select
                aria-label={i18n.t("skills.assignFolder")}
                value={skillSources.folderFor(selected.pkg) ?? ""}
                onchange={(event) => void assignSelectedFolder(event.currentTarget.value)}
              >
                <option value="">{i18n.t("skills.noFolder")}</option>
                {#each personalFolders.toSorted((left, right) => left.localeCompare(right)) as folder (folder)}
                  <option value={folder}>{folder}</option>
                {/each}
              </select>
              {#if folderError === i18n.t("skills.folderSaveError")}
                <p class="alert" role="alert">{folderError}</p>
              {/if}
            </section>
            <section class="detail-section">
              <h4>{i18n.t("skills.lifecyclePolicy")}</h4>
              <select
                class="policy-select"
                aria-label={i18n.t("skills.lifecyclePolicy")}
                value={skillSources.updatePolicy(selected.pkg)}
                onchange={(event) => void skillSources.setUpdatePolicy(selected.pkg, event.currentTarget.value as SkillUpdatePolicy)}
              >
                <option value="notify">{i18n.t("skills.policyNotify")}</option>
                <option value="autoTrusted">{i18n.t("skills.policyAutoTrusted")}</option>
                <option value="pin">{i18n.t("skills.policyPin")}</option>
                <option value="reviewScripts">{i18n.t("skills.policyReviewScripts")}</option>
              </select>
              {#if selected.pkg.dependencies.length > 0 || selected.pkg.recommendedSkills.length > 0}
                <dl>
                  <div><dt>{i18n.t("skills.dependencies")}</dt><dd>{selected.pkg.dependencies.join(", ") || i18n.t("skills.none")}</dd></div>
                  <div><dt>{i18n.t("skills.recommendedSkills")}</dt><dd>{selected.pkg.recommendedSkills.join(", ") || i18n.t("skills.none")}</dd></div>
                </dl>
              {/if}
              {#if missingDependencies(selected.pkg).length > 0}
                <p class="warning">{i18n.t("skills.missingDependencies", { names: missingDependencies(selected.pkg).join(", ") })}</p>
              {/if}
              {#if packageConflicts(selected.pkg).length > 0}
                <p class="warning">{i18n.t("skills.nameConflict", { count: packageConflicts(selected.pkg).length })}</p>
              {/if}
            </section>
            <section class="detail-section">
              <h4>{i18n.t("skills.provenance")}</h4>
              <dl>
                <div><dt>{i18n.t("skills.type")}</dt><dd>{typeLabel(selected.pkg.skillType)}</dd></div>
                <div><dt>{i18n.t("skills.group")}</dt><dd>{selected.pkg.group.length ? selected.pkg.group.map(taxonomyLabel).join(" / ") : i18n.t("skills.none")}</dd></div>
                <div><dt>{i18n.t("skills.tags")}</dt><dd>{selected.pkg.tags.length ? selected.pkg.tags.join(", ") : i18n.t("skills.none")}</dd></div>
                <div><dt>{i18n.t("skills.source")}</dt><dd>{sourceKind(selected.source)}</dd></div>
                <div><dt>{i18n.t("skills.identity")}</dt><dd title={sourceLabel(selected.source)}>{sourceLabel(selected.source)}</dd></div>
                {#if selected.source.kind.kind === "github"}
                  <div><dt>{i18n.t("skills.ref")}</dt><dd>{selected.source.kind.gitRef ?? i18n.t("skills.defaultBranch")}</dd></div>
                  <div><dt>{i18n.t("skills.subdirectoryLabel")}</dt><dd>{selected.source.kind.subdirectory ?? i18n.t("skills.repositoryRoot")}</dd></div>
                {/if}
                <div><dt>{i18n.t("skills.packagePath")}</dt><dd>{selected.pkg.relativePath || "."}</dd></div>
                <div><dt>{i18n.t("skills.version")}</dt><dd>{selected.pkg.version ?? i18n.t("skills.unversioned")} · {selected.pkg.channel}</dd></div>
                {#if selected.pkg.publisher}
                  <div><dt>{i18n.t("skills.publisher")}</dt><dd>{selected.pkg.publisher} · {selected.pkg.publisherVerified ? i18n.t("skills.signatureVerified") : i18n.t("skills.signatureInvalid")}</dd></div>
                {/if}
              </dl>
              {#if selected.pkg.changelog}<p class="section-help">{selected.pkg.changelog}</p>{/if}
              {#if packageConflicts(selected.pkg).length > 0}
                <Button size="sm" onclick={() => void skillSources.setPreferredSource(selected.pkg)}>{i18n.t("skills.preferSource")}</Button>
              {/if}
            </section>
            {/if}

            {#if detailTab === "security"}
            <section class="detail-section">
              <h4>{i18n.t("skills.validation")}</h4>
              {#if selected.pkg.errors.length === 0}
                <p class="healthy">{i18n.t("skills.noValidationIssues")}</p>
              {:else}
                <ul class="diagnostics">
                  {#each selected.pkg.errors as error}
                    <li><code>{error.code}</code> · {error.path || "."}: {error.message}</li>
                  {/each}
                </ul>
              {/if}
              {#if selected.pkg.validationResults.length > 0}
                <ul class="quality-list">
                  {#each selected.pkg.validationResults as result (result)}<li>{result}</li>{/each}
                </ul>
              {/if}
              {#if selected.pkg.publisher && selected.pkg.publisherKey && selected.pkg.publisherVerified}
                {@const publisherTrust = skillSources.folderState.publisherTrust.find((trust) => trust.publicKey === selected.pkg.publisherKey)}
                <p class:healthy={publisherTrust?.trusted} class:warning={publisherTrust?.revoked}>
                  {publisherTrust?.revoked ? i18n.t("skills.publisherRevoked") : publisherTrust?.trusted ? i18n.t("skills.publisherTrusted") : i18n.t("skills.publisherVerifiedUntrusted")}
                </p>
                <div class="button-row">
                  <Button size="sm" onclick={() => void skillSources.setPublisherTrust({ name: selected.pkg.publisher!, publicKey: selected.pkg.publisherKey!, trusted: true, revoked: false })}>{i18n.t("skills.trustPublisher")}</Button>
                  <Button size="sm" variant="danger" onclick={() => void skillSources.setPublisherTrust({ name: selected.pkg.publisher!, publicKey: selected.pkg.publisherKey!, trusted: false, revoked: true })}>{i18n.t("skills.revokePublisher")}</Button>
                </div>
              {/if}
              {#if requiresTrust(selected.pkg)}
                <Button
                  size="sm"
                  loading={skillSources.trusting[skillSources.packageKey(selected.pkg)] === true}
                  ariaLabel={`${i18n.t("skills.trustExactVersion")} ${selected.pkg.name ?? selected.pkg.relativePath}`}
                  onclick={() => (trustCandidate = selected)}
                >{i18n.t("skills.trustExactVersion")}</Button>
              {:else if trustedScripts(selected.pkg)}
                <Button
                  size="sm"
                  variant="secondary"
                  loading={skillSources.trusting[skillSources.packageKey(selected.pkg)] === true}
                  ariaLabel={`${i18n.t("skills.revokeTrust")} ${selected.pkg.name ?? selected.pkg.relativePath}`}
                  onclick={() => void revokeTrust(selected.pkg)}
                >{i18n.t("skills.revokeTrust")}</Button>
              {/if}
            </section>
            <section class="detail-section">
              <h4>{i18n.t("skills.permissionsManifest")}</h4>
              {#if selected.pkg.permissions.length === 0}
                <p class="healthy">{i18n.t("skills.noElevatedPermissions")}</p>
              {:else}
                <ul class="permission-list">
                  {#each selected.pkg.permissions as permission (permission)}
                    <li>{permission}</li>
                  {/each}
                </ul>
              {/if}
              <h4>{i18n.t("skills.qualityScore", { score: selected.pkg.qualityScore })}</h4>
              <ul class="quality-list">
                {#each selected.pkg.qualityChecks as check (check)}
                  <li>{check}</li>
                {/each}
              </ul>
            </section>
            {/if}

            {#if detailTab === "overview"}
            <section class="detail-section">
              <h4>{i18n.t("skills.destinations")}</h4>
              <p class="section-help">{i18n.t("skills.destinationsHelp")}</p>
              {#if !selected.pkg.installable}
                <p class="quiet">{i18n.t("skills.rejectedNoDestinations")}</p>
              {:else}
                <DeploymentTargetGrid
                  columns={deploymentColumns}
                  rows={deploymentRows}
                  cell={deploymentCell}
                  onToggle={(column, row) => void installSelected(column, row)}
                  notApplicable={() => i18n.t("skills.runtimeUnsupported")}
                />
                {#if projects.list.length === 0}
                  <Button size="sm" ariaLabel={i18n.t("skills.addProjectDestination")} onclick={() => void addProjectDestination()}>{i18n.t("skills.addProjectDestination")}</Button>
                {/if}
                {#if selectedInstalls.length > 0}
                  <ul class="skill-installs" aria-label={i18n.t("skills.managedInstalls")}>
                    {#each selectedInstalls as installed (lifecycleKey(installed))}
                      {@const busy = skillSources.installing[lifecycleKey(installed)] === true}
                      <li>
                        <div>
                          <strong>{destinationLabel(installed)}</strong>
                          <span class="install-state">{installed.state}</span>
                          <code title={installed.path}>{installed.path}</code>
                        </div>
                        <div class="lifecycle-actions">
                          {#if installed.state === "outdated" || installed.state === "missing"}
                            <Button size="sm" loading={busy} ariaLabel={`${i18n.t("skills.update")} ${destinationLabel(installed)}`} onclick={() => void runLifecycle("update", installed)}>{i18n.t("skills.update")}</Button>
                          {/if}
                          {#if ["current", "outdated", "sourceUnavailable"].includes(installed.state)}
                            <Button size="sm" loading={busy} ariaLabel={`${i18n.t("skills.disable")} ${destinationLabel(installed)}`} onclick={() => void runLifecycle("disable", installed)}>{i18n.t("skills.disable")}</Button>
                          {:else if installed.state === "disabled"}
                            <Button size="sm" loading={busy} ariaLabel={`${i18n.t("skills.enable")} ${destinationLabel(installed)}`} onclick={() => void runLifecycle("enable", installed)}>{i18n.t("skills.enable")}</Button>
                          {/if}
                          {#if installed.tracked}
                            <Button size="sm" variant="danger" disabled={busy} ariaLabel={`${i18n.t("skills.uninstall")} ${destinationLabel(installed)}`} onclick={() => (uninstallCandidate = installed)}>{i18n.t("skills.uninstall")}</Button>
                          {/if}
                        </div>
                        {#if skillSources.installErrors[lifecycleKey(installed)]}
                          <div class="alert" role="alert">{skillSources.installErrors[lifecycleKey(installed)]}</div>
                        {/if}
                        <details
                          class="version-history"
                          ontoggle={(event) => {
                            if (event.currentTarget.open && !versionHistory[lifecycleKey(installed)]) {
                              void loadVersionHistory(installed);
                            }
                          }}
                        >
                          <summary>{i18n.t("skills.versionHistory")}</summary>
                          {#if (versionHistory[lifecycleKey(installed)] ?? []).length === 0}
                            <p class="quiet">{i18n.t("skills.noVersionHistory")}</p>
                          {:else}
                            <ul>
                              {#each versionHistory[lifecycleKey(installed)] as snapshot (snapshot.path)}
                                <li>
                                  <span>{new Date(snapshot.createdAt).toLocaleString()}</span>
                                  <Button size="sm" onclick={() => void rollbackVersion(installed, snapshot.path)}>{i18n.t("skills.rollback")}</Button>
                                </li>
                              {/each}
                            </ul>
                          {/if}
                        </details>
                      </li>
                    {/each}
                  </ul>
                {/if}
                {#if skillSources.backups.length > 0}
                  <details class="backups">
                    <summary>{i18n.t("skills.backups")} <span>{skillSources.backups.length}</span></summary>
                    <ul>
                      {#each skillSources.backups as backup (backup)}
                        <li><code title={backup}>{backup}</code></li>
                      {/each}
                    </ul>
                  </details>
                {/if}
              {/if}
            </section>
            {/if}

            {#if detailTab === "files"}
            <section class="detail-section">
              <h4>{i18n.t("skills.packageFiles")} <span>{selected.pkg.files.length}</span></h4>
              {#if selected.pkg.files.length === 0}
                <p class="quiet">{i18n.t("skills.noSafeFiles")}</p>
              {:else}
                <ul class="files">
                  {#each selected.pkg.files as file (file.relativePath)}
                    <li>
                      <div><code>{file.relativePath}</code><span>{formatBytes(file.sizeBytes)}</span></div>
                      <code class="hash" title={file.sha256}>{file.sha256}</code>
                    </li>
                  {/each}
                </ul>
              {/if}
            </section>
            {/if}
          </div>
        {:else}
          <EmptyState title={i18n.t("skills.select")} body={i18n.t("skills.selectBody")} />
        {/if}
      </section>
    </div>
  {/if}
</div>

{#if folderModalOpen}
  <Modal open title={i18n.t("skills.newFolder")} defaultFocus="first" onClose={() => (folderModalOpen = false)}>
    <form
      class="folder-form"
      onsubmit={(event) => {
        event.preventDefault();
        void createPersonalFolder();
      }}
    >
      <label>
        <span>{i18n.t("skills.folderName")}</span>
        <Input
          bind:value={folderName}
          placeholder={i18n.t("skills.folderNamePlaceholder")}
          ariaLabel={i18n.t("skills.folderName")}
          ariaDescribedby="folder-error"
          invalid={folderError.length > 0}
        />
      </label>
      <label>
        <span>{i18n.t("skills.parentFolder")}</span>
        <select bind:value={folderParent} aria-label={i18n.t("skills.parentFolder")}>
          <option value="">{i18n.t("skills.folderRoot")}</option>
          {#each personalFolders.toSorted((left, right) => left.localeCompare(right)) as folder (folder)}
            <option value={folder}>{folder}</option>
          {/each}
        </select>
      </label>
      {#if folderError}<p id="folder-error" class="alert" role="alert">{folderError}</p>{/if}
      <button class="form-submit" type="submit" tabindex="-1" aria-hidden="true"></button>
    </form>
    {#snippet actions()}
      <Button variant="secondary" modalAction="cancel" onclick={() => (folderModalOpen = false)}>{i18n.t("common.cancel")}</Button>
      <Button variant="primary" modalAction="confirm" onclick={() => void createPersonalFolder()}>{i18n.t("skills.createFolder")}</Button>
    {/snippet}
  </Modal>
{/if}

{#if creatorOpen}
  <Modal open size="wide" title={i18n.t("skills.createSkill")} defaultFocus="first" onClose={() => (creatorOpen = false)}>
    <div class="creator-form">
      <label><span>{i18n.t("skills.skillName")}</span><Input bind:value={creatorName} placeholder="my-skill" ariaLabel={i18n.t("skills.skillName")} /></label>
      <label><span>{i18n.t("skills.description")}</span><Input bind:value={creatorDescription} ariaLabel={i18n.t("skills.description")} /></label>
      <div class="creator-row">
        <label>
          <span>{i18n.t("skills.type")}</span>
          <select bind:value={creatorType}>
            {#each ["design", "development", "testing", "devops", "security", "data", "ai", "productivity", "other"] as type}
              <option value={type}>{typeLabel(type as SkillType)}</option>
            {/each}
          </select>
        </label>
        <label><span>{i18n.t("skills.group")}</span><Input bind:value={creatorGroup} placeholder="frontend/svelte" ariaLabel={i18n.t("skills.group")} /></label>
        <label><span>{i18n.t("skills.tags")}</span><Input bind:value={creatorTags} placeholder="typescript, ui" ariaLabel={i18n.t("skills.tags")} /></label>
      </div>
      <label>
        <span>{i18n.t("skills.instructions")}</span>
        <textarea bind:value={creatorBody} rows="12" placeholder="# My skill&#10;&#10;Describe when and how to use it."></textarea>
      </label>
      {#if folderError}<p class="alert" role="alert">{folderError}</p>{/if}
    </div>
    {#snippet actions()}
      <Button variant="secondary" modalAction="cancel" onclick={() => (creatorOpen = false)}>{i18n.t("common.cancel")}</Button>
      <Button variant="primary" modalAction="confirm" disabled={!creatorName.trim() || !creatorDescription.trim() || !creatorBody.trim()} onclick={() => void createSkillDraft()}>{i18n.t("skills.submitForReview")}</Button>
    {/snippet}
  </Modal>
{/if}

<AgentCreatorModal
  open={agentCreatorSkill !== null}
  fromSkill={agentCreatorSkill}
  onClose={() => (agentCreatorSkill = null)}
/>

{#if editorOpen && selected}
  <Modal open size="wide" title={i18n.t("skills.editSkill", { name: selected.pkg.name ?? selected.pkg.relativePath })} defaultFocus="first" onClose={() => (editorOpen = false)}>
    <div class="creator-form">
      <p>{i18n.t("skills.editorHelp")}</p>
      <textarea bind:value={editorText} rows="20" aria-label={i18n.t("skills.skillMarkdown")}></textarea>
      {#if folderError}<p class="alert" role="alert">{folderError}</p>{/if}
    </div>
    {#snippet actions()}
      <Button variant="secondary" modalAction="cancel" onclick={() => (editorOpen = false)}>{i18n.t("common.cancel")}</Button>
      <Button variant="primary" modalAction="confirm" disabled={!editorText.trim()} onclick={() => void submitEditDraft()}>{i18n.t("skills.submitForReview")}</Button>
    {/snippet}
  </Modal>
{/if}

{#if organizeOpen}
  <Modal open size="wide" title={i18n.t("skills.organize")} defaultFocus="first" onClose={() => (organizeOpen = false)}>
    <div class="organize-grid">
      <section>
        <h4>{i18n.t("skills.collections")}</h4>
        <p>{i18n.t("skills.collectionsHelp", { count: filtered.length })}</p>
        <div class="organize-form">
          <Input bind:value={collectionName} placeholder={i18n.t("skills.collectionName")} ariaLabel={i18n.t("skills.collectionName")} />
          <Button size="sm" disabled={!collectionName.trim() || filtered.length === 0} onclick={() => void saveCurrentCollection()}>{i18n.t("skills.saveResults")}</Button>
        </div>
        <ul>
          {#each skillSources.folderState.collections as collection (collection.name)}
            <li><span>{collection.name} · {collection.skills.length}</span><Button size="sm" variant="danger" onclick={() => (organizeDelete = { kind: "collection", name: collection.name })}>{i18n.t("common.delete")}</Button></li>
          {/each}
        </ul>
      </section>
      <section>
        <h4>{i18n.t("skills.smartFolders")}</h4>
        <p>{i18n.t("skills.smartFoldersHelp")}</p>
        <div class="organize-form">
          <Input bind:value={smartFolderName} placeholder={i18n.t("skills.smartFolderName")} ariaLabel={i18n.t("skills.smartFolderName")} />
          <Button size="sm" disabled={!smartFolderName.trim()} onclick={() => void saveCurrentSmartFolder()}>{i18n.t("skills.saveFilters")}</Button>
        </div>
        <ul>
          {#each skillSources.folderState.smartFolders as smartFolder (smartFolder.name)}
            <li><span>{smartFolder.name}</span><Button size="sm" variant="danger" onclick={() => (organizeDelete = { kind: "smart", name: smartFolder.name })}>{i18n.t("common.delete")}</Button></li>
          {/each}
        </ul>
      </section>
      <section>
        <h4>{i18n.t("skills.workspaceProfiles")}</h4>
        <p>{i18n.t("skills.workspaceProfilesHelp")}</p>
        <div class="organize-form">
          <Input bind:value={profileName} placeholder={i18n.t("skills.profileName")} ariaLabel={i18n.t("skills.profileName")} />
          <Button size="sm" disabled={!profileName.trim()} onclick={() => void saveCurrentProfile()}>{i18n.t("skills.saveProfile")}</Button>
        </div>
        <ul>
          {#each skillSources.folderState.profiles as profile (profile.name)}
            <li><span>{profile.name}</span><Button size="sm" variant="danger" onclick={() => (organizeDelete = { kind: "profile", name: profile.name })}>{i18n.t("common.delete")}</Button></li>
          {/each}
        </ul>
      </section>
      <section>
        <h4>{i18n.t("skills.portableLibrary")}</h4>
        <p>{i18n.t("skills.portableLibraryHelp")}</p>
        <div class="organize-form">
          <Button size="sm" onclick={() => void exportLibrary()}>{i18n.t("skills.exportLibrary")}</Button>
          <Button size="sm" onclick={() => void importLibrary()}>{i18n.t("skills.importLibrary")}</Button>
        </div>
      </section>
      {#if folderError}<p class="alert" role="alert">{folderError}</p>{/if}
    </div>
  </Modal>
{/if}

{#if collectionInstallOpen}
  <Modal open title={i18n.t("skills.manageCollection")} onClose={() => (collectionInstallOpen = false)}>
    <div class="folder-form">
      <label>
        <span>{i18n.t("skills.operation")}</span>
        <select bind:value={collectionOperation}>
          <option value="install">{i18n.t("common.install")}</option>
          <option value="update">{i18n.t("common.update")}</option>
          <option value="uninstall">{i18n.t("common.uninstall")}</option>
        </select>
      </label>
      <label>
        <span>{i18n.t("skills.runtime")}</span>
        <select bind:value={collectionRuntime}>
          <option value="claudeCode">Claude Code</option>
          <option value="codex">Codex</option>
        </select>
      </label>
      <label>
        <span>{i18n.t("skills.destination")}</span>
        <select bind:value={collectionProject}>
          <option value="">{i18n.t("skills.globalDestination")}</option>
          {#each projects.list as project (project.path)}
            <option value={project.path}>{project.label}</option>
          {/each}
        </select>
      </label>
      <p class="quiet">{i18n.t("skills.manageCollectionHelp", { operation: collectionOperation, count: filtered.length })}</p>
    </div>
    {#snippet actions()}
      <Button variant="secondary" modalAction="cancel" onclick={() => (collectionInstallOpen = false)}>{i18n.t("common.cancel")}</Button>
      <Button variant="primary" modalAction="confirm" onclick={() => void installCurrentCollection()}>{i18n.t("skills.applyCollectionOperation")}</Button>
    {/snippet}
  </Modal>
{/if}

{#if installPlan && plannedInstall}
  {@const plan = installPlan}
  <Modal open title={i18n.t("skills.installPlan")} size="wide" onClose={() => { installPlan = null; plannedInstall = null; }}>
    <div class="plan">
      <p>{i18n.t("skills.installPlanHelp", { count: installPlan.packages.length })}</p>
      <ul class="files">
        {#each installPlan.packages as item (`${item.sourceId}:${item.relativePath}`)}
          <li>
            <div><strong>{item.name}</strong><span>{item.dependency ? i18n.t("skills.dependency") : i18n.t("skills.requestedSkill")}</span></div>
            <code>{item.destination}</code>
            {#if item.permissions.length > 0}<span>{item.permissions.join(", ")}</span>{/if}
          </li>
        {/each}
      </ul>
      {#each installPlan.warnings as warning}<p class="warning">{warning}</p>{/each}
      {#each installPlan.blockers as blocker}<p class="alert" role="alert">{blocker}</p>{/each}
      <p class="quiet">{installPlan.rollbackAvailable ? i18n.t("skills.rollbackAvailable") : i18n.t("skills.newInstallPlan")}</p>
    </div>
    {#snippet actions()}
      <Button variant="secondary" modalAction="cancel" onclick={() => { installPlan = null; plannedInstall = null; }}>{i18n.t("common.cancel")}</Button>
      <Button variant="primary" modalAction="confirm" disabled={plan.blockers.length > 0} onclick={() => void confirmPlannedInstall()}>{i18n.t("common.install")}</Button>
    {/snippet}
  </Modal>
{/if}

<DestructiveConfirm
  open={organizeDelete !== null}
  title={i18n.t("skills.deleteOrganizeTitle")}
  confirmLabel={i18n.t("common.delete")}
  onConfirm={() => void deleteOrganizeItem()}
  onCancel={() => (organizeDelete = null)}
>
  <p>{i18n.t("skills.deleteOrganizeBody", { name: organizeDelete?.name ?? "" })}</p>
</DestructiveConfirm>

<DestructiveConfirm
  open={removeCandidate !== null}
  title={i18n.t("skills.removeSourceTitle")}
  confirmLabel={i18n.t("skills.removeSource")}
  confirmDisabled={removeCandidate ? skillSources.isRemoving(removeCandidate.id) : false}
  onConfirm={() => void removeSource()}
  onCancel={() => (removeCandidate = null)}
>
  <p>{i18n.t("skills.removeSourceBody")}</p>
</DestructiveConfirm>

<DestructiveConfirm
  open={trustCandidate !== null}
  title={i18n.t("skills.trustTitle")}
  confirmLabel={i18n.t("skills.trustExactVersion")}
  confirmVariant="primary"
  confirmDisabled={trustCandidate ? skillSources.trusting[skillSources.packageKey(trustCandidate.pkg)] === true : false}
  onConfirm={() => void grantTrust()}
  onCancel={() => (trustCandidate = null)}
>
  <p>{i18n.t("skills.trustBody")}</p>
  {#if trustCandidate}
    <dl class="trust-fingerprint">
      <div><dt>{i18n.t("skills.trustSource")}</dt><dd><code>{trustCandidate.pkg.sourceId}</code></dd></div>
      <div><dt>{i18n.t("skills.trustPackage")}</dt><dd><code>{trustCandidate.pkg.relativePath}</code></dd></div>
      <div><dt>{i18n.t("skills.trustTreeHash")}</dt><dd><code>{trustCandidate.pkg.trustFingerprint?.treeHash ?? "—"}</code></dd></div>
    </dl>
    <h4>{i18n.t("skills.trustScripts")}</h4>
    <ul class="files">
      {#each trustCandidate.pkg.trustFingerprint?.executables ?? [] as file (file.relativePath)}
        <li>
          <div><code>{file.relativePath}</code><span>{file.executable ? i18n.t("skills.executable") : i18n.t("skills.notExecutable")}</span></div>
          <code class="hash" title={file.sha256}>{file.sha256}</code>
        </li>
      {/each}
    </ul>
  {/if}
</DestructiveConfirm>

<DestructiveConfirm
  open={rejectDraftCandidate !== null}
  title={i18n.t("skills.rejectDraftTitle")}
  confirmLabel={i18n.t("skills.rejectDraft")}
  onConfirm={() => void rejectDraft()}
  onCancel={() => (rejectDraftCandidate = null)}
>
  <p>{i18n.t("skills.rejectDraftBody")}</p>
</DestructiveConfirm>

<DestructiveConfirm
  open={uninstallCandidate !== null}
  title={i18n.t("skills.uninstallTitle")}
  confirmLabel={i18n.t("skills.uninstall")}
  confirmDisabled={uninstallCandidate ? skillSources.installing[lifecycleKey(uninstallCandidate)] === true : false}
  onConfirm={() => uninstallCandidate && void runLifecycle("uninstall", uninstallCandidate)}
  onCancel={() => (uninstallCandidate = null)}
>
  <p>{i18n.t("skills.uninstallBody")}</p>
</DestructiveConfirm>

<style>
  .workspace { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  header { position: relative; z-index: 3; flex: none; display: flex; align-items: center; justify-content: space-between; gap: var(--space-4); padding: var(--space-4); border-bottom: 1px solid var(--color-border); }
  .header-actions { display: flex; align-items: center; gap: var(--space-2); }
  h2 { font-size: var(--text-h2); font-weight: var(--fw-semibold); color: var(--color-text-primary); text-wrap: balance; }
  header p, .quiet, .section-help { margin-top: var(--space-1); color: var(--color-text-muted); font-size: var(--text-body-sm); }
  .announcement { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border: 0; }
  .unavailable { display: grid; grid-template-columns: auto 1fr; align-items: center; gap: var(--space-2); padding: var(--space-3) var(--space-4); border-bottom: 1px solid var(--color-warning); color: var(--color-text-secondary); font-size: var(--text-body-sm); }
  .unavailable > div { grid-column: 1 / -1; display: flex; align-items: center; gap: var(--space-2); }
  .unavailable code { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .source-manager, .draft-inbox { position: relative; }
  .source-manager summary, .draft-inbox summary { cursor: pointer; list-style: none; padding: var(--space-2) var(--space-3); border: 1px solid var(--color-border); border-radius: var(--radius-md); font-size: var(--text-body-sm); color: var(--color-text-secondary); }
  .source-manager summary span, .draft-inbox summary span { margin-left: var(--space-1); color: var(--color-text-muted); }
  .source-popover { position: absolute; top: calc(100% + var(--space-2)); right: 0; width: min(620px, calc(100vw - 80px)); max-height: 70vh; overflow-y: auto; display: grid; gap: var(--space-3); padding: var(--space-4); border: 1px solid var(--color-border); border-radius: var(--radius-lg); background: var(--color-surface-raised); box-shadow: var(--shadow-modal); }
  .draft-popover { position: absolute; top: calc(100% + var(--space-2)); right: 0; width: min(520px, calc(100vw - 80px)); max-height: 70vh; overflow-y: auto; display: grid; gap: var(--space-2); padding: var(--space-3); border: 1px solid var(--color-border); border-radius: var(--radius-lg); background: var(--color-surface-raised); box-shadow: var(--shadow-modal); }
  .draft { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: var(--space-2); padding: var(--space-3); border: 1px solid var(--color-border); border-radius: var(--radius-md); }
  .draft > div:first-child { min-width: 0; display: grid; }
  .draft span { color: var(--color-text-muted); font-size: var(--text-caption); }
  .draft-actions { display: flex; align-items: center; gap: var(--space-2); }
  .draft-error, .draft .diagnostics { grid-column: 1 / -1; }
  .draft-error { color: var(--color-danger); font-size: var(--text-body-sm); }
  .local-add, .source-actions, .github-options, .filter-row, .row-top, .detail-title, .files div { display: flex; align-items: center; gap: var(--space-2); }
  .local-add { justify-content: flex-end; }
  .picked { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--color-text-muted); font-family: var(--font-mono); font-size: var(--text-caption); }
  .github-add { display: grid; gap: var(--space-2); }
  .github-options > :global(*) { flex: 1; }
  .github-add p { color: var(--color-text-muted); font-size: var(--text-caption); }
  .source-list { display: grid; gap: var(--space-2); }
  .source { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: var(--space-2); padding: var(--space-3); border: 1px solid var(--color-border); border-radius: var(--radius-md); }
  .source > div:first-child { min-width: 0; display: grid; }
  .source strong { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .source span { color: var(--color-text-muted); font-size: var(--text-caption); }
  .source .alert, .source .diagnostics { grid-column: 1 / -1; }
  .browser { flex: 1; min-height: 0; display: grid; grid-template-columns: 224px minmax(320px, 380px) minmax(360px, 1fr); }
  .library-pane, .package-list { min-height: 0; display: flex; flex-direction: column; border-right: 1px solid var(--color-border); }
  .library-pane { overflow-y: auto; background: var(--color-surface-sunken); }
  .pane-heading { display: flex; align-items: center; justify-content: space-between; min-height: 42px; padding: 0 var(--space-3); border-bottom: 1px solid var(--color-border); color: var(--color-text-primary); font-size: var(--text-body-sm); font-weight: var(--fw-semibold); }
  .pane-heading span:last-child { color: var(--color-text-muted); font-variant-numeric: tabular-nums; font-weight: var(--fw-normal); }
  .pane-tools { display: flex; align-items: center; gap: var(--space-2); }
  .pane-tools button, .favorite-button { display: inline-flex; padding: var(--space-1); border-radius: var(--radius-sm); color: var(--color-text-muted); }
  .pane-tools button:hover, .favorite-button:hover { background: var(--color-surface-sunken); color: var(--color-text-primary); }
  .favorite-button.active { color: var(--color-brand); }
  .quick-filters { display: grid; gap: 2px; padding: var(--space-2); }
  .quick-filters button { display: flex; align-items: center; justify-content: space-between; width: 100%; padding: var(--space-2); border-radius: var(--radius-sm); color: var(--color-text-secondary); text-align: left; font-size: var(--text-body-sm); }
  .quick-filters button:hover { background: var(--color-surface-raised); color: var(--color-text-primary); }
  .quick-filters button.active { background: color-mix(in srgb, var(--color-brand) 14%, transparent); color: var(--color-text-primary); }
  .quick-filters button span:last-child { color: var(--color-text-muted); font-size: var(--text-caption); font-variant-numeric: tabular-nums; }
  .tree-heading { padding: var(--space-3) var(--space-3) var(--space-1); color: var(--color-text-muted); font-size: var(--text-caption); font-weight: var(--fw-semibold); text-transform: uppercase; }
  .folder-heading { display: flex; align-items: center; justify-content: space-between; }
  .add-folder { display: inline-flex; padding: var(--space-1); border-radius: var(--radius-sm); color: var(--color-text-muted); }
  .add-folder:hover { background: var(--color-surface-raised); color: var(--color-text-primary); }
  .empty-folder-action { margin: var(--space-1) var(--space-3) var(--space-2); color: var(--color-brand); font-size: var(--text-body-sm); text-align: left; }
  .personal-folder-tree { padding-bottom: var(--space-2); }
  .personal-folder button { display: flex; justify-content: space-between; width: 100%; padding: var(--space-2) var(--space-3); padding-left: calc(var(--space-3) + var(--tree-depth) * 14px); color: var(--color-text-secondary); font-size: var(--text-body-sm); text-align: left; }
  .personal-folder button:hover { background: var(--color-surface-raised); color: var(--color-text-primary); }
  .personal-folder button.active { background: color-mix(in srgb, var(--color-brand) 14%, transparent); color: var(--color-text-primary); }
  .personal-folder button span:last-child { color: var(--color-text-muted); font-variant-numeric: tabular-nums; }
  .named-views { padding-bottom: var(--space-2); }
  .named-views button { display: flex; justify-content: space-between; width: 100%; padding: var(--space-2) var(--space-3); color: var(--color-text-secondary); font-size: var(--text-body-sm); text-align: left; }
  .named-views button:hover { background: var(--color-surface-raised); color: var(--color-text-primary); }
  .named-views button.active { background: color-mix(in srgb, var(--color-brand) 14%, transparent); color: var(--color-text-primary); }
  .named-views button span:last-child { color: var(--color-text-muted); font-variant-numeric: tabular-nums; }
  .taxonomy-tree { padding-bottom: var(--space-3); }
  .filters { min-width: 0; display: grid; gap: var(--space-2); padding: var(--space-3); border-bottom: 1px solid var(--color-border); }
  .search { display: flex; align-items: center; gap: var(--space-2); padding: var(--space-2) var(--space-3); border: 1px solid var(--color-border); border-radius: var(--radius-md); color: var(--color-text-muted); background: var(--color-surface-raised); }
  .search:focus-within { border-color: var(--color-brand); }
  .search input { flex: 1; min-width: 0; background: transparent; color: var(--color-text-primary); }
  .filter-row { flex-wrap: wrap; justify-content: space-between; gap: var(--space-2); }
  .segments { display: flex; padding: 2px; border-radius: var(--radius-md); background: var(--color-surface-sunken); }
  .segments button { padding: var(--space-1) var(--space-2); border-radius: var(--radius-sm); color: var(--color-text-muted); font-size: var(--text-caption); }
  .segments button.active { background: var(--color-surface-raised); color: var(--color-text-primary); box-shadow: var(--shadow-sm); }
  select { min-width: 0; max-width: 45%; padding: var(--space-1) var(--space-2); border: 1px solid var(--color-border); border-radius: var(--radius-md); background: var(--color-surface-raised); color: var(--color-text-secondary); font-size: var(--text-caption); }
  .result-count { padding: var(--space-2) var(--space-3); color: var(--color-text-muted); font-size: var(--text-caption); border-bottom: 1px solid var(--color-border); }
  .results { flex: 1; min-height: 0; overflow-y: auto; }
  .skill-group summary { display: flex; justify-content: space-between; padding: var(--space-2) var(--space-3); padding-left: calc(var(--space-3) + var(--tree-depth) * 14px); cursor: pointer; color: var(--color-text-secondary); font-size: var(--text-body-sm); }
  .skill-group summary:hover { background: var(--color-surface-raised); color: var(--color-text-primary); }
  .skill-group summary.active { background: color-mix(in srgb, var(--color-brand) 14%, transparent); color: var(--color-text-primary); }
  .skill-group summary span:last-child { color: var(--color-text-muted); font-weight: var(--fw-normal); }
  .results button { width: 100%; display: grid; gap: var(--space-1); padding: var(--space-3); border-bottom: 1px solid var(--color-border); color: var(--color-text-primary); text-align: left; }
  .results button:hover { background: var(--color-surface-sunken); }
  .results button.selected { background: color-mix(in srgb, var(--color-brand) 14%, transparent); box-shadow: inset 3px 0 var(--color-brand); }
  .row-top { justify-content: space-between; }
  .description { display: -webkit-box; overflow: hidden; line-clamp: 2; -webkit-line-clamp: 2; -webkit-box-orient: vertical; color: var(--color-text-secondary); font-size: var(--text-body-sm); text-wrap: pretty; }
  .provenance { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--color-text-muted); font-size: var(--text-caption); }
  .status-badge { flex: none; padding: 2px var(--space-2); border-radius: var(--radius-full); font-size: var(--text-caption); font-weight: var(--fw-semibold); }
  .status-badge.ready { color: var(--color-success); background: color-mix(in srgb, var(--color-success) 12%, transparent); }
  .status-badge.rejected { color: var(--color-danger); background: color-mix(in srgb, var(--color-danger) 12%, transparent); }
  .detail { min-width: 0; min-height: 0; display: flex; }
  .detail-scroll { flex: 1; min-width: 0; overflow-y: auto; padding: var(--space-5); }
  .detail-title { align-items: flex-start; justify-content: space-between; padding-bottom: var(--space-4); }
  .detail-actions { display: flex; align-items: center; gap: var(--space-2); }
  .detail-title h3 { margin-top: var(--space-1); font-size: var(--text-h2); color: var(--color-text-primary); text-wrap: balance; }
  .detail-title p { margin-top: var(--space-2); max-width: 70ch; color: var(--color-text-secondary); text-wrap: pretty; }
  .detail-tabs { position: sticky; top: 0; z-index: 1; display: flex; gap: var(--space-1); padding: var(--space-2) 0; border-bottom: 1px solid var(--color-border); background: var(--color-surface-raised); }
  .detail-tabs button { padding: var(--space-2) var(--space-3); border-radius: var(--radius-sm); color: var(--color-text-muted); font-size: var(--text-body-sm); }
  .detail-tabs button:hover { background: var(--color-surface-sunken); color: var(--color-text-primary); }
  .detail-tabs button.active { background: color-mix(in srgb, var(--color-brand) 14%, transparent); color: var(--color-text-primary); }
  .eyebrow { color: var(--color-text-muted); font-size: var(--text-caption); font-weight: var(--fw-semibold); text-transform: uppercase; }
  .detail-section { display: grid; gap: var(--space-3); padding: var(--space-4) 0; border-top: 1px solid var(--color-border); }
  .detail-section h4 { color: var(--color-text-primary); font-size: var(--text-body); font-weight: var(--fw-semibold); }
  .detail-section h4 span { color: var(--color-text-muted); font-weight: var(--fw-normal); }
  .section-title-row { display: flex; align-items: center; justify-content: space-between; gap: var(--space-3); }
  .folder-assignment select { max-width: 100%; width: min(360px, 100%); }
  .policy-select { max-width: 100%; width: min(360px, 100%); }
  .warning { color: var(--color-warning); font-size: var(--text-body-sm); }
  .permission-list, .quality-list { display: flex; flex-wrap: wrap; gap: var(--space-2); }
  .permission-list li, .quality-list li { padding: var(--space-1) var(--space-2); border: 1px solid var(--color-border); border-radius: var(--radius-full); color: var(--color-text-secondary); font-size: var(--text-caption); }
  .folder-form { display: grid; gap: var(--space-3); }
  .folder-form label { display: grid; gap: var(--space-1); color: var(--color-text-secondary); font-size: var(--text-body-sm); }
  .folder-form select { width: 100%; max-width: none; min-height: 30px; }
  .form-submit { position: absolute; width: 1px; height: 1px; overflow: hidden; opacity: 0; }
  .organize-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--space-4); }
  .organize-grid section { display: grid; align-content: start; gap: var(--space-2); padding: var(--space-3); border: 1px solid var(--color-border); border-radius: var(--radius-md); }
  .organize-grid h4 { color: var(--color-text-primary); font-weight: var(--fw-semibold); text-wrap: balance; }
  .organize-grid p { color: var(--color-text-muted); font-size: var(--text-body-sm); text-wrap: pretty; }
  .organize-form { display: flex; align-items: center; gap: var(--space-2); }
  .organize-form > :global(:first-child) { flex: 1; }
  .organize-grid ul { display: grid; gap: var(--space-1); }
  .organize-grid li { display: flex; align-items: center; justify-content: space-between; gap: var(--space-2); color: var(--color-text-secondary); font-size: var(--text-body-sm); }
  .organize-grid > .alert { grid-column: 1 / -1; }
  .creator-form { display: grid; gap: var(--space-3); }
  .creator-form label { display: grid; gap: var(--space-1); color: var(--color-text-secondary); font-size: var(--text-body-sm); }
  .creator-row { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: var(--space-2); }
  .creator-row select { width: 100%; max-width: none; min-height: 30px; }
  .creator-form textarea { width: 100%; resize: vertical; padding: var(--space-3); border: 1px solid var(--color-border); border-radius: var(--radius-md); background: var(--color-surface-sunken); color: var(--color-text-primary); font-family: var(--font-mono); font-size: var(--text-body-sm); }
  .creator-form textarea:focus { border-color: var(--color-brand); box-shadow: var(--shadow-focus-ring); outline: none; }
  .creator-form p { color: var(--color-text-muted); font-size: var(--text-body-sm); text-wrap: pretty; }
  dl { display: grid; gap: var(--space-2); }
  dl div { display: grid; grid-template-columns: 120px minmax(0, 1fr); gap: var(--space-3); }
  dt { color: var(--color-text-muted); font-size: var(--text-body-sm); }
  dd { min-width: 0; overflow-wrap: anywhere; color: var(--color-text-secondary); font-size: var(--text-body-sm); }
  .healthy { color: var(--color-success); font-size: var(--text-body-sm); }
  .diagnostics { display: grid; gap: var(--space-2); color: var(--color-danger); font-size: var(--text-body-sm); }
  .alert { padding: var(--space-3); border: 1px solid var(--color-danger); border-radius: var(--radius-md); color: var(--color-danger); font-size: var(--text-body-sm); }
  code { font-family: var(--font-mono); }
  .files { display: grid; gap: var(--space-2); }
  .skill-installs { display: grid; gap: var(--space-2); }
  .skill-installs li { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: var(--space-2); padding: var(--space-3); border: 1px solid var(--color-border); border-radius: var(--radius-md); }
  .skill-installs li > div:first-child { min-width: 0; display: grid; gap: 2px; }
  .skill-installs code, .backups code { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--color-text-muted); font-size: var(--text-caption); }
  .install-state { color: var(--color-text-muted); font-size: var(--text-caption); text-transform: capitalize; }
  .lifecycle-actions { display: flex; align-items: center; gap: var(--space-2); }
  .skill-installs .alert { grid-column: 1 / -1; }
  .version-history { grid-column: 1 / -1; }
  .version-history summary { cursor: pointer; color: var(--color-text-secondary); font-size: var(--text-body-sm); }
  .version-history ul { display: grid; gap: var(--space-1); margin-top: var(--space-2); }
  .version-history li { display: flex; align-items: center; justify-content: space-between; padding: var(--space-2); }
  .version-history li span { color: var(--color-text-muted); font-size: var(--text-caption); }
  .backups summary { cursor: pointer; color: var(--color-text-secondary); font-size: var(--text-body-sm); }
  .backups ul { display: grid; gap: var(--space-1); margin-top: var(--space-2); }
  .files li { min-width: 0; padding: var(--space-3); border-radius: var(--radius-md); background: var(--color-surface-sunken); }
  .files div { justify-content: space-between; }
  .files div code { overflow-wrap: anywhere; color: var(--color-text-primary); font-size: var(--text-body-sm); }
  .files div span, .hash { color: var(--color-text-muted); font-size: var(--text-caption); }
  .hash { display: block; margin-top: var(--space-1); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  @media (max-width: 1100px) {
    .browser { grid-template-columns: 200px minmax(300px, 1fr); }
    .detail { grid-column: 1 / -1; border-top: 1px solid var(--color-border); min-height: 45vh; }
  }
  @media (max-width: 760px) {
    .browser { grid-template-columns: 1fr; overflow-y: auto; }
    .library-pane, .package-list { border-right: 0; max-height: 45vh; }
    .detail { border-top: 1px solid var(--color-border); min-height: 50vh; }
    header { align-items: flex-start; }
    .header-actions { flex-wrap: wrap; justify-content: flex-end; }
    .source-popover { position: fixed; left: var(--space-4); right: var(--space-4); width: auto; }
    .draft-popover { position: fixed; left: var(--space-4); right: var(--space-4); width: auto; }
    .organize-grid { grid-template-columns: 1fr; }
    .creator-row { grid-template-columns: 1fr; }
  }
</style>
