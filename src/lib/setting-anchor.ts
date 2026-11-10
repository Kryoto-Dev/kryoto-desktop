/**
 * How a settings group (`Section` in ui/index.tsx) is addressed: an anchor made
 * from its title. Kept apart from lib/settings-links.ts, which knows Desktop's
 * Settings page, because ui/index.tsx is shared with Kryoto Mobile.
 */

/** A section title as its anchor: "Chat & calls" is `chat-and-calls`. */
export function anchorOf(title: string): string {
  return title
    .toLowerCase()
    .replace(/&/g, ' and ')
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
}

/** The element id an anchor is drawn with. Prefixed, so it never meets an id of the page around it. */
export const anchorId = (anchor: string) => `setting-${anchor}`
