<script lang="ts">
  import { onMount, tick } from "svelte";
  import ActivityIcon from "@lucide/svelte/icons/activity";
  import DownloadIcon from "@lucide/svelte/icons/download";
  import Trash2 from "@lucide/svelte/icons/trash-2";
  import RefreshIcon from "@lucide/svelte/icons/refresh-cw";
  import PlusIcon from "@lucide/svelte/icons/plus";
  import ToggleRightIcon from "@lucide/svelte/icons/toggle-right";
  import LayersIcon from "@lucide/svelte/icons/layers";

  import Button from "./Button.svelte";
  import EmptyState from "./EmptyState.svelte";
  import { activity, safeActivityDetail, type JournalEntry } from "$lib/stores/activity.svelte";
  import { i18n } from "$lib/stores/i18n.svelte";
  import { install } from "$lib/stores/install.svelte";
  import { ui } from "$lib/stores/ui.svelte";
  import type { MessageKey } from "$lib/i18n/messages";
  import {
    agentLibraryList,
    expertActivationRequests,
    expertCreationRequests,
    expertRunsList,
    projectReadinessGet,
    projectRecommendationsList,
    projectsList,
    skillFoldersList,
  } from "$lib/api";

  type ReviewSource = "agent" | "skill" | "expert-change" | "expert-run" | "expert-activation" | "recommendation";
  type ReviewItem = {
    id: string;
    source: ReviewSource;
    label: string;
    meta: string;
    projectPath?: string;
  };
  type SourceState = { status: "loading" | "ready" | "unavailable"; items: ReviewItem[]; error: string };

  const sourceState = (): SourceState => ({ status: "loading", items: [], error: "" });
  let mode = $state<"review" | "history">(ui.activityReceiptId ? "history" : "review");
  let modeAnnouncement = $state(ui.activityReceiptId ? "History mode" : "Review mode");
  let review: Record<ReviewSource, SourceState> = $state({
    agent: sourceState(), skill: sourceState(), "expert-change": sourceState(),
    "expert-run": sourceState(), "expert-activation": sourceState(), recommendation: sourceState(),
  });
  const approvalSources: ReviewSource[] = ["agent", "skill", "expert-change", "expert-run", "expert-activation"];
  const approvalTotal = $derived(approvalSources.reduce((count, source) =>
    count + (review[source].status === "ready" ? review[source].items.length : 0), 0));
  const reviewPartial = $derived([...approvalSources, "recommendation" as const]
    .some((source) => review[source].status !== "ready"));

  onMount(() => {
    void activity.refreshMcpAudit();
    void Promise.all([
      loadSource("agent"), loadSource("skill"), loadSource("expert-change"),
      loadSource("expert-run"), loadSource("expert-activation"), loadSource("recommendation"),
    ]);
  });

  let root: HTMLElement | undefined = $state();
  let receiptAnnouncement = $state("");
  let reviewAnnouncement = $state("");

  function setMode(next: "review" | "history") {
    mode = next;
    modeAnnouncement = next === "review" ? "Review mode" : "History mode";
  }

  function sourceLabel(source: ReviewSource): string {
    if (source === "agent") return "Agent approvals";
    if (source === "skill") return "Skill approvals";
    if (source === "expert-change") return "Expert change requests";
    if (source === "expert-run") return "Expert runs awaiting review";
    if (source === "expert-activation") return "Expert activation requests";
    return "Subscription Recommendations";
  }

  async function readSource(source: ReviewSource): Promise<ReviewItem[]> {
    if (source === "agent") {
      return (await agentLibraryList()).approvals.filter((item) => item.state === "pending").map((item) => ({
        id: item.id, source, label: `Agent ${item.request.action}`, meta: `Requested by ${item.requestedBy}`,
      }));
    }
    if (source === "skill") {
      return (await skillFoldersList()).approvals.filter((item) => item.state === "pending").map((item) => ({
        id: item.id, source, label: `Skill ${item.request.action}`, meta: `Requested by ${item.requestedBy}`,
      }));
    }
    if (source === "expert-change") {
      return (await expertCreationRequests()).filter((item) => item.state === "pending").map((item) => ({
        id: item.id, source, label: `${item.kind} ${item.proposal.name}`, meta: `Requested by ${item.requestedBy}`,
      }));
    }
    if (source === "expert-run") {
      return (await expertRunsList()).filter((item) => item.state === "awaitingReview").map((item) => ({
        id: item.id, source, label: `Run ${item.id.slice(0, 8)}`, meta: item.expertId,
      }));
    }
    if (source === "expert-activation") {
      return (await expertActivationRequests()).filter((item) => item.state === "pending").map((item) => ({
        id: item.id, source, label: `Activate ${item.expertId}`, meta: `Requested by ${item.requestedBy}`,
      }));
    }
    const registered = await projectsList();
    const labels = new Map(registered.map((project) => [project.path, project.label]));
    const subscribed = (await Promise.all(registered.map((project) => projectReadinessGet(project.path))))
      .filter((report) => report.subscribed);
    const lists = await Promise.all(subscribed.map((report) => projectRecommendationsList(report.projectPath)));
    return lists.flatMap((items, index) => items
      .filter((item) => ["new", "pending"].includes(item.lifecycle))
      .map((item) => ({
        id: item.id, source, label: item.summary, meta: labels.get(subscribed[index].projectPath) ?? "Subscribed project",
        projectPath: subscribed[index].projectPath,
      })));
  }

  async function loadSource(source: ReviewSource): Promise<"ready" | "unavailable"> {
    review = { ...review, [source]: { ...review[source], status: "loading", error: "" } };
    reviewAnnouncement = `${sourceLabel(source)} loading.`;
    try {
      const items = await readSource(source);
      review = { ...review, [source]: { status: "ready", items, error: "" } };
      reviewAnnouncement = `${sourceLabel(source)} ready. ${items.length} pending.`;
      return "ready";
    } catch (error) {
      const detail = safeActivityDetail(error);
      review = { ...review, [source]: { ...review[source], status: "unavailable", error: detail } };
      reviewAnnouncement = `${sourceLabel(source)} unavailable. ${detail}`;
      return "unavailable";
    }
  }

  async function retrySource(source: ReviewSource) {
    const status = await loadSource(source);
    await tick();
    const group = [...(root?.querySelectorAll<HTMLElement>("[data-review-group]") ?? [])]
      .find((candidate) => candidate.dataset.reviewGroup === source);
    group?.querySelector<HTMLElement>("button, h2")?.focus({ preventScroll: true });
    if (status === "ready") reviewAnnouncement = `${sourceLabel(source)} refreshed.`;
  }

  function openReview(item: ReviewItem, triggerId: string) {
    if (item.source === "agent") ui.openAgentApproval(item.id, triggerId);
    else if (item.source === "skill") ui.openSkillApproval(item.id, triggerId);
    else if (item.source === "expert-change") ui.openExpertReview("change", item.id, triggerId);
    else if (item.source === "expert-run") ui.openExpertReview("run", item.id, triggerId);
    else if (item.source === "expert-activation") ui.openExpertReview("activation", item.id, triggerId);
    else if (item.projectPath) ui.openProjectRecommendation(item.projectPath, item.id, triggerId);
  }

  $effect(() => {
    const id = ui.activityReceiptId;
    if (!id) return;
    if (mode !== "history") setMode("history");
    void tick().then(() => {
      if (ui.activityReceiptId !== id) return;
      const details = [...(root?.querySelectorAll<HTMLDetailsElement>("details[data-activity-id]") ?? [])]
        .find((candidate) => candidate.dataset.activityId === id);
      if (!details) {
        receiptAnnouncement = i18n.optional("activity.receiptUnavailable", "Receipt is no longer available.");
        ui.activityReceiptId = null;
        return;
      }
      details.open = true;
      details.scrollIntoView?.({ block: "center" });
      details.querySelector<HTMLElement>("summary")?.focus({ preventScroll: true });
      receiptAnnouncement = i18n.optional("activity.receiptOpened", "Receipt opened in Activity.");
      ui.activityReceiptId = null;
    });
  });

  $effect(() => {
    const intent = ui.reviewIntent;
    if (ui.section !== "activity" || !intent) return;
    const allSettled = Object.values(review).every((state) => state.status !== "loading");
    void tick().then(() => {
      if (ui.reviewIntent !== intent || ui.section !== "activity" || !root?.isConnected) return;
      const trigger = [...(root?.querySelectorAll<HTMLButtonElement>("[data-review-trigger]") ?? [])]
        .find((candidate) => candidate.dataset.reviewTrigger === intent.triggerId);
      const source = intent.kind === "project" ? "recommendation" : intent.kind as ReviewSource;
      const fallback = [...(root?.querySelectorAll<HTMLElement>("[data-review-group]") ?? [])]
        .find((candidate) => candidate.dataset.reviewGroup === source)
        ?.querySelector<HTMLElement>("h2");
      const target = trigger ?? (allSettled ? fallback : null);
      if (!target) return;
      target.focus({ preventScroll: true });
      ui.consumeReviewIntent();
      receiptAnnouncement = trigger ? "Returned to Review." : "Review item is no longer available.";
    });
  });

  /** Lucide icon per journal action. */
  const ACTION_ICON = {
    install: DownloadIcon,
    uninstall: Trash2,
    update: RefreshIcon,
    disable: ToggleRightIcon,
    enable: ToggleRightIcon,
    sourceAdd: PlusIcon,
    sourceRefresh: RefreshIcon,
    sourceRemove: Trash2,
    draftCreate: PlusIcon,
    draftEdit: RefreshIcon,
    draftPublish: DownloadIcon,
    draftReject: Trash2,
    organize: LayersIcon,
    rollback: RefreshIcon,
    approvalApprove: ToggleRightIcon,
    approvalReject: Trash2,
    track: PlusIcon,
    switch: ToggleRightIcon,
    sync: RefreshIcon,
    bulk: LayersIcon,
    mcp: ActivityIcon,
  } as const;

  /** Sentence-case verb shown at the head of each row. */
  const ACTION_VERB: Record<Exclude<JournalEntry["action"], "mcp">, MessageKey> = {
    install: "activity.action.install",
    uninstall: "activity.action.uninstall",
    update: "activity.action.update",
    disable: "activity.action.disable",
    enable: "activity.action.enable",
    sourceAdd: "activity.action.sourceAdd",
    sourceRefresh: "activity.action.sourceRefresh",
    sourceRemove: "activity.action.sourceRemove",
    draftCreate: "activity.action.draftCreate",
    draftEdit: "activity.action.draftEdit",
    draftPublish: "activity.action.draftPublish",
    draftReject: "activity.action.draftReject",
    organize: "activity.action.organize",
    rollback: "activity.action.rollback",
    approvalApprove: "activity.action.approvalApprove",
    approvalReject: "activity.action.approvalReject",
    track: "activity.action.track",
    switch: "activity.action.switch",
    sync: "activity.action.sync",
    bulk: "activity.action.bulk",
  };

  /** Basename of a project path, for the " · project" suffix. */
  function basename(p: string): string {
    const parts = p.replace(/[\\/]+$/, "").split(/[\\/]/);
    return parts[parts.length - 1] || p;
  }

  /** Human row text, tuned per action so target-less / summary ops read
      naturally instead of "Verb + sentence-fragment". */
  function rowText(e: JournalEntry): string {
    const tool = e.tool ? install.toolLabel(e.tool) : "";
    if (e.action === "mcp") {
      return `${e.subjectName ?? "MCP"} · ${e.detail ?? "read"}`;
    }
    // Default-target toggle: the tool IS the subject, detail is the descriptor.
    if (e.action === "switch") {
      return tool ? `${tool} · ${e.detail ?? i18n.t("common.defaultTargetChanged")}` : (e.detail ?? i18n.t("common.defaultTargetChanged"));
    }
    // Batch sweeps: detail is already a self-contained phrase ("Updated 3 agents").
    if (e.action === "sync" || e.action === "bulk") {
      return e.detail ?? i18n.t(ACTION_VERB[e.action]);
    }
    // Single-agent ops: "Verb agent → Tool · project".
    let s = `${i18n.t(ACTION_VERB[e.action])} ${e.subjectName ?? e.agentName ?? e.agentSlug ?? ""}`.trim();
    if (tool) s += ` → ${tool}`;
    if (e.scope === "project" && e.projectPath) s += ` · ${basename(e.projectPath)}`;
    return s;
  }

  // ─── Day grouping (Today / Yesterday / locale date) ───────────────────────
  const DAY_MS = 86_400_000;
  function dayKey(iso: string): number {
    const d = new Date(iso);
    return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  }
  function dayLabel(key: number): string {
    const today = (() => {
      const n = new Date();
      return new Date(n.getFullYear(), n.getMonth(), n.getDate()).getTime();
    })();
    if (key === today) return i18n.t("common.today");
    if (key === today - DAY_MS) return i18n.t("common.yesterday");
    return new Date(key).toLocaleDateString(undefined, {
      weekday: "short",
      month: "short",
      day: "numeric",
    });
  }

  /** Entries are already newest-first; bucket them into ordered day groups. */
  const groups = $derived.by<{ key: number; label: string; entries: JournalEntry[] }[]>(() => {
    const out: { key: number; label: string; entries: JournalEntry[] }[] = [];
    let current: { key: number; label: string; entries: JournalEntry[] } | null = null;
    for (const e of activity.entries) {
      const k = dayKey(e.ts);
      if (!current || current.key !== k) {
        current = { key: k, label: dayLabel(k), entries: [] };
        out.push(current);
      }
      current.entries.push(e);
    }
    return out;
  });

  // ─── Relative timestamp ("just now" / "5m" / "3h" / locale time) ──────────
  function relTime(iso: string): string {
    const diff = Date.now() - new Date(iso).getTime();
    const sec = Math.floor(diff / 1000);
    if (sec < 45) return i18n.t("common.justNow");
    const min = Math.floor(sec / 60);
    if (min < 60) return `${min}m`;
    const hr = Math.floor(min / 60);
    if (hr < 24) return `${hr}h`;
    return new Date(iso).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  }
