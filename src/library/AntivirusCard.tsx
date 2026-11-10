import { useCallback, useEffect, useState } from 'react'
import { Download, ShieldAlert, ShieldCheck } from '@/ui/pixel-icons'
import { Button, Card, Label } from '@/ui'
import { call, errorText, isTauri, on } from '@/lib/bridge'
import { isWindowsHost, type LibraryGame } from '@/lib/library'
import { openSettingsSection } from '@/lib/window'
import { ManualSteps, covered, knownExcluded, type DefenderStatus, type ExcludeResult } from '@/settings/DefenderSection'

/** src-tauri antivirus.rs `Hit` and `Report`. */
type Hit = { file: string; threat: string; action: string; at: number }
type Report = { missing: string[]; missingCount: number; hits: Hit[]; canRestore: boolean }

/** Play's own words when Windows said an antivirus stopped a file as a threat (library.rs, useLibrary.ts). */
const THREAT = /antivirus blocked/i
/** Play's own words for a file Kryoto installed that has gone: worth reading Windows Security's log for. */
const GONE = /is gone from the game's folder/i

const name = (file: string) => file.split('/').pop() ?? file
const when = (at: number) => new Date(at * 1000).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' })
const did = (action: string) => {
  const a = action.toLowerCase()
  if (a.includes('quarantine')) return 'quarantined'
  if (a.includes('remove') || a.includes('clean')) return 'removed'
  if (a.includes('block')) return 'blocked'
  if (a.includes('allow')) return 'allowed'
  return 'flagged'
}

/**
 * What an antivirus did to this game, said plainly, with the way back: the
 * folder exclusion (the same one as Settings > Storage > Windows Security,
 * with Windows' administrator prompt) and the files fetched again over the
 * folder, which leaves saves and settings alone.
 *
 * Only Windows Security's own log is called Windows Security; files that are
 * gone with nothing in its log are only "missing" (src-tauri antivirus.rs).
 * Shows nothing when nothing is wrong.
 */
