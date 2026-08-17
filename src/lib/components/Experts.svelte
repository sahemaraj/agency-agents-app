<script lang="ts">
  import { onMount } from "svelte";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import Search from "@lucide/svelte/icons/search";
  import Sparkles from "@lucide/svelte/icons/sparkles";
  import Copy from "@lucide/svelte/icons/copy";
  import Plus from "@lucide/svelte/icons/plus";
  import Modal from "./Modal.svelte";
  import Runbooks from "./Runbooks.svelte";
  import { experts, summarizeExpertPerformance } from "$lib/stores/experts.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { corpus } from "$lib/stores/corpus.svelte";
  import { ui } from "$lib/stores/ui.svelte";
  import { teams } from "$lib/stores/teams.svelte";
  import { activity } from "$lib/stores/activity.svelte";
  import { toast } from "$lib/stores/toast.svelte";
  import {
    appErrorMessage,
    isAppError,
    type ExpertActivationPlan,
    type ExpertClient,
    type ExpertCreationRequest,
    type ExpertDefinition,
    type ExpertProposalInput,
    type ExpertResolved,
    type ExpertRun,
  } from "$lib/types";

  onMount(() => {
    void Promise.all([experts.load(), projects.refresh(), corpus.ensureLoaded()]);
  });

  let tab = $state<"experts" | "drafts" | "runs" | "runbooks">("experts");
  let query = $state("");
  let filter = $state<"all" | "ready" | "setup" | "custom" | "recent">("all");
  let projectPath = $state("");
  let client = $state<ExpertClient | "">("");
  let plan = $state<ExpertActivationPlan | null>(null);
  let planning = $state(false);
  let activating = $state(false);
  let builder = $state<ExpertDefinition | null>(null);
  let creationReview = $state<ExpertCreationRequest | null>(null);
  let runReview = $state<ExpertRun | null>(null);
  let waiverReason = $state("");
  const pendingRequests = $derived(experts.requests.filter((request) => request.state === "pending"));

  const knownAgents = $derived(new Map(corpus.agents.map((agent) => [agent.slug, agent])));
  function ready(expert: ExpertResolved): boolean {
    return expert.unresolvedAgents.length === 0 && expert.unresolvedSkills.length === 0 && !expert.unresolvedRunbook;
  }
  const visible = $derived(experts.list.filter((expert) => {
    const hay = `${expert.name} ${expert.summary} ${expert.category} ${expert.tags.join(" ")}`.toLowerCase();
    if (query && !hay.includes(query.trim().toLowerCase())) return false;
    if (filter === "ready" && !ready(expert)) return false;
    if (filter === "setup" && ready(expert)) return false;
    if (filter === "custom" && expert.source !== "custom") return false;
    if (filter === "recent" && !experts.history.some((record) => record.expertId === expert.id)) return false;
    return true;
  }));

  function newExpert(): ExpertDefinition {
    return {
      id: `custom-${Date.now()}`,
      name: "New Expert",
      summary: "",
      category: "Custom",
      tags: [],
      version: 1,
      leadAgent: corpus.agents[0]?.slug ?? "",
      supportingAgents: [],
      requiredSkills: [],
      optionalSkills: [],
      runbook: null,
      preferredClient: null,
      starterPrompt: "Use {{expert}} for {{project}}. Lead with {{leadAgent}} and verify the outcome before completion.",
      qualityContract: { version: 1, checks: [] },
      source: "custom",
    };
  }

  function cloneExpert(expert: ExpertResolved): ExpertDefinition {
    return {
      ...structuredClone(expert),
      id: `custom-${Date.now()}`,
      name: `${expert.name} Copy`,
      source: "custom",
    };
  }

  async function review() {
    const expert = experts.selected;
    if (!expert || !projectPath) return;
    planning = true;
    try {
      plan = await experts.plan(expert.id, projectPath, client || null);
    } catch (error) {
      toast.error("Could not plan activation", isAppError(error) ? appErrorMessage(error) : String(error));
    } finally {
      planning = false;
    }
  }

  async function activate() {
    if (!plan || plan.blockers.length) return;
    activating = true;
    try {
      const pending = pendingRequests.find((item) => item.expertId === plan?.expert.id && item.projectPath === plan?.projectPath);
      const activation = await experts.activate(plan.expert.id, plan.projectPath, plan.client);
      if (pending) await experts.resolveRequest(pending.id, true);
      await experts.load();
      activity.log({
        action: "bulk",
        subject: "agent",
        subjectName: plan.expert.name,
        projectPath: plan.projectPath,
        outcome: "ok",
        detail: `Activated Expert with ${plan.rollbackScope.length} new component(s)`,
      });
      const prompt = activation.runId
        ? `${plan.promptPreview}\n\nRun ID: ${activation.runId}. Read the quality contract with expert_runs_get_contract, then submit evidence for each check before requesting review.`
        : plan.promptPreview;
      await navigator.clipboard.writeText(prompt);
      toast.success(`${plan.expert.name} activated; starter prompt copied`);
      closeLinkedReview(() => (plan = null));
    } catch (error) {
      activity.log({
        action: "bulk",
        subject: "agent",
        subjectName: plan?.expert.name,
        projectPath: plan?.projectPath,
        outcome: "error",
        detail: isAppError(error) ? appErrorMessage(error) : String(error),
      });
      toast.error("Activation failed", isAppError(error) ? appErrorMessage(error) : String(error));
    } finally {
      activating = false;
    }
  }

  async function copyPrompt(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      toast.success("Starter prompt copied");
    } catch (error) {
      toast.error("Copy failed", String(error));
    }
  }

  async function saveBuilder() {
    if (!builder) return;
    try {
      if (creationReview) {
        await experts.approveCreation(creationReview.id, proposalFrom(builder));
        activity.log({
          action: "bulk",
          subject: "agent",
          subjectName: builder.name,
          projectPath: creationReview.projectPath,
          outcome: "ok",
          detail: "Approved Expert proposal",
        });
        creationReview = null;
      } else {
        await experts.save(builder);
      }
      closeLinkedReview(() => (builder = null));
      toast.success("Expert saved");
    } catch (error) {
      toast.error("Could not save Expert", isAppError(error) ? appErrorMessage(error) : String(error));
    }
  }

  function proposalFrom(expert: ExpertDefinition): ExpertProposalInput {
    const { id: _id, version: _version, source: _source, ...proposal } = expert;
    return proposal;
  }

  function reviewCreation(request: ExpertCreationRequest) {
    creationReview = request;
    builder = {
      id: `proposal-${request.id}`,
      version: 1,
      source: "custom",
      ...structuredClone($state.snapshot(request.proposal)),
    };
  }

  async function rejectCreation(request: ExpertCreationRequest) {
    try {
      await experts.rejectCreation(request.id);
      activity.log({
        action: "bulk",
        subject: "agent",
        subjectName: request.proposal.name,
        projectPath: request.projectPath,
        outcome: "ok",
        detail: "Rejected Expert proposal",
      });
    } catch (error) {
      toast.error("Could not reject Expert proposal", isAppError(error) ? appErrorMessage(error) : String(error));
    }
  }

  function missingRequired(run: ExpertRun | null): string[] {
    if (!run) return [];
    return run.contract.checks.filter((check) => check.required && !run.evidence.some((evidence) => evidence.checkName === check.name && evidence.result === "pass")).map((check) => check.name);
  }

  function reportedChecks(run: ExpertRun): number {
    return new Set(run.evidence.map((item) => item.checkName)).size;
  }

  async function finishRun(verdict: "accepted" | "rework" | "rejected" | "cancelled", waive = false) {
    if (!runReview) return;
    const waivers = waive ? missingRequired(runReview).map((checkName) => ({ checkName, reason: waiverReason.trim() })) : [];
    try {
      await experts.reviewRun(runReview.id, verdict, waivers);
      closeLinkedReview(() => (runReview = null));
      waiverReason = "";
      toast.success(`Run marked ${verdict}`);
    } catch (error) {
      toast.error("Could not review run", isAppError(error) ? appErrorMessage(error) : String(error));
    }
  }

  function normalizedSkill(value: string): string {
    return value.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
  }

  const pendingRequiredSkill = $derived(creationReview !== null && builder !== null && builder.requiredSkills.some((name) => {
    const normalized = normalizedSkill(name);
    return creationReview!.linkedSkillStates.some((link) =>
      normalizedSkill(link.skillName) === normalized && link.state === "pending"
    );
  }));

  async function reviewRequest(request: (typeof experts.requests)[number]) {
    projectPath = request.projectPath;
    client = request.client ?? "";
    experts.selectedId = request.expertId;
    await review();
  }

  let handledReviewLink = $state("");
  $effect(() => {
    const link = ui.expertReview;
    if (!link || experts.loading || handledReviewLink === `${link.kind}:${link.id}`) return;
    if (link.kind === "change") {
      const request = experts.creationRequests.find((item) => item.id === link.id);
      if (!request) return;
      tab = "drafts";
      reviewCreation(request);
    } else if (link.kind === "run") {
      const run = experts.runs.find((item) => item.id === link.id && item.state === "awaitingReview");
      if (!run) return;
      tab = "runs";
      runReview = run;
      waiverReason = "";
    } else {
      const request = experts.requests.find((item) => item.id === link.id && item.state === "pending");
      if (!request) return;
      tab = "experts";
      void reviewRequest(request);
    }
    handledReviewLink = `${link.kind}:${link.id}`;
    ui.expertReview = null;
  });

  function closeLinkedReview(close: () => void) {
    close();
    if (ui.reviewReturnId) ui.returnToActivityReview();
  }

  async function importExperts() {
    const path = await open({ multiple: false, filters: [{ name: "Expert JSON", extensions: ["json"] }] });
    if (typeof path !== "string") return;
    try { toast.success(`Imported ${await experts.importFile(path)} Expert(s)`); }
    catch (error) { toast.error("Import failed", isAppError(error) ? appErrorMessage(error) : String(error)); }
  }

  async function exportExperts() {
    const path = await save({ defaultPath: "experts.json", filters: [{ name: "Expert JSON", extensions: ["json"] }] });
    if (!path) return;
    try { toast.success(`Exported ${await experts.exportFile(path)} Expert(s)`); }
    catch (error) { toast.error("Export failed", isAppError(error) ? appErrorMessage(error) : String(error)); }
  }

  function saveRoster(expert: ExpertResolved) {
    teams.hydrate();
    teams.save(expert.name, [expert.leadAgent, ...expert.supportingAgents]);
    toast.success("Roster saved as Team");
  }
