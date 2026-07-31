import type {
  InstalledSkill,
  SkillFolderState,
  SkillPackageResult,
  SkillReference,
  SkillSmartFolderRule,
  SkillSource,
  SkillType,
} from "$lib/types";

export type StatusFilter = "all" | "ready" | "rejected";
export type SortOrder = "name" | "type" | "source";
export type PackageView = { pkg: SkillPackageResult; source: SkillSource };
export type SkillGroupNode = {
  key: string;
  label: string;
  children: SkillGroupNode[];
  packages: PackageView[];
};
export type PersonalFolderNode = {
  path: string;
  label: string;
  children: PersonalFolderNode[];
};

type FilterOptions = {
  packages: PackageView[];
  installed: InstalledSkill[];
  folderState: SkillFolderState;
  query: string;
  statusFilter: StatusFilter;
  sourceFilter: string;
  libraryFilter: string;
  sortOrder: SortOrder;
};

function sameSkill(left: SkillReference, right: SkillReference): boolean {
  return left.sourceId === right.sourceId && left.relativePath === right.relativePath;
}

export function sourceLabel(source: SkillSource): string {
  return source.kind.kind === "local" ? source.kind.root : source.kind.repository;
}

export function taxonomyLabel(value: string): string {
  return value
    .split("-")
    .map((part) => (part ? `${part[0].toUpperCase()}${part.slice(1)}` : part))
    .join(" ");
}

export function typeLabel(value: SkillType): string {
  return value === "ai" ? "AI" : value === "devops" ? "DevOps" : taxonomyLabel(value);
}

export function isInstalled(pkg: SkillPackageResult, installed: InstalledSkill[]): boolean {
  return installed.some((record) => sameSkill(record, pkg) && record.state !== "missing");
}

export function requiresTrust(pkg: SkillPackageResult): boolean {
  return pkg.errors.some((error) => error.code === "trustRequired");
}

export function trustedScripts(pkg: SkillPackageResult): boolean {
  return pkg.installable
    && pkg.files.some((file) => file.relativePath.toLowerCase().startsWith("scripts/"));
}

export function packageConflicts(pkg: SkillPackageResult, packages: PackageView[]): PackageView[] {
  return packages.filter((candidate) =>
    candidate.pkg.sourceId !== pkg.sourceId
    && candidate.pkg.name !== null
    && candidate.pkg.name === pkg.name
  );
}

export function matchesSmartFolder(
  pkg: SkillPackageResult,
  rule: SkillSmartFolderRule,
  favorites: SkillReference[],
): boolean {
  if (rule.query) {
    const query = rule.query.toLowerCase();
    if (![pkg.name ?? "", pkg.description ?? "", pkg.relativePath].some((value) =>
      value.toLowerCase().includes(query)
    )) return false;
  }
  if (rule.skillType && pkg.skillType !== rule.skillType) return false;
  if (rule.tag && !pkg.tags.includes(rule.tag)) return false;
  if (rule.sourceId && pkg.sourceId !== rule.sourceId) return false;
  if (rule.installable !== null && pkg.installable !== rule.installable) return false;
  if (rule.favorite !== null && favorites.some((favorite) => sameSkill(favorite, pkg)) !== rule.favorite) return false;
  return true;
}

