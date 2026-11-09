import { useEffect, useMemo, useRef, useState } from 'react'
import { NotebookPen, X } from '@/ui/pixel-icons'
import { Button, Caption, IconButton } from '@/ui'
import { useSettings } from '@/lib/settings'
import { viaStore } from '@/lib/friends'
import { isTauri } from '@/lib/bridge'
import { logWarn } from '@/lib/log'
import { usePromptTurn, useQuiet } from '@/lib/prompts'
import { markSeen } from '@/ui/FeatureNotice'
import { hoursAndMinutes } from '@/shell/PlaytimePrompt'
import type { LibraryGame } from '@/lib/library'

/** About two hours in Kryoto Desktop is enough to have something to say. */
export const WRITEUP_AFTER_SECONDS = 2 * 3600
/** Only a game played lately: nobody is asked about one they left a year ago. */
const RECENT_MS = 14 * 86_400_000
/** "Don't ask again", for every game. */
export const WRITEUP_NUDGES_OFF = 'writeup-nudges-off'

/**
 * The games already asked about (or written up), on this PC. Kept beside the
 * shell rather than in `seenNotices`, which takes short ids only and at most
 * 64 of them; a lost list only means one more polite question per game.
 */
const ASKED_KEY = 'kryoto.writeupAsked'
const MAX_ASKED = 500
function askedGames(): Set<string> {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(ASKED_KEY) ?? '[]')
    return new Set(Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : [])
  } catch {
    return new Set()
  }
}
function settleGame(slug: string) {
  try {
    const list = [...askedGames(), slug].slice(-MAX_ASKED)
    localStorage.setItem(ASKED_KEY, JSON.stringify([...new Set(list)]))
  } catch {
    // Storage refused: it may ask once more after a restart.
  }
}

/**
 * A small card, once per game, after about two hours of it: write it up on
 * kryo.to? Never while anything is running, never over a dialog (it waits its
 * turn in lib/prompts.ts like the other cards), at most once per start of the
 * app, and only for a kryo.to game the account has not written up yet.
 * "Not now" settles that game; "Don't ask again" settles every game.
 */
export function WriteupNudge({
  games,
  running,
  signedIn,
  onWeb,
}: {
  games: LibraryGame[]
  running: ReadonlySet<string>
  signedIn: boolean
  /** Opens a kryo.to path in the Store. */
  onWeb: (path: string) => void
}) {
  const settings = useSettings()
  const quiet = useQuiet(20000)
  const [shown, setShown] = useState<LibraryGame | null>(null)
  const [done, setDone] = useState(false)
  const asked = useRef(new Set<string>())
  // Bumped when an answer settles a game, so the next one is looked at.
  const [answers, setAnswers] = useState(0)
  const mounted = useRef(true)
  useEffect(() => {
    // Set here too: a dev build mounts twice, and the first cleanup would leave it false.
    mounted.current = true
    return () => {
      mounted.current = false
    }
  }, [])

  // The most recently played game that qualifies, if any.
  const candidate = useMemo(() => {
    if (!settings || !signedIn || done || running.size) return null
    if (settings.seenNotices.includes(WRITEUP_NUDGES_OFF)) return null
    const settled = askedGames()
    const now = Date.now()
    return (
      games
        .filter(
          (g) =>
            g.slug &&
            !g.hidden &&
            g.playtimeSeconds >= WRITEUP_AFTER_SECONDS &&
            g.lastPlayed != null &&
            now - g.lastPlayed * 1000 < RECENT_MS &&
            !settled.has(g.slug) &&
            !asked.current.has(g.slug),
        )
        .sort((a, b) => (b.lastPlayed ?? 0) - (a.lastPlayed ?? 0))[0] ?? null
    )
    // `answers` re-reads `asked`, which is a ref.
  }, [games, running, settings, signedIn, done, answers])

  // Ask kryo.to whether it is written up already, and that the game takes write-ups.
  useEffect(() => {
    if (!candidate?.slug || shown || !quiet || !isTauri()) return
    const slug = candidate.slug
    // Asked once per start of the app; the library re-polling does not cancel the answer.
    asked.current.add(slug)
    const game = candidate
    viaStore({ method: 'GET', path: `/api/games/${slug}/writeups?mine=1` })
      .then((data) => {
        if (!mounted.current) return
        if (data.mine === true || data.open !== true) {
          // Nothing to ask about this game, now or later.
          if (data.mine === true) settleGame(slug)
          setAnswers((n) => n + 1)
          return
        }
        setShown(game)
      })
      .catch((e: unknown) => {
        logWarn('writeup', `could not ask about ${slug}: ${String(e)}`)
        // kryo.to could not be asked (the Store was loading, or offline): try again in a minute.
        window.setTimeout(() => {
          if (!mounted.current) return
          asked.current.delete(slug)
          setAnswers((n) => n + 1)
        }, 60_000)
      })
  }, [candidate, shown, quiet])

  // A game started meanwhile: the card waits for later rather than sitting over play.
  const wanted = Boolean(shown) && running.size === 0
  const turn = usePromptTurn('writeup', wanted)
  if (!shown?.slug || !turn) return null
  const slug = shown.slug

  const close = (forGood: boolean) => {
    settleGame(slug)
    if (forGood) markSeen(WRITEUP_NUDGES_OFF)
    setShown(null)
    setDone(true)
  }

  return (
    <aside
      role="status"
      aria-label={`Write up ${shown.title}?`}
      className="kryo-pop kryo-radius grid w-[380px] max-w-[calc(100vw-32px)] gap-3 border border-border bg-card/95 p-4 shadow-2xl shadow-black/60 backdrop-blur"
    >
      <div className="flex items-start justify-between gap-3">
        <Caption>Write-ups</Caption>
        <IconButton label="Not now" onClick={() => close(false)} className="-mr-1 -mt-1 size-7">
          <X className="size-3.5" />
        </IconButton>
      </div>
      <div className="grid gap-1">
        <p className="text-sm font-bold text-foreground">
          {hoursAndMinutes(shown.playtimeSeconds)} in {shown.title}. What did you think?
        </p>
        <p className="text-xs leading-relaxed text-muted-foreground">
          A write-up on its kryo.to page helps the next player decide: what worked, what did not, and whether you recommend it.
        </p>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          onClick={() => {
            onWeb(`/game/${encodeURIComponent(slug)}#write-up`)
            close(false)
          }}
        >
          <NotebookPen className="size-3" />
          Write one
        </Button>
        <Button size="sm" variant="ghost" onClick={() => close(false)}>
          Not now
        </Button>
        <Button size="sm" variant="ghost" className="ml-auto" onClick={() => close(true)}>
          Don&apos;t ask again
        </Button>
      </div>
    </aside>
  )
}
