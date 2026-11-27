<script lang="ts" module>
  // One shared set of 24x24 line icons (stroke follows the text colour). Paths only: no external
  // files, no text, so an icon never adds anything for assistive technology to read.
  export const ICONS = {
    overview: 'M3 3h7v9H3z M14 3h7v5h-7z M14 12h7v9h-7z M3 16h7v5H3z',
    nodes: 'M4 4h16v6H4z M4 14h16v6H4z M8 7h.01 M8 17h.01',
    findings: 'M10.3 3.9 2.4 17.5a2 2 0 0 0 1.7 3h15.8a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z M12 9v4 M12 17h.01',
    assistant: 'M12 3l1.8 4.7 4.7 1.8-4.7 1.8L12 16l-1.8-4.7-4.7-1.8 4.7-1.8z M19 15l.8 2.2 2.2.8-2.2.8L19 21l-.8-2.2-2.2-.8 2.2-.8z',
    proposals: 'M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z M8 12.5l2.7 2.7L16 9.8',
    plans: 'M9 4 3 6v14l6-2 6 2 6-2V4l-6 2z M9 4v14 M15 6v14',
    transports: 'M4 8h14 M14 4l4 4-4 4 M20 16H6 M10 12l-4 4 4 4',
    memory: 'M12 3 3 7.5l9 4.5 9-4.5z M3 12l9 4.5 9-4.5 M3 16.5l9 4.5 9-4.5',
    settings: 'M4 6h9 M17 6h3 M4 12h3 M11 12h9 M4 18h11 M19 18h1 M15 4v4 M9 10v4 M17 16v4',
    search: 'M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14z M20 20l-4-4',
    menu: 'M4 7h16 M4 12h16 M4 17h16',
    close: 'M6 6l12 12 M18 6 6 18',
    plus: 'M12 5v14 M5 12h14',
    sun: 'M12 16a4 4 0 1 0 0-8 4 4 0 0 0 0 8z M12 2v2 M12 20v2 M4.9 4.9l1.4 1.4 M17.7 17.7l1.4 1.4 M2 12h2 M20 12h2 M4.9 19.1l1.4-1.4 M17.7 6.3l1.4-1.4',
    globe: 'M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z M3 12h18 M12 3a14 14 0 0 1 0 18 14 14 0 0 1 0-18z',
    check: 'M5 12.5l4.5 4.5L19 7.5',
    alert: 'M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z M12 8v5 M12 16h.01',
    info: 'M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z M12 11v5 M12 8h.01',
    cross: 'M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z M9 9l6 6 M15 9l-6 6',
    circle: 'M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z',
    clock: 'M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z M12 7v5l3 2',
    chevron: 'M9 6l6 6-6 6',
    arrow: 'M5 12h14 M13 6l6 6-6 6',
    send: 'M21 3 10 14 M21 3l-7 18-4-7-7-4z',
    stop: 'M7 7h10v10H7z',
    shield: 'M12 3l8 3v6c0 4.5-3.3 8-8 9-4.7-1-8-4.5-8-9V6z',
    sort: 'M8 9l4-4 4 4 M8 15l4 4 4-4',
    up: 'M7 14l5-5 5 5',
    down: 'M7 10l5 5 5-5',
    inbox: 'M3 13l3-8h12l3 8 M3 13v6h18v-6 M3 13h5l1 3h6l1-3h5',
    return: 'M9 10l-5 5 5 5 M20 4v7a4 4 0 0 1-4 4H4',
  } as const;

  export type IconName = keyof typeof ICONS;
</script>

<script lang="ts">
  interface Props {
    name: IconName;
    /** Size in rem-based steps: sm 1rem, md 1.25rem, lg 1.5rem, xl 2rem. */
    size?: 'sm' | 'md' | 'lg' | 'xl';
  }

  let { name, size = 'md' }: Props = $props();
</script>

<svg
  class={['nv-icon', `nv-icon-${size}`]}
  data-icon={name}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  focusable="false"
>
  <path d={ICONS[name]} />
</svg>
