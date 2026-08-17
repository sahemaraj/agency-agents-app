/**
 * UI store — current sidebar section, theme, modal/drawer state.
 * Uses Svelte 5 runes inside a module-scope class instance.
 */

import type { AgentReference, SettingsSection, SidebarSection, SkillReference, ThemePreference, Tool } from "$lib/types";

type ExpertReviewLink = { kind: "change" | "run" | "activation"; id: string };
type AgentRecoveryLink = { reference: AgentReference; tool: Tool; projectPath: string | null };
type SkillRecoveryLink = { reference: SkillReference; runtime: "claudeCode" | "codex"; projectPath: string | null };
export type ReviewIntentKind = "agent" | "skill" | "expert-change" | "expert-run" | "expert-activation" | "project";
export type RecoveryIntentKind = "agent" | "skill";
export type ReviewIntent = { origin: "activity-review"; kind: ReviewIntentKind; exactId: string; triggerId: string };
export type RecoveryIntent = { origin: "settings-recovery"; kind: RecoveryIntentKind; exactId: string; triggerId: string };
const MAX_INTENT_ID_BYTES = 2 * 1024;
const MAX_INTENT_TRIGGER_BYTES = 512;

function boundedIntentField(value: string, maxBytes: number): boolean {
  return value.length > 0 && new TextEncoder().encode(value).length <= maxBytes;
}

/** A navigable app location — the unit of back/forward history. Captures the
    section plus the full Agents workspace view-state so a back/forward jump
    restores the filter, category, and the open agent. */
interface NavLocation {
  section: SidebarSection;
  agentsCategory: string | null;
  agentsLens: string;
  agentsSelected: string | null;
  agentsReference: AgentReference | null;
  skillsSelected: SkillReference | null;
  projectsSelected: string | null;
  teamsSelected: string | null;
}

/** Default width of the package detail pane in pixels — the original fixed width. */
export const DETAIL_PANE_DEFAULT_WIDTH = 420;
/** Min allowed width — below this the actions footer crowds up. */
export const DETAIL_PANE_MIN_WIDTH = 320;
/** Storage key for the user's preferred pane width. */
const DETAIL_PANE_WIDTH_KEY = "agency-agents:detail-pane-width";

/** Sidebar width bounds (px). Default matches the original fixed 200px. */
export const SIDEBAR_DEFAULT_WIDTH = 200;
export const SIDEBAR_MIN_WIDTH = 168;
export const SIDEBAR_MAX_WIDTH = 360;
const SIDEBAR_WIDTH_KEY = "agency-agents:sidebar-width";

/** Clamp a sidebar width to its allowed range. */
export function clampSidebarWidth(w: number): number {
  if (!Number.isFinite(w)) return SIDEBAR_DEFAULT_WIDTH;
  return Math.min(Math.max(Math.round(w), SIDEBAR_MIN_WIDTH), SIDEBAR_MAX_WIDTH);
}

/** Storage keys for Settings-modal preferences (Phase 12b). */
const DEFAULT_SECTION_KEY = "agency-agents:default-section";
const VIBRANCY_MATERIAL_KEY = "agency-agents:vibrancy-material";
const CONFIRM_DESTRUCTIVE_KEY = "agency-agents:confirm-destructive";
const ACTIVITY_MAX_JOBS_KEY = "agency-agents:activity:max-jobs";
const ACTIVITY_MAX_LINES_KEY = "agency-agents:activity:max-lines";
const SIDEBAR_COLLAPSED_KEY = "agency-agents:sidebar-collapsed";

/** Defaults for the Activity-retention settings (Phase 12b). */
export const ACTIVITY_MAX_JOBS_DEFAULT = 50;
export const ACTIVITY_MAX_JOBS_MIN = 1;
export const ACTIVITY_MAX_JOBS_MAX = 1000;
export const ACTIVITY_MAX_LINES_DEFAULT = 500;
export const ACTIVITY_MAX_LINES_MIN = 100;
export const ACTIVITY_MAX_LINES_MAX = 10_000;

/** Known vibrancy materials per the macOS NSVisualEffectMaterial enum exposed
    by the `window-vibrancy` crate. Frozen as a const tuple so the type stays
    in sync with `setVibrancyMaterial` and the localStorage validator. */
