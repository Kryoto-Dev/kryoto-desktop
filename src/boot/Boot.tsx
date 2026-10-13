import { useEffect, useRef, useState, type ReactNode } from 'react'
import { artSrc } from '@/lib/art'
import { ArrowLeft, Minus, RotateCw, X } from 'lucide-react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { AsciiBar, Button } from '@/ui'
import { AsciiArt } from '@/ui/ascii/AsciiArt'
import { WORDMARK } from '@/ui/ascii/cells'
import { SplashScene } from '@/boot/SplashScene'
import { DevEndpointNotice } from '@/ui/DevEndpointNotice'
import { isTauri, exitApp, mountStore, navigateCatalog, setStoreVisible, signOut, STORE_HOME } from '@/lib/window'
import type { Account } from '@/hooks/useAccount'
import type { BrowserPageState } from '@/hooks/useBrowserPage'
import { cn } from '@/lib/utils'

/**
 * Before the client: the splash box, then the welcome screen with sign-in.
 *
 * The splash has nothing to press: it moves on by itself (see App). The
 * welcome screen offers a kryo.to account or a guest visit. Sign-in is kryo.to's own page, shown inside the
 * welcome screen in the same web view the Store uses: every way the site
 * signs in (a phone scan, Discord, email) works, and the session it makes is
 * the one the Store then has.
 */

/** The window's own two buttons, for the frameless boxes. */
function BoxControls() {
  if (!isTauri()) return null
  const btn =
    'no-drag kryo-pill grid size-7 place-items-center text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground'
  return (
    <div className="absolute right-2.5 top-2.5 z-20 flex gap-1">
      <button type="button" aria-label="Minimize" title="Minimize" className={btn} onClick={() => void getCurrentWindow().minimize()}>
        <Minus className="size-3.5" />
      </button>
      <button type="button" aria-label="Quit" title="Quit" className={btn} onClick={() => void exitApp()}>
        <X className="size-3.5" />
      </button>
    </div>
  )
}

/** The box is the whole window; Windows rounds it and draws its shadow. */
function Box({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <main className={cn('drag kryo-square relative h-full overflow-hidden bg-background text-foreground', className)}>
      <BoxControls />
      <DevEndpointNotice className="absolute left-1/2 top-3 z-20 -translate-x-1/2" />
      {children}
    </main>
  )
}

/* ── Splash ─────────────────────────────────────────────── */

export function Splash({ onRevealed }: { onRevealed: () => void }) {
  // The loading bar and the name come in once the mark has lit up.
  const [lit, setLit] = useState(false)
  return (
    <Box className="bg-black">
      <SplashScene
        onRevealed={() => {
          setLit(true)
          onRevealed()
        }}
      />
      {lit ? (
        <>
          <div className="kryo-in pointer-events-none absolute inset-x-0 bottom-20 grid place-items-center">
            <AsciiBar fraction={null} cells={16} showPct={false} className="text-foreground/60" />
          </div>
          {/* The maker's mark on the line where the app names itself, as the
              site's footer does: which build of Kryoto this is. */}
          <p
            className="kryo-in pointer-events-none absolute inset-x-0 bottom-5 flex items-baseline justify-center gap-2 text-center text-[10px] uppercase tracking-[0.3em] text-muted-foreground"
            style={{ animationDelay: '120ms' }}
          >
            <span aria-hidden className="kryo-ascii-art font-bold tracking-tight opacity-60">
              K//
            </span>
            Kryoto Desktop {__APP_VERSION__}
          </p>
        </>
      ) : null}
    </Box>
  )
}

/* ── Welcome and sign-in ────────────────────────────────── */