</script>

<section class="experts">
  <header class="head">
    <div><h1>Experts</h1><p>Project-ready specialist workspaces.</p></div>
    <div class="tabs" role="tablist">
      <button role="tab" aria-selected={tab === "experts"} class:active={tab === "experts"} onclick={() => (tab = "experts")}>Experts</button>
      <button role="tab" aria-selected={tab === "drafts"} class:active={tab === "drafts"} onclick={() => { tab = "drafts"; void experts.load(); }}>Changes ({experts.creationRequests.filter((request) => request.state === "pending").length})</button>
      <button role="tab" aria-selected={tab === "runs"} class:active={tab === "runs"} onclick={() => { tab = "runs"; void experts.load(); }}>Runs ({experts.runs.filter((run) => run.state === "awaitingReview").length})</button>
      <button role="tab" aria-selected={tab === "runbooks"} class:active={tab === "runbooks"} onclick={() => (tab = "runbooks")}>Runbooks</button>
    </div>
  </header>

  {#if tab === "runbooks"}
    <div class="runbooks"><Runbooks /></div>
  {:else if tab === "runs"}
    <div class="drafts" aria-label="Expert runs">
      {#if experts.runs.length === 0}<p class="empty">No Expert runs.</p>
      {:else}{#each experts.runs as run (run.id)}
        <article class="draft-card">
          <div class="title"><div><h2>{experts.list.find((expert) => expert.id === run.expertId)?.name ?? run.expertId}</h2><p>{run.projectPath} · {run.client} · {new Date(run.startedAt).toLocaleString()}</p></div><span class:ready={run.state === "accepted"} class="status">{run.state}</span></div>
          <p>{reportedChecks(run)}/{run.contract.checks.length} checks reported · {run.blockers.length} blockers</p>
          {#if run.state === "awaitingReview"}<footer><button onclick={() => { runReview = run; waiverReason = ""; }}>Review run</button></footer>{/if}
        </article>
      {/each}{/if}
    </div>
  {:else if tab === "drafts"}
    <div class="drafts" aria-label="Expert change requests">
      {#if experts.loading}
        <p class="empty">Loading Expert changes…</p>
      {:else if experts.error}
        <p class="empty">{experts.error}</p>
      {:else if experts.creationRequests.length === 0}
        <p class="empty">No Expert change requests.</p>
      {:else}
        {#each experts.creationRequests as request (request.id)}
          <article class="draft-card">
            <div class="title">
              <div>
                <h2>{request.kind}: {request.proposal.name}</h2>
                <p>{request.requestedBy} · {request.projectPath} · {new Date(request.requestedAt).toLocaleString()}</p>
              </div>
              <span class:ready={request.readiness === "ready"} class="status">{request.readiness}</span>
            </div>
            {#if request.blockers.length}
              <div class="warning">{request.blockers.join(" · ")}</div>
            {/if}
            {#if request.agentSubstitutions.length}
              <section>
                <h3>Agent substitutions</h3>
                {#each request.agentSubstitutions as substitution}
                  <p>{substitution.neededCapability} → {substitution.selectedCatalogSlug}: {substitution.rationale}</p>
                {/each}
              </section>
            {/if}
            {#if request.linkedSkillStates.length}
              <section>
                <h3>Linked skills</h3>
                <div class="chips">
                  {#each request.linkedSkillStates as skill}
                    <button onclick={() => ui.setSection("skills")}>{skill.skillName} · {skill.state ?? "missing"}</button>
                  {/each}
                </div>
              </section>
            {/if}
            <footer>
              {#if request.state === "pending"}
                <button onclick={() => rejectCreation(request)}>Reject</button>
                <button class="primary" onclick={() => reviewCreation(request)}>Review</button>
              {:else}
                <span>{request.state}{request.savedExpertId ? ` · ${request.savedExpertId}` : ""}</span>
              {/if}
            </footer>
          </article>
        {/each}
      {/if}
    </div>
  {:else}
    <div class="toolbar">
      <label class="search"><Search size={14} /><input aria-label="Search Experts" placeholder="Search Experts" bind:value={query} /></label>
      <select aria-label="Project" bind:value={projectPath}>
        <option value="">Select project…</option>
        {#each projects.list as project (project.path)}<option value={project.path}>{project.label}</option>{/each}
      </select>
      <button class="primary" onclick={() => (builder = newExpert())}><Plus size={14} /> New Expert</button>
      <button onclick={importExperts}>Import</button>
      <button onclick={exportExperts}>Export</button>
    </div>
    <div class="filters">
      {#each [["all","All"],["ready","Ready"],["setup","Needs setup"],["custom","Custom"],["recent","Recently used"]] as item}
        <button class:active={filter === item[0]} onclick={() => (filter = item[0] as typeof filter)}>{item[1]}</button>
      {/each}
    </div>
    {#if pendingRequests.length}
      <div class="inbox">
        <strong>Activation requests</strong>
        {#each pendingRequests as request (request.id)}
          <span>{request.requestedBy} requested {experts.list.find((item) => item.id === request.expertId)?.name ?? request.expertId}</span>
          <button onclick={() => reviewRequest(request)}>Review</button>
          <button onclick={() => experts.resolveRequest(request.id, false)}>Reject</button>
        {/each}
      </div>
    {/if}

    <div class="workspace">
      <aside class="list" aria-label="Experts">
        {#if experts.loading}<p class="empty">Loading…</p>
        {:else if experts.error}<p class="empty">{experts.error}</p>
        {:else if visible.length === 0}<p class="empty">No Experts match this view.</p>
        {:else}
          {#each visible as expert (expert.id)}
            <button class:selected={experts.selected?.id === expert.id} onclick={() => (experts.selectedId = expert.id)}>
              <span class="row-title">{expert.name}</span>
              <span class="row-meta">{knownAgents.get(expert.leadAgent)?.name ?? expert.leadAgent} · {expert.preferredClient ?? "Choose client"}</span>
              <span class:ready={ready(expert)} class="status">{ready(expert) ? "Ready" : "Needs setup"}</span>
            </button>
          {/each}
        {/if}
      </aside>

      <article class="detail">
        {#if experts.selected}
          {@const expert = experts.selected}
          {@const performance = summarizeExpertPerformance(expert, experts.runs)}
          <div class="title"><div><h2>{expert.name}</h2><p>{expert.summary}</p></div><span class="source">{expert.source}</span></div>
          <section><h3>Expected outcome</h3><p>{expert.summary}</p></section>
          <section><h3>Quality contract</h3><p>{expert.qualityContract.checks.length ? expert.qualityContract.checks.map((check) => `${check.name} · ${check.required ? "required" : "optional"}`).join(" · ") : "No checks configured"}</p></section>
          <section class="performance">
            <h3>Performance and Improvement Coach</h3>
            {#if performance.eligible}
              <p>Based on {performance.comparableRuns} comparable terminal runs with this Expert version and quality contract.</p>
              <div class="metrics">
                <strong>Acceptance rate {performance.acceptanceRate}%</strong>
                <span>Rework {performance.rework} · Rejected {performance.rejected} · Runs with waivers {performance.waiverRate}%</span>
              </div>
              <ul>{#each performance.suggestions as suggestion}<li>{suggestion}</li>{/each}</ul>
            {:else}
              <p>{performance.comparableRuns} of 5 comparable terminal runs. Metrics and suggestions appear after five quality verdicts for this exact Expert version and contract.</p>
            {/if}
          </section>
          <section>
            <h3>Roster</h3>
            <button class="link" onclick={() => ui.openAgents(null)}>{knownAgents.get(expert.leadAgent)?.name ?? expert.leadAgent} · lead</button>
            {#each expert.supportingAgents as slug}<button class="link" onclick={() => ui.openAgents(null)}>{knownAgents.get(slug)?.name ?? slug}</button>{/each}
          </section>
          <section>
            <h3>Skills</h3>
            <div class="chips">
              {#each expert.requiredSkills as skill}<button onclick={() => ui.setSection("skills")}>{skill} · required</button>{/each}
              {#each expert.optionalSkills as skill}<button onclick={() => ui.setSection("skills")}>{skill} · optional</button>{/each}
            </div>
          </section>
          <section class="grid">
            <label>Client<select bind:value={client}><option value="">Use preference</option><option value="claudeCode">Claude Code</option><option value="codex">Codex</option></select></label>
            <div><h3>Readiness</h3><strong class:ok={ready(expert)}>{ready(expert) ? "Ready" : "Blocked"}</strong></div>
          </section>
          {#if expert.unresolvedAgents.length || expert.unresolvedSkills.length || expert.unresolvedRunbook}
            <div class="warning">Unresolved: {[...expert.unresolvedAgents, ...expert.unresolvedSkills, ...(expert.unresolvedRunbook ? [expert.runbook ?? "runbook"] : [])].join(", ")}</div>
          {/if}
          <section><h3>Starter prompt</h3><pre>{expert.starterPrompt}</pre><button onclick={() => copyPrompt(expert.starterPrompt)}><Copy size={14} /> Copy</button></section>
          <footer>
            {#if expert.source === "custom"}<button onclick={() => experts.remove(expert.id)}>Delete</button>{/if}
            <button onclick={() => (builder = cloneExpert(expert))}>Clone and customize</button>
            <button onclick={() => saveRoster(expert)}>Save roster as Team</button>
            <button class="primary" disabled={!projectPath || planning} onclick={review}><Sparkles size={14} /> {planning ? "Planning…" : "Activate Expert"}</button>
          </footer>
        {:else}<p class="empty">Select an Expert.</p>{/if}
      </article>
    </div>
  {/if}
</section>

{#if plan}
  <Modal open title={`Review ${plan.expert.name}`} size="wide" onClose={() => closeLinkedReview(() => (plan = null))}>
    <div class="review">
      <p><strong>Destination:</strong> {plan.projectPath}</p>
      <p><strong>Client:</strong> {plan.client}</p>
      <h3>Agents</h3>
      {#each plan.agents as agent}<p>{agent.slug} — {agent.status}{agent.destination ? ` → ${agent.destination}` : ""}</p>{/each}
      <h3>Skills and dependencies</h3>
      {#each plan.skills as skill}{#each skill.packages as pkg}<p>{pkg.dependency ? "Dependency" : "Skill"}: {pkg.name} → {pkg.destination}</p>{/each}{/each}
      {#if plan.blockers.length}<div class="warning"><strong>Blocked</strong>{#each plan.blockers as blocker}<p>{blocker}</p>{/each}</div>{/if}
      <h3>Rollback scope</h3><p>{plan.rollbackScope.join(", ") || "No new components"}</p>
      <h3>Quality contract</h3><p>{plan.expert.qualityContract.checks.length ? plan.expert.qualityContract.checks.map((check) => check.name).join(", ") : "No checks configured"}</p>
      <h3>Generated starter prompt</h3><pre>{plan.promptPreview}</pre>
    </div>
    {#snippet actions()}
      <button onclick={() => closeLinkedReview(() => (plan = null))}>Cancel</button>
      <button class="primary" disabled={(plan?.blockers.length ?? 1) > 0 || activating} onclick={activate}>{activating ? "Activating…" : "Approve activation"}</button>
    {/snippet}
  </Modal>
{/if}

{#if runReview}
  <Modal open title={`Review run ${runReview.id.slice(0, 8)}`} size="wide" onClose={() => closeLinkedReview(() => (runReview = null))}>
    <div class="review">
      <p><strong>Expert:</strong> {runReview.expertId} v{runReview.expertVersion}</p>
      <p><strong>Project:</strong> {runReview.projectPath}</p>
      <h3>Checks</h3>
      {#each runReview.contract.checks as check}
        {@const evidence = runReview.evidence.filter((item) => item.checkName === check.name).at(-1)}
        <p>{check.name} · {check.required ? "required" : "optional"} — {evidence?.result ?? "missing"}{evidence?.summary ? `: ${evidence.summary}` : ""}</p>
      {/each}
      {#if runReview.blockers.length}<h3>Blockers</h3>{#each runReview.blockers as blocker}<p>{blocker.kind}: {blocker.summary}</p>{/each}{/if}
      {#if missingRequired(runReview).length}<label>Waiver reason<textarea maxlength="4096" bind:value={waiverReason}></textarea></label>{/if}
    </div>
    {#snippet actions()}
      <button onclick={() => finishRun("cancelled")}>Cancel run</button>
      <button onclick={() => finishRun("rejected")}>Reject</button>
      <button onclick={() => finishRun("rework")}>Request rework</button>
      {#if missingRequired(runReview).length}
        <button class="primary" disabled={!waiverReason.trim()} onclick={() => finishRun("accepted", true)}>Accept with waiver</button>
      {:else}<button class="primary" onclick={() => finishRun("accepted")}>Accept</button>{/if}
    {/snippet}
  </Modal>
{/if}

{#if builder}
  <Modal open title={creationReview ? `Review ${creationReview.kind} request` : builder.source === "custom" ? "Edit Expert" : "Clone Expert"} size="wide" onClose={() => closeLinkedReview(() => { builder = null; creationReview = null; })}>
    <form class="builder" onsubmit={(event) => { event.preventDefault(); void saveBuilder(); }}>
      <label>Name<input required maxlength="120" bind:value={builder.name} /></label>
      <label>Summary<textarea required maxlength="1000" bind:value={builder.summary}></textarea></label>
      <label>Category<input required maxlength="80" bind:value={builder.category} /></label>
      <label>Tags<input maxlength="500" value={builder.tags.join(", ")} oninput={(event) => (builder!.tags = event.currentTarget.value.split(",").map((item) => item.trim()).filter(Boolean))} /></label>
      <label>Lead agent<select bind:value={builder.leadAgent}>{#each corpus.agents as agent}<option value={agent.slug}>{agent.name}</option>{/each}</select></label>
      <label>Supporting agents<select multiple size="7" value={builder.supportingAgents} onchange={(event) => (builder!.supportingAgents = [...event.currentTarget.selectedOptions].map((item) => item.value))}>{#each corpus.agents as agent}<option value={agent.slug}>{agent.name}</option>{/each}</select></label>
      <label>Required skills<input maxlength="1000" value={builder.requiredSkills.join(", ")} oninput={(event) => (builder!.requiredSkills = event.currentTarget.value.split(",").map((item) => item.trim()).filter(Boolean))} /></label>
      <label>Optional skills<input maxlength="1000" value={builder.optionalSkills.join(", ")} oninput={(event) => (builder!.optionalSkills = event.currentTarget.value.split(",").map((item) => item.trim()).filter(Boolean))} /></label>
      <label>Runbook slug<input maxlength="160" value={builder.runbook ?? ""} oninput={(event) => (builder!.runbook = event.currentTarget.value.trim() || null)} /></label>
      <label>Preferred client<select bind:value={builder.preferredClient}><option value={null}>Ask during activation</option><option value="claudeCode">Claude Code</option><option value="codex">Codex</option></select></label>
      <label>Starter prompt<textarea required maxlength="4096" rows="6" bind:value={builder.starterPrompt}></textarea></label>
      <label>Required quality checks<input maxlength="500" value={builder.qualityContract.checks.filter((check) => check.required).map((check) => check.name).join(", ")} oninput={(event) => { const optional = builder!.qualityContract.checks.filter((check) => !check.required); builder!.qualityContract = { version: 1, checks: [...event.currentTarget.value.split(",").map((name) => name.trim()).filter(Boolean).map((name) => ({ name, kind: name, required: true, evidenceMode: "clientReported" as const })), ...optional] }; }} /></label>
      <label>Optional quality checks<input maxlength="500" value={builder.qualityContract.checks.filter((check) => !check.required).map((check) => check.name).join(", ")} oninput={(event) => { const required = builder!.qualityContract.checks.filter((check) => check.required); builder!.qualityContract = { version: 1, checks: [...required, ...event.currentTarget.value.split(",").map((name) => name.trim()).filter(Boolean).map((name) => ({ name, kind: name, required: false, evidenceMode: "clientReported" as const }))] }; }} /></label>
    </form>
    {#snippet actions()}<button onclick={() => closeLinkedReview(() => { builder = null; creationReview = null; })}>Cancel</button><button class="primary" disabled={pendingRequiredSkill} onclick={saveBuilder}>Save</button>{/snippet}
  </Modal>
{/if}

<style>
  .experts { height: 100%; min-height: 0; display: flex; flex-direction: column; }
  .head,.toolbar,.filters,footer,.title,.tabs,.chips,.grid { display: flex; align-items: center; gap: var(--space-2); }
  .head { justify-content: space-between; padding: var(--space-3) var(--space-4); border-bottom: 1px solid var(--color-border); }
  h1 { font-size: var(--text-h2); } h2 { font-size: var(--text-h2); } h3 { font-size: var(--text-body); margin-bottom: 6px; }
  p,.row-meta { color: var(--color-text-secondary); font-size: var(--text-body-sm); }
  button,select,input,textarea { border: 1px solid var(--color-border); border-radius: var(--radius-md); background: var(--color-surface-raised); color: var(--color-text-primary); padding: 7px 10px; }
  button { cursor: pointer; display: inline-flex; align-items: center; gap: 5px; } button:disabled { opacity: .5; cursor: default; }
  .primary,.tabs .active,.filters .active { background: var(--color-brand); color: var(--color-text-inverse); border-color: var(--color-brand); }
  .runbooks { flex: 1; min-height: 0; } .toolbar { padding: var(--space-3) var(--space-4); } .toolbar .search { flex: 1; display:flex;align-items:center;gap:6px; }
  .drafts { flex:1; min-height:0; overflow:auto; padding:var(--space-4); display:grid; gap:var(--space-3); align-content:start; }
  .draft-card { border:1px solid var(--color-border); border-radius:var(--radius-lg); padding:var(--space-4); background:var(--color-surface-raised); }
  .draft-card section { margin-top:var(--space-3); }.draft-card footer { justify-content:flex-end; margin-top:var(--space-3); }
  .inbox { display:grid; grid-template-columns:auto 1fr auto auto; align-items:center; gap:var(--space-2); padding:var(--space-2) var(--space-4); background:var(--color-brand-subtle); border-top:1px solid var(--color-border); }
  .search input { flex: 1; border: 0; padding: 0; background: transparent; } .filters { padding: 0 var(--space-4) var(--space-3); }
  .workspace { flex: 1; min-height: 0; display:grid; grid-template-columns:minmax(280px,34%) 1fr; border-top:1px solid var(--color-border); }
  .list { overflow:auto; border-right:1px solid var(--color-border); padding:var(--space-2); }
  .list>button { width:100%; display:flex; flex-direction:column; align-items:flex-start; margin-bottom:4px; text-align:left; }
  .list>button.selected { border-color:var(--color-brand); background:var(--color-brand-subtle); } .row-title { font-weight:var(--fw-semibold); }
  .status { font-size:var(--text-caption); color:var(--color-warning); }.status.ready,.ok { color:var(--color-success); }
  .detail { overflow:auto; padding:var(--space-4); } .detail section { margin-top:var(--space-4); } .title { justify-content:space-between; }.source { text-transform:capitalize; }
  .metrics { display:flex; gap:var(--space-2); flex-wrap:wrap; align-items:center; margin-top:var(--space-2); }.performance ul { margin:var(--space-2) 0 0; padding-left:var(--space-4); }.performance li { margin-top:4px; font-size:var(--text-body-sm); }
  .link { display:block; border:0; background:transparent; color:var(--color-brand); padding-left:0; }.chips { flex-wrap:wrap; }.grid { align-items:flex-start; justify-content:space-between; }
  label { display:flex; flex-direction:column; gap:5px; font-size:var(--text-body-sm); }.detail footer { justify-content:flex-end; margin-top:var(--space-4); }
  pre { white-space:pre-wrap; padding:var(--space-3); background:var(--color-surface-sunken); border-radius:var(--radius-md); font-size:var(--text-body-sm); }
  .warning { padding:var(--space-3); margin-top:var(--space-3); border:1px solid var(--color-warning); border-radius:var(--radius-md); color:var(--color-warning); }
  .review { max-height:60vh; overflow:auto; }.builder { display:grid; gap:var(--space-3); max-height:60vh; overflow:auto; }.builder input,.builder textarea,.builder select { width:100%; }
  .empty { padding:var(--space-4); }
</style>
