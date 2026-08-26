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

import { expertRunsList, mcpAuditList, projectsList } from "$lib/api";
import { toolMeta } from "$lib/data/toolRegistry";
import { projectFactoryRun } from "$lib/stores/experts.svelte";
import { appErrorMessage, isAppError, type ExpertRun, type McpAuditEntry, type Tool } from "$lib/types";

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
const FACTORY_REVISION_MAX = 256;
const FACTORY_LIMITATIONS_MAX = 64;

export type ActivityReceiptOperation = "install" | "update" | "track" | "uninstall" | "repair";

export interface ActivityReceiptItem {
  kind: "agent" | "skill";
  name: string;
  destination: string | null;
  outcome: "ok" | "error";
  detail?: string;
}

export interface ActivityMutationReceipt {
  operation: ActivityReceiptOperation;
  succeeded: number;
  failed: number;
  items: ActivityReceiptItem[];
}

export type FactoryReceiptOutcome = "accepted" | "rework" | "rejected" | "cancelled" | "attemptExhausted";

export interface FactoryReceiptCheck {
  name: string;
  result: "pass" | "fail" | "skipped" | "waived" | "missing";
}

export interface FactoryActivityReceipt {
  operation: "factory";
  /** Compatibility fields retained for existing generic receipt consumers. */
  succeeded: 0;
  failed: 0;
  items: [];
  runId: string;
  ticketReference: string;
  workTitle: string;
  projectLabel: string;
  outcome: FactoryReceiptOutcome;
  planRevision: string | null;
  baseCommit: string | null;
  headCommit: string | null;
  checks: FactoryReceiptCheck[];
  reviewStatus: "passed" | "waived" | "rework" | "failed" | "missing";
  deliveryReference: string | null;
  retryCount: number;
  limitations: string[];
  provenance: "clientReported";
  detail?: string;
}

export type ActivityReceipt = ActivityMutationReceipt | FactoryActivityReceipt;

export function safeActivityDetail(value: unknown, max = 512): string {
  const detail = (isAppError(value) ? appErrorMessage(value) : String(value))
    .replace(/-----BEGIN [^-\n]*PRIVATE KEY-----[\s\S]*?-----END [^-\n]*PRIVATE KEY-----/gi, "[redacted]")
    .replace(/:\/\/[^/\s:@]+:[^@\s/]+@/g, "://[redacted]@")
    .replace(/authorization:\s*bearer\s+\S+/gi, "Authorization: [redacted]")
    .replace(/https?:\/\/[^\s"\x60<>{}\[\]]+/gi, (candidate) => {
      try {
        return activityUrlHasCredential(new URL(candidate)) ? "[redacted]" : candidate;
      } catch {
        return candidate;
      }
    });
  return redactCredentialAssignments(detail)
    .replace(/\b(?:gh[pousr]_|sk-)[A-Za-z0-9_-]{8,}\b/g, "[redacted]")
    .replace(COMMON_BEARER_CANDIDATE, (candidate) =>
      commonBearerCredentialValue(candidate) ? "[redacted]" : candidate)
    .replace(COMPACT_CREDENTIAL_CANDIDATE, (candidate) =>
      compactCredentialValue(candidate) ? "[redacted]" : candidate)
    .replace(/private\s+key(?:\s+\S+)*/gi, "[redacted]")
    .replace(/[\u0000-\u001F\u007F]/g, " ")
    .slice(0, max);
}

function boundedReceiptText(value: unknown, max: number): string {
  return typeof value === "string"
    ? value.replace(/[\u0000-\u001F\u007F]/g, " ").trim().slice(0, max)
    : "";
}

function boundedReceiptDestination(value: unknown): string {
  const bounded = boundedReceiptText(value, RECEIPT_DESTINATION_MAX);
  if (!bounded) return "";
  return safeActivityDetail(bounded, RECEIPT_DESTINATION_MAX);
}

function boundedReceiptDetail(value: unknown): string {
  const redacted = safeActivityDetail(value);
  return factoryTextUnsafeReplacement(redacted) ?? redacted;
}

