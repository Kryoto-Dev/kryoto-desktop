import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { errorText, isTauri } from '@/lib/bridge'
import { library, type LibraryGame } from '@/lib/library'
import { notify } from '@/lib/notify'
import { alarms, plainSummary } from '@/lib/release-files'
import { quickExit } from '@/lib/quick-exit'

/**
 * The Library's live state: the saved games, which are running, and the last
 * thing that went wrong. Playtime is written by Rust when a game closes, so a
 * `game-state` event reloads the list rather than guessing the new totals.
 */
export function useLibrary() {
  const [games, setGames] = useState<LibraryGame[]>([])
  const [running, setRunning] = useState<Set<string>>(new Set())
  /**
   * Pressed Play, process not there yet.
   *
   * Before the game's process exists the client finds its exe, picks the
   * Proton or Wine to run it with and writes the start of its log; on Linux,
   * with a prefix to set up, that is seconds. The Play button used to sit
   * there until the process appeared, which read as a press that did nothing,
   * so the game counts as running from the press. The process's own
   * `game-state` takes over when it comes.
   */
  const [starting, setStarting] = useState<Set<string>>(new Set())
  /** Stop was pressed before the process existed: stop it as soon as it does. */
  const stopWhenUp = useRef<Set<string>>(new Set())
  const pendingLaunches = useRef<Map<string, { cancelled: boolean; stopped: boolean }>>(new Map())
  const runningRevision = useRef(0)
  const without = (set: Set<string>, id: string) => {
    if (!set.has(id)) return set
    const next = new Set(set)
    next.delete(id)
    return next
  }
  const [loaded, setLoaded] = useState(false)
  const [error, setErrorState] = useState<string | null>(null)
  const [refreshError, setRefreshError] = useState<string | null>(null)
  const generation = useRef(0)
  /**
   * The current error is an expected outcome to tell the player, not a fault
   * to report to kryo.to (Shell logs the rest).
   */
  const [errorQuiet, setErrorQuiet] = useState(false)
  /** The game it is about, whose page may say it; none is for a notice. */
  const [errorGame, setErrorGame] = useState<string | null>(null)
  /** A launch failure's catalog slug, for the logged line only (`reportSlug`). */
  const [errorSlug, setErrorSlug] = useState<string | null>(null)
  const setError = useCallback((message: string | null, quiet = false, game: string | null = null, slug: string | null = null) => {
    setErrorState(message)
    setErrorQuiet(quiet)
    setErrorGame(game)
    setErrorSlug(slug)
  }, [])
  const gamesRef = useRef<LibraryGame[]>([])
  useEffect(() => {
    gamesRef.current = games
  }, [games])

  const reload = useCallback(async () => {
    const request = ++generation.current
    const revision = runningRevision.current
    try {
      const [list, live] = await Promise.all([library.list(), library.running()])
      if (request !== generation.current) return
      setGames(list)
      if (revision === runningRevision.current) setRunning(new Set(live.filter(id => !stopWhenUp.current.has(id))))
      setRefreshError(null)
    } catch (e) {
      if (request === generation.current) setRefreshError(errorText(e))
    } finally {
      if (request === generation.current) setLoaded(true)
    }
  }, [])

  // A download that just installed adds a game from the Rust side.
  useEffect(() => {
    let stop: (() => void) | undefined
    let cancelled = false
    void library
      .onChanged(() => void reload())
      .then((fn) => {
        if (cancelled) fn()
        else stop = fn
      })
    return () => {
      cancelled = true
      stop?.()
    }
  }, [reload])

  useEffect(() => {
    let stop: (() => void) | undefined
    let cancelled = false
    void library
      .onState((event) => {
        const cancelledStart = stopWhenUp.current.has(event.id)
        runningRevision.current++
        setRunning((current) => {
          const next = new Set(current)
          if (event.running && !cancelledStart) next.add(event.id)
          else next.delete(event.id)
          return next
        })
        setStarting((current) => without(current, event.id))
        if (event.running && stopWhenUp.current.has(event.id)) void library.stop(event.id).catch((e) => setError(errorText(e), false, event.id))
        // The list changed with it: lastPlayed is stamped at launch, playtime added at exit.
        void reload()
        if (!event.running) {
          // A game gone within seconds almost never ran - say so instead of
          // leaving a Play button that silently did nothing.
          if (!event.stopped && !stopWhenUp.current.has(event.id) && (event.seconds ?? 0) < 5) {
            const [message, quiet] = quickExit(event.code, event.detail, event.steam)
            setError(message, quiet, event.id, reportSlug(gamesRef.current.find((g) => g.id === event.id)))
            checkFilesAfterFailure(gamesRef.current.find((g) => g.id === event.id))
          }
          stopWhenUp.current.delete(event.id)
        }
      })
      .then((fn) => {
        if (cancelled) fn()
        else stop = fn
        if (!cancelled) void reload()
      })
    return () => {
      cancelled = true
      stop?.()
    }
  }, [reload])

  /** `vr`: "Play in VR", the headset's runtime made ready first. */
  const play = useCallback(async (id: string, entry: number | null, joinLobby?: string, vr = false) => {
    if (pendingLaunches.current.has(id)) return
    const request = { cancelled: false, stopped: false }
    pendingLaunches.current.set(id, request)
    stopWhenUp.current.delete(id)
    setError(null)
    setStarting((current) => new Set(current).add(id))
    try {
      if (vr) await library.launchVr(id, entry)
      else await library.launch(id, entry, joinLobby)
      if (request.cancelled && !request.stopped) await library.stop(id)
      // Reconcile even if the start/exit event arrived before the listener.
      if (pendingLaunches.current.get(id) === request) {
        setStarting((current) => without(current, id))
        void reload()
      }
    } catch (e) {
      // It never started: back to Play, and say why.
      if (pendingLaunches.current.get(id) === request) {
        setStarting((current) => without(current, id))
        // Started from somewhere else (the phone, the Store): that is what was
        // wanted, and the button only has to catch up.
        if (errorText(e).includes('already running')) void reload()
        else if (!request.cancelled) {
          setError(errorText(e), false, id)
          // Its file gone or blocked (library.rs start_failure): which ones, exactly.
          if (FILE_TROUBLE.test(errorText(e))) checkFilesAfterFailure(gamesRef.current.find((g) => g.id === id))
        }
      }
    } finally {
      if (pendingLaunches.current.get(id) === request) pendingLaunches.current.delete(id)
      if (!pendingLaunches.current.has(id)) stopWhenUp.current.delete(id)
    }
  }, [reload])

  const stopGame = useCallback(async (id: string) => {
    stopWhenUp.current.add(id)
    const pending = pendingLaunches.current.get(id)
    if (pending) pending.cancelled = true
    try {
      await library.stop(id)
      if (pending) {
        pending.stopped = true
        if (pendingLaunches.current.get(id) === pending) pendingLaunches.current.delete(id)
      }
      runningRevision.current++
      setStarting((current) => without(current, id))
      setRunning((current) => without(current, id))
      void reload()
    } catch (e) {
      // The stop request can reach Rust before the launch command. Keep the
      // cancellation queued for its acknowledgement, with immediate feedback.
      if (errorText(e).includes('not running')) {
        runningRevision.current++
        setStarting((current) => without(current, id))
        setRunning((current) => without(current, id))
        if (!pending) stopWhenUp.current.delete(id)
        void reload()
      } else {
        if (pending) pending.cancelled = false
        stopWhenUp.current.delete(id)
        setError(errorText(e), false, id)
      }
    }
  }, [reload])

  const upsert = useCallback((game: LibraryGame) => {
    setGames((list) => {
      const i = list.findIndex((g) => g.id === game.id)
      if (i < 0) return [...list, game]
      const next = list.slice()
      next[i] = game
      return next
    })
  }, [])

  const drop = useCallback((id: string) => setGames((list) => list.filter((g) => g.id !== id)), [])

  // Starting counts as running everywhere the client shows it.
  const live = useMemo(() => (starting.size ? new Set([...running, ...starting]) : running), [running, starting])

  const runningNow = useRef(running)
  runningNow.current = running
  const confirmation = useRef(0)

  /**
   * Which games Rust has running. The list is read again only when that
   * changed without its event (lastPlayed is stamped at launch, playtime added
   * at exit). Starting games are left alone: they are not running there yet.
   */
  const confirmRunning = useCallback(async () => {
    const request = ++confirmation.current
    const revision = runningRevision.current
    let ids: string[]
    try {
      ids = await library.running()
    } catch {
      return // asked again on the next tick
    }
    if (request !== confirmation.current || revision !== runningRevision.current) return
    const now = new Set(ids.filter((id) => !stopWhenUp.current.has(id)))
    const current = runningNow.current
    if (now.size === current.size && [...now].every((id) => current.has(id))) return
    // A start or exit went without its event.
    void reload()
  }, [reload])

  // IPC events can be missed during a remount or a reload. Confirm active
  // sessions against Rust so a lost exit event cannot leave Stop stuck.
  useEffect(() => {
    if (!live.size) return
    const timer = window.setInterval(() => void confirmRunning(), 2_000)
    return () => window.clearInterval(timer)
  }, [live.size, confirmRunning])

  return { games, running: live, loaded, error: error ?? refreshError, errorQuiet: error ? errorQuiet : false, errorGame: error ? errorGame : null, errorSlug: error ? errorSlug : null, setError, reload, play, stop: stopGame, upsert, drop }
}

