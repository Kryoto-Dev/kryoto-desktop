import { useEffect, useState } from 'react'
import { Check as Tick, ChevronDown, Copy, ShieldCheck } from '@/ui/pixel-icons'
import { Button, Segmented } from '@/ui'
import { call, errorText } from '@/lib/bridge'
import { isWindowsHost } from '@/lib/library'
import { cn } from '@/lib/utils'

/** src-tauri defender.rs `DefenderStatus`. */
export type DefenderStatus = Status
type Status = {
  supported: boolean
  realtime: boolean | null
  passive: boolean
  others: string[]
  /** Null: Windows shows its exclusions only to an administrator. */
  excluded: string[] | null
  /** What Kryoto itself added on this PC (src-tauri defender.rs). */
  remembered: string[]
  folders: string[]
  drives: string[]
}
export type ExcludeResult = { ok: boolean; reason: 'declined' | 'blocked' | 'failed' | null; message: string | null; excluded: string[] | null }
type Result = ExcludeResult
type Kind = 'folders' | 'drives'

const norm = (p: string) => p.replace(/[\\/]+$/, '').toLowerCase()
/** Covered by an exclusion: the path itself or a folder above it. */
export const covered = (path: string, excluded: string[]) => excluded.some((e) => norm(path) === norm(e) || norm(path).startsWith(`${norm(e)}\\`))

/** What is known to be excluded: Windows' own list, or for anyone it hides that from, what Kryoto added here. */
export const knownExcluded = (status: Pick<Status, 'excluded' | 'remembered'>) => status.excluded ?? status.remembered

/** Nothing left to ask: no Windows Security here, another antivirus does the scanning, or the exclusions are in. */
export async function defenderDone(): Promise<boolean> {
  const status = await call<Status>('defender_status')
  if (!status.supported || status.passive) return true
  return status.folders.length > 0 && status.folders.every((p) => covered(p, knownExcluded(status)))
}

/**
 * Windows Security (Microsoft Defender) exclusions for the game folders, in
 * the first-start setup and in Settings, Storage. Windows only.
 *
 * It says what the exclusion does and what it costs, shows which folders are
 * already excluded, shows the exact script before it runs, and asks Windows
 * for administrator permission once. When that cannot work (declined, a
 * policy in charge, another antivirus), it lists the steps to do it by hand
 * with each path to copy.
 */
