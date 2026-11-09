import { createElement, isValidElement, type ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { Check } from '@/ui/pixel-icons'
import { call, errorText, isTauri, on } from '@/lib/bridge'
import { logWarn } from '@/lib/log'
import type { Inbox } from '@/hooks/useInbox'

/**
 * Menus over the Store.
 *
 * The Store is a native web view on top of the shell, so a menu drawn in the
 * shell's page would open *under* it. In the app, menus are drawn by one more
 * web view inside the main window, stacked above the Store (`menus.rs`). This
 * side sends what to draw and where, runs whatever gets picked, and keeps
 * track of which menu is open so its trigger can light up.
 */

export type PopupItem =
  | { id: string; label: string; hint?: string; icon?: string; danger?: boolean; disabled?: boolean; checked?: boolean }
  | { separator: true }
  | { heading: string }

/** The shell's colours as the menu in the Store's page needs them (that page has its own styles). */
export type MenuColors = {
  popover: string
  foreground: string
  border: string
  muted: string
  primary: string
  primaryForeground: string
  destructive: string
  destructiveForeground: string
  radius: string
  font: string
}

export type PopupLook = { palette: string; radius: string; font: string | undefined; season?: string; theme: 'light' | 'dark'; colors?: MenuColors }

export type PopupPayload =
  | { menu: string; kind: 'menu'; items: PopupItem[]; minWidth?: number; focus?: boolean; look: PopupLook }
  | { menu: string; kind: 'inbox'; inbox: Inbox; look: PopupLook }
  | { menu: string; kind: 'tip'; text: string; look: PopupLook }

/** A menu entry the shell hands over: the same shape the page menus use. */
export type NativeEntry =
  | { label: string; onSelect: () => void; hint?: string; disabled?: boolean; danger?: boolean; icon?: ReactNode; checked?: boolean }
  | { separator: true }
  | { heading: string }

/** Where a menu hangs from: its trigger's box, in the main window. */
export type Anchor = { left: number; right: number; bottom: number }

let handlers = new Map<string, () => void>()
let current: string | null = null
const listeners = new Set<(open: string | null) => void>()
const hoverListeners = new Set<(inside: boolean) => void>()
let wired = false
/** The open menu is on screen, and not still on its way there. */
let shown = false
/** The menu a press on its own trigger just closed: the click that ends the press must not reopen it. */
let pressClosed: string | null = null

function setCurrent(menu: string | null) {
  if (current === menu) return
  current = menu
  shown = false
  listeners.forEach((l) => l(menu))
}

function openFailed(menu: string, e: unknown) {
  logWarn('menu', `${menu} did not open: ${errorText(e)}`)
  if (current === menu) setCurrent(null)
}

function wire() {
  if (wired || !isTauri()) return
  wired = true
  void on<string>('menu-pick', (id) => handlers.get(id)?.())
  void on<string>('menu-shown', (menu) => {
    if (menu === current) shown = true
  })
  void on<{ menu: string | null; back?: boolean }>('menu-closed', ({ menu, back }) => {
    if (!menu || menu === current) setCurrent(null)
    // Closed from the keyboard: the keyboard stays where it was.
    if (back && menu) document.querySelector<HTMLElement>(`[data-menu="${CSS.escape(menu)}"]`)?.focus({ preventScroll: true })
  })
  void on<boolean>('menu-hover', (inside) => hoverListeners.forEach((l) => l(inside)))
  // A press anywhere in the shell closes the open menu, its own trigger
  // included. A menu still on its way stays: on its trigger that press is the
  // same request again, and the first menu of a session takes a moment.
  window.addEventListener(
    'pointerdown',
    (e) => {
      if (!current) return
      const trigger = (e.target as Element | null)?.closest?.('[data-menu]')
      const own = trigger?.getAttribute('data-menu') === current
      if (own && !shown) return
      if (own) pressClosed = current
      closeMenu()
    },
    true,
  )
  // A press can end somewhere else, and then no click comes to clear it.
  window.addEventListener('pointerup', () => window.setTimeout(() => (pressClosed = null), 50), true)
  window.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && current) closeMenu()
  })
}

function menuColors(): MenuColors {
  const css = getComputedStyle(document.documentElement)
  const v = (name: string) => css.getPropertyValue(name).trim()
  return {
    popover: v('--popover'),
    foreground: v('--popover-foreground'),
    border: v('--border'),
    muted: v('--muted-foreground'),
    primary: v('--primary'),
    primaryForeground: v('--primary-foreground'),
    destructive: v('--destructive'),
    destructiveForeground: v('--destructive-foreground'),
    radius: v('--kryo-radius'),
    font: getComputedStyle(document.body).fontFamily,
  }
}

