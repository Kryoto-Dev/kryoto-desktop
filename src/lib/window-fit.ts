/**
 * The client window's size kept inside the screen it opens on, as a plain
 * function (no app imports) so `pnpm test` runs it. At 150% a 1920 by 1080
 * screen is 1280 by 720 in the window's own pixels, less the taskbar: the
 * client's 1280 by 800 opened past its bottom edge, where nothing could
 * reach the edge to resize it (KB-0321).
 */
export type Size = { width: number; height: number }

/** Room left around the window on each side, so its edges stay in reach. */
export const SCREEN_MARGIN = 16

/**
 * `size` and `min`, both no larger than `room` (the work area, in logical
 * pixels, or null when the screen is unknown) less the margin.
 */
export function fitWindow(size: Size, min: Size, room: Size | null): { size: Size; min: Size } {
  if (!room) return { size, min }
  const width = Math.max(1, Math.floor(room.width - 2 * SCREEN_MARGIN))
  const height = Math.max(1, Math.floor(room.height - 2 * SCREEN_MARGIN))
  const fitMin = { width: Math.min(min.width, width), height: Math.min(min.height, height) }
  return {
    size: { width: Math.round(Math.min(size.width, width)), height: Math.round(Math.min(size.height, height)) },
    min: fitMin,
  }
}
