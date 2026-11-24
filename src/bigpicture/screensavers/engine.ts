/**
 * The cell engine every screensaver draws through.
 *
 * The screen is one grid of cells, and a cell is either a dot of Kryoto's 3x3
 * matrix (ui/Matrix.tsx) or one ASCII glyph, at one of the matrix's four inks
 * (full, half, low, a trace) or not drawn at all. A screensaver only says what
 * each cell shows; the engine draws it, and keeps it cheap enough to leave on
 * a TV for hours:
 *
 * - One 2D canvas. Each dot and glyph is drawn once into a sprite sheet, and a
 *   frame only copies the cells that changed since the last one. Dots switch
 *   in hard steps, so a frame is usually a few dozen copies.
 * - At most `fps` frames a second (20 full screen), and none at all while the
 *   window is hidden.
 * - Nothing slides. The grid holds perfectly still, every cell on whole
 *   device pixels at any screen scale. What is lit moves only by going out
 *   dot by dot and coming back a whole number of cells away, every minute
 *   or two, so nothing stays lit on the same pixels for long; the trace dots
 *   are too dim to mark a screen.
 * - With motion reduced, one still frame, changed every few minutes.
 *
 * Kept free of imports from the app (and of the DOM at load time) so the pure
 * parts run under `pnpm test`.
 */

/* ── Cells ─────────────────────────────────────────────────── */

/** The matrix's four inks, and nothing. */
export const OFF = 0
export const TRACE = 1
export const LOW = 2
export const HALF = 3
export const FULL = 4
/**
 * Brighter than full: the light of a sweep passing, and nothing else, so the
 * passing light is the brightest thing on screen and a large lit shape is not.
 */
export const PEAK = 5
/**
 * components/ui/matrix.tsx's inks (full, half, low, trace), set a little
 * lower for a TV in a dark room: full is the foreground at 0.8, and only PEAK
 * reaches the foreground itself.
 */
export const INK_ALPHA = [0, 0.12, 0.26, 0.48, 0.8, 1] as const

const LEVEL_MASK = 7
const ACCENT = 8

/** A dot of the matrix. */
export const dot = (level: number, accent = false): number => (level ? level | (accent ? ACCENT : 0) : 0)

/** One character in a cell; a space is the grid's trace dot. */
export function glyph(ch: string, level: number, accent = false): number {
  if (!level) return 0
  if (ch === ' ') return dot(TRACE)
  return (ch.charCodeAt(0) << 4) | level | (accent ? ACCENT : 0)
}

export const levelOf = (cell: number) => cell & LEVEL_MASK
export const isGlyph = (cell: number) => cell >>> 4 !== 0

/* ── Pure helpers ──────────────────────────────────────────── */

/** A stable pseudo-random number in [0, 1) for a pair of numbers. */
export function hash(a: number, b = 0): number {
  const n = Math.sin(a * 127.1 + b * 311.7 + 7.3) * 43758.5453
  return n - Math.floor(n)
}