function look(): PopupLook {
  const d = document.documentElement.dataset
  return { palette: d.palette ?? 'monochrome', radius: d.radius ?? 'pill', font: d.font, season: d.season, theme: document.documentElement.classList.contains('dark') ? 'dark' : 'light', colors: menuColors() }
}

function iconMarkup(icon: ReactNode): string | undefined {
  if (!isValidElement(icon)) return undefined
  try {
    return renderToStaticMarkup(icon)
  } catch {
    return undefined
  }
}

function anchorOf(r: DOMRect | Anchor): Anchor {
  return { left: r.left, right: r.right, bottom: r.bottom }
}

export function popupOpen() {
  return current
}

/** Whether this click is the end of the press that closed `menu`, or `menu` is still opening: either way it is not a new request. */
export function pressHandled(menu: string) {
  const closed = pressClosed === menu
  pressClosed = null
  return closed || (current === menu && !shown)
}

/** Make the menu view ahead of the click that needs it: the pointer is on its way to a trigger. */
export function warmMenus() {
  if (isTauri()) void call('menu_warm').catch(() => {})
}

export async function openMenu(menu: string, anchor: DOMRect | Anchor, entries: NativeEntry[], align: 'left' | 'right' = 'left', minWidth?: number, focus = false) {
  wire()
  handlers = new Map()
  const items: PopupItem[] = entries.map((e, i) => {
    if ('separator' in e || 'heading' in e) return e
    const id = `${menu}:${i}`
    handlers.set(id, e.onSelect)
    // A checked row without an icon of its own shows a check, in both views
    // that draw menus (PopupApp, and the Store page's `__kryoMenu`).
    const icon = e.icon ?? (e.checked ? createElement(Check) : undefined)
    return { id, label: e.label, hint: e.hint, danger: e.danger, disabled: e.disabled, checked: e.checked, icon: iconMarkup(icon) }
  })
  setCurrent(menu)
  await call<boolean>('menu_open', { scale: window.devicePixelRatio || null, anchor: anchorOf(anchor), right: align === 'right', payload: { menu, kind: 'menu', items, minWidth, focus, look: look() } }).then(
    (drawn) => {
      if (drawn && current === menu) shown = true
    },
    (e: unknown) => openFailed(menu, e),
  )
}

export async function openInbox(
  menu: string,
  anchor: DOMRect | Anchor,
  inbox: Inbox,
  actions: { open: (url: string | null) => void; markRead: () => void; all: () => void },
) {
  wire()
  handlers = new Map()
  inbox.notifications.forEach((n, i) => handlers.set(`${menu}:open:${i}`, () => actions.open(n.url)))
  handlers.set(`${menu}:read`, actions.markRead)
  handlers.set(`${menu}:all`, actions.all)
  setCurrent(menu)
  await call('menu_open', { scale: window.devicePixelRatio || null, anchor: anchorOf(anchor), right: true, payload: { menu, kind: 'inbox', inbox, look: look() } }).catch((e: unknown) =>
    openFailed(menu, e),
  )
}

export function closeMenu() {
  if (!current) return
  const menu = current
  setCurrent(null)
  void call('menu_close', { menu }).catch(() => {})
}

/** Where the Store's view is while it is on screen (`lib/window.ts`). */
let storeBox: { x: number; y: number; width: number; height: number } | null = null
let storeShown = false
export function noteStore(box: typeof storeBox, shown?: boolean) {
  if (box) storeBox = box
  if (shown !== undefined) storeShown = shown
}

/** Whether this part of the window is under the Store: anything the shell draws there is covered. */
export function underStore(r: { left: number; right: number; top: number; bottom: number }) {
  const s = storeBox
  return !!s && storeShown && r.left < s.x + s.width && r.right > s.x && r.top < s.y + s.height && r.bottom > s.y
}

/**
 * A tooltip over the Store: the menu view draws it, as it does a menu, because
 * the shell's page is under the Store there. `top` is where it goes; it never
 * takes the keyboard.
 */
export function openTip(text: string, target: DOMRect, top: number) {
  // The view has one thing to show, and an open menu is the one that matters.
  if (current) return
  void call('menu_open', {
    scale: window.devicePixelRatio || null,
    anchor: { left: target.left, right: target.right, bottom: top - 6 },
    right: false,
    payload: { menu: 'tip', kind: 'tip', text, look: look() },
  }).catch(() => {})
}

export function closeTip() {
  void call('menu_close', { menu: 'tip' }).catch(() => {})
}

/** Which menu is open, as it changes. */
export function onPopupChange(fn: (open: string | null) => void) {
  wire()
  listeners.add(fn)
  return () => {
    listeners.delete(fn)
  }
}

/** The pointer entering or leaving the menu, for menus that open on hover. */
export function onPopupHover(fn: (inside: boolean) => void) {
  wire()
  hoverListeners.add(fn)
  return () => {
    hoverListeners.delete(fn)
  }
}
