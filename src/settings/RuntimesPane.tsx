import { useCallback, useEffect, useMemo, useState } from 'react'
import { Check as CheckIcon, Download, RotateCw, ShieldCheck, Trash2 } from 'lucide-react'
import { AsciiBar, Button, Check, Section } from '@/ui'
import { errorText } from '@/lib/bridge'
import { formatBytes } from '@/lib/downloads'
import { asksAtStartup, busy, runtimesApi, setAskAtStartup, useRuntimesProgress, type Runtime, type RuntimesProgress } from '@/lib/runtimes'
import { cn } from '@/lib/utils'

/**
 * Settings > Game runtimes: what is installed, what is missing, and one
 * button for the missing ones (or every ticked one, to repair a broken copy).
 * Windows asks for permission once per install; the installers come from
 * Microsoft and are checked for Microsoft's signature before they run.
 */
export function RuntimesPane() {
  const [list, setList] = useState<Runtime[] | null>(null)
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const [note, setNote] = useState<string | null>(null)
  const [ask, setAsk] = useState(asksAtStartup)
  const progress = useRuntimesProgress()
  const running = busy(progress)

  const refresh = useCallback(() => {
    void runtimesApi
      .list()
      .then((l) => {
        setList(l)
        // Start with the missing recommended ones ticked.
        setPicked((cur) => (cur.size ? cur : new Set(l.filter((r) => !r.installed && r.recommended).map((r) => r.id))))
      })
      .catch((e: unknown) => setNote(errorText(e)))
  }, [])
  useEffect(refresh, [refresh])
  // A finished job changes what is installed.
  useEffect(() => {
    if (progress && !busy(progress)) refresh()
  }, [progress, refresh])

  const groups = useMemo(() => {
    const m = new Map<string, Runtime[]>()
    for (const r of list ?? []) m.set(r.group, [...(m.get(r.group) ?? []), r])
    return [...m.entries()]
  }, [list])
  const missing = (list ?? []).filter((r) => !r.installed)
  const chosen = (list ?? []).filter((r) => picked.has(r.id))
  const repairing = chosen.some((r) => r.installed)
  const mb = chosen.filter((r) => !r.cached).reduce((n, r) => n + r.mb, 0)

  const toggle = (id: string, on: boolean) =>
    setPicked((cur) => {
      const next = new Set(cur)
      if (on) next.add(id)
      else next.delete(id)
      return next
    })
  const install = (ids: string[], repair: boolean) => {
    setNote(null)
    void runtimesApi.install(ids, repair).catch((e: unknown) => setNote(errorText(e)))
  }

  if (list && !list.length) {
    return (
      <Section title="Game runtimes">
        <p className="text-xs leading-relaxed text-muted-foreground">Only for Windows. On Linux, Proton brings the runtimes Windows games need.</p>
      </Section>
    )
  }

  return (
    <>
      <Section
        title="On this PC"
        hint="Most Windows games need some of these Microsoft runtimes and fail to start, or say a DLL is missing, without them. Kryoto Desktop downloads them from Microsoft, checks each one carries Microsoft's signature, and installs them together, so Windows asks for permission once."
      >
        {list === null ? (
          <AsciiBar fraction={null} cells={16} showPct={false} />
        ) : (
          <p className="text-xs text-foreground">
            {missing.length ? `${missing.length} of ${list.length} are not installed on this PC.` : 'Everything is installed.'}
          </p>
        )}
      </Section>

      {progress && progress.phase !== 'idle' ? <RuntimesProgressView progress={progress} onCancel={() => void runtimesApi.cancel()} /> : null}

      {groups.map(([group, items]) => (
        <Section key={group} title={group}>
          <ul className="grid gap-1.5">
            {items.map((r) => (
              <li key={r.id} className="grid grid-cols-[1fr_auto] items-start gap-4">
                <div className="grid gap-0.5">
                  <Check disabled={running} checked={picked.has(r.id)} onChange={(v) => toggle(r.id, v)} label={r.name} />
                  <p className="pl-[26px] text-[11px] leading-relaxed text-muted-foreground">{r.detail}</p>
                </div>
                <span className={cn('flex items-center gap-1.5 pt-0.5 text-[10px] font-bold uppercase tracking-wider', r.installed ? 'text-success' : 'text-muted-foreground')}>
                  {r.installed ? <CheckIcon className="size-3" /> : null}
                  {r.installed ? 'Installed' : 'Missing'}
                </span>
              </li>
            ))}
          </ul>
        </Section>
      ))}

      {list?.length ? (
        <Section title="Install">
          <div className="flex flex-wrap items-center gap-2">
            <Button variant="primary" disabled={running || !chosen.length} onClick={() => install(chosen.map((r) => r.id), repairing)}>
              {repairing ? <RotateCw className="size-3" /> : <Download className="size-3" />}
              {repairing ? `Install or repair ${chosen.length}` : chosen.length ? `Install ${chosen.length}` : 'Pick some to install'}
            </Button>
            <Button disabled={running || !missing.length} onClick={() => install(missing.map((r) => r.id), false)}>
              Install everything missing
            </Button>
            <Button variant="ghost" disabled={running} onClick={refresh}>
              Check again
            </Button>
          </div>
          <p className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
            <ShieldCheck className="size-3.5" />
            {mb ? `About ${mb} MB to download. ` : ''}
            Installing over one that is already there repairs it.
          </p>
        </Section>
      ) : null}

      <Section title="At startup">
        <Check
          checked={ask}
          onChange={(v) => {
            setAsk(v)
            setAskAtStartup(v)
          }}
          label="Offer missing runtimes when Kryoto Desktop starts"
        />
      </Section>

      <Section title="Downloaded installers" hint="Kept so a repair needs no download. They take up a little over 100 MB when all are there.">
        <div>
          <Button
            size="sm"
            disabled={running}
            onClick={() =>
              void runtimesApi
                .clearCache()
                .then((n) => {
                  setNote(n ? `Freed ${formatBytes(n)}.` : 'Nothing to clear.')
                  refresh()
                })
                .catch((e: unknown) => setNote(errorText(e)))
            }
          >
            <Trash2 className="size-3" />
            Clear downloaded installers
          </Button>
        </div>
      </Section>

      {note ? <p className="text-xs text-muted-foreground" role="status">{note}</p> : null}
    </>
  )
}