export const VIBRANCY_MATERIALS = ["HudWindow", "Sidebar", "FullScreenUI", "Off"] as const;
export type VibrancyMaterial = (typeof VIBRANCY_MATERIALS)[number];

/** SidebarSection values we allow as "default landing" — keeps the validator
    in one place. Mirrors the `SidebarSection` union in `types.ts`. */
const DEFAULT_SECTION_VALUES = [
  "dashboard",
  "personas",
  "skills",
  "tools",
  "teams",
  "projects",
  "experts",
  "activity",
] as const;

function clampInt(v: number, lo: number, hi: number, fallback: number): number {
  if (!Number.isFinite(v)) return fallback;
  const n = Math.round(v);
  if (n < lo) return lo;
  if (n > hi) return hi;
  return n;
}

/** Clamp `w` to [min, maxFromWindow]; maxFromWindow defaults to 60% of innerWidth. */
export function clampDetailPaneWidth(w: number, windowWidth?: number): number {
  const ww = windowWidth ?? (typeof window === "undefined" ? 1100 : window.innerWidth);
  const max = Math.max(DETAIL_PANE_MIN_WIDTH, Math.floor(ww * 0.6));
  if (!Number.isFinite(w)) return DETAIL_PANE_DEFAULT_WIDTH;
  return Math.min(Math.max(Math.round(w), DETAIL_PANE_MIN_WIDTH), max);
}

/** The hardcoded first-launch section, before any saved default-landing or
    navigation is applied. `loadDefaultSectionFromStorage` uses this as the
    "untouched" sentinel: it only applies the saved default when `section` is
    still this value (i.e. nothing else has routed yet). Keeping it in one
    constant prevents the guard from drifting away from the initializer — the
    exact bug where the home screen moved to `personas` but the guard still
    checked for `dashboard`, silently disabling the default-landing setting. */
const INITIAL_SECTION: SidebarSection = "personas";

class UiStore {
  /** First-launch landing. The Agents (personas) catalog is the home screen —
      the agent catalog is the front door, not the Dashboard. Clicking the
      sidebar brand returns here. */
  section: SidebarSection = $state(INITIAL_SECTION);

  drawerOpen: boolean = $state(false);
  drawerMinimized: boolean = $state(false);
  /** Transient post-action receipt target consumed by Activity after render. */
  activityReceiptId: string | null = $state(null);
  /** Transient Review Center targets. Domain surfaces consume these; Activity
      never owns approval or review mutations. */
  reviewIntent: ReviewIntent | null = $state(null);
  agentApprovalId: string | null = $state(null);
  skillApprovalId: string | null = $state(null);
  expertReview: ExpertReviewLink | null = $state(null);
  projectRecommendationId: string | null = $state(null);
  /** Transient Recovery targets consumed by existing exact rollback controls. */
  recoveryIntent: RecoveryIntent | null = $state(null);
  agentRecovery: AgentRecoveryLink | null = $state(null);
  skillRecovery: SkillRecoveryLink | null = $state(null);
  paletteOpen: boolean = $state(false);
  /** Settings modal (Phase 12b). Opened via the top-right gear icon or ⌘,. */
  settingsOpen: boolean = $state(false);
  /** Optional initial section to land on when the modal opens. `null`
      means "use the modal's default (Appearance)". Cleared by closeSettings. */
  settingsInitialSection: SettingsSection | null = $state(null);
  /** About modal — native menu "About Agency Agents" + sidebar footer link. */
  aboutOpen: boolean = $state(false);
  /** Playbook modal — "how to get real work out of your agents" (title-bar ?). */
  playbookOpen: boolean = $state(false);
  theme: ThemePreference = $state("system");
  /** Active category ("division") filter for the Agents workspace; null = all.
      Lifted into ui so division pills can deep-link to it and so back/forward
      can restore it. */
  agentsCategory: string | null = $state(null);
  /** Install-state lens for the Agents workspace ("all" | "attention" | "current"
      | "outdated" | "modified" | "foreign" | "missing" | "disabled" | "sourceUnavailable" | "none"). In ui so the Dashboard can
      deep-link a filter and back/forward can restore it. */
  agentsLens: string = $state("all");
  /** Slug of the agent open in the workspace detail pane; null = none. In ui so
      back/forward can restore the open agent. */
  agentsSelected: string | null = $state(null);
  /** Exact source-qualified Agent selected from the library. */
  agentsReference: AgentReference | null = $state(null);
  /** Exact source-qualified Skill selected in the Skills workspace. */
  skillsSelected: SkillReference | null = $state(null);
  /** Tool selected in the Tools console; null = let it auto-pick. Set by the
      Dashboard "Coverage by tool" rows so a click lands on that tool's console. */
  toolsSelected: Tool | null = $state(null);
  /** Absolute path of the project open in the Projects detail pane; null = the
      project list. In ui so the title-bar back/forward restores it. */
  projectsSelected: string | null = $state(null);
  /** Key of the team open in the Teams detail pane (`preset:<slug>` /
      `saved:<id>`); null = the team list. In ui so back/forward restores it. */
  teamsSelected: string | null = $state(null);

