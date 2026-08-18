import { createRawSnippet, mount, tick, unmount } from "svelte";
import { readFileSync } from "node:fs";
import { invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import Modal from "$lib/components/Modal.svelte";
import StorageMigrationGate from "$lib/components/StorageMigrationGate.svelte";
import UpdatesModal from "$lib/components/UpdatesModal.svelte";
import DivisionsLanding from "$lib/components/DivisionsLanding.svelte";
import Sidebar from "$lib/components/Sidebar.svelte";
import CommandPalette from "$lib/components/CommandPalette.svelte";
import SettingsSectionMcp from "$lib/components/SettingsSectionMcp.svelte";
import RunbooksView from "$lib/components/Runbooks.svelte";
import agentsWorkspaceSource from "$lib/components/AgentsWorkspace.svelte?raw";
import skillsWorkspaceSource from "$lib/components/SkillsWorkspace.svelte?raw";
import installModalSource from "$lib/components/InstallModal.svelte?raw";
import settingsSource from "$lib/components/Settings.svelte?raw";
import activityHistorySource from "$lib/components/ActivityHistory.svelte?raw";
import sidebarSource from "$lib/components/Sidebar.svelte?raw";
import catalogFirstRunSource from "$lib/components/CatalogFirstRun.svelte?raw";
import titlebarControlsSource from "$lib/components/TitlebarControls.svelte?raw";
import pageSource from "../routes/+page.svelte?raw";
import ollamaDeployModalSource from "$lib/components/OllamaDeployModal.svelte?raw";
import deployBrowserSource from "$lib/components/DeployBrowser.svelte?raw";
import deploymentTargetGridSource from "$lib/components/DeploymentTargetGrid.svelte?raw";
import dashboardSource from "$lib/components/AgencyDashboard.svelte?raw";
import toolsViewSource from "$lib/components/ToolsView.svelte?raw";
import teamsSource from "$lib/components/Teams.svelte?raw";
import projectsSource from "$lib/components/Projects.svelte?raw";
import {
  activity,
  mergeActivityEntries,
  normalizeActivityReceipt,
  normalizePersistedActivityEntries,
  safeActivityDetail,
  selectMcpAuditEntries,
} from "$lib/stores/activity.svelte";
import type { JournalEntry } from "$lib/stores/activity.svelte";
import { agentLibrary } from "$lib/stores/agentLibrary.svelte";
import { catalog } from "$lib/stores/catalog.svelte";
import { corpus } from "$lib/stores/corpus.svelte";
import { experts, summarizeExpertPerformance } from "$lib/stores/experts.svelte";
import { install, SUPPORTED_TOOLS } from "$lib/stores/install.svelte";
import { i18n } from "$lib/stores/i18n.svelte";
import { projects } from "$lib/stores/projects.svelte";
import { settings } from "$lib/stores/settings.svelte";
import { filterPlaybooks, runbooks } from "$lib/stores/runbooks.svelte";
import { skillSources } from "$lib/stores/skillSources.svelte";
import { teams } from "$lib/stores/teams.svelte";
import { toast } from "$lib/stores/toast.svelte";
import { ui } from "$lib/stores/ui.svelte";
import { SETTINGS_DEFAULTS, type Agent, type AgentMutationPlan, type AgentPackageResult, type AgentSource, type CatalogFeedState, type CatalogStatus, type DoctorReport, type ExpertResolved, type ExpertRun, type InstalledAgent, type InstalledSkill, type InstallRecord, type McpAuditEntry, type ProjectInstructionPlan, type ProjectInstructionTarget, type ProjectReadinessReport, type ProjectRecommendation, type SidebarSection, type WorkspacePackPlan } from "$lib/types";

const notificationMocks = vi.hoisted(() => ({
  isPermissionGranted: vi.fn(async () => true),
  requestPermission: vi.fn(async () => "granted" as NotificationPermission),
  sendNotification: vi.fn(),
  action: null as ((notification: { extra?: Record<string, unknown> }) => void) | null,
  unlisten: vi.fn(),
}));

const skillRecommendation = (sourceId = "skills-b") => ({
  kind: "skill" as const,
  package: {
    sourceId, relativePath: "nested/reviewer", name: "Reviewer", description: "Reviews Rust",
    skillType: "testing" as const, group: [], tags: ["rust"], dependencies: [], recommendedSkills: [],
    version: null, channel: "stable", changelog: null, publisher: null, publisherKey: null,
    publisherVerified: false, validationResults: [], permissions: [], qualityScore: 80,
    qualityChecks: [], files: [], trustFingerprint: null, errors: [], installable: true,
  },
  score: 4,
  reasons: ["task:name:reviewer"],
});

const rel01Sources = import.meta.glob([
  "./components/Teams.svelte", "./components/SettingsSectionCatalog.svelte", "./components/ToolsView.svelte",
  "./components/AgentDetailTabs.svelte", "./components/CatalogFirstRun.svelte", "./components/SkillsWorkspace.svelte",
  "./components/Experts.svelte", "./components/InstallModal.svelte", "./components/DiffModal.svelte",
  "./components/AgencyDashboard.svelte", "./components/Projects.svelte", "./components/Runbooks.svelte",
  "./stores/catalog.svelte.ts", "./stores/settings.svelte.ts", "./stores/experts.svelte.ts",
  "./stores/activity.svelte.ts", "./stores/install.svelte.ts",
], { query: "?raw", import: "default", eager: true }) as Record<string, string>;

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (command: string) => ["skill_folders_list", "agent_library_list"].includes(command)
    ? {
        folders: [], assignments: [], favorites: [], recent: [], collections: [], smartFolders: [],
        profiles: [], updatePolicies: [], publisherTrust: [], preferredSources: [], usage: [], approvals: [],
      }
    : []),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => undefined),
}));

vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: notificationMocks.isPermissionGranted,
  requestPermission: notificationMocks.requestPermission,
  sendNotification: notificationMocks.sendNotification,
  onAction: vi.fn(async (callback: (notification: { extra?: Record<string, unknown> }) => void) => {
    notificationMocks.action = callback;
    return { unregister: async () => { notificationMocks.unlisten(); } };
  }),
}));

const emptyFolderState = () => ({
  folders: [], assignments: [], favorites: [], recent: [], collections: [], smartFolders: [],
  profiles: [], updatePolicies: [], publisherTrust: [], preferredSources: [], usage: [], approvals: [],
});

const staleControlAgent: Agent = {
  slug: "reviewer", name: "Reviewer", description: "Reviews code", category: "engineering",
  emoji: null, color: null, vibe: null, body: "Review carefully.",
};
const staleControlRow: InstalledAgent = {
  slug: "reviewer", name: "Reviewer", sourceId: "", relativePath: "",
  tool: "claudeCode", scope: "user", projectPath: null, dest: "/tmp/reviewer.md",
  state: "foreign", updateKind: null, tracked: false,
};
const staleControlTool = {
  tool: "claudeCode" as const, label: "Claude Code", detected: true, scope: "user" as const,
  userDest: "/tmp", installedCount: 1, customPath: null,
};
const installRecord = (slug: string, dest: string): InstallRecord => ({
  slug, sourceId: "built-in", relativePath: `${slug}.md`, tool: "claudeCode", scope: "user",
  projectPath: null, dest, sourceHash: "source", bodyHash: "body", renderedHash: "rendered",
  disabledPath: null, sourceSnapshotHash: "snapshot", capabilities: [], publisherKey: null,
  publisherVerified: false, installedAt: "2026-08-14T01:00:00Z", corpusVersion: "test",
});
const catalogStatusFixture: CatalogStatus = {
  source: { kind: "bundled" }, root: null, isGit: false, branch: null, commit: null,
  lastCommitSubject: null, lastCommitDate: null, dirtyCount: 0, remoteUrl: null,
  repoSlug: null, version: "test", fetchedAt: "2026-08-17T00:00:00Z", agentCount: 1,
};
const catalogFeedFixture = (
  path = "engineering/reviewer.md",
  stale = false,
  error: string | null = null,
): CatalogFeedState => ({
  lastSuccessAt: "2026-08-17T00:00:00Z",
  stale,
  error,
  batches: [{
    at: "2026-08-17T00:00:00Z",
    changes: [{
      kind: "added",
      item: {
        category: "engineering", relativePath: path,
        sourceHash: "a".repeat(64), bodyHash: "b".repeat(64),
      },
    }],
  }],
});
const readinessFixture = (projectPath = "/tmp/project", subscribed = true): ProjectReadinessReport => ({
  projectPath,
  overall: "needsAttention",
  subscribed,
  baseline: {
    projectPath,
    label: "Review baseline",
    agentRequirements: [{
      reference: { sourceId: "built-in", relativePath: "engineering/old-reviewer.md" },
      tool: "claudeCode",
    }],
    skillRequirements: [{
      reference: { sourceId: "skills", relativePath: "audit" },
      runtime: "codex",
    }],
    agents: [{ sourceId: "built-in", relativePath: "engineering/old-reviewer.md" }],
    skills: [{ sourceId: "skills", relativePath: "audit" }],
    instructions: [{ id: "agents", known: true }],
    mcpServers: [{ id: "agency-agents", known: true }],
    tools: ["claudeCode"],
  },
  categories: [
    { category: "agentRoster", state: "needsAttention", rows: [{ id: "built-in:engineering/old-reviewer.md:claudeCode", label: "Old reviewer", state: "needsAttention", evidence: "Drifted" }] },
    { category: "skills", state: "needsAttention", rows: [{ id: "skills:audit:codex", label: "Audit", state: "needsAttention", evidence: "Missing" }] },
    { category: "instructions", state: "needsAttention", rows: [{ id: "agents", label: "AGENTS.md", state: "needsAttention", evidence: "Missing" }] },
    { category: "mcp", state: "unavailable", rows: [{ id: "agency-agents", label: "agency-agents", state: "unavailable", evidence: "Inspection failed" }] },
    { category: "tools", state: "needsAttention", rows: [{ id: "claudeCode", label: "Claude Code", state: "needsAttention", evidence: "Not detected" }] },
  ],
});
const renamedProjectRecommendation = (projectPath = "/tmp/project"): ProjectRecommendation => ({
  id: "a".repeat(64),
  projectPath,
  batchAt: "2026-08-17T01:00:00Z",
  lifecycle: "new",
  summary: "Required Agent was renamed: engineering/old-reviewer.md → engineering/new-reviewer.md",
  changeKind: "renamed",
  baselineReference: { sourceId: "built-in", relativePath: "engineering/old-reviewer.md" },
  agentReferences: [{ sourceId: "built-in", relativePath: "engineering/new-reviewer.md" }],
  finalizeOnly: false,
  targets: [{
    reference: { sourceId: "built-in", relativePath: "engineering/new-reviewer.md" },
    tool: "claudeCode",
    projectPath,
    operation: "install",
  }],
});
const updatedProjectRecommendation = (projectPath = "/tmp/project"): ProjectRecommendation => ({
  id: "b".repeat(64),
  projectPath,
  batchAt: "2026-08-17T02:00:00Z",
  lifecycle: "new",
  summary: "Required Agent was updated in a successful catalog refresh",
  changeKind: "updated",
  baselineReference: { sourceId: "built-in", relativePath: "engineering/reviewer.md" },
  agentReferences: [{ sourceId: "built-in", relativePath: "engineering/reviewer.md" }],
  finalizeOnly: false,
  targets: [{
    reference: { sourceId: "built-in", relativePath: "engineering/reviewer.md" },
    tool: "claudeCode",
    projectPath,
    operation: "update",
  }],
});
const staleControlPackage: AgentPackageResult = {
  reference: { sourceId: "built-in", relativePath: "reviewer.md" }, agent: staleControlAgent,
  sourceHash: "source", frontmatterHash: "frontmatter", bodyHash: "body", version: null,
  channel: null, changelog: null, publisher: null, publisherKey: null, publisherVerified: false,
  requiredAgents: [], requiredSkills: [], recommendedAgents: [], groups: [], tags: [], capabilities: [],
  permissions: [], qualityScore: 100, qualityChecks: [], diagnostics: [], installable: true,
};
const repairSkillInspection = (sha256: string) => [{
  source: { id: "skills", kind: { kind: "local", root: "/tmp/skills" } },
  packages: [{
    sourceId: "skills", relativePath: "audit", name: "Audit", description: "Audit package",
    skillType: "testing", group: [], tags: [], dependencies: [], recommendedSkills: [], version: null,
    channel: "stable", changelog: null, publisher: null, publisherKey: null, publisherVerified: false,
    validationResults: [], permissions: [], qualityScore: 100, qualityChecks: [],
    files: [{ relativePath: "SKILL.md", sizeBytes: 12, sha256 }], trustFingerprint: null,
    errors: [], installable: true,
  }],
  errors: [],
}];

const performanceExpert = (): ExpertResolved => ({
  id: "reviewer", name: "Reviewer", summary: "Reviews code", category: "Engineering", tags: [],
  version: 2, leadAgent: "reviewer", supportingAgents: [], requiredSkills: [], optionalSkills: [],
  runbook: null, preferredClient: null, starterPrompt: "Review carefully.", source: "curated",
  qualityContract: { version: 3, checks: [
    { name: "Tests", kind: "command", required: true, evidenceMode: "clientReported" },
    { name: "Review", kind: "human", required: true, evidenceMode: "userConfirmed" },
  ] },
  unresolvedAgents: [], unresolvedSkills: [], unresolvedRunbook: false,
});

const performanceRun = (
  id: string,
  state: ExpertRun["state"],
  evidence: ExpertRun["evidence"] = [],
  waivers: ExpertRun["waivers"] = [],
  changes: Partial<ExpertRun> = {},
): ExpertRun => ({
  id, expertId: "reviewer", expertVersion: 2, projectPath: "/tmp/project", client: "codex",
  leadAgent: "reviewer", supportingAgents: [], requiredSkills: [], optionalSkills: [], runbook: null,
  contract: structuredClone(performanceExpert().qualityContract), state,
  startedAt: "2026-08-14T01:00:00Z", endedAt: "2026-08-14T02:00:00Z",
  evidence, blockers: [], waivers, ...changes,
});

beforeEach(async () => {
  vi.unstubAllGlobals();
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
    removeItem: (key: string) => storage.delete(key),
    clear: () => storage.clear(),
  });
  vi.mocked(invoke).mockReset().mockImplementation(async (command: string) =>
    ["skill_folders_list", "agent_library_list"].includes(command) ? emptyFolderState() as never : [] as never);
  catalog.busy = false;
  catalog.error = null;
  catalog.source = { kind: "bundled" };
  catalog.configured = true;
  catalog.status = null;
  catalog.updateCheck = null;
  catalog.detection = null;
  catalog.checking = false;
  catalog.scanning = false;
  agentLibrary.results = [];
  corpus.agents = [];
  corpus.categories = [];
  corpus.loading = false;
  corpus.error = null;
  experts.error = null;
  experts.list = [];
  experts.requests = [];
  experts.creationRequests = [];
  experts.history = [];
  experts.runs = [];
  experts.loading = false;
  install.installed = [];
  install.rosters = [];
  install.rosterReconciling = false;
  install.rostersReconciled = false;
  install.rosterReconcileError = null;
  install.tools = [];
  install.reconciling = false;
  install.reconciled = false;
  install.reconcileError = null;
  install.reconcileAttempt = 0;
  install.reconcileTerminal = 0;
  projects.list = [];
  runbooks.list = [];
  runbooks.playbooks = [];
  runbooks.selected = null;
  runbooks.loaded = false;
  runbooks.loading = false;
  runbooks.runbooksError = null;
  runbooks.error = null;
  runbooks.reading = false;
  runbooks.readError = null;
  settings.data = null;
  settings.error = null;
  settings.corruptOnDisk = false;
  notificationMocks.isPermissionGranted.mockReset().mockResolvedValue(true);
  notificationMocks.requestPermission.mockReset().mockResolvedValue("granted");
  notificationMocks.sendNotification.mockReset();
  notificationMocks.action = null;
  notificationMocks.unlisten.mockReset();
  skillSources.sources = [];
  skillSources.results = {};
  skillSources.installed = [];
  skillSources.backups = [];
  skillSources.drafts = [];
  skillSources.folderState = emptyFolderState();
  skillSources.reconciling = false;
  skillSources.reconciled = false;
  skillSources.reconcileError = null;
  skillSources.reconcileAttempt = 0;
  skillSources.reconcileTerminal = 0;
  skillSources.addError = null;
  teams.saved = [];
  toast.items = [];
  ui.projectsSelected = null;
  ui.teamsSelected = null;
  ui.toolsSelected = null;
  ui.activityReceiptId = null;
  ui.clearReviewIntent();
  ui.clearRecoveryIntent();
  ui.agentsReference = null;
  ui.skillsSelected = null;
  ui.paletteOpen = false;
  ui.settingsOpen = false;
  ui.settingsInitialSection = null;
  await tick();
});

afterEach(async () => {
  await Promise.resolve();
  await tick();
  vi.unstubAllGlobals();
  document.body.replaceChildren();
  toast.items = [];
});

