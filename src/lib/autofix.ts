/**
 * Releases Kryoto's auto-fixer (Repair game: `crates/kryoto-repair`, Properties
 * > Repair game, Kryoto Online's Set up, Try another emulator) cannot fix.
 *
 * Some releases are built in a way the auto-fixer does not understand, like
 * Phantom Blade Zero's hypervisor-style release: players were offered Repair
 * game, ran it, and staff then had to tell them not to (five tickets,
 * 2026-11-21). kryo.to marks such a release; while it is marked, Kryoto offers
 * the release's own notice in place of the auto-fixer.
 *
 * The field's name lives here alone, so it changes in one place: read it only
 * through `autofixBlock`. Absent, or anything it does not recognise, is a
 * normal release.
 */
export const NO_AUTOFIX_FIELD = 'no_autofix'
/** The release's own words for players, when kryo.to has them. */
export const NO_AUTOFIX_NOTICE_FIELD = 'no_autofix_notice'

export const NO_AUTOFIX_TEXT =
  "Kryoto's repair cannot fix this release, and running it can stop the game from starting. Follow the notes on its kryo.to page instead."

/**
 * Null for a normal release; else what to show instead of the auto-fixer.
 * `game` is kryo.to's game (or release) object as the API sends it. The mark
 * may be `true`, or the notice itself as text.
 */
export function autofixBlock(game: Record<string, unknown> | null | undefined): { notice: string } | null {
  if (!game) return null
  const mark = game[NO_AUTOFIX_FIELD]
  const text = (v: unknown) => (typeof v === 'string' && v.trim() ? v.trim().slice(0, 600) : null)
  if (mark === true) return { notice: text(game[NO_AUTOFIX_NOTICE_FIELD]) ?? NO_AUTOFIX_TEXT }
  const own = text(mark)
  return own ? { notice: own } : null
}
