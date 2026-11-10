import { viaStore } from '@/lib/friends'
import { carry } from '@/lib/launchers'
import type { LibraryGame } from '@/lib/library'
import { countedHere, playtimeGap, syncKey } from '@/lib/playtime-math'
import { logWarn } from '@/lib/log'
import { pendingPlaySeconds } from '@/lib/play-reports'

/**
 * Play time this PC counted that kryo.to never got.
 *
 * Sessions reach the account one by one (`play-reports.ts`), but some never
 * did: a 401 dropped them until 0.3.1, a session Kryoto kept from before it
 * closed (`recover_sessions`) was never sent, nor one under a minute. So a
 * write-up offered fewer hours than the library showed (BeamNG, 2026-11-09:
 * 2.5 h here, 1.7 h on kryo.to). This sends the difference once, as carried
 * time, the way a game linked later brings its hours along.
 *
 * Only what this PC has beyond the account, minus what is still on its way,
 * and never while the game runs. The key names both counts, so a repeated send
 * counts once and a later difference is a new one. A game that another
 * launcher keeps, or that kryo.to does not list, sends nothing.
 */

// The sums (what to send, its key, what this PC counted) are in playtime-math.ts, tested there.
export { playtimeGap, syncKey, countedHere, SYNC_MIN_SECONDS } from '@/lib/playtime-math'

/** After a send, leave that game alone this long so the answer can land first. */
const SETTLE_MS = 5 * 60_000
/** The account route takes at most this many games in one ask. */
const BATCH = 50

const settled = new Map<string, { local: number; at: number }>()
let syncing = false

/** Send what is missing. Quiet when the Store is not there or nothing is missing. */
export async function syncPlaytime(games: LibraryGame[], running: string[]): Promise<void> {
  if (syncing) return
  syncing = true
  try {
    const now = Date.now()
    const todo = countedHere(games, running).filter((r) => {
      const s = settled.get(r.slug)
      return !s || (s.local !== r.local && now - s.at > SETTLE_MS)
    })
    for (let i = 0; i < todo.length; i += BATCH) {
      const batch = todo.slice(i, i + BATCH)
      // On their way before the ask, or after it: a session that lands while
      // the account is read is subtracted rather than counted twice.
      const before = new Map(batch.map((r) => [r.slug, pendingPlaySeconds(r.slug)]))
      const account = await viaStore({ method: 'GET', path: `/api/account/playtime?slugs=${batch.map((r) => r.slug).join(',')}` })
      for (const r of batch) {
        const has = Number(account[r.slug] ?? 0)
        const onTheWay = Math.max(before.get(r.slug) ?? 0, pendingPlaySeconds(r.slug)) + r.carrying
        const gap = playtimeGap(r.local, Number.isFinite(has) ? has : 0, onTheWay)
        if (gap > 0) await carry.send(r.id, r.slug, gap, syncKey(r.slug, r.local, has))
        settled.set(r.slug, { local: r.local, at: Date.now() })
      }
    }
  } catch (e) {
    // Signed out, offline, the Store not loaded: the next round tries again.
    logWarn('playtime', `sync: ${String(e)}`)
  } finally {
    syncing = false
  }
}