  /** Back/forward history of app locations + the cursor into it. */
  navStack: NavLocation[] = $state([]);
  navIndex: number = $state(-1);
  /** True only while applying a back/forward jump, so restoring state doesn't
      push a fresh history entry. Plain (non-reactive) field on purpose. */
  private applyingNav = false;
  canBack = $derived(this.navIndex > 0);
  canForward = $derived(this.navIndex < this.navStack.length - 1);

  /** the package currently shown in the detail panel; null = panel closed */
  selectedPackage: { name: string; kind: "formula" | "cask" } | null = $state(null);
  /** width of the package detail pane in px; persisted to localStorage */
  detailPaneWidth: number = $state(DETAIL_PANE_DEFAULT_WIDTH);

  /** Which section the app opens on at launch. `dashboard` by default; the
      user can change this from Settings → Appearance. Persists to localStorage
      and is applied by `loadDefaultSectionFromStorage` (called from layout
      onMount) — only when the user hasn't already navigated. */
  defaultSection: SidebarSection = $state("personas");

  /** Vibrancy material applied to the macOS window via NSVisualEffectView.
      Restart-required because Tauri 2 applies vibrancy in the setup hook.
      Persisted to localStorage so the next launch reads it. */
  vibrancyMaterial: VibrancyMaterial = $state("HudWindow");

  /** Whether destructive actions require
      a confirm dialog. Defaults true; turning it off is a "trust me" mode
      for power users. */
  confirmDestructive: boolean = $state(true);

  /** Activity persistence caps (Phase 12b). These are the future limits
      for the `activity` store's localStorage mirror. Existing retained
      data is not retroactively trimmed when the user changes these. */
  activityMaxJobs: number = $state(ACTIVITY_MAX_JOBS_DEFAULT);
  activityMaxLines: number = $state(ACTIVITY_MAX_LINES_DEFAULT);

  /** When true, the sidebar collapses to an icon-only rail with native
      tooltips on hover. Persisted to localStorage so the choice survives
      app launches. */
  sidebarCollapsed: boolean = $state(false);

  /** Sidebar width in px (when expanded); persisted to localStorage so a
      resized sidebar survives app launches. */
  sidebarWidth: number = $state(SIDEBAR_DEFAULT_WIDTH);

  setSection(s: SidebarSection) {
    this.clearReviewIntent();
    this.clearRecoveryIntent();
    this.section = s;
    // Navigating to ANY section closes the package detail slide-over and resets
    // the Projects detail to its list (a fresh sidebar click lands on the list).
    this.selectedPackage = null;
    this.projectsSelected = null;
    this.teamsSelected = null;
    this.agentsReference = null;
    this.skillsSelected = null;
    this.commitNav();
  }

  openActivityReceipt(id: string) {
    this.activityReceiptId = id;
    this.setSection("activity");
  }

  private setReviewIntent(kind: ReviewIntentKind, exactId: string, triggerId: string) {
    if (!boundedIntentField(exactId, MAX_INTENT_ID_BYTES)
      || !boundedIntentField(triggerId, MAX_INTENT_TRIGGER_BYTES)) return;
    this.reviewIntent = { origin: "activity-review", kind, exactId, triggerId };
  }

