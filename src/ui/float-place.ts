/**
 * Where a floating panel (a menu, a dropdown's list, a right-click menu) goes,
 * as a plain function with no app imports so `pnpm test` runs it.
 *
 * It opens on `side` and turns to the other one when that has more room, never
 * comes closer than `edge` to the window's sides, and gets a `maxHeight` when
 * it is taller than the room: then it scrolls (`ui/index.tsx` `Floating`).
 */
export type Anchor = { left: number; right: number; top: number; bottom: number }
export type FloatInput = {
  anchor: Anchor
  /** The panel's own size, at its full height. */
  width: number
  height: number
  /** The window's inner size, in CSS pixels. */
  view: { width: number; height: number }
  side?: 'top' | 'bottom'
  align?: 'left' | 'right'
  gap?: number
  edge?: number
  /**
   * May cover its anchor to show more of itself: a right-click menu opened in
   * the middle of a short window uses the whole window's height, like the
   * system's menus, instead of half of it.
   */
  cover?: boolean
}

export function placeFloat({ anchor: a, width, height, view, side = 'bottom', align = 'left', gap = 6, edge = 8, cover = false }: FloatInput) {
  const below = view.height - a.bottom - gap - edge
  const above = a.top - gap - edge
  const down = side === 'bottom' ? height <= below || below >= above : height > above && below > above
  const whole = Math.max(0, view.height - 2 * edge)
  const room = Math.max(0, Math.min(down ? below : above, whole))
  const shown = Math.min(height, cover ? whole : room)
  // Clamped: a control scrolled out of the window (or a pointer near its
  // bottom) takes its panel only as far as the edge, and a panel that covers
  // moves up over its anchor rather than off the window.
  const wanted = down ? a.bottom + gap : a.top - gap - shown
  const top = Math.max(edge, Math.min(wanted, view.height - shown - edge))
  const x = align === 'right' ? a.right - width : a.left
  const left = Math.max(edge, Math.min(x, view.width - width - edge))
  return { top, left, maxHeight: shown }
}