export function AntivirusCard({ game, running, error, onGetFiles }: { game: LibraryGame; running: boolean; error: string | null; onGetFiles: () => void }) {
  const [report, setReport] = useState<Report | null>(null)
  const [status, setStatus] = useState<DefenderStatus | null>(null)
  const [busy, setBusy] = useState<'exclude' | 'restore' | null>(null)
  const [result, setResult] = useState<ExcludeResult | null>(null)
  const [failure, setFailure] = useState<string | null>(null)
  // Windows said so: an antivirus stopped it. Gone only: something took it, and the log may say what.
  const blocked = !!error && THREAT.test(error)
  const deep = blocked || (!!error && GONE.test(error))

  const load = useCallback(() => {
    if (!isTauri() || game.launcher) return
    void call<Report>('game_antivirus', { id: game.id, deep })
      .then(setReport)
      .catch(() => setReport(null))
  }, [game.id, game.launcher, deep])

  useEffect(() => {
    setReport(null)
    setResult(null)
    setFailure(null)
    load()
  }, [load])
  useEffect(() => {
    let stop: (() => void) | undefined
    let cancelled = false
    void on('library-changed', load).then((fn) => (cancelled ? fn() : (stop = fn)))
    return () => {
      cancelled = true
      stop?.()
    }
  }, [load])

  const found = !!report && (report.missingCount > 0 || report.hits.length > 0)
  // Asked only once there is something to say: it starts PowerShell.
  useEffect(() => {
    if (!isWindowsHost() || !(found || blocked)) return
    let cancelled = false
    void call<DefenderStatus>('defender_status').then((s) => !cancelled && setStatus(s)).catch(() => {})
    return () => {
      cancelled = true
    }
  }, [found, blocked])

  if (!report || !(found || blocked)) return null

  const hits = report.hits
  const byDefender = hits.length > 0
  const gone = hits.some((h) => ['quarantined', 'removed'].includes(did(h.action)))
  const title = byDefender
    ? gone
      ? 'Windows Security removed files from this game'
      : 'Windows Security blocked a file in this game'
    : blocked
      ? 'Your antivirus blocked this game'
      : 'Files are missing from this game'
  const defenderOn = !!status?.supported && !status.passive
  const excluded = !!status && (result?.ok || covered(game.installDir, knownExcluded(status)))
  const others = status?.others ?? []

  const exclude = async () => {
    setBusy('exclude')
    setFailure(null)
    setResult(null)
    try {
      const r = await call<ExcludeResult>('defender_exclude', { kind: 'game', game: game.id })
      setResult(r)
      if (r.ok) void call<DefenderStatus>('defender_status').then(setStatus).catch(() => {})
    } catch (e) {
      setFailure(errorText(e))
    } finally {
      setBusy(null)
    }
  }
  const restore = async () => {
    setBusy('restore')
    setFailure(null)
    try {
      await call<string>('game_restore_files', { id: game.id })
      onGetFiles()
    } catch (e) {
      setFailure(errorText(e))
    } finally {
      setBusy(null)
    }
  }

  return (
    <Card className="grid gap-3 border-warning/50">
      <Label>Antivirus</Label>
      <p className="flex items-center gap-2 text-sm font-bold text-foreground">
        <ShieldAlert className="size-4 shrink-0 text-warning" />
        {title}
      </p>

      {byDefender ? (
        <ul className="grid gap-1 text-xs text-muted-foreground">
          {hits.slice(0, 3).map((h) => (
            <li key={h.file} className="min-w-0">
              <b className="select-text break-all font-mono text-[11px] text-foreground">{h.file}</b> {did(h.action)} {h.at ? `on ${when(h.at)}` : ''}
              {h.threat ? <span className="block select-text text-[11px]">as {h.threat}</span> : null}
            </li>
          ))}
          {hits.length > 3 ? <li>and {hits.length - 3} more</li> : null}
        </ul>
      ) : report.missingCount > 0 ? (
        <p className="text-xs leading-relaxed text-muted-foreground">
          <b className="select-text break-all font-mono text-[11px] text-foreground">{report.missing.slice(0, 3).map(name).join(', ')}</b>
          {report.missingCount > 3 ? ` and ${report.missingCount - 3} more` : ''} {report.missingCount === 1 ? 'was' : 'were'} part of the game when Kryoto
          installed it and {report.missingCount === 1 ? 'is' : 'are'} gone now.
        </p>
      ) : null}

      <p className="text-xs leading-relaxed text-muted-foreground">
        {byDefender || blocked
          ? 'Game cracks and the emulators that let a game start without its store are often flagged as threats, though they are what the game needs. '
          : others.length > 0
            ? `${others.join(', ')} ${others.length === 1 ? 'protects' : 'protect'} this PC and may have taken them: add the game's folder to its exclusions before getting them back, or it takes them again. `
            : isWindowsHost()
              ? "Windows Security's log does not name them, so something else removed them. "
              : 'Something removed them. '}
        {byDefender || blocked
          ? excluded
            ? 'Its folder is excluded now, so it stays as it is.'
            : 'Excluding the game\'s folder stops that. The risk: nothing in the folder is scanned any more, so only exclude folders that hold games.'
          : ''}
      </p>

      <div className="flex flex-wrap items-center gap-2">
        {isWindowsHost() && defenderOn && !excluded && (byDefender || blocked) ? (
          <Button size="sm" variant="primary" disabled={!!busy} onClick={() => void exclude()}>
            <ShieldCheck className="size-3" />
            {busy === 'exclude' ? 'Waiting for Windows' : 'Exclude the folder'}
          </Button>
        ) : null}
        {report.missingCount > 0 && report.canRestore ? (
          <Button size="sm" variant={excluded || !(byDefender || blocked) ? 'primary' : 'outline'} disabled={!!busy || running} onClick={() => void restore()}>
            <Download className="size-3" />
            Get the files back
          </Button>
        ) : null}
        {isWindowsHost() ? (
          <Button size="sm" variant="ghost" onClick={() => openSettingsSection('storage', 'defender')}>
            Windows Security settings
          </Button>
        ) : null}
      </div>
      {isWindowsHost() && defenderOn && !excluded && (byDefender || blocked) ? (
        <p className="text-[10px] text-muted-foreground">Windows asks for administrator permission first. Nothing else on your PC changes.</p>
      ) : null}
      {report.missingCount > 0 && !report.canRestore ? (
        <p className="text-[11px] text-muted-foreground">This game is not from kryo.to, so Kryoto cannot fetch its files again. Reinstall it from where you got it.</p>
      ) : null}
      {report.missingCount > 0 && report.canRestore && running ? <p className="text-[11px] text-muted-foreground">Close the game before getting its files back.</p> : null}
      {result?.ok ? (
        <p className="flex items-center gap-2 text-[11px] text-success">
          <ShieldCheck className="size-3.5" />
          Windows Security skips the game&apos;s folder now.
        </p>
      ) : null}
      {result && !result.ok ? <ManualSteps reason={result.reason} message={result.message} paths={[game.installDir]} /> : null}
      {failure ? <p className="text-[11px] text-destructive">{failure}</p> : null}
    </Card>
  )
}