export function DefenderSection({ compact = false }: { compact?: boolean }) {
  const [status, setStatus] = useState<Status | null>(null)
  const [kind, setKind] = useState<Kind>('folders')
  const [script, setScript] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<Result | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [excluded, setExcluded] = useState<string[] | null>(null)
  const [details, setDetails] = useState(false)

  useEffect(() => {
    if (!isWindowsHost()) return
    let cancelled = false
    void call<Status>('defender_status')
      .then((s) => {
        if (cancelled) return
        setStatus(s)
        setExcluded(s.excluded ?? (s.remembered.length ? s.remembered : null))
      })
      .catch((e: unknown) => !cancelled && setError(errorText(e)))
    return () => {
      cancelled = true
    }
  }, [])

  if (!isWindowsHost()) return null
  if (error && !status) return <p className="text-[11px] text-destructive">{error}</p>
  if (!status) return <p className="text-[11px] text-muted-foreground">Asking Windows Security…</p>
  if (!status.supported) {
    return <p className="text-[11px] leading-relaxed text-muted-foreground">Windows Security&apos;s settings cannot be reached on this PC, so there is nothing for Kryoto to change here.</p>
  }

  const paths = kind === 'drives' ? status.drives : status.folders
  const someoneElse = status.passive || status.others.length > 0
  const allDone = excluded !== null && paths.length > 0 && paths.every((p) => covered(p, excluded))
  // Done is one line. The explanation, the risk and the list are for deciding.
  if (allDone && !details && !compact) {
    return (
      <div className="flex flex-wrap items-center gap-3">
        <p className="flex items-center gap-2 text-[11px] text-success">
          <ShieldCheck className="size-3.5" />
          Windows Security skips your game folders.
        </p>
        <Button size="sm" variant="ghost" onClick={() => setDetails(true)}>
          Details
        </Button>
      </div>
    )
  }

  const add = async () => {
    setBusy(true)
    setError(null)
    setResult(null)
    try {
      const r = await call<Result>('defender_exclude', { kind })
      setResult(r)
      if (r.ok) setExcluded(r.excluded ?? [...knownExcluded({ excluded, remembered: status?.remembered ?? [] }), ...paths])
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }
  const showScript = async () => {
    if (script !== null) return setScript(null)
    try {
      setScript(await call<string>('defender_script', { kind }))
    } catch (e) {
      setError(errorText(e))
    }
  }

  return (
    <div className={cn('grid w-full gap-3 text-left', compact && 'gap-2.5')}>
      <p className="text-xs leading-relaxed text-muted-foreground">
        Windows Security scans every file a game unpacks as it is written. That slows downloads and unpacking, and it can delete files a game needs to run
        (a false alarm on the files that let a game start without its store). Kryoto can ask Windows to skip your game folders.
      </p>
      <p className="text-[11px] leading-relaxed text-warning">
        The risk: nothing in those folders is scanned any more, so a harmful file in them would not be caught. Only exclude folders that hold games, and keep
        Windows Security on for everything else.
      </p>

      {someoneElse ? (
        <p className="kryo-radius border border-border bg-card px-3 py-2 text-[11px] leading-relaxed text-muted-foreground">
          {status.others.length > 0 ? `${status.others.join(', ')} ${status.others.length === 1 ? 'protects' : 'protect'} this PC` : 'Another antivirus protects this PC'}
          {status.passive ? ', so Windows Security is not the one scanning: its exclusions would change nothing.' : '. If it scans your games too, add the same folders in its own settings.'}
        </p>
      ) : null}

      {status.drives.length > 0 ? (
        <Segmented
          label="What to exclude"
          value={kind}
          options={[
            { value: 'folders', label: 'Game folders' },
            { value: 'drives', label: status.drives.length === 1 ? 'The whole drive' : 'The whole drives' },
          ]}
          onChange={(v) => {
            setKind(v)
            setScript(null)
            setResult(null)
          }}
        />
      ) : null}

      <ul className="grid gap-1.5">
        {paths.map((p) => (
          <PathRow key={p} path={p} state={excluded === null ? null : covered(p, excluded)} />
        ))}
      </ul>

      {allDone ? (
        <p className="flex items-center gap-2 text-[11px] text-success">
          <ShieldCheck className="size-3.5" />
          Windows Security already skips {paths.length === 1 ? 'this' : 'these'}.
        </p>
      ) : (
        <div className="flex flex-wrap items-center gap-2">
          <Button size="sm" variant="primary" disabled={busy || status.passive || paths.length === 0} onClick={() => add()}>
            <ShieldCheck className="size-3" />
            {busy ? 'Waiting for Windows' : 'Add to Windows Security exclusions'}
          </Button>
          <Button size="sm" variant="ghost" onClick={() => showScript()}>
            <ChevronDown className={cn('size-3 transition-transform', script !== null && 'rotate-180')} />
            What it runs
          </Button>
        </div>
      )}
      {!allDone ? <p className="text-[10px] text-muted-foreground">Windows asks for administrator permission first. Nothing else on your PC changes.</p> : null}
      {script !== null ? (
        <pre className="kryo-radius max-h-48 select-text overflow-auto border border-border bg-background p-3 font-mono text-[10px] leading-relaxed text-muted-foreground">{script}</pre>
      ) : null}

      {result?.ok ? (
        <p className="flex items-center gap-2 text-[11px] text-success">
          <Tick className="size-3.5" />
          Added. Windows Security now skips {paths.length === 1 ? 'it' : 'them'}.
        </p>
      ) : null}
      {error ? <p className="text-[11px] text-destructive">{error}</p> : null}
      {result && !result.ok ? <ManualSteps reason={result.reason} message={result.message} paths={paths} /> : null}
    </div>
  )
}

function PathRow({ path, state }: { path: string; state: boolean | null }) {
  const [copied, setCopied] = useState(false)
  return (
    <li className="kryo-radius flex items-center gap-2 border border-border px-3 py-1.5">
      <span className="min-w-0 grow select-text truncate font-mono text-[11px] text-foreground">{path}</span>
      {state !== null ? <span className={cn('shrink-0 text-[10px] uppercase tracking-wider', state ? 'text-success' : 'text-muted-foreground')}>{state ? 'Excluded' : 'Scanned'}</span> : null}
      <button
        type="button"
        aria-label={`Copy ${path}`}
        onClick={() =>
          void navigator.clipboard.writeText(path).then(() => {
            setCopied(true)
            window.setTimeout(() => setCopied(false), 1500)
          })
        }
        className="kryo-pill grid size-6 shrink-0 place-items-center text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
      >
        {copied ? <Tick className="size-3" /> : <Copy className="size-3" />}
      </button>
    </li>
  )
}

/** When Kryoto cannot do it: why, and the same thing by hand in Windows Security. */
export function ManualSteps({ reason, message, paths }: { reason: Result['reason']; message: string | null; paths: string[] }) {
  const why =
    reason === 'declined'
      ? 'The administrator prompt was declined, so nothing changed.'
      : reason === 'blocked'
        ? `Windows refused the change${message ? ` (${message})` : ''}. On a PC where an organisation, Tamper Protection or another antivirus manages Windows Security, only it can add exclusions.`
        : `It did not work${message ? `: ${message}` : ''}.`
  return (
    <div className="kryo-radius grid gap-2 border border-border bg-card p-3 text-[11px] leading-relaxed text-muted-foreground">
      <p className="text-foreground">{why}</p>
      <p>To add them yourself:</p>
      <ol className="grid list-decimal gap-1 pl-5">
        <li>Open Start, type Windows Security, and open it.</li>
        <li>Go to Virus &amp; threat protection, then Manage settings under Virus &amp; threat protection settings.</li>
        <li>Scroll to Exclusions and choose Add or remove exclusions. Allow the administrator prompt.</li>
        <li>Choose Add an exclusion, then Folder, and pick each of these (copy a path into the folder box):</li>
      </ol>
      <ul className="grid gap-1.5">
        {paths.map((p) => (
          <PathRow key={p} path={p} state={null} />
        ))}
      </ul>
    </div>
  )
}
