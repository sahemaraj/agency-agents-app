<script lang="ts">
  import { onMount } from "svelte";
  import Button from "$lib/components/Button.svelte";
  import type { StorageMigrationStatus } from "$lib/types";

  interface Props {
    status: StorageMigrationStatus;
    busy: boolean;
    error: string | null;
    onStart: () => void;
    onRetry: () => void;
    onOpenData: () => void;
  }

  let { status, busy, error, onStart, onRetry, onOpenData }: Props = $props();
  let gate: HTMLElement | null = $state(null);
  const failed = $derived(status.state === "corrupt" || error !== null);
  const unsupported = $derived(status.state === "unsupported");
  const complete = $derived(status.state === "complete" && status.stage !== "recovery");
  const details = $derived(error ?? status.detail ?? "No technical details were reported.");
  const stages = [
    ["checkingData", "Checking data"],
    ["verifyingBackup", "Verifying backup"],
    ["movingRecords", "Moving records"],
    ["verifyingDatabase", "Verifying database"],
    ["finishing", "Finishing"],
  ] as const;

  onMount(() => gate?.querySelector<HTMLButtonElement>("button.btn-primary")?.focus());

  async function copyDetails() {
    await navigator.clipboard?.writeText(details);
  }
</script>

<div bind:this={gate} class="gate" role="dialog" aria-modal="true" aria-busy={busy} aria-labelledby="migration-title">
  <div class="card">
    {#if unsupported}
      <h1 id="migration-title">A newer Shikigami version is required</h1>
      <p>This data was created by a newer Shikigami version. Nothing was changed.</p>
    {:else if complete}
      <h1 id="migration-title">Data update complete</h1>
      <p>Reopen connected Claude and Codex sessions so they use the updated storage.</p>
    {:else if failed}
      <h1 id="migration-title">The data update could not finish</h1>
      <p>Nothing was lost. Your current data and verified backup remain available.</p>
    {:else}
      <h1 id="migration-title">Shikigami needs a one-time data update</h1>
      <p>Close connected Claude and Codex sessions first. Skill and Agent package files are not moved or changed.</p>
      <ol aria-label="Data update stages">
        {#each stages as [key, label]}
          <li class:current={status.stage === key}>{label}</li>
        {/each}
      </ol>
    {/if}

    {#if status.detail || error}
      <details>
        <summary>Show details</summary>
        <p class="detail">{details}</p>
        <Button variant="secondary" size="sm" onclick={copyDetails}>Copy details</Button>
      </details>
    {/if}

    <div class="actions">
      {#if !unsupported && !complete}
        <Button variant="primary" size="lg" loading={busy} onclick={failed ? onRetry : onStart}>
          {failed ? "Retry data update" : "Start data update"}
        </Button>
      {/if}
      <Button variant="secondary" size="lg" onclick={onOpenData}>Open data folder</Button>
    </div>
  </div>
</div>

<style>
  .gate {
    position: fixed;
    inset: 0;
    z-index: 1000;
    display: grid;
    place-items: center;
    padding: var(--space-6);
    background: var(--color-surface);
  }
  .card {
    width: min(560px, 100%);
    display: grid;
    gap: var(--space-4);
    padding: var(--space-6);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
    background: var(--color-surface-raised);
  }
  h1 { margin: 0; font-size: var(--text-h1); color: var(--color-text-primary); }
  p { margin: 0; color: var(--color-text-secondary); line-height: var(--lh-normal); }
  ol { display: grid; gap: var(--space-2); margin: 0; padding-left: var(--space-5); color: var(--color-text-muted); }
  li.current { color: var(--color-text-primary); font-weight: var(--fw-semibold); }
  details { border-top: 1px solid var(--color-border); padding-top: var(--space-3); }
  summary { cursor: pointer; color: var(--color-text-secondary); }
  .detail { margin: var(--space-2) 0; overflow-wrap: anywhere; }
  .actions { display: flex; flex-wrap: wrap; gap: var(--space-3); }
  @media (prefers-reduced-motion: reduce) { .gate { scroll-behavior: auto; } }
</style>
