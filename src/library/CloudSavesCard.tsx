import { useEffect, useState } from 'react'
import { Check, Cloud, CloudUpload, FolderOpen, FolderPlus, History, Plus, X } from 'lucide-react'
import { Button, Caption, Card, IconButton, Label, Modal } from '@/ui'
import { errorText } from '@/lib/bridge'
import { cloudSaves, formatAgo, formatBytes, useGameSaves, type SaveLocation, type SaveVersion } from '@/lib/cloud-saves'
import { library, type LibraryGame } from '@/lib/library'
import { pickPath } from '@/lib/pick'
import { openSettingsSection, openStorePage } from '@/lib/window'
import { cn } from '@/lib/utils'

/**
 * A game's saves on the kryo.to account (src-tauri saves/): where this PC
 * keeps them, and why each place counts; when they were last backed up; the
 * account's versions to go back to. Locations found by watching the game are
 * offered to keep or ignore. Only shown where kryo.to has cloud saves on.
 */
export function CloudSavesCard({ game, running = false }: { game: LibraryGame; running?: boolean }) {
  const { data, error, reload } = useGameSaves(game.id)
  const [busy, setBusy] = useState<string | null>(null)
  const [message, setMessage] = useState<string | null>(null)
  const [versionsOpen, setVersionsOpen] = useState(false)

  if (!data || !data.feature) {
    return error && data?.feature ? <Card><p className="text-xs text-destructive">{error}</p></Card> : null
  }

  async function run(what: string, job: () => Promise<unknown>) {
    setBusy(what)
    setMessage(null)
    try {
      const out = await job()
      setMessage(typeof out === 'string' ? out : null)
    } catch (e) {
      setMessage(errorText(e))
    } finally {
      setBusy(null)
      reload()
    }
  }

  async function addFolder() {
    const picked = await pickPath({ title: `Choose a folder ${game.title} saves in`, directory: true, defaultPath: game.installDir })
    if (typeof picked === 'string') await run('add', () => cloudSaves.addFolder(game.id, picked))
  }

  const s = data.state
  const noPerk = data.perk === false
  const off = !data.on

  return (
    <Card className="grid gap-4">
      <div className="flex items-baseline justify-between gap-3">
        <Label>Saves</Label>
        <Caption>
          {data.files ? `${data.files} file${data.files === 1 ? '' : 's'} · ${formatBytes(data.bytes)}` : 'None found yet'}
          {s.lastBackupAt ? ` · backed up ${formatAgo(s.lastBackupAt)}` : ''}
        </Caption>
      </div>

      {s.conflict && !noPerk ? (
        <div role="alert" className="kryo-radius grid gap-3 border border-foreground/40 p-3">
          <p className="text-xs leading-relaxed text-foreground">
            This PC and your account both have newer saves for this game
            {s.cloudHead ? ` (the account's are from ${s.cloudHead.machine ?? 'another PC'}, ${formatAgo(s.cloudHead.createdAt)})` : ''}. Pick which to keep. The other is kept as a backup.
          </p>
          <div className="flex flex-wrap gap-2">
            <Button size="sm" disabled={running || !!busy} onClick={() => void run('local', () => cloudSaves.backup(game.id, true))}>Keep this PC&apos;s</Button>
            {s.cloudHead ? (
              <Button size="sm" disabled={running || !!busy} onClick={() => void run('cloud', () => cloudSaves.restore(game.id, s.cloudHead!.id))}>Use the account&apos;s</Button>
            ) : null}
          </div>
        </div>
      ) : null}

      <ul className="grid gap-2" aria-label="Save locations">
        {data.locations.map((l) => (
          <LocationRow
            key={l.key}
            l={l}
            onRemove={() => void run('loc', () => cloudSaves.location(game.id, l.key, 'ignore'))}
          />
        ))}
        {!data.locations.length ? <li className="text-xs text-muted-foreground">No save location known yet. Play the game and Kryoto Desktop finds where it saves.</li> : null}
      </ul>

      {data.suggestions.length ? (
        <div className="grid gap-2">
          <Caption>Possible save locations, seen while the game ran</Caption>
          <ul className="grid gap-2">
            {data.suggestions.map((l) => (
              <LocationRow
                key={l.key}
                l={l}
                suggestion
                onKeep={() => void run('loc', () => cloudSaves.location(game.id, l.key, 'keep'))}
                onRemove={() => void run('loc', () => cloudSaves.location(game.id, l.key, 'ignore'))}
              />
            ))}
          </ul>
        </div>
      ) : null}

      {noPerk || off ? (
        <p className="flex items-center gap-2 border-t border-border pt-3 text-[11px] text-muted-foreground">
          <Cloud className="size-3.5 shrink-0" aria-hidden />
          <span className="grow">{off ? 'Cloud saves are off in Settings.' : 'Supporters keep their saves on their kryo.to account, and get them back on any PC.'}</span>
          <button
            type="button"
            onClick={() => (off ? openSettingsSection('cloud') : openStorePage('/donate'))}
            className="shrink-0 font-bold uppercase tracking-wider text-foreground hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring"
          >
            {off ? 'Settings' : 'See how'}
          </button>
        </p>
      ) : (
        <div className="flex flex-wrap items-center gap-2 border-t border-border pt-3">
          <Button size="sm" variant="primary" disabled={running || !!busy || !data.files} onClick={() => void run('backup', () => cloudSaves.backup(game.id))}>
            <CloudUpload className="size-3" aria-hidden />
            {busy === 'backup' ? 'Backing up' : 'Back up now'}
          </Button>
          <Button size="sm" disabled={!!busy} onClick={() => setVersionsOpen(true)}>
            <History className="size-3" aria-hidden />
            Versions
          </Button>
          <Button size="sm" variant="ghost" disabled={!!busy} onClick={() => void addFolder()}>
            <FolderPlus className="size-3" aria-hidden />
            Add folder
          </Button>
          <button
            type="button"
            onClick={() => openSettingsSection('cloud')}
            className="ml-auto text-[10px] font-bold uppercase tracking-wider text-muted-foreground hover:text-foreground hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring"
          >
            Manage
          </button>
        </div>
      )}
      {running && !noPerk && !off ? <p className="text-[11px] text-muted-foreground">Backed up when the game closes.</p> : null}
      {s.tooBig ? <p className="text-[11px] text-destructive">These saves are bigger than a backup can be. Take off a location that is not saves.</p> : null}
      {message ? <p className="text-[11px] text-muted-foreground">{message}</p> : s.error && !s.conflict ? <p className="text-[11px] text-destructive">{s.error}</p> : null}

      {versionsOpen ? <VersionsModal game={game} running={running} localBackups={data.localBackups} baseVersion={s.baseVersion} onClose={() => { setVersionsOpen(false); reload() }} /> : null}
    </Card>
  )
}