</script>

<section class="hist" bind:this={root}>
  <div class="sr-only" role="status" aria-live="polite" aria-atomic="true">{receiptAnnouncement || reviewAnnouncement} {modeAnnouncement}</div>
  <header class="panel-head" data-tauri-drag-region>
    <div class="modes" data-tauri-drag-region="false" aria-label="Activity mode">
      <button type="button" aria-pressed={mode === "review"} onclick={() => setMode("review")}>Review</button>
      <button type="button" aria-pressed={mode === "history"} onclick={() => setMode("history")}>History</button>
    </div>
    {#if mode === "history" && activity.hasLocalEntries}
      <span class="action-wrap" data-tauri-drag-region="false">
        <Button size="sm" variant="ghost" onclick={() => activity.clear()}>
          {#snippet icon()}<Trash2 size={14} />{/snippet}
          {i18n.t("activity.clearLocal")}
        </Button>
      </span>
    {/if}
  </header>

  <div class="list-wrap">
    {#if mode === "review"}
      <div class="review-summary">{approvalTotal} pending{reviewPartial ? " · partial" : ""}</div>
      {#each approvalSources as source (source)}
        {@const state = review[source]}
        <section class="review-group" data-review-group={source} aria-busy={state.status === "loading"}>
          <h2 tabindex="-1">{sourceLabel(source)} {state.status === "ready" ? state.items.length : ""} <span>{state.status === "ready" ? "Ready" : state.status === "loading" ? "Loading" : "Unavailable"}</span></h2>
          {#if state.status === "unavailable"}
            <p class="review-error" role="alert">{state.error}</p>
            <button type="button" data-review-retry onclick={() => void retrySource(source)}>Retry</button>
          {:else if state.status === "ready" && state.items.length === 0}
            <p class="review-empty">Nothing pending.</p>
          {:else}
            <ul class="review-list">
              {#each state.items as item (item.id)}
                {@const triggerId = `${source}:${item.id}`}
                <li><div><strong>{item.label}</strong><span>{item.meta}</span></div><button type="button" data-review-source={source} data-review-trigger={triggerId} onclick={() => openReview(item, triggerId)}>Open review</button></li>
              {/each}
            </ul>
          {/if}
        </section>
      {/each}
      {@const recommendations = review.recommendation}
      <section class="review-group recommendations" data-review-group="recommendation" aria-busy={recommendations.status === "loading"}>
        <h2 tabindex="-1">{sourceLabel("recommendation")} {recommendations.status === "ready" ? recommendations.items.length : ""} <span>{recommendations.status === "ready" ? "Ready" : recommendations.status === "loading" ? "Loading" : "Unavailable"}</span></h2>
        {#if recommendations.status === "unavailable"}
          <p class="review-error" role="alert">{recommendations.error}</p><button type="button" data-review-retry onclick={() => void retrySource("recommendation")}>Retry</button>
        {:else if recommendations.status === "ready" && recommendations.items.length === 0}<p class="review-empty">No subscription recommendations.</p>
        {:else}<ul class="review-list">{#each recommendations.items as item (item.id)}{@const triggerId = `recommendation:${item.id}`}<li><div><strong>{item.label}</strong><span>{item.meta}</span></div><button type="button" data-review-source="recommendation" data-review-trigger={triggerId} onclick={() => openReview(item, triggerId)}>Open recommendation</button></li>{/each}</ul>{/if}
      </section>
    {:else if activity.entries.length === 0}
      <EmptyState
        title={i18n.t("activity.emptyTitle")}
        body={i18n.t("activity.emptyBody")}
      >
        {#snippet icon()}<ActivityIcon size={48} />{/snippet}
      </EmptyState>
    {:else}
      {#each groups as group (group.key)}
        <h2 class="day">{group.label}</h2>
        <ul class="list">
          {#each group.entries as e (e.id)}
            {@const Icon = ACTION_ICON[e.action]}
            <li class="row" class:with-receipt={!!e.receipt}>
              <span class="ico" aria-hidden="true"><Icon size={15} /></span>
              <div class="text" title={e.outcome === "error" && e.detail ? e.detail : rowText(e)}>
                <span class="truncate">{rowText(e)}</span>
                {#if e.receipt}
                  <details data-activity-id={e.id}>
                    <summary>
                      {i18n.optional("activity.receiptDetails", "Receipt details")}
                      <span class="receipt-counts">{i18n.optional("activity.receiptSummary", "{succeeded} succeeded · {failed} failed", { succeeded: e.receipt.succeeded, failed: e.receipt.failed })}</span>
                    </summary>
                    <ul class="receipt-items">
                      {#each e.receipt.items as item, index (`${item.kind}:${item.destination}:${index}`)}
                        <li>
                          <span class="receipt-name">{item.name}</span>
                          <span class="receipt-meta">{item.kind === "agent" ? "Agent" : "Skill"} · {item.outcome === "ok" ? i18n.t("common.succeeded") : i18n.t("common.failed")}</span>
                          <span class="receipt-path" title={item.destination ?? undefined}>{item.destination ?? i18n.optional("activity.destinationUnavailable", "No destination was changed or returned")}</span>
                          {#if item.detail}<span class="receipt-error">{item.detail}</span>{/if}
                        </li>
                      {/each}
                    </ul>
                  </details>
                {/if}
              </div>
              <span class="time" title={new Date(e.ts).toLocaleString()}>{relTime(e.ts)}</span>
              <span
                class="status-dot"
                class:ok={e.outcome === "ok"}
                class:fail={e.outcome === "error"}
                aria-label={e.outcome === "pending"
                  ? i18n.t("activity.pending")
                  : e.outcome === "error"
                    ? i18n.t("common.failed")
                    : i18n.t("common.succeeded")}
                title={e.outcome === "pending"
                  ? i18n.t("activity.pending")
                  : e.outcome === "error"
                    ? (e.detail ?? i18n.t("common.failed"))
                    : i18n.t("common.succeeded")}
              ></span>
            </li>
          {/each}
        </ul>
      {/each}
    {/if}
  </div>
</section>

<style>
  .hist { display: flex; flex-direction: column; min-height: 0; height: 100%; }
  .panel-head {
    display: flex; justify-content: space-between; align-items: center;
    padding: var(--space-4);
    border-bottom: 1px solid var(--color-border);
  }
  .modes { display: flex; gap: var(--space-1); }
  .modes button, .review-group > button, .review-list button {
    padding: 6px var(--space-3); border: 1px solid var(--color-border); border-radius: var(--radius-md);
    color: var(--color-text-primary); background: var(--color-surface-raised); cursor: pointer;
  }
  .modes button[aria-pressed="true"] { color: var(--color-text-inverse); background: var(--color-brand); border-color: var(--color-brand); }
  .modes button:focus-visible, .review-group > button:focus-visible, .review-list button:focus-visible { outline: 2px solid var(--color-brand); outline-offset: 2px; }
  .review-summary { padding: var(--space-3) var(--space-4); font-weight: var(--fw-semibold); border-bottom: 1px solid var(--color-border); }
  .review-group { padding: var(--space-3) var(--space-4); border-bottom: 1px solid var(--color-border); }
  .review-group h2 { font-size: var(--text-body); font-weight: var(--fw-semibold); }
  .review-group h2 span { margin-left: var(--space-1); color: var(--color-text-muted); font-size: var(--text-caption); font-weight: var(--fw-medium); }
  .review-list { display: grid; gap: var(--space-2); margin-top: var(--space-2); }
  .review-list li { display: flex; align-items: center; justify-content: space-between; gap: var(--space-3); }
  .review-list li div { min-width: 0; display: grid; }
  .review-list span, .review-empty { color: var(--color-text-muted); font-size: var(--text-body-sm); }
  .review-error { margin: var(--space-2) 0; color: var(--color-danger); font-size: var(--text-body-sm); overflow-wrap: anywhere; }
  .recommendations { border-left: 3px solid var(--color-brand); }

  .list-wrap { flex: 1; overflow-y: auto; min-height: 0; }

  .day {
    position: sticky;
    top: 0;
    z-index: 1;
    padding: var(--space-2) var(--space-3);
    background: var(--color-surface);
    color: var(--color-text-muted);
    font-size: var(--text-caption);
    font-weight: var(--fw-semibold);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    border-bottom: 1px solid var(--color-border);
  }

  .list { display: flex; flex-direction: column; }
  .row {
    display: grid;
    grid-template-columns: 24px 1fr auto 10px;
    align-items: center;
    width: 100%;
    padding: var(--space-2) var(--space-3);
    min-height: 36px;
    text-align: left;
    color: var(--color-text-primary);
    font-size: var(--text-body);
    border-bottom: 1px solid var(--color-border);
    gap: var(--space-3);
  }

  .row.with-receipt { align-items: start; }
  .text { min-width: 0; font-weight: var(--fw-medium); }
  details { margin-top: 2px; font-weight: var(--fw-normal); }
  summary { width: fit-content; color: var(--color-brand); font-size: var(--text-body-sm); cursor: pointer; }
  summary:focus-visible { outline: 2px solid var(--color-brand); outline-offset: 2px; border-radius: var(--radius-sm); }
  .receipt-counts { margin-left: var(--space-2); color: var(--color-text-muted); }
  .receipt-items { display: grid; gap: var(--space-2); margin-top: var(--space-2); }
  .receipt-items li { display: grid; gap: 2px; padding: var(--space-2); border-radius: var(--radius-sm); background: var(--color-surface-sunken); }
  .receipt-name { font-weight: var(--fw-medium); }
  .receipt-meta, .receipt-path, .receipt-error { color: var(--color-text-muted); font-size: var(--text-body-sm); }
  .receipt-path { overflow-wrap: anywhere; }
  .receipt-error { color: var(--color-danger); }

  .ico { display: inline-flex; color: var(--color-text-muted); }
  .time {
    text-align: right;
    color: var(--color-text-muted);
    font-size: var(--text-body-sm);
    font-variant-numeric: tabular-nums;
  }

  .status-dot {
    width: 8px; height: 8px;
    border-radius: var(--radius-full);
    background: var(--color-text-muted);
  }
  .status-dot.ok   { background: var(--color-success); }
  .status-dot.fail { background: var(--color-danger); }
  @media (max-width: 600px) {
    .panel-head { flex-wrap: wrap; gap: var(--space-2); padding: var(--space-3); }
    .review-summary, .review-group { padding-left: var(--space-3); padding-right: var(--space-3); }
    .review-list li { align-items: stretch; flex-direction: column; gap: var(--space-2); }
    .review-list button { align-self: flex-start; }
    .row { grid-template-columns: 20px minmax(0, 1fr) auto 8px; gap: var(--space-2); padding: var(--space-2); }
    .receipt-counts { display: block; margin-left: 0; }
  }
</style>
