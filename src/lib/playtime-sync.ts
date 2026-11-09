import { viaStore } from '@/lib/friends'
import { carry } from '@/lib/launchers'
import type { LibraryGame } from '@/lib/library'
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

/** Less than this is not worth a send. */
export const SYNC_MIN_SECONDS = 60
/** After a send, leave that game alone this long so the answer can land first. */
const SETTLE_MS = 5 * 60_000
/** The account route takes at most this many games in one ask. */
const BATCH = 50

/** Seconds to send: what this PC has beyond the account and what is still on its way. Pure. */
export function playtimeGap(local: number, account: number, onTheWay: number): number {
  const gap = Math.floor(local - account - onTheWay)
  return gap >= SYNC_MIN_SECONDS ? gap : 0
}

/** The send's key: both counts, so the same difference counts once. At most 80 characters. Pure. */
export function syncKey(slug: string, local: number, account: number): string {
  return `carry-sync-${Math.floor(local)}-${Math.floor(account)}-${slug.slice(0, 40)}`.replace(/[^a-zA-Z0-9-]/g, '-')
}

type Slug = { id: string; slug: string; local: number; carrying: number }

/** This PC's count per kryo.to game, for the games Kryoto itself counts. Pure. */
export function countedHere(games: LibraryGame[], running: string[]): Slug[] {
  const by = new Map<string, Slug>()
  const busy = new Set<string>()
  for (const g of games) {
    if (!g.slug || g.launcher) continue
    if (running.includes(g.id)) busy.add(g.slug)
    const row = by.get(g.slug) ?? { id: g.id, slug: g.slug, local: 0, carrying: 0 }
    row.local += g.playtimeSeconds || 0
    row.carrying += (g.carrySends ?? []).reduce((sum, c) => sum + (c.seconds || 0), 0)
    by.set(g.slug, row)
  }
  return [...by.values()].filter((r) => r.local > 0 && !busy.has(r.slug))
}

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
