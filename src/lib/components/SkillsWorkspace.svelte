<script lang="ts">
  import { onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import FolderPlus from "@lucide/svelte/icons/folder-plus";

  import Button from "$lib/components/Button.svelte";
  import DestructiveConfirm from "$lib/components/DestructiveConfirm.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import LoadingState from "$lib/components/LoadingState.svelte";
  import { skillSources } from "$lib/stores/skillSources.svelte";
  import type { SkillSource, SkillSourceResult } from "$lib/types";

  let localPath = $state("");
  let announcement = $state("");
  let removeCandidate: SkillSource | null = $state(null);

  onMount(() => {
    void skillSources.load();
  });

  function sourceLabel(source: SkillSource): string {
    return source.kind.kind === "local" ? source.kind.root : source.kind.repository;
  }

  function packageCount(result: SkillSourceResult | undefined): number {
    return result?.packages.filter((pkg) => pkg.installable).length ?? 0;
  }

  async function chooseLocalFolder(): Promise<void> {
    if (skillSources.loading || skillSources.adding) return;
    const picked = await openDialog({
      directory: true,
      multiple: false,
      title: "Choose a skill source folder",
    });
    if (typeof picked !== "string") return;

    localPath = picked;
    const outcome = await skillSources.addLocal(picked);
    if (outcome.registrationSucceeded && outcome.initialRefreshSucceeded) {
      const source = skillSources.sources.find(
        (candidate) => candidate.kind.kind === "local" && candidate.kind.root === picked,
      );
      announcement = `Skill source added with ${packageCount(source ? skillSources.results[source.id] : undefined)} valid packages.`;
      localPath = "";
    }
  }

  async function refresh(source: SkillSource): Promise<void> {
    const succeeded = await skillSources.refresh(source.id);
    if (succeeded) {
      announcement = `${sourceLabel(source)} refreshed with ${packageCount(skillSources.results[source.id])} valid packages.`;
    }
  }

  async function removeSource(): Promise<void> {
    if (!removeCandidate) return;
    const source = removeCandidate;
    if (await skillSources.remove(source.id)) {
      announcement = `${sourceLabel(source)} removed from Agency Agents.`;
      removeCandidate = null;
    }
  }
</script>

<div class="workspace">
  <header>
    <div>
      <h2>Skill sources</h2>
      <p>Add trusted folders and refresh them only when you choose.</p>
    </div>
    <section class="local-add" aria-label="Add local skill source" aria-busy={skillSources.loading || skillSources.adding}>
      {#if localPath}<span class="picked" title={localPath}>{localPath}</span>{/if}
      <Button
        variant="primary"
        loading={skillSources.adding}
        disabled={skillSources.loading || skillSources.adding}
        ariaLabel="Choose and add a local skill source folder"
        onclick={() => void chooseLocalFolder()}
      >
        {#snippet icon()}<FolderPlus size={16} />{/snippet}
        Add local folder
      </Button>
    </section>
  </header>

  <div class="announcement" role="status" aria-live="polite">{announcement}</div>

  {#if skillSources.addError}
    <div class="alert" role="alert">{skillSources.addError}</div>
  {/if}

  <div class="content">
    {#if skillSources.loading && skillSources.sources.length === 0}
      <LoadingState rows={4} label="Loading skill sources" />
    {:else if skillSources.sources.length === 0}
      <EmptyState
        title="No skill sources yet"
        body="Choose a local folder to discover its validated skill packages."
      >
        {#snippet icon()}<FolderPlus size={28} />{/snippet}
      </EmptyState>
    {:else}
      <div class="sources">
        {#each skillSources.sources as source (source.id)}
          {@const result = skillSources.results[source.id]}
          {@const refreshError = skillSources.refreshErrors[source.id]}
          {@const refreshing = skillSources.isRefreshing(source.id)}
          {@const removeError = skillSources.removeErrors[source.id]}
          {@const removing = skillSources.isRemoving(source.id)}
          <article class="source" aria-busy={refreshing || removing}>
            <div class="source-head">
              <div class="identity">
                <span class="kind">{source.kind.kind === "local" ? "Local folder" : "GitHub"}</span>
                <strong title={sourceLabel(source)}>{sourceLabel(source)}</strong>
              </div>
              <div class="source-actions">
                <Button
                  size="sm"
                  loading={refreshing}
                  disabled={refreshing || removing}
                  ariaLabel={`Refresh skill source ${sourceLabel(source)}`}
                  onclick={() => void refresh(source)}
                >
                  Refresh
                </Button>
                <Button
                  size="sm"
                  variant="danger"
                  disabled={refreshing || removing}
                  ariaLabel={`Remove skill source ${sourceLabel(source)}`}
                  onclick={() => (removeCandidate = source)}
                >
                  Remove
                </Button>
              </div>
            </div>

            {#if refreshError || removeError}
              <div class="alert" role="alert">{refreshError ?? removeError}</div>
            {/if}

            {#if result}
              {#if result.errors.length > 0}
                <ul class="diagnostics" aria-label="Source errors">
                  {#each result.errors as error}
                    <li><code>{error.code}</code> · {error.path}: {error.message}</li>
                  {/each}
                </ul>
              {/if}

              {@const validPackages = result.packages.filter((pkg) => pkg.installable)}
              {#if validPackages.length > 0}
                <ul class="packages" aria-label="Valid skill packages">
                  {#each validPackages as pkg (pkg.relativePath)}
                    <li>
                      <strong>{pkg.name}</strong>
                      <span>{pkg.description}</span>
                    </li>
                  {/each}
                </ul>
              {:else}
                <p class="quiet">No valid packages were returned.</p>
              {/if}

              {#each result.packages.filter((pkg) => !pkg.installable) as pkg (pkg.relativePath)}
                <div class="rejected">
                  <strong>Rejected: {pkg.relativePath}</strong>
                  <ul class="diagnostics" aria-label={`Errors for ${pkg.relativePath}`}>
                    {#each pkg.errors as error}
                      <li><code>{error.code}</code> · {pkg.relativePath}{error.path ? `/${error.path}` : ""}: {error.message}</li>
                    {/each}
                  </ul>
                </div>
              {/each}
            {:else}
              <p class="quiet">Refresh this source to validate its packages.</p>
            {/if}
          </article>
        {/each}
      </div>
    {/if}
  </div>
</div>

<DestructiveConfirm
  open={removeCandidate !== null}
  title="Remove skill source?"
  confirmLabel="Remove source"
  confirmDisabled={removeCandidate ? skillSources.isRemoving(removeCandidate.id) : false}
  onConfirm={() => void removeSource()}
  onCancel={() => (removeCandidate = null)}
>
  <p>This only removes the source from Agency Agents. The original folder and installed skills will not be deleted.</p>
</DestructiveConfirm>

<style>
  .workspace {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  header {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-4);
    border-bottom: 1px solid var(--color-border);
  }
  h2 {
    font-size: var(--text-h2);
    font-weight: var(--fw-semibold);
    color: var(--color-text-primary);
  }
  header p, .quiet {
    margin-top: var(--space-1);
    color: var(--color-text-muted);
    font-size: var(--text-body-sm);
  }
  .local-add {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }
  .picked {
    max-width: 280px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--color-text-muted);
    font-family: var(--font-mono);
    font-size: var(--text-caption);
  }
  .announcement {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }
  .content {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: var(--space-4);
  }
  .sources {
    display: grid;
    gap: var(--space-3);
  }
  .source {
    display: grid;
    gap: var(--space-3);
    padding: var(--space-4);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
    background: var(--color-surface-raised);
  }
  .source-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }
  .source-actions {
    display: flex;
    gap: var(--space-2);
  }
  .identity {
    min-width: 0;
    display: grid;
    gap: 2px;
  }
  .identity strong {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--color-text-primary);
  }
  .kind {
    color: var(--color-text-muted);
    font-size: var(--text-caption);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .packages, .diagnostics {
    display: grid;
    gap: var(--space-2);
  }
  .packages li {
    display: grid;
    gap: 2px;
    padding: var(--space-3);
    border-radius: var(--radius-md);
    background: var(--color-surface-sunken);
  }
  .packages span, .diagnostics {
    color: var(--color-text-secondary);
    font-size: var(--text-body-sm);
  }
  .alert, .rejected {
    padding: var(--space-3);
    border: 1px solid var(--color-danger);
    border-radius: var(--radius-md);
    color: var(--color-danger);
    font-size: var(--text-body-sm);
  }
  .rejected {
    display: grid;
    gap: var(--space-2);
  }
  code { font-family: var(--font-mono); }
</style>