/** A small seeded generator (mulberry32), so a run can be replayed in tests. */
export function seeded(seed: number): () => number {
  let s = seed >>> 0
  return () => {
    s = (s + 0x6d2b79f5) >>> 0
    let t = s
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

/** The display's own frame lands within this of when it was asked for. */
const VSYNC_MS = 16

/**
 * How long to wait before asking for the next frame, so frames come no more
 * often than `fps` a second. Asked a display frame early: the frame itself
 * then waits for `capReady`.
 */
export function capDelay(last: number, now: number, fps: number): number {
  return Math.max(0, last + 1000 / fps - VSYNC_MS - now)
}

/** Whether a display frame at `now` may be drawn (a millisecond of slack for timer jitter). */
export function capReady(last: number, now: number, fps: number): boolean {
  return now - last >= 1000 / fps - 1
}

/**
 * A cell's width in device pixels: 80 across a 16:9 screen at `scale` 1, so
 * the picture is the same at 1080p and 4K, just sharper. Whole pixels, so
 * every dot is the same size.
 */
export function pitchFor(width: number, height: number, scale = 1): number {
  return Math.max(2, Math.round((Math.min(width, (height * 16) / 9) / 80) * scale))
}

/* ── What a screensaver gets ───────────────────────────────── */

export type Grid = {
  cols: number
  rows: number
  /** One value per cell, row by row (`dot`, `glyph`): what the cell shows this frame. */
  cells: Uint32Array
  /** The cells wholly on screen. */
  view: { c0: number; r0: number; c1: number; r1: number }
  /** A row's height as a share of a column's width. */
  rowPitch: number
}

export type Scene = {
  grid: Grid
  /** Game titles a screensaver may show, played last first: none hidden, none adult. */
  titles: () => readonly string[]
  /** The game played last, if any. */
  lastPlayed: () => string | null
  /** Motion reduced: draw a still composition for the time given. */
  reduced: boolean
  /** A thumbnail in Settings: same picture, nothing that needs reading. */
  thumb: boolean
  rand: () => number
  /** Ask for the start screen's own scene underneath (Field), or none. */
  underlay: (kind: 'splash' | null) => void
  /** Lay the grid out again for another screensaver's cells (Shuffle, between two). */
  setScale: (scale: number, rowPitch?: number) => Grid
}

export type Runner = {
  /** Fill `scene.grid.cells` for `t` ms after the start. */
  frame: (t: number) => void
}

export type VariantSpec = {
  id: string
  name: string
  /** Cell size against the default (Clock's numerals are drawn larger). */
  scale?: number
  /** Rows closer than columns (the mark's true shape). Default: the mark's. */
  rowPitch?: number
  /** With motion reduced, how long until the still frame changes. */
  stillFor?: (now: Date) => number
  make: (scene: Scene) => Runner
}

/** With motion reduced, a new still this often. */
export const STILL_MS = 180_000
/** Rows a little closer than columns, as the mark's dots are (mark-data.ts). */
export const ROW_PITCH = 0.885
/** A dot's side as a share of a column (the matrix's 14% gaps: 0.62). */
const DOT = 0.62

/**
 * The largest backing-store size, in device pixels, that fits `css` pixels and
 * whose CSS size the browser can lay out exactly (it works in 64ths of a CSS
 * pixel): then canvas and screen pixels match one to one at 125%, 150% or any
 * scale, and nothing is resampled into a shimmer.
 */
export function wholeBacking(css: number, dpr: number): number {
  let b = Math.max(1, Math.floor(css * dpr + 1e-6))
  for (let i = 0; i < 64 && b > 1; i++, b--) {
    const units = (b / dpr) * 64
    if (Math.abs(units - Math.round(units)) < 1e-6) return b
  }
  return Math.max(1, Math.floor(css * dpr + 1e-6))
}

/**
 * Five resting places for a block of `w` x `h` cells: the middle first, then
 * four a modest step from it (a `share` of the room there is), in whole cells.
 * A screensaver goes out and comes back at the next one, so the picture never
 * sits hard in a corner and never slides.
 */
export function restSpots(view: Grid['view'], w: number, h: number, share = 0.35): { c0: number; r0: number }[] {
  const c0 = Math.floor((view.c0 + view.c1 - w) / 2)
  const r0 = Math.floor((view.r0 + view.r1 - h) / 2)
  const dx = Math.max(0, Math.floor(((view.c1 - view.c0 - w) / 2) * share))
  const dy = Math.max(0, Math.floor(((view.r1 - view.r0 - h) / 2) * share))
  return [
    [0, 0],
    [-1, -1],
    [1, 1],
    [1, -1],
    [-1, 1],
  ].map(([a, b]) => ({ c0: c0 + a! * dx, r0: r0 + b! * dy }))
}

/* ── The engine ────────────────────────────────────────────── */

export type MountOptions = {
  fps: number
  titles: () => readonly string[]
  lastPlayed: () => string | null
  thumb?: boolean
  onUnderlay?: (kind: 'splash' | null) => void
  /** Measured for performance checks: the work of each frame, in ms. */
  onFrame?: (ms: number) => void
}

function reducedMotion() {
  return typeof window !== 'undefined' && !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches
}

/**
 * Draw `spec` into `canvas`, which must sit inside a positioned box: the
 * canvas is sized to that box in whole device pixels. Returns the stop.
 */
export function mountSaver(canvas: HTMLCanvasElement, spec: VariantSpec, opts: MountOptions): () => void {
  const ctx = canvas.getContext('2d', { alpha: true })
  const host = canvas.parentElement
  if (!ctx || !host) return () => {}

  const reduced = reducedMotion()
  let scale = spec.scale ?? 1
  let rowPitch = spec.rowPitch ?? ROW_PITCH

  // Geometry, in device pixels unless named CSS.
  let dpr = 1
  let pw = 2
  let ph = 2
  let ox = 0
  let oy = 0
  let grid: Grid = { cols: 0, rows: 0, cells: new Uint32Array(0), view: { c0: 0, r0: 0, c1: 0, r1: 0 }, rowPitch }
  let shown = new Uint32Array(0)

  // Colours: the ink is the foreground, the accent the reader's palette.
  let ink = '#ededed'
  let accent = ink
  let font = 'ui-monospace, monospace'
  const readPalette = () => {
    const css = getComputedStyle(canvas)
    ink = css.color || ink
    accent = css.getPropertyValue('--primary').trim() || ink
    font = css.getPropertyValue('--font-ascii').trim() || font
  }

  // The sprite sheet: every look a cell can have, drawn once.
  const sheet = document.createElement('canvas')
  const sctx = sheet.getContext('2d')!
  const SLOTS_X = 32
  const SLOTS_Y = 24
  let slots = new Map<number, number>()
  const clearSprites = () => {
    slots = new Map()
    sheet.width = pw * SLOTS_X
    sheet.height = ph * SLOTS_Y
    shown.fill(0xffffffff)
  }
  const sprite = (cell: number): number => {
    let slot = slots.get(cell)
    if (slot !== undefined) return slot
    if (slots.size >= SLOTS_X * SLOTS_Y) clearSprites()
    slot = slots.size
    slots.set(cell, slot)
    const x = (slot % SLOTS_X) * pw
    const y = Math.floor(slot / SLOTS_X) * ph
    sctx.clearRect(x, y, pw, ph)
    sctx.globalAlpha = INK_ALPHA[levelOf(cell)]!
    sctx.fillStyle = cell & ACCENT ? accent : ink
    const code = cell >>> 4
    if (!code) {
      const side = Math.max(1, Math.min(Math.round(pw * DOT), ph - 1))
      const dx = x + Math.floor((pw - side) / 2)
      const dy = y + Math.floor((ph - side) / 2)
      sctx.beginPath()
      if (side >= 6 && sctx.roundRect) sctx.roundRect(dx, dy, side, side, side * 0.2)
      else sctx.rect(dx, dy, side, side)
      sctx.fill()
    } else {
      sctx.font = `700 ${Math.max(4, Math.round(ph * 0.92))}px ${font}`
      sctx.textAlign = 'center'
      sctx.textBaseline = 'middle'
      sctx.fillText(String.fromCharCode(code), x + pw / 2, y + ph / 2 + ph * 0.04)
    }
    sctx.globalAlpha = 1
    return slot
  }

  const scene: Scene = {
    get grid() {
      return grid
    },
    titles: opts.titles,
    lastPlayed: opts.lastPlayed,
    reduced,
    thumb: !!opts.thumb,
    rand: Math.random,
    underlay: (kind) => opts.onUnderlay?.(kind),
    setScale: (next, rows) => {
      scale = next
      rowPitch = rows ?? ROW_PITCH
      layout()
      return grid
    },
  }

  const layout = () => {
    const box = host.getBoundingClientRect()
    const w = Math.max(1, box.width)
    const h = Math.max(1, box.height)
    dpr = window.devicePixelRatio || 1
    pw = pitchFor(w * dpr, h * dpr, scale)
    ph = Math.max(2, Math.round(pw * rowPitch))
    // Whole device pixels: the canvas is exactly as many CSS pixels as its
    // backing store allows, so the browser never resamples it (no shimmer at
    // 125% or 150%).
    canvas.width = wholeBacking(w, dpr)
    canvas.height = wholeBacking(h, dpr)
    Object.assign(canvas.style, { position: 'absolute', left: '0px', top: '0px', width: `${canvas.width / dpr}px`, height: `${canvas.height / dpr}px` })
    const cols = Math.floor(canvas.width / pw)
    const rows = Math.floor(canvas.height / ph)
    ox = Math.floor((canvas.width - cols * pw) / 2)
    oy = Math.floor((canvas.height - rows * ph) / 2)
    grid = {
      cols,
      rows,
      cells: new Uint32Array(cols * rows),
      view: { c0: 0, r0: 0, c1: cols, r1: rows },
      rowPitch,
    }
    shown = new Uint32Array(cols * rows)
    ctx.clearRect(0, 0, canvas.width, canvas.height)
    readPalette()
    clearSprites()
  }

  const paint = () => {
    const { cells, cols } = grid
    for (let i = 0; i < cells.length; i++) {
      const v = cells[i]!
      if (v === shown[i]) continue
      shown[i] = v
      const x = ox + (i % cols) * pw
      const y = oy + Math.floor(i / cols) * ph
      ctx.clearRect(x, y, pw, ph)
      if (!levelOf(v)) continue
      const slot = sprite(v)
      ctx.drawImage(sheet, (slot % SLOTS_X) * pw, Math.floor(slot / SLOTS_X) * ph, pw, ph, x, y, pw, ph)
    }
  }

  layout()
  let runner = spec.make(scene)

  const t0 = performance.now()
  let last = -Infinity
  let timer = 0
  let raf = 0
  let stopped = false

  const frame = (now: number) => {
    raf = 0
    if (stopped || document.hidden) return
    if (!reduced && !capReady(last, now, opts.fps)) {
      raf = requestAnimationFrame(frame)
      return
    }
    last = now
    const began = performance.now()
    // A display frame's time is when the frame began, which can be a moment before the start.
    const t = Math.max(0, now - t0)
    runner.frame(t)
    paint()
    opts.onFrame?.(performance.now() - began)
    schedule()
  }
  const schedule = () => {
    if (stopped || document.hidden || timer || raf) return
    const delay = reduced ? (spec.stillFor?.(new Date()) ?? STILL_MS) : capDelay(last, performance.now(), opts.fps)
    timer = window.setTimeout(() => {
      timer = 0
      raf = requestAnimationFrame(frame)
    }, delay)
  }
  raf = requestAnimationFrame(frame)

  // Hidden: nothing scheduled at all. Back: straight on.
  const visibility = () => {
    if (document.hidden) {
      window.clearTimeout(timer)
      cancelAnimationFrame(raf)
      timer = raf = 0
    } else if (!timer && !raf) raf = requestAnimationFrame(frame)
  }
  document.addEventListener('visibilitychange', visibility)

  // A new size lays the grid out again and starts the picture over.
  let size = `${host.clientWidth}x${host.clientHeight}x${window.devicePixelRatio}`
  const resized = () => {
    const next = `${host.clientWidth}x${host.clientHeight}x${window.devicePixelRatio}`
    if (next === size) return
    size = next
    scale = spec.scale ?? 1
    rowPitch = spec.rowPitch ?? ROW_PITCH
    layout()
    runner = spec.make(scene)
    if (reduced) {
      window.clearTimeout(timer)
      timer = 0
      if (!raf) raf = requestAnimationFrame(frame)
    }
  }
  const watch = new ResizeObserver(resized)
  watch.observe(host)
  window.addEventListener('resize', resized)

  // The palette or the font changed (Settings, a font arriving): redraw every cell.
  const repaint = () => {
    readPalette()
    clearSprites()
  }
  const themes = new MutationObserver(repaint)
  themes.observe(document.documentElement, { attributes: true, attributeFilter: ['class', 'data-palette', 'style'] })
  document.fonts?.addEventListener?.('loadingdone', repaint)

  return () => {
    stopped = true
    window.clearTimeout(timer)
    cancelAnimationFrame(raf)
    document.removeEventListener('visibilitychange', visibility)
    watch.disconnect()
    window.removeEventListener('resize', resized)
    themes.disconnect()
    document.fonts?.removeEventListener?.('loadingdone', repaint)
    opts.onUnderlay?.(null)
  }
}

/* ── Small drawing helpers for the screensavers ────────────── */

/** Fill every cell with `value` (the trace grid, or nothing). */
export function fill(grid: Grid, value: number) {
  grid.cells.fill(value)
}

/** Put one cell, ignoring anything off the grid. */
export function put(grid: Grid, c: number, r: number, value: number) {
  if (c < 0 || r < 0 || c >= grid.cols || r >= grid.rows) return
  grid.cells[r * grid.cols + c] = value
}

/** Write a line of text from (c, r), one character per cell. */
export function text(grid: Grid, c: number, r: number, line: string, level: number, accent = false) {
  for (let i = 0; i < line.length; i++) put(grid, c + i, r, glyph(line[i]!, level, accent))
}
