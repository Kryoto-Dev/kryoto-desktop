import { Fragment, useEffect, useMemo, useState, type ReactNode } from 'react'
import { artSrc, reportArt } from '@/lib/art'
import { coverFor } from '@/lib/game-artwork'
import { usePersisted } from '@/hooks/usePersisted'
import { ChevronDown, ChevronRight, CircleCheck, Check, Download as DownloadIcon, ExternalLink, EyeOff, Home, ListChecks, Plus, Search, Square, Trash2, X } from '@/ui/pixel-icons'
import { ContextMenu, asciiTrack, Dropdown, type MenuEntry } from '@/ui'
import { isActive, progressOf, type Download } from '@/lib/downloads'
import type { LibraryGame } from '@/lib/library'
import { launcherName } from '@/lib/launchers'
import { cn } from '@/lib/utils'
import { adultBlur, useHideAdult, useShowAdult } from '@/lib/adult'
import { STATUSES, STATUS_LABEL, isPlayStatus, useAdultFlags, type SavedEntry, type SavedStatus } from '@/hooks/useSaved'
import { gameContext } from '@/library/context'
import { BranchTag, copyLabel } from '@/library/BranchTag'
import { askInstall, collections, useCollections, type Collection, type CollectionGame } from '@/lib/collections'
import { NewDot } from '@/ui/FeatureNotice'

type Shelf = 'installed' | 'saved' | SavedStatus

const isShelf = (v: string): v is Shelf => v === 'installed' || v === 'saved' || (STATUSES as string[]).includes(v)

/** The search, kept while the client is open (the Library remounts on every visit). */
let lastQuery = ''
/** Letters and digits only, lower case: "S.T.A.L.K.E.R." reads as "stalker". */
const bare = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, '')

/**
 * The Library's left rail: Home, search, every game - and under them the ones
 * still downloading, with their ASCII progress, the way Steam lists a game
 * you own but have not finished installing.
 *
 * "Show" switches the list to one of the account's kryo.to shelves (Playing,
 * Plan to Play, Favorite...), Steam's collections: games on this PC open as
 * usual, the rest are dimmed and open a page of their own in the Library.
 * The shelf picked is remembered, and the search while the client is open.
 * On "On this PC", Kryoto's games are grouped under their status there.
 *
 * Games another launcher keeps (Steam, through Bridge) have a group of their
 * own under Kryoto's, with a way to take them all off the Library.
 *
 * Under them, the account's kryo.to collections and the ones it follows, each
 * showing how much of it is on this PC, with a way to install the rest. Every
 * group folds, and what is folded is remembered.
 *
 * Last of all, folded, the games the player hid: out of every list above, found
 * again by opening the group or searching. They run and update as usual.
 */
