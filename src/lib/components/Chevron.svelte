<script lang="ts">
  // The one drawing of the collapse chevron (following RefIcon/StarIcon: one icon is
  // drawn in one place). Four collapsing entry points share it — a folder row in the
  // file tree, a section head in the outline panel, a group in Agenda, a same-minute
  // cluster in the timeline. The rotation still belongs to each caller: the first two
  // are driven by a JS open class and the last two are <details open>, so the
  // selectors differ while the drawing exists only once.
  let { size = 10 }: { size?: number } = $props();
</script>

<svg width={size} height={size} viewBox="0 0 10 10" fill="none" aria-hidden="true">
  <path d="M3.5 2 L7 5 L3.5 8" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" />
</svg>
