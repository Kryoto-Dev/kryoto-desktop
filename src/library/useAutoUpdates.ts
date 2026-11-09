import { useEffect, useRef } from 'react'
import { fetchCatalogGame, knowsBuild, latestFor, type LibraryGame } from '@/lib/library'
import { isActive, type Download } from '@/lib/downloads'

/** How often kryo.to is asked about the library's games. */
const CHECK_EVERY_MS = 6 * 60 * 60 * 1000
/** Between two games' checks, so a big library is not a burst. */
const GAP_MS = 1500

/** A game an update may be started for: from kryo.to, on a known build, not kept, its files not the player's own. */
export function followsUpdates(g: LibraryGame): boolean {
  return !!g.slug && knowsBuild(g) && !g.keepFiles && !(g.pinnedVersion && g.pinnedVersion === g.version)
}

/**
 * The next game to update, or null. One at a time, never while a game is
 * running (its files are in use) or any download is going, never a game
 * already tried this run (a failure waits for the next launch rather than
 * looping), and never one with its own download queued.
 */
export function nextUpdate(
  games: LibraryGame[],
  latest: Record<string, string | null>,
  running: Set<string>,
  downloads: Download[],
  tried: Set<string>,
): LibraryGame | null {
  if (running.size > 0 || downloads.some(isActive)) return null
  return (
    games.find((g) => {
      if (!followsUpdates(g) || tried.has(g.slug!)) return false
      const want = latest[g.slug!]
      return !!want && want !== (g.version ?? '')
    }) ?? null
  )
}

/**
 * Settings > Downloads > "Update my games automatically". Off: nothing here
 * runs. On: every six hours (and when the connection comes back) each
 * installed game's current build is asked from kryo.to, and newer ones are
 * started one by one through the same download as the Update button, which
 * backs up and can roll back (src-tauri game_update.rs).
 */
export function useAutoUpdates({
  enabled,
  online,
  games,
  running,
  downloads,
  latest,
  noteLatest,
  start,
}: {
  enabled: boolean
  online: boolean
  games: LibraryGame[]
  running: Set<string>
  downloads: Download[]
  latest: Record<string, string | null>
  noteLatest: (slug: string, latest: string | null) => void
  start: (game: LibraryGame) => void
}) {
  const gamesRef = useRef(games)
  gamesRef.current = games
  const tried = useRef(new Set<string>())

  // Checking.
  useEffect(() => {
    if (!enabled || !online) return
    let cancelled = false
    async function check() {
      for (const g of gamesRef.current.filter(followsUpdates)) {
        if (cancelled) return
        try {
          const c = await fetchCatalogGame(g.slug!)
          const latest = await latestFor(g, c.version)
          if (!cancelled) noteLatest(g.slug!, latest)
        } catch {
          // One game kryo.to cannot answer for does not stop the rest.
        }
        await new Promise((r) => setTimeout(r, GAP_MS))
      }
    }
    void check()
    const timer = window.setInterval(() => void check(), CHECK_EVERY_MS)
    return () => {
      cancelled = true
      window.clearInterval(timer)
    }
  }, [enabled, online, noteLatest])

  // Starting, one at a time, when nothing else is going.
  useEffect(() => {
    if (!enabled || !online) return
    const next = nextUpdate(games, latest, running, downloads, tried.current)
    if (!next) return
    tried.current.add(next.slug!)
    start(next)
  }, [enabled, online, games, latest, running, downloads, start])
}
