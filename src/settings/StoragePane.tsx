import { useCallback, useEffect, useMemo, useState } from 'react'
import { pickPath } from '@/lib/pick'
import { FolderInput, FolderOpen, HardDrive, Plus, Star, Trash2 } from 'lucide-react'
import { AsciiBar, Button, Caption, Check, Dropdown, IconButton } from '@/ui'
import { call, errorText, isTauri, on } from '@/lib/bridge'
import { formatBytes } from '@/lib/downloads'
import { library } from '@/lib/library'
import { formatLastPlayed } from '@/lib/format'
import { settingsApi } from '@/lib/settings'
import { cn } from '@/lib/utils'

type StoredGame = { id: string; title: string; folder: string; bytes: number; lastPlayed: number | null; cover: string | null; nsfw: boolean }
type Folder = {
  path: string
  drive: string
  isDefault: boolean
  exists: boolean
  total: number
  free: number
  gamesBytes: number
  games: StoredGame[]
}
type Overview = { folders: Folder[]; elsewhere: StoredGame[] }
type MoveProgress = { id: string; copied: number; total: number; done: boolean; error: string | null }

const SEG = 48

/** The drive in whole blocks: this client's games, everything else, free. */
function DriveBar({ f }: { f: Folder }) {
  const games = f.total ? Math.max(f.gamesBytes > 0 ? 1 : 0, Math.round((f.gamesBytes / f.total) * SEG)) : 0
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
  useEffect(() => {
    void load()
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
              {f.drive}
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
                  <Button size="sm" disabled={!!busy} onClick={() => void run('default', () => call('storage_set_default', { path: folder.path }))}>
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
            <DriveBar f={folder} />
            <dl className="flex flex-wrap gap-x-6 gap-y-1 text-[10px] uppercase tracking-wider text-muted-foreground">
              <Legend tone="games" label="Kryoto games" value={formatBytes(folder.gamesBytes)} />
              <Legend tone="other" label="Other" value={formatBytes(Math.max(0, folder.total - folder.free - folder.gamesBytes))} />
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
                      options={others.map((o) => ({ value: o.path, label: `${o.drive} ${o.path.split(/[\\/]/).pop()}`, hint: formatBytes(o.free) }))}
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
            <GameTable games={folder.games} picked={picked} onToggle={toggle} />
          </section>
        </>
      ) : null}

      {data.elsewhere.length ? (
        <section className="grid gap-2">
          <Caption>Added from elsewhere · not moved or deleted by Kryoto</Caption>
          <GameTable games={data.elsewhere} />
        </section>
      ) : null}
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
    </div>
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

function GameTable({ games, picked, onToggle }: { games: StoredGame[]; picked?: Set<string>; onToggle?: (id: string) => void }) {
  if (!games.length) return <p className="kryo-radius border border-dashed border-border p-4 text-center text-xs text-muted-foreground">No games in this folder.</p>
  return (
    <div className="kryo-radius overflow-hidden border border-border">
      <div className="grid grid-cols-[28px_1fr_110px_96px] gap-3 border-b border-border bg-card px-3 py-2 text-[10px] uppercase tracking-wider text-muted-foreground">
        <span />
        <span>Name</span>
        <span>Last played</span>
        <span className="text-right">Size</span>
      </div>
      {games.map((g) => (
        <div key={g.id} className="grid grid-cols-[28px_1fr_110px_96px] items-center gap-3 border-b border-border px-3 py-2 last:border-b-0">
          {onToggle ? <Check checked={!!picked?.has(g.id)} onChange={() => onToggle(g.id)} label={<span className="sr-only">Select {g.title}</span>} /> : <span />}
          <span className="truncate text-xs text-foreground" title={g.folder}>
            {g.title}
          </span>
          <span className="text-[11px] text-muted-foreground">{formatLastPlayed(g.lastPlayed)}</span>
          <span className="text-right text-[11px] tabular-nums text-foreground">{formatBytes(g.bytes)}</span>
        </div>
      ))}
    </div>
  )
}
