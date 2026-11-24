<script lang="ts" module>
  export interface Column<R> {
    id: string;
    /** Localized header text. */
    header: string;
    /** Makes the column sortable; returns the value to compare. */
    sortValue?: (row: R) => string | number;
  }
</script>

<script lang="ts" generics="Row">
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';

  interface Props {
    /** Localized caption: names the table for assistive technology. */
    caption: string;
    columns: Column<Row>[];
    rows: Row[];
    rowKey: (row: Row) => string;
    /** Renders one cell; the first column is the row header and gets its own element. */
    cell: Snippet<[Row, Column<Row>]>;
    /** Localized text when there are no rows. */
    empty: string;
    captionId: string;
  }

  let { caption, columns, rows, rowKey, cell, empty, captionId }: Props = $props();

  let sortId = $state<string | null>(null);
  let descending = $state(false);

  const sorted = $derived.by(() => {
    const column = columns.find((c) => c.id === sortId);
    const value = column?.sortValue;
    if (!value) return rows;
    const direction = descending ? -1 : 1;
    return [...rows].sort((a, b) => {
      const x = value(a);
      const y = value(b);
      const order =
        typeof x === 'number' && typeof y === 'number'
          ? x - y
          : String(x).localeCompare(String(y), undefined, { numeric: true, sensitivity: 'base' });
      return order * direction;
    });
  });

  function toggle(column: Column<Row>) {
    if (sortId === column.id) {
      descending = !descending;
    } else {
      sortId = column.id;
      descending = false;
    }
  }

  function ariaSort(column: Column<Row>): 'ascending' | 'descending' | 'none' | undefined {
    if (!column.sortValue) return undefined;
    if (sortId !== column.id) return 'none';
    return descending ? 'descending' : 'ascending';
  }
</script>

<!-- A scrollable region must be reachable by keyboard so that wide tables can be scrolled. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div class="scroll" role="region" aria-labelledby={captionId} tabindex="0">
  <table>
    <caption id={captionId}>{caption}</caption>
    <thead>
      <tr>
        {#each columns as column (column.id)}
          <th scope="col" aria-sort={ariaSort(column)}>
            {#if column.sortValue}
              <button type="button" class="sort" onclick={() => toggle(column)}>
                {column.header}
                <Icon name={sortId === column.id ? (descending ? 'down' : 'up') : 'sort'} size="sm" />
              </button>
            {:else}
              {column.header}
            {/if}
          </th>
        {/each}
      </tr>
    </thead>
    <tbody>
      {#each sorted as row (rowKey(row))}
        <tr>
          {#each columns as column, index (column.id)}
            {#if index === 0}
              <th scope="row">{@render cell(row, column)}</th>
            {:else}
              <td>{@render cell(row, column)}</td>
            {/if}
          {/each}
        </tr>
      {:else}
        <tr>
          <td colspan={columns.length} class="muted">{empty}</td>
        </tr>
      {/each}
    </tbody>
  </table>
</div>

<style>
  /* The region scrolls in both directions and keeps the header in view. On a phone the soft shadows
     on the left and right edge show that more columns wait off screen (they fade out at the end). */
  .scroll {
    overflow: auto;
    max-height: min(75vh, 52rem);
    border: var(--nv-border-width) solid var(--nv-border);
    border-radius: var(--nv-radius);
    box-shadow: var(--nv-shadow-1);
    background:
      linear-gradient(to right, var(--nv-surface) 30%, transparent) left center / 2rem 100% no-repeat local,
      linear-gradient(to left, var(--nv-surface) 30%, transparent) right center / 2rem 100% no-repeat local,
      linear-gradient(to right, color-mix(in srgb, var(--nv-text) 16%, transparent), transparent) left center / 0.75rem 100% no-repeat scroll,
      linear-gradient(to left, color-mix(in srgb, var(--nv-text) 16%, transparent), transparent) right center / 0.75rem 100% no-repeat scroll,
      var(--nv-surface);
  }
  table {
    border-collapse: separate;
    border-spacing: 0;
    width: 100%;
  }
  caption {
    text-align: start;
    font-weight: 650;
    padding: var(--nv-space-3) var(--nv-space-4);
    background: var(--nv-surface);
  }
  th,
  td {
    text-align: start;
    padding: var(--nv-space-3) var(--nv-space-4);
    border-top: var(--nv-border-width) solid var(--nv-border);
    vertical-align: middle;
    overflow-wrap: anywhere;
  }
  td {
    min-width: 6rem;
  }
  td :global(a),
  tbody th :global(a) {
    display: inline-flex;
    align-items: center;
    min-height: var(--nv-target);
  }
  /* A header never breaks inside a word, and a column never gets narrower than a few characters:
     on a phone the table scrolls sideways (the region is focusable) instead of squeezing text
     into one letter per line. */
  thead th {
    position: sticky;
    inset-block-start: 0;
    z-index: 1;
    background: var(--nv-surface-2);
    font-size: var(--nv-text-caption);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--nv-text-muted);
    white-space: nowrap;
    overflow-wrap: normal;
  }
  tbody tr {
    background: var(--nv-surface);
    transition: background var(--nv-motion) var(--nv-ease);
  }
  tbody tr:nth-child(even) {
    background: color-mix(in srgb, var(--nv-surface-2) 45%, var(--nv-surface));
  }
  tbody tr:hover {
    background: var(--nv-tint-primary);
  }
  tbody th {
    font-weight: 600;
  }
  .sort {
    display: inline-flex;
    align-items: center;
    gap: var(--nv-space-1);
    min-height: var(--nv-target);
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    letter-spacing: inherit;
    text-transform: inherit;
    font-weight: 700;
    cursor: pointer;
  }
  .sort:hover {
    color: var(--nv-text);
  }
</style>