export function Sidebar({
  games,
  downloads,
  running,
  selectedId,
  homeActive,
  onHome,
  onSelect,
  onPlay,
  onStop,
  onContext,
  onSavedContext,
  onDownloadContext,
  onDownloads,
  saved,
  hiddenSlugs,
  selectedSlug,
  onCatalogGame,
  onWeb,
  signedIn = false,
  onRemoveLauncher,
  onBulk,
}: {
  games: LibraryGame[]
  downloads: Download[]
  running: Set<string>
  selectedId: string | null
  homeActive: boolean
  onHome: () => void
  onSelect: (id: string) => void
  onPlay: (game: LibraryGame) => void
  onStop: (game: LibraryGame) => void
  onContext: (game: LibraryGame, x: number, y: number) => void
  onSavedContext?: (game: SavedEntry, x: number, y: number) => void
  onDownloadContext?: (download: Download, x: number, y: number) => void
  onDownloads: () => void
  saved: SavedEntry[]
  /** The games the account hid, on this PC or not. */
  hiddenSlugs: string[]
  /** The kryo.to game (not on this PC) whose page is open. */
  selectedSlug: string | null
  onCatalogGame: (slug: string) => void
  /** A kryo.to page, in the Store. */
  onWeb: (path: string) => void
  /** Signed in: My collections shows even when empty, with its New collection button. */
  signedIn?: boolean
  /** Take every game this launcher keeps off the Library; it asks first. */
  onRemoveLauncher: (launcher: string) => void
  /** Move many games on a kryo.to shelf at once, or take them out with `null` (suggestion #70). */
  onBulk?: (slugs: string[], status: SavedStatus | null) => Promise<void>
}) {
  const [query, setQueryState] = useState(lastQuery)
  const setQuery = (q: string) => {
    lastQuery = q
    setQueryState(q)
  }
  const [picked, setShelf] = usePersisted<Shelf>('kryoto.library.shelf', 'installed', isShelf)
  const showAdult = useShowAdult()
  const q = query.trim().toLowerCase()
  // Punctuation does not count, so "stalker" finds S.T.A.L.K.E.R. (suggestion #69).
  const bareQ = bare(q)
  const matches = (text: string) => text.toLowerCase().includes(q) || (bareQ.length >= 2 && bare(text).includes(bareQ))
  // The groups that differ from how they start, by id: what the player folded.
  const [flipped, setFlipped] = usePersisted<string>('kryoto.sidebar.folds', '', (v): v is string => typeof v === 'string')
  const folds = useMemo(() => new Set(flipped.split(',').filter(Boolean)), [flipped])
  // A search looks inside everything.
  const isOpen = (id: string, start: boolean) => !!q || folds.has(id) !== start
  const flip = (id: string) => {
    const next = new Set(folds)
    if (!next.delete(id)) next.add(id)
    setFlipped([...next].join(','))
  }
  const lists = useCollections()
  const [menu, setMenu] = useState<{ collection: Collection; missing: number; x: number; y: number } | null>(null)
  const list = useMemo(
    () =>
      games
        .filter((g) => matches(g.title))
        .sort((a, b) => a.title.localeCompare(b.title, undefined, { sensitivity: 'base' })),
    [games, q],
  )
  const secretGames = useMemo(() => list.filter((g) => g.hidden), [list])
  const shownGames = useMemo(() => list.filter((g) => !g.hidden), [list])
  const own = useMemo(() => shownGames.filter((g) => !g.launcher), [shownGames])
  // This PC's games under the status they have on kryo.to (Playing, On Hold...),
  // in the order the site lists them; the ones with none come last.
  const ownGroups = useMemo(() => {
    const statusOf = new Map(saved.map((e) => [e.slug, e.status]))
    const by = new Map<SavedStatus | 'none', LibraryGame[]>()
    for (const g of own) {
      const st = (g.slug && statusOf.get(g.slug)) || 'none'
      by.set(st, [...(by.get(st) ?? []), g])
    }
    const rest = by.get('none') ?? []
    return [
      ...STATUSES.filter((st) => by.has(st)).map((st) => ({ id: `status:${st}`, label: STATUS_LABEL[st], games: by.get(st)! })),
      ...(rest.length ? [{ id: 'installed', label: by.size > 1 ? 'No status' : 'Installed', games: rest }] : []),
    ]
  }, [own, saved])
  const launchers = useMemo(() => [...new Set(shownGames.map((g) => g.launcher).filter((l): l is string => !!l))].sort(), [shownGames])
  const bySlug = useMemo(() => new Map(games.filter((g) => g.slug).map((g) => [g.slug!, g])), [games])
  // The saved games the player hid are in the Hidden group too, not on the shelves.
  const secret = useMemo(() => new Set([...hiddenSlugs, ...games.filter((g) => g.hidden && g.slug).map((g) => g.slug!)]), [hiddenSlugs, games])
  const secretSaved = useMemo(
    () => saved.filter((e) => secret.has(e.slug) && !bySlug.has(e.slug) && matches(e.title)),
    [saved, secret, bySlug, q],
  )
  const shelved = useMemo(() => saved.filter((e) => !secret.has(e.slug)), [saved, secret])
  // A remembered shelf that is empty now (or not reported yet) shows this PC's games.
  const shelf: Shelf =
    picked === 'installed' || (picked === 'saved' ? shelved.some((e) => isPlayStatus(e.status)) : shelved.some((e) => e.status === picked)) ? picked : 'installed'
  const shelfEntries = useMemo(
    () =>
      shelf === 'installed'
        ? []
        : shelved
            .filter((e) => (shelf === 'saved' ? isPlayStatus(e.status) : e.status === shelf) && matches(e.title))
            .sort((a, b) => a.title.localeCompare(b.title, undefined, { sensitivity: 'base' })),
    [shelved, shelf, q],
  )
  const missingSlugs = useMemo(() => [...shelfEntries, ...secretSaved].filter((e) => !bySlug.has(e.slug)).map((e) => e.slug), [shelfEntries, secretSaved, bySlug])
  const isAdult = useAdultFlags(missingSlugs)
  // Hiding adult games on kryo.to leaves the ones saved there off this list
  // too; one installed here stays, blurred, under Installed.
  const hideAdult = useHideAdult()
  const listed = useMemo(
    () => (hideAdult ? shelfEntries.filter((e) => bySlug.has(e.slug) || !isAdult(e.slug)) : shelfEntries),
    [hideAdult, shelfEntries, bySlug, isAdult],
  )
  // Selecting games on a kryo.to shelf to move them all at once (suggestion #70).
  const [picks, setPicks] = useState<Set<string> | null>(null)
  const [bulkMenu, setBulkMenu] = useState<{ x: number; y: number } | null>(null)
  const [bulkBusy, setBulkBusy] = useState(false)
  const selecting = picks !== null && shelf !== 'installed'
  useEffect(() => {
    setPicks(null)
  }, [shelf])
  const togglePick = (slug: string) =>
    setPicks((p) => {
      if (!p) return p
      const next = new Set(p)
      if (!next.delete(slug)) next.add(slug)
      return next
    })
  const allPicked = selecting && listed.length > 0 && listed.every((e) => picks!.has(e.slug))
  const runBulk = async (status: SavedStatus | null) => {
    if (!onBulk || !picks?.size || bulkBusy) return
    setBulkBusy(true)
    try {
      await onBulk([...picks], status)
      setPicks(new Set())
    } finally {
      setBulkBusy(false)
    }
  }
  const bulkItems: MenuEntry[] = [
    { heading: `Move ${picks?.size ?? 0} to` },
    ...STATUSES.filter((st) => st !== shelf).map((st) => ({ label: STATUS_LABEL[st], onSelect: () => void runBulk(st) })),
    { separator: true },
    { label: 'Remove from list', icon: <Trash2 />, onSelect: () => void runBulk(null) },
  ]
  const count = (st: SavedStatus) => shelved.filter((e) => e.status === st).length
  // "Everything" is what they play; "Not interested" keeps its own shelf.
  const played = shelved.filter((e) => isPlayStatus(e.status))
  const shelfOptions = [
    { value: 'installed' as Shelf, label: 'On this PC', hint: String(games.filter((g) => !g.hidden).length) },
    ...(played.length ? [{ value: 'saved' as Shelf, label: 'Everything on kryo.to', hint: String(played.length) }] : []),
    ...STATUSES.filter((st) => count(st) > 0).map((st) => ({ value: st as Shelf, label: STATUS_LABEL[st], hint: String(count(st)) })),
  ]
  const pending = downloads.filter((d) => (isActive(d) || d.status === 'paused') && matches(d.meta.title))
  const shownLists = useMemo(
    () =>
      lists.collections
        .filter((c) => c.role !== 'visitor' || lists.installed.includes(c.id))
        .map((c) => {
          // Hiding adult games on kryo.to hides the ones not on this PC here too.
          const games = hideAdult ? c.games.filter((g) => !g.nsfw || bySlug.has(g.slug)) : c.games
          const missing = games.filter((g) => !bySlug.has(g.slug)).length
          const named = matches(c.name)
          return { collection: c, games: named ? games : games.filter((g) => matches(g.title)), total: games.length, missing, named }
        })
        .filter((l) => !q || l.named || l.games.length > 0),
    [lists, hideAdult, bySlug, q],
  )
  const collectionMenu = (c: Collection, missing: number): MenuEntry[] => [
    { label: 'Open on kryo.to', icon: <ExternalLink />, onSelect: () => onWeb(c.href) },
    ...(missing ? [{ label: `Install ${missing} missing game${missing === 1 ? '' : 's'}`, icon: <DownloadIcon />, onSelect: () => askInstall(c.id) }] : []),
    ...(lists.installed.includes(c.id) ? [{ label: 'Stop watching for new games', onSelect: () => void collections.forget(c.id).catch(() => {}) }] : []),
  ]
  const catalogRow = (slug: string, title: string, cover: string | null, adult: boolean, indent = false, pickable = false) => {
    const game = bySlug.get(slug)
    const here = game ? game.id === selectedId : slug === selectedSlug
    if (pickable && selecting) {
      const on = picks!.has(slug)
      return (
        <button
          key={slug}
          type="button"
          aria-pressed={on}
          onClick={() => togglePick(slug)}
          {...gameContext((x, y) => {
            if (!on) togglePick(slug)
            setBulkMenu({ x, y })
          })}
          className={cn('kryo-pill flex h-9 w-full items-center gap-2.5 px-2 text-left text-xs transition-colors', on ? 'bg-secondary text-foreground' : 'text-muted-foreground hover:bg-secondary/60 hover:text-foreground')}
        >
          <span aria-hidden className={cn('kryo-radius grid size-4 shrink-0 place-items-center border', on ? 'border-primary bg-primary text-primary-foreground' : 'border-border text-transparent')}>
            <Check className="size-2.5" />
          </span>
          <Thumb cover={cover} blur={!!adultBlur(adult, showAdult)} />
          <span className="min-w-0 grow truncate">{game?.title ?? title}</span>
        </button>
      )
    }
    return (
      <button
        key={slug}
        type="button"
        aria-current={here ? 'page' : undefined}
        title={game ? undefined : 'Not on this PC'}
        onClick={() => (game ? onSelect(game.id) : onCatalogGame(slug))}
        onDoubleClick={() => game && onPlay(game)}
        {...gameContext((x, y) => (game ? onContext(game, x, y) : onSavedContext?.({ slug, title, cover: cover ?? '', status: 'plan', updatedAt: '' }, x, y)))}
        className={cn(
          'kryo-pill flex h-9 w-full items-center gap-2.5 px-2 text-left text-xs transition-colors',
          indent && 'pl-5',
          here ? 'bg-secondary text-foreground' : 'text-muted-foreground hover:bg-secondary/60 hover:text-foreground',
          !game && !here && 'opacity-55 hover:opacity-100',
        )}
      >
        <Thumb cover={cover} blur={!!adultBlur(adult, showAdult)} />
        <span className="min-w-0 grow truncate">{game?.title ?? title}</span>
        {!game ? <DownloadIcon className="size-3 shrink-0" aria-label="Not installed" /> : null}
      </button>
    )
  }
  const gameRow = (g: LibraryGame) => (
    <div key={g.id} className="relative">
      <button
        type="button"
        aria-current={g.id === selectedId ? 'page' : undefined}
        onClick={() => onSelect(g.id)}
        onDoubleClick={() => onPlay(g)}
        {...gameContext((x, y) => onContext(g, x, y))}
        className={cn(
          'kryo-pill flex h-9 w-full items-center gap-2.5 px-2 text-left text-xs transition-colors',
          running.has(g.id) && 'pr-9',
          g.id === selectedId ? 'bg-secondary text-foreground' : 'text-muted-foreground hover:bg-secondary/60 hover:text-foreground',
        )}
      >
        <Thumb cover={coverFor(g)} blur={!!adultBlur(g.nsfw, showAdult)} />
        <span className="min-w-0 grow truncate">{g.title}</span>
        <BranchTag branch={g.steamBranch} title={g.title} copy={copyLabel(g, games)} />
        {running.has(g.id) ? (
          <span className="flex items-center gap-1 text-[9px] font-bold uppercase tracking-wider text-success">
            <span className="kryo-blink size-1.5 rounded-full bg-success" />
            Running
          </span>
        ) : null}
      </button>
      {running.has(g.id) ? (
        <button
          type="button"
          aria-label={`Stop ${g.title}`}
          title="Stop"
          onClick={() => onStop(g)}
          className="kryo-pill absolute right-1 top-1/2 grid size-7 -translate-y-1/2 place-items-center text-foreground transition-colors hover:bg-foreground hover:text-background"
        >
          <Square className="size-3 fill-current" />
        </button>
      ) : null}
    </div>
  )
  const listGroup = (id: string, label: string, entries: typeof shownLists, action?: ReactNode) =>
    entries.length || action ? (
      <>
        <Group label={label} count={entries.length} open={isOpen(id, true)} onToggle={() => flip(id)} action={action} />
        {isOpen(id, true)
          ? entries.map(({ collection: c, games: shown, total, missing }) => {
              const open = isOpen(`c${c.id}`, false)
              return (
                <div key={c.id}>
                  <div className="relative">
                    <button
                      type="button"
                      aria-expanded={open}
                      onClick={() => flip(`c${c.id}`)}
                      onDoubleClick={() => onWeb(c.href)}
                      {...gameContext((x, y) => setMenu({ collection: c, missing, x, y }))}
                      className={cn('kryo-pill flex h-8 w-full items-center gap-1.5 px-2 text-left text-xs text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground', missing > 0 && 'pr-9')}
                    >
                      {open ? <ChevronDown className="size-3 shrink-0" aria-hidden /> : <ChevronRight className="size-3 shrink-0" aria-hidden />}
                      <span className="min-w-0 grow truncate">{c.name}</span>
                      {lists.updates.some((u) => u.id === c.id) ? <NewDot /> : null}
                      <span className="shrink-0 text-[10px] tabular-nums" title={`${total - missing} of ${total} on this PC`}>
                        {total - missing}/{total}
                      </span>
                      {missing === 0 && lists.installed.includes(c.id) ? <CircleCheck className="size-3 shrink-0" aria-label="Installed" /> : null}
                    </button>
                    {missing > 0 ? (
                      <button
                        type="button"
                        aria-label={`Install ${c.name}`}
                        title={`Install ${missing} missing game${missing === 1 ? '' : 's'}`}
                        onClick={() => askInstall(c.id)}
                        className="kryo-pill absolute right-1 top-1/2 grid size-6 -translate-y-1/2 place-items-center text-muted-foreground transition-colors hover:bg-foreground hover:text-background"
                      >
                        <DownloadIcon className="size-3" />
                      </button>
                    ) : null}
                  </div>
                  {open ? shown.map((g: CollectionGame) => catalogRow(g.slug, g.title, bySlug.get(g.slug) ? coverFor(bySlug.get(g.slug)!) : g.cover, bySlug.get(g.slug)?.nsfw ?? g.nsfw, true)) : null}
                  {open && total === 0 ? <p className="py-1 pl-5 text-[11px] text-muted-foreground">No games yet.</p> : null}
                </div>
              )
            })
          : null}
      </>
    ) : null

  return (
    <aside aria-label="Your games" className="flex min-h-0 w-72 shrink-0 flex-col border-r border-border bg-card/40">
      <div className="grid gap-2 p-3">
        <button
          type="button"
          aria-current={homeActive ? 'page' : undefined}
          onClick={onHome}
          className={cn(
            'kryo-pill flex h-9 items-center gap-2 px-4 text-[11px] font-bold uppercase tracking-[0.2em] transition-colors',
            homeActive ? 'bg-primary text-primary-foreground' : 'border border-border text-muted-foreground hover:text-foreground',
          )}
        >
          <Home className="size-3.5" />
          Home
        </button>
        <label className="kryo-pill flex h-9 items-center gap-2 border border-border bg-background px-3 text-muted-foreground focus-within:border-foreground">
          <Search className="size-3.5 shrink-0" />
          <input
            id="library-search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            // Escape clears the search first, then leaves the field.
            onKeyDown={(e) => {
              if (e.key !== 'Escape') return
              if (query) setQuery('')
              else e.currentTarget.blur()
            }}
            placeholder="Search your games (Ctrl+F)"
            aria-label="Search your games"
            className="kryo-square min-w-0 grow bg-transparent text-xs text-foreground outline-none placeholder:text-muted-foreground"
          />
        </label>
      </div>

      {shelfOptions.length > 1 ? (
        <div className="px-3 pb-1">
          <Dropdown label="Show" value={shelf} options={shelfOptions} onChange={setShelf} />
        </div>
      ) : null}

      <div className="min-h-0 grow overflow-auto px-2 pb-3">
        {shelf !== 'installed' ? (
          <>
            <Group
              label={shelf === 'saved' ? 'On kryo.to' : STATUS_LABEL[shelf]}
              count={listed.length}
              open={isOpen('shelf', true)}
              onToggle={() => flip('shelf')}
              action={
                onBulk && listed.length > 1 ? (
                  <button
                    type="button"
                    aria-pressed={selecting}
                    aria-label={selecting ? 'Stop selecting' : 'Select games'}
                    title={selecting ? 'Stop selecting' : 'Select games to move them all at once'}
                    onClick={() => setPicks((p) => (p ? null : new Set()))}
                    className={cn('kryo-pill grid size-5 place-items-center transition-colors hover:bg-secondary hover:text-foreground', selecting ? 'text-foreground' : 'text-muted-foreground')}
                  >
                    {selecting ? <X className="size-3" aria-hidden /> : <ListChecks className="size-3" aria-hidden />}
                  </button>
                ) : undefined
              }
            />
            {selecting && isOpen('shelf', true) ? (
              <div className="mb-1 flex flex-wrap items-center gap-1 px-2 py-1 text-[11px] text-muted-foreground">
                <span className="mr-auto tabular-nums text-foreground" aria-live="polite">{picks!.size} selected</span>
                <button type="button" onClick={() => setPicks(allPicked ? new Set() : new Set(listed.map((e) => e.slug)))} className="kryo-pill px-1.5 py-0.5 hover:bg-secondary hover:text-foreground">
                  {allPicked ? 'Clear' : 'All'}
                </button>
                <button
                  type="button"
                  disabled={!picks!.size || bulkBusy}
                  onClick={(ev) => {
                    const r = ev.currentTarget.getBoundingClientRect()
                    setBulkMenu({ x: r.left, y: r.bottom + 4 })
                  }}
                  className="kryo-pill border border-border px-1.5 py-0.5 text-foreground hover:bg-secondary disabled:opacity-50"
                >
                  Move to…
                </button>
              </div>
            ) : null}
            {isOpen('shelf', true)
              ? listed.map((e) => {
                  const game = bySlug.get(e.slug)
                  return catalogRow(e.slug, e.title, game ? coverFor(game) : e.cover || null, game ? game.nsfw : isAdult(e.slug), false, true)
                })
              : null}
            {listed.length === 0 ? <p className="px-2 py-3 text-xs text-muted-foreground">Nothing here.</p> : null}
          </>
        ) : null}
        {shelf === 'installed'
          ? ownGroups.map((group) => (
              <Fragment key={group.id}>
                <Group label={group.label} count={group.games.length} open={isOpen(group.id, true)} onToggle={() => flip(group.id)} />
                {isOpen(group.id, true) ? group.games.map(gameRow) : null}
              </Fragment>
            ))
          : null}
        {shelf === 'installed'
          ? launchers.map((l) => {
              const kept = shownGames.filter((g) => g.launcher === l)
              const name = launcherName(l) ?? l
              return (
                <Fragment key={l}>
                  <Group
                    label={name}
                    count={kept.length}
                    open={isOpen(`launcher:${l}`, true)}
                    onToggle={() => flip(`launcher:${l}`)}
                    action={
                      <button
                        type="button"
                        aria-label={`Remove all ${name} games from Kryoto`}
                        title={`Remove all ${name} games from Kryoto`}
                        onClick={() => onRemoveLauncher(l)}
                        className="kryo-pill grid size-5 place-items-center text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
                      >
                        <Trash2 className="size-3" />
                      </button>
                    }
                  />
                  {isOpen(`launcher:${l}`, true) ? kept.map(gameRow) : null}
                </Fragment>
              )
            })
          : null}

        {pending.length ? (
          <>
            <Group label="Downloading" count={pending.length} />
            {pending.map((d) => (
              <button
                key={d.id}
                type="button"
                onClick={onDownloads}
                {...gameContext(onDownloadContext ? (x, y) => onDownloadContext(d, x, y) : undefined)}
                className="kryo-pill grid w-full gap-1 px-2 py-2 text-left text-xs text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
              >
                <span className="truncate">{d.meta.title}</span>
                <span className="kryo-ascii-art flex justify-between text-[10px] tracking-normal">
                  {/* Drawn for the eye; a screen reader hears the figure beside it. */}
                  <span aria-hidden>{asciiTrack(progressOf(d), 18)}</span>
                  <span>{d.status === 'paused' ? 'paused' : `${Math.round(progressOf(d) * 100)}%`}</span>
                </span>
              </button>
            ))}
          </>
        ) : null}

        {listGroup(
          'mine',
          'My collections',
          shownLists.filter((l) => l.collection.role === 'owner' || l.collection.role === 'collaborator'),
          // kryo.to's own New collection dialog, opened in the Store; it lands on
          // the new collection with Add games open.
          signedIn && !q ? (
            <button
              type="button"
              aria-label="New collection"
              title="New collection"
              onClick={() => onWeb('/library?tab=collections&new=1')}
              className="kryo-pill grid size-5 place-items-center text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
            >
              <Plus className="size-3" aria-hidden />
            </button>
          ) : undefined,
        )}
        {listGroup('following', 'Following', shownLists.filter((l) => l.collection.role === 'following' || l.collection.role === 'visitor'))}

        {secretGames.length + secretSaved.length > 0 ? (
          <>
            <Group label="Hidden" icon={<EyeOff className="size-3" aria-hidden />} count={secretGames.length + secretSaved.length} open={isOpen('hidden', false)} onToggle={() => flip('hidden')} />
            {/* A hidden game that is running, or open, stays where it can be seen. */}
            {(isOpen('hidden', false) ? secretGames : secretGames.filter((g) => running.has(g.id) || g.id === selectedId)).map(gameRow)}
            {isOpen('hidden', false) ? secretSaved.map((e) => catalogRow(e.slug, e.title, e.cover || null, isAdult(e.slug))) : null}
          </>
        ) : null}

        {shelf === 'installed' && games.length > 0 && list.length === 0 && pending.length === 0 && shownLists.length === 0 ? (
          <p className="px-2 py-3 text-xs text-muted-foreground">Nothing matches &quot;{query}&quot;.</p>
        ) : null}
      </div>
      {menu ? <ContextMenu x={menu.x} y={menu.y} items={collectionMenu(menu.collection, menu.missing)} onClose={() => setMenu(null)} /> : null}
      {bulkMenu && selecting && picks!.size ? <ContextMenu x={bulkMenu.x} y={bulkMenu.y} items={bulkItems} onClose={() => setBulkMenu(null)} /> : null}
    </aside>
  )
}

