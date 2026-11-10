import { useEffect, useRef, useState } from 'react'
import { BookOpen, Lock, LockOpen, Plus, RotateCw, ShieldBan, X, Library } from '@/ui/pixel-icons'
import { AsciiBar, Button, Matrix, Modal } from '@/ui'
import { SceneMotion } from '@/ui/ascii/SceneMotion'
import { ADSWEEP } from '@/ui/ascii/motion'
import { settingsApi, useSettings } from '@/lib/settings'
import { logWarn } from '@/lib/log'
import { isTauri, mountStore, openExternal, setMainPlacer, STORE_HOME } from '@/lib/window'
import type { ServiceHealth } from '@/lib/health'
import { resolveUserInputToUrl } from '@/lib/browser'
import { cn } from '@/lib/utils'
import type { BrowserPageState } from '@/hooks/useBrowserPage'

/**
 * The slot the Store's native web view is laid over.
 *
 * The web view is created once, on first mount, and moved to wherever this
 * slot is whenever the window changes size. Showing and hiding it is the
 * shell's call, since the shell knows when a menu or dialog needs the space.
 * What is drawn here is only ever seen when the view is not: while it first
 * loads, and when a page fails.
 */
export function WebSlot({
  page,
  onRetry,
  offline = false,
  outage = null,
  onLibrary,
}: {
  page: BrowserPageState
  onRetry: () => void
  /** No connection: the Store says so, once, and points at what still works. */
  offline?: boolean
  /** Kryoto is out and its status page cannot stand in: this slot says so instead. */
  outage?: ServiceHealth | null
  onLibrary?: () => void
}) {
  const slot = useRef<HTMLDivElement | null>(null)
  const [mountError, setMountError] = useState<string | null>(null)

  useEffect(() => {
    const el = slot.current
    if (!el || !isTauri()) return
    const place = () => {
      const r = el.getBoundingClientRect()
      if (r.width < 2 || r.height < 2) return
      mountStore(page.url || STORE_HOME, { x: r.left, y: r.top, width: r.width, height: r.height }, true)
        .then(() => setMountError(null))
        .catch((e: unknown) => setMountError(String(e)))
    }
    place()
    setMainPlacer(place)
    const ro = new ResizeObserver(place)
    ro.observe(el)
    window.addEventListener('resize', place)
    // A new display scale (another monitor, a changed Windows setting) can
    // leave the slot's CSS size unchanged; the view still has to be re-placed.
    let dpr: MediaQueryList | null = null
    const watchScale = () => {
      dpr?.removeEventListener('change', onScale)
      dpr = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`)
      dpr.addEventListener('change', onScale)
    }
    const onScale = () => {
      place()
      watchScale()
    }
    watchScale()
    return () => {
      ro.disconnect()
      setMainPlacer(null)
      window.removeEventListener('resize', place)
      dpr?.removeEventListener('change', onScale)
    }
    // Placed once per mount; later addresses come from navigation.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const problem = mountError ?? page.error?.message ?? null
  // Once kryo.to has shown a page, this slot is only ever seen behind a
  // dialog - where a loading bar would read as "the Store is stuck".
  const [shown, setShown] = useState(false)
  useEffect(() => {
    if (!page.loading) setShown(true)
  }, [page.loading])
  return (
    <div ref={slot} className="absolute inset-0 grid place-content-center justify-items-center gap-4 bg-background text-center">
      {offline ? (
        <>
          <Matrix state="connect" className="size-6 text-muted-foreground" />
          <p className="text-xs uppercase tracking-[0.25em] text-primary">The Store could not load</p>
          <p className="max-w-sm text-xs leading-relaxed text-muted-foreground">
            This PC is offline. Your library, installed games and settings all still work, and the Store comes back by itself
            when the connection does.
          </p>
          {onLibrary ? (
            <Button variant="primary" onClick={onLibrary}>
              Go to your library
            </Button>
          ) : null}
        </>
      ) : outage ? (
        <>
          <Matrix state="connect" className="size-6 text-muted-foreground" />
          <p className="text-xs uppercase tracking-[0.25em] text-primary">{outage.state === 'down' ? 'Kryoto is down' : 'Kryoto is not answering'}</p>
          <p className="max-w-sm text-xs leading-relaxed text-muted-foreground">
            Your library, installed games and settings all still work, and the Store comes back by itself, on the page you were on.
          </p>
          <div className="flex flex-wrap justify-center gap-2">
            {onLibrary ? (
              <Button variant="primary" onClick={onLibrary}>
                Go to your library
              </Button>
            ) : null}
            {outage.statusUp ? <Button onClick={() => openExternal(outage.statusUrl)}>Status page</Button> : null}
          </div>
        </>
      ) : problem ? (
        <>
          <p className="text-xs uppercase tracking-[0.25em] text-primary">The store did not open</p>
          <p className="max-w-md text-xs text-muted-foreground">{problem}</p>
          <Button variant="primary" onClick={onRetry}>
            Try again
          </Button>
        </>
      ) : page.retrying ? (
        <>
          <Matrix state="busy" className="size-6 text-muted-foreground" />
          <p role="status" className="text-[10px] uppercase tracking-[0.25em] text-muted-foreground">
            Trying again
          </p>
        </>
      ) : shown ? null : isTauri() ? (
        <>
          <AsciiBar fraction={null} cells={24} />
          <p className="text-[10px] uppercase tracking-[0.25em] text-muted-foreground">Opening kryo.to</p>
        </>
      ) : (
        <>
          <p className="text-xs uppercase tracking-[0.25em] text-primary">kryo.to opens here in the app</p>
          <p className="max-w-md text-xs text-muted-foreground">
            The browser preview cannot show it: the site refuses to be framed. Everything else works on sample data.
          </p>
        </>
      )}
    </div>
  )
}

/**
 * A thin line along the pill's foot while a page loads. The web view only
 * says "started" and "finished", so it eases towards 90% and then completes;
 * it animates here, so a load redraws this line and not the whole client.
 */
function LoadLine({ loading }: { loading: boolean }) {
  const [progress, setProgress] = useState<number | null>(null)
  useEffect(() => {
    if (!loading) {
      setProgress((p) => (p === null ? null : 100))
      const t = window.setTimeout(() => setProgress(null), 250)
      return () => window.clearTimeout(t)
    }
    setProgress(8)
    const t = window.setInterval(() => setProgress((p) => Math.min(90, (p ?? 8) + Math.max(0.5, (90 - (p ?? 8)) * 0.08))), 120)
    return () => window.clearInterval(t)
  }, [loading])
  if (progress === null) return null
  return (
    <span
      className="absolute bottom-0 left-0 h-px bg-foreground transition-[width,opacity] duration-200"
      style={{ width: `${progress}%`, opacity: progress >= 100 ? 0 : 1 }}
    />
  )
}

/**
 * The Store's address, as a pill in the nav row: secure or not, the address
 * (editable - a search falls back to a search engine), reload or stop, a thin
 * load line, the ad blocker's switch, and "Add to Library" on a kryo.to game
 * page.
 */
export function UrlPill({
  page,
  actions,
  onLibrary,
  inLibrary,
  onAdblock,
  compact = false,
  narrow = false,
}: {
  page: BrowserPageState
  actions: { navigate: (url: string) => void; reload: () => void; stop: () => void }
  onLibrary: (() => void) | null
  inLibrary: boolean
  /** Opens `AdblockDialog`. */
  onAdblock: () => void
  /** The row has folded all it can: Add to library is its icon alone. */
  compact?: boolean
  /** Short of room: the address may be narrower than it likes. */
  narrow?: boolean
}) {
  const [draft, setDraft] = useState<string | null>(null)
  const blocking = useSettings()?.blockAds ?? true
  const secure = page.url.startsWith('https://')
  const shown = draft ?? page.url.replace(/^https:\/\//, '')
  return (
    <>
      <div className={cn('kryo-pill relative flex h-9 grow items-center gap-2 overflow-hidden border border-border bg-card pl-3 pr-1', narrow ? 'min-w-40' : 'min-w-56')}>
        {secure ? <Lock className="size-3 shrink-0 text-muted-foreground" /> : <LockOpen className="size-3 shrink-0 text-warning" />}
        <input
          aria-label="Address"
          spellCheck={false}
          value={shown}
          onFocus={(e) => {
            setDraft(page.url)
            requestAnimationFrame(() => e.target.select())
          }}
          onBlur={() => setDraft(null)}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && draft) {
              actions.navigate(resolveUserInputToUrl(draft))
              ;(e.target as HTMLInputElement).blur()
            } else if (e.key === 'Escape') (e.target as HTMLInputElement).blur()
          }}
          className="kryo-square min-w-0 grow truncate bg-transparent text-[11px] tracking-wide text-muted-foreground outline-none focus:text-foreground"
        />
        <button
          type="button"
          aria-label={blocking ? 'Ad blocker: on' : 'Ad blocker: off'}
          title={blocking ? 'Ad blocker: on' : 'Ad blocker: off'}
          onClick={onAdblock}
          className={`kryo-pill grid size-7 shrink-0 place-items-center transition-colors hover:bg-secondary hover:text-foreground ${blocking ? 'text-primary' : 'text-muted-foreground'}`}
        >
          <ShieldBan className="size-3.5" />
        </button>
        <button
          type="button"
          aria-label={page.loading ? 'Stop' : 'Reload'}
          onClick={page.loading ? actions.stop : actions.reload}
          className="kryo-pill grid size-7 shrink-0 place-items-center text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
        >
          {page.loading ? <X className="size-3.5" /> : <RotateCw className="size-3.5" />}
        </button>
        <LoadLine loading={page.loading} />
      </div>
      {onLibrary ? (
        <Button variant={inLibrary ? 'outline' : 'primary'} className="shrink-0" onClick={onLibrary} title={compact ? (inLibrary ? 'In your library' : 'Add to library') : undefined}>
          {inLibrary ? <Library className="size-3.5" /> : <Plus className="size-3.5" />}
          {compact ? null : inLibrary ? 'In your library' : 'Add to library'}
        </Button>
      ) : null}
    </>
  )
}

/**
 * The built-in ad blocker, from its shield in the address pill: what it is, and
 * the switch (Settings, General has the same one). It filters requests natively
 * (src-tauri adblock.rs); kryo.to is never filtered, and its own ads open in the
 * player's browser.
 */
export function AdblockDialog({ onClose, onLearnMore }: { onClose: () => void; onLearnMore: () => void }) {
  const on = useSettings()?.blockAds ?? true
  const [saving, setSaving] = useState(false)
  const flip = () => {
    setSaving(true)
    void settingsApi
      .get()
      .then((latest) => settingsApi.save({ ...latest, blockAds: !latest.blockAds }))
      .catch((e: unknown) => logWarn('adblock', `could not switch the ad blocker: ${String(e)}`))
      .finally(() => setSaving(false))
  }
  return (
    <Modal
      title="Ad blocker"
      onClose={onClose}
      footer={
        <>
          <Button variant="ghost" onClick={onLearnMore}>
            <BookOpen className="size-3" />
            Learn more
          </Button>
          <Button variant={on ? 'outline' : 'primary'} disabled={saving} onClick={flip}>
            {on ? 'Turn it off' : 'Turn it on'}
          </Button>
        </>
      }
    >
      <div className="grid gap-4">
        <SceneMotion scene={ADSWEEP} className="h-[72px] text-foreground" label="Ads swept off a web page" />
        <p className="text-xs leading-relaxed text-foreground">
          Kryoto Desktop has a built-in adblocker, so you can browse the web ad-free. Want to turn it off for some reason?
        </p>
        <p className="text-[11px] leading-relaxed text-muted-foreground">
          It is {on ? 'on' : 'off'}. kryo.to itself is never filtered: Kryoto&apos;s own ads keep it free, and they open in your own browser.
        </p>
      </div>
    </Modal>
  )
}
