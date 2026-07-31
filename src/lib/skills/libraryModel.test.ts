import { describe, expect, it } from "vitest";
import type {
  InstalledSkill,
  SkillFolderState,
  SkillPackageResult,
  SkillSource,
} from "$lib/types";
import {
  buildPersonalFolderTree,
  filterPackages,
  groupPackages,
  libraryMetrics,
  matchesSmartFolder,
  packageConflicts,
  typeLabel,
  type PackageView,
} from "./libraryModel";

const local: SkillSource = { id: "local", kind: { kind: "local", root: "/skills" } };
const github: SkillSource = {
  id: "github",
  kind: {
    kind: "github",
    repository: "owner/repo",
    gitRef: null,
    subdirectory: null,
    activeCheckout: null,
  },
};

function skill(overrides: Partial<SkillPackageResult> = {}): SkillPackageResult {
  return {
    sourceId: "local",
    relativePath: "alpha",
    name: "Alpha",
    description: "Testing helper",
    skillType: "testing",
    group: ["quality"],
    tags: ["fast"],
    dependencies: [],
    recommendedSkills: [],
    version: "1.0.0",
    channel: "stable",
    changelog: null,
    publisher: null,
    publisherKey: null,
    publisherVerified: false,
    validationResults: [],
    permissions: [],
    qualityScore: 80,
    qualityChecks: [],
    files: [],
    trustFingerprint: null,
    errors: [],
    installable: true,
    ...overrides,
  };
}

const alpha = skill();
const alphaDuplicate = skill({
  sourceId: "github",
  relativePath: "copies/alpha",
  qualityScore: 55,
});
const beta = skill({
  sourceId: "github",
  relativePath: "beta",
  name: "Beta",
  description: "Deployment helper",
  skillType: "devops",
  group: ["delivery"],
  tags: ["release"],
  files: [{ relativePath: "scripts/run.sh", sizeBytes: 12, sha256: "a".repeat(64) }],
});
const rejected = skill({
  relativePath: "broken",
  name: "Broken",
  installable: false,
  qualityScore: 10,
  errors: [{ code: "invalidMetadata", path: "SKILL.md", message: "invalid" }],
});
const packages: PackageView[] = [
  { pkg: alpha, source: local },
  { pkg: alphaDuplicate, source: github },
  { pkg: beta, source: github },
  { pkg: rejected, source: local },
];

const installed: InstalledSkill[] = [{
  sourceId: "local",
  relativePath: "alpha",
  name: "Alpha",
  runtime: "codex",
  scope: "user",
  projectPath: null,
  path: "/installed/alpha",
  state: "current",
  tracked: true,
}];

const folderState: SkillFolderState = {
  folders: ["Work", "Work/QA", "Personal"],
  assignments: [{ sourceId: "local", relativePath: "alpha", folderPath: "Work/QA" }],
  favorites: [{ sourceId: "github", relativePath: "beta" }],
  recent: [
    { skill: { sourceId: "github", relativePath: "beta" }, viewedAt: "2026-07-31T01:00:00Z" },
    { skill: { sourceId: "local", relativePath: "alpha" }, viewedAt: "2026-07-31T00:00:00Z" },
  ],
  collections: [{ name: "Release", skills: [{ sourceId: "github", relativePath: "beta" }] }],
  smartFolders: [{
    name: "Fast tests",
    rule: {
      query: "testing",
      skillType: "testing",
      tag: "fast",
      sourceId: "local",
      installable: true,
      favorite: false,
    },
  }],
  profiles: [],
  updatePolicies: [],
  publisherTrust: [],
  preferredSources: [],
  usage: [{
    skill: { sourceId: "local", relativePath: "alpha" },
    fetches: 1,
    installs: 0,
    rejections: 0,
    lastUsedAt: "2026-07-31T00:00:00Z",
  }],
  approvals: [],
};

describe("Skills library model", () => {
  it("filters built-in, collection, folder, taxonomy, and query views", () => {
    const base = { packages, installed, folderState, statusFilter: "all" as const, sourceFilter: "all", sortOrder: "name" as const };

    expect(filterPackages({ ...base, libraryFilter: "installed", query: "" }).map(({ pkg }) => pkg.name)).toEqual(["Alpha"]);
    expect(filterPackages({ ...base, libraryFilter: "favorites", query: "" }).map(({ pkg }) => pkg.name)).toEqual(["Beta"]);
    expect(filterPackages({ ...base, libraryFilter: "collection:Release", query: "" }).map(({ pkg }) => pkg.name)).toEqual(["Beta"]);
    expect(filterPackages({ ...base, libraryFilter: "personal:Work", query: "" }).map(({ pkg }) => pkg.name)).toEqual(["Alpha"]);
    expect(filterPackages({ ...base, libraryFilter: "taxonomy:devops/delivery", query: "" }).map(({ pkg }) => pkg.name)).toEqual(["Beta"]);
    expect(filterPackages({ ...base, libraryFilter: "all", query: "deployment" }).map(({ pkg }) => pkg.name)).toEqual(["Beta"]);
  });

  it("keeps recent order and applies source/type sorting", () => {
    const base = { packages, installed, folderState, query: "", statusFilter: "all" as const, sourceFilter: "all" };

    expect(filterPackages({ ...base, libraryFilter: "recent", sortOrder: "name" }).map(({ pkg }) => pkg.name)).toEqual(["Beta", "Alpha"]);
    expect(filterPackages({ ...base, libraryFilter: "all", sortOrder: "type" }).map(({ pkg }) => pkg.name)).toEqual(["Beta", "Alpha", "Alpha", "Broken"]);
    expect(filterPackages({ ...base, libraryFilter: "all", sortOrder: "source" }).map(({ pkg }) => pkg.relativePath)).toEqual(["alpha", "broken", "copies/alpha", "beta"]);
  });

  it("matches smart folders and detects duplicate names across sources", () => {
    expect(matchesSmartFolder(alpha, folderState.smartFolders[0].rule, folderState.favorites)).toBe(true);
    expect(matchesSmartFolder(beta, folderState.smartFolders[0].rule, folderState.favorites)).toBe(false);
    expect(packageConflicts(alpha, packages)).toEqual([packages[1]]);
    expect(packageConflicts(rejected, packages)).toEqual([]);
  });

  it("builds stable taxonomy and personal-folder trees", () => {
    expect(groupPackages(packages).map((node) => [node.label, node.children[0]?.label])).toEqual([
      ["DevOps", "Delivery"],
      ["Testing", "Quality"],
    ]);
    expect(buildPersonalFolderTree(folderState.folders)).toEqual([
      {
        path: "Personal",
        label: "Personal",
        children: [],
      },
      {
        path: "Work",
        label: "Work",
        children: [{ path: "Work/QA", label: "QA", children: [] }],
      },
    ]);
    expect(typeLabel("ai")).toBe("AI");
  });

  it("calculates library metrics from the same state used by filters", () => {
    expect(libraryMetrics(packages, installed, folderState)).toEqual({
      installed: 1,
      trusted: 1,
      review: 1,
      recommendations: 1,
      duplicates: 2,
      cleanup: 0,
    });
  });
});
