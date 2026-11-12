import { useCallback, useEffect, useRef, useState } from 'react'
import { ChevronDown, Cloud, EyeOff, Trophy, Wrench } from '@/ui/pixel-icons'
import { AsciiBar, Busy, Button, Caption, Card, Label, Modal } from '@/ui'
import { call, errorText, on } from '@/lib/bridge'
import type { LibraryGame } from '@/lib/library'
import { cn } from '@/lib/utils'
import { artSrc } from '@/lib/art'
import { openSettingsSection, openStorePage } from '@/lib/window'
import { GameRepair } from './GameRepair'

/** One achievement and whether it is yours (achievements.rs, game_achievements). */
type GameAchievement = {
  name: string
  displayName: string
  description: string
  hidden: boolean
  icon: string | null
  iconGray: string | null
  percent: number | null
  unlocked: boolean
  unlockedAt: number | null
}
type GameAchievements = {
  appid: string | null
  total: number
  unlocked: number
  list: GameAchievement[]
  /** `fixable`: published DRM-free, so nothing records until the switch puts
   *  Kryoto's emulator in (achievements_turn_on). `absent`: no Steam API in
   *  the game at all, so there is nothing to show. */
  tracking: 'supported' | 'unsupported' | 'unknown' | 'fixable' | 'absent'
  emulator: string | null
  /** What Steam lists for the game, tracked or not. */
  available: number
}

const SHOWN = 8

/** Achievements saved to the kryo.to account (achievements_cloud.rs). */
type CloudStatus = { perk: boolean | null; games: number; lastSync: number | null }

function useCloudStatus() {
  const [s, setS] = useState<CloudStatus | null>(null)
  useEffect(() => {
    let stop: (() => void) | undefined
    let cancelled = false
    void call<CloudStatus>('cloud_achievements_status').then((v) => !cancelled && setS(v)).catch(() => {})
    void on<CloudStatus>('cloud-achievements', (v) => !cancelled && setS(v)).then((fn) => (cancelled ? fn() : (stop = fn)))
    return () => {
      cancelled = true
      stop?.()
    }
  }, [])
  return s
}

/**
 * A game's achievements: how far along you are, the latest unlocks and the
 * rest, rarest last. Read from the emulator's own save, so it fills in as you
 * play, here and in a notification. Hidden ones keep their secret until
 * they are unlocked. An incompatible emulator gets an explanation and a way
 * into Repair; a game published DRM-free gets one button that turns them on;
 * a game with no achievements, or no Steam API to report any, needs no card.
 */
