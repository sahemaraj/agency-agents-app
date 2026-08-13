import { describe, expect, it } from "vitest";

import {
  FIRST_DEPLOYMENT_COMPLETION,
  defaultFirstDeploymentTool,
  recommendFirstDeploymentPreset,
  shouldShowFirstDeployment,
} from "./firstDeployment";

const presets = [
  { slug: "first", agents: ["a", "b"] },
  { slug: "ai-builders", agents: ["b", "c"] },
  { slug: "third", agents: ["c"] },
];

describe("guided first deployment", () => {
  it("prefers AI Builders, falls back in declaration order, and rejects partial presets", () => {
    expect(recommendFirstDeploymentPreset(new Set(["a", "b", "c"]), presets)?.slug)
      .toBe("ai-builders");
    expect(recommendFirstDeploymentPreset(new Set(["a", "b"]), presets)?.slug)
      .toBe("first");
    expect(recommendFirstDeploymentPreset(new Set(["c"]), presets)?.slug)
      .toBe("third");
    expect(recommendFirstDeploymentPreset(new Set(["a"]), presets)).toBeNull();
  });

  it("defaults to detected Claude Code before Codex and never to an undetected target", () => {
    expect(defaultFirstDeploymentTool([
      { tool: "codex", detected: true },
      { tool: "claudeCode", detected: true },
    ])).toBe("claudeCode");
    expect(defaultFirstDeploymentTool([{ tool: "codex", detected: true }])).toBe("codex");
    expect(defaultFirstDeploymentTool([{ tool: "claudeCode", detected: false }])).toBeNull();
  });

  it("shows only for unfinished users after catalog configuration and fresh empty truth", () => {
    const eligible = {
      catalogLoaded: true,
      catalogConfigured: true,
      completion: null,
      reconciled: true,
      reconcileError: null,
      managedInstallCount: 0,
    };
    expect(shouldShowFirstDeployment(eligible)).toBe(true);
    expect(shouldShowFirstDeployment({ ...eligible, catalogConfigured: false })).toBe(true);
    expect(shouldShowFirstDeployment({ ...eligible, catalogLoaded: false })).toBe(false);
    expect(shouldShowFirstDeployment({ ...eligible, completion: FIRST_DEPLOYMENT_COMPLETION })).toBe(false);
    expect(shouldShowFirstDeployment({ ...eligible, reconciled: false })).toBe(false);
    expect(shouldShowFirstDeployment({ ...eligible, reconcileError: "scan failed" })).toBe(false);
    expect(shouldShowFirstDeployment({ ...eligible, managedInstallCount: 1 })).toBe(false);
    expect(shouldShowFirstDeployment({ ...eligible, completion: "v0" })).toBe(true);
  });
});
