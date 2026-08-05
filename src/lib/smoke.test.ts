import { createRawSnippet, mount, tick, unmount } from "svelte";
import { invoke } from "@tauri-apps/api/core";
import { describe, expect, it, vi } from "vitest";
import Modal from "$lib/components/Modal.svelte";
import { mergeActivityEntries, safeActivityDetail, selectMcpAuditEntries } from "$lib/stores/activity.svelte";
import type { JournalEntry } from "$lib/stores/activity.svelte";
import type { McpAuditEntry } from "$lib/types";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (command: string) => command === "skill_folders_list"
    ? {
        folders: [], assignments: [], favorites: [], recent: [], collections: [], smartFolders: [],
        profiles: [], updatePolicies: [], publisherTrust: [], preferredSources: [], usage: [], approvals: [],
      }
    : []),
}));

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
});
