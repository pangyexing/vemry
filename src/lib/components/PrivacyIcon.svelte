<script lang="ts">
  // The one drawing of the privacy-zone badge (following Chevron/RefIcon/StarIcon: one
  // icon is drawn in one place): the AI's four-point sparkle, struck out — "no agent
  // here", which is exactly what a privacy zone is (the content does not exist to the
  // model, while a person sees all of it). The sparkle is the same mark the pinned
  // VEMRY.md row uses for the agent's memory file. It is solid rather than outlined
  // because the badge lives at 12–14px, where a 1.6 stroke thins to a hairline and an
  // outlined star reads as an asterisk; the slash lies over the whole body, since a
  // knocked-out slash cuts the star into two arrowheads.
  // Every "this is in the privacy zone" mark shares it — the status bar, a zone root in
  // the file tree, a note badge in the projection views, a private reference chip in the
  // chat, the privacy row of the session replay — instead of each reaching for an emoji
  // (which every platform's fonts render differently).
  let { size = 14 }: { size?: number } = $props();
</script>

<svg width={size} height={size} viewBox="0 0 20 20" aria-hidden="true">
  <path d="M10 2.5 Q12.6 7.4 17.5 10 Q12.6 12.6 10 17.5 Q7.4 12.6 2.5 10 Q7.4 7.4 10 2.5 Z" fill="currentColor" />
  <path d="M3.5 3.5 L16.5 16.5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
</svg>
