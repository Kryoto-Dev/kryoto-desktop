import { useEffect, useState } from 'react'
import { Check, ChevronDown, Cloud, CloudUpload, FolderOpen, FolderPlus, History, Plus, RotateCcw, ScanSearch, X } from '@/ui/pixel-icons'
import { AsciiBar, Busy, Button, Caption, Card, IconButton, Label, Modal, Tag } from '@/ui'
import { errorText } from '@/lib/bridge'
import { PHASE_LABEL, cloudSaves, formatAgo, formatBytes, useGameSaves, useSaveProgress, type SaveFinding, type SaveLocation, type SaveVersion } from '@/lib/cloud-saves'
import { library, type LibraryGame } from '@/lib/library'
import { pickPath } from '@/lib/pick'
import { anchorId } from '@/lib/settings-links'
import { useSettings } from '@/lib/settings'
import { openSettingsSection, openStorePage } from '@/lib/window'
import { cn } from '@/lib/utils'
import { KryotoCloudCard, SavePlace, keptIn } from '@/settings/SavePlace'

/**
 * A game's saves and their versions (src-tauri saves/), kept on this PC, on
 * the player's own server or on the kryo.to account: where this PC keeps the
 * saves, and why each place counts; when they were last backed up; the
 * versions to go back to. Locations found by watching the game are offered to
 * keep or ignore. Before a place is picked (no kryo.to perk), it offers the
 * free ones.
 */
