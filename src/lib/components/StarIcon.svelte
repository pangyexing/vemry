<script lang="ts">
  // The one drawing of the bookmark star (following RefIcon: one icon is drawn in one
  // place). Both bookmarking entry points — the main window's page card and the clip
  // toolbar at the bottom of a browser window — share it, each at its own size.
  let { filled = false, size = 14 }: { filled?: boolean; size?: number } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 20 20"
  fill={filled ? "currentColor" : "none"}
  stroke="currentColor"
  stroke-width="1.5"
  stroke-linejoin="round"
  aria-hidden="true"
>
  <path d="M10 2.5l2.32 4.7 5.18.75-3.75 3.66.89 5.16L10 14.33l-4.64 2.44.89-5.16L2.5 7.95l5.18-.75L10 2.5z" />
</svg>
