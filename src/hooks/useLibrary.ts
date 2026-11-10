import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { errorText } from '@/lib/bridge'
import { library, type LibraryGame } from '@/lib/library'

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
  const setError = useCallback((message: string | null, quiet = false, game: string | null = null) => {
    setErrorState(message)
    setErrorQuiet(quiet)
    setErrorGame(game)
  }, [])

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
        if (!event.running) {
          void reload()
          // A game gone within seconds almost never ran - say so instead of
          // leaving a Play button that silently did nothing.
          if (!event.stopped && !stopWhenUp.current.has(event.id) && (event.seconds ?? 0) < 5) {
            const [message, quiet] = quickExit(event.code)
            setError(message, quiet, event.id)
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
        else if (!request.cancelled) setError(errorText(e), false, id)
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

  // IPC events can be missed during a remount or a reload. Confirm active
  // sessions against Rust so a lost exit event cannot leave Stop stuck.
  useEffect(() => {
    if (!live.size) return
    const timer = window.setInterval(() => void reload(), 2_000)
    return () => window.clearInterval(timer)
  }, [live.size, reload])

  return { games, running: live, loaded, error: error ?? refreshError, errorQuiet: error ? errorQuiet : false, errorGame: error ? errorGame : null, setError, reload, play, stop: stopGame, upsert, drop }
}

const RUNTIMES_HINT = 'Install everything under Settings > Game runtimes, then press Play again.'

/**
 * What a game's exit code means, for the ones that keep coming back in reports.
 * "Check the exe" sent these people to the wrong place: their exe was right.
 */
const EXIT_REASONS: Record<number, string> = {
  // 0xC0000906: Windows refused to run a file it flagged.
  [-1073739514]: "Your antivirus blocked a file in this game as a threat. Exclude the game's folder, then get the file back: both are on the game's page.",
  // 0xC0000142: a DLL failed as it loaded, mostly a blocked or missing one.
  [-1073741502]: `A file the game needs could not start, usually because an antivirus blocked it or a runtime is missing. ${RUNTIMES_HINT}`,
  // 0xC06D007E: a DLL loaded late was not found, the same missing runtime.
  [-1066598274]: `A file the game needs is missing, usually a runtime. ${RUNTIMES_HINT}`,
  // 0xC0000139: a DLL is there but too old for the game.
  [-1073741511]: `A runtime on this PC is too old for the game. ${RUNTIMES_HINT}`,
  // 0xC000007B: a 32-bit and a 64-bit file mixed up, nearly always a broken runtime.
  [-1073741701]: `A runtime on this PC is damaged or the wrong kind for the game. ${RUNTIMES_HINT}`,
  // 0xC0000005 and 0xC0000409: the game itself crashed.
  [-1073741819]: 'The game crashed as it started. Update your graphics driver and use Repair game. If it keeps crashing, report it on the game\'s page.',
  [-1073740791]: 'The game crashed as it started. Update your graphics driver and use Repair game. If it keeps crashing, report it on the game\'s page.',
  // 0xE0434352: an unhandled .NET error.
  [-532462766]: `The game crashed with a .NET error. ${RUNTIMES_HINT}`,
  // The Steam DRM's own code: the game would not start without Steam.
  53: 'The game stopped at its Steam check. If this keeps happening, report it on the game\'s page so the release can be fixed.',
}

/**
 * What to say when a game was gone within seconds, and whether that is worth
 * reporting. The client already follows a launcher that hands over to the real
 * game (handoff.rs), so reaching this means nothing from the game's folder was
 * running afterwards either.
 */
function quickExit(code: number | null | undefined): [string, boolean] {
  // ERROR_CANCELLED: the game needs administrator rights and Windows' prompt was turned down.
  if (code === 1223) return ['It did not start: this game needs administrator rights. Press Play and answer Yes on Windows\' prompt.', true]
  // 0x80131700: the .NET runtime could not start, so it is missing.
  if (code === -2146232576) {
    return [
      'It closed straight away because the .NET Framework it needs is not installed. Install .NET Framework 4.8 (and 3.5 for older games) from Microsoft, then press Play again.',
      true,
    ]
  }
  // 0xC0000135: a DLL it needs is missing, almost always a Visual C++ runtime.
  if (code === -1073741515) {
    return [
      'It closed straight away because a DLL it needs is missing. Install the Visual C++ Redistributables (2015-2022, x64 and x86) and the DirectX runtime, then press Play again.',
      true,
    ]
  }
  const known = code == null ? undefined : EXIT_REASONS[code]
  if (known) return [`${known} (exit code ${code})`, false]
  if (code === 0 || code == null) {
    return [
      'It started and closed again without an error. If no window opened, pick the game\'s own .exe (not a launcher) in Properties.',
      true,
    ]
  }
  return [
    `It closed again straight away (exit code ${code}). Check the launch options and the exe in Properties.`,
    false,
  ]
}
