/**
 * Activity store — a JOURNAL of discrete agent actions (install, remove,
 * update, track, tool default-target switch, sync, bulk ops).
 *
 * Drives the "Activity" view. This is NOT a live stream: each entry is a
 * single, already-resolved record appended after a backend action returns.
 * Local entries are clearable; durable MCP audit entries are not.
 *
 * Persistence: the journal is mirrored to localStorage so the Activity view
 * survives app restarts. The cap keeps the mirror bounded; older entries drop
 * off the tail. On any persist failure we console-warn (NOT a silent swallow)
 * so a regression — quota exhaustion, serialization failure, a webview-side
 * storage policy quirk — is visible during dev/testing instead of presenting
 * as "the activity log silently empties."
 */

import { mcpAuditList } from "$lib/api";
import { appErrorMessage, isAppError, type McpAuditEntry, type Tool } from "$lib/types";

/** Bumped v1 -> v2: the persisted shape changed from streaming jobs to journal
 *  entries. The old v1 store was never populated (no backend emitted stream
 *  events), so there's nothing to migrate. */
const STORAGE_KEY = "agency-agents:activity:v2";
/** Cap how many entries we persist. Older drop off the tail. */
const MAX_ENTRIES = 500;
/** How long to wait after a change before writing to localStorage. */
const PERSIST_DEBOUNCE_MS = 400;
const RECEIPT_NAME_MAX = 160;
const RECEIPT_DESTINATION_MAX = 4096;

export type ActivityReceiptOperation = "install" | "update" | "track" | "uninstall" | "repair";

export interface ActivityReceiptItem {
  kind: "agent" | "skill";
  name: string;
  destination: string | null;
  outcome: "ok" | "error";
  detail?: string;
}

export interface ActivityReceipt {
  operation: ActivityReceiptOperation;
  succeeded: number;
  failed: number;
  items: ActivityReceiptItem[];
}

export function safeActivityDetail(value: unknown): string {
  return (isAppError(value) ? appErrorMessage(value) : String(value))
    .replace(/-----BEGIN [^-\n]*PRIVATE KEY-----[\s\S]*?-----END [^-\n]*PRIVATE KEY-----/gi, "[redacted]")
    .replace(/:\/\/[^/\s:@]+:[^@\s/]+@/g, "://[redacted]@")
    .replace(/authorization:\s*bearer\s+\S+/gi, "Authorization: [redacted]")
    .replace(/\b(token|api[_-]?key|secret|password)\s*[:=]\s*\S+/gi, "$1=[redacted]")
    .replace(/\b(?:gh[pousr]_|sk-)[A-Za-z0-9_-]{8,}\b/g, "[redacted]")
    .replace(/private\s+key(?:\s+\S+)*/gi, "[redacted]")
    .replace(/[\u0000-\u001F\u007F]/g, " ")
    .slice(0, 512);
}

function boundedReceiptText(value: unknown, max: number): string {
  return typeof value === "string"
    ? value.replace(/[\u0000-\u001F\u007F]/g, " ").trim().slice(0, max)
    : "";
}

export function normalizeActivityReceipt(value: unknown): ActivityReceipt | undefined {
  if (!value || typeof value !== "object") return undefined;
  const candidate = value as Record<string, unknown>;
  const operations: ActivityReceiptOperation[] = ["install", "update", "track", "uninstall", "repair"];
  if (!operations.includes(candidate.operation as ActivityReceiptOperation) || !Array.isArray(candidate.items)) return undefined;
  const items: ActivityReceiptItem[] = candidate.items.flatMap((raw) => {
    if (!raw || typeof raw !== "object") return [];
    const item = raw as Record<string, unknown>;
    if (!(["agent", "skill"] as const).includes(item.kind as "agent" | "skill")) return [];
    if (!(["ok", "error"] as const).includes(item.outcome as "ok" | "error")) return [];
    const name = boundedReceiptText(item.name, RECEIPT_NAME_MAX);
    const destinationText = item.destination == null
      ? ""
      : boundedReceiptText(item.destination, RECEIPT_DESTINATION_MAX);
    const destination = destinationText || null;
    if (!name || (item.outcome === "ok" && !destination)) return [];
    const detail = item.detail != null
      ? safeActivityDetail(item.detail)
      : undefined;
    return [{
      kind: item.kind as "agent" | "skill",
      name,
      destination,
      outcome: item.outcome as "ok" | "error",
      ...(detail ? { detail } : {}),
    }];
  });
  if (items.length === 0) return undefined;
  return {
    operation: candidate.operation as ActivityReceiptOperation,
    succeeded: items.filter((item) => item.outcome === "ok").length,
    failed: items.filter((item) => item.outcome === "error").length,
    items,
  };
}

