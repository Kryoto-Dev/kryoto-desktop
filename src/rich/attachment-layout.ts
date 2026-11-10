/**
 * How pictures and clips sit in a comment or a chat message: each one whole, at
 * its real shape, scaled down to fit and never cropped. One file is drawn as
 * big as the limits allow but never past its own size. Several sit in rows of
 * up to two, the row's items sharing one height, so a tall picture beside a
 * wide one takes less width and neither loses an edge.
 *
 * Every number comes from the file's known size, so the space is held before
 * the file loads and nothing below it jumps. A file with no known size (an
 * older one) is laid out as 16:9 until it has loaded.
 *
 * The page gets a row's `maxWidth` in pixels and an `aspectRatio` per item, and
 * CSS does the rest: on a phone the row shrinks to the column and every item
 * keeps its shape.
 *
 * Pure and dependency-free: copied byte for byte into Kryoto Desktop and Kryoto
 * Mobile with the rest of rich text (`pnpm sync:rich-text`).
 */

export type Sized = { width?: number | null; height?: number | null }

/** The limits of the whole block, as on Discord. */
export const MEDIA_MAX_WIDTH = 550
export const MEDIA_MAX_HEIGHT = 350
/** The gap between items, in pixels. */
export const MEDIA_GAP = 4

const FALLBACK_RATIO = 16 / 9
/** A panorama or a sliver still gets a shape a person can click. */
const MIN_RATIO = 0.25
const MAX_RATIO = 6

export type MediaCell<T> = { item: T; ratio: number }
export type MediaRow<T> = { cells: MediaCell<T>[]; maxWidth: number }

/** width / height, kept within what can be drawn. */
export function mediaRatio(file: Sized): number {
  const { width, height } = file
  if (!width || !height || width <= 0 || height <= 0) return FALLBACK_RATIO
  return Math.min(MAX_RATIO, Math.max(MIN_RATIO, width / height))
}

/** One file alone: as big as the limits allow, never bigger than it is. */
export function singleMediaWidth(file: Sized, limits: { width: number; height: number } = { width: MEDIA_MAX_WIDTH, height: MEDIA_MAX_HEIGHT }): number {
  const ratio = mediaRatio(file)
  const fit = Math.min(limits.width, limits.height * ratio)
  return Math.max(1, Math.round(file.width ? Math.min(file.width, fit) : fit))
}

/** Files in rows of up to two, an odd one out first and alone, each row sized to the limits. */
export function mediaRows<T extends Sized>(items: readonly T[]): MediaRow<T>[] {
  if (items.length === 0) return []
  if (items.length === 1) return [{ cells: [{ item: items[0]!, ratio: mediaRatio(items[0]!) }], maxWidth: singleMediaWidth(items[0]!) }]
  const rows: MediaRow<T>[] = []
  let at = 0
  // Three files: the first on its own above the other two, as Discord lays them out.
  if (items.length % 2 === 1) {
    rows.push(rowOf(items.slice(0, 1)))
    at = 1
  }
  for (; at < items.length; at += 2) rows.push(rowOf(items.slice(at, at + 2)))
  return rows
}

function rowOf<T extends Sized>(items: readonly T[]): MediaRow<T> {
  const cells = items.map((item) => ({ item, ratio: mediaRatio(item) }))
  // All items share a height: the width that gives a row of the limit's height.
  const sum = cells.reduce((n, c) => n + c.ratio, 0)
  const gaps = MEDIA_GAP * (cells.length - 1)
  return { cells, maxWidth: Math.round(Math.min(MEDIA_MAX_WIDTH, MEDIA_MAX_HEIGHT * sum + gaps)) }
}
