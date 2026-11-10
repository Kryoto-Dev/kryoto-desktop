import { useEffect, useRef, useState } from 'react'
import { ArrowUpToLine, ChevronDown, ChevronUp, Download, FolderOpen, HardDrive, HeartHandshake, Pause, Play, RotateCw, ShieldAlert, X } from '@/ui/pixel-icons'
import { settingsApi, useSettings } from '@/lib/settings'
import { HatchZipExplainer } from '@/ui/FeatureNotice'
import { AsciiBar, AsciiSpark, Busy, Button, Caption, IconButton, Label, Matrix, Stat, type MatrixState } from '@/ui'
import { downloads as api, formatBytes, formatEta, formatLeft, isActive, isWorking, phaseOf, progressOf, type Download as Dl } from '@/lib/downloads'
import { Art } from '@/library/Art'
import { capsulesFor, library } from '@/lib/library'
import { getRelease, useReleases } from '@/library/Versions'
import { errorText } from '@/lib/bridge'
import { catalogApiUrl } from '@/lib/endpoint'
import { openExternal, STORE_HOME } from '@/lib/window'
import { EmptyState } from '@/ui/EmptyState'
import { BranchTag } from '@/library/BranchTag'
import { INBOX } from '@/ui/ascii/scenes'

/**
 * Downloads: the game moving now across the top with its speed drawn in
 * block characters, then what is waiting, then what finished - all in the
 * site's ASCII progress voice.
 */
/** Finished downloads are history: a few, and the rest on request. */
const FINISHED_ROWS = 5

export function DownloadsPage({
  list,
  onOpenGame,
  onRepair,
  onStore,
  onDonate,
  loading,
  error,
  onRetry,
}: {
  list: Dl[]
  onOpenGame: (id: string) => void
  /** Properties, Repair, for a game that unpacked with damage. */
  onRepair: (id: string) => void
  onStore: () => void
  /** kryo.to's donate page; absent for supporters, who are never asked. */
  onDonate: (() => void) | null
  loading: boolean
  error: string | null
  onRetry: () => void
}) {
  const [listError, setListError] = useState<string | null>(null)
  const current = list.find(isWorking)
  // In queue order, which the arrows change.
  const waiting = list
    .filter((d) => d !== current && (isActive(d) || d.status === 'paused' || d.status === 'failed'))
    .sort((a, b) => (a.queueOrder || Number.MAX_SAFE_INTEGER) - (b.queueOrder || Number.MAX_SAFE_INTEGER) || a.addedAt - b.addedAt)
  const done = list.filter((d) => d.status === 'installed' || d.status === 'canceled')
  const stopped = waiting.filter((d) => d.status === 'paused' || d.status === 'failed')
  const [allDone, setAllDone] = useState(false)

  if (!list.length && (loading || error)) {
    return <EmptyState art={INBOX} title={error ? 'Downloads unavailable' : 'Loading downloads'} body={error ?? 'Getting your download list.'}>
      {error ? <Button onClick={onRetry}>Retry</Button> : null}
    </EmptyState>
  }
  if (list.length === 0) {
    return (
      <EmptyState art={INBOX} title="No downloads" body="Press Download on a game in the store. It installs itself and appears in your library.">
        <Button variant="primary" onClick={onStore}>
          Browse the store
        </Button>
      </EmptyState>
    )
  }

  return (
    <div className="grid min-h-0 grow content-start gap-8 overflow-auto p-6">
      {error ? <div role="alert" className="flex items-center gap-3 text-xs text-destructive"><p>{error}</p><Button size="sm" onClick={onRetry}>Retry</Button></div> : null}
      {listError ? <p role="alert" className="text-xs text-destructive">{listError}</p> : null}
      {onDonate ? <Donate onDonate={onDonate} /> : null}
      {current ? <Current d={current} /> : null}
      {waiting.length ? (
        <section className="grid gap-3">
          <div className="flex items-center justify-between gap-3">
            <Label>Up next · {waiting.length}</Label>
            {/* Everything stopped, back in the queue in one press: after a
                restart, or a night the connection kept dropping. */}
            {stopped.length > 1 ? (
              <Button
                size="sm"
                onClick={() => {
                  setListError(null)
                  return Promise.all(stopped.map((d) => api.resume(d.id).catch((e: unknown) => setListError(errorText(e)))))
                }}
              >
                <Download className="size-3" />
                Resume all
              </Button>
            ) : null}
          </div>
          {waiting.map((d, i) => (
            <Row key={d.id} d={d} onOpenGame={onOpenGame} onRepair={onRepair} queue={{ first: i === 0, last: i === waiting.length - 1 }} />
          ))}
        </section>
      ) : null}
      {done.length ? (
        <section className="grid gap-3">
          <div className="flex items-center justify-between gap-3">
            <Label>Finished · {done.length}</Label>
            {/* One press for the whole list instead of one per row. The games
                stay installed; only the history goes. */}
            <Button
              size="sm"
              onClick={() => {
                setListError(null)
                return Promise.all(done.map((d) => api.remove(d.id).catch((e: unknown) => setListError(errorText(e)))))
              }}
            >
              <X className="size-3" />
              Clear all
            </Button>
          </div>
          {(allDone ? done : done.slice(0, FINISHED_ROWS)).map((d) => (
            <Row key={d.id} d={d} onOpenGame={onOpenGame} onRepair={onRepair} />
          ))}
          {done.length > FINISHED_ROWS ? (
            <Button size="sm" variant="ghost" className="w-fit" onClick={() => setAllDone(!allDone)} aria-expanded={allDone}>
              {allDone ? 'Show fewer' : `Show all ${done.length}`}
            </Button>
          ) : null}
        </section>
      ) : null}
    </div>
  )
}

