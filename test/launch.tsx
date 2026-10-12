import { createRoot } from 'react-dom/client'
import { useLibrary } from '@/hooks/useLibrary'
import { useWebBackend } from '@/lib/bridge'
import type { LibraryGame } from '@/lib/library'

const game = { id: 'fixture', title: 'Fixture game', slug: null, cover: null, hero: null, installDir: 'fixture', executable: 'game.exe', defaultArgs: '', entries: [], source: null, preferredEntry: null, launchOptions: '', compatTool: null, applyOverrides: false, playtimeSeconds: 0, lastPlayed: null, addedAt: 1, version: null, short: null, developer: null, nsfw: false } satisfies LibraryGame
const listeners = new Map<string, Set<(v: unknown) => void>>()
const live = new Set<string>()
const pending: { resolve: (v: unknown) => void; reject: (e: unknown) => void }[] = []
let deferredRunning: ((v: unknown) => void) | null = null
let deferRunning = false
let stopError: string | null = null
let launches = 0
let stops = 0
useWebBackend({
  call: (command) => {
    if (command === 'library_list') return Promise.resolve([game])
    if (command === 'game_running') {
      if (deferRunning) { deferRunning = false; return new Promise(resolve => { deferredRunning = resolve }) }
      return Promise.resolve([...live])
    }
    if (command === 'game_launch') { launches++; return new Promise((resolve, reject) => { pending.push({ resolve, reject }) }) }
    if (command === 'game_stop') {
      stops++
      if (stopError) return Promise.reject(new Error(stopError))
      live.clear()
      return Promise.resolve(null)
    }
    return Promise.resolve(null)
  },
  on: (event, fn) => {
    const entries = listeners.get(event) ?? new Set()
    entries.add(fn); listeners.set(event, entries)
    return () => { entries.delete(fn) }
  },
})
Object.assign(window, { launchFixture: {
  stats: () => ({ launches, stops }),
  acknowledge: () => { pending.shift()?.resolve(null) },
  fail: (message: string) => { pending.shift()?.reject(new Error(message)) },
  stopError: (message: string | null) => { stopError = message },
  live: (running: boolean) => { if (running) live.add(game.id); else live.delete(game.id) },
  event: (running: boolean, stopped = false) => { listeners.get('game-state')?.forEach(fn => fn({ id: game.id, running, seconds: running ? null : 1, code: running ? null : 0, stopped })) },
  deferRunning: () => { deferRunning = true },
  staleRunning: () => { deferredRunning?.([game.id]); deferredRunning = null },
} })
function Harness() {
  const lib = useLibrary()
  const running = lib.running.has(game.id)
  return <main>
    <button id="play-stop" onClick={() => void (running ? lib.stop(game.id) : lib.play(game.id, null))}>{running ? 'Stop' : 'Play'}</button>
    <button id="reload" onClick={() => void lib.reload()}>Reload</button>
    <output id="error">{lib.error}</output>
    <output id="loaded">{String(lib.loaded)}</output>
  </main>
}
createRoot(document.getElementById('root')!).render(<Harness />)