describe("frontend test harness", () => {
  it("runs unit tests", () => {
    expect(true).toBe(true);
  });

  it("uses one owned, keyboard-roving Settings tab stop", async () => {
    ui.openSettings();
    const { default: Settings } = await import("$lib/components/Settings.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(Settings, { target });
    try {
      const tabs = await vi.waitFor(() => {
        const candidates = [...target.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
        expect(candidates).toHaveLength(8);
        return candidates;
      });
      expect(target.querySelector('[role="tablist"]')?.tagName).toBe("UL");
      expect([...target.querySelectorAll('[role="tablist"] li')]
        .every((item) => item.getAttribute("role") === "presentation")).toBe(true);
      expect(tabs.map((tab) => tab.tabIndex)).toEqual([0, -1, -1, -1, -1, -1, -1, -1]);

      tabs[0].focus();
      tabs[0].dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
      await tick();
      expect(document.activeElement).toBe(tabs[1]);
      expect(tabs[1].getAttribute("aria-selected")).toBe("true");
      expect(tabs[1].tabIndex).toBe(0);
      expect(tabs[0].tabIndex).toBe(-1);
      expect(target.querySelector('[role="tabpanel"]')?.getAttribute("aria-labelledby")).toBe(tabs[1].id);

      tabs[1].dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true }));
      await tick();
      expect(document.activeElement).toBe(tabs.at(-1));
      expect(tabs.at(-1)?.getAttribute("aria-selected")).toBe("true");
    } finally {
      unmount(component);
      target.remove();
      ui.closeSettings();
    }
  });

  it("moves focus to and announces an asynchronously prepared install plan", async () => {
    let resolvePlan!: (plan: AgentMutationPlan) => void;
    const planPromise = new Promise<AgentMutationPlan>((resolve) => { resolvePlan = resolve; });
    const plan: AgentMutationPlan = {
      revision: "focus-plan", operation: "install", tool: "claudeCode", scope: "user", projectPath: null,
      agents: [{
        reference: staleControlPackage.reference, name: "Reviewer", sourceHash: "source",
        dependency: false, destination: "/tmp/reviewer.md", renderedFileCount: 1, capabilities: [],
      }],
      warnings: [], blockers: [], rollbackAvailable: true,
    };
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "projects_list" || command === "installs_reconcile") return [] as never;
      if (command === "tools_list") return [staleControlTool] as never;
      if (command === "agent_install_plan") return planPromise as never;
      return [] as never;
    });
    install.installed = [];
    install.tools = [staleControlTool];
    install.reconciled = true;
    projects.list = [];
    const { default: InstallModal } = await import("$lib/components/InstallModal.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(InstallModal, {
      target,
      props: { title: "Install Reviewer", agentPackage: staleControlPackage, onClose: vi.fn() },
    });
    try {
      const toggle = await vi.waitFor(() => {
        const candidate = target.querySelector<HTMLButtonElement>(".grid-wrap .toggle");
        expect(candidate?.disabled).toBe(false);
        return candidate!;
      });
      toggle.focus();
      toggle.click();
      await vi.waitFor(() => expect(target.textContent).toContain("Building the exact mutation plan"));
      resolvePlan(plan);
      const planRegion = await vi.waitFor(() => {
        const candidate = target.querySelector<HTMLElement>('[data-install-plan]');
        expect(candidate).toBeTruthy();
        return candidate!;
      });
      expect(document.activeElement).toBe(planRegion);
      expect(target.querySelector('[data-plan-announcement]')?.textContent).toContain("Mutation plan ready");
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("keeps light brand buttons above 4.5:1 and reflows the app surfaces at 375px", () => {
    const tokensSource = readFileSync("src/lib/styles/tokens.css", "utf8");
    expect(tokensSource).toContain("#9a3412");
    const hex = tokensSource.match(/--color-brand:\s*(#[0-9a-f]{6})/i)?.[1];
    expect(hex).toBeTruthy();
    const channel = (value: number) => {
      const normalized = value / 255;
      return normalized <= 0.04045 ? normalized / 12.92 : ((normalized + 0.055) / 1.055) ** 2.4;
    };
    const rgb = hex!.slice(1).match(/.{2}/g)!.map((value) => Number.parseInt(value, 16));
    const luminance = 0.2126 * channel(rgb[0]) + 0.7152 * channel(rgb[1]) + 0.0722 * channel(rgb[2]);
    expect(1.05 / (luminance + 0.05)).toBeGreaterThanOrEqual(4.5);

    expect(pageSource).toMatch(/@media \(max-width: 600px\)[^]*?\.main :global\(\.handle\)[^]*?display:\s*none/);
    const drawerLayer = Number(pageSource.match(/@media \(max-width: 600px\)[^]*?\.main :global\(\.sidebar\)[^]*?z-index:\s*(\d+)/)?.[1]);
    const onboardingLayer = Number(catalogFirstRunSource.match(/\.scrim\s*\{[^}]*z-index:\s*(\d+)/)?.[1]);
    const themeMenuLayer = Number(titlebarControlsSource.match(/\.popover\s*\{[^}]*z-index:\s*(\d+)/)?.[1]);
    expect(drawerLayer).toBeGreaterThan(41);
    expect(onboardingLayer).toBeGreaterThan(drawerLayer);
    expect(themeMenuLayer).toBeGreaterThan(drawerLayer);
    expect(pageSource).toMatch(/@media \(max-width: 600px\)[^]*?\.app\.sidebar-collapsed \.main :global\(\.sidebar\)[^]*?display:\s*none/);
    expect(pageSource).toContain("const sidebarCollapsed = $derived(narrowViewport ? !mobileSidebarOpen : ui.sidebarCollapsed)");
    expect(sidebarSource).not.toContain("toggleSidebarCollapsed");
    expect(pageSource).toMatch(/@media \(max-width: 600px\)[^]*?\.titlebar-nav[^]*?display:\s*none/);
    expect(pageSource).toMatch(/@media \(max-width: 600px\)[^]*?\.titlebar-title[^]*?width:\s*1px[^]*?clip:\s*rect\(0, 0, 0, 0\)/);
    expect(pageSource).not.toMatch(/@media \(max-width: 600px\)[^}]*?\.titlebar-title[^}]*?display:\s*none/);
    expect(settingsSource).toMatch(/@media \(max-width: 600px\)[^]*?grid-template-columns:\s*minmax\(0, 1fr\)/);
    expect(activityHistorySource).toMatch(/@media \(max-width: 600px\)[^]*?\.review-list li[^]*?flex-direction:\s*column/);
    expect(activityHistorySource).toContain('role="group" aria-label="Activity mode"');
    expect(deploymentTargetGridSource).toContain('role="note" title={reason} aria-label={reason}');
    expect(installModalSource).toContain('color: var(--color-warning-strong)');
    expect(projectsSource).toMatch(/@media \(max-width: 600px\)[^]*?\.pr-head[^]*?flex-wrap:\s*wrap/);
    expect(agentsWorkspaceSource).toMatch(/@media \(max-width: 600px\)[^]*?\.lp-search-row[^]*?flex-wrap:\s*wrap/);
    expect(agentsWorkspaceSource).toMatch(/@media \(max-width: 600px\)[^]*?\.lp-search-row :global\(\.wrap\)[^]*?flex:\s*1 1 160px/);
  });

  it("delegates section navigation without changing the persisted collapse preference", async () => {
    localStorage.setItem("agency-agents:sidebar-collapsed", "0");
    ui.section = "dashboard";
    const navigate = vi.fn((section: SidebarSection) => ui.setSection(section));
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(Sidebar, { target, props: { collapsed: false, onNavigate: navigate } });
    try {
      const projectsButton = [...target.querySelectorAll<HTMLButtonElement>("nav button")]
        .find((button) => button.textContent?.includes("Projects"));
      expect(projectsButton).toBeTruthy();
      projectsButton!.click();
      await tick();
      expect(navigate).toHaveBeenCalledWith("projects");
      expect(ui.section).toBe("projects");
      expect(localStorage.getItem("agency-agents:sidebar-collapsed")).toBe("0");
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("uses right-to-left document direction for Persian", async () => {
    try {
      await i18n.setLocale("fa");
      expect(document.documentElement.dir).toBe("rtl");
    } finally {
      await i18n.setLocale("en");
    }
    expect(document.documentElement.dir).toBe("ltr");
  });

  it("summarizes only five or more exact comparable Expert quality verdicts", () => {
    const evidence = (id: string, checkName: string, result: "pass" | "fail" | "skipped") => ({
      id, idempotencyKey: id, checkName, result, commandLabel: null, summary: result,
      submittedAt: `2026-08-14T01:0${id.at(-1) ?? "0"}:00Z`,
    });
    const comparable = [
      performanceRun("run-1", "accepted", [evidence("e-1", "Tests", "fail"), evidence("e-2", "Tests", "pass"), evidence("e-3", "Review", "pass")]),
      performanceRun("run-2", "rework", [evidence("e-4", "Tests", "pass")], [{ checkName: "Review", reason: "Unavailable", createdAt: "2026-08-14T02:00:00Z" }]),
      performanceRun("run-3", "rejected", [evidence("e-5", "Tests", "fail"), evidence("e-6", "Review", "pass")]),
      performanceRun("run-4", "accepted", [evidence("e-7", "Tests", "pass"), evidence("e-8", "Review", "skipped")], [{ checkName: "Review", reason: "Unavailable", createdAt: "2026-08-14T02:00:00Z" }]),
      performanceRun("run-5", "accepted", [evidence("e-9", "Tests", "pass"), evidence("e-0", "Review", "pass")]),
    ];
    const excluded = [
      performanceRun("cancelled", "cancelled"),
      performanceRun("pending", "awaitingReview"),
      performanceRun("old-version", "accepted", [], [], { expertVersion: 1 }),
      performanceRun("old-contract", "accepted", [], [], { contract: { version: 2, checks: [] } }),
    ];

    const belowThreshold = summarizeExpertPerformance(performanceExpert(), comparable.slice(0, 4));
    expect(belowThreshold).toMatchObject({ comparableRuns: 4, eligible: false });
    expect(belowThreshold.acceptanceRate).toBeNull();
    expect(belowThreshold.suggestions).toEqual([]);

    const summary = summarizeExpertPerformance(performanceExpert(), [...comparable, ...excluded]);
    expect(summary).toMatchObject({
      comparableRuns: 5, eligible: true, accepted: 3, rework: 1, rejected: 1,
      acceptanceRate: 60, waiverRate: 40,
    });
    expect(summary.checks).toEqual([
      { name: "Tests", issueRuns: 1, waiverRuns: 0 },
      { name: "Review", issueRuns: 2, waiverRuns: 2 },
    ]);
    expect(summary.suggestions).toEqual([
      "2 of 5 runs ended in rework or rejection; review the Expert instructions and roster for recurring gaps.",
      "Review had missing, skipped, or failed evidence in 2 of 5 runs; review its instructions or tooling.",
      "Review was waived in 2 of 5 runs; clarify the check or improve its evidence path.",
    ]);
  });

  it("gates the Expert Improvement Coach until five comparable terminal runs", async () => {
    const expert = performanceExpert();
    const accepted = Array.from({ length: 5 }, (_, index) => performanceRun(`run-${index}`, "accepted", [
      { id: `tests-${index}`, idempotencyKey: `tests-${index}`, checkName: "Tests", result: "pass", commandLabel: null, summary: "Passed", submittedAt: "2026-08-14T02:00:00Z" },
      { id: `review-${index}`, idempotencyKey: `review-${index}`, checkName: "Review", result: "pass", commandLabel: null, summary: "Passed", submittedAt: "2026-08-14T02:00:00Z" },
    ]));
    let runs = accepted.slice(0, 4);
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "experts_list") return [expert] as never;
      if (command === "expert_runs_list") return runs as never;
      return [] as never;
    });
    const { default: Experts } = await import("$lib/components/Experts.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    let component = mount(Experts, { target });
    try {
      await vi.waitFor(() => expect(target.textContent).toContain("4 of 5 comparable terminal runs"));
      expect(target.textContent).not.toContain("Acceptance rate");
      unmount(component);
      target.replaceChildren();
      runs = accepted;
      component = mount(Experts, { target });
      await vi.waitFor(() => expect(target.textContent).toContain("Acceptance rate 100%"));
      expect(target.textContent).toContain("No recurring improvement signal was detected");
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("renders the canonical Doctor report, copies it, and routes every safe action without mutation", async () => {
    const report: DoctorReport = {
      generatedAt: "2026-08-13T12:00:00Z",
      overall: "needsAttention",
      counts: { healthy: 1, needsAttention: 1, unavailable: 6 },
      copyText: "canonical redacted report",
      checks: [
        { id: "storage", category: "core", title: "Storage", classification: "healthy", evidence: "SQLite complete", guidance: null, action: null },
        { id: "retry", category: "core", title: "Retry", classification: "unavailable", evidence: "Retry needed", guidance: "Retry Doctor.", action: "retryDoctor" },
        { id: "catalog", category: "library", title: "Catalog", classification: "needsAttention", evidence: "Missing", guidance: "Open Catalog.", action: "openCatalog" },
        { id: "agents", category: "library", title: "Agent sources", classification: "unavailable", evidence: "Unknown", guidance: "Open Agents.", action: "openAgents" },
        { id: "skills", category: "library", title: "Skill sources", classification: "unavailable", evidence: "Unknown", guidance: "Open Skills.", action: "openSkills" },
        { id: "tools", category: "tools", title: "Tools", classification: "unavailable", evidence: "None", guidance: "Open Tools.", action: "openTools" },
        { id: "mcp", category: "integrations", title: "MCP", classification: "unavailable", evidence: "None", guidance: "Open MCP.", action: "openMcp" },
        { id: "updates", category: "updates", title: "Updates", classification: "unavailable", evidence: "No cache", guidance: "Open Network.", action: "openNetwork" },
      ],
    };
    vi.mocked(invoke).mockResolvedValue(report as never);
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal("navigator", { ...navigator, clipboard: { writeText } });
    const { default: SettingsSectionDoctor } = await import("$lib/components/SettingsSectionDoctor.svelte");
    const target = document.createElement("div");
    const main = document.createElement("main");
    const workspaceHeading = document.createElement("h2");
    main.append(workspaceHeading);
    document.body.append(main);
    document.body.append(target);
    const component = mount(SettingsSectionDoctor, { target });
    try {
      await vi.waitFor(() => expect(target.textContent).toContain("SQLite complete"));
      expect(target.textContent).toContain("Healthy 1");
      expect(target.textContent).toContain("Needs attention 1");
      expect(target.textContent).toContain("Unavailable 6");
      expect(target.querySelectorAll("[data-doctor-category]").length).toBeGreaterThanOrEqual(5);
      for (const category of ["Core", "Library", "Tools", "Integrations", "Updates"])
        expect(target.textContent).toContain(category);

      target.querySelector<HTMLButtonElement>('[data-doctor-copy]')!.click();
      await vi.waitFor(() => expect(writeText).toHaveBeenCalledWith(report.copyText));
      await vi.waitFor(() => expect(target.querySelector('[role="status"]')?.textContent).toContain("copied"));

      const action = (name: string) => target.querySelector<HTMLButtonElement>(`[data-doctor-action="${name}"]`)!;
      ui.settingsOpen = true;
      action("openCatalog").click();
      expect(ui.settingsInitialSection).toBe("catalog");
      action("openMcp").click();
      expect(ui.settingsInitialSection).toBe("mcp");
      action("openNetwork").click();
      expect(ui.settingsInitialSection).toBe("network");
      action("openAgents").click();
      expect(ui.section).toBe("personas");
      expect(ui.agentsLens).toBe("attention");
      await vi.waitFor(() => expect(document.activeElement).toBe(workspaceHeading));
      ui.settingsOpen = true;
      action("openSkills").click();
      expect(ui.section).toBe("skills");
      ui.settingsOpen = true;
      action("openTools").click();
      expect(ui.section).toBe("tools");

      const callsBeforeRetry = vi.mocked(invoke).mock.calls.length;
      action("retryDoctor").click();
      await vi.waitFor(() => expect(vi.mocked(invoke).mock.calls.length).toBe(callsBeforeRetry + 1));
      expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "doctor_report")).toHaveLength(2);
    } finally {
      unmount(component);
      target.remove();
      main.remove();
    }
  });

  it("keeps stale Doctor evidence during one refresh and reports retryable global and copy failures", async () => {
    const report: DoctorReport = {
      generatedAt: "2026-08-13T12:00:00Z", overall: "unavailable",
      counts: { healthy: 0, needsAttention: 0, unavailable: 1 }, copyText: "safe",
      checks: [{ id: "catalog", category: "library", title: "Catalog", classification: "unavailable", evidence: "Cached local evidence", guidance: "Retry.", action: "retryDoctor" }],
    };
    let resolveRefresh: ((value: DoctorReport) => void) | undefined;
    let doctorCalls = 0;
    vi.mocked(invoke).mockImplementation((command: string) => {
      if (command !== "doctor_report") return Promise.resolve([] as never);
      doctorCalls += 1;
      if (doctorCalls === 1) return Promise.resolve(report as never);
      if (doctorCalls === 2) return new Promise((resolve) => { resolveRefresh = resolve as (value: DoctorReport) => void; });
      return Promise.reject({ code: "internal", message: "doctor unavailable" });
    });
    const writeText = vi.fn().mockRejectedValue(new Error("clipboard denied"));
    vi.stubGlobal("navigator", { ...navigator, clipboard: { writeText } });
    const { default: SettingsSectionDoctor } = await import("$lib/components/SettingsSectionDoctor.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(SettingsSectionDoctor, { target });
    try {
      await vi.waitFor(() => expect(target.textContent).toContain("Cached local evidence"));
      const refresh = target.querySelector<HTMLButtonElement>('[data-doctor-refresh]')!;
      refresh.click();
      await vi.waitFor(() => expect(resolveRefresh).toBeTypeOf("function"));
      expect(refresh.disabled).toBe(true);
      expect(target.textContent).toContain("prior evidence");
      expect(target.textContent).toContain("Cached local evidence");
      refresh.click();
      expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "doctor_report")).toHaveLength(2);
      resolveRefresh?.(report);
      await vi.waitFor(() => expect(refresh.disabled).toBe(false));
      refresh.click();
      await vi.waitFor(() => expect(target.querySelector('[role="alert"]')?.textContent).toContain("doctor unavailable"));
      expect(target.textContent).toContain("Cached local evidence");
      expect(target.textContent).not.toContain("All checks are healthy");

      target.querySelector<HTMLButtonElement>('[data-doctor-copy]')!.click();
      await vi.waitFor(() => expect(target.querySelector('[role="alert"]')?.textContent).toContain("Could not copy"));
      expect(target.querySelector('[aria-live="polite"]')).not.toBeNull();
      expect(refresh.getAttribute("aria-keyshortcuts")).toBe("Enter Space");
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("restores exact Agent and Skill recommendation navigation", () => {
    const agent = { sourceId: "source-b", relativePath: "nested/reviewer.md" };
    const skill = { sourceId: "skills-b", relativePath: "nested/reviewer" };
    const nav = ui as unknown as {
      openAgentReference(reference: typeof agent): void;
      openSkill(reference: typeof skill): void;
      agentsReference: typeof agent | null;
      skillsSelected: typeof skill | null;
    };

    ui.navStack = [];
    ui.navIndex = -1;
    nav.openAgentReference(agent);
    nav.openSkill(skill);
    ui.back();
    expect(ui.section).toBe("personas");
    expect(nav.agentsReference).toEqual(agent);
    ui.forward();
    expect(ui.section).toBe("skills");
    expect(nav.skillsSelected).toEqual(skill);
  });

  it("opens an exact recommendation without invoking mutation commands", async () => {
    const recommendation = skillRecommendation();
    vi.mocked(invoke).mockImplementation(async (command: string) =>
      command === "task_recommendations" ? [recommendation] as never : [] as never);
    ui.openPalette();
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(CommandPalette, { target });
    try {
      await tick();
      const input = target.querySelector<HTMLInputElement>("input")!;
      input.value = "review rust";
      input.dispatchEvent(new Event("input", { bubbles: true }));
      await vi.waitFor(() => expect(target.querySelectorAll('[role="option"]')).toHaveLength(1));
      target.querySelector<HTMLButtonElement>('[role="option"]')!.click();
      expect(ui.section).toBe("skills");
      expect((ui as unknown as { skillsSelected: unknown }).skillsSelected)
        .toEqual({ sourceId: "skills-b", relativePath: "nested/reviewer" });
      expect(vi.mocked(invoke).mock.calls.map(([command]) => command))
        .toEqual(["task_recommendations"]);
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("distinguishes duplicate Agent names and opens the keyboard-selected exact source", async () => {
    const recommendations = ["source-a", "source-b"].map((sourceId) => ({
      kind: "agent" as const,
      package: { ...staleControlPackage, reference: { sourceId, relativePath: "reviewer.md" } },
      score: 4,
      reasons: ["task:name:reviewer"],
    }));
    vi.mocked(invoke).mockImplementation(async (command: string) =>
      command === "task_recommendations" ? recommendations as never : [] as never);
    ui.openPalette();
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(CommandPalette, { target });
    try {
      await tick();
      const input = target.querySelector<HTMLInputElement>("input")!;
      input.value = "review agent";
      input.dispatchEvent(new Event("input", { bubbles: true }));
      await vi.waitFor(() => expect(target.querySelectorAll('[role="option"]')).toHaveLength(2));
      expect(target.textContent).toContain("source-a · reviewer.md");
      expect(target.textContent).toContain("source-b · reviewer.md");
      input.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
      input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      expect(ui.section).toBe("personas");
      expect(ui.agentsReference).toEqual({ sourceId: "source-b", relativePath: "reviewer.md" });
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("keeps command, Agent, and Skill groups together and restores focus on Escape", async () => {
    vi.mocked(invoke).mockImplementation(async (command: string) => command === "task_recommendations"
      ? [
          { kind: "agent", package: staleControlPackage, score: 4, reasons: ["task:name:reviewer"] },
          skillRecommendation(),
        ] as never
      : [] as never);
    const trigger = document.createElement("button");
    document.body.append(trigger);
    trigger.focus();
    ui.openPalette();
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(CommandPalette, { target });
    try {
      await tick();
      const input = target.querySelector<HTMLInputElement>("input")!;
      input.value = "open agents";
      input.dispatchEvent(new Event("input", { bubbles: true }));
      await vi.waitFor(() => expect(target.querySelectorAll('[role="option"]')).toHaveLength(3));
      expect(target.textContent).toContain("Commands");
      expect(target.textContent).toContain("Agents");
      expect(target.textContent).toContain("Skills");
      input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      await vi.waitFor(() => expect(document.activeElement).toBe(trigger));
      expect(ui.paletteOpen).toBe(false);
    } finally {
      unmount(component);
      target.remove();
      trigger.remove();
    }
  });

  it("debounces task search, ignores stale results, and preserves commands on failure", async () => {
    let resolveOld: ((value: unknown) => void) | undefined;
    let resolveNew: ((value: unknown) => void) | undefined;
    vi.mocked(invoke).mockImplementation((command: string, args?: unknown) => {
      if (command !== "task_recommendations") return Promise.resolve([] as never);
      const task = (args as { task: string }).task;
      if (task === "old query") return new Promise((resolve) => { resolveOld = resolve; });
      if (task === "new query") return new Promise((resolve) => { resolveNew = resolve; });
      return Promise.reject({ code: "internal", message: "offline index" });
    });
    ui.openPalette();
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(CommandPalette, { target });
    try {
      await tick();
      const input = target.querySelector<HTMLInputElement>("input")!;
      input.value = "ab";
      input.dispatchEvent(new Event("input", { bubbles: true }));
      await new Promise((resolve) => setTimeout(resolve, 220));
      expect(vi.mocked(invoke)).not.toHaveBeenCalled();

      input.value = "old query";
      input.dispatchEvent(new Event("input", { bubbles: true }));
      await vi.waitFor(() => expect(resolveOld).toBeTypeOf("function"));
      input.value = "new query";
      input.dispatchEvent(new Event("input", { bubbles: true }));
      await vi.waitFor(() => expect(resolveNew).toBeTypeOf("function"));
      resolveNew?.([skillRecommendation("new-source")]);
      await vi.waitFor(() => expect(target.textContent).toContain("new-source"));
      resolveOld?.([skillRecommendation("old-source")]);
      await tick();
      expect(target.textContent).not.toContain("old-source");

      input.value = "open agents";
      input.dispatchEvent(new Event("input", { bubbles: true }));
      await vi.waitFor(() => expect(target.textContent).toContain("Recommendations unavailable"));
      expect(target.textContent).toContain("Open Agents");
      expect(target.querySelector("[role=status]")?.textContent).toContain("offline index");
      expect(input.maxLength).toBe(2048);
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("bounds and redacts Activity failure details", () => {
    const detail = safeActivityDetail(
      `failed token=secret123 Authorization: Bearer abc private key ${"x".repeat(700)}`,
    );
    expect(detail.length).toBeLessThanOrEqual(512);
    expect(detail).not.toContain("secret123");
    expect(detail).not.toContain("Bearer abc");
    expect(detail).not.toContain("private key");
    expect(detail).toContain("[redacted]");
    expect(safeActivityDetail("https://user:password@example.test ghp_abcdefghijklmnopqrstuvwxyz sk-secretvalue"))
      .toBe("https://[redacted]@example.test [redacted] [redacted]");
  });

  it("formats AppError Activity details before redaction and bounding", () => {
    expect(safeActivityDetail({ code: "io", message: "scan failed" }))
      .toBe("I/O error: scan failed");

    const detail = safeActivityDetail({
      code: "io",
      message: `token=secret123 ${"x".repeat(700)}`,
    });
    expect(detail).toHaveLength(512);
    expect(detail).not.toContain("secret123");
    expect(detail).toContain("token=[redacted]");
  });

  it("normalizes exact Activity receipts and preserves additive v2 entries", () => {
    const legacy: JournalEntry = {
      id: "legacy", ts: "2026-08-14T01:00:00Z", action: "bulk", outcome: "ok", detail: "Installed 1 agent",
    };
    const normalized = normalizeActivityReceipt({
      operation: "update",
      succeeded: 99,
      failed: 99,
      items: [
        { kind: "agent", name: "Reviewer", destination: `/tmp/\0reviewer-${"x".repeat(5000)}`, outcome: "ok" },
        { kind: "skill", name: "Audit", destination: "/tmp/audit", outcome: "error", detail: "token=secret123\nfailed" },
      ],
    });
    expect(normalized).toMatchObject({ operation: "update", succeeded: 1, failed: 1 });
    expect(normalized?.items).toHaveLength(2);
    expect(normalized?.items[0]?.destination).not.toContain("\0");
    expect(normalized?.items[0]?.destination?.length).toBeLessThanOrEqual(4096);
    expect(normalized?.items[1]?.detail).toContain("token=[redacted]");
    expect(normalized?.items[1]?.detail).not.toContain("secret123");
    expect(normalized?.items[1]?.detail).not.toContain("\n");

    const restored = normalizePersistedActivityEntries([
      legacy,
      { ...legacy, id: "receipt", receipt: normalized },
      { ...legacy, id: "invalid", receipt: { operation: "execute", items: [] } },
    ]);
    expect(restored[0]).toEqual(legacy);
    expect(restored[1]?.receipt).toEqual(normalized);
    expect(restored[2]?.receipt).toBeUndefined();
  });

  it("returns the generated id for one normalized Activity receipt", () => {
    const id = activity.log({
      action: "bulk",
      outcome: "error",
      detail: "1 repaired · 1 failed",
      receipt: {
        operation: "repair",
        succeeded: 0,
        failed: 0,
        items: [
          { kind: "agent", name: "Reviewer", destination: "/tmp/reviewer.md", outcome: "ok" },
          { kind: "skill", name: "Audit", destination: "/tmp/audit", outcome: "error", detail: "failed" },
        ],
      },
    });
    expect(id).toEqual(expect.any(String));
    expect(activity.entries.find((entry) => entry.id === id)?.receipt).toMatchObject({
      operation: "repair", succeeded: 1, failed: 1,
    });
  });

  it("reviews then applies one mixed Workspace Pack and records exact retained results", async () => {
    const plan: WorkspacePackPlan = {
      pack: {
        workspacePack: 1, name: "Review workspace", scope: "project",
        agents: [{ reference: { sourceId: "agents", relativePath: "reviewer.md" }, tool: "codex" }],
        skills: [{ reference: { sourceId: "skills", relativePath: "audit" }, runtime: "codex" }],
        runbook: "review-flow", instructions: ["Follow AGENTS.md"], mcpServers: ["memory"],
      },
      projectPath: "/project",
      agents: [{ reference: { sourceId: "agents", relativePath: "reviewer.md" }, name: "Reviewer", tool: "codex", destinations: ["/project/.codex/agents/reviewer.toml"], dependency: false, state: "missing" }],
      skills: [{ reference: { sourceId: "skills", relativePath: "audit" }, name: "Audit", runtime: "codex", destinations: ["/project/.agents/skills/audit"], dependency: false, state: "current", permissions: [] }],
      warnings: ["MCP requirements are declarative"], blockers: [],
      rollbackScope: ["/project/.codex/agents/reviewer.toml"], revision: "revision-1",
    };
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "loadout_import") return plan as never;
      if (command === "loadout_apply") return {
        plan,
        result: {
          revision: plan.revision, outcome: "succeeded", rolledBack: false, rollbackErrors: [],
          items: [
            { kind: "agent", sourceId: "agents", relativePath: "reviewer.md", target: "codex", destination: "/project/.codex/agents/reviewer.toml", outcome: "installed", message: null },
            { kind: "skill", sourceId: "skills", relativePath: "audit", target: "codex", destination: "/project/.agents/skills/audit", outcome: "current", message: "token=secret123 retained" },
          ],
        },
      } as never;
      if (["installs_reconcile", "tools_list", "skill_installs_reconcile", "skill_backups_list"].includes(command)) return [] as never;
      return [] as never;
    });
    const inspected = await install.inspectWorkspacePack("/tmp/review.json", "/project");
    expect(inspected).toEqual(plan);
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("loadout_import", { path: "/tmp/review.json", projectPath: "/project" });
    const applied = await install.applyWorkspacePack("/tmp/review.json", "/project", plan, ["/project"]);
    expect(applied.response.result?.items.map((item) => item.outcome)).toEqual(["installed", "current"]);
    expect(applied.response.result?.items[1]?.message).toContain("token=[redacted]");
    expect(applied.response.result?.items[1]?.message).not.toContain("secret123");
    const receipt = activity.entries.find((entry) => entry.id === applied.receiptId)?.receipt;
    expect(receipt).toMatchObject({ operation: "install", succeeded: 2, failed: 0 });
    expect(receipt?.items.map((item) => [item.kind, item.destination])).toEqual([
      ["agent", "/project/.codex/agents/reviewer.toml"],
      ["skill", "/project/.agents/skills/audit"],
    ]);
    expect(vi.mocked(invoke).mock.calls.map(([command]) => command)).toContain("skill_installs_reconcile");
  });

  it("keeps Workspace Pack review, approval, passive requirements, focus, and Activity handoff in Teams", () => {
    for (const marker of [
      /const capturedPath = picked/,
      /install\.inspectWorkspacePack\(capturedPath, null\)/,
      /install\.applyWorkspacePack\(capturedPath, capturedProject, reviewedPlan, registeredProjects\)/,
      /packPlan\.pack\.instructions\.length[^]*?not applied automatically/,
      /packPlan\.pack\.mcpServers\.length[^]*?not configured automatically/,
      /reviewedPack\.pack\.scope === "user"[^]*?disabled=\{!packTruthFresh \|\| reviewedPack\.blockers\.length > 0\}/,
      /role="status" aria-live="polite"/,
      /packTrigger\?\.focus\(\{ preventScroll: true \}\)/,
      /ui\.openActivityReceipt\(receiptId\)/,
    ]) expect(teamsSource).toMatch(marker);
    expect(teamsSource).not.toContain("install.importLoadout(");
  });

  it("keeps project readiness opt-in, evidence, announcements, and exact-reference review in existing surfaces", () => {
    for (const marker of [
      /projects\.readiness\(projectPath\)/,
      /projects\.recommendations\(projectPath\)/,
      /Import Workspace Pack baseline/,
      /type="checkbox" checked=\{readiness\.subscribed\}/,
      /role="status" aria-live="polite" aria-atomic="true"/,
      /await agentLibrary\.load\(true\)/,
      /agentPackage=\{recommendationPackage\}/,
      /reviewIntent=\{\{/,
      /recommendation\.changeKind === "removed"/,
      /Recommendation could not be dismissed/,
      /trigger\?\.focus\(\{ preventScroll: true \}\)/,
    ]) expect(projectsSource).toMatch(marker);
    for (const marker of [
      /projects\.saveTeamBaseline\(projectPath, teamLabel, requirements, subscribe\)/,
      /baselineRequirements\.map/,
      /Save targets and subscribe/,
      /role="status" aria-live="polite" aria-atomic="true"/,
    ]) expect(teamsSource).toMatch(marker);
    expect(projectsSource).not.toContain("autoApply");
    expect(teamsSource).not.toContain("autoApply");
  });

  it("blocks a partial Team baseline without saving or subscribing", async () => {
    const { default: Teams } = await import("$lib/components/Teams.svelte");
    const projectPath = "/tmp/project";
    const reviewer = { ...staleControlAgent };
    const writer = { ...staleControlAgent, slug: "writer", name: "Writer" };
    const deployedReviewer: InstalledAgent = {
      ...staleControlRow,
      slug: reviewer.slug,
      name: reviewer.name,
      sourceId: "built-in",
      relativePath: "engineering/reviewer.md",
      projectPath,
      scope: "project",
      state: "current",
      tracked: true,
    };
    const projectRows = [{ path: projectPath, label: "Project", installedCount: 1 }];
    teams.saved = [{
      id: "partial",
      name: "Partial Team",
      agents: [reviewer.slug, writer.slug],
      createdAt: "2026-08-17T00:00:00Z",
    }];
    corpus.agents = [reviewer, writer];
    corpus.categories = [{ slug: "engineering", label: "Engineering", color: "#2563eb", icon: "Code", count: 2 }];
    install.installed = [deployedReviewer];
    install.reconciled = true;
    projects.list = projectRows;
    ui.teamsSelected = "saved:partial";
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "installs_reconcile") return [deployedReviewer] as never;
      if (command === "tools_list") return [staleControlTool] as never;
      if (command === "skill_installs_reconcile" || command === "skill_backups_list") return [] as never;
      if (command === "projects_list") return projectRows as never;
      if (command === "project_baseline_save_team") return readinessFixture(projectPath).baseline as never;
      return [] as never;
    });
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(Teams, { target });
    try {
      const presetsTab = [...target.querySelectorAll<HTMLButtonElement>('[role="tab"]')]
        .find((button) => button.textContent?.includes("Team presets"))!;
      presetsTab.click();
      await vi.waitFor(() => expect(target.textContent).toContain("Partial Team"));
      const project = target.querySelector<HTMLSelectElement>(".team-baseline select")!;
      project.value = projectPath;
      project.dispatchEvent(new Event("change", { bubbles: true }));
      await tick();

      const error = target.querySelector<HTMLElement>('.team-baseline [role="alert"]');
      expect(error?.textContent).toContain("1 Team member");
      expect(error?.textContent).toContain("Writer");
      const subscribe = [...target.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.includes("Save targets and subscribe"))!;
      expect(subscribe.disabled).toBe(true);
      subscribe.click();
      await tick();
      expect(invokeMock.mock.calls.some(([command]) => command === "project_baseline_save_team")).toBe(false);
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("keeps a completed receipt in memory when journal persistence fails", () => {
    vi.useFakeTimers();
    const warning = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const setItem = vi.spyOn(localStorage, "setItem").mockImplementation(() => {
      throw new Error("quota exceeded");
    });
    try {
      const id = activity.log({
        action: "bulk",
        outcome: "ok",
        receipt: {
          operation: "repair",
          succeeded: 1,
          failed: 0,
          items: [{ kind: "agent", name: "Reviewer", destination: "/tmp/reviewer.md", outcome: "ok" }],
        },
      });
      vi.advanceTimersByTime(400);
      expect(activity.entries.find((entry) => entry.id === id)?.receipt?.items).toHaveLength(1);
      expect(setItem).toHaveBeenCalledOnce();
      expect(warning).toHaveBeenCalledWith(expect.stringContaining("persistNow failed"));
    } finally {
      setItem.mockRestore();
      warning.mockRestore();
      vi.useRealTimers();
    }
  });

  it("opens, discloses, and focuses the exact Activity receipt with locale fallback", async () => {
    const id = activity.log({
      action: "bulk",
      outcome: "error",
      detail: "1 updated · 1 failed",
      receipt: {
        operation: "update",
        succeeded: 1,
        failed: 1,
        items: [
          { kind: "agent", name: "Reviewer", destination: "/tmp/reviewer.md", outcome: "ok" },
          { kind: "skill", name: "Audit", destination: "/tmp/audit", outcome: "error", detail: "permission denied" },
        ],
      },
    });
    await i18n.setLocale("fr");
    ui.openActivityReceipt(id);
    expect(ui.section).toBe("activity");
    expect(ui.activityReceiptId).toBe(id);

    const { default: ActivityHistory } = await import("$lib/components/ActivityHistory.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(ActivityHistory, { target });
    try {
      const details = await vi.waitFor(() => {
        const value = target.querySelector<HTMLDetailsElement>(`details[data-activity-id="${id}"]`);
        expect(value).not.toBeNull();
        expect(value?.open).toBe(true);
        return value!;
      });
      const summary = details.querySelector<HTMLElement>("summary")!;
      expect(document.activeElement).toBe(summary);
      expect(summary.textContent).toContain("Receipt details");
      expect(details.textContent).toContain("Reviewer");
      expect(details.textContent).toContain("/tmp/reviewer.md");
      expect(details.textContent).toContain(i18n.t("common.succeeded"));
      expect(details.textContent).toContain(i18n.t("common.failed"));
      expect(details.textContent).toContain("permission denied");
    } finally {
      unmount(component);
      target.remove();
      await i18n.setLocale("en");
    }
  });

  it("opens Activity and announces an expired receipt without focusing another row", async () => {
    ui.openActivityReceipt("receipt-no-longer-retained");
    const { default: ActivityHistory } = await import("$lib/components/ActivityHistory.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(ActivityHistory, { target });
    try {
      await vi.waitFor(() => {
        expect(target.querySelector('[role="status"]')?.textContent).toContain("Receipt is no longer available");
      });
      expect(ui.section).toBe("activity");
      expect(ui.activityReceiptId).toBeNull();
      expect(target.querySelector("details[open]")).toBeNull();
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("switches an already-mounted Activity Review to History before opening an exact receipt", async () => {
    const id = activity.log({
      action: "bulk",
      outcome: "ok",
      receipt: {
        operation: "repair",
        succeeded: 1,
        failed: 0,
        items: [{ kind: "agent", name: "Reviewer", destination: "/tmp/reviewer.md", outcome: "ok" }],
      },
    });
    ui.section = "activity";
    const { default: ActivityHistory } = await import("$lib/components/ActivityHistory.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(ActivityHistory, { target });
    try {
      const review = [...target.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.trim() === "Review")!;
      const history = [...target.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.trim() === "History")!;
      expect(review.getAttribute("aria-pressed")).toBe("true");
      ui.openActivityReceipt(id);
      await vi.waitFor(() => {
        const details = target.querySelector<HTMLDetailsElement>(`details[data-activity-id="${id}"]`);
        expect(history.getAttribute("aria-pressed")).toBe("true");
        expect(details?.open).toBe(true);
        expect(document.activeElement).toBe(details?.querySelector("summary"));
      });
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("bounds structured return intents, rejects unrelated owners, and clears them on local navigation", () => {
    ui.section = "activity";
    ui.navStack = [];
    ui.navIndex = -1;
    ui.initNav();
    ui.openAgentApproval("agent-exact", "agent:agent-exact");
    expect(ui.reviewIntent).toMatchObject({ kind: "agent", exactId: "agent-exact", triggerId: "agent:agent-exact" });
    expect(ui.returnToActivityReview("skill", "agent-exact")).toBe(false);
    expect(ui.returnToActivityReview("agent", "another-agent")).toBe(false);
    expect(ui.section).toBe("personas");
    ui.setSection("tools");
    expect(ui.reviewIntent).toBeNull();

    const section = ui.section;
    ui.openSkillApproval("x".repeat(2049), "skill:oversized");
    expect(ui.section).toBe(section);
    expect(ui.reviewIntent).toBeNull();

    ui.openProjectRecommendation("/tmp/project-a", "same-id", "recommendation:same-id");
    expect(ui.isReviewIntent("project", ui.projectReviewExactId("/tmp/project-b", "same-id"))).toBe(false);
    expect(ui.isReviewIntent("project", ui.projectReviewExactId("/tmp/project-a", "same-id"))).toBe(true);
  });

  it("falls back safely when exact Agent, Skill, every Expert, or Project review target disappeared", async () => {
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "agent_library_list" || command === "skill_folders_list") return emptyFolderState() as never;
      if (["expert_creation_requests", "expert_runs_list", "expert_activation_requests", "projects_list"].includes(command)) return [] as never;
      return [] as never;
    });
    ui.section = "activity";
    ui.navStack = [];
    ui.navIndex = -1;
    ui.initNav();
    const { default: ActivityHistory } = await import("$lib/components/ActivityHistory.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(ActivityHistory, { target });
    try {
      await vi.waitFor(() => expect(target.textContent).toContain("Subscription Recommendations 0"));
      const cases = [
        { kind: "agent" as const, exactId: "missing-agent", group: "agent", open: () => ui.openAgentApproval("missing-agent", "agent:missing-agent") },
        { kind: "skill" as const, exactId: "missing-skill", group: "skill", open: () => ui.openSkillApproval("missing-skill", "skill:missing-skill") },
        { kind: "expert-change" as const, exactId: "missing-change", group: "expert-change", open: () => ui.openExpertReview("change", "missing-change", "expert-change:missing-change") },
        { kind: "expert-run" as const, exactId: "missing-run", group: "expert-run", open: () => ui.openExpertReview("run", "missing-run", "expert-run:missing-run") },
        { kind: "expert-activation" as const, exactId: "missing-activation", group: "expert-activation", open: () => ui.openExpertReview("activation", "missing-activation", "expert-activation:missing-activation") },
        {
          kind: "project" as const,
          exactId: ui.projectReviewExactId("/tmp/missing-project", "missing-project-review"),
          group: "recommendation",
          open: () => ui.openProjectRecommendation("/tmp/missing-project", "missing-project-review", "recommendation:missing-project-review"),
        },
      ];
      for (const item of cases) {
        item.open();
        expect(ui.returnToActivityReview(item.kind, item.exactId)).toBe(true);
        await vi.waitFor(() => {
          const group = target.querySelector<HTMLElement>(`[data-review-group="${item.group}"]`)!;
          expect(group.contains(document.activeElement)).toBe(true);
          expect(target.querySelector('[role="status"]')?.textContent).toContain("Review item is no longer available");
          expect(ui.reviewIntent).toBeNull();
        });
      }
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it.each([
    ["agent", "missing-agent-owner"],
    ["skill", "missing-skill-owner"],
    ["expert-change", "missing-change-owner"],
    ["expert-run", "missing-run-owner"],
    ["expert-activation", "missing-activation-owner"],
    ["project", "missing-project-owner"],
  ] as const)("returns safely when the %s owner cannot find its exact target", async (kind, id) => {
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "agent_library_list" || command === "skill_folders_list") return emptyFolderState() as never;
      if (command === "project_readiness_get") return readinessFixture("/tmp/missing-owner", true) as never;
      if (["agent_sources_inspect", "agent_drafts_list", "skill_sources_inspect", "skill_drafts_list",
        "experts_list", "expert_creation_requests", "expert_runs_list", "expert_activation_requests",
        "expert_activation_history", "projects_list", "project_recommendations_list", "installs_reconcile",
        "skill_installs_reconcile", "skill_backups_list", "project_instructions_inspect"].includes(command)) return [] as never;
      return [] as never;
    });
    ui.section = "activity";
    ui.navStack = [];
    ui.navIndex = -1;
    ui.initNav();
    let ownerName: "AgentsWorkspace" | "SkillsWorkspace" | "Experts" | "Projects";
    if (kind === "agent") { ui.openAgentApproval(id, `agent:${id}`); ownerName = "AgentsWorkspace"; }
    else if (kind === "skill") { ui.openSkillApproval(id, `skill:${id}`); ownerName = "SkillsWorkspace"; }
    else if (kind === "project") {
      ui.openProjectRecommendation("/tmp/missing-owner", id, `recommendation:${id}`);
      ownerName = "Projects";
    } else {
      ui.openExpertReview(kind.slice("expert-".length) as "change" | "run" | "activation", id, `${kind}:${id}`);
      ownerName = "Experts";
    }
    const Owner = (await import(`$lib/components/${ownerName}.svelte`)).default;
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(Owner, { target });
    try {
      await vi.waitFor(() => expect(ui.section).toBe("activity"));
      expect(ui.reviewIntent).toMatchObject({ kind, exactId: kind === "project" ? ui.projectReviewExactId("/tmp/missing-owner", id) : id });
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it.each(["agent", "skill"] as const)("returns safely when the %s rollback owner cannot find its exact install", async (kind) => {
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "agent_library_list" || command === "skill_folders_list") return emptyFolderState() as never;
      if (["agent_sources_inspect", "agent_drafts_list", "skill_sources_inspect", "skill_drafts_list",
        "projects_list", "installs_reconcile", "skill_installs_reconcile", "skill_backups_list"].includes(command)) return [] as never;
      return [] as never;
    });
    ui.settingsOpen = false;
    ui.section = "activity";
    ui.navStack = [];
    ui.navIndex = -1;
    ui.initNav();
    const link = kind === "agent"
      ? { reference: { sourceId: "missing", relativePath: "agent.md" }, tool: "claudeCode" as const, projectPath: null }
      : { reference: { sourceId: "missing", relativePath: "skill" }, runtime: "codex" as const, projectPath: null };
    if (kind === "agent") ui.openAgentRecovery(link as Parameters<typeof ui.openAgentRecovery>[0], "agent-recovery-missing-owner");
    else ui.openSkillRecovery(link as Parameters<typeof ui.openSkillRecovery>[0], "skill-recovery-missing-owner");
    const Owner = kind === "agent"
      ? (await import("$lib/components/AgentsWorkspace.svelte")).default
      : (await import("$lib/components/SkillsWorkspace.svelte")).default;
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(Owner, { target });
    try {
      await vi.waitFor(() => expect(ui.settingsOpen).toBe(true));
      expect(ui.recoveryIntent).toMatchObject({ kind, exactId: ui.recoveryExactId(kind, link) });
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it.each(["agent", "skill"] as const)("falls back safely when an exact %s Recovery target disappeared", async (kind) => {
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "doctor_report") return {
        generatedAt: "2026-08-17T00:00:00Z", overall: "healthy",
        counts: { healthy: 0, needsAttention: 0, unavailable: 0 }, copyText: "healthy", checks: [],
      } as never;
      if (command === "storage_migration_status") return { state: "complete" } as never;
      if (["installs_reconcile", "skill_installs_reconcile", "projects_list"].includes(command)) return [] as never;
      return [] as never;
    });
    ui.settingsOpen = true;
    ui.settingsInitialSection = "doctor";
    const { default: SettingsSectionDoctor } = await import("$lib/components/SettingsSectionDoctor.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(SettingsSectionDoctor, { target });
    try {
      await vi.waitFor(() => expect(target.textContent).toContain("Showing 0 of 0 installs checked"));
      const link = kind === "agent"
        ? { reference: { sourceId: "missing", relativePath: "agent.md" }, tool: "claudeCode" as const, projectPath: null }
        : { reference: { sourceId: "missing", relativePath: "skill" }, runtime: "codex" as const, projectPath: null };
      if (kind === "agent") ui.openAgentRecovery(link as Parameters<typeof ui.openAgentRecovery>[0], "agent-recovery-missing");
      else ui.openSkillRecovery(link as Parameters<typeof ui.openSkillRecovery>[0], "skill-recovery-missing");
      const exactId = ui.recoveryExactId(kind, link);
      expect(ui.returnToSettingsRecovery(kind, exactId)).toBe(true);
      await vi.waitFor(() => {
        const group = target.querySelector<HTMLElement>(`[data-recovery-source="${kind === "agent" ? "agents" : "skills"}"]`)!;
        expect(group.contains(document.activeElement)).toBe(true);
        expect(target.querySelector('.recovery [role="status"]')?.textContent).toContain("Recovery item is no longer available");
        expect(ui.recoveryIntent).toBeNull();
      });
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("aggregates five review groups, separates recommendations, and deep-links without mutation authority", async () => {
    const projectPath = "/private/work/review-project";
    const recommendation = updatedProjectRecommendation(projectPath);
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "agent_library_list") return {
        ...emptyFolderState(),
        approvals: [{
          id: "agent-approval", submittedAt: "2026-08-17T01:00:00Z", state: "pending",
          requestedBy: "codex", request: { action: "draftPublish", id: "agent-draft", planRevision: "agent-revision" }, result: null,
        }],
      } as never;
      if (command === "skill_folders_list") return {
        ...emptyFolderState(),
        approvals: [{
          id: "skill-approval", submittedAt: "2026-08-17T02:00:00Z", state: "pending",
          requestedBy: "claude", request: { action: "draftPublish", id: "skill-draft", planRevision: "skill-revision" }, result: null,
        }],
      } as never;
      if (command === "expert_creation_requests") return [{
        id: "expert-change", state: "pending", requestedBy: "codex", requestedAt: "2026-08-17T03:00:00Z",
        kind: "update", targetExpertId: "reviewer", proposal: { name: "Review Expert" },
      }] as never;
      if (command === "expert_runs_list") return [{
        id: "expert-run", expertId: "reviewer", state: "awaitingReview", startedAt: "2026-08-17T04:00:00Z",
      }] as never;
      if (command === "expert_activation_requests") return [{
        id: "expert-activation", expertId: "reviewer", projectPath, state: "pending",
        requestedBy: "codex", requestedAt: "2026-08-17T05:00:00Z", client: "codex",
      }] as never;
      if (command === "projects_list") return [{ path: projectPath, label: "Review project", installedCount: 0 }] as never;
      if (command === "project_readiness_get") return readinessFixture(projectPath, true) as never;
      if (command === "project_recommendations_list") return [recommendation] as never;
      return [] as never;
    });
    ui.section = "activity";
    ui.navStack = [];
    ui.navIndex = -1;
    ui.initNav();
    const { default: ActivityHistory } = await import("$lib/components/ActivityHistory.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(ActivityHistory, { target });
    try {
      const review = await vi.waitFor(() => {
        const button = [...target.querySelectorAll<HTMLButtonElement>("button")]
          .find((candidate) => candidate.textContent?.trim() === "Review");
        expect(button).toBeTruthy();
        return button!;
      });
      const history = [...target.querySelectorAll<HTMLButtonElement>("button")]
        .find((candidate) => candidate.textContent?.trim() === "History")!;
      expect(review.getAttribute("aria-pressed")).toBe("true");
      expect(history.getAttribute("aria-pressed")).toBe("false");
      expect(review.getAttribute("role")).toBeNull();
      expect(target.querySelector('[role="tablist"]')).toBeNull();
      history.focus();
      history.click();
      await tick();
      expect(history.getAttribute("aria-pressed")).toBe("true");
      expect(target.querySelector('[role="status"]')?.textContent).toContain("History mode");
      review.click();
      await tick();
      expect(review.getAttribute("aria-pressed")).toBe("true");
      await vi.waitFor(() => expect(target.textContent).toContain("5 pending"));
      for (const label of [
        "Agent approvals 1", "Skill approvals 1", "Expert change requests 1",
        "Expert runs awaiting review 1", "Expert activation requests 1",
      ]) expect(target.textContent).toContain(label);
      expect(target.textContent).toContain("Subscription Recommendations 1");
      expect(target.textContent).not.toContain(projectPath);

      const deepLink = (source: string) => target.querySelector<HTMLButtonElement>(`[data-review-source="${source}"]`)!;
      deepLink("agent").click();
      expect(ui.section).toBe("personas");
      expect((ui as unknown as { agentApprovalId: string | null }).agentApprovalId).toBe("agent-approval");
      expect(ui.returnToActivityReview("agent", "agent-approval")).toBe(true);
      await vi.waitFor(() => expect(document.activeElement).toBe(deepLink("agent")));

      deepLink("skill").click();
      expect(ui.section).toBe("skills");
      expect((ui as unknown as { skillApprovalId: string | null }).skillApprovalId).toBe("skill-approval");
      expect(ui.returnToActivityReview("skill", "skill-approval")).toBe(true);

      deepLink("expert-change").click();
      expect(ui.section).toBe("experts");
      expect((ui as unknown as { expertReview: unknown }).expertReview).toEqual({ kind: "change", id: "expert-change" });
      expect(ui.returnToActivityReview("expert-change", "expert-change")).toBe(true);

      deepLink("expert-run").click();
      expect((ui as unknown as { expertReview: unknown }).expertReview).toEqual({ kind: "run", id: "expert-run" });
      expect(ui.returnToActivityReview("expert-run", "expert-run")).toBe(true);

      deepLink("expert-activation").click();
      expect((ui as unknown as { expertReview: unknown }).expertReview).toEqual({ kind: "activation", id: "expert-activation" });
      expect(ui.returnToActivityReview("expert-activation", "expert-activation")).toBe(true);

      deepLink("recommendation").click();
      expect(ui.section).toBe("projects");
      expect(ui.projectsSelected).toBe(projectPath);
      expect((ui as unknown as { projectRecommendationId: string | null }).projectRecommendationId).toBe(recommendation.id);
      expect(vi.mocked(invoke).mock.calls.map(([command]) => command)
        .some((command) => /approve|reject|resolve|review$|finalize/.test(command))).toBe(false);
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("keeps review sources partial on failure and retries only the unavailable group", async () => {
    let skillAttempts = 0;
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "agent_library_list") return { ...emptyFolderState(), approvals: [] } as never;
      if (command === "skill_folders_list") {
        skillAttempts += 1;
        if (skillAttempts === 1) throw new Error(`token=secret123 ${"x".repeat(800)}`);
        return { ...emptyFolderState(), approvals: [{
          id: "skill-retry", submittedAt: "2026-08-17T02:00:00Z", state: "pending", requestedBy: "codex",
          request: { action: "draftPublish", id: "skill-draft", planRevision: "revision" }, result: null,
        }] } as never;
      }
      if (["expert_creation_requests", "expert_runs_list", "expert_activation_requests", "projects_list"].includes(command)) return [] as never;
      return [] as never;
    });
    const { default: ActivityHistory } = await import("$lib/components/ActivityHistory.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(ActivityHistory, { target });
    try {
      await vi.waitFor(() => expect(target.querySelector('[data-review-group="skill"]')?.textContent).toContain("Unavailable"));
      expect(target.textContent).toContain("0 pending · partial");
      const skillGroup = target.querySelector<HTMLElement>('[data-review-group="skill"]')!;
      expect(skillGroup.textContent).toContain("Unavailable");
      expect(skillGroup.textContent).not.toContain("secret123");
      expect(skillGroup.textContent!.length).toBeLessThan(800);
      skillGroup.querySelector<HTMLButtonElement>("[data-review-retry]")!.click();
      await vi.waitFor(() => expect(target.textContent).toContain("1 pending"));
      expect(target.textContent).not.toContain("partial");
      expect(skillGroup.contains(document.activeElement)).toBe(true);
      expect(skillAttempts).toBe(2);
      expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "agent_library_list")).toHaveLength(1);
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("keeps Review partial while Recommendations is unavailable and announces retry outcomes accurately", async () => {
    const projectPath = "/tmp/recommendation-partial";
    let recommendationAttempts = 0;
    let rejectRetry!: (error: Error) => void;
    const retryPending = new Promise<ProjectRecommendation[]>((_resolve, reject) => {
      rejectRetry = reject;
    });
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "agent_library_list" || command === "skill_folders_list") return emptyFolderState() as never;
      if (["expert_creation_requests", "expert_runs_list", "expert_activation_requests"].includes(command)) return [] as never;
      if (command === "projects_list") return [{ path: projectPath, label: "Subscribed", installedCount: 0 }] as never;
      if (command === "project_readiness_get") return readinessFixture(projectPath, true) as never;
      if (command === "project_recommendations_list") {
        recommendationAttempts += 1;
        if (recommendationAttempts === 1) throw new Error("recommendations offline");
        if (recommendationAttempts === 2) return retryPending as never;
        return [] as never;
      }
      return [] as never;
    });
    const { default: ActivityHistory } = await import("$lib/components/ActivityHistory.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(ActivityHistory, { target });
    try {
      const group = await vi.waitFor(() => {
        const candidate = target.querySelector<HTMLElement>('[data-review-group="recommendation"]');
        expect(candidate?.textContent).toContain("Unavailable");
        return candidate!;
      });
      expect(target.textContent).toContain("0 pending · partial");

      group.querySelector<HTMLButtonElement>("[data-review-retry]")!.click();
      await vi.waitFor(() => expect(target.querySelector('[role="status"]')?.textContent)
        .toContain("Subscription Recommendations loading"));
      rejectRetry(new Error("token=secret123 recommendations still offline"));
      await vi.waitFor(() => expect(group.textContent).toContain("Unavailable"));
      expect(target.querySelector('[role="status"]')?.textContent).toContain("Subscription Recommendations unavailable");
      expect(target.querySelector('[role="status"]')?.textContent).not.toContain("refreshed");
      expect(target.textContent).not.toContain("secret123");

      group.querySelector<HTMLButtonElement>("[data-review-retry]")!.click();
      await vi.waitFor(() => expect(group.textContent).toContain("Ready"));
      expect(target.querySelector('[role="status"]')?.textContent).toContain("Subscription Recommendations refreshed");
      expect(target.textContent).not.toContain("partial");
      expect(recommendationAttempts).toBe(3);
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("focuses a finalize-only Project recommendation without mutating and returns to its Review group after explicit finish", async () => {
    const projectPath = "/tmp/finalize-focus";
    const recommendation: ProjectRecommendation = {
      ...renamedProjectRecommendation(projectPath),
      lifecycle: "pending",
      targets: [],
      finalizeOnly: true,
    };
    const projectRows = [{ path: projectPath, label: "Finalize focus", installedCount: 1 }];
    let finalized = false;
    vi.mocked(invoke).mockImplementation(async (command: string, args) => {
      if (command === "agent_library_list" || command === "skill_folders_list") return emptyFolderState() as never;
      if (["expert_creation_requests", "expert_runs_list", "expert_activation_requests"].includes(command)) return [] as never;
      if (command === "projects_list") return projectRows as never;
      if (command === "project_readiness_get") return readinessFixture(projectPath, true) as never;
      if (command === "project_recommendations_list") return (finalized ? [] : [recommendation]) as never;
      if (command === "project_recommendations_acknowledge") return true as never;
      if (command === "project_instructions_inspect" || command === "installs_reconcile"
        || command === "skill_installs_reconcile" || command === "skill_backups_list") return [] as never;
      if (command === "project_recommendation_finalize") {
        expect(args).toEqual({ projectPath, recommendationId: recommendation.id });
        finalized = true;
        return readinessFixture(projectPath, true).baseline as never;
      }
      return [] as never;
    });
    ui.section = "activity";
    ui.navStack = [];
    ui.navIndex = -1;
    ui.initNav();
    const { default: ActivityHistory } = await import("$lib/components/ActivityHistory.svelte");
    const { default: Projects } = await import("$lib/components/Projects.svelte");
    const activityTarget = document.createElement("div");
    document.body.append(activityTarget);
    const activityComponent = mount(ActivityHistory, { target: activityTarget });
    const trigger = await vi.waitFor(() => {
      const candidate = activityTarget.querySelector<HTMLButtonElement>('[data-review-source="recommendation"]');
      expect(candidate).toBeTruthy();
      return candidate!;
    });
    trigger.click();
    expect(ui.section).toBe("projects");
    expect(ui.projectRecommendationId).toBe(recommendation.id);
    unmount(activityComponent);
    activityTarget.remove();

    const projectsTarget = document.createElement("div");
    document.body.append(projectsTarget);
    const projectsComponent = mount(Projects, { target: projectsTarget });
    try {
      const finish = await vi.waitFor(() => {
        const candidate = [...projectsTarget.querySelectorAll<HTMLButtonElement>("button")]
          .find((button) => button.textContent?.trim() === "Finish rename");
        expect(candidate).toBeTruthy();
        expect(ui.projectRecommendationId).toBeNull();
        expect(document.activeElement).toBe(candidate);
        return candidate!;
      });
      expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "project_recommendation_finalize")).toBe(false);
      finish.click();
      await vi.waitFor(() => expect(ui.section).toBe("activity"));
      expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "project_recommendation_finalize")).toHaveLength(1);
    } finally {
      unmount(projectsComponent);
      projectsTarget.remove();
    }

    const returnTarget = document.createElement("div");
    document.body.append(returnTarget);
    const returnComponent = mount(ActivityHistory, { target: returnTarget });
    try {
      await vi.waitFor(() => {
        const group = returnTarget.querySelector<HTMLElement>('[data-review-group="recommendation"]')!;
        expect(group.textContent).toContain("Ready");
        expect(group.contains(document.activeElement)).toBe(true);
      });
      expect(ui.reviewIntent).toBeNull();
    } finally {
      unmount(returnComponent);
      returnTarget.remove();
    }
  });

  it("focuses a Project recommendation without opening it and returns from the real review modal to the exact Activity trigger", async () => {
    const projectPath = "/tmp/recommendation-modal-focus";
    const recommendation = updatedProjectRecommendation(projectPath);
    const projectRows = [{ path: projectPath, label: "Recommendation modal", installedCount: 0 }];
    const pkg: AgentPackageResult = {
      ...staleControlPackage,
      reference: recommendation.agentReferences[0],
      agent: { ...staleControlAgent, slug: "reviewer", name: "Reviewer" },
    };
    install.tools = [staleControlTool];
    corpus.agents = [pkg.agent!];
    projects.list = projectRows;
    vi.mocked(invoke).mockImplementation(async (command: string, args) => {
      if (command === "agent_library_list" || command === "skill_folders_list") return emptyFolderState() as never;
      if (["expert_creation_requests", "expert_runs_list", "expert_activation_requests"].includes(command)) return [] as never;
      if (command === "projects_list") return projectRows as never;
      if (command === "installs_reconcile" || command === "skill_installs_reconcile" || command === "skill_backups_list") return [] as never;
      if (command === "tools_list") return [staleControlTool] as never;
      if (command === "project_instructions_inspect") return [] as never;
      if (command === "project_readiness_get") return readinessFixture(projectPath, true) as never;
      if (command === "project_recommendations_list") return [recommendation] as never;
      if (command === "project_recommendations_acknowledge") return true as never;
      if (command === "project_recommendation_open") return recommendation as never;
      if (command === "agent_sources_inspect") return [{
        source: { id: recommendation.agentReferences[0].sourceId, label: "Built in", enabled: true, kind: { kind: "builtIn" } },
        agents: [pkg], errors: [], revision: "recommendation",
      }] as never;
      if (command === "agent_drafts_list") return [] as never;
      if (command === "agent_update_plan") return {
        revision: "recommendation-plan", operation: "update", tool: "claudeCode", scope: "project", projectPath,
        agents: [{
          reference: recommendation.agentReferences[0], name: "Reviewer", sourceHash: "source", dependency: false,
          destination: `${projectPath}/.agents/reviewer.md`, renderedFileCount: 1, capabilities: [],
        }], warnings: [], blockers: [], rollbackAvailable: true,
      } as never;
      return [] as never;
    });
    ui.section = "activity";
    ui.navStack = [];
    ui.navIndex = -1;
    ui.initNav();
    const { default: ActivityHistory } = await import("$lib/components/ActivityHistory.svelte");
    const { default: Projects } = await import("$lib/components/Projects.svelte");
    const activityTarget = document.createElement("div");
    document.body.append(activityTarget);
    const activityComponent = mount(ActivityHistory, { target: activityTarget });
    const activityTrigger = await vi.waitFor(() => {
      const candidate = activityTarget.querySelector<HTMLButtonElement>('[data-review-source="recommendation"]');
      expect(candidate).toBeTruthy();
      return candidate!;
    });
    const triggerId = activityTrigger.dataset.reviewTrigger!;
    activityTrigger.click();
    unmount(activityComponent);
    activityTarget.remove();

    const projectsTarget = document.createElement("div");
    document.body.append(projectsTarget);
    const projectsComponent = mount(Projects, { target: projectsTarget });
    try {
      const open = await vi.waitFor(() => {
        const candidate = [...projectsTarget.querySelectorAll<HTMLButtonElement>("button")]
          .find((button) => button.textContent?.trim() === "Open review");
        expect(candidate).toBeTruthy();
        expect(document.activeElement).toBe(candidate);
        return candidate!;
      });
      expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "project_recommendation_open")).toBe(false);
      expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "update_agent")).toBe(false);
      open.click();
      const dialog = await vi.waitFor(() => {
        const candidate = [...projectsTarget.querySelectorAll<HTMLElement>('[role="dialog"]')]
          .find((element) => element.querySelector("h1")?.textContent?.trim() === "Review catalog recommendation");
        expect(candidate).toBeTruthy();
        return candidate!;
      });
      expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "update_agent")).toBe(false);
      dialog.querySelector<HTMLButtonElement>('button[aria-label="Close"]')!.click();
      await vi.waitFor(() => expect(ui.section).toBe("activity"));
    } finally {
      unmount(projectsComponent);
      projectsTarget.remove();
    }

    const returnTarget = document.createElement("div");
    document.body.append(returnTarget);
    const returned = mount(ActivityHistory, { target: returnTarget });
    try {
      await vi.waitFor(() => expect((document.activeElement as HTMLElement | null)?.dataset.reviewTrigger).toBe(triggerId));
      expect(ui.reviewIntent).toBeNull();
    } finally {
      unmount(returned);
      returnTarget.remove();
    }
  });

  async function mountLinkedAgentApprovalOwner(options: {
    mutation?: "agent_approval_approve" | "agent_approval_reject";
    failMutation?: boolean;
  } = {}) {
    const approval = {
      id: "agent-owner-approval", submittedAt: "2026-08-17T02:00:00Z", state: "pending" as const,
      requestedBy: "codex", request: { action: "draftPublish" as const, id: "agent-draft", planRevision: "revision" }, result: null,
    };
    vi.mocked(invoke).mockImplementation(async (command: string, args) => {
      if (command === "agent_library_list") return { ...emptyFolderState(), approvals: [approval] } as never;
      if (command === options.mutation) {
        expect(args).toEqual({ id: approval.id });
        if (options.failMutation) throw { code: "io", message: "approval mutation offline" };
        return { ...approval, state: command === "agent_approval_approve" ? "approved" : "rejected" } as never;
      }
      if (command === "skill_folders_list") return emptyFolderState() as never;
      if (["agent_sources_inspect", "agent_drafts_list", "expert_creation_requests", "expert_runs_list",
        "expert_activation_requests", "projects_list"].includes(command)) return [] as never;
      return [] as never;
    });
    agentLibrary.library = { ...emptyFolderState(), approvals: [approval] } as never;
    agentLibrary.error = null;
    agentLibrary.busy = false;
    corpus.agents = [staleControlAgent];
    ui.section = "activity";
    ui.navStack = [];
    ui.navIndex = -1;
    ui.initNav();

    const { default: ActivityHistory } = await import("$lib/components/ActivityHistory.svelte");
    const { default: AgentsWorkspace } = await import("$lib/components/AgentsWorkspace.svelte");
    const activityTarget = document.createElement("div");
    document.body.append(activityTarget);
    const activityComponent = mount(ActivityHistory, { target: activityTarget });
    const activityTrigger = await vi.waitFor(() => {
      const candidate = activityTarget.querySelector<HTMLButtonElement>('[data-review-source="agent"]');
      expect(candidate).toBeTruthy();
      return candidate!;
    });
    const triggerId = activityTrigger.dataset.reviewTrigger!;
    activityTrigger.click();
    expect(ui.section).toBe("personas");
    expect(ui.agentApprovalId).toBe(approval.id);
    unmount(activityComponent);
    activityTarget.remove();

    const ownerTarget = document.createElement("div");
    document.body.append(ownerTarget);
    const ownerComponent = mount(AgentsWorkspace, { target: ownerTarget });
    const dialog = await vi.waitFor(() => {
      const candidate = ownerTarget.querySelector<HTMLElement>('[role="dialog"]');
      expect(candidate?.querySelector("h1")?.textContent).toContain("Agent approval inbox");
      expect((document.activeElement as HTMLElement | null)?.closest("[data-agent-approval-id]")
        ?.getAttribute("data-agent-approval-id")).toBe(approval.id);
      return candidate!;
    });
    const returnToActivity = async () => {
      unmount(ownerComponent);
      ownerTarget.remove();
      const returnTarget = document.createElement("div");
      document.body.append(returnTarget);
      const returnComponent = mount(ActivityHistory, { target: returnTarget });
      try {
        await vi.waitFor(() => expect((document.activeElement as HTMLElement | null)?.dataset.reviewTrigger).toBe(triggerId));
        expect(ui.reviewIntent).toBeNull();
      } finally {
        unmount(returnComponent);
        returnTarget.remove();
      }
    };
    return { approval, dialog, ownerComponent, ownerTarget, returnToActivity };
  }

  it("returns a focused Agent approval through its real owner on Close without deciding it", async () => {
    const owner = await mountLinkedAgentApprovalOwner();
    try {
      expect(vi.mocked(invoke).mock.calls.some(([command]) => command.startsWith("agent_approval_"))).toBe(false);
      owner.dialog.querySelector<HTMLButtonElement>('button[aria-label="Close"]')!.click();
      await vi.waitFor(() => expect(ui.section).toBe("activity"));
      expect(vi.mocked(invoke).mock.calls.some(([command]) => command.startsWith("agent_approval_"))).toBe(false);
      await owner.returnToActivity();
    } catch (error) {
      unmount(owner.ownerComponent);
      owner.ownerTarget.remove();
      throw error;
    }
  });

  it.each([
    ["Approve", "agent_approval_approve"],
    ["Reject", "agent_approval_reject"],
  ] as const)("returns a focused Agent approval through its real owner after successful %s", async (actionLabel, mutation) => {
    const owner = await mountLinkedAgentApprovalOwner({ mutation });
    try {
      expect(vi.mocked(invoke).mock.calls.filter(([command]) => command.startsWith("agent_approval_"))).toHaveLength(0);
      [...owner.dialog.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.trim() === actionLabel)!.click();
      await vi.waitFor(() => expect(ui.section).toBe("activity"));
      expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === mutation)).toHaveLength(1);
      expect(vi.mocked(invoke).mock.calls.filter(([command]) => command.startsWith("agent_approval_") && command !== mutation)).toHaveLength(0);
      await owner.returnToActivity();
    } catch (error) {
      unmount(owner.ownerComponent);
      owner.ownerTarget.remove();
      throw error;
    }
  });

  it("keeps a failed focused Agent approval open and moves focus to its announced error", async () => {
    const owner = await mountLinkedAgentApprovalOwner({ mutation: "agent_approval_approve", failMutation: true });
    try {
      [...owner.dialog.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.trim() === "Approve")!.click();
      const status = await vi.waitFor(() => {
        const candidate = owner.dialog.querySelector<HTMLElement>('.status[aria-live="polite"]');
        expect(candidate?.textContent).toContain("approval mutation offline");
        expect(document.activeElement).toBe(candidate);
        return candidate!;
      });
      expect(status.closest('[role="dialog"]')).toBe(owner.dialog);
      expect(ui.section).toBe("personas");
      expect(ui.agentApprovalId).toBe(owner.approval.id);
      expect(ui.reviewIntent).toMatchObject({ kind: "agent", exactId: owner.approval.id });
      expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "agent_approval_approve")).toHaveLength(1);
      expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "agent_approval_reject")).toBe(false);
    } finally {
      unmount(owner.ownerComponent);
      owner.ownerTarget.remove();
    }
  });

  it("keeps a successfully decided locally opened Agent approval inbox open", async () => {
    const approval = {
      id: "agent-local-approval", submittedAt: "2026-08-17T02:00:00Z", state: "pending" as const,
      requestedBy: "codex", request: { action: "draftPublish" as const, id: "local-draft", planRevision: "revision" }, result: null,
    };
    vi.mocked(invoke).mockImplementation(async (command: string, args) => {
      if (command === "agent_library_list") return { ...emptyFolderState(), approvals: [approval] } as never;
      if (command === "agent_approval_approve") {
        expect(args).toEqual({ id: approval.id });
        return { ...approval, state: "approved" } as never;
      }
      if (["agent_sources_inspect", "agent_drafts_list"].includes(command)) return [] as never;
      return [] as never;
    });
    agentLibrary.library = { ...emptyFolderState(), approvals: [approval] } as never;
    agentLibrary.error = null;
    agentLibrary.busy = false;
    corpus.agents = [staleControlAgent];
    ui.section = "personas";
    ui.navStack = [];
    ui.navIndex = -1;
    ui.initNav();
    const { default: AgentsWorkspace } = await import("$lib/components/AgentsWorkspace.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(AgentsWorkspace, { target });
    try {
      const open = await vi.waitFor(() => {
        const candidate = [...target.querySelectorAll<HTMLButtonElement>("button")]
          .find((button) => button.textContent?.trim().startsWith("Approvals"));
        expect(candidate).toBeTruthy();
        return candidate!;
      });
      open.click();
      const dialog = await vi.waitFor(() => {
        const candidate = target.querySelector<HTMLElement>('[role="dialog"]');
        expect(candidate?.querySelector("h1")?.textContent).toContain("Agent approval inbox");
        return candidate!;
      });
      [...dialog.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.trim() === "Approve")!.click();
      await vi.waitFor(() => {
        expect(vi.mocked(invoke).mock.calls
          .filter(([command]) => command === "agent_approval_approve")).toHaveLength(1);
        expect(agentLibrary.busy).toBe(false);
      });
      expect(target.querySelector('[role="dialog"]')).toBe(dialog);
      expect(ui.section).toBe("personas");
      expect(ui.reviewIntent).toBeNull();
      expect(ui.agentApprovalId).toBeNull();
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it.each([
    ["Approve", "skill_approval_approve"],
    ["Reject", "skill_approval_reject"],
  ] as const)("routes a Skill approval through its real owner, returns on close, and returns after %s", async (actionLabel, mutationCommand) => {
    const approval = {
      id: "skill-owner-approval", submittedAt: "2026-08-17T02:00:00Z", state: "pending" as const,
      requestedBy: "codex", request: { action: "draftPublish" as const, id: "skill-draft", planRevision: "revision" }, result: null,
    };
    let resolved = false;
    vi.mocked(invoke).mockImplementation(async (command: string, args) => {
      if (command === "skill_folders_list") return {
        ...emptyFolderState(), approvals: resolved ? [] : [approval],
      } as never;
      if (command === mutationCommand) {
        expect(args).toEqual({ id: approval.id });
        resolved = true;
        return { ...approval, state: actionLabel === "Approve" ? "approved" : "rejected" } as never;
      }
      if (command === "agent_library_list") return emptyFolderState() as never;
      if (["expert_creation_requests", "expert_runs_list", "expert_activation_requests", "projects_list",
        "skill_sources_inspect", "skill_drafts_list", "skill_installs_reconcile", "skill_backups_list"].includes(command)) return [] as never;
      return [] as never;
    });
    ui.section = "activity";
    ui.navStack = [];
    ui.navIndex = -1;
    ui.initNav();
    const { default: ActivityHistory } = await import("$lib/components/ActivityHistory.svelte");
    const { default: SkillsWorkspace } = await import("$lib/components/SkillsWorkspace.svelte");

    const openFromActivity = async () => {
      const target = document.createElement("div");
      document.body.append(target);
      const component = mount(ActivityHistory, { target });
      const trigger = await vi.waitFor(() => {
        const candidate = target.querySelector<HTMLButtonElement>('[data-review-source="skill"]');
        expect(candidate).toBeTruthy();
        return candidate!;
      });
      trigger.click();
      unmount(component);
      target.remove();
      return trigger.dataset.reviewTrigger!;
    };

    const firstTriggerId = await openFromActivity();
    const firstOwnerTarget = document.createElement("div");
    document.body.append(firstOwnerTarget);
    const firstOwner = mount(SkillsWorkspace, { target: firstOwnerTarget });
    try {
      const inbox = await vi.waitFor(() => {
        const candidate = firstOwnerTarget.querySelector<HTMLDetailsElement>("details.draft-inbox")!;
        expect(candidate.open).toBe(true);
        const row = candidate.querySelector<HTMLElement>(`[data-skill-approval-id="${approval.id}"]`)!;
        expect(row.contains(document.activeElement)).toBe(true);
        return candidate;
      });
      expect(vi.mocked(invoke).mock.calls.some(([command]) => command === mutationCommand)).toBe(false);
      inbox.querySelector<HTMLElement>("summary")!.click();
      await vi.waitFor(() => expect(ui.section).toBe("activity"));
    } finally {
      unmount(firstOwner);
      firstOwnerTarget.remove();
    }

    const returnTarget = document.createElement("div");
    document.body.append(returnTarget);
    const returned = mount(ActivityHistory, { target: returnTarget });
    try {
      await vi.waitFor(() => expect((document.activeElement as HTMLElement | null)?.dataset.reviewTrigger).toBe(firstTriggerId));
      returnTarget.querySelector<HTMLButtonElement>('[data-review-source="skill"]')!.click();
    } finally {
      unmount(returned);
      returnTarget.remove();
    }

    const resolveTarget = document.createElement("div");
    document.body.append(resolveTarget);
    const resolver = mount(SkillsWorkspace, { target: resolveTarget });
    try {
      const action = await vi.waitFor(() => {
        const candidate = [...resolveTarget.querySelectorAll<HTMLButtonElement>("button")]
          .find((button) => button.textContent?.trim() === actionLabel);
        expect(candidate).toBeTruthy();
        return candidate!;
      });
      action.click();
      await vi.waitFor(() => expect(ui.section).toBe("activity"));
      expect(ui.reviewIntent).toMatchObject({ kind: "skill", exactId: approval.id });
      expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === mutationCommand)).toHaveLength(1);
    } finally {
      unmount(resolver);
      resolveTarget.remove();
    }

    const finalTarget = document.createElement("div");
    document.body.append(finalTarget);
    const finalActivity = mount(ActivityHistory, { target: finalTarget });
    try {
      await vi.waitFor(() => {
        const group = finalTarget.querySelector<HTMLElement>('[data-review-group="skill"]')!;
        expect(group.textContent).toContain("Ready");
        expect(group.contains(document.activeElement)).toBe(true);
      });
      expect(ui.reviewIntent).toBeNull();
    } finally {
      unmount(finalActivity);
      finalTarget.remove();
    }
  });

  it.each([
    ["expert-change", "change", "Review update request", "Save", "expert_creation_request_approve"],
    ["expert-run", "run", "Review run expert-r", "Reject", "expert_run_review"],
    ["expert-activation", "activation", "Review Reviewer", "Approve activation", "expert_activate"],
  ] as const)("routes %s through the real Expert owner with close, explicit resolution, and return focus", async (source, kind, title, actionLabel, mutationCommand) => {
    const expert = performanceExpert();
    const { id: _id, version: _version, source: _source, unresolvedAgents: _ua, unresolvedSkills: _us, unresolvedRunbook: _ur, ...proposal } = expert;
    const creation = {
      id: "expert-change", clientRequestId: "client-change", outcome: "update", projectPath: "/tmp/expert-owner",
      requestedBy: "codex", requestedAt: "2026-08-17T03:00:00Z", proposal,
      linkedSkillDrafts: [], linkedSkillStates: [], agentSubstitutions: [], state: "pending" as const,
      savedExpertId: null, kind: "update" as const, targetExpertId: expert.id, baseVersion: expert.version,
      readiness: "ready" as const, blockers: [], warnings: [],
    };
    const run = performanceRun("expert-run", "awaitingReview");
    const activation = {
      id: "expert-activation", expertId: expert.id, projectPath: "/tmp/expert-owner", client: "codex" as const,
      requestedBy: "codex", requestedAt: "2026-08-17T04:00:00Z", state: "pending" as const,
    };
    let resolved = false;
    vi.stubGlobal("navigator", { ...navigator, clipboard: { writeText: vi.fn(async () => undefined) } });
    corpus.agents = [staleControlAgent];
    projects.list = [{ path: activation.projectPath, label: "Expert owner", installedCount: 0 }];
    vi.mocked(invoke).mockImplementation(async (command: string, args) => {
      if (command === "agent_library_list" || command === "skill_folders_list") return emptyFolderState() as never;
      if (command === "experts_list") return [expert] as never;
      if (command === "expert_creation_requests") return (!resolved && kind === "change" ? [creation] : []) as never;
      if (command === "expert_runs_list") return (!resolved && kind === "run" ? [run] : []) as never;
      if (command === "expert_activation_requests") return (!resolved && kind === "activation" ? [activation] : []) as never;
      if (command === "expert_activation_history" || command === "projects_list") return [] as never;
      if (command === "expert_plan_activation") return {
        expert, projectPath: activation.projectPath, client: "codex", agents: [], skills: [], existing: [],
        warnings: [], blockers: [], promptPreview: "Start the Expert", rollbackScope: [],
      } as never;
      if (command === mutationCommand) {
        resolved = true;
        if (command === "expert_run_review") expect(args).toMatchObject({ id: run.id, verdict: "rejected" });
        return command === "expert_activate" ? {
          id: "activation-record", expertId: expert.id, expertVersion: expert.version,
          projectPath: activation.projectPath, client: "codex", activatedAt: "2026-08-17T05:00:00Z",
          installedAgents: [], installedSkills: [], runId: null,
        } as never : undefined as never;
      }
      if (command === "expert_activation_request_resolve") return undefined as never;
      return [] as never;
    });
    ui.section = "activity";
    ui.navStack = [];
    ui.navIndex = -1;
    ui.initNav();
    const { default: ActivityHistory } = await import("$lib/components/ActivityHistory.svelte");
    const { default: Experts } = await import("$lib/components/Experts.svelte");

    const openReview = async () => {
      const target = document.createElement("div");
      document.body.append(target);
      const component = mount(ActivityHistory, { target });
      const trigger = await vi.waitFor(() => {
        const candidate = target.querySelector<HTMLButtonElement>(`[data-review-source="${source}"]`);
        expect(candidate).toBeTruthy();
        return candidate!;
      });
      const triggerId = trigger.dataset.reviewTrigger!;
      trigger.click();
      unmount(component);
      target.remove();
      return triggerId;
    };

    const firstTriggerId = await openReview();
    const firstOwnerTarget = document.createElement("div");
    document.body.append(firstOwnerTarget);
    const firstOwner = mount(Experts, { target: firstOwnerTarget });
    try {
      const dialog = await vi.waitFor(() => {
        const candidate = [...firstOwnerTarget.querySelectorAll<HTMLElement>('[role="dialog"]')]
          .find((element) => element.querySelector("h1")?.textContent?.trim() === title) ?? null;
        expect(candidate).toBeTruthy();
        expect(candidate!.contains(document.activeElement)).toBe(true);
        return candidate!;
      });
      expect(vi.mocked(invoke).mock.calls.some(([command]) => command === mutationCommand)).toBe(false);
      dialog.querySelector<HTMLButtonElement>('button[aria-label="Close"]')!.click();
      await vi.waitFor(() => expect(ui.section).toBe("activity"));
    } finally {
      unmount(firstOwner);
      firstOwnerTarget.remove();
    }

    const returnedTarget = document.createElement("div");
    document.body.append(returnedTarget);
    const returned = mount(ActivityHistory, { target: returnedTarget });
    try {
      await vi.waitFor(() => expect((document.activeElement as HTMLElement | null)?.dataset.reviewTrigger).toBe(firstTriggerId));
      returnedTarget.querySelector<HTMLButtonElement>(`[data-review-source="${source}"]`)!.click();
    } finally {
      unmount(returned);
      returnedTarget.remove();
    }

    const resolveTarget = document.createElement("div");
    document.body.append(resolveTarget);
    const resolver = mount(Experts, { target: resolveTarget });
    try {
      const action = await vi.waitFor(() => {
        const candidate = [...resolveTarget.querySelectorAll<HTMLButtonElement>("button")]
          .find((button) => button.textContent?.trim() === actionLabel);
        expect(candidate).toBeTruthy();
        return candidate!;
      });
      action.click();
      await vi.waitFor(() => expect(ui.section).toBe("activity"));
      expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === mutationCommand)).toHaveLength(1);
    } finally {
      unmount(resolver);
      resolveTarget.remove();
    }

    const finalTarget = document.createElement("div");
    document.body.append(finalTarget);
    const finalActivity = mount(ActivityHistory, { target: finalTarget });
    try {
      await vi.waitFor(() => {
        const group = finalTarget.querySelector<HTMLElement>(`[data-review-group="${source}"]`)!;
        expect(group.textContent).toContain("Ready");
        expect(group.contains(document.activeElement)).toBe(true);
      });
      expect(ui.reviewIntent).toBeNull();
    } finally {
      unmount(finalActivity);
      finalTarget.remove();
    }
  });

  it("loads recovery sources independently and uses exact existing rollback and reveal boundaries", async () => {
    const agent = { ...staleControlRow, sourceId: "agents", relativePath: "engineering/reviewer.md", tracked: true, state: "current" as const };
    const skill: InstalledSkill = {
      sourceId: "skills", relativePath: "audit", name: "Audit", runtime: "codex", scope: "user",
      projectPath: null, path: "/private/.codex/skills/audit", state: "current", tracked: true,
    };
    const unrelatedSkill: InstalledSkill = {
      ...skill, relativePath: "unrelated", name: "Unrelated", path: "/private/.codex/skills/unrelated",
    };
    const backupPath = "/private/app/state/backups/agency-agents-verified.sqlite3";
    let agentAttempts = 0;
    vi.mocked(invoke).mockImplementation(async (command: string, args?: unknown) => {
      if (command === "doctor_report") return {
        generatedAt: "2026-08-17T00:00:00Z", overall: "healthy",
        counts: { healthy: 1, needsAttention: 0, unavailable: 0 }, copyText: "healthy", checks: [],
      } as never;
      if (command === "installs_reconcile") {
        agentAttempts += 1;
        if (agentAttempts === 1) throw new Error("Agent inventory unavailable");
        return [agent] as never;
      }
      if (command === "agent_version_history") return [{
        id: "agent-snapshot", createdAt: "2026-08-17T00:00:00Z",
        sourceHash: "a".repeat(64), renderedHash: "b".repeat(64), contentPath: "/private/agent-snapshot",
      }] as never;
      if (command === "projects_list") return [] as never;
      if (command === "skill_installs_reconcile") return [skill, unrelatedSkill] as never;
      if (command === "skill_version_history_list") {
        const exact = args as { sourceId: string; relativePath: string; runtime: string; projectPath: string | null };
        expect(exact).toMatchObject({ sourceId: "skills", runtime: "codex", projectPath: null });
        return exact.relativePath === skill.relativePath
          ? [{ path: "/private/skill-backup", createdAt: "2026-08-17T00:00:00Z" }] as never
          : [] as never;
      }
      if (command === "skill_backups_list") throw new Error("raw orphan inventory unavailable");
      if (command === "storage_migration_status") return { state: "complete", stage: null, detail: null, legacyConflicts: [] } as never;
      if (command === "storage_backup") return backupPath as never;
      if (command === "reveal_path") {
        expect(args).toEqual({ path: backupPath });
        return undefined as never;
      }
      return [] as never;
    });
    const { default: SettingsSectionDoctor } = await import("$lib/components/SettingsSectionDoctor.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(SettingsSectionDoctor, { target });
    try {
      await vi.waitFor(() => expect(target.querySelector('[data-recovery-source="agents"]')?.textContent).toContain("Unavailable"));
      expect(target.querySelector(".summary")?.getAttribute("role")).toBe("group");
      expect(target.querySelector('[data-recovery-source="skills"]')?.textContent).toContain("1 rollback point");
      expect(target.querySelector('[data-recovery-source="skills"]')?.textContent).toContain("Audit");
      expect(target.querySelector('[data-recovery-source="skills"]')?.textContent).not.toContain("Unrelated");
      expect(target.querySelector('[data-recovery-source="storage"]')?.textContent).toContain("Ready");
      expect(target.textContent).toContain("offline/manual");
      expect(target.textContent).toContain("WAL");
      expect(target.textContent).not.toContain("/private/skill-backup");
      expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "skill_version_history_list")).toHaveLength(2);

      target.querySelector<HTMLButtonElement>('[data-recovery-source="agents"] [data-recovery-retry]')!.click();
      await vi.waitFor(() => expect(target.querySelector('[data-recovery-source="agents"]')?.textContent).toContain("1 rollback point"));
      expect(target.querySelector('[data-recovery-source="agents"]')?.contains(document.activeElement)).toBe(true);
      target.querySelector<HTMLButtonElement>('[data-agent-recovery]')!.click();
      expect(ui.section).toBe("personas");
      expect((ui as unknown as { agentRecovery: unknown }).agentRecovery).toEqual({
        reference: { sourceId: "agents", relativePath: "engineering/reviewer.md" },
        tool: agent.tool, projectPath: null,
      });

      ui.openSettings("doctor");
      target.querySelector<HTMLButtonElement>('[data-skill-recovery]')!.click();
      expect(ui.section).toBe("skills");
      expect((ui as unknown as { skillRecovery: unknown }).skillRecovery).toEqual({
        reference: { sourceId: "skills", relativePath: "audit" }, runtime: "codex", projectPath: null,
      });

      const createBackup = target.querySelector<HTMLButtonElement>("[data-storage-backup]")!;
      createBackup.click();
      await vi.waitFor(() => expect(target.textContent).toContain("Verified backup created"));
      expect(document.activeElement).toBe(target.querySelector("[data-storage-reveal]"));
      expect(target.textContent).toContain("agency-agents-verified.sqlite3");
      expect(target.textContent).not.toContain(backupPath);
      target.querySelector<HTMLButtonElement>("[data-storage-reveal]")!.click();
      await vi.waitFor(() => expect(vi.mocked(invoke)).toHaveBeenCalledWith("reveal_path", { path: backupPath }));
      expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "open" || command === "shell_open")).toBe(false);
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("keeps verified backup failure visible without erasing Agent and Skill recovery truth", async () => {
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "doctor_report") return {
        generatedAt: "2026-08-17T00:00:00Z", overall: "healthy",
        counts: { healthy: 0, needsAttention: 0, unavailable: 0 }, copyText: "healthy", checks: [],
      } as never;
      if (command === "installs_reconcile" || command === "projects_list" || command === "skill_installs_reconcile" || command === "skill_backups_list") return [] as never;
      if (command === "storage_migration_status") return { state: "complete", stage: null, detail: null, legacyConflicts: [] } as never;
      if (command === "storage_backup") throw new Error("token=secret123 backup verification failed");
      return [] as never;
    });
    const { default: SettingsSectionDoctor } = await import("$lib/components/SettingsSectionDoctor.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(SettingsSectionDoctor, { target });
    try {
      await vi.waitFor(() => expect(target.querySelector('[data-recovery-source="agents"]')?.textContent).toContain("0 rollback points"));
      target.querySelector<HTMLButtonElement>("[data-storage-backup]")!.click();
      await vi.waitFor(() => expect(target.querySelector('[data-recovery-source="storage"] [role="alert"]')).not.toBeNull());
      expect(target.textContent).toContain("backup verification failed");
      expect(target.textContent).not.toContain("secret123");
      expect(target.querySelector('[data-recovery-source="agents"]')?.textContent).toContain("0 rollback points");
      expect(target.querySelector('[data-recovery-source="skills"]')?.textContent).toContain("0 rollback points");
      expect(target.querySelector('[aria-live="polite"]')?.textContent).toBeTruthy();
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("pages Recovery inventories without hiding version history after the first 100 installs", async () => {
    const skills = Array.from({ length: 101 }, (_, index): InstalledSkill => ({
      sourceId: `skills-${String(index).padStart(3, "0")}`,
      relativePath: `skill-${String(index).padStart(3, "0")}`,
      name: index === 100 ? "Late history" : `Skill ${index}`,
      runtime: "codex",
      scope: "user",
      projectPath: null,
      path: `/private/skills/${index}`,
      state: "current",
      tracked: true,
    })).reverse();
    vi.mocked(invoke).mockImplementation(async (command: string, args) => {
      if (command === "doctor_report") return {
        generatedAt: "2026-08-17T00:00:00Z", overall: "healthy",
        counts: { healthy: 0, needsAttention: 0, unavailable: 0 }, copyText: "healthy", checks: [],
      } as never;
      if (command === "installs_reconcile" || command === "projects_list") return [] as never;
      if (command === "skill_installs_reconcile") return skills as never;
      if (command === "skill_version_history_list") {
        const exact = args as { sourceId: string };
        return exact.sourceId === "skills-100"
          ? [{ path: "/private/exact-skill-snapshot", createdAt: "2026-08-17T00:00:00Z", contentHash: "c".repeat(64) }] as never
          : [] as never;
      }
      if (command === "storage_migration_status") return { state: "complete", stage: null, detail: null, legacyConflicts: [] } as never;
      return [] as never;
    });
    const { default: SettingsSectionDoctor } = await import("$lib/components/SettingsSectionDoctor.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(SettingsSectionDoctor, { target });
    try {
      const group = await vi.waitFor(() => {
        const candidate = target.querySelector<HTMLElement>('[data-recovery-source="skills"]')!;
        expect(candidate.textContent).toContain("Partial");
        expect(candidate.textContent).toContain("Showing 100 of 101 installs checked");
        return candidate;
      });
      expect(group.textContent).not.toContain("Late history");
      expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "skill_version_history_list")).toHaveLength(100);
      group.querySelector<HTMLButtonElement>("[data-recovery-load-more]")!.click();
      await vi.waitFor(() => {
        expect(group.textContent).toContain("Ready");
        expect(group.textContent).toContain("Showing 101 of 101 installs checked");
        expect(group.textContent).toContain("Late history");
      });
      expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "skill_version_history_list")).toHaveLength(101);
      expect(target.querySelector('.recovery [role="status"]')?.textContent).toContain("Skill recovery ready");
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("announces Recovery retry loading and failure without a contradictory refreshed result", async () => {
    let attempts = 0;
    let rejectRetry!: (error: Error) => void;
    const pendingRetry = new Promise<InstalledAgent[]>((_resolve, reject) => (rejectRetry = reject));
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "doctor_report") return {
        generatedAt: "2026-08-17T00:00:00Z", overall: "healthy",
        counts: { healthy: 0, needsAttention: 0, unavailable: 0 }, copyText: "healthy", checks: [],
      } as never;
      if (command === "installs_reconcile") {
        attempts += 1;
        if (attempts === 1) throw new Error("agent history offline");
        if (attempts === 2) return pendingRetry as never;
        return [] as never;
      }
      if (command === "projects_list" || command === "skill_installs_reconcile") return [] as never;
      if (command === "storage_migration_status") return { state: "complete", stage: null, detail: null, legacyConflicts: [] } as never;
      return [] as never;
    });
    const { default: SettingsSectionDoctor } = await import("$lib/components/SettingsSectionDoctor.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(SettingsSectionDoctor, { target });
    try {
      const group = await vi.waitFor(() => {
        const candidate = target.querySelector<HTMLElement>('[data-recovery-source="agents"]')!;
        expect(candidate.textContent).toContain("Unavailable");
        return candidate;
      });
      group.querySelector<HTMLButtonElement>("[data-recovery-retry]")!.click();
      await vi.waitFor(() => expect(target.querySelector('.recovery [role="status"]')?.textContent).toContain("Agent recovery loading"));
      rejectRetry(new Error("token=secret123 agent history still offline"));
      await vi.waitFor(() => expect(group.textContent).toContain("Unavailable"));
      expect(target.querySelector('.recovery [role="status"]')?.textContent).toContain("Agent recovery unavailable");
      expect(target.querySelector('.recovery [role="status"]')?.textContent).not.toContain("refreshed");
      expect(target.textContent).not.toContain("secret123");

      group.querySelector<HTMLButtonElement>("[data-recovery-retry]")!.click();
      await vi.waitFor(() => expect(group.textContent).toContain("0 rollback points"));
      expect(target.querySelector('.recovery [role="status"]')?.textContent).toContain("Agent recovery refreshed");
      expect(attempts).toBe(3);
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("routes exact Agent recovery through its real rollback owner and returns to the originating Settings trigger", async () => {
    const reference = { sourceId: "agents", relativePath: "engineering/reviewer.md" };
    const row: InstalledAgent = {
      ...staleControlRow, slug: "reviewer", name: "Reviewer", ...reference,
      tool: "claudeCode", projectPath: null, state: "current", tracked: true,
    };
    const pkg: AgentPackageResult = { ...staleControlPackage, reference };
    const snapshot = {
      id: "agent-recovery-snapshot", createdAt: "2026-08-17T00:00:00Z",
      sourceHash: "a".repeat(64), renderedHash: "b".repeat(64), contentPath: "/private/agent-snapshot",
    };
    vi.mocked(invoke).mockImplementation(async (command: string, args) => {
      if (command === "doctor_report") return {
        generatedAt: "2026-08-17T00:00:00Z", overall: "healthy",
        counts: { healthy: 1, needsAttention: 0, unavailable: 0 }, copyText: "healthy", checks: [],
      } as never;
      if (command === "installs_reconcile") return [row] as never;
      if (command === "agent_version_history") {
        expect(args).toMatchObject({ sourceId: reference.sourceId, relativePath: reference.relativePath, tool: row.tool, projectPath: null });
        return [snapshot] as never;
      }
      if (command === "agent_version_rollback") {
        expect(args).toMatchObject({ sourceId: reference.sourceId, relativePath: reference.relativePath, snapshotId: snapshot.id });
        return { ...installRecord("reviewer", row.dest), ...reference } as never;
      }
      if (command === "projects_list" || command === "skill_installs_reconcile") return [] as never;
      if (command === "storage_migration_status") return { state: "complete", stage: null, detail: null, legacyConflicts: [] } as never;
      if (command === "tools_list") return [staleControlTool] as never;
      if (command === "agent_sources_inspect") return [{
        source: { id: "agents", label: "Agents", enabled: true, kind: { kind: "builtIn" } },
        agents: [pkg], errors: [], revision: "recovery",
      }] as never;
      if (command === "agent_drafts_list") return [] as never;
      if (command === "agent_library_list") return emptyFolderState() as never;
      return [] as never;
    });
    agentLibrary.results = [{
      source: { id: "agents", label: "Agents", enabled: true, kind: { kind: "builtIn" } },
      agents: [pkg], errors: [], revision: "recovery",
    }];
    corpus.agents = [pkg.agent!];
    install.installed = [row];
    install.tools = [staleControlTool];
    install.reconciled = true;
    ui.section = "activity";
    ui.navStack = [];
    ui.navIndex = -1;
    ui.initNav();
    ui.openSettings("doctor");
    const { default: Settings } = await import("$lib/components/Settings.svelte");
    const { default: AgentsWorkspace } = await import("$lib/components/AgentsWorkspace.svelte");
    const settingsTarget = document.createElement("div");
    document.body.append(settingsTarget);
    const settingsComponent = mount(Settings, { target: settingsTarget });

    const openRecovery = async () => {
      const trigger = await vi.waitFor(() => {
        const candidate = settingsTarget.querySelector<HTMLButtonElement>("[data-agent-recovery]");
        expect(candidate).toBeTruthy();
        return candidate!;
      });
      const triggerId = trigger.dataset.recoveryTrigger!;
      trigger.click();
      expect(ui.settingsOpen).toBe(false);
      expect(ui.section).toBe("personas");
      return triggerId;
    };

    const firstTriggerId = await openRecovery();
    const firstOwnerTarget = document.createElement("div");
    document.body.append(firstOwnerTarget);
    const firstOwner = mount(AgentsWorkspace, { target: firstOwnerTarget });
    try {
      const rollback = await vi.waitFor(() => {
        const candidate = firstOwnerTarget.querySelector<HTMLButtonElement>(".snapshot button");
        expect(candidate?.textContent).toContain("Rollback");
        expect(document.activeElement).toBe(candidate);
        return candidate!;
      });
      expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "agent_version_rollback")).toBe(false);
      const done = [...firstOwnerTarget.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.trim() === "Done")!;
      done.click();
      await vi.waitFor(() => {
        expect(ui.settingsOpen).toBe(true);
        expect((document.activeElement as HTMLElement | null)?.dataset.recoveryTrigger).toBe(firstTriggerId);
      });
    } finally {
      unmount(firstOwner);
      firstOwnerTarget.remove();
    }

    const secondTriggerId = await openRecovery();
    const actionTarget = document.createElement("div");
    document.body.append(actionTarget);
    const actionOwner = mount(AgentsWorkspace, { target: actionTarget });
    try {
      const rollback = await vi.waitFor(() => {
        const candidate = actionTarget.querySelector<HTMLButtonElement>(".snapshot button");
        expect(candidate?.textContent).toContain("Rollback");
        return candidate!;
      });
      rollback.click();
      await tick();
      rollback.click();
      await vi.waitFor(() => {
        expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "agent_version_rollback")).toHaveLength(1);
        expect(ui.settingsOpen).toBe(true);
        expect((document.activeElement as HTMLElement | null)?.dataset.recoveryTrigger).toBe(secondTriggerId);
      });
    } finally {
      unmount(actionOwner);
      actionTarget.remove();
      unmount(settingsComponent);
      settingsTarget.remove();
    }
  });

  it("routes exact Skill recovery through its real rollback owner and returns to the originating Settings trigger", async () => {
    const installed: InstalledSkill = {
      sourceId: "skills", relativePath: "audit", name: "Audit", runtime: "codex", scope: "user",
      projectPath: null, path: "/private/.codex/skills/audit", state: "current", tracked: true,
    };
    const snapshot = { path: "/private/skill-backup", createdAt: "2026-08-17T00:00:00Z", contentHash: "a".repeat(64) };
    vi.mocked(invoke).mockImplementation(async (command: string, args) => {
      if (command === "doctor_report") return {
        generatedAt: "2026-08-17T00:00:00Z", overall: "healthy",
        counts: { healthy: 1, needsAttention: 0, unavailable: 0 }, copyText: "healthy", checks: [],
      } as never;
      if (command === "installs_reconcile" || command === "projects_list") return [] as never;
      if (command === "skill_installs_reconcile") return [installed] as never;
      if (command === "skill_version_history_list") {
        expect(args).toMatchObject({
          sourceId: installed.sourceId, relativePath: installed.relativePath,
          runtime: installed.runtime, projectPath: null,
        });
        return [snapshot] as never;
      }
      if (command === "skill_version_rollback") {
        expect(args).toMatchObject({
          sourceId: installed.sourceId, relativePath: installed.relativePath,
          runtime: installed.runtime, projectPath: null, snapshotPath: snapshot.path,
        });
        return installed as never;
      }
      if (command === "storage_migration_status") return { state: "complete", stage: null, detail: null, legacyConflicts: [] } as never;
      if (command === "skill_sources_inspect") return repairSkillInspection("a".repeat(64)) as never;
      if (command === "skill_drafts_list" || command === "skill_backups_list") return [] as never;
      if (command === "skill_folders_list" || command === "agent_library_list") return emptyFolderState() as never;
      return [] as never;
    });
    ui.section = "activity";
    ui.navStack = [];
    ui.navIndex = -1;
    ui.initNav();
    ui.openSettings("doctor");
    const { default: Settings } = await import("$lib/components/Settings.svelte");
    const { default: SkillsWorkspace } = await import("$lib/components/SkillsWorkspace.svelte");
    const settingsTarget = document.createElement("div");
    document.body.append(settingsTarget);
    const settingsComponent = mount(Settings, { target: settingsTarget });

    const openRecovery = async () => {
      const trigger = await vi.waitFor(() => {
        const candidate = settingsTarget.querySelector<HTMLButtonElement>("[data-skill-recovery]");
        expect(candidate).toBeTruthy();
        return candidate!;
      });
      const triggerId = trigger.dataset.recoveryTrigger!;
      trigger.click();
      expect(ui.settingsOpen).toBe(false);
      expect(ui.section).toBe("skills");
      return triggerId;
    };

    const firstTriggerId = await openRecovery();
    const firstOwnerTarget = document.createElement("div");
    document.body.append(firstOwnerTarget);
    const firstOwner = mount(SkillsWorkspace, { target: firstOwnerTarget });
    try {
      const rollback = await vi.waitFor(() => {
        const candidate = firstOwnerTarget.querySelector<HTMLButtonElement>("details[data-skill-history] li button");
        expect(candidate?.textContent).toContain("Rollback");
        expect(document.activeElement).toBe(candidate);
        return candidate!;
      });
      expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "skill_version_rollback")).toBe(false);
      rollback.closest("details")!.querySelector<HTMLElement>("summary")!.click();
      await vi.waitFor(() => {
        expect(ui.settingsOpen).toBe(true);
        expect((document.activeElement as HTMLElement | null)?.dataset.recoveryTrigger).toBe(firstTriggerId);
      });
    } finally {
      unmount(firstOwner);
      firstOwnerTarget.remove();
    }

    const secondTriggerId = await openRecovery();
    const actionTarget = document.createElement("div");
    document.body.append(actionTarget);
    const actionOwner = mount(SkillsWorkspace, { target: actionTarget });
    try {
      const rollback = await vi.waitFor(() => {
        const candidate = actionTarget.querySelector<HTMLButtonElement>("details[data-skill-history] li button");
        expect(candidate?.textContent).toContain("Rollback");
        return candidate!;
      });
      rollback.click();
      await vi.waitFor(() => {
        expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "skill_version_rollback")).toHaveLength(1);
        expect(ui.settingsOpen).toBe(true);
        expect((document.activeElement as HTMLElement | null)?.dataset.recoveryTrigger).toBe(secondTriggerId);
      });
    } finally {
      unmount(actionOwner);
      actionTarget.remove();
      unmount(settingsComponent);
      settingsTarget.remove();
    }
  });

  it.each(["install", "update", "track", "uninstall"] as const)(
    "records one exact %s bulk receipt and continues after failure",
    async (action) => {
      install.installed = action === "install" ? [] : [
        { ...staleControlRow, slug: "good", name: "Good", dest: "/before/good.md", state: "outdated", tracked: true },
        { ...staleControlRow, slug: "broken", name: "Broken", dest: "/before/broken.md", state: "outdated", tracked: true },
      ];
      vi.mocked(invoke).mockImplementation(async (command: string, args?: unknown) => {
        if (command === "installs_reconcile" || command === "tools_list") return [] as never;
        const slug = (args as { slug?: string } | undefined)?.slug;
        if (slug === "broken") throw new Error("token=secret123 failed");
        if (command === "uninstall_agent") return undefined as never;
        return installRecord(slug ?? "unknown", `/after/${slug}.md`) as never;
      });
      const priorIds = new Set(activity.entries.map((entry) => entry.id));

      const result = await install.bulk(action, [
        { slug: "good", tool: "claudeCode", projectPath: null },
        { slug: "broken", tool: "claudeCode", projectPath: null },
      ]);
      expect(result).toMatchObject({ ok: 1, fail: 1, receiptId: expect.any(String) });
      const receipt = activity.entries.find((entry) => entry.id === result.receiptId)?.receipt;
      expect(receipt).toMatchObject({ operation: action, succeeded: 1, failed: 1 });
      expect(receipt?.items).toHaveLength(2);
      expect(receipt?.items[0]).toMatchObject({ name: "good", outcome: "ok" });
      expect(receipt?.items[0]?.destination).toBe(action === "uninstall" ? "/before/good.md" : "/after/good.md");
      expect(receipt?.items[1]).toMatchObject({ name: "broken", outcome: "error" });
      expect(receipt?.items[1]?.destination).toBe(action === "install" ? null : "/before/broken.md");
      expect(receipt?.items[1]?.detail).toContain("token=[redacted]");
      expect(receipt?.items[1]?.detail).not.toContain("secret123");
      expect(activity.entries.filter((entry) => !priorIds.has(entry.id))).toHaveLength(1);
    },
  );

  it.each(["batch", "collection"] as const)(
    "records exact destinations for reviewed Agent %s application",
    async (kind) => {
      const plan: AgentMutationPlan = {
        revision: "revision-1", operation: "install", tool: "claudeCode", scope: "user", projectPath: null,
        agents: [
          { reference: { sourceId: "built-in", relativePath: "reviewer.md" }, name: "Reviewer", sourceHash: "one", dependency: false, destination: "/plan/reviewer.md", renderedFileCount: 1, capabilities: [] },
          { reference: { sourceId: "built-in", relativePath: "audit.md" }, name: "Audit", sourceHash: "two", dependency: false, destination: "/plan/audit.md", renderedFileCount: 1, capabilities: [] },
        ],
        warnings: [], blockers: [], rollbackAvailable: false,
      };
      const records = [installRecord("reviewer", "/actual/reviewer.md"), installRecord("audit", "/actual/audit.md")];
      records[0] = {
        ...records[0],
        deploymentNotice: "Workspace files installed; OpenClaw registration and restart remain required.",
      } as InstallRecord;
      vi.mocked(invoke).mockImplementation(async (command: string) => {
        if (command === "agent_batch_apply" || command === "agent_collection_apply") return records as never;
        if (command === "installs_reconcile" || command === "tools_list") return [] as never;
        return [] as never;
      });

      const result = kind === "batch"
        ? await install.applyBatch(plan)
        : await install.applyCollection("Review team", plan);
      expect(result).toMatchObject({ records, receiptId: expect.any(String) });
      const receipt = activity.entries.find((entry) => entry.id === result.receiptId)?.receipt;
      expect(receipt).toMatchObject({ operation: "install", succeeded: 2, failed: 0 });
      expect(receipt?.items.map((item) => item.destination)).toEqual(["/actual/reviewer.md", "/actual/audit.md"]);
      expect(receipt?.items.map((item) => item.name)).toEqual(["Reviewer", "Audit"]);
      expect(receipt?.items.map((item) => item.detail)).toEqual([
        "Workspace files installed; OpenClaw registration and restart remain required.",
        undefined,
      ]);
    },
  );

  it("surfaces a sanitized OpenClaw activation notice through the mounted exact install path", async () => {
    const notice = "Workspace files installed; OpenClaw registration and restart remain required.\napi_key=secret123";
    const plan: AgentMutationPlan = {
      revision: "openclaw-plan", operation: "install", tool: "openclaw", scope: "user", projectPath: null,
      agents: [{
        reference: staleControlPackage.reference,
        name: "Reviewer",
        sourceHash: "source",
        dependency: false,
        destination: "/tmp/.openclaw/agency-agents/reviewer/SOUL.md",
        renderedFileCount: 3,
        capabilities: [],
      }],
      warnings: [], blockers: [], rollbackAvailable: true,
    };
    const record = {
      ...installRecord("reviewer", plan.agents[0].destination),
      tool: "openclaw",
      deploymentNotice: notice,
    } as InstallRecord;
    const openclawTool = {
      ...staleControlTool,
      tool: "openclaw" as const,
      label: "OpenClaw",
      userDest: "/tmp/.openclaw/agency-agents",
      installedCount: 0,
    };
    install.installed = [];
    install.tools = [openclawTool];
    install.reconciled = true;
    projects.list = [];
    const priorIds = new Set(activity.entries.map((entry) => entry.id));
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "projects_list") return [] as never;
      if (command === "tools_list") return [openclawTool] as never;
      if (command === "installs_reconcile") return [] as never;
      if (command === "agent_install_plan") return plan as never;
      if (command === "agent_install_with_dependencies") return [record] as never;
      return [] as never;
    });
    const { default: InstallModal } = await import("$lib/components/InstallModal.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(InstallModal, {
      target,
      props: {
        title: "Install Reviewer",
        agentPackage: staleControlPackage,
        allowedTools: ["openclaw"],
        onClose: vi.fn(),
      },
    });
    try {
      await vi.waitFor(() => expect(target.querySelector<HTMLButtonElement>(".grid-wrap .toggle")?.disabled).toBe(false));
      target.querySelector<HTMLButtonElement>(".grid-wrap .toggle")!.click();
      const apply = await vi.waitFor(() => {
        const button = [...target.querySelectorAll<HTMLButtonElement>("button")]
          .find((candidate) => candidate.textContent?.trim() === "Apply plan");
        expect(button).toBeTruthy();
        return button!;
      });
      apply.click();
      await vi.waitFor(() => expect(invokeMock.mock.calls
        .filter(([command]) => command === "agent_install_with_dependencies")).toHaveLength(1));

      const entry = await vi.waitFor(() => {
        const candidate = activity.entries.find((item) => !priorIds.has(item.id));
        expect(candidate?.detail).toBe("Workspace files installed; OpenClaw registration and restart remain required. api_key=[redacted]");
        return candidate!;
      });
      expect(toast.items.at(-1)?.body).toBe(entry?.detail);
      expect(toast.items.at(-1)?.body).not.toContain("secret123");
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("links every Agent bulk completion surface to its exact Activity receipt", () => {
    for (const source of [agentsWorkspaceSource, toolsViewSource, deployBrowserSource, installModalSource]) {
      expect(source).toContain("ui.openActivityReceipt(receiptId)");
    }
    expect(installModalSource).toContain("install.applyCollection(pendingCollection, plan)");
  });

  it("guards the reviewed local Ollama lifecycle from Agent detail", () => {
    const detailSource = rel01Sources["./components/AgentDetailTabs.svelte"];
    expect(detailSource).toContain("onLocalModel");
    expect(agentsWorkspaceSource).toContain("<OllamaDeployModal");
    expect(agentsWorkspaceSource).toContain("onLocalModel={() => (ollamaOpen = true)}");

    expect(ollamaDeployModalSource).toContain("ollamaStatus()");
    expect(ollamaDeployModalSource).toContain("ollamaPlan(pkg.reference, operation, baseModel)");
    expect(ollamaDeployModalSource).toContain("ollamaApply(pkg.reference, plan.operation, plan.baseModel, plan.revision)");
    expect(ollamaDeployModalSource).toContain('"This device"');
    expect(ollamaDeployModalSource).toContain("plan.promptPreview");
    expect(ollamaDeployModalSource).toContain("plan.blockers");
    expect(ollamaDeployModalSource).toContain("plan.warnings");
    expect(ollamaDeployModalSource).toContain("statusError");
    expect(ollamaDeployModalSource).toContain("activity.log");
    expect(ollamaDeployModalSource).toContain("safeActivityDetail");
    expect(ollamaDeployModalSource).toContain("ui.openActivityReceipt(receiptId)");
    expect(ollamaDeployModalSource).not.toContain("detail: plan.promptPreview");
    expect(ollamaDeployModalSource).not.toContain("destination: plan.promptPreview");
  });

  it("renders semantic AppErrors in both Agent source alerts", async () => {
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockRejectedValue({ code: "io", message: "scan failed" });
    const { default: AgentDetailTabs } = await import("$lib/components/AgentDetailTabs.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const agent: Agent = {
      slug: "reviewer", name: "Reviewer", description: "Reviews code", category: "engineering",
      emoji: null, color: null, vibe: null, body: "Review carefully.",
    };
    const pkg: AgentPackageResult = {
      reference: { sourceId: "local", relativePath: "reviewer.md" }, agent,
      sourceHash: "source", frontmatterHash: "frontmatter", bodyHash: "body", version: null,
      channel: null, changelog: null, publisher: null, publisherKey: null,
      publisherVerified: false, requiredAgents: [], requiredSkills: [], recommendedAgents: [],
      groups: [], tags: [], capabilities: [], permissions: [], qualityScore: 100,
      qualityChecks: [], diagnostics: [], installable: true,
    };
    const source: AgentSource = {
      id: "local", label: "Local", enabled: true, kind: { kind: "local", root: "/tmp/agents" },
    };
    const component = mount(AgentDetailTabs, {
      target,
      props: {
        agent, pkg, source,
        onCategory: vi.fn(), onInstall: vi.fn(), onDiff: vi.fn(),
      },
    });
    try {
      target.querySelector<HTMLButtonElement>('[data-agent-tab="source"]')!.click();
      await vi.waitFor(() => expect(target.querySelectorAll('[role="alert"]')).toHaveLength(2));
      for (const alert of target.querySelectorAll('[role="alert"]')) {
        expect(alert.textContent).toContain("I/O error: scan failed");
        expect(alert.textContent).not.toContain("[object Object]");
      }
    } finally {
      unmount(component);
      target.remove();
      invokeMock.mockImplementation(async (command: string) => command === "skill_folders_list"
        ? {
            folders: [], assignments: [], favorites: [], recent: [], collections: [], smartFolders: [],
            profiles: [], updatePolicies: [], publisherTrust: [], preferredSources: [], usage: [], approvals: [],
          } as never
        : [] as never);
    }
  });

  it("keeps native Error fallback text in both Agent source alerts", async () => {
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockRejectedValue(new Error("preview unavailable"));
    const { default: AgentDetailTabs } = await import("$lib/components/AgentDetailTabs.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const agent: Agent = {
      slug: "reviewer", name: "Reviewer", description: "Reviews code", category: "engineering",
      emoji: null, color: null, vibe: null, body: "Review carefully.",
    };
    const pkg: AgentPackageResult = {
      reference: { sourceId: "local", relativePath: "reviewer.md" }, agent,
      sourceHash: "source", frontmatterHash: "frontmatter", bodyHash: "body", version: null,
      channel: null, changelog: null, publisher: null, publisherKey: null,
      publisherVerified: false, requiredAgents: [], requiredSkills: [], recommendedAgents: [],
      groups: [], tags: [], capabilities: [], permissions: [], qualityScore: 100,
      qualityChecks: [], diagnostics: [], installable: true,
    };
    const source: AgentSource = {
      id: "local", label: "Local", enabled: true, kind: { kind: "local", root: "/tmp/agents" },
    };
    const component = mount(AgentDetailTabs, {
      target,
      props: {
        agent, pkg, source,
        onCategory: vi.fn(), onInstall: vi.fn(), onDiff: vi.fn(),
      },
    });
    try {
      target.querySelector<HTMLButtonElement>('[data-agent-tab="source"]')!.click();
      await vi.waitFor(() => expect(target.querySelectorAll('[role="alert"]')).toHaveLength(2));
      for (const alert of target.querySelectorAll('[role="alert"]')) {
        expect(alert.textContent).toContain("Error: preview unavailable");
      }
    } finally {
      unmount(component);
      target.remove();
      invokeMock.mockImplementation(async (command: string) => command === "skill_folders_list"
        ? {
            folders: [], assignments: [], favorites: [], recent: [], collections: [], smartFolders: [],
            profiles: [], updatePolicies: [], publisherTrust: [], preferredSources: [], usage: [], approvals: [],
          } as never
        : [] as never);
    }
  });

  it.each([
    ["catalog status", "catalog_status", { code: "io", message: "scan failed" }, () => catalog.loadStatus(), "I/O error: scan failed"],
    ["corrupt settings", "settings_get", { code: "internal", message: "settings unreadable" }, () => settings.load(), "Internal error: settings unreadable"],
    ["invalid settings", "settings_set", { code: "invalid_argument", message: "setting is invalid" }, () => settings.save({ paranoidMode: true }), "Invalid argument: setting is invalid"],
    ["settings reset", "settings_reset", { code: "storage_busy" }, () => settings.reset(), "Agency Agents is busy in another desktop or MCP session. Close it and try again."],
    ["Expert load", "experts_list", { code: "network", url: "https://example.test", message: "offline" }, () => experts.load(), "Network error: offline"],
  ])("renders semantic %s store failures", async (_label, command, payload, run, expected) => {
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (candidate: string) => {
      if (candidate === command) throw payload;
      return [] as never;
    });
    catalog.error = null;
    settings.error = null;
    experts.error = null;

    await run();

    const actual = command.startsWith("catalog_")
      ? catalog.error
      : command.startsWith("settings_")
        ? settings.error
        : experts.error;
    expect(actual).toBe(expected);
  });

  it("loads the catalog feed on initial render", async () => {
    let resolveFeed!: (feed: CatalogFeedState) => void;
    const feedResponse = new Promise<CatalogFeedState>((resolve) => { resolveFeed = resolve; });
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "catalog_source_get") return { kind: "bundled" } as never;
      if (command === "catalog_configured") return true as never;
      if (command === "catalog_status") return catalogStatusFixture as never;
      if (command === "catalog_detect") return { gitAvailable: false, scanned: false, candidates: [] } as never;
      if (command === "github_status") return { signedIn: false, username: null, scopes: [] } as never;
      if (command === "catalog_feed_list") return feedResponse as never;
      return [] as never;
    });
    const { default: SettingsSectionCatalog } = await import("$lib/components/SettingsSectionCatalog.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(SettingsSectionCatalog, { target });
    try {
      await vi.waitFor(() => expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "catalog_feed_list")).toBe(true));
      expect(target.querySelector(".feed")?.textContent).toContain("Loading");
      resolveFeed({ lastSuccessAt: null, stale: false, error: null, batches: [] });
      await vi.waitFor(() => expect(target.querySelector(".feed")?.textContent).toContain("No successful catalog refreshes yet"));
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("renders catalog feed history outside one concise live status", async () => {
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "catalog_source_get") return { kind: "bundled" } as never;
      if (command === "catalog_configured") return true as never;
      if (command === "catalog_status") return catalogStatusFixture as never;
      if (command === "catalog_detect") return { gitAvailable: false, scanned: false, candidates: [] } as never;
      if (command === "github_status") return { signedIn: false, username: null, scopes: [] } as never;
      if (command === "catalog_feed_list") return catalogFeedFixture() as never;
      return [] as never;
    });
    const { default: SettingsSectionCatalog } = await import("$lib/components/SettingsSectionCatalog.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(SettingsSectionCatalog, { target });
    try {
      await vi.waitFor(() => expect(target.textContent).toContain("engineering/reviewer.md"));
      expect(target.querySelectorAll('.feed [role="status"][aria-live="polite"]')).toHaveLength(1);
      expect(target.querySelector(".batches")?.closest('[aria-live="polite"]')).toBeNull();
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("retains prior catalog batches and timestamp when refresh fails", async () => {
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "catalog_source_get") return { kind: "bundled" } as never;
      if (command === "catalog_configured") return true as never;
      if (command === "catalog_status") return catalogStatusFixture as never;
      if (command === "catalog_detect") return { gitAvailable: false, scanned: false, candidates: [] } as never;
      if (command === "github_status") return { signedIn: false, username: null, scopes: [] } as never;
      if (command === "catalog_feed_list") return catalogFeedFixture() as never;
      if (command === "catalog_source_transition_recover") return false as never;
      if (command === "catalog_pull") throw { code: "network", url: "https://example.test", message: "offline" };
      return [] as never;
    });
    const { default: SettingsSectionCatalog } = await import("$lib/components/SettingsSectionCatalog.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(SettingsSectionCatalog, { target });
    try {
      await vi.waitFor(() => expect(target.textContent).toContain("engineering/reviewer.md"));
      const timestamp = target.querySelector(".feed-head .hint")?.textContent;
      target.querySelector<HTMLButtonElement>("button.primary")!.click();
      await vi.waitFor(() => expect(target.querySelector(".feed-error")?.textContent).toContain("Network error: offline"));
      expect(target.textContent).toContain("engineering/reviewer.md");
      expect(target.querySelector(".feed-head .hint")?.textContent).toBe(timestamp);
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("runs local recovery before the normal writable Retry pull", async () => {
    let feedCalls = 0;
    const retryCommands: string[] = [];
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "catalog_source_get") return { kind: "bundled" } as never;
      if (command === "catalog_configured") return true as never;
      if (command === "catalog_status") return catalogStatusFixture as never;
      if (command === "catalog_detect") return { gitAvailable: false, scanned: false, candidates: [] } as never;
      if (command === "github_status") return { signedIn: false, username: null, scopes: [] } as never;
      if (command === "catalog_feed_list") {
        feedCalls += 1;
        return (feedCalls === 1
          ? catalogFeedFixture("engineering/old.md", true, "Previous refresh failed")
          : catalogFeedFixture("engineering/recovered.md")) as never;
      }
      if (command === "catalog_source_transition_recover") {
        retryCommands.push(command);
        return false as never;
      }
      if (command === "catalog_pull") retryCommands.push(command);
      if (command === "catalog_pull") return { version: "test", commit: null, fetchedAt: "2026-08-17T00:01:00Z", count: 1 } as never;
      if (command === "corpus_list" || command === "corpus_categories") return [] as never;
      return [] as never;
    });
    const { default: SettingsSectionCatalog } = await import("$lib/components/SettingsSectionCatalog.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(SettingsSectionCatalog, { target });
    try {
      await vi.waitFor(() => expect(target.querySelector(".feed-error")?.textContent).toContain("Previous refresh failed"));
      target.querySelector<HTMLButtonElement>(".feed-error button")!.click();
      await vi.waitFor(() => expect(target.textContent).toContain("engineering/recovered.md"));
      expect(target.querySelector(".feed-error")).toBeNull();
      expect(feedCalls).toBe(2);
      expect(retryCommands).toEqual(["catalog_source_transition_recover", "catalog_pull"]);
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("keeps read-only Retry enabled and stops after local transition recovery", async () => {
    let feedCalls = 0;
    const retryCommands: string[] = [];
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "catalog_source_get") return { kind: "userClone", path: "/read-only", manage: false } as never;
      if (command === "catalog_configured") return true as never;
      if (command === "catalog_status") return { ...catalogStatusFixture, source: { kind: "userClone", path: "/read-only", manage: false } } as never;
      if (command === "catalog_detect") return { gitAvailable: false, scanned: false, candidates: [] } as never;
      if (command === "github_status") return { signedIn: false, username: null, scopes: [] } as never;
      if (command === "catalog_feed_list") {
        feedCalls += 1;
        return (feedCalls === 1
          ? catalogFeedFixture("engineering/hidden.md", true, "Catalog source change is incomplete")
          : { lastSuccessAt: null, stale: false, error: null, batches: [] }) as never;
      }
      if (command === "catalog_source_transition_recover") {
        retryCommands.push(command);
        return true as never;
      }
      if (command === "catalog_pull") retryCommands.push(command);
      return [] as never;
    });
    const { default: SettingsSectionCatalog } = await import("$lib/components/SettingsSectionCatalog.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(SettingsSectionCatalog, { target });
    try {
      await vi.waitFor(() => expect(target.querySelector(".feed-error")?.textContent).toContain("Catalog source change is incomplete"));
      const retry = target.querySelector<HTMLButtonElement>(".feed-error button")!;
      expect(retry.disabled).toBe(false);
      retry.click();
      await vi.waitFor(() => expect(target.querySelector(".feed-error")).toBeNull());
      expect(feedCalls).toBe(2);
      expect(retryCommands).toEqual(["catalog_source_transition_recover"]);
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("reloads honest stale state without network when read-only Retry has no transition", async () => {
    let feedCalls = 0;
    const retryCommands: string[] = [];
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "catalog_source_get") return { kind: "userClone", path: "/read-only", manage: false } as never;
      if (command === "catalog_configured") return true as never;
      if (command === "catalog_status") return { ...catalogStatusFixture, source: { kind: "userClone", path: "/read-only", manage: false } } as never;
      if (command === "catalog_detect") return { gitAvailable: false, scanned: false, candidates: [] } as never;
      if (command === "github_status") return { signedIn: false, username: null, scopes: [] } as never;
      if (command === "catalog_feed_list") {
        feedCalls += 1;
        return catalogFeedFixture("engineering/retained.md", true, "Read-only catalog remains stale") as never;
      }
      if (command === "catalog_source_transition_recover") {
        retryCommands.push(command);
        return false as never;
      }
      if (command === "catalog_pull") retryCommands.push(command);
      return [] as never;
    });
    const { default: SettingsSectionCatalog } = await import("$lib/components/SettingsSectionCatalog.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(SettingsSectionCatalog, { target });
    try {
      await vi.waitFor(() => expect(target.querySelector(".feed-error")?.textContent).toContain("Read-only catalog remains stale"));
      const retry = target.querySelector<HTMLButtonElement>(".feed-error button")!;
      expect(retry.disabled).toBe(false);
      retry.click();
      await vi.waitFor(() => expect(feedCalls).toBe(2));
      expect(retryCommands).toEqual(["catalog_source_transition_recover"]);
      expect(target.querySelector(".feed-error")?.textContent).toContain("Read-only catalog remains stale");
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("reloads and clears stale catalog feed state after a source switch", async () => {
    let feedCalls = 0;
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "catalog_source_get") return { kind: "bundled" } as never;
      if (command === "catalog_configured") return true as never;
      if (command === "catalog_status") return catalogStatusFixture as never;
      if (command === "catalog_detect") return { gitAvailable: false, scanned: false, candidates: [] } as never;
      if (command === "github_status") return { signedIn: false, username: null, scopes: [] } as never;
      if (command === "catalog_feed_list") {
        feedCalls += 1;
        return (feedCalls === 1
          ? catalogFeedFixture("engineering/old-source.md", true, "Old source failed")
          : { lastSuccessAt: null, stale: false, error: null, batches: [] }) as never;
      }
      if (command === "catalog_source_set") return { version: "test", commit: null, fetchedAt: "2026-08-17T00:01:00Z", count: 0 } as never;
      if (command === "corpus_list" || command === "corpus_categories") return [] as never;
      return [] as never;
    });
    const { default: SettingsSectionCatalog } = await import("$lib/components/SettingsSectionCatalog.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(SettingsSectionCatalog, { target });
    try {
      await vi.waitFor(() => expect(target.querySelector(".feed-error")?.textContent).toContain("Old source failed"));
      target.querySelectorAll<HTMLButtonElement>("button.card")[1]!.click();
      await vi.waitFor(() => expect(feedCalls).toBe(2));
      expect(target.querySelector(".feed-error")).toBeNull();
      expect(target.textContent).not.toContain("engineering/old-source.md");
      expect(target.querySelector(".feed")?.textContent).toContain("No successful catalog refreshes yet");
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("renders semantic catalog action failures in the existing toast", async () => {
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "catalog_source_get") return { kind: "bundled" } as never;
      if (command === "catalog_configured") return true as never;
      if (command === "catalog_status") return { isGit: false, repoSlug: null } as never;
      if (command === "catalog_detect") return { candidates: [] } as never;
      if (command === "catalog_source_transition_recover") return false as never;
      if (command === "catalog_pull") throw { code: "http_status", url: "https://example.test/catalog", status: 503 };
      return [] as never;
    });
    toast.items = [];
    const { default: SettingsSectionCatalog } = await import("$lib/components/SettingsSectionCatalog.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(SettingsSectionCatalog, { target });
    try {
      await tick();
      target.querySelector<HTMLButtonElement>("button.primary")!.click();
      await vi.waitFor(() => {
        expect(toast.items.at(-1)?.body).toBe("HTTP 503 from https://example.test/catalog");
      });
    } finally {
      unmount(component);
      target.remove();
      toast.items = [];
      invokeMock.mockImplementation(async (command: string) => command === "skill_folders_list"
        ? {
            folders: [], assignments: [], favorites: [], recent: [], collections: [], smartFolders: [],
            profiles: [], updatePolicies: [], publisherTrust: [], preferredSources: [], usage: [], approvals: [],
          } as never
        : [] as never);
    }
  });

  it("renders semantic application failures in the existing Diff inline error", async () => {
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "agent_diff") throw { code: "io", message: "diff unavailable" };
      return [] as never;
    });
    const { default: DiffModal } = await import("$lib/components/DiffModal.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(DiffModal, {
      target,
      props: {
        slug: "reviewer",
        tool: "claudeCode",
        projectPath: null,
        name: "Reviewer",
        onClose: vi.fn(),
      },
    });
    try {
      await vi.waitFor(() => {
        expect(target.querySelector(".err")?.textContent).toBe("I/O error: diff unavailable");
      });
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("renders semantic first-run failures in the existing toast", async () => {
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "catalog_detect") return { gitAvailable: true, scanned: false, candidates: [] } as never;
      if (command === "catalog_provision_managed") throw { code: "storage_corrupt", message: "catalog state is invalid" };
      return [] as never;
    });
    catalog.configured = false;
    catalog.busy = false;
    toast.items = [];
    const { default: CatalogFirstRun } = await import("$lib/components/CatalogFirstRun.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(CatalogFirstRun, { target });
    try {
      await tick();
      target.querySelectorAll<HTMLButtonElement>("button.card.simple")[0]!.click();
      await vi.waitFor(() => {
        expect(toast.items.at(-1)?.body).toBe("Stored data needs attention: catalog state is invalid");
      });
    } finally {
      unmount(component);
      target.remove();
      toast.items = [];
    }
  });

  it("blocks deployment without a detected target and defers without an Agent write", async () => {
    const invokeMock = vi.mocked(invoke);
    catalog.configured = true;
    install.reconciled = true;
    const onFinish = vi.fn();
    const { default: CatalogFirstRun } = await import("$lib/components/CatalogFirstRun.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(CatalogFirstRun, { target, props: { onFinish } });
    try {
      await tick();
      expect(target.textContent).toContain("No supported target detected");
      expect(target.querySelector<HTMLButtonElement>("button.primary")?.disabled).toBe(true);
      target.querySelector<HTMLButtonElement>("button.ghost")!.click();
      expect(onFinish).toHaveBeenCalledOnce();
      expect(localStorage.getItem("agency-agents:first-deployment")).toBe("v1");
      expect(invokeMock.mock.calls.some(([command]) => String(command).includes("install"))).toBe(false);
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("keeps the native clipboard Error fallback in Experts", async () => {
    const expert: ExpertResolved = {
      id: "reviewer",
      name: "Reviewer",
      summary: "Reviews code",
      category: "Engineering",
      tags: [],
      version: 1,
      leadAgent: "reviewer",
      supportingAgents: [],
      requiredSkills: [],
      optionalSkills: [],
      runbook: null,
      preferredClient: null,
      starterPrompt: "Review carefully.",
      qualityContract: { version: 1, checks: [] },
      source: "curated",
      unresolvedAgents: [],
      unresolvedSkills: [],
      unresolvedRunbook: false,
    };
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "experts_list") return [expert] as never;
      return [] as never;
    });
    vi.stubGlobal("navigator", {
      clipboard: { writeText: vi.fn().mockRejectedValue(new Error("permission denied")) },
    });
    vi.stubGlobal("localStorage", {
      getItem: () => null,
      setItem: () => undefined,
      removeItem: () => undefined,
    });
    toast.items = [];
    const { default: Experts } = await import("$lib/components/Experts.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(Experts, { target });
    try {
      await vi.waitFor(() => expect(target.textContent).toContain("Review carefully."));
      [...target.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.trim() === "Copy")!
        .click();
      await vi.waitFor(() => {
        expect(toast.items.at(-1)?.body).toBe("Error: permission denied");
      });
    } finally {
      unmount(component);
      target.remove();
      toast.items = [];
      vi.unstubAllGlobals();
      invokeMock.mockImplementation(async (command: string) => command === "skill_folders_list"
        ? {
            folders: [], assignments: [], favorites: [], recent: [], collections: [], smartFolders: [],
            profiles: [], updatePolicies: [], publisherTrust: [], preferredSources: [], usage: [], approvals: [],
          } as never
        : [] as never);
    }
  });

  it("keeps the durable terminal MCP outcome when local Activity is cleared", () => {
    const attempt: McpAuditEntry = {
      id: "approval-1", timestamp: "2026-08-04T10:00:00Z", client: "codex", tool: "agents_uninstall",
      action: "agent_destructive", phase: "attempt", success: false, projectPath: null,
    };
    const terminal = { ...attempt, timestamp: "2026-08-04T10:01:00Z", phase: "terminal" as const, success: true };
    expect(selectMcpAuditEntries([attempt, terminal])).toEqual([terminal]);
    const durable: JournalEntry = {
      id: "mcp:approval-1", ts: terminal.timestamp, action: "mcp", subject: "mcp",
      subjectName: terminal.tool, outcome: "ok", detail: "codex · agent_destructive",
    };
    expect(mergeActivityEntries([], [durable])).toEqual([durable]);
  });

  it("focuses the safe modal action, traps Tab, and handles Escape", async () => {
    const onClose = vi.fn();
    const target = document.createElement("div");
    document.body.append(target);
    const children = createRawSnippet(() => ({
      render: () => '<div><button data-modal-action="cancel">Cancel</button><button>Other</button></div>',
    }));
    const component = mount(Modal, { target, props: { open: true, title: "Contract", onClose, children } });
    await tick();
    const buttons = target.querySelectorAll<HTMLButtonElement>(".body button");
    expect(document.activeElement).toBe(buttons[0]);
    buttons[1].focus();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true }));
    expect(document.activeElement).toBe(target.querySelector("button.close"));
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    expect(onClose).toHaveBeenCalledOnce();
    unmount(component);
    target.remove();
  });

  it("debounces rapid foreground reconciliation for both install ledgers", async () => {
    vi.useFakeTimers();
    vi.stubGlobal("matchMedia", vi.fn(() => ({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })));
    const agentReconcile = vi.spyOn(install, "reconcile").mockResolvedValue();
    const projectRefresh = vi.spyOn(projects, "refresh").mockResolvedValue();
    const skillReconcile = vi.spyOn(skillSources, "reconcileInstalls").mockResolvedValue();
    projects.list = [{ path: "/tmp/project", label: "project", installedCount: 0 }];
    const { default: Layout } = await import("../routes/+layout.svelte");
    const target = document.createElement("div");
    const children = createRawSnippet(() => ({ render: () => "<main>App</main>" }));
    document.body.append(target);
    const component = mount(Layout, { target, props: { children } });
    try {
      await tick();
      await Promise.resolve();
      agentReconcile.mockClear();
      projectRefresh.mockClear();
      skillReconcile.mockClear();

      window.dispatchEvent(new Event("focus"));
      window.dispatchEvent(new Event("focus"));
      window.dispatchEvent(new Event("focus"));
      await vi.advanceTimersByTimeAsync(249);
      expect(agentReconcile).not.toHaveBeenCalled();
      expect(skillReconcile).not.toHaveBeenCalled();

      await vi.advanceTimersByTimeAsync(1);
      await Promise.resolve();
      expect(agentReconcile).toHaveBeenCalledOnce();
      expect(projectRefresh).not.toHaveBeenCalled();
      expect(skillReconcile).toHaveBeenCalledOnce();
      expect(skillReconcile).toHaveBeenCalledWith(["/tmp/project"]);
    } finally {
      unmount(component);
      target.remove();
      agentReconcile.mockRestore();
      projectRefresh.mockRestore();
      skillReconcile.mockRestore();
      vi.useRealTimers();
    }
  });

  it("shares mount reconciliation when focus overlaps the in-flight scan", async () => {
    vi.useFakeTimers();
    vi.stubGlobal("matchMedia", vi.fn(() => ({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })));
    let resolveAgent!: (rows: InstalledAgent[]) => void;
    let resolveSkills!: (rows: InstalledSkill[]) => void;
    const agentScan = new Promise<InstalledAgent[]>((resolve) => (resolveAgent = resolve));
    const skillScan = new Promise<InstalledSkill[]>((resolve) => (resolveSkills = resolve));
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "installs_reconcile") return agentScan as never;
      if (command === "skill_installs_reconcile") return skillScan as never;
      if (command === "skill_backups_list") return [] as never;
      return [] as never;
    });
    const projectRefresh = vi.spyOn(projects, "refresh").mockResolvedValue();
    const { default: Layout } = await import("../routes/+layout.svelte");
    const target = document.createElement("div");
    const children = createRawSnippet(() => ({ render: () => "<main>App</main>" }));
    document.body.append(target);
    const component = mount(Layout, { target, props: { children } });
    try {
      await tick();
      await Promise.resolve();
      expect(install.reconcileAttempt).toBe(1);
      expect(skillSources.reconcileAttempt).toBe(1);

      window.dispatchEvent(new Event("focus"));
      await vi.advanceTimersByTimeAsync(250);
      expect(invokeMock.mock.calls.filter(([command]) => command === "installs_reconcile")).toHaveLength(1);
      expect(invokeMock.mock.calls.filter(([command]) => command === "skill_installs_reconcile")).toHaveLength(1);
      expect(install.reconcileAttempt).toBe(1);
      expect(skillSources.reconcileAttempt).toBe(1);

      resolveAgent([]);
      resolveSkills([]);
      await Promise.all([agentScan, skillScan]);
      await Promise.resolve();
    } finally {
      unmount(component);
      target.remove();
      projectRefresh.mockRestore();
      vi.useRealTimers();
    }
  });

  it("cancels pending foreground work when the root layout unmounts", async () => {
    vi.useFakeTimers();
    vi.stubGlobal("matchMedia", vi.fn(() => ({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })));
    const agentReconcile = vi.spyOn(install, "reconcile").mockResolvedValue();
    const projectRefresh = vi.spyOn(projects, "refresh").mockResolvedValue();
    const skillReconcile = vi.spyOn(skillSources, "reconcileInstalls").mockResolvedValue();
    const { default: Layout } = await import("../routes/+layout.svelte");
    const target = document.createElement("div");
    const children = createRawSnippet(() => ({ render: () => "<main>App</main>" }));
    document.body.append(target);
    const component = mount(Layout, { target, props: { children } });
    await tick();
    await Promise.resolve();
    agentReconcile.mockClear();
    projectRefresh.mockClear();
    skillReconcile.mockClear();

    window.dispatchEvent(new Event("focus"));
    unmount(component);
    await vi.advanceTimersByTimeAsync(250);
    expect(agentReconcile).not.toHaveBeenCalled();
    expect(projectRefresh).not.toHaveBeenCalled();
    expect(skillReconcile).not.toHaveBeenCalled();

    target.remove();
    agentReconcile.mockRestore();
    projectRefresh.mockRestore();
    skillReconcile.mockRestore();
    vi.useRealTimers();
  });

  it("uses only local read commands for foreground reconciliation", async () => {
    vi.useFakeTimers();
    vi.stubGlobal("matchMedia", vi.fn(() => ({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })));
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "projects_list") return [{ path: "/tmp/project", label: "project", installedCount: 0 }] as never;
      return [] as never;
    });
    const { default: Layout } = await import("../routes/+layout.svelte");
    const target = document.createElement("div");
    const children = createRawSnippet(() => ({ render: () => "<main>App</main>" }));
    document.body.append(target);
    const component = mount(Layout, { target, props: { children } });
    try {
      await tick();
      await vi.waitFor(() => expect(projects.list).toHaveLength(1));
      await vi.waitFor(() => expect(skillSources.reconciling).toBe(false));
      invokeMock.mockClear();

      window.dispatchEvent(new Event("focus"));
      await vi.advanceTimersByTimeAsync(250);
      await vi.waitFor(() => expect(skillSources.reconciling).toBe(false));
      expect(invokeMock.mock.calls.map(([command]) => command)).toEqual([
        "installs_reconcile",
        "skill_installs_reconcile",
        "skill_backups_list",
      ]);
    } finally {
      unmount(component);
      target.remove();
      vi.useRealTimers();
    }
  });

  it("retains both ledgers when foreground reconciliation fails and recovers on retry", async () => {
    vi.useFakeTimers();
    vi.stubGlobal("matchMedia", vi.fn(() => ({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })));
    const agentRow = staleControlRow;
    const skillRow: InstalledSkill = {
      sourceId: "built-in", relativePath: "reviewer", name: "reviewer", runtime: "codex",
      scope: "user", projectPath: null, path: "/tmp/reviewer/SKILL.md", state: "current", tracked: true,
    };
    install.installed = [agentRow];
    install.reconciled = true;
    skillSources.installed = [skillRow];
    skillSources.reconciled = true;
    const invokeMock = vi.mocked(invoke);
    let failScans = false;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "installs_reconcile") {
        if (failScans) throw { code: "io", message: "agent focus scan failed" };
        return [agentRow] as never;
      }
      if (command === "skill_installs_reconcile") {
        if (failScans) throw { code: "io", message: "skill focus scan failed" };
        return [skillRow] as never;
      }
      if (command === "skill_backups_list") return [] as never;
      return [] as never;
    });
    const projectRefresh = vi.spyOn(projects, "refresh").mockResolvedValue();
    const { default: Layout } = await import("../routes/+layout.svelte");
    const target = document.createElement("div");
    const children = createRawSnippet(() => ({ render: () => "<main>App</main>" }));
    document.body.append(target);
    const component = mount(Layout, { target, props: { children } });
    try {
      await tick();
      await vi.waitFor(() => expect(skillSources.reconciling).toBe(false));
      failScans = true;
      window.dispatchEvent(new Event("focus"));
      await vi.advanceTimersByTimeAsync(250);
      await vi.waitFor(() => expect(skillSources.reconciling).toBe(false));

      expect(install.installed).toEqual([agentRow]);
      expect(install.reconcileError).toBe("I/O error: agent focus scan failed");
      expect(skillSources.installed).toEqual([skillRow]);
      expect(skillSources.reconcileError).toBe("I/O error: skill focus scan failed");

      failScans = false;
      await Promise.all([install.reconcile(), skillSources.reconcileInstalls([])]);
      expect(install.reconcileError).toBeNull();
      expect(skillSources.reconcileError).toBeNull();
      expect(install.installed).toEqual([agentRow]);
      expect(skillSources.installed).toEqual([skillRow]);
    } finally {
      unmount(component);
      target.remove();
      projectRefresh.mockRestore();
      vi.useRealTimers();
    }
  });

  it("keeps native drift notifications opt-in and persists only granted permission", async () => {
    settings.data = { ...SETTINGS_DEFAULTS, driftNotifications: false };
    notificationMocks.isPermissionGranted.mockResolvedValue(false);
    notificationMocks.requestPermission.mockResolvedValue("denied");
    const save = vi.spyOn(settings, "save").mockResolvedValue();
    const { default: SettingsSectionNetwork } = await import("$lib/components/SettingsSectionNetwork.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(SettingsSectionNetwork, { target });
    try {
      const toggle = target.querySelector<HTMLInputElement>('[data-drift-notifications]')!;
      expect(toggle.checked).toBe(false);
      toggle.click();
      await vi.waitFor(() => expect(notificationMocks.requestPermission).toHaveBeenCalledOnce());
      expect(save).not.toHaveBeenCalledWith({ driftNotifications: true });
      expect(target.querySelector('[data-drift-notification-error]')?.textContent).toContain("permission");

      notificationMocks.requestPermission.mockResolvedValue("granted");
      toggle.click();
      await vi.waitFor(() => expect(save).toHaveBeenCalledWith({ driftNotifications: true }));
    } finally {
      unmount(component);
      target.remove();
      save.mockRestore();
    }
  });

  it("notifies once for newly actionable background drift and routes activation without mutation", async () => {
    vi.useFakeTimers();
    vi.stubGlobal("matchMedia", vi.fn(() => ({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })));
    const visibility = vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
    settings.data = { ...SETTINGS_DEFAULTS, driftNotifications: true };
    const loadSettings = vi.spyOn(settings, "load").mockResolvedValue();
    const projectRefresh = vi.spyOn(projects, "refresh").mockResolvedValue();
    const currentAgent: InstalledAgent = {
      ...staleControlRow, sourceId: "built-in", relativePath: "reviewer.md",
      state: "current", tracked: true,
    };
    const driftedAgent = { ...currentAgent, state: "modified" as const };
    const currentSkill: InstalledSkill = {
      sourceId: "built-in", relativePath: "audit", name: "Audit", runtime: "codex",
      scope: "user", projectPath: null, path: "/private/project/.codex/skills/audit/SKILL.md",
      state: "current", tracked: true,
    };
    let agentRows = [currentAgent];
    let skillRows = [currentSkill];
    let failAgentScan = false;
    const agentReconcile = vi.spyOn(install, "reconcile").mockImplementation(async () => {
      if (failAgentScan) {
        install.reconcileError = "agent background scan failed";
        return;
      }
      install.installed = agentRows;
      install.reconciled = true;
      install.reconcileError = null;
    });
    const skillReconcile = vi.spyOn(skillSources, "reconcileInstalls").mockImplementation(async () => {
      skillSources.installed = skillRows;
      skillSources.reconciled = true;
      skillSources.reconcileError = null;
    });
    const openAgents = vi.spyOn(ui, "openAgents");
    const setSection = vi.spyOn(ui, "setSection");
    const { default: Layout } = await import("../routes/+layout.svelte");
    const target = document.createElement("div");
    const children = createRawSnippet(() => ({ render: () => "<main>App</main>" }));
    document.body.append(target);
    const component = mount(Layout, { target, props: { children } });
    try {
      await tick();
      await Promise.resolve();
      expect(notificationMocks.sendNotification).not.toHaveBeenCalled();

      agentRows = [driftedAgent];
      failAgentScan = true;
      await vi.advanceTimersByTimeAsync(15 * 60 * 1000);
      expect(notificationMocks.sendNotification).not.toHaveBeenCalled();

      failAgentScan = false;
      await vi.advanceTimersByTimeAsync(15 * 60 * 1000);
      await Promise.resolve();
      expect(notificationMocks.sendNotification).toHaveBeenCalledOnce();
      expect(notificationMocks.sendNotification.mock.calls[0]?.[0]).toMatchObject({
        title: "Agent drift needs review",
        extra: { review: "agents" },
      });
      expect(JSON.stringify(notificationMocks.sendNotification.mock.calls[0]?.[0])).not.toContain("/private/project");

      await vi.advanceTimersByTimeAsync(15 * 60 * 1000);
      expect(notificationMocks.sendNotification).toHaveBeenCalledOnce();

      agentRows = [currentAgent];
      await vi.advanceTimersByTimeAsync(15 * 60 * 1000);
      agentRows = [driftedAgent];
      await vi.advanceTimersByTimeAsync(15 * 60 * 1000);
      expect(notificationMocks.sendNotification).toHaveBeenCalledTimes(2);

      agentRows = [currentAgent];
      skillRows = [{ ...currentSkill, state: "missing" }];
      await vi.advanceTimersByTimeAsync(15 * 60 * 1000);
      expect(notificationMocks.sendNotification).toHaveBeenCalledTimes(3);
      expect(notificationMocks.sendNotification.mock.calls[2]?.[0]).toMatchObject({
        title: "Skill drift needs review",
        extra: { review: "skills" },
      });

      notificationMocks.action?.({ extra: { review: "agents" } });
      expect(openAgents).toHaveBeenCalledWith(null, "attention");

      notificationMocks.action?.({ extra: { review: "skills" } });
      expect(setSection).toHaveBeenCalledWith("skills");
    } finally {
      unmount(component);
      expect(notificationMocks.unlisten).toHaveBeenCalledOnce();
      target.remove();
      visibility.mockRestore();
      loadSettings.mockRestore();
      projectRefresh.mockRestore();
      agentReconcile.mockRestore();
      skillReconcile.mockRestore();
      openAgents.mockRestore();
      setSection.mockRestore();
      vi.useRealTimers();
    }
  });

  it("skips background drift scans while the app is visible", async () => {
    expect(SETTINGS_DEFAULTS.driftNotifications).toBe(false);
    vi.useFakeTimers();
    vi.stubGlobal("matchMedia", vi.fn(() => ({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })));
    const visibility = vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
    settings.data = { ...SETTINGS_DEFAULTS, driftNotifications: true };
    const loadSettings = vi.spyOn(settings, "load").mockResolvedValue();
    const projectRefresh = vi.spyOn(projects, "refresh").mockResolvedValue();
    const agentReconcile = vi.spyOn(install, "reconcile").mockResolvedValue();
    const skillReconcile = vi.spyOn(skillSources, "reconcileInstalls").mockResolvedValue();
    const { default: Layout } = await import("../routes/+layout.svelte");
    const target = document.createElement("div");
    const children = createRawSnippet(() => ({ render: () => "<main>App</main>" }));
    document.body.append(target);
    const component = mount(Layout, { target, props: { children } });
    try {
      await tick();
      await Promise.resolve();
      agentReconcile.mockClear();
      skillReconcile.mockClear();
      await vi.advanceTimersByTimeAsync(15 * 60 * 1000);
      expect(agentReconcile).not.toHaveBeenCalled();
      expect(skillReconcile).not.toHaveBeenCalled();
      expect(notificationMocks.sendNotification).not.toHaveBeenCalled();
    } finally {
      unmount(component);
      target.remove();
      visibility.mockRestore();
      loadSettings.mockRestore();
      projectRefresh.mockRestore();
      agentReconcile.mockRestore();
      skillReconcile.mockRestore();
      vi.useRealTimers();
    }
  });

  it("keeps Skills popovers open for inside clicks and closes them for outside clicks", async () => {
    vi.stubGlobal("localStorage", {
      getItem: () => null,
      removeItem: () => undefined,
    });
    const { default: SkillsWorkspace } = await import("$lib/components/SkillsWorkspace.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(SkillsWorkspace, { target });
    await tick();
    try {
      const sourceManager = target.querySelector<HTMLDetailsElement>("details.source-manager")!;
      const approvalInbox = target.querySelector<HTMLDetailsElement>("details.draft-inbox")!;
      sourceManager.open = true;
      sourceManager.querySelector("input")!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      expect(sourceManager.open).toBe(true);
      document.body.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      expect(sourceManager.open).toBe(false);

      approvalInbox.open = true;
      document.body.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      expect(approvalInbox.open).toBe(false);
    } finally {
      unmount(component);
      target.remove();
      vi.unstubAllGlobals();
    }
  });

  it("explains the one-time storage update and exposes named stages", async () => {
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(StorageMigrationGate, {
      target,
      props: {
        status: { state: "legacy", stage: "checkingData", detail: null, legacyConflicts: [] },
        busy: false,
        error: null,
        onStart: vi.fn(),
        onRetry: vi.fn(),
        onOpenData: vi.fn(),
      },
    });
    await tick();
    expect(target.textContent).toContain("Agency Agents needs a one-time data update");
    expect(target.textContent).toContain("Checking data");
    expect(target.textContent).toContain("Verifying backup");
    expect(target.textContent).toContain("Moving records");
    expect(target.querySelector("[aria-busy='false']")).not.toBeNull();
    expect(document.activeElement).toBe(target.querySelector("button.btn-primary"));
    unmount(component);
    target.remove();
  });

  it("keeps migration failures reassuring and retryable but blocks unsupported data", async () => {
    const target = document.createElement("div");
    document.body.append(target);
    const retry = vi.fn();
    const component = mount(StorageMigrationGate, {
      target,
      props: {
        status: { state: "corrupt", stage: "failed", detail: "Invalid settings", legacyConflicts: [] },
        busy: false,
        error: "Invalid settings",
        onStart: vi.fn(),
        onRetry: retry,
        onOpenData: vi.fn(),
      },
    });
    await tick();
    expect(target.textContent).toContain("Nothing was lost");
    target.querySelector<HTMLButtonElement>("button.btn-primary")!.click();
    expect(retry).toHaveBeenCalledOnce();
    unmount(component);

    const unsupported = mount(StorageMigrationGate, {
      target,
      props: {
        status: { state: "unsupported", stage: "unsupported", detail: null, legacyConflicts: [] },
        busy: false,
        error: null,
        onStart: vi.fn(),
        onRetry: vi.fn(),
        onOpenData: vi.fn(),
      },
    });
    await tick();
    expect(target.textContent).toContain("newer Agency Agents version");
    expect(target.querySelector("button.btn-primary")).toBeNull();
    unmount(unsupported);
    target.remove();
  });

  it("shows one inbox card when a pending draft already has an exact approval request", async () => {
    const draft = {
      id: "draft-1",
      submittedAt: "2026-08-05T13:00:00Z",
      state: "pending",
      treeHash: "a".repeat(64),
      files: [{ relativePath: "SKILL.md", sizeBytes: 10, sha256: "b".repeat(64) }],
      validation: {
        sourceId: "draft",
        relativePath: "reviewer",
        name: "reviewer",
        description: "Reviews code",
        skillType: "ai",
        group: [],
        tags: ["review"],
        dependencies: [],
        recommendedSkills: [],
        version: null,
        channel: "stable",
        changelog: null,
        publisher: null,
        publisherKey: null,
        publisherVerified: false,
        validationResults: [],
        permissions: [],
        qualityScore: 100,
        qualityChecks: [],
        files: [{ relativePath: "SKILL.md", sizeBytes: 10, sha256: "b".repeat(64) }],
        trustFingerprint: null,
        errors: [],
        installable: true,
      },
      publishedSourceId: null,
    };
    const folders = {
      folders: [], assignments: [], favorites: [], recent: [], collections: [], smartFolders: [],
      profiles: [], updatePolicies: [], publisherTrust: [], preferredSources: [], usage: [],
      approvals: [{
        id: "approval-1",
        submittedAt: "2026-08-05T13:01:00Z",
        state: "pending",
        requestedBy: "codex",
        request: { action: "draftPublish", id: draft.id, planRevision: draft.treeHash },
        result: null,
      }],
    };
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "skill_drafts_list") return [draft] as never;
      if (command === "skill_folders_list") return folders as never;
      return [] as never;
    });
    vi.stubGlobal("localStorage", {
      getItem: () => null,
      removeItem: () => undefined,
    });
    const { default: SkillsWorkspace } = await import("$lib/components/SkillsWorkspace.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(SkillsWorkspace, { target });
    try {
      await vi.waitFor(() => {
        expect(target.querySelector("details.draft-inbox summary")?.textContent).toContain("1");
      });
      expect(target.querySelectorAll("details.draft-inbox article.draft")).toHaveLength(1);
      expect(target.querySelector("details.draft-inbox article.draft strong")?.textContent)
        .toBe("Publish Skill draft draft-1");
    } finally {
      unmount(component);
      target.remove();
      vi.unstubAllGlobals();
    }
  });

  it("refreshes Skills when the inbox opens without closing it, stealing focus, or resetting scroll", async () => {
    let folderLoads = 0;
    const folders = {
      folders: [], assignments: [], favorites: [], recent: [], collections: [], smartFolders: [],
      profiles: [], updatePolicies: [], publisherTrust: [], preferredSources: [], usage: [], approvals: [],
    };
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "skill_folders_list") {
        folderLoads += 1;
        return folders as never;
      }
      return [] as never;
    });
    vi.stubGlobal("localStorage", {
      getItem: () => null,
      removeItem: () => undefined,
    });
    const { default: SkillsWorkspace } = await import("$lib/components/SkillsWorkspace.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(SkillsWorkspace, { target });
    try {
      await vi.waitFor(() => {
        expect(folderLoads).toBeGreaterThan(0);
        expect(target.querySelector("details.draft-inbox summary")?.textContent).toContain("0");
      });
      const inbox = target.querySelector<HTMLDetailsElement>("details.draft-inbox")!;
      const popover = inbox.querySelector<HTMLElement>(".draft-popover")!;
      const summary = inbox.querySelector<HTMLElement>("summary")!;
      inbox.open = true;
      popover.scrollTop = 37;
      summary.focus();
      folders.approvals = [{
        id: "skill-approval-live",
        submittedAt: "2026-08-06T03:35:47Z",
        state: "pending",
        requestedBy: "codex",
        request: { action: "draftPublish", id: "skill-draft-live", planRevision: "revision-live" },
        result: null,
      }] as never;
      const loadsBeforeOpen = folderLoads;
      inbox.dispatchEvent(new Event("toggle"));

      await vi.waitFor(() => {
        expect(folderLoads).toBeGreaterThan(loadsBeforeOpen);
        expect(summary.textContent).toContain("1");
      });
      expect(inbox.open).toBe(true);
      expect(document.activeElement).toBe(summary);
      expect(popover.scrollTop).toBe(37);
      expect(target.textContent).toContain("skill-draft-live");
    } finally {
      unmount(component);
      target.remove();
      vi.unstubAllGlobals();
    }
  });

  it("refreshes persisted Agent approvals when the inbox opens", async () => {
    const library = {
      folders: [], assignments: [], favorites: [], recent: [], collections: [], smartFolders: [],
      profiles: [], updatePolicies: [], publisherTrust: [], preferredSources: [], usage: [],
      approvals: [{
        id: "agent-approval-1",
        submittedAt: "2026-08-05T15:54:33Z",
        state: "pending",
        requestedBy: "codex",
        request: { action: "draftPublish", id: "agent-draft-1", planRevision: "revision-1" },
        result: null,
      }],
    };
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "agent_library_list") return library as never;
      return [] as never;
    });
    const { default: AgentApprovalInbox } = await import("$lib/components/AgentApprovalInbox.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(AgentApprovalInbox, {
      target,
      props: { open: true, focusId: "agent-approval-1", onClose: vi.fn() },
    });
    try {
      await vi.waitFor(() => {
        expect(target.textContent).toContain("agent-draft-1");
        expect((document.activeElement as HTMLElement).closest("[data-agent-approval-id]")?.getAttribute("data-agent-approval-id"))
          .toBe("agent-approval-1");
      });
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("retains Agent install truth and records semantic reconcile failures", async () => {
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockClear();
    const row: InstalledAgent = {
      slug: "reviewer", name: "Reviewer", sourceId: "built-in", relativePath: "reviewer.md",
      tool: "claudeCode", scope: "user", projectPath: null, dest: "/tmp/reviewer.md",
      state: "current", updateKind: null, tracked: true,
    };
    install.installed = [];
    install.reconciled = false;
    install.reconciling = false;
    install.reconcileError = null;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "installs_reconcile") return [row] as never;
      return [] as never;
    });

    await install.reconcile();
    expect(install.installed).toEqual([row]);
    expect(install.reconciled).toBe(true);
    expect(install.reconcileError).toBeNull();

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "installs_reconcile") throw { code: "io", message: "scan failed" };
      return [] as never;
    });
    await install.reconcile();

    expect(install.installed).toEqual([row]);
    expect(install.reconciled).toBe(true);
    expect(install.reconcileError).toBe("I/O error: scan failed");
  });

  it("keeps Agent reconcile failure visible while a single-flight retry recovers", async () => {
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockClear();
    let resolveScan!: (rows: InstalledAgent[]) => void;
    const pendingScan = new Promise<InstalledAgent[]>((resolve) => (resolveScan = resolve));
    install.installed = [];
    install.reconciled = false;
    install.reconciling = false;
    install.reconcileError = "I/O error: previous failure";
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "installs_reconcile") return pendingScan as never;
      return [] as never;
    });

    const first = install.reconcile();
    const second = install.reconcile();
    expect(install.reconciling).toBe(true);
    expect(install.reconciled).toBe(false);
    expect(install.reconcileError).toBe("I/O error: previous failure");
    expect(invokeMock.mock.calls.filter(([command]) => command === "installs_reconcile")).toHaveLength(1);

    resolveScan([]);
    await Promise.all([first, second]);
    expect(install.installed).toEqual([]);
    expect(install.reconciled).toBe(true);
    expect(install.reconcileError).toBeNull();
    expect(install.reconciling).toBe(false);
    expect(invokeMock.mock.calls.map(([command]) => command)).toEqual(["installs_reconcile"]);
  });

  it("keeps Skill reconcile state separate from source-management errors", async () => {
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockClear();
    const row: InstalledSkill = {
      sourceId: "built-in", relativePath: "reviewer", name: "reviewer", runtime: "claudeCode",
      scope: "user", projectPath: null, path: "/tmp/reviewer/SKILL.md", state: "current", tracked: true,
    };
    skillSources.installed = [];
    skillSources.backups = [];
    skillSources.reconciled = false;
    skillSources.reconciling = false;
    skillSources.reconcileError = null;
    skillSources.addError = "Source refresh failed";
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "skill_installs_reconcile") return [row] as never;
      if (command === "skill_backups_list") return ["/tmp/backup"] as never;
      return [] as never;
    });

    await skillSources.reconcileInstalls([]);
    expect(skillSources.installed).toEqual([row]);
    expect(skillSources.reconciled).toBe(true);
    expect(skillSources.reconcileError).toBeNull();
    expect(skillSources.addError).toBe("Source refresh failed");

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "skill_installs_reconcile") throw { code: "storage_busy" };
      return [] as never;
    });
    await skillSources.reconcileInstalls([]);

    expect(skillSources.installed).toEqual([row]);
    expect(skillSources.reconciled).toBe(true);
    expect(skillSources.reconcileError).toBe("Agency Agents is busy in another desktop or MCP session. Close it and try again.");
    expect(skillSources.addError).toBe("Source refresh failed");

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "skill_installs_reconcile") return [] as never;
      if (command === "skill_backups_list") throw { code: "io", message: "backup scan failed" };
      return [] as never;
    });
    await skillSources.reconcileInstalls([]);
    expect(skillSources.installed).toEqual([row]);
    expect(skillSources.reconciled).toBe(true);
    expect(skillSources.reconcileError).toBe("I/O error: backup scan failed");
    expect(skillSources.addError).toBe("Source refresh failed");
  });

  it("coalesces Skill retries and clears the prior warning only after success", async () => {
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockClear();
    let resolveScan!: (rows: InstalledSkill[]) => void;
    const pendingScan = new Promise<InstalledSkill[]>((resolve) => (resolveScan = resolve));
    skillSources.installed = [];
    skillSources.reconciled = false;
    skillSources.reconciling = false;
    skillSources.reconcileError = "I/O error: previous failure";
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "skill_installs_reconcile") return pendingScan as never;
      if (command === "skill_backups_list") return [] as never;
      return [] as never;
    });

    const first = skillSources.reconcileInstalls(["/tmp/project"]);
    const second = skillSources.reconcileInstalls(["/tmp/project"]);
    expect(skillSources.reconciling).toBe(true);
    expect(skillSources.reconciled).toBe(false);
    expect(skillSources.reconcileError).toBe("I/O error: previous failure");
    expect(invokeMock.mock.calls.filter(([command]) => command === "skill_installs_reconcile")).toHaveLength(1);

    resolveScan([]);
    await Promise.all([first, second]);
    expect(skillSources.installed).toEqual([]);
    expect(skillSources.reconciled).toBe(true);
    expect(skillSources.reconcileError).toBeNull();
    expect(skillSources.reconciling).toBe(false);
    expect(invokeMock.mock.calls.map(([command]) => command)).toEqual(["skill_installs_reconcile", "skill_backups_list"]);
  });

  it("queues a different canonical Skill project scope before reporting reconciliation complete", async () => {
    const invokeMock = vi.mocked(invoke);
    let resolveFirst!: (rows: InstalledSkill[]) => void;
    let resolveSecond!: (rows: InstalledSkill[]) => void;
    const firstScan = new Promise<InstalledSkill[]>((resolve) => (resolveFirst = resolve));
    const secondScan = new Promise<InstalledSkill[]>((resolve) => (resolveSecond = resolve));
    let scans = 0;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "skill_installs_reconcile") return (++scans === 1 ? firstScan : secondScan) as never;
      if (command === "skill_backups_list") return [] as never;
      return [] as never;
    });

    const first = skillSources.reconcileInstalls(["/tmp/b", "/tmp/a", "/tmp/a"]);
    const second = skillSources.reconcileInstalls(["/tmp/c"]);
    expect(scans).toBe(1);
    expect(skillSources.reconciling).toBe(true);
    expect(skillSources.reconciled).toBe(false);

    resolveFirst([]);
    await vi.waitFor(() => expect(scans).toBe(2));
    expect(skillSources.reconciling).toBe(true);
    expect(skillSources.reconciled).toBe(false);

    resolveSecond([]);
    await Promise.all([first, second]);
    expect(skillSources.reconciling).toBe(false);
    expect(skillSources.reconciled).toBe(true);
    expect(invokeMock.mock.calls
      .filter(([command]) => command === "skill_installs_reconcile")
      .map(([, args]) => (args as { projectPaths: string[] }).projectPaths))
      .toEqual([["/tmp/a", "/tmp/b"], ["/tmp/c"]]);
  });

  it("drains A→B→A Skill scopes to the latest A before any caller settles", async () => {
    const invokeMock = vi.mocked(invoke);
    const aRow: InstalledSkill = {
      sourceId: "built-in", relativePath: "a", name: "a", runtime: "claudeCode",
      scope: "project", projectPath: "/tmp/a", path: "/tmp/a/SKILL.md", state: "current", tracked: true,
    };
    let resolveFirstA!: (rows: InstalledSkill[]) => void;
    let resolveLatestA!: (rows: InstalledSkill[]) => void;
    const firstA = new Promise<InstalledSkill[]>((resolve) => (resolveFirstA = resolve));
    const latestA = new Promise<InstalledSkill[]>((resolve) => (resolveLatestA = resolve));
    const scans: string[][] = [];
    invokeMock.mockImplementation(async (command: string, args?: unknown) => {
      if (command === "skill_installs_reconcile") {
        scans.push((args as { projectPaths: string[] }).projectPaths);
        return (scans.length === 1 ? firstA : latestA) as never;
      }
      if (command === "skill_backups_list") return [] as never;
      return [] as never;
    });

    const settled: string[] = [];
    const first = skillSources.reconcileInstalls(["/tmp/a"]).then(() => settled.push("first-a"));
    const middle = skillSources.reconcileInstalls(["/tmp/b"]).then(() => settled.push("b"));
    const last = skillSources.reconcileInstalls(["/tmp/a"]).then(() => settled.push("latest-a"));

    resolveFirstA([{ ...aRow, name: "obsolete-a" }]);
    await vi.waitFor(() => expect(scans).toHaveLength(2));
    expect(scans).toEqual([["/tmp/a"], ["/tmp/a"]]);
    expect(settled).toEqual([]);
    expect(skillSources.installed).toEqual([]);

    resolveLatestA([aRow]);
    await Promise.all([first, middle, last]);
    expect(scans.at(-1)).toEqual(["/tmp/a"]);
    expect(skillSources.installed).toEqual([aRow]);
    expect(settled).toHaveLength(3);
  });

  it("does not replace Skill rows when backup reconciliation fails", async () => {
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockClear();
    const row: InstalledSkill = {
      sourceId: "built-in", relativePath: "reviewer", name: "reviewer", runtime: "claudeCode",
      scope: "user", projectPath: null, path: "/tmp/reviewer/SKILL.md", state: "current", tracked: true,
    };
    skillSources.installed = [row];
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "skill_installs_reconcile") return [] as never;
      if (command === "skill_backups_list") throw { code: "io", message: "backup scan failed" };
      return [] as never;
    });

    await skillSources.reconcileInstalls([]);

    expect(skillSources.installed).toEqual([row]);
  });

  it("renders the Agent reconcile warning through one live region and recovers focus", async () => {
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockClear();
    const semanticFailure = `I/O error: ${"status-code-".repeat(8)} /Users/developer/${"deep-path/".repeat(12)}SKILL.md`;
    install.installed = [];
    install.reconciled = false;
    install.reconciling = false;
    install.reconcileError = semanticFailure;
    corpus.agents = [{
      slug: "reviewer", name: "Reviewer", description: "Reviews code", category: "engineering",
      emoji: null, color: null, vibe: null, body: "Review carefully.",
    }];
    corpus.categories = [];
    corpus.loading = false;
    corpus.error = null;
    let rejectScan!: (error: unknown) => void;
    const pendingScan = new Promise<InstalledAgent[]>((_resolve, reject) => (rejectScan = reject));
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "installs_reconcile") return pendingScan as never;
      if (command === "agent_library_list") return {
        folders: [], assignments: [], favorites: [], recent: [], collections: [], smartFolders: [],
        profiles: [], updatePolicies: [], publisherTrust: [], preferredSources: [], usage: [], approvals: [],
      } as never;
      return [] as never;
    });
    const { default: AgentsWorkspace } = await import("$lib/components/AgentsWorkspace.svelte");
    const target = document.createElement("div");
    target.style.width = "280px";
    document.body.append(target);
    const component = mount(AgentsWorkspace, { target });
    try {
      await tick();
      const warning = target.querySelector(".reconcile-warning")!;
      expect(warning.textContent).toContain("Installation status may be out of date");
      expect(warning.textContent).toContain(semanticFailure);
      expect(warning.textContent).toContain("Installation status is unavailable until a retry succeeds.");
      expect(target.querySelectorAll(".reconcile-warning")).toHaveLength(1);
      expect(target.querySelectorAll('[role="status"][aria-live="polite"]')).toHaveLength(1);
      expect(target.querySelector('[role="alert"]')).toBeNull();
      expect(toast.items).toHaveLength(0);

      const retry = [...target.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.trim() === "Retry status check")!;
      retry.focus();
      const invokesBeforeRetry = invokeMock.mock.calls.length;
      retry.click();
      retry.click();
      await tick();
      expect(retry.textContent).toContain("Retrying…");
      expect(retry.disabled).toBe(true);
      expect(target.querySelector(".reconcile-warning")?.textContent).toContain(semanticFailure);
      expect(invokeMock.mock.calls.filter(([command]) => command === "installs_reconcile")).toHaveLength(1);
      expect(invokeMock.mock.calls.slice(invokesBeforeRetry).map(([command]) => command))
        .toEqual(["installs_reconcile"]);

      rejectScan({ code: "io", message: "retry failed" });
      await vi.waitFor(() => {
        expect(retry.disabled).toBe(false);
        expect(target.querySelector(".reconcile-warning")?.textContent).toContain("I/O error: retry failed");
      });
      expect(document.activeElement).toBe(retry);

      invokeMock.mockImplementation(async (command: string) => {
        if (command === "installs_reconcile") return [] as never;
        return [] as never;
      });
      retry.click();
      await vi.waitFor(() => expect(target.querySelector(".reconcile-warning")).toBeNull());
      expect(document.activeElement).toBe(target.querySelector('[data-install-rescan]'));
    } finally {
      unmount(component);
      target.remove();
      toast.items = [];
    }
  });

  it("announces the terminal result when direct Agent rescans fail twice with the same payload", async () => {
    const invokeMock = vi.mocked(invoke);
    const semanticFailure = "I/O error: repeated scan failure";
    install.reconciled = true;
    install.reconcileError = semanticFailure;
    corpus.agents = [{
      slug: "reviewer", name: "Reviewer", description: "Reviews code", category: "engineering",
      emoji: null, color: null, vibe: null, body: "Review carefully.",
    }];
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "installs_reconcile") throw { code: "io", message: "repeated scan failure" };
      if (command === "agent_library_list") return emptyFolderState() as never;
      return [] as never;
    });
    const { default: AgentsWorkspace } = await import("$lib/components/AgentsWorkspace.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(AgentsWorkspace, { target });
    try {
      await tick();
      const rescan = target.querySelector<HTMLButtonElement>("[data-install-rescan]")!;
      const status = target.querySelector<HTMLElement>('[role="status"]')!;

      rescan.click();
      await vi.waitFor(() => expect(install.reconciling).toBe(false));
      expect(status.textContent).toContain(`Installation status is still out of date. ${semanticFailure}`);

      rescan.click();
      await vi.waitFor(() => expect(invokeMock.mock.calls
        .filter(([command]) => command === "installs_reconcile")).toHaveLength(2));
      await vi.waitFor(() => expect(install.reconciling).toBe(false));
      expect(status.textContent).toContain(`Installation status is still out of date. ${semanticFailure}`);
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("renders the Skill reconcile warning without a third live region or verified zero claim", async () => {
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockClear();
    const semanticFailure = `Storage error: ${"semantic-code-".repeat(8)} /Users/developer/${"nested/".repeat(12)}SKILL.md`;
    skillSources.sources = [];
    skillSources.results = {};
    skillSources.installed = [];
    skillSources.reconciled = false;
    skillSources.reconciling = false;
    skillSources.reconcileError = null;
    skillSources.addError = null;
    let resolveScan!: (rows: InstalledSkill[]) => void;
    const pendingScan = new Promise<InstalledSkill[]>((resolve) => (resolveScan = resolve));
    let scans = 0;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "skill_installs_reconcile") {
        if (++scans === 1) return [] as never;
        return pendingScan as never;
      }
      if (command === "skill_folders_list") return {
        folders: [], assignments: [], favorites: [], recent: [], collections: [], smartFolders: [],
        profiles: [], updatePolicies: [], publisherTrust: [], preferredSources: [], usage: [], approvals: [],
      } as never;
      if (command === "agent_library_list") return {
        folders: [], assignments: [], favorites: [], recent: [], collections: [], smartFolders: [],
        profiles: [], updatePolicies: [], publisherTrust: [], preferredSources: [], usage: [], approvals: [],
      } as never;
      return [] as never;
    });
    vi.stubGlobal("localStorage", { getItem: () => null, setItem: () => undefined, removeItem: () => undefined });
    const { default: SkillsWorkspace } = await import("$lib/components/SkillsWorkspace.svelte");
    const target = document.createElement("div");
    target.style.width = "280px";
    document.body.append(target);
    const component = mount(SkillsWorkspace, { target });
    try {
      await vi.waitFor(() => {
        expect(scans).toBe(1);
        expect(skillSources.reconciling).toBe(false);
      });
      skillSources.reconciled = false;
      skillSources.reconcileError = semanticFailure;
      await vi.waitFor(() => expect(target.querySelector(".reconcile-warning")).not.toBeNull());
      const warning = target.querySelector(".reconcile-warning")!;
      expect(warning.textContent).toContain("Installation status may be out of date");
      expect(warning.textContent).toContain(semanticFailure);
      expect(warning.textContent).toContain("Installation status is unavailable until a retry succeeds.");
      expect(target.querySelectorAll(".reconcile-warning")).toHaveLength(1);
      expect(target.querySelectorAll('[role="status"][aria-live="polite"]')).toHaveLength(2);
      expect(target.querySelector('[role="alert"]')).toBeNull();
      expect(warning.textContent).toContain("Retry status check");
      expect(target.textContent).not.toContain("Installed 0");

      skillSources.installed = [{
        sourceId: "built-in", relativePath: "reviewer", name: "reviewer", runtime: "claudeCode",
        scope: "user", projectPath: null, path: "/tmp/reviewer/SKILL.md", state: "current", tracked: true,
      }];
      skillSources.reconciled = true;
      await tick();
      expect(target.querySelector(".reconcile-warning")?.textContent)
        .toContain("Your last known installation data is still shown.");

      const retry = [...target.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.trim() === "Retry status check")!;
      retry.focus();
      const invokesBeforeRetry = invokeMock.mock.calls.length;
      retry.click();
      retry.click();
      await tick();
      expect(retry.textContent).toContain("Retrying…");
      expect(retry.disabled).toBe(true);
      expect(invokeMock.mock.calls.slice(invokesBeforeRetry)
        .filter(([command]) => command === "skill_installs_reconcile")).toHaveLength(1);

      resolveScan([]);
      await vi.waitFor(() => expect(target.querySelector(".reconcile-warning")).toBeNull());
      expect(invokeMock.mock.calls.slice(invokesBeforeRetry).map(([command]) => command))
        .toEqual(["skill_installs_reconcile", "skill_backups_list"]);
      expect(document.activeElement).toBe(target.querySelector("h2"));
    } finally {
      unmount(component);
      target.remove();
      vi.unstubAllGlobals();
    }
  });

  it("announces repeated identical Skill retry failures as distinct terminal results", async () => {
    skillSources.reconciled = true;
    skillSources.reconcileError = "I/O error: repeated skill scan failure";
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "skill_installs_reconcile") throw { code: "io", message: "repeated skill scan failure" };
      if (command === "skill_folders_list") return emptyFolderState() as never;
      return [] as never;
    });
    const { default: SkillsWorkspace } = await import("$lib/components/SkillsWorkspace.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(SkillsWorkspace, { target });
    try {
      await vi.waitFor(() => expect(skillSources.reconcileTerminal).toBeGreaterThan(0));
      const retry = [...target.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.trim() === "Retry status check")!;
      const status = target.querySelector<HTMLElement>('.announcement[role="status"]')!;
      const firstTerminal = skillSources.reconcileTerminal;
      retry.click();
      await vi.waitFor(() => expect(skillSources.reconcileTerminal).toBe(firstTerminal + 1));
      expect(status.textContent).toContain("Installation status is still out of date. I/O error: repeated skill scan failure");
      retry.click();
      await vi.waitFor(() => expect(skillSources.reconcileTerminal).toBe(firstTerminal + 2));
      expect(status.textContent).toContain("Installation status is still out of date. I/O error: repeated skill scan failure");
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("keeps a populated Skill catalog truthful when Installed is selected before reconciliation succeeds", async () => {
    const source = { id: "local", kind: { kind: "local", root: "/tmp/skills" } } as const;
    const pkg = {
      sourceId: "local", relativePath: "reviewer", name: "reviewer", description: "Reviews code",
      skillType: "ai", group: [], tags: ["review"], dependencies: [], recommendedSkills: [],
      version: null, channel: "stable", changelog: null, publisher: null, publisherKey: null,
      publisherVerified: false, validationResults: [], permissions: [], qualityScore: 100,
      qualityChecks: [], files: [{ relativePath: "SKILL.md", sizeBytes: 10, sha256: "a".repeat(64) }],
      trustFingerprint: null, errors: [], installable: true,
    } as const;
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "skill_sources_inspect") return [{ source, packages: [pkg], errors: [] }] as never;
      if (command === "skill_drafts_list") return [] as never;
      if (command === "skill_folders_list") return emptyFolderState() as never;
      if (command === "skill_installs_reconcile") throw { code: "io", message: "scan failed" };
      return [] as never;
    });
    const { default: SkillsWorkspace } = await import("$lib/components/SkillsWorkspace.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(SkillsWorkspace, { target });
    try {
      await vi.waitFor(() => expect(target.textContent).toContain("reviewer"));
      [...target.querySelectorAll<HTMLButtonElement>(".quick-filters button")]
        .find((button) => button.textContent?.includes("Installed"))!.click();
      await tick();

      expect(target.querySelector(".package-list")?.textContent)
        .toContain("Installation status is unavailable until a retry succeeds.");
      expect(target.querySelector(".package-list")?.textContent).not.toContain("0 results");
      expect(target.querySelector(".package-list")?.textContent).not.toContain("No matches");
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("retains the selected Installed Skill row and count after a failed retry", async () => {
    const source = { id: "local", kind: { kind: "local", root: "/tmp/skills" } } as const;
    const pkg = {
      sourceId: "local", relativePath: "reviewer", name: "reviewer", description: "Reviews code",
      skillType: "ai", group: [], tags: ["review"], dependencies: [], recommendedSkills: [],
      version: null, channel: "stable", changelog: null, publisher: null, publisherKey: null,
      publisherVerified: false, validationResults: [], permissions: [], qualityScore: 100,
      qualityChecks: [], files: [{ relativePath: "SKILL.md", sizeBytes: 10, sha256: "a".repeat(64) }],
      trustFingerprint: null, errors: [], installable: true,
    } as const;
    const row: InstalledSkill = {
      sourceId: "local", relativePath: "reviewer", name: "reviewer", runtime: "claudeCode",
      scope: "user", projectPath: null, path: "/tmp/reviewer/SKILL.md", state: "current", tracked: true,
    };
    let scans = 0;
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "skill_sources_inspect") return [{ source, packages: [pkg], errors: [] }] as never;
      if (command === "skill_drafts_list") return [] as never;
      if (command === "skill_folders_list" || command === "skill_recent_touch") return emptyFolderState() as never;
      if (command === "skill_installs_reconcile") {
        if (++scans === 1) return [row] as never;
        throw { code: "io", message: "retry scan failed" };
      }
      if (command === "skill_backups_list") return [] as never;
      return [] as never;
    });
    const { default: SkillsWorkspace } = await import("$lib/components/SkillsWorkspace.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(SkillsWorkspace, { target });
    try {
      await vi.waitFor(() => expect([...target.querySelectorAll<HTMLButtonElement>(".quick-filters button")]
        .find((button) => button.textContent?.includes("Installed"))?.textContent).toContain("1"));
      const installedFilter = [...target.querySelectorAll<HTMLButtonElement>(".quick-filters button")]
        .find((button) => button.textContent?.includes("Installed"))!;
      installedFilter.click();
      await tick();
      target.querySelector<HTMLButtonElement>(".package-row button")!.click();
      await tick();

      await skillSources.reconcileInstalls([]);
      await tick();

      expect(installedFilter.getAttribute("aria-pressed")).toBe("true");
      expect(installedFilter.textContent).toContain("1");
      expect(target.querySelectorAll(".package-row")).toHaveLength(1);
      expect(target.querySelector(".package-row button")?.classList.contains("selected")).toBe(true);
      expect(target.querySelector(".detail h3")?.textContent).toBe("reviewer");
      expect(target.querySelector(".reconcile-warning")?.textContent).toContain("I/O error: retry scan failed");
      expect([...target.querySelectorAll<HTMLButtonElement>(".lifecycle-actions button")]
        .every((button) => button.disabled)).toBe(true);
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it.each([
    ["cold", false, null],
    ["stale", true, "I/O error: scan failed"],
  ])("blocks Skill rollback and every collection operation while install truth is %s", async (_state, reconciled, reconcileError) => {
    const source = { id: "local", kind: { kind: "local", root: "/tmp/skills" } } as const;
    const pkg = {
      sourceId: "local", relativePath: "reviewer", name: "reviewer", description: "Reviews code",
      skillType: "ai", group: [], tags: [], dependencies: [], recommendedSkills: [], version: null,
      channel: "stable", changelog: null, publisher: null, publisherKey: null, publisherVerified: false,
      validationResults: [], permissions: [], qualityScore: 100, qualityChecks: [], files: [],
      trustFingerprint: null, errors: [], installable: true,
    } as const;
    const row: InstalledSkill = {
      sourceId: "local", relativePath: "reviewer", name: "reviewer", runtime: "claudeCode",
      scope: "user", projectPath: null, path: "/tmp/reviewer/SKILL.md", state: "current", tracked: true,
    };
    const folders = { ...emptyFolderState(), collections: [{ name: "Reviewers", skills: [{ sourceId: "local", relativePath: "reviewer" }] }] };
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "skill_sources_inspect") return [{ source, packages: [pkg], errors: [] }] as never;
      if (command === "skill_drafts_list") return [] as never;
      if (command === "skill_folders_list" || command === "skill_recent_touch") return folders as never;
      if (command === "skill_installs_reconcile") return [row] as never;
      if (command === "skill_backups_list") return [] as never;
      if (command === "skill_version_history_list") return [{ path: "/tmp/snapshot", createdAt: "2026-08-12T00:00:00Z" }] as never;
      if (command === "skill_collection_batch") return { operation: "install", completed: [], rolledBack: false } as never;
      return [] as never;
    });
    const { default: SkillsWorkspace } = await import("$lib/components/SkillsWorkspace.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(SkillsWorkspace, { target });
    try {
      await vi.waitFor(() => expect(target.querySelector(".package-row")).not.toBeNull());
      target.querySelector<HTMLButtonElement>(".package-row button")!.click();
      await tick();
      const history = target.querySelector<HTMLDetailsElement>("details.version-history")!;
      history.open = true;
      history.dispatchEvent(new Event("toggle"));
      await vi.waitFor(() => expect([...history.querySelectorAll<HTMLButtonElement>("button")]
        .some((button) => button.textContent?.includes("Rollback"))).toBe(true));

      [...target.querySelectorAll<HTMLButtonElement>(".named-views button")]
        .find((button) => button.textContent?.includes("Reviewers"))!.click();
      await tick();
      [...target.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.includes("Manage collection"))!.click();
      await tick();

      skillSources.reconciled = reconciled;
      skillSources.reconcileError = reconcileError;
      await tick();
      vi.mocked(invoke).mockClear();

      const rollback = [...history.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.includes("Rollback"))!;
      const collectionConfirm = target.querySelector<HTMLButtonElement>('[data-modal-action="confirm"]')!;
      expect(rollback.disabled).toBe(true);
      expect(collectionConfirm.disabled).toBe(true);
      rollback.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      for (const operation of ["install", "update", "uninstall"] as const) {
        const operationSelect = target.querySelector<HTMLSelectElement>(".folder-form select")!;
        operationSelect.value = operation;
        operationSelect.dispatchEvent(new Event("change", { bubbles: true }));
        collectionConfirm.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      }
      await tick();
      expect(vi.mocked(invoke).mock.calls.map(([command]) => command)).toEqual([]);
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("does not present cold Agent ledger consumers as verified empty", async () => {
    install.reconcileError = "I/O error: scan failed";
    const invokeMock = vi.mocked(invoke);
    let pendingReconcile: Promise<InstalledAgent[]> | null = null;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "installs_reconcile") return (pendingReconcile ?? Promise.reject({ code: "io", message: "scan failed" })) as never;
      if (command === "projects_list") return [{ path: "/tmp/project", label: "project", installedCount: 0 }] as never;
      return [] as never;
    });

    for (const name of ["AgencyDashboard", "ToolsView", "Teams", "Projects"] as const) {
      const { default: Component } = await import(`$lib/components/${name}.svelte`);
      const target = document.createElement("div");
      document.body.append(target);
      const component = mount(Component, { target });
      try {
        await vi.waitFor(() => expect(target.textContent)
          .toContain("Installation status is unavailable until a retry succeeds."));
        const retry = [...target.querySelectorAll<HTMLButtonElement>("button")]
          .find((button) => button.textContent?.trim() === "Retry status check")!;
        expect(retry).toBeTruthy();
        if (name === "AgencyDashboard") {
          await vi.waitFor(() => {
            expect(invokeMock.mock.calls.some(([command]) => command === "catalog_status")).toBe(true);
            expect(invokeMock.mock.calls.some(([command]) => command === "projects_list")).toBe(true);
          });
        } else if (name === "ToolsView") {
          await vi.waitFor(() => {
            expect(invokeMock.mock.calls.some(([command]) => command === "tools_list")).toBe(true);
            expect(invokeMock.mock.calls.some(([command]) => command === "tool_versions")).toBe(true);
          });
        } else if (name === "Projects") {
          await vi.waitFor(() => {
            expect(invokeMock.mock.calls.some(([command]) => command === "projects_list")).toBe(true);
            expect(invokeMock.mock.calls.some(([command]) => command === "skill_backups_list")).toBe(true);
          });
        }
        let resolveRetry!: (rows: InstalledAgent[]) => void;
        pendingReconcile = new Promise<InstalledAgent[]>((resolve) => (resolveRetry = resolve));
        const invokesBeforeRetry = invokeMock.mock.calls.length;
        retry.click();
        await vi.waitFor(() => expect(install.reconciling).toBe(true));
        expect(invokeMock.mock.calls.slice(invokesBeforeRetry).map(([command]) => command))
          .toEqual(["installs_reconcile"]);
        resolveRetry(install.installed);
        await vi.waitFor(() => expect(install.reconciling).toBe(false));
        pendingReconcile = null;
        install.reconciled = false;
        install.reconcileError = "I/O error: scan failed";
      } finally {
        unmount(component);
        target.remove();
      }
    }
  });

  it("renders retained prior-success Agent rows and counts while supplemental mutations stay stale-gated", async () => {
    const retainedRow: InstalledAgent = {
      ...staleControlRow,
      scope: "project",
      projectPath: "/tmp/project",
      dest: "/tmp/project/reviewer.md",
      state: "current",
      tracked: true,
    };
    corpus.agents = [staleControlAgent];
    corpus.categories = [{ slug: "engineering", label: "Engineering", color: "#2563eb", icon: "Code", count: 1 }];
    install.installed = [retainedRow];
    install.tools = [staleControlTool];
    install.reconciled = true;
    install.reconcileError = "I/O error: later scan failed";
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "installs_reconcile") throw { code: "io", message: "later scan failed" };
      if (command === "tools_list") return [staleControlTool] as never;
      if (command === "tool_versions") return [] as never;
      if (command === "projects_list") return [{ path: "/tmp/project", label: "project", installedCount: 1 }] as never;
      if (command === "skill_installs_reconcile" || command === "skill_backups_list") return [] as never;
      return [] as never;
    });

    const cases = [
      {
        name: "AgencyDashboard",
        select: () => undefined,
        assertRetained: (target: HTMLElement) => {
          expect(target.querySelectorAll(".s-num")[1]?.textContent).toBe("1");
          expect(target.querySelector(".install-truth-warning")?.textContent)
            .toContain("Your last known installation data is still shown.");
          expect(target.textContent).not.toContain("No agents installed yet");
        },
      },
      {
        name: "ToolsView",
        select: () => { ui.toolsSelected = "claudeCode"; },
        assertRetained: (target: HTMLElement) => {
          expect(target.querySelector(".trow-sub")?.textContent).toContain("1 agent");
          expect([...target.querySelectorAll<HTMLButtonElement>("button")]
            .find((button) => button.textContent?.includes("Remove all"))?.disabled).toBe(true);
        },
      },
      {
        name: "Teams",
        select: () => undefined,
        assertRetained: (target: HTMLElement) => {
          expect(target.querySelector(".lo-sub")?.textContent).toContain("1 agent");
          for (const label of ["Save as team", "Export"]) {
            expect([...target.querySelectorAll<HTMLButtonElement>("button")]
              .find((button) => button.textContent?.includes(label))?.disabled).toBe(true);
          }
        },
      },
      {
        name: "Projects",
        select: () => { ui.projectsSelected = "/tmp/project"; },
        assertRetained: (target: HTMLElement) => {
          expect(target.querySelector(".dh-count")?.textContent).toContain("1 agent");
          expect(target.querySelector<HTMLButtonElement>(".pr-head.detail .btn.primary")?.disabled).toBe(true);
          expect(target.querySelector<HTMLButtonElement>(".danger-ic")?.disabled).toBe(true);
        },
      },
    ] as const;

    for (const { name, select, assertRetained } of cases) {
      install.installed = [retainedRow];
      install.tools = [staleControlTool];
      install.reconciled = true;
      install.reconcileError = null;
      select();
      const { default: Component } = await import(`$lib/components/${name}.svelte`);
      const target = document.createElement("div");
      document.body.append(target);
      const component = mount(Component, { target });
      try {
        await tick();
        install.installed = [retainedRow];
        install.tools = [staleControlTool];
        install.reconciled = true;
        install.reconcileError = "I/O error: later scan failed";
        await vi.waitFor(() => expect(target.textContent)
          .toContain("Your last known installation data is still shown."));
        assertRetained(target);
      } finally {
        unmount(component);
        target.remove();
        ui.projectsSelected = null;
        ui.toolsSelected = null;
      }
    }
  });

  it.each([
    ["cold", false, null],
    ["stale", true, "I/O error: scan failed"],
  ])("blocks every Agent bulk mutation while install truth is %s", async (_state, reconciled, reconcileError) => {
    const invokeMock = vi.mocked(invoke);
    corpus.agents = [{
      slug: "reviewer", name: "Reviewer", description: "Reviews code", category: "engineering",
      emoji: null, color: null, vibe: null, body: "Review carefully.",
    }];
    install.installed = [{
      slug: "reviewer", name: "Reviewer", sourceId: "built-in", relativePath: "reviewer.md",
      tool: "claudeCode", scope: "user", projectPath: null, dest: "/tmp/reviewer.md",
      state: "foreign", updateKind: null, tracked: false,
    }];
    install.reconciled = reconciled;
    install.reconcileError = reconcileError;
    ui.agentsCategory = "engineering";
    ui.agentsLens = "all";
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "agent_library_list") return emptyFolderState() as never;
      return [] as never;
    });
    const { default: AgentsWorkspace } = await import("$lib/components/AgentsWorkspace.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(AgentsWorkspace, { target });
    try {
      await tick();
      [...target.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.trim() === "Select")!.click();
      await tick();
      target.querySelector<HTMLInputElement>(".row .check")!.click();
      await tick();
      const openMenu = async () => {
        [...target.querySelectorAll<HTMLButtonElement>("button")]
          .find((button) => button.textContent?.includes("With selected"))!.click();
        await tick();
      };

      invokeMock.mockClear();
      await openMenu();
      const update = [...target.querySelectorAll<HTMLButtonElement>(".bulk-opt")]
        .find((button) => button.textContent?.includes("Update —"))!;
      const track = [...target.querySelectorAll<HTMLButtonElement>(".bulk-opt")]
        .find((button) => button.textContent?.includes("Track —"))!;
      const destructive = [...target.querySelectorAll<HTMLButtonElement>(".bulk-opt")]
        .find((button) => button.textContent?.includes("Delete —"))!;
      expect(update.disabled).toBe(true);
      expect(track.disabled).toBe(true);
      expect(destructive.disabled).toBe(true);
      update.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      track.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      destructive.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await tick();
      expect(invokeMock.mock.calls.map(([command]) => command)).toEqual([]);
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("keeps an open Agent rollback armed but disabled while install truth is stale", async () => {
    const invokeMock = vi.mocked(invoke);
    const row = { ...staleControlRow, sourceId: "built-in", relativePath: "reviewer.md", state: "current" as const, tracked: true };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "installs_reconcile") return [row] as never;
      if (command === "tools_list") return [staleControlTool] as never;
      if (command === "projects_list") return [] as never;
      if (command === "agent_version_history") return [{ id: "snapshot-1", createdAt: "2026-08-12T00:00:00Z", sourceHash: "a".repeat(64), renderedHash: "b".repeat(64), contentPath: "/tmp/snapshot" }] as never;
      return undefined as never;
    });
    install.installed = [row];
    install.reconciled = true;
    install.tools = [staleControlTool];
    const { default: InstallModal } = await import("$lib/components/InstallModal.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(InstallModal, { target, props: { title: "Install Reviewer", agentPackage: staleControlPackage, onClose: vi.fn() } });
    try {
      await vi.waitFor(() => expect([...target.querySelectorAll<HTMLButtonElement>("button")]
        .some((button) => button.textContent?.includes("Version history"))).toBe(true));
      [...target.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.includes("Version history"))!.click();
      await vi.waitFor(() => expect(target.querySelector<HTMLButtonElement>(".snapshot button")?.textContent).toContain("Rollback"));
      let rollback = target.querySelector<HTMLButtonElement>(".snapshot button")!;
      rollback.click();
      await tick();
      expect(rollback.textContent).toContain("Confirm rollback");

      install.reconcileError = "I/O error: scan failed";
      await tick();
      rollback = target.querySelector<HTMLButtonElement>(".snapshot button")!;
      expect(rollback.disabled).toBe(true);
      const invokesBeforeBlockedAttempt = invokeMock.mock.calls.length;
      rollback.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      rollback.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      await tick();
      expect(target.querySelector(".history")).not.toBeNull();
      expect(rollback.textContent).toContain("Confirm rollback");
      expect(invokeMock.mock.calls.slice(invokesBeforeBlockedAttempt)).toEqual([]);

      install.reconcileError = null;
      await tick();
      expect(rollback.disabled).toBe(false);
      rollback.click();
      await vi.waitFor(() => expect(invokeMock.mock.calls.filter(([command]) => command === "agent_version_rollback")).toHaveLength(1));
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("keeps InstallModal foreign removal open and disabled while install truth is stale", async () => {
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "installs_reconcile") return [staleControlRow] as never;
      if (command === "tools_list") return [staleControlTool] as never;
      if (command === "projects_list") return [] as never;
      return undefined as never;
    });
    corpus.agents = [staleControlAgent];
    install.installed = [staleControlRow];
    install.reconciled = true;
    install.tools = [staleControlTool];
    const { default: InstallModal } = await import("$lib/components/InstallModal.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(InstallModal, { target, props: { title: "Install Reviewer", agentSlugs: ["reviewer"], onClose: vi.fn() } });
    try {
      await vi.waitFor(() => expect(invokeMock.mock.calls.some(([command]) => command === "installs_reconcile")).toBe(true));
      await vi.waitFor(() => expect(install.reconciling).toBe(false));
      await vi.waitFor(() => expect(target.querySelector<HTMLButtonElement>(".grid-wrap .toggle")?.disabled).toBe(false));
      target.querySelector<HTMLButtonElement>(".grid-wrap .toggle")!.click();
      await tick();
      let confirm = target.querySelector<HTMLButtonElement>('[data-modal-action="confirm"]')!;
      expect(confirm).toBeTruthy();
      confirm.focus();
      expect(document.activeElement).toBe(confirm);

      install.reconcileError = "I/O error: scan failed";
      await tick();
      confirm = target.querySelector<HTMLButtonElement>('[data-modal-action="confirm"]')!;
      expect(confirm.disabled).toBe(true);
      const invokesBeforeBlockedAttempt = invokeMock.mock.calls.length;
      confirm.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      expect(target.contains(confirm)).toBe(true);
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
      await tick();
      expect(target.contains(confirm)).toBe(true);
      expect(invokeMock.mock.calls.slice(invokesBeforeBlockedAttempt)).toEqual([]);

      install.reconcileError = null;
      await tick();
      expect(confirm.disabled).toBe(false);
      confirm.click();
      await vi.waitFor(() => expect(invokeMock.mock.calls.filter(([command]) => command === "uninstall_agent")).toHaveLength(1));
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("keeps DeployBrowser foreign removal open and disabled while install truth is stale", async () => {
    const invokeMock = vi.mocked(invoke);
    const row = { ...staleControlRow, scope: "project" as const, projectPath: "/tmp/project", dest: "/tmp/project/reviewer.md" };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "installs_reconcile") return [row] as never;
      return undefined as never;
    });
    corpus.agents = [staleControlAgent];
    install.installed = [row];
    install.reconciled = true;
    install.tools = [staleControlTool];
    const { default: DeployBrowser } = await import("$lib/components/DeployBrowser.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(DeployBrowser, { target, props: { projectPath: "/tmp/project", onClose: vi.fn() } });
    try {
      await vi.waitFor(() => expect([...target.querySelectorAll<HTMLButtonElement>(".li")]
        .some((button) => button.textContent?.includes("Reviewer"))).toBe(true));
      [...target.querySelectorAll<HTMLButtonElement>(".li")]
        .find((button) => button.textContent?.includes("Reviewer"))!.click();
      await vi.waitFor(() => expect(target.querySelector<HTMLButtonElement>(".grid-wrap .toggle")?.disabled).toBe(false));
      target.querySelector<HTMLButtonElement>(".grid-wrap .toggle")!.click();
      await tick();
      let confirm = target.querySelector<HTMLButtonElement>('[data-modal-action="confirm"]')!;
      expect(confirm).toBeTruthy();

      install.reconcileError = "I/O error: scan failed";
      await tick();
      confirm = target.querySelector<HTMLButtonElement>('[data-modal-action="confirm"]')!;
      expect(confirm.disabled).toBe(true);
      const invokesBeforeBlockedAttempt = invokeMock.mock.calls.length;
      confirm.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      confirm.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      await tick();
      expect(target.querySelector('[role="dialog"] [data-modal-action="confirm"]')).not.toBeNull();
      expect(invokeMock.mock.calls.slice(invokesBeforeBlockedAttempt)).toEqual([]);

      install.reconcileError = null;
      await tick();
      expect(confirm.disabled).toBe(false);
      confirm.click();
      await vi.waitFor(() => expect(invokeMock.mock.calls.filter(([command]) => command === "uninstall_agent")).toHaveLength(1));
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("keeps Tools remove-all open and disabled while install truth is stale", async () => {
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "tools_list") return [staleControlTool] as never;
      if (command === "tool_versions") return [] as never;
      if (command === "installs_reconcile") return [staleControlRow] as never;
      return undefined as never;
    });
    corpus.agents = [staleControlAgent];
    install.installed = [staleControlRow];
    install.reconciled = true;
    install.tools = [staleControlTool];
    const { default: ToolsView } = await import("$lib/components/ToolsView.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(ToolsView, { target });
    try {
      await vi.waitFor(() => expect([...target.querySelectorAll<HTMLButtonElement>("button")]
        .some((button) => button.textContent?.includes("Remove all"))).toBe(true));
      [...target.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.includes("Remove all"))!.click();
      await tick();
      let confirm = target.querySelector<HTMLButtonElement>(".cd-delete")!;
      expect(confirm).toBeTruthy();

      install.reconcileError = "I/O error: scan failed";
      await tick();
      confirm = target.querySelector<HTMLButtonElement>(".cd-delete")!;
      expect(confirm.disabled).toBe(true);
      const invokesBeforeBlockedAttempt = invokeMock.mock.calls.length;
      confirm.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      confirm.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      await tick();
      expect(target.querySelector('[role="alertdialog"] .cd-delete')).not.toBeNull();
      expect(invokeMock.mock.calls.slice(invokesBeforeBlockedAttempt)).toEqual([]);

      install.reconcileError = null;
      await tick();
      expect(confirm.disabled).toBe(false);
      confirm.click();
      await vi.waitFor(() => expect(invokeMock.mock.calls.filter(([command]) => command === "uninstall_agent")).toHaveLength(1));
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("offers only exact tracked outdated and missing Agent and Skill repairs", async () => {
    install.reconciled = true;
    skillSources.reconciled = true;
    install.installed = [
      { ...staleControlRow, sourceId: "built-in", relativePath: "reviewer.md", state: "outdated", tracked: true },
      { ...staleControlRow, slug: "writer", name: "Writer", sourceId: "local", relativePath: "nested/writer.md", projectPath: "/tmp/project", dest: "/tmp/project/writer.md", state: "missing", tracked: true },
      { ...staleControlRow, slug: "edited", name: "Edited", sourceId: "local", relativePath: "edited.md", state: "modified", tracked: true },
      { ...staleControlRow, slug: "foreign", name: "Foreign", sourceId: "", relativePath: "", state: "foreign", tracked: false },
      { ...staleControlRow, slug: "other-foreign", name: "Other Foreign", sourceId: "", relativePath: "", dest: "/tmp/other-foreign.md", state: "foreign", tracked: false },
    ];
    skillSources.installed = [
      { sourceId: "skills", relativePath: "audit", name: "Audit", runtime: "codex", scope: "user", projectPath: null, path: "/tmp/audit", state: "outdated", tracked: true },
      { sourceId: "skills", relativePath: "build", name: "Build", runtime: "claudeCode", scope: "project", projectPath: "/tmp/project", path: "/tmp/project/build", state: "missing", tracked: true },
      { sourceId: "skills", relativePath: "off", name: "Off", runtime: "codex", scope: "user", projectPath: null, path: "/tmp/off", state: "disabled", tracked: true },
      { sourceId: "gone", relativePath: "source", name: "Gone", runtime: "codex", scope: "user", projectPath: null, path: "/tmp/gone", state: "sourceUnavailable", tracked: true },
    ];
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(UpdatesModal, { target, props: { onClose: vi.fn() } });
    try {
      await tick();
      const choices = [...target.querySelectorAll<HTMLInputElement>('input[name="repair-item"]')];
      expect(choices).toHaveLength(4);
      expect(choices.every((choice) => choice.checked)).toBe(true);
      expect(choices.map((choice) => choice.dataset.candidateKey)).toEqual([
        "agent\0built-in\0reviewer.md\0claudeCode\0",
        "agent\0local\0nested/writer.md\0claudeCode\0/tmp/project",
        "skill\0skills\0audit\0codex\0",
        "skill\0skills\0build\0claudeCode\0/tmp/project",
      ]);
      expect(target.textContent).toContain("Update");
      expect(target.textContent).toContain("Reinstall");
      expect(target.textContent).toContain("Local changes require manual review");
      expect(target.textContent).toContain("Untracked content requires manual review");
      expect(target.textContent).toContain("Other Foreign");
      expect(target.textContent).toContain("Enable this installation before repair");
      expect(target.textContent).toContain("Restore its source before repair");
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("blocks repair review until both ledgers have fresh successful truth", async () => {
    install.reconciled = true;
    install.installed = [{ ...staleControlRow, sourceId: "built-in", relativePath: "reviewer.md", state: "outdated", tracked: true }];
    skillSources.reconciled = false;
    skillSources.reconcileError = "I/O error: scan failed";
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(UpdatesModal, { target, props: { onClose: vi.fn() } });
    try {
      await tick();
      expect(target.textContent).toContain("Skill installation status is unavailable");
      expect(target.querySelector<HTMLButtonElement>('[data-modal-action="confirm"]')?.disabled).toBe(true);
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("shows complete Agent and Skill plans and disables blocked approval", async () => {
    const agent = { ...staleControlRow, sourceId: "built-in", relativePath: "reviewer.md", state: "outdated" as const, tracked: true };
    const skill: InstalledSkill = { sourceId: "skills", relativePath: "audit", name: "Audit", runtime: "codex", scope: "user", projectPath: null, path: "/tmp/audit", state: "missing", tracked: true };
    install.reconciled = true;
    skillSources.reconciled = true;
    install.installed = [agent];
    skillSources.installed = [skill];
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "agent_update_plan") return {
        revision: "rev-1", operation: "update", tool: "claudeCode", scope: "user", projectPath: null,
        agents: [{ reference: { sourceId: "built-in", relativePath: "reviewer.md" }, name: "Reviewer", sourceHash: "hash", dependency: false, destination: "/tmp/reviewer.md", renderedFileCount: 1, capabilities: ["review"] }],
        warnings: ["Agent warning"], blockers: [], rollbackAvailable: true,
      } as never;
      if (command === "skill_install_plan") return {
        operation: "install", runtime: "codex", projectPath: null,
        packages: [{ sourceId: "skills", relativePath: "audit", name: "Audit", dependency: false, destination: "/tmp/audit", fileCount: 3, permissions: ["filesystem"] }],
        warnings: ["Skill warning"], blockers: ["Trust required"], rollbackAvailable: false,
      } as never;
      if (command === "skill_sources_inspect") return repairSkillInspection("skill-hash-1") as never;
      return [] as never;
    });
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(UpdatesModal, { target, props: { onClose: vi.fn() } });
    try {
      await tick();
      target.querySelector<HTMLButtonElement>('[data-modal-action="confirm"]')!.click();
      await vi.waitFor(() => expect(target.textContent).toContain("Trust required"));
      expect(target.textContent).toContain("/tmp/reviewer.md");
      expect(target.textContent).toContain("Agent warning");
      expect(target.textContent).toContain("Rollback available");
      expect(target.textContent).toContain("/tmp/audit");
      expect(target.textContent).toContain("3 files");
      expect(target.textContent).toContain("filesystem");
      expect(target.textContent).toContain("Skill warning");
      expect([...target.querySelectorAll<HTMLButtonElement>("button")].some((button) => button.textContent?.includes("View diff"))).toBe(true);
      expect(target.querySelector<HTMLButtonElement>('[data-modal-action="confirm"]')!.disabled).toBe(true);
      expect(vi.mocked(invoke).mock.calls.map(([command]) => command)).toEqual(["skill_sources_inspect", "agent_update_plan", "skill_install_plan"]);
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("invalidates changed repair plans before the first mutation", async () => {
    const agent = { ...staleControlRow, sourceId: "built-in", relativePath: "reviewer.md", state: "outdated" as const, tracked: true };
    install.reconciled = true;
    skillSources.reconciled = true;
    install.installed = [agent];
    let planRevision = "rev-1";
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "agent_update_plan") return {
        revision: planRevision, operation: "update", tool: "claudeCode", scope: "user", projectPath: null,
        agents: [{ reference: { sourceId: "built-in", relativePath: "reviewer.md" }, name: "Reviewer", sourceHash: "hash", dependency: false, destination: "/tmp/reviewer.md", renderedFileCount: 1, capabilities: [] }],
        warnings: [], blockers: [], rollbackAvailable: true,
      } as never;
      if (command === "installs_reconcile") return [agent] as never;
      if (command === "skill_installs_reconcile" || command === "skill_backups_list") return [] as never;
      return [] as never;
    });
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(UpdatesModal, { target, props: { onClose: vi.fn() } });
    try {
      await tick();
      target.querySelector<HTMLButtonElement>('[data-modal-action="confirm"]')!.click();
      await vi.waitFor(() => expect(target.textContent).toContain("Approve repairs"));
      planRevision = "rev-2";
      vi.mocked(invoke).mockClear();
      target.querySelector<HTMLButtonElement>('[data-modal-action="confirm"]')!.click();
      await vi.waitFor(() => expect(target.textContent).toContain("Repair plan changed"));
      expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "update_agent")).toBe(false);
      expect(target.textContent).toContain("Review the updated plan");
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("performs no repair when the approval-time reconciliation fails", async () => {
    const agent = { ...staleControlRow, sourceId: "built-in", relativePath: "reviewer.md", state: "outdated" as const, tracked: true };
    install.reconciled = true;
    skillSources.reconciled = true;
    install.installed = [agent];
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "agent_update_plan") return {
        revision: "rev-1", operation: "update", tool: "claudeCode", scope: "user", projectPath: null,
        agents: [{ reference: { sourceId: "built-in", relativePath: "reviewer.md" }, name: "Reviewer", sourceHash: "hash", dependency: false, destination: "/tmp/reviewer.md", renderedFileCount: 1, capabilities: [] }],
        warnings: [], blockers: [], rollbackAvailable: true,
      } as never;
      if (command === "installs_reconcile") throw new Error("fresh scan failed");
      if (command === "skill_installs_reconcile" || command === "skill_backups_list") return [] as never;
      return [] as never;
    });
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(UpdatesModal, { target, props: { onClose: vi.fn() } });
    try {
      await tick();
      target.querySelector<HTMLButtonElement>('[data-modal-action="confirm"]')!.click();
      await vi.waitFor(() => expect(target.textContent).toContain("Approve repairs"));
      vi.mocked(invoke).mockClear();
      target.querySelector<HTMLButtonElement>('[data-modal-action="confirm"]')!.click();
      await vi.waitFor(() => expect(target.textContent).toContain("Fresh reconciliation failed"));
      expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "update_agent")).toBe(false);
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("invalidates approval when Skill source bytes change behind an identical displayed plan", async () => {
    const skill: InstalledSkill = { sourceId: "skills", relativePath: "audit", name: "Audit", runtime: "codex", scope: "user", projectPath: null, path: "/tmp/audit", state: "outdated", tracked: true };
    install.reconciled = true;
    skillSources.reconciled = true;
    skillSources.installed = [skill];
    let sourceHash = "skill-hash-1";
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "skill_sources_inspect") return repairSkillInspection(sourceHash) as never;
      if (command === "skill_install_plan") return {
        operation: "install", runtime: "codex", projectPath: null,
        packages: [{ sourceId: "skills", relativePath: "audit", name: "Audit", dependency: false, destination: "/tmp/audit", fileCount: 1, permissions: [] }],
        warnings: [], blockers: [], rollbackAvailable: true,
      } as never;
      if (command === "installs_reconcile") return [] as never;
      if (command === "skill_installs_reconcile") return [skill] as never;
      if (command === "skill_backups_list") return [] as never;
      return [] as never;
    });
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(UpdatesModal, { target, props: { onClose: vi.fn() } });
    try {
      await tick();
      target.querySelector<HTMLButtonElement>('[data-modal-action="confirm"]')!.click();
      await vi.waitFor(() => expect(target.textContent).toContain("Approve repairs"));
      sourceHash = "skill-hash-2";
      vi.mocked(invoke).mockClear();
      target.querySelector<HTMLButtonElement>('[data-modal-action="confirm"]')!.click();
      await vi.waitFor(() => expect(target.textContent).toContain("Repair plan changed"));
      expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "skill_update")).toBe(false);
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("continues exact repairs after failure and opens one mixed exact receipt", async () => {
    const agent = { ...staleControlRow, sourceId: "built-in", relativePath: "reviewer.md", state: "outdated" as const, tracked: true };
    const skill: InstalledSkill = { sourceId: "skills", relativePath: "audit", name: "Audit", runtime: "codex", scope: "user", projectPath: null, path: "/tmp/audit", state: "missing", tracked: true };
    const currentSkill = { ...skill, state: "current" as const };
    install.reconciled = true;
    skillSources.reconciled = true;
    install.installed = [agent];
    skillSources.installed = [skill];
    const startingEntries = activity.entries.length;
    let skillRepaired = false;
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "agent_update_plan") return {
        revision: "rev-1", operation: "update", tool: "claudeCode", scope: "user", projectPath: null,
        agents: [{ reference: { sourceId: "built-in", relativePath: "reviewer.md" }, name: "Reviewer", sourceHash: "hash", dependency: false, destination: "/tmp/reviewer.md", renderedFileCount: 1, capabilities: [] }],
        warnings: [], blockers: [], rollbackAvailable: true,
      } as never;
      if (command === "skill_install_plan") return {
        operation: "install", runtime: "codex", projectPath: null,
        packages: [{ sourceId: "skills", relativePath: "audit", name: "Audit", dependency: false, destination: "/tmp/audit", fileCount: 3, permissions: [] }],
        warnings: [], blockers: [], rollbackAvailable: false,
      } as never;
      if (command === "skill_sources_inspect") return repairSkillInspection("skill-hash-1") as never;
      if (command === "installs_reconcile") return [agent] as never;
      if (command === "skill_installs_reconcile") return [skillRepaired ? currentSkill : skill] as never;
      if (command === "skill_backups_list") return [] as never;
      if (command === "update_agent") throw new Error("token=secret123 agent write failed");
      if (command === "skill_update") { skillRepaired = true; return currentSkill as never; }
      return [] as never;
    });
    const target = document.createElement("div");
    document.body.append(target);
    const onClose = vi.fn();
    const component = mount(UpdatesModal, { target, props: { onClose } });
    try {
      await tick();
      target.querySelector<HTMLButtonElement>('[data-modal-action="confirm"]')!.click();
      await vi.waitFor(() => expect(target.textContent).toContain("Approve repairs"));
      vi.mocked(invoke).mockClear();
      target.querySelector<HTMLButtonElement>('[data-modal-action="confirm"]')!.click();
      await vi.waitFor(() => expect(target.textContent).toContain("Repair results"));
      expect(target.textContent).toContain("Reviewer");
      expect(target.textContent).toContain("token=[redacted] agent write failed");
      expect(target.textContent).not.toContain("secret123");
      expect(target.textContent).toContain("/tmp/reviewer.md");
      expect(target.textContent).toContain("/tmp/audit");
      expect(target.textContent).toContain("Audit");
      expect(target.textContent).toContain("Repaired");
      expect(target.textContent).toContain("In sync");
      const mutations = vi.mocked(invoke).mock.calls
        .map(([command]) => command)
        .filter((command) => command === "update_agent" || command === "skill_update");
      expect(mutations).toEqual(["update_agent", "skill_update"]);
      const entries = activity.entries.slice(0, activity.entries.length - startingEntries);
      expect(entries.filter((entry) => entry.action === "update" && entry.subject === "agent" && entry.outcome === "error")).toHaveLength(1);
      expect(entries.filter((entry) => entry.action === "update" && entry.subject === "skill" && entry.outcome === "ok")).toHaveLength(1);
      const summaries = entries.filter((entry) => entry.action === "bulk" && entry.subject === "agentLibrary" && entry.detail === "1 repaired · 1 failed");
      expect(summaries).toHaveLength(1);
      expect(summaries[0]?.receipt).toMatchObject({ operation: "repair", succeeded: 1, failed: 1 });
      expect(summaries[0]?.receipt?.items).toEqual([
        { kind: "agent", name: "Reviewer", destination: "/tmp/reviewer.md", outcome: "error", detail: "token=[redacted] agent write failed" },
        { kind: "skill", name: "Audit", destination: "/tmp/audit", outcome: "ok" },
      ]);
      const viewActivity = [...target.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.includes("View Activity"));
      expect(viewActivity).toBeTruthy();
      viewActivity!.click();
      expect(onClose).toHaveBeenCalledOnce();
      expect(ui.section).toBe("activity");
      expect(ui.activityReceiptId).toBe(summaries[0]?.id);
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("keeps existing repair entry points visible for a Skill-only repair", async () => {
    install.reconciled = true;
    skillSources.reconciled = true;
    skillSources.installed = [{ sourceId: "skills", relativePath: "audit", name: "Audit", runtime: "codex", scope: "user", projectPath: null, path: "/tmp/audit", state: "outdated", tracked: true }];
    const target = document.createElement("div");
    document.body.append(target);
    const divisions = mount(DivisionsLanding, { target });
    const sidebar = mount(Sidebar, { target });
    try {
      await tick();
      expect([...target.querySelectorAll<HTMLButtonElement>("button")].some((button) => button.textContent?.includes("1 repair"))).toBe(true);
      expect(target.querySelector<HTMLElement>(".sidebar .badge")?.textContent).toBe("1");
      expect(target.querySelector<HTMLElement>(".sidebar .badge")?.title).toContain("1 repairable installation");
    } finally {
      unmount(sidebar);
      unmount(divisions);
      target.remove();
    }
  });

  it("moves safe focus to review and announces active repair progress", async () => {
    const agent = { ...staleControlRow, sourceId: "built-in", relativePath: "reviewer.md", state: "outdated" as const, tracked: true };
    install.reconciled = true;
    skillSources.reconciled = true;
    install.installed = [agent];
    let releaseUpdate!: () => void;
    const updatePending = new Promise<void>((resolve) => (releaseUpdate = resolve));
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "agent_update_plan") return {
        revision: "rev-1", operation: "update", tool: "claudeCode", scope: "user", projectPath: null,
        agents: [{ reference: { sourceId: "built-in", relativePath: "reviewer.md" }, name: "Reviewer", sourceHash: "hash", dependency: false, destination: "/tmp/reviewer.md", renderedFileCount: 1, capabilities: [] }],
        warnings: [], blockers: [], rollbackAvailable: true,
      } as never;
      if (command === "installs_reconcile") return [agent] as never;
      if (command === "skill_installs_reconcile" || command === "skill_backups_list") return [] as never;
      if (command === "update_agent") { await updatePending; return {} as never; }
      return [] as never;
    });
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(UpdatesModal, { target, props: { onClose: vi.fn() } });
    try {
      await tick();
      target.querySelector<HTMLButtonElement>('[data-modal-action="confirm"]')!.click();
      await vi.waitFor(() => expect(target.textContent).toContain("Approve repairs"));
      expect(document.activeElement?.textContent).toContain("Back");
      target.querySelector<HTMLButtonElement>('[data-modal-action="confirm"]')!.click();
      await vi.waitFor(() => expect(target.querySelector('[role="status"]')?.textContent).toContain("Repairing 0 of 1"));
      expect(target.querySelector('[role="dialog"]')?.getAttribute("aria-modal")).toBe("true");
      releaseUpdate();
      await vi.waitFor(() => expect(target.textContent).toContain("Repair results"));
    } finally {
      releaseUpdate();
      unmount(component);
      target.remove();
    }
  });

  it("cannot dismiss or leave the approved review while fresh preflight is running", async () => {
    const agent = { ...staleControlRow, sourceId: "built-in", relativePath: "reviewer.md", state: "outdated" as const, tracked: true };
    install.reconciled = true;
    skillSources.reconciled = true;
    install.installed = [agent];
    let releaseReconcile!: () => void;
    const reconcilePending = new Promise<void>((resolve) => (releaseReconcile = resolve));
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "agent_update_plan") return {
        revision: "rev-1", operation: "update", tool: "claudeCode", scope: "user", projectPath: null,
        agents: [{ reference: { sourceId: "built-in", relativePath: "reviewer.md" }, name: "Reviewer", sourceHash: "hash", dependency: false, destination: "/tmp/reviewer.md", renderedFileCount: 1, capabilities: [] }],
        warnings: [], blockers: [], rollbackAvailable: true,
      } as never;
      if (command === "installs_reconcile") { await reconcilePending; return [agent] as never; }
      if (command === "skill_installs_reconcile" || command === "skill_backups_list") return [] as never;
      if (command === "update_agent") return {} as never;
      return [] as never;
    });
    const onClose = vi.fn();
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(UpdatesModal, { target, props: { onClose } });
    try {
      await tick();
      target.querySelector<HTMLButtonElement>('[data-modal-action="confirm"]')!.click();
      await vi.waitFor(() => expect(target.textContent).toContain("Approve repairs"));
      target.querySelector<HTMLButtonElement>('[data-modal-action="confirm"]')!.click();
      await vi.waitFor(() => expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "installs_reconcile")).toBe(true));
      expect(target.querySelector<HTMLButtonElement>('[data-modal-action="cancel"]')?.disabled).toBe(true);
      expect(target.querySelector<HTMLButtonElement>("button.close")).toBeNull();
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
      await tick();
      expect(onClose).not.toHaveBeenCalled();
      releaseReconcile();
      await vi.waitFor(() => expect(target.textContent).toContain("Repair results"));
    } finally {
      releaseReconcile();
      unmount(component);
      target.remove();
    }
  });

  it("uses one neutral initial-unknown contract across direct Agent-ledger consumers", () => {
    for (const source of [dashboardSource, toolsViewSource, teamsSource, projectsSource]) {
      expect(source).toContain('i18n.optional("reconcile.checking", "Checking installation status…")');
      expect(source).toMatch(/reconcileError\s*\?/);
      expect(source).toMatch(/\{#if install\.reconcileError\}/);
      expect(source).not.toMatch(/\{#if !install\.reconciled\}[^]*?Installation status is unavailable until a retry succeeds\./);
    }
  });

  it("guards the complete researched REL-01 conversion inventory and its three native fallbacks", () => {
    const inventory = new Map<string, RegExp[]>([
      ["./components/Teams.svelte", [/teams\.exportFailed[^\n]*appErrorMessage/, /teams\.restoreFailed[^\n]*appErrorMessage/]],
      ["./components/SettingsSectionCatalog.svelte", [/catalog\.actionFailed[^\n]*appErrorMessage/]],
      ["./components/ToolsView.svelte", [/common\.actionFailed[^\n]*appErrorMessage/, /common\.couldNotOpenFolder[^\n]*appErrorMessage/, /Could not save install location[^\n]*appErrorMessage/]],
      ["./components/AgentDetailTabs.svelte", [/sourceError = isAppError[^\n]*appErrorMessage/, /renderError = isAppError[^\n]*appErrorMessage/]],
      ["./components/CatalogFirstRun.svelte", [/firstRun\.error[^\n]*appErrorMessage/]],
      ["./components/SkillsWorkspace.svelte", [/skillCollectionBatch[^]*?announcement = isAppError[^\n]*appErrorMessage/, /readSkillText[^]*?folderError = isAppError[^\n]*appErrorMessage/, /skillInstallPlan[^]*?announcement = isAppError[^\n]*appErrorMessage/]],
      ["./components/Experts.svelte", [/Could not plan activation[^\n]*appErrorMessage/, /detail: isAppError[^\n]*appErrorMessage/, /Activation failed[^\n]*appErrorMessage/, /Could not save Expert[^\n]*appErrorMessage/, /Could not reject Expert proposal[^\n]*appErrorMessage/, /Could not review run[^\n]*appErrorMessage/, /Import failed[^\n]*appErrorMessage/, /Export failed[^\n]*appErrorMessage/]],
      ["./components/InstallModal.svelte", [/async function reviewPlan[^]*?actionError = isAppError[^\n]*appErrorMessage/, /async function reviewCollection[^]*?actionError = isAppError[^\n]*appErrorMessage/, /async function applyPlan[^]*?actionError = isAppError[^\n]*appErrorMessage/, /async function runLifecycle[^]*?actionError = isAppError[^\n]*appErrorMessage/, /async function showHistory[^]*?actionError = isAppError[^\n]*appErrorMessage/, /async function rollback[^]*?actionError = isAppError[^\n]*appErrorMessage/, /async function reviewRoster[^]*?actionError = isAppError[^\n]*appErrorMessage/, /async function applyRosterPlan[^]*?actionError = isAppError[^\n]*appErrorMessage/, /async function runRosterLifecycle[^]*?actionError = isAppError[^\n]*appErrorMessage/, /async function showRosterHistory[^]*?actionError = isAppError[^\n]*appErrorMessage/, /async function rollbackRoster[^]*?actionError = isAppError[^\n]*appErrorMessage/]],
      ["./components/DiffModal.svelte", [/install\.diff[^]*?appErrorMessage/]],
      ["./components/AgencyDashboard.svelte", [/async function updateCatalog[^]*?appErrorMessage/]],
      ["./components/Projects.svelte", [/async function reveal[^]*?appErrorMessage/, /async function forgetProject[^]*?appErrorMessage/, /async function uninstallAndRemove[^]*?appErrorMessage/, /async function refreshProjectInstructions[^]*?appErrorMessage/, /async function reviewInstruction[^]*?appErrorMessage/, /async function applyInstruction[^]*?appErrorMessage/]],
      ["./stores/catalog.svelte.ts", [/catalog_status[^]*?this\.error = isAppError[^\n]*appErrorMessage/, /catalog_check_updates[^]*?this\.error = isAppError[^\n]*appErrorMessage/, /catalog_detect[^]*?this\.error = isAppError[^\n]*appErrorMessage/, /async setSource[^]*?this\.error = isAppError[^\n]*appErrorMessage/, /async provisionManaged[^]*?this\.error = isAppError[^\n]*appErrorMessage/, /catalog_pull[^]*?this\.error = isAppError[^\n]*appErrorMessage/]],
      ["./stores/settings.svelte.ts", [/corruptOnDisk = true[^]*?this\.error = appErrorMessage/, /else if \(isAppError\(e\)\)[^]*?this\.error = appErrorMessage/, /async save[^]*?this\.error = appErrorMessage/, /async reset[^]*?this\.error = appErrorMessage/, /async applySecurityPosture[^]*?this\.error = isAppError[^\n]*appErrorMessage/]],
      ["./stores/experts.svelte.ts", [/expert_runs_list[^]*?this\.error = isAppError[^\n]*appErrorMessage/]],
      ["./stores/activity.svelte.ts", [/safeActivityDetail[^]*?isAppError\(value\) \? appErrorMessage\(value\)/]],
    ]);
    expect([...inventory.values()].flat()).toHaveLength(52);
    for (const [path, markers] of inventory) {
      const source = rel01Sources[path];
      expect(source, path).toBeTruthy();
      for (const marker of markers) expect(source.match(marker) ?? [], `${path}: ${marker}`).toHaveLength(1);
    }
    expect([...inventory.keys()].flatMap((path) => rel01Sources[path].match(/\bappErrorMessage\(/g) ?? []))
      .toHaveLength(64);

    const installSource = rel01Sources["./stores/install.svelte.ts"];
    const propagationEdges = new Map([
      ["exactMutation", /private async exactMutation<T>\([^]*?detail: safeActivityDetail\(/],
      ["applyCollection", /async applyCollection\([^]*?detail: safeActivityDetail\(/],
      ["legacy install", /async install\([^]*?detail: safeActivityDetail\(/],
      ["uninstall", /async uninstall\([^]*?detail: safeActivityDetail\(/],
      ["update", /async update\([^]*?detail: safeActivityDetail\(/],
      ["track", /async track\([^]*?detail: safeActivityDetail\(/],
      ["bulk", /async bulk\([^]*?detail: safeActivityDetail\(/],
    ]);
    for (const [name, marker] of propagationEdges) {
      expect(installSource.match(marker) ?? [], name).toHaveLength(1);
    }
    expect(installSource.match(/safeActivityDetail\(/g) ?? []).toHaveLength(11); // seven live edges + legacy install() + Workspace Pack retained-result, receipt, and success-notice boundaries

    const rawFallbacks = Object.entries(rel01Sources).flatMap(([path, source]) =>
      [...source.matchAll(/^\s*toast\.error\([^\n]*,\s*String\((?:e|error)\)\);?$/gm)]
        .map((match) => `${path}:${match[0].trim()}`)).toSorted();
    expect(rawFallbacks).toEqual([
      './components/ToolsView.svelte:toast.error("Could not open the folder picker", String(e));',
      './components/Experts.svelte:toast.error("Copy failed", String(error));',
      './components/Runbooks.svelte:toast.error(i18n.t("common.copyFailed"), String(e));',
    ].toSorted());
  });

  it("exposes project roster targets only for multiple exact Agents with isolated truth", () => {
    expect(SUPPORTED_TOOLS.filter((tool) => tool.installKind === "roster").map((tool) => tool.id))
      .toEqual(["aider", "windsurf"]);
    expect(SUPPORTED_TOOLS.filter((tool) => tool.installKind === "roster").every((tool) =>
      tool.supportsProject && !tool.supportsUser)).toBe(true);
    expect(installModalSource).toContain('t.installKind !== "roster" || exactReferences.length > 1');
    expect(installModalSource).toContain("rosterPending.destination");
    expect(installModalSource).toContain("rosterPending.members");
    expect(installModalSource).toContain("sameRosterMembers");
    expect(installModalSource).toContain("reviewRoster(cov.roster ? (cov.all ? \"uninstall\" : \"update\") : \"install\"");
    expect(installModalSource).toContain("install.rostersReconciled && !install.rosterReconciling && !install.rosterReconcileError");
    const installStoreSource = rel01Sources["./stores/install.svelte.ts"];
    expect(installStoreSource).toContain('invoke<InstalledAgent[]>("installs_reconcile"');
    expect(installStoreSource).toContain("async reconcileRosters()");
    expect(installStoreSource).toContain("agentRostersReconcile()");
  });

  it("keeps roster lifecycle and modified-file rollback live while Agent truth is unavailable", async () => {
    const projectPath = "/tmp/roster-project";
    const packages = [
      staleControlPackage,
      {
        ...staleControlPackage,
        reference: { sourceId: "built-in", relativePath: "auditor.md" },
        agent: { ...staleControlAgent, slug: "auditor", name: "Auditor" },
      },
    ];
    agentLibrary.results = [{
      source: { id: "built-in", label: "Built in", enabled: true, kind: { kind: "builtIn" } },
      agents: packages,
      errors: [],
      revision: "roster-ui",
    }];
    const projectRows = [{ path: projectPath, label: "Roster project", installedCount: 0 }];
    const aiderTool = {
      ...staleControlTool,
      tool: "aider" as const,
      label: "Aider",
      scope: "project" as const,
      userDest: null,
      installedCount: 0,
    };
    const members = packages.map((pkg) => ({
      reference: pkg.reference,
      name: pkg.agent!.name,
      sourceHash: pkg.sourceHash,
    }));
    const record = {
      tool: "aider" as const,
      scope: "project" as const,
      projectPath,
      dest: `${projectPath}/CONVENTIONS.md`,
      members,
      renderedHash: "a".repeat(64),
      disabledPath: null as string | null,
      installedAt: "2026-08-17T00:00:00Z",
    };
    const plan = {
      revision: "b".repeat(64),
      operation: "install" as const,
      tool: "aider" as const,
      scope: "project" as const,
      projectPath,
      destination: record.dest,
      members,
      state: null,
      destinationObservation: {
        active: { kind: "missing" as const, hash: null },
        disabled: { kind: "missing" as const, hash: null },
      },
      warnings: [],
      blockers: [],
      rollbackAvailable: false,
    };
    let rosterRows: Array<{ record: typeof record; state: "current" | "disabled" | "modified" }> = [];
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string, args) => {
      if (command === "projects_list") return projectRows as never;
      if (command === "tools_list") return [aiderTool] as never;
      if (command === "installs_reconcile") throw new Error("Agent install truth unavailable");
      if (command === "agent_rosters_reconcile") return structuredClone(rosterRows) as never;
      if (command === "agent_roster_plan") {
        expect(args).toMatchObject({ references: packages.map((pkg) => pkg.reference), operation: "install", tool: "aider", projectPath });
        return plan as never;
      }
      if (command === "agent_roster_apply") {
        rosterRows = [{ record, state: "current" }];
        return record as never;
      }
      if (command === "agent_roster_disable") {
        record.disabledPath = `${record.dest}.disabled`;
        rosterRows = [{ record, state: "disabled" }];
        return record as never;
      }
      if (command === "agent_roster_enable") {
        record.disabledPath = null;
        rosterRows = [{ record, state: "modified" }];
        return record as never;
      }
      if (command === "agent_roster_version_history") return [{
        id: "roster-snapshot", createdAt: "2026-08-17T00:00:00Z",
        sourceHash: "c".repeat(64), renderedHash: "d".repeat(64), contentPath: "/tmp/snapshot",
      }] as never;
      if (command === "agent_roster_version_rollback") {
        expect(args).toMatchObject({
          tool: "aider", projectPath, snapshotId: "roster-snapshot", confirmed: true,
        });
        rosterRows = [{ record, state: "current" }];
        return record as never;
      }
      return [] as never;
    });
    projects.list = projectRows;
    install.tools = [aiderTool];
    install.reconciled = false;
    install.reconcileError = "Agent install truth unavailable";
    const { default: InstallModal } = await import("$lib/components/InstallModal.svelte");
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(InstallModal, {
      target,
      props: {
        title: "Install roster",
        agentReferences: packages.map((pkg) => pkg.reference),
        allowedTools: ["aider"],
        onClose: vi.fn(),
      },
    });
    const buttonNamed = (name: string) => [...target.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent?.trim() === name);
    try {
      const toggle = await vi.waitFor(() => {
        const candidate = target.querySelector<HTMLButtonElement>(".grid-wrap .toggle");
        expect(candidate?.disabled).toBe(false);
        return candidate!;
      });
      expect(install.reconciled).toBe(false);
      const genericReconcilesAfterMount = invokeMock.mock.calls
        .filter(([command]) => command === "installs_reconcile").length;
      toggle.click();
      const apply = await vi.waitFor(() => {
        expect(invokeMock.mock.calls.filter(([command]) => command === "agent_roster_plan")).toHaveLength(1);
        const candidate = buttonNamed("Apply plan");
        expect(candidate).toBeTruthy();
        return candidate!;
      });
      apply.click();
      await vi.waitFor(() => expect(buttonNamed("Disable")).toBeTruthy());

      buttonNamed("Disable")!.click();
      await vi.waitFor(() => expect(buttonNamed("Enable")).toBeTruthy());
      buttonNamed("Enable")!.click();
      await vi.waitFor(() => expect(buttonNamed("Version history")).toBeTruthy());

      buttonNamed("Version history")!.click();
      const rollback = await vi.waitFor(() => {
        const candidate = target.querySelector<HTMLButtonElement>(".history .snapshot button");
        expect(candidate?.textContent).toContain("Rollback");
        return candidate!;
      });
      rollback.click();
      await tick();
      rollback.click();
      await vi.waitFor(() => expect(invokeMock.mock.calls
        .filter(([command]) => command === "agent_roster_version_rollback")).toHaveLength(1));
      await vi.waitFor(() => expect(invokeMock.mock.calls
        .filter(([command]) => command === "agent_rosters_reconcile").length).toBeGreaterThanOrEqual(5));
      expect(invokeMock.mock.calls.filter(([command]) => command === "installs_reconcile"))
        .toHaveLength(genericReconcilesAfterMount);
      expect(install.reconciled).toBe(false);
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("counts and uninstalls exact project rosters before unregistering the project", async () => {
    const { default: Projects } = await import("$lib/components/Projects.svelte");
    const projectPath = "/tmp/roster-project";
    const projectRows = [{ path: projectPath, label: "Roster project", installedCount: 2 }];
    const members = [
      { reference: { sourceId: "built-in", relativePath: "reviewer.md" }, name: "Reviewer", sourceHash: "a".repeat(64) },
      { reference: { sourceId: "built-in", relativePath: "auditor.md" }, name: "Auditor", sourceHash: "b".repeat(64) },
    ];
    const record = {
      tool: "aider" as const,
      scope: "project" as const,
      projectPath,
      dest: `${projectPath}/CONVENTIONS.md`,
      members,
      renderedHash: "c".repeat(64),
      disabledPath: null,
      installedAt: "2026-08-18T00:00:00Z",
    };
    const plan = {
      revision: "d".repeat(64),
      operation: "uninstall" as const,
      tool: "aider" as const,
      scope: "project" as const,
      projectPath,
      destination: record.dest,
      members,
      state: "current" as const,
      destinationObservation: {
        active: { kind: "file" as const, hash: record.renderedHash },
        disabled: { kind: "missing" as const, hash: null },
      },
      warnings: [],
      blockers: [],
      rollbackAvailable: true,
    };
    let rosterInstalled = true;
    let projectRegistered = true;
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string, args) => {
      if (command === "installs_reconcile" || command === "skill_installs_reconcile" || command === "skill_backups_list") return [] as never;
      if (command === "agent_rosters_reconcile") return (rosterInstalled ? [{ record, state: "current" }] : []) as never;
      if (command === "projects_list") return (projectRegistered ? projectRows : []) as never;
      if (command === "project_instructions_inspect" || command === "project_recommendations_list") return [] as never;
      if (command === "project_readiness_get") return readinessFixture(projectPath, false) as never;
      if (command === "agent_roster_plan") {
        expect(args).toMatchObject({
          references: members.map((member) => member.reference),
          operation: "uninstall",
          tool: "aider",
          projectPath,
        });
        return plan as never;
      }
      if (command === "agent_roster_apply") {
        expect(args).toMatchObject({ operation: "uninstall", tool: "aider", projectPath, confirmed: true });
        rosterInstalled = false;
        return record as never;
      }
      if (command === "project_unregister") {
        expect(rosterInstalled).toBe(false);
        projectRegistered = false;
        return true as never;
      }
      return [] as never;
    });
    corpus.agents = [staleControlAgent];
    projects.list = projectRows;
    ui.projectsSelected = projectPath;
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(Projects, { target });
    try {
      const remove = await vi.waitFor(() => {
        expect(target.textContent).toContain("2 agents");
        const candidate = target.querySelector<HTMLButtonElement>(".danger-ic");
        expect(candidate?.disabled).toBe(false);
        return candidate!;
      });
      remove.click();
      const uninstall = await vi.waitFor(() => {
        const candidate = [...target.querySelectorAll<HTMLButtonElement>("button")]
          .find((button) => button.textContent?.trim() === "Remove & uninstall 2");
        expect(candidate).toBeTruthy();
        return candidate!;
      });
      uninstall.click();
      await vi.waitFor(() => expect(invokeMock.mock.calls.some(([command]) => command === "project_unregister")).toBe(true));
      expect(invokeMock.mock.calls.filter(([command]) => command === "agent_roster_plan")).toHaveLength(1);
      expect(invokeMock.mock.calls.filter(([command]) => command === "agent_roster_apply")).toHaveLength(1);
      expect(invokeMock.mock.calls.some(([command]) => command === "uninstall_agent")).toBe(false);
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("pins every Phase 6 truth-aware install-ledger consumer and every deployment mutation control", () => {
    const consumers = new Map<string, { source: string; markers: RegExp[] }>([
      ["AgentsWorkspace", { source: agentsWorkspaceSource, markers: [
        /for \(const r of install\.installed\)/,
        /if \(!install\.reconciled\) return s;/,
        /const installTruthFresh = \$derived\(install\.reconciled && !install\.reconcileError\);/,
        /\{#if install\.reconcileError\}[^]*?retryReconcile\(event\)/,
        /async function retryReconcile[^]*?await install\.reconcile\(\);/,
        /async function runBulk[^]*?if \(!installTruthFresh\) return;/,
      ] }],
      ["SkillsWorkspace", { source: skillsWorkspaceSource, markers: [
        /const installTruthKnown = \$derived\(skillSources\.reconciled\);/,
        /const installTruthFresh = \$derived\(installTruthKnown && !skillSources\.reconcileError\);/,
        /installed: skillSources\.installed,/,
        /\{#if skillSources\.reconcileError\}[^]*?retryReconcile\(event\)/,
        /async function retryReconcile[^]*?await skillSources\.reconcileInstalls\(projects\.list\.map/,
      ] }],
      ["AgencyDashboard", { source: dashboardSource, markers: [
        /const managed = \$derived\(install\.installed\.filter/,
        /\{install\.reconciled \? managed : "—"\}/,
        /\{#if install\.reconcileError\}[^]*?retryReconcile\(event\)/,
        /async function retryReconcile[^]*?await install\.reconcile\(\);/,
      ] }],
      ["ToolsView", { source: toolsViewSource, markers: [
        /const installTruthFresh = \$derived\(install\.reconciled && !install\.reconciling && !install\.reconcileError\);/,
        /install\.installed\.filter\(\(i\) => i\.tool === selectedTool\)/,
        /install\.reconciled \? \(h\.total > 0 \? i18n\.count/,
        /\{#if install\.reconcileError\}[^]*?retryReconcile\(event\)/,
        /async function runToolBulk[^]*?if \(!install\.reconciled \|\| install\.reconciling \|\| install\.reconcileError\) return;/,
      ] }],
      ["Teams", { source: teamsSource, markers: [
        /const managed = \$derived\(install\.installed\.filter/,
        /\{:else if managed\.length === 0\}/,
        /disabled=\{!installTruthFresh\} onclick=\{openSave\}/,
        /disabled=\{!packTruthFresh \|\| busy \|\| portableManagedCount === 0\}/,
        /\{#if install\.reconcileError\}[^]*?retryReconcile\(event\)/,
      ] }],
      ["Projects", { source: projectsSource, markers: [
        /for \(const r of install\.installed\)/,
        /install\.rosters[^]*?row\.record\.projectPath === path/,
        /i18n\.count\(agentCountFor\(selected\.path\)/,
        /const rosterTruthFresh = \$derived\(install\.rostersReconciled[^;]+\);/,
        /const mutationTruthFresh = \$derived\(installTruthFresh && rosterTruthFresh && skillSources\.reconciled && !skillSources\.reconcileError\);/,
        /for \(const roster of aggregateRostersFor\(path\)\)[^]*?install\.planRoster\([^]*?install\.applyRoster\(plan\)/,
        /disabled=\{!installTruthFresh\} onclick=\{\(\) => \(browseFor = selected\.path\)\}/,
        /\{#if install\.reconcileError\}[^]*?retryReconcile\(event\)/,
      ] }],
      ["InstallModal", { source: installModalSource, markers: [
        /const installTruthFresh = \$derived\(install\.reconciled && !install\.reconciling && !install\.reconcileError\);/,
        /install\.installed\.filter\(matchesExact\)/,
        /if \(!installTruthFresh \|\| !pending \|\| !canApplyAgentPlan\(pending\.plan\)\) return;/,
        /if \(!install\.reconciled \|\| install\.reconciling \|\| install\.reconcileError \|\| !confirm\) return;/,
      ] }],
      ["DeployBrowser", { source: deployBrowserSource, markers: [
        /const installTruthFresh = \$derived\(install\.reconciled && !install\.reconciling && !install\.reconcileError\);/,
        /const rows = install\.installed\.filter/,
        /if \(!installTruthFresh \|\| busy \|\| !selected\) return;/,
        /if \(!install\.reconciled \|\| install\.reconciling \|\| install\.reconcileError \|\| !confirm\) return;/,
      ] }],
    ]);
    expect([...consumers.keys()]).toEqual([
      "AgentsWorkspace", "SkillsWorkspace", "AgencyDashboard", "ToolsView",
      "Teams", "Projects", "InstallModal", "DeployBrowser",
    ]);
    for (const [name, { source, markers }] of consumers) {
      for (const marker of markers) expect(source.match(marker) ?? [], `${name}: ${marker}`).toHaveLength(1);
    }

    const mutationControls = new Map<string, { source: string; freshnessUses: number; markers: RegExp[] }>([
      ["SkillsWorkspace", { source: skillsWorkspaceSource, freshnessUses: 19, markers: [
        /disabled: !installTruthFresh \|\| !selected\.pkg\.installable \|\| !canInstall,/,
        /size="sm" disabled=\{!installTruthFresh\}[^\n]*runLifecycle\("disable", installed\)/,
        /size="sm" variant="danger" disabled=\{!installTruthFresh\}[^\n]*uninstallCandidate = installed/,
        /disabled=\{!installTruthFresh\} onclick=\{\(\) => \(collectionInstallOpen = true\)\}/,
        /loading=\{busy\} disabled=\{!installTruthFresh\}[^\n]*runLifecycle\("update", installed\)/,
        /loading=\{busy\} disabled=\{!installTruthFresh\}[^\n]*runLifecycle\("disable", installed\)/,
        /loading=\{busy\} disabled=\{!installTruthFresh\}[^\n]*runLifecycle\("enable", installed\)/,
        /variant="danger" disabled=\{busy \|\| !installTruthFresh\}[^\n]*uninstallCandidate = installed/,
        /disabled=\{!installTruthFresh\} onclick=\{\(\) => void rollbackVersion/,
        /modalAction="confirm" disabled=\{!installTruthFresh\} onclick=\{\(\) => void installCurrentCollection\(\)\}/,
        /modalAction="confirm" disabled=\{!installTruthFresh \|\| plan\.blockers\.length > 0\}/,
        /confirmDisabled=\{!installTruthFresh \|\| \(uninstallCandidate/,
      ] }],
      ["InstallModal", { source: installModalSource, freshnessUses: 18, markers: [
        /const truthFresh = isRosterTool\(t\.id\) \? rosterTruthFresh : installTruthFresh;/,
        /disabled: !truthFresh \|\| total === 0,/,
        /disabled=\{!installTruthFresh\}[^\n]*reviewPlan\("update"/,
        /disabled=\{!installTruthFresh\}[^\n]*runLifecycle\("track"/,
        /disabled=\{!installTruthFresh\}[^\n]*runLifecycle\("enable"/,
        /disabled=\{!installTruthFresh\}[^\n]*runLifecycle\("disable"/,
        /disabled=\{!installTruthFresh\}[^\n]*reviewPlan\("uninstall"/,
        /disabled=\{!installTruthFresh\}[^\n]*reviewCollection\("update"/,
        /disabled=\{!installTruthFresh\}[^\n]*reviewCollection\("uninstall"/,
        /disabled=\{!installTruthFresh\} onclick=\{\(\) => rollback\(snapshot\.id\)\}/,
        /disabled=\{!installTruthFresh \|\| !canApplyAgentPlan\(pending\.plan\) \|\| !!busy\}/,
        /confirmDisabled=\{!installTruthFresh\}/,
        /disabled=\{!rosterTruthFresh[^\n]*reviewRoster\("update"/,
        /disabled=\{!rosterTruthFresh\}[^\n]*runRosterLifecycle\(row, true\)/,
        /disabled=\{!rosterTruthFresh\}[^\n]*runRosterLifecycle\(row, false\)/,
        /disabled=\{!rosterTruthFresh\}[^\n]*rollbackRoster\(snapshot\.id\)/,
      ] }],
      ["DeployBrowser", { source: deployBrowserSource, freshnessUses: 3, markers: [
        /disabled=\{!installTruthFresh \|\| isBusy \|\| setTotal === 0\}/,
        /confirmDisabled=\{!installTruthFresh\}/,
      ] }],
    ]);
    for (const [name, { source, freshnessUses, markers }] of mutationControls) {
      expect(source.match(/!installTruthFresh/g) ?? [], `${name}: exact freshness-use inventory`)
        .toHaveLength(freshnessUses);
      for (const marker of markers) expect(source.match(marker) ?? [], `${name}: ${marker}`).toHaveLength(1);
    }
  });

  it("keeps long reconcile detail and Retry wrap-safe in both workspaces", () => {
    for (const source of [agentsWorkspaceSource, skillsWorkspaceSource]) {
      expect(source).toContain("overflow-wrap: anywhere");
      expect(source).toContain("text-wrap: balance");
      expect(source).toContain("text-wrap: pretty");
      expect(source).toMatch(/\.reconcile-warning[^}]*flex-wrap:\s*wrap/s);
      expect(source).toMatch(/\.reconcile-copy[^}]*min-width:\s*0/s);
      expect(source).not.toContain("flex: 1 1 240px");
      expect(source).not.toMatch(/\.reconcile-warning[^}]*overflow-x:\s*(auto|scroll|hidden)/s);
      expect(source).not.toMatch(/\.reconcile-message[^}]*text-overflow:\s*ellipsis/s);
    }
  });

  it("acknowledges recommendations only after the current generation is rendered", async () => {
    const { default: Projects } = await import("$lib/components/Projects.svelte");
    const projectRows = [
      { path: "/tmp/project-one", label: "project-one", installedCount: 0 },
      { path: "/tmp/project-two", label: "project-two", installedCount: 0 },
    ];
    let resolveFirst!: (value: ProjectRecommendation[]) => void;
    const firstRecommendations = new Promise<ProjectRecommendation[]>((resolve) => {
      resolveFirst = resolve;
    });
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string, args) => {
      const projectPath = (args as { projectPath?: string } | undefined)?.projectPath;
      if (command === "installs_reconcile" || command === "skill_installs_reconcile" || command === "skill_backups_list") return [] as never;
      if (command === "projects_list") return projectRows as never;
      if (command === "project_instructions_inspect") return [] as never;
      if (command === "project_readiness_get") return readinessFixture(projectPath) as never;
      if (command === "project_recommendations_list") {
        return (projectPath === "/tmp/project-one" ? firstRecommendations : []) as never;
      }
      if (command === "project_recommendations_acknowledge") {
        throw new Error("discarded generation must not acknowledge");
      }
      return [] as never;
    });
    corpus.agents = [staleControlAgent];
    corpus.categories = [{ slug: "engineering", label: "Engineering", color: "#2563eb", icon: "Code", count: 1 }];
    projects.list = projectRows;
    ui.projectsSelected = "/tmp/project-one";
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(Projects, { target });
    try {
      await vi.waitFor(() => expect(invokeMock.mock.calls.some(([command, args]) =>
        command === "project_recommendations_list"
        && (args as { projectPath?: string }).projectPath === "/tmp/project-one")).toBe(true));
      ui.selectProject("/tmp/project-two");
      await vi.waitFor(() => expect(invokeMock.mock.calls.some(([command, args]) =>
        command === "project_recommendations_list"
        && (args as { projectPath?: string }).projectPath === "/tmp/project-two")).toBe(true));
      resolveFirst([renamedProjectRecommendation("/tmp/project-one")]);
      await tick();
      await Promise.resolve();
      expect(invokeMock.mock.calls.some(([command]) => command === "project_recommendations_acknowledge")).toBe(false);
      expect(target.textContent).not.toContain("old-reviewer.md → engineering/new-reviewer.md");
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("renders rename recommendations before acknowledging and hands the exact new ref to review", async () => {
    const { default: Projects } = await import("$lib/components/Projects.svelte");
    const projectPath = "/tmp/project";
    const recommendation = renamedProjectRecommendation(projectPath);
    const projectRows = [{ path: projectPath, label: "project", installedCount: 0 }];
    const newPackage: AgentPackageResult = {
      ...staleControlPackage,
      reference: recommendation.agentReferences[0],
      agent: { ...staleControlAgent, slug: "new-reviewer", name: "New Reviewer" },
    };
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "agent_sources_inspect") return [{
        source: { id: "built-in", label: "Built in", enabled: true, kind: { kind: "builtIn" } },
        agents: [{
          ...newPackage,
          reference: recommendation.baselineReference,
          agent: { ...staleControlAgent, slug: "old-reviewer", name: "Old Reviewer" },
        }], errors: [], revision: "stale",
      }] as never;
      if (command === "agent_drafts_list") return [] as never;
      if (command === "agent_library_list") return emptyFolderState() as never;
      return [] as never;
    });
    await agentLibrary.load(true);
    install.tools = [staleControlTool];
    projects.list = projectRows;
    ui.projectsSelected = projectPath;
    corpus.agents = [staleControlAgent];
    corpus.categories = [{ slug: "engineering", label: "Engineering", color: "#2563eb", icon: "Code", count: 1 }];
    const target = document.createElement("div");
    document.body.append(target);
    invokeMock.mockImplementation(async (command: string, args) => {
      if (command === "installs_reconcile" || command === "skill_installs_reconcile" || command === "skill_backups_list") return [] as never;
      if (command === "projects_list") return projectRows as never;
      if (command === "tools_list") return [staleControlTool] as never;
      if (command === "project_instructions_inspect") return [] as never;
      if (command === "project_readiness_get") return readinessFixture(projectPath) as never;
      if (command === "project_recommendations_list") return [recommendation] as never;
      if (command === "project_recommendations_acknowledge") {
        expect(target.textContent).toContain(recommendation.summary);
        expect(args).toEqual({
          projectPath,
          batchAt: recommendation.batchAt,
          recommendationIds: [recommendation.id],
        });
        return true as never;
      }
      if (command === "project_recommendation_open") return recommendation as never;
      if (command === "agent_sources_inspect") return [{
        source: { id: "built-in", label: "Built in", enabled: true, kind: { kind: "builtIn" } },
        agents: [newPackage], errors: [], revision: "refreshed",
      }] as never;
      if (command === "agent_drafts_list") return [] as never;
      if (command === "agent_library_list") return emptyFolderState() as never;
      if (command === "agent_install_plan") {
        expect(args).toMatchObject({
          sourceId: recommendation.agentReferences[0].sourceId,
          relativePath: recommendation.agentReferences[0].relativePath,
          tool: "claudeCode",
          projectPath,
        });
        throw new Error("review boundary reached");
      }
      return [] as never;
    });
    const component = mount(Projects, { target });
    try {
      await vi.waitFor(() => expect(invokeMock.mock.calls.some(([command]) =>
        command === "project_recommendations_acknowledge")).toBe(true));
      const open = [...target.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.trim() === "Open review")!;
      open.click();
      await vi.waitFor(() => expect(target.textContent).toContain("Review catalog recommendation"));
      await vi.waitFor(() => expect(invokeMock.mock.calls.some(([command]) =>
        command === "agent_install_plan")).toBe(true));
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("remounts a mixed rename with only its remaining per-target InstallModal operation", async () => {
    const { default: Projects } = await import("$lib/components/Projects.svelte");
    const projectPath = "/tmp/project";
    const initialRecommendation = renamedProjectRecommendation(projectPath);
    const recommendation: ProjectRecommendation = {
      ...initialRecommendation,
      targets: [
        { ...initialRecommendation.targets[0], tool: "claudeCode", operation: "update" },
        { ...initialRecommendation.targets[0], tool: "codex", operation: "install" },
      ],
    };
    const projectRows = [{ path: projectPath, label: "project", installedCount: 0 }];
    const nextReference = recommendation.agentReferences[0];
    const newPackage: AgentPackageResult = {
      ...staleControlPackage,
      reference: nextReference,
      agent: { ...staleControlAgent, slug: "new-reviewer", name: "New Reviewer" },
    };
    const planFor = (tool: string, operation: "install" | "update"): AgentMutationPlan => ({
      revision: "rename-install",
      operation,
      tool,
      scope: "project",
      projectPath,
      agents: [{
        reference: nextReference,
        name: "New Reviewer",
        sourceHash: "source",
        dependency: false,
        destination: `/tmp/project/.agents/${tool}/new-reviewer.md`,
        renderedFileCount: 1,
        capabilities: [],
      }],
      warnings: [], blockers: [], rollbackAvailable: true,
    });
    const targetStates = new Map<string, "outdated" | "current">([["claudeCode", "outdated"]]);
    let finalized = false;
    const liveRecommendation = (): ProjectRecommendation => {
      const targets = recommendation.targets.filter((target) => targetStates.get(target.tool) !== "current");
      return {
        ...recommendation,
        lifecycle: targetStates.get("claudeCode") === "current" ? "pending" : "new",
        targets,
        finalizeOnly: targets.length === 0,
      };
    };
    install.tools = [
      staleControlTool,
      { ...staleControlTool, tool: "codex", label: "Codex" },
    ];
    projects.list = projectRows;
    ui.projectsSelected = projectPath;
    corpus.agents = [staleControlAgent];
    corpus.categories = [{ slug: "engineering", label: "Engineering", color: "#2563eb", icon: "Code", count: 1 }];
    const target = document.createElement("div");
    document.body.append(target);
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string, args) => {
      if (command === "installs_reconcile") return [...targetStates].map(([tool, state]) => ({
        ...staleControlRow,
        slug: "new-reviewer",
        name: "New Reviewer",
        sourceId: nextReference.sourceId,
        relativePath: nextReference.relativePath,
        tool,
        projectPath,
        dest: `/tmp/project/.agents/${tool}/new-reviewer.md`,
        scope: "project",
        state,
        tracked: true,
      })) as never;
      if (command === "skill_installs_reconcile" || command === "skill_backups_list") return [] as never;
      if (command === "projects_list") return projectRows as never;
      if (command === "tools_list") return install.tools as never;
      if (command === "project_instructions_inspect") return [] as never;
      if (command === "project_readiness_get") return readinessFixture(projectPath) as never;
      if (command === "project_recommendations_list") return (finalized ? [] : [liveRecommendation()]) as never;
      if (command === "project_recommendations_acknowledge") return true as never;
      if (command === "project_recommendation_open") return liveRecommendation() as never;
      if (command === "agent_sources_inspect") return [{
        source: { id: "built-in", label: "Built in", enabled: true, kind: { kind: "builtIn" } },
        agents: [newPackage], errors: [], revision: "fresh",
      }] as never;
      if (command === "agent_drafts_list") return [] as never;
      if (command === "agent_library_list") return emptyFolderState() as never;
      if (command === "agent_update_plan" || command === "agent_install_plan") {
        const operation = command === "agent_update_plan" ? "update" : "install";
        return planFor((args as { tool: string }).tool, operation) as never;
      }
      if (command === "update_agent") {
        expect((args as { tool: string }).tool).toBe("claudeCode");
        targetStates.set("claudeCode", "current");
        return {
          ...installRecord("new-reviewer", `/tmp/project/.agents/claudeCode/new-reviewer.md`),
          sourceId: nextReference.sourceId,
          relativePath: nextReference.relativePath,
          tool: "claudeCode",
          projectPath,
          scope: "project",
        } as never;
      }
      if (command === "agent_install_with_dependencies") {
        expect(invokeMock.mock.calls.some(([name]) => name === "project_recommendation_finalize")).toBe(false);
        const tool = (args as { tool: string }).tool;
        expect(tool).toBe("codex");
        const plan = planFor(tool, "install");
        targetStates.set(tool, "current");
        return [{
          ...installRecord("new-reviewer", plan.agents[0].destination),
          sourceId: nextReference.sourceId,
          relativePath: nextReference.relativePath,
          tool,
          projectPath,
          scope: "project",
        }] as never;
      }
      if (command === "project_recommendation_finalize") {
        expect([...targetStates].sort()).toEqual([
          ["claudeCode", "current"],
          ["codex", "current"],
        ]);
        expect(args).toEqual({ projectPath, recommendationId: recommendation.id });
        finalized = true;
        return {
          ...readinessFixture(projectPath).baseline,
          agentRequirements: [{ reference: nextReference, tool: "claudeCode" }],
          agents: [nextReference],
        } as never;
      }
      return [] as never;
    });
    const component = mount(Projects, { target });
    let firstMounted = true;
    let resumed: ReturnType<typeof mount> | null = null;
    let resumedTarget: HTMLDivElement | null = null;
    try {
      await vi.waitFor(() => expect(target.textContent).toContain(recommendation.summary));
      const open = [...target.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.trim() === "Open review")!;
      open.focus();
      open.click();
      await vi.waitFor(() => expect(invokeMock.mock.calls.some(([command]) => command === "agent_update_plan")).toBe(true));
      const apply = [...target.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.trim() === "Apply plan")!;
      apply.click();

      await vi.waitFor(() => expect(invokeMock.mock.calls.some(([command]) => command === "agent_install_plan")).toBe(true));
      expect(invokeMock.mock.calls.filter(([command]) =>
        command === "agent_update_plan")).toHaveLength(1);
      expect(invokeMock.mock.calls.filter(([command]) =>
        command === "agent_install_plan")).toHaveLength(1);
      expect(invokeMock.mock.calls.some(([command]) => command === "project_recommendation_finalize")).toBe(false);
      unmount(component);
      firstMounted = false;
      target.remove();

      resumedTarget = document.createElement("div");
      document.body.append(resumedTarget);
      resumed = mount(Projects, { target: resumedTarget });
      await vi.waitFor(() => expect(resumedTarget?.textContent).toContain(recommendation.summary));
      const resumeOpen = [...resumedTarget.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.trim() === "Open review")!;
      resumeOpen.click();
      await vi.waitFor(() => expect(invokeMock.mock.calls.filter(([command]) =>
        command === "agent_install_plan")).toHaveLength(2));
      expect(invokeMock.mock.calls.filter(([command]) =>
        command === "agent_update_plan")).toHaveLength(1);
      const secondApply = [...resumedTarget.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.trim() === "Apply plan")!;
      secondApply.click();

      await vi.waitFor(() => expect(invokeMock.mock.calls.some(([command]) =>
        command === "project_recommendation_finalize")).toBe(true));
      expect(invokeMock.mock.calls.filter(([command]) => command === "update_agent")).toHaveLength(1);
      expect(invokeMock.mock.calls.filter(([command]) =>
        command === "agent_install_with_dependencies")).toHaveLength(1);
      await vi.waitFor(() => expect(resumedTarget?.textContent).toContain("No catalog recommendations."));
      expect((document.activeElement as HTMLButtonElement).textContent?.trim()).toBe("Retry");
    } finally {
      if (firstMounted) unmount(component);
      if (resumed) unmount(resumed);
      target.remove();
      resumedTarget?.remove();
    }
  });

  it("keeps a failed finalize-only rename visible across remount and retries without reinstalling", async () => {
    const { default: Projects } = await import("$lib/components/Projects.svelte");
    const projectPath = "/tmp/project";
    const recommendation: ProjectRecommendation = {
      ...renamedProjectRecommendation(projectPath),
      lifecycle: "pending",
      targets: [],
      finalizeOnly: true,
    };
    const projectRows = [{ path: projectPath, label: "project", installedCount: 2 }];
    let finalizeAttempts = 0;
    let finalized = false;
    projects.list = projectRows;
    ui.projectsSelected = projectPath;
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string, args) => {
      if (command === "installs_reconcile" || command === "skill_installs_reconcile" || command === "skill_backups_list") return [] as never;
      if (command === "projects_list") return projectRows as never;
      if (command === "project_instructions_inspect") return [] as never;
      if (command === "project_readiness_get") return readinessFixture(projectPath) as never;
      if (command === "project_recommendations_list") return (finalized ? [] : [recommendation]) as never;
      if (command === "project_recommendation_finalize") {
        expect(args).toEqual({ projectPath, recommendationId: recommendation.id });
        finalizeAttempts += 1;
        if (finalizeAttempts === 1) throw new Error("finalize interrupted");
        finalized = true;
        return readinessFixture(projectPath).baseline as never;
      }
      return [] as never;
    });

    const firstTarget = document.createElement("div");
    document.body.append(firstTarget);
    const first = mount(Projects, { target: firstTarget });
    await vi.waitFor(() => expect(firstTarget.textContent).toContain(recommendation.summary));
    const firstFinish = [...firstTarget.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent?.trim() === "Finish rename")!;
    firstFinish.click();
    await vi.waitFor(() => expect(firstTarget.textContent).toContain("finalize interrupted"));
    expect(firstTarget.textContent).toContain(recommendation.summary);
    unmount(first);
    firstTarget.remove();

    const secondTarget = document.createElement("div");
    document.body.append(secondTarget);
    const second = mount(Projects, { target: secondTarget });
    try {
      await vi.waitFor(() => expect(secondTarget.textContent).toContain("Finish rename"));
      const retry = [...secondTarget.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.trim() === "Finish rename")!;
      retry.click();
      await vi.waitFor(() => expect(secondTarget.textContent).toContain("No catalog recommendations."));
      expect(finalizeAttempts).toBe(2);
      expect(invokeMock.mock.calls.some(([command]) => command === "project_recommendation_open")).toBe(false);
      expect(invokeMock.mock.calls.some(([command]) => command === "agent_install_plan")).toBe(false);
    } finally {
      unmount(second);
      secondTarget.remove();
    }
  });

  it("removes a pending recommendation after dismissal and keeps it gone on remount", async () => {
    const { default: Projects } = await import("$lib/components/Projects.svelte");
    const projectPath = "/tmp/project";
    const recommendation: ProjectRecommendation = {
      ...updatedProjectRecommendation(projectPath),
      lifecycle: "pending",
    };
    const projectRows = [{ path: projectPath, label: "project", installedCount: 1 }];
    let dismissed = false;
    projects.list = projectRows;
    ui.projectsSelected = projectPath;
    vi.mocked(invoke).mockImplementation(async (command: string, args) => {
      if (command === "installs_reconcile" || command === "skill_installs_reconcile" || command === "skill_backups_list") return [] as never;
      if (command === "projects_list") return projectRows as never;
      if (command === "project_instructions_inspect") return [] as never;
      if (command === "project_readiness_get") return readinessFixture(projectPath) as never;
      if (command === "project_recommendations_list") return (dismissed ? [] : [recommendation]) as never;
      if (command === "project_recommendation_dismiss") {
        expect(args).toEqual({ projectPath, recommendationId: recommendation.id });
        dismissed = true;
      }
      return [] as never;
    });

    const firstTarget = document.createElement("div");
    document.body.append(firstTarget);
    const first = mount(Projects, { target: firstTarget });
    await vi.waitFor(() => expect(firstTarget.textContent).toContain(recommendation.summary));
    const dismiss = [...firstTarget.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent?.trim() === "Dismiss")!;
    dismiss.click();
    await vi.waitFor(() => expect(firstTarget.textContent).toContain("No catalog recommendations."));
    unmount(first);
    firstTarget.remove();

    const secondTarget = document.createElement("div");
    document.body.append(secondTarget);
    const second = mount(Projects, { target: secondTarget });
    try {
      await vi.waitFor(() => expect(secondTarget.textContent).toContain("No catalog recommendations."));
      expect(secondTarget.textContent).not.toContain(recommendation.summary);
    } finally {
      unmount(second);
      secondTarget.remove();
    }
  });

  it("opens an Updated recommendation in the exact update review with no remove action", async () => {
    const { default: Projects } = await import("$lib/components/Projects.svelte");
    const projectPath = "/tmp/project";
    const recommendation = updatedProjectRecommendation(projectPath);
    const projectRows = [{ path: projectPath, label: "project", installedCount: 1 }];
    const pkg: AgentPackageResult = {
      ...staleControlPackage,
      reference: recommendation.agentReferences[0],
      agent: { ...staleControlAgent, slug: "reviewer", name: "Reviewer" },
    };
    const outdated = {
      ...staleControlRow,
      sourceId: recommendation.agentReferences[0].sourceId,
      relativePath: recommendation.agentReferences[0].relativePath,
      projectPath,
      scope: "project" as const,
      state: "outdated" as const,
      tracked: true,
    };
    const plan: AgentMutationPlan = {
      revision: "update-review", operation: "update", tool: "claudeCode", scope: "project", projectPath,
      agents: [{ reference: recommendation.agentReferences[0], name: "Reviewer", sourceHash: "hash", dependency: false, destination: outdated.dest, renderedFileCount: 1, capabilities: [] }],
      warnings: [], blockers: [], rollbackAvailable: true,
    };
    install.tools = [staleControlTool];
    projects.list = projectRows;
    ui.projectsSelected = projectPath;
    const target = document.createElement("div");
    document.body.append(target);
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string, args) => {
      if (command === "installs_reconcile") return [outdated] as never;
      if (command === "skill_installs_reconcile" || command === "skill_backups_list") return [] as never;
      if (command === "projects_list") return projectRows as never;
      if (command === "tools_list") return [staleControlTool] as never;
      if (command === "project_instructions_inspect") return [] as never;
      if (command === "project_readiness_get") return readinessFixture(projectPath) as never;
      if (command === "project_recommendations_list") return [recommendation] as never;
      if (command === "project_recommendations_acknowledge") return true as never;
      if (command === "project_recommendation_open") return recommendation as never;
      if (command === "agent_sources_inspect") return [{
        source: { id: "built-in", label: "Built in", enabled: true, kind: { kind: "builtIn" } },
        agents: [pkg], errors: [], revision: "fresh",
      }] as never;
      if (command === "agent_drafts_list") return [] as never;
      if (command === "agent_library_list") return emptyFolderState() as never;
      if (command === "agent_update_plan") {
        expect(args).toMatchObject({
          sourceId: recommendation.agentReferences[0].sourceId,
          relativePath: recommendation.agentReferences[0].relativePath,
          tool: "claudeCode",
          projectPath,
        });
        return plan as never;
      }
      return [] as never;
    });
    const component = mount(Projects, { target });
    try {
      await vi.waitFor(() => expect(target.textContent).toContain(recommendation.summary));
      const open = [...target.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.trim() === "Open review")!;
      open.focus();
      open.click();
      await vi.waitFor(() => expect(invokeMock.mock.calls.some(([command]) => command === "agent_update_plan")).toBe(true));
      expect(target.textContent).toContain("update");
      expect([...target.querySelectorAll<HTMLButtonElement>("button")]
        .some((button) => button.textContent?.trim() === "Uninstall")).toBe(false);
      target.querySelector<HTMLButtonElement>("button.close")!.click();
      await vi.waitFor(() => expect((document.activeElement as HTMLElement).dataset.recommendationId)
        .toBe(recommendation.id));
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("fails visibly when a cold refreshed Agent library lacks the backend recommendation ref", async () => {
    const { default: Projects } = await import("$lib/components/Projects.svelte");
    const projectPath = "/tmp/project";
    const recommendation = renamedProjectRecommendation(projectPath);
    const projectRows = [{ path: projectPath, label: "project", installedCount: 0 }];
    projects.list = projectRows;
    ui.projectsSelected = projectPath;
    const target = document.createElement("div");
    document.body.append(target);
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "installs_reconcile" || command === "skill_installs_reconcile" || command === "skill_backups_list") return [] as never;
      if (command === "projects_list") return projectRows as never;
      if (command === "project_instructions_inspect") return [] as never;
      if (command === "project_readiness_get") return readinessFixture(projectPath) as never;
      if (command === "project_recommendations_list") return [recommendation] as never;
      if (command === "project_recommendations_acknowledge") return true as never;
      if (command === "project_recommendation_open") return recommendation as never;
      if (command === "agent_sources_inspect" || command === "agent_drafts_list") return [] as never;
      if (command === "agent_library_list") return emptyFolderState() as never;
      return [] as never;
    });
    const component = mount(Projects, { target });
    try {
      await vi.waitFor(() => expect(target.textContent).toContain(recommendation.summary));
      [...target.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.trim() === "Open review")!.click();
      await vi.waitFor(() => expect(target.textContent).toContain("absent from the refreshed Agent library"));
      expect(target.textContent).not.toContain("Review catalog recommendation");
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("offers accessible category-specific repair links that reuse existing flows", async () => {
    const { default: Projects } = await import("$lib/components/Projects.svelte");
    const projectPath = "/tmp/project";
    const projectRows = [{ path: projectPath, label: "project", installedCount: 0 }];
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "installs_reconcile" || command === "skill_installs_reconcile" || command === "skill_backups_list") return [] as never;
      if (command === "projects_list") return projectRows as never;
      if (command === "project_instructions_inspect") return [] as never;
      if (command === "project_readiness_get") return readinessFixture(projectPath, false) as never;
      return [] as never;
    });
    projects.list = projectRows;
    ui.projectsSelected = projectPath;
    corpus.agents = [staleControlAgent];
    corpus.categories = [{ slug: "engineering", label: "Engineering", color: "#2563eb", icon: "Code", count: 1 }];
    const target = document.createElement("div");
    document.body.append(target);
    const component = mount(Projects, { target });
    const button = (label: string) => [...target.querySelectorAll<HTMLButtonElement>("button")]
      .find((candidate) => candidate.getAttribute("aria-label") === label)!;
    try {
      await vi.waitFor(() => expect(button("Review Agent readiness: Old reviewer")).toBeTruthy());
      const retry = [...target.querySelectorAll<HTMLButtonElement>("button")]
        .find((candidate) => candidate.textContent?.trim() === "Retry")!;
      expect(retry).toBeTruthy();
      expect(retry.disabled).toBe(false);
      expect(button("Review Skill readiness: Audit")).toBeTruthy();
      expect(button("Review project instructions: AGENTS.md")).toBeTruthy();
      expect(button("Open MCP settings for agency-agents")).toBeTruthy();
      expect(button("Open Tools for Claude Code")).toBeTruthy();

      button("Review project instructions: AGENTS.md").click();
      await vi.waitFor(() => expect(document.activeElement?.classList.contains("instruction-manager")).toBe(true));

      button("Open MCP settings for agency-agents").click();
      expect(ui.settingsOpen).toBe(true);
      expect(ui.settingsInitialSection).toBe("mcp");
      ui.closeSettings();

      button("Review Agent readiness: Old reviewer").click();
      expect(ui.agentsReference).toEqual(readinessFixture().baseline?.agents[0]);
      ui.selectProject(projectPath);
      await vi.waitFor(() => expect(button("Review Skill readiness: Audit")).toBeTruthy());
      button("Review Skill readiness: Audit").click();
      expect(ui.skillsSelected).toEqual(readinessFixture().baseline?.skills[0]);

      ui.selectProject(projectPath);
      await vi.waitFor(() => expect(button("Open Tools for Claude Code")).toBeTruthy());
      button("Open Tools for Claude Code").click();
      expect(ui.toolsSelected).toBe("claudeCode");
      expect(ui.section).toBe("tools");
    } finally {
      unmount(component);
      target.remove();
    }
  });

  it("binds project instruction inspection, revision apply, exact evidence, and redaction", async () => {
    const target: ProjectInstructionTarget = {
      id: "agents", label: "AGENTS.md", relativePath: "AGENTS.md",
      destination: "/tmp/project/AGENTS.md", state: "existingUnmanaged", exists: true,
      current: "existing\n", snippets: [], blockers: [],
    };
    const plan: ProjectInstructionPlan = {
      projectPath: "/tmp/project", target: "agents", label: "AGENTS.md",
      relativePath: "AGENTS.md", destination: "/tmp/project/AGENTS.md",
      operation: "upsert", snippetId: "review-rules", current: "existing\n",
      proposed: "existing\n\nmanaged\n", exists: true, adoption: true,
      backupRequired: true, noOp: false, warnings: ["adoption"], blockers: [], revision: "rev-1",
    };
    vi.mocked(invoke).mockImplementation(async (command: string, args) => {
      if (command === "project_instructions_inspect") return [target] as never;
      if (command === "project_instruction_plan") return plan as never;
      if (command === "project_instruction_apply") {
        expect(args as Record<string, unknown>).toMatchObject({ revision: "rev-1", confirmed: true });
        return {
          plan,
          result: {
            destination: "/tmp/project/AGENTS.md", outcome: "succeeded",
            backupPath: "/tmp/backups/AGENTS.md.bak",
            message: "token=should-not-survive",
          },
        } as never;
      }
      return [] as never;
    });
    const before = activity.entries.length;

    expect(await projects.inspectInstructions("/tmp/project")).toEqual([target]);
    expect(await projects.planInstruction("/tmp/project", "agents", "upsert", "review-rules", "Review.")).toEqual(plan);
    const applied = await projects.applyInstruction(plan, "Review.");

    expect(applied.result?.outcome).toBe("succeeded");
    expect(activity.entries).toHaveLength(before + 1);
    expect(activity.entries[0]).toMatchObject({
      action: "update", scope: "project", projectPath: "/tmp/project",
      subjectName: "AGENTS.md", detail: "/tmp/project/AGENTS.md · token=[redacted]",
    });
  });

  it("keeps the project instruction workflow in the existing Projects review surface", () => {
    for (const marker of [
      /projects\.inspectInstructions\(selected\.path\)/,
      /projects\.planInstruction\(/,
      /projects\.applyInstruction\(instructionPlan, instructionDraft\.content\)/,
      /diffLines\(instructionPlan\.current, instructionPlan\.proposed\)/,
      /Apply reviewed change/,
      /instructionPlan\.blockers\.length > 0 \|\| instructionPlan\.noOp/,
      /aria-live="polite"/,
      /restoreInstructionFocus/,
    ]) expect(projectsSource.match(marker) ?? [], String(marker)).toHaveLength(1);
    expect(projectsSource).not.toContain("window.fetch");
    expect(projectsSource).not.toContain("execute(");
  });
});

describe("bounded playbook library", () => {
  const entries = [
    { relativePath: "strategy/zeta.md", title: "Zeta", kind: "strategy" as const, sizeBytes: 12 },
    { relativePath: "examples/alpha.md", title: "Alpha workflow", kind: "example" as const, sizeBytes: 20 },
  ];

  it("filters locally in deterministic source-relative order", () => {
    expect(filterPlaybooks(entries, "workflow").map((entry) => entry.relativePath))
      .toEqual(["examples/alpha.md"]);
    expect(filterPlaybooks(entries, "strategy").map((entry) => entry.relativePath))
      .toEqual(["strategy/zeta.md"]);
    expect(filterPlaybooks(entries, "20")).toEqual([]);
    expect(filterPlaybooks(entries.toReversed(), "").map((entry) => entry.relativePath))
      .toEqual(["examples/alpha.md", "strategy/zeta.md"]);
  });

  it("retains the last successful rows when refresh fails and exposes error plus Retry", async () => {
    let listAttempts = 0;
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "playbooks_list") {
        listAttempts += 1;
        if (listAttempts === 2) throw { code: "io", message: "refresh unavailable" };
        return entries as never;
      }
      return [] as never;
    });

    const component = mount(RunbooksView, { target: document.body });
    await vi.waitFor(() => expect(runbooks.playbooks).toEqual(entries));
    const modes = [...document.querySelectorAll<HTMLButtonElement>("[data-runbooks-mode]")];
    expect(modes[0].parentElement?.getAttribute("role")).toBe("group");
    modes[1].click();
    await tick();
    expect(document.body.textContent).toContain("Alpha workflow");
    expect(document.body.textContent).toContain("Zeta");

    await runbooks.retryPlaybooks();
    await tick();
    expect(runbooks.playbooks).toEqual(entries);
    expect(document.querySelector('[role="status"]')?.textContent).toContain("refresh unavailable");
    expect(document.querySelector("[data-playbooks-retry]")).not.toBeNull();
    expect(document.body.textContent).not.toContain("No playbooks available");

    document.querySelector<HTMLButtonElement>("[data-playbooks-retry]")?.click();
    await vi.waitFor(() => expect(listAttempts).toBe(3));
    await tick();
    expect(document.body.textContent).toContain("Alpha workflow");
    expect(document.body.textContent).toContain("Zeta");
    unmount(component);
  });

  it("shows safe text, provenance, copy, empty/error retry, and accessible modes", async () => {
    const copy = vi.fn(async () => undefined);
    vi.stubGlobal("navigator", { ...navigator, clipboard: { writeText: copy } });
    let listAttempts = 0;
    vi.mocked(invoke).mockImplementation(async (command: string, args?: unknown) => {
      if (command === "playbooks_list") {
        listAttempts += 1;
        if (listAttempts === 1) throw { code: "io", message: "catalog unavailable" };
        return entries as never;
      }
      if (command === "playbook_read") {
        expect(args).toEqual({ relativePath: "examples/alpha.md" });
        return {
          ...entries[1],
          content: "# Alpha workflow\n<script>never execute</script>",
        } as never;
      }
      return [] as never;
    });

    const component = mount(RunbooksView, { target: document.body });
    await Promise.resolve();
    await tick();
    await Promise.resolve();
    await tick();

    const modes = [...document.querySelectorAll<HTMLButtonElement>("[data-runbooks-mode]")];
    expect(modes.map((button) => [button.textContent?.trim(), button.getAttribute("aria-pressed")]))
      .toEqual([["Runbooks", "true"], ["Playbooks", "false"]]);
    modes[1].click();
    await tick();
    expect(document.querySelector('[role="status"]')?.textContent).toContain("catalog unavailable");

    document.querySelector<HTMLButtonElement>("[data-playbooks-retry]")?.click();
    await Promise.resolve();
    await tick();
    expect(document.body.textContent).toContain("Alpha workflow");
    expect(document.body.textContent).toContain("examples/alpha.md");

    const search = document.querySelector<HTMLInputElement>("[data-playbooks-search]")!;
    search.value = "alpha";
    search.dispatchEvent(new Event("input", { bubbles: true }));
    await tick();
    expect(document.body.textContent).not.toContain("Zeta");

    document.querySelector<HTMLButtonElement>('[data-playbook-path="examples/alpha.md"]')?.click();
    await Promise.resolve();
    await tick();
    expect(document.querySelector('[data-playbook-path="examples/alpha.md"]')?.getAttribute("aria-current")).toBe("true");
    expect(document.querySelector("pre")?.getAttribute("tabindex")).toBe("0");
    expect(document.querySelector("pre")?.getAttribute("aria-label")).toBe("Alpha workflow source");
    expect(document.querySelector("pre")?.textContent).toContain("<script>never execute</script>");
    expect(document.querySelector("pre script")).toBeNull();

    document.querySelector<HTMLButtonElement>("[data-playbook-copy]")?.click();
    await Promise.resolve();
    expect(copy).toHaveBeenCalledWith("# Alpha workflow\n<script>never execute</script>");
    expect(document.querySelector('[aria-live="polite"]')).not.toBeNull();
    unmount(component);
  });
});

describe("MCP inventory settings", () => {
  it("renders bounded source, validation, tool, issue, and trusted-template evidence without foreign actions", async () => {
    settings.data = { ...SETTINGS_DEFAULTS, mcpClientPolicies: {} };
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "mcp_clients_status") {
        return [
          { client: "claude", installed: true, state: "exact", command: "", detail: "Connected" },
          { client: "codex", installed: true, state: "exact", command: "", detail: "Connected" },
        ] as never;
      }
      if (command === "mcp_inventory") {
        return {
          servers: [
            {
              client: "claude", name: "project-memory", scope: "project", projectPath: "/project",
              transport: "stdio", endpoint: "memory-server", enabled: true,
              environmentKeys: ["MCP_TOKEN"], toolNames: [], toolDiscovery: "unavailable",
              validation: "blocked", warnings: [], blockers: ["Unsupported transport"], trustedTemplate: false,
              rawConfig: "secret-value",
            },
            {
              client: "codex", name: "agency-agents", scope: "user", projectPath: null,
              transport: "stdio", endpoint: "agency-agents-app", enabled: true,
              environmentKeys: [], toolNames: ["agents_search", "skills_search"], toolDiscovery: "known",
              validation: "valid", warnings: [], blockers: [], trustedTemplate: true,
            },
            {
              client: "codex", name: "declared-tools", scope: "user", projectPath: null,
              transport: "stdio", endpoint: "local-server", enabled: true,
              environmentKeys: [], toolNames: ["declared_read"], toolDiscovery: "declared",
              validation: "valid", warnings: [], blockers: [], trustedTemplate: false,
            },
          ],
          trustedTemplates: [{
            id: "agency-agents", name: "Agency Agents", clients: ["claude", "codex"],
            toolNames: ["agents_search", "skills_search"], automaticConfiguration: true,
          }],
          issues: ["Codex inventory partially unavailable"],
        } as never;
      }
      return [] as never;
    });

    const component = mount(SettingsSectionMcp, { target: document.body });
    await Promise.resolve();
    await tick();
    await Promise.resolve();
    await tick();

    const text = document.body.textContent ?? "";
    for (const evidence of [
      "MCP inventory", "project-memory", "Project", "/project", "Blocked",
      "Tools unavailable", "Agency Agents", "agents_search", "skills_search",
      "declared-tools", "declared_read", "Codex inventory partially unavailable",
    ]) expect(text).toContain(evidence);
    expect(text).not.toContain("secret-value");
    expect(document.querySelector('[data-server="project-memory"] button')).toBeNull();
    expect(document.querySelector('[data-inventory-announcement]')?.textContent).toContain("issues");

    const refresh = document.querySelector<HTMLButtonElement>('[aria-label="Refresh MCP inventory"]');
    expect(refresh).not.toBeNull();
    refresh?.click();
    await Promise.resolve();
    await tick();
    expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "mcp_inventory")).toHaveLength(2);
    unmount(component);
    settings.data = null;
  });
});

describe("security posture presets", () => {
  it("classifies exact shapes, previews every policy field, and applies explicitly", async () => {
    settings.data = {
      ...SETTINGS_DEFAULTS,
      githubEnabled: true,
      updateAutoCheck: true,
      driftNotifications: true,
      mcpSourceAccess: true,
      mcpClientPolicies: {
        claude: {
          sourceAccess: true,
          installAccess: true,
          destructiveAccess: true,
          agentSourceAccess: true,
          agentInstallAccess: true,
          agentDestructiveAccess: true,
        },
      },
      mcpProjectAllowlist: ["/projects/retained"],
    };
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "mcp_clients_status") return [] as never;
      if (command === "mcp_inventory") return { servers: [], trustedTemplates: [], issues: [] } as never;
      if (command === "security_posture_apply") {
        return {
          ...settings.data,
          paranoidMode: true,
          githubEnabled: false,
          updateAutoCheck: false,
          driftNotifications: false,
          mcpSourceAccess: false,
          mcpInstallAccess: false,
          mcpDestructiveAccess: false,
          mcpAgentSourceAccess: false,
          mcpAgentInstallAccess: false,
          mcpAgentDestructiveAccess: false,
          mcpClientPolicies: {},
        } as never;
      }
      return [] as never;
    });

    const component = mount(SettingsSectionMcp, { target: document.body });
    await Promise.resolve();
    await tick();
    await Promise.resolve();
    await tick();

    expect(document.querySelector("[data-security-posture-current]")?.textContent).toContain("Custom");
    expect(document.querySelector(".preset-options")?.getAttribute("role")).toBe("group");
    document.querySelector<HTMLButtonElement>('[data-security-posture="strict"]')?.click();
    await tick();
    const preview = document.querySelector("[data-security-posture-preview]")?.textContent ?? "";
    for (const field of [
      "Offline mode", "GitHub access", "Automatic update checks", "Drift notifications",
      "Skill source mutations", "Skill install mutations", "Skill destructive mutations",
      "Agent source mutations", "Agent install mutations", "Agent destructive mutations",
      "Client overrides", "Claude Skill override", "Claude Agent override",
      "Codex Skill override", "Codex Agent override", "Project allowlist",
    ]) expect(preview).toContain(field);
    expect(preview).toContain("1 retained");
    const rows = Object.fromEntries(
      [...document.querySelectorAll<HTMLTableRowElement>("[data-security-posture-preview] tbody tr")]
        .map((row) => [...row.children].map((cell) => cell.textContent?.trim() ?? ""))
        .map(([label, before, after]) => [label, [before, after]]),
    );
    expect(rows["Offline mode"]).toEqual(["Off", "On"]);
    expect(rows["GitHub access"]).toEqual(["On", "Off"]);
    expect(rows["Client overrides"]).toEqual(["1 configured", "None"]);
    expect(rows["Claude Skill override"]).toEqual(["On / On / On", "Inherit"]);
    expect(rows["Claude Agent override"]).toEqual(["On / On / On", "Inherit"]);
    expect(rows["Codex Skill override"]).toEqual(["Inherit", "Inherit"]);
    expect(rows["Codex Agent override"]).toEqual(["Inherit", "Inherit"]);
    expect(rows["Project allowlist"]).toEqual(["1 retained", "1 retained"]);
    expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "security_posture_apply")).toBe(false);

    const apply = document.querySelector<HTMLButtonElement>("[data-security-posture-apply]")!;
    apply.click();
    await Promise.resolve();
    await tick();
    await Promise.resolve();
    await tick();
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("security_posture_apply", { preset: "strict" });
    expect(document.querySelector("[data-security-posture-current]")?.textContent).toContain("Strict");
    expect(document.querySelector('[data-security-posture-announcement]')?.textContent).toContain("Strict");
    await vi.waitFor(() => expect(document.activeElement).toBe(apply));
    unmount(component);
    settings.data = null;
  });

  it("announces a failed apply and restores focus to Apply without exposing a named posture", async () => {
    settings.data = {
      ...SETTINGS_DEFAULTS,
      githubEnabled: true,
      mcpSourceAccess: true,
      mcpClientPolicies: {},
    };
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "mcp_clients_status") return [] as never;
      if (command === "mcp_inventory") return { servers: [], trustedTemplates: [], issues: [] } as never;
      if (command === "security_posture_apply") throw { code: "io", message: "settings write failed" };
      return [] as never;
    });

    const component = mount(SettingsSectionMcp, { target: document.body });
    await Promise.resolve();
    await tick();
    await Promise.resolve();
    await tick();
    const apply = document.querySelector<HTMLButtonElement>("[data-security-posture-apply]")!;
    apply.click();
    await vi.waitFor(() => expect(settings.error).toContain("settings write failed"));
    await tick();
    const failureAnnouncement = document.querySelector('[data-security-posture-announcement]')?.textContent ?? "";
    expect(failureAnnouncement).toContain("Strict security posture failed");
    expect(failureAnnouncement).toContain("settings write failed");
    await vi.waitFor(() => expect(document.activeElement).toBe(apply));
    expect(document.querySelector("[data-security-posture-current]")?.textContent).toContain("Custom");
    unmount(component);
    settings.data = null;
  });
});