/** A row's small cover. */
function Thumb({ cover, blur }: { cover: string | null | undefined; blur: boolean }) {
  const corners = { borderRadius: 'min(var(--kryo-radius), 6px)' }
  if (!cover) return <span className="size-6 shrink-0 bg-secondary" style={corners} />
  return <img decoding="async" src={artSrc(cover) ?? undefined} onError={() => reportArt(cover, 'sidebar')} alt="" className={cn('size-6 shrink-0 object-cover', blur && 'blur-[3px]')} style={corners} />
}

/** A heading in the rail. With `onToggle`, what is under it folds away. */
function Group({ label, icon, count, open, onToggle, action }: { label: string; icon?: ReactNode; count: number; open?: boolean; onToggle?: () => void; action?: ReactNode }) {
  const body = (
    <>
      <span className="flex items-center gap-1 text-[10px] uppercase tracking-[0.25em] text-primary">
        {onToggle ? open ? <ChevronDown className="size-3" aria-hidden /> : <ChevronRight className="size-3" aria-hidden /> : null}
        {icon}
        {label}
      </span>
      <span className="text-[10px] tabular-nums text-muted-foreground">{count}</span>
    </>
  )
  const shape = 'flex w-full items-baseline justify-between px-2 pb-1.5 pt-3'
  if (!onToggle) return <div className={shape}>{body}</div>
  return (
    <div className="relative">
      <button type="button" aria-expanded={open} onClick={onToggle} className={cn(shape, 'kryo-square text-left', action && 'pr-8')}>
        {body}
      </button>
      {action ? <span className="absolute bottom-1 right-1.5">{action}</span> : null}
    </div>
  )
}