  private validIntent(exactId: string, triggerId: string): boolean {
    return boundedIntentField(exactId, MAX_INTENT_ID_BYTES)
      && boundedIntentField(triggerId, MAX_INTENT_TRIGGER_BYTES);
  }

  clearReviewIntent() {
    this.reviewIntent = null;
    this.agentApprovalId = null;
    this.skillApprovalId = null;
    this.expertReview = null;
    this.projectRecommendationId = null;
  }

  clearRecoveryIntent() {
    this.recoveryIntent = null;
    this.agentRecovery = null;
    this.skillRecovery = null;
  }

  isReviewIntent(kind: ReviewIntentKind, exactId: string): boolean {
    return this.reviewIntent?.origin === "activity-review"
      && this.reviewIntent.kind === kind
      && this.reviewIntent.exactId === exactId;
  }

  openAgentApproval(id: string, triggerId: string) {
    if (!this.validIntent(id, triggerId)) return;
    this.openAgents();
    this.agentApprovalId = id;
    this.setReviewIntent("agent", id, triggerId);
  }

  openSkillApproval(id: string, triggerId: string) {
    if (!this.validIntent(id, triggerId)) return;
    this.setSection("skills");
    this.skillApprovalId = id;
    this.setReviewIntent("skill", id, triggerId);
  }

  openExpertReview(kind: ExpertReviewLink["kind"], id: string, triggerId: string) {
    if (!this.validIntent(id, triggerId)) return;
    this.setSection("experts");
    this.expertReview = { kind, id };
    this.setReviewIntent(`expert-${kind}`, id, triggerId);
  }

  openProjectRecommendation(projectPath: string, id: string, triggerId: string) {
    const exactId = this.projectReviewExactId(projectPath, id);
    if (!this.validIntent(exactId, triggerId)) return;
    this.selectProject(projectPath);
    this.projectRecommendationId = id;
    this.setReviewIntent("project", exactId, triggerId);
  }

  projectReviewExactId(projectPath: string, id: string): string {
    return JSON.stringify([projectPath, id]);
  }

  returnToActivityReview(kind: ReviewIntentKind, exactId: string): boolean {
    if (!this.isReviewIntent(kind, exactId)) return false;
    if (this.canBack) this.back(true);
    else {
      this.section = "activity";
      this.commitNav();
    }
    return true;
  }

  consumeReviewIntent(): ReviewIntent | null {
    const intent = this.reviewIntent;
    this.reviewIntent = null;
    return intent;
  }

  openAgentRecovery(link: AgentRecoveryLink, triggerId: string) {
    const exactId = this.recoveryExactId("agent", link);
    if (!this.validIntent(exactId, triggerId)) return;
    this.openAgentReference(link.reference);
    this.agentRecovery = { ...link, reference: { ...link.reference } };
    this.setRecoveryIntent("agent", exactId, triggerId);
  }

  openSkillRecovery(link: SkillRecoveryLink, triggerId: string) {
    const exactId = this.recoveryExactId("skill", link);
    if (!this.validIntent(exactId, triggerId)) return;
    this.openSkill(link.reference);
    this.skillRecovery = { ...link, reference: { ...link.reference } };
    this.setRecoveryIntent("skill", exactId, triggerId);
  }

  private setRecoveryIntent(kind: RecoveryIntentKind, exactId: string, triggerId: string) {
    if (!boundedIntentField(exactId, MAX_INTENT_ID_BYTES)
      || !boundedIntentField(triggerId, MAX_INTENT_TRIGGER_BYTES)) return;
    this.recoveryIntent = { origin: "settings-recovery", kind, exactId, triggerId };
  }

  recoveryExactId(kind: RecoveryIntentKind, link: AgentRecoveryLink | SkillRecoveryLink): string {
    const destination = "tool" in link ? link.tool : link.runtime;
    return JSON.stringify([kind, link.reference.sourceId, link.reference.relativePath, destination, link.projectPath]);
  }

  isRecoveryIntent(kind: RecoveryIntentKind, exactId: string): boolean {
    return this.recoveryIntent?.origin === "settings-recovery"
      && this.recoveryIntent.kind === kind
      && this.recoveryIntent.exactId === exactId;
  }

