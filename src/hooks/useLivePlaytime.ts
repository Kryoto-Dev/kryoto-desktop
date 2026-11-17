import { useEffect, useRef, useState } from 'react'
import type { LibraryGame } from '@/lib/library'
import { sessionStart } from '@/lib/playtime-math'

/**
 * A game's play time, counting the session running now.
 *
 * The session is counted from `lastPlayed`, which Rust stamps when the game's
 * process starts. Pressed Play here, the game counts as running at once, while
 * `lastPlayed` still holds the previous session's start until the list
 * reloads: the card added the whole time since then (25.3 hours became 27.3
 * on pressing Play, KB-0344). So a start seen here counts from when it was
 * seen, until the fresh `lastPlayed` arrives. A game already running when the
 * view opened counts from its `lastPlayed`, as before.
 */
export function useLivePlaytime(game: Pick<LibraryGame, 'playtimeSeconds' | 'lastPlayed'>, running: boolean): number {
  const [tick, setTick] = useState(() => Date.now())
  // When this view saw the game start (seconds), or null when it was already running.
  const seenStart = useRef<number | null>(null)
  const wasRunning = useRef(running)
  if (running && !wasRunning.current) seenStart.current = Math.floor(Date.now() / 1000)
  if (!running) seenStart.current = null
  wasRunning.current = running
  useEffect(() => {
    if (!running) return
    setTick(Date.now())
    const timer = window.setInterval(() => setTick(Date.now()), 15_000)
    return () => window.clearInterval(timer)
  }, [running])
  if (!running) return game.playtimeSeconds
  const since = sessionStart(game.lastPlayed ?? null, seenStart.current)
  const session = since === null ? 0 : Math.max(0, Math.floor(tick / 1000 - since))
  return game.playtimeSeconds + session
}