export function CloudSavesCard({ game, running = false }: { game: LibraryGame; running?: boolean }) {
  const { data, error, reload } = useGameSaves(game.id)
  const settings = useSettings()
  // A backup started anywhere (Back up now, a game closing, Play restoring)
  // shows here as it goes; a big one takes minutes and looked stuck.
  const progress = useSaveProgress(game.id)
  const [busy, setBusy] = useState<string | null>(null)
  const [message, setMessage] = useState<string | null>(null)
  const [versionsOpen, setVersionsOpen] = useState(false)

  if (!data) return error ? <Card><p className="text-xs text-destructive">{error}</p></Card> : null
  if (!data.feature) {
    return (
      <Card className="grid gap-3">
        <div className="grid gap-1">
          <Label>Save versions</Label>
          <p className="text-[11px] leading-relaxed text-muted-foreground">Keep versions of {game.title}'s saves to go back to, on this PC or on your own server.</p>
        </div>
        <SavePlace what="saves" compact />
        <KryotoCloudCard />
      </Card>
    )
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
    await run('add', async () => {
      const picked = await pickPath({ title: `Choose a folder ${game.title} saves in`, directory: true, defaultPath: game.installDir })
      if (typeof picked === 'string') await cloudSaves.addFolder(game.id, picked)
    })
  }

  const s = data.state
  const noPerk = data.perk === false
  const off = !data.on
  const found = [
    data.files ? `${data.files} file${data.files === 1 ? '' : 's'} · ${formatBytes(data.bytes)}` : null,
    data.registryValues ? `${data.registryValues} registry value${data.registryValues === 1 ? '' : 's'}` : null,
  ].filter(Boolean)
  const hasSaves = found.length > 0
  // Places checked but empty here (most emulators' folders, for one game)
  // fold away; what this PC has, or the player added, stays in sight.
  const shown = data.locations.filter((l) => l.exists || l.source === 'player')
  const elsewhere = data.locations.filter((l) => !shown.includes(l))
  const overCap = s.tooBig || data.bytes > data.maxBackupBytes
  const leaveOut = (key: string, folder: string, on: boolean) => void run('leave', () => cloudSaves.leaveOut(game.id, key, folder, on))

  const place = keptIn(settings, 'saves')

  return (
    <div id={anchorId('game-saves')}>
    <Card className="grid gap-4">
      <div className="flex items-baseline justify-between gap-3">
        <Label>Saves</Label>
        <Caption>
          {hasSaves ? found.join(' · ') : 'None found yet'}
          {s.lastBackupAt ? ` · backed up ${formatAgo(s.lastBackupAt)}` : ''}
        </Caption>
      </div>

      {progress ? (
        <div className="grid gap-1.5" role="status" aria-live="polite">
          <div className="flex items-baseline justify-between gap-3 text-[11px]">
            <span className="font-bold text-foreground">{PHASE_LABEL[progress.phase as keyof typeof PHASE_LABEL]}</span>
            <span className="tabular-nums text-muted-foreground">
              {progress.total ? `${formatBytes(progress.done)} of ${formatBytes(progress.total)}` : ''}
            </span>
          </div>
          <AsciiBar fraction={progress.total ? progress.done / progress.total : null} cells={32} showPct={!!progress.total} />
        </div>
      ) : null}

      {s.conflict && !noPerk ? (
        <div role="alert" className="kryo-radius grid gap-3 border border-foreground/40 p-3">
          <p className="text-xs leading-relaxed text-foreground">
            This PC and your account both have newer saves for this game
            {s.cloudHead ? ` (the account's are from ${s.cloudHead.machine ?? 'another PC'}, ${formatAgo(s.cloudHead.createdAt)})` : ''}. Pick which to keep. The other is kept as a backup.
          </p>
          <div className="flex flex-wrap gap-2">
            <Button size="sm" disabled={running || !!busy} onClick={() => run('local', () => cloudSaves.backup(game.id, true))}>Keep this PC&apos;s</Button>
            {s.cloudHead ? (
              <Button size="sm" disabled={running || !!busy} onClick={() => run('cloud', () => cloudSaves.restore(game.id, s.cloudHead!.id))}>Use the account&apos;s</Button>
            ) : null}
          </div>
        </div>
      ) : null}

      <ul className="grid gap-2" aria-label="Save locations">
        {shown.map((l) => (
          <LocationRow
            key={l.key}
            l={l}
            onRemove={() => void run('loc', () => cloudSaves.location(game.id, l.key, 'ignore'))}
            onInclude={(folder) => leaveOut(l.key, folder, false)}
          />
        ))}
        {!shown.length ? (
          <li className="text-xs text-muted-foreground">
            {data.locations.length ? 'Nothing saved in the usual places yet.' : 'No save location known yet.'} Play the game and Kryoto Desktop finds where it saves.
          </li>
        ) : null}
      </ul>

      {elsewhere.length ? (
        <details className="group">
          <summary className="flex cursor-pointer list-none items-center gap-1.5 text-[10px] uppercase tracking-wider text-muted-foreground transition-colors hover:text-foreground">
            <ChevronDown className="size-3 transition-transform group-open:rotate-180" aria-hidden />
            Also checks {elsewhere.length} more place{elsewhere.length === 1 ? '' : 's'}, empty on this PC
          </summary>
          <ul className="mt-2 grid gap-2">
            {elsewhere.map((l) => (
              <LocationRow key={l.key} l={l} onRemove={() => void run('loc', () => cloudSaves.location(game.id, l.key, 'ignore'))} />
            ))}
          </ul>
        </details>
      ) : null}

      {overCap ? (
        <div id={anchorId('game-saves-fix')} role="alert" className="kryo-radius grid gap-3 border border-foreground/40 p-3">
          <p className="text-xs leading-relaxed text-foreground">
            Too big to back up: {formatBytes(data.bytes)} of saves, and a backup can be {formatBytes(data.maxBackupBytes)} here.
          </p>
          <Caption>{data.heavy.length ? 'Leave out what is not your saves (downloaded maps, mods, replays)' : 'Take a location that is not saves off the list above'}</Caption>
          {data.heavy.length ? <ul className="grid gap-2">
            {data.heavy.map((h) => (
              <li key={`${h.location}|${h.folder}`} className="grid grid-cols-[1fr_auto] items-center gap-3">
                <span className="min-w-0">
                  <span className="kryo-ascii-art block truncate text-[11px] text-foreground" title={h.label}>{h.label}</span>
                  <span className="block text-[10px] text-muted-foreground">{h.files} file{h.files === 1 ? '' : 's'} · {formatBytes(h.bytes)}</span>
                </span>
                <Button size="sm" disabled={!!busy} onClick={() => leaveOut(h.location, h.folder, true)}>Leave out</Button>
              </li>
            ))}
          </ul> : null}
          {place === 'account' ? (
            <div className="grid gap-2 border-t border-border pt-3">
              <Caption>Or keep them where the limit is higher: 8 GB on this PC</Caption>
              <SavePlace what="saves" compact />
              {data.maxBackupBytes < 1024 ** 3 ? (
                <Button size="sm" variant="ghost" className="w-fit" onClick={() => openStorePage('/perks#cloud')}>
                  See bigger limits on kryo.to
                </Button>
              ) : null}
            </div>
          ) : place === 'patchbay' ? (
            <div className="flex flex-wrap items-center gap-3 border-t border-border pt-3">
              <Caption>Your own server sets this limit</Caption>
              <Button size="sm" variant="ghost" onClick={() => openSettingsSection('patchbay')}>
                Open Patchbay
              </Button>
            </div>
          ) : null}
        </div>
      ) : null}

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

      {!noPerk && !off ? <PreciseFinding game={game} finding={data.finding} busy={!!busy} onSet={(on) => void run('precise', () => cloudSaves.findPrecisely(game.id, on))} /> : null}

      {data.ignored.length ? (
        <details className="group">
          <summary className="flex cursor-pointer list-none items-center gap-1.5 text-[10px] uppercase tracking-wider text-muted-foreground transition-colors hover:text-foreground">
            <ChevronDown className="size-3 transition-transform group-open:rotate-180" aria-hidden />
            Taken off ({data.ignored.length})
          </summary>
          <ul className="mt-2 grid gap-2">
            {data.ignored.map((l) => (
              <li key={l.key} className="grid grid-cols-[1fr_auto] items-center gap-3">
                <span className="min-w-0">
                  <span className="kryo-ascii-art block truncate text-[11px] text-muted-foreground" title={l.path ?? l.label}>{l.label}</span>
                  <span className="block text-[10px] text-muted-foreground">{l.sourceLabel}</span>
                </span>
                <Button size="sm" variant="ghost" disabled={!!busy} onClick={() => run('loc', () => cloudSaves.location(game.id, l.key, 'restore'))}>
                  <RotateCcw className="size-3" aria-hidden />
                  Put back
                </Button>
              </li>
            ))}
          </ul>
        </details>
      ) : null}

      {noPerk ? (
        <div className="grid gap-3 border-t border-border pt-3">
          <p className="text-[11px] text-muted-foreground">Keep this game's save versions on this PC or on your own server, for free.</p>
          <SavePlace what="saves" compact />
          <KryotoCloudCard />
        </div>
      ) : off ? (
        <p className="flex items-center gap-2 border-t border-border pt-3 text-[11px] text-muted-foreground">
          <Cloud className="size-3.5 shrink-0" aria-hidden />
          <span className="grow">Save versions are off in Settings.</span>
          <button
            type="button"
            onClick={() => openSettingsSection('cloud')}
            className="shrink-0 font-bold uppercase tracking-wider text-foreground hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring"
          >
            Settings
          </button>
        </p>
      ) : (
        <div className="flex flex-wrap items-center gap-2 border-t border-border pt-3">
          <Button size="sm" variant="primary" disabled={running || !!busy || !!progress || !hasSaves} onClick={() => run('backup', () => cloudSaves.backup(game.id))}>
            <CloudUpload className="size-3" aria-hidden />
            {busy === 'backup' || progress ? 'Backing up' : 'Back up now'}
          </Button>
          <Button size="sm" disabled={!!busy} onClick={() => setVersionsOpen(true)}>
            <History className="size-3" aria-hidden />
            Versions
          </Button>
          <Button size="sm" variant="ghost" disabled={!!busy} onClick={() => addFolder()}>
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
      {message ? <p className="text-[11px] text-muted-foreground">{message}</p> : s.error && !s.conflict ? <p className="text-[11px] text-destructive">{s.error}</p> : null}

      {versionsOpen ? <VersionsModal game={game} running={running} localBackups={data.localBackups} baseVersion={s.baseVersion} onClose={() => { setVersionsOpen(false); reload() }} /> : null}
    </Card>
    </div>
  )
}

/**
 * Precise finding for this game: that it will ask Windows next time, or the
 * offer to, when its saves were not found that way (smart mode, or the
 * player said no before). Nothing once they were.
 */
function PreciseFinding({ game, finding, busy, onSet }: { game: LibraryGame; finding: SaveFinding; busy: boolean; onSet: (on: boolean) => void }) {
  if (!finding.canPrecise || finding.preciseFound) return null
  if (finding.preciseNext) {
    return (
      <p className="flex items-start gap-2 text-[11px] leading-relaxed text-muted-foreground">
        <ScanSearch className="mt-0.5 size-3.5 shrink-0" aria-hidden />
        <span className="grow">Next time you play, Windows asks you once to let Kryoto Desktop see exactly which files {game.title} writes.</span>
        <button
          type="button"
          disabled={busy}
          onClick={() => onSet(false)}
          className="shrink-0 font-bold uppercase tracking-wider text-foreground hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring disabled:opacity-50"
        >
          Not now
        </button>
      </p>
    )
  }
  return (
    <p className="flex items-start gap-2 text-[11px] leading-relaxed text-muted-foreground">
      <ScanSearch className="mt-0.5 size-3.5 shrink-0" aria-hidden />
      <span className="grow">Missing saves? Find them precisely: Windows asks you once, the next time you play, and Kryoto Desktop sees exactly which files the game writes.</span>
      <button
        type="button"
        disabled={busy}
        onClick={() => onSet(true)}
        className="shrink-0 font-bold uppercase tracking-wider text-foreground hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring disabled:opacity-50"
      >
        Find precisely
      </button>
    </p>
  )
}

function LocationRow({
  l,
  suggestion = false,
  onKeep,
  onRemove,
  onInclude,
}: {
  l: SaveLocation
  suggestion?: boolean
  onKeep?: () => void
  onRemove: () => void
  /** Take a left-out folder back into backups. */
  onInclude?: (folder: string) => void
}) {
  return (
    <li className={cn('grid grid-cols-[1fr_auto] items-center gap-3', suggestion && 'opacity-80')}>
      <span className="min-w-0">
        <span className="flex items-center gap-2">
          <Tag title={l.reasons.join(', ') || undefined}>{l.sourceLabel}</Tag>
          <span className="kryo-ascii-art truncate text-[11px] text-foreground" title={l.path ?? l.label}>{l.label}</span>
        </span>
        <span className="block text-[10px] text-muted-foreground">
          {l.kind === 'registry'
            ? l.values
              ? `${l.values} registry value${l.values === 1 ? '' : 's'}`
              : 'Not on this PC yet'
            : l.exists
              ? `${l.files} file${l.files === 1 ? '' : 's'} · ${formatBytes(l.bytes)}`
              : 'Not on this PC yet'}
          {suggestion && l.reasons.length ? ` · ${l.reasons.join(', ')}` : ''}
        </span>
        {l.leftOut.length && onInclude ? (
          <span className="mt-0.5 flex flex-wrap items-center gap-x-2 text-[10px] text-muted-foreground">
            Left out:
            {l.leftOut.map((f) => (
              <button
                key={f}
                type="button"
                onClick={() => onInclude(f)}
                title={`Back up ${f} again`}
                className="inline-flex items-center gap-0.5 underline-offset-2 hover:text-foreground hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring"
              >
                {f}
                <RotateCcw className="size-3" aria-hidden />
              </button>
            ))}
          </span>
        ) : null}
      </span>
      <span className="flex gap-2">
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
          {versions === null && !error ? <Busy className="size-3.5 text-muted-foreground" label="Loading backups" /> : null}
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
                <Button size="sm" disabled={running || !!busy} onClick={() => restore(v.id, () => cloudSaves.restore(game.id, v.id))}>
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
                  <Button size="sm" disabled={running || !!busy} onClick={() => restore(b.file, () => cloudSaves.restoreLocal(game.id, b.file))}>
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