export function AchievementsCard({ game, running = false }: { game: LibraryGame; running?: boolean }) {
  const [data, setData] = useState<GameAchievements | null>(null)
  const [all, setAll] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [repairOpen, setRepairOpen] = useState(false)
  const [ignored, setIgnored] = useState<string | null>(null)
  const cloud = useCloudStatus()
  const request = useRef(0)
  const ignoreKey = `kryoto.achievements.ignore.${game.id}`
  const notice = `${game.version ?? ''}:${data?.emulator ?? ''}`

  useEffect(() => {
    try { setIgnored(localStorage.getItem(ignoreKey)) }
    catch { setIgnored(null) }
  }, [ignoreKey])

  const load = useCallback(() => {
    const revision = ++request.current
    setError(null)
    void call<GameAchievements>('game_achievements', { id: game.id })
      .then((next) => { if (revision === request.current) setData(next) })
      .catch((e: unknown) => {
        if (revision !== request.current) return
        setData(null)
        setError(errorText(e))
      })
  }, [game.id])

  // A switched game starts blank; a refresh of the same game keeps what is on
  // screen until the answer arrives, so the card never flashes while it plays.
  useEffect(() => {
    setData(null)
    setAll(false)
  }, [game.id])

  // The library hands out fresh objects on every reload, so depend on what
  // changes tracking (emulator, Online, add-ons), not on object identity.
  const trackingKey = `${game.source ?? ''}|${game.version ?? ''}|${game.online ? 1 : 0}|${(game.addons ?? []).map((a) => `${a.label}:${a.files.join(',')}`).join(';')}`
  useEffect(() => {
    load()
    return () => { request.current++ }
  }, [load, trackingKey])

  // A new unlock while the page is open.
  useEffect(() => {
    const stops: (() => void)[] = []
    let cancelled = false
    const keep = (fn: () => void) => cancelled ? fn() : stops.push(fn)
    void on<{ gameId: string }>('achievement-unlocked', (e) => {
      if (e.gameId === game.id) load()
    }).then(keep).catch(() => {})
    void on('library-changed', load).then(keep).catch(() => {})
    return () => {
      cancelled = true
      stops.forEach((stop) => stop())
    }
  }, [game.id, load])

  function ignore() {
    setIgnored(notice)
    try { localStorage.setItem(ignoreKey, notice) } catch { /* Still hide it for this visit. */ }
  }

  const shown = data ? (all ? data.list : data.list.slice(0, SHOWN)) : []

  return (
    <>
      {data?.tracking === 'fixable' && data.available > 0 ? (
        <TurnOn game={game} available={data.available} running={running} />
      ) : data?.tracking === 'unsupported' && ignored !== notice ? (
        <Card className="grid gap-3">
          <Label>Achievements</Label>
          <p className="text-sm font-bold text-foreground">Achievement tracking is unavailable</p>
          <p className="text-xs leading-relaxed text-muted-foreground">
            This game uses <b className="text-foreground">{data.emulator}</b>. Kryoto Desktop cannot track achievements with it.
          </p>
          <p className="text-xs leading-relaxed text-muted-foreground">
            You can switch its Steam emulator to gbe_fork or RUNE for achievement tracking. Only the Steam files change; the rest of the crack stays as it is. The game may stop working, and Kryoto Online multiplayer will be unavailable. Repair keeps a backup so you can Undo and try another emulator if it breaks.
          </p>
          <div className="flex flex-wrap gap-2">
            {game.slug ? <Button size="sm" disabled={running} onClick={() => setRepairOpen(true)}><Wrench className="size-3" aria-hidden /> Try another emulator</Button> : null}
            <Button size="sm" variant="ghost" onClick={ignore}>Ignore for this game</Button>
          </div>
          {running ? <p className="text-[11px] text-muted-foreground">Close the game before switching its emulator.</p> : null}
          {!game.slug ? <p className="text-[11px] text-muted-foreground">Link this game to its Kryoto page under Properties &gt; kryo.to to use Repair.</p> : null}
        </Card>
      ) : error ? (
        <Card className="grid gap-3">
          <Label>Achievements</Label>
          <p className="text-xs text-muted-foreground">Achievements could not be loaded. Try again.</p>
          <p className="break-words text-[11px] text-muted-foreground">{error}</p>
          <Button size="sm" className="justify-self-start" onClick={load}>Retry</Button>
        </Card>
      ) : data && data.tracking !== 'unsupported' && data.total > 0 ? (
        <Card className="grid gap-4">
          <div className="flex items-baseline justify-between gap-3">
            <Label>Achievements</Label>
            <Caption>{data.unlocked} of {data.total}</Caption>
          </div>
          <AsciiBar fraction={data.unlocked / data.total} cells={32} />
          <ul className="grid gap-2">
            {shown.map((a) => <Row key={a.name} a={a} />)}
          </ul>
          {data.list.length > SHOWN ? (
            <Button size="sm" variant="outline" className="justify-self-start" onClick={() => setAll((v) => !v)}>
              <ChevronDown className={cn('size-3 transition-transform', all && 'rotate-180')} aria-hidden />
              {all ? 'Show fewer' : `Show all ${data.list.length}`}
            </Button>
          ) : null}
          {cloud?.perk != null && data.appid ? (
            <p className="flex items-center gap-2 border-t border-border pt-3 text-[11px] text-muted-foreground">
              <Cloud className="size-3.5 shrink-0" aria-hidden />
              <span className="grow">{cloud.perk ? 'Saved to your kryo.to account, and back on any PC you sign in on.' : 'Supporters keep their achievements on their kryo.to account.'}</span>
              <button
                type="button"
                onClick={() => (cloud.perk ? openSettingsSection('cloud') : openStorePage('/donate'))}
                className="shrink-0 font-bold uppercase tracking-wider text-foreground hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring"
              >
                {cloud.perk ? 'Manage' : 'See how'}
              </button>
            </p>
          ) : null}
        </Card>
      ) : null}
      {repairOpen ? <Modal title="Switch emulator for achievements" onClose={() => setRepairOpen(false)}><GameRepair gameId={game.id} purpose="achievements" /></Modal> : null}
    </>
  )
}

/**
 * A game published DRM-free: Valve's own Steam files, which count
 * achievements only while Steam runs. One button puts Kryoto's emulator in
 * their place through Repair (backed up, undone under Properties > Repair);
 * the card then becomes the list, as for any other game.
 */
