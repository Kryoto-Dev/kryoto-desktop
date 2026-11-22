/**
 * Releases Kryoto's auto-fixer (Repair game: `crates/kryoto-repair`, Properties
 * > Emulator & fixes, Kryoto Online's Set up, Try another emulator) cannot fix.
 *
 * A hypervisor-style release (Phantom Blade Zero) gets past its protection
 * with a hypervisor bypass and its own setup: players were offered Repair
 * game, ran it, and staff then had to tell them not to (five tickets,
 * 2026-11-21). kryo.to marks such a release `hypervisor: true` (kryo.to
 * lib/release-notices.ts, read off the release's Source or Label) at the top
 * of `/api/games/<slug>` and of `/downloads`, and on each of its `releases`.
 * While it is marked, Kryoto shows the release's notice in place of the
 * auto-fixer.
 *
 * The field's name lives here alone: read it only through `autofixBlock`.
 * Absent (an older kryo.to), or anything but `true`, is a normal release.
 */
export const NO_AUTOFIX_FIELD = 'hypervisor'

/** The game page's own words for such a release (kryo.to app/game/[slug]/page.tsx), and what that means here. */
export const NO_AUTOFIX_TEXT =
  "This release uses a hypervisor bypass. It needs its own setup before the game starts, and your antivirus will warn about it. Kryoto's repair cannot fix it, and running it can stop the game from starting: follow the steps on its kryo.to page instead."

/** Null for a normal release; else what to show instead of the auto-fixer. `game` is kryo.to's game (or release) object as the API sends it. */
export function autofixBlock(game: Record<string, unknown> | null | undefined): { notice: string } | null {
  if (!game) return null
  return game[NO_AUTOFIX_FIELD] === true ? { notice: NO_AUTOFIX_TEXT } : null
}
