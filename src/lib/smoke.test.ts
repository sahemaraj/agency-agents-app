import { createRawSnippet, mount, tick, unmount } from "svelte";
import { invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import Modal from "$lib/components/Modal.svelte";
import StorageMigrationGate from "$lib/components/StorageMigrationGate.svelte";
import agentsWorkspaceSource from "$lib/components/AgentsWorkspace.svelte?raw";
import skillsWorkspaceSource from "$lib/components/SkillsWorkspace.svelte?raw";
import installModalSource from "$lib/components/InstallModal.svelte?raw";
import deployBrowserSource from "$lib/components/DeployBrowser.svelte?raw";
import dashboardSource from "$lib/components/AgencyDashboard.svelte?raw";
import toolsViewSource from "$lib/components/ToolsView.svelte?raw";
import teamsSource from "$lib/components/Teams.svelte?raw";
import projectsSource from "$lib/components/Projects.svelte?raw";
import { mergeActivityEntries, safeActivityDetail, selectMcpAuditEntries } from "$lib/stores/activity.svelte";
import type { JournalEntry } from "$lib/stores/activity.svelte";
import { catalog } from "$lib/stores/catalog.svelte";
import { corpus } from "$lib/stores/corpus.svelte";
import { experts } from "$lib/stores/experts.svelte";
import { install } from "$lib/stores/install.svelte";
import { projects } from "$lib/stores/projects.svelte";
import { settings } from "$lib/stores/settings.svelte";
import { skillSources } from "$lib/stores/skillSources.svelte";
import { teams } from "$lib/stores/teams.svelte";
import { toast } from "$lib/stores/toast.svelte";
import { ui } from "$lib/stores/ui.svelte";
import type { Agent, AgentPackageResult, AgentSource, ExpertResolved, InstalledAgent, InstalledSkill, McpAuditEntry } from "$lib/types";

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
const staleControlPackage: AgentPackageResult = {
  reference: { sourceId: "built-in", relativePath: "reviewer.md" }, agent: staleControlAgent,
  sourceHash: "source", frontmatterHash: "frontmatter", bodyHash: "body", version: null,
  channel: null, changelog: null, publisher: null, publisherKey: null, publisherVerified: false,
  requiredAgents: [], requiredSkills: [], recommendedAgents: [], groups: [], tags: [], capabilities: [],
  permissions: [], qualityScore: 100, qualityChecks: [], diagnostics: [], installable: true,
};

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
  corpus.agents = [];
  corpus.categories = [];
  corpus.loading = false;
  corpus.error = null;
  experts.error = null;
  install.installed = [];
  install.tools = [];
  install.reconciling = false;
  install.reconciled = false;
  install.reconcileError = null;
  install.reconcileAttempt = 0;
  install.reconcileTerminal = 0;
  projects.list = [];
  settings.error = null;
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

  it("renders semantic catalog action failures in the existing toast", async () => {
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "catalog_source_get") return { kind: "bundled" } as never;
      if (command === "catalog_configured") return true as never;
      if (command === "catalog_status") return { isGit: false, repoSlug: null } as never;
      if (command === "catalog_detect") return { candidates: [] } as never;
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
      props: { open: true, onClose: vi.fn() },
    });
    try {
      await vi.waitFor(() => {
        expect(target.textContent).toContain("agent-draft-1");
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
      ["./components/InstallModal.svelte", [/async function reviewPlan[^]*?actionError = isAppError[^\n]*appErrorMessage/, /async function reviewCollection[^]*?actionError = isAppError[^\n]*appErrorMessage/, /async function applyPlan[^]*?actionError = isAppError[^\n]*appErrorMessage/, /async function runLifecycle[^]*?actionError = isAppError[^\n]*appErrorMessage/, /async function showHistory[^]*?actionError = isAppError[^\n]*appErrorMessage/, /async function rollback[^]*?actionError = isAppError[^\n]*appErrorMessage/]],
      ["./components/DiffModal.svelte", [/install\.diff[^]*?appErrorMessage/]],
      ["./components/AgencyDashboard.svelte", [/async function updateCatalog[^]*?appErrorMessage/]],
      ["./components/Projects.svelte", [/async function reveal[^]*?appErrorMessage/, /async function forgetProject[^]*?appErrorMessage/, /async function uninstallAndRemove[^]*?appErrorMessage/]],
      ["./stores/catalog.svelte.ts", [/catalog_status[^]*?this\.error = isAppError[^\n]*appErrorMessage/, /catalog_check_updates[^]*?this\.error = isAppError[^\n]*appErrorMessage/, /catalog_detect[^]*?this\.error = isAppError[^\n]*appErrorMessage/, /async setSource[^]*?this\.error = isAppError[^\n]*appErrorMessage/, /async provisionManaged[^]*?this\.error = isAppError[^\n]*appErrorMessage/, /catalog_pull[^]*?this\.error = isAppError[^\n]*appErrorMessage/]],
      ["./stores/settings.svelte.ts", [/corruptOnDisk = true[^]*?this\.error = appErrorMessage/, /else if \(isAppError\(e\)\)[^]*?this\.error = appErrorMessage/, /async save[^]*?this\.error = appErrorMessage/, /async reset[^]*?this\.error = appErrorMessage/]],
      ["./stores/experts.svelte.ts", [/expert_runs_list[^]*?this\.error = isAppError[^\n]*appErrorMessage/]],
      ["./stores/activity.svelte.ts", [/safeActivityDetail[^]*?isAppError\(value\) \? appErrorMessage\(value\)/]],
    ]);
    expect([...inventory.values()].flat()).toHaveLength(43);
    for (const [path, markers] of inventory) {
      const source = rel01Sources[path];
      expect(source, path).toBeTruthy();
      for (const marker of markers) expect(source.match(marker) ?? [], `${path}: ${marker}`).toHaveLength(1);
    }
    expect([...inventory.keys()].flatMap((path) => rel01Sources[path].match(/\bappErrorMessage\(/g) ?? []))
      .toHaveLength(44);

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
    expect(installSource.match(/safeActivityDetail\(/g) ?? []).toHaveLength(8); // seven live edges + retained legacy install()

    const rawFallbacks = Object.entries(rel01Sources).flatMap(([path, source]) =>
      [...source.matchAll(/^\s*toast\.error\([^\n]*,\s*String\((?:e|error)\)\);?$/gm)]
        .map((match) => `${path}:${match[0].trim()}`)).toSorted();
    expect(rawFallbacks).toEqual([
      './components/ToolsView.svelte:toast.error("Could not open the folder picker", String(e));',
      './components/Experts.svelte:toast.error("Copy failed", String(error));',
      './components/Runbooks.svelte:toast.error(i18n.t("common.copyFailed"), String(e));',
    ].toSorted());
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
        /disabled=\{!installTruthFresh \|\| busy \|\| managed\.length === 0\}/,
        /\{#if install\.reconcileError\}[^]*?retryReconcile\(event\)/,
      ] }],
      ["Projects", { source: projectsSource, markers: [
        /for \(const r of install\.installed\)/,
        /install\.reconciled \? i18n\.count\(rosterFor\(selected\.path\)\.length/,
        /const mutationTruthFresh = \$derived\(installTruthFresh && skillSources\.reconciled && !skillSources\.reconcileError\);/,
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
      ["SkillsWorkspace", { source: skillsWorkspaceSource, freshnessUses: 17, markers: [
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
      ["InstallModal", { source: installModalSource, freshnessUses: 17, markers: [
        /disabled: !installTruthFresh \|\| total === 0,/,
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
});
