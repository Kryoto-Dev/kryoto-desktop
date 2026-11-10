import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { pickPath } from '@/lib/pick'
import { FolderInput, FolderOpen, HardDrive, Plus, Star, Trash2 } from '@/ui/pixel-icons'
import { AsciiBar, Button, Caption, Check, Dropdown, IconButton, inputCls, Section } from '@/ui'
import { call, errorText, isTauri, on } from '@/lib/bridge'
import { formatBytes } from '@/lib/downloads'
import { library } from '@/lib/library'
import { formatLastPlayed } from '@/lib/format'
import { launcherName } from '@/lib/launchers'
import { settingsApi } from '@/lib/settings'
import { openLibrary } from '@/lib/window'
import { useDiskSizes } from '@/hooks/useDiskSizes'
import { cn } from '@/lib/utils'

type StoredGame = {
  id: string
  title: string
  folder: string
  lastPlayed: number | null
  cover: string | null
  nsfw: boolean
  /** Brought over from Hydra. */
  fromHydra?: boolean
  /** Outside the library folders: why it cannot be moved in (null when it can). */
  blocked?: string | null
  /** The launcher that keeps it. */
  launcher?: string | null
  version?: string | null
}
type Folder = {
  path: string
  drive: string
  isDefault: boolean
  exists: boolean
  total: number
  free: number
  games: StoredGame[]
}
type Overview = { folders: Folder[]; elsewhere: StoredGame[] }
type MoveProgress = { id: string; copied: number; total: number; done: boolean; error: string | null }

const SEG = 48

/** Sizes as they are measured, by game. */
type Sizes = Record<string, { bytes: number } | undefined>

const lastPart = (path: string) => path.split(/[\\/]/).filter(Boolean).at(-1) ?? path

/** A folder in a list: its drive and name, or its whole path where that alone would read the same as another's. */
function folderLabel(f: Folder, all: Folder[]): string {
  const twin = all.some((o) => o !== f && o.drive === f.drive && lastPart(o.path) === lastPart(f.path))
  return twin ? f.path : `${f.drive} ${lastPart(f.path)}`
}

/** The drive in whole blocks: this client's games, everything else, free. */
function DriveBar({ f, gamesBytes }: { f: Folder; gamesBytes: number }) {
  const games = f.total ? Math.max(gamesBytes > 0 ? 1 : 0, Math.round((gamesBytes / f.total) * SEG)) : 0
  const used = f.total ? Math.round(((f.total - f.free) / f.total) * SEG) : 0
  const other = Math.max(0, used - games)
  return (
    <div className="flex h-4 gap-[2px]" aria-hidden>
      {Array.from({ length: SEG }).map((_, i) => (
        <span
          key={i}
          className={cn('h-full flex-1', i < games ? 'bg-foreground' : i < games + other ? 'bg-foreground/35' : 'bg-secondary')}
        />
      ))}
    </div>
  )
}

function Swatch({ tone }: { tone: 'games' | 'other' | 'free' }) {
  return <span className={cn('inline-block size-2.5', tone === 'games' ? 'bg-foreground' : tone === 'other' ? 'bg-foreground/35' : 'bg-secondary')} />
}

/**
 * Settings > Storage - Steam's storage manager. Every library folder across
 * the top, the selected one's drive as a bar (this client's games, everything
 * else, free), and its games by size. Tick games to move them to another
 * folder or uninstall them. New downloads go to the default folder (the star).
 * The page is there at once: sizes fill in behind it, one game at a time, and
 * are remembered, so only a game that moved or updated is walked again.
 */