  returnToSettingsRecovery(kind: RecoveryIntentKind, exactId: string): boolean {
    if (!this.isRecoveryIntent(kind, exactId)) return false;
    this.agentRecovery = null;
    this.skillRecovery = null;
    if (this.canBack) this.back(true);
    this.openSettings("doctor", true);
    return true;
  }

  consumeRecoveryIntent(): RecoveryIntent | null {
    const intent = this.recoveryIntent;
    this.recoveryIntent = null;
    return intent;
  }

  /** Open the Projects detail pane for a project path (null = back to the list).
      A nav location, so the title-bar back button returns to the list. Also
      switches to the Projects section, so deep-links from the Dashboard
      ("mirror-mesh" row / sunburst slice) actually land on the project. */
  selectProject(path: string | null) { this.clearReviewIntent(); this.clearRecoveryIntent(); this.section = "projects"; this.projectsSelected = path; this.selectedPackage = null; this.commitNav(); }

  /** Open the Teams detail pane for a team key (null = back to the list).
      A nav location, so the title-bar back button returns to the list. */
  selectTeam(key: string | null) { this.clearReviewIntent(); this.clearRecoveryIntent(); this.teamsSelected = key; this.selectedPackage = null; this.commitNav(); }

  /** Open the Tools console with `tool` preselected (null = let it auto-pick). */
  openTools(tool: Tool | null = null) {
    this.toolsSelected = tool;
    this.setSection("tools");
  }

  /** Jump to the unified Agents workspace (optionally within a category);
      resets the open agent. Used by the sidebar, the Dashboard stat cards, and
      the command palette. */
  openAgents(category: string | null = null, lens: string = "all") {
    this.clearReviewIntent();
    this.clearRecoveryIntent();
    this.applyingNav = true;
    this.agentsCategory = category;
    this.skillsSelected = null;
    this.agentsLens = lens;
    this.agentsSelected = null;
    this.agentsReference = null;
    this.skillsSelected = null;
    this.section = "personas";
    this.selectedPackage = null;
    this.projectsSelected = null;
    this.teamsSelected = null;
    this.applyingNav = false;
    this.commitNav();
  }

  /** Open a "division" (category) in the Agents workspace, KEEPING any open
      agent (so clicking an agent's own division shows its siblings beside it).
      Wired to every division pill so a division click anywhere lands on its
      agents. */
  openDivision(category: string) {
    this.clearReviewIntent();
    this.clearRecoveryIntent();
    this.applyingNav = true;
    this.agentsCategory = category;
    this.section = "personas";
    this.selectedPackage = null;
    this.projectsSelected = null;
    this.teamsSelected = null;
    this.applyingNav = false;
    this.commitNav();
  }

  setAgentsCategory(c: string | null) { this.clearReviewIntent(); this.clearRecoveryIntent(); this.agentsCategory = c; this.commitNav(); }
  setAgentsLens(l: string) { this.clearReviewIntent(); this.clearRecoveryIntent(); this.agentsLens = l; this.commitNav(); }
  selectAgent(slug: string | null) { this.agentsSelected = slug; this.agentsReference = null; this.commitNav(); }

  openAgentReference(reference: AgentReference) {
    this.clearReviewIntent();
    this.clearRecoveryIntent();
    this.section = "personas";
    this.agentsSelected = null;
    this.agentsReference = { ...reference };
    this.skillsSelected = null;
    this.projectsSelected = null;
    this.teamsSelected = null;
    this.commitNav();
  }

  selectSkill(reference: SkillReference | null) {
    this.skillsSelected = reference ? { ...reference } : null;
    this.commitNav();
  }

  openSkill(reference: SkillReference) {
    this.clearReviewIntent();
    this.clearRecoveryIntent();
    this.section = "skills";
    this.skillsSelected = { ...reference };
    this.agentsSelected = null;
    this.agentsReference = null;
    this.projectsSelected = null;
    this.teamsSelected = null;
    this.commitNav();
  }

  /** Seed history with the current location — call once at startup, after the
      default landing section has been applied. */
  initNav() { this.commitNav(); }

