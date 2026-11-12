import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import {
  ArrowUpRight,
  Bell as BellIcon,
  Bookmark,
  ChartColumn,
  Cloud,
  Download as DownloadIcon,
  FlaskConical,
  FolderHeart,
  FolderInput,
  FolderPlus,
  Eye,
  EyeOff,
  FolderOpen,
  Globe,
  House,
  LayoutGrid,
  LogOut,
  MessageCircle,
  MessageSquarePlus,
  Newspaper,
  PenLine,
  Play,
  Plus,
  Settings as SettingsIcon,
  Sparkles,
  Square,
  Trash2,
  User,
  Gamepad2,
  Layers,
  Monitor,
  ScrollText,
  Wrench,
  Wine,
} from '@/ui/pixel-icons'
import { ACTION_FAILED_EVENT, Button, Check, ContextMenu, Modal, type MenuEntry } from '@/ui'
import { KryoAscii } from '@/ui/ascii/KryoAscii'
import { EmptyState } from '@/ui/EmptyState'
import { SHELF } from '@/ui/ascii/scenes'
import { FriendsPage } from '@/friends/FriendsPage'
import { branchLabel, copyLabel, titleWithBranch } from '@/library/BranchTag'
import { CallOverlay } from '@/friends/CallOverlay'
import { friendName, useFriends } from '@/hooks/useFriends'
import { FOLD, TitleBar, useRowFold } from '@/shell/TitleBar'
import { compactKryos } from '@/ui/KryosCoin'
import { NavBar, type NavTabSpec, type TopTab } from '@/shell/NavBar'
import { AdblockDialog, UrlPill, WebSlot } from '@/shell/WebView'
import { BottomBar } from '@/shell/BottomBar'
import { ServiceNotice } from '@/shell/ServiceNotice'
import { UpdateCheck, UpdatePrompt } from '@/shell/UpdatePrompt'
import { CloudSavesPrompt } from '@/shell/CloudSavesPrompt'
import { FeatureNotice, useNewSettings, type WhatsNewPlace } from '@/ui/FeatureNotice'
import { WriteupNudge } from '@/ui/WriteupNudge'
import { ReleaseIntro } from '@/ui/ReleaseIntro'
import { SETUP_AGAIN, SETUP_NEWS, SetupDialog, SetupPrompt } from '@/setup/FirstStart'
import { FoundGamesAfterUpdate } from '@/library/FoundGames'
import { InstallFolderPrompt } from '@/downloads/InstallFolderPrompt'
import { CollectionInstall } from '@/library/CollectionInstall'
import { usePlaytimeGate } from '@/shell/PlaytimePrompt'
import { Toasts, useToasts, type Toast } from '@/shell/Toasts'
import { Sidebar } from '@/library/Sidebar'
import { LibraryHome } from '@/library/LibraryHome'
import { GamePage } from '@/library/GamePage'
import { CatalogGamePage } from '@/library/CatalogGamePage'
import { LaunchChooser } from '@/library/LaunchChooser'
import { GameProperties } from '@/library/GameProperties'
import { AddGameDialog } from '@/library/AddGameDialog'
import { Bridge } from '@/library/Bridge'
import { carry, launcherName, merges } from '@/lib/launchers'
import { discordGames, useDiscord } from '@/lib/discord'
import { SITE_ORIGIN } from '@/lib/endpoint'
import { DownloadsPage } from '@/downloads/DownloadsPage'
import { isWebSection, SettingsPage, type SettingsSection } from '@/settings/SettingsPage'
import { useLibrary } from '@/hooks/useLibrary'
import type { Account } from '@/hooks/useAccount'
import type { useInbox } from '@/hooks/useInbox'
import { requestPad, useControllerFeature, usePadStore } from '@/hooks/usePads'
import { usePadNavigation } from '@/lib/pad-nav'
import { BigPicture } from '@/bigpicture/BigPicture'
import { PadHints } from '@/controller/PadHints'
import { familyOf, padGames } from '@/lib/pad'
import { setSavedStatus, setSavedStatusMany, useHiddenSlugs, useSaved, STATUSES, STATUS_LABEL, type SavedEntry } from '@/hooks/useSaved'
import { collections as collectionsApi, useCollections } from '@/lib/collections'
import type { useBrowserPage } from '@/hooks/useBrowserPage'
import { downloads, useDownloads, type Download } from '@/lib/downloads'
import { useAutoUpdates } from '@/library/useAutoUpdates'
import { useOnline } from '@/lib/online'
import { isOutage, pokeHealth, useServiceHealth } from '@/lib/health'
import { holdPrompts, useDialogCount } from '@/lib/prompts'
import { settingsApi, useSettings } from '@/lib/settings'
import { parseSettingsAddress, revealSetting, settingsHome } from '@/lib/settings-links'
import * as nav from '@/lib/history'
import type { View } from '@/lib/history'
import { call, errorText, on } from '@/lib/bridge'
import { startActiveBeacon } from '@/lib/chat'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { logError, logWarn } from '@/lib/log'
import { flushPlays, isExpectedReportError, reportPlay } from '@/lib/play-reports'
import { syncPlaytime } from '@/lib/playtime-sync'
import { DISCORD_URL, REDDIT_URL, SOURCE_URL, YOUTUBE_URL } from '@/lib/community'
import { entryIsVr, entryLabel, folderName, gameLogs, isWindowsHost, knowsBuild, library, plainPath, playTarget, type LibraryGame } from '@/lib/library'
import {
  exitApp,
  isTauri,
  OPEN_BIG_PICTURE_EVENT,
  OPEN_GAME_EVENT,
  OPEN_LIBRARY_EVENT,
  OPEN_SETTINGS_EVENT,
  OPEN_SETUP_EVENT,
  OPEN_STORE_EVENT,
  openExternal,
  setStoreVisible,
  signOut,
  toggleFullscreen,
} from '@/lib/window'

type Browser = ReturnType<typeof useBrowserPage>

type Overlay =
  | { kind: 'add'; slug: string | null }
  | { kind: 'choose'; id: string }
  | { kind: 'props'; id: string; tab?: 'versions' | 'logs' | 'repair' | 'controller' }
  | { kind: 'uninstall'; id: string }
  | { kind: 'about' }
  /** The built-in ad blocker, from its shield in the address pill. */
  | { kind: 'adblock' }
  /** Games in from, and out to, other launchers (library/Bridge.tsx). */
  | { kind: 'bridge' }
  /** Every game a launcher keeps, off the Library. */
  | { kind: 'unlist'; launcher: string }
  /** The setup again, from Settings, or What's new alone, from the account menu. */
  | { kind: 'setup' }
  | { kind: 'news' }

/** Which top tab a kryo.to address lights, the way Steam lights its nav. */
/** A Store page's path for a report: never its query, which can carry a code. */
function safePath(url: string): string {
  try {
    return new URL(url).pathname
  } catch {
    return ''
  }
}

function tabForUrl(url: string, catalogEndpoint?: string): TopTab {
  let path = '/'
  try {
    const u = new URL(url)
    const endpoint = catalogEndpoint?.trim()
    const isCatalog = endpoint
      ? u.origin === new URL(endpoint).origin
      : /(^|\.)kryo\.to$/.test(u.hostname)
    if (!isCatalog) return 'store'
    path = u.pathname
  } catch {
    return 'store'
  }
  if (/^\/(user|settings|account|notifications|library|login|signup|register)(\/|$)/.test(path)) return 'profile'
  if (/^\/(blog|collections|testers|stats|community|rolls)(\/|$)/.test(path)) return 'community'
  return 'store'
}

/** One of kryo.to's own pages, not a site opened in the Store or the status page. */
function onKryotoPage(url: string, catalogEndpoint?: string): boolean {
  try {
    const page = new URL(url)
    const endpoint = catalogEndpoint?.trim()
    if (endpoint) return page.origin === new URL(endpoint).origin
    return page.hostname !== 'status.kryo.to' && /(^|\.)kryo\.to$/.test(page.hostname)
  } catch {
    return false
  }
}

function slugOnPage(url: string, catalogEndpoint?: string): string | null {
  try {
    const page = new URL(url)
    const endpoint = catalogEndpoint?.trim()
    const isCatalog = endpoint
      ? page.origin === new URL(endpoint).origin
      : /(^|\.)kryo\.to$/.test(page.hostname)
    if (!isCatalog) return null
    const m = page.pathname.match(/^\/game\/([a-z0-9-]+)/i)
    return m?.[1] ? m[1].toLowerCase() : null
  } catch {
    return null
  }
}

