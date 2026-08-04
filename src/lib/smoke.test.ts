import { createRawSnippet, mount, tick, unmount } from "svelte";
import { describe, expect, it, vi } from "vitest";
import Modal from "$lib/components/Modal.svelte";
import { mergeActivityEntries, safeActivityDetail, selectMcpAuditEntries } from "$lib/stores/activity.svelte";
import type { JournalEntry } from "$lib/stores/activity.svelte";
import type { McpAuditEntry } from "$lib/types";

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
});
