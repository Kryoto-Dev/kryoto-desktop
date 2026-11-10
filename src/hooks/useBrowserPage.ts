import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { isOnline } from '@/lib/online'
import { listen } from '@tauri-apps/api/event'
import { BROWSER_HOME } from '@/lib/browser'
import type { WebNav } from '@/lib/history'
import { browserNavigate, catalogUrl, controlBrowser, isTauri } from '@/lib/window'

/**
 * The Store's web view, as the shell sees it: where it is, whether it is
 * loading, and what went wrong.
 *
 * Kept small on purpose: this state lives at the top of the app, so every
 * change here draws the whole client again. The load bar animates inside
 * the address pill, not here, and nothing polls.
 */

export type BrowserErrorKind = 'blocked' | 'failed' | 'offline'

export interface BrowserError {
  kind: BrowserErrorKind
  message: string
  url: string
}

export interface BrowserPageState {
  url: string
  title: string
  loading: boolean
  /** The page's own previous and next addresses (its tab history). */
  back: string | null
  forward: string | null
  error: BrowserError | null
  /** Try again was pressed and the page has not answered yet. */
  retrying: boolean
  /** Transient shell notice (blocked link, slow load). */
  notice: string | null
}

/** Payload emitted by the native shell (`browser-state`). */
interface BrowserStatePayload {
  url?: string
  title?: string
  loading?: boolean
  nav?: WebNav | null
  back?: string | null
  forward?: string | null
  error?: string | null
}

interface BrowserErrorPayload {
  url?: string
  reason?: string
}

export type NavListener = (url: string, nav: WebNav) => void

const SLOW_LOAD_MS = 12000

const INITIAL_STATE: BrowserPageState = {
  url: BROWSER_HOME,
  title: '',
  loading: isTauri(),
  back: null,
  forward: null,
  error: null,
  retrying: false,
  notice: null,
}

function fatalMessage(reason: string, url: string): string {
  if (/offline|network|internet/i.test(reason)) return 'You appear to be offline.'
  if (/blocked|scheme/i.test(reason)) return 'This address was blocked.'
  if (/ssl|cert|tls/i.test(reason)) return 'The secure connection could not be verified.'
  if (url) return `Could not reach ${url}.`
  return 'The page could not be loaded.'
}

export function useBrowserPage(homeUrl: string = BROWSER_HOME) {
  const [state, setState] = useState<BrowserPageState>(INITIAL_STATE)
  const noticeTimer = useRef<number | null>(null)
  const stateRef = useRef(state)
  stateRef.current = state
  const navListeners = useRef(new Set<NavListener>())

  const flashNotice = useCallback((notice: string) => {
    setState((current) => ({ ...current, notice }))
    if (noticeTimer.current) window.clearTimeout(noticeTimer.current)
    noticeTimer.current = window.setTimeout(() => setState((current) => ({ ...current, notice: null })), 6000)
  }, [])

  const applyPayload = useCallback((p: BrowserStatePayload) => {
    setState((current) => {
      const next: BrowserPageState = {
        ...current,
        url: p.url || current.url,
        // "Started" events carry no title; keep the old one mid-navigation.
        title: p.title || current.title,
        loading: p.loading ?? current.loading,
        back: p.nav ? (p.back ?? null) : current.back,
        forward: p.nav ? (p.forward ?? null) : current.forward,
        error: p.loading ? null : current.error,
        retrying: p.loading === false ? false : current.retrying,
      }
      const same =
        next.url === current.url &&
        next.title === current.title &&
        next.loading === current.loading &&
        next.back === current.back &&
        next.forward === current.forward &&
        next.error === current.error &&
        next.retrying === current.retrying
      return same ? current : next
    })
    if (p.nav && p.url) navListeners.current.forEach((l) => l(p.url!, p.nav!))
  }, [])

  /* ── Slow-load hint ── */
  useEffect(() => {
    if (!isTauri() || !state.loading) return
    const t = window.setTimeout(() => flashNotice(`Still loading ${stateRef.current.url}. Esc stops it.`), SLOW_LOAD_MS)
    return () => window.clearTimeout(t)
  }, [state.loading, flashNotice])

  useEffect(() => {
    if (!isTauri()) return
    let cancelled = false
    const stops: Array<() => void> = []
    const keep = (p: Promise<() => void>) => void p.then((stop) => (cancelled ? stop() : stops.push(stop)))
    keep(listen<BrowserStatePayload>('browser-state', (e) => applyPayload(e.payload)))
    keep(
      listen<BrowserErrorPayload>('browser-error', (event) => {
        const url = event.payload.url ?? stateRef.current.url
        const reason = event.payload.reason ?? 'Navigation was blocked.'
        if (/^blocked|^unsupported/i.test(reason)) return flashNotice(`${reason} ${url}`.trim())
        setState((current) => ({ ...current, loading: false, retrying: false, error: { kind: 'failed', message: fatalMessage(reason, url), url } }))
      }),
    )
    // Where the view already is (the window was reloaded).
    void catalogUrl()
      .then((url) => !cancelled && url && setState((c) => (c.url === url ? c : { ...c, url })))
      .catch(() => {})
    return () => {
      cancelled = true
      stops.forEach((stop) => stop())
    }
  }, [applyPayload, flashNotice])

  useEffect(() => () => void (noticeTimer.current && window.clearTimeout(noticeTimer.current)), [])

  const navigate = useCallback(
    (url: string) => {
      setState((current) => ({ ...current, loading: true, error: null, notice: null }))
      if (!isTauri()) return
      if (!isOnline()) {
        setState((current) => ({ ...current, loading: false, retrying: false, error: { kind: 'offline', message: 'You appear to be offline.', url } }))
        return
      }
      void browserNavigate(url).catch((error: unknown) => {
        const message = typeof error === 'string' ? error : error instanceof Error ? error.message : 'Navigation failed.'
        if (/offline/i.test(message)) {
          setState((current) => ({ ...current, loading: false, retrying: false, error: { kind: 'offline', message, url } }))
        } else if (/block|invalid|unsupported/i.test(message)) {
          setState((current) => ({ ...current, loading: false, retrying: false }))
          flashNotice(message)
        } else {
          setState((current) => ({ ...current, loading: false, retrying: false, error: { kind: 'failed', message: fatalMessage(message, url), url } }))
        }
      })
    },
    [flashNotice],
  )

  const actions = useMemo(
    () => ({
      navigate,
      /** The page's own back and forward: instant, from its cache. */
      back: () => void controlBrowser('back'),
      forward: () => void controlBrowser('forward'),
      reload: () => {
        setState((current) => ({ ...current, loading: true, error: null }))
        void controlBrowser('reload')
      },
      stop: () => {
        setState((current) => ({ ...current, loading: false }))
        void controlBrowser('stop')
      },
      retry: () => {
        setState((current) => ({ ...current, retrying: true }))
        navigate(stateRef.current.error?.url || stateRef.current.url || homeUrl)
      },
      /** Hear each change of address the page reports, with how it happened. */
      onNav: (fn: NavListener) => {
        navListeners.current.add(fn)
        return () => void navListeners.current.delete(fn)
      },
    }),
    [navigate, homeUrl],
  )

  return { state, actions }
}
