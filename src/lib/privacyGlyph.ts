// The one drawing of the privacy-zone badge, as path data: the AI's four-point
// sparkle, struck out. PrivacyIcon.svelte renders it everywhere in the app; the
// editor's private-callout icon (a CodeMirror widget, plain DOM) draws the same
// two paths from here, so the mark is defined once.
export const PRIVACY_GLYPH_VIEWBOX = "0 0 20 20";
export const PRIVACY_GLYPH_STAR = "M10 2.5 Q12.6 7.4 17.5 10 Q12.6 12.6 10 17.5 Q7.4 12.6 2.5 10 Q7.4 7.4 10 2.5 Z";
export const PRIVACY_GLYPH_SLASH = "M3.5 3.5 L16.5 16.5";

/** The badge as an inline SVG string (for DOM built outside Svelte). */
export function privacyGlyphSvg(size: number): string {
  return `<svg width="${size}" height="${size}" viewBox="${PRIVACY_GLYPH_VIEWBOX}" aria-hidden="true"><path d="${PRIVACY_GLYPH_STAR}" fill="currentColor"/><path d="${PRIVACY_GLYPH_SLASH}" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>`;
}
