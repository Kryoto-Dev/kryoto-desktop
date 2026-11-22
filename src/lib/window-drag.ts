import { getCurrentWindow } from '@tauri-apps/api/window'
import { isTauri } from '@/lib/window'

/**
 * Moving the frameless window by its `.drag` areas, where the system does not.
 *
 * On Windows the `.drag` areas are `app-region: drag` (styles.css): WebView2
 * hands them to Windows as the title bar, so the press never reaches this
 * page and dragging, Aero Snap, double-click and the window menu are the
 * system's. This is the fallback for WebKitGTK (no app-region) and for any
 * WebView2 that still delivers the press. A press arms a move, which starts
 * once the pointer actually moves a few pixels (starting on the press itself
 * swallowed the second click of a double-click), and a double press on an
 * area marked `data-maximize` maximizes or restores.
 */
/**
 * A touch screen (a ROG Ally, a Legion Go, a touch laptop): no `app-region`
 * at all (styles.css `[data-touch]`). Chromium on Windows does not deliver a
 * tap to a control inside a `drag` region, even one marked `no-drag`: on a
 * ROG Ally, Sign in and the close button did nothing to a finger, while the
 * pet (a window with no drag region) answered (KB-0369). Dragging the window
 * by its title bar is then this file's, as on Linux.
 */
export function markTouchScreen() {
  try {
    if (navigator.maxTouchPoints > 0) document.documentElement.dataset.touch = '1'
  } catch {
    /* only a styling hint */
  }
}

const CONTROL = '.drag, .no-drag, button, a[href], input, textarea, select, [role="button"], [contenteditable="true"]'
/** How far the pointer moves with the button down before the window follows. */
const SLOP = 4

export function installWindowDrag() {
  if (!isTauri()) return
  markTouchScreen()
  let armed: { x: number; y: number } | null = null

  window.addEventListener('mousedown', (e) => {
    armed = null
    if (e.button !== 0 || !(e.target instanceof Element)) return
    const zone = e.target.closest(CONTROL)
    if (!zone?.classList.contains('drag')) return
    e.preventDefault()
    if (e.detail === 2) {
      if (zone.closest('[data-maximize]')) void getCurrentWindow().toggleMaximize()
      return
    }
    armed = { x: e.screenX, y: e.screenY }
  })
  window.addEventListener('mousemove', (e) => {
    if (!armed) return
    if (!(e.buttons & 1)) {
      armed = null
      return
    }
    if (Math.abs(e.screenX - armed.x) < SLOP && Math.abs(e.screenY - armed.y) < SLOP) return
    armed = null
    void getCurrentWindow().startDragging()
  })
  window.addEventListener('mouseup', () => (armed = null))
  window.addEventListener('blur', () => (armed = null))
}