/** A discrete, already-resolved agent action recorded in the journal. */
export interface JournalEntry {
  /** Stable id (crypto.randomUUID). */
  id: string;
  /** ISO timestamp the action resolved. */
  ts: string;
  action:
    | "install"
    | "uninstall"
    | "update"
    | "disable"
    | "enable"
    | "sourceAdd"
    | "sourceRefresh"
    | "sourceRemove"
    | "draftCreate"
    | "draftEdit"
    | "draftPublish"
    | "draftReject"
    | "organize"
    | "rollback"
    | "approvalApprove"
    | "approvalReject"
    | "track"
    | "switch"
    | "sync"
    | "bulk"
    | "mcp";
  subject?: "agent" | "agentSource" | "agentDraft" | "agentLibrary" | "agentApproval" | "skill" | "skillSource" | "mcp";
  subjectName?: string;
  agentSlug?: string;
  agentName?: string;
  tool?: Tool;
  scope?: "user" | "project";
  projectPath?: string;
  outcome: "ok" | "error" | "pending";
  /** Free-form detail — error message, bulk summary ("3 agents"), etc. */
  detail?: string;
  receipt?: ActivityReceipt;
}

export function normalizePersistedActivityEntries(entries: unknown[]): JournalEntry[] {
  return entries.flatMap((value) => {
    if (!value || typeof value !== "object") return [];
    const { receipt: rawReceipt, ...entry } = value as JournalEntry & { receipt?: unknown };
    const receipt = normalizeActivityReceipt(rawReceipt);
    return [{ ...entry, ...(receipt ? { receipt } : {}) } as JournalEntry];
  });
}

export function mergeActivityEntries(
  localEntries: JournalEntry[],
  mcpEntries: JournalEntry[],
): JournalEntry[] {
  return [...localEntries, ...mcpEntries].sort(
    (left, right) => Date.parse(right.ts) - Date.parse(left.ts),
  );
}

interface PersistedShape {
  v: 2;
  entries: JournalEntry[];
}

export function selectMcpAuditEntries(entries: McpAuditEntry[]): McpAuditEntry[] {
  const selected = new Map<string, McpAuditEntry>();
  for (const entry of entries) {
    const current = selected.get(entry.id);
    if (
      !current
      || (current.phase === "attempt" && entry.phase === "terminal")
      || (
        current.phase === entry.phase
        && Date.parse(entry.timestamp) > Date.parse(current.timestamp)
      )
    ) {
      selected.set(entry.id, entry);
    }
  }
  return [...selected.values()].sort(
    (left, right) => Date.parse(right.timestamp) - Date.parse(left.timestamp),
  );
}

class ActivityStore {
  /** The journal, newest-first. */
  entries: JournalEntry[] = $state([]);
  hasLocalEntries: boolean = $state(false);

  private localEntries: JournalEntry[] = [];
  private mcpEntries: JournalEntry[] = [];
  private persistTimer: ReturnType<typeof setTimeout> | null = null;
  private hydrated = false;