  back(preserveIntent = false) {
    if (!preserveIntent) { this.clearReviewIntent(); this.clearRecoveryIntent(); }
    if (this.navIndex > 0) {
      this.navIndex -= 1;
      this.applyNav(this.navStack[this.navIndex]);
    }
  }
  forward(preserveIntent = false) {
    if (!preserveIntent) { this.clearReviewIntent(); this.clearRecoveryIntent(); }
    if (this.navIndex < this.navStack.length - 1) {
      this.navIndex += 1;
      this.applyNav(this.navStack[this.navIndex]);
    }
  }

  private snapshotNav(): NavLocation {
    return {
      section: this.section,
      agentsCategory: this.agentsCategory,
      agentsLens: this.agentsLens,
      agentsSelected: this.agentsSelected,
      agentsReference: this.agentsReference ? { ...this.agentsReference } : null,
      skillsSelected: this.skillsSelected ? { ...this.skillsSelected } : null,
      projectsSelected: this.projectsSelected,
      teamsSelected: this.teamsSelected,
    };
  }
  /** Push the current location onto history (truncating any forward entries),
      unless it's identical to the cursor or we're mid back/forward. */
  private commitNav() {
    if (this.applyingNav) return;
    const loc = this.snapshotNav();
    const cur = this.navStack[this.navIndex];
    if (
      cur &&
      cur.section === loc.section &&
      cur.agentsCategory === loc.agentsCategory &&
      cur.agentsLens === loc.agentsLens &&
      cur.agentsSelected === loc.agentsSelected &&
      cur.agentsReference?.sourceId === loc.agentsReference?.sourceId &&
      cur.agentsReference?.relativePath === loc.agentsReference?.relativePath &&
      cur.skillsSelected?.sourceId === loc.skillsSelected?.sourceId &&
      cur.skillsSelected?.relativePath === loc.skillsSelected?.relativePath &&
      cur.projectsSelected === loc.projectsSelected &&
      cur.teamsSelected === loc.teamsSelected
    ) {
      return;
    }
    const next = [...this.navStack.slice(0, this.navIndex + 1), loc];
    const CAP = 100;
    this.navStack = next.length > CAP ? next.slice(next.length - CAP) : next;
    this.navIndex = this.navStack.length - 1;
  }
  private applyNav(loc: NavLocation) {
    this.applyingNav = true;
    this.section = loc.section;
    this.agentsCategory = loc.agentsCategory;
    this.agentsLens = loc.agentsLens;
    this.agentsSelected = loc.agentsSelected;
    this.agentsReference = loc.agentsReference ? { ...loc.agentsReference } : null;
    this.skillsSelected = loc.skillsSelected ? { ...loc.skillsSelected } : null;
    this.projectsSelected = loc.projectsSelected;
    this.teamsSelected = loc.teamsSelected;
    this.selectedPackage = null;
    this.applyingNav = false;
  }

  openDrawer() {
    this.drawerOpen = true;
    this.drawerMinimized = false;
  }

  toggleDrawer() {
    if (this.drawerOpen && !this.drawerMinimized) {
      this.drawerMinimized = true;
    } else if (this.drawerOpen && this.drawerMinimized) {
      this.drawerMinimized = false;
    } else {
      this.drawerOpen = true;
      this.drawerMinimized = false;
    }
  }

  closeDrawer() {
    this.drawerOpen = false;
    this.drawerMinimized = false;
  }

  openPalette() { this.clearReviewIntent(); this.clearRecoveryIntent(); this.paletteOpen = true; }
  closePalette() { this.paletteOpen = false; }

  openSettings(section: SettingsSection | null = null, preserveIntent = false) {
    if (!preserveIntent) { this.clearReviewIntent(); this.clearRecoveryIntent(); }
    this.settingsInitialSection = section;
    this.settingsOpen = true;
  }
  closeSettings() {
    this.settingsOpen = false;
    this.settingsInitialSection = null;
  }

  openAbout() { this.clearReviewIntent(); this.clearRecoveryIntent(); this.aboutOpen = true; }
  closeAbout() { this.aboutOpen = false; }

  openPlaybook() { this.clearReviewIntent(); this.clearRecoveryIntent(); this.playbookOpen = true; }
  closePlaybook() { this.playbookOpen = false; }

  // ---------------- Settings (Phase 12b) ----------------