export function Shell({ startPage, account, browser, inbox: inboxState }: { startPage: 'store' | 'library'; account: Account; browser: Browser; inbox: ReturnType<typeof useInbox> }) {
  const lib = useLibrary()
  const shownGames = useMemo(() => lib.games.filter((g) => !g.hidden), [lib.games])
  const { inbox, news, markAllRead, markRead } = inboxState
  const downloadState = useDownloads()
  const dl = downloadState.list
  const dlRef = useRef(dl)
  dlRef.current = dl
  const saved = useSaved()
  const ownLists = useCollections().collections.filter((c) => c.role === 'owner' || c.role === 'collaborator')
  const hiddenSlugs = useHiddenSlugs()
  const { state: page, actions: web } = browser
  const { toasts, push, dismiss } = useToasts()
  const guest = !!account.guest
  const leaveAccount = () => void signOut().catch((e: unknown) => push({ title: 'Could not sign out', body: errorText(e), gameId: null }))

  const settings = useSettings()
  const catalogBase = (settings?.catalogEndpoint?.trim() || SITE_ORIGIN).replace(/\/+$/, '')

  /* ── History: one pair of arrows for the whole client (lib/history.ts) ── */
  const [history, setHistory] = useState(() => nav.start(startPage === 'store' ? { kind: 'web', url: page.url } : { kind: 'home' }))
  const view: View = nav.current(history)
  const go = useCallback((next: View) => setHistory((h) => nav.go(h, next)), [])
  /** A kryo.to path (or full address) in the Store; none resumes the page it is on. */
  const openWeb = useCallback(
    (path?: string) => go({ kind: 'web', url: !path ? page.url : /^https?:/.test(path) ? path : `${catalogBase}${path}` }),
    [go, page.url, catalogBase],
  )
  const back = useCallback(() => setHistory((h) => nav.step(h, -1)), [])
  const forward = useCallback(() => setHistory((h) => nav.step(h, 1)), [])
  const canBack = history.index > 0
  const canForward = history.index < history.stack.length - 1

  // The Store's web view follows the entry being shown. Where the page's own
  // history already has it one step away, that step is taken (instant, from
  // its cache); anything else is loaded. `pending` is where it was sent, so
  // a redirect on the way replaces the entry instead of adding one.
  const pending = useRef<{ url: string; from: string; at: number; stale: string[] } | null>(null)
  // Kryoto in front: this account's other devices leave chat notifications to it.
  useEffect(() => (isTauri() ? startActiveBeacon(() => getCurrentWindow().isFocused().catch(() => false)) : undefined), [])
  useEffect(() => {
    if (view.kind !== 'web' || nav.sameUrl(view.url, page.url)) return
    const p = pending.current
    if (p && nav.sameUrl(p.url, view.url) && Date.now() - p.at < 15_000) return
    // Addresses this navigation replaced (the page it left, and any target
    // that was still on its way): their reports can still arrive, late.
    const before = pending.current && Date.now() - pending.current.at < 15_000 ? pending.current : null
    pending.current = { url: view.url, from: page.url, at: Date.now(), stale: [...(before?.stale ?? []), page.url, ...(before ? [before.url] : [])].slice(-8) }
    if (page.back && nav.sameUrl(page.back, view.url)) web.back()
    else if (page.forward && nav.sameUrl(page.forward, view.url)) web.forward()
    else web.navigate(view.url)
    // Only when the entry changes: the page moving on its own is its report's job.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [view])
  useEffect(
    () =>
      web.onNav((url, how) => {
        const p = pending.current
        // The page being left still reports itself for a moment after the Store
        // was sent elsewhere. That is not where the navigation ended up, and
        // taking it as such put the old page back in the history (the tab
        // jumped back and the new page seemed to take forever).
        if (p && Date.now() - p.at < 15_000 && !nav.sameUrl(url, p.url) && p.stale.some((u) => nav.sameUrl(u, url))) return
        pending.current = null
        setHistory((h) => nav.webMoved(h, url, how, !!p && Date.now() - p.at < 15_000))
      }),
    [web],
  )
  // The mouse's side buttons and Alt+arrows, anywhere in the client's own
  // chrome (inside the Store, the web view handles them itself).
  useEffect(() => {
    const onMouse = (e: MouseEvent) => {
      if (e.button === 3) back()
      else if (e.button === 4) forward()
      else return
      e.preventDefault()
    }
    const onKey = (e: KeyboardEvent) => {
      if (!e.altKey) return
      if (e.key === 'ArrowLeft') back()
      else if (e.key === 'ArrowRight') forward()
    }
    window.addEventListener('mouseup', onMouse)
    window.addEventListener('keydown', onKey)
    // The same buttons pressed over the Store, reported by its page.
    let stopNav: (() => void) | undefined
    let cancelled = false
    void on<boolean>('nav-button', (fwd) => (fwd ? forward() : back())).then((fn) => (cancelled ? fn() : (stopNav = fn)))
    return () => {
      cancelled = true
      stopNav?.()
      window.removeEventListener('mouseup', onMouse)
      window.removeEventListener('keydown', onKey)
    }
  }, [back, forward])

  // Ctrl+R and F5 reload the Store page (when it is showing), never the app.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.key === 'F5' || ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'r'))) return
      e.preventDefault()
      if (view.kind === 'web') web.reload()
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [view.kind, web])

  /* ── Overlays over the native web view ── */
  const [overlay, setOverlay] = useState<Overlay | null>(null)
  const [ctx, setCtx] = useState<({ game: LibraryGame } | { saved: Pick<SavedEntry, 'slug' | 'title' | 'cover'> } | { download: Download }) & { x: number; y: number; track?: boolean; collect?: boolean } | null>(null)
  // Big Picture: full screen, over everything, the Store hidden under it.
  const [bigPicture, setBigPicture] = useState(false)
  useEffect(() => holdPrompts(bigPicture), [bigPicture])
  // Menus open in their own window over the Store; only dialogs hide it, every
  // one of them: the Store is a native layer and would cover any it left up.
  const dialogs = useDialogCount()
  const online = useOnline()
  // Offline says so by itself (the title bar): the service's health only counts with a connection.
  const liveHealth = useServiceHealth()
  const health = online ? liveHealth : { ...liveHealth, state: 'ok' as const, parts: [] }
  const outage = isOutage(health)
  const storeOpen = view.kind === 'web'
  // Kryoto out and nothing to show for it in the Store: its slot says so instead.
  const storeBlocked = outage && (health.state === 'unreachable' || !health.statusUp)
  const storeVisible =
    online &&
    (view.kind === 'web' || (view.kind === 'settings' && isWebSection(view.section))) &&
    !dialogs &&
    !bigPicture &&
    !page.error &&
    !page.retrying &&
    !(storeBlocked && storeOpen) &&
    !settings?.recoveryError
  // A guest has no kryo.to settings to show; the client's own are all there is.
  // With `anchor`, one setting in the section is scrolled to and lit
  // (lib/settings-links.ts); `at` makes asking for the same one again count.
  const [settingsReveal, setSettingsReveal] = useState<{ anchor: string; at: number } | null>(null)
  const openSettings = useCallback(
    (asked: SettingsSection = 'general', askedAnchor?: string) => {
      const { section, anchor } = settingsHome({ section: asked, anchor: askedAnchor })
      const web = guest && isWebSection(section)
      go({ kind: 'settings', section: web ? 'general' : section })
      setSettingsReveal(anchor && !web ? { anchor, at: Date.now() } : null)
    },
    [go, guest],
  )
  const signIn = useCallback(() => openWeb('/login?next=/'), [openWeb])
  // Pages deep in the app ask for a Store page or a Settings section by event
  // (lib/window.ts), so the view switches instead of loading behind it.
  useEffect(() => {
    const store = (e: Event) => {
      const path = (e as CustomEvent<string>).detail
      if (typeof path === 'string' && path.startsWith('/')) openWeb(path)
    }
    const settingsSection = (e: Event) => {
      const target = parseSettingsAddress(String((e as CustomEvent<string>).detail ?? ''))
      if (target) openSettings(target.section, target.anchor)
    }
    const setup = () => setOverlay({ kind: 'setup' })
    const home = () => go({ kind: 'home' })
    const game = (e: Event) => {
      const { id, anchor } = (e as CustomEvent<{ id: string; anchor?: string }>).detail
      go({ kind: 'game', id })
      if (anchor) revealSetting(anchor)
    }
    const couch = () => setBigPicture(true)
    window.addEventListener(OPEN_BIG_PICTURE_EVENT, couch)
    window.addEventListener(OPEN_GAME_EVENT, game)
    window.addEventListener(OPEN_LIBRARY_EVENT, home)
    window.addEventListener(OPEN_STORE_EVENT, store)
    window.addEventListener(OPEN_SETTINGS_EVENT, settingsSection)
    window.addEventListener(OPEN_SETUP_EVENT, setup)
    return () => {
      window.removeEventListener(OPEN_STORE_EVENT, store)
      window.removeEventListener(OPEN_SETTINGS_EVENT, settingsSection)
      window.removeEventListener(OPEN_SETUP_EVENT, setup)
      window.removeEventListener(OPEN_LIBRARY_EVENT, home)
      window.removeEventListener(OPEN_GAME_EVENT, game)
      window.removeEventListener(OPEN_BIG_PICTURE_EVENT, couch)
    }
  }, [openWeb, openSettings, go])
  // Leaving Settings: the account may have changed its look or name there.
  const wasSettings = useRef(false)
  useEffect(() => {
    if (wasSettings.current && view.kind !== 'settings') void call('store_refresh_account').catch(() => {})
    wasSettings.current = view.kind === 'settings'
  }, [view.kind])
  // Before the page paints, so the Store is out of the way of the page that
  // is sliding in over it instead of showing through for a frame.
  useLayoutEffect(() => {
    void setStoreVisible(storeVisible)
  }, [storeVisible])

  // Back: who is signed in, what the account has waiting, every play session
  // kept for it and every download that stopped because the line went.
  const catchUp = useCallback(() => {
    void call('store_refresh_account').catch(() => {})
    void flushPlays()
    flushCarried()
    for (const d of dlRef.current) {
      if (d.status === 'failed' && /connection|network|dns|timed out|offline|resolve|reach|server error|\b50\d\b/i.test(d.error ?? '')) {
        void downloads.resume(d.id).catch(() => {})
      }
    }
  }, [])
  // Back online: the Store's page first (whatever the web view showed while
  // the line was down is an error page).
  const wasOnline = useRef(online)
  useEffect(() => {
    if (online && !wasOnline.current) {
      web.reload()
      catchUp()
    }
    wasOnline.current = online
  }, [online, web, catchUp])

  // Kryoto down with the Store open: its status page takes the Store's place,
  // and when it is back the Store returns to the page it was on, by itself.
  // Only kryo.to's pages: a site of someone else's has nothing to do with this.
  const parked = useRef<string | null>(null)
  const onKryoto = onKryotoPage(page.url, settings?.catalogEndpoint)
  useEffect(() => {
    if (health.state !== 'down' || !health.statusUp || !storeOpen || parked.current || !onKryoto) return
    parked.current = page.url
    setHistory((h) => nav.replaceCurrent(h, { kind: 'web', url: health.statusUrl }))
  }, [health.state, health.statusUp, health.statusUrl, storeOpen, onKryoto, page.url])
  const wasOut = useRef(outage)
  useEffect(() => {
    const was = wasOut.current
    wasOut.current = outage
    if (outage || !was) return
    const back = parked.current
    parked.current = null
    if (back) {
      const status = health.statusUrl
      const shown = (v: View) => v.kind === 'web' && v.url.startsWith(status)
      setHistory((h) => ({ ...h, stack: h.stack.map((v) => (shown(v) ? { kind: 'web' as const, url: back } : v)) }))
      if (view.kind !== 'web') web.navigate(back)
    } else {
      web.reload()
    }
    owing.current = true
    // Only when an outage ends.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [outage])
  // Catching up goes through the Store's page, so it waits for kryo.to to be in it.
  const owing = useRef(false)
  useEffect(() => {
    if (!owing.current || outage || !onKryoto || page.loading) return
    owing.current = false
    catchUp()
  }, [outage, onKryoto, page.loading, catchUp])

  // A pressed button's work failed and nothing else caught it.
  useEffect(() => {
    const failed = (e: Event) => push({ title: 'That did not work', body: errorText((e as CustomEvent<unknown>).detail), gameId: null })
    window.addEventListener(ACTION_FAILED_EVENT, failed)
    return () => window.removeEventListener(ACTION_FAILED_EVENT, failed)
  }, [push])

  // A page that failed may be the first sign of something: look now.
  useEffect(() => {
    if (page.error) pokeHealth()
  }, [page.error])

  // Ctrl+F searches your library, Ctrl+, opens Settings - the keys every
  // desktop app uses. Over the Store the page keeps its own Ctrl+F.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey || e.shiftKey) return
      if (e.key === ',') {
        e.preventDefault()
        openSettings()
      } else if (e.key.toLowerCase() === 'f' && view.kind !== 'web') {
        e.preventDefault()
        if (view.kind !== 'home' && view.kind !== 'game' && view.kind !== 'catalog') go({ kind: 'home' })
        // After the Library has drawn its rail.
        setTimeout(() => {
          const field = document.getElementById('library-search') as HTMLInputElement | null
          field?.focus()
          field?.select()
        }, 30)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [view.kind, go, openSettings])

  // F11, like every other full-screen app.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'F11' || !isTauri()) return
      e.preventDefault()
      void toggleFullscreen()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  // A download of ours refused as made for another network (downloads.rs
  // `refused_ours`): the game's page, with its sheet open, so one press on
  // Download mints a fresh link and the download carries on.
  const openOursRef = useRef<(slug: string, release?: string | null, update?: boolean) => void>(() => {})
  useEffect(() => {
    let stop: (() => void) | undefined
    let cancelled = false
    void on<string>('store-download-page', (slug) => {
      if (/^[a-z0-9-]{1,160}$/i.test(slug)) openOursRef.current(slug, null, true)
    }).then((fn) => (cancelled ? fn() : (stop = fn)))
    return () => {
      cancelled = true
      stop?.()
    }
  }, [])

  // The pet put away from its own right-click menu (pet.rs): said here, with
  // Undo, and the notice opens the setting that brings it back.
  useEffect(() => {
    let stop: (() => void) | undefined
    let cancelled = false
    void on<{ pet: boolean }>('pet-settings', (p) => {
      if (p.pet) return
      push({
        title: 'Pet hidden',
        body: 'Bring it back in Settings, under General.',
        gameId: null,
        settings: { section: 'general', anchor: 'desktop-pet' },
        action: { label: 'Undo', run: () => settingsApi.get().then((s) => settingsApi.save({ ...s, pet: true })) },
      })
    }).then((fn) => (cancelled ? fn() : (stop = fn)))
    return () => {
      cancelled = true
      stop?.()
    }
  }, [push])

  // The tray menu's shortcuts.
  useEffect(() => {
    let stop: (() => void) | undefined
    let cancelled = false
    void on<string>('tray-go', (where) => {
      if (where === 'store') openWeb('/')
      else if (where === 'library') go({ kind: 'home' })
      else if (where === 'downloads') go({ kind: 'downloads' })
      else if (where === 'settings') openSettings()
    }).then((fn) => (cancelled ? fn() : (stop = fn)))
    return () => {
      cancelled = true
      stop?.()
    }
  }, [go, openWeb, openSettings])

  // A kryo.to game closing adds the session to the account's play time, and
  // one starting or closing tells kryo.to what is being played right now
  // (Settings > Windows can turn both off).
  const gamesRef = useRef(lib.games)
  gamesRef.current = lib.games
  const shareRef = useRef(true)
  // Play time and "playing now" belong to an account; a guest has none.
  shareRef.current = !guest && (settings?.sharePlaytime ?? true)
  useEffect(() => {
    let stop: (() => void) | undefined
    let cancelled = false
    void on<{ id: string; running: boolean; seconds: number | null }>('game-state', (e) => {
      if (!shareRef.current) return
      const slug = gamesRef.current.find((g) => g.id === e.id)?.slug
      if (!slug) return
      // "Playing right now" on kryo.to: on as it starts, off as it closes.
      void call('store_report_playing', { slug, playing: e.running }).catch((err) =>
        isExpectedReportError(err) ? undefined : logError('playing', err),
      )
      if (e.running || !e.seconds || e.seconds < 60) return
      const now = Math.floor(Date.now() / 1000)
      const key = `${e.id.slice(0, 40)}-${now}`.replace(/[^a-zA-Z0-9-]/g, '-')
      void reportPlay({ slug, startedAt: now - e.seconds, seconds: Math.min(e.seconds, 86_400), key })
    }).then((fn) => (cancelled ? fn() : (stop = fn)))
    return () => {
      cancelled = true
      stop?.()
    }
  }, [])

  // While a game runs, say so again every few minutes: kryo.to forgets a
  // game it has not heard about for ten, so a crash or a lost connection
  // never leaves somebody "playing" forever.
  const runningRef = useRef(lib.running)
  runningRef.current = lib.running
  useEffect(() => {
    const t = window.setInterval(() => {
      if (!shareRef.current) return
      for (const id of runningRef.current) {
        const slug = gamesRef.current.find((g) => g.id === id)?.slug
        if (slug) {
          void call('store_report_playing', { slug, playing: true }).catch((err) =>
            isExpectedReportError(err) ? undefined : logError('playing', err),
          )
        }
      }
      // Sessions that ended while kryo.to was not open go now.
      void flushPlays()
      flushCarried()
      // Hours this PC counted that never reached the account (playtime-sync.ts).
      void syncPlaytime(gamesRef.current, [...runningRef.current])
    }, 4 * 60_000)
    return () => window.clearInterval(t)
  }, [])

  // A game just linked to its kryo.to page (brought in with Bridge, or one
  // of the player's own that kryo.to now lists) sends the hours it had before,
  // once each. A game kryo.to does not list never sends anything.
  const carrying = useRef<Set<string>>(new Set())
  const flushCarried = useCallback(() => {
    if (!shareRef.current) return
    for (const g of gamesRef.current) {
      if (!g.slug || !g.carrySends?.length) continue
      for (const c of g.carrySends) {
        if (carrying.current.has(c.key)) continue
        carrying.current.add(c.key)
        // Marked sent when kryo.to answers; until then not sent again for a
        // minute, then tried again (a signed-out page, a busy server).
        void carry
          .send(g.id, g.slug, c.seconds, c.key)
          .catch((err) => (isExpectedReportError(err) ? undefined : logError('playtime', err)))
          .finally(() => window.setTimeout(() => carrying.current.delete(c.key), 60_000))
      }
    }
  }, [])
  useEffect(() => flushCarried(), [lib.games, flushCarried])

  // One of the player's own games is on kryo.to now (the same Steam game, or a
  // sure match): one small question each, yes to link it, no for good. A
  // question left unanswered comes back the next time Kryoto starts.
  const offered = useRef<Set<string>>(new Set())
  useEffect(() => {
    const ask = () =>
      void merges
        .offers()
        .then((list) => {
          for (const o of list.filter((o) => !offered.current.has(o.id)).slice(0, 3)) {
            offered.current.add(o.id)
            push({
              title: `Kryoto now has ${o.kryoTitle}`,
              body: `Link your ${o.title} to it? It gets Kryoto's art and details and keeps its play time.`,
              gameId: null,
              action: {
                label: 'Yes',
                run: () =>
                  merges
                    .answer(o.id, o.slug, true)
                    .then((g) => push({ title: `${g.title} is linked`, body: 'Its play time came along.', gameId: g.id }))
                    .catch((err) => push({ title: 'Could not link it', body: errorText(err), gameId: null })),
              },
              decline: { label: 'No', run: () => merges.answer(o.id, o.slug, false) },
            })
          }
        })
        .catch((err) => logWarn('merge', errorText(err)))
    const first = window.setTimeout(ask, 20_000)
    const every = window.setInterval(ask, 6 * 3600_000)
    return () => {
      window.clearTimeout(first)
      window.clearInterval(every)
    }
  }, [push])

  // What the library reports as an error goes in the log too.
  useEffect(() => {
    if (lib.error && !lib.errorQuiet) logError('library', lib.error)
  }, [lib.error, lib.errorQuiet])

  const gameById = (id: string) => lib.games.find((g) => g.id === id) ?? null

  // A game's own page says what went wrong with it. Anywhere else (Play on the
  // Library's home, another game's page, Big Picture) it is a notice that opens
  // that page, and an empty Library says it for itself.
  useEffect(() => {
    if (!lib.error || !lib.games.length) return
    if (!bigPicture && view.kind === 'game' && view.id === lib.errorGame) return
    const game = lib.errorGame ? gameById(lib.errorGame) : null
    push({ title: game?.title ?? 'That did not work', body: lib.error, gameId: game?.id ?? null })
    // Only when the error changes: moving around must not say it again.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lib.error, lib.errorGame])

  // kryo.to's current build per game, as game pages learn it, so a game's
  // menu can offer the update without asking kryo.to again.
  const [latestBySlug, setLatestBySlug] = useState<Record<string, string | null>>({})
  const noteLatest = useCallback((slug: string, latest: string | null) => setLatestBySlug((m) => (m[slug] === latest ? m : { ...m, [slug]: latest })), [])
  /** The newer build kryo.to has for an installed game, when one is known and the game is not kept on its build. */
  const updateFor = (g: LibraryGame): string | null => {
    if (!g.slug || !knowsBuild(g)) return null
    const latest = latestBySlug[g.slug]
    if (!latest || latest === (g.version ?? '')) return null
    if (g.pinnedVersion && g.pinnedVersion === g.version) return null
    return latest
  }
  /**
   * Our own copy of a game, through its Store page's download sheet. `update`
   * says the press was the app's own (Get the update, Reinstall, Switch): the
   * site then lets the sheet's Download start the file at once (the visit's
   * first press would otherwise have gone to the ad, and the update looked
   * like a button that did nothing).
   */
  const openOurs = useCallback(
    (slug: string, release?: string | null, update = false) =>
      openWeb(`/game/${slug}?download=1${update ? '&update=1' : ''}${release ? `&release=${encodeURIComponent(release)}` : ''}`),
    [openWeb],
  )
  openOursRef.current = openOurs

  // Settings > Downloads > Game updates: opt-in, one game at a time.
  const startAutoUpdate = useCallback((g: LibraryGame) => openOurs(g.slug!, null, true), [openOurs])
  useAutoUpdates({
    enabled: !!settings?.autoUpdateGames && !guest,
    online,
    games: lib.games,
    running: lib.running,
    downloads: dl,
    latest: latestBySlug,
    noteLatest,
    start: startAutoUpdate,
  })

  /* ── Playing ── */
  // "Restore your playtime?" before a game this PC undercounts (PlaytimePrompt.tsx).
  const playtimeGate = usePlaytimeGate({ signedIn: !guest, onChanged: lib.upsert })
  const gate = playtimeGate.gate
  const play = useCallback(
    (game: LibraryGame, forceAsk = false) => {
      const target = playTarget(game)
      if (forceAsk || target === 'ask') setOverlay({ kind: 'choose', id: game.id })
      else gate(game, () => void lib.play(game.id, target))
    },
    [lib, gate],
  )
  const playEntry = useCallback((game: LibraryGame, entry: number | null) => gate(game, () => void lib.play(game.id, entry)), [lib, gate])

  // Play on a game's own kryo.to page (`store_play`), started the way the
  // Library's Play starts it. The page follows the library and what is
  // running, so it shows Play, Running or Download as they change.
  const playRef = useRef(play)
  playRef.current = play
  useEffect(() => {
    let stop: (() => void) | undefined
    let cancelled = false
    void on<string>('store-play', (id) => {
      const game = gamesRef.current.find((g) => g.id === id)
      if (game) playRef.current(game)
    }).then((fn) => (cancelled ? fn() : (stop = fn)))
    return () => {
      cancelled = true
      stop?.()
    }
  }, [])
  // Only what the page shows: which kryo.to games are here, at which build,
  // and which are running (`lib.running` is a new set on every poll).
  const storeLibraryKey = [
    ...lib.games.filter((g) => g.slug).map((g) => `${g.slug}@${g.version ?? ''}`),
    ...[...lib.running].map((id) => `run:${id}`),
  ]
    .sort()
    .join('|')
  useEffect(() => {
    void call('store_library_changed').catch(() => {})
  }, [storeLibraryKey])

  // "Join" on a game invite in chat: start the game (into the host's Steam
  // lobby when the invite has one), ask Steam to join if it is already
  // running, or open its page to install it.
  const joinInvite = useCallback(
    (invite: { slug: string; steamLobby: string; hostSteamId: string }) => {
      const game = lib.games.find((g) => g.slug === invite.slug)
      if (!game) {
        openWeb(`/game/${encodeURIComponent(invite.slug)}`)
        return
      }
      go({ kind: 'game', id: game.id })
      if (lib.running.has(game.id)) {
        if (invite.steamLobby && invite.hostSteamId) {
          void call('steam_join_lobby', { lobby: invite.steamLobby, host: invite.hostSteamId }).catch((e: unknown) => push({ title: game.title, body: errorText(e), gameId: game.id }))
        }
        return
      }
      const target = playTarget(game)
      void lib.play(game.id, target === 'ask' ? null : target, invite.steamLobby || undefined)
    },
    [lib, go, openWeb],
  )
  const inviteGames = useMemo(
    () =>
      lib.games
        .filter((g): g is LibraryGame & { slug: string } => !!g.slug)
        .map((g) => ({ id: g.id, slug: g.slug, title: titleWithBranch(g.title, g.steamBranch), cover: g.cover, running: lib.running.has(g.id) })),
    [lib.games, lib.running],
  )

  // `kryoto://` links from kryo.to (src-tauri/src/links.rs): a game's page, or
  // starting it - through the same Play as the library, so a game with more
  // than one mode still asks which. Not installed: its page, to get it.
  // kryoto://chat/<username>: the Friends page opens that conversation.
  const [chatWith, setChatWith] = useState<{ username: string; at: number } | null>(null)
  const openLink = useCallback(
    (link: { action: string; slug: string }) => {
      if (link.action === 'settings') {
        const target = parseSettingsAddress(link.slug)
        if (target) openSettings(target.section, target.anchor)
        return
      }
      if (link.action === 'chat') {
        setChatWith({ username: link.slug, at: Date.now() })
        go({ kind: 'friends' })
        return
      }
      // Another launcher's shortcut to a library game (Bridge). Never kryo.to:
      // it may be one of the player's own.
      if (link.action === 'library') {
        const own = lib.games.find((g) => g.id === link.slug)
        if (!own) return lib.setError('That game is no longer in your library.', true)
        go({ kind: 'game', id: own.id })
        if (!lib.running.has(own.id)) play(own)
        return
      }
      const game = lib.games.find((g) => g.slug === link.slug)
      if (link.action === 'play' && game) {
        go({ kind: 'game', id: game.id })
        if (!lib.running.has(game.id)) play(game)
      } else {
        openWeb(`/game/${encodeURIComponent(link.slug)}`)
      }
    },
    [lib.games, lib.running, go, play, openWeb, openSettings],
  )
  const openLinkRef = useRef(openLink)
  openLinkRef.current = openLink
  // Links are held natively until taken (links.rs), so one that arrived before
  // sign-in, or started the app, is still there. Taken once the library has
  // loaded - "play" needs to know what is installed - and again on each
  // "deep-link" nudge.
  const libLoaded = useRef(false)
  libLoaded.current = lib.loaded
  const takeLink = useCallback(() => {
    if (!libLoaded.current) return
    void call<{ action: string; slug: string } | null>('take_pending_link')
      .then((link) => link && openLinkRef.current(link))
      .catch(() => {})
  }, [])
  useEffect(() => {
    if (lib.loaded) takeLink()
  }, [lib.loaded, takeLink])
  useEffect(() => {
    let stop: (() => void) | undefined
    let cancelled = false
    void on('deep-link', () => takeLink()).then((fn) => (cancelled ? fn() : (stop = fn)))
    return () => {
      cancelled = true
      stop?.()
    }
  }, [takeLink])

  /** `onBar`: for the game page's Manage, whose bar has the folder and the Store page beside it. */
  const discord = useDiscord()
  const discordItem = (g: LibraryGame): MenuEntry[] => {
    const d = discord?.games.find((x) => x.id === g.id)
    if (!d || d.why === 'off') return []
    return [{ label: d.why ? 'Show on Discord' : 'Hide from Discord', icon: <MessageCircle />, onSelect: () => void discordGames.set(g.id, !!d.why) }]
  }
  const makeShortcut = (g: LibraryGame) =>
    void library
      .shortcut(g.id)
      .then((place) =>
        push({
          title: place === 'menu' ? 'Added to your applications menu' : 'Shortcut on your desktop',
          body: `${g.title} starts through Kryoto Desktop from there, so play time and cloud saves keep working.`,
          gameId: null,
        }),
      )
      .catch((e: unknown) => lib.setError(errorText(e)))
  /** Manage > Rename folder: only offered while the folder's name has a letter some games cannot start from. */
  const plainFolder = (g: LibraryGame) =>
    void library
      .plainFolder(g.id)
      .then((next) => {
        lib.upsert(next)
        push({ title: 'Folder renamed', body: `${g.title} is in ${folderName(next.installDir)} now. Its play time, settings and saves came along.`, gameId: null })
      })
      .catch((e: unknown) => lib.setError(errorText(e), false, g.id))
  /** Says what happened, with Undo: the game has just gone from where the player was looking. */
  const hideGames = (list: LibraryGame[], hidden: boolean, adult = false) => {
    // Only what this hid: an adult game hidden before stays hidden.
    const undo = () => library.setHidden(list.map((g) => g.id), !hidden).catch((e) => lib.setError(errorText(e)))
    void library
      .setHidden(list.map((g) => g.id), hidden, { adult })
      .then(() => {
        if (!hidden) return
        const name = adult ? 'Adult games' : list[0]?.title
        push({ title: adult ? 'Adult games hidden' : `${name} hidden`, body: `${adult ? 'They are' : 'It is'} under Hidden in your Library now.${guest ? '' : ` Nobody else can see ${adult ? 'them' : 'it'}.`}`, gameId: null, action: { label: 'Undo', run: undo } })
      })
      .catch((e) => lib.setError(errorText(e)))
  }
  const hideItems = (g: LibraryGame): MenuEntry[] => {
    const adult = lib.games.filter((x) => x.nsfw && !x.hidden)
    return [
      { label: g.hidden ? 'Show game' : 'Hide game', icon: g.hidden ? <Eye /> : <EyeOff />, onSelect: () => hideGames([g], !g.hidden) },
      ...(g.nsfw && !g.hidden && adult.length > 1 ? [{ label: 'Hide all adult games', icon: <EyeOff />, hint: String(adult.length), onSelect: () => hideGames(adult, true, !guest) }] : []),
    ]
  }
  const manageMenu = (g: LibraryGame, onBar = false): MenuEntry[] => g.launcher ? [
    // Its files, settings and saves are that launcher's.
    ...(onBar ? [] : [{ label: 'Open folder', icon: <FolderOpen />, onSelect: () => void library.openFolder(g.installDir) }]),
    ...hideItems(g),
    { label: 'Remove from Library', icon: <Trash2 />, danger: true, onSelect: () => setOverlay({ kind: 'uninstall', id: g.id }) },
  ] : [
    { label: 'Repair', icon: <Wrench />, disabled: lib.running.has(g.id), onSelect: () => setOverlay({ kind: 'props', id: g.id, tab: 'repair' }) },
    { label: 'Properties', icon: <SettingsIcon />, onSelect: () => setOverlay({ kind: 'props', id: g.id }) },
    ...hideItems(g),
    ...discordItem(g),
    ...(g.slug ? [{ label: 'Builds', icon: <Layers />, onSelect: () => setOverlay({ kind: 'props', id: g.id, tab: 'versions' }) }] : []),
    ...(padStore.on ? [{ label: 'Controller', icon: <Gamepad2 />, onSelect: () => setOverlay({ kind: 'props', id: g.id, tab: 'controller' }) }] : []),
    ...(padStore.on && padSetups.includes(g.id) ? [{ label: 'Remove controller setup', icon: <Gamepad2 />, onSelect: () => removePadSetup(g) }] : []),
    ...(onBar ? [] : [{ label: 'Open folder', icon: <FolderOpen />, onSelect: () => void library.openFolder(g.installDir) }]),
    ...(!plainPath(g.installDir) ? [{ label: 'Rename folder', hint: 'Plain letters', icon: <PenLine />, disabled: lib.running.has(g.id), onSelect: () => plainFolder(g) }] : []),
    ...(onBar ? [] : [{ label: 'Create desktop shortcut', icon: <Monitor />, onSelect: () => makeShortcut(g) }]),
    // Where Wine or Proton keeps this game's Windows: saves, settings, crash
    // dumps. Linux only; Windows runs the game itself.
    ...(!isWindowsHost()
      ? [
          {
            label: 'Open Wine prefix',
            icon: <Wine />,
            onSelect: () =>
              void gameLogs
                .prefix(g.id)
                .then((dir) =>
                  dir
                    ? library.openFolder(dir)
                    : lib.setError('This game has no Wine prefix yet. It is made the first time the game starts.', true),
                )
                .catch((e) => lib.setError(errorText(e))),
          },
        ]
      : []),
    // One log per Play press, with everything the game (and Wine) printed
    // when it crashed: here, not three clicks down in Properties.
    { label: 'Logs', icon: <ScrollText />, onSelect: () => setOverlay({ kind: 'props', id: g.id, tab: 'logs' }) },
    ...(g.slug && !onBar ? [{ label: 'Store page', icon: <Globe />, onSelect: () => openWeb(`/game/${g.slug}`) }] : []),
    // A game of the player's own (or from Hydra) that kryo.to does not list: only sent when they press this.
    ...(!g.slug ? [{ label: 'Request on Kryoto', icon: <MessageSquarePlus />, onSelect: () => openWeb(`/requests/new?q=${encodeURIComponent(g.title.slice(0, 200))}`) }] : []),
    { separator: true },
    { label: 'Uninstall', icon: <Trash2 />, danger: true, onSelect: () => setOverlay({ kind: 'uninstall', id: g.id }) },
  ]
  const gameMenu = (g: LibraryGame): MenuEntry[] => [
    // A beta branch or a second copy says which one this menu is for.
    ...(branchLabel(g.steamBranch, g.title) || copyLabel(g, lib.games)
      ? [{ heading: [branchLabel(g.steamBranch, g.title) ? titleWithBranch(g.title, g.steamBranch) : g.title, copyLabel(g, lib.games)].filter(Boolean).join(' · ') } as MenuEntry]
      : []),
    lib.running.has(g.id)
      ? { label: 'Stop', icon: <Square />, onSelect: () => void lib.stop(g.id) }
      : { label: 'Play', icon: <Play />, onSelect: () => play(g) },
    ...(g.entries.length > 1 && !lib.running.has(g.id)
      ? [
          { heading: 'Play as' } as MenuEntry,
          ...g.entries.map((e, i) => ({
            label: `${entryLabel(e)}${entryIsVr(e) ? ' (VR)' : ''}`,
            onSelect: () => playEntry(g, i),
          })),
        ]
      : []),
    ...(g.slug && updateFor(g) && !lib.running.has(g.id)
      ? [{ label: `Update to ${updateFor(g)}`, icon: <DownloadIcon />, onSelect: () => openOurs(g.slug!, null, true) }]
      : []),
    { separator: true },
    ...(g.slug && !guest ? shelfMenu(g.slug) : []),
    ...manageMenu(g),
  ]
  // Taking a game off your kryo.to list says so, with Undo: the menu that did
  // it has closed, and the list is the only record of what it was saved as.
  const removeSaved = (slug: string, game?: Pick<LibraryGame, 'title' | 'cover'>) => {
    const entry = saved.find((e) => e.slug === slug)
    if (!entry) return
    const art = { title: game?.title || entry.title, cover: game?.cover ?? entry.cover }
    void setSavedStatus(slug, null)
      .then(() =>
        push({
          title: `${art.title} removed`,
          body: `Taken off your saved games on kryo.to. It was ${STATUS_LABEL[entry.status]}.`,
          gameId: null,
          action: { label: 'Undo', run: () => setSavedStatus(slug, entry.status, art).catch((e) => lib.setError(errorText(e))) },
        }),
      )
      .catch((e) => lib.setError(errorText(e)))
  }
  // One Track row in the right-click menu; it opens the statuses in its place.
  const shelfMenu = (slug: string): MenuEntry[] => {
    const now = saved.find(e => e.slug === slug)?.status
    return [
      { label: 'Track', icon: <Bookmark />, hint: now ? STATUS_LABEL[now] : undefined, onSelect: () => setCtx(ctx && { ...ctx, track: true }) },
      // Steam-style categories are kryo.to collections; they show as groups in the Library (suggestion #71).
      { label: 'Add to collection', icon: <FolderPlus />, hint: ownLists.filter((c) => c.games.some((x) => x.slug === slug)).length ? String(ownLists.filter((c) => c.games.some((x) => x.slug === slug)).length) : undefined, onSelect: () => setCtx(ctx && { ...ctx, collect: true }) },
      { separator: true },
    ]
  }
  const trackMenu = (slug: string, game?: Pick<LibraryGame, 'title' | 'cover'>): MenuEntry[] => [
    { heading: 'Track on kryo.to' },
    ...STATUSES.map(status => ({
      label: STATUS_LABEL[status],
      checked: saved.some(e => e.slug === slug && e.status === status),
      onSelect: () => void setSavedStatus(slug, status, game).catch(e => lib.setError(errorText(e))),
    })),
    ...(saved.some(e => e.slug === slug) ? [{ separator: true } as MenuEntry, { label: 'Remove from list', onSelect: () => removeSaved(slug, game) }] : []),
  ]
  const collectMenu = (slug: string): MenuEntry[] => [
    { heading: 'Your collections' },
    ...ownLists.map((c) => {
      const has = c.games.some((x) => x.slug === slug)
      return { label: c.name, checked: has, onSelect: () => void collectionsApi.setItem(c.id, slug, !has).catch((e) => lib.setError(errorText(e))) }
    }),
    ...(ownLists.length ? [{ separator: true } as MenuEntry] : []),
    { label: 'New collection', icon: <FolderPlus />, onSelect: () => openWeb('/library?tab=collections&new=1') },
  ]
  const contextItems = (): MenuEntry[] => {
    if (!ctx) return []
    if (ctx.collect) {
      const slug = 'game' in ctx ? ctx.game.slug : 'saved' in ctx ? ctx.saved.slug : null
      if (slug) return collectMenu(slug)
    }
    if (ctx.track) {
      const slug = 'game' in ctx ? ctx.game.slug : 'saved' in ctx ? ctx.saved.slug : null
      if (slug) return trackMenu(slug, 'game' in ctx ? ctx.game : 'saved' in ctx ? ctx.saved : undefined)
    }
    if ('game' in ctx) {
      // Hiding a game is done where it stands; everything else opens something behind Big Picture.
      const stays = new Set(hideItems(ctx.game).map(entry => ('label' in entry ? entry.label : '')))
      return gameMenu(ctx.game).map(entry => 'onSelect' in entry && bigPicture && !stays.has(entry.label) ? { ...entry, onSelect: () => { setBigPicture(false); entry.onSelect() } } : entry)
    }
    if ('saved' in ctx) {
      const g = ctx.saved
      return [
        { label: 'Install', icon: <DownloadIcon />, onSelect: () => openOurs(g.slug) },
        { label: 'Game details', onSelect: () => go({ kind: 'catalog', slug: g.slug }) },
        { label: 'Store page', icon: <Globe />, onSelect: () => openWeb(`/game/${g.slug}`) },
        hiddenSlugs.includes(g.slug)
          ? { label: 'Show game', icon: <Eye />, onSelect: () => void library.setHidden([], false, { slugs: [g.slug] }).catch(e => lib.setError(errorText(e))) }
          : { label: 'Hide game', icon: <EyeOff />, onSelect: () => void library.setHidden([], true, { slugs: [g.slug] }).catch(e => lib.setError(errorText(e))) },
        { separator: true },
        ...(!guest ? shelfMenu(g.slug) : []),
      ]
    }
    const d = ctx.download
    return [
      { label: d.status === 'paused' ? 'Resume download' : 'Pause download', onSelect: () => void (d.status === 'paused' ? downloads.resume(d.id) : downloads.pause(d.id)).catch(e => lib.setError(errorText(e))) },
      { label: 'View downloads', onSelect: () => go({ kind: 'downloads' }) },
      ...(d.slug ? [{ label: 'Store page', onSelect: () => openWeb(`/game/${d.slug}`) }] : []),
    ]
  }

  const username = encodeURIComponent(account.username)
  // Back to the Store from the Library (or anywhere else in the client)
  // resumes the page it was on, as it was left. Only a press while already
  // looking at the Store, or with the view on another section's page (a
  // profile, the blog), goes to its home.
  const openStore = () =>
    openWeb(view.kind !== 'web' && tabForUrl(page.url, settings?.catalogEndpoint) === 'store' ? undefined : '/')
  const webUrl = view.kind === 'web' ? view.url : page.url
  const tabs: NavTabSpec[] = [
    {
      id: 'store',
      label: 'Store',
      onOpen: openStore,
      items: [
        { label: 'Home', icon: <House />, onSelect: () => openWeb('/') },
        { label: 'Browse', icon: <LayoutGrid />, onSelect: () => openWeb('/browse') },
        { label: 'Requests', icon: <MessageSquarePlus />, onSelect: () => openWeb('/requests') },
      ],
    },
    {
      id: 'library',
      label: 'Library',
      onOpen: () => go({ kind: 'home' }),
      items: [
        { label: 'Home', icon: <House />, onSelect: () => go({ kind: 'home' }) },
        { label: 'Downloads', icon: <DownloadIcon />, onSelect: () => go({ kind: 'downloads' }) },
        { label: 'Bridge', icon: <FolderInput />, onSelect: () => setOverlay({ kind: 'bridge' }) },
      ],
    },
    {
      id: 'community',
      label: 'Community',
      onOpen: () => openWeb('/community'),
      items: [
        ...(account.friends && !guest ? [{ label: 'Friends & chat', icon: <MessageCircle />, onSelect: () => go({ kind: 'friends' }) }, { separator: true as const }] : []),
        { label: 'Statistics', icon: <ChartColumn />, onSelect: () => openWeb('/community?tab=stats') },
        { label: 'Blog', icon: <Newspaper />, onSelect: () => openWeb('/blog') },
        { label: 'Collections', icon: <Layers />, onSelect: () => openWeb('/collections/discover') },
        { label: 'Playtest', icon: <FlaskConical />, onSelect: () => openWeb('/testers') },
        { separator: true },
        { label: 'Discord', icon: <ArrowUpRight />, onSelect: () => void openExternal(DISCORD_URL) },
        { label: 'Reddit', icon: <ArrowUpRight />, onSelect: () => void openExternal(REDDIT_URL) },
        { label: 'YouTube', icon: <ArrowUpRight />, onSelect: () => void openExternal(YOUTUBE_URL) },
      ],
    },
    guest
      ? { id: 'profile', label: 'Sign in', onOpen: signIn, items: [] }
      : {
          id: 'profile',
          label: account.displayName || account.username,
          onOpen: () => openWeb(`/user/${username}`),
          items: [
            { label: 'Profile', icon: <User />, onSelect: () => openWeb(`/user/${username}`) },
            { label: 'Saved games', icon: <Bookmark />, onSelect: () => openWeb('/library') },
            { label: 'My collections', icon: <FolderHeart />, onSelect: () => openWeb('/library?tab=collections') },
            ...(account.friends ? [{ label: 'Friends & chat', icon: <MessageCircle />, onSelect: () => go({ kind: 'friends' }) }] : []),
            { label: 'Notifications', icon: <BellIcon />, onSelect: () => openWeb('/notifications') },
          ],
        },
  ]
  const currentTab: TopTab =
    view.kind === 'web'
      ? tabForUrl(webUrl, settings?.catalogEndpoint)
      : view.kind === 'friends' || view.kind === 'settings'
        ? 'profile'
        : 'library'

  // Controllers (kryo.to feature flag `controller`): read while the account
  // has it, move around the client with one, and open Settings > Devices
  // from a new pad's notification.
  useControllerFeature(!!account.controller && !guest)
  const padStore = usePadStore()
  // Which games have a controller setup, asked as a right-click menu opens.
  const [padSetups, setPadSetups] = useState<string[]>([])
  const menuOpen = !!ctx
  useEffect(() => {
    if (menuOpen && padStore.on) void padGames.configured().then(setPadSetups).catch(() => {})
  }, [menuOpen, padStore.on])
  // Says so with Undo: the menu has closed, and this is the only record of the setup.
  const removePadSetup = (g: LibraryGame) => {
    void padGames
      .get(g.id)
      .then(({ profile }) =>
        padGames.set(g.id, null).then(() =>
          push({
            title: 'Controller setup removed',
            body: `${g.title} follows the shared default again.`,
            gameId: null,
            action: { label: 'Undo', run: () => padGames.set(g.id, profile).catch((e) => lib.setError(errorText(e))) },
          }),
        ),
      )
      .catch((e: unknown) => lib.setError(errorText(e)))
  }
  const announcedPads = useRef(new Set<number>())
  useEffect(() => {
    if (!padStore.on) { announcedPads.current.clear(); return }
    if (!padStore.config) return
    if (padStore.config.notify) for (const pad of padStore.pads) {
      if (!announcedPads.current.has(pad.id)) push({
        title: `${pad.name} connected`,
        body: pad.mapped ? 'Ready to use. Click to test buttons or change your setup.' : 'Click to set up your buttons. It takes about a minute.',
        gameId: null,
        controllerGuid: pad.guid,
      })
    }
    announcedPads.current = new Set(padStore.pads.map(p => p.id))
  }, [padStore.on, padStore.pads, padStore.config, push])
  const firstPad = padStore.pads[0] ?? null
  usePadNavigation({
    enabled: padStore.on && !!padStore.config?.navigate,
    haptics: !!padStore.config?.haptics,
    paused: view.kind === 'settings' && view.section === 'devices',
    family: familyOf(firstPad, firstPad ? padStore.config?.pads[firstPad.guid] : null),
    onBack: back,
    // Only in: Big Picture is left from its own menu (lib/pad-nav.ts).
    onGuide: () => setBigPicture(true),
    forward: storeVisible && !bigPicture,
    onTab: (step) => {
      const i = tabs.findIndex((t) => t.id === currentTab)
      tabs[(i + step + tabs.length) % tabs.length]?.onOpen()
    },
  })
  useEffect(() => {
    if (!account.controller) return
    let stop: (() => void) | undefined
    let cancelled = false
    void on<string>('pad-configure', (guid) => {
      requestPad(guid)
      openSettings('controller')
    }).then((fn) => (cancelled ? fn() : (stop = fn)))
    return () => {
      cancelled = true
      stop?.()
    }
  }, [account.controller, openSettings])

  const newSettings = useNewSettings(guest)
  // The window's one row folds on a narrow window; what left it is here instead.
  const { row, fold } = useRowFold([account.displayName, account.username, account.kryos, news?.version, online, view.kind === 'web' ? view.url : ''].join('|'))
  const folded: MenuEntry[] = [
    ...(fold >= FOLD.fullscreen ? [{ label: 'Full screen', hint: 'F11', onSelect: () => void toggleFullscreen() }] : []),
  ]
  const foldedProfile: MenuEntry[] =
    fold >= FOLD.profile
      ? [
          { label: 'Saved games', icon: <Bookmark />, onSelect: () => openWeb('/library') },
          { label: 'My collections', icon: <FolderHeart />, onSelect: () => openWeb('/library?tab=collections') },
          ...(account.friends ? [{ label: 'Friends & chat', icon: <MessageCircle />, onSelect: () => go({ kind: 'friends' }) }] : []),
          { label: 'Notifications', icon: <BellIcon />, onSelect: () => openWeb('/notifications') },
        ]
      : []
  // What a menu bar held, and nothing else has a place for: help and leaving.
  const common: MenuEntry[] = [
    ...(account.controller ? [{ label: 'Big Picture', onSelect: () => setBigPicture(true) }] : []),
    { separator: true },
    { label: 'Kryoto support', onSelect: () => openWeb('/support') },
    { label: "What's new", onSelect: () => setOverlay({ kind: 'news' }) },
    { label: 'About Kryoto Desktop', onSelect: () => setOverlay({ kind: 'about' }) },
    { separator: true },
  ]
  // The menu bar beside the mark (it folds into one menu on the mark when the row is short).
  const recent = lib.games
    .filter((g) => g.lastPlayed)
    .sort((a, b) => (b.lastPlayed ?? 0) - (a.lastPlayed ?? 0))
    .slice(0, 4)
  const titleMenus: { label: string; items: MenuEntry[] }[] = [
    {
      label: 'Kryoto',
      items: [
        { label: 'Settings', icon: <SettingsIcon />, hint: 'Ctrl ,', onSelect: () => openSettings() },
        { label: 'Downloads', icon: <DownloadIcon />, onSelect: () => go({ kind: 'downloads' }) },
        { separator: true },
        guest
          ? { label: 'Sign in to kryo.to', icon: <User />, onSelect: signIn }
          : { label: `Sign out ${account.username}`, icon: <LogOut />, onSelect: leaveAccount },
        { label: 'Exit Kryoto', onSelect: () => void exitApp() },
      ],
    },
    {
      label: 'View',
      items: [
        { label: 'Store', onSelect: openStore },
        { label: 'Library', onSelect: () => go({ kind: 'home' }) },
        { label: 'Downloads', onSelect: () => go({ kind: 'downloads' }) },
        { label: 'Community', onSelect: () => openWeb('/community') },
        { label: 'Blog', onSelect: () => openWeb('/blog') },
        ...(account.friends && !guest ? [{ label: 'Friends & chat', onSelect: () => go({ kind: 'friends' }) }] : []),
        { separator: true },
        { label: 'Reload page', hint: 'Ctrl R', disabled: view.kind !== 'web', onSelect: () => web.reload() },
        { label: 'Full screen', hint: 'F11', onSelect: () => void toggleFullscreen().catch(() => {}) },
        ...(account.controller ? [{ label: 'Big Picture', onSelect: () => setBigPicture(true) }] : []),
      ],
    },
    {
      label: 'Games',
      items: [
        { label: 'View games library', onSelect: () => go({ kind: 'home' }) },
        {
          label: 'Search your games',
          hint: 'Ctrl F',
          onSelect: () => {
            go({ kind: 'home' })
            setTimeout(() => document.getElementById('library-search')?.focus(), 30)
          },
        },
        { label: 'Add a game on this PC', icon: <Plus />, onSelect: () => setOverlay({ kind: 'add', slug: null }) },
        { label: 'Bridge', icon: <FolderInput />, onSelect: () => setOverlay({ kind: 'bridge' }) },
        ...(recent.length
          ? [{ separator: true } as MenuEntry, { heading: 'Recent' } as MenuEntry, ...recent.map((g) => ({ label: titleWithBranch(g.title, g.steamBranch), icon: <Play />, onSelect: () => play(g) }))]
          : []),
      ],
    },
    {
      label: 'Help',
      items: [
        { label: 'Kryoto support', onSelect: () => openWeb('/support') },
        { label: "What's new", onSelect: () => setOverlay({ kind: 'news' }) },
        { separator: true },
        { label: 'Discord', icon: <ArrowUpRight />, onSelect: () => void openExternal(DISCORD_URL) },
        { label: 'Reddit', icon: <ArrowUpRight />, onSelect: () => void openExternal(REDDIT_URL) },
        { label: 'YouTube', icon: <ArrowUpRight />, onSelect: () => void openExternal(YOUTUBE_URL) },
        { separator: true },
        { label: 'About Kryoto Desktop', onSelect: () => setOverlay({ kind: 'about' }) },
      ],
    },
  ]
  const accountMenu: MenuEntry[] = guest
    ? [
        { label: 'Sign in to kryo.to', icon: <User />, onSelect: signIn },
        { label: 'Settings', icon: <SettingsIcon />, hint: 'Ctrl ,', onSelect: () => openSettings() },
        ...folded,
        ...common,
        { label: 'Exit Kryoto', onSelect: () => void exitApp() },
      ]
    : [
        { label: 'View profile', icon: <User />, onSelect: () => openWeb(`/user/${username}`) },
        ...foldedProfile,
        { label: 'Kryos and perks', icon: <Sparkles />, hint: fold >= FOLD.kryos && typeof account.kryos === 'number' ? compactKryos(account.kryos) : undefined, onSelect: () => openWeb('/perks') },
        { label: 'Saves & achievements', icon: <Cloud />, onSelect: () => openSettings('cloud') },
        { label: 'Settings', icon: <SettingsIcon />, hint: newSettings.length ? 'New' : 'Ctrl ,', onSelect: () => openSettings(newSettings[0] ?? 'profile') },
        ...folded,
        ...common,
        { label: 'Sign out', icon: <LogOut />, onSelect: leaveAccount },
        { label: 'Exit Kryoto', onSelect: () => void exitApp() },
      ]

  /** A row of What's new, pressed: the place it names. */
  const openPlace = (place: WhatsNewPlace) => {
    if (place.kind === 'settings') openSettings(place.section, place.anchor)
    else if (place.kind === 'bridge') setOverlay({ kind: 'bridge' })
    else if (place.kind === 'library') go({ kind: 'home' })
    else openWeb('/')
  }

  /* ── Store helpers ── */
  const pageSlug = view.kind === 'web' ? slugOnPage(webUrl, settings?.catalogEndpoint) : null
  const pageGame = pageSlug ? (lib.games.find((g) => g.slug === pageSlug) ?? null) : null
  const addFromPage = pageSlug
    ? () => (pageGame ? go({ kind: 'game', id: pageGame.id }) : setOverlay({ kind: 'add', slug: pageSlug }))
    : null

  // Downloads that start while the client is open (not the resumable leftovers
  // it starts with) are news.
  const known = useRef<Set<string> | null>(null)
  useEffect(() => {
    if (known.current === null) {
      if (dl.length) known.current = new Set(dl.map((d) => d.id))
      else window.setTimeout(() => (known.current ??= new Set()), 1500)
      return
    }
    for (const d of dl) {
      if (known.current.has(d.id)) continue
      known.current.add(d.id)
      // Before kryo.to has named it, the title is a slug or a placeholder.
      const title = d.meta.title?.trim() ?? ''
      const named = title !== '' && title.toLowerCase() !== 'download' && title !== d.slug
      push({
        title: named ? `Downloading ${title}` : 'Download started',
        body: 'It installs itself when done. Open Downloads to watch it.',
        gameId: null,
      })
    }
  }, [dl, push])

  const openToast = (t: Toast) => {
    setBigPicture(false)
    if (t.settings) openSettings(t.settings.section, t.settings.anchor)
    else if (t.controllerGuid) { requestPad(t.controllerGuid); openSettings('controller') }
    else if (t.gameId) go({ kind: 'game', id: t.gameId })
    else go({ kind: 'downloads' })
  }
  const openNotification = (url: string | null, id?: string) => {
    if (id) markRead(id)
    if (!url) return openWeb('/notifications')
    if (url.startsWith('/')) return openWeb(url)
    if (/^https:\/\/kryo\.to\//.test(url)) return openWeb(url.replace('https://kryo.to', ''))
    void openExternal(url)
  }

  /* ── Library area ── */
  // A listed game that has since been installed opens as the game it now is.
  const catalogInstalled = view.kind === 'catalog' ? (lib.games.find((g) => g.slug === view.slug) ?? null) : null
  const selectedId = view.kind === 'game' ? view.id : catalogInstalled?.id ?? null
  const selected = selectedId ? gameById(selectedId) : null
  const catalogSlug = view.kind === 'catalog' && !catalogInstalled ? view.slug : null

  // Changing tab slides the new page in from the side that tab is on: right
  // for a tab further along the row, left for one before it. The Store is a
  // native view and shows as it is; this is for the client's own pages.
  const stage = useRef({ key: '', tab: currentTab, dx: 0 })
  const stageKey = view.kind === 'web' ? 'web' : view.kind === 'settings' ? 'settings' : view.kind === 'friends' ? 'friends' : 'library'
  if (stage.current.key !== stageKey) {
    const order: TopTab[] = ['store', 'library', 'community', 'profile']
    const step = Math.sign(order.indexOf(currentTab) - order.indexOf(stage.current.tab))
    stage.current = { key: stageKey, tab: currentTab, dx: stage.current.key === '' ? 0 : step * 28 }
  }

  let content: React.ReactNode = null
  if (view.kind === 'settings') {
    content = (
      <div className="absolute inset-0 flex bg-background">
        <SettingsPage
          section={view.section}
          reveal={settingsReveal}
          onSection={(section) => {
            openSettings(section)
          }}
          account={account}
          page={page}
          onSignOut={leaveAccount}
          onSignIn={signIn}
        />
      </div>
    )
  } else if (view.kind === 'friends') {
    content = (
      <div className="absolute inset-0 flex bg-background">
        <FriendsPage account={account} onProfile={guest ? signIn : () => openWeb(`/user/${username}`)} onWeb={openWeb} chatWith={chatWith} games={inviteGames} onJoinInvite={joinInvite} />
      </div>
    )
  } else if (view.kind === 'downloads') {
    content = (
      <div className="absolute inset-0 flex bg-background">
        <DownloadsPage
          loading={downloadState.loading}
          error={downloadState.error}
          onRetry={downloadState.retry}
          list={dl}
          onOpenGame={(id) => go({ kind: 'game', id })}
          onOpenCatalog={(slug) => go({ kind: 'catalog', slug })}
          onRepair={(id) => setOverlay({ kind: 'props', id, tab: 'repair' })}
          onStore={() => openWeb('/')}
          onDonate={account.supporter ? null : () => openWeb('/donate')}
        />
      </div>
    )
  } else if (view.kind === 'home' || view.kind === 'game' || view.kind === 'catalog') {
    content = (
      <div className="absolute inset-0 flex bg-background">
        <Sidebar
          games={lib.games}
          hiddenSlugs={hiddenSlugs}
          downloads={dl}
          running={lib.running}
          selectedId={selectedId}
          homeActive={view.kind === 'home'}
          onHome={() => go({ kind: 'home' })}
          onSelect={(id) => go({ kind: 'game', id })}
          onPlay={play}
          onStop={(game) => void lib.stop(game.id)}
          onContext={(game, x, y) => setCtx({ game, x, y })}
          onSavedContext={(saved, x, y) => setCtx({ saved, x, y })}
          onDownloadContext={(download, x, y) => setCtx({ download, x, y })}
          onDownloads={() => go({ kind: 'downloads' })}
          saved={saved}
          selectedSlug={catalogSlug}
          onCatalogGame={(slug) => go({ kind: 'catalog', slug })}
          onWeb={openWeb}
          signedIn={!guest}
          onBulk={guest ? undefined : (slugs, status) => setSavedStatusMany(slugs, status).catch(e => lib.setError(errorText(e)))}
          onRemoveLauncher={(launcher) => setOverlay({ kind: 'unlist', launcher })}
        />
        {catalogSlug ? (
          <CatalogGamePage
            key={catalogSlug}
            slug={catalogSlug}
            onContext={(x, y) => setCtx({ saved: { slug: catalogSlug, title: saved.find(e => e.slug === catalogSlug)?.title ?? catalogSlug, cover: saved.find(e => e.slug === catalogSlug)?.cover ?? '' }, x, y })}
            fallback={(() => {
              const entry = saved.find((e) => e.slug === catalogSlug)
              return { title: entry?.title ?? catalogSlug, cover: entry?.cover || null }
            })()}
            download={dl.find((d) => d.slug === catalogSlug) ?? null}
            savedStatus={saved.find((e) => e.slug === catalogSlug)?.status ?? null}
            onSetStatus={(st) => void setSavedStatus(catalogSlug, st).catch((e) => lib.setError(errorText(e)))}
            onInstall={(release) => openOurs(catalogSlug, release)}
            onStorePage={() => openWeb(`/game/${catalogSlug}`)}
            onHaveIt={() => setOverlay({ kind: 'add', slug: catalogSlug })}
            onDownloads={() => go({ kind: 'downloads' })}
          />
        ) : (view.kind === 'game' || catalogInstalled) && selected ? (
          <GamePage
            key={selected.id}
            game={selected}
            copy={copyLabel(selected, lib.games)}
            onContext={(x, y) => setCtx({ game: selected, x, y })}
            running={lib.running.has(selected.id)}
            error={lib.errorGame === selected.id ? lib.error : null}
            onDismissError={() => lib.setError(null)}
            onRepair={() => setOverlay({ kind: 'props', id: selected.id, tab: 'repair' })}
            onPlay={() => play(selected)}
            onPlayEntry={(i) => playEntry(selected, i)}
            onPlayVr={() => gate(selected, () => void lib.play(selected.id, null, undefined, true))}
            onStop={() => void lib.stop(selected.id)}
            gearItems={manageMenu(selected, true)}
            onStorePage={selected.slug ? () => openWeb(`/game/${selected.slug}`) : null}
            onWeb={openWeb}
            onGetUpdate={() => selected.slug && openOurs(selected.slug, null, true)}
            download={selected.slug ? (dl.find((d) => d.slug === selected.slug) ?? null) : null}
            onDownloads={() => go({ kind: 'downloads' })}
            onLatest={noteLatest}
            onGameChanged={lib.upsert}
            savedStatus={selected.slug ? (saved.find((e) => e.slug === selected.slug)?.status ?? null) : null}
            onSetStatus={guest ? undefined : (st) =>
              selected.slug &&
              void setSavedStatus(selected.slug, st, {
                title: selected.title,
                cover: selected.hero ?? selected.cover,
              }).catch((e) => lib.setError(errorText(e)))
            }
          />
        ) : !lib.loaded || (lib.error && lib.games.length === 0) ? (
          <EmptyState art={SHELF} title={lib.error ? 'Your library did not load' : 'Loading your games'} body={lib.error ?? undefined}>
            {lib.error ? <Button onClick={() => lib.reload()}>Retry</Button> : null}
          </EmptyState>
        ) : lib.games.length === 0 ? (
          <EmptyLibrary onStore={() => openWeb('/')} onAdd={() => setOverlay({ kind: 'add', slug: null })} />
        ) : (
          <LibraryHome
            games={shownGames}
            running={lib.running}
            onOpen={(id) => go({ kind: 'game', id })}
            onPlay={play}
            onStop={(game) => void lib.stop(game.id)}
            onContext={(game, x, y) => setCtx({ game, x, y })}
            saved={saved}
            hiddenSlugs={hiddenSlugs}
            downloads={dl}
            onCatalogGame={(slug) => go({ kind: 'catalog', slug })}
            onInstall={(slug) => openOurs(slug)}
            onSavedContext={(entry, x, y) => setCtx({ saved: entry, x, y })}
          />
        )}
      </div>
    )
  }

  const overlayGame = overlay && 'id' in overlay ? gameById(overlay.id) : null
  // Voice calls ring wherever you are in the app.
  const friendsState = useFriends()
  const callerOf = (id: string) => friendsState?.friends.find((x) => x.id === id) ?? friendsState?.messageRequests.accepted.find((x) => x.id === id)
  const callerName = (id: string) => {
    const f = callerOf(id)
    return f ? friendName(f) : 'Someone'
  }
  const callerAvatar = (id: string) => callerOf(id)?.avatarUrl ?? null

  return (
    <div className="flex h-full flex-col bg-background">
      <TitleBar
        offline={!online}
        account={account}
        inbox={inbox}
        news={news}
        menus={titleMenus}
        accountMenu={accountMenu}
        newSettings={newSettings.length > 0}
        rowRef={row}
        fold={fold}
        onNews={() => openWeb('/changelog')}
        onOpenNotification={openNotification}
        onMarkRead={markAllRead}
        onAllNotifications={() => openWeb('/notifications')}
        onKryos={() => openWeb('/perks#kryos')}
        where={view.kind === 'web' ? safePath(page.url) : view.kind}
        nav={
          <NavBar
            current={currentTab}
            overStore={storeVisible}
            tabs={tabs}
            canBack={canBack}
            canForward={canForward}
            onBack={back}
            onForward={forward}
            fold={fold}
            right={
              view.kind === 'web' ? (
                <UrlPill page={page} actions={web} onLibrary={addFromPage} inLibrary={!!pageGame} onAdblock={() => setOverlay({ kind: 'adblock' })} compact={fold >= FOLD.addIcon} narrow={fold >= FOLD.address} />
              ) : null
            }
          />
        }
      />
      <main className="relative min-h-0 grow">
        {/* Always mounted: the Store loads (and says who is signed in) even
            when the client opens on the Library. */}
        <div className="absolute inset-0" style={{ visibility: view.kind === 'web' ? 'visible' : 'hidden' }}>
          <WebSlot
            page={page}
            onRetry={web.retry}
            offline={!online}
            outage={outage && (storeBlocked || !!page.error) ? health : null}
            onLibrary={() => go({ kind: 'home' })}
          />
        </div>
        {content ? (
          <div key={stageKey} className="kryo-tab-in absolute inset-0" style={{ '--tab-dx': `${stage.current.dx}px` } as React.CSSProperties}>
            {content}
          </div>
        ) : null}
      </main>
      {padStore.on && padStore.config?.navigate && !bigPicture ? (
        <PadHints family={familyOf(firstPad, firstPad ? padStore.config?.pads[firstPad.guid] : null)} store={storeVisible} />
      ) : null}
      <ServiceNotice health={health} store={storeOpen} />
      <BottomBar
        downloads={dl}
        notice={view.kind === 'web' ? (toasts[toasts.length - 1] ?? null) : null}
        onNotice={(t) => {
          dismiss(t.key)
          openToast(t)
        }}
        onDismissNotice={dismiss}
        onAddGame={() => setOverlay({ kind: 'add', slug: pageSlug })}
        onDownloads={() => go({ kind: 'downloads' })}
        onFriends={account.friends ? () => go({ kind: 'friends' }) : undefined}
        friendsActive={view.kind === 'friends'}
      />
      {view.kind !== 'web' || bigPicture ? (
        <Toasts toasts={toasts} onOpen={openToast} onDismiss={dismiss}>
          <FeatureNotice onWeb={openWeb} />
          <WriteupNudge games={lib.games} running={lib.running} signedIn={!guest} onWeb={openWeb} />
        </Toasts>
      ) : null}

      {ctx ? <ContextMenu key={ctx.track ? 'track' : 'menu'} x={ctx.x} y={ctx.y} items={contextItems()} onClose={() => setCtx(null)} /> : null}

      {overlay?.kind === 'add' ? (
        <AddGameDialog
          initialSlug={overlay.slug}
          onClose={() => setOverlay(null)}
          onImport={() => setOverlay({ kind: 'bridge' })}
          onDownload={(slug) => {
            setOverlay(null)
            openOurs(slug)
          }}
          onSave={
            guest
              ? null
              : (game) => {
                  setOverlay(null)
                  const was = saved.find((e) => e.slug === game.slug)?.status ?? null
                  const art = { title: game.title, cover: game.cover ?? '' }
                  void setSavedStatus(game.slug, 'plan', art)
                    .then(() =>
                      push({
                        title: `${game.title} saved`,
                        body: 'It is in your library under All on Kryoto, as Plan to Play.',
                        gameId: null,
                        action: { label: 'Undo', run: () => setSavedStatus(game.slug, was, art).catch((e) => lib.setError(errorText(e))) },
                      }),
                    )
                    .catch(() => push({ title: 'Could not save it', body: 'kryo.to did not answer. Try again in a moment.', gameId: null }))
                }
          }
          onAdded={(game) => {
            lib.upsert(game)
            setOverlay(null)
            go({ kind: 'game', id: game.id })
            push({ title: `${game.title} added`, body: 'It is in your library, ready to play.', gameId: game.id })
          }}
        />
      ) : null}
      {overlay?.kind === 'choose' && overlayGame ? (
        <LaunchChooser
          game={overlayGame}
          onClose={() => setOverlay(null)}
          onPlay={(entry, remember) => {
            setOverlay(null)
            if (remember) {
              void library
                .save({ ...overlayGame, preferredEntry: entry, askEachTime: false })
                .then(lib.upsert)
                .catch((e) => lib.setError(errorText(e)))
            }
            playEntry(overlayGame, entry)
          }}
        />
      ) : null}
      {overlay?.kind === 'props' && overlayGame ? (
        <GameProperties
          key={overlayGame.id}
          game={overlayGame}
          startTab={overlay.tab}
          onClose={() => setOverlay(null)}
          onSaved={(game) => {
            lib.upsert(game)
            setOverlay(null)
          }}
          onUninstall={() => setOverlay({ kind: 'uninstall', id: overlayGame.id })}
          onOurs={(release) => overlayGame.slug && openOurs(overlayGame.slug, release, true)}
        />
      ) : null}
      {overlay?.kind === 'uninstall' && overlayGame ? (
        <UninstallDialog
          key={overlayGame.id}
          game={overlayGame}
          onClose={() => setOverlay(null)}
          onDone={(deleted) => {
            lib.drop(overlayGame.id)
            setOverlay(null)
            go({ kind: 'home' })
            push({
              title: `${overlayGame.title} ${deleted ? 'uninstalled' : 'removed'}`,
              body: deleted ? 'Its files were deleted.' : 'Taken off your library. Its files are where they were.',
              gameId: null,
            })
          }}
        />
      ) : null}
      {overlay?.kind === 'unlist' ? (
        <UnlistDialog
          launcher={overlay.launcher}
          games={lib.games.filter((g) => g.launcher === overlay.launcher)}
          onClose={() => setOverlay(null)}
          onRemoved={(ids) => {
            ids.forEach((id) => lib.drop(id))
            setOverlay(null)
            if (view.kind === 'game' && ids.includes(view.id)) go({ kind: 'home' })
            push({
              title: `${launcherName(overlay.launcher)} games removed`,
              body: `They are still in ${launcherName(overlay.launcher)}. Bridge brings them back.`,
              gameId: null,
              action: { label: 'Open Bridge', run: () => setOverlay({ kind: 'bridge' }) },
            })
          }}
        />
      ) : null}
      <CloudSavesPrompt />
      {playtimeGate.modal}
      {/* These and the notice above open by themselves, one at a time (lib/prompts.ts). */}
      <SetupPrompt onPlace={openPlace} />
      <ReleaseIntro onWeb={openWeb} />
      <FoundGamesAfterUpdate />
      <InstallFolderPrompt />
      <CollectionInstall />
      <UpdatePrompt
        busy={{
          games: lib.running.size,
          downloads: dl.filter((d) => d.status === 'downloading' || d.status === 'queued').length,
        }}
      />
      {overlay?.kind === 'bridge' ? <Bridge onClose={() => setOverlay(null)} /> : null}
      {overlay?.kind === 'setup' ? <SetupDialog steps={SETUP_AGAIN} title="Set up Kryoto Desktop" onClose={() => setOverlay(null)} /> : null}
      {overlay?.kind === 'news' ? (
        <SetupDialog
          steps={SETUP_NEWS}
          title={`What's new in Kryoto Desktop ${__APP_VERSION__}`}
          onClose={() => setOverlay(null)}
          onPlace={openPlace}
        />
      ) : null}
      {overlay?.kind === 'adblock' ? (
        <AdblockDialog
          onClose={() => setOverlay(null)}
          onLearnMore={() => {
            setOverlay(null)
            openWeb('/faq#desktop-adblock')
          }}
        />
      ) : null}
      {overlay?.kind === 'about' ? (
        <Modal title="About" onClose={() => setOverlay(null)}>
          <div className="grid justify-items-center gap-5 py-4 text-center">
            <KryoAscii animate className="text-[11px]" />
            <p className="text-[10px] uppercase tracking-[0.3em] text-muted-foreground">Kryoto Desktop {__APP_VERSION__}</p>
            <div className="flex gap-2">
              <Button size="sm" onClick={() => { setOverlay(null); openWeb('/changelog') }}>
                Changelog
              </Button>
              <Button size="sm" onClick={() => { setOverlay(null); openWeb('/support') }}>
                Support
              </Button>
              <Button size="sm" onClick={() => openExternal(SOURCE_URL)}>
                Source
                <ArrowUpRight className="size-3" />
              </Button>
            </div>
            <UpdateCheck onInstall={() => setOverlay(null)} />
            <p className="max-w-xs text-[11px] leading-relaxed text-muted-foreground">
              Kryoto Desktop is open source. Read the code, report a bug or send a fix on GitHub, and star it if you like it.
            </p>
          </div>
        </Modal>
      ) : null}
      {account.voice ? <CallOverlay nameOf={callerName} avatarOf={callerAvatar} usernameOf={id => callerOf(id)?.username ?? null} /> : null}
      {bigPicture ? (
        <BigPicture
          games={lib.games}
          running={lib.running}
          onPlay={play}
          onStop={(g) => void lib.stop(g.id)}
          onContext={(game, x, y) => setCtx({ game, x, y })}
          onExit={() => setBigPicture(false)}
          playerName={account.displayName || account.username || 'Guest'}
          playerUsername={!guest && account.username ? account.username : undefined}
          onProfile={() => { setBigPicture(false); openWeb(`/user/${encodeURIComponent(account.username)}`) }}
        />
      ) : null}
    </div>
  )
}

function EmptyLibrary({ onStore, onAdd }: { onStore: () => void; onAdd: () => void }) {
  return (
    <EmptyState art={SHELF} title="No games yet" body="Games you download from the store appear here. You can also add games already on this PC.">
      <Button variant="primary" onClick={onStore}>
        Browse the store
      </Button>
      <Button onClick={onAdd}>
        <Plus className="size-3.5" />
        Add a game
      </Button>
    </EmptyState>
  )
}

/** Off the Library, every one: nothing in the launcher changes, and Bridge can bring them back. */
function UnlistDialog({ launcher, games, onClose, onRemoved }: { launcher: string; games: LibraryGame[]; onClose: () => void; onRemoved: (ids: string[]) => void }) {
  const name = launcherName(launcher) ?? launcher
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const remove = async () => {
    setBusy(true)
    const gone: string[] = []
    try {
      for (const g of games) {
        await library.remove(g.id, false)
        gone.push(g.id)
      }
      onRemoved(gone)
    } catch (e) {
      setError(errorText(e))
      setBusy(false)
    }
  }
  return (
    <Modal
      title={`Remove ${games.length} ${name} game${games.length === 1 ? '' : 's'}`}
      onClose={onClose}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="danger" disabled={busy} onClick={() => remove()}>
            <Trash2 className="size-3.5" />
            {busy ? 'Removing' : 'Remove from Kryoto'}
          </Button>
        </>
      }
    >
      <p className="text-xs leading-relaxed text-muted-foreground">They leave your Kryoto Library only. Nothing changes in {name}: the games stay installed and start from there as before. Bridge can bring them back.</p>
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
    </Modal>
  )
}

function UninstallDialog({ game, onClose, onDone }: { game: LibraryGame; onClose: () => void; onDone: (deleted: boolean) => void }) {
  const keeper = launcherName(game.launcher)
  const [deleteFiles, setDeleteFiles] = useState(!keeper)
  const [deletePrefix, setDeletePrefix] = useState(false)
  const [hasPrefix, setHasPrefix] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    let off = false
    gameLogs.prefix(game.id).then((p) => !off && setHasPrefix(!!p)).catch(() => {})
    return () => {
      off = true
    }
  }, [game.id])
  return (
    <Modal
      title={keeper ? `Remove ${game.title}` : `Uninstall ${game.title}`}
      onClose={onClose}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="danger"
            disabled={busy}
            onClick={() => {
              setBusy(true)
              library
                .remove(game.id, deleteFiles, hasPrefix && deletePrefix)
                .then(onDone)
                .catch((e) => {
                  setError(errorText(e))
                  setBusy(false)
                })
            }}
          >
            <Trash2 className="size-3.5" />
            {keeper ? 'Remove' : busy ? 'Uninstalling' : 'Uninstall'}
          </Button>
        </>
      }
    >
      {keeper ? (
        <p className="text-xs text-muted-foreground">{`It stays installed in ${keeper}. Only its place in your Kryoto Library goes.`}</p>
      ) : (
        <>
          <Check checked={deleteFiles} onChange={setDeleteFiles} label="Delete the game's files" />
          <p className="kryo-ascii-art select-text break-all text-[11px] text-muted-foreground">{game.installDir}</p>
        </>
      )}
      {hasPrefix ? (
        <Check
          checked={deletePrefix}
          onChange={setDeletePrefix}
          label="Also delete its Wine prefix (its settings, and any saves it keeps there)"
        />
      ) : null}
      {keeper ? null : (
        <p className="text-xs leading-relaxed text-muted-foreground">
          Files are only deleted for games installed into your library folder. A game you added from somewhere else is only taken
          off the list. Play time is forgotten either way.
        </p>
      )}
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
    </Modal>
  )
}
