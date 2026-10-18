import { useCallback, useEffect, useRef, useState } from 'react'
import { ChevronDown, Cloud, EyeOff, Trophy, Wrench } from 'lucide-react'
import { AsciiBar, Button, Caption, Card, Label, Modal } from '@/ui'
import { call, errorText, on } from '@/lib/bridge'
import type { LibraryGame } from '@/lib/library'
import { cn } from '@/lib/utils'
import { artSrc } from '@/lib/art'
import { navigateCatalog } from '@/lib/window'
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
  tracking: 'supported' | 'unsupported' | 'unknown'
  emulator: string | null
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
 * into Repair; a game with no achievements still needs no card.
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
      {data?.tracking === 'unsupported' && ignored !== notice ? (
        <Card className="grid gap-3">
          <Label>Achievements</Label>
          <p className="text-sm font-bold text-foreground">Achievement tracking is unavailable</p>
          <p className="text-xs leading-relaxed text-muted-foreground">
            This game uses <b className="text-foreground">{data.emulator}</b>. Kryoto Desktop cannot track achievements with this emulator.
          </p>
          <p className="text-xs leading-relaxed text-muted-foreground">
            You can try switching to gbe_fork or RUNE for achievement tracking. The game may stop working, and Kryoto Online multiplayer will be unavailable. Repair keeps a backup so you can Undo and try another emulator if it breaks.
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
          {cloud?.perk != null ? (
            <p className="flex items-center gap-2 border-t border-border pt-3 text-[11px] text-muted-foreground">
              <Cloud className="size-3.5 shrink-0" aria-hidden />
              <span className="grow">{cloud.perk ? 'Saved to your kryo.to account, and back on any PC you sign in on.' : 'Supporters keep their achievements on their kryo.to account.'}</span>
              <button
                type="button"
                onClick={() => navigateCatalog(cloud.perk ? '/settings#achievements' : '/donate')}
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

function Row({ a }: { a: GameAchievement }) {
  const secret = a.hidden && !a.unlocked
  const picture = a.unlocked ? a.icon : (a.iconGray ?? a.icon)
  return (
    <li className={cn('grid grid-cols-[40px_1fr_auto] items-center gap-3', !a.unlocked && 'opacity-60')}>
      <span className="grid size-10 place-items-center overflow-hidden border border-border bg-background">
        {secret ? (
          <EyeOff className="size-4 text-muted-foreground" aria-hidden />
        ) : picture ? (
          <img src={artSrc(picture) ?? undefined} alt="" className={cn('size-full object-cover', !a.unlocked && !a.iconGray && 'grayscale')} />
        ) : (
          <Trophy className="size-4 text-muted-foreground" aria-hidden />
        )}
      </span>
      <span className="min-w-0">
        <span className="block truncate text-xs font-bold text-foreground">{secret ? 'Hidden achievement' : a.displayName || a.name}</span>
        <span className="block text-[11px] leading-snug text-muted-foreground">
          {secret ? 'Keep playing to find out.' : a.description}
        </span>
      </span>
      <span className="text-right text-[10px] tabular-nums text-muted-foreground">
        {a.unlocked && a.unlockedAt ? new Date(a.unlockedAt * 1000).toLocaleDateString() : a.percent != null ? `${a.percent.toFixed(1)}%` : null}
      </span>
    </li>
  )
}
