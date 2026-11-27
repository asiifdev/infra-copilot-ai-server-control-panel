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
    empty?: string;
    captionId: string;
  }

  let { caption, columns, rows, rowKey, cell, empty = '', captionId }: Props = $props();

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
<div class="nv-table" role="region" aria-labelledby={captionId} tabindex="0">
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
