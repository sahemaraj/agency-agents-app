import type {
  AgentApprovalAction,
  AgentPackageResult,
  AgentReference,
  AgentSmartFolderRule,
  AgentSource,
  AgentUpdatePolicy,
  InstallState,
  Tool,
} from "$lib/types";

export type AgentDetailTab = "overview" | "source" | "security";
const AGENT_DETAIL_TABS: AgentDetailTab[] = ["overview", "source", "security"];

export type AgentPackageView = { pkg: AgentPackageResult; source: AgentSource };
export type AgentFolderNode = {
  path: string;
  label: string;
  children: AgentFolderNode[];
};

export function sameAgent(left: AgentReference, right: AgentReference): boolean {
  return left.sourceId === right.sourceId && left.relativePath === right.relativePath;
}

export function findAgentPackage(
  packages: AgentPackageResult[],
  reference: AgentReference | null,
): AgentPackageResult | null {
  return reference
    ? packages.find((candidate) => sameAgent(candidate.reference, reference)) ?? null
    : null;
}

export function agentInstallKey(
  reference: AgentReference,
  tool: Tool,
  projectPath: string | null,
): string {
  return JSON.stringify([reference.sourceId, reference.relativePath, tool, projectPath]);
}

export function agentStateBuckets(states: readonly InstallState[]): Record<InstallState, number> {
  const counts: Record<InstallState, number> = {
    current: 0,
    outdated: 0,
    modified: 0,
    missing: 0,
    foreign: 0,
    disabled: 0,
    sourceUnavailable: 0,
  };
  for (const state of states) counts[state]++;
  return counts;
}

export function installStateMessageKey(state: InstallState): `state.${InstallState}` {
  return `state.${state}`;
}

export function canApplyAgentPlan(plan: Pick<{ blockers: string[] }, "blockers">): boolean {
  return plan.blockers.length === 0;
}

export function agentUpdateDecision(
  policy: AgentUpdatePolicy,
  capabilitiesBroadened: boolean,
  publisherStillVerified: boolean,
): { blocked: boolean; requiresConfirmation: boolean } {
  if (policy === "pin") return { blocked: true, requiresConfirmation: false };
  return {
    blocked: false,
    requiresConfirmation:
      policy !== "autoTrusted" || capabilitiesBroadened || !publisherStillVerified,
  };
}

export function buildAgentFolderTree(paths: string[]): AgentFolderNode[] {
  const roots: AgentFolderNode[] = [];
  for (const path of [...paths].sort()) {
    let nodes = roots;
    let current = "";
    for (const label of path.split("/")) {
      current = current ? `${current}/${label}` : label;
      let node = nodes.find((candidate) => candidate.path === current);
      if (!node) {
        node = { path: current, label, children: [] };
        nodes.push(node);
        nodes.sort((left, right) => left.label.localeCompare(right.label));
      }
      nodes = node.children;
    }
  }
  return roots;
}

export function agentConflicts(
  pkg: AgentPackageResult,
  packages: AgentPackageView[],
): AgentPackageView[] {
  const name = pkg.agent?.name;
  if (!name) return [];
  return packages.filter(({ pkg: candidate }) =>
    !sameAgent(candidate.reference, pkg.reference) && candidate.agent?.name === name
  );
}

export function agentPackageLabel(view: AgentPackageView, packages: AgentPackageView[]): string {
  const name = view.pkg.agent?.name ?? view.pkg.reference.relativePath;
  return agentConflicts(view.pkg, packages).length > 0 ? `${name} · ${view.source.label}` : name;
}

export function nextAgentDetailTab(current: AgentDetailTab, key: string): AgentDetailTab {
  if (key === "Home") return AGENT_DETAIL_TABS[0];
  if (key === "End") return AGENT_DETAIL_TABS[AGENT_DETAIL_TABS.length - 1];
  if (key !== "ArrowLeft" && key !== "ArrowRight") return current;
  const offset = key === "ArrowRight" ? 1 : -1;
  return AGENT_DETAIL_TABS[
    (AGENT_DETAIL_TABS.indexOf(current) + offset + AGENT_DETAIL_TABS.length)
      % AGENT_DETAIL_TABS.length
  ];
}

export function agentApprovalFacts(action: AgentApprovalAction): {
  kind: AgentApprovalAction["action"];
  subject: string;
  planRevision: string | null;
} {
  const reference = "reference" in action ? action.reference : null;
  const subject = reference
    ? `${reference.sourceId} · ${reference.relativePath}`
    : "sourceId" in action ? action.sourceId
    : "path" in action ? action.path
    : "name" in action ? action.name
    : "collectionName" in action ? action.collectionName
    : "";
  return {
    kind: action.action,
    subject,
    planRevision: "planRevision" in action ? action.planRevision : null,
  };
}

function lifecycle(view: AgentPackageView): string {
  if (view.source.kind.kind === "builtIn") return "builtIn";
  if (view.source.kind.kind === "published") return "published";
  return "external";
}

export function matchesAgentSmartFolder(
  view: AgentPackageView,
  rule: AgentSmartFolderRule,
  favorites: AgentReference[],
): boolean {
  const { pkg, source } = view;
  if (rule.query) {
    const query = rule.query.toLowerCase();
    if (![pkg.agent?.name ?? "", pkg.agent?.description ?? "", pkg.reference.relativePath]
      .some((value) => value.toLowerCase().includes(query))) return false;
  }
  if (rule.division && pkg.agent?.category !== rule.division) return false;
  if (rule.sourceId && source.id !== rule.sourceId) return false;
  if (rule.capability && !pkg.capabilities.includes(rule.capability)) return false;
  if (rule.lifecycleState && lifecycle(view) !== rule.lifecycleState) return false;
  if (rule.installable !== null && pkg.installable !== rule.installable) return false;
  const favorite = favorites.some((item) => sameAgent(item, pkg.reference));
  if (rule.favorite !== null && favorite !== rule.favorite) return false;
  return true;
}