/**
 * Every file comes from kryo.to's own filehost, and storage is what it costs:
 * said where the downloads are, so it is seen by the people using it.
 */
function Donate({ onDonate }: { onDonate: () => void }) {
  const [fact, setFact] = useState('Donations help keep Kryoto online.')
  useEffect(() => {
    let live = true
    void catalogApiUrl('/api/facts')
      .then((url) => fetch(url))
      .then((r) => (r.ok ? (r.json() as Promise<{ donation?: string }>) : null))
      .then((d) => {
        if (live && d?.donation) setFact(d.donation)
      })
      .catch(() => {})
    return () => {
      live = false
    }
  }, [])
  return (
    <section className="kryo-radius flex items-center gap-4 border border-border bg-card px-4 py-3">
      <HeartHandshake className="size-5 shrink-0 text-muted-foreground" aria-hidden />
      <p className="grow text-xs leading-relaxed text-muted-foreground">
        {fact}
      </p>
      <Button size="sm" onClick={onDonate}>
        Donate
      </Button>
    </section>
  )
}

/**
 * A download that stopped for want of room: what unpacking in place is, and
 * the switch that turns it on for every install, with Retry right there.
 * (In-place unpacking already steps in when the drive is short; this is the
 * case where even that was not enough, or the archive gains nothing from it.)
 */