export function filterPackages(options: FilterOptions): PackageView[] {
  const { packages, installed, folderState, statusFilter, sourceFilter, libraryFilter, sortOrder } = options;
  const query = options.query.trim().toLowerCase();
  const visible = packages.filter(({ pkg, source }) => {
    if (statusFilter === "ready" && !pkg.installable) return false;
    if (statusFilter === "rejected" && pkg.installable) return false;
    if (sourceFilter !== "all" && source.id !== sourceFilter) return false;
    if (libraryFilter === "installed" && !isInstalled(pkg, installed)) return false;
    if (libraryFilter === "trusted" && !trustedScripts(pkg)) return false;
    if (libraryFilter === "review" && pkg.installable && !requiresTrust(pkg)) return false;
    if (libraryFilter === "favorites" && !folderState.favorites.some((favorite) => sameSkill(favorite, pkg))) return false;
    if (libraryFilter === "recommendations" && (isInstalled(pkg, installed) || pkg.qualityScore < 60)) return false;
    if (libraryFilter === "duplicates" && packageConflicts(pkg, packages).length === 0) return false;
    if (libraryFilter === "cleanup" && (!isInstalled(pkg, installed) || folderState.usage.some((usage) =>
      sameSkill(usage.skill, pkg) && (usage.fetches > 0 || usage.installs > 0)
    ))) return false;
    if (libraryFilter === "recent" && !folderState.recent.some((recent) => sameSkill(recent.skill, pkg))) return false;
    if (libraryFilter.startsWith("collection:")) {
      const collection = folderState.collections.find((item) =>
        item.name === libraryFilter.slice("collection:".length)
      );
      if (!collection?.skills.some((skill) => sameSkill(skill, pkg))) return false;
    }
    if (libraryFilter.startsWith("smart:")) {
      const rule = folderState.smartFolders.find((item) =>
        item.name === libraryFilter.slice("smart:".length)
      )?.rule;
      if (!rule || !matchesSmartFolder(pkg, rule, folderState.favorites)) return false;
    }
    if (libraryFilter.startsWith("personal:")) {
      const folder = libraryFilter.slice("personal:".length);
      const assigned = folderState.assignments.find((item) => sameSkill(item, pkg))?.folderPath;
      if (assigned !== folder && !assigned?.startsWith(`${folder}/`)) return false;
    }
    if (libraryFilter.startsWith("taxonomy:")) {
      const path = libraryFilter.slice("taxonomy:".length).split("/");
      if (pkg.skillType !== path[0]) return false;
      if (!path.slice(1).every((segment, index) => pkg.group[index] === segment)) return false;
    }
    return !query || [
      pkg.name ?? "",
      pkg.description ?? "",
      pkg.relativePath,
      pkg.skillType,
      ...pkg.group,
      ...pkg.tags,
      sourceLabel(source),
    ].some((value) => value.toLowerCase().includes(query));
  });

  return visible.sort((left, right) => {
    if (libraryFilter === "recent") {
      const index = (view: PackageView): number =>
        folderState.recent.findIndex((recent) => sameSkill(recent.skill, view.pkg));
      return index(left) - index(right);
    }
    if (sortOrder === "type") {
      const byType = typeLabel(left.pkg.skillType).localeCompare(typeLabel(right.pkg.skillType));
      if (byType !== 0) return byType;
    }
    if (sortOrder === "source") {
      const bySource = sourceLabel(left.source).localeCompare(sourceLabel(right.source));
      if (bySource !== 0) return bySource;
    }
    return (left.pkg.name ?? left.pkg.relativePath)
      .localeCompare(right.pkg.name ?? right.pkg.relativePath);
  });
}

export function groupPackages(packages: PackageView[]): SkillGroupNode[] {
  const roots = new Map<string, SkillGroupNode>();
  for (const view of packages) {
    let root = roots.get(view.pkg.skillType);
    if (!root) {
      root = {
        key: view.pkg.skillType,
        label: typeLabel(view.pkg.skillType),
        children: [],
        packages: [],
      };
      roots.set(view.pkg.skillType, root);
    }
    let node = root;
    for (const segment of view.pkg.group) {
      let child = node.children.find((candidate) => candidate.key === `${node.key}/${segment}`);
      if (!child) {
        child = {
          key: `${node.key}/${segment}`,
          label: taxonomyLabel(segment),
          children: [],
          packages: [],
        };
        node.children.push(child);
      }
      node = child;
    }
    node.packages.push(view);
  }

  const sort = (nodes: SkillGroupNode[]): void => {
    nodes.sort((left, right) => left.label.localeCompare(right.label));
    for (const node of nodes) {
      node.packages.sort((left, right) =>
        (left.pkg.name ?? left.pkg.relativePath)
          .localeCompare(right.pkg.name ?? right.pkg.relativePath)
      );
      sort(node.children);
    }
  };
  const result = [...roots.values()];
  sort(result);
  return result;
}

export function buildPersonalFolderTree(paths: string[]): PersonalFolderNode[] {
  const roots: PersonalFolderNode[] = [];
  for (const path of paths) {
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

export function libraryMetrics(
  packages: PackageView[],
  installed: InstalledSkill[],
  folderState: SkillFolderState,
) {
  return {
    installed: packages.filter(({ pkg }) => isInstalled(pkg, installed)).length,
    trusted: packages.filter(({ pkg }) => trustedScripts(pkg)).length,
    review: packages.filter(({ pkg }) => !pkg.installable || requiresTrust(pkg)).length,
    recommendations: packages.filter(({ pkg }) =>
      !isInstalled(pkg, installed) && pkg.qualityScore >= 60
    ).length,
    duplicates: packages.filter(({ pkg }) => packageConflicts(pkg, packages).length > 0).length,
    cleanup: packages.filter(({ pkg }) =>
      isInstalled(pkg, installed) && !folderState.usage.some((usage) =>
        sameSkill(usage.skill, pkg) && (usage.fetches > 0 || usage.installs > 0)
      )
    ).length,
  };
}
