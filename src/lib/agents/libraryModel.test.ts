import { describe, expect, it } from "vitest";

import {
  agentApprovalFacts,
  agentConflicts,
  findAgentPackage,
  agentPackageLabel,
  agentInstallKey,
  agentStateBuckets,
  agentUpdateDecision,
  buildAgentFolderTree,
  canApplyAgentPlan,
  installStateMessageKey,
  matchesAgentSmartFolder,
  nextAgentDetailTab,
} from "./libraryModel";

const packageView = (sourceId: string, relativePath: string, name: string) => ({
  source: { id: sourceId, label: sourceId, enabled: true, kind: { kind: "local" as const, root: "/tmp" } },
  pkg: {
    reference: { sourceId, relativePath },
    agent: { slug: "reviewer", name, description: "Reviews code", category: "engineering", emoji: null, color: null, vibe: null, body: "" },
    sourceHash: "hash", frontmatterHash: "front", bodyHash: "body", version: null, channel: null,
    changelog: null, publisher: null, publisherKey: null, publisherVerified: false,
    requiredAgents: [], recommendedAgents: [], groups: [], tags: [], capabilities: ["review"], permissions: [],
    qualityScore: 75, qualityChecks: [], diagnostics: [], installable: true,
  },
});

describe("Agent library model", () => {
  it("builds deterministic nested logical folders", () => {
    expect(buildAgentFolderTree(["Work/Review", "Personal", "Work"]))
      .toEqual([
        { path: "Personal", label: "Personal", children: [] },
        { path: "Work", label: "Work", children: [
          { path: "Work/Review", label: "Review", children: [] },
        ] },
      ]);
  });

  it("keeps same-name Agents distinct by canonical reference", () => {
    const first = packageView("one", "a/reviewer.md", "Reviewer");
    const second = packageView("two", "b/reviewer.md", "Reviewer");
    expect(agentConflicts(first.pkg, [first, second])).toEqual([second]);
    expect(agentPackageLabel(first, [first, second])).toBe("Reviewer · one");
  });

  it("restores an exact selection from refreshed package objects", () => {
    const refreshed = packageView("one", "a/reviewer.md", "Reviewer").pkg;
    expect(findAgentPackage(
      [refreshed, packageView("two", "a/reviewer.md", "Reviewer").pkg],
      { sourceId: "one", relativePath: "a/reviewer.md" },
    )).toBe(refreshed);
    expect(findAgentPackage([refreshed], null)).toBeNull();
  });

  it("moves detail tabs with arrow, Home, and End keys", () => {
    expect(nextAgentDetailTab("overview", "ArrowRight")).toBe("source");
    expect(nextAgentDetailTab("overview", "ArrowLeft")).toBe("security");
    expect(nextAgentDetailTab("source", "Home")).toBe("overview");
    expect(nextAgentDetailTab("source", "End")).toBe("security");
    expect(nextAgentDetailTab("source", "Enter")).toBe("source");
  });

  it("exposes structured approval subjects and exact plan revisions", () => {
    expect(agentApprovalFacts({
      action: "install",
      reference: { sourceId: "one", relativePath: "engineering/reviewer.md" },
      tool: "claudeCode",
      projectPath: "/work/app",
      includeDependencies: true,
      planRevision: "rev-42",
    })).toEqual({
      kind: "install",
      subject: "one · engineering/reviewer.md",
      planRevision: "rev-42",
    });
    expect(agentApprovalFacts({ action: "sourceRemove", sourceId: "source:one" }))
      .toEqual({ kind: "sourceRemove", subject: "source:one", planRevision: null });
  });

  it("matches Agent-specific smart-folder fields", () => {
    const view = packageView("one", "engineering/reviewer.md", "Reviewer");
    expect(matchesAgentSmartFolder(view, {
      query: "review", division: "engineering", sourceId: "one", capability: "review",
      lifecycleState: "external", installable: true, favorite: true,
    }, [view.pkg.reference])).toBe(true);
  });

  it("keys installs by exact source, path, tool, and project", () => {
    const base = { sourceId: "one", relativePath: "engineering/reviewer.md" };
    expect(agentInstallKey(base, "claudeCode", null)).not.toBe(
      agentInstallKey({ ...base, sourceId: "two" }, "claudeCode", null),
    );
    expect(agentInstallKey(base, "claudeCode", "/work/a")).not.toBe(
      agentInstallKey(base, "claudeCode", "/work/b"),
    );
  });

  it("groups and labels every lifecycle state without color-only meaning", () => {
    const states = [
      "current", "outdated", "modified", "missing", "foreign", "disabled", "sourceUnavailable",
    ] as const;
    expect(agentStateBuckets(states)).toEqual({
      current: 1, outdated: 1, modified: 1, missing: 1, foreign: 1, disabled: 1, sourceUnavailable: 1,
    });
    expect(states.map(installStateMessageKey)).toEqual([
      "state.current", "state.outdated", "state.modified", "state.missing", "state.foreign",
      "state.disabled", "state.sourceUnavailable",
    ]);
  });

  it("blocks executable plans and explains update policy confirmation", () => {
    expect(canApplyAgentPlan({ blockers: ["Pinned by policy"] })).toBe(false);
    expect(canApplyAgentPlan({ blockers: [] })).toBe(true);
    expect(agentUpdateDecision("pin", false, true)).toEqual({ blocked: true, requiresConfirmation: false });
    expect(agentUpdateDecision("notify", false, true)).toEqual({ blocked: false, requiresConfirmation: true });
    expect(agentUpdateDecision("reviewScripts", false, true)).toEqual({ blocked: false, requiresConfirmation: true });
    expect(agentUpdateDecision("autoTrusted", false, true)).toEqual({ blocked: false, requiresConfirmation: false });
    expect(agentUpdateDecision("autoTrusted", true, true)).toEqual({ blocked: false, requiresConfirmation: true });
    expect(agentUpdateDecision("autoTrusted", false, false)).toEqual({ blocked: false, requiresConfirmation: true });
  });
});