function TurnOn({ game, available, running }: { game: LibraryGame; available: number; running: boolean }) {
  const [busy, setBusy] = useState(false)
  const [status, setStatus] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!busy) return
    let stop: (() => void) | undefined
    let cancelled = false
    void on<{ gameId: string; text: string }>('repair-progress', (p) => {
      if (!cancelled && p.gameId === game.id) setStatus(p.text)
    }).then((fn) => (cancelled ? fn() : (stop = fn))).catch(() => {})
    return () => {
      cancelled = true
      stop?.()
    }
  }, [busy, game.id])

  async function turnOn() {
    if (busy) return
    setBusy(true)
    setError(null)
    setStatus(null)
    // On success the library changes and the card reloads as the list.
    try { await call('achievements_turn_on', { id: game.id }) }
    catch (e) { setError(errorText(e)) }
    finally { setBusy(false); setStatus(null) }
  }

  return (
    <Card className="grid gap-3">
      <div className="flex items-baseline justify-between gap-3">
        <Label>Achievements</Label>
        <Caption>{available} on Steam</Caption>
      </div>
      <p className="text-sm font-bold text-foreground">This copy does not record achievements</p>
      <p className="text-xs leading-relaxed text-muted-foreground">
        It came without a Steam emulator, so its achievements only count while Steam runs. Kryoto can add one so they unlock and show here. You can undo it under Properties &gt; Repair.
      </p>
      {game.slug ? (
        <Button size="sm" className="justify-self-start" disabled={busy || running} onClick={() => void turnOn()}>
          {busy ? <Busy className="size-3" /> : <Trophy className="size-3" aria-hidden />} Turn on achievements
        </Button>
      ) : (
        <p className="text-[11px] text-muted-foreground">Link this game to its Kryoto page under Properties &gt; kryo.to first.</p>
      )}
      {running ? <p className="text-[11px] text-muted-foreground">Close the game first.</p> : null}
      {busy && status ? <p className="text-[11px] text-muted-foreground" role="status" aria-live="polite">{status}</p> : null}
      {error ? <p className="break-words text-[11px] text-destructive" role="alert">{error}</p> : null}
    </Card>
  )
}

/**
 * A locked hidden achievement keeps its secret until it is clicked: the row is
 * then a button, and a second click hides it again (kryo.to suggestion #78).
 */
function Row({ a }: { a: GameAchievement }) {
  const [revealed, setRevealed] = useState(false)
  const hiddenLocked = a.hidden && !a.unlocked
  const secret = hiddenLocked && !revealed
  const picture = a.unlocked ? a.icon : (a.iconGray ?? a.icon)
  const body = (
    <>
      <span className="grid size-10 place-items-center overflow-hidden border border-border bg-background">
        {secret ? (
          <EyeOff className="size-4 text-muted-foreground" aria-hidden />
        ) : picture ? (
          <img decoding="async" src={artSrc(picture) ?? undefined} alt="" className={cn('size-full object-cover', !a.unlocked && !a.iconGray && 'grayscale')} />
        ) : (
          <Trophy className="size-4 text-muted-foreground" aria-hidden />
        )}
      </span>
      <span className="min-w-0">
        <span className="block truncate text-xs font-bold text-foreground">{secret ? 'Hidden achievement' : a.displayName || a.name}</span>
        <span className="block text-[11px] leading-snug text-muted-foreground">
          {secret ? 'Keep playing to find out, or click to see it.' : a.description}
        </span>
      </span>
      <span className="text-right text-[10px] tabular-nums text-muted-foreground">
        {a.unlocked && a.unlockedAt ? new Date(a.unlockedAt * 1000).toLocaleDateString() : a.percent != null ? `${a.percent.toFixed(1)}%` : null}
      </span>
    </>
  )
  const grid = cn('grid grid-cols-[40px_1fr_auto] items-center gap-3', !a.unlocked && 'opacity-60')
  if (!hiddenLocked) return <li className={grid}>{body}</li>
  return (
    <li>
      <button
        type="button"
        aria-expanded={revealed}
        title={revealed ? 'Hide it again' : 'Show this hidden achievement'}
        onClick={() => setRevealed((r) => !r)}
        className={cn(grid, 'w-full text-left outline-none hover:bg-secondary/50 focus-visible:ring-1 focus-visible:ring-foreground')}
      >
        {body}
      </button>
    </li>
  )
}
