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
  // The path data lives in privacyGlyph.ts so the editor's private-callout icon (plain
  // DOM inside CodeMirror) draws the identical mark.
  import { PRIVACY_GLYPH_SLASH, PRIVACY_GLYPH_STAR, PRIVACY_GLYPH_VIEWBOX } from "$lib/privacyGlyph";
  let { size = 14 }: { size?: number } = $props();
</script>

<svg width={size} height={size} viewBox={PRIVACY_GLYPH_VIEWBOX} aria-hidden="true">
  <path d={PRIVACY_GLYPH_STAR} fill="currentColor" />
  <path d={PRIVACY_GLYPH_SLASH} fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
</svg>
