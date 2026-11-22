import { useCallback, useEffect, useRef, useState } from 'react'
import { Download, ListChecks, X } from '@/ui/pixel-icons'
import { AsciiBar, Button, Section } from '@/ui'
import { call, errorText, isTauri, on } from '@/lib/bridge'
import { library, type LibraryGame } from '@/lib/library'
import { alarms, checkedWhen, countLine, fileName, restoredLine, roleWords, type FileFinding, type FilesProgress, type FilesState, type RestoreProgress } from '@/lib/release-files'

/** antivirus.rs `Report`, for a release with no file list: the programs Kryoto installed that are gone. */
type InstalledReport = { missing: string[]; missingCount: number }

/**
 * Properties > Emulator & fixes > Files: the game's folder checked against
 * the list of files its release was made with (src-tauri release_files.rs),
 * naming exactly which file is missing and what it is, and getting it back.
 * A release with no list (every one made before Forge kept them) is checked
 * against the programs Kryoto installed, as the game page's antivirus card is.
 *
 * Stays when the auto-fixer is off for a release (lib/autofix.ts): checking
 * files changes nothing, only swapping the emulator is held back.
 */
export function FilesRow({ game, running, onGetFiles }: { game: LibraryGame; running: boolean; onGetFiles: () => void }) {
  const [state, setState] = useState<FilesState | null>(null)
  const [installed, setInstalled] = useState<InstalledReport | null>(null)
  const [busy, setBusy] = useState<'check' | 'full' | 'restore' | 'download' | null>(null)
  const [progress, setProgress] = useState<FilesProgress | null>(null)
  const [restoring, setRestoring] = useState<RestoreProgress | null>(null)
  const [done, setDone] = useState<string | null>(null)
  const [left, setLeft] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const inFlight = useRef(false)
  const id = game.id

  const load = useCallback(() => {
    void library.filesState(id).then(setState).catch(() => {})
  }, [id])
  useEffect(() => {
    if (!isTauri()) return
    setState(null)
    setInstalled(null)
    // Opening Properties is when the list is fetched, if it is due.
    void library.filesState(id).then((s) => {
      setState(s)
      if (s.list !== 'yes') void library.filesFetch(id).then(setState).catch(() => {})
    }).catch((e: unknown) => setError(errorText(e)))
  }, [id])
  useEffect(() => {
    const stops: (() => void)[] = []
    let cancelled = false
    const keep = (fn: () => void) => (cancelled ? fn() : stops.push(fn))
    void on<FilesProgress>('files-check-progress', (p) => p.gameId === id && setProgress(p)).then(keep)
    void on<RestoreProgress>('files-restore-progress', (p) => p.gameId === id && setRestoring(p)).then(keep)
    void on<string>('files-checked', (g) => { if (g === id && !inFlight.current) load() }).then(keep)
    return () => {
      cancelled = true
      stops.forEach((f) => f())
    }
  }, [id, load])

  const run = async (kind: 'check' | 'full' | 'restore', work: () => Promise<void>) => {
    if (inFlight.current) return
    inFlight.current = true
    setBusy(kind)
    setError(null)
    setDone(null)
    setLeft(false)
    setProgress(null)
    setRestoring(null)
    try {
      await work()
    } catch (e) {
      setError(errorText(e))
    } finally {
      inFlight.current = false
      setBusy(null)
      setProgress(null)
      setRestoring(null)
    }
  }

  const check = (full: boolean) =>
    run(full ? 'full' : 'check', async () => {
      // Fetches the list first when it is due.
      const next = await library.filesCheck(id, full)
      if (next.report?.cancelled) {
        setDone('Stopped. Nothing was changed.')
        load()
        return
      }
      setState(next)
      if (next.list === 'yes') return
      // No list for this release: the programs Kryoto installed.
      setInstalled(await call<InstalledReport>('game_antivirus', { id, deep: false }))
    })
  const restore = () =>
    run('restore', async () => {
      const r = await library.filesRestore(id)
      setDone(restoredLine(r))
      setLeft(r.left.length > 0)
      load()
    })
  const download = async () => {
    setBusy('download')
    try {
      await call<string>('game_restore_files', { id })
      onGetFiles()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(null)
    }
  }

  if (!isTauri() || !game.slug || game.launcher) return null

  const report = state?.report ?? null
  const found: FileFinding[] = report ? alarms([...report.missing, ...report.broken]) : []
  const withList = state?.list === 'yes'
  const checking = busy === 'check' || busy === 'full' || !!state?.checking
  const fraction = progress && progress.bytesTotal > 0 ? progress.bytesDone / progress.bytesTotal : progress && progress.total > 0 ? progress.done / progress.total : null

  return (
    <Section
      title="Files"
      hint={
        withList
          ? `Checks the game's folder against the ${state.files.toLocaleString('en-US')} files its release was made with, to find one an antivirus took. Files your mods changed are fine.`
          : "Checks that the programs Kryoto installed are still in the game's folder, to find one an antivirus took."
      }
    >
      {withList && report && !checking ? (
        found.length ? (
          <FoundList found={found} />
        ) : (
          <p className="text-xs text-foreground">
            {report.files - report.skipped === report.files ? 'All' : 'The'} {(report.files - report.skipped).toLocaleString('en-US')} files are there
            {report.full ? ' and match the release' : ''}, checked {checkedWhen(report.at)}.
          </p>
        )
      ) : null}
      {withList && report && !checking && report.changed > 0 ? (
        <p className="text-[11px] leading-snug text-muted-foreground">
          {report.changed.toLocaleString('en-US')} {report.changed === 1 ? 'file is' : 'files are'} not as the release shipped{' '}
          {report.changed === 1 ? 'it' : 'them'}, like a mod or a setting the game saved. That is fine:{' '}
          <span className="select-text break-all font-mono">{report.changedSample.slice(0, 3).map((f) => fileName(f.path)).join(', ')}</span>
          {report.changed > 3 ? ` and ${(report.changed - 3).toLocaleString('en-US')} more` : ''}.
        </p>
      ) : null}
      {!withList && installed && !checking ? (
        installed.missingCount ? (
          <p className="text-xs leading-relaxed text-foreground">
            {countLine(installed.missingCount, 'missing')}:{' '}
            <span className="select-text break-all font-mono text-[11px]">{installed.missing.slice(0, 3).join(', ')}</span>
            {installed.missingCount > 3 ? ` and ${installed.missingCount - 3} more` : ''}.
          </p>
        ) : (
          <p className="text-xs text-foreground">Every program Kryoto installed is there.</p>
        )
      ) : null}

      {checking ? (
        <div role="status" aria-live="polite" className="grid gap-1.5">
          <p className="text-xs text-muted-foreground">
            {busy === 'full' && progress ? `Checking ${progress.done.toLocaleString('en-US')} of ${progress.total.toLocaleString('en-US')} files` : 'Checking the files'}
          </p>
          {busy === 'full' ? <AsciiBar fraction={fraction} cells={24} /> : null}
        </div>
      ) : null}
      {busy === 'restore' && restoring ? (
        <div role="status" aria-live="polite" className="grid gap-1.5">
          <p className="text-xs text-muted-foreground">{restoring.text}</p>
          <AsciiBar fraction={restoring.percent == null ? null : restoring.percent / 100} cells={24} />
        </div>
      ) : null}
      {done ? <p role="status" className="text-xs text-foreground">{done}</p> : null}

      <div className="flex flex-wrap gap-2">
        {withList && found.length && !checking ? (
          <Button size="sm" variant="primary" disabled={!!busy || running} busy={busy === 'restore'} onClick={() => void restore()}>
            <Download className="size-3.5" aria-hidden /> Get the files back
          </Button>
        ) : null}
        {left || (!withList && (installed?.missingCount ?? 0) > 0) ? (
          <Button size="sm" variant={left ? 'primary' : 'outline'} disabled={!!busy || running} busy={busy === 'download'} onClick={() => void download()}>
            <Download className="size-3.5" aria-hidden /> Download it again{state?.downloadSize ? ` (${state.downloadSize})` : ''}
          </Button>
        ) : null}
        {busy === 'full' ? (
          <Button size="sm" variant="ghost" onClick={() => void library.filesCheckCancel(id)}>
            <X className="size-3.5" aria-hidden /> Cancel
          </Button>
        ) : (
          <>
            <Button size="sm" variant="outline" disabled={!!busy || !state} busy={busy === 'check'} onClick={() => void check(false)}>
              <ListChecks className="size-3.5" aria-hidden /> Check files
            </Button>
            {withList ? (
              <Button size="sm" variant="ghost" disabled={!!busy} onClick={() => void check(true)}>
                Check every file
              </Button>
            ) : null}
          </>
        )}
      </div>
      {withList && !checking ? (
        <p className="text-[10px] text-muted-foreground">Check every file reads the whole game, which takes a while on a hard disk.</p>
      ) : null}
      {running && (found.length > 0 || left) ? <p className="text-[11px] text-muted-foreground">Close the game before getting its files back.</p> : null}
      {error ? <p role="alert" className="break-words text-xs text-destructive">{error}</p> : null}
    </Section>
  )
}

/** "2 files are missing: `bin/steam_api64.dll` (the emulator's Steam file), `PlayGTAV.exe` (part of the game)." */
export function FoundList({ found, max = 3 }: { found: FileFinding[]; max?: number }) {
  return (
    <>
      {(['missing', 'broken'] as const).map((problem) => {
        const these = found.filter((f) => f.problem === problem)
        if (!these.length) return null
        return (
          <p key={problem} className="text-xs leading-relaxed text-foreground">
            {countLine(these.length, problem)}:{' '}
            {these.slice(0, max).map((f, i) => (
              <span key={f.path}>
                {i ? ', ' : ''}
                <b className="select-text break-all font-mono text-[11px] text-foreground">{f.path}</b>{' '}
                <span className="text-muted-foreground">({roleWords(f.role)})</span>
              </span>
            ))}
            {these.length > max ? ` and ${these.length - max} more` : ''}.
          </p>
        )
      })}
    </>
  )
}
