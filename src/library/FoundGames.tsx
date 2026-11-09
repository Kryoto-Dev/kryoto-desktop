import { useEffect, useState } from 'react'
import { FolderSearch } from '@/ui/pixel-icons'
import { AsciiBar, Button, Check, Modal } from '@/ui'
import { errorText } from '@/lib/bridge'
import { artSrc } from '@/lib/art'
import { adultBlur, useShowAdult } from '@/lib/adult'
import { formatPlaytime } from '@/lib/format'
import { library, type FoundGame } from '@/lib/library'
import { useSettings } from '@/lib/settings'
import { logWarn } from '@/lib/log'
import { usePromptTurn, useQuiet } from '@/lib/prompts'
import { markSeen } from '@/ui/FeatureNotice'
import { cn } from '@/lib/utils'

/**
 * Games Kryoto installed that the library has lost (suggestion #58): every
 * install leaves a `.kryoto-game.json` in its folder, and the scan reads them
 * from the library folders (library.rs `library_find_kryoto_games`). Settings
 * > Storage has a button for it, and after an update Desktop looks once by
 * itself and asks, as Myth suggested. Never on a timer: D: is a hard disk.
 */
export function FoundGamesDialog({ games, title, onClose }: { games: FoundGame[]; title: string; onClose: (added: number) => void }) {
  const showAdult = useShowAdult()
  const [picked, setPicked] = useState<Set<string>>(() => new Set(games.map((g) => g.installDir)))
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const toggle = (dir: string) =>
    setPicked((p) => {
      const next = new Set(p)
      if (next.has(dir)) next.delete(dir)
      else next.add(dir)
      return next
    })
  const add = () => {
    setBusy(true)
    setError(null)
    library
      .restoreFound([...picked])
      .then((n) => onClose(n))
      .catch((e: unknown) => setError(errorText(e)))
      .finally(() => setBusy(false))
  }

  return (
    <Modal
      title={title}
      onClose={() => onClose(0)}
      footer={
        <>
          <Button variant="ghost" onClick={() => onClose(0)}>
            Not now
          </Button>
          <Button variant="primary" disabled={busy || picked.size === 0} onClick={add}>
            Add {picked.size === games.length ? 'them' : picked.size} back
          </Button>
        </>
      }
    >
      <p className="text-xs leading-relaxed text-muted-foreground">
        {games.length === 1 ? 'This game is' : `These ${games.length} games are`} in your library folders, not where your library has {games.length === 1 ? 'it' : 'them'}. They come back as they were, with
        their play time and settings. Nothing is downloaded.
      </p>
      <ul className="grid gap-1">
        {games.map((g) => (
          <li key={g.installDir} className={cn('kryo-radius flex items-center gap-3 border border-border p-2', !picked.has(g.installDir) && 'opacity-50')}>
            <Check checked={picked.has(g.installDir)} onChange={() => toggle(g.installDir)} label={<span className="sr-only">Add {g.title} back</span>} />
            {g.cover ? (
              <img
                decoding="async"
                src={artSrc(g.cover) ?? undefined}
                alt=""
                className={cn('h-12 w-8 shrink-0 bg-secondary object-cover', adultBlur(g.nsfw, showAdult))}
                style={{ borderRadius: 'min(var(--kryo-radius), 4px)' }}
                onError={(e) => (e.currentTarget.style.visibility = 'hidden')}
              />
            ) : (
              <span className="h-12 w-8 shrink-0 bg-secondary" style={{ borderRadius: 'min(var(--kryo-radius), 4px)' }} />
            )}
            <span className="grid min-w-0 grow">
              <span className="truncate text-sm text-foreground">{g.title}</span>
              <span className="truncate text-[11px] text-muted-foreground" title={g.installDir}>
                {[g.moved ? 'Moved here' : null, g.playtimeSeconds ? formatPlaytime(g.playtimeSeconds) : null, g.installDir].filter(Boolean).join(' · ')}
              </span>
            </span>
          </li>
        ))}
      </ul>
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
    </Modal>
  )
}

/** Settings > Storage: look now. */
export function FindGamesButton() {
  const [state, setState] = useState<'idle' | 'busy' | 'none' | FoundGame[]>('idle')
  const [note, setNote] = useState<string | null>(null)
  const find = () => {
    setState('busy')
    setNote(null)
    library
      .findKryotoGames()
      .then((games) => setState(games.length ? games : 'none'))
      .catch((e: unknown) => {
        setState('idle')
        setNote(errorText(e))
      })
  }
  return (
    <div className="grid gap-2">
      <Button size="sm" className="justify-self-start" disabled={state === 'busy'} onClick={find}>
        <FolderSearch className="size-3" aria-hidden />
        Find games Kryoto installed
      </Button>
      {state === 'busy' ? <AsciiBar fraction={null} cells={24} showPct={false} className="text-muted-foreground" /> : null}
      {state === 'none' ? <p className="text-[11px] text-muted-foreground">Every game Kryoto installed in your library folders is already in your library.</p> : null}
      {note ? <p className="text-[11px] text-muted-foreground">{note}</p> : null}
      {Array.isArray(state) ? (
        <FoundGamesDialog
          games={state}
          title="Games found in your library folders"
          onClose={(n) => {
            setState('idle')
            if (n) setNote(`${n} game${n === 1 ? '' : 's'} added back to your library.`)
          }}
        />
      ) : null}
    </div>
  )
}

/** Its id in `settings.seenNotices`, once per version. */
const checkId = () => `found-games-${__APP_VERSION__.replace(/\./g, '-')}`

/**
 * Once after Desktop updates (and after the first-start setup): look for lost
 * games when nothing else is on screen, and ask only if there are some. A look
 * that failed is tried again at the next start.
 */
export function FoundGamesAfterUpdate() {
  const settings = useSettings()
  const due = !!settings && settings.welcomed && !settings.seenNotices.includes(checkId())
  const quiet = useQuiet(8000, due)
  const [games, setGames] = useState<FoundGame[] | null>(null)
  const turn = usePromptTurn('found', !!games)
  useEffect(() => {
    if (!due || !quiet) return
    let alive = true
    library
      .findKryotoGames()
      .then((found) => {
        if (alive && found.length) setGames(found)
        markSeen(checkId())
      })
      .catch((e: unknown) => logWarn('library', `could not look for lost games: ${String(e)}`))
    return () => {
      alive = false
    }
  }, [due, quiet])
  if (!games || !turn) return null
  return <FoundGamesDialog games={games} title="Games missing from your library" onClose={() => setGames(null)} />
}