const PHASE: Record<RuntimesProgress['phase'], string> = {
  idle: '',
  downloading: 'Downloading',
  permission: 'Waiting for permission',
  installing: 'Installing',
  done: 'Done',
  failed: 'Not finished',
  canceled: 'Canceled',
}

/**
 * Where an install has got to, in the ASCII bar: the step, the runtime, and
 * Cancel while it runs. Shared with the start-screen prompt.
 */
export function RuntimesProgressView({ progress: p, onCancel, compact = false, children }: { progress: RuntimesProgress; onCancel?: () => void; compact?: boolean; children?: React.ReactNode }) {
  const running = busy(p)
  const failed = p.results.filter((r) => !r.ok)
  return (
    <div role="status" aria-live="polite" className={cn('grid gap-2', compact ? 'justify-items-center text-center' : 'kryo-radius border border-border bg-card/60 p-4')}>
      <p className="text-[10px] font-bold uppercase tracking-[0.25em] text-muted-foreground">
        {PHASE[p.phase]}
        {running && p.steps ? ` · ${Math.max(1, p.step)} of ${p.steps}` : ''}
      </p>
      <AsciiBar fraction={p.phase === 'permission' ? null : p.fraction} cells={compact ? 18 : 28} showPct={!compact || running} />
      <p className={cn('text-xs', p.phase === 'failed' ? 'text-destructive' : 'text-foreground')}>
        {running ? (p.phase === 'permission' ? 'Say yes in the Windows prompt to install.' : p.current) : p.message}
      </p>
      {!running && failed.length ? (
        <ul className={cn('grid gap-0.5 text-[11px] text-muted-foreground', compact && 'hidden')}>
          {failed.map((f) => (
            <li key={f.id}>
              {f.name}: {f.detail}
            </li>
          ))}
        </ul>
      ) : null}
      {running && onCancel ? (
        <div className="flex gap-2">
          <Button size="sm" variant="ghost" onClick={onCancel}>
            Cancel
          </Button>
          {children}
        </div>
      ) : (
        children
      )}
    </div>
  )
}
