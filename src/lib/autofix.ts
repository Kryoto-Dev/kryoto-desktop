/**
 * Releases Kryoto's auto-fixer (Repair game: `crates/kryoto-repair`, Properties
 * > Emulator & fixes, Kryoto Online's Set up, Try another emulator) cannot fix.
 *
 * A hypervisor-style release (Phantom Blade Zero) gets past its protection
 * with a hypervisor bypass and its own setup: players were offered Repair
 * game, ran it, and staff then had to tell them not to (five tickets,
 * 2026-11-21). kryo.to marks such a release `hypervisor: true` (kryo.to
 * lib/release-notices.ts, read off the release's Source or Label), and any
 * release staff block (Denuvo, say) `autofix: "blocked"` with a short
 * `autofix_reason` (kryo.to lib/release-files-format.ts `effectiveAutofix`).
 * Both sit at the top of `/api/games/<slug>` and of `/downloads`, and on each
 * of its `releases`. While either says so, Kryoto shows the release's notice
 * in place of the auto-fixer.
 *
 * The fields' names live here alone: read them only through `autofixBlock`.
 * Absent (an older kryo.to), or any other value, is a normal release.
 */
export const NO_AUTOFIX_FIELD = 'hypervisor'
export const AUTOFIX_FIELD = 'autofix'
export const AUTOFIX_BLOCKED = 'blocked'
export const AUTOFIX_REASON_FIELD = 'autofix_reason'

/** The game page's own words for such a release (kryo.to app/game/[slug]/page.tsx), and what that means here. */
export const NO_AUTOFIX_TEXT =
  "This release uses a hypervisor bypass. It needs its own setup before the game starts, and your antivirus will warn about it. Kryoto's repair cannot fix it, and running it can stop the game from starting: follow the steps on its kryo.to page instead."

/** A release staff blocked for another reason (its own reason is shown beside it). */
export const BLOCKED_TEXT =
  "Kryoto's repair cannot fix this release, and running it can stop the game from starting: follow the steps on its kryo.to page instead."

/** kryo.to keeps a reason short; one sent longer is cut here, never drawn whole. */
const REASON_MAX = 120

/**
 * Null for a normal release; else what to show instead of the auto-fixer: the
 * notice and, when kryo.to gave one, the reason. `game` is kryo.to's answer
 * (the top of `/api/games/<slug>` or `/downloads`) or one of its releases.
 */
export function autofixBlock(game: Record<string, unknown> | null | undefined): { notice: string; reason: string | null } | null {
  if (!game) return null
  const hypervisor = game[NO_AUTOFIX_FIELD] === true
  const blocked = game[AUTOFIX_FIELD] === AUTOFIX_BLOCKED
  if (!hypervisor && !blocked) return null
  const raw = game[AUTOFIX_REASON_FIELD]
  const reason = typeof raw === 'string' && raw.trim() ? raw.trim().slice(0, REASON_MAX) : null
  // The hypervisor notice already says why; its reason would only repeat it.
  if (hypervisor) return { notice: NO_AUTOFIX_TEXT, reason: reason && !/hypervisor/i.test(reason) ? reason : null }
  return { notice: BLOCKED_TEXT, reason }
}
