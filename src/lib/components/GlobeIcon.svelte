<script lang="ts">
  // The one drawing of the globe (following StarIcon: one icon is drawn in one place). The
  // research panel header's browser entry and the chat composer's "open as a page" button
  // share it, each at its own size and stroke.
  let { size = 16, strokeWidth = 1.6 }: { size?: number; strokeWidth?: number } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 20 20"
  fill="none"
  stroke="currentColor"
  stroke-width={strokeWidth}
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  <path d="M3 10 a7 7 0 1 0 14 0 a7 7 0 1 0 -14 0 M3 10 H17 M10 3 c-3.6 3.2 -3.6 10.8 0 14 M10 3 c3.6 3.2 3.6 10.8 0 14" />
</svg>
