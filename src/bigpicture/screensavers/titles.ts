/**
 * Which game titles a screensaver may show. A screensaver plays on a TV in a
 * shared room, so it is stricter than the library:
 *
 * - never a hidden game (the Hidden group is hidden from other people, and a
 *   TV is other people);
 * - never an adult game unless the reader shows adult art (Settings >
 *   Interface) and does not hide adult games (the account's "Hide").
 *
 * Played last first, then the rest by name. Kept free of imports so
 * `pnpm test` can load it.
 */

export type TitleSource = {
  title: string
  hidden?: boolean | null
  nsfw?: boolean | null
  lastPlayed?: number | null
}

/** At most this many: a screensaver cycles through them, it is not a list. */
export const MAX_TITLES = 24

export function saverTitles(games: readonly TitleSource[], adult: { show: boolean; hide: boolean }): string[] {
  const adultOk = adult.show && !adult.hide
  return games
    .filter((g) => !g.hidden && (adultOk || !g.nsfw) && g.title.trim())
    .slice()
    .sort((a, b) => (b.lastPlayed ?? 0) - (a.lastPlayed ?? 0) || a.title.localeCompare(b.title))
    .slice(0, MAX_TITLES)
    .map((g) => g.title.trim())
}

/** The game played last, among the ones that may be shown; none if nothing was played. */
export function lastPlayedTitle(games: readonly TitleSource[], adult: { show: boolean; hide: boolean }): string | null {
  const shown = new Set(saverTitles(games, adult))
  let best: TitleSource | null = null
  for (const g of games) if (g.lastPlayed && shown.has(g.title.trim()) && (!best || g.lastPlayed > (best.lastPlayed ?? 0))) best = g
  return best ? best.title.trim() : null
}

/** A title in the screensavers' type: capitals, plain ASCII, cut to fit. */
export function saverText(title: string, max: number): string {
  const plain = title
    .replace(/[™®©]/g, '')
    .normalize('NFKD')
    .replace(/[̀-ͯ]/g, '')
    .replace(/[‘’]/g, "'")
    .replace(/[–—]/g, '-')
    .replace(/[^\x20-\x7e]/g, '')
    .replace(/\s+/g, ' ')
    .trim()
    .toUpperCase()
  if (plain.length <= max) return plain
  return `${plain.slice(0, Math.max(1, max - 3)).trimEnd()}...`
}
