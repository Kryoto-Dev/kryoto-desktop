// Play time this PC counted that kryo.to never got: the sums, with no imports so
// `node --test` can run them (playtime-sync.ts sends them).
import type { LibraryGame } from '@/lib/library'

/** Less than this is not worth a send. */
export const SYNC_MIN_SECONDS = 60

/** Seconds to send: what this PC has beyond the account and what is still on its way. Pure. */
export function playtimeGap(local: number, account: number, onTheWay: number): number {
  const gap = Math.floor(local - account - onTheWay)
  return gap >= SYNC_MIN_SECONDS ? gap : 0
}

/** The send's key: both counts, so the same difference counts once. At most 80 characters. Pure. */
export function syncKey(slug: string, local: number, account: number): string {
  return `carry-sync-${Math.floor(local)}-${Math.floor(account)}-${slug.slice(0, 40)}`.replace(/[^a-zA-Z0-9-]/g, '-')
}

export type Slug = { id: string; slug: string; local: number; carrying: number }

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

