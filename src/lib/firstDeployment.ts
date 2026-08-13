import type { Tool, ToolInfo } from "$lib/types";

export const FIRST_DEPLOYMENT_COMPLETION = "v1";
export const FIRST_DEPLOYMENT_STORAGE_KEY = "agency-agents:first-deployment";

type Preset = { slug: string; agents: string[] };
type DetectedTool = Pick<ToolInfo, "tool" | "detected">;

export function recommendFirstDeploymentPreset<T extends Preset>(
  catalogSlugs: ReadonlySet<string>,
  presets: readonly T[],
): T | null {
  const complete = (preset: T) => preset.agents.every((slug) => catalogSlugs.has(slug));
  const preferred = presets.find((preset) => preset.slug === "ai-builders");
  return preferred && complete(preferred) ? preferred : presets.find(complete) ?? null;
}

export function defaultFirstDeploymentTool(tools: readonly DetectedTool[]): Tool | null {
  const detected = new Set(tools.filter((tool) => tool.detected).map((tool) => tool.tool));
  return detected.has("claudeCode") ? "claudeCode" : detected.has("codex") ? "codex" : null;
}

export function shouldShowFirstDeployment(input: {
  catalogLoaded: boolean;
  catalogConfigured: boolean;
  completion: string | null;
  reconciled: boolean;
  reconcileError: string | null;
  managedInstallCount: number;
}): boolean {
  if (!input.catalogLoaded) return false;
  if (!input.catalogConfigured) return true;
  return input.completion !== FIRST_DEPLOYMENT_COMPLETION
    && input.reconciled
    && !input.reconcileError
    && input.managedInstallCount === 0;
}