export function StoragePane({ onChanged }: { onChanged: () => void }) {
  const [data, setData] = useState<Overview | null>(null)
  const [at, setAt] = useState(0)
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const [target, setTarget] = useState<string>('')
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [moving, setMoving] = useState<MoveProgress | null>(null)

  const load = useCallback(async () => {
    try {
      const o = await call<Overview>('storage_overview')
      setData(o)
      setAt((i) => Math.min(i, Math.max(0, o.folders.length - 1)))
    } catch (e) {
      setError(errorText(e))
    }
  }, [])
  // A folder that has gone is forgotten as the page opens: the rest of the page follows.
  useEffect(() => {
    void load().then(onChanged)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [load])
  useEffect(() => {
    let stop: (() => void) | undefined
    void on<MoveProgress>('storage-move', (p) => {
      setMoving(p.done ? null : p)
      // A finished move can still carry a note: the old folder was left behind.
      if (p.done && p.error) setError(p.error)
    }).then((fn) => (stop = fn))
    return () => stop?.()
  }, [])

  const folder = data?.folders[at] ?? null
  const others = useMemo(() => (data?.folders ?? []).filter((_, i) => i !== at), [data, at])
  // Another launcher's games are not ours to measure, and it can take minutes.
  const measured = useMemo(
    () =>
      [...(folder?.games ?? []), ...(data?.elsewhere ?? []).filter((g) => !g.blocked)].map((g) => ({
        id: g.id,
        installDir: g.folder,
        version: g.version ?? null,
      })),
    [folder, data],
  )
  const { sizes, measuring } = useDiskSizes(measured, true, 'kryoto.storage.sizes')
  useEffect(() => {
    setPicked(new Set())
    setTarget(others[0]?.path ?? '')
  }, [at, others])

  const run = async (label: string, job: () => Promise<unknown>) => {
    setBusy(label)
    setError(null)
    try {
      await job()
      await settingsApi.reload().catch(() => {})
      onChanged()
      await load()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(null)
    }
  }

  const addFolder = async () => {
    if (!isTauri()) return run('add', () => call('storage_add_folder', { path: 'E:\\Kryoto Games' }))
    // The picker inside run() too: a picker that fails says so here instead
    // of the button doing nothing (kryoto-bugs KB-0042).
    await run('add', async () => {
      const dir = await pickPath({ directory: true, title: 'Add a library folder' })
      if (typeof dir === 'string') await call('storage_add_folder', { path: dir })
    })
  }

  if (!data) {
    return error ? <p className="text-xs text-destructive">{error}</p> : <AsciiBar fraction={null} cells={16} showPct={false} />
  }
  const gamesBytes = (folder?.games ?? []).reduce((sum, g) => sum + (sizes[g.id]?.bytes ?? 0), 0)

  const toggle = (id: string) =>
    setPicked((p) => {
      const next = new Set(p)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  const pickedGames = folder?.games.filter((g) => picked.has(g.id)) ?? []

  return (
    <div className="grid gap-5">
      <div className="flex flex-wrap gap-2">
        {data.folders.map((f, i) => (
          <button
            key={f.path}
            type="button"
            onClick={() => setAt(i)}
            aria-pressed={i === at}
            className={cn(
              'kryo-radius grid min-w-44 gap-1 border px-3 py-2.5 text-left transition-colors',
              i === at ? 'border-foreground bg-secondary' : 'border-border hover:border-foreground/50',
            )}
          >
            <span className="flex items-center gap-2 text-xs font-bold text-foreground">
              <HardDrive className="size-3.5" />
              <span className="max-w-52 truncate" title={f.path}>
                {folderLabel(f, data.folders)}
              </span>
              {f.isDefault ? <Star className="size-3 fill-current text-warning" aria-label="Default" /> : null}
            </span>
            <span className="text-[10px] tabular-nums text-muted-foreground">
              {f.total ? `${formatBytes(f.free)} free of ${formatBytes(f.total)}` : 'Not available'}
            </span>
          </button>
        ))}
        <button
          type="button"
          onClick={() => void addFolder()}
          disabled={!!busy}
          aria-label="Add a library folder"
          title="Add a library folder"
          className="kryo-radius grid min-w-16 place-items-center border border-dashed border-border text-muted-foreground transition-colors hover:border-foreground hover:text-foreground"
        >
          <Plus className="size-4" />
        </button>
      </div>

      {folder ? (
        <>
          <section className="grid gap-3">
            <div className="flex items-start justify-between gap-3">
              <div className="grid min-w-0 gap-1">
                <p className="kryo-ascii-art select-text truncate text-[11px] text-foreground">{folder.path}</p>
                <Caption>
                  {folder.isDefault ? 'New games install here' : 'Library folder'}
                  {folder.exists ? '' : ' · missing'}
                </Caption>
              </div>
              <div className="flex shrink-0 gap-1.5">
                {!folder.isDefault ? (
                  <Button size="sm" disabled={!!busy} onClick={() => run('default', () => call('storage_set_default', { path: folder.path }))}>
                    <Star className="size-3" />
                    Make default
                  </Button>
                ) : null}
                <IconButton label="Open folder" className="size-7" onClick={() => void library.openFolder(folder.path, true)}>
                  <FolderOpen className="size-3.5" />
                </IconButton>
                {!folder.isDefault ? (
                  <IconButton
                    label="Stop using this folder"
                    className="size-7 hover:border-destructive hover:text-destructive"
                    disabled={!!busy}
                    onClick={() => void run('remove', () => call('storage_remove_folder', { path: folder.path }))}
                  >
                    <Trash2 className="size-3.5" />
                  </IconButton>
                ) : null}
              </div>
            </div>
            <DriveBar f={folder} gamesBytes={gamesBytes} />
            <dl className="grid grid-cols-3 gap-x-6 gap-y-1 text-[10px] uppercase tracking-wider text-muted-foreground">
              <Legend tone="games" label="Kryoto games" value={measuring ? '...' : formatBytes(gamesBytes)} />
              <Legend tone="other" label="Other" value={measuring ? '...' : formatBytes(Math.max(0, folder.total - folder.free - gamesBytes))} />
              <Legend tone="free" label="Free" value={formatBytes(folder.free)} />
            </dl>
          </section>

          <section className="grid gap-2">
            <div className="flex items-center justify-between gap-3">
              <Caption>
                {folder.games.length} game{folder.games.length === 1 ? '' : 's'}
              </Caption>
              <div className="flex items-center gap-2">
                {others.length ? (
                  <>
                    <Dropdown
                      className="w-56"
                      label="Move to"
                      value={target}
                      options={others.map((o) => ({ value: o.path, label: folderLabel(o, data.folders), hint: formatBytes(o.free) }))}
                      onChange={setTarget}
                    />
                    <Button
                      size="sm"
                      disabled={!pickedGames.length || !target || !!busy}
                      onClick={() =>
                        void run('move', async () => {
                          for (const g of pickedGames) await call('storage_move', { id: g.id, to: target })
                          setPicked(new Set())
                        })
                      }
                    >
                      <FolderInput className="size-3" />
                      Move
                    </Button>
                  </>
                ) : (
                  <span className="text-[11px] text-muted-foreground">Add another folder to move games between drives.</span>
                )}
                <Button
                  size="sm"
                  variant="danger"
                  disabled={!pickedGames.length || !!busy}
                  onClick={() =>
                    void run('uninstall', async () => {
                      for (const g of pickedGames) await library.remove(g.id, true)
                      setPicked(new Set())
                    })
                  }
                >
                  <Trash2 className="size-3" />
                  Uninstall
                </Button>
              </div>
            </div>
            {moving ? (
              <div className="kryo-radius grid gap-1.5 border border-border p-3">
                <Caption>Moving {data.folders.flatMap((f) => f.games).find((g) => g.id === moving.id)?.title ?? 'game'}</Caption>
                <AsciiBar fraction={moving.total ? moving.copied / moving.total : null} cells={36} />
              </div>
            ) : null}
            {busy === 'move' && !moving ? <AsciiBar fraction={null} cells={24} showPct={false} className="text-muted-foreground" /> : null}
            <GameTable games={folder.games} sizes={sizes} settled={!measuring} picked={picked} onToggle={toggle} />
          </section>
        </>
      ) : null}

      {data.elsewhere.length ? <Elsewhere games={data.elsewhere} sizes={sizes} settled={!measuring} folders={data.folders} busy={!!busy} moving={moving} run={run} /> : null}
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
    </div>
  )
}

/**
 * Games added from elsewhere on the PC (brought over from Hydra, or picked in
 * Add a game): where each one is, and a way to bring them into a Kryoto
 * library folder, so every game sits in one place. Moving takes the game's
 * whole folder, keeps its play time and settings, and asks first: anything
 * else that points at the old place (Hydra, a shortcut) will not find it.
 *
 * Games another launcher keeps cannot move, so they are one line, not a row
 * each: there is nothing to do with them here, and a Steam library is long.
 */
function Elsewhere({
  games: all,
  sizes,
  settled,
  folders,
  busy,
  moving,
  run,
}: {
  games: StoredGame[]
  sizes: Sizes
  settled: boolean
  folders: Folder[]
  busy: boolean
  moving: MoveProgress | null
  run: (label: string, job: () => Promise<unknown>) => Promise<void>
}) {
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const [target, setTarget] = useState(() => folders.find((f) => f.isDefault)?.path ?? folders[0]?.path ?? '')
  const [asking, setAsking] = useState(false)
  const kept = all.filter((g) => g.launcher)
  const keepers = [...new Set(kept.map((g) => launcherName(g.launcher) ?? 'another launcher'))]
  const games = all.filter((g) => !g.launcher)
  const movable = games.filter((g) => !g.blocked)
  const chosen = movable.filter((g) => picked.has(g.id))
  const bytes = chosen.reduce((s, g) => s + (sizes[g.id]?.bytes ?? 0), 0)
  const dest = folders.find((f) => f.path === target)
  const hydra = chosen.some((g) => g.fromHydra)
  const toggle = (id: string) =>
    setPicked((p) => {
      const next = new Set(p)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  const move = () =>
    void run('move', async () => {
      setAsking(false)
      for (const g of chosen) await call('storage_move', { id: g.id, to: target })
      setPicked(new Set())
    })
  const keptLine = kept.length ? (
    <p className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-muted-foreground">
      {kept.length} game{kept.length === 1 ? '' : 's'} from {keepers.join(' and ')} stay{kept.length === 1 ? 's' : ''} where {kept.length === 1 ? 'it is' : 'they are'}, and {kept.length === 1 ? 'is' : 'are'} in your Library.
      <Button size="sm" variant="ghost" onClick={openLibrary}>
        Open Library
      </Button>
    </p>
  ) : null
  if (!games.length) return keptLine
  return (
    <section className="grid gap-2">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="grid gap-0.5">
          <Caption>Added from elsewhere · {games.length}</Caption>
          <span className="text-[11px] text-muted-foreground">Move them into a library folder to keep all your games together.</span>
        </div>
        {movable.length ? (
          <div className="flex items-center gap-2">
            <Dropdown
              className="w-56"
              label="Move into"
              value={target}
              options={folders.map((o) => ({ value: o.path, label: folderLabel(o, folders), hint: formatBytes(o.free) }))}
              onChange={setTarget}
            />
            <Button size="sm" disabled={!chosen.length || !target || busy} onClick={() => setAsking(true)}>
              <FolderInput className="size-3" />
              Move into Kryoto
            </Button>
          </div>
        ) : null}
      </div>
      {asking && chosen.length ? (
        <div role="alertdialog" aria-label="Move these games" className="kryo-radius grid gap-2 border border-warning/50 bg-warning/10 p-3 text-xs">
          <p className="text-foreground">
            Move {chosen.length === 1 ? chosen[0]!.title : `${chosen.length} games`} ({formatBytes(bytes)}) into {target}? Each game's whole folder moves; its play time and settings stay.
            {dest && dest.free < bytes ? ' That drive may not have room for all of them.' : ''}
          </p>
          <p className="text-muted-foreground">
            {hydra ? 'Hydra will not find the games it had there any more. ' : ''}Shortcuts or other apps that point at the old folder will not either.
          </p>
          <div className="flex gap-2">
            <Button size="sm" variant="primary" onClick={move}>
              Move
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setAsking(false)}>
              Cancel
            </Button>
          </div>
        </div>
      ) : null}
      {moving && games.some((g) => g.id === moving.id) ? (
        <div className="kryo-radius grid gap-1.5 border border-border p-3">
          <Caption>Moving {games.find((g) => g.id === moving.id)?.title ?? 'game'}</Caption>
          <AsciiBar fraction={moving.total ? moving.copied / moving.total : null} cells={36} />
        </div>
      ) : null}
      <GameTable
        games={games}
        sizes={sizes}
        settled={settled}
        picked={picked}
        onToggle={toggle}
        showFolder
        onRemove={(g) => void run('remove', () => library.remove(g.id, false))}
      />
      {keptLine}
    </section>
  )
}

/** Settings > Storage > Download folder: empty is inside the library folder. */
export function DownloadFolder({ value, onChange }: { value: string; onChange: (dir: string) => void }) {
  const [error, setError] = useState<string | null>(null)
  const choose = async () => {
    setError(null)
    try {
      const dir = await pickPath({ directory: true, title: 'Download games to', defaultPath: value || undefined })
      if (typeof dir === 'string' && dir) onChange(dir)
    } catch (e) {
      setError(`The folder picker did not open: ${errorText(e)}`)
    }
  }
  return (
    <div className="grid gap-1.5">
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" variant="outline" onClick={() => choose()}>
          <FolderOpen className="size-3" />
          {value ? 'Change folder' : 'Choose a folder'}
        </Button>
        {value ? (
          <>
            <span className="kryo-ascii-art min-w-0 select-text truncate text-[11px] text-foreground">{value}</span>
            <Button size="sm" variant="ghost" onClick={() => onChange('')}>
              Use the library folder
            </Button>
          </>
        ) : (
          <span className="text-[11px] text-muted-foreground">Downloads go inside the library folder each game installs to.</span>
        )}
      </div>
      {error ? <p className="text-[11px] text-destructive">{error}</p> : null}
    </div>
  )
}

type LeftoverPrefix = { id: string; path: string; bytes: number; modified: number | null }

/**
 * Settings > Storage on Linux: Wine prefixes left by games no longer in the
 * library. Only drawn when there are some. Deleting one asks first, because a
 * game's saves can live in its prefix.
 */
export function LeftoverPrefixes() {
  const [list, setList] = useState<LeftoverPrefix[] | null>(null)
  const [asking, setAsking] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const load = useCallback(() => {
    call<LeftoverPrefix[]>('storage_leftover_prefixes').then(setList).catch((e: unknown) => setError(errorText(e)))
  }, [])
  useEffect(load, [load])
  const remove = async (id: string) => {
    setBusy(true)
    setError(null)
    try {
      await call('storage_delete_prefix', { id })
      setAsking(null)
      load()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }
  if (!list?.length && !error) return null
  return (
    <Section
      title="Leftover Wine prefixes"
      anchor="leftover-prefixes"
      hint="Left behind by games you uninstalled. Each holds that game's settings, and maybe saves it kept there. Your installed games are not listed."
    >
      <ul className="kryo-radius grid gap-px overflow-hidden border border-border bg-border">
        {(list ?? []).map((p) => (
          <li key={p.id} className="grid gap-2 bg-card px-3 py-2.5">
            <div className="flex items-center gap-3">
              <span className="grid min-w-0 grow">
                <b className="truncate text-xs text-foreground">{p.id}</b>
                <span className="truncate text-[10px] text-muted-foreground" title={p.path}>
                  {formatBytes(p.bytes)}
                  {p.modified ? ` · last used ${formatLastPlayed(p.modified)}` : ''}
                </span>
              </span>
              <IconButton label="Open folder" className="size-7" onClick={() => void library.openFolder(p.path)}>
                <FolderOpen className="size-3.5" />
              </IconButton>
              <Button size="sm" variant="ghost" disabled={busy} onClick={() => setAsking(p.id)}>
                <Trash2 className="size-3" />
                Delete
              </Button>
            </div>
            {asking === p.id ? (
              <div role="alertdialog" aria-label={`Delete ${p.id}`} className="kryo-radius grid gap-2 border border-warning/50 bg-warning/10 p-3 text-xs">
                <p className="text-foreground">Delete this prefix for good? Any saves the game kept in it go too.</p>
                <div className="flex gap-2">
                  <Button size="sm" variant="danger" disabled={busy} onClick={() => remove(p.id)}>
                    Delete
                  </Button>
                  <Button size="sm" variant="ghost" disabled={busy} onClick={() => setAsking(null)}>
                    Cancel
                  </Button>
                </div>
              </div>
            ) : null}
          </li>
        ))}
      </ul>
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
    </Section>
  )
}

function Legend({ tone, label, value }: { tone: 'games' | 'other' | 'free'; label: string; value: string }) {
  return (
    <div className="flex items-center gap-1.5">
      <Swatch tone={tone} />
      <dt>{label}</dt>
      <dd className="m-0 tabular-nums text-foreground">{value}</dd>
    </div>
  )
}

/** How many rows a table shows before it asks. */
const ROWS = 6

function GameTable({
  games,
  sizes,
  settled,
  picked,
  onToggle,
  showFolder = false,
  onRemove,
}: {
  games: StoredGame[]
  sizes: Sizes
  /** Every size is in: the biggest go first then, in one move and not at each answer. */
  settled: boolean
  picked?: Set<string>
  onToggle?: (id: string) => void
  showFolder?: boolean
  /** Off the Library, for a row that cannot be moved. */
  onRemove?: (game: StoredGame) => void
}) {
  const [query, setQuery] = useState('')
  const [all, setAll] = useState(false)
  const order = useRef(sizes)
  if (settled) order.current = sizes
  const ranked = order.current
  const sorted = useMemo(
    () => games.slice().sort((a, b) => (ranked[b.id]?.bytes ?? 0) - (ranked[a.id]?.bytes ?? 0) || a.title.localeCompare(b.title)),
    [games, ranked],
  )
  if (!games.length) return <p className="kryo-radius border border-dashed border-border p-4 text-center text-xs text-muted-foreground">No games in this folder.</p>
  const words = query.trim().toLowerCase()
  const found = words ? sorted.filter((g) => g.title.toLowerCase().includes(words)) : sorted
  const shown = words || all ? found : found.slice(0, ROWS)
  return (
    <div className="grid gap-2">
      {games.length > ROWS ? <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder={`Search ${games.length} games`} aria-label="Search these games" className={inputCls} /> : null}
      <div className="kryo-radius overflow-hidden border border-border">
      <div className="grid grid-cols-[28px_1fr_110px_96px] gap-3 border-b border-border bg-card px-3 py-2 text-[10px] uppercase tracking-wider text-muted-foreground">
        <span />
        <span>Name</span>
        <span>Last played</span>
        <span className="text-right">Size</span>
      </div>
      {shown.map((g) => (
        <div key={g.id} className="grid grid-cols-[28px_1fr_110px_96px] items-center gap-3 border-b border-border px-3 py-2 last:border-b-0">
          {onToggle && !g.blocked ? (
            <Check checked={!!picked?.has(g.id)} onChange={() => onToggle(g.id)} label={<span className="sr-only">Select {g.title}</span>} />
          ) : onRemove ? (
            <IconButton label={`Remove ${g.title} from the Library`} className="size-6 border-0" onClick={() => onRemove(g)}>
              <Trash2 className="size-3" />
            </IconButton>
          ) : (
            <span />
          )}
          <span className="grid min-w-0">
            <span className="truncate text-xs text-foreground" title={g.folder}>
              {g.title}
              {g.fromHydra ? <span className="ml-2 text-[10px] uppercase tracking-wider text-muted-foreground">From Hydra</span> : null}
            </span>
            {showFolder ? (
              <span className="truncate text-[10px] text-muted-foreground" title={g.blocked ?? g.folder}>
                {g.blocked ?? g.folder}
              </span>
            ) : null}
          </span>
          <span className="text-[11px] text-muted-foreground">{formatLastPlayed(g.lastPlayed)}</span>
          <span className="text-right text-[11px] tabular-nums text-foreground">{g.blocked ? '' : sizes[g.id] ? formatBytes(sizes[g.id]!.bytes) : '...'}</span>
        </div>
      ))}
      {!shown.length ? <p className="p-4 text-center text-xs text-muted-foreground">No game called that.</p> : null}
      </div>
      {!words && found.length > ROWS ? (
        <Button size="sm" variant="ghost" className="w-fit" onClick={() => setAll(!all)} aria-expanded={all}>
          {all ? 'Show fewer' : `Show all ${found.length}`}
        </Button>
      ) : null}
    </div>
  )
}