export function Welcome({
  account,
  page,
  onContinue,
  onGuest,
  onRetry,
}: {
  account: Account | null | undefined
  page: BrowserPageState
  onContinue: () => void
  onGuest: () => void
  onRetry: () => void
}) {
  const [signingIn, setSigningIn] = useState(false)
  const [switching, setSwitching] = useState(false)

  // Signing in on the page finishes the job: the account arriving is the cue.
  useEffect(() => {
    if (signingIn && account) onContinue()
  }, [signingIn, account, onContinue])
  // "Use another account" signs out first, then opens sign-in.
  useEffect(() => {
    if (switching && account === null) {
      setSwitching(false)
      setSigningIn(true)
    }
  }, [switching, account])

  const name = account ? account.displayName || account.username : ''

  return (
    <Box className="bg-black">
      <img
        src="/brand/og-backdrop.jpg"
        alt=""
        draggable={false}
        className="pointer-events-none absolute inset-0 size-full scale-105 object-cover opacity-45"
      />
      <div className="pointer-events-none absolute inset-0 bg-[radial-gradient(ellipse_at_center,transparent_0%,rgba(0,0,0,0.55)_70%)]" />
      <div className="pointer-events-none absolute inset-x-0 bottom-0 h-1/2 bg-gradient-to-t from-background to-transparent" />

      <div
        className={cn(
          'relative z-10 grid h-full justify-items-center transition-[padding] duration-500',
          signingIn ? 'grid-rows-[auto_1fr] gap-5 px-8 pb-8 pt-9' : 'place-content-center gap-10',
        )}
      >
        <div className="grid justify-items-center gap-3">
          <AsciiArt
            lines={WORDMARK}
            mode={signingIn ? 'still' : 'reveal'}
            revealMs={1300}
            className={cn('drop-shadow-[0_4px_24px_rgba(0,0,0,0.8)] transition-all duration-500', signingIn ? 'h-9' : 'h-20')}
            label="kryo.to"
          />
          {!signingIn ? <p className="text-[11px] uppercase tracking-[0.5em] text-foreground/70">Desktop</p> : null}
        </div>

        {signingIn ? (
          <SignInPanel
            page={page}
            onBack={() => setSigningIn(false)}
            onRetry={onRetry}
          />
        ) : (
          <div className="no-drag kryo-in grid w-80 justify-items-center gap-3" style={{ animationDelay: '900ms' }}>
            {account === undefined || switching ? (
              <>
                <AsciiBar fraction={null} cells={18} showPct={false} className="text-foreground/70" />
                <p className="text-[10px] uppercase tracking-[0.25em] text-muted-foreground">
                  {page.error ? 'kryo.to is not answering' : 'Checking your account'}
                </p>
                {page.error ? (
                  <>
                    <Button size="sm" onClick={onRetry}>
                      <RotateCw className="size-3" />
                      Try again
                    </Button>
                    <GuestButton onGuest={onGuest} />
                  </>
                ) : null}
              </>
            ) : account ? (
              <>
                <Button variant="primary" size="lg" className="w-full gap-3" onClick={onContinue}>
                  {account.avatarUrl ? <img src={artSrc(account.avatarUrl) ?? undefined} alt="" className="kryo-pill size-6 object-cover" /> : null}
                  <span className="truncate">Continue as {name}</span>
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    setSwitching(true)
                    void signOut().catch(() => setSwitching(false))
                  }}
                >
                  Use another account
                </Button>
              </>
            ) : (
              <>
                <Button variant="primary" size="lg" className="w-full" onClick={() => setSigningIn(true)}>
                  Sign in with kryo.to
                </Button>
                <GuestButton onGuest={onGuest} />
              </>
            )}
          </div>
        )}
      </div>
      <p className="pointer-events-none absolute bottom-3 left-4 z-10 text-[10px] uppercase tracking-[0.3em] text-muted-foreground/70">
        {__APP_VERSION__}
      </p>
    </Box>
  )
}

/** Into the client without an account. */
function GuestButton({ onGuest }: { onGuest: () => void }) {
  return (
    <>
      <Button variant="ghost" size="sm" onClick={onGuest}>
        Continue as guest
      </Button>
      <p className="max-w-64 text-center text-[10px] leading-relaxed text-muted-foreground">
        Your library works without an account. Sign in any time from the top bar.
      </p>
    </>
  )
}

/** kryo.to's sign-in page in a box, in the Store's web view. */
function SignInPanel({ page, onBack, onRetry }: { page: BrowserPageState; onBack: () => void; onRetry: () => void }) {
  const slot = useRef<HTMLDivElement | null>(null)
  const [opened, setOpened] = useState(false)

  useEffect(() => {
    const el = slot.current
    if (!el || !isTauri()) return
    let first = true
    const place = () => {
      const r = el.getBoundingClientRect()
      if (r.width < 2 || r.height < 2) return
      void mountStore(`${STORE_HOME}login?next=/`, { x: r.left, y: r.top, width: r.width, height: r.height }, true).then(() => {
        if (!first) return
        first = false
        void setStoreVisible(true)
        void navigateCatalog('/login?next=/').finally(() => setOpened(true))
      })
    }
    // After the panel's grow-in has settled, so the page lands where the box is.
    const t = window.setTimeout(place, 320)
    const ro = new ResizeObserver(() => !first && place())
    ro.observe(el)
    return () => {
      window.clearTimeout(t)
      ro.disconnect()
      void setStoreVisible(false)
    }
  }, [])

  const ready = opened && !page.loading
  return (
    <section className="no-drag kryo-pop kryo-radius grid min-h-0 w-[460px] grid-rows-[auto_1fr] overflow-hidden border border-border bg-background shadow-2xl shadow-black/70">
      <header className="flex items-center gap-2 border-b border-border px-2 py-2">
        <button
          type="button"
          aria-label="Back"
          title="Back"
          onClick={onBack}
          className="kryo-pill grid size-7 place-items-center text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
        >
          <ArrowLeft className="size-3.5" />
        </button>
        <span className="grow text-[10px] uppercase tracking-[0.25em] text-foreground/80">Sign in to kryo.to</span>
        <button
          type="button"
          aria-label="Reload"
          title="Reload"
          onClick={onRetry}
          className="kryo-pill grid size-7 place-items-center text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
        >
          <RotateCw className="size-3.5" />
        </button>
      </header>
      <div ref={slot} className="relative grid min-h-0 place-content-center justify-items-center gap-3 bg-background">
        {page.error ? (
          <>
            <p className="text-xs uppercase tracking-[0.25em] text-primary">kryo.to did not open</p>
            <p className="max-w-xs text-center text-xs text-muted-foreground">{page.error.message}</p>
            <Button variant="primary" onClick={onRetry}>
              Try again
            </Button>
          </>
        ) : !ready ? (
          <AsciiBar fraction={null} cells={16} showPct={false} className="text-muted-foreground" />
        ) : null}
      </div>
    </section>
  )
}
