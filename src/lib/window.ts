import { useEffect, useState } from 'react'
import { LogicalSize } from '@tauri-apps/api/dpi'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { call, isTauri, on } from '@/lib/bridge'

export { isTauri }

/**
 * The window and the Store web view.
 *
 * The Store is a real web view (kryo.to refuses to be framed), created in Rust
 * and laid over the shell exactly where `WebSlot` sits. It is a native layer
 * on top of the page, so anything the shell draws over that area - a menu, a
 * dialog - needs the view hidden while it is open (`setStoreVisible`).
 */

/**
 * `splash`: the small box the client starts in. `welcome`: the sign-in screen.
 * `main`: the client. The first two are fixed-size boxes; Windows rounds the
 * corners and draws the shadow (see `round_corners` in system.rs).
 */
export type WindowKind = 'splash' | 'welcome' | 'main'

const SIZES: Record<WindowKind, { width: number; height: number }> = {
  splash: { width: 340, height: 380 },
  welcome: { width: 960, height: 640 },
  main: { width: 1280, height: 800 },
}

let currentKind: WindowKind | null = null

export async function applyWindow(kind: WindowKind) {
  if (kind === 'main') delete document.documentElement.dataset.shape
  else document.documentElement.dataset.shape = 'box'
  if (!isTauri() || currentKind === kind) return
  const win = getCurrentWindow()
  const first = currentKind === null
  currentKind = kind
  const size = SIZES[kind]
  const logical = new LogicalSize(size.width, size.height)
  if (kind === 'main' && !first) {
    // Keep a size the player chose last time they were in the client.
    try {
      const saved = JSON.parse(localStorage.getItem('kryoto.window') ?? 'null') as { width: number; height: number } | null
      if (saved && saved.width >= 1000 && saved.height >= 640) logical.width = saved.width
      if (saved && saved.height >= 640) logical.height = saved.height
    } catch {
      /* first run, or storage unavailable */
    }
  }
  await win.setResizable(kind === 'main')
  if (kind === 'main') {
    await win.setMaxSize(null)
    await win.setMinSize(new LogicalSize(1000, 640))
  } else {
    await win.setMinSize(logical)
    await win.setMaxSize(logical)
  }
  await win.setSize(logical)
  await win.center()
}

/* ── Window controls ───────────────────────────────────── */

export type WindowState = { maximized: boolean; fullscreen: boolean }

const NORMAL: WindowState = { maximized: false, fullscreen: false }

/**
 * Whether the window is maximized or full screen, as it changes. The native
 * side says so once per change (`window-state`), so nothing here asks the
 * window again on every step of a resize.
 */
export function useWindowState(): WindowState {
  const [state, setState] = useState<WindowState>(NORMAL)
  useEffect(() => {
    if (!isTauri()) return
    let stop: (() => void) | undefined
    let cancelled = false
    void call<WindowState>('window_state_get').then((s) => !cancelled && setState(s)).catch(() => {})
    void on<WindowState>('window-state', setState).then((fn) => (cancelled ? fn() : (stop = fn)))
    return () => {
      cancelled = true
      stop?.()
    }
  }, [])
  return state
}

export const minimizeWindow = () => (isTauri() ? getCurrentWindow().minimize() : Promise.resolve())
export const toggleMaximize = () => (isTauri() ? getCurrentWindow().toggleMaximize() : Promise.resolve())
/** The close button: to the tray or quit, as Settings says (system.rs). */
export const closeWindow = () => (isTauri() ? getCurrentWindow().close() : Promise.resolve())
export async function toggleFullscreen() {
  if (!isTauri()) return
  const win = getCurrentWindow()
  await win.setFullscreen(!(await win.isFullscreen()))
}
/** Quit for real (the close button may only hide to the tray). */
export const exitApp = () => (isTauri() ? call<void>('app_exit') : Promise.resolve())

/* ── Store web view ─────────────────────────────────────── */

export const STORE_HOME = 'https://kryo.to/'

/** Remember the client's size for next time. */
export function rememberWindowSize(width: number, height: number) {
  try {
    localStorage.setItem('kryoto.window', JSON.stringify({ width, height }))
  } catch {
    /* not important */
  }
}

/**
 * Create the Store web view (first call) or move it. It carries this
 * window's own user agent with `KryotoDesktop/<version>` on the end, which is
 * how kryo.to knows to hide its own navigation and to name the device.
 */
export function mountStore(url: string, rect: { x: number; y: number; width: number; height: number }, visible?: boolean) {
  if (!isTauri()) return Promise.resolve()
  // The page's own pixel ratio, so the view lands where it was measured even
  // when Windows reports another scale for the window (src-tauri placement.rs).
  return call<void>('store_mount', { url, ...rect, visible: visible ?? null, userAgent: navigator.userAgent, scale: window.devicePixelRatio || null })
}

/*
 * The Store's web view has one home, the main slot, and sometimes a guest
 * spot (Settings shows kryo.to's settings in its pane). The main slot leaves
 * a way to put the view back when the guest is done.
 */
let mainPlacer: (() => void) | null = null
export function setMainPlacer(fn: (() => void) | null) {
  mainPlacer = fn
}
export function placeMainStore() {
  mainPlacer?.()
}

export function setStoreVisible(visible: boolean) {
  if (!isTauri()) return Promise.resolve()
  return call<void>('store_visible', { visible }).catch(() => {})
}

/**
 * Show a kryo.to page in the Store tab from anywhere in the app. The Shell owns
 * which view is on screen, so it takes this and switches to the Store first:
 * navigateCatalog alone loads the page behind whatever view is showing, which
 * is how the achievements card's Manage button seemed to do nothing.
 */
export const OPEN_STORE_EVENT = 'kryo:open-store'
export function openStorePage(path: string) {
  window.dispatchEvent(new CustomEvent<string>(OPEN_STORE_EVENT, { detail: path }))
}

/** Open a section of Kryoto Desktop's Settings from anywhere in the app. */
export const OPEN_SETTINGS_EVENT = 'kryo:open-settings'
export function openSettingsSection(section: string) {
  window.dispatchEvent(new CustomEvent<string>(OPEN_SETTINGS_EVENT, { detail: section }))
}

/** Go to a kryo.to path in the Store. */
export function navigateCatalog(path: string) {
  if (!isTauri()) return Promise.resolve()
  return call<void>('navigate_catalog', { path })
}

/** Go to any http(s) address in the Store. */
export function browserNavigate(url: string) {
  if (!isTauri()) return Promise.resolve()
  return call<void>('browser_navigate', { url })
}

export function catalogUrl() {
  if (!isTauri()) return Promise.resolve(STORE_HOME)
  return call<string>('catalog_url')
}

export function controlBrowser(action: 'back' | 'forward' | 'reload' | 'stop') {
  if (!isTauri()) return Promise.resolve()
  return call<void>('control_catalog', { action })
}

export function signOut() {
  if (!isTauri()) return Promise.resolve()
  return call<void>('store_sign_out')
}

export function openExternal(url: string) {
  if (!isTauri()) {
    window.open(url, '_blank', 'noopener')
    return Promise.resolve()
  }
  return call<void>('open_external', { url })
}