function factoryTextUnsafeReplacement(value: string): string | null {
  let inspected = value;
  for (let remaining = value.length + 1; remaining > 0; remaining -= 1) {
    inspected = inspected.replace(/\b([A-Za-z0-9._-]+)=\[redacted\]/gi,
      (match, key: string) => credentialParameterName(key) ? "redacted credential" : match);
    try {
      const parsed = JSON.parse(inspected);
      if (parsed !== null && typeof parsed === "object") return "[redacted unsafe metadata]";
    } catch {
      // Non-JSON prose continues through the structural checks below.
    }
    const collapsed = inspected.replace(/\s+/g, " ").trim();
    const sqlCandidate = collapsed.replace(/;+$/, "").trimEnd();
    // ponytail: these reviewed UI phrases are also valid SQL; keep this list exact and mirrored by backend tests.
    const knownSqlShapedProse = collapsed === "Select items from catalog"
      || collapsed === "Select items from catalog for review."
      || collapsed === "Delete from history";
    // ponytail: these reviewed UI sentences contain `=`; every other assignment-shaped line fails closed.
    const knownAssignmentShapedProse = collapsed === "Status = ready when all checks pass."
      || collapsed === "Result = output only after validation."
      || collapsed.split(/\s+/, 1).some((assignment) => {
        const [key, redacted] = assignment.split("=", 2);
        return credentialParameterName(key) && redacted === "[redacted]";
      });
    const knownTransactionShapedProse = /^(?:Begin|Commit|Rollback|Abort|End)$/.test(collapsed);
    const knownYamlShapedProse = inspected.trim() === "Database:\nHost details remain client-reported."
      || inspected.trim() === "Status: ready for desktop review."
      || inspected.trim() === "Status: ready"
      || inspected.trim() === "Client-reported: shown only as bounded metadata.";
    const yamlShaped = !knownYamlShapedProse && inspected.split(/\r?\n/).some((rawLine) => {
      const trimmed = rawLine.trim();
      const listItem = trimmed.startsWith("- ");
      const line = listItem ? trimmed.slice(2) : trimmed;
      const match = line.match(/^(?:"((?:\\.|[^"\\\n])*)"|'((?:''|[^'\n])*)'|([A-Za-z][A-Za-z0-9_.-]*))\s*:\s*(.*)$/);
      if (!match) return false;
      const [, doubleQuotedKey, singleQuotedKey, plainKey, value] = match;
      const key = doubleQuotedKey ?? singleQuotedKey ?? plainKey;
      const quotedKey = doubleQuotedKey !== undefined || singleQuotedKey !== undefined;
      const lowerKey = key.toLowerCase();
      if (["agents", "skills", "project"].includes(lowerKey) && value.toLowerCase() === "ready") return false;
      const scalar = /^(?:true|false|null|~|-?\d+(?:\.\d+)?|["'{\[])/i.test(value);
      const technicalKey = /^(?:database|datasource|host|hostname|server|port|url|uri|endpoint|schema|table|username|repository|registry|environment|config|configuration)$/i.test(key);
      const narrativeKey = /^(?:risk|result|note|status|owner|priority|severity)$/i.test(key);
      const lowercaseConfig = key === lowerKey
        && (/[_\-.]/.test(key) || value.length === 0 || !/\s/.test(value));
      const capitalBareScalar = /^[A-Z]/.test(key) && !narrativeKey && value.length > 0 && !/\s/.test(value);
      const blockScalar = /^[|>](?:(?:[+-][1-9]?)|(?:[1-9][+-]?)?)?(?:\s+#.*)?$/.test(value);
      return quotedKey || listItem || scalar || /^[&*!]/.test(value) || technicalKey
        || lowercaseConfig || capitalBareScalar || blockScalar;
    });
    if (/(?:^|[\s=:(\[<{])(?:\/(?!\/)|[A-Z]:[\\/]|\\{2})/i.test(inspected)) {
      return "[private path]";
    }
    if (/diff --git|raw output:|stdout:|stderr:|```/i.test(inspected)
      || /(^|\n)\s*(?:pub(?:\([^)]*\))?\s+|export\s+)?(?:async\s+)?(?:fn|function|def|class|struct|interface|enum|trait|impl|type)\b[^\n]*(?:[({;=]|=>)/i.test(inspected)
      || /(^|\n)\s*return\b[^\n]*;/i.test(inspected)
      || /(^|\n)\s*(?:if|elif|else|for|while|switch|match|try|catch|with|async\s+(?:with|for))\b[^\n]*(?:\(|\{|:)\s*/i.test(inspected)
      || /(^|\n)\s*import\s+\S[^\n]*(?:\n|$)/.test(inspected)
      || /(^|\n)\s*from\s+\S[^\n]*\s+import\s+\S[^\n]*(?:\n|$)/.test(inspected)
      || /(^|\n)\s*[A-Za-z_$][\w$.[\],: \t]*[+\-*/%]=\s*[^\n;]*;?\s*(?:\n|$)/i.test(inspected)
      || /(^|\n)\s*[A-Za-z_$][\w$.[\],: \t]*(?:\?\?|\|\||&&|<<|>>|\*\*|[|&^])=\s*[^\n;]*;?\s*(?:\n|$)/i.test(inspected)
      || /(^|\n)\s*[A-Za-z_$][\w$.[\],: \t]*(?:\+\+|--)\s*;?\s*(?:\n|$)/i.test(inspected)
      || /(^|\n)\s*(?=[^=\n]*[._[\],:$])[A-Za-z_$][\w$.[\],: \t]*\s*=\s*[^\n]+(?:;|$)/i.test(inspected)
      || /(^|\n)\s*[a-z_$][\w$]*\s*=\s*[^\n]+\s*(?:\n|$)/.test(inspected)
      || /(^|\n)\s*[A-Za-z_$][\w$]*\s*=\s*(?:await\s+)?new\s+\S[^\n]*(?:\n|$)/.test(inspected)
      || /(^|\n)\s*[A-Za-z_$][\w$]*\s*=\s*(?:(?:await\s+)?(?:new\s+)?[A-Za-z_$][\w$]*(?:[.:][A-Za-z_$][\w$]*)*\s*\(|[\[{])[^\n]*(?:\n|$)/.test(inspected)
      || /(^|\n)\s*[A-Za-z_$][\w$]*=[^\s\n]+\s*(?:\n|$)/.test(inspected)
      || (!knownAssignmentShapedProse && /(^|\n)\s*[A-Za-z_$][\w$]*\s*=(?!=|>)\s*(?![=>])\S[^\n]*(?:\n|$)/.test(inspected))
      || /(^|\n)\s*\([^=\n]+\)\s*=(?!=|>)\s*[^\n]+(?:\n|$)/.test(inspected)
      || /(^|\n)\s*\(\{[^}=\n]+\}\s*=(?!=|>)\s*[^\n]+\)\s*(?:\n|$)/.test(inspected)
      || /(^|\n)\s*[\[({][^\n]*=(?!=|>)[^\n]*(?:\n|$)/.test(inspected)
      || /(^|\n)\s*(?:#![^\n]*|<\?(?:php\b|=)[^\n]*|#\s*(?:(?:include|import)(?:\s*[<"][^>"\n]+[>"]|\s+[A-Za-z_$][\w$]*)|(?:define|pragma|if|ifdef|ifndef|elif|else|endif|undef|error|warning|line|nullable|region|endregion|r|load|checksum)\b[^\n]*)|#!?\[[^\]\n]+\]|(?:pub\s+)?mod\s+[A-Za-z_$][\w$]*\s*(?:;|\{)|macro_rules!\s*[A-Za-z_$][\w$]*\s*\{|@[A-Za-z_$][\w$]*(?:\.[A-Za-z_$][\w$]*)*(?:\([^\n)]*\))?|@(?:autoreleasepool|try|catch|finally|synchronized)\s*(?:\([^\n)]*\)\s*)?\{|@throw\s+[^;\n]+;|@(?:interface|implementation|protocol|class|property|synthesize|dynamic|compatibility_alias|end)\b[^\n]*|(?:global\s+)?using\s+[^;\n]+;|(?:pub(?:\([^)]*\))?\s+)?use\s+[^;\n]+;|(?:@|export\s+)import\s+[^;\n]+;)\s*(?:\n|$)/.test(inspected)
      || /(^|\n)\s*(?:await\s+)?[A-Za-z_$][\w$]*(?:[.:][A-Za-z_$][\w$]*)*\s*\([^;\n]*\)\s*(?:;|$)/i.test(inspected)
      || /(^|\n)\s*(?:await\s+)?[A-Za-z_$][\w$]*(?:[.:][A-Za-z_$][\w$]*)*\s*\([\s\S]*\)\s*;?\s*$/i.test(inspected)
      || (!/[.!?]$/.test(collapsed) && /^(?:(?:create|alter|drop)\s+\S+\s+\S+|comment\s+on\s+\S+\s+\S+)/i.test(sqlCandidate))
      || (collapsed.endsWith(";") && /^(?:select\b.*\bfrom\b|insert\b.*\binto\b|update\b.*\bset\b|delete\b.*\bfrom\b|(?:create|alter|drop)\s+\S+\s+\S+|comment\s+on\s+\S+\s+\S+|with\b.*\bselect\b)/i.test(collapsed))
      || (!knownSqlShapedProse && /^select\b[\s\S]+\bfrom\b[\s\S]*(?:,|\(|\b(?:union|intersect|except|fetch|for)\b)/i.test(sqlCandidate))
      || (!knownSqlShapedProse && /^select\s+(?:(?:distinct|all|distinctrow)\s+|top(?:\s*(?:\(\s*\d+\s*\)|\d+))?(?:\s+percent)?(?:\s+with\s+ties)?\s+)?(?:\*|[^\s,]+(?:\s+(?:as\s+)?[^\s,]+)?(?:\s*,\s*[^\s,]+(?:\s+(?:as\s+)?[^\s,]+)?)*)\s+from\s+(?:[A-Za-z_$][\w$]*(?:\.[A-Za-z_$][\w$])*|"[^"\n]+"|`[^`\n]+`|\[[^\]\n]+\])(?:\s+(?:as\s+)?(?:[A-Za-z_$][\w$]*|"[^"\n]+"|`[^`\n]+`|\[[^\]\n]+\]))?(?:\s+(?:where|join|left|right|inner|outer|group|order|limit|offset|having|union|intersect|except|fetch|for)\b[\s\S]*)?$/i.test(sqlCandidate))
      || /^select\s+(?:-?\d+(?:\.\d+)?|null|true|false|'(?:[^']|'')*'|"(?:[^"]|"")*"|[A-Za-z_$][\w$]*\s*\([^)]*\))$/i.test(sqlCandidate)
      || /^with\s+(?:recursive\s+)?[A-Za-z_$][\w$]*\s+as\s*\([\s\S]+\)\s+(?:select|insert\s+into|update|delete\s+from)\b[\s\S]*$/i.test(sqlCandidate)
      || /^explain\s+(?:(?:analy[sz]e|verbose|extended|partitions)\s+|query\s+plan\s+|format(?:\s*=\s*|\s+)\S+\s+|\([^)]*\)\s+)*(?:select|with|insert|update|delete|create|alter|drop|values)\b[\s\S]*$/i.test(sqlCandidate)
      || /^explain\s+plan\s+for\s+(?:select|with|insert|update|delete|create|alter|drop|values)\b[\s\S]*$/i.test(sqlCandidate)
      || /^grant\s+\S[\s\S]*\s+on\s+(?:(?:table|sequence|function|procedure|database|schema)\s+)?\S+\s+to\s+\S+[\s\S]*$/i.test(sqlCandidate)
      || /^grant\s+\S+(?:\s*,\s*\S+)*\s+to\s+\S+(?:\s*,\s*\S+)*(?:\s+with\s+grant\s+option)?$/i.test(sqlCandidate)
      || /^revoke\s+\S[\s\S]*\s+on\s+(?:(?:table|sequence|function|procedure|database|schema)\s+)?\S+\s+from\s+\S+[\s\S]*$/i.test(sqlCandidate)
      || /^revoke\s+\S+(?:\s*,\s*\S+)*\s+from\s+\S+(?:\s*,\s*\S+)*(?:\s+(?:cascade|restrict))?$/i.test(sqlCandidate)
      || /^show\s+(?:(?:tables|databases)(?:\s+(?:from|in|like)\s+\S+)?|(?:variables|status)(?:\s+like\s+\S+)?|(?:columns|fields)\s+(?:from|in)\s+\S+(?:\s+(?:from|in)\s+\S+)?|create\s+table\s+\S+)$/i.test(sqlCandidate)
      || /^(?:describe|desc)\s+[A-Za-z_$][\w$.]*(?:\s+[A-Za-z_$][\w$]*)?$/i.test(sqlCandidate)
      || /^merge\s+into\s+[A-Za-z_$][\w$.]*(?:\s+(?:as\s+)?[A-Za-z_$][\w$]*)?\s+using\s+[\s\S]+\s+on\s+[\s\S]+\s+when\s+[\s\S]+$/i.test(sqlCandidate)
      || /^replace\s+into\s+[A-Za-z_$][\w$.]*(?:\s*\([^)]*\))?\s*(?:values\s*\(|select\s+|default\s+values\s*$|set\s+[^=]+=)[\s\S]*$/i.test(sqlCandidate)
      || /^call\s+[A-Za-z_$][\w$.]*\s*\([^)]*\)$/i.test(sqlCandidate)
      || /^(?:exec|execute)\s+[A-Za-z_$][\w$.]*(?:\s+[\s\S]*)?$/i.test(sqlCandidate)
      || /^(?:vacuum|analy[sz]e)\s+[A-Za-z_$][\w$.]*$/i.test(sqlCandidate)
      || (collapsed.endsWith(";") && /^(?:vacuum|analy[sz]e)$/i.test(sqlCandidate))
      || /^truncate\s+(?:table\s+)?[A-Za-z_$][\w$.]*$/i.test(sqlCandidate)
      || /^copy\s+[\s\S]+\s+(?:to|from)\s+\S[\s\S]*$/i.test(sqlCandidate)
      || /^upsert\s+into\s+[A-Za-z_$][\w$.]*(?:\s*\([^)]*\))?[\s\S]+$/i.test(sqlCandidate)
      || /^create\s+(?:(?:or\s+replace)\s+)?(?:function|procedure|trigger)\s+[A-Za-z_$][\w$.]*(?:\s*\(|\s+(?:returns|language|before|after|instead|on|execute)\b)[\s\S]*$/i.test(sqlCandidate)
      || (collapsed.endsWith(";") && /^show\s+[A-Za-z_$][\w$.]*$/i.test(sqlCandidate))
      || /^pragma\s+[A-Za-z_$][\w$.]*(?:\s*=\s*[^\s]+|\s*\([^)]*\))?$/i.test(sqlCandidate)
      || /^values\s*\([\s\S]*\)$/i.test(sqlCandidate)
      || (!knownTransactionShapedProse && /^(?:begin(?:\s+(?:(?:deferred|immediate|exclusive)(?:\s+(?:transaction|work))?|transaction|work))?|start\s+transaction|(?:commit|end|abort)(?:\s+(?:transaction|work))?(?:\s+and\s+(?:no\s+)?chain)?|rollback(?:\s+(?:transaction|work))?(?:\s+and\s+(?:no\s+)?chain|\s+to(?:\s+savepoint)?\s+[A-Za-z_$][\w$]*)?|(?:release(?:\s+savepoint)?|savepoint)\s+[A-Za-z_$][\w$]*|set\s+(?:session\s+characteristics\s+as\s+)?transaction\s+(?:isolation\s+level\s+\S[\s\S]*|read\s+(?:only|write)|(?:not\s+)?deferrable))$/i.test(sqlCandidate))
      || /^insert\s+into\s+[A-Za-z_$][\w$.]*(?:\s*\([^)]*\))?\s*(?:values\s*\(|select\s+|default\s+values\s*$|set\s+[^=]+=)[\s\S]*$/i.test(sqlCandidate)
      || /^update\s+[A-Za-z_$][\w$.]*(?:\s+(?:as\s+)?[A-Za-z_$][\w$]*)?\s+set\s+[^=\n]+=[\s\S]+$/i.test(sqlCandidate)
      || (!knownSqlShapedProse && /^delete\s+from\s+[A-Za-z_$][\w$.]*(?:\s+(?:as\s+)?[A-Za-z_$][\w$]*)?(?:\s+(?:where|using|returning)\b[\s\S]*)?$/i.test(sqlCandidate))
      || /^(?:(?:create\s+(?:(?:temp(?:orary)?|unlogged|(?:global|local)\s+temporary)\s+)?table)|(?:create\s+(?:unique\s+)?index(?:\s+concurrently)?)|(?:create\s+(?:(?:or\s+replace)\s+)?(?:(?:materialized|temp(?:orary)?|recursive)\s+)*view)|(?:create\s+(?:(?:temp(?:orary)?|unlogged)\s+)?sequence)|(?:create\s+type)|(?:alter|drop)\s+(?:database|schema|sequence)|(?:create\s+(?:database|schema))|(?:alter|drop)\s+(?:materialized\s+)?view|(?:refresh\s+materialized\s+view(?:\s+concurrently)?)|(?:alter|drop|truncate)\s+table|(?:alter|drop)\s+index(?:\s+concurrently)?)\s+(?:(?:if\s+(?:not\s+)?exists)\s+)?[A-Za-z_$][\w$.]*(?:\s*(?:\(|(?:add|drop|rename|alter|as|on|using|with|like|enable|disable|owner|set|reset|validate|attach|detach|cluster|without|cascade|restrict)\b)[\s\S]*)?$/i.test(sqlCandidate)
      || /(^|\n)\s*\[[A-Za-z0-9_.-]+\]\s*(?:\n|$)/.test(inspected)
      || /(^|\n)\s*[A-Za-z][\w.-]*\s*:\s*(?:true|false|null|~|-?\d+(?:\.\d+)?|["'{\[])/i.test(inspected)
      || /(^|\n)\s*[A-Za-z][\w.-]*_[\w.-]*\s*:\s*\S[^\n]*$/i.test(inspected)
      || /(^|\n)\s*package\s+[A-Za-z_$][\w$]*(?:\s*\n|\s*$)/.test(inspected)
      || /(^|\n)\s*set\s+[-+]\S+(?:\s+[^\n]*)?(?:\n|$)/.test(inspected)
      || /(^|\n)\s*(?:export\s+)?(?:inline\s+)?namespace(?:\s+[A-Za-z_$][\w$]*(?:(?:::|\.)[A-Za-z_$][\w$]*)*)?\s*(?:[{;]|=\s*[A-Za-z_$][\w$]*(?:(?:::|\.)[A-Za-z_$][\w$]*)*\s*;)\s*(?:\n|$)/i.test(inspected)
      || /^[^{}\n]+\{\s*(?:--?[\w-]+|[A-Za-z_][\w-]*)\s*:\s*[^{};]+;?[\s\S]*\}\s*$/.test(collapsed)
      || /^@[A-Za-z-]+\b[^{}]*\{[\s\S]*\{[\s\S]*:[\s\S]*\}[\s\S]*\}\s*$/.test(collapsed)
      || yamlShaped
      || /^\s*<([A-Za-z][\w:.-]*)\b[^>]*>[\s\S]*<\/\1>\s*$/i.test(inspected)
      || /^\s*<[A-Za-z][\w:.-]*\b[^>]*\/>\s*$/i.test(inspected)
      || /^\s*<[\s\S]*>\s*$/.test(inspected)) return "[redacted unsafe metadata]";
    for (const match of inspected.matchAll(/https?:\/\/[^\s"`<>{}\[\]]+/gi)) {
      try {
        const url = new URL(match[0]);
        if (EMBEDDED_PRIVATE_PATH.test(url.pathname)) return "[private path]";
        if (url.username || url.password || activityUrlHasCredential(url)) {
          return "[redacted unsafe metadata]";
        }
      } catch {
        return "[redacted unsafe metadata]";
      }
    }
    const next = inspected.replace(/%([0-9A-F]{2})/gi, (_, hex: string) =>
      String.fromCharCode(Number.parseInt(hex, 16)));
    if (next === inspected) return null;
    inspected = next;
  }
  return "[redacted unsafe metadata]";
}

function boundedFactoryText(value: unknown, max = 512): string {
  const unsafeReplacement = factoryTextUnsafeReplacement(String(value));
  if (unsafeReplacement) return unsafeReplacement;
  return safeActivityDetail(value)
    .replace(/file:\/{3}[^\s"'()<>{}\[\]),;]+/gi, "[private path]")
    .replace(/\b[A-Z]:[\\/][^\s"'()<>{}\[\]),;]+/gi, "[private path]")
    .replace(/\\{2}[^\\/\s]+[\\/][^\s"'()<>{}\[\]),;]+/g, "[private path]")
    .replace(/:(\/(?!\/)[^\s"'()<>{}\[\]),;]+)/g, ":[private path]")
    .replace(/(^|[^A-Za-z0-9:/])\/(?!\/)[^\s"'()<>{}\[\]),;]+/g, "$1[private path]")
    .slice(0, max);
}

const CREDENTIAL_PARAMETER = /(?:^|[_-])(?:access[_-]?token|refresh[_-]?token|id[_-]?token|jwt|token|api[_-]?key|apikey|client[_-]?secret|private[_-]?key|secret|password|passwd|pwd|credentials?|auth|authorization|signature|sig)(?:$|[_-])/i;
const CREDENTIAL_VALUE = /(?:\b(?:gh[pousr]_|sk-)[A-Za-z0-9_-]{8,}\b|-----BEGIN [^-\n]*PRIVATE KEY-----)/i;
const COMMON_BEARER_CANDIDATE = /[A-Za-z0-9_.-]{19,}/g;
const COMPACT_CREDENTIAL_CANDIDATE = /eyJ[A-Za-z0-9_-]*(?:\.[A-Za-z0-9_-]*){2,4}/g;
const CREDENTIAL_PARAMETER_NAMES = new Set([
  "accesstoken", "refreshtoken", "idtoken", "jwt", "token", "apikey", "clientsecret", "sig", "signature",
  "secret", "privatekey", "password", "passwd", "pwd", "credential", "credentials", "auth", "authorization", "signature", "xamzsignature",
  "session", "sid", "sessionid", "sessionkey", "sessiontoken", "sessionsecret", "sessioncookie", "phpsessid", "jsessionid", "aspnetsessionid", "connectsid",
]);
const PRIVATE_PATH_VALUE = /^(?:\/(?!\/)|[A-Z]:[\\/]|\\{2})/i;
const EMBEDDED_PRIVATE_PATH = /(?:^|\/)\/(?:Users|home|opt|srv|tmp|etc)\//i;

function credentialParameterName(value: string): boolean {
  const normalized = value.toLowerCase().replace(/[^a-z0-9]/g, "");
  return CREDENTIAL_PARAMETER.test(value)
    || CREDENTIAL_PARAMETER_NAMES.has(normalized)
    || ["accesstoken", "refreshtoken", "idtoken", "jwt", "token", "apikey", "accesskey", "secretkey", "privatekey", "clientsecret", "password", "passwd", "pwd", "credential", "credentials", "auth", "authorization", "signature", "sig", "sessionid", "sessionkey", "sessiontoken", "sessionsecret", "sessioncookie"]
      .some((suffix) => normalized.length > suffix.length && normalized.endsWith(suffix));
}

function compactCredentialValue(value: string): boolean {
  const parts = value.split(".");
  const encoded = (part: string) => /^[A-Za-z0-9_-]*$/.test(part);
  return parts[0]?.startsWith("eyJ") === true && (
    (parts.length === 3 && parts[1].length > 0 && parts.every(encoded))
    || (parts.length === 5 && parts[1] !== undefined && encoded(parts[1])
      && [parts[0], parts[2], parts[3], parts[4]]
        .every((part) => part !== undefined && part.length > 0 && encoded(part)))
  );
}

function commonBearerCredentialValue(value: string): boolean {
  const prefixes = [
    "xoxb-", "xoxp-", "xoxa-", "xoxr-", "xoxs-", "xapp-", "xwfp-", "glpat-",
    "glsoat-", "glffct-", "gloas-", "gldt-", "glrt-", "glcbt-", "glptt-", "glft-",
    "glimt-", "glagent-", "glwt-", "hf_", "npm_", "dckr_pat_", "pypi-", "sk_live_",
    "sk_test_", "rk_live_", "rk_test_", "whsec_",
    "dop_v1_", "AIzaSy", "ya29.",
  ];
  return prefixes.some((prefix) => {
    if (!value.startsWith(prefix)) return false;
    const tail = value.slice(prefix.length);
    return tail.length >= 16 && /^[A-Za-z0-9_-]+$/.test(tail);
  }) || (() => {
    const multipart = value.split(".");
    if (multipart.length === 3 && /^[A-Z]{2,8}$/.test(multipart[0] ?? "")
      && (multipart[1]?.length ?? 0) >= 16 && (multipart[2]?.length ?? 0) >= 16
      && multipart.slice(1).every((part) => /^[A-Za-z0-9_-]+$/.test(part))) return true;
    const match = value.match(/^(.+)[_-]([A-Za-z0-9_-]{16,})$/);
    if (!match) return false;
    const prefix = match[1].toLowerCase().replace(/[_-]/g, "");
    return prefix.includes("api") || prefix.includes("pat");
  })();
}

function credentialBearingHttpsPath(url: URL): boolean {
  const host = url.hostname.toLowerCase();
  const decodedPath = decodeReceiptComponent(url.pathname);
  if (decodedPath === null) return true;
  const segments = decodedPath.split("/").filter(Boolean);
  const normalizedSegments = segments.map((segment) => segment.toLowerCase());
  const discordWebhook = (segments[0] === "api" && segments[1] === "webhooks"
      && segments.length >= 4)
    || (segments[0] === "api" && /^v\d+$/.test(segments[1] ?? "")
      && segments[2] === "webhooks" && segments.length >= 5);
  const telegramBot = host === "api.telegram.org"
    && ((segments[0]?.startsWith("bot") && segments[0].length > 3)
      || (segments[0] === "file" && segments[1]?.startsWith("bot")
        && segments[1].length > 3));
  const webhookMarkers = new Set(["hook", "hooks", "webhook", "webhooks", "webhookb2", "incomingwebhook"]);
  const opaqueTail = (tail: string[]) => tail.length > 0
    && tail.every((segment) => /^[A-Za-z0-9_.@~!$&'()*+,;=:\-]+$/.test(segment))
    && tail.reduce((length, segment) => length + segment.length, 0) >= 16;
  const genericWebhook = normalizedSegments.some((segment, index) =>
    webhookMarkers.has(segment) && opaqueTail(normalizedSegments.slice(index + 1)))
    || (host.split(".").some((label) => label.includes("hook") || label.includes("webhook"))
      && opaqueTail(normalizedSegments));
  return ((host === "hooks.slack.com" || host === "hooks.slack-gov.com")
      && segments[0] === "services" && segments.length >= 4)
    || ((host === "discord.com" || host === "discordapp.com") && discordWebhook)
    || telegramBot
    || genericWebhook;
}

function credentialValue(value: string): boolean {
  return CREDENTIAL_VALUE.test(value)
    || (value.match(COMMON_BEARER_CANDIDATE) ?? []).some(commonBearerCredentialValue)
    || (value.match(COMPACT_CREDENTIAL_CANDIDATE) ?? []).some(compactCredentialValue);
}

function containsCredentialAssignment(value: string): boolean {
  return [...value.matchAll(/\b([A-Za-z0-9._-]+)\s*[:=]\s*(\S+)/gi)]
    .some(([, key, assigned]) => credentialParameterName(key) || credentialValue(assigned));
}

function redactCredentialAssignments(value: string): string {
  return value.replace(/\b([A-Za-z0-9._-]+)\s*[:=]\s*(\S+)/gi,
    (match, key: string, assigned: string) => {
      if (credentialParameterName(key) || credentialValue(assigned)) return `${key}=[redacted]`;
      const nested = redactCredentialAssignments(assigned);
      return nested === assigned ? match : match.replace(assigned, nested);
    });
}

function decodeReceiptComponent(value: string): string | null {
  let decoded = value;
  try {
    for (let remaining = value.length + 1; remaining > 0; remaining -= 1) {
      const next = decodeURIComponent(decoded);
      if (next === decoded) return decoded;
      decoded = next;
    }
  } catch {
    return null;
  }
  return null;
}

function urlComponentHasCredential(value: string): boolean {
  const decoded = decodeReceiptComponent(value);
  if (decoded === null) return true;
  return decoded.split(/[\s/\\:?#&=]+/).some(credentialParameterName);
}

function activityUrlHasCredential(url: URL): boolean {
  if (url.username || url.password || credentialBearingHttpsPath(url)
    || urlComponentHasCredential(url.pathname)) return true;
  const pathname = decodeReceiptComponent(url.pathname);
  if (pathname === null || credentialValue(pathname)
    || containsCredentialAssignment(pathname)) return true;
  for (const [name, value] of url.searchParams) {
    const decodedName = decodeReceiptComponent(name);
    const decodedValue = decodeReceiptComponent(value);
    if (decodedName === null || decodedValue === null
      || credentialParameterName(decodedName)
      || urlComponentHasCredential(decodedName)
      || credentialValue(decodedValue)
      || containsCredentialAssignment(`${decodedName}:${decodedValue}`)) return true;
  }
  const fragment = decodeReceiptComponent(url.hash.slice(1));
  if (fragment === null) return true;
  if (credentialValue(fragment)
    || urlComponentHasCredential(fragment)
    || containsCredentialAssignment(fragment)) return true;
  for (const [name, value] of new URLSearchParams(fragment)) {
    const decodedName = decodeReceiptComponent(name);
    const decodedValue = decodeReceiptComponent(value);
    if (decodedName === null || decodedValue === null
      || credentialParameterName(decodedName)
      || urlComponentHasCredential(decodedName)
      || credentialValue(decodedValue)) return true;
  }
  return false;
}

function safeHttpsReference(value: unknown): string | null {
  const bounded = boundedReceiptText(value, RECEIPT_DESTINATION_MAX);
  if (!bounded) return null;
  try {
    const url = new URL(bounded);
    if (url.protocol !== "https:" || url.username || url.password) return null;
    if (activityUrlHasCredential(url)) return null;
    const pathname = decodeReceiptComponent(url.pathname);
    if (pathname === null) return null;
    if (credentialBearingHttpsPath(url)
      || credentialValue(pathname)
      || containsCredentialAssignment(pathname)
      || EMBEDDED_PRIVATE_PATH.test(pathname)
      || /[A-Z]:[\\/]/i.test(pathname)) return null;
    for (const [name, parameterValue] of url.searchParams) {
      const decodedName = decodeReceiptComponent(name);
      const decodedValue = decodeReceiptComponent(parameterValue);
      if (decodedName === null || decodedValue === null
        || credentialParameterName(decodedName)
        || credentialValue(decodedValue)
        || containsCredentialAssignment(`${decodedName}:${decodedValue}`)
        || PRIVATE_PATH_VALUE.test(decodedValue)) return null;
    }
    const fragment = decodeReceiptComponent(url.hash.slice(1));
    if (fragment === null) return null;
    if (credentialValue(fragment)
      || containsCredentialAssignment(fragment)
      || PRIVATE_PATH_VALUE.test(fragment)) return null;
    for (const [name, parameterValue] of new URLSearchParams(fragment)) {
      const decodedName = decodeReceiptComponent(name);
      const decodedValue = decodeReceiptComponent(parameterValue);
      if (decodedName === null || decodedValue === null
        || credentialParameterName(decodedName)
        || credentialValue(decodedValue)
        || PRIVATE_PATH_VALUE.test(decodedValue)) return null;
    }
    return url.toString();
  } catch {
    return null;
  }
}

function normalizeFactoryReceipt(candidate: Record<string, unknown>): FactoryActivityReceipt | undefined {
  const outcomes: FactoryReceiptOutcome[] = ["accepted", "rework", "rejected", "cancelled", "attemptExhausted"];
  const reviewStatuses: FactoryActivityReceipt["reviewStatus"][] = ["passed", "waived", "rework", "failed", "missing"];
  const runId = boundedFactoryText(candidate.runId, RECEIPT_NAME_MAX);
  const ticketReference = boundedFactoryText(candidate.ticketReference, RECEIPT_NAME_MAX);
  const workTitle = boundedFactoryText(candidate.workTitle, RECEIPT_NAME_MAX);
  const projectLabel = boundedFactoryText(candidate.projectLabel, RECEIPT_NAME_MAX);
  if (!runId || !ticketReference || !workTitle || !projectLabel
    || !outcomes.includes(candidate.outcome as FactoryReceiptOutcome)
    || !reviewStatuses.includes(candidate.reviewStatus as FactoryActivityReceipt["reviewStatus"])
    || candidate.provenance !== "clientReported"
    || !Array.isArray(candidate.checks)
    || !Array.isArray(candidate.limitations)) return undefined;

  const checks = candidate.checks.slice(0, 64).flatMap((raw): FactoryReceiptCheck[] => {
    if (!raw || typeof raw !== "object") return [];
    const check = raw as Record<string, unknown>;
    const name = boundedFactoryText(check.name, RECEIPT_NAME_MAX);
    const results: FactoryReceiptCheck["result"][] = ["pass", "fail", "skipped", "waived", "missing"];
    return name && results.includes(check.result as FactoryReceiptCheck["result"])
      ? [{ name, result: check.result as FactoryReceiptCheck["result"] }]
      : [];
  });
  const limitations = candidate.limitations.slice(0, FACTORY_LIMITATIONS_MAX)
    .map((item) => boundedFactoryText(item))
    .filter(Boolean);
  const optionalRevision = (value: unknown): string | null => {
    const bounded = value == null ? "" : boundedFactoryText(value, FACTORY_REVISION_MAX);
    return bounded || null;
  };
  const retryCount = typeof candidate.retryCount === "number" && Number.isInteger(candidate.retryCount)
    ? Math.min(Math.max(candidate.retryCount, 0), 3)
    : 0;
  const detail = candidate.detail == null ? "" : boundedFactoryText(candidate.detail);
  return {
    operation: "factory",
    succeeded: 0,
    failed: 0,
    items: [],
    runId,
    ticketReference,
    workTitle,
    projectLabel,
    outcome: candidate.outcome as FactoryReceiptOutcome,
    planRevision: optionalRevision(candidate.planRevision),
    baseCommit: optionalRevision(candidate.baseCommit),
    headCommit: optionalRevision(candidate.headCommit),
    checks,
    reviewStatus: candidate.reviewStatus as FactoryActivityReceipt["reviewStatus"],
    deliveryReference: safeHttpsReference(candidate.deliveryReference),
    retryCount,
    limitations,
    provenance: "clientReported",
    ...(detail ? { detail } : {}),
  };
}

export function isFactoryActivityReceipt(receipt: ActivityReceipt): receipt is FactoryActivityReceipt {
  return receipt.operation === "factory";
}

export function factoryReceiptFromRun(run: ExpertRun, projectLabel: string): FactoryActivityReceipt | undefined {
  const projection = projectFactoryRun(run);
  const workflow = projection?.workflow;
  if (!projection || !workflow?.terminal) return undefined;
  const evidence = new Map(projection.latestEvidence.map((item) => [item.checkName, item]));
  const checks: FactoryReceiptCheck[] = workflow.workContract.qualityContract.checks
    .filter((check) => check.required)
    .map((check) => {
      const waived = workflow.humanWaivers.some((waiver) =>
        waiver.kind === "qualityCheck" && waiver.checkName === check.name);
      return {
        name: check.name,
        result: waived ? "waived" : evidence.get(check.name)?.result ?? "missing",
      };
    });
  const reviewStatus: FactoryActivityReceipt["reviewStatus"] = workflow.review?.verdict === "pass"
    ? "passed"
    : workflow.review?.verdict === "rework"
      ? "rework"
      : workflow.humanWaivers.some((waiver) => waiver.kind === "independentReview")
        ? "waived"
        : "missing";
  const limitations = [...new Set([
    ...(workflow.plan?.knownLimitations ?? []),
    ...(workflow.delivery?.knownLimitations ?? []),
  ])];
  const raw = normalizeActivityReceipt({
    operation: "factory",
    runId: run.id,
    ticketReference: workflow.workContract.ticketReference,
    workTitle: workflow.workContract.title,
    projectLabel,
    outcome: workflow.terminal.outcome,
    planRevision: workflow.planApproval?.planRevision ?? null,
    baseCommit: workflow.planApproval?.baseCommit ?? null,
    headCommit: projection.headCommit,
    checks,
    reviewStatus,
    deliveryReference: workflow.delivery?.reference ?? null,
    retryCount: Math.max(0, projection.attempt - 1),
    limitations,
    provenance: "clientReported",
    detail: workflow.terminal.outcome === "cancelled"
      ? `Agency Agents revoked control-plane authority; external work was not stopped or deleted.${workflow.terminal.safeDetail ? ` ${workflow.terminal.safeDetail}` : ""}`
      : (workflow.terminal.safeDetail ?? undefined),
  });
  return raw && isFactoryActivityReceipt(raw) ? raw : undefined;
}

export function normalizeActivityReceipt(value: unknown): ActivityReceipt | undefined {
  if (!value || typeof value !== "object") return undefined;
  const candidate = value as Record<string, unknown>;
  if (candidate.operation === "factory") return normalizeFactoryReceipt(candidate);
  const operations: ActivityReceiptOperation[] = ["install", "update", "track", "uninstall", "repair"];
  if (!operations.includes(candidate.operation as ActivityReceiptOperation) || !Array.isArray(candidate.items)) return undefined;
  const items: ActivityReceiptItem[] = candidate.items.slice(0, MAX_ENTRIES).flatMap((raw) => {
    if (!raw || typeof raw !== "object") return [];
    const item = raw as Record<string, unknown>;
    if (!(["agent", "skill"] as const).includes(item.kind as "agent" | "skill")) return [];
    if (!(["ok", "error"] as const).includes(item.outcome as "ok" | "error")) return [];
    const name = boundedFactoryText(item.name, RECEIPT_NAME_MAX);
    const destinationText = item.destination == null
      ? ""
      : boundedReceiptDestination(item.destination);
    const destination = destinationText || null;
    if (!name || (item.outcome === "ok" && !destination)) return [];
    const detail = item.detail != null
      ? boundedReceiptDetail(item.detail)
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
    | "factory"
    | "mcp";
  subject?: "agent" | "agentSource" | "agentDraft" | "agentLibrary" | "agentApproval" | "skill" | "skillSource" | "factory" | "mcp";
  subjectName?: string;
  agentSlug?: string;
  agentName?: string;
  tool?: Tool;
  scope?: "user" | "project";
  projectPath?: string;
  projectLabel?: string;
  outcome: "ok" | "error" | "pending";
  /** Free-form detail — error message, bulk summary ("3 agents"), etc. */
  detail?: string;
  receipt?: ActivityReceipt;
}

const ACTIVITY_ACTIONS: JournalEntry["action"][] = [
  "install", "uninstall", "update", "disable", "enable", "sourceAdd", "sourceRefresh",
  "sourceRemove", "draftCreate", "draftEdit", "draftPublish", "draftReject", "organize",
  "rollback", "approvalApprove", "approvalReject", "track", "switch", "sync", "bulk",
  "factory", "mcp",
];
const ACTIVITY_SUBJECTS: NonNullable<JournalEntry["subject"]>[] = [
  "agent", "agentSource", "agentDraft", "agentLibrary", "agentApproval", "skill",
  "skillSource", "factory", "mcp",
];
const ACTIVITY_OUTCOMES: JournalEntry["outcome"][] = ["ok", "error", "pending"];

function safeActivityIdentity(value: unknown): string | undefined {
  const bounded = boundedReceiptText(value, RECEIPT_NAME_MAX);
  if (!bounded || !/^[A-Za-z0-9:_-]+$/.test(bounded) || credentialValue(bounded)) return undefined;
  const [prefix, assigned] = bounded.split(":", 2);
  return assigned && credentialParameterName(prefix) ? undefined : bounded;
}

function safeActivityProjectLabel(label: unknown, legacyPath: unknown): string | undefined {
  const candidate = typeof label === "string"
    ? label
    : typeof legacyPath === "string"
      ? legacyPath.replace(/[\\/]+$/, "").split(/[\\/]/).pop()
      : undefined;
  return candidate ? boundedFactoryText(candidate, RECEIPT_NAME_MAX) : undefined;
}

function validActivityTimestamp(value: string): boolean {
  const match = value.match(/^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d+)?(?:Z|[+-](\d{2}):(\d{2}))$/);
  if (!match) return false;
  const [, yearText, monthText, dayText, hourText, minuteText, secondText, offsetHour = "0", offsetMinute = "0"] = match;
  const [year, month, day, hour, minute, second, zoneHour, zoneMinute] = [
    yearText, monthText, dayText, hourText, minuteText, secondText, offsetHour, offsetMinute,
  ].map(Number);
  const leapYear = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
  const daysInMonth = [31, leapYear ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][month - 1] ?? 0;
  return day >= 1 && day <= daysInMonth && hour <= 23 && minute <= 59 && second <= 59
    && zoneHour <= 23 && zoneMinute <= 59 && Number.isFinite(Date.parse(value));
}

export function normalizePersistedActivityEntries(entries: unknown[]): JournalEntry[] {
  const seen = new Set<string>();
  return entries.flatMap((value) => {
    if (!value || typeof value !== "object") return [];
    const raw = value as Record<string, unknown>;
    const id = safeActivityIdentity(raw.id);
    const timestamp = typeof raw.ts === "string" ? raw.ts : "";
    const action = ACTIVITY_ACTIONS.includes(raw.action as JournalEntry["action"])
      ? raw.action as JournalEntry["action"]
      : undefined;
    if (!id || seen.has(id) || timestamp.length > 64
      || !validActivityTimestamp(timestamp) || action === undefined
      || !ACTIVITY_OUTCOMES.includes(raw.outcome as JournalEntry["outcome"])) return [];
    seen.add(id);
    const subject = ACTIVITY_SUBJECTS.includes(raw.subject as NonNullable<JournalEntry["subject"]>)
      ? raw.subject as NonNullable<JournalEntry["subject"]>
      : undefined;
    const scope = raw.scope === "user" || raw.scope === "project" ? raw.scope : undefined;
    const receipt = normalizeActivityReceipt(raw.receipt);
    const safeText = (candidate: unknown, max = 512) =>
      typeof candidate === "string" ? boundedFactoryText(candidate, max) : undefined;
    const detail = safeText(raw.detail);
    const subjectName = safeText(raw.subjectName, RECEIPT_NAME_MAX);
    const agentName = safeText(raw.agentName, RECEIPT_NAME_MAX);
    const agentSlug = safeText(raw.agentSlug, RECEIPT_NAME_MAX);
    const safeTool = safeText(raw.tool, RECEIPT_NAME_MAX);
    const tool = safeTool && toolMeta(safeTool) ? safeTool : undefined;
    const projectLabel = safeActivityProjectLabel(raw.projectLabel, raw.projectPath);
    return [{
      id,
      ts: timestamp,
      action,
      outcome: raw.outcome as JournalEntry["outcome"],
      ...(subject === undefined ? {} : { subject }),
      ...(tool === undefined ? {} : { tool }),
      ...(scope === undefined ? {} : { scope }),
      ...(detail === undefined ? {} : { detail }),
      ...(subjectName === undefined ? {} : { subjectName }),
      ...(agentName === undefined ? {} : { agentName }),
      ...(agentSlug === undefined ? {} : { agentSlug }),
      ...(projectLabel === undefined ? {} : { projectLabel }),
      ...(receipt ? { receipt } : {}),
    }];
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
          this.persistNow();
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
    void this.refreshFactoryReceipts();
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
      const trimmed = normalizePersistedActivityEntries(this.localEntries).slice(0, MAX_ENTRIES);
      this.localEntries = trimmed;
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
    const full = normalizePersistedActivityEntries([{
      ...base,
      ...(receipt ? { receipt } : {}),
      id: crypto.randomUUID(),
      ts: new Date().toISOString(),
    }])[0]!;
    this.localEntries = [full, ...this.localEntries].slice(0, MAX_ENTRIES);
    this.mergeEntries();
    // Debounced so a bulk loop's per-item logs coalesce into one write.
    this.schedulePersist();
    return full.id;
  }

  /** Record one bounded terminal Factory receipt per run and return its stable Activity id. */
  recordFactoryRunReceipt(run: ExpertRun, projectLabel: string): string | null {
    const receipt = factoryReceiptFromRun(run, projectLabel);
    if (!receipt) return null;
    const matchesRun = (entry: JournalEntry) =>
      entry.receipt?.operation === "factory" && entry.receipt.runId === receipt.runId;
    const existing = this.localEntries.find(matchesRun);
    if (existing) {
      const deduplicated = this.localEntries.filter((entry) => entry === existing || !matchesRun(entry));
      if (deduplicated.length !== this.localEntries.length) {
        this.localEntries = deduplicated;
        this.mergeEntries();
        this.schedulePersist();
      }
      return existing.id;
    }
    return this.log({
      action: "factory",
      subject: "factory",
      subjectName: receipt.workTitle,
      outcome: receipt.outcome === "accepted" ? "ok" : "error",
      detail: `Factory result ${receipt.outcome}`,
      receipt,
    });
  }

  /** Reconcile terminal Factory results independently of the current app route. */
  async refreshFactoryReceipts(): Promise<void> {
    try {
      const [runs, registered] = await Promise.all([expertRunsList(), projectsList()]);
      const labels = new Map(registered.map((project) => [project.path, project.label]));
      for (const run of runs) {
        if (run.factory?.terminal) {
          this.recordFactoryRunReceipt(run, labels.get(run.projectPath) ?? "Registered project");
        }
      }
    } catch (error) {
      console.warn(
        `[activity] Factory receipt reconciliation failed: ${error instanceof Error ? error.message : String(error)}`,
      );
    }
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
        .flatMap((entry) => {
          const normalized = this.fromMcpAudit(entry);
          return normalized ? [normalized] : [];
        });
      this.mergeEntries();
    } catch (error) {
      console.warn(
        `[activity] MCP audit load failed: ${error instanceof Error ? error.message : String(error)}`,
      );
    }
  }

  private fromMcpAudit(entry: McpAuditEntry): JournalEntry | undefined {
    return normalizePersistedActivityEntries([{
      id: `mcp:${entry.id}`,
      ts: entry.timestamp,
      action: "mcp",
      subject: "mcp",
      subjectName: entry.tool,
      projectPath: entry.projectPath ?? undefined,
      outcome: entry.phase === "attempt" ? "pending" : entry.success ? "ok" : "error",
      detail: `${entry.client ?? "unknown client"} · ${entry.action}`,
    }])[0];
  }

  private mergeEntries(): void {
    this.hasLocalEntries = this.localEntries.length > 0;
    this.entries = mergeActivityEntries(this.localEntries, this.mcpEntries);
  }
}

export const activity = new ActivityStore();