  /** Persist a new default landing section. Writes to localStorage; the
      runtime application happens at next app launch via
      `loadDefaultSectionFromStorage`. */
  setDefaultSection(s: SidebarSection) {
    this.defaultSection = s;
    try { localStorage.setItem(DEFAULT_SECTION_KEY, s); } catch { /* ignore */ }
  }

  /** On first paint, read the saved default-landing and (if the user
      hasn't already navigated) override `section`. We treat the hardcoded
      `INITIAL_SECTION` as "untouched" — if some early code already routed
      the user elsewhere we leave it alone. Validates against the known
      enum on read per Phase 12 security review § 12b. */
  loadDefaultSectionFromStorage() {
    try {
      const stored = localStorage.getItem(DEFAULT_SECTION_KEY);
      const v = stored === "runbooks" ? "experts" : stored;
      if (v !== null && (DEFAULT_SECTION_VALUES as readonly string[]).includes(v)) {
        const validated = v as SidebarSection;
        this.defaultSection = validated;
        // Only override the initial section if it's still the hardcoded
        // landing — anything else means something already routed.
        if (this.section === INITIAL_SECTION) {
          this.section = validated;
        }
      }
    } catch { /* ignore */ }
  }

  /** Persist a new vibrancy material. The active material does not change
      until the app is restarted (NSVisualEffectView is applied once in
      the Tauri setup hook). */
  setVibrancyMaterial(m: VibrancyMaterial) {
    this.vibrancyMaterial = m;
    try { localStorage.setItem(VIBRANCY_MATERIAL_KEY, m); } catch { /* ignore */ }
  }

  loadVibrancyMaterialFromStorage() {
    try {
      const v = localStorage.getItem(VIBRANCY_MATERIAL_KEY);
      if (v !== null && (VIBRANCY_MATERIALS as readonly string[]).includes(v)) {
        this.vibrancyMaterial = v as VibrancyMaterial;
      }
    } catch { /* ignore */ }
  }

  setConfirmDestructive(v: boolean) {
    this.confirmDestructive = v;
    try { localStorage.setItem(CONFIRM_DESTRUCTIVE_KEY, v ? "1" : "0"); } catch { /* ignore */ }
  }

  loadConfirmDestructiveFromStorage() {
    try {
      const v = localStorage.getItem(CONFIRM_DESTRUCTIVE_KEY);
      if (v === "0") this.confirmDestructive = false;
      else if (v === "1") this.confirmDestructive = true;
    } catch { /* ignore */ }
  }

  setActivityMaxJobs(n: number) {
    const clamped = clampInt(n, ACTIVITY_MAX_JOBS_MIN, ACTIVITY_MAX_JOBS_MAX, ACTIVITY_MAX_JOBS_DEFAULT);
    this.activityMaxJobs = clamped;
    try { localStorage.setItem(ACTIVITY_MAX_JOBS_KEY, String(clamped)); } catch { /* ignore */ }
  }

  setActivityMaxLines(n: number) {
    const clamped = clampInt(n, ACTIVITY_MAX_LINES_MIN, ACTIVITY_MAX_LINES_MAX, ACTIVITY_MAX_LINES_DEFAULT);
    this.activityMaxLines = clamped;
    try { localStorage.setItem(ACTIVITY_MAX_LINES_KEY, String(clamped)); } catch { /* ignore */ }
  }

  /** Load both Activity-retention caps with clamp-on-read so a corrupted
      or hostile localStorage entry can't ask us to keep "999999999 jobs". */
  loadActivitySettingsFromStorage() {
    try {
      const j = localStorage.getItem(ACTIVITY_MAX_JOBS_KEY);
      if (j !== null) {
        const n = Number(j);
        this.activityMaxJobs = clampInt(n, ACTIVITY_MAX_JOBS_MIN, ACTIVITY_MAX_JOBS_MAX, ACTIVITY_MAX_JOBS_DEFAULT);
      }
      const l = localStorage.getItem(ACTIVITY_MAX_LINES_KEY);
      if (l !== null) {
        const n = Number(l);
        this.activityMaxLines = clampInt(n, ACTIVITY_MAX_LINES_MIN, ACTIVITY_MAX_LINES_MAX, ACTIVITY_MAX_LINES_DEFAULT);
      }
    } catch { /* ignore */ }
  }

