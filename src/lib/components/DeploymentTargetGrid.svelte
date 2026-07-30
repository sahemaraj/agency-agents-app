<script lang="ts">
  import FolderIcon from "@lucide/svelte/icons/folder";
  import GlobeIcon from "@lucide/svelte/icons/globe";
  import { i18n } from "$lib/stores/i18n.svelte";

  export type DeploymentColumn = {
    id: string;
    label: string;
    supportsUser: boolean;
    supportsProject: boolean;
  };
  export type DeploymentRow =
    | { kind: "global" }
    | { kind: "project"; path: string; label: string };
  export type DeploymentCell = {
    state: "off" | "partial" | "on";
    busy?: boolean;
    disabled?: boolean;
    title: string;
    ariaLabel: string;
  };

  interface Props {
    columns: DeploymentColumn[];
    rows: DeploymentRow[];
    cell: (column: DeploymentColumn, row: DeploymentRow) => DeploymentCell;
    onToggle: (column: DeploymentColumn, row: DeploymentRow) => void;
    notApplicable: (column: DeploymentColumn, row: DeploymentRow) => string;
    flashPath?: string | null;
    registerDestination?: (node: HTMLElement, path: string | null) => { destroy?: () => void };
  }

  let {
    columns,
    rows,
    cell,
    onToggle,
    notApplicable,
    flashPath = null,
    registerDestination = () => ({}),
  }: Props = $props();

  const target = (row: DeploymentRow) => row.kind === "global" ? null : row.path;
  const applicable = (column: DeploymentColumn, row: DeploymentRow) =>
    row.kind === "global" ? column.supportsUser : column.supportsProject;
</script>

<div class="grid-wrap">
  <div class="grid" style="--cols: {columns.length}">
    <div class="cell head corner"></div>
    {#each columns as column (column.id)}
      <div class="cell head tool" title={column.label}>{column.label}</div>
    {/each}

    {#each rows as row (row.kind === "global" ? "global" : row.path)}
      <div class="cell dest" class:flash={flashPath !== null && target(row) === flashPath} use:registerDestination={target(row)}>
        {#if row.kind === "global"}
          <span class="d-ic"><GlobeIcon size={15} /></span>
          <span class="d-body"><span class="d-label">{i18n.t("common.global")}</span><span class="d-path">{i18n.t("common.everyMachine")}</span></span>
        {:else}
          <span class="d-ic"><FolderIcon size={15} /></span>
          <span class="d-body"><span class="d-label">{row.label}</span><span class="d-path" title={row.path}>{row.path}</span></span>
        {/if}
      </div>
      {#each columns as column (column.id)}
        {#if applicable(column, row)}
          {@const value = cell(column, row)}
          <button
            class="cell toggle"
            class:on={value.state === "on"}
            class:partial={value.state === "partial"}
            disabled={value.busy || value.disabled}
            title={value.title}
            aria-label={value.ariaLabel}
            onclick={() => onToggle(column, row)}
          >
            {#if value.busy}<span class="dot busy"></span>
            {:else if value.state === "on"}<span class="dot full"></span>
            {:else if value.state === "partial"}<span class="dot half"></span>
            {:else}<span class="dot"></span>{/if}
          </button>
        {:else}
          {@const reason = notApplicable(column, row)}
          <div class="cell na" title={reason} aria-label={reason}>—</div>
        {/if}
      {/each}
    {/each}
  </div>
</div>

<style>
  .grid-wrap { overflow-x: auto; border: 1px solid var(--color-border); border-radius: var(--radius-md); }
  .grid { display: grid; grid-template-columns: minmax(180px, 1fr) repeat(var(--cols), 68px); width: max-content; min-width: 100%; align-items: stretch; }
  .cell { display: flex; align-items: center; justify-content: center; padding: var(--space-2); border-bottom: 1px solid var(--color-border); }
  .head { min-height: 34px; background: var(--color-surface-sunken); color: var(--color-text-muted); font-size: var(--text-caption); font-weight: var(--fw-semibold); }
  .head.tool { padding: var(--space-2) 8px; text-align: center; line-height: 1.15; }
  .dest { justify-content: flex-start; gap: var(--space-2); min-width: 0; }
  .d-ic { flex: none; display: inline-flex; color: var(--color-text-secondary); }
  .d-body { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .d-label, .d-path { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .d-label { color: var(--color-text-primary); font-size: var(--text-body-sm); font-weight: var(--fw-medium); }
  .d-path { color: var(--color-text-muted); font-size: var(--text-caption); }
  .toggle { background: transparent; cursor: pointer; }
  .toggle:hover:not(:disabled) { background: var(--color-surface-sunken); }
  .toggle:disabled { cursor: default; }
  .na { color: var(--color-text-muted); opacity: .4; }
  .dot { width: 16px; height: 16px; border: 1.5px solid var(--color-border-strong, var(--color-text-muted)); border-radius: 999px; box-sizing: border-box; }
  .dot.full { border-color: var(--color-brand); background: var(--color-brand); }
  .dot.half { border-color: var(--color-brand); background: linear-gradient(90deg, var(--color-brand) 50%, transparent 50%); }
  .dot.busy { border-color: var(--color-text-muted); border-top-color: transparent; animation: spin .6s linear infinite; }
  .dest.flash { animation: destFlash 1.2s var(--motion-ease-out, ease-out); }
  @keyframes spin { to { transform: rotate(360deg); } }
  @keyframes destFlash { 0%, 100% { background: transparent; } 25% { background: var(--color-brand-subtle, color-mix(in srgb, var(--color-brand) 16%, transparent)); } }
</style>
