import { useMemo, useRef } from 'react'
import { gameContext } from './context'
import { ChevronDown, Download as DownloadIcon, Play, Square } from '@/ui/pixel-icons'
import { Button, Check, Label, MenuButton, Segmented } from '@/ui'
import { capsulesFor, type LibraryGame } from '@/lib/library'
import { formatLastPlayed, formatPlaytime } from '@/lib/format'
import { formatBytes, isActive, progressOf, type Download } from '@/lib/downloads'
import { useHideAdult } from '@/lib/adult'
import { STATUSES, STATUS_LABEL, isPlayStatus, useAdultFlags, type SavedEntry, type SavedStatus } from '@/hooks/useSaved'
import { useDiskSizes } from '@/hooks/useDiskSizes'
import { usePersisted } from '@/hooks/usePersisted'
import { useLivePlaytime } from '@/hooks/useLivePlaytime'
import { Art } from '@/library/Art'
import { coverFor, heroFor } from '@/lib/game-artwork'
import { cn } from '@/lib/utils'
import { BranchTag, copyLabel } from '@/library/BranchTag'
import { VrTag } from '@/library/VrTag'

/** A small Stop for a game that is running, where a card or row has no room for the big one. */
export function StopChip({ game, onStop, className }: { game: LibraryGame; onStop: (game: LibraryGame) => void; className?: string }) {
  return (
    <button
      type="button"
      aria-label={`Stop ${game.title}`}
      title="Stop"
      onClick={() => onStop(game)}
      className={cn(
        'kryo-pill grid size-7 place-items-center bg-background/80 text-foreground backdrop-blur transition-colors hover:bg-foreground hover:text-background',
        className,
      )}
    >
      <Square className="size-3 fill-current" />
    </button>
  )
}

/** A game's time played, counting up while it runs. */
function LivePlaytime({ game, running }: { game: LibraryGame; running: boolean }) {
  return <>{formatPlaytime(useLivePlaytime(game, running))}</>
}

type Sort = 'alpha' | 'recent' | 'playtime' | 'added' | 'size'

const SORTS: Record<Sort, string> = {
  alpha: 'A to Z',
  recent: 'Last played',
  playtime: 'Hours played',
  added: 'Recently added',
  size: 'Size on disk',
}
const isSort = (v: string): v is Sort => v in SORTS

type Filter = 'all' | SavedStatus
const isFilter = (v: string): v is Filter => v === 'all' || (STATUSES as string[]).includes(v)
const isFlag = (v: string): v is 'on' | 'off' => v === 'on' || v === 'off'

/**
 * Library home. The game you played last, big, one press from Play - then
 * Steam's Recent shelf and every game as a poster grid with Play on hover.
 *
 * Above the grid, the statuses the account gives games on kryo.to (Playing,
 * Plan to Play, On Hold...) filter it, and under them "Show games not on this
 * PC" adds the account's games that are not installed here, one press from
 * Install. "All" means every game they play, so "Not interested" stays out of
 * it unless picked.
 */