  /** Toggle the sidebar between full-width and icon-only mode. Persists
      to localStorage so the choice survives launches. */
  toggleSidebarCollapsed() {
    this.sidebarCollapsed = !this.sidebarCollapsed;
    try {
      localStorage.setItem(SIDEBAR_COLLAPSED_KEY, this.sidebarCollapsed ? "1" : "0");
    } catch { /* ignore */ }
  }

  /** Restore the saved collapsed state on app start. Called once from
      +layout.svelte after the DOM is available. */
  loadSidebarCollapsedFromStorage() {
    try {
      const v = localStorage.getItem(SIDEBAR_COLLAPSED_KEY);
      if (v !== null) this.sidebarCollapsed = v === "1";
    } catch { /* ignore */ }
  }

  selectPackage(name: string, kind: "formula" | "cask") {
    this.selectedPackage = { name, kind };
  }

  closeDetail() {
    this.selectedPackage = null;
  }

  setTheme(t: ThemePreference) {
    this.theme = t;
    try { localStorage.setItem("agency-agents.theme", t); } catch { /* ignore */ }
    applyTheme(t);
  }

  loadThemeFromStorage() {
    try {
      const v = localStorage.getItem("agency-agents.theme");
      if (v === "light" || v === "dark" || v === "system") {
        this.theme = v;
      }
    } catch { /* ignore */ }
    applyTheme(this.theme);
  }

  /** Load persisted detail-pane width on app mount; clamps in case window shrank since. */
  loadDetailPaneWidthFromStorage() {
    try {
      const raw = localStorage.getItem(DETAIL_PANE_WIDTH_KEY);
      if (raw != null) {
        const n = Number(raw);
        if (Number.isFinite(n)) this.detailPaneWidth = clampDetailPaneWidth(n);
      }
    } catch { /* ignore */ }
  }

  /** Set + persist; clamps to [min, 60% of window width]. */
  setDetailPaneWidth(w: number) {
    this.detailPaneWidth = clampDetailPaneWidth(w);
    try { localStorage.setItem(DETAIL_PANE_WIDTH_KEY, String(this.detailPaneWidth)); } catch { /* ignore */ }
  }

  /** Reset to default width (used by double-clicking the resize handle). */
  resetDetailPaneWidth() {
    this.setDetailPaneWidth(DETAIL_PANE_DEFAULT_WIDTH);
  }

  /** Load persisted sidebar width on app mount. */
  loadSidebarWidthFromStorage() {
    try {
      const raw = localStorage.getItem(SIDEBAR_WIDTH_KEY);
      if (raw != null) {
        const n = Number(raw);
        if (Number.isFinite(n)) this.sidebarWidth = clampSidebarWidth(n);
      }
    } catch { /* ignore */ }
  }

  /** Set + persist the sidebar width (clamped to its allowed range). */
  setSidebarWidth(w: number) {
    this.sidebarWidth = clampSidebarWidth(w);
    try { localStorage.setItem(SIDEBAR_WIDTH_KEY, String(this.sidebarWidth)); } catch { /* ignore */ }
  }

  /** Reset to default sidebar width (double-click the resize handle). */
  resetSidebarWidth() {
    this.setSidebarWidth(SIDEBAR_DEFAULT_WIDTH);
  }
}

function applyTheme(t: ThemePreference) {
  if (typeof document === "undefined") return;
  const html = document.documentElement;
  let resolved: "light" | "dark";
  if (t === "system") {
    resolved = window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  } else {
    resolved = t;
  }
  html.dataset.theme = resolved;
}

/** Subscribe matchMedia to flip data-theme when "system" is selected. */
export function watchSystemTheme(getCurrent: () => ThemePreference) {
  if (typeof window === "undefined") return () => {};
  const mq = window.matchMedia("(prefers-color-scheme: dark)");
  const handler = () => {
    if (getCurrent() === "system") {
      document.documentElement.dataset.theme = mq.matches ? "dark" : "light";
    }
  };
  mq.addEventListener("change", handler);
  return () => mq.removeEventListener("change", handler);
}

export const ui = new UiStore();
