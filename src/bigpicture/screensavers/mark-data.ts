/**
 * The K// mark as cells, ported from the Discord art recipe
 * (kryo.to scripts/discord-assets/build.py, `trace_mark`), which traces
 * kryo.to's own image (public/brand/kryo-mark-ascii.png). These are that
 * function's output, copied as data, never re-traced by eye:
 *
 *   `#` full ink, a block of the lettering
 *   `+` low ink, a line of its double-line shadow
 *   `.` a trace, the grid showing through
 *
 * MARK_CELLS is `trace_mark()`: one cell per character the mark was typeset
 * in, the server icon (server-icon-mark.gif). MARK_DOTS is `trace_mark(2, 4)`:
 * every character cut into 2 x 4 dots, the board the invite art drew, for a
 * mark drawn big enough that the shadow reads as lines. Rows of MARK_DOTS sit
 * MARK_ROW_PITCH of a column's pitch apart, so the mark keeps its true shape.
 *
 * Kept free of imports so `pnpm test` can load it.
 */

export const MARK_CELLS: readonly string[] = [
  '##+..##+.....##+..##',
  '##+.##++....##+..##+',
  '#####++....##+..##+.',
  '##++##+...##+..##+..',
  '##+.+##+.##+..##+...',
  '+++..+++##+..##+....',
]

export const MARK_DOTS: readonly string[] = [
  '####+.....####+...........####+.....####',
  '####++....####++..........####+.....####',
  '####++....#####+..........####+.....####',
  '####++....####++..........####+.....####',
  '####++..######++........######+...######',
  '####++..####++++........####+.....####+.',
  '####++..####++++........####+.....####+.',
  '####++..####++..........####+.....####+.',
  '############++........######+...######+.',
  '##########++++........####+.....####+...',
  '##########++++........####+.....####+...',
  '##########++..........####+.....####+...',
  '############+.......######+...######+...',
  '####++++####++......####+.....####+.....',
  '####++++####++......####+.....####+.....',
  '####++..####++......####+.....####+.....',
  '####++..######+...######+...######+.....',
  '####++....####++..####+.....####+.......',
  '####++....#####+..####+.....####+.......',
  '####++....####++..####+.....####+.......',
  '####++....####++######+...######+.......',
  '++++++....++++++####+.....####+.........',
  '++++++....++++++####+.....####+.........',
  '................####+.....####+.........',
]

/** A character of the mark is 1.77 times as tall as it is wide; cut into 2 x 4 dots, a row is this much of a column. */
export const MARK_ROW_PITCH = (2 * 1.77) / 4

export const MARK_COLS = MARK_DOTS[0]!.length
export const MARK_ROWS = MARK_DOTS.length