/**
 * The slug a logged launch failure carries, so "It closed again straight
 * away" reports can be grouped by game. Only a kryo.to catalog game's: none
 * for a game the player added themselves or brought from another launcher,
 * and none for one they keep hidden.
 */
export function reportSlug(game: Pick<LibraryGame, 'slug' | 'origin' | 'launcher' | 'hidden'> | undefined): string | null {
  if (!game?.slug || game.origin || game.launcher || game.hidden) return null
  return /^[a-z0-9-]{1,160}$/.test(game.slug) ? game.slug : null
}

/** library.rs `start_failure`'s words for a file that is gone or that an antivirus blocked. */
const FILE_TROUBLE = /gone from the game's folder|antivirus blocked/i

/**
 * A start that failed within seconds: the game's files checked against its
 * release's own list (src-tauri release_files.rs), which names on the game's
 * page any file an antivirus took. With the window in the background, the
 * system notification says it too, as for the app's other news (lib/notify.ts).
 * A release with no list is left as it was: nothing is checked or said.
 */
function checkFilesAfterFailure(game: LibraryGame | undefined) {
  if (!game?.slug || game.launcher || !isTauri()) return
  void library
    .filesCheck(game.id)
    .then((s) => {
      const found = alarms([...(s.report?.missing ?? []), ...(s.report?.broken ?? [])])
      if (!found.length || document.hasFocus()) return
      const missing = found.some((f) => f.problem === 'missing')
      void notify(`${game.title} ${missing ? 'is missing files' : 'has damaged files'}`, `${plainSummary(found, 2)} Open the game to get them back.`)
    })
    .catch(() => {})
}