export function LibraryHome({
  games,
  running,
  onOpen,
  onPlay,
  onStop,
  onContext,
  saved = [],
  hiddenSlugs = [],
  downloads = [],
  onCatalogGame,
  onInstall,
  onSavedContext,
}: {
  games: LibraryGame[]
  running: Set<string>
  onOpen: (id: string) => void
  onPlay: (game: LibraryGame) => void
  onStop: (game: LibraryGame) => void
  onContext: (game: LibraryGame, x: number, y: number) => void
  /** The account's games on kryo.to, with their status. Empty for a guest. */
  saved?: SavedEntry[]
  /** The games the account hid: never in the "not on this PC" part. */
  hiddenSlugs?: string[]
  downloads?: Download[]
  /** Open a game that is not on this PC (its page in the Library). */
  onCatalogGame?: (slug: string) => void
  /** Install a game that is not on this PC. */
  onInstall?: (slug: string) => void
  onSavedContext?: (entry: SavedEntry, x: number, y: number) => void
}) {
  const [sort, setSort] = usePersisted<Sort>('kryoto.library.sort', 'alpha', isSort)
  const [picked, setFilter] = usePersisted<Filter>('kryoto.library.filter', 'all', isFilter)
  const [notHere, setNotHere] = usePersisted<'on' | 'off'>('kryoto.library.notHere', 'off', isFlag)
  const elsewhere = notHere === 'on'
  const statusOf = useMemo(() => new Map(saved.map((e) => [e.slug, e.status])), [saved])
  const here = useMemo(() => new Set(games.map((g) => g.slug).filter(Boolean)), [games])
  const hidden = useMemo(() => new Set(hiddenSlugs), [hiddenSlugs])
  const hideAdult = useHideAdult()
  // The account's games that are not installed here, minus the hidden ones.
  const away = useMemo(() => saved.filter((e) => !here.has(e.slug) && !hidden.has(e.slug)), [saved, here, hidden])
  const isAdult = useAdultFlags(useMemo(() => (elsewhere ? away.map((e) => e.slug) : []), [elsewhere, away]))
  // Hiding adult games on kryo.to keeps the ones not installed here out too.
  const awayListed = elsewhere ? away.filter((e) => !hideAdult || !isAdult(e.slug)) : []
  const inFilter = (status: SavedStatus | undefined, f: Filter) => (f === 'all' ? !status || isPlayStatus(status) : status === f)
  const used = STATUSES.filter((st) => saved.some((e) => e.status === st))
  // A remembered status nobody has any more falls back to All.
  const filter: Filter = picked === 'all' || used.includes(picked) ? picked : 'all'
  // Every installed game is under All, whatever its status.
  const shown = useMemo(() => (filter === 'all' ? games : games.filter((g) => g.slug && statusOf.get(g.slug) === filter)), [games, filter, statusOf])
  const count = (f: Filter) =>
    (f === 'all' ? games.length : games.filter((g) => g.slug && statusOf.get(g.slug) === f).length) + awayListed.filter((e) => inFilter(e.status, f)).length
  const awayShown = awayListed
    .filter((e) => inFilter(e.status, filter))
    .sort((a, b) => a.title.localeCompare(b.title, undefined, { sensitivity: 'base' }))
  const { sizes, measuring } = useDiskSizes(shown, sort === 'size')
  const recent = useMemo(
    () => games.filter((g) => g.lastPlayed).sort((a, b) => (b.lastPlayed ?? 0) - (a.lastPlayed ?? 0)),
    [games],
  )
  // Sizes arrive one game at a time. The grid waits for the last of them and
  // moves once, instead of reshuffling under the pointer with every answer.
  const settledSizes = useRef(sizes)
  if (!measuring) settledSizes.current = sizes
  const orderBy = settledSizes.current
  const sorted = useMemo(() => {
    const list = shown.slice()
    if (sort === 'alpha' || (sort === 'size' && !Object.keys(orderBy).length)) list.sort((a, b) => a.title.localeCompare(b.title, undefined, { sensitivity: 'base' }))
    if (sort === 'recent') list.sort((a, b) => (b.lastPlayed ?? 0) - (a.lastPlayed ?? 0))
    if (sort === 'playtime') list.sort((a, b) => b.playtimeSeconds - a.playtimeSeconds)
    if (sort === 'added') list.sort((a, b) => b.addedAt - a.addedAt)
    // Heaviest first; a game not measured yet, or with no folder, goes last.
    if (sort === 'size') list.sort((a, b) => (orderBy[b.id]?.bytes ?? 0) - (orderBy[a.id]?.bytes ?? 0) || a.title.localeCompare(b.title, undefined, { sensitivity: 'base' }))
    return list
  }, [shown, sort, orderBy])
  // A status picked is what they came for: its games come first, without the Continue and Recent rows.
  const last = filter === 'all' ? recent[0] : undefined
  const ctx = (g: LibraryGame) => gameContext((x, y) => onContext(g, x, y))

  return (
    <div className="grid min-h-0 grow content-start gap-8 overflow-auto p-6">
      {saved.length ? (
        <div className="grid gap-3">
          <Segmented
            label="Show games by status"
            value={filter}
            options={[
              { value: 'all' as Filter, label: `All · ${count('all')}` },
              ...used.map((st) => ({ value: st as Filter, label: `${STATUS_LABEL[st]} · ${count(st)}` })),
            ]}
            onChange={setFilter}
          />
          <Check checked={elsewhere} onChange={(on) => setNotHere(on ? 'on' : 'off')} label={away.length ? `Show games not on this PC (${away.length})` : 'Show games not on this PC'} />
        </div>
      ) : null}

      {last ? (
        <section
          {...ctx(last)}
          className="kryo-radius kryo-in relative h-64 overflow-hidden border border-border bg-card"
        >
          <Art adult={last.nsfw} src={heroFor(last)} fallback={[last.header, coverFor(last)]} title="" className="absolute inset-0 size-full object-cover" />
          <div className="hero-side absolute inset-0" />
          <div className="relative flex h-full flex-col justify-end gap-3 p-7">
            <span className="text-[10px] uppercase tracking-[0.25em] text-muted-foreground">
              {running.has(last.id) ? 'Playing now' : `Continue · ${formatLastPlayed(last.lastPlayed)}`}
            </span>
            <span className="flex gap-1.5 empty:hidden">
              <BranchTag branch={last.steamBranch} launcher={last.launcher} title={last.title} copy={copyLabel(last, games)} className="px-2 py-1 text-[10px]" />
            </span>
            <button type="button" onClick={() => onOpen(last.id)} className="kryo-square w-fit text-left">
              <h1 className="max-w-xl text-3xl font-bold leading-tight text-foreground">{last.title}</h1>
            </button>
            <div className="flex items-center gap-3">
              {running.has(last.id) ? (
                <Button variant="outline" size="lg" onClick={() => onStop(last)} className="border-foreground text-foreground">
                  <Square className="size-3.5 fill-current" />
                  Stop
                </Button>
              ) : (
                <Button variant="primary" size="lg" onClick={() => onPlay(last)}>
                  <Play className="size-4 fill-current" />
                  Play
                </Button>
              )}
              <span className="text-[10px] uppercase tracking-wider text-muted-foreground">
                {last.playtimeSeconds || running.has(last.id) ? (
                  <>
                    <LivePlaytime game={last} running={running.has(last.id)} /> played
                  </>
                ) : (
                  'Not played yet'
                )}
              </span>
            </div>
          </div>
        </section>
      ) : null}

      {filter === 'all' && recent.length > 1 ? (
        <section className="grid min-w-0 gap-3">
          <Label>Recent</Label>
          {/* Capped: with one or two recent games a `1fr` card filled the whole width at 460:215 and dwarfed the page (KB-0021). */}
          <div className="grid auto-cols-[minmax(260px,360px)] grid-flow-col justify-start gap-3 overflow-x-auto pb-1 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
            {recent.slice(1, 7).map((g) => (
              <div key={g.id} className="group/lift relative">
              <button
                type="button"
                onClick={() => onOpen(g.id)}
                onDoubleClick={() => onPlay(g)}
                {...ctx(g)}
                className="kryo-radius group grid w-full overflow-hidden border border-border bg-card text-left transition-[transform,border-color] duration-200 group-hover/lift:-translate-y-0.5 group-hover/lift:border-foreground/50"
              >
                <Art adult={g.nsfw} src={capsulesFor(g)[0]} fallback={capsulesFor(g).slice(1)} title={g.title} className="aspect-[460/215] w-full object-cover" />
                <span className="grid gap-0.5 px-3 py-2.5">
                  <span className="flex min-w-0 items-center gap-1.5">
                    <span className="truncate text-xs font-bold text-foreground">{g.title}</span>
                    <BranchTag branch={g.steamBranch} launcher={g.launcher} title={g.title} copy={copyLabel(g, games)} />
                  </span>
                  <span className="text-[10px] uppercase tracking-wider text-muted-foreground">
                    {running.has(g.id) ? <>Running now · <LivePlaytime game={g} running /></> : g.playtimeSeconds ? `${formatLastPlayed(g.lastPlayed)} · ${formatPlaytime(g.playtimeSeconds)}` : formatLastPlayed(g.lastPlayed)}
                  </span>
                </span>
              </button>
              {running.has(g.id) ? <StopChip game={g} onStop={onStop} className="absolute right-2 top-2" /> : null}
              </div>
            ))}
          </div>
        </section>
      ) : null}

      <section className="grid min-w-0 gap-3">
        <div className="flex items-center gap-3">
          <Label>
            {elsewhere ? 'On this PC' : filter === 'all' ? 'All games' : STATUS_LABEL[filter]} · {shown.length}
          </Label>
          {sort === 'size' && measuring > 0 ? <span className="text-[10px] uppercase tracking-wider text-muted-foreground">Measuring {measuring} more</span> : null}
          <span className="h-px grow bg-border" />
          <MenuButton
            align="right"
            className="kryo-pill flex h-7 items-center gap-1.5 border border-border px-3 text-[10px] uppercase tracking-wider text-muted-foreground hover:text-foreground"
            trigger={
              <>
                {SORTS[sort]}
                <ChevronDown className="size-3" />
              </>
            }
            items={(Object.keys(SORTS) as Sort[]).map((s) => ({ label: SORTS[s], onSelect: () => setSort(s) }))}
          />
        </div>
        <div className="grid grid-cols-[repeat(auto-fill,minmax(150px,1fr))] gap-4">
          {sorted.map((g) => (
            // The card lifts on hover; the hover is the cell's, which does not
            // move, so a pointer just under a lifted card keeps it up instead of
            // dropping and lifting it again and again (KB-0290).
            <div key={g.id} className="group/lift">
            <div
              {...ctx(g)}
              className="kryo-radius group relative aspect-[2/3] overflow-hidden border border-border bg-card transition-[transform,border-color] duration-200 group-hover/lift:-translate-y-1 group-hover/lift:border-foreground/60"
            >
              <button type="button" onClick={() => onOpen(g.id)} className="kryo-square absolute inset-0" aria-label={`Open ${g.title}`}>
                <Art adult={g.nsfw} src={coverFor(g)} fallback={[g.header]} title={g.title} className="size-full object-cover" />
              </button>
              <span className="pointer-events-none absolute right-2 top-2 flex flex-col items-end gap-1">
                {/* No title: the cover shows no name, so a branch the title names
                    ("GoreBox (public-beta)") still needs its pill here. */}
                <BranchTag branch={g.steamBranch} launcher={g.launcher} copy={copyLabel(g, games)} className="backdrop-blur" />
                <VrTag vr={g.vr} className="backdrop-blur" />
              </span>
              {sort === 'size' && sizes[g.id]?.bytes ? (
                <span className="kryo-pill pointer-events-none absolute bottom-3 left-2 bg-background/80 px-2 py-0.5 text-[10px] font-bold tabular-nums text-foreground backdrop-blur">
                  {formatBytes(sizes[g.id]?.bytes)}
                </span>
              ) : null}
              {running.has(g.id) ? (
                <span className="kryo-pill absolute left-2 top-2 flex items-center gap-1 bg-background/80 px-2 py-0.5 text-[9px] font-bold uppercase tracking-wider text-success backdrop-blur">
                  <span className="kryo-blink size-1.5 rounded-full bg-success" />
                  Running
                </span>
              ) : null}
              {running.has(g.id) ? (
                <button
                  type="button"
                  aria-label={`Stop ${g.title}`}
                  onClick={() => onStop(g)}
                  className="kryo-pill absolute bottom-3 right-3 grid size-11 min-h-[44px] min-w-[44px] place-items-center bg-secondary text-foreground shadow-lg shadow-black/50 transition-colors hover:bg-foreground hover:text-background"
                >
                  <Square className="size-3.5 fill-current" />
                </button>
              ) : (
                <button
                  type="button"
                  aria-label={`Play ${g.title}`}
                  onClick={() => onPlay(g)}
                  className="kryo-pill absolute bottom-3 right-3 grid size-11 min-h-[44px] min-w-[44px] translate-y-2 place-items-center bg-primary text-primary-foreground opacity-0 shadow-lg shadow-black/50 transition-all duration-200 group-hover/lift:translate-y-0 group-hover/lift:opacity-100 group-focus-within:translate-y-0 group-focus-within:opacity-100 focus-visible:translate-y-0 focus-visible:opacity-100 [@media(pointer:coarse)]:translate-y-0 [@media(pointer:coarse)]:opacity-100"
                >
                  <Play className="size-4 fill-current" />
                </button>
              )}
            </div>
            </div>
          ))}
        </div>
        {shown.length === 0 ? <p className="text-xs text-muted-foreground">No {filter === 'all' ? '' : `${STATUS_LABEL[filter]} `}games on this PC.</p> : null}
      </section>

      {elsewhere ? (
        <section className="grid min-w-0 gap-3">
          <div className="flex items-center gap-3">
            <Label>Not on this PC · {awayShown.length}</Label>
            <span className="h-px grow bg-border" />
          </div>
          <div className="grid grid-cols-[repeat(auto-fill,minmax(150px,1fr))] gap-4">
            {awayShown.map((e) => {
              const download = downloads.find((d) => d.slug === e.slug && (isActive(d) || d.status === 'paused'))
              return (
                <div key={e.slug} className="group/lift">
                <div
                  {...gameContext(onSavedContext ? (x, y) => onSavedContext(e, x, y) : undefined)}
                  className="kryo-radius group relative aspect-[2/3] overflow-hidden border border-dashed border-border bg-card transition-[transform,border-color] duration-200 group-hover/lift:-translate-y-1 group-hover/lift:border-foreground/60"
                >
                  <button type="button" onClick={() => onCatalogGame?.(e.slug)} className="kryo-square absolute inset-0" aria-label={`Open ${e.title}`}>
                    {/* The tall cover, like the cards of games on this PC; the wide one only when kryo.to has no tall one. */}
                    <Art adult={isAdult(e.slug)} src={e.coverVertical || e.cover || null} fallback={[e.cover]} title={e.title} className="size-full object-cover opacity-70 transition-opacity group-hover:opacity-100" />
                  </button>
                  {filter === 'all' ? (
                    <span className="kryo-pill pointer-events-none absolute left-2 top-2 bg-background/80 px-2 py-0.5 text-[9px] font-bold uppercase tracking-wider text-foreground backdrop-blur">
                      {STATUS_LABEL[e.status]}
                    </span>
                  ) : null}
                  {download ? (
                    <span className="kryo-pill pointer-events-none absolute bottom-3 left-2 bg-background/80 px-2 py-0.5 text-[10px] font-bold tabular-nums text-foreground backdrop-blur">
                      {download.status === 'paused' ? 'Paused' : `Downloading ${Math.round(progressOf(download) * 100)}%`}
                    </span>
                  ) : onInstall ? (
                    <button
                      type="button"
                      aria-label={`Install ${e.title}`}
                      title="Install"
                      onClick={() => onInstall(e.slug)}
                      className="kryo-pill absolute bottom-3 right-3 grid size-11 min-h-[44px] min-w-[44px] place-items-center bg-primary text-primary-foreground shadow-lg shadow-black/50 transition-colors hover:bg-foreground hover:text-background"
                    >
                      <DownloadIcon className="size-4" />
                    </button>
                  ) : null}
                </div>
                </div>
              )
            })}
          </div>
          {awayShown.length === 0 ? (
            <p className="text-xs text-muted-foreground">
              Every {filter === 'all' ? '' : `${STATUS_LABEL[filter]} `}game on your kryo.to list is on this PC.
            </p>
          ) : null}
        </section>
      ) : null}
    </div>
  )
}