function ShortOnSpace({ d }: { d: Dl }) {
  const settings = useSettings()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  if (!settings) return null
  const turnOn = async () => {
    setBusy(true)
    setError(null)
    try {
      const latest = await settingsApi.get()
      if (!latest.extractInPlace) await settingsApi.save({ ...latest, extractInPlace: true })
      await api.resume(d.id)
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="kryo-radius grid gap-3 border border-border bg-background/40 p-3">
      <HatchZipExplainer compact />
      {!settings.extractInPlace ? (
        <Button size="sm" disabled={busy} onClick={() => turnOn()} className="w-fit">
          <HardDrive className="size-3" />
          Extract in place and retry
        </Button>
      ) : null}
      {error ? <p role="alert" className="text-xs text-destructive">{error}</p> : null}
    </div>
  )
}

/** kryo.to turned a direct download down because they are for supporters right now (downloads.rs `SUPPORTER_REQUIRED`). */
export function isSupporterLocked(error: string): boolean {
  return /for supporters right now/i.test(error)
}

/**
 * Under a direct download refused for non-supporters: the two ways on,
 * side by side - donate (the site's donate page, in the browser), or one of
 * the release's mirrors, which Kryoto fetches itself.
 */
function SupporterLocked({ d, slug }: { d: Dl; slug: string }) {
  const { releases } = useReleases(slug)
  const release = releases?.find((r) => (d.release ? r.version === d.release : r.primary)) ?? null
  const mirrors = release?.sources.filter((s) => s.kind === 'mirror') ?? []
  const [error, setError] = useState<string | null>(null)
  return (
    <div className="flex flex-wrap items-center gap-2 pt-1">
      <Button variant="primary" size="sm" onClick={() => openExternal(`${STORE_HOME}donate`)}>
        <HeartHandshake className="size-3" />
        Donate
      </Button>
      {mirrors.map((m) => (
        <Button
          key={m.url}
          size="sm"
          onClick={() =>
            release && void getRelease(slug, d.meta.title, release, m, () => {}).catch((e: unknown) => setError(errorText(e)))
          }
        >
          <Download className="size-3" />
          {m.host}
        </Button>
      ))}
      {release && !mirrors.length ? <Caption>No mirrors for this build yet.</Caption> : null}
      {error ? <span className="text-xs text-destructive">{error}</span> : null}
    </div>
  )
}

/**
 * An action that takes a moment to land (pausing has to wait for the
 * connections to stop): which one was pressed, until the download's status
 * moves or ten seconds pass. The button shows it is working instead of
 * looking like it ignored the click.
 */
function usePending(d: Dl) {
  const [pending, setPending] = useState<string | null>(null)
  const [failed, setFailed] = useState<string | null>(null)
  useEffect(() => {
    setPending(null)
    setFailed(null)
  }, [d.status])
  useEffect(() => {
    if (!pending) return
    const t = setTimeout(() => setPending(null), 10_000)
    return () => clearTimeout(t)
  }, [pending])
  /** For an action that lands at once, with nothing to wait on. */
  const attempt = (action: () => Promise<unknown>) => {
    setFailed(null)
    void action().catch((e: unknown) => {
      setPending(null)
      setFailed(errorText(e))
    })
  }
  const run = (what: string, action: () => Promise<unknown>) => {
    if (pending) return
    setPending(what)
    attempt(action)
  }
  return { pending, failed, run, attempt }
}

/**
 * A download's state as one glyph of the matrix alphabet - the same family
 * as the site's toasts and the Store's loading marks. Downloading is the one
 * determinate state: the glyph fills, bottom row first, with the real
 * progress, so a glance at the list says how far each one is.
 */
const GLYPH: Partial<Record<Dl['status'], MatrixState>> = {
  queued: 'queue',
  resolving: 'connect',
  verifying: 'scan',
  extracting: 'process',
  paused: 'idle',
  failed: 'error',
  installed: 'success',
  canceled: 'unavailable',
}

function StatusGlyph({ d, className }: { d: Dl; className?: string }) {
  const tone =
    d.status === 'failed' ? 'text-destructive' : d.status === 'installed' ? 'text-success' : 'text-foreground'
  if (d.status === 'downloading') {
    return <Matrix progress={d.total ? d.received / d.total : 0} className={`${tone} ${className ?? ''}`} />
  }
  return <Matrix state={GLYPH[d.status] ?? 'busy'} className={`${tone} ${className ?? ''}`} />
}

function Current({ d }: { d: Dl }) {
  const samples = useSpeedHistory(d)
  const { pending, failed, run } = usePending(d)
  // Past the download: checking the hash, then unpacking.
  const extracting = d.status === 'extracting' || d.status === 'verifying'
  const unpackRate = useRate(extracting ? d.extracted : null, `${d.id}:${d.status}`)
  const resolving = d.status === 'resolving'
  // The same figure as the rail and the bottom bar, so all three agree and it never runs back to nothing between phases.
  const fraction = resolving || (extracting && !d.extractTotal) || (!extracting && !d.total) ? null : progressOf(d)
  return (
    <section className="kryo-radius kryo-in grid grid-cols-[260px_1fr] gap-6 border border-border bg-card p-5">
      <Art adult={d.meta.nsfw} src={capsulesFor(d.meta)[0]} fallback={capsulesFor(d.meta).slice(1)} title={d.meta.title} where="downloads" className="kryo-radius aspect-[460/215] w-full object-cover" />
      <div className="grid content-start gap-4">
        <div className="flex items-start justify-between gap-4">
          <div className="grid gap-1">
            <Caption className="flex items-center gap-2">
              <StatusGlyph d={d} className="size-3" />
              {phaseOf(d)}
              {d.addon ? ` · ${d.addon}` : ''}
              {d.mirror && d.status !== 'resolving' ? ` · from ${d.mirror.host}` : ''}
              {d.release ? ` · build ${d.release}` : ''}
            </Caption>
            <h2 className="flex min-w-0 items-center gap-2 text-xl font-bold text-foreground">
              <span className="truncate">{d.meta.title}</span>
              <BranchTag branch={d.meta.steamBranch} title={d.meta.title} />
            </h2>
          </div>
          {!extracting ? (
            <div className="flex gap-2">
              <Button size="sm" disabled={Boolean(pending)} onClick={() => run('pause', () => api.pause(d.id))}>
                {pending === 'pause' ? <Busy className="size-3" /> : <Pause className="size-3" />}
                {pending === 'pause' ? 'Pausing' : 'Pause'}
              </Button>
              <Button size="sm" variant="ghost" disabled={Boolean(pending)} onClick={() => run('cancel', () => api.cancel(d.id))}>
                {pending === 'cancel' ? <Busy className="size-3" /> : null}
                {pending === 'cancel' ? 'Cancelling' : 'Cancel'}
              </Button>
            </div>
          ) : null}
        </div>
        <AsciiBar fraction={fraction} cells={44} className="text-sm" />
        {failed ? <p role="alert" className="text-xs text-destructive">{failed}</p> : null}
        {/* One size from start to finish, and each figure in a column of its
            own: a value changing length moves nothing beside it, and a phase
            with less to say leaves the card as tall as it was. */}
        <div className="grid min-h-[5rem] content-start gap-4">
          {resolving ? (
            <p className="text-xs leading-relaxed text-muted-foreground">
              Getting the file's address from {d.mirror?.host ?? 'the mirror'}. Hosts with a check open their page in a window of its own:
              if one appears, pass the check there and the download carries on here.
            </p>
          ) : (
            <dl className="grid grid-cols-[repeat(3,minmax(0,11rem))] gap-x-6">
              {extracting ? (
                <>
                  <Stat label="Speed" value={unpackRate > 0 ? `${formatBytes(unpackRate)}/s` : '...'} />
                  <Stat
                    label={d.status === 'verifying' ? 'Checked' : 'Unpacked'}
                    value={`${formatBytes(d.extracted)}${d.extractTotal ? ` / ${formatBytes(d.extractTotal)}` : ''}`}
                  />
                  {unpackRate > 0 && d.extractTotal ? (
                    <Stat label="Time left" value={formatLeft((d.extractTotal - d.extracted) / unpackRate)} />
                  ) : null}
                </>
              ) : (
                <>
                  <Stat label="Speed" value={`${formatBytes(d.speed)}/s`} />
                  <Stat label="Downloaded" value={`${formatBytes(d.received)}${d.total ? ` / ${formatBytes(d.total)}` : ''}`} />
                  {formatEta(d) ? <Stat label="Time left" value={formatEta(d)!} /> : null}
                </>
              )}
            </dl>
          )}
          {d.notice ? (
            <p className="flex items-center gap-2 text-xs leading-relaxed text-muted-foreground" role="status">
              <Busy className="size-3" />
              {d.notice}
            </p>
          ) : !extracting && !resolving ? (
            <AsciiSpark samples={samples} width={56} />
          ) : null}
        </div>
      </div>
    </section>
  )
}

function Row({
  d,
  onOpenGame,
  onRepair,
  queue,
}: {
  d: Dl
  onOpenGame: (id: string) => void
  onRepair: (id: string) => void
  /** Set for rows in the queue: where it is, for the arrows. */
  queue?: { first: boolean; last: boolean }
}) {
  const pct = progressOf(d)
  const { pending, failed, run, attempt } = usePending(d)
  const status =
    d.status === 'installed'
      ? `Installed${d.finishedAt ? ` ${new Date(d.finishedAt * 1000).toLocaleDateString()}` : ''}`
      : d.status === 'canceled'
        ? 'Cancelled'
        : d.status === 'failed'
          ? 'Failed'
          : d.status === 'paused'
            ? 'Paused'
            : d.status === 'resolving'
              ? phaseOf(d)
              : 'Queued'
  return (
    <div className="kryo-radius grid grid-cols-[150px_1fr_auto] items-center gap-4 border border-border bg-card p-3">
      <Art adult={d.meta.nsfw} src={capsulesFor(d.meta)[0]} fallback={capsulesFor(d.meta).slice(1)} title={d.meta.title} where="downloads" className="kryo-radius aspect-[460/215] w-full object-cover" />
      <div className="grid min-w-0 gap-1.5">
        <span className="flex min-w-0 items-center gap-2">
          <b className="truncate text-sm text-foreground">{d.meta.title}</b>
          <BranchTag branch={d.meta.steamBranch} title={d.meta.title} />
        </span>
        <span className="flex items-center gap-2 text-[10px] uppercase tracking-wider text-muted-foreground">
          <StatusGlyph d={d} className="size-2.5" />
          {status}
          {d.mirror ? ` · ${d.mirror.host}` : ''}
          {d.release ? ` · build ${d.release}` : ''}
          {d.total ? ` · ${formatBytes(d.status === 'installed' ? d.total : d.received)}${d.status === 'installed' ? '' : ` of ${formatBytes(d.total)}`}` : ''}
        </span>
        {d.status === 'paused' || d.status === 'queued' ? <AsciiBar fraction={pct} cells={30} /> : null}
        {failed ?? d.error ? <span role={failed ? 'alert' : undefined} className="text-xs text-destructive">{failed ?? d.error}</span> : null}
        {d.error && isSupporterLocked(d.error) && d.slug ? <SupporterLocked d={d} slug={d.slug} /> : null}
        {d.error && d.status === 'failed' && /^Not enough space/.test(d.error) ? <ShortOnSpace d={d} /> : null}
        {d.warning ? (
          <span className="flex flex-wrap items-center gap-x-2 text-xs leading-relaxed text-warning">
            {d.warning}
            {d.gameId ? (
              <Button size="sm" onClick={() => onRepair(d.gameId!)}>
                Repair game
              </Button>
            ) : null}
          </span>
        ) : null}
      </div>
      <div className="flex items-center gap-2">
        {queue && !(queue.first && queue.last) ? (
          <div className="flex items-center gap-1">
            {!queue.first ? (
              <IconButton label="Download now" onClick={() => run('now', () => api.move(d.id, 'now'))} disabled={Boolean(pending)}>
                {pending === 'now' ? <Busy className="size-3.5" /> : <ArrowUpToLine className="size-3.5" />}
              </IconButton>
            ) : null}
            <IconButton label="Move up" disabled={queue.first} onClick={() => attempt(() => api.move(d.id, 'up'))}>
              <ChevronUp className="size-3.5" />
            </IconButton>
            <IconButton label="Move down" disabled={queue.last} onClick={() => attempt(() => api.move(d.id, 'down'))}>
              <ChevronDown className="size-3.5" />
            </IconButton>
          </div>
        ) : null}
        {d.status === 'installed' && d.installDir ? (
          <IconButton label="Open folder" onClick={() => void library.openFolder(d.installDir!)}>
            <FolderOpen className="size-3.5" />
          </IconButton>
        ) : null}
        {d.status === 'installed' && d.gameId ? (
          <Button variant="primary" size="sm" onClick={() => onOpenGame(d.gameId!)}>
            <Play className="size-3 fill-current" />
            Play
          </Button>
        ) : null}
        {/* The checksum failed and the archive is kept: the player may
            install it anyway. Never done for them - Retry stays the default. */}
        {d.status === 'failed' && d.hashMismatch ? (
          <Button size="sm" variant="outline" disabled={Boolean(pending)} onClick={() => run('anyway', () => api.installAnyway(d.id))}>
            {pending === 'anyway' ? <Busy className="size-3" /> : <ShieldAlert className="size-3" />}
            Install anyway
          </Button>
        ) : null}
        {d.status === 'paused' || d.status === 'failed' ? (
          <Button variant="primary" size="sm" disabled={Boolean(pending)} onClick={() => run('resume', () => api.resume(d.id))}>
            {pending === 'resume' ? <Busy className="size-3" /> : d.status === 'failed' ? <RotateCw className="size-3" /> : <Download className="size-3" />}
            {d.status === 'failed' ? 'Retry' : 'Resume'}
          </Button>
        ) : null}
        {d.status === 'paused' || d.status === 'queued' || d.status === 'failed' ? (
          <Button size="sm" variant="ghost" disabled={Boolean(pending)} onClick={() => run('cancel', () => api.cancel(d.id))}>
            {pending === 'cancel' ? <Busy className="size-3" /> : null}
            {pending === 'cancel' ? 'Cancelling' : 'Cancel'}
          </Button>
        ) : null}
        {d.status === 'installed' || d.status === 'canceled' ? (
          <IconButton label="Clear from list" onClick={() => attempt(() => api.remove(d.id))}>
            <X className="size-3.5" />
          </IconButton>
        ) : null}
      </div>
    </div>
  )
}

/**
 * Bytes a second a counter is moving at, smoothed over a few seconds. For
 * unpacking and checking, whose speed the app does not report itself. `key`
 * starts it over (a new download, or checking turning into unpacking).
 */
function useRate(value: number | null, key: string) {
  const [rate, setRate] = useState(0)
  const last = useRef<{ key: string; value: number; at: number } | null>(null)
  useEffect(() => {
    if (value === null) {
      last.current = null
      setRate(0)
      return
    }
    const now = performance.now()
    const prev = last.current
    if (!prev || prev.key !== key || value < prev.value) {
      last.current = { key, value, at: now }
      setRate(0)
      return
    }
    const dt = (now - prev.at) / 1000
    // Updates come every ~200 ms; measuring over at least a second keeps the
    // figure from jumping with each one.
    if (dt < 1) return
    const instant = (value - prev.value) / dt
    setRate((r) => (r ? r * 0.6 + instant * 0.4 : instant))
    last.current = { key, value, at: now }
  }, [value, key])
  return rate
}

/** The last minute or so of speed readings, for the graph. */
function useSpeedHistory(d: Dl) {
  const [samples, setSamples] = useState<number[]>([])
  const id = useRef(d.id)
  useEffect(() => {
    if (id.current !== d.id) {
      id.current = d.id
      setSamples([])
    }
    setSamples((s) => [...s.slice(-79), d.speed])
  }, [d.id, d.speed, d.received])
  return samples
}