  /**
   * Restore persisted entries from localStorage. Safe to call multiple times —
   * only the first call hydrates. Should be invoked once during app bootstrap
   * (e.g. from `+layout.svelte`).
   */
  hydrate(): void {
    if (this.hydrated || typeof window === "undefined") return;
    this.hydrated = true;
    try {
      const raw = localStorage.getItem(STORAGE_KEY);
      if (!raw) {
        console.info("[activity] hydrate: no persisted entry (first launch or storage cleared)");
      } else {
        const parsed = JSON.parse(raw) as PersistedShape;
        if (!parsed || parsed.v !== 2 || !Array.isArray(parsed.entries)) {
          console.warn("[activity] hydrate: persisted entry has unexpected shape; ignoring");
        } else {
          this.localEntries = normalizePersistedActivityEntries(parsed.entries).slice(0, MAX_ENTRIES);
          console.info(`[activity] hydrate: restored ${parsed.entries.length} entry(ies) from localStorage`);
        }
      }
    } catch (e) {
      console.warn(
        `[activity] hydrate failed (corrupt entry): ${
          e instanceof Error ? e.message : String(e)
        }`,
      );
      try { localStorage.removeItem(STORAGE_KEY); } catch { /* ignore */ }
    }
    this.mergeEntries();
    void this.refreshMcpAudit();
  }

  /**
   * Schedule a debounced write to localStorage. Coalesces rapid bursts (e.g. a
   * bulk loop that logs once per item) into a single write at most every
   * PERSIST_DEBOUNCE_MS milliseconds.
   */
  private schedulePersist(): void {
    if (typeof window === "undefined") return;
    if (this.persistTimer) clearTimeout(this.persistTimer);
    this.persistTimer = setTimeout(() => {
      this.persistTimer = null;
      this.persistNow();
    }, PERSIST_DEBOUNCE_MS);
  }

  /**
   * Write current entries to localStorage immediately. Caps entry count to keep
   * storage bounded. On failure, logs a warning to the console.
   */
  private persistNow(): void {
    if (typeof window === "undefined") return;
    try {
      const trimmed = this.localEntries.slice(0, MAX_ENTRIES);
      const payload: PersistedShape = { v: 2, entries: trimmed };
      localStorage.setItem(STORAGE_KEY, JSON.stringify(payload));
    } catch (e) {
      console.warn(
        `[activity] persistNow failed (entries=${this.entries.length}): ${
          e instanceof Error ? e.message : String(e)
        }`,
      );
    }
  }

  /**
   * Record a resolved action. Generates id + ts, prepends (newest-first), and
   * persists. Callers pass everything but `id`/`ts`.
   */
  log(entry: Omit<JournalEntry, "id" | "ts">): string {
    const { receipt: rawReceipt, ...base } = entry;
    const receipt = normalizeActivityReceipt(rawReceipt);
    const full: JournalEntry = {
      ...base,
      ...(receipt ? { receipt } : {}),
      id: crypto.randomUUID(),
      ts: new Date().toISOString(),
    };
    this.localEntries = [full, ...this.localEntries].slice(0, MAX_ENTRIES);
    this.mergeEntries();
    // Debounced so a bulk loop's per-item logs coalesce into one write.
    this.schedulePersist();
    return full.id;
  }

  /** Wipe the local journal and its mirror; durable MCP audit entries remain. */
  clear(): void {
    this.localEntries = [];
    this.mergeEntries();
    this.persistNow();
  }

  async refreshMcpAudit(): Promise<void> {
    try {
      this.mcpEntries = selectMcpAuditEntries(await mcpAuditList())
        .map((entry) => this.fromMcpAudit(entry));
      this.mergeEntries();
    } catch (error) {
      console.warn(
        `[activity] MCP audit load failed: ${error instanceof Error ? error.message : String(error)}`,
      );
    }
  }

  private fromMcpAudit(entry: McpAuditEntry): JournalEntry {
    return {
      id: `mcp:${entry.id}`,
      ts: entry.timestamp,
      action: "mcp",
      subject: "mcp",
      subjectName: entry.tool,
      projectPath: entry.projectPath ?? undefined,
      outcome: entry.phase === "attempt" ? "pending" : entry.success ? "ok" : "error",
      detail: `${entry.client ?? "unknown client"} · ${entry.action}`,
    };
  }

  private mergeEntries(): void {
    this.hasLocalEntries = this.localEntries.length > 0;
    this.entries = mergeActivityEntries(this.localEntries, this.mcpEntries);
  }
}

export const activity = new ActivityStore();
