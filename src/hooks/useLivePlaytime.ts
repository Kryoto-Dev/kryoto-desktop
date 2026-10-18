import { useEffect, useState } from 'react'
import type { LibraryGame } from '@/lib/library'

/**
 * A game's playtime including the session in progress. The saved total only
 * grows when the game closes, so while it runs this adds the time since it was
 * launched (lastPlayed is stamped at launch), refreshed every 15 seconds.
 */
export function useLivePlaytime(game: Pick<LibraryGame, 'playtimeSeconds' | 'lastPlayed'>, running: boolean): number {
  const [tick, setTick] = useState(() => Date.now())
  useEffect(() => {
    if (!running) return
    setTick(Date.now())
    const timer = window.setInterval(() => setTick(Date.now()), 15_000)
    return () => window.clearInterval(timer)
  }, [running])
  const session = running && game.lastPlayed ? Math.max(0, Math.floor(tick / 1000 - game.lastPlayed)) : 0
  return game.playtimeSeconds + session
}