function LocationRow({ l, suggestion = false, onKeep, onRemove }: { l: SaveLocation; suggestion?: boolean; onKeep?: () => void; onRemove: () => void }) {
  return (
    <li className={cn('grid grid-cols-[1fr_auto] items-center gap-3', suggestion && 'opacity-80')}>
      <span className="min-w-0">
        <span className="flex items-center gap-2">
          <span className="kryo-pill shrink-0 border border-border px-1.5 text-[9px] uppercase tracking-wider text-muted-foreground" title={l.reasons.join(', ') || undefined}>
            {l.sourceLabel}
          </span>
          <span className="kryo-ascii-art truncate text-[11px] text-foreground" title={l.path ?? l.label}>{l.label}</span>
        </span>
        <span className="block text-[10px] text-muted-foreground">
          {l.kind === 'registry' ? 'Registry values' : l.exists ? `${l.files} file${l.files === 1 ? '' : 's'} · ${formatBytes(l.bytes)}` : 'Not on this PC yet'}
          {suggestion && l.reasons.length ? ` · ${l.reasons.join(', ')}` : ''}
        </span>
      </span>
      <span className="flex gap-1">
        {suggestion && onKeep ? (
          <IconButton label="Keep this location" onClick={onKeep} className="size-7">
            <Plus className="size-3.5" />
          </IconButton>
        ) : null}
        {l.kind === 'files' && l.exists && l.path ? (
          <IconButton label="Open folder" onClick={() => void library.openFolder(l.path!)} className="size-7">
            <FolderOpen className="size-3.5" />
          </IconButton>
        ) : null}
        <IconButton label={suggestion ? 'Ignore this location' : 'Take this location off'} onClick={onRemove} className="size-7">
          <X className="size-3.5" />
        </IconButton>
      </span>
    </li>
  )
}

function VersionsModal({
  game,
  running,
  localBackups,
  baseVersion,
  onClose,
}: {
  game: LibraryGame
  running: boolean
  localBackups: { file: string; createdAt: number; size: number }[]
  baseVersion: string | null
  onClose: () => void
}) {
  const [versions, setVersions] = useState<SaveVersion[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const [done, setDone] = useState<string | null>(null)
  const [confirm, setConfirm] = useState<string | null>(null)

  useEffect(() => {
    void cloudSaves.versions(game.id).then(setVersions).catch((e: unknown) => setError(errorText(e)))
  }, [game.id])

  async function restore(key: string, job: () => Promise<string>) {
    if (confirm !== key) return setConfirm(key)
    setConfirm(null)
    setBusy(key)
    setError(null)
    try {
      setDone(await job())
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(null)
    }
  }

  return (
    <Modal title={`${game.title} saves`} onClose={onClose}>
      <div className="grid gap-5">
        <p className="text-xs leading-relaxed text-muted-foreground">
          Restoring puts that version on this PC. What is here now is kept first, under On this PC, so you can go back.
        </p>
        <section className="grid gap-2">
          <Caption>On your kryo.to account</Caption>
          {versions === null && !error ? <p className="text-xs text-muted-foreground">Loading…</p> : null}
          {versions && !versions.length ? <p className="text-xs text-muted-foreground">No backups yet.</p> : null}
          <ul className="grid gap-2">
            {(versions ?? []).map((v) => (
              <li key={v.id} className="grid grid-cols-[1fr_auto] items-center gap-3 text-xs">
                <span>
                  <b className="text-foreground">{new Date(v.createdAt * 1000).toLocaleString()}</b>
                  {v.id === baseVersion ? <Check className="ml-1.5 inline size-3 text-success" aria-label="On this PC" /> : null}
                  <span className="block text-[10px] text-muted-foreground">
                    {v.machine ?? 'A PC'} · {v.fileCount} file{v.fileCount === 1 ? '' : 's'} · {formatBytes(v.size)}
                  </span>
                </span>
                <Button size="sm" disabled={running || !!busy} onClick={() => void restore(v.id, () => cloudSaves.restore(game.id, v.id))}>
                  {busy === v.id ? 'Restoring' : confirm === v.id ? 'Press again' : 'Restore'}
                </Button>
              </li>
            ))}
          </ul>
        </section>
        {localBackups.length ? (
          <section className="grid gap-2">
            <Caption>On this PC</Caption>
            <ul className="grid gap-2">
              {localBackups.map((b) => (
                <li key={b.file} className="grid grid-cols-[1fr_auto] items-center gap-3 text-xs">
                  <span>
                    <b className="text-foreground">{new Date(b.createdAt * 1000).toLocaleString()}</b>
                    <span className="block text-[10px] text-muted-foreground">Kept before a restore · {formatBytes(b.size)}</span>
                  </span>
                  <Button size="sm" disabled={running || !!busy} onClick={() => void restore(b.file, () => cloudSaves.restoreLocal(game.id, b.file))}>
                    {busy === b.file ? 'Restoring' : confirm === b.file ? 'Press again' : 'Put back'}
                  </Button>
                </li>
              ))}
            </ul>
          </section>
        ) : null}
        {running ? <p className="text-[11px] text-muted-foreground">Close the game to restore.</p> : null}
        {done ? <p className="text-[11px] text-success">{done}</p> : null}
        {error ? <p className="text-[11px] text-destructive">{error}</p> : null}
      </div>
    </Modal>
  )
}
